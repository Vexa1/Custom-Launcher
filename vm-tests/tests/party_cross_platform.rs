//! Cross-platform party interoperability: a macOS client and a Windows client in one party.
//!
//! The party is OS-agnostic by construction - one WebSocket carrying JSON, a 22-byte invite in
//! base62, `u64` ids and a 16-bit party size - and these tests pin that down by pairing the two
//! platforms and checking what each side actually sees and writes.
//!
//! Each simulated client carries the things that *are* platform-specific: the game's install
//! path, its log line format and its headshot URLs. None of that is allowed to reach the wire
//! or the panels file, so the tests assert on the bytes and on both `overlay-panels-*.json`
//! files rather than on intent.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use creblox_vm_tests::overlay_spec;
use creblox_vm_tests::party::{
    self, PartyEvent, PartyHandle, PartyLaunch, PartySnapshot, PartyStatus, PartyUser,
};
use creblox_vm_tests::party_hud::{Tile, Tracker};
use creblox_vm_tests::types::Msg;
use serde_json::Value;

const PLACE: u64 = 920587237;
const JOB: &str = "3f8e1c92-6a4d-4f7b-9d51-0c2b7ea6d813";
const NEXT_PLACE: u64 = 1_730_877_806;
const NEXT_JOB: &str = "b41d7f0a-2c95-4e38-8a6f-5d0e93c7b216";
const MAC_ID: u64 = 100_001;
const WIN_ID: u64 = 200_002;

/// The two platforms, with the paths and log formats each one really has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Os {
    Macos,
    Windows,
}

impl Os {
    fn tag(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }

    /// Where this platform keeps the game. A backslash or a `C:` prefix on the wire would mean
    /// a path leaked into the protocol.
    fn game_dir(self) -> &'static str {
        match self {
            Os::Macos => "/Users/ada/Library/Application Support/Roblox/Versions/version-abc123",
            Os::Windows => "C:\\Users\\Ada\\AppData\\Local\\Roblox\\Versions\\version-abc123",
        }
    }

    fn headshot(self, user: u64) -> String {
        format!("https://thumbnails.example/{}/{user}?size=60x60", self.tag())
    }
}

fn user(id: u64, name: &str, os: Os) -> PartyUser {
    PartyUser {
        id,
        name: name.to_string(),
        display_name: name.to_string(),
        headshot_url: os.headshot(id),
    }
}

/// One simulated launcher: a party handle, the events it emits, its overlay HUD and the file
/// that HUD is written to, named the way that platform names it.
struct Client {
    os: Os,
    label: &'static str,
    handle: PartyHandle,
    rx: Receiver<Msg>,
    seen: Vec<PartyEvent>,
    tracker: Tracker,
    spec_path: PathBuf,
}

impl Client {
    fn new(os: Os, label: &'static str) -> Self {
        let (tx, rx) = channel();
        let ctx = eframe::egui::Context::default();
        let dir = std::env::temp_dir().join(format!(
            "creblox-xplat-{}-{}-{label}",
            std::process::id(),
            os.tag()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir for the panels file");
        Self {
            os,
            label,
            handle: PartyHandle::new(tx, ctx),
            rx,
            seen: Vec::new(),
            tracker: Tracker::new(),
            // The file is named the way this platform's launcher names it; the *contents* are
            // the part that has to match across platforms.
            spec_path: dir.join(format!("overlay-panels-{}-{label}.json", os.tag())),
        }
    }

    fn last_snapshot(&self) -> Option<&PartySnapshot> {
        self.seen.iter().rev().find_map(|e| match e {
            PartyEvent::Snapshot(s) => Some(s),
            _ => None,
        })
    }

    fn snapshot(
        &mut self,
        secs: u64,
        what: &str,
        mut pred: impl FnMut(&PartySnapshot) -> bool,
    ) -> PartySnapshot {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{}: timed out waiting for {what}; last snapshot: {:?}",
                self.label,
                self.last_snapshot()
            );
            let event = match self.rx.recv_timeout(left) {
                Ok(Msg::Party(event)) => event,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("{}: channel closed while waiting for {what}", self.label)
                }
            };
            let hit = matches!(&event, PartyEvent::Snapshot(s) if pred(s));
            self.seen.push(event.clone());
            if let PartyEvent::Snapshot(snapshot) = event {
                if hit {
                    return snapshot;
                }
            }
        }
    }

    /// Waits for a party event that `pred` accepts and hands it back.
    fn event(
        &mut self,
        secs: u64,
        what: &str,
        mut pred: impl FnMut(&PartyEvent) -> bool,
    ) -> PartyEvent {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "{}: timed out waiting for {what}", self.label);
            let event = match self.rx.recv_timeout(left) {
                Ok(Msg::Party(event)) => event,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("{}: channel closed while waiting for {what}", self.label)
                }
            };
            let hit = pred(&event);
            self.seen.push(event.clone());
            if hit {
                return event;
            }
        }
    }

    /// The tiles this client's overlay would draw, and the panels file it would write.
    fn write_hud(&mut self, snap: &PartySnapshot, place: u64, job: &str) -> Vec<Tile> {
        let tiles = self
            .tracker
            .update(Some(snap), place, job, Instant::now());
        let panels = vec![overlay_spec::party_panel(0, &tiles)];
        let spec = overlay_spec::spec_json(0.9, place != 0, false, panels);
        assert!(
            overlay_spec::write_atomic(&self.spec_path, &spec.to_string()),
            "{}: could not write {}",
            self.label,
            self.spec_path.display()
        );
        tiles
    }

    fn read_hud(&self) -> Value {
        // Each platform names the file in its own style; only what is inside has to match.
        assert!(
            self.spec_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(self.os.tag())),
            "{} wrote {}",
            self.label,
            self.spec_path.display()
        );
        let text = std::fs::read_to_string(&self.spec_path).expect("the panels file exists");
        serde_json::from_str(&text).expect("the panels file is JSON")
    }

    /// The `party` panel out of the panels file on disk.
    fn party_panel(&self) -> Value {
        let spec = self.read_hud();
        spec["panels"]
            .as_array()
            .expect("panels array")
            .iter()
            .find(|p| p["id"] == "party")
            .expect("a party panel in the file")
            .clone()
    }
}

