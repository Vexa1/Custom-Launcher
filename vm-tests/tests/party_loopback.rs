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
use creblox_vm_tests::roblox_process;
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
            assert!(
                !left.is_zero(),
                "{}: timed out waiting for {what}; {} events seen, last snapshot: {:?}",
                self.label,
                self.seen.len(),
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
    let names: Vec<&str> = roster
        .members
        .iter()
        .map(|m| m.user.name.as_str())
        .collect();
    assert!(
        names.contains(&"Ada") && names.contains(&"Grace"),
        "{names:?}"
    );

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

// ---------------------------------------------------------------------------------------
// Multi-member rooms: roster sync, kicks, host leave, group launch, presence fallback
// ---------------------------------------------------------------------------------------

/// `api::set_next` is process-wide, so the two presence tests take turns.
static PRESENCE_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

const MAX_PARTY: usize = party::MAX_PARTY_SIZE;

fn roster_user(id: u64, name: &str) -> PartyUser {
    PartyUser {
        id,
        name: name.to_string(),
        display_name: format!("{name} (display)"),
        headshot_url: format!("https://thumb.test/u{id}.png"),
    }
}

fn name_of(id: u64) -> &'static str {
    match id {
        1 => "hostess",
        2 => "guest-a",
        3 => "guest-b",
        _ => "guest-c",
    }
}

/// Events that arrived during `secs`, without the ones already collected.
fn drain_for(client: &mut Client, secs: u64) -> Vec<PartyEvent> {
    let mut fresh = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match client.rx.recv_timeout(left) {
            Ok(Msg::Party(event)) => {
                client.seen.push(event.clone());
                fresh.push(event);
            }
            Err(_) => break,
        }
    }
    fresh
}

fn wait_left(client: &mut Client, secs: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{}: the party never ended; saw {:?}",
            client.label,
            client.seen
        );
        let Ok(Msg::Party(event)) = client.rx.recv_timeout(left) else {
            panic!(
                "{}: channel closed waiting for the party to end",
                client.label
            );
        };
        client.seen.push(event.clone());
        if let PartyEvent::Left(reason) = event {
            return reason;
        }
    }
}

fn wait_launch(client: &mut Client, secs: u64) -> party::PartyLaunch {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{}: no launch arrived; saw {:?}",
            client.label,
            client.seen
        );
        let Ok(Msg::Party(event)) = client.rx.recv_timeout(left) else {
            panic!("{}: channel closed waiting for a launch", client.label);
        };
        client.seen.push(event.clone());
        if let PartyEvent::Launch(launch) = event {
            return launch;
        }
    }
}

fn launches(events: &[PartyEvent]) -> Vec<party::PartyLaunch> {
    events
        .iter()
        .filter_map(|event| match event {
            PartyEvent::Launch(launch) => Some(launch.clone()),
            _ => None,
        })
        .collect()
}

fn ids(snapshot: &PartySnapshot) -> Vec<u64> {
    let mut ids: Vec<u64> = snapshot.members.iter().map(|m| m.user.id).collect();
    ids.sort_unstable();
    ids
}

fn members_match(snapshot: &PartySnapshot, wanted: &[u64]) -> bool {
    ids(snapshot) == wanted
}

/// A host opens a room, `guests` join with the published invite and everybody sees `total`.
fn room_of(labels: &[&'static str]) -> (Client, Vec<Client>) {
    let mut host = Client::new("host");
    host.handle
        .create(roster_user(1, "hostess"), MAX_PARTY, None);
    let open = host.snapshot(30, "the invite", |s| !s.invite.is_empty());
    let invite = party::parse_invite(&open.invite).expect("the published invite parses");
    assert_eq!(invite.host_id, 1);

    let mut guests = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let id = index as u64 + 2;
        let guest = Client::new(label);
        guest.handle.join(invite, roster_user(id, label), None);
        guests.push(guest);
    }
    let total = labels.len() + 1;
    for client in std::iter::once(&mut host).chain(guests.iter_mut()) {
        client.snapshot(30, "the whole roster", |s| {
            s.members.len() == total && s.connected
        });
    }
    (host, guests)
}

