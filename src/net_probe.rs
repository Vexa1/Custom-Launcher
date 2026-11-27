//! Round-trip time to the game server the player is actually connected to.
//!
//! A ping is only meaningful against the machine serving the game. Roblox's web edge is in
//! the US and answers in ~125 ms from New Zealand, while the live server in Sydney answers in
//! ~30 ms - so a web probe reports a number that has nothing to do with the session. This
//! module probes the server IP the client log names (over the same UDP path the game uses,
//! then ICMP) and returns `None` when that server does not answer, which is what the HUD
//! shows as `-- ms`.
//!
//! No image, no egui, no platform APIs: the whole module runs in the VM test harness.

use std::net::{ToSocketAddrs, UdpSocket};
use std::process::Command;
use std::time::{Duration, Instant};

/// RakNet's default game port when the log line carries no port.
pub const DEFAULT_GAME_PORT: u16 = 53_640;

const UDP_TIMEOUT: Duration = Duration::from_millis(400);

/// `00 ff ff 00 fe fe fe fe fd fd fd fd 12 34 56 78`: the offline-message magic RakNet
/// servers expect in an unconnected ping.
const RAKNET_MAGIC: [u8; 16] = [
    0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78,
];

/// The 25-byte RakNet `ID_UNCONNECTED_PING`: `0x01`, the 8-byte send time, then the
/// offline-message magic. This is the packet the game's own servers answer, so the measured
/// number is the path the player is on rather than an HTTP round trip somewhere else.
pub fn unconnected_ping(sent_at_ms: u64) -> [u8; 25] {
    let mut ping = [0u8; 25];
    ping[0] = 0x01; // ID_UNCONNECTED_PING
    ping[1..9].copy_from_slice(&sent_at_ms.to_be_bytes());
    ping[9..].copy_from_slice(&RAKNET_MAGIC);
    ping
}

/// `true` for `ID_UNCONNECTED_PONG` (`0x1c`), the only datagram that proves the game server
/// received our ping. A relay (UDMUX) that answers something else must never be timed as if
/// it were the server's round trip.
pub fn is_unconnected_pong(datagram: &[u8]) -> bool {
    // 0x1c + 8-byte time + 8-byte server guid + 16-byte magic is the smallest real pong.
    datagram.len() >= 33 && datagram[0] == 0x1c
}

/// One RakNet `ID_UNCONNECTED_PING` on a connected UDP socket, timed until the matching
/// `ID_UNCONNECTED_PONG` comes back. Datagrams of any other shape are ignored, and the whole
/// exchange gives up after [`UDP_TIMEOUT`] so a silent server costs one tick, not the loop.
pub fn udp_rtt(ip: &str, port: u16) -> Option<u32> {
    let ip = ip.trim();
    if ip.is_empty() {
        return None;
    }
    let addr = (ip, port)
        .to_socket_addrs()
        .ok()?
        .find(|a| a.is_ipv4())?;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(addr).ok()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let start = Instant::now();
    sock.send(&unconnected_ping(now)).ok()?;
    let deadline = start + UDP_TIMEOUT;
    let mut buf = [0u8; 512];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        sock.set_read_timeout(Some(left)).ok()?;
        let Ok(n) = sock.recv(&mut buf) else {
            return None;
        };
        if is_unconnected_pong(&buf[..n]) {
            return Some(start.elapsed().as_millis().clamp(1, 9_999) as u32);
        }
        // Not our answer: keep waiting for the pong until the deadline.
    }
}

/// The ping the public room list reports for one job.
///
/// This is the fallback for a server behind a UDMUX relay that drops unconnected pings: it is
/// the only published number tied to the exact job the player is in, so a job that is not
/// listed yields `None` instead of a stranger's ping.
pub fn ping_for_job(entries: &[(String, Option<u64>)], job_id: &str) -> Option<u32> {
    if job_id.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|(id, _)| id == job_id)
        .and_then(|(_, ping)| *ping)
        .filter(|ping| *ping > 0)
        .map(|ping| ping.min(9_999) as u32)
}

/// One ICMP echo through the system `ping`, for servers that answer echo but filter TCP and
/// ignore RakNet's unconnected ping.
pub fn icmp_rtt(ip: &str) -> Option<u32> {
    let out = ping_command(ip)?.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_ping_time(&text)
}

#[cfg(target_os = "macos")]
fn ping_command(ip: &str) -> Option<Command> {
    let mut cmd = Command::new("/sbin/ping");
    cmd.args(["-c", "1", "-t", "1", ip]);
    Some(cmd)
}

#[cfg(target_os = "windows")]
fn ping_command(ip: &str) -> Option<Command> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = Command::new("ping");
    cmd.args(["-n", "1", "-w", "1000", ip])
        .creation_flags(CREATE_NO_WINDOW);
    Some(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn ping_command(_ip: &str) -> Option<Command> {
    None
}

/// `time=30.4 ms` (macOS), `time=30ms` / `time<1ms` (Windows).
pub fn parse_ping_time(text: &str) -> Option<u32> {
    let idx = text.find("time")?;
    let rest = &text[idx + 4..];
    let eq = rest.find(['=', '<'])?;
    let num: String = rest[eq + 1..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let ms = num.parse::<f32>().ok()?;
    Some(ms.round().max(1.0) as u32)
}