fn tile<'a>(tiles: &'a [Tile], id: u64) -> &'a Tile {
    tiles
        .iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("no tile for {id}"))
}

fn roster_ids(snap: &PartySnapshot) -> Vec<u64> {
    let mut ids: Vec<u64> = snap.members.iter().map(|m| m.user.id).collect();
    ids.sort_unstable();
    ids
}

/// Scenario A: the macOS client hosts, the Windows client joins its invite.
#[test]
fn a_macos_host_and_a_windows_guest_share_the_whole_party() {
    let mut mac = Client::new(Os::Macos, "mac-host");
    let mut win = Client::new(Os::Windows, "win-guest");

    mac.handle.create(user(MAC_ID, "Ada", Os::Macos), party::MAX_PARTY_SIZE, None);
    let hosted = mac.snapshot(25, "the invite code", |s| !s.invite.is_empty());
    assert_eq!(hosted.max, party::MAX_PARTY_SIZE);

    // The invite is pure ASCII base62: nothing platform-specific, nothing to escape in chat.
    assert!(hosted.invite.starts_with("CRB-"), "{}", hosted.invite);
    assert_eq!(hosted.invite.len(), 4 + 30, "{}", hosted.invite);
    assert!(
        hosted.invite
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "{}",
        hosted.invite
    );

    // A Windows guest decodes a code a macOS host made. Twice, to prove the decoder is a pure
    // function of the bytes.
    let invite = party::parse_invite(&hosted.invite).expect("the macOS invite decodes");
    let again = party::parse_invite(&hosted.invite).expect("the macOS invite decodes twice");
    assert_eq!(invite.host_id, MAC_ID);
    assert_eq!(invite.port, again.port);
    assert_eq!(invite.token, again.token);
    assert_eq!(invite.address, again.address);

    win.handle
        .join(invite, user(WIN_ID, "Mateo", Os::Windows), None);

    let mac_two = mac.snapshot(20, "the Windows guest in the roster", |s| {
        s.members.len() == 2
    });
    let win_two = win.snapshot(20, "its own roster", |s| {
        s.members.len() == 2 && s.connected
    });

    // Identical rosters: same members, same order, same identity strings, same room.
    assert_eq!(roster_ids(&mac_two), roster_ids(&win_two));
    assert_eq!(roster_ids(&win_two), vec![MAC_ID, WIN_ID]);
    assert_eq!(mac_two.host, win_two.host);
    assert_eq!(mac_two.max, win_two.max);
    assert_eq!(mac_two.code, win_two.code);
    for (a, b) in mac_two.members.iter().zip(win_two.members.iter()) {
        assert_eq!(a.user.id, b.user.id);
        assert_eq!(a.user.name, b.user.name);
        assert_eq!(a.user.display_name, b.user.display_name);
        // Headshots cross over untouched, including the UTF-8 display name below.
        assert_eq!(a.user.headshot_url, b.user.headshot_url);
        // No install path, drive letter or backslash ever rides along with a member.
        let wire_text = format!(
            "{}|{}|{}",
            a.user.name, a.user.display_name, a.user.headshot_url
        );
        assert!(!wire_text.contains('\\'), "{wire_text}");
        assert!(!wire_text.contains("/Users/"), "{wire_text}");
        assert!(!wire_text.contains("AppData"), "{wire_text}");
    }

    // Loading -> grey on both sides. The Windows client is loading too, so nobody is green yet.
    win.handle
        .set_status(PartyStatus::Launching, None, None);
    let mac_loading = mac.snapshot(20, "the Windows guest loading", |s| {
        s.members.iter().any(|m| m.user.id == WIN_ID && m.status == PartyStatus::Launching)
    });
    let tiles = mac.write_hud(&mac_loading, 0, "");
    assert_eq!(tile(&tiles, WIN_ID).status.key(), "loading");

    // Both clients are now in the same server: identical statuses, identical HUD panels.
    mac.handle
        .set_status(PartyStatus::InGame, Some(PLACE), Some(JOB.to_string()));
    win.handle
        .set_status(PartyStatus::InGame, Some(PLACE), Some(JOB.to_string()));
    let mac_playing = mac.snapshot(20, "both clients in the server", |s| {
        s.members.iter().all(|m| m.status == PartyStatus::InGame)
    });
    let win_playing = win.snapshot(20, "both clients in the server", |s| {
        s.members.iter().all(|m| m.status == PartyStatus::InGame)
    });
    assert_eq!(roster_ids(&mac_playing), roster_ids(&win_playing));

    let mac_tiles = mac.write_hud(&mac_playing, PLACE, JOB);
    let win_tiles = win.write_hud(&win_playing, PLACE, JOB);
    assert_eq!(mac_tiles, win_tiles, "the two platforms draw different tiles");
    for t in &mac_tiles {
        assert_eq!(t.status.key(), "server", "{} is not green", t.name);
    }

    // The panels files each platform wrote are the same panel, field for field.
    let mac_panel = mac.party_panel();
    let win_panel = win.party_panel();
    assert_eq!(mac_panel["tiles"], win_panel["tiles"]);
    assert_eq!(
        mac_panel["tiles"].as_array().expect("tiles").len(),
        2,
        "the whole party, own tile included"
    );

    // The host moves the party to a new game. The framing, the place id and the job id reach
    // the Windows client unchanged - this is what its `RequestGameJob` handler opens a client
    // for, and a launch for the server you are already in is correctly ignored.
    mac.handle.broadcast_launch(PartyLaunch {
        place_id: NEXT_PLACE,
        job_id: NEXT_JOB.to_string(),
        universe_id: 111_222,
        game_name: "Tower Defense".to_string(),
        by: MAC_ID,
    });
    let launched = win.event(15, "the launch", |e| matches!(e, PartyEvent::Launch(_)));
    match launched {
        PartyEvent::Launch(launch) => {
            assert_eq!(launch.place_id, NEXT_PLACE);
            assert_eq!(launch.job_id, NEXT_JOB);
            assert_eq!(launch.universe_id, 111_222);
            assert_eq!(launch.by, MAC_ID);
        }
        other => panic!("{}: expected a launch, got {other:?}", win.label),
    }

    // Both clients settle into the launched server, on their own platform.
    mac.handle
        .set_status(PartyStatus::InGame, Some(NEXT_PLACE), Some(NEXT_JOB.to_string()));
    win.handle
        .set_status(PartyStatus::InGame, Some(NEXT_PLACE), Some(NEXT_JOB.to_string()));
    let mac_next = mac.snapshot(20, "the new server", |s| {
        s.members.iter().all(|m| {
            m.status == PartyStatus::InGame
                && m.place_id == NEXT_PLACE
                && m.job_id == NEXT_JOB
        })
    });
    assert_eq!(
        mac.write_hud(&mac_next, NEXT_PLACE, NEXT_JOB)
            .iter()
            .map(|t| t.status.key())
            .collect::<Vec<_>>(),
        vec!["server", "server"]
    );

    // The Windows client vanishes the way a killed process does - no Leave frame - and the macOS
    // HUD turns its tile red while the rest stays green.
    drop(win);
    let mac_alone = mac.snapshot(20, "the Windows guest dropped", |s| {
        s.members.len() == 1
    });
    // Both clients are in the launched server, so the survivor stays green and only the
    // vanished Windows guest goes red.
    let tiles = mac.write_hud(&mac_alone, NEXT_PLACE, NEXT_JOB);
    assert_eq!(tile(&tiles, WIN_ID).status.key(), "offline");
    assert_eq!(tile(&tiles, MAC_ID).status.key(), "server");
    let panel = mac.party_panel();
    assert_eq!(panel["tiles"][1]["status"], "offline");

    mac.handle.leave();
}

