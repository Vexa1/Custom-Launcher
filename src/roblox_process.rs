use crate::api::{enc, post_csrf};
#[cfg(windows)]
use crate::auth::data_dir;
use crate::auth::load_settings;
use crate::types::Msg;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, FALSE, FILETIME, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, TRUE, WPARAM,
};
#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    TerminateProcess,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
};
#[cfg(windows)]
use windows_sys::core::BOOL;
#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::*;

#[cfg(windows)]
pub const CMD_KEY: &str = r"Software\Classes\roblox-player\shell\open\command";

/// Locate the installed Roblox Player executable.
#[cfg(windows)]
pub fn find_player() -> Option<PathBuf> {
    let base = PathBuf::from(env::var("LOCALAPPDATA").ok()?)
        .join("Roblox")
        .join("Versions");
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(base).ok()?.flatten() {
        let exe = entry.path().join("RobloxPlayerBeta.exe");
        if let Some(time) = fs::metadata(&exe).ok().and_then(|m| m.modified().ok()) {
            if best.as_ref().map_or(true, |(t, _)| time > *t) {
                best = Some((time, exe));
            }
        }
    }
    best.map(|(_, path)| path)
}

/// macOS installs Roblox as a single app bundle. Check the usual locations,
/// then look inside `~/Library/Application Support/Roblox/Versions` in case a
/// versioned layout is present.
#[cfg(target_os = "macos")]
pub fn find_player() -> Option<PathBuf> {
    let home = PathBuf::from(env::var("HOME").ok()?);
    let candidates = [
        PathBuf::from("/Applications/Roblox.app"),
        home.join("Applications").join("Roblox.app"),
    ];
    for app in candidates {
        let player = app.join("Contents").join("MacOS").join("RobloxPlayer");
        if player.is_file() {
            return Some(player);
        }
    }
    let versions = home
        .join("Library")
        .join("Application Support")
        .join("Roblox")
        .join("Versions");
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(versions).ok()?.flatten() {
        let player = entry
            .path()
            .join("RobloxPlayer.app")
            .join("Contents")
            .join("MacOS")
            .join("RobloxPlayer");
        if let Some(time) = fs::metadata(&player).ok().and_then(|m| m.modified().ok()) {
            if best.as_ref().is_none_or(|(t, _)| time > *t) {
                best = Some((time, player));
            }
        }
    }
    best.map(|(_, path)| path)
}

/// Directory that holds `ClientSettings/ClientAppSettings.json`.
///
/// Windows uses the active version folder; macOS reads FastFlags from
/// `~/Library/Preferences/com.roblox.Roblox` instead, so there is no version
/// directory to resolve there.
pub fn version_dir_for_player(player: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let _ = player;
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        player.parent().map(Path::to_path_buf)
    }
}

pub fn apply_fps_setting(target_fps: Option<u32>) -> Result<(), String> {
    let cfg = crate::fastflags::load_fastflags();
    crate::fastflags::sync_client_fastflags(&cfg, target_fps, &[])
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(windows)]
#[repr(C)]
struct NtUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process_handle: HANDLE,
        process_information_class: u32,
        process_information: *mut std::ffi::c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

#[cfg(windows)]
fn read_process_cmdline(handle: HANDLE) -> String {
    let mut qwords = vec![0u64; 512];
    let mut ret_len: u32 = 0;
    let mut status = unsafe {
        NtQueryInformationProcess(
            handle,
            60,
            qwords.as_mut_ptr() as *mut std::ffi::c_void,
            (qwords.len() * 8) as u32,
            &mut ret_len,
        )
    };
    if status < 0 && ret_len > (qwords.len() * 8) as u32 && ret_len <= 131_072 {
        qwords.resize((ret_len as usize).div_ceil(8), 0);
        status = unsafe {
            NtQueryInformationProcess(
                handle,
                60,
                qwords.as_mut_ptr() as *mut std::ffi::c_void,
                (qwords.len() * 8) as u32,
                &mut ret_len,
            )
        };
    }
    if status < 0 || qwords.len() * 8 < std::mem::size_of::<NtUnicodeString>() {
        return String::new();
    }
    unsafe {
        let us = &*(qwords.as_ptr() as *const NtUnicodeString);
        let start = qwords.as_ptr() as usize;
        let end = start + qwords.len() * 8;
        let buf_addr = us.buffer as usize;
        let byte_len = us.length as usize;
        if us.buffer.is_null()
            || byte_len == 0
            || byte_len % 2 != 0
            || buf_addr < start
            || buf_addr.saturating_add(byte_len) > end
        {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(us.buffer, byte_len / 2);
        String::from_utf16_lossy(slice)
    }
}

#[cfg(windows)]
fn filetime_to_unix_ms(ft: FILETIME) -> u64 {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    ticks.saturating_sub(116_444_736_000_000_000) / 10_000
}

#[cfg(windows)]
fn read_process_start_ms(handle: HANDLE) -> u64 {
    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut zero = created;
    let ok = unsafe { GetProcessTimes(handle, &mut created, &mut zero, &mut zero, &mut zero) };
    if ok != 0 {
        filetime_to_unix_ms(created)
    } else {
        0
    }
}

pub fn parse_crash_handler_attachment(cmd: &str) -> Option<(u32, PathBuf)> {
    let lower = cmd.to_ascii_lowercase();
    let pid_tag = "--annotation=pid=";
    let pid_pos = lower.find(pid_tag)? + pid_tag.len();
    let pid_digits: String = cmd[pid_pos..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let pid = pid_digits.parse::<u32>().ok()?;

    let att_tag = "--attachment=";
    let att_start = lower.find(att_tag)? + att_tag.len();
    let eq_rel = cmd[att_start..].find('=')?;
    let path_start = att_start + eq_rel + 1;
    let rest = &cmd[path_start..];
    let rest_lower = &lower[path_start..];
    let log_end = rest_lower.find(".log")? + 4;
    let raw = rest[..log_end].trim_matches('"');
    if raw.is_empty() {
        None
    } else {
        Some((pid, PathBuf::from(raw)))
    }
}

#[derive(Clone)]
pub struct RbxProc {
    pub pid: u32,
    pub start_ms: u64,
    pub cmdline: String,
}

#[cfg(windows)]
pub fn enumerate_roblox_processes() -> (Vec<RbxProc>, HashMap<u32, PathBuf>) {
    let mut players = Vec::new();
    let mut crash_logs = HashMap::new();
    let self_pid = std::process::id();
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return (players, crash_logs);
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    if unsafe { Process32FirstW(snap, &mut entry) } != FALSE {
        loop {
            let pid = entry.th32ProcessID;
            if pid != 0 && pid != self_pid {
                let nul = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe = String::from_utf16_lossy(&entry.szExeFile[..nul]);
                let is_player = exe.eq_ignore_ascii_case("RobloxPlayerBeta.exe");
                let is_crash = exe.eq_ignore_ascii_case("RobloxCrashHandler.exe");
                if is_player || is_crash {
                    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
                    if !h.is_null() {
                        let cmdline = read_process_cmdline(h);
                        let start_ms = if is_player {
                            read_process_start_ms(h)
                        } else {
                            0
                        };
                        unsafe { CloseHandle(h) };
                        if is_player {
                            players.push(RbxProc {
                                pid,
                                start_ms,
                                cmdline,
                            });
                        } else if let Some((target_pid, log_path)) =
                            parse_crash_handler_attachment(&cmdline)
                        {
                            crash_logs.insert(target_pid, log_path);
                        }
                    } else if is_player {
                        players.push(RbxProc {
                            pid,
                            start_ms: 0,
                            cmdline: String::new(),
                        });
                    }
                }
            }
            if unsafe { Process32NextW(snap, &mut entry) } == FALSE {
                break;
            }
        }
    }
    unsafe { CloseHandle(snap) };
    (players, crash_logs)
}

#[cfg(windows)]
struct CloseWinCtx {
    pid: u32,
    posted: bool,
}

#[cfg(windows)]
unsafe extern "system" fn enum_close_pid_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let ctx = &mut *(lparam as *mut CloseWinCtx);
        let mut win_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut win_pid);
        if win_pid == ctx.pid && PostMessageW(hwnd, WM_CLOSE, 0 as WPARAM, 0) != FALSE {
            ctx.posted = true;
        }
    }
    TRUE
}

