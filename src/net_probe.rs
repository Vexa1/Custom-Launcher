//! Round-trip time to the game server the player is actually connected to.
//!
//! A ping is only meaningful against the machine serving the game. Roblox's web edge is in
//! the US and answers in ~125 ms from New Zealand, while the live server in Sydney answers in
//! ~30 ms - so a web probe reports a number that has nothing to do with the session. This
//! module measures the server IP the client log names, in four tiers, and returns `None` when
//! none of them answers, which is what the HUD shows as `-- ms`:
//!
//! 1. The Roblox client's **own latency sample**, read from its Player log - no packets.
//! 2. The **public room list's ping** for the exact job, authenticated and cached per match.
//! 3. A **regional edge handshake** in the city the server was geolocated to.
//! 4. **ICMP / RakNet / TCP** against the server itself, only for a host that is not a Roblox
//!    relay: `128.116.x.x` blackholes every unsolicited packet, so probing it is wasted work.
//!
//! Roblox's UDMUX relays sit behind a DDoS filter that drops unauthenticated RakNet pings,
//! ICMP and TCP 80/443 alike, which left the row at `-- ms` for a whole session. The first
//! three sources never send a packet the relay would have to answer.
//!
//! No image, no egui, no platform APIs: the whole module runs in the VM test harness.

use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
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
/// Returns the RTT, or the reason it produced no number, so the probe can put that in
/// `log.txt` instead of leaving `-- ms` unexplained.
pub fn udp_rtt_traced(ip: &str, port: u16) -> Result<u32, String> {
    let ip = ip.trim();
    if ip.is_empty() {
        return Err("empty address".to_string());
    }
    let addr = (ip, port)
        .to_socket_addrs()
        .map_err(|e| format!("unresolved address ({e})"))?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| "no IPv4 address".to_string())?;
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind failed ({e})"))?;
    sock.connect(addr)
        .map_err(|e| format!("connect failed ({e})"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let start = Instant::now();
    sock.send(&unconnected_ping(now))
        .map_err(|e| format!("send failed ({e})"))?;
    let deadline = start + UDP_TIMEOUT;
    let timeout = || format!("no pong in {} ms", UDP_TIMEOUT.as_millis());
    let mut buf = [0u8; 512];
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| timeout())?;
        if left.is_zero() {
            return Err(timeout());
        }
        sock.set_read_timeout(Some(left))
            .map_err(|e| format!("read timeout failed ({e})"))?;
        let n = match sock.recv(&mut buf) {
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(timeout());
            }
            Err(e) => return Err(format!("recv failed ({e})")),
        };
        if is_unconnected_pong(&buf[..n]) {
            return Ok(start.elapsed().as_millis().clamp(1, 9_999) as u32);
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
///
/// This is tier one: Roblox's regional data-centre IPs answer echo over the same path the
/// game uses, so the number matches what the game's own performance stats show. A filtered
/// echo costs the bounded wait below and nothing else.
/// Returns the RTT, or the reason it produced no number (no reply, no tool, exit status).
pub fn icmp_rtt_traced(ip: &str) -> Result<u32, String> {
    let ip = ip.trim();
    if ip.is_empty() {
        return Err("empty address".to_string());
    }
    let Some(mut cmd) = ping_command(ip) else {
        return Err("no icmp tool on this platform".to_string());
    };
    let out = cmd
        .output()
        .map_err(|e| format!("ping could not run ({e})"))?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(code) => format!("no echo reply (ping exit {code})"),
            None => "no echo reply (ping killed)".to_string(),
        });
    }
    parse_ping_time(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "unparsable ping output".to_string())
}

#[cfg(target_os = "macos")]
fn ping_command(ip: &str) -> Option<Command> {
    use std::process::Stdio;
    let mut cmd = Command::new("/sbin/ping");
    // `-W` is the per-reply wait in milliseconds and `-t` bounds the whole run, so a relay
    // that drops echo can never hold the probe thread; `-n` keeps it numeric (no reverse DNS
    // on an address that may be dead) and stdin is closed so the child never waits on us.
    cmd.args(["-n", "-c", "1", "-W", "800", "-t", "1", ip])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    Some(cmd)
}

