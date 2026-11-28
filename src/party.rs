//! Party: ephemeral rooms that let CREBLOX users launch into the same Roblox server.
//!
//! Design constraints, in order of importance:
//!
//! * **Nothing to deploy.** The host *is* the server: Create Party binds a TCP listener
//!   inside this process, Join dials the host directly, and the whole feature ships inside
//!   the compiled binary. No account, no relay, no URL in settings.
//! * **Nothing runs when you are not in a party.** One dedicated worker thread blocks on its
//!   command channel while idle; no listener, no socket, no timer.
//! * **One thread, blocking sockets.** No async runtime and no tokio. While a party is open
//!   the worker wakes on a 25 ms tick and reads each socket with a 1 ms timeout, so it costs
//!   a handful of syscalls a second and nothing at all once the party closes.
//! * **No Roblox credentials on the wire.** Members are identified by public user id, name,
//!   display name and headshot URL. Launch tickets and `.ROBLOSECURITY` never leave the
//!   machine: each member's own client joins the same (place id, job id).
//! * **A party survives a closed port.** When the host cannot be dialled (CGNAT, strict
//!   router, port mapping refused) members keep the party alive by watching the host's public
//!   Roblox presence and launch into the server the host enters.
//! * **Ephemeral.** A room exists only while its members are listening; leaving drops it.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::server::{
    ErrorResponse, Request as ServerRequest, Response as ServerResponse,
};
use tungstenite::http::header::{HeaderName, HeaderValue};
use tungstenite::http::StatusCode;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::{Message, WebSocket};

use crate::types::Msg;

/// The host's room size. Friends beyond this are refused rather than silently dropped.
pub const MAX_PARTY_SIZE: usize = 8;

/// Keepalive to every peer. Also how a vanished host is noticed while connected.
///
/// A socket that closes is dropped on the very tick it notices, but a peer that disappears
/// without closing its socket (wifi drop, sleep, killed network) is only found by a keepalive
/// write that fails - so this also bounds how long a peer can sit stale. Five seconds is a
/// handful of tiny frames a minute to the few people in a party.
const HEARTBEAT: Duration = Duration::from_secs(5);
/// Worker wake-up while a party is open. Small enough that a click feels instant, large
/// enough that an idle party is free.
const TICK: Duration = Duration::from_millis(25);
/// One socket read per peer per tick.
const READ_TIMEOUT: Duration = Duration::from_millis(1);
/// Dialling a host has to fail fast: a CGNAT host must fall back to presence, not hang.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// A peer that connects and then says nothing is dropped after this.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const BACKOFF_MAX: Duration = Duration::from_secs(16);
/// Presence fallback poll: one user id, while in a party.
const PRESENCE_POLL: Duration = Duration::from_secs(4);
/// How long Create waits for a router to accept a port mapping before falling back to LAN.
const MAPPING_WAIT: Duration = Duration::from_secs(5);

/// CREBLOX-only handshake: a host refuses anything without these.
const MAGIC_HEADER: &str = "x-creblox-party";
const TOKEN_HEADER: &str = "x-creblox-token";
const MAGIC_VALUE: &str = "1";

/// Close code the host sends when it leaves. `4001` already means "you were removed";
/// this one means "the party is over", which is terminal for every member too.
const PARTY_CLOSED_CODE: u16 = 4000;

/// Whether this client is in a party right now. Read by the UI without touching a lock.
static IN_PARTY: AtomicBool = AtomicBool::new(false);

pub fn in_party() -> bool {
    IN_PARTY.load(Ordering::Relaxed)
}

/// A `creblox://party/<code>` link the process was started with (double-clicked, or handed
/// over by the protocol handler). The UI takes it once the window exists, so a shared link
/// behaves exactly like pasting the code into the join box.
static PENDING_INVITE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn set_pending_invite(raw: &str) {
    if let Ok(mut slot) = PENDING_INVITE.lock() {
        *slot = Some(raw.to_string());
    }
}

pub fn take_pending_invite() -> Option<String> {
    PENDING_INVITE.lock().ok().and_then(|mut slot| slot.take())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartyStatus {
    Ready,
    Launching,
    InGame,
}

impl PartyStatus {
    pub fn wire(self) -> &'static str {
        match self {
            PartyStatus::Ready => "ready",
            PartyStatus::Launching => "launching",
            PartyStatus::InGame => "ingame",
        }
    }

    fn from_wire(raw: &str) -> Self {
        match raw {
            "launching" => PartyStatus::Launching,
            "ingame" => PartyStatus::InGame,
            _ => PartyStatus::Ready,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PartyStatus::Ready => "Ready",
            PartyStatus::Launching => "Launching",
            PartyStatus::InGame => "In game",
        }
    }
}

/// The public identity shown to the rest of the party. Never contains a cookie or a ticket.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyUser {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub headshot_url: String,
}

impl PartyUser {
    pub fn label(&self) -> &str {
        if self.display_name.is_empty() {
            &self.name
        } else {
            &self.display_name
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyMember {
    pub user: PartyUser,
    pub status: PartyStatus,
    pub place_id: u64,
    pub job_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartySnapshot {
    /// Short room label, e.g. `CRB-8X2K0M`. Safe to read out loud.
    pub code: String,
    /// The full invite the host shares: endpoint, token and host user id, `CRB-…`. Empty for
    /// the second or two the host spends asking its router for a port.
    pub invite: String,
    pub host: u64,
    pub max: usize,
    pub members: Vec<PartyMember>,
    /// The socket is up right now (false while the direct connection is down and the party is
    /// running on the presence fallback).
    pub connected: bool,
    /// Ready Check and Game Vote. Empty when nobody uses them.
    pub board: PartyBoard,
}

impl PartySnapshot {
    pub fn host_member(&self) -> Option<&PartyMember> {
        self.members.iter().find(|m| m.user.id == self.host)
    }

    pub fn is_host(&self, user_id: u64) -> bool {
        self.host == user_id
    }

    /// Place and job the host is playing, for the "Join Party's Server" button.
    pub fn host_target(&self) -> Option<(u64, String)> {
        self.host_member()
            .filter(|m| m.place_id > 0 && !m.job_id.is_empty())
            .map(|m| (m.place_id, m.job_id.clone()))
    }
}

/// What the host asks everyone else to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyLaunch {
    pub place_id: u64,
    pub job_id: String,
    pub universe_id: u64,
    pub game_name: String,
    pub by: u64,
}

/// Whether this member should open a client for the server a launch names.
///
/// Auto-follow has two sources - the host's launch frame and the host's Roblox presence -
/// and both can name the same server more than once. Follow each job id once, and never
/// relaunch into the server this member is already playing in.
pub fn should_follow(
    last_job: Option<&str>,
    current_job: Option<&str>,
    place_id: u64,
    job_id: &str,
) -> bool {
    if place_id == 0 || job_id.is_empty() {
        return false;
    }
    last_job != Some(job_id) && current_job != Some(job_id)
}

impl Guest {
    /// Whether to follow this server, remembering the job either way so the other follow
    /// path (the host's frame vs. their presence) does not repeat it.
    fn accept_launch(&mut self, local: Option<&Local>, place_id: u64, job_id: &str) -> bool {
        let current = local.map(|l| l.job_id.as_str()).filter(|job| !job.is_empty());
        let follow = should_follow(self.last_followed.as_deref(), current, place_id, job_id);
        if place_id > 0 && !job_id.is_empty() {
            self.last_followed = Some(job_id.to_string());
        }
        follow
    }
}

/// One game a member suggested for the party to play next.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartySuggestion {
    pub place_id: u64,
    #[serde(default)]
    pub universe_id: u64,
    #[serde(default)]
    pub name: String,
}

/// Ready Check and Game Vote state.
///
/// The host owns this; guests mirror the last `Members` frame. It travels as an *optional
/// field on existing frames*, never as a new frame type: a 0.5.5 client in the room ignores
/// the key, and a 0.5.5 host ignores the optional guest fields, so neither side disconnects.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyBoard {
    /// Wall-clock ms when the running ready check expires; 0 when none is running.
    #[serde(default)]
    pub ready_until_ms: u64,
    /// `(user id, ready)` for everybody who answered the running check.
    #[serde(default)]
    pub ready: Vec<(u64, bool)>,
    #[serde(default)]
    pub suggestions: Vec<PartySuggestion>,
    /// `(user id, place id)`: one vote per member.
    #[serde(default)]
    pub votes: Vec<(u64, u64)>,
}

impl PartyBoard {
    pub const READY_SECONDS: u64 = 10;
    pub const MAX_SUGGESTIONS: usize = 3;

