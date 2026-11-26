//! Party: ephemeral rooms that let CREBLOX users launch into the same Roblox server.
//!
//! Design constraints, in order of importance:
//!
//! * **Nothing runs when you are not in a party.** One dedicated worker thread blocks on its
//!   command channel; it only opens a socket after a Create/Join and closes it on Leave.
//! * **One socket, one thread.** No async runtime, no tokio, no polling loop of the app: the
//!   socket thread blocks in `read()` with a short timeout so commands still go out promptly.
//! * **No Roblox credentials on the wire.** Members are identified by public user id, name,
//!   display name and headshot URL. Launch tickets and `.ROBLOSECURITY` never leave the
//!   machine: each member's client fetches its own ticket and joins the same job id.
//! * **Ephemeral.** The relay keeps a room only while someone is connected, and the client
//!   drops its state on Leave. Rooms carry no history.

use std::net::TcpStream;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::types::Msg;

/// Point this at your own deployment of `relay/` (see `relay/README.md`). It is only used
/// when `settings.json` has no `party_relay_url`, so an existing install keeps working.
pub const DEFAULT_RELAY_URL: &str = "wss://creblox-party.workers.dev/party";

/// The host's room size. The relay enforces the same ceiling.
pub const MAX_PARTY_SIZE: usize = 8;

/// Heartbeat interval while connected. The relay answers `pong`; the read timeout notices a
/// dead socket within a few seconds of the first missed beat.
const HEARTBEAT: Duration = Duration::from_secs(20);
/// How long `read()` blocks before the thread checks its command channel. Only while a party
/// is open, so an idle CREBLOX never wakes for it.
const READ_POLL: Duration = Duration::from_millis(400);
const BACKOFF_MAX: Duration = Duration::from_secs(16);

/// Whether this client is in a party right now. Read by the UI without touching a lock.
static IN_PARTY: AtomicBool = AtomicBool::new(false);

