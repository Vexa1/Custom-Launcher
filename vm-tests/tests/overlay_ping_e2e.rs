//! End-to-end Overlay & Ping Pipeline, driven with the real shipping code.
//!
//! The pieces below are the ones a live session uses, not replicas:
//!
//! 1. `game_log` parses a Roblox log chunk into `(ip, port, job, place)`.
//! 2. `net_probe::probe_server` runs the shipping ICMP -> RakNet -> TCP chain against local
//!    mock relays, including one that behaves like a UDMUX relay and silently drops
//!    unauthenticated RakNet pings.
//! 3. `overlay_spec::build` turns that measurement into the panels JSON, and
//!    `overlay_spec::write_atomic` writes `overlay-panels-<id>.json` the way the launcher does.
//! 4. `hud` draws the file's bytes with the shipping HUD tokens and saves a PNG that is
//!    inspected by eye and checked by the layout rules (no clipping, no overlap, contrast).
//!
//! No Roblox install and no network access: every target is a socket in this process.

use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use creblox_vm_tests::game_log::{extract_server_info_from_log, inspect_player_log_text, LogSessionState};
use creblox_vm_tests::net_probe::{
    icmp_rtt_traced, probe_line, probe_server, udp_rtt_traced,
};
use creblox_vm_tests::overlay_spec::{
    self, Live, LiveServer, Panels, ServerPanel, StatsPanel,
};
use creblox_vm_tests::{hud, shots};

const PLACE_ID: u64 = 142_823_291;
const JOB_ID: &str = "6f1c2b40-0000-4000-8000-0000000000aa";

/// A UDMUX-like relay: bound, reachable, and it drops every datagram it receives. This is what
/// Roblox's live relay does to unauthenticated `ID_UNCONNECTED_PING`s.
fn udmux_relay() -> (u16, Arc<AtomicBool>) {
    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind the mock UDMUX relay");
    sock.set_read_timeout(Some(Duration::from_millis(50)))
        .expect("timeout the mock relay");
    let port = sock.local_addr().expect("mock relay address").port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while !flag.load(Ordering::Relaxed) {
            // Read and drop: the packet reaches a bound socket and is never answered.
            let _ = sock.recv_from(&mut buf);
        }
    });
    (port, stop)
}

/// A RakNet server that answers `ID_UNCONNECTED_PONG` after `delay`, which is what a healthy
/// game server on the player's path does.
fn pong_server(delay: Duration) -> (u16, Arc<AtomicBool>) {
    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind the mock game server");
    sock.set_read_timeout(Some(Duration::from_millis(50)))
        .expect("timeout the mock game server");
    let port = sock.local_addr().expect("mock server address").port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while !flag.load(Ordering::Relaxed) {
            let Ok((n, peer)) = sock.recv_from(&mut buf) else {
                continue;
            };
            if n == 0 {
                continue;
            }
            std::thread::sleep(delay);
            // The smallest real pong: 0x1c, the echoed send time, a guid and the magic.
            let mut pong = vec![0u8; 33];
            pong[0] = 0x1c;
            let echoed = 8.min(n.saturating_sub(1));
            pong[1..1 + echoed].copy_from_slice(&buf[1..1 + echoed]);
            let _ = sock.send_to(&pong, peer);
        }
    });
    (port, stop)
}

/// A TCP edge that accepts and immediately closes: the tier that answers when a relay filters
/// both ICMP and RakNet.
fn tcp_edge() -> (u16, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the mock TCP edge");
    listener
        .set_nonblocking(true)
        .expect("non-blocking mock edge");
    let port = listener.local_addr().expect("mock edge address").port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => drop(stream),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    (port, stop)
}

/// A loopback port nothing is listening on, for the "every tier failed" case.
fn closed_port(udp: bool) -> u16 {
    if udp {
        let sock = UdpSocket::bind("127.0.0.1:0").expect("bind a scratch UDP socket");
        sock.local_addr().expect("scratch UDP address").port()
    } else {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a scratch TCP socket");
        listener.local_addr().expect("scratch TCP address").port()
    }
}

/// A realistic Roblox session chunk: the join banner names a private RCC address, then the
/// UDMUX relay line names the address the client actually talks to - here the local mock.
fn session_log(relay_port: u16) -> String {
    format!(
        "\
2026-11-27T09:00:00.000Z,1.000000,0a01,6 [FLog::Output] ! Joining game '{JOB_ID}' place {PLACE_ID} at [10.12.2.219]:63976
2026-11-27T09:00:00.100Z,1.100000,0a01,7 [FLog::Network] serverId: 10.12.2.219|63976
2026-11-27T09:00:00.200Z,1.200000,0a01,7 [FLog::Network] UDMUX Address = 127.0.0.1, Port = {relay_port}
2026-11-27T09:00:00.300Z,1.300000,0a01,6 [FLog::Output] Connecting to 127.0.0.1:{relay_port}
2026-11-27T09:00:03.000Z,4.000000,0a01,6 [FLog::SessionTransitionFSM] Entered play session.
"
    )
}

