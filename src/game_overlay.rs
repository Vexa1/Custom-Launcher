//! In-game overlays drawn on top of the Roblox game window (also in fullscreen):
//! server region, game info, live stats (CPU / RAM / ping / session timer / clock / FPS cap),
//! a view of the active FastFlags and a crosshair.
//!
//! The overlays are drawn by a second copy of this app running in `--overlay-window` mode
//! (see [`crate::overlay_window`]): a transparent, click-through window kept on top of the
//! Roblox window. This module feeds it a JSON spec about once a second from a background
//! thread, so it works the same whether it is driven by the launcher window or by the
//! headless companion.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::api::{games_by_places, lookup_server_location};
use crate::auth::data_dir;
use crate::fastflags::{FastFlagsConfig, build_client_app_settings};
use crate::roblox_process::enumerate_roblox_processes;
use crate::types::Settings;

pub const OVERLAY_WINDOW_FLAG: &str = "--overlay-window";
const NO_PING: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct ServerPanelCfg {
    pub corner: u8,
    /// 0 keeps the panel until the game closes.
    pub duration_secs: u32,
    pub show_ip: bool,
    pub show_job_id: bool,
}

#[derive(Clone, Debug)]
pub struct StatsPanelCfg {
    pub corner: u8,
    pub cpu: bool,
    pub ram: bool,
    pub ping: bool,
    pub session: bool,
    pub clock: bool,
    pub fps_cap: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct FlagsPanelCfg {
    pub corner: u8,
    pub max_lines: usize,
    /// Which built-in preset the live config matches: `Competitive`, `Quality` or `Custom`.
    pub preset: &'static str,
    /// Pre-rendered `Key = value` lines of every FastFlag CREBLOX writes.
    pub flags: Vec<String>,
}

/// Which built-in preset the live FastFlags config matches.
///
/// Derived from the config itself rather than remembered from the last click, so a hand-edited
/// `fastflags.json` or a half-applied preset can never make the overlay claim a preset that is
/// not actually on disk.
fn active_preset_name(ff: &FastFlagsConfig) -> &'static str {
    let mut competitive = ff.clone();
    competitive.apply_competitive_preset();
    if *ff == competitive {
        return "Competitive";
    }
    let mut quality = ff.clone();
    quality.apply_quality_preset();
    if *ff == quality {
        return "Quality";
    }
    "Custom"
}

#[derive(Clone, Debug)]
pub struct GamePanelCfg {
    pub corner: u8,
}

/// A picture drawn inside the game (logo, watermark, map or reference image).
#[derive(Clone, Debug)]
pub struct ImageOverlayCfg {
    pub path: String,
    pub corner: u8,
    /// Picture height as a percentage of the game window height.
    pub size_pct: u8,
    pub opacity: f32,
    pub offset_x: i16,
    pub offset_y: i16,
}

#[derive(Clone, Debug)]
pub struct CrosshairCfg {
    pub style: u8,
    pub size: u8,
    pub thickness: u8,
    pub gap: u8,
    pub color: [u8; 3],
    pub opacity: f32,
    pub offset_x: i8,
    pub offset_y: i8,
    /// Optional PNG drawn instead of the shapes (empty = shapes).
    pub image: String,
}

#[derive(Clone, Debug)]
pub struct OverlayConfig {
    /// A game session is actually being played. When false the overlay window stays alive
    /// but hides itself, so sitting in the Roblox menu never shows panels over it.
    pub game_active: bool,
    pub opacity: f32,
    /// Where the user parked the HUD, in points from its corner anchor.
    pub offset_x: i16,
    pub offset_y: i16,
    /// Interactive position editing: the overlay becomes draggable and stops being
    /// click-through until the user confirms or cancels on the HUD itself.
    pub edit: bool,
    pub server: Option<ServerPanelCfg>,
    pub stats: Option<StatsPanelCfg>,
    pub flags: Option<FlagsPanelCfg>,
    pub game: Option<GamePanelCfg>,
    pub image: Option<ImageOverlayCfg>,
    pub crosshair: Option<CrosshairCfg>,
}