#[cfg(windows)]
pub fn terminate_roblox_process(pid: u32) -> bool {
    let h = unsafe { OpenProcess(PROCESS_TERMINATE, FALSE, pid) };
    if !h.is_null() {
        let ok = unsafe { TerminateProcess(h, 0) } != FALSE;
        unsafe { CloseHandle(h) };
        if ok {
            return true;
        }
    }
    let mut ctx = CloseWinCtx { pid, posted: false };
    unsafe {
        EnumWindows(Some(enum_close_pid_window), (&raw mut ctx) as LPARAM);
    }
    ctx.posted
}

/// macOS process enumeration using libproc (equivalent of the Toolhelp scan).
#[cfg(target_os = "macos")]
const PROC_ALL_PIDS: u32 = 1;

#[cfg(target_os = "macos")]
fn mac_cmdline(pid: u32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size: libc::size_t = 0;
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
    }
    if size == 0 || size > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u8; size];
    let mut got = size;
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut got,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
    }
    buf.truncate(got);
    if buf.len() < 4 {
        return None;
    }
    let argc = i32::from_ne_bytes(buf[0..4].try_into().ok()?) as usize;
    let rest = &buf[4..];
    let exec_end = rest.iter().position(|&b| b == 0)?;
    let mut i = exec_end;
    while i < rest.len() && rest[i] == 0 {
        i += 1;
    }
    let mut args = Vec::new();
    for _ in 0..argc.saturating_sub(1) {
        if i >= rest.len() {
            break;
        }
        let Some(end) = rest[i..].iter().position(|&b| b == 0) else {
            break;
        };
        args.push(String::from_utf8_lossy(&rest[i..i + end]).to_string());
        i += end + 1;
    }
    Some(args.join(" "))
}

#[cfg(target_os = "macos")]
fn mac_process_start_ms(pid: u32) -> u64 {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if n <= 0 {
        return 0;
    }
    info.pbi_start_tvsec
        .saturating_mul(1000)
        .saturating_add(info.pbi_start_tvusec / 1000)
}

#[cfg(target_os = "macos")]
pub fn enumerate_roblox_processes() -> (Vec<RbxProc>, HashMap<u32, PathBuf>) {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut players = Vec::new();
    let mut crash_logs = HashMap::new();
    let self_pid = std::process::id();

    let mut pids: Vec<libc::pid_t> = vec![0; 1024];
    loop {
        let bytes = pids.len() * std::mem::size_of::<libc::pid_t>();
        let n = unsafe {
            libc::proc_listpids(
                PROC_ALL_PIDS,
                0,
                pids.as_mut_ptr().cast(),
                bytes as libc::c_int,
            )
        };
        if n <= 0 {
            return (players, crash_logs);
        }
        let count = n as usize / std::mem::size_of::<libc::pid_t>();
        if count < pids.len() {
            pids.truncate(count);
            break;
        }
        pids.resize(pids.len() * 2, 0);
    }

    let mut buf = [0u8; 4096];
    for pid in pids {
        if pid <= 0 || pid as u32 == self_pid {
            continue;
        }
        let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if len <= 0 {
            continue;
        }
        // Cheap pre-filter on the file name; only allocate for Roblox processes.
        let raw = &buf[..len as usize];
        let base = raw.rsplit(|&b| b == b'/').next().unwrap_or(raw);
        if !base.starts_with(b"Roblox") {
            continue;
        }
        let path = PathBuf::from(OsString::from_vec(raw.to_vec()));
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_player = name == "RobloxPlayer" || name == "RobloxPlayerBeta";
        let is_crash = name == "RobloxCrashHandler";
        if !(is_player || is_crash) {
            continue;
        }
        let cmdline = mac_cmdline(pid as u32).unwrap_or_default();
        if is_player {
            players.push(RbxProc {
                pid: pid as u32,
                start_ms: mac_process_start_ms(pid as u32),
                cmdline,
            });
        } else if let Some((target_pid, log_path)) = parse_crash_handler_attachment(&cmdline) {
            crash_logs.insert(target_pid, log_path);
        }
    }
    (players, crash_logs)
}

