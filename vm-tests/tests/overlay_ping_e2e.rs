//! End-to-end Overlay & Ping Pipeline, driven with the real shipping code.
//!
//! The pieces below are the ones a live session uses, not replicas:
//!
//! 1. `game_log` parses a Roblox log chunk into `(ip, port, job, place)`, and reads the
//!    latency sample the client itself wrote.
//! 2. `net_probe` resolves the number in the shipping order: the client's own sample, the
//!    room list's ping for the job, a regional edge handshake, then the direct ICMP ->
//!    RakNet -> TCP chain against local mock relays.
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

use creblox_vm_tests::game_log::{
    client_ping_from_log, extract_server_info_from_log, inspect_player_log_text, LogSessionState,
};
use creblox_vm_tests::net_probe::{
    self, icmp_rtt_traced, is_roblox_datacenter, ping_for_job, preferred_ping, probe_line,
    probe_server, regional_edge, tcp_rtt_to_host, udp_rtt_traced,
};
use creblox_vm_tests::overlay_spec::{self, Live, LiveServer, Panels, ServerPanel, StatsPanel};
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
    let raknet =
        udp_rtt_traced(&info.ip, info.port).expect_err("the relay drops unconnected pings");
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
    let healthy_outcome =
        probe_server(&healthy.ip, healthy.port, &[edge_port]).expect("the healthy server answers");
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
    // One fixed anchor for both writes: the HUD clocks itself from it, so the rendered
    // `Session` row is 1:05 whatever the wall clock does while the test runs.
    let now_ms = overlay_spec::unix_ms_now();
    let live_for = |ping: Option<u32>| Live {
        server: Some(LiveServer {
            ip: &healthy.ip,
            job_id: &healthy.job_id,
            place_id: healthy.place_id,
        }),
        server_age_secs: 12,
        joined_at_unix_ms: now_ms.saturating_sub(65_000),
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
    // The timer row is an anchor plus a token: the seconds are the overlay's, not the file's.
    assert!(disk.contains("joined_at_unix_ms"), "{disk}");
    assert!(!disk.contains("Session  01:05"), "{disk}");
    let parsed: serde_json::Value = serde_json::from_str(&disk).expect("the file is valid JSON");

    // --- 4. Render those exact bytes and inspect the HUD ----------------------------------
    let viewport = egui::vec2(1280.0, 800.0);
    let mut harness = shots::Harness::new();
    let (shot, probe) = harness.render("overlay_live_ping_raknet", viewport, 1.5, |ui, probe| {
        hud::draw(ui, probe, &parsed, now_ms)
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
        "overlay_live_ping_raknet.png: {} ({} bytes, ping {} ms via {})",
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

/// The live case this release exists for: a real `128.116.51.33:50782` relay behind Roblox's
/// DDoS filter. ICMP, RakNet and TCP 80/443 are all blackholed, and unauthenticated paging of
/// the room list returns 429 - so the number has to come from the client's own log, then the
/// authenticated room list, then a regional edge, and the relay is never probed at all.
#[test]
fn a_filtered_datacenter_relay_gets_its_ping_without_probing_it() {
    let log = format!(
        "\
2026-11-27T09:00:00.000Z,1.000000,0a01,6 [FLog::Output] ! Joining game '{JOB_ID}' place {PLACE_ID} at [10.12.2.219]:50782
2026-11-27T09:00:00.100Z,1.100000,0a01,7 [FLog::Network] UDMUX Address = 128.116.51.33, Port = 50782
2026-11-27T09:00:05.000Z,6.000000,0a01,6 [DFLog::RakNet] Ping: 34 ms
2026-11-27T09:00:10.000Z,11.000000,0a01,6 [DFLog::RakNet] Ping: 36 ms
"
    );
    let info = extract_server_info_from_log(&log).expect("the session names its relay");
    assert_eq!(info.ip, "128.116.51.33");
    assert_eq!(info.port, 50_782);
    assert_eq!(info.job_id, JOB_ID);
    assert_eq!(info.place_id, PLACE_ID);
    assert!(
        is_roblox_datacenter(&info.ip),
        "a 128.116.x.x relay must never be probed directly"
    );

    // Primary source: the sample Roblox's own client wrote while playing.
    let client_ping = client_ping_from_log(&log).expect("the client's own latency sample");
    assert_eq!(client_ping, 36, "the newest sample, not the first");

    // The relay answers nothing: TEST-NET-1 stands in for 128.116.x.x, because both blackhole
    // every unsolicited packet and a test must never depend on Roblox's live routing.
    let miss = probe_server("192.0.2.1", info.port, &[closed_port(false)])
        .expect_err("a filtered relay answers no tier");
    assert_eq!(miss.reasons.len(), 3, "one reason per direct tier");

    // Secondary source: page one of the authenticated room list already carries this job.
    let page_one: Vec<(String, Option<u64>)> = vec![(JOB_ID.to_string(), Some(31))];
    assert_eq!(
        ping_for_job(&page_one, &info.job_id),
        Some(31),
        "the room list's ping for the exact job"
    );

    // Tertiary source: the edge in the geolocated city, measured for real over loopback.
    let edge = regional_edge("Sydney, New South Wales, AU").expect("Sydney has a regional edge");
    assert_eq!(edge.region, "ap-southeast-2");
    let (edge_port, edge_stop) = tcp_edge();
    let regional_ms = tcp_rtt_to_host("127.0.0.1", edge_port).expect("the edge answers");
    assert!(
        regional_ms > 0 && regional_ms < 1_000,
        "the regional handshake measured {regional_ms} ms"
    );

    // The shipping priority: the client's number wins, then the room list, then the edge.
    let reading = preferred_ping(Some(client_ping), Some(31), Some(regional_ms), None)
        .expect("a filtered relay still gets a number");
    assert_eq!(reading.source, net_probe::SOURCE_CLIENT_LOG);
    assert_eq!(reading.ms, 36);
    assert_eq!(
        preferred_ping(None, Some(31), Some(regional_ms), None)
            .unwrap()
            .source,
        net_probe::SOURCE_ROOM_LIST
    );
    assert_eq!(
        preferred_ping(None, None, Some(regional_ms), None)
            .unwrap()
            .source,
        net_probe::SOURCE_REGIONAL_EDGE
    );
    assert_eq!(
        overlay_spec::ping_row(None, Some(net_probe::SOURCE_REGIONAL_EDGE)),
        "Ping     -- ms (regional edge)",
        "a failed edge is named in the HUD, not left blank"
    );

    // Write the panels file the overlay window reads and render those exact bytes.
    let dir = std::env::temp_dir().join(format!(
        "creblox-e2e-blocked-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create the e2e folder");
    let spec_path = dir.join(format!("overlay-panels-{}.json", info.place_id));
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
    let now_ms = overlay_spec::unix_ms_now();
    let live = Live {
        server: Some(LiveServer {
            ip: &info.ip,
            job_id: &info.job_id,
            place_id: info.place_id,
        }),
        server_age_secs: 12,
        joined_at_unix_ms: now_ms.saturating_sub(65_000),
        detect_secs: 65,
        region: Some("Sydney, AU"),
        ping_ms: Some(reading.ms),
        ping_path: None,
        game: Some(("Adopt Me!", Some(312_884))),
        cpu_pct: Some(12.0),
        ram_mb: Some(640.0),
        clock: "09:00".to_string(),
        edit_preview: false,
    };
    let text = overlay_spec::spec_json(
        0.9,
        true,
        false,
        overlay_spec::build(
            &Panels {
                server: Some(&server),
                stats: Some(&stats),
                flags: None,
                game: None,
            },
            &live,
        ),
    )
    .to_string();
    assert!(
        overlay_spec::write_atomic(&spec_path, &text),
        "the atomic writer installs the file"
    );
    let disk = std::fs::read_to_string(&spec_path).expect("read the spec");
    assert!(
        disk.contains(&format!("Ping     {} ms", reading.ms)),
        "the HUD carries the client's measurement: {disk}"
    );
    assert!(!disk.contains("-- ms"), "no row may stay blank: {disk}");
    assert!(disk.contains("Server: Sydney, AU"), "{disk}");
    let parsed: serde_json::Value = serde_json::from_str(&disk).expect("valid JSON");

    let viewport = egui::vec2(1280.0, 800.0);
    let mut harness = shots::Harness::new();
    let (shot, probe) = harness.render("overlay_live_ping", viewport, 1.5, |ui, probe| {
        hud::draw(ui, probe, &parsed, now_ms)
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
        reading.ms,
        reading.source
    );

    edge_stop.store(true, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_failed_tier_is_named_with_its_target_and_reason() {
    // TEST-NET-1 is never routed: actual network failures, no mock sockets to keep alive.
    // Loopback is unusable here because a host that answers ICMP to itself would pass tier
    // one and never exercise the failure path.
    let target_ip = "192.0.2.1";
    let game_port = closed_port(true);
    let edge_port = closed_port(false);
    let miss =
        probe_server(target_ip, game_port, &[edge_port]).expect_err("nothing answers TEST-NET-1");

    assert_eq!(miss.last, "tcp", "the chain gives up in the TCP tier");
    assert_eq!(
        miss.reasons.len(),
        3,
        "one reason per tier: {:?}",
        miss.reasons
    );
    assert!(miss.reasons[0].starts_with("icmp "), "{:?}", miss.reasons);
    assert!(miss.reasons[1].starts_with("raknet "), "{:?}", miss.reasons);
    assert!(miss.reasons[2].starts_with("tcp "), "{:?}", miss.reasons);

    let target = format!("{target_ip}:{game_port}");
    let line = probe_line(&target, None, None, &miss.reasons);
    assert!(line.contains(&target), "the line names the target: {line}");
    assert!(line.contains("none ("), "{line}");
    assert!(
        line.contains("raknet "),
        "the dropped tier is named: {line}"
    );
    assert!(line.contains("tcp "), "the last tier is named: {line}");

    // The same failure is what the HUD shows, instead of a bare `-- ms`.
    let row = overlay_spec::ping_row(None, Some(miss.last));
    assert_eq!(row, "Ping     -- ms (tcp)");
}
