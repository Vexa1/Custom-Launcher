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

/// One RakNet `ID_UNCONNECTED_PING` on a connected UDP socket, timed until any datagram comes
/// back. This is the protocol the game itself uses, so the number is the path the player is
/// on rather than an HTTP round trip somewhere else.
pub fn udp_rtt(ip: &str, port: u16) -> Option<u32> {
    let addr = (ip, port)
        .to_socket_addrs()
        .ok()?
        .find(|a| a.is_ipv4())?;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(UDP_TIMEOUT)).ok()?;
    sock.connect(addr).ok()?;

    let mut ping = [0u8; 25];
    ping[0] = 0x01; // ID_UNCONNECTED_PING
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    ping[1..9].copy_from_slice(&now.to_be_bytes());
    ping[9..].copy_from_slice(&RAKNET_MAGIC);

    let start = Instant::now();
    sock.send(&ping).ok()?;
    let mut buf = [0u8; 256];
    sock.recv(&mut buf).ok()?;
    Some(start.elapsed().as_millis().clamp(1, 9_999) as u32)
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