pub fn in_party() -> bool {
    IN_PARTY.load(Ordering::Relaxed)
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
    pub code: String,
    pub host: u64,
    pub max: usize,
    pub members: Vec<PartyMember>,
    /// The socket is up right now (false while reconnecting).
    pub connected: bool,
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

#[derive(Clone, Debug)]
pub enum PartyEvent {
    Snapshot(PartySnapshot),
    Launch(PartyLaunch),
    /// A one-line message for the status bar (not an error state).
    Notice(String),
    /// The party ended: left, kicked, or unrecoverable.
    Left(String),
}

enum Command {
    Create {
        me: PartyUser,
        max: usize,
        url: String,
    },
    Join {
        code: String,
        me: PartyUser,
        url: String,
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

    pub fn create(&self, me: PartyUser, max: usize, url: String) {
        let _ = self.tx.send(Command::Create { me, max, url });
    }

    pub fn join(&self, code: String, me: PartyUser, url: String) {
        let _ = self.tx.send(Command::Join { code, me, url });
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
}

impl Drop for PartyHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

/// Accepts `CRB-8X2K`, `8x2k`, `creblox://party/CRB-8X2K` or a pasted invite URL.
pub fn parse_invite(raw: &str) -> Option<String> {
    let tail = raw.trim().rsplit('/').next()?.trim();
    if tail.is_empty() {
        return None;
    }
    let upper = tail.to_ascii_uppercase();
    let body = upper.strip_prefix("CRB-").unwrap_or(&upper);
    let code: String = body.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !(4..=8).contains(&code.len()) {
        return None;
    }
    Some(format!("CRB-{code}"))
}

/// Resolve the relay endpoint: the setting when set, otherwise the built-in default.
pub fn relay_url(configured: &str) -> String {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        DEFAULT_RELAY_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum WireOut {
    Create {
        user: PartyUser,
        max: usize,
    },
    Join {
        code: String,
        user: PartyUser,
        max: usize,
    },
    Leave,
    Kick {
        user_id: u64,
    },
    Promote {
        user_id: u64,
    },
    Status {
        status: String,
        place_id: u64,
        job_id: String,
    },
    Launch {
        place_id: u64,
        job_id: String,
        universe_id: u64,
        game_name: String,
    },
    Ping,
}

#[derive(Deserialize)]
#[serde(tag = "t")]
enum WireIn {
    #[serde(rename = "welcome")]
    Welcome { code: String, host: u64, max: usize },
    #[serde(rename = "members")]
    Members {
        host: u64,
        max: usize,
        members: Vec<WireMember>,
    },
    #[serde(rename = "launch")]
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
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "pong")]
    Pong,
}

#[derive(Deserialize)]
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

/// What the worker wants to be connected to. Kept while reconnecting, cleared on Leave.
struct Session {
    url: String,
    /// `None` until the relay has answered `welcome` for a newly created room.
    code: Option<String>,
    me: PartyUser,
    max: usize,
    status: PartyStatus,
    place_id: u64,
    job_id: String,
}

impl Session {
    fn endpoint(&self) -> String {
        match &self.code {
            None => join_url(&self.url, "create=1"),
            Some(code) => join_url(&self.url, &format!("code={code}")),
        }
    }
}

fn join_url(base: &str, query: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{}{}{}", base.trim_end_matches('?'), sep, query)
}

/// Single blocking thread that owns the socket. It sleeps on the command channel while not
/// in a party, so an idle launcher spends nothing on this feature.
fn worker(rx: Receiver<Command>, tx: Sender<Msg>, ctx: egui::Context) {
    let mut session: Option<Session> = None;
    let mut socket: Option<WebSocket<MaybeTlsStream<TcpStream>>> = None;
    let mut retry_at: Option<Instant> = None;
    let mut attempts: u32 = 0;
    let mut last_ping = Instant::now();
    let mut known_code: Option<String> = None;
    let mut connected = false;

    let emit = |event: PartyEvent| {
        let _ = tx.send(Msg::Party(event));
        ctx.request_repaint();
    };

    loop {
        // --- 1. Take a command. Without a session this blocks forever (no polling at all);
        //        with one it is a non-blocking drain before reading the socket.
        let command = if socket.is_some() {
            match rx.try_recv() {
                Ok(cmd) => Some(cmd),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        } else if let Some(at) = retry_at {
            let wait = at.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait.max(Duration::from_millis(1))) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => break,
            }
        };

        if let Some(cmd) = command {
            match cmd {
                Command::Shutdown => {
                    if let Some(ws) = socket.as_mut() {
                        let _ = ws.close(None);
                    }
                    let _ = send(&mut socket, WireOut::Leave);
                    break;
                }
                Command::Create { me, max, url } => {
                    if let Some(ws) = socket.as_mut() {
                        let _ = ws.close(None);
                    }
                    socket = None;
                    known_code = None;
                    connected = false;
                    attempts = 0;
                    retry_at = Some(Instant::now());
                    session = Some(Session {
                        url,
                        code: None,
                        me,
                        max,
                        status: PartyStatus::Ready,
                        place_id: 0,
                        job_id: String::new(),
                    });
                }
                Command::Join { code, me, url } => {
                    if let Some(ws) = socket.as_mut() {
                        let _ = ws.close(None);
                    }
                    socket = None;
                    known_code = Some(code.clone());
                    connected = false;
                    attempts = 0;
                    retry_at = Some(Instant::now());
                    session = Some(Session {
                        url,
                        code: Some(code),
                        me,
                        max: MAX_PARTY_SIZE,
                        status: PartyStatus::Ready,
                        place_id: 0,
                        job_id: String::new(),
                    });
                }
                Command::Leave => {
                    let _ = send(&mut socket, WireOut::Leave);
                    if let Some(ws) = socket.as_mut() {
                        let _ = ws.close(None);
                    }
                    socket = None;
                    session = None;
                    known_code = None;
                    connected = false;
                    retry_at = None;
                    IN_PARTY.store(false, Ordering::Relaxed);
                    emit(PartyEvent::Left("Left the party".to_string()));
                }
                Command::Kick(user_id) => {
                    let _ = send(&mut socket, WireOut::Kick { user_id });
                }
                Command::Promote(user_id) => {
                    let _ = send(&mut socket, WireOut::Promote { user_id });
                }
                Command::Status {
                    status,
                    place_id,
                    job_id,
                } => {
                    if let Some(s) = session.as_mut() {
                        s.status = status;
                        s.place_id = place_id;
                        s.job_id = job_id.clone();
                    }
                    let _ = send(
                        &mut socket,
                        WireOut::Status {
                            status: status.wire().to_string(),
                            place_id,
                            job_id,
                        },
                    );
                }
                Command::Launch(launch) => {
                    let _ = send(
                        &mut socket,
                        WireOut::Launch {
                            place_id: launch.place_id,
                            job_id: launch.job_id,
                            universe_id: launch.universe_id,
                            game_name: launch.game_name,
                        },
                    );
                }
            }
        }

        // --- 2. Connect when a session wants a socket and the backoff has elapsed.
        if socket.is_none() {
            let Some(want) = session.as_ref() else {
                continue;
            };
            if let Some(at) = retry_at {
                if Instant::now() < at {
                    continue;
                }
            }
            match connect(&want.endpoint()) {
                Ok(mut ws) => {
                    attempts = 0;
                    retry_at = None;
                    last_ping = Instant::now();
                    set_read_timeout(&mut ws, Some(READ_POLL));
                    // First frame identifies us: a room we allocated (`create`) or one we
                    // are rejoining with the code we already know (`join`).
                    let hello = match &known_code {
                        None => WireOut::Create {
                            user: want.me.clone(),
                            max: want.max,
                        },
                        Some(code) => WireOut::Join {
                            code: code.clone(),
                            user: want.me.clone(),
                            max: want.max,
                        },
                    };
                    let _ = write(&mut ws, &hello);
                    socket = Some(ws);
                }
                Err(e) => {
                    attempts = attempts.saturating_add(1);
                    let backoff = backoff(attempts);
                    retry_at = Some(Instant::now() + backoff);
                    if attempts == 1 {
                        emit(PartyEvent::Notice(format!(
                            "Party relay unreachable ({e}); retrying"
                        )));
                    }
                }
            }
            continue;
        }

        // --- 3. Read. A short timeout keeps commands responsive; a dead socket is noticed
        //        by the heartbeat, not by a busy loop.
        let Some(ws) = socket.as_mut() else {
            continue;
        };
        if last_ping.elapsed() >= HEARTBEAT {
            last_ping = Instant::now();
            let _ = write(ws, &WireOut::Ping);
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(frame) = serde_json::from_str::<WireIn>(&text) {
                    match frame {
                        WireIn::Welcome { code, host, max } => {
                            known_code = Some(code.clone());
                            connected = true;
                            IN_PARTY.store(true, Ordering::Relaxed);
                            if let Some(s) = session.as_mut() {
                                s.code = Some(code.clone());
                                s.max = max;
                            }
                            emit(PartyEvent::Snapshot(PartySnapshot {
                                code,
                                host,
                                max,
                                members: Vec::new(),
                                connected: true,
                            }));
                            // Announce the state we already have, so a reconnect does not
                            // leave the others looking at a stale "Launching" dot.
                            if let Some(s) = session.as_ref() {
                                let _ = write(
                                    ws,
                                    &WireOut::Status {
                                        status: s.status.wire().to_string(),
                                        place_id: s.place_id,
                                        job_id: s.job_id.clone(),
                                    },
                                );
                            }
                        }
                        WireIn::Members { host, max, members } => {
                            connected = true;
                            emit(PartyEvent::Snapshot(PartySnapshot {
                                code: known_code.clone().unwrap_or_default(),
                                host,
                                max,
                                members: members.into_iter().map(WireMember::into_member).collect(),
                                connected: true,
                            }));
                        }
                        WireIn::Launch {
                            place_id,
                            job_id,
                            universe_id,
                            game_name,
                            by,
                        } => emit(PartyEvent::Launch(PartyLaunch {
                            place_id,
                            job_id,
                            universe_id,
                            game_name,
                            by,
                        })),
                        WireIn::Error { message } => {
                            // "full" / "no such party" are terminal: retrying would loop.
                            let fatal = message.contains("full")
                                || message.contains("No such")
                                || message.contains("required");
                            if fatal {
                                session = None;
                                known_code = None;
                                connected = false;
                                IN_PARTY.store(false, Ordering::Relaxed);
                                let _ = ws.close(None);
                                socket = None;
                                emit(PartyEvent::Left(message));
                            } else {
                                emit(PartyEvent::Notice(message));
                            }
                        }
                        WireIn::Pong => {}
                    }
                }
            }
            Ok(Message::Close(frame)) => {
                // 4001 is the relay saying "you were removed". Reconnecting would walk
                // straight back in: without accounts, a kicked member's next connection
                // looks exactly like a legitimate rejoin, so the party would silently
                // un-kick them a second after the host removed them.
                let removed = frame
                    .as_ref()
                    .map(|f| u16::from(f.code) == 4001)
                    .unwrap_or(false);
                socket = None;
                connected = false;
                if removed {
                    session = None;
                    known_code = None;
                    retry_at = None;
                    IN_PARTY.store(false, Ordering::Relaxed);
                    emit(PartyEvent::Left("Removed from the party".to_string()));
                } else {
                    attempts = attempts.saturating_add(1);
                    retry_at = Some(Instant::now() + backoff(attempts));
                    if session.is_none() {
                        IN_PARTY.store(false, Ordering::Relaxed);
                    }
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                socket = None;
                if connected {
                    connected = false;
                    emit(PartyEvent::Notice(format!(
                        "Party connection lost ({e}); reconnecting"
                    )));
                }
                attempts = attempts.saturating_add(1);
                retry_at = Some(Instant::now() + backoff(attempts));
            }
        }
    }

    IN_PARTY.store(false, Ordering::Relaxed);
}

fn backoff(attempts: u32) -> Duration {
    let secs = 1u64 << attempts.min(5);
    Duration::from_secs(secs.min(BACKOFF_MAX.as_secs()))
}

fn install_crypto_provider() {
    // rustls 0.23 needs a process-wide provider; ureq installs one, but the party thread may
    // be the first to use TLS on a fresh install, so claim it explicitly (idempotent).
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, tungstenite::Error> {
    install_crypto_provider();
    let (ws, _response) = tungstenite::connect(url)?;
    Ok(ws)
}

fn set_read_timeout(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, timeout: Option<Duration>) {
    match ws.get_ref() {
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(timeout);
        }
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_read_timeout(timeout);
        }
        _ => {}
    }
}