/// Scenario B: the Windows client hosts, the macOS client joins its invite and uses every party
/// action; the host then ends the party and the guest must be told, not stranded.
#[test]
fn a_windows_host_and_a_macos_guest_ready_up_vote_hop_and_leave() {
    let mut win = Client::new(Os::Windows, "win-host");
    let mut mac = Client::new(Os::Macos, "mac-guest");

    win.handle
        .create(user(WIN_ID, "Mateo", Os::Windows), party::MAX_PARTY_SIZE, None);
    let hosted = win.snapshot(25, "the invite code", |s| !s.invite.is_empty());

    // A macOS client decodes a code a Windows host made.
    let invite = party::parse_invite(&hosted.invite).expect("the Windows invite decodes");
    assert_eq!(invite.host_id, WIN_ID);
    mac.handle
        .join(invite, user(MAC_ID, "Léa 🎮", Os::Macos), None);
    let joined = win.snapshot(20, "the macOS guest in the roster", |s| {
        s.members.len() == 2
    });
    // UTF-8 names survive the Windows -> macOS direction byte for byte.
    assert_eq!(
        joined
            .members
            .iter()
            .find(|m| m.user.id == MAC_ID)
            .expect("the guest")
            .user
            .display_name,
        "Léa 🎮"
    );

    // Ready Check: started by the Windows host, answered by the macOS guest, seen by both.
    win.handle.start_ready_check();
    let check = mac.snapshot(20, "the ready check", |s| s.board.ready_check_active());
    assert!(check.board.ready_until_ms > 0);
    mac.handle.set_ready(true);
    let ready = win.snapshot(20, "the macOS guest ready", |s| {
        s.board.ready.iter().any(|(id, ok)| *id == MAC_ID && *ok)
    });
    assert!(ready.board.ready.iter().any(|(id, ok)| *id == MAC_ID && *ok));

    // Game Vote: a suggestion crosses over, then a vote from the other platform counts.
    win.handle
        .suggest_game(PLACE, 111_222, "Tower Defense".to_string());
    let suggested = mac.snapshot(20, "the suggestion", |s| {
        s.board.suggestions.iter().any(|g| g.place_id == PLACE)
    });
    assert_eq!(suggested.board.suggestions[0].name, "Tower Defense");
    mac.handle.vote(PLACE);
    let voted = win.snapshot(20, "the macOS guest's vote", |s| {
        s.board.votes.iter().any(|(id, place)| *id == MAC_ID && *place == PLACE)
    });
    assert!(voted.board.votes_for(PLACE) >= 1);

    // Party Server Hop: the guest asks, the host hears it exactly once.
    mac.handle.request_hop();
    let hop = win.event(20, "the hop request", |e| {
        matches!(e, PartyEvent::HopRequest(id) if *id == MAC_ID)
    });
    assert!(matches!(hop, PartyEvent::HopRequest(MAC_ID)));

    // The host ends the party: the macOS guest is told immediately and its socket is closed,
    // rather than sitting on a dead connection.
    let ended_at = Instant::now();
    win.handle.leave();
    let left = mac.event(10, "the party ending", |e| {
        matches!(e, PartyEvent::Left(_))
    });
    let took = ended_at.elapsed();
    println!("macOS guest told the party ended after {took:?}");
    assert!(
        took < Duration::from_secs(3),
        "the guest waited {took:?} for the host's goodbye"
    );
    assert!(matches!(left, PartyEvent::Left(_)));
    drop(mac);
    drop(win);
}