#[test]
fn a_room_of_four_syncs_roster_statuses_and_the_host_badge() {
    let (mut host, mut guests) = room_of(&["guest-a", "guest-b", "guest-c"]);

    // Full roster everywhere: identities, headshots, display names and the host badge.
    for client in std::iter::once(&mut host).chain(guests.iter_mut()) {
        let snapshot = client
            .last_snapshot()
            .expect("the roster both sides agreed on")
            .clone();
        assert!(members_match(&snapshot, &[1, 2, 3, 4]) && snapshot.connected);
        assert_eq!(snapshot.host, 1);
        assert_eq!(snapshot.max, MAX_PARTY);
        assert!(snapshot.is_host(1));
        assert_eq!(snapshot.host_member().map(|m| m.user.id), Some(1));
        assert!(
            !snapshot.invite.is_empty(),
            "{}: no invite on the guest side",
            client.label
        );
        for member in &snapshot.members {
            let id = member.user.id;
            assert_eq!(
                member.user.display_name,
                format!("{} (display)", name_of(id))
            );
            assert_eq!(
                member.user.headshot_url,
                format!("https://thumb.test/u{id}.png")
            );
            assert_eq!(member.status, PartyStatus::Ready);
        }
    }

    // A member's session propagates to every other client.
    guests[0].handle.set_status(
        PartyStatus::InGame,
        Some(9001),
        Some("job-live".to_string()),
    );
    for client in std::iter::once(&mut host).chain(guests.iter_mut()) {
        let snapshot = client.snapshot(20, "guest-a in a server", |s| {
            s.members.iter().any(|m| {
                m.user.id == 2
                    && m.status == PartyStatus::InGame
                    && m.place_id == 9001
                    && m.job_id == "job-live"
            })
        });
        assert_eq!(snapshot.host, 1);
    }

    // Promotion moves the host badge without dropping anybody.
    host.handle.promote(3);
    for client in std::iter::once(&mut host).chain(guests.iter_mut()) {
        let snapshot = client.snapshot(20, "the new host badge", |s| s.host == 3);
        assert!(snapshot.is_host(3));
        assert_eq!(snapshot.host_member().map(|m| m.user.id), Some(3));
        assert_eq!(
            ids(&snapshot),
            vec![1, 2, 3, 4],
            "{}: {}",
            client.label,
            "promotion lost a member"
        );
    }
    host.handle.leave();
}

#[test]
fn a_kicked_member_is_told_and_stays_out() {
    let (mut host, mut guests) = room_of(&["guest-a", "guest-b"]);
    let (mut guest_a, mut guest_b) = (guests.remove(0), guests.remove(0));

    host.handle.kick(3);
    let reason = wait_left(&mut guest_b, 20);
    assert!(
        reason.to_lowercase().contains("removed"),
        "the kicked member was told {reason:?}"
    );
    for client in [&mut host, &mut guest_a] {
        client.snapshot(20, "two members after the kick", |s| {
            members_match(s, &[1, 2])
        });
    }

    // A kicked member must not walk back in on the reconnect timer: give it several retries
    // and check that nobody ever sees three again.
    let after = drain_for(&mut guest_b, 6);
    let rejoined: Vec<&PartySnapshot> = after
        .iter()
        .filter_map(|event| match event {
            PartyEvent::Snapshot(s) if s.connected || s.members.len() > 1 => Some(s),
            _ => None,
        })
        .collect();
    assert!(
        rejoined.is_empty(),
        "guest-b reconnected after being kicked: {rejoined:?}"
    );
    let host_after = drain_for(&mut host, 1);
    assert!(
        !host_after
            .iter()
            .any(|e| matches!(e, PartyEvent::Snapshot(s) if s.members.len() > 2)),
        "the kicked member came back into the roster"
    );
}

#[test]
fn the_host_leaving_ends_the_party_for_everyone() {
    let (host, mut guests) = room_of(&["guest-a", "guest-b"]);
    host.handle.leave();

    for guest in guests.iter_mut() {
        let reason = wait_left(guest, 20);
        assert!(
            reason.to_lowercase().contains("host"),
            "{}: the party ended with {reason:?}",
            guest.label
        );
        // Nothing to reconnect to: the host is gone, so a retry loop would just spin.
        let after = drain_for(guest, 4);
        assert!(
            !after
                .iter()
                .any(|e| matches!(e, PartyEvent::Snapshot(s) if s.connected)),
            "{}: kept reconnecting to a host that left",
            guest.label
        );
    }
}

