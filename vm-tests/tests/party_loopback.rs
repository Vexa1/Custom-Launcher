//! End-to-end checks for the real party client over loopback.
//!
//! These drive `src/party.rs` exactly as the launcher does - a host that listens on a real
//! socket, a guest that dials it and a guest that knocks with the wrong room token - so a
//! change to the invite codec, the handshake or the roster fails here before it ships.

use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use creblox_vm_tests::party::{
    self, PartyEvent, PartyHandle, PartySnapshot, PartyStatus, PartyUser,
};
use creblox_vm_tests::types::Msg;

struct Client {
    label: &'static str,
    handle: PartyHandle,
    rx: Receiver<Msg>,
    seen: Vec<PartyEvent>,
}

impl Client {
    fn new(label: &'static str) -> Self {
        let (tx, rx) = channel();
        let ctx = eframe::egui::Context::default();
        Self {
            label,
            handle: PartyHandle::new(tx, ctx),
            rx,
            seen: Vec::new(),
        }
    }

    /// Waits for the first snapshot matching `pred`, failing the test on timeout.
    fn snapshot(
        &mut self,
        secs: u64,
        what: &str,
        mut pred: impl FnMut(&PartySnapshot) -> bool,
    ) -> PartySnapshot {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "{}: timed out waiting for {what}", self.label);
            let Ok(Msg::Party(event)) = self.rx.recv_timeout(left) else {
                panic!("{}: channel closed while waiting for {what}", self.label);
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

    /// Drains events until one carries `needle`, failing the test on timeout.
    fn wait_notice(&mut self, secs: u64, needle: &str) -> PartyEvent {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{}: no notice containing {needle:?}; saw {} events",
                self.label,
                self.seen.len()
            );
            let Ok(Msg::Party(event)) = self.rx.recv_timeout(left) else {
                panic!("{}: channel closed waiting for {needle:?}", self.label);
            };
            let hit = matches!(&event, PartyEvent::Notice(text) if text.contains(needle));
            self.seen.push(event.clone());
            if hit {
                return event;
            }
        }
    }

    fn last_snapshot(&self) -> Option<&PartySnapshot> {
        self.seen.iter().rev().find_map(|event| match event {
            PartyEvent::Snapshot(snapshot) => Some(snapshot),
            _ => None,
        })
    }
}

fn user(id: u64, name: &str) -> PartyUser {
    PartyUser {
        id,
        name: name.to_string(),
        display_name: format!("{name} display"),
        headshot_url: String::new(),
    }
}

/// A host opens a room and its invite code carries a reachable endpoint and its own user id.
#[test]
fn a_host_publishes_an_invite_that_parses() {
    let mut host = Client::new("host");
    host.handle.create(user(100_001, "Ada"), 8, None);

    let ready = host.snapshot(25, "the invite code", |s| !s.invite.is_empty());
    let invite = party::parse_invite(&ready.invite).expect("the host's invite does not parse");

    assert!(ready.code.starts_with("CRB-"), "code was {}", ready.code);
    assert_eq!(invite.host_id, 100_001);
    assert!(invite.port > 0, "the invite must carry the listening port");
    assert_ne!(
        ready.invite, ready.code,
        "the invite is the full code while the pill shows the short label"
    );
    host.handle.leave();
}

/// A guest with the right code joins, and both sides agree on the roster.
#[test]
fn a_guest_joins_and_both_rosters_match() {
    let mut host = Client::new("host");
    let mut guest = Client::new("guest");

    host.handle.create(user(100_001, "Ada"), 8, None);
    let ready = host.snapshot(25, "the invite code", |s| !s.invite.is_empty());
    let invite = party::parse_invite(&ready.invite).expect("the host's invite does not parse");

    guest.handle.join(invite, user(100_002, "Grace"), None);
    let roster = guest.snapshot(20, "the guest's roster", |s| {
        s.connected && s.members.len() == 2
    });
    assert_eq!(roster.code, ready.code, "the guest landed in another room");
    assert_eq!(roster.host, 100_001);
    let names: Vec<&str> = roster.members.iter().map(|m| m.user.name.as_str()).collect();
    assert!(names.contains(&"Ada") && names.contains(&"Grace"), "{names:?}");

    let host_roster = host.snapshot(15, "the host's roster", |s| s.members.len() == 2);
    assert_eq!(host_roster.members.len(), 2);

    // The host's session reaches the guest as a joinable target, then a kick removes it.
    host.handle.set_status(
        PartyStatus::InGame,
        Some(920_587_237),
        Some("job-xyz".to_string()),
    );
    let seen = guest.snapshot(15, "the host's game state", |s| {
        s.members
            .iter()
            .any(|m| m.user.id == 100_001 && m.job_id == "job-xyz")
    });
    assert_eq!(
        seen.host_target(),
        Some((920_587_237, "job-xyz".to_string()))
    );

    host.handle.kick(100_002);
    let after = host.snapshot(15, "the roster without the kicked member", |s| {
        s.members.len() == 1
    });
    assert!(!after.members.iter().any(|m| m.user.id == 100_002));

    guest.handle.leave();
    host.handle.leave();
}

/// The room token is the only secret: a CREBLOX client that guesses wrong never gets in.
#[test]
fn a_wrong_room_token_is_refused() {
    let mut host = Client::new("host");
    let mut intruder = Client::new("intruder");

    host.handle.create(user(100_001, "Ada"), 8, None);
    let ready = host.snapshot(25, "the invite code", |s| !s.invite.is_empty());
    let mut invite = party::parse_invite(&ready.invite).expect("the host's invite does not parse");
    invite.token = [0, 0, 0, 0, 0, 0];

    intruder.handle.join(invite, user(100_009, "Mallory"), None);
    let refused = intruder.snapshot(12, "a refused handshake", |s| !s.connected);
    assert!(!refused.connected, "a wrong room token opened a socket");

    // The teaching notice arrives after the socket gives up, so it is waited for rather than
    // assumed to be in the same batch as the snapshot.
    let explained = intruder.wait_notice(5, "did not open");
    assert!(
        matches!(explained, PartyEvent::Notice(_)),
        "the refusal was silent instead of explained"
    );
    assert_eq!(
        host.last_snapshot().map(|s| s.members.len()),
        Some(1),
        "the refused stranger reached the roster"
    );

    intruder.handle.leave();
    host.handle.leave();
}
