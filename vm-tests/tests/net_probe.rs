//! The game-server RTT probe, exercised over loopback.
//!
//! The overlay must never print a number measured against something other than the server
//! the player is in, so the probe reports `None` rather than falling back to a web host.

use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

use creblox_vm_tests::net_probe;

#[test]
fn a_udp_round_trip_to_an_answering_socket_is_measured() {
    let echo = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = echo.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let mut buf = [0u8; 512];
        if let Ok((n, from)) = echo.recv_from(&mut buf) {
            let _ = echo.send_to(&buf[..n], from);
        }
    });

    let rtt = net_probe::udp_rtt("127.0.0.1", port).expect("an answering socket gives an RTT");
    assert!(rtt >= 1 && rtt < 1_000, "loopback RTT was {rtt} ms");
    server.join().unwrap();
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