#[cfg(target_os = "windows")]
fn ping_command(ip: &str) -> Option<Command> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = Command::new("ping");
    cmd.args(["-n", "1", "-w", "800", ip])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    Some(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn ping_command(_ip: &str) -> Option<Command> {
    None
}

/// Ports a Roblox data-centre edge answers even when it filters ICMP and RakNet: the HTTP(S)
/// listeners in front of the same machines that host the game server.
pub const TCP_PROBE_PORTS: [u16; 2] = [443, 80];

/// How long one handshake may take before the next port is tried.
const TCP_TIMEOUT: Duration = Duration::from_millis(400);

/// Tier three: the TCP handshake RTT to the same IP.
///
/// UDMUX relays drop unauthenticated RakNet pings, and some drop ICMP too, but the edge in
/// front of the server still completes a handshake - and a handshake is a real round trip
/// over the player's path, not a web request to another continent. `None` when no port
/// answers, so the row falls through to the room list instead of inventing a number.
/// Returns the RTT, or the reason it produced no number, one word per port tried.
pub fn tcp_rtt_traced(ip: &str, ports: &[u16]) -> Result<u32, String> {
    let ip = ip.trim();
    if ip.is_empty() {
        return Err("empty address".to_string());
    }
    if ports.is_empty() {
        return Err("no port to try".to_string());
    }
    // Resolve once for every port: a dead address is not looked up again per port.
    let addrs = (ip, ports[0])
        .to_socket_addrs()
        .map_err(|e| format!("unresolved address ({e})"))?;
    let ip = addrs
        .map(|a| a.ip())
        .find(|ip| ip.is_ipv4())
        .ok_or_else(|| "no IPv4 address".to_string())?;
    let mut tried = Vec::new();
    for port in ports {
        let addr = std::net::SocketAddr::new(ip, *port);
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, TCP_TIMEOUT) {
            Ok(_) => return Ok(start.elapsed().as_millis().clamp(1, 9_999) as u32),
            Err(e) => tried.push(format!("{port} {}", tcp_reason(&e))),
        }
    }
    Err(tried.join(", "))
}

/// One word for a failed handshake, short enough to read inside a log line.
fn tcp_reason(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused => "refused",
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "timeout",
        std::io::ErrorKind::PermissionDenied => "blocked",
        std::io::ErrorKind::NetworkUnreachable => "unreachable",
        std::io::ErrorKind::HostUnreachable => "unreachable",
        _ => "failed",
    }
}

/// Which tier produced a number, and that number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// `icmp`, `raknet` or `tcp`.
    pub via: &'static str,
    pub ms: u32,
}

/// Why one whole probe run had no number: the tier the chain gave up on, plus one reason per
/// tier in the order they were tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeMiss {
    /// The last tier tried, which is what the HUD names after `-- ms`.
    pub last: &'static str,
    pub reasons: Vec<String>,
}

/// Runs every network tier in the shipping order and returns the first number, or every
/// reason it had none.
///
/// This is the exact chain the overlay runs on its probe thread (an integration test in
/// `vm-tests` drives it against local mock relays), so the order cannot drift between what
/// ships and what is tested: ICMP, then the RakNet unconnected ping, then a TCP handshake.
/// `tcp_ports` is passed in so the caller chooses what an edge can accept; the overlay uses
/// [`TCP_PROBE_PORTS`].
pub fn probe_server(ip: &str, port: u16, tcp_ports: &[u16]) -> Result<ProbeOutcome, ProbeMiss> {
    let mut reasons = Vec::new();
    match icmp_rtt_traced(ip) {
        Ok(ms) => return Ok(ProbeOutcome { via: "icmp", ms }),
        Err(why) => reasons.push(format!("icmp {why}")),
    }
    match udp_rtt_traced(ip, port) {
        Ok(ms) => return Ok(ProbeOutcome { via: "raknet", ms }),
        Err(why) => reasons.push(format!("raknet {why}")),
    }
    match tcp_rtt_traced(ip, tcp_ports) {
        Ok(ms) => return Ok(ProbeOutcome { via: "tcp", ms }),
        Err(why) => reasons.push(format!("tcp {why}")),
    }
    Err(ProbeMiss {
        last: "tcp",
        reasons,
    })
}