#[cfg(target_os = "macos")]
pub fn terminate_roblox_process(pid: u32) -> bool {
    // SIGKILL (not SIGTERM): Roblox ignores / delays polite quit requests, which left its
    // Dock dot and menu-bar icon behind. A hard kill ends the process immediately.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 }
}

/// Force-kills every process that lives inside `Roblox.app` (game, crash handler,
/// installer helpers) so no Dock dot / menu-bar icon is left behind.
/// With `helpers_only` the game executables themselves are spared.
#[cfg(target_os = "macos")]
pub fn force_kill_roblox_bundle(helpers_only: bool) -> usize {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let self_pid = std::process::id();
    let mut pids: Vec<libc::pid_t> = vec![0; 4096];
    let bytes = pids.len() * std::mem::size_of::<libc::pid_t>();
    let n = unsafe {
        libc::proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr().cast(),
            bytes as libc::c_int,
        )
    };
    if n <= 0 {
        return 0;
    }
    pids.truncate(n as usize / std::mem::size_of::<libc::pid_t>());

    let mut buf = [0u8; 4096];
    let mut killed = 0;
    for pid in pids {
        if pid <= 0 || pid as u32 == self_pid {
            continue;
        }
        let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if len <= 0 {
            continue;
        }
        let path = PathBuf::from(OsString::from_vec(buf[..len as usize].to_vec()));
        let text = path.to_string_lossy();
        if !text.contains("/Roblox.app/") && !text.contains("/RobloxPlayerInstaller.app/") {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if helpers_only && (name == "RobloxPlayer" || name == "RobloxPlayerBeta") {
            continue;
        }
        if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
            killed += 1;
        }
    }
    killed
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
        "Joining game '",
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
    let joined = text.rfind("Joining game '");
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
    let idx = text.rfind("Joining game '")?;
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

/// Where the player is connected, parsed from the Roblox log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub ip: String,
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