    pub fn ready_check_active(&self) -> bool {
        self.ready_until_ms > 0
    }

    /// Whole seconds left, rounded up, for the countdown in the party bar and the HUD.
    pub fn ready_seconds_left(&self, now_ms: u64) -> u64 {
        self.ready_until_ms
            .saturating_sub(now_ms)
            .div_ceil(1000)
    }

    pub fn start_ready_check(&mut self, now_ms: u64) {
        self.ready_until_ms = now_ms.saturating_add(Self::READY_SECONDS * 1000);
        self.ready.clear();
    }

    /// Clears an expired check. Returns whether anything changed, so the host only announces
    /// on the transition.
    pub fn expire(&mut self, now_ms: u64) -> bool {
        if self.ready_until_ms > 0 && now_ms >= self.ready_until_ms {
            self.ready_until_ms = 0;
            self.ready.clear();
            return true;
        }
        false
    }

    pub fn is_ready(&self, user: u64) -> Option<bool> {
        self.ready
            .iter()
            .find(|(id, _)| *id == user)
            .map(|(_, ready)| *ready)
    }

    /// `(ready, answered)` for the running check.
    pub fn ready_count(&self) -> (usize, usize) {
        (
            self.ready.iter().filter(|(_, ready)| *ready).count(),
            self.ready.len(),
        )
    }

    pub fn set_ready(&mut self, user: u64, ready: bool) -> bool {
        if let Some(entry) = self.ready.iter_mut().find(|(id, _)| *id == user) {
            if entry.1 == ready {
                return false;
            }
            entry.1 = ready;
            return true;
        }
        self.ready.push((user, ready));
        true
    }

    /// Adds a suggestion: at most three, one per place, and never a duplicate.
    pub fn suggest(&mut self, suggestion: PartySuggestion) -> bool {
        if suggestion.place_id == 0
            || self.suggestions.len() >= Self::MAX_SUGGESTIONS
            || self
                .suggestions
                .iter()
                .any(|s| s.place_id == suggestion.place_id)
        {
            return false;
        }
        self.suggestions.push(suggestion);
        true
    }

    /// One vote per member, and only for a game that is actually on the poll.
    pub fn vote(&mut self, user: u64, place_id: u64) -> bool {
        if !self.suggestions.iter().any(|s| s.place_id == place_id) {
            return false;
        }
        if let Some(vote) = self.votes.iter_mut().find(|(id, _)| *id == user) {
            if vote.1 == place_id {
                return false;
            }
            vote.1 = place_id;
            return true;
        }
        self.votes.push((user, place_id));
        true
    }

    pub fn votes_for(&self, place_id: u64) -> usize {
        self.votes
            .iter()
            .filter(|(_, place)| *place == place_id)
            .count()
    }

    pub fn my_vote(&self, user: u64) -> Option<u64> {
        self.votes
            .iter()
            .find(|(id, _)| *id == user)
            .map(|(_, place)| *place)
    }

    /// Votes per suggestion, most votes first; ties keep the order the games were suggested
    /// in, so the poll never reshuffles on a repaint.
    pub fn tally(&self) -> Vec<(u64, usize)> {
        let mut out: Vec<(u64, usize)> = self
            .suggestions
            .iter()
            .map(|s| (s.place_id, self.votes_for(s.place_id)))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }

    /// The game with the most votes, or `None` while nobody has voted.
    pub fn winner(&self) -> Option<&PartySuggestion> {
        let (place_id, votes) = *self.tally().first()?;
        if votes == 0 {
            return None;
        }
        self.suggestions.iter().find(|s| s.place_id == place_id)
    }
}

#[derive(Clone, Debug)]
pub enum PartyEvent {
    Snapshot(PartySnapshot),
    Launch(PartyLaunch),
    /// A one-line message for the status bar (not an error state).
    Notice(String),
    /// A member cannot join the party's server and asks the host to move it.
    HopRequest(u64),
    /// The party ended: left, kicked, or unrecoverable.
    Left(String),
}

// ---------------------------------------------------------------------------------------
// Invite codes
// ---------------------------------------------------------------------------------------

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// 22 payload bytes = 176 bits; 62^30 > 2^176, so 30 digits always fit and never overflow.
const INVITE_DIGITS: usize = 30;
const INVITE_PREFIX: &str = "CRB-";
const INVITE_VERSION: u8 = 1;

/// Everything a friend needs to reach this host's party, carried in the invite code: the
/// public endpoint, the host's Roblox user id (for the presence fallback) and the room token
/// that authenticates the socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartyInvite {
    pub address: Ipv4Addr,
    pub port: u16,
    pub host_id: u64,
    pub token: [u8; 6],
}

impl PartyInvite {
    fn payload(&self) -> [u8; 22] {
        let mut out = [0u8; 22];
        out[0] = INVITE_VERSION;
        out[1..5].copy_from_slice(&self.address.octets());
        out[5..7].copy_from_slice(&self.port.to_be_bytes());
        out[7..15].copy_from_slice(&self.host_id.to_be_bytes());
        out[15..21].copy_from_slice(&self.token);
        out[21] = crc8(&out[..21]);
        out
    }

    fn from_payload(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 22 || bytes[0] != INVITE_VERSION || crc8(&bytes[..21]) != bytes[21] {
            return None;
        }
        let mut token = [0u8; 6];
        token.copy_from_slice(&bytes[15..21]);
        Some(PartyInvite {
            address: Ipv4Addr::new(bytes[1], bytes[2], bytes[3], bytes[4]),
            port: u16::from_be_bytes([bytes[5], bytes[6]]),
            host_id: u64::from_be_bytes([
                bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
            ]),
            token,
        })
    }

    /// The full, copyable invite.
    pub fn code(&self) -> String {
        format!(
            "{INVITE_PREFIX}{}",
            base62_encode(&self.payload(), INVITE_DIGITS)
        )
    }

    /// Short label for the party pill: derived from the token, so it changes per room.
    pub fn short_code(&self) -> String {
        format!("{INVITE_PREFIX}{}", base62_encode(&self.token[..3], 6))
    }