impl OverlayConfig {
    pub fn from_settings(s: &Settings, ff: &FastFlagsConfig) -> Self {
        let server = s.overlay_enabled.then(|| ServerPanelCfg {
            corner: s.overlay_corner.min(3),
            duration_secs: s.overlay_duration_secs as u32,
            show_ip: s.overlay_show_ip,
            show_job_id: s.overlay_show_job_id,
        });
        let stats = s.overlay_stats_enabled.then(|| StatsPanelCfg {
            corner: s.overlay_stats_corner.min(3),
            cpu: s.overlay_stats_cpu,
            ram: s.overlay_stats_ram,
            ping: s.overlay_stats_ping,
            session: s.overlay_stats_session,
            clock: s.overlay_stats_clock,
            fps_cap: if s.overlay_stats_fps_cap {
                s.target_fps
            } else {
                None
            },
        });
        let flags = s.overlay_flags_enabled.then(|| {
            let map = build_client_app_settings(ff, s.target_fps, None, &[]);
            let flags = map
                .iter()
                .map(|(k, v)| {
                    let val = match v {
                        Value::String(t) => t.clone(),
                        other => other.to_string(),
                    };
                    shorten(&format!("{k} = {val}"), 52)
                })
                .collect();
            FlagsPanelCfg {
                corner: s.overlay_flags_corner.min(3),
                max_lines: (s.overlay_flags_max_lines as usize).clamp(1, 40),
                preset: active_preset_name(ff),
                flags,
            }
        });
        let game = s.overlay_game_enabled.then(|| GamePanelCfg {
            corner: s.overlay_game_corner.min(3),
        });
        let image = if s.overlay_image_enabled && !s.overlay_image_path.trim().is_empty() {
            Some(ImageOverlayCfg {
                path: s.overlay_image_path.clone(),
                corner: s.overlay_image_corner.min(3),
                size_pct: s.overlay_image_size.clamp(5, 100),
                opacity: s.overlay_image_opacity.clamp(0.1, 1.0),
                offset_x: s.overlay_image_offset_x,
                offset_y: s.overlay_image_offset_y,
            })
        } else {
            None
        };
        let crosshair = s.crosshair_enabled.then(|| CrosshairCfg {
            style: s.crosshair_style.min(3),
            size: s.crosshair_size.clamp(4, 120),
            thickness: s.crosshair_thickness.clamp(1, 10),
            gap: s.crosshair_gap.min(60),
            color: s.crosshair_color,
            opacity: s.crosshair_opacity.clamp(0.1, 1.0),
            offset_x: s.crosshair_offset_x,
            offset_y: s.crosshair_offset_y,
            image: s.crosshair_image.clone(),
        });
        Self {
            // Callers that know the session state (the launcher) override this; standalone
            // drivers only build a config while a game is actually running.
            game_active: true,
            opacity: s.overlay_opacity.clamp(0.2, 1.0),
            offset_x: s.overlay_offset_x,
            offset_y: s.overlay_offset_y,
            edit: false,
            server,
            stats,
            flags,
            game,
            image,
            crosshair,
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.server.is_some()
            || self.stats.is_some()
            || self.flags.is_some()
            || self.game.is_some()
            || self.image.is_some()
            || self.crosshair.is_some()
    }
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[derive(Clone, PartialEq, Eq)]
struct ServerData {
    ip: String,
    job_id: String,
    place_id: u64,
}

struct Shared {
    cfg: OverlayConfig,
    server: Option<ServerData>,
    server_since: Instant,
    locations: HashMap<String, String>,
    /// `(game name, players online)` for the current place.
    game_info: Option<(String, u64)>,
    stop: bool,
    /// A config or server change arrived: rebuild the spec now instead of sleeping out
    /// the rest of the tick. This is what makes a tray toggle land immediately.
    dirty: bool,
}

struct Running {
    shared: Arc<Mutex<Shared>>,
    thread: JoinHandle<()>,
}

/// Starts / feeds / stops the in-game overlays. Cheap to create; does nothing until `start`.
#[derive(Default)]
pub struct OverlayManager {
    running: Option<Running>,
}

impl OverlayManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn reap(&mut self) {
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.thread.is_finished())
        {
            self.running = None;
        }
    }

    #[allow(dead_code)]
    pub fn is_running(&mut self) -> bool {
        self.reap();
        self.running.is_some()
    }