/// The one `log.txt` line per probe run: the target, the result, and - when there is no
/// number - every tier's reason. Kept next to [`probe_server`] so the log text a support
/// report quotes is the same text the integration test asserts.
pub fn probe_line(
    target: &str,
    rtt: Option<u32>,
    source: Option<&str>,
    misses: &[String],
) -> String {
    match (rtt, source) {
        (Some(ms), Some(source)) => format!("overlay: ping probe {target} -> {ms} ms ({source})"),
        (Some(ms), None) => format!("overlay: ping probe {target} -> {ms} ms"),
        (None, _) => format!(
            "overlay: ping probe {target} -> none ({})",
            misses.join("; ")
        ),
    }
}

/// Where a resolved ping came from: the sources in the order the overlay tries them.
pub const SOURCE_CLIENT_LOG: &str = "client log";
pub const SOURCE_ROOM_LIST: &str = "room list";
pub const SOURCE_REGIONAL_EDGE: &str = "regional edge";

/// One resolved ping and the source that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PingReading {
    pub ms: u32,
    pub source: &'static str,
}

/// The overlay's priority order: the client's own sample, then the room list's ping for this
/// job, then a regional edge handshake, then a direct probe of the server itself. The direct
/// probe is last because Roblox's game relays blackhole it; it is kept for local and
/// non-Roblox servers, where it is the real measurement.
pub fn preferred_ping(
    client_log: Option<u32>,
    room_list: Option<u32>,
    regional: Option<u32>,
    server: Option<ProbeOutcome>,
) -> Option<PingReading> {
    let pick = |ms: u32, source: &'static str| {
        Some(PingReading {
            ms: ms.clamp(1, 9_999),
            source,
        })
    };
    if let Some(ms) = client_log {
        return pick(ms, SOURCE_CLIENT_LOG);
    }
    if let Some(ms) = room_list {
        return pick(ms, SOURCE_ROOM_LIST);
    }
    if let Some(ms) = regional {
        return pick(ms, SOURCE_REGIONAL_EDGE);
    }
    server.map(|outcome| PingReading {
        ms: outcome.ms,
        source: outcome.via,
    })
}

/// Roblox's public game relays live in `128.116.0.0/16` behind a DDoS filter that blackholes
/// unsolicited ICMP, RakNet and TCP 80/443. The direct probe tiers are skipped for them
/// instead of spending each tick on packets that cannot be answered.
pub fn is_roblox_datacenter(ip: &str) -> bool {
    match ip.trim().parse::<std::net::Ipv4Addr>() {
        Ok(v4) => {
            let octets = v4.octets();
            octets[0] == 128 && octets[1] == 116
        }
        Err(_) => false,
    }
}

/// An HTTPS edge in the region the game server was geolocated to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionalEdge {
    pub host: &'static str,
    /// The edge's own region name, for `log.txt`: `ap-southeast-2`.
    pub region: &'static str,
    pub port: u16,
}

