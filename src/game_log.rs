//! The Roblox client log -> the game server the player is connected to.
//!
//! Pure text parsing and log-file selection: no filesystem, no processes, no platform APIs,
//! so the VM test harness includes this file directly and runs the cases in
//! `vm-tests/tests/roblox_log.rs`.
//!
//! The overlay pings and geolocates the address found here, which is why every spelling the
//! client has shipped is accepted: a missed format is a HUD stuck on "Server info unavailable".

/// Where the player is connected, parsed from the Roblox log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub ip: String,
    /// RakNet port from the same log line, `0` when the line carries none.
    pub port: u16,
    pub job_id: String,
    pub place_id: u64,
    /// `universeId` when the log carries one, `0` otherwise. Server Hop only needs the
    /// place, so a missing universe id never blocks a hop.
    pub universe_id: u64,
}

/// `universeId: 123` / `UniverseId = 123`, when the client writes it at all. Not every
/// build does, and the join banner never includes it.
fn universe_id_from_log(text: &str) -> u64 {
    let lower = text.to_ascii_lowercase();
    let Some(p) = lower.rfind("universeid") else {
        return 0;
    };
    let rest = text[p + "universeid".len()..].trim_start_matches([' ', '=', ':', '"']);
    rest.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn looks_like_ip(s: &str) -> bool {
    s.parse::<std::net::IpAddr>().is_ok()
}

/// First public address on a network line, with its port when the line carries one.
///
/// Handles every spelling the client has shipped:
///   `128.116.45.33:63976`   `128.116.45.33|63976`   `[10.12.2.219]:63976`
///   `UDMUX Address = 128.116.45.33, Port = 63976`
///
/// Private addresses are skipped: the RCC server in these lines is always a `10.x`
/// datacenter address that can neither be geolocated nor pinged from a home network.
fn public_addr_in_line(line: &str) -> Option<(String, u16)> {
    for token in line.split(|c: char| {
        c.is_whitespace()
            || c == ','
            || c == '|'
            || c == ':'
            || c == '='
            || c == '['
            || c == ']'
    }) {
        if !looks_like_ip(token) || is_private_ip(token) {
            continue;
        }
        return Some((token.to_string(), port_for(line, token)));
    }
    None
}

/// The port written next to `ip`: `ip:port`, `ip|port`, `[ip]:port`, or `Port = N`.
fn port_for(line: &str, ip: &str) -> u16 {
    if let Some(pos) = line.find(ip) {
        let rest = line[pos + ip.len()..].trim_start_matches(']');
        let rest = rest.trim_start_matches([':', '|']);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(port) = digits.parse::<u16>() {
            if port > 0 {
                return port;
            }
        }
    }
    let lower = line.to_ascii_lowercase();
    let Some(pos) = lower.find("port") else {
        return 0;
    };
    lower[pos + "port".len()..]
        .trim_start_matches([' ', '=', ':'])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn addr_from_network_lines(text: &str) -> Option<(String, u16)> {
    // The spellings the desktop client actually writes, macOS included:
    //   [FLog::Network] serverId: 128.116.1.2|53640          <- the canonical one
    //   [FLog::Output] Connection accepted from 128.116.1.2|53640
    //   [FLog::Network] UDMUX Address = 128.116.1.2, Port = 53640
    //   [FLog::Output] Connecting to UDMUX server 128.116.1.2:53640, and RCC server 10.x
    //   [FLog::Output] Connecting to 128.116.1.2:53640
    const MARKERS: [&str; 7] = [
        "serverid",
        "udmux address",
        "udmux server",
        "connection accepted from",
        "connected to",
        "connecting to",
        "raknet",
    ];
    // The whole chunk, not the last 600 lines: the connect line is written once when the
    // session starts, and a long session pushes it arbitrarily far back. Lowercased once
    // rather than per line - this runs twice a second while a game is live. Digits, dots
    // and ports survive lowercasing, so scanning the lowered copy is enough.
    let lower = text.to_ascii_lowercase();
    lower.lines().rev().find_map(|line| {
        if !MARKERS.iter().any(|m| line.contains(m)) {
            return None;
        }
        public_addr_in_line(line)
    })
}

/// The UDMUX address the client writes right after the join banner. That public relay is
/// the address the game client actually talks to; the RCC address beside it is private.
fn udmux_addr_after(text: &str) -> Option<(String, u16)> {
    let lower = text.to_ascii_lowercase();
    // The last one wins: a rejoin can write another UDMUX line without a new banner.
    lower
        .lines()
        .filter(|line| line.contains("udmux address") || line.contains("udmux server"))
        .rev()
        .find_map(public_addr_in_line)
}

/// LAN / loopback / link-local addresses. The client writes its local UDMUX relay address
/// on some network lines; resolving that with GeoIP gives "Unknown location" and a ping to
/// nothing, so such candidates are skipped in favour of the real public server address.
fn is_private_ip(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback() || v6.is_unspecified(),
        Err(_) => true,
    }
}

/// The newest join banner (`Joining game <job> place <id> at <address>`, quoted or not)
/// plus the public UDMUX/relay address that follows it. Without a banner, the address on
/// the newest network line is used, so a rotated log still yields a server to ping.
pub fn extract_server_info_from_log(text: &str) -> Option<ServerInfo> {
    // The banner has shipped both with the job id quoted and without:
    //   Joining game '6f1c...' place 1818 at [10.12.2.219]:63976
    //   Joining game 82a6e115-... place 10694573182 at [10.12.2.219]:63976
    let Some(idx) = text.rfind("Joining game ") else {
        // No join banner in this chunk (old/rotated log, or a format change): still
        // report the address from the network lines so ping and the server panel work.
        return addr_from_network_lines(text).map(|(ip, port)| ServerInfo {
            ip,
            port,
            job_id: String::new(),
            place_id: 0,
            universe_id: universe_id_from_log(text),
        });
    };
    let tail = &text[idx + "Joining game ".len()..];
    let line = tail.lines().next().unwrap_or(tail);
    let job_id = line
        .trim_start()
        .trim_start_matches('\'')
        .split(|c: char| c == '\'' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .to_string();
    let place_id: u64 = line
        .split(" place ")
        .nth(1)
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0);
    // The UDMUX line after the banner is the public relay in front of the game server. The
    // banner itself can carry the private RCC address (`at [10.12.2.219]:63976`), which is
    // skipped here in favour of the public address on the network lines.
    let (ip, port) = udmux_addr_after(tail)
        .or_else(|| {
            line.rsplit_once(" at ")
                .and_then(|(_, addr)| public_addr_in_line(addr))
        })
        .or_else(|| addr_from_network_lines(tail))?;
    Some(ServerInfo {
        ip,
        port,
        job_id,
        place_id,
        universe_id: universe_id_from_log(text),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSessionState {
    TrayOnly,
    Playing,
    AppAfterPlay,
    AppOnly,
    Unknown,
}

fn last_pattern_offset(text: &str, patterns: &[&str]) -> Option<usize> {
    patterns.iter().filter_map(|p| text.rfind(p)).max()
}

pub fn inspect_player_log_text(text: &str, total_len: u64) -> (LogSessionState, bool) {
    let is_handoff = text.contains("waitForNewPlayerProcess");

    let play_markers = [
        "[FLog::SessionTransitionFSM] Entered play session.",
        "[FLog::SingleSurfaceApp] setStage: (stage:UGCGame)",
        "[FLog::SingleSurfaceApp] launchUGCGame:",
        "[FLog::SessionTransitionFSM] Teleported.",
        // The join banner is written only when a game is actually joined. Treating it as
        // a play marker keeps session detection (tray, overlay, playtime) working even
        // if Roblox renames the FSM lines above.
        "Joining game ",
    ];
    let app_markers = [
        "[FLog::SessionTransitionFSM] Entered app session.",
        "[FLog::SingleSurfaceApp] setStage: (stage:LuaApp)",
        // Roblox does not always follow this line with the FSM/app-stage markers. When it
        // is the last lifecycle line in the log, treating it as a play marker only left the
        // state on `Playing` after the player pressed Leave.
        "[FLog::SingleSurfaceApp] returnToLuaApp: (stage:UGCGame)",
    ];
    let last_play = last_pattern_offset(text, &play_markers);
    let last_app = last_pattern_offset(text, &app_markers);
    // A join banner that no explicit leave follows is a live session, whatever the app-stage
    // markers say. On macOS the client writes the LuaApp stage while the loading screen is
    // up, so an app marker can be newer than the last play marker *while playing*; the state
    // then read "played, then left", and `left_game` threw away the server address that was
    // sitting in the same log - which is what left the panel on "Server info unavailable"
    // and the ping on `--` for a whole session.
    let joined = text.rfind("Joining game ");
    let left = last_pattern_offset(text, &["returnToLuaApp", "Entered app session."]);
    if joined.is_some_and(|join| left.is_none_or(|leave| join > leave)) {
        return (LogSessionState::Playing, is_handoff);
    }

    let has_play = last_play.is_some()
        || text.contains("returnToLuaApp: (stage:UGCGame)")
        || total_len > 32_768;

    let state = if let Some(app_pos) = last_app {
        if last_play.is_none_or(|play_pos| app_pos > play_pos) {
            if has_play {
                LogSessionState::AppAfterPlay
            } else {
                LogSessionState::AppOnly
            }
        } else {
            LogSessionState::Playing
        }
    } else if last_play.is_some() {
        LogSessionState::Playing
    } else if text.contains("AppState/TrayMode") && !text.contains("User open main window") {
        LogSessionState::TrayOnly
    } else {
        LogSessionState::Unknown
    };
    (state, is_handoff)
}

pub fn extract_active_place_id_from_log(text: &str) -> Option<u64> {
    // Match the join banner with or without the leading "! ": Roblox has shipped both.
    let idx = text.rfind("Joining game ")?;
    let line = &text[idx..];
    let first_line = line.lines().next().unwrap_or(line);
    let place_kw = first_line.find(" place ")?;
    let after_place = &first_line[place_kw + " place ".len()..];
    let digits: String = after_place
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse::<u64>()
        .ok()
        .filter(|&id| id > 0)
        .or_else(|| place_id_from_marker(text))
}

/// `placeId:1818` / `PlaceId: 1818` - written by the join pipeline and by several macOS
/// builds that do not emit the `! Joining game ... place N` banner at all.
fn place_id_from_marker(text: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let p = lower.rfind("placeid")?;
    let rest = text[p + "placeid".len()..].trim_start_matches([' ', '=', ':', '"']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u64>().ok().filter(|&id| id > 0)
}

/// One player log in a Roblox logs directory, with the timestamps that identify the session
/// it belongs to. The caller fills these from a directory listing; everything below is pure
/// so it is testable without a Roblox install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    pub path: std::path::PathBuf,
    pub created_ms: u64,
    pub modified_ms: u64,
}

/// Whether a file name is a player log rather than a crash handler's log.
///
/// Windows logs always carry `_Player_`; macOS logs may not, so there any non-crash
/// `_last.log` counts and the timestamp scoring below tells the sessions apart.
pub fn is_player_log_name(name: &str, macos: bool) -> bool {
    if name.contains("CrashHandler") {
        return false;
    }
    if macos {
        name.contains("_last.log")
    } else {
        name.contains("_Player_") && name.ends_with("_last.log")
    }
}

/// The player log written by the process that started at `start_ms`.
///
/// The closest creation time wins; a tie goes to the most recently written file. Logs that
/// predate the process (beyond the client's start-up slack) belong to an earlier session.
pub fn pick_player_log(files: &[LogFile], start_ms: u64) -> Option<&std::path::Path> {
    let mut best: Option<(u64, u64, &std::path::Path)> = None;
    for f in files {
        if start_ms > 0 {
            if f.modified_ms + 2_000 < start_ms {
                continue;
            }
            if f.created_ms > 0 && f.created_ms + 4_000 < start_ms {
                continue;
            }
        }
        let dist = if start_ms > 0 && f.created_ms > 0 {
            f.created_ms.abs_diff(start_ms)
        } else {
            0
        };
        if best.as_ref().is_none_or(|(b_dist, b_mod, _)| {
            dist < *b_dist || (dist == *b_dist && f.modified_ms > *b_mod)
        }) {
            best = Some((dist, f.modified_ms, f.path.as_path()));
        }
    }
    best.map(|(_, _, path)| path)
}