#[test]
fn a_group_launch_reaches_every_guest_with_the_same_server() {
    let _ = roblox_process::take_launches();
    let (host, mut guests) = room_of(&["guest-a", "guest-b", "guest-c"]);

    let launch = party::PartyLaunch {
        place_id: 7788,
        job_id: "job-group-1".to_string(),
        universe_id: 55,
        game_name: "Group Game".to_string(),
        by: 1,
    };
    host.handle.broadcast_launch(launch.clone());

    let mut delivered = Vec::new();
    for guest in guests.iter_mut() {
        let seen = wait_launch(guest, 20);
        assert_eq!(seen, launch, "{} got a different server", guest.label);
        // Exactly the call `app.rs` makes for `PartyEvent::Launch`.
        let launched = roblox_process::launch_authenticated_server(
            seen.place_id,
            Some(&seen.job_id),
            "cookie",
        );
        assert!(
            launched.is_ok(),
            "{}: {}",
            guest.label,
            launched.unwrap_err()
        );
        delivered.push(seen);
    }
    assert_eq!(delivered.len(), 3);
    assert!(
        delivered.iter().all(|launch| *launch == delivered[0]),
        "every guest must follow the same server"
    );

    let recorded = roblox_process::take_launches();
    assert_eq!(
        recorded.len(),
        3,
        "one launch handler per guest: {recorded:?}"
    );
    for (place, job, cookie) in &recorded {
        assert_eq!(*place, 7788);
        assert_eq!(job.as_deref(), Some("job-group-1"));
        assert_eq!(cookie, "cookie");
    }

    // One launch per guest even though the roster keeps flowing afterwards.
    let later = drain_for(&mut guests[0], 5);
    assert!(launches(&later).is_empty(), "guest-a launched twice");
    assert!(
        launches(&host.seen).is_empty(),
        "the host launches for itself"
    );

    // The roster shows the host in the target server, which powers "Join Party's Server".
    let snapshot = guests[0]
        .last_snapshot()
        .expect("the roster after the launch")
        .clone();
    assert_eq!(
        snapshot.host_target(),
        Some((7788, "job-group-1".to_string()))
    );
    assert_eq!(
        snapshot.host_member().map(|m| m.status),
        Some(PartyStatus::Launching)
    );
    host.handle.leave();
}

#[test]
fn a_blocked_direct_connection_follows_the_host_through_presence() {
    let guard = PRESENCE_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    creblox_vm_tests::api::clear_next();

    // A room whose endpoint nothing listens on: the port is bound and released, so the dial
    // is refused the same way a host behind a strict router is.
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
        listener.local_addr().expect("the bound port").port()
    };
    let invite = party::PartyInvite {
        address: std::net::Ipv4Addr::new(127, 0, 0, 1),
        port,
        host_id: 42,
        token: [7, 8, 9, 10, 11, 12],
    };
    creblox_vm_tests::api::set_next(4242, "job-presence", 17, "Presence Game");

    let mut follower = Client::new("follower");
    follower.handle.join(
        invite,
        roster_user(9, "follower"),
        Some("cookie".to_string()),
    );

    follower.wait_notice(20, "following their Roblox presence");
    let launch = wait_launch(&mut follower, 25);
    assert_eq!(launch.place_id, 4242);
    assert_eq!(launch.job_id, "job-presence");
    assert_eq!(launch.universe_id, 17);
    assert_eq!(launch.game_name, "Presence Game");
    assert_eq!(launch.by, 42);

    // Two more polls of the same session must not launch again.
    let after = drain_for(&mut follower, 9);
    assert!(
        launches(&after).is_empty(),
        "the same presence session launched twice: {:?}",
        launches(&after)
    );
    creblox_vm_tests::api::clear_next();
    drop(guard);
}