    pub fn endpoint(&self) -> SocketAddr {
        SocketAddr::from((self.address, self.port))
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Base62 of a big-endian byte string, fixed width. The invite is 22 bytes (176 bits), which
/// does not fit a u128, so the digits are produced by repeated in-place division.
fn base62_encode(bytes: &[u8], width: usize) -> String {
    let mut number = bytes.to_vec();
    let mut out = vec![b'0'; width];
    for slot in out.iter_mut().rev() {
        let mut remainder: u32 = 0;
        for byte in number.iter_mut() {
            let current = (remainder << 8) | u32::from(*byte);
            *byte = (current / 62) as u8;
            remainder = current % 62;
        }
        *slot = BASE62[remainder as usize];
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Inverse of [`base62_encode`]. `None` for a non-base62 character or a value too wide for
/// `width` bytes (which is also how a code that was over-typed past the alphabet is rejected).
fn base62_decode(text: &str, width: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; width];
    for byte in text.bytes() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'Z' => byte - b'A' + 10,
            b'a'..=b'z' => byte - b'a' + 36,
            _ => return None,
        } as u32;
        let mut carry = digit;
        for slot in out.iter_mut().rev() {
            let current = u32::from(*slot) * 62 + carry;
            *slot = (current & 0xff) as u8;
            carry = current >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(out)
}

/// Accepts a bare `CRB-…` code, one with stray spaces, `creblox://party/<code>` or a pasted
/// invite URL. The body is case-sensitive (base62), so it is never upper-cased.
pub fn parse_invite(raw: &str) -> Option<PartyInvite> {
    let tail = raw.trim().rsplit('/').next()?.trim();
    let body = if tail.len() >= 4 && tail[..4].eq_ignore_ascii_case("CRB-") {
        tail[4..].trim()
    } else {
        tail
    };
    if body.len() != INVITE_DIGITS {
        return None;
    }
    PartyInvite::from_payload(&base62_decode(body, 22)?)
}

// ---------------------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum WireOut {
    Join {
        code: String,
        user: PartyUser,
        max: usize,
    },
    Leave,
    Status {
        status: String,
        place_id: u64,
        job_id: String,
        // Party actions ride on the existing status frame, omitted when unset: an older host
        // reads exactly the frame it always did and never sees an unknown frame type.
        #[serde(skip_serializing_if = "Option::is_none")]
        ready: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        vote: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        suggest: Option<PartySuggestion>,
        /// A member asking the host to move the party to a server with room.
        #[serde(skip_serializing_if = "Option::is_none")]
        hop: Option<bool>,
    },
    Launch {
        place_id: u64,
        job_id: String,
        universe_id: u64,
        game_name: String,
    },
    Ping,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum WireIn {
    Welcome {
        code: String,
        host: u64,
        max: usize,
    },
    Members {
        host: u64,
        max: usize,
        members: Vec<WireMember>,
        /// Ready Check and Game Vote. An unknown key for a 0.5.5 guest, which ignores it.
        #[serde(default)]
        board: PartyBoard,
    },
    Launch {
        place_id: u64,
        job_id: String,
        #[serde(default)]
        universe_id: u64,
        #[serde(default)]
        game_name: String,
        #[serde(default)]
        by: u64,
    },
    Error {
        message: String,
    },
    Pong,
}

#[derive(Clone, Serialize, Deserialize)]
struct WireMember {
    user: PartyUser,
    #[serde(default)]
    status: String,
    #[serde(default)]
    place_id: u64,
    #[serde(default)]
    job_id: String,
}

impl WireMember {
    fn into_member(self) -> PartyMember {
        PartyMember {
            status: PartyStatus::from_wire(&self.status),
            user: self.user,
            place_id: self.place_id,
            job_id: self.job_id,
        }
    }
}

/// A member's first frame, and every later frame it sends the host.
#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum PeerFrame {
    Join {
        #[serde(default)]
        user: Option<PartyUser>,
    },
    Status {
        #[serde(default)]
        status: String,
        #[serde(default)]
        place_id: u64,
        #[serde(default)]
        job_id: String,
        #[serde(default)]
        ready: Option<bool>,
        #[serde(default)]
        vote: Option<u64>,
        #[serde(default)]
        suggest: Option<PartySuggestion>,
        #[serde(default)]
        hop: Option<bool>,
    },
    Launch {
        #[serde(default)]
        place_id: u64,
        #[serde(default)]
        job_id: String,
        #[serde(default)]
        universe_id: u64,
        #[serde(default)]
        game_name: String,
    },
    Kick {
        #[serde(default)]
        user_id: u64,
    },
    Promote {
        #[serde(default)]
        user_id: u64,
    },
    Leave,
    Ping,
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------

enum Command {
    Create {
        me: PartyUser,
        max: usize,
        cookie: Option<String>,
    },
    Join {
        invite: PartyInvite,
        me: PartyUser,
        cookie: Option<String>,
    },
    Leave,
    Kick(u64),
    Promote(u64),
    Status {
        status: PartyStatus,
        place_id: u64,
        job_id: String,
    },
    Launch(PartyLaunch),
    /// Host starts a 10-second Ready Check for everybody.
    ReadyCheck,
    /// This member answers the running check.
    SetReady(bool),
    /// Add a game to the poll (at most three).
    Suggest(PartySuggestion),
    /// Vote for a suggested game.
    Vote(u64),
    /// This member cannot join the party's server and asks the host to move it.
    RequestHop,
    Shutdown,
}

/// Cheap handle used by the UI and by `app.rs`.
pub struct PartyHandle {
    tx: Sender<Command>,
}

impl PartyHandle {
    pub fn new(tx: Sender<Msg>, ctx: egui::Context) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("creblox-party".to_string())
            .spawn(move || worker(cmd_rx, tx, ctx));
        Self { tx: cmd_tx }
    }

    /// `cookie` is the signed-in session the presence fallback borrows; it stays inside this
    /// process and is never sent to another member.
    pub fn create(&self, me: PartyUser, max: usize, cookie: Option<String>) {
        let _ = self.tx.send(Command::Create { me, max, cookie });
    }

    /// Ask the host to move the party to a server with room. Sent only when a late join
    /// found the party's server full.
    pub fn request_hop(&self) {
        let _ = self.tx.send(Command::RequestHop);
    }

    pub fn join(&self, invite: PartyInvite, me: PartyUser, cookie: Option<String>) {
        let _ = self.tx.send(Command::Join { invite, me, cookie });
    }

    pub fn leave(&self) {
        let _ = self.tx.send(Command::Leave);
    }

    pub fn kick(&self, user_id: u64) {
        let _ = self.tx.send(Command::Kick(user_id));
    }

    pub fn promote(&self, user_id: u64) {
        let _ = self.tx.send(Command::Promote(user_id));
    }

    pub fn set_status(&self, status: PartyStatus, place_id: Option<u64>, job_id: Option<String>) {
        let _ = self.tx.send(Command::Status {
            status,
            place_id: place_id.unwrap_or(0),
            job_id: job_id.unwrap_or_default(),
        });
    }

    pub fn broadcast_launch(&self, launch: PartyLaunch) {
        let _ = self.tx.send(Command::Launch(launch));
    }

    /// Host only; ignored for guests.
    pub fn start_ready_check(&self) {
        let _ = self.tx.send(Command::ReadyCheck);
    }

    pub fn set_ready(&self, ready: bool) {
        let _ = self.tx.send(Command::SetReady(ready));
    }

    pub fn suggest_game(&self, place_id: u64, universe_id: u64, name: String) {
        let _ = self.tx.send(Command::Suggest(PartySuggestion {
            place_id,
            universe_id,
            name,
        }));
    }

    pub fn vote(&self, place_id: u64) {
        let _ = self.tx.send(Command::Vote(place_id));
    }
}

impl Drop for PartyHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

// ---------------------------------------------------------------------------------------
// Worker state
// ---------------------------------------------------------------------------------------

/// The local member: who we are, what we are playing, and the cookie the presence fallback
/// borrows. The cookie never leaves this process.
struct Local {
    me: PartyUser,
    status: PartyStatus,
    place_id: u64,
    job_id: String,
    cookie: Option<String>,
}

struct Peer {
    user: PartyUser,
    ws: WebSocket<TcpStream>,
    status: PartyStatus,
    place_id: u64,
    job_id: String,
    dead: bool,
}

enum Role {
    Idle,
    Host(Box<Host>),
    Guest(Box<Guest>),
}

struct Host {
    listener: TcpListener,
    invite: PartyInvite,
    max: usize,
    peers: Vec<Peer>,
    handshake_tx: Sender<Result<(PartyUser, WebSocket<TcpStream>), String>>,
    handshake_rx: Receiver<Result<(PartyUser, WebSocket<TcpStream>), String>>,
    mapping: Option<Receiver<Option<Ipv4Addr>>>,
    mapping_deadline: Instant,
    mapping_done: bool,
    last_heartbeat: Instant,
    announce: bool,
    force_snapshot: bool,
    board: PartyBoard,
}

struct Guest {
    invite: PartyInvite,
    ws: Option<WebSocket<TcpStream>>,
    /// Who to follow. Starts as the invite's host and moves when the host promotes somebody.
    host: u64,
    max: usize,
    code: String,
    attempts: u32,
    retry_at: Option<Instant>,
    ever_connected: bool,
    presence_at: Instant,
    last_followed: Option<String>,
    last_ping: Instant,
    degraded: bool,
    /// Terminal: kicked, the host left, or the room refused us. Nothing is retried.
    stopped: bool,
    /// Mirror of the host's board, from the last `Members` frame.
    board: PartyBoard,
}

struct Out {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Out {
    fn emit(&self, event: PartyEvent) {
        let _ = self.tx.send(Msg::Party(event));
        self.ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------------------

fn worker(rx: Receiver<Command>, tx: Sender<Msg>, ctx: egui::Context) {
    let out = Out { tx, ctx };
    let mut role = Role::Idle;
    let mut local: Option<Local> = None;

    loop {
        // Idle blocks here forever: no timer, no socket, no wake-up at all.
        let command = match &role {
            Role::Idle => match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => break,
            },
            _ => match rx.recv_timeout(TICK) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
        };

        if let Some(cmd) = command {
            match cmd {
                Command::Shutdown => {
                    shutdown(&mut role, &mut local);
                    break;
                }
                Command::Create { me, max, cookie } => {
                    shutdown(&mut role, &mut local);
                    match open_host(me.clone(), max, &out) {
                        Ok(host) => {
                            let short = host.invite.short_code();
                            local = Some(Local {
                                me,
                                status: PartyStatus::Ready,
                                place_id: 0,
                                job_id: String::new(),
                                cookie,
                            });
                            role = Role::Host(Box::new(host));
                            IN_PARTY.store(true, Ordering::Relaxed);
                            if let Role::Host(host) = &role {
                                out.emit(PartyEvent::Snapshot(host_snapshot(host, local.as_ref())));
                            }
                            out.emit(PartyEvent::Notice(format!(
                                "Party open ({short}) - copy the invite from the party bar"
                            )));
                        }
                        Err(e) => {
                            IN_PARTY.store(false, Ordering::Relaxed);
                            out.emit(PartyEvent::Left(format!(
                                "Could not open a party on this machine: {e}"
                            )));
                        }
                    }
                }
                Command::Join { invite, me, cookie } => {
                    shutdown(&mut role, &mut local);
                    let guest = Guest {
                        invite,
                        ws: None,
                        host: invite.host_id,
                        max: MAX_PARTY_SIZE,
                        code: invite.short_code(),
                        attempts: 0,
                        retry_at: Some(Instant::now()),
                        ever_connected: false,
                        presence_at: Instant::now() + PRESENCE_POLL,
                        last_followed: None,
                        last_ping: Instant::now(),
                        degraded: false,
                        stopped: false,
                        board: PartyBoard::default(),
                    };
                    local = Some(Local {
                        me,
                        status: PartyStatus::Ready,
                        place_id: 0,
                        job_id: String::new(),
                        cookie,
                    });
                    out.emit(PartyEvent::Snapshot(guest_snapshot(&guest, local.as_ref())));
                    role = Role::Guest(Box::new(guest));
                    IN_PARTY.store(true, Ordering::Relaxed);
                }
                Command::Leave => {
                    shutdown(&mut role, &mut local);
                    IN_PARTY.store(false, Ordering::Relaxed);
                    out.emit(PartyEvent::Left("Left the party".to_string()));
                }
                Command::Kick(user_id) => {
                    if let Role::Host(host) = &mut role {
                        kick(host, user_id, &out);
                    }
                }
                Command::Promote(user_id) => {
                    if let Role::Host(host) = &mut role {
                        promote(host, user_id, &out);
                    }
                }
                Command::Status {
                    status,
                    place_id,
                    job_id,
                } => {
                    if let Some(local) = local.as_mut() {
                        local.status = status;
                        local.place_id = place_id;
                        local.job_id = job_id.clone();
                    }
                    match &mut role {
                        Role::Host(host) => {
                            host.announce = true;
                        }
                        Role::Guest(guest) => {
                            send_status(guest, local.as_ref(), None, None, None, None);
                        }
                        Role::Idle => {}
                    }
                }
                Command::ReadyCheck => {
                    if let Role::Host(host) = &mut role {
                        host.board.start_ready_check(wall_ms());
                        host.announce = true;
                    }
                }
                Command::SetReady(ready) => match &mut role {
                    Role::Host(host) => {
                        let host_id = host.invite.host_id;
                        if host.board.set_ready(host_id, ready) {
                            host.announce = true;
                        }
                    }
                    Role::Guest(guest) => {
                        send_status(guest, local.as_ref(), Some(ready), None, None, None);
                        let me = local.as_ref().map(|l| l.me.id).unwrap_or(0);
                        guest.board.set_ready(me, ready);
                        out.emit(PartyEvent::Snapshot(guest_snapshot(guest, local.as_ref())));
                    }
                    Role::Idle => {}
                },
                Command::Suggest(suggestion) => match &mut role {
                    Role::Host(host) => {
                        if host.board.suggest(suggestion) {
                            host.announce = true;
                        }
                    }
                    Role::Guest(guest) => {
                        send_status(guest, local.as_ref(), None, None, Some(suggestion), None);
                    }
                    Role::Idle => {}
                },
                Command::Vote(place_id) => match &mut role {
                    Role::Host(host) => {
                        let host_id = host.invite.host_id;
                        if host.board.vote(host_id, place_id) {
                            host.announce = true;
                        }
                    }
                    Role::Guest(guest) => {
                        send_status(guest, local.as_ref(), None, Some(place_id), None, None);
                        let me = local.as_ref().map(|l| l.me.id).unwrap_or(0);
                        guest.board.vote(me, place_id);
                        out.emit(PartyEvent::Snapshot(guest_snapshot(guest, local.as_ref())));
                    }
                    Role::Idle => {}
                },
                Command::RequestHop => match &mut role {
                    Role::Guest(guest) => {
                        send_status(guest, local.as_ref(), None, None, None, Some(true));
                    }
                    // The host is in the game it would hop from; asking itself does nothing.
                    Role::Host(_) | Role::Idle => {}
                },
                Command::Launch(launch) => match &mut role {
                    Role::Host(host) => {
                        if let Some(local) = local.as_mut() {
                            local.status = PartyStatus::Launching;
                            local.place_id = launch.place_id;
                            local.job_id = launch.job_id.clone();
                        }
                        let frame = WireIn::Launch {
                            place_id: launch.place_id,
                            job_id: launch.job_id,
                            universe_id: launch.universe_id,
                            game_name: launch.game_name,
                            by: host.invite.host_id,
                        };
                        broadcast(host, &frame);
                        host.force_snapshot = true;
                        host.announce = true;
                    }
                    Role::Guest(guest) => {
                        if let Some(ws) = guest.ws.as_mut() {
                            let _ = write(
                                ws,
                                &WireOut::Launch {
                                    place_id: launch.place_id,
                                    job_id: launch.job_id,
                                    universe_id: launch.universe_id,
                                    game_name: launch.game_name,
                                },
                            );
                        }
                    }
                    Role::Idle => {}
                },
            }
        }

        // Take the role out so the tick helpers can borrow the local member and the output.
        let mut current = std::mem::replace(&mut role, Role::Idle);
        match &mut current {
            Role::Idle => {}
            Role::Host(host) => host_tick(host, local.as_ref(), &out),
            Role::Guest(guest) => guest_tick(guest, local.as_ref(), &out),
        }
        role = current;
    }

    shutdown(&mut role, &mut local);
    IN_PARTY.store(false, Ordering::Relaxed);
}

fn shutdown(role: &mut Role, local: &mut Option<Local>) {
    match std::mem::replace(role, Role::Idle) {
        Role::Idle => {}
        Role::Host(host) => {
            for mut peer in host.peers {
                let _ = peer.ws.close(Some(CloseFrame {
                    code: CloseCode::from(PARTY_CLOSED_CODE),
                    reason: "The host left the party".into(),
                }));
            }
            close_port_mapping(host.invite.port);
        }
        Role::Guest(mut guest) => {
            if let Some(ws) = guest.ws.as_mut() {
                let _ = write(ws, &WireOut::Leave);
                let _ = ws.close(None);
            }
        }
    }
    *local = None;
}

// ---------------------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------------------

fn open_host(me: PartyUser, max: usize, out: &Out) -> Result<Host, String> {
    let listener = TcpListener::bind(("0.0.0.0", 0)).map_err(|e| format!("no free port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("listener: {e}"))?;

    let invite = PartyInvite {
        address: local_ipv4(),
        port,
        host_id: me.id,
        token: random_token(),
    };

    let (handshake_tx, handshake_rx) = mpsc::channel();
    let (map_tx, map_rx) = mpsc::channel();
    let local_ip = local_ipv4();
    std::thread::Builder::new()
        .name("creblox-party-map".to_string())
        .spawn(move || {
            let _ = map_tx.send(map_port(port, local_ip));
        })
        .map_err(|e| format!("no thread for port mapping: {e}"))?;

    out.emit(PartyEvent::Notice(
        "Asking your router for a port so friends can join directly...".to_string(),
    ));

    Ok(Host {
        listener,
        invite,
        max,
        peers: Vec::new(),
        handshake_tx,
        handshake_rx,
        mapping: Some(map_rx),
        mapping_deadline: Instant::now() + MAPPING_WAIT,
        mapping_done: false,
        last_heartbeat: Instant::now(),
        announce: true,
        force_snapshot: true,
        board: PartyBoard::default(),
    })
}

/// Wall-clock milliseconds, for the ready-check deadline members compare against.
fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn host_tick(host: &mut Host, local: Option<&Local>, out: &Out) {
    // A ready check that ran out is cleared by the host, so a guest's clock cannot keep a
    // stale countdown on screen.
    if host.board.expire(wall_ms()) {
        host.announce = true;
    }
    accept_peers(host);
    drain_handshakes(host, out);
    read_peers(host, out);
    resolve_mapping(host, out);

    if host.announce || host.force_snapshot {
        host.announce = false;
        host.force_snapshot = false;
        announce(host, local);
        out.emit(PartyEvent::Snapshot(host_snapshot(host, local)));
    }

    if host.last_heartbeat.elapsed() >= HEARTBEAT {
        host.last_heartbeat = Instant::now();
        let before = host.peers.len();
        host.peers
            .retain_mut(|peer| !peer.dead && write(&mut peer.ws, &WireOut::Ping));
        // A keepalive that failed just removed a peer: everyone else's roster (and their party
        // HUD, where that member turns red) has to hear about it on this tick.
        if host.peers.len() != before {
            host.announce = true;
        }
    }
}

fn accept_peers(host: &mut Host) {
    loop {
        match host.listener.accept() {
            Ok((stream, _addr)) => {
                let token = host.invite.token;
                let tx = host.handshake_tx.clone();
                let _ = std::thread::Builder::new()
                    .name("creblox-party-join".to_string())
                    .spawn(move || {
                        let _ = tx.send(accept_one(stream, token));
                    });
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
}

/// Runs on its own thread: one slow or hostile handshake can never stall the party.
fn accept_one(
    stream: TcpStream,
    token: [u8; 6],
) -> Result<(PartyUser, WebSocket<TcpStream>), String> {
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    let expected = hex(&token);
    let callback =
        move |req: &ServerRequest, resp: ServerResponse| -> Result<ServerResponse, ErrorResponse> {
            let magic_ok = req
                .headers()
                .get(MAGIC_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|v| v == MAGIC_VALUE)
                .unwrap_or(false);
            let token_ok = req
                .headers()
                .get(TOKEN_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|v| v == expected)
                .unwrap_or(false);
            if magic_ok && token_ok {
                Ok(resp)
            } else {
                Err(tungstenite::http::Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Some("CREBLOX party: bad room token".to_string()))
                    .expect("a static response"))
            }
        };
    let mut ws = tungstenite::accept_hdr(stream, callback).map_err(|e| e.to_string())?;
    // The first frame identifies the member; anything else is not a CREBLOX client. Read it
    // under the handshake timeout: a client that is slow to send its hello is not hostile,
    // and the 1 ms tick timeout used later would drop it for losing a scheduling race.
    let mut joined = match ws.read() {
        Ok(Message::Text(text)) => match serde_json::from_str::<PeerFrame>(&text) {
            Ok(PeerFrame::Join {
                user: Some(user), ..
            }) => Ok((user, ws)),
            _ => Err("unexpected first frame".to_string()),
        },
        _ => Err("no hello".to_string()),
    }?;
    // Only now drop to the 1 ms timeout the steady state uses.
    set_read_timeout(&mut joined.1);
    Ok(joined)
}

fn drain_handshakes(host: &mut Host, out: &Out) {
    loop {
        match host.handshake_rx.try_recv() {
            Ok(Ok((user, ws))) => {
                if host.max.saturating_sub(host.peers.len() + 1) == 0 {
                    let mut ws = ws;
                    let _ = write(
                        &mut ws,
                        &WireIn::Error {
                            message: "Party is full".to_string(),
                        },
                    );
                    let _ = ws.close(None);
                    continue;
                }
                // A member who reconnects replaces their stale socket instead of doubling up.
                host.peers.retain_mut(|peer| {
                    if peer.user.id == user.id {
                        let _ = peer.ws.close(None);
                        false
                    } else {
                        true
                    }
                });
                host.peers.push(Peer {
                    user,
                    ws,
                    status: PartyStatus::Ready,
                    place_id: 0,
                    job_id: String::new(),
                    dead: false,
                });
                host.announce = true;
                out.emit(PartyEvent::Notice("Someone joined the party".to_string()));
            }
            Ok(Err(_)) => {}
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
}

fn read_peers(host: &mut Host, out: &Out) {
    let mut inbox: Vec<(usize, PeerFrame)> = Vec::new();
    let mut announce = false;
    for (index, peer) in host.peers.iter_mut().enumerate() {
        loop {
            match peer.ws.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(frame) = serde_json::from_str::<PeerFrame>(&text) {
                        inbox.push((index, frame));
                    }
                }
                Ok(Message::Close(_)) => {
                    peer.dead = true;
                    announce = true;
                    break;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if is_idle(&e) => break,
                Err(_) => {
                    // A crash: the socket ended without a `Leave` frame. The roster below must
                    // be rebroadcast now, or every other guest keeps drawing the member who
                    // is already gone (and their HUD never turns the tile red).
                    peer.dead = true;
                    announce = true;
                    break;
                }
            }
        }
    }

    let host_id = host.invite.host_id;
    let mut launches: Vec<PartyLaunch> = Vec::new();
    let mut hops: Vec<u64> = Vec::new();
    let mut kicks: Vec<u64> = Vec::new();
    let mut promotes: Vec<u64> = Vec::new();
    let mut refuse: Vec<usize> = Vec::new();
    let mut pong: Vec<usize> = Vec::new();

    for (index, frame) in inbox {
        let Some(peer) = host.peers.get_mut(index) else {
            continue;
        };
        if peer.dead {
            continue;
        }
        match frame {
            PeerFrame::Status {
                status,
                place_id,
                job_id,
                ready,
                vote,
                suggest,
                hop,
            } => {
                peer.status = PartyStatus::from_wire(&status);
                peer.place_id = place_id;
                peer.job_id = job_id;
                if let Some(ready) = ready {
                    host.board.set_ready(peer.user.id, ready);
                }
                if let Some(vote) = vote {
                    host.board.vote(peer.user.id, vote);
                }
                if let Some(suggestion) = suggest {
                    host.board.suggest(suggestion);
                }
                if hop == Some(true) {
                    hops.push(peer.user.id);
                }
                host.announce = true;
            }
            PeerFrame::Launch {
                place_id,
                job_id,
                universe_id,
                game_name,
            } => {
                if peer.user.id != host_id {
                    refuse.push(index);
                } else {
                    launches.push(PartyLaunch {
                        place_id,
                        job_id,
                        universe_id,
                        game_name,
                        by: host_id,
                    });
                }
            }
            PeerFrame::Kick { user_id } => {
                if peer.user.id == host_id {
                    kicks.push(user_id);
                } else {
                    refuse.push(index);
                }
            }
            PeerFrame::Promote { user_id } => {
                if peer.user.id == host_id {
                    promotes.push(user_id);
                } else {
                    refuse.push(index);
                }
            }
            PeerFrame::Ping => pong.push(index),
            PeerFrame::Leave => {
                peer.dead = true;
                host.announce = true;
            }
            PeerFrame::Join { .. } | PeerFrame::Other => {}
        }
    }

    for index in pong {
        if let Some(peer) = host.peers.get_mut(index) {
            let _ = write(&mut peer.ws, &WireIn::Pong);
        }
    }
    for user in hops {
        out.emit(PartyEvent::HopRequest(user));
    }
    for index in refuse {
        if let Some(peer) = host.peers.get_mut(index) {
            let _ = write(
                &mut peer.ws,
                &WireIn::Error {
                    message: "Only the host can do that".to_string(),
                },
            );
        }
    }
    for launch in launches {
        // This process may be hosting for a host who was promoted after the party opened:
        // the launch then belongs to somebody else, so relay it *and* follow it here.
        broadcast(
            host,
            &WireIn::Launch {
                place_id: launch.place_id,
                job_id: launch.job_id.clone(),
                universe_id: launch.universe_id,
                game_name: launch.game_name.clone(),
                by: launch.by,
            },
        );
        out.emit(PartyEvent::Launch(launch));
    }
    for user_id in kicks {
        kick(host, user_id, out);
    }
    for user_id in promotes {
        promote(host, user_id, out);
    }

    host.peers.retain(|peer| !peer.dead);
    if announce {
        host.announce = true;
    }
}

fn kick(host: &mut Host, user_id: u64, out: &Out) {
    if user_id == host.invite.host_id {
        out.emit(PartyEvent::Notice("The host cannot be removed".to_string()));
        return;
    }
    if let Some(peer) = host.peers.iter_mut().find(|p| p.user.id == user_id) {
        let _ = peer.ws.close(Some(CloseFrame {
            code: CloseCode::from(4001u16),
            reason: "Removed from the party".into(),
        }));
    }
    host.peers.retain(|p| p.user.id != user_id);
    host.announce = true;
}

fn promote(host: &mut Host, user_id: u64, out: &Out) {
    let known = host.peers.iter().any(|p| p.user.id == user_id) || user_id == host.invite.host_id;
    if !known {
        out.emit(PartyEvent::Notice("That member already left".to_string()));
        return;
    }
    host.invite.host_id = user_id;
    host.announce = true;
    host.force_snapshot = true;
    out.emit(PartyEvent::Notice("The party has a new host".to_string()));
}

fn announce(host: &mut Host, local: Option<&Local>) {
    let members: Vec<WireMember> = local
        .map(|l| WireMember {
            user: l.me.clone(),
            status: l.status.wire().to_string(),
            place_id: l.place_id,
            job_id: l.job_id.clone(),
        })
        .into_iter()
        .chain(host.peers.iter().map(|p| WireMember {
            user: p.user.clone(),
            status: p.status.wire().to_string(),
            place_id: p.place_id,
            job_id: p.job_id.clone(),
        }))
        .collect();
    broadcast(
        host,
        &WireIn::Members {
            host: host.invite.host_id,
            max: host.max,
            members,
            board: host.board.clone(),
        },
    );
}

fn broadcast(host: &mut Host, frame: &WireIn) {
    let Ok(text) = serde_json::to_string(frame) else {
        return;
    };
    host.peers
        .retain_mut(|peer| !peer.dead && peer.ws.send(Message::Text(text.clone().into())).is_ok());
}

fn resolve_mapping(host: &mut Host, out: &Out) {
    if host.mapping_done {
        return;
    }
    let resolved = match host.mapping.as_ref().map(|rx| rx.try_recv()) {
        Some(Ok(Some(external))) => Some(Some(external)),
        Some(Ok(None)) => Some(None),
        Some(Err(TryRecvError::Empty)) => {
            if Instant::now() >= host.mapping_deadline {
                Some(None)
            } else {
                None
            }
        }
        _ => Some(None),
    };
    let Some(resolved) = resolved else {
        return;
    };
    host.mapping_done = true;
    host.mapping = None;
    host.force_snapshot = true;
    match resolved {
        Some(external) => {
            host.invite.address = external;
            out.emit(PartyEvent::Notice(
                "Party is reachable over the internet - share the invite".to_string(),
            ));
        }
        None => out.emit(PartyEvent::Notice(
            "No port forwarding on this network - friends will follow your game instead"
                .to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------------------
// Guest
// ---------------------------------------------------------------------------------------

fn guest_tick(guest: &mut Guest, local: Option<&Local>, out: &Out) {
    if guest.stopped {
        // The party ended for this member: no dial, no presence poll, no heartbeat.
        return;
    }
    if guest.ws.is_none() {
        let due = guest
            .retry_at
            .map(|at| Instant::now() >= at)
            .unwrap_or(true);
        if due {
            let me = local.map(|l| l.me.clone()).unwrap_or_default();
            match dial(&guest.invite) {
                Ok(mut ws) => {
                    guest.attempts = 0;
                    guest.ever_connected = true;
                    guest.retry_at = None;
                    guest.last_ping = Instant::now();
                    guest.degraded = false;
                    let hello = WireOut::Join {
                        code: guest.code.clone(),
                        user: me,
                        max: guest.max,
                    };
                    let _ = write(&mut ws, &hello);
                    guest.ws = Some(ws);
                }
                Err(e) => {
                    guest.attempts = guest.attempts.saturating_add(1);
                    guest.retry_at = Some(Instant::now() + backoff(guest.attempts));
                    if !guest.degraded {
                        guest.degraded = true;
                        out.emit(PartyEvent::Notice(format!(
                            "Direct connection to the host did not open ({e}); following their \
                             Roblox presence instead"
                        )));
                        out.emit(PartyEvent::Snapshot(guest_snapshot(guest, local)));
                    }
                }
            }
        }
    }

    read_guest(guest, local, out);
    poll_presence(guest, local, out);

    if guest.last_ping.elapsed() >= HEARTBEAT {
        guest.last_ping = Instant::now();
        let lost = match guest.ws.as_mut() {
            Some(ws) => !write(ws, &WireOut::Ping),
            None => false,
        };
        if lost {
            guest.ws = None;
            guest.attempts = guest.attempts.saturating_add(1);
            guest.retry_at = Some(Instant::now() + backoff(guest.attempts));
        }
    }
}

fn read_guest(guest: &mut Guest, local: Option<&Local>, out: &Out) {
    let mut frames: Vec<WireIn> = Vec::new();
    let mut closed = false;
    {
        let Some(ws) = guest.ws.as_mut() else {
            return;
        };
        loop {
            match ws.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(frame) = serde_json::from_str::<WireIn>(&text) {
                        frames.push(frame);
                    }
                }
                Ok(Message::Close(frame)) => {
                    // 4001 is the host saying "you were removed" and 4000 that the room is
                    // closing. Both are terminal: retrying would either walk straight back
                    // into the room we were removed from or hammer a dead port forever.
                    closed = true;
                    let code = frame
                        .as_ref()
                        .map(|f| u16::from(f.code))
                        .unwrap_or_default();
                    if code == 4001 || code == PARTY_CLOSED_CODE {
                        guest.ws = None;
                        guest.retry_at = None;
                        guest.stopped = true;
                        IN_PARTY.store(false, Ordering::Relaxed);
                        let reason = if code == PARTY_CLOSED_CODE {
                            "The host ended the party"
                        } else {
                            "Removed from the party"
                        };
                        out.emit(PartyEvent::Left(reason.to_string()));
                        return;
                    }
                    break;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if is_idle(&e) => break,
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }
    }

    for frame in frames {
        match frame {
            WireIn::Welcome { code, host, max } => {
                guest.code = code;
                guest.host = host;
                guest.max = max;
                guest.ever_connected = true;
                IN_PARTY.store(true, Ordering::Relaxed);
                out.emit(PartyEvent::Snapshot(guest_snapshot(guest, local)));
                // Announce the state we already have, so a reconnect does not leave the
                // other members looking at a stale dot.
                if let (Some(local), Some(ws)) = (local, guest.ws.as_mut()) {
                    let _ = write(
                        ws,
                                                    &WireOut::Status {
                                status: local.status.wire().to_string(),
                                place_id: local.place_id,
                                job_id: local.job_id.clone(),
                                ready: None,
                                vote: None,
                                suggest: None,
                                hop: None,
                            },
                    );
                }
            }
            WireIn::Members {
                host,
                max,
                members,
                board,
            } => {
                guest.host = host;
                guest.max = max;
                guest.board = board.clone();
                out.emit(PartyEvent::Snapshot(PartySnapshot {
                    code: guest.code.clone(),
                    invite: guest.invite.code(),
                    host,
                    max,
                    members: members.into_iter().map(WireMember::into_member).collect(),
                    connected: true,
                    board,
                }));
            }
            WireIn::Launch {
                place_id,
                job_id,
                universe_id,
                game_name,
                by,
            } => {
                if !guest.accept_launch(local, place_id, &job_id) {
                    continue;
                }
                out.emit(PartyEvent::Launch(PartyLaunch {
                    place_id,
                    job_id,
                    universe_id,
                    game_name,
                    by,
                }));
            }
            WireIn::Error { message } => {
                // "full" / "no such party" are terminal: retrying would loop.
                let fatal = message.contains("full")
                    || message.contains("No such")
                    || message.contains("required");
                if fatal {
                    guest.ws = None;
                    guest.retry_at = None;
                    guest.stopped = true;
                    IN_PARTY.store(false, Ordering::Relaxed);
                    out.emit(PartyEvent::Left(message));
                } else {
                    out.emit(PartyEvent::Notice(message));
                }
            }
            WireIn::Pong => {}
        }
    }

    if closed && guest.ws.is_some() {
        guest.ws = None;
        guest.attempts = guest.attempts.saturating_add(1);
        guest.retry_at = Some(Instant::now() + backoff(guest.attempts));
        guest.degraded = false;
        out.emit(PartyEvent::Snapshot(guest_snapshot(guest, local)));
    }
}

/// The fallback that makes a party work through CGNAT: watch one public user id and join
/// whatever server the host enters. One small request every four seconds, and nothing at all
/// outside a party.
fn poll_presence(guest: &mut Guest, local: Option<&Local>, out: &Out) {
    if Instant::now() < guest.presence_at {
        return;
    }
    guest.presence_at = Instant::now() + PRESENCE_POLL;
    let Some(cookie) = local.and_then(|l| l.cookie.as_deref()) else {
        return;
    };
    let Some(spot) = crate::api::party_presence(guest.host, cookie) else {
        return;
    };
    if !spot.in_game || spot.place_id == 0 || spot.job_id.is_empty() {
        return;
    }
    if !guest.accept_launch(local, spot.place_id, &spot.job_id) {
        return;
    }
    out.emit(PartyEvent::Launch(PartyLaunch {
        place_id: spot.place_id,
        job_id: spot.job_id,
        universe_id: spot.universe_id,
        game_name: spot.game_name,
        by: guest.host,
    }));
}

fn dial(invite: &PartyInvite) -> Result<WebSocket<TcpStream>, String> {
    let addr = invite.endpoint();
    let stream =
        TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| format!("{addr}: {e}"))?;
    let _ = stream.set_nodelay(true);
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    // `into_client_request` fills in Host, Upgrade, Connection, Sec-WebSocket-Version and a
    // fresh Sec-WebSocket-Key; a hand-built request without them is refused by the host.
    let uri = format!("ws://{addr}/party");
    let mut request = uri
        .as_str()
        .into_client_request()
        .map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        HeaderName::from_static(MAGIC_HEADER),
        HeaderValue::from_static(MAGIC_VALUE),
    );
    request.headers_mut().insert(
        HeaderName::from_static(TOKEN_HEADER),
        HeaderValue::from_str(&hex(&invite.token)).map_err(|e| e.to_string())?,
    );
    let (mut ws, _response) = tungstenite::client(request, stream)
        .map_err(|e| format!("the host refused this invite ({e})"))?;
    set_read_timeout(&mut ws);
    Ok(ws)
}

fn backoff(attempts: u32) -> Duration {
    let secs = 1u64 << attempts.min(5);
    Duration::from_secs(secs.min(BACKOFF_MAX.as_secs()))
}

// ---------------------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------------------

fn host_snapshot(host: &Host, local: Option<&Local>) -> PartySnapshot {
    let mut members: Vec<PartyMember> = local
        .map(|l| PartyMember {
            user: l.me.clone(),
            status: l.status,
            place_id: l.place_id,
            job_id: l.job_id.clone(),
        })
        .into_iter()
        .collect();
    members.extend(host.peers.iter().map(|p| PartyMember {
        user: p.user.clone(),
        status: p.status,
        place_id: p.place_id,
        job_id: p.job_id.clone(),
    }));
    PartySnapshot {
        code: host.invite.short_code(),
        invite: if host.mapping_done {
            host.invite.code()
        } else {
            String::new()
        },
        host: host.invite.host_id,
        max: host.max,
        members,
        connected: true,
        board: host.board.clone(),
    }
}

fn guest_snapshot(guest: &Guest, local: Option<&Local>) -> PartySnapshot {
    let mut members: Vec<PartyMember> = local
        .map(|l| PartyMember {
            user: l.me.clone(),
            status: l.status,
            place_id: l.place_id,
            job_id: l.job_id.clone(),
        })
        .into_iter()
        .collect();
    if guest.ws.is_some() {
        // Connected: the real roster arrives in its own frame, so do not show a roster of
        // one next to a live socket.
        members.clear();
    }
    PartySnapshot {
        code: guest.code.clone(),
        invite: guest.invite.code(),
        host: guest.host,
        max: guest.max,
        members,
        connected: guest.ws.is_some(),
        board: guest.board.clone(),
    }
}

// ---------------------------------------------------------------------------------------
// Sockets, ports and network helpers
// ---------------------------------------------------------------------------------------

fn set_read_timeout(ws: &mut WebSocket<TcpStream>) {
    let _ = ws.get_ref().set_read_timeout(Some(READ_TIMEOUT));
}

fn is_idle(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Guest-side status frame, carrying at most one party action. Everything else stays
/// `None`, so an unset action serializes to exactly the frame 0.5.5 hosts already parse.
fn send_status(
    guest: &mut Guest,
    local: Option<&Local>,
    ready: Option<bool>,
    vote: Option<u64>,
    suggest: Option<PartySuggestion>,
    hop: Option<bool>,
) {
    let (Some(local), Some(ws)) = (local, guest.ws.as_mut()) else {
        return;
    };
    let _ = write(
        ws,
        &WireOut::Status {
            status: local.status.wire().to_string(),
            place_id: local.place_id,
            job_id: local.job_id.clone(),
            ready,
            vote,
            suggest,
            hop,
        },
    );
}

fn write(ws: &mut WebSocket<TcpStream>, frame: &impl Serialize) -> bool {
    let Ok(text) = serde_json::to_string(frame) else {
        return false;
    };
    ws.send(Message::Text(text.into())).is_ok()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Six bytes of room secret from the OS. `getrandom` is already a macOS dependency; other
/// builds mix the clock and a stack address so the token still differs per room.
fn random_token() -> [u8; 6] {
    #[allow(unused_mut)]
    let mut token = [0u8; 6];
    #[cfg(target_os = "macos")]
    {
        if getrandom::getrandom(&mut token).is_ok() {
            return token;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stack = &token as *const _ as u128;
    let mix = nanos.rotate_left(17) ^ stack;
    for (index, slot) in token.iter_mut().enumerate() {
        *slot = (mix >> (index * 8)) as u8;
    }
    token
}

/// The address friends on the same network can reach. A UDP `connect` only asks the routing
/// table which interface would be used - it sends no packet.
fn local_ipv4() -> Ipv4Addr {
    if let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)) {
        if socket.connect(("1.1.1.1", 53)).is_ok() {
            if let Ok(SocketAddr::V4(addr)) = socket.local_addr() {
                return *addr.ip();
            }
        }
    }
    Ipv4Addr::LOCALHOST
}

/// Ask the router to forward `port` to this machine. Returns the public address to put in
/// the invite, or `None` when the network will not allow it (CGNAT, UPnP off, no router).
fn map_port(port: u16, local: Ipv4Addr) -> Option<Ipv4Addr> {
    let gateway = igd_next::search_gateway(igd_next::SearchOptions::default()).ok()?;
    let external = gateway.get_external_ip().ok()?;
    // `add_port` keeps the external and local port identical, which is what the invite
    // carries; `add_any_port` would hand back a different one.
    gateway
        .add_port(
            igd_next::PortMappingProtocol::TCP,
            port,
            SocketAddr::from((local, port)),
            0,
            "CREBLOX Party",
        )
        .ok()?;
    match external {
        IpAddr::V4(address) => Some(address),
        IpAddr::V6(_) => None,
    }
}

/// Best-effort tidy-up so the mapping does not outlive the party.
fn close_port_mapping(port: u16) {
    let _ = std::thread::Builder::new()
        .name("creblox-party-unmap".to_string())
        .spawn(move || {
            if let Ok(gateway) = igd_next::search_gateway(igd_next::SearchOptions::default()) {
                let _ = gateway.remove_port(igd_next::PortMappingProtocol::TCP, port);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite() -> PartyInvite {
        PartyInvite {
            address: Ipv4Addr::new(203, 0, 113, 9),
            port: 51_234,
            host_id: 8_123_456_789,
            token: [1, 2, 3, 4, 5, 6],
        }
    }

    #[test]
    fn an_invite_code_round_trips() {
        let original = invite();
        let code = original.code();
        assert!(code.starts_with("CRB-"));
        assert_eq!(code.len(), 4 + INVITE_DIGITS);
        assert_eq!(parse_invite(&code), Some(original));
    }

    #[test]
    fn invites_accept_links_and_surrounding_space() {
        let original = invite();
        let code = original.code();
        assert_eq!(parse_invite(&format!("  {code}  ")), Some(original));
        assert_eq!(
            parse_invite(&format!("creblox://party/{code}")),
            Some(original)
        );
        assert_eq!(
            parse_invite(&format!("https://example.test/p/{code}")),
            Some(original)
        );
        assert_eq!(parse_invite(""), None);
        assert_eq!(parse_invite("CRB-8X2K"), None);
    }

    #[test]
    fn a_corrupted_invite_is_rejected() {
        let code = invite().code();
        let bytes = code.as_bytes();
        for index in 4..bytes.len() {
            let mut broken = bytes.to_vec();
            broken[index] = if broken[index] == b'0' { b'1' } else { b'0' };
            let broken = String::from_utf8(broken).unwrap();
            assert!(
                parse_invite(&broken).is_none(),
                "a typo at {index} slipped through: {broken}"
            );
        }
    }

    #[test]
    fn every_field_of_the_invite_survives_encoding() {
        for (address, port, host_id) in [
            (Ipv4Addr::LOCALHOST, 1, 1),
            (Ipv4Addr::new(255, 255, 255, 255), 65_535, u64::MAX),
            (Ipv4Addr::new(10, 0, 0, 7), 8787, 4_294_967_297),
        ] {
            let original = PartyInvite {
                address,
                port,
                host_id,
                token: [9, 8, 7, 6, 5, 4],
            };
            assert_eq!(parse_invite(&original.code()), Some(original));
        }
    }

    #[test]
    fn the_short_code_follows_the_room_token() {
        let mut first = invite();
        let mut second = first;
        second.token = [7, 7, 7, 7, 7, 7];
        assert_ne!(first.short_code(), second.short_code());
        assert!(first.short_code().starts_with("CRB-"));
        assert_eq!(first.short_code().len(), 10);
        first.token = second.token;
        assert_eq!(first.short_code(), second.short_code());
    }

    #[test]
    fn base62_is_fixed_width_and_never_overflows() {
        assert_eq!(base62_encode(&[0], 1), "0");
        assert_eq!(base62_encode(&[61], 1), "z");
        assert_eq!(base62_encode(&[62], 2), "10");
        assert_eq!(base62_encode(&[0], 6), "000000");
        let payload = invite().payload();
        let text = base62_encode(&payload, INVITE_DIGITS);
        assert_eq!(text.len(), INVITE_DIGITS);
        assert_eq!(base62_decode(&text, 22).as_deref(), Some(&payload[..]));
        // Wider than the payload: not one of our codes.
        assert_eq!(base62_decode(&"z".repeat(INVITE_DIGITS), 22), None);
        assert_eq!(base62_decode("not base62!", 22), None);
    }

    #[test]
    fn a_status_frame_without_actions_is_unchanged_on_the_wire() {
        let frame = WireOut::Status {
            status: "ready".to_string(),
            place_id: 7,
            job_id: "job".to_string(),
            ready: None,
            vote: None,
            suggest: None,
            hop: None,
        };
        assert_eq!(
            serde_json::to_string(&frame).unwrap(),
            r#"{"t":"status","status":"ready","place_id":7,"job_id":"job"}"#
        );
        let acted = WireOut::Status {
            status: "ready".to_string(),
            place_id: 7,
            job_id: "job".to_string(),
            ready: Some(true),
            vote: None,
            suggest: None,
            hop: None,
        };
        assert_eq!(
            serde_json::to_string(&acted).unwrap(),
            r#"{"t":"status","status":"ready","place_id":7,"job_id":"job","ready":true}"#
        );
    }

    #[test]
    fn host_target_needs_a_place_and_a_job() {
        let mut snap = PartySnapshot {
            code: "CRB-AB12CD".to_string(),
            invite: "CRB-AB12CD".to_string(),
            host: 7,
            max: 8,
            members: Vec::new(),
            connected: true,
            board: PartyBoard::default(),
        };
        assert!(snap.host_target().is_none());
        snap.members.push(PartyMember {
            user: PartyUser {
                id: 7,
                name: "host".to_string(),
                display_name: String::new(),
                headshot_url: String::new(),
            },
            status: PartyStatus::InGame,
            place_id: 1818,
            job_id: "job".to_string(),
        });
        assert_eq!(snap.host_target(), Some((1818, "job".to_string())));
        assert!(snap.is_host(7));
        assert!(!snap.is_host(8));
    }

    #[test]
    fn status_wire_round_trips() {
        for status in [
            PartyStatus::Ready,
            PartyStatus::Launching,
            PartyStatus::InGame,
        ] {
            assert_eq!(PartyStatus::from_wire(status.wire()), status);
        }
    }

    #[test]
    fn frames_use_the_party_wire_format() {
        let json = serde_json::to_string(&WireOut::Launch {
            place_id: 1818,
            job_id: "abc".to_string(),
            universe_id: 9,
            game_name: "Test".to_string(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"t":"launch","place_id":1818,"job_id":"abc","universe_id":9,"game_name":"Test"}"#
        );
        let parsed: WireIn =
            serde_json::from_str(r#"{"t":"welcome","code":"CRB-AB12CD","host":7,"max":8}"#)
                .unwrap();
        assert!(matches!(parsed, WireIn::Welcome { host: 7, .. }));
        let hello: PeerFrame = serde_json::from_str(
            r#"{"t":"join","code":"CRB-AB12CD","user":{"id":7,"name":"a"},"max":8}"#,
        )
        .unwrap();
        assert!(matches!(hello, PeerFrame::Join { user: Some(user), .. } if user.id == 7));
    }

    #[test]
    fn the_backoff_never_grows_past_the_cap() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(2), Duration::from_secs(4));
        assert_eq!(backoff(9), BACKOFF_MAX);
        assert_eq!(backoff(99), BACKOFF_MAX);
    }
}