    /// Starts the overlays, or just updates the configuration if they are already running.
    pub fn start(&mut self, cfg: OverlayConfig) {
        self.reap();
        if !cfg.any_enabled() {
            self.stop();
            return;
        }
        if let Some(r) = &self.running {
            if let Ok(mut g) = r.shared.lock() {
                g.cfg = cfg;
                g.dirty = true;
            }
            return;
        }
        let shared = Arc::new(Mutex::new(Shared {
            cfg,
            server: None,
            server_since: Instant::now(),
            locations: HashMap::new(),
            game_info: None,
            stop: false,
            dirty: false,
        }));
        let thread_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("creblox-overlay".to_string())
            .spawn(move || run_loop(thread_shared));
        if let Ok(thread) = thread {
            self.running = Some(Running { shared, thread });
        }
    }

    /// Tells the overlays which server the game joined: `(ip, job id, place id)`.
    pub fn set_server(&mut self, server: Option<(String, String, u64)>) {
        self.reap();
        let Some(r) = &self.running else {
            return;
        };
        let new = server.map(|(ip, job_id, place_id)| ServerData {
            ip,
            job_id,
            place_id,
        });
        if let Ok(mut g) = r.shared.lock() {
            if g.server != new {
                g.server = new;
                g.server_since = Instant::now();
                g.game_info = None;
                g.dirty = true;
            }
        }
    }

    /// Puts the running overlay into (or out of) interactive position editing.
    ///
    /// Only flips a flag and wakes the loop: the overlay process is what actually becomes
    /// draggable, so entering edit mode never restarts the window.
    pub fn set_edit(&mut self, edit: bool) {
        self.reap();
        let Some(r) = &self.running else {
            return;
        };
        let Ok(mut g) = r.shared.lock() else {
            return;
        };
        if g.cfg.edit != edit {
            g.cfg.edit = edit;
            g.dirty = true;
        }
    }

    /// Asks the overlays to close without blocking the caller.
    pub fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            if let Ok(mut g) = r.shared.lock() {
                g.stop = true;
            }
            let handle = r.thread;
            let _ = thread::Builder::new()
                .name("creblox-overlay-stop".to_string())
                .spawn(move || {
                    let _ = handle.join();
                });
        }
    }

    /// Closes the overlays and waits until the overlay window process is gone.
    pub fn stop_and_wait(&mut self) {
        if let Some(r) = self.running.take() {
            if let Ok(mut g) = r.shared.lock() {
                g.stop = true;
            }
            let _ = r.thread.join();
        }
    }
}

impl Drop for OverlayManager {
    fn drop(&mut self) {
        self.stop_and_wait();
    }
}

// ---------------------------------------------------------------------------
// Background thread
// ---------------------------------------------------------------------------

fn sleep_unless_stopped(shared: &Arc<Mutex<Shared>>, total: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < total {
        thread::sleep(step);
        waited += step;
        let (stop, dirty) = shared
            .lock()
            .map(|g| (g.stop, g.dirty))
            .unwrap_or((true, false));
        if stop {
            return false;
        }
        // A tray toggle changed the config mid-tick: return so the loop rebuilds the spec
        // immediately. The worst-case latency becomes one 100 ms step, not a full second.
        if dirty {
            return true;
        }
    }
    true
}

fn write_spec(path: &std::path::Path, text: &str) {
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// Where the overlay process leaves the answer of its position editor.
///
/// The overlay owns the drag (it has the pointer), the launcher owns `settings.json`, so the
/// answer comes back as one small file in the same folder rather than two processes writing
/// the same config.
pub fn position_result_path() -> std::path::PathBuf {
    data_dir().join("overlay-position.json")
}

/// What the user did in the position editor.
pub struct PositionResult {
    pub confirmed: bool,
    pub offset_x: i16,
    pub offset_y: i16,
}

/// Reads and removes the editor's answer. `None` while the user is still dragging.
pub fn take_position_result() -> Option<PositionResult> {
    let path = position_result_path();
    let text = fs::read_to_string(&path).ok()?;
    // Remove first: a failed parse must not leave a file that is read again every tick.
    let _ = fs::remove_file(&path);
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(PositionResult {
        confirmed: v["state"].as_str() == Some("confirmed"),
        offset_x: v["offset_x"].as_i64().unwrap_or(0).clamp(-8000, 8000) as i16,
        offset_y: v["offset_y"].as_i64().unwrap_or(0).clamp(-8000, 8000) as i16,
    })
}

fn local_clock() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::from("--:--");
        }
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