#[test]
fn a_socket_launch_is_not_repeated_by_the_presence_poll() {
    let guard = PRESENCE_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    creblox_vm_tests::api::clear_next();

    let mut host = Client::new("host");
    host.handle.create(
        roster_user(1, "hostess"),
        MAX_PARTY,
        Some("cookie".to_string()),
    );
    let open = host.snapshot(30, "the invite", |s| !s.invite.is_empty());
    let invite = party::parse_invite(&open.invite).expect("the published invite parses");
    let mut guest = Client::new("guest-a");
    guest.handle.join(
        invite,
        roster_user(2, "guest-a"),
        Some("cookie".to_string()),
    );
    for client in [&mut host, &mut guest] {
        client.snapshot(30, "both members", |s| s.members.len() == 2);
    }

    // The host's presence says the same thing the socket is about to say.
    creblox_vm_tests::api::set_next(6001, "job-once", 21, "Once Game");
    host.handle.broadcast_launch(party::PartyLaunch {
        place_id: 6001,
        job_id: "job-once".to_string(),
        universe_id: 21,
        game_name: "Once Game".to_string(),
        by: 1,
    });
    let launch = wait_launch(&mut guest, 20);
    assert_eq!(launch.place_id, 6001);
    assert_eq!(launch.job_id, "job-once");

    // The presence poll runs while connected; it must recognise a session already followed.
    let after = drain_for(&mut guest, 10);
    assert!(
        launches(&after).is_empty(),
        "the presence poll repeated a socket launch: {:?}",
        launches(&after)
    );
    creblox_vm_tests::api::clear_next();
    drop(guard);
    host.handle.leave();
}

/// Waits for a `HopRequest` naming `user_id`, consuming whatever else arrives.
fn wait_hop(client: &mut Client, user_id: u64, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        let events = drain_for(client, 1);
        if events
            .iter()
            .any(|e| matches!(e, PartyEvent::HopRequest(id) if *id == user_id))
        {
            return true;
        }
    }
    false
}

#[test]
fn a_hop_request_reaches_the_host_exactly_once() {
    // `room_of` already waited for the guest to be in the party.
    let (mut host, mut guests) = room_of(&["guest-a"]);
    let guest = &mut guests[0];

    guest.handle.request_hop();
    assert!(
        wait_hop(&mut host, 2, 20),
        "the host never saw the hop request"
    );
    let after = drain_for(&mut host, 2);
    assert!(
        !after
            .iter()
            .any(|e| matches!(e, PartyEvent::HopRequest(_))),
        "one request produced more than one HopRequest: {after:?}"
    );

    // A plain status update is not a hop request, so the host bar stays quiet.
    guest.handle.set_status(PartyStatus::Ready, None, None);
    let status = drain_for(&mut host, 2);
    assert!(
        !status
            .iter()
            .any(|e| matches!(e, PartyEvent::HopRequest(_))),
        "a status update raised a hop request: {status:?}"
    );
    host.handle.leave();
}

#[test]
fn a_member_already_in_the_announced_server_is_not_relaunched() {
    let (mut host, mut guests) = room_of(&["guest-a"]);
    let guest = &mut guests[0];

    // The guest is already playing in the server the host is about to announce, which is
    // what a manual join (or a restarted party worker) looks like from here.
    guest
        .handle
        .set_status(PartyStatus::InGame, Some(7001), Some("job-here".to_string()));
    host.snapshot(20, "the guest's session", |s| {
        s.members
            .iter()
            .any(|m| m.user.id == 2 && m.job_id == "job-here")
    });

    host.handle.broadcast_launch(party::PartyLaunch {
        place_id: 7001,
        job_id: "job-here".to_string(),
        universe_id: 33,
        game_name: "Here".to_string(),
        by: 1,
    });
    let quiet = drain_for(guest, 3);
    assert!(
        launches(&quiet).is_empty(),
        "a member already in that server was relaunched: {:?}",
        launches(&quiet)
    );

    // A new server is still followed automatically.
    host.handle.broadcast_launch(party::PartyLaunch {
        place_id: 7002,
        job_id: "job-next".to_string(),
        universe_id: 33,
        game_name: "Next".to_string(),
        by: 1,
    });
    let launch = wait_launch(guest, 20);
    assert_eq!(launch.job_id, "job-next");
    assert_eq!(launch.place_id, 7002);
    host.handle.leave();
}