/// How long one regional handshake (or its DNS lookup) may take.
const REGIONAL_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Every `s3.<region>.amazonaws.com` edge the city/country table below can name.
const REGIONAL_HOSTS: [(&str, &str); 17] = [
    ("ap-southeast-2", "s3.ap-southeast-2.amazonaws.com"),
    ("ap-southeast-4", "s3.ap-southeast-4.amazonaws.com"),
    ("ap-southeast-1", "s3.ap-southeast-1.amazonaws.com"),
    ("ap-northeast-1", "s3.ap-northeast-1.amazonaws.com"),
    ("ap-northeast-2", "s3.ap-northeast-2.amazonaws.com"),
    ("ap-south-1", "s3.ap-south-1.amazonaws.com"),
    ("ap-east-1", "s3.ap-east-1.amazonaws.com"),
    ("eu-west-1", "s3.eu-west-1.amazonaws.com"),
    ("eu-west-2", "s3.eu-west-2.amazonaws.com"),
    ("eu-central-1", "s3.eu-central-1.amazonaws.com"),
    ("eu-west-3", "s3.eu-west-3.amazonaws.com"),
    ("eu-north-1", "s3.eu-north-1.amazonaws.com"),
    ("sa-east-1", "s3.sa-east-1.amazonaws.com"),
    ("us-east-1", "s3.us-east-1.amazonaws.com"),
    ("us-west-1", "s3.us-west-1.amazonaws.com"),
    ("us-west-2", "s3.us-west-2.amazonaws.com"),
    ("af-south-1", "s3.af-south-1.amazonaws.com"),
];

/// Cities and countries whose nearest edge is known, paired with its AWS region.
const REGIONAL_PLACES: [(&str, &str); 32] = [
    ("sydney", "ap-southeast-2"),
    ("melbourne", "ap-southeast-4"),
    ("brisbane", "ap-southeast-2"),
    ("perth", "ap-southeast-2"),
    ("adelaide", "ap-southeast-2"),
    ("auckland", "ap-southeast-2"),
    ("singapore", "ap-southeast-1"),
    ("tokyo", "ap-northeast-1"),
    ("osaka", "ap-northeast-1"),
    ("seoul", "ap-northeast-2"),
    ("mumbai", "ap-south-1"),
    ("hong kong", "ap-east-1"),
    ("london", "eu-west-2"),
    ("dublin", "eu-west-1"),
    ("frankfurt", "eu-central-1"),
    ("paris", "eu-west-3"),
    ("stockholm", "eu-north-1"),
    ("sao paulo", "sa-east-1"),
    ("são paulo", "sa-east-1"),
    ("cape town", "af-south-1"),
    ("ashburn", "us-east-1"),
    ("washington", "us-east-1"),
    ("new york", "us-east-1"),
    ("dallas", "us-east-1"),
    ("chicago", "us-east-1"),
    ("atlanta", "us-east-1"),
    ("miami", "us-east-1"),
    ("los angeles", "us-west-1"),
    ("san francisco", "us-west-1"),
    ("san jose", "us-west-1"),
    ("seattle", "us-west-2"),
    ("portland", "us-west-2"),
];

/// Country codes whose edge is known but whose cities are not all listed above.
const REGIONAL_COUNTRIES: [(&str, &str); 28] = [
    ("hk", "ap-east-1"),
    ("gb", "eu-west-2"),
    ("uk", "eu-west-2"),
    ("ie", "eu-west-1"),
    ("de", "eu-central-1"),
    ("at", "eu-central-1"),
    ("ch", "eu-central-1"),
    ("pl", "eu-central-1"),
    ("cz", "eu-central-1"),
    ("tr", "eu-central-1"),
    ("fr", "eu-west-3"),
    ("es", "eu-west-3"),
    ("nl", "eu-west-1"),
    ("be", "eu-west-1"),
    ("it", "eu-west-1"),
    ("se", "eu-north-1"),
    ("no", "eu-north-1"),
    ("dk", "eu-north-1"),
    ("fi", "eu-north-1"),
    ("br", "sa-east-1"),
    ("ar", "sa-east-1"),
    ("cl", "sa-east-1"),
    ("us", "us-east-1"),
    ("ca", "us-east-1"),
    ("mx", "us-west-2"),
    ("za", "af-south-1"),
    ("au", "ap-southeast-2"),
    ("nz", "ap-southeast-2"),
];