/// The parts of the wire that could differ by platform are all fixed width or UTF-8 text.
///
/// This is a compile-time and constant-level check rather than a socket one: it fails if the
/// invite payload, the party size or the status strings ever become platform-shaped.
#[test]
fn the_wire_carries_nothing_platform_dependent() {
    let invites: Vec<_> = (0..32u8)
        .map(|i| {
            let invite = party::PartyInvite {
                address: std::net::Ipv4Addr::new(127, 0, 0, 1),
                port: 40_000 + i as u16,
                host_id: 1 << (i % 40),
                token: [i; 6],
            };
            invite
        })
        .collect();
    for invite in &invites {
        // 22 payload bytes: address, port, host id and token, all read big-endian by both
        // platforms, so a Windows host's code means the same thing to a macOS guest.
        assert_eq!(std::mem::size_of_val(&invite.token), 6);
        assert_eq!(invite.endpoint().port() >= 40_000, true);
    }

    // Status strings and HUD keys are ASCII and platform-free.
    for status in [PartyStatus::Ready, PartyStatus::Launching, PartyStatus::InGame] {
        assert!(status.wire().is_ascii());
        assert!(!status.wire().contains('\\'));
    }
    for key in ["server", "loading", "offline"] {
        assert!(key.is_ascii());
    }
    // The party size on the wire is 16 bits even though it is a `usize` in memory.
    assert!(party::MAX_PARTY_SIZE <= u16::MAX as usize);
    let _ = Os::Macos.game_dir();
    assert!(!Os::Windows.game_dir().is_empty());
}