#[test]
fn a_member_that_is_slow_to_say_hello_is_still_let_in() {
    use tungstenite::client::IntoClientRequest;
    use tungstenite::http::header::{HeaderName, HeaderValue};

    let mut host = Client::new("host");
    host.handle
        .create(roster_user(1, "hostess"), MAX_PARTY, None);
    let open = host.snapshot(30, "the invite", |s| !s.invite.is_empty());
    let invite = party::parse_invite(&open.invite).expect("the published invite parses");

    // A hand-rolled peer that wins the TCP race but sends its hello 200 ms later, which is
    // what a scheduling stall looks like to the host.
    let addr = invite.endpoint();
    let mut request = format!("ws://{addr}/party")
        .as_str()
        .into_client_request()
        .expect("a websocket request");
    request.headers_mut().insert(
        HeaderName::from_static("x-creblox-party"),
        HeaderValue::from_static("1"),
    );
    let token: String = invite
        .token
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    request.headers_mut().insert(
        HeaderName::from_static("x-creblox-token"),
        HeaderValue::from_str(&token).expect("hex token"),
    );
    let stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .expect("the host's port is open");
    let (mut ws, _) = tungstenite::client(request, stream).expect("the handshake");
    std::thread::sleep(Duration::from_millis(200));
    ws.send(tungstenite::Message::Text(
        r#"{"t":"join","code":"CRB-test","user":{"id":7,"name":"slowpoke","display_name":"slowpoke","headshot_url":""},"max":8}"#
            .into(),
    ))
    .expect("the hello frame");

    let snapshot = host.snapshot(15, "the slow member in the roster", |s| {
        members_match(s, &[1, 7])
    });
    assert_eq!(snapshot.max, MAX_PARTY);
    host.handle.leave();
}

#[test]
fn a_promoted_member_can_lead_the_launch() {
    let (mut host, mut guests) = room_of(&["guest-a", "guest-b"]);
    let (mut guest_a, mut guest_b) = (guests.remove(0), guests.remove(0));

    host.handle.promote(3);
    for client in [&mut host, &mut guest_a, &mut guest_b] {
        client.snapshot(20, "the promoted host badge", |s| s.host == 3);
    }

    // The promoted member's Play goes through the room the same way the original host's does:
    // its frame is authorised, relayed to everyone and followed by the hosting process.
    guest_b.handle.broadcast_launch(party::PartyLaunch {
        place_id: 5150,
        job_id: "job-promoted".to_string(),
        universe_id: 12,
        game_name: "Promoted Game".to_string(),
        by: 3,
    });

    let from_guest = wait_launch(&mut guest_a, 20);
    assert_eq!(from_guest.by, 3, "the launch is attributed to the new host");
    assert_eq!(from_guest.place_id, 5150);
    assert_eq!(from_guest.job_id, "job-promoted");
    assert_eq!(from_guest.universe_id, 12);
    assert_eq!(from_guest.game_name, "Promoted Game");

    let from_host = wait_launch(&mut host, 20);
    assert_eq!(from_host, from_guest, "the hosting process follows too");

    // The sender is sent its own frame back; `app.rs` drops it when `by` is the local user.
    let echo = wait_launch(&mut guest_b, 20);
    assert_eq!(echo, from_guest);
    host.handle.leave();
}

#[test]
fn a_member_over_the_room_size_is_turned_away_for_good() {
    let mut host = Client::new("host");
    host.handle.create(roster_user(1, "hostess"), 2, None);
    let open = host.snapshot(30, "the invite", |s| !s.invite.is_empty());
    assert_eq!(open.max, 2);
    let invite = party::parse_invite(&open.invite).expect("the published invite parses");

    let mut first = Client::new("guest-a");
    first.handle.join(invite, roster_user(2, "guest-a"), None);
    first.snapshot(30, "the two-member room", |s| members_match(s, &[1, 2]));

    // The room is full: the refusal is terminal, not another backoff attempt.
    let mut second = Client::new("guest-b");
    second.handle.join(invite, roster_user(3, "guest-b"), None);
    let reason = wait_left(&mut second, 20);
    assert!(
        reason.contains("full"),
        "the refused member was told {reason:?}"
    );
    let after = drain_for(&mut second, 5);
    assert!(
        !after
            .iter()
            .any(|e| matches!(e, PartyEvent::Snapshot(s) if s.connected)),
        "a member the room refused kept knocking"
    );
    host.handle.leave();
}