#[test]
fn the_pipeline_from_a_roblox_log_to_a_rendered_live_ping() {
    // --- 1. Replay the session into the real parser ---------------------------------------
    let (relay_port, relay_stop) = udmux_relay();
    let log = session_log(relay_port);
    let info = extract_server_info_from_log(&log).expect("the session names a server");
    assert_eq!(info.ip, "127.0.0.1", "the UDMUX relay, not the private RCC");
    assert_eq!(info.port, relay_port);
    assert_eq!(info.job_id, JOB_ID);
    assert_eq!(info.place_id, PLACE_ID);
    assert_eq!(
        inspect_player_log_text(&log, log.len() as u64).0,
        LogSessionState::Playing,
        "a join banner with no leave is a live session"
    );

    // --- 2. Probe the mock UDMUX relay with the shipping chain ----------------------------
    // The relay drops RakNet: that tier must say so rather than hang or answer 0 ms.
    let raknet = udp_rtt_traced(&info.ip, info.port).expect_err("the relay drops unconnected pings");
    assert!(
        raknet.contains("no pong in 400 ms"),
        "the dropped ping is explained: {raknet}"
    );

    // A reachable TCP edge is the local stand-in for the data-centre edge. Whether ICMP
    // answers depends on the host (it is unavailable on Linux), so the chain may legitimately
    // take tier one first; the point is that a live address never stays at None.
    let (edge_port, edge_stop) = tcp_edge();
    let outcome = probe_server("127.0.0.1", relay_port, &[edge_port])
        .expect("a reachable address always produces a number");
    assert!(outcome.ms > 0, "a real measurement, not 0 ms: {outcome:?}");
    if icmp_rtt_traced("127.0.0.1").is_err() {
        assert_eq!(
            outcome.via, "tcp",
            "with ICMP unavailable and RakNet dropped, the TCP tier is the fallback"
        );
    }

    // Mock target B: a healthy RakNet server that answers after ~30 ms. The same shipping
    // chain must report that number through the RakNet tier, and this session is the one the
    // HUD picture below shows - a measured round trip, not a loopback 1 ms.
    let (pong_port, pong_stop) = pong_server(Duration::from_millis(30));
    let healthy_log = session_log(pong_port);
    let healthy = extract_server_info_from_log(&healthy_log).expect("the healthy session");
    assert_eq!(healthy.port, pong_port);
    let healthy_outcome = probe_server(&healthy.ip, healthy.port, &[edge_port])
        .expect("the healthy server answers");
    if icmp_rtt_traced("127.0.0.1").is_err() {
        assert_eq!(
            healthy_outcome.via, "raknet",
            "a pong must be measured by the RakNet tier, not the TCP fallback"
        );
        assert!(
            (25..=60).contains(&healthy_outcome.ms),
            "a 30 ms pong measures as tens of milliseconds, not {} ms",
            healthy_outcome.ms
        );
    }

    // --- 3. The panels file the overlay window reads --------------------------------------
    let dir = std::env::temp_dir().join(format!(
        "creblox-e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create the e2e folder");
    let spec_path = dir.join(format!("overlay-panels-{}.json", healthy.place_id));

    let server = ServerPanel {
        corner: 2,
        duration_secs: 0,
        show_ip: true,
        show_job_id: true,
    };
    let stats = StatsPanel {
        corner: 3,
        cpu: true,
        ram: true,
        ping: true,
        session: true,
        clock: true,
        fps_cap: Some(240),
    };
    let live_for = |ping: Option<u32>| Live {
        server: Some(LiveServer {
            ip: &healthy.ip,
            job_id: &healthy.job_id,
            place_id: healthy.place_id,
        }),
        server_age_secs: 12,
        session_secs: 65,
        detect_secs: 65,
        region: Some("Sydney, AU"),
        ping_ms: ping,
        ping_path: None,
        game: Some(("Adopt Me!", Some(312_884))),
        cpu_pct: Some(12.0),
        ram_mb: Some(640.0),
        clock: "09:00".to_string(),
        edit_preview: false,
    };

    // First write: the beat before the probe answers, which is the `-- ms` state.
    let before = overlay_spec::spec_json(
        0.9,
        true,
        0,
        0,
        false,
        overlay_spec::build(
            &Panels {
                server: Some(&server),
                stats: Some(&stats),
                flags: None,
                game: None,
            },
            &live_for(None),
        ),
    )
    .to_string();
    assert!(
        overlay_spec::write_atomic(&spec_path, &before),
        "the atomic writer installs the file"
    );
    let blank = std::fs::read_to_string(&spec_path).expect("read the first spec");
    assert!(blank.contains("Ping     -- ms"), "{blank}");

    // Second write: the probe answered. The same path is rewritten atomically with the number
    // and the region, and the completed file is what the renderer below is given.
    let after = overlay_spec::spec_json(
        0.9,
        true,
        0,
        0,
        false,
        overlay_spec::build(
            &Panels {
                server: Some(&server),
                stats: Some(&stats),
                flags: None,
                game: None,
            },
            &live_for(Some(healthy_outcome.ms)),
        ),
    )
    .to_string();
    assert!(
        overlay_spec::write_atomic(&spec_path, &after),
        "the rewrite replaces the file"
    );
    assert!(
        !spec_path.with_extension("json.tmp").exists(),
        "the temporary file never survives the rename"
    );
    let disk = std::fs::read_to_string(&spec_path).expect("read the rewritten spec");
    assert!(
        disk.contains(&format!("Ping     {} ms", healthy_outcome.ms)),
        "the HUD carries the measured number: {disk}"
    );
    assert!(!disk.contains("-- ms"), "no row may stay blank: {disk}");
    assert!(disk.contains("Server: Sydney, AU"), "{disk}");
    assert!(disk.contains("127.0.0.1"), "{disk}");
    let parsed: serde_json::Value = serde_json::from_str(&disk).expect("the file is valid JSON");

    // --- 4. Render those exact bytes and inspect the HUD ----------------------------------
    let viewport = egui::vec2(1280.0, 800.0);
    let mut harness = shots::Harness::new();
    let (shot, probe) = harness.render("overlay_live_ping", viewport, 1.5, |ui, probe| {
        hud::draw(ui, probe, &parsed)
    });
    let problems = probe.problems(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport));
    assert!(
        problems.is_empty(),
        "the rendered HUD has layout problems:\n{}",
        problems.join("\n")
    );
    let path = shot.save(&shots::artifact_dir());
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    assert!(bytes > 4_096, "the PNG is only {bytes} bytes: {path:?}");
    assert!(
        shot.distinct_colors() >= 60,
        "the frame looks blank ({} colours)",
        shot.distinct_colors()
    );
    println!(
        "overlay_live_ping.png: {} ({} bytes, ping {} ms via {})",
        path.display(),
        bytes,
        healthy_outcome.ms,
        healthy_outcome.via
    );

    relay_stop.store(true, Ordering::Relaxed);
    edge_stop.store(true, Ordering::Relaxed);
    pong_stop.store(true, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_healthy_raknet_server_reports_its_round_trip() {
    let (port, stop) = pong_server(Duration::from_millis(30));
    let started = Instant::now();
    let ms = udp_rtt_traced("127.0.0.1", port).expect("the mock game server answers");
    assert!(
        (25..=60).contains(&ms),
        "a 30 ms pong must measure as tens of milliseconds, not {} ms (elapsed {:?})",
        ms,
        started.elapsed()
    );
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn every_failed_tier_is_named_with_its_target_and_reason() {
    // TEST-NET-1 is never routed: actual network failures, no mock sockets to keep alive.
    // Loopback is unusable here because a host that answers ICMP to itself would pass tier
    // one and never exercise the failure path.
    let target_ip = "192.0.2.1";
    let game_port = closed_port(true);
    let edge_port = closed_port(false);
    let miss = probe_server(target_ip, game_port, &[edge_port])
        .expect_err("nothing answers TEST-NET-1");

    assert_eq!(miss.last, "tcp", "the chain gives up in the TCP tier");
    assert_eq!(miss.reasons.len(), 3, "one reason per tier: {:?}", miss.reasons);
    assert!(miss.reasons[0].starts_with("icmp "), "{:?}", miss.reasons);
    assert!(miss.reasons[1].starts_with("raknet "), "{:?}", miss.reasons);
    assert!(miss.reasons[2].starts_with("tcp "), "{:?}", miss.reasons);

    let target = format!("{target_ip}:{game_port}");
    let line = probe_line(&target, None, None, &miss.reasons);
    assert!(line.contains(&target), "the line names the target: {line}");
    assert!(line.contains("none ("), "{line}");
    assert!(line.contains("raknet "), "the dropped tier is named: {line}");
    assert!(line.contains("tcp "), "the last tier is named: {line}");

    // The same failure is what the HUD shows, instead of a bare `-- ms`.
    let row = overlay_spec::ping_row(None, Some(miss.last));
    assert_eq!(row, "Ping     -- ms (tcp)");
}