/// First IP-looking token after one of the markers, e.g. `UDMUX Address = 1.2.3.4,`.
fn ip_after_marker(text: &str, markers: &[&str]) -> Option<String> {
    for marker in markers {
        let Some(p) = text.rfind(marker) else {
            continue;
        };
        let rest = text[p + marker.len()..].trim_start_matches([' ', '=', ':']);
        let end = rest
            .find(|c: char| c == ',' || c.is_whitespace())
            .unwrap_or(rest.len());
        let candidate = &rest[..end];
        if looks_like_ip(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Last address the client says it connected to on a server network line. Used only
/// when the join banner is missing, so the overlay can still ping a real game server.
/// True when a running Roblox player has a *join banner* in its log.
///
/// The client only writes one when a game is really joined, so the banner is proof of a
/// session on its own. The overlay process uses this instead of trusting the launcher's
/// `game_active` flag: that flag needs the place id to parse, and a log whose lifecycle
/// markers are not recognised left it false for the whole session - which is why the HUD
/// could never appear at all, however healthy the rest of the pipeline was.
pub fn session_banner_active() -> bool {
    let (procs, crash_logs) = enumerate_roblox_processes();
    if procs.is_empty() {
        return false;
    }
    let logs_dir = roblox_logs_dir();
    for p in &procs {
        let Some(path) = find_player_log_for_proc(p, &crash_logs, logs_dir.as_deref()) else {
            continue;
        };
        let (state, _, _, server) = inspect_player_log_full(&path);
        // A play state is a session even when no server address could be parsed. The join
        // banner is in the log, but the only network line of that session can carry the
        // private UDMUX relay, and `extract_server_info_from_log` drops private addresses -
        // requiring a public IP hid the HUD for the whole session.
        if matches!(state, LogSessionState::Playing) {
            return true;
        }
        // No banner, no session: sitting in the Roblox menu must not show the HUD.
        if server.is_none() {
            continue;
        }
        if !matches!(
            state,
            LogSessionState::AppAfterPlay | LogSessionState::AppOnly | LogSessionState::TrayOnly
        ) {
            return true;
        }
    }
    false
}

fn ip_from_network_lines(text: &str) -> Option<String> {
    // The spellings the desktop client actually writes, macOS included:
    //   [FLog::Network] serverId: 128.116.1.2|53640          <- the canonical one
    //   [FLog::Network] Connection accepted from 128.116.1.2|53640
    //   [FLog::Network] UDMUX Address = 128.116.1.2, Port = 53640
    //   [FLog::Network] Connected to 128.116.1.2|53640
    // `serverId:` was missing entirely, and that is the line a normal join writes - which
    // is why the server panel could sit on "Server info unavailable" for a whole session.
    const MARKERS: [&str; 6] = [
        "serverid",
        "udmux address",
        "connection accepted from",
        "connected to",
        "connecting to",
        "raknet",
    ];
    // The whole chunk, not the last 600 lines: the connect line is written once when the
    // session starts, and a long session pushes it arbitrarily far back. Lowercased once
    // rather than per line - this runs twice a second while a game is live.
    // Tokens are digits and dots, which lowercasing cannot change, so scanning the lowered
    // copy is enough and costs one allocation instead of thousands.
    let lower = text.to_ascii_lowercase();
    lower.lines().rev().find_map(|line| {
        if !MARKERS.iter().any(|m| line.contains(m)) {
            return None;
        }
        line.split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find(|token| looks_like_ip(token) && !is_private_ip(token))
            .map(str::to_string)
    })
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

/// Finds the newest `Joining game '<job>' place <id> at <ip>` line and, if present,
/// the real server address from the `UDMUX Address = <ip>,` line that follows it.
pub fn extract_server_info_from_log(text: &str) -> Option<ServerInfo> {
    let Some(idx) = text.rfind("Joining game '") else {
        // No join banner in this chunk (old/rotated log, or a format change): still
        // report the address from the network lines so ping and the server panel work.
        return ip_from_network_lines(text).map(|ip| ServerInfo {
            ip,
            job_id: String::new(),
            place_id: 0,
            universe_id: universe_id_from_log(text),
        });
    };
    let tail = &text[idx..];
    let line = tail.lines().next().unwrap_or(tail);
    let job_id = line.split('\'').nth(1).unwrap_or("").trim().to_string();
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
    let join_ip = line
        .rsplit_once(" at ")
        .map(|(_, ip)| ip.trim().to_string())
        .filter(|ip| looks_like_ip(ip));
    // A public address wins over the local UDMUX relay the line sometimes carries.
    let ip = ip_after_marker(tail, &["UDMUX Address =", "UDMUX Address="])
        .filter(|ip| !is_private_ip(ip))
        .or(join_ip)
        .or_else(|| ip_from_network_lines(tail))?;
    Some(ServerInfo {
        ip,
        job_id,
        place_id,
        universe_id: universe_id_from_log(text),
    })
}

pub fn inspect_player_log_session(path: &Path) -> (LogSessionState, bool, Option<u64>) {
    let (state, handoff, place, _) = inspect_player_log_full(path);
    (state, handoff, place)
}

type FullLogInspection = (LogSessionState, bool, Option<u64>, Option<ServerInfo>);

/// Last inspection per log file, valid while the file's size and mtime are unchanged.
/// While a game runs the log is polled constantly but rarely changes, so this avoids
/// re-reading and re-scanning up to 512 KB twice a second.
fn log_inspection_cache()
-> &'static Mutex<HashMap<PathBuf, (u64, Option<SystemTime>, FullLogInspection)>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (u64, Option<SystemTime>, FullLogInspection)>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn inspect_player_log_full(path: &Path) -> FullLogInspection {
    let meta = fs::metadata(path).ok();
    let stamp = meta.as_ref().map(|m| (m.len(), m.modified().ok()));
    if let Some((len, mtime)) = stamp {
        if let Ok(cache) = log_inspection_cache().lock() {
            if let Some((clen, cmtime, result)) = cache.get(path) {
                if *clen == len && *cmtime == mtime {
                    return result.clone();
                }
            }
        }
    }
    let result = inspect_player_log_full_uncached(path);
    if let Some((len, mtime)) = stamp {
        if let Ok(mut cache) = log_inspection_cache().lock() {
            if cache.len() > 16 {
                cache.clear();
            }
            cache.insert(path.to_path_buf(), (len, mtime, result.clone()));
        }
    }
    result
}

fn inspect_player_log_full_uncached(path: &Path) -> FullLogInspection {
    let Ok(mut f) = fs::File::open(path) else {
        return (LogSessionState::Unknown, false, None, None);
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    const MAX_TAIL: u64 = 512 * 1024;
    if len > MAX_TAIL {
        let _ = f.seek(SeekFrom::Start(len - MAX_TAIL));
    }
    let mut buf = Vec::with_capacity(len.min(MAX_TAIL) as usize);
    if f.read_to_end(&mut buf).is_err() {
        return (LogSessionState::Unknown, false, None, None);
    }
    let text = String::from_utf8_lossy(&buf);
    let (mut state, is_handoff) = inspect_player_log_text(&text, len);
    // In a long session every lifecycle marker can scroll out of the 512 KB tail, which
    // made the state look Unknown and ended the session while the game was still open.
    // The join banner at the head of the log proves the client is in a game.
    let mut head_info = None;
    if state == LogSessionState::Unknown {
        head_info = inspect_player_log_head(path);
        if head_info.as_ref().is_some_and(|(place, _)| place.is_some()) {
            state = LogSessionState::Playing;
        }
    }
    // Parse the join banner / connection line regardless of the lifecycle state. The banner
    // is only written when a game is actually joined, so it is trustworthy on its own; gating
    // it on `Playing` meant a log whose FSM markers were not recognised (the normal case for
    // some macOS client builds) left the server panel and the ping empty for the whole
    // session, even though the address was sitting right there in the file.
    let left_game = matches!(
        state,
        LogSessionState::AppAfterPlay | LogSessionState::AppOnly | LogSessionState::TrayOnly
    );
    let (mut place_id, mut server) = if left_game {
        (None, None)
    } else {
        (
            extract_active_place_id_from_log(&text),
            extract_server_info_from_log(&text),
        )
    };
    // The join banner sits at the *start* of the session, while only the tail of the log
    // is read above. In a long session it scrolls out of that window, which left the
    // overlay stuck on "Detecting server..." and the ping empty for the whole game.
    // Fall back to the head of the file, where the join was written.
    if !left_game && (place_id.is_none() || server.is_none()) {
        let head = head_info.take().or_else(|| inspect_player_log_head(path));
        if let Some((head_place, head_server)) = head {
            if place_id.is_none() {
                place_id = head_place;
            }
            if server.is_none() {
                server = head_server;
            }
        }
    }
    (state, is_handoff, place_id, server)
}

/// Parses the first chunk of a player log (max 256 KB), where the join banner lives.
fn inspect_player_log_head(path: &Path) -> Option<(Option<u64>, Option<ServerInfo>)> {
    let mut f = fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(256 * 1024);
    if f.by_ref().take(256 * 1024).read_to_end(&mut buf).is_err() {
        return None;
    }
    let text = String::from_utf8_lossy(&buf);
    Some((
        extract_active_place_id_from_log(&text),
        extract_server_info_from_log(&text),
    ))
}

pub fn inspect_player_log(path: &Path) -> (LogSessionState, bool) {
    let (state, is_handoff, _) = inspect_player_log_session(path);
    (state, is_handoff)
}

#[cfg(windows)]
pub fn roblox_logs_dir() -> Option<PathBuf> {
    Some(
        PathBuf::from(env::var("LOCALAPPDATA").ok()?)
            .join("Roblox")
            .join("logs"),
    )
}

#[cfg(target_os = "macos")]
pub fn roblox_logs_dir() -> Option<PathBuf> {
    Some(
        PathBuf::from(env::var("HOME").ok()?)
            .join("Library")
            .join("Logs")
            .join("Roblox"),
    )
}

pub fn find_player_log_for_proc(
    proc: &RbxProc,
    crash_logs: &HashMap<u32, PathBuf>,
    logs_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(p) = crash_logs.get(&proc.pid) {
        if p.exists() {
            return Some(p.clone());
        }
    }
    let dir = logs_dir?;
    let mut best: Option<(u64, u64, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Windows logs always carry `_Player_`; macOS logs may not, so there we
        // accept any non-crash `_last.log` and score it by timestamp below.
        let player_log = if cfg!(target_os = "macos") {
            name_str.contains("_last.log") && !name_str.contains("CrashHandler")
        } else {
            name_str.contains("_Player_")
                && name_str.ends_with("_last.log")
                && !name_str.contains("CrashHandler")
        };
        if !player_log {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let created_ms = meta
            .created()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        if proc.start_ms > 0 {
            if modified_ms + 2_000 < proc.start_ms {
                continue;
            }
            if created_ms > 0 && created_ms + 4_000 < proc.start_ms {
                continue;
            }
        }
        let dist = if proc.start_ms > 0 && created_ms > 0 {
            created_ms.abs_diff(proc.start_ms)
        } else {
            0
        };
        if best.as_ref().is_none_or(|(b_dist, b_mod, _)| {
            dist < *b_dist || (dist == *b_dist && modified_ms > *b_mod)
        }) {
            best = Some((dist, modified_ms, entry.path()));
        }
    }
    best.map(|(_, _, p)| p)
}

pub fn close_tray_roblox_processes() -> usize {
    let (procs, crash_logs) = enumerate_roblox_processes();
    let logs_dir = roblox_logs_dir();
    let mut closed = 0;
    for p in procs {
        let lower = p.cmdline.to_ascii_lowercase();
        let is_tray_cmd = lower.contains("--launch-to-tray");
        let is_tray_log = if !is_tray_cmd && lower.is_empty() {
            find_player_log_for_proc(&p, &crash_logs, logs_dir.as_deref())
                .is_some_and(|log| inspect_player_log(&log).0 == LogSessionState::TrayOnly)
        } else {
            false
        };
        if (is_tray_cmd || is_tray_log) && terminate_roblox_process(p.pid) {
            closed += 1;
        }
    }
    closed
}

pub fn kill_all_roblox_processes() -> usize {
    let (procs, _) = enumerate_roblox_processes();
    let mut closed = 0;
    for p in procs {
        if terminate_roblox_process(p.pid) {
            closed += 1;
        }
    }
    #[cfg(target_os = "macos")]
    {
        closed += force_kill_roblox_bundle(false);
    }
    closed
}

pub fn start_roblox_watcher(
    tx: Sender<Msg>,
    ctx: egui::Context,
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    last_launch_ms: Arc<AtomicU64>,
) {
    thread::spawn(move || {
        // Multi-instance mode never terminates Roblox processes: a second account's copy
        // looks exactly like the background process the cleanup sweep would close.
        let multi_instance = load_settings().multi_instance;
        let mut seen_app_since: HashMap<u32, Instant> = HashMap::new();
        // Wall-clock age for processes whose start time could not be read from libproc.
        let mut first_seen: HashMap<u32, Instant> = HashMap::new();
        // Processes the log has already shown inside a game at least once. Once a game
        // ended, the app/home screen that follows is safe to close immediately.
        let mut ever_playing: HashSet<u32> = HashSet::new();
        let mut recently_closed: HashMap<u32, Instant> = HashMap::new();
        let mut reported_pids: HashSet<u32> = HashSet::new();
        let mut notified_kill_pids: HashSet<u32> = HashSet::new();
        let mut last_sent_session: Option<Option<u64>> = None;
        let mut last_sent_server: Option<Option<ServerInfo>> = None;
        // Whether any Roblox process exists at all. The overlay window uses this so it can
        // stay alive (and keep receiving tray/settings changes) even before the player log
        // tells us which place was joined.
        let mut last_sent_procs: Option<bool> = None;
        while !stop.load(Ordering::Relaxed) {
            let auto_close = enabled.load(Ordering::Relaxed) && !multi_instance;
            if !auto_close {
                seen_app_since.clear();
            }
            let (procs, crash_logs) = enumerate_roblox_processes();
            if procs.is_empty() {
                seen_app_since.clear();
                first_seen.clear();
                ever_playing.clear();
                reported_pids.clear();
                notified_kill_pids.clear();
                recently_closed.retain(|_, t| t.elapsed() < Duration::from_secs(10));
                if last_sent_session != Some(None) {
                    last_sent_session = Some(None);
                    let _ = tx.send(Msg::RobloxSessionUpdated(None));
                    ctx.request_repaint();
                }
                if last_sent_server != Some(None) {
                    last_sent_server = Some(None);
                    let _ = tx.send(Msg::RobloxServerUpdated(None));
                    ctx.request_repaint();
                }
                if last_sent_procs != Some(false) {
                    last_sent_procs = Some(false);
                    let _ = tx.send(Msg::RobloxProcesses(false));
                    ctx.request_repaint();
                }
                // Nothing is running: poll once a second to keep idle CPU low. A launch
                // made from CREBLOX itself (or a session that just ended) keeps the brisk
                // cadence, so the overlay still appears within a fraction of a second.
                let launch_age_ms =
                    now_unix_ms().saturating_sub(last_launch_ms.load(Ordering::Relaxed));
                let idle = launch_age_ms > 20_000;
                thread::sleep(Duration::from_millis(if idle { 1_000 } else { 300 }));
                continue;
            }

            let now_ms = now_unix_ms();
            let launch_age_ms = now_ms.saturating_sub(last_launch_ms.load(Ordering::Relaxed));
            let active_pids: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
            seen_app_since.retain(|pid, _| active_pids.contains(pid));
            first_seen.retain(|pid, _| active_pids.contains(pid));
            ever_playing.retain(|pid| active_pids.contains(pid));
            reported_pids.retain(|pid| active_pids.contains(pid));
            notified_kill_pids.retain(|pid| active_pids.contains(pid));
            recently_closed.retain(|pid, t| {
                active_pids.contains(pid) && t.elapsed() < Duration::from_secs(10)
            });

            let logs_dir = roblox_logs_dir();
            let mut active_playing_place: Option<u64> = None;
            let mut active_server: Option<ServerInfo> = None;
            // The server of a session that is actually `Playing`, which is what a hop may
            // leave. `active_server` is broader on purpose (the overlay wants an address
            // even in an unrecognised state), so the two are tracked separately.
            let mut active_playing_server: Option<ServerInfo> = None;
            let mut closed_any = false;
            for p in procs {
                if recently_closed.contains_key(&p.pid) {
                    continue;
                }
                let lower = p.cmdline.to_ascii_lowercase();
                let first_seen_at = *first_seen.entry(p.pid).or_insert_with(Instant::now);
                let proc_age_ms = if p.start_ms > 0 {
                    now_ms.saturating_sub(p.start_ms)
                } else {
                    u64::try_from(first_seen_at.elapsed().as_millis()).unwrap_or(u64::MAX)
                };

                // Always close lingering background tray process when auto_close is enabled
                if auto_close && lower.contains("--launch-to-tray") {
                    if terminate_roblox_process(p.pid) {
                        recently_closed.insert(p.pid, Instant::now());
                        if notified_kill_pids.insert(p.pid) {
                            let _ = tx.send(Msg::RobloxBackgroundKilled { pid: p.pid });
                            ctx.request_repaint();
                        }
                    }
                    continue;
                }

                let full_session = find_player_log_for_proc(&p, &crash_logs, logs_dir.as_deref())
                    .map(|log| inspect_player_log_full(&log));
                let log_session = full_session
                    .as_ref()
                    .map(|(st, handoff, pid, _)| (*st, *handoff, *pid));
                let log_state = log_session.map(|(st, handoff, _)| (st, handoff));

                let explicit_app = !lower.is_empty()
                    && (lower.contains("--app") || lower.contains("launchmode:app"))
                    && !lower.contains("launchmode:play")
                    && !lower.contains("placelauncher");
                // A process launched for a game (site Play button, private server, deep link)
                // is protected while the game may still be starting: Roblox can spend a
                // while on the app/home screen before the first join marker reaches the
                // log, and killing it there would look like a crash. That protection must
                // not last for the whole process lifetime - it used to, so a game-launched
                // Roblox was never auto-closed after the player left the game.
                let launched_game = lower.contains("launchmode:play")
                    || lower.contains("placelauncher")
                    || lower.contains("roblox-player:");
                let is_playing = matches!(log_state, Some((LogSessionState::Playing, _)));
                if is_playing {
                    ever_playing.insert(p.pid);
                    active_playing_server = full_session.as_ref().and_then(|f| f.3.clone());
                }
                let launch_may_be_starting = launched_game
                    && proc_age_ms < 90_000
                    && !ever_playing.contains(&p.pid)
                    && !matches!(log_state, Some((LogSessionState::AppAfterPlay, _)));

                // Adopt the parsed address whenever the log does not show the player
                // having left. `Unknown` used to mean "no server at all", which is exactly
                // why the panel and the ping stayed empty on logs whose lifecycle markers
                // are not recognised.
                let left_game = matches!(
                    log_state,
                    Some((
                        LogSessionState::AppAfterPlay
                            | LogSessionState::AppOnly
                            | LogSessionState::TrayOnly,
                        _
                    ))
                );
                if left_game {
                    active_server = None;
                } else if let Some(server) = full_session.as_ref().and_then(|f| f.3.clone()) {
                    active_server = Some(server);
                }
                if is_playing {
                    let parsed_pid = log_session.and_then(|(_, _, pid)| pid);
                    active_playing_place = Some(match (active_playing_place, parsed_pid) {
                        (_, Some(pid)) if pid > 0 => pid,
                        (Some(prev), _) => prev,
                        _ => 0,
                    });
                }

                let should_close = if auto_close {
                    match log_state {
                        Some((LogSessionState::AppAfterPlay, _)) => {
                            // Roblox wrote `returnToLuaApp` / `Entered app session.`: the game
                            // is over, so this process is safe to close no matter how it was
                            // started. This is the path that used to be blocked forever by
                            // the game-launch guard, leaving Roblox open long after leaving
                            // and keeping CREBLOX hidden behind it.
                            let first = seen_app_since.entry(p.pid).or_insert_with(Instant::now);
                            first.elapsed() >= Duration::from_millis(700)
                        }
                        Some((LogSessionState::TrayOnly, _)) => proc_age_ms >= 2_000,
                        Some((LogSessionState::AppOnly, false)) => {
                            if launch_may_be_starting {
                                seen_app_since.remove(&p.pid);
                                false
                            } else if launch_age_ms >= 15_000 || explicit_app || launched_game {
                                let first =
                                    seen_app_since.entry(p.pid).or_insert_with(Instant::now);
                                first.elapsed() >= Duration::from_millis(2_500)
                            } else {
                                seen_app_since.remove(&p.pid);
                                false
                            }
                        }
                        Some((LogSessionState::Playing, _)) => {
                            seen_app_since.remove(&p.pid);
                            false
                        }
                        _ => {
                            // Unknown log state: only close a process we can positively
                            // identify as the Roblox app window (`--app` / `launchmode:app`).
                            // A game that just launched, or one whose log we cannot read,
                            // looks exactly like this — killing it presents as a crash.
                            if explicit_app && proc_age_ms >= 5_000 {
                                let first =
                                    seen_app_since.entry(p.pid).or_insert_with(Instant::now);
                                first.elapsed() >= Duration::from_millis(3_000)
                            } else {
                                false
                            }
                        }
                    }
                } else {
                    false
                };

                if should_close {
                    closed_any = true;
                    seen_app_since.remove(&p.pid);
                    reported_pids.remove(&p.pid);
                    crate::auth::log(&format!(
                        "watcher: closing Roblox pid {} after leaving the game (state {:?})",
                        p.pid,
                        log_state.map(|(state, _)| state)
                    ));
                    if terminate_roblox_process(p.pid) {
                        recently_closed.insert(p.pid, Instant::now());
                        if notified_kill_pids.insert(p.pid) {
                            let _ = tx.send(Msg::RobloxBackgroundKilled { pid: p.pid });
                            let _ = tx.send(Msg::RobloxAppClosed);
                            ctx.request_repaint();
                        }
                    }
                } else if !auto_close && !is_playing && reported_pids.insert(p.pid) {
                    let _ = tx.send(Msg::RobloxBackgroundDetected {
                        pid: p.pid,
                        is_playing,
                    });
                    ctx.request_repaint();
                }
            }
            // After closing a finished session, sweep leftover Roblox helper processes so the
            // Dock dot / menu-bar icon disappear. Never while a game is running or just launched.
            #[cfg(target_os = "macos")]
            {
                if closed_any && active_playing_place.is_none() && launch_age_ms >= 15_000 {
                    // Helpers only: a game process must never be swept up here.
                    let _ = force_kill_roblox_bundle(true);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = closed_any;
            // Publish the session to the hop engine. Only a live `Playing` session with both
            // the place and the job known can be hopped; leaving the game clears it, which
            // is what grays the tray submenu out again.
            let hop_context = active_playing_server
                .filter(|s| s.place_id > 0 && !s.job_id.is_empty())
                .map(|s| crate::server_hop::HopContext {
                    place_id: s.place_id,
                    job_id: s.job_id,
                    universe_id: s.universe_id,
                });
            if crate::server_hop::set_context(hop_context) {
                let _ = tx.send(Msg::ServerHopAvailability);
                ctx.request_repaint();
            }
            if last_sent_server.as_ref() != Some(&active_server) {
                // One line per change. With no server this is the only place that separates
                // "the player log was read and had no address in it" from "CREBLOX never
                // found the log", which is otherwise invisible from log.txt.
                match &active_server {
                    Some(s) => crate::auth::log(&format!(
                        // No address: this line ends up in bug reports.
                        "watcher: server place {} (playing place {:?})",
                        s.place_id, active_playing_place
                    )),
                    None => crate::auth::log(&format!(
                        "watcher: no server in the player log (playing place {:?})",
                        active_playing_place
                    )),
                }
                last_sent_server = Some(active_server.clone());
                let _ = tx.send(Msg::RobloxServerUpdated(active_server));
                ctx.request_repaint();
            }
            if last_sent_procs != Some(true) {
                last_sent_procs = Some(true);
                let _ = tx.send(Msg::RobloxProcesses(true));
                ctx.request_repaint();
            }
            if last_sent_session != Some(active_playing_place) {
                last_sent_session = Some(active_playing_place);
                let _ = tx.send(Msg::RobloxSessionUpdated(active_playing_place));
                ctx.request_repaint();
            }
            thread::sleep(Duration::from_millis(if auto_close { 300 } else { 500 }));
        }
    });
}

pub fn launch_from_uri(uri: &str, preclean_tray: bool) -> Result<(), String> {
    let settings = load_settings();
    let _ = apply_fps_setting(settings.target_fps);
    if preclean_tray
        && settings.auto_close_roblox
        && !settings.multi_instance
        && close_tray_roblox_processes() > 0
    {
        thread::sleep(Duration::from_millis(100));
    }
    let player =
        find_player().ok_or("Roblox is not installed. Install it once from roblox.com first.")?;
    spawn_player(&player, uri, settings.multi_instance).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn spawn_player(player: &Path, uri: &str, multi_instance: bool) -> std::io::Result<()> {
    // Hand the roblox-player: URL to the official Roblox app through
    // LaunchServices. Target the app explicitly when possible so a browser
    // that grabbed the URL scheme cannot intercept the launch.
    //
    // `open -n` asks LaunchServices for a second copy of the app instead of raising the
    // running one, which is what makes a second account possible.
    let discovered = player
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf);
    let app = discovered.unwrap_or_else(|| PathBuf::from("/Applications/Roblox.app"));
    if app.exists() {
        let mut cmd = Command::new("/usr/bin/open");
        if multi_instance {
            cmd.arg("-n");
        }
        let ok = cmd
            .arg("-a")
            .arg(&app)
            .arg(uri)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
    }
    let mut cmd = Command::new("/usr/bin/open");
    if multi_instance {
        cmd.arg("-n");
    }
    cmd.arg(uri).spawn().map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn spawn_player(player: &Path, uri: &str, _multi_instance: bool) -> std::io::Result<()> {
    // Windows already allows a new player process per spawn; Roblox's own singleton
    // check is inside the client, so there is nothing else to do here.
    Command::new(player).arg(uri).spawn().map(|_| ())
}

pub fn build_place_launcher_url(
    place_id: u64,
    job_id: Option<&str>,
    private_server_code: Option<&str>,
) -> String {
    if let Some(code) = private_server_code.map(str::trim).filter(|s| !s.is_empty()) {
        return format!(
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestPrivateGame&browserTrackerId=0&placeId={place_id}&linkCode={}&isPlayTogetherGame=false",
            enc(code)
        );
    }
    if let Some(jid) = job_id.map(str::trim).filter(|s| !s.is_empty()) {
        return format!(
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGameJob&browserTrackerId=0&placeId={place_id}&gameId={jid}&isPlayTogetherGame=false"
        );
    }
    format!(
        "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame&browserTrackerId=0&placeId={place_id}&isPlayTogetherGame=false"
    )
}

pub fn launch_authenticated(place_id: u64, cookie: &str) -> Result<String, String> {
    launch_authenticated_target(place_id, None, None, cookie)
}

pub fn launch_authenticated_server(
    place_id: u64,
    job_id: Option<&str>,
    cookie: &str,
) -> Result<String, String> {
    launch_authenticated_target(place_id, job_id, None, cookie)
}

pub fn launch_authenticated_private_server(
    place_id: u64,
    link_code: &str,
    cookie: &str,
) -> Result<String, String> {
    launch_authenticated_target(place_id, None, Some(link_code), cookie)
}

fn launch_authenticated_target(
    place_id: u64,
    job_id: Option<&str>,
    private_server_code: Option<&str>,
    cookie: &str,
) -> Result<String, String> {
    let resp = post_csrf(
        "https://auth.roblox.com/v1/authentication-ticket",
        serde_json::json!({}),
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    let ticket = resp
        .header("rbx-authentication-ticket")
        .ok_or_else(|| "Session expired — please sign in again".to_string())?;
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let launcher_url = build_place_launcher_url(place_id, job_id, private_server_code);
    let uri = format!(
        "roblox-player:1+launchmode:play+gameinfo:{ticket}+launchtime:{ms}+placelauncherurl:{}+robloxLocale:en_us+gameLocale:en_us",
        enc(&launcher_url)
    );
    let _ = crate::fastflags::sync_saved_fastflags_before_launch();
    launch_from_uri(&uri, true)?;
    Ok(
        if private_server_code.is_some_and(|s| !s.trim().is_empty()) {
            "Joining private server...".to_string()
        } else if job_id.is_some_and(|s| !s.trim().is_empty()) {
            "Joining selected server...".to_string()
        } else {
            "Launching Roblox (signed in)...".to_string()
        },
    )
}

#[cfg(windows)]
pub fn handler_status() -> String {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(CMD_KEY, KEY_READ) else {
        return "Not registered (Roblox not installed)".to_string();
    };
    let Ok(current) = key.get_value::<String, _>("") else {
        return "Unknown".to_string();
    };
    if let Ok(me) = env::current_exe() {
        let mine = format!("\"{}\" \"%1\"", me.display());
        if current.eq_ignore_ascii_case(&mine) {
            return "creblox (this app)".to_string();
        }
    }
    if current.to_ascii_lowercase().contains("creblox") {
        "creblox (another path)".to_string()
    } else {
        "Default Roblox launcher".to_string()
    }
}

#[cfg(windows)]
pub fn register() -> Result<String, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(CMD_KEY, KEY_READ | KEY_WRITE)
        .map_err(|e| format!("Roblox handler not found: {e}"))?;
    let current: String = key.get_value("").map_err(|e| e.to_string())?;
    let me = env::current_exe().map_err(|e| e.to_string())?;
    let mine = format!("\"{}\" \"%1\"", me.display());
    if current == mine {
        return Ok("Already registered".to_string());
    }
    fs::write(data_dir().join("original_command.txt"), &current).map_err(|e| e.to_string())?;
    key.set_value("", &mine).map_err(|e| e.to_string())?;
    Ok("The launcher now opens Roblox links".to_string())
}

#[cfg(windows)]
pub fn restore() -> Result<String, String> {
    let original = fs::read_to_string(data_dir().join("original_command.txt"))
        .map_err(|_| "No backup found".to_string())?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(CMD_KEY, KEY_WRITE)
        .map_err(|e| e.to_string())?;
    key.set_value("", &original).map_err(|e| e.to_string())?;
    Ok("Original Roblox launcher restored".to_string())
}

// macOS registers the `roblox-player` URL scheme through the application
// bundle's Info.plist and LaunchServices, so there is no registry equivalent
// to rewrite. These keep the same API for the settings screen.
#[cfg(target_os = "macos")]
pub fn handler_status() -> String {
    match find_player() {
        Some(_) => "Roblox app (macOS default)".to_string(),
        None => "Not installed (install Roblox first)".to_string(),
    }
}

#[cfg(target_os = "macos")]
pub fn register() -> Result<String, String> {
    Ok(
        "macOS opens roblox-player links with the installed Roblox app, so no change is needed."
            .to_string(),
    )
}

#[cfg(target_os = "macos")]
pub fn restore() -> Result<String, String> {
    Ok("macOS manages Roblox links itself; there is no launcher override to restore.".to_string())
}

/// Same as [`enumerate_roblox_processes`] but reuses a scan younger than one second.
/// For UI code that runs every frame (a full process scan per frame is expensive).
pub fn enumerate_roblox_processes_cached() -> (Vec<RbxProc>, HashMap<u32, PathBuf>) {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    type Cache = Option<(Instant, Vec<RbxProc>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, procs)) = guard.as_ref() {
            // UI code asks for this every frame; spawning `ps` once a second was
            // measurable on slow Macs.
            if at.elapsed() < Duration::from_millis(3000) {
                return (procs.clone(), HashMap::new());
            }
        }
    }
    let (procs, logs) = enumerate_roblox_processes();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), procs.clone()));
    }
    (procs, logs)
}
