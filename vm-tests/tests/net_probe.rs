//! The game-server RTT probe, exercised over loopback.
//!
//! The overlay must never print a number measured against something other than the server
//! the player is in, so the probe reports `None` rather than falling back to a web host.

use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

use creblox_vm_tests::net_probe;

#[test]
fn the_unconnected_ping_is_the_25_byte_raknet_packet() {
    let ping = net_probe::unconnected_ping(0x0102_0304_0506_0708);
    assert_eq!(ping.len(), 25);
    assert_eq!(ping[0], 0x01, "ID_UNCONNECTED_PING");
    assert_eq!(&ping[1..9], &[1, 2, 3, 4, 5, 6, 7, 8], "the send time");
    assert_eq!(
        &ping[9..],
        &[
            0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34,
            0x56, 0x78
        ],
        "RakNet's offline-message magic"
    );

    assert!(net_probe::is_unconnected_pong(&[0x1c; 33]));
    assert!(!net_probe::is_unconnected_pong(&[0x00; 33]));
    assert!(!net_probe::is_unconnected_pong(&[0x1c; 20]));
}

/// A real RakNet server: it receives `ID_UNCONNECTED_PING` and answers with
/// `ID_UNCONNECTED_PONG` (`0x1c`), which the probe must time.
#[test]
fn a_raknet_unconnected_ping_is_answered_with_a_pong() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 512];
        let (n, from) = server.recv_from(&mut buf).expect("the ping arrives");
        assert_eq!(n, 25, "the probe sends exactly the 25-byte packet");
        assert_eq!(buf[0], 0x01);
        assert_eq!(&buf[9..25], &net_probe::unconnected_ping(0)[9..25]);

        let mut pong = Vec::new();
        pong.push(0x1c); // ID_UNCONNECTED_PONG
        pong.extend_from_slice(&buf[1..9]); // the time we were sent
        pong.extend_from_slice(&[0u8; 8]); // server guid
        pong.extend_from_slice(&net_probe::unconnected_ping(0)[9..25]);
        server.send_to(&pong, from).expect("the pong is delivered");
    });

    let rtt = net_probe::udp_rtt("127.0.0.1", port).expect("a pong gives an RTT");
    assert!(rtt >= 1 && rtt < 1_000, "loopback RTT was {rtt} ms");
    handle.join().unwrap();
}

/// A relay that answers something that is not a pong must never be reported as the server's
/// round trip - `-- ms` is honest, a number measured against a relay is not.
#[test]
fn a_datagram_that_is_not_a_pong_is_not_timed() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 512];
        if let Ok((_, from)) = server.recv_from(&mut buf) {
            let _ = server.send_to(&[0u8; 25], from);
        }
    });

    let start = Instant::now();
    assert!(net_probe::udp_rtt("127.0.0.1", port).is_none());
    assert!(start.elapsed() < Duration::from_secs(2));
    handle.join().unwrap();
}

/// The room-list fallback is only ever the ping of the job the player is in.
#[test]
fn the_room_list_ping_is_used_only_for_the_matching_job() {
    let entries = vec![
        ("job-a".to_string(), Some(45u64)),
        ("job-b".to_string(), None),
        ("job-c".to_string(), Some(0)),
    ];
    assert_eq!(net_probe::ping_for_job(&entries, "job-a"), Some(45));
    assert_eq!(
        net_probe::ping_for_job(&entries, "job-b"),
        None,
        "the API did not report a ping for this job"
    );
    assert_eq!(
        net_probe::ping_for_job(&entries, "job-c"),
        None,
        "0 is the API's unknown"
    );
    assert_eq!(net_probe::ping_for_job(&entries, "job-z"), None);
    assert_eq!(
        net_probe::ping_for_job(&entries, ""),
        None,
        "no job id: nothing to match"
    );
}

#[test]
fn a_silent_server_yields_no_measurement_instead_of_a_wrong_one() {
    // Bind and drop, so nothing is listening on that port.
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);

    let start = Instant::now();
    let rtt = net_probe::udp_rtt("127.0.0.1", port);
    assert!(rtt.is_none(), "nothing answered, yet an RTT was reported");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn an_empty_address_is_never_probed() {
    assert!(net_probe::udp_rtt("", 0).is_none());
    assert!(net_probe::udp_rtt("   ", 53_640).is_none());
}

#[test]
fn ping_output_from_both_desktop_platforms_parses() {
    // macOS
    assert_eq!(
        net_probe::parse_ping_time("64 bytes from 128.116.51.33: icmp_seq=0 ttl=54 time=30.412 ms"),
        Some(30)
    );
    // Windows
    assert_eq!(
        net_probe::parse_ping_time("Reply from 128.116.51.33: bytes=32 time=30ms TTL=117"),
        Some(30)
    );
    assert_eq!(
        net_probe::parse_ping_time("Reply from 128.116.51.33: bytes=32 time<1ms TTL=128"),
        Some(1)
    );
    assert_eq!(net_probe::parse_ping_time("Request timed out."), None);
}