/// The nearest edge endpoint for a geolocated `City, Region, Country` string.
///
/// The game server itself cannot be pinged once it is behind Roblox's filter, so the edge in
/// the same city/country is measured instead: a handshake to `s3.<region>.amazonaws.com`
/// terminates in the nearest AWS point of presence, which is the same path and roughly the
/// same distance the game traffic takes. `None` for an unknown location, so the HUD names the
/// path that failed instead of inventing a number.
pub fn regional_edge(region: &str) -> Option<RegionalEdge> {
    let lower = region.to_ascii_lowercase();
    if lower.trim().is_empty() || lower.contains("unknown") {
        return None;
    }
    // `Sydney, New South Wales, AU`: the country is the last comma-separated token.
    let country = lower.rsplit(',').next().unwrap_or("").trim();
    let city = REGIONAL_PLACES
        .iter()
        .find(|(keyword, _)| lower.contains(keyword))
        .map(|(_, aws)| *aws);
    let aws = match city {
        Some(aws) => aws,
        None if country.len() == 2 => REGIONAL_COUNTRIES
            .iter()
            .find(|(code, _)| code.eq_ignore_ascii_case(country))
            .map(|(_, aws)| *aws)?,
        None => return None,
    };
    REGIONAL_HOSTS
        .iter()
        .find(|(name, _)| *name == aws)
        .map(|(region, host)| RegionalEdge {
            host,
            region,
            port: 443,
        })
}

/// One TCP handshake to `host:port`, timed. Resolution is bounded too: a hangs-on-DNS lookup
/// must cost one probe tick, not the probe thread.
pub fn tcp_rtt_to_host(host: &str, port: u16) -> Result<u32, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("empty host".to_string());
    }
    let addrs = resolve_bounded(host, port)?;
    let mut last = "no address".to_string();
    for addr in addrs.into_iter().take(4) {
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, REGIONAL_TIMEOUT) {
            Ok(_) => return Ok(start.elapsed().as_millis().clamp(1, 9_999) as u32),
            Err(e) => last = tcp_reason(&e).to_string(),
        }
    }
    Err(last)
}

fn resolve_bounded(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let name = host.to_string();
    std::thread::spawn(move || {
        let addrs = (name.as_str(), port)
            .to_socket_addrs()
            .map(|iter| iter.collect::<Vec<_>>());
        let _ = tx.send(addrs);
    });
    match rx.recv_timeout(REGIONAL_TIMEOUT) {
        Ok(Ok(addrs)) if !addrs.is_empty() => Ok(addrs),
        Ok(Ok(_)) => Err("no address".to_string()),
        Ok(Err(e)) => Err(format!("unresolved host ({e})")),
        Err(_) => Err("dns timeout".to_string()),
    }
}

/// The regional-edge measurement: the endpoint chosen for `region`, and its handshake RTT.
pub fn regional_edge_rtt(region: &str) -> Result<(RegionalEdge, u32), String> {
    let edge = regional_edge(region).ok_or_else(|| format!("no edge for location \"{region}\""))?;
    let ms =
        tcp_rtt_to_host(edge.host, edge.port).map_err(|why| format!("{} {why}", edge.region))?;
    Ok((edge, ms))
}

/// The room-list fallback's paging rules: page one first, at most two pages, and a pause
/// between them so one authenticated client stays well inside Roblox's rate limit.
pub const ROOM_LIST_PAGES: usize = 2;
pub const ROOM_LIST_PAGE_DELAY: Duration = Duration::from_millis(1_500);

/// A `429` is never retried straight away; the next lookup waits this long.
pub const ROOM_LIST_429_BACKOFF: Duration = Duration::from_secs(300);

/// Whether an API error is Roblox's rate limiter, which must never be answered with another
/// request - that is what turns a single `429` into a wall of them.
pub fn is_rate_limited(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("429") || lower.contains("too many requests")
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
