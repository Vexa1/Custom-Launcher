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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::api::{games_by_places, lookup_server_location};
use crate::auth::data_dir;
use crate::fastflags::{FastFlagsConfig, build_client_app_settings};
use crate::overlay_spec;
use crate::types::Settings;

pub const OVERLAY_WINDOW_FLAG: &str = "--overlay-window";
const NO_PING: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq)]
pub struct ServerPanelCfg {
    pub corner: u8,
    /// 0 keeps the panel until the game closes.
    pub duration_secs: u32,
    pub show_ip: bool,
    pub show_job_id: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatsPanelCfg {
    pub corner: u8,
    pub cpu: bool,
    pub ram: bool,
    pub ping: bool,
    pub session: bool,
    pub clock: bool,
    pub fps_cap: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct GamePanelCfg {
    pub corner: u8,
}

/// A picture drawn inside the game (logo, watermark, map or reference image).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageOverlayCfg {
    pub path: String,
    pub corner: u8,
    /// Picture height as a percentage of the game window height.
    pub size_pct: u8,
    pub opacity: f32,
    pub offset_x: i16,
    pub offset_y: i16,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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
        // The same three switches the tray's check items flip: one reduction, so the menu and
        // the panels can never disagree about what is on.
        let toggles = overlay_spec::HudToggles {
            server: s.overlay_enabled,
            stats: s.overlay_stats_enabled,
            crosshair: s.crosshair_enabled,
        };
        let server = toggles.server.then(|| ServerPanelCfg {
            corner: s.overlay_corner.min(3),
            duration_secs: s.overlay_duration_secs as u32,
            show_ip: s.overlay_show_ip,
            show_job_id: s.overlay_show_job_id,
        });
        let stats = toggles.stats.then(|| StatsPanelCfg {
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
                    overlay_spec::shorten(&format!("{k} = {val}"), 52)
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
        let crosshair = toggles.crosshair.then(|| CrosshairCfg {
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

#[derive(Clone, PartialEq, Eq)]
pub struct ServerData {
    pub ip: String,
    /// RakNet port from the log, `0` when unknown (the probe falls back to its default).
    pub port: u16,
    pub job_id: String,
    pub place_id: u64,
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
    /// Bumped on every mutation. The loop snapshots it so it can tell whether the `dirty`
    /// flag it is about to clear was consumed by the spec it just wrote, or arrived while
    /// that spec was being built (in which case the wake must be kept).
    rev: u64,
    /// The one panels file this run writes. A tray click rewrites it itself, so the file no
    /// longer depends on the loop's next tick (or on the launcher window drawing a frame).
    spec_path: std::path::PathBuf,
    /// The bytes last written. Shared so the loop and a tray click can never write a spec
    /// older than what is already on disk.
    last_spec: String,
    /// The overlay's own clock: the `Session` row and how long `Detecting server...` shows.
    session_start: Instant,
    usage: Usage,
    /// `NO_PING` until a probe answers. The probe thread and the late room-list answer both
    /// write it, so the row can go from `-- ms` to a number without another probe.
    ping: Arc<AtomicU32>,
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
                // Only wake the loop when something actually changed. This runs on every
                // tray click, settings save and watcher tick, and waking unconditionally
                // rebuilt the spec (and re-measured every label) each time for nothing.
                if g.cfg != cfg {
                    g.cfg = cfg;
                    g.dirty = true;
                    g.rev = g.rev.wrapping_add(1);
                }
            }
            return;
        }
        // Seed from the server the watcher already reported: starting the overlay mid-session
        // must not cost the player a tick of "Server info unavailable".
        let known_server = LAST_SERVER.lock().ok().and_then(|g| g.clone());
        // One spec file per run, so an overlay window that is still shutting down can never
        // read (or be told to quit by) the next run's spec.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let spec_path = crate::auth::data_dir().join(format!("overlay-panels-{stamp}.json"));
        write_spec(&spec_path, r#"{"panels":[]}"#);
        let shared = Arc::new(Mutex::new(Shared {
            cfg,
            server: known_server,
            server_since: Instant::now(),
            locations: HashMap::new(),
            game_info: None,
            stop: false,
            dirty: false,
            rev: 0,
            spec_path,
            last_spec: r#"{"panels":[]}"#.to_string(),
            session_start: Instant::now(),
            usage: Usage::new(),
            ping: Arc::new(AtomicU32::new(NO_PING)),
        }));
        let thread_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("creblox-overlay".to_string())
            .spawn(move || run_loop(thread_shared));
        if let Ok(thread) = thread {
            if let Ok(mut slot) = LIVE.lock() {
                *slot = Some(Arc::downgrade(&shared));
            }
            self.running = Some(Running { shared, thread });
        }
    }

    /// Tells the overlays which server the game joined: `(ip, job id, place id)`.
    pub fn set_server(&mut self, server: Option<ServerData>) {
        self.reap();
        publish_server(server);
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
            g.rev = g.rev.wrapping_add(1);
        }
    }

    /// Asks the overlays to close without blocking the caller.
    pub fn stop(&mut self) {
        if let Ok(mut slot) = LIVE.lock() {
            *slot = None;
        }
        // Forget the server too: the next session must not open seeded with the previous
        // game's address now that the launcher only publishes a server it actually knows.
        if let Ok(mut last) = LAST_SERVER.lock() {
            *last = None;
        }
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
        if let Ok(mut slot) = LIVE.lock() {
            *slot = None;
        }
        if let Ok(mut last) = LAST_SERVER.lock() {
            *last = None;
        }
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

/// The running overlay loop, so the Roblox watcher can hand it the server it just resolved
/// without going through the launcher window.
///
/// The launcher is normally hidden for the whole game (`close_launcher_during_game`, the tray
/// default), so its event loop is not reading messages while the player is in a match. When the
/// only path from "watcher found the server" to "the HUD knows the server" was that event loop,
/// the overlay kept the snapshot it was started with: `Server info unavailable` and `-- ms` for
/// the entire session. The watcher thread now writes the server straight into this state.
static LIVE: Mutex<Option<std::sync::Weak<Mutex<Shared>>>> = Mutex::new(None);

/// The last server any thread reported. A newly started overlay window is seeded from it, so it
/// never has to wait for another change to learn which server the player is already in.
static LAST_SERVER: Mutex<Option<ServerData>> = Mutex::new(None);

fn publish_server(server: Option<ServerData>) {
    if let Ok(mut last) = LAST_SERVER.lock() {
        *last = server.clone();
    }
    let Some(shared) = LIVE
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(std::sync::Weak::upgrade))
    else {
        return;
    };
    let Ok(mut g) = shared.lock() else {
        return;
    };
    if g.server != server {
        g.server = server;
        g.server_since = Instant::now();
        g.game_info = None;
        g.dirty = true;
        g.rev = g.rev.wrapping_add(1);
    }
}

/// Reports the server the watcher resolved (or `None` when the player left the game).
///
/// Safe to call from any thread and at any time: it only updates an overlay that is running.
pub fn set_live_server(server: Option<ServerData>) {
    publish_server(server);
}

/// Applies a HUD check item's click to `settings.json` **and** to the running overlay, on
/// whatever thread the menu event was read on.
///
/// The launcher window is hidden for the whole game (`NSApp hide`), so its egui loop is not
/// the place to apply a menu click: a toggle that only travelled as a `Msg` was saved to
/// `settings.json` and stayed invisible until the next launch. This path has no window
/// dependency - the overlay loop is a thread of this process - and waking it rewrites
/// `overlay-panels-<id>.json` within one 100 ms step.
/// Returns the state now in effect and whether this call applied it; `false` means it was a
/// duplicate delivery of a click that had already been applied.
pub fn apply_tray_toggle(
    toggle: overlay_spec::TrayToggle,
) -> (overlay_spec::HudToggles, bool) {
    let mut settings = crate::auth::load_settings();
    let current = overlay_spec::HudToggles {
        server: settings.overlay_enabled,
        stats: settings.overlay_stats_enabled,
        crosshair: settings.crosshair_enabled,
    };
    // One physical click flips one switch once. macOS delivers a menu action to every reader
    // of the global menu channel, so the tray thread and the window's drain can both see the
    // same click; the copy that arrives second would flip the setting - and the HUD - back.
    let first = HUD_CLICK_DEBOUNCE
        .lock()
        .map(|mut seen| seen.accept(toggle, Instant::now()))
        .unwrap_or(true);
    if !first {
        return (current, false);
    }
    let now = current.flipped(toggle);
    settings.overlay_enabled = now.server;
    settings.overlay_stats_enabled = now.stats;
    settings.crosshair_enabled = now.crosshair;
    crate::auth::save_settings(&settings);
    let flags = crate::fastflags::load_fastflags();
    push_config(OverlayConfig::from_settings(&settings, &flags));
    // Rewrite the file here, not on the overlay loop's next tick: this is the proof, in
    // log.txt, that a menu click reached the HUD while the launcher window was hidden.
    match rewrite_spec_now() {
        Some((name, true)) => crate::auth::log(&format!(
            "tray: rewrote overlay panels spec ({name}: server {}, stats {}, crosshair {})",
            on_off(now.server),
            on_off(now.stats),
            on_off(now.crosshair)
        )),
        Some((name, false)) => crate::auth::log(&format!(
            "tray: overlay panels spec already current ({name}: server {}, stats {}, crosshair {})",
            on_off(now.server),
            on_off(now.stats),
            on_off(now.crosshair)
        )),
        None => crate::auth::log("tray: no overlay running - settings saved, no panels spec to rewrite"),
    }
    (now, true)
}

/// The last tray click applied in this process, so a duplicate delivery of that click is
/// dropped instead of undoing it.
static HUD_CLICK_DEBOUNCE: Mutex<overlay_spec::ClickDebounce> =
    Mutex::new(overlay_spec::ClickDebounce::new());

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// Rebuilds and writes the running overlay's panels file on the calling thread.
///
/// Returns the file name and whether the bytes changed. `None` when no overlay is running.
fn rewrite_spec_now() -> Option<(String, bool)> {
    let shared = LIVE
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(std::sync::Weak::upgrade))?;
    let Ok(mut g) = shared.lock() else {
        return None;
    };
    let changed = write_current_spec(&mut g);
    let name = g
        .spec_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Some((name, changed))
}

/// Hands a new configuration to the running overlay from any thread, without starting or
/// stopping it.
///
/// `game_active` and `edit` describe the session (launcher state, the position editor), not
/// the settings file, so a tray click must never reset them.
fn push_config(mut cfg: OverlayConfig) {
    let Some(shared) = LIVE
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(std::sync::Weak::upgrade))
    else {
        return;
    };
    let Ok(mut g) = shared.lock() else {
        return;
    };
    cfg.game_active = g.cfg.game_active;
    cfg.edit = g.cfg.edit;
    if g.cfg != cfg {
        g.cfg = cfg;
        g.dirty = true;
        g.rev = g.rev.wrapping_add(1);
    }
}

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

#[cfg(target_os = "macos")]
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

/// Wall-clock time from the OS, no C runtime needed.
#[cfg(windows)]
fn local_clock() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut now: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    format!("{:02}:{:02}", now.wHour, now.wMinute)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn local_clock() -> String {
    String::from("--:--")
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
    // Accept any write big enough to hold the four fields below. Requiring an exact size
    // match meant a single short read (interpreter, sandbox) left CPU/RAM on `--` for the
    // whole session even though the process was perfectly healthy.
    if (written as usize) < std::mem::size_of::<u64>() * 4 {
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

/// `(cpu %, resident MB)` of a process via `/bin/ps`.
///
/// Only used when `process_counters` cannot read the process (sandboxing, permissions).
/// `ps`'s `%cpu` is an average over the process's whole lifetime, so this is a fallback that
/// keeps a number on screen - not the primary path.
#[cfg(target_os = "macos")]
fn ps_usage(pid: u32) -> Option<(f32, f32)> {
    let out = Command::new("/bin/ps")
        .args(["-o", "%cpu=,rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace();
    let cpu: f32 = it.next()?.parse().ok()?;
    let rss_kb: f32 = it.next()?.parse().ok()?;
    Some((cpu, rss_kb / 1024.0))
}

#[cfg(not(target_os = "macos"))]
fn ps_usage(_pid: u32) -> Option<(f32, f32)> {
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

    /// Forgets the previous sample, so re-enabling CPU/RAM does not compute a delta over
    /// the whole disabled stretch (which reads as a flat 0 %) or reuse old values.
    fn reset(&mut self) {
        self.pid = None;
        self.last_total_ns = 0;
        self.last_at = Instant::now();
        self.cpu = 0.0;
        self.mem_mb = 0.0;
        self.have = false;
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
            // The process could not be inspected (sandboxing, permissions). Fall back to
            // `/bin/ps` instead of leaving CPU/RAM at `--` for the whole session, and show
            // `--` only when that fails too.
            match ps_usage(pid) {
                Some((cpu, mem_mb)) => {
                    self.pid = Some(pid);
                    // No cumulative counter to diff against: the next real sample must not
                    // read as a spike of every CPU second since the process started.
                    self.last_total_ns = 0;
                    self.last_at = Instant::now();
                    self.cpu = cpu;
                    self.mem_mb = mem_mb;
                    self.have = true;
                }
                None => {
                    self.pid = Some(pid);
                    self.have = false;
                }
            }
            return;
        };
        let now = Instant::now();
        let wall_ns = now.duration_since(self.last_at).as_nanos().max(1) as f64;
        if self.pid == Some(pid)
            && self.have
            && self.last_total_ns > 0
            && total_ns >= self.last_total_ns
        {
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

/// True while a probe is running, so the 2 s tick can never stack a second one: a silent
/// server costs 400 ms of UDP plus up to a second of ICMP, and overlapping threads would
/// only add sockets and out-of-order results.
static PING_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// The server already reported as silent, so overlay.log gets one line per host and not one
/// every two seconds.
static PING_LOGGED: Mutex<Option<String>> = Mutex::new(None);

/// The last room-list answer, so the fallback costs one request a minute instead of one per
/// probe (the probe runs every two seconds while the game is on).
static PING_FROM_LIST: Mutex<Option<(String, Option<u32>, Instant)>> = Mutex::new(None);

/// Set while a room-list lookup runs, so the probes never start a second one - and never wait
/// for the first one either.
static PING_LIST_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// The job the live probe is measuring, so a room-list answer that arrives late only lands
/// while the player is still in that server.
static PING_TARGET: Mutex<Option<(String, Arc<AtomicU32>)>> = Mutex::new(None);

/// A room-list ping already fetched for this job, if it is still fresh.
///
/// This only reads a cache: it never performs the request, so the probe thread is never held
/// behind an HTTP round trip.
fn cached_room_list_ping(server: &ServerData) -> Option<u32> {
    let cache = PING_FROM_LIST.lock().ok()?;
    let (job, ping, at) = cache.as_ref()?;
    if job != &server.job_id {
        return None;
    }
    // An answer is good for a minute; a miss is retried sooner, so a job that appears in the
    // list a moment later is picked up without hammering the API.
    let ttl = if ping.is_some() { 60 } else { 20 };
    (at.elapsed() < Duration::from_secs(ttl)).then_some(*ping)?
}

/// Starts (at most one) detached room-list lookup for this job.
///
/// Roblox gameplay servers sit behind UDMUX relays that often drop RakNet unconnected pings
/// and ICMP, which left the row at `-- ms` for a whole session even though the address and
/// job id were known. The lookup runs on its own thread: the probe thread only ever times
/// packets, so nothing here can stall it or the two-second probe cadence.
fn request_room_list_ping(server: ServerData) {
    if server.place_id == 0 || server.job_id.is_empty() {
        return;
    }
    if PING_LIST_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = thread::Builder::new()
        .name("creblox-ping-list".to_string())
        .spawn(move || {
            let ping = fetch_room_list_ping(&server);
            if let Ok(mut cache) = PING_FROM_LIST.lock() {
                *cache = Some((server.job_id.clone(), ping, Instant::now()));
            }
            if let Some(ms) = ping {
                // Land it on the live row when the player is still in that job; otherwise the
                // next probe would overwrite it with `-- ms` two seconds later.
                if let Ok(target) = PING_TARGET.lock() {
                    if let Some((job, out)) = target.as_ref() {
                        if job == &server.job_id {
                            out.store(ms, Ordering::Relaxed);
                        }
                    }
                }
                log_room_list_ping(&server.job_id);
            }
            PING_LIST_IN_FLIGHT.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        PING_LIST_IN_FLIGHT.store(false, Ordering::Release);
    }
}

/// The network half of the fallback, run on its own thread.
fn fetch_room_list_ping(server: &ServerData) -> Option<u32> {
    let pool = crate::api::fetch_public_server_pool(server.place_id, "Desc", 2, None).ok()?;
    let entries: Vec<(String, Option<u64>)> = pool.into_iter().map(|s| (s.id, s.ping)).collect();
    crate::net_probe::ping_for_job(&entries, &server.job_id)
}

/// One line the first time a job's ping comes from the room list instead of the server: the
/// HUD shows a number either way, so log.txt is where the difference is visible.
fn log_room_list_ping(job_id: &str) {
    static LOGGED: Mutex<Option<String>> = Mutex::new(None);
    let first = LOGGED
        .lock()
        .map(|mut seen| {
            if seen.as_deref() == Some(job_id) {
                false
            } else {
                *seen = Some(job_id.to_string());
                true
            }
        })
        .unwrap_or(false);
    if first {
        crate::auth::log(
            "ping: the game server answered neither ICMP, a RakNet ping nor a TCP connect - using the room list's ping for this job",
        );
    }
}

/// Measures the live game server, never a web edge: a probe to `www.roblox.com` reported
/// ~125 ms from New Zealand while the server the player was in (Sydney) answered in ~30 ms.
///
/// Tiers, in the order they run: ICMP echo to that exact IP (what the game's own stats
/// show), then the RakNet unconnected ping over the game's UDP port, then a TCP handshake to
/// the data-centre edge, then the public room list's ping for this job. Roblox's UDMUX relays
/// drop unauthenticated RakNet pings and often ICMP too, which is why one silent tier does
/// not leave the row at `-- ms` for the whole session.
fn spawn_ping(server: ServerData, out: Arc<AtomicU32>) {
    if PING_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Ok(mut target) = PING_TARGET.lock() {
        *target = Some((server.job_id.clone(), Arc::clone(&out)));
    }
    let spawned = thread::Builder::new()
        .name("creblox-ping".to_string())
        .spawn(move || {
            let port = if server.port == 0 {
                crate::net_probe::DEFAULT_GAME_PORT
            } else {
                server.port
            };
            let mut measured = crate::net_probe::icmp_rtt(&server.ip).map(|ms| ("icmp", ms));
            if measured.is_none() {
                measured = crate::net_probe::udp_rtt(&server.ip, port).map(|ms| ("raknet", ms));
            }
            if measured.is_none() {
                measured = crate::net_probe::tcp_rtt(&server.ip, &crate::net_probe::TCP_PROBE_PORTS)
                    .map(|ms| ("tcp", ms));
            }
            if let Some((method, ms)) = measured {
                log_ping_method(&server, method, ms);
            }
            let mut rtt = measured.map(|(_, ms)| ms);
            if rtt.is_none() {
                // Only ever a cache read here: the HTTP fallback is started on its own
                // thread, so it can never stall this probe or the in-flight flag with it.
                rtt = cached_room_list_ping(&server);
                if rtt.is_none() {
                    request_room_list_ping(server.clone());
                    log_silent_server(&server.ip, port);
                }
            }
            out.store(rtt.unwrap_or(NO_PING), Ordering::Relaxed);
            PING_IN_FLIGHT.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        PING_IN_FLIGHT.store(false, Ordering::Release);
    }
}

/// The tier that answered, logged once per job: `-- ms` for a whole match used to be
/// indistinguishable from a probe that never ran, and a number from another path (the room
/// list, a web edge) must be visible as such.
static PING_METHOD_LOGGED: Mutex<Option<String>> = Mutex::new(None);

fn log_ping_method(server: &ServerData, method: &str, ms: u32) {
    let key = format!(
        "{}:{method}",
        if server.job_id.is_empty() {
            server.ip.as_str()
        } else {
            server.job_id.as_str()
        }
    );
    let first = PING_METHOD_LOGGED
        .lock()
        .map(|mut seen| {
            if seen.as_deref() == Some(key.as_str()) {
                false
            } else {
                *seen = Some(key);
                true
            }
        })
        .unwrap_or(false);
    if first {
        crate::auth::log(&format!("overlay: ping probe active ({method}: {ms} ms)"));
    }
}

/// One line in overlay.log the first time a host answers none of the probes. The HUD can only
/// show `-- ms`, which is indistinguishable from a broken probe; this records which host
/// was asked and that it stayed silent, next to the overlay window's own stderr.
fn log_silent_server(ip: &str, port: u16) {
    let key = format!("{ip}:{port}");
    let first = PING_LOGGED
        .lock()
        .map(|mut seen| {
            if seen.as_deref() == Some(key.as_str()) {
                false
            } else {
                *seen = Some(key.clone());
                true
            }
        })
        .unwrap_or(false);
    if !first {
        return;
    }
    use std::io::Write;
    let path = data_dir().join("overlay.log");
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "ping {key}: no reply to ICMP echo, a RakNet ping or a TCP connect"
        );
    }
}

/// Newest Roblox player process found by name.
///
/// Used when the regular process scan comes back empty, so CPU / RAM do not sit at `--`
/// while the game is clearly running. `pgrep` lists pids ascending, so the last one is the
/// most recently started player.
#[cfg(target_os = "macos")]
fn find_roblox_pid_by_name() -> Option<u32> {
    for args in [
        ["-n", "-x", "RobloxPlayer"],
        ["-n", "-f", "Roblox.app/Contents/MacOS"],
    ] {
        let Ok(out) = Command::new("/usr/bin/pgrep").args(args).output() else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        if let Some(pid) = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .last()
            .and_then(|t| t.parse::<u32>().ok())
        {
            return Some(pid);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn find_roblox_pid_by_name() -> Option<u32> {
    None
}

/// The exact file the overlay window reads, built from the running state.
///
/// One builder serves the loop's tick and a tray click's immediate rewrite; the assembly
/// itself lives in [`overlay_spec`], where `vm-tests` asserts the bytes the HUD receives.
fn build_spec(g: &Shared) -> Value {
    let cfg = &g.cfg;
    let server = &g.server;
    let server_panel = cfg.server.as_ref().map(|sp| overlay_spec::ServerPanel {
        corner: sp.corner,
        duration_secs: sp.duration_secs,
        show_ip: sp.show_ip,
        show_job_id: sp.show_job_id,
    });
    let stats_panel = cfg.stats.as_ref().map(|st| overlay_spec::StatsPanel {
        corner: st.corner,
        cpu: st.cpu,
        ram: st.ram,
        ping: st.ping,
        session: st.session,
        clock: st.clock,
        fps_cap: st.fps_cap,
    });
    let flags_panel = cfg.flags.as_ref().map(|fl| overlay_spec::FlagsPanel {
        corner: fl.corner,
        max_lines: fl.max_lines,
        preset: fl.preset.to_string(),
        flags: fl.flags.clone(),
    });
    let game_panel = cfg
        .game
        .as_ref()
        .map(|gp| overlay_spec::GamePanel { corner: gp.corner });
    let region = server
        .as_ref()
        .and_then(|s| g.locations.get(&s.ip))
        .map(String::as_str);
    let rtt = match g.ping.load(Ordering::Relaxed) {
        NO_PING => None,
        p => Some(p),
    };
    // The helper outlives a session (it is kept alive while Roblox runs), so the session
    // timer starts at this join, not when the helper process started.
    let session_elapsed = match server {
        Some(_) => g.server_since.elapsed(),
        None => g.session_start.elapsed(),
    };
    let panels = overlay_spec::build(
        &overlay_spec::Panels {
            server: server_panel.as_ref(),
            stats: stats_panel.as_ref(),
            flags: flags_panel.as_ref(),
            game: game_panel.as_ref(),
        },
        &overlay_spec::Live {
            server: server.as_ref().map(|s| overlay_spec::LiveServer {
                ip: s.ip.as_str(),
                job_id: s.job_id.as_str(),
                place_id: s.place_id,
            }),
            server_age_secs: match server {
                Some(_) => g.server_since.elapsed().as_secs(),
                None => 0,
            },
            session_secs: session_elapsed.as_secs(),
            detect_secs: g.session_start.elapsed().as_secs(),
            region,
            ping_ms: rtt,
            game: g
                .game_info
                .as_ref()
                .map(|(name, playing)| (name.as_str(), Some(*playing))),
            cpu_pct: g.usage.have.then_some(g.usage.cpu),
            ram_mb: g.usage.have.then_some(g.usage.mem_mb),
            clock: local_clock(),
            edit_preview: cfg.edit,
        },
    );
    let mut spec = overlay_spec::spec_json(
        cfg.opacity,
        cfg.game_active,
        cfg.offset_x,
        cfg.offset_y,
        cfg.edit,
        panels,
    );
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
    overlay_spec::set_crosshair(
        &mut spec,
        cfg.crosshair.as_ref().map(|ch| {
            json!({
                "style": ch.style,
                "size": ch.size,
                "thickness": ch.thickness,
                "gap": ch.gap,
                "color": ch.color,
                "opacity": ch.opacity,
                "offset_x": ch.offset_x,
                "offset_y": ch.offset_y,
                "image": ch.image,
            })
        }),
    );
    spec
}

/// Writes the panels file when its bytes changed; `true` when it did.
fn write_current_spec(g: &mut Shared) -> bool {
    let text = build_spec(g).to_string();
    if text == g.last_spec {
        return false;
    }
    write_spec(&g.spec_path, &text);
    g.last_spec = text;
    true
}

fn run_loop(shared: Arc<Mutex<Shared>>) {
    let dir = data_dir();
    let (spec_path, ping) = match shared.lock() {
        Ok(g) => (g.spec_path.clone(), Arc::clone(&g.ping)),
        Err(_) => return,
    };

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

    let mut last_ping = Instant::now() - Duration::from_secs(60);
    let mut last_ping_target: Option<(String, u16, String)> = None;
    let mut roblox_pid: Option<u32> = None;
    let mut last_pid_scan = Instant::now() - Duration::from_secs(60);
    let mut pending_locations: Vec<String> = Vec::new();
    let mut last_game_fetch: Option<Instant> = None;
    let mut last_place: Option<u64> = None;
    // What was last written to log.txt, so only changes are logged.
    let mut logged_pid: Option<Option<u32>> = None;
    let mut logged_server: Option<Option<String>> = None;
    // The `(job, region, ping)` triple last written into the spec, so log.txt gets one
    // line per resolved server instead of one per tick.
    let mut logged_spec: Option<(String, String, String)> = None;

    loop {
        let (cfg, server, locations, rev) = {
            let Ok(g) = shared.lock() else { break };
            if g.stop {
                break;
            }
            (g.cfg.clone(), g.server.clone(), g.locations.clone(), g.rev)
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            // Say so: "overlay window started" followed by silence is what a helper that
            // dies on its first frame looks like from log.txt.
            crate::auth::log("overlay: overlay window process exited");
            break; // overlay window exited (Roblox window gone)
        }

        // Which server the panels are describing. One line per change: this is the first
        // thing to read when the HUD says it does not know the server.
        // No address in this line: log.txt is pasted into bug reports, and the job id
        // already identifies the server.
        let server_key = server
            .as_ref()
            .map(|s| format!("job {} place {}", s.job_id, s.place_id));
        if logged_server.as_ref() != Some(&server_key) {
            crate::auth::log(&format!(
                "overlay: server is {}",
                server_key.as_deref().unwrap_or("not known yet")
            ));
            logged_server = Some(server_key);
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

        // Game name / player count. Refreshed every 30 s, and at once when the place
        // changes: `set_server` clears `game_info`, so a timer-only refresh left a new
        // game labelled "Game" with no player count for up to half a minute.
        if cfg.game.is_some() {
            match &server {
                Some(s) => {
                    let stale =
                        last_game_fetch.is_none_or(|t| t.elapsed() >= Duration::from_secs(30));
                    let new_place = last_place != Some(s.place_id);
                    if s.place_id != 0 && (stale || new_place) {
                        last_game_fetch = Some(Instant::now());
                        last_place = Some(s.place_id);
                        let place = s.place_id;
                        let shared2 = Arc::clone(&shared);
                        let _ = thread::Builder::new()
                            .name("creblox-gameinfo".to_string())
                            .spawn(move || {
                                if let Some(game) = games_by_places(&[place]).into_iter().next() {
                                    if let Ok(mut g) = shared2.lock() {
                                        // A slow answer for a server the player already
                                        // left must not overwrite the current one.
                                        if g.server.as_ref().is_some_and(|s| s.place_id == place) {
                                            g.game_info = Some((game.name, game.playing));
                                        }
                                    }
                                }
                            });
                    }
                }
                // Back in the Roblox menu: the next join is a fresh session, even when it
                // returns to the same place, so the next fetch must not wait out the timer.
                None => {
                    last_place = None;
                    last_game_fetch = None;
                }
            }
        }

        // Which process are the stats about, and what is it using right now?
        if cfg.stats.as_ref().is_some_and(|s| s.cpu || s.ram) {
            if last_pid_scan.elapsed() >= Duration::from_secs(3) {
                last_pid_scan = Instant::now();
                let (procs, _) = crate::roblox_process::enumerate_roblox_processes_cached();
                roblox_pid = procs
                    .iter()
                    .max_by_key(|p| p.start_ms)
                    .map(|p| p.pid)
                    // The scan can miss the player (helper names, sandboxing, permissions);
                    // a name lookup keeps CPU/RAM from sitting at `--` for the session.
                    .or_else(find_roblox_pid_by_name);
                if logged_pid != Some(roblox_pid) {
                    logged_pid = Some(roblox_pid);
                    crate::auth::log(&format!(
                        "overlay: stats are measuring process {roblox_pid:?}"
                    ));
                }
            }
            if let Ok(mut g) = shared.lock() {
                g.usage.sample(roblox_pid);
            }
        } else {
            // Nothing is being sampled: start clean if it is switched back on.
            if let Ok(mut g) = shared.lock() {
                g.usage.reset();
            }
        }
        // Ping: the live game server only, over every path that can reach it (ICMP, RakNet,
        // a TCP handshake, then the room list for this job). Until the log yields that
        // server's address the row stays `-- ms`, so it can never show a number measured
        // against a web API on another continent.
        if let Some(stats) = &cfg.stats {
            if stats.ping {
                let target = server.as_ref().filter(|s| !s.ip.is_empty()).cloned();
                let key = target
                    .as_ref()
                    .map(|s| (s.ip.clone(), s.port, s.job_id.clone()));
                if last_ping_target != key {
                    last_ping_target = key;
                    ping.store(NO_PING, Ordering::Relaxed);
                    last_ping = Instant::now() - Duration::from_secs(60);
                }
                if let Some(target) = target {
                    if last_ping.elapsed() >= Duration::from_secs(2) {
                        last_ping = Instant::now();
                        spawn_ping(target, Arc::clone(&ping));
                    }
                }
            } else {
                // Ping is off: drop the target so switching it back on shows `-- ms` until
                // the first real measurement instead of a number from minutes ago.
                last_ping_target = None;
                ping.store(NO_PING, Ordering::Relaxed);
            }
        }

        // Build and write through the one function a tray click also uses: one definition of
        // what the HUD shows, and one writer for the file the overlay window polls.
        let (wrote, rtt, region) = match shared.lock() {
            Ok(mut g) => {
                let rtt = match g.ping.load(Ordering::Relaxed) {
                    NO_PING => None,
                    p => Some(p),
                };
                let region = g
                    .server
                    .as_ref()
                    .and_then(|s| g.locations.get(&s.ip))
                    .cloned();
                (write_current_spec(&mut g), rtt, region)
            }
            Err(_) => break,
        };
        if wrote {
            // One line when the panels start describing a real server: the overlay window
            // logs its own `spec loaded (...)`, so the pair is what shows a resolved server
            // reached the HUD instead of the snapshot it opened with.
            let resolved = server.as_ref().map(|s| {
                (
                    format!("{}:{}", s.job_id, s.place_id),
                    region.unwrap_or_else(|| "locating...".to_string()),
                    rtt.map(|p| p.to_string())
                        .unwrap_or_else(|| "--".to_string()),
                )
            });
            if resolved != logged_spec {
                if let Some((_, loc, ms)) = &resolved {
                    crate::auth::log(&format!(
                        "overlay: updated panels spec with server {loc} ({ms} ms)"
                    ));
                }
                logged_spec = resolved;
            }
        }
        // Clear only the wake this tick consumed. A change that arrived while the spec
        // above was being built bumped `rev`; keeping `dirty` set makes the sleep below
        // return at once and apply it, instead of leaving it until the next one-second
        // tick (which is what made a tray toggle occasionally feel one beat slow).
        if let Ok(mut g) = shared.lock() {
            g.dirty &= g.rev != rev;
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