/// Cumulative CPU time (ns) and resident bytes of one process.
///
/// The old implementation shelled out to `/bin/ps` once per sample: a fork plus an exec every
/// second, and `ps`'s `%cpu` is an average over the process's whole lifetime, so the number
/// lagged minutes behind what the game was actually doing. These are one syscall each.
#[cfg(target_os = "macos")]
fn process_counters(pid: u32) -> Option<(u64, u64)> {
    use std::ffi::c_void;

    // `struct proc_taskinfo` from <libproc.h>. Only the first four fields are read; the rest
    // keeps the size (and therefore the kernel-side copy) exactly right.
    #[repr(C)]
    #[derive(Default)]
    struct ProcTaskInfo {
        pti_virtual_size: u64,
        pti_resident_size: u64,
        pti_total_user: u64,
        pti_total_system: u64,
        pti_threads_user: u64,
        pti_threads_system: u64,
        pti_policy: i32,
        pti_faults: i32,
        pti_pageins: i32,
        pti_cow_faults: i32,
        pti_messages_sent: i32,
        pti_messages_received: i32,
        pti_syscalls_mach: i32,
        pti_syscalls_unix: i32,
        pti_csw: i32,
        pti_threadnum: i32,
        pti_numrunning: i32,
        pti_priority: i32,
    }
    const PROC_PIDTASKINFO: i32 = 4;
    unsafe extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
    }

    let mut info = ProcTaskInfo::default();
    let size = std::mem::size_of::<ProcTaskInfo>() as i32;
    let written = unsafe {
        proc_pidinfo(
            pid as i32,
            PROC_PIDTASKINFO,
            0,
            (&mut info as *mut ProcTaskInfo).cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // `pti_total_*` are nanoseconds, so no mach timebase conversion is involved. This works
    // unchanged on Apple Silicon, where the timebase is 125/3.
    Some((
        info.pti_total_user.saturating_add(info.pti_total_system),
        info.pti_resident_size,
    ))
}

/// Cumulative CPU time (100 ns ticks → ns) and working-set bytes of one process.
#[cfg(windows)]
fn process_counters(pid: u32) -> Option<(u64, u64)> {
    use core::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    fn ticks(t: &FileTime) -> u64 {
        ((t.high as u64) << 32 | t.low as u64).saturating_mul(100)
    }

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        let times_ok = GetProcessTimes(
            handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        ) != 0;
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        let mem_ok = GetProcessMemoryInfo(
            handle,
            &mut counters,
            std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ) != 0;
        CloseHandle(handle);
        if !times_ok {
            return None;
        }
        let cpu = ticks(&kernel).saturating_add(ticks(&user));
        let mem = if mem_ok {
            counters.working_set_size as u64
        } else {
            0
        };
        Some((cpu, mem))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn process_counters(_pid: u32) -> Option<(u64, u64)> {
    None
}

/// Live CPU / memory of the Roblox process, sampled once per overlay tick.
///
/// CPU is the delta of the process's own CPU time over the wall time between two samples, so
/// it is instantaneous rather than a lifetime average, and 100 % means one fully busy core -
/// the same scale Activity Monitor uses for a single process.
struct Usage {
    pid: Option<u32>,
    last_total_ns: u64,
    last_at: Instant,
    cpu: f32,
    mem_mb: f32,
    have: bool,
}

impl Usage {
    fn new() -> Self {
        Self {
            pid: None,
            last_total_ns: 0,
            last_at: Instant::now(),
            cpu: 0.0,
            mem_mb: 0.0,
            have: false,
        }
    }

    fn sample(&mut self, pid: Option<u32>) {
        let Some(pid) = pid else {
            self.pid = None;
            self.have = false;
            self.cpu = 0.0;
            self.mem_mb = 0.0;
            return;
        };
        let Some((total_ns, mem_bytes)) = process_counters(pid) else {
            // The process is gone or not inspectable (sandboxing): show `--`, never a stale
            // number from the previous server.
            self.pid = Some(pid);
            self.have = false;
            return;
        };
        let now = Instant::now();
        let wall_ns = now.duration_since(self.last_at).as_nanos().max(1) as f64;
        if self.pid == Some(pid) && self.have && total_ns >= self.last_total_ns {
            let used = (total_ns - self.last_total_ns) as f64;
            self.cpu = (used / wall_ns * 100.0).clamp(0.0, 1600.0) as f32;
        }
        self.pid = Some(pid);
        self.last_total_ns = total_ns;
        self.last_at = now;
        self.mem_mb = mem_bytes as f32 / (1024.0 * 1024.0);
        self.have = true;
    }
}

/// Round-trip time in milliseconds to `target` (an IP or a host name), timed with a TCP
/// handshake on port 443.
///
/// Roblox game servers drop ICMP echo requests, so the old `/sbin/ping` probe almost always
/// came back empty and the overlay showed `Ping --`. A TCP connect to the same machine's
/// HTTPS port is answered by Roblox's own servers and measures the same network round trip.
fn tcp_rtt_on(target: &str, port: u16) -> Option<u32> {
    use std::net::{TcpStream, ToSocketAddrs};
    let addr = (target, port).to_socket_addrs().ok()?.next()?;
    let start = Instant::now();
    match TcpStream::connect_timeout(&addr, Duration::from_millis(1200)) {
        Ok(_) => Some(start.elapsed().as_millis().clamp(1, 9_999) as u32),
        Err(_) => None,
    }
}

/// Game servers answer HTTPS on some hosts and plain HTTP on others; try both before
/// giving up on the TCP probe.
fn tcp_rtt(target: &str) -> Option<u32> {
    tcp_rtt_on(target, 443).or_else(|| tcp_rtt_on(target, 80))
}

/// ICMP fallback for the rare case that TCP is filtered but echo requests are answered.
fn icmp_rtt(target: &str) -> Option<u32> {
    let out = Command::new("/sbin/ping")
        .args(["-c", "1", "-t", "1", target])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let after = text.split("time=").nth(1)?;
    let num: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f32>().ok().map(|ms| ms.round() as u32)
}

/// Round trip to the game server itself. There is deliberately no fallback to a Roblox web
/// host: that number measures the website, not your game, and the overlay must not print a
/// value that is not the truth.
fn measure_ping(target: &str) -> Option<u32> {
    tcp_rtt(target).or_else(|| icmp_rtt(target))
}

fn spawn_ping(target: String, out: Arc<AtomicU32>) {
    let _ = thread::Builder::new()
        .name("creblox-ping".to_string())
        .spawn(move || {
            out.store(measure_ping(&target).unwrap_or(NO_PING), Ordering::Relaxed);
        });
}

fn fmt_session(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn run_loop(shared: Arc<Mutex<Shared>>) {
    let dir = data_dir();
    // One spec file per run, so an overlay window that is still shutting down can never read
    // (or be told to quit by) the next run's spec.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let spec_path = dir.join(format!("overlay-panels-{stamp}.json"));
    write_spec(&spec_path, r#"{"panels":[]}"#);

    let Ok(exe) = env::current_exe() else {
        return;
    };
    let log = fs::File::create(dir.join("overlay.log")).ok();
    let mut cmd = Command::new(exe);
    cmd.arg(OVERLAY_WINDOW_FLAG)
        .arg(&spec_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    match log {
        Some(f) => cmd.stderr(Stdio::from(f)),
        None => cmd.stderr(Stdio::null()),
    };
    let Ok(mut child): Result<Child, _> = cmd.spawn() else {
        crate::auth::log("overlay: could not start the overlay window process");
        let _ = fs::remove_file(&spec_path);
        return;
    };
    crate::auth::log(&format!(
        "overlay: overlay window started (pid {}, panels file {})",
        child.id(),
        spec_path.file_name().unwrap_or_default().to_string_lossy()
    ));

    let session_start = Instant::now();
    let ping_ms = Arc::new(AtomicU32::new(NO_PING));
    let mut last_ping = Instant::now() - Duration::from_secs(60);
    let mut last_ping_target: Option<String> = None;
    let mut roblox_pid: Option<u32> = None;
    let mut last_pid_scan = Instant::now() - Duration::from_secs(60);
    // One syscall per tick instead of a `/bin/ps` fork, and a delta-based CPU figure.
    let mut usage = Usage::new();
    let mut pending_locations: Vec<String> = Vec::new();
    let mut last_game_fetch: Option<Instant> = None;
    let mut last_text = String::new();

    loop {
        let (cfg, server, since, locations, game_info) = {
            let Ok(g) = shared.lock() else { break };
            if g.stop {
                break;
            }
            (
                g.cfg.clone(),
                g.server.clone(),
                g.server_since,
                g.locations.clone(),
                g.game_info.clone(),
            )
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            break; // overlay window exited (Roblox window gone)
        }

        // Resolve the server's location once per IP.
        if let Some(s) = &server {
            if !locations.contains_key(&s.ip) && !pending_locations.contains(&s.ip) {
                pending_locations.push(s.ip.clone());
                let ip = s.ip.clone();
                let shared2 = Arc::clone(&shared);
                let _ = thread::Builder::new()
                    .name("creblox-geoip".to_string())
                    .spawn(move || {
                        let loc = lookup_server_location(&ip)
                            .unwrap_or_else(|| "Unknown location".to_string());
                        if let Ok(mut g) = shared2.lock() {
                            g.locations.insert(ip, loc);
                        }
                    });
            }
        }

        // Game name / player count (refreshed every 30 s).
        if cfg.game.is_some() {
            if let Some(s) = &server {
                let due = last_game_fetch.is_none_or(|t| t.elapsed() >= Duration::from_secs(30));
                if due && s.place_id != 0 {
                    last_game_fetch = Some(Instant::now());
                    let place = s.place_id;
                    let shared2 = Arc::clone(&shared);
                    let _ = thread::Builder::new()
                        .name("creblox-gameinfo".to_string())
                        .spawn(move || {
                            if let Some(game) = games_by_places(&[place]).into_iter().next() {
                                if let Ok(mut g) = shared2.lock() {
                                    g.game_info = Some((game.name, game.playing));
                                }
                            }
                        });
                }
            }
        }

        // Which process are the stats about, and what is it using right now?
        if cfg.stats.as_ref().is_some_and(|s| s.cpu || s.ram) {
            if last_pid_scan.elapsed() >= Duration::from_secs(3) {
                last_pid_scan = Instant::now();
                let (procs, _) = enumerate_roblox_processes();
                roblox_pid = procs.iter().max_by_key(|p| p.start_ms).map(|p| p.pid);
            }
            usage.sample(roblox_pid);
        }
        // Ping: measured even before the server IP is known (Roblox's web edge is the
        // fallback), and reset while switching servers so no stale number is shown.
        if let Some(stats) = &cfg.stats {
            if stats.ping {
                let target = server
                    .as_ref()
                    .map(|s| s.ip.clone())
                    .filter(|ip| !ip.is_empty())
                    .unwrap_or_default();
                if target.is_empty() {
                    // No server yet: show `--` rather than a made-up website number.
                    last_ping_target = None;
                    ping_ms.store(NO_PING, Ordering::Relaxed);
                } else {
                    if last_ping_target.as_deref() != Some(target.as_str()) {
                        last_ping_target = Some(target.clone());
                        ping_ms.store(NO_PING, Ordering::Relaxed);
                        last_ping = Instant::now() - Duration::from_secs(60);
                    }
                    if last_ping.elapsed() >= Duration::from_secs(2) {
                        last_ping = Instant::now();
                        spawn_ping(target, Arc::clone(&ping_ms));
                    }
                }
            }
        }

        let mut panels: Vec<Value> = Vec::new();

        // The server panel is drawn even before the server is known. With only this
        // overlay enabled (the default) an unresolved server used to mean *no panels at
        // all*, so the overlay window was on screen but completely empty - which looked
        // exactly like "the overlay does not work".
        if let Some(sp) = &cfg.server {
            match &server {
                Some(s) => {
                    let visible = sp.duration_secs == 0
                        || since.elapsed() < Duration::from_secs(sp.duration_secs as u64);
                    if visible {
                        let title = match locations.get(&s.ip) {
                            Some(loc) => format!("Server: {loc}"),
                            None => "Server: locating...".to_string(),
                        };
                        let mut lines = Vec::new();
                        if sp.show_ip {
                            lines.push(s.ip.clone());
                        }
                        if sp.show_job_id && !s.job_id.is_empty() {
                            lines.push(format!("Job ID: {}", s.job_id));
                        }
                        panels.push(json!({
                            "id": "server",
                            "corner": sp.corner,
                            "title": title,
                            "lines": lines
                        }));
                    }
                }
                None => {
                    // No duration here: this shows until a server is detected, so there is
                    // always something on screen as soon as the game starts. If the log never
                    // yields an address, say so instead of spinning on "Detecting..." forever.
                    let text = if session_start.elapsed() < Duration::from_secs(20) {
                        "Detecting server..."
                    } else {
                        "Server info unavailable"
                    };
                    panels.push(json!({
                        "id": "server",
                        "corner": sp.corner,
                        "title": "CREBLOX overlay",
                        "lines": [text]
                    }));
                }
            }
        }

        if let (Some(gp), Some(s)) = (&cfg.game, &server) {
            let title = match &game_info {
                Some((name, _)) => shorten(name, 36),
                None => "Game".to_string(),
            };
            let mut lines = Vec::new();
            if s.place_id != 0 {
                lines.push(format!("Place ID {}", s.place_id));
            }
            if let Some((_, playing)) = &game_info {
                lines.push(format!("Playing  {playing}"));
            }
            lines.push(format!("In server {}", fmt_session(since.elapsed())));
            panels.push(json!({"id": "game", "corner": gp.corner, "title": title, "lines": lines}));
        }

        if let Some(st) = &cfg.stats {
            let mut lines: Vec<String> = Vec::new();
            if st.cpu || st.ram {
                if st.cpu {
                    lines.push(if usage.have {
                        format!("CPU      {:.0}%", usage.cpu)
                    } else {
                        "CPU      --".to_string()
                    });
                }
                if st.ram {
                    lines.push(if usage.have {
                        format!("RAM      {:.0} MB", usage.mem_mb)
                    } else {
                        "RAM      --".to_string()
                    });
                }
            }
            if st.ping {
                let p = ping_ms.load(Ordering::Relaxed);
                lines.push(if p == NO_PING {
                    "Ping     --".to_string()
                } else {
                    format!("Ping     {p} ms")
                });
            }
            if let Some(fps) = st.fps_cap {
                lines.push(format!("FPS cap  {fps}"));
            }
            if st.session {
                lines.push(format!("Session  {}", fmt_session(session_start.elapsed())));
            }
            if st.clock {
                lines.push(format!("Time     {}", local_clock()));
            }
            if !lines.is_empty() {
                panels.push(json!({
                    "id": "stats",
                    "corner": st.corner,
                    "title": "Roblox stats",
                    "lines": lines
                }));
            }
        }

        if let Some(fl) = &cfg.flags {
            let total = fl.flags.len();
            let mut lines: Vec<String> = fl.flags.iter().take(fl.max_lines).cloned().collect();
            if total > fl.max_lines {
                lines.push(format!("+{} more", total - fl.max_lines));
            }
            if lines.is_empty() {
                lines.push("No FastFlags are active".to_string());
            }
            panels.push(json!({
                "id": "flags",
                "corner": fl.corner,
                "title": format!("FastFlags {} ({total})", fl.preset),
                "lines": lines
            }));
        }

        let mut spec = json!({
            "opacity": cfg.opacity,
            "game_active": cfg.game_active,
            "offset_x": cfg.offset_x,
            "offset_y": cfg.offset_y,
            "edit": cfg.edit,
            "panels": panels
        });
        if let Some(img) = &cfg.image {
            spec["image"] = json!({
                "path": img.path,
                "corner": img.corner,
                "size": img.size_pct,
                "opacity": img.opacity,
                "offset_x": img.offset_x,
                "offset_y": img.offset_y,
            });
        }
        if let Some(ch) = &cfg.crosshair {
            spec["crosshair"] = json!({
                "style": ch.style,
                "size": ch.size,
                "thickness": ch.thickness,
                "gap": ch.gap,
                "color": ch.color,
                "opacity": ch.opacity,
                "offset_x": ch.offset_x,
                "offset_y": ch.offset_y,
                "image": ch.image,
            });
        }
        let text = spec.to_string();
        if text != last_text {
            write_spec(&spec_path, &text);
            last_text = text;
        }
        // Everything this tick is published; clear the wake flag so the sleep below is not
        // interrupted by a change that has already been applied.
        if let Ok(mut g) = shared.lock() {
            g.dirty = false;
        }

        if !sleep_unless_stopped(&shared, Duration::from_secs(1)) {
            break;
        }
    }

    // Ask the overlay window to close, then make sure it is gone.
    write_spec(&spec_path, r#"{"quit":true,"panels":[]}"#);
    for _ in 0..10 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            let _ = fs::remove_file(&spec_path);
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_file(&spec_path);
}