fn write(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, frame: &WireOut) -> bool {
    let Ok(text) = serde_json::to_string(frame) else {
        return false;
    };
    ws.send(Message::Text(text.into())).is_ok()
}

/// Best-effort send when the socket may not exist (Leave arrives before a connect finishes).
fn send(socket: &mut Option<WebSocket<MaybeTlsStream<TcpStream>>>, frame: WireOut) -> bool {
    match socket.as_mut() {
        Some(ws) => write(ws, &frame),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invites_accept_codes_and_links() {
        assert_eq!(parse_invite("CRB-8X2K"), Some("CRB-8X2K".to_string()));
        assert_eq!(parse_invite(" 8x2k "), Some("CRB-8X2K".to_string()));
        assert_eq!(
            parse_invite("creblox://party/CRB-8X2K"),
            Some("CRB-8X2K".to_string())
        );
        assert_eq!(
            parse_invite("https://creblox.app/p/ab12cd"),
            Some("CRB-AB12CD".to_string())
        );
        assert_eq!(parse_invite(""), None);
        assert_eq!(parse_invite("ab"), None);
        assert_eq!(parse_invite("CRB-"), None);
    }

    #[test]
    fn relay_url_falls_back_to_default() {
        assert_eq!(relay_url(""), DEFAULT_RELAY_URL);
        assert_eq!(relay_url("  "), DEFAULT_RELAY_URL);
        assert_eq!(
            relay_url("ws://127.0.0.1:8787/party"),
            "ws://127.0.0.1:8787/party"
        );
    }

    #[test]
    fn endpoint_keeps_existing_query_strings() {
        assert_eq!(
            join_url("wss://x/party", "create=1"),
            "wss://x/party?create=1"
        );
        assert_eq!(
            join_url("ws://127.0.0.1:8787/party?room=1", "code=CRB-AB12CD"),
            "ws://127.0.0.1:8787/party?room=1&code=CRB-AB12CD"
        );
    }

    #[test]
    fn host_target_needs_a_place_and_a_job() {
        let mut snap = PartySnapshot {
            code: "CRB-AB12CD".to_string(),
            host: 7,
            max: 8,
            members: Vec::new(),
            connected: true,
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
    fn frames_use_the_relay_wire_format() {
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
    }
}
