//! "Close the launcher while playing" support.
//!
//! When enabled, the launcher window quits right after it starts a game (freeing its RAM) and
//! leaves behind this tiny headless copy of itself (`creblox --companion`, no window, no
//! graphics). The companion watches the game, runs the in-game overlays, and when you leave
//! the game it closes the leftover Roblox app and opens the launcher again.

use std::env;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Detaches a helper from this process, so it survives the launcher closing.
///
/// Unix: its own process group. Windows: no console at all (a GUI-subsystem launcher would
/// otherwise attach the child to the parent's console) and its own process group so the
/// console's Ctrl+C / close does not take the companion down with the launcher.
#[cfg(unix)]
fn detach(cmd: &mut Command) {
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

use crate::auth::{load_settings, log};
use crate::fastflags::load_fastflags;
use crate::game_overlay::{OverlayConfig, OverlayManager};
use crate::roblox_process::{
    LogSessionState, enumerate_roblox_processes, find_player_log_for_proc, inspect_player_log_full,
    kill_all_roblox_processes, roblox_logs_dir,
};

pub const COMPANION_FLAG: &str = "--companion";

/// Starts the headless companion as an independent process (it outlives the launcher).
///
/// The launcher no longer hands over to the companion (it stays alive in the menu bar
/// instead), but `creblox --companion` still works for anyone who starts it by hand.
#[allow(dead_code)]
pub fn spawn_companion() -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg(COMPANION_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// `.../CREBLOX.app` when running from a bundle.
#[cfg(target_os = "macos")]
fn app_bundle_path() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    app.extension()
        .is_some_and(|e| e == "app")
        .then(|| app.to_path_buf())
}

fn relaunch_launcher() {
    // On macOS a bundle is reopened through LaunchServices, which is what makes it come back
    // as a normal app instead of a bare executable. Windows has nothing to hand over to: the
    // launcher executable is started directly.
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app_bundle_path() {
        match Command::new("/usr/bin/open").arg(bundle).status() {
            Ok(_) => return,
            Err(e) => log(&format!("companion: could not reopen the launcher: {e}")),
        }
    }
    let result = match env::current_exe() {
        Ok(exe) => {
            let mut cmd = Command::new(exe);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            detach(&mut cmd);
            cmd.spawn().map(|_| ())
        }
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        log(&format!("companion: could not reopen the launcher: {e}"));
    }
}

/// Entry point for `creblox --companion`.
pub fn run() {
    log("companion: started");
    // No window means no Dock icon: a Regular policy here is a running dot that no click can
    // satisfy, because the process has nothing to show and no reopen delegate.
    #[cfg(target_os = "macos")]
    crate::tray::set_accessory_policy();
    let mut settings = load_settings();
    let flags = load_fastflags();
    let mut cfg = OverlayConfig::from_settings(&settings, &flags);
    let logs_dir = roblox_logs_dir();

    let mut overlay = OverlayManager::new();
    let started = Instant::now();
    let mut seen_process = false;
    let mut was_playing = false;
    let mut not_playing_polls = 0u32;
    let mut empty_polls = 0u32;
    let mut last_server: Option<crate::game_overlay::ServerData> = None;
    let mut last_settings_check = Instant::now();
    let left_game;

    loop {
        // Polling quickly keeps leaving a game (and the launcher reopening) instant.
        thread::sleep(Duration::from_millis(300));

        // Settings edited in CREBLOX while the game runs are picked up here, so the overlay
        // follows them live even though the launcher window is closed.
        if last_settings_check.elapsed() >= Duration::from_secs(2) {
            last_settings_check = Instant::now();
            let latest = load_settings();
            if latest != settings {
                settings = latest;
                cfg = OverlayConfig::from_settings(&settings, &load_fastflags());
            }
        }

        let (procs, crash_logs) = enumerate_roblox_processes();

        let mut playing = false;
        let mut server: Option<crate::game_overlay::ServerData> = None;
        for p in &procs {
            let Some(log_path) = find_player_log_for_proc(p, &crash_logs, logs_dir.as_deref())
            else {
                continue;
            };
            let (state, _, _, srv) = inspect_player_log_full(&log_path);
            // `Playing` is the explicit case; `Unknown` with a join banner is the same game
            // on clients whose lifecycle markers are not recognised, and treating that as
            // "no game" left the companion blind to the session it was watching.
            if state == LogSessionState::Playing
                || (state == LogSessionState::Unknown && srv.is_some())
            {
                playing = true;
                if let Some(s) = srv {
                    server = Some(crate::game_overlay::ServerData {
                        ip: s.ip,
                        port: s.port,
                        job_id: s.job_id,
                        place_id: s.place_id,
                    });
                }
            }
        }

        if procs.is_empty() {
            empty_polls += 1;
        } else {
            seen_process = true;
            empty_polls = 0;
        }

        if playing {
            was_playing = true;
            not_playing_polls = 0;
            overlay.start(cfg.clone());
            if server != last_server {
                overlay.set_server(server.clone());
                last_server = server;
            }
        } else if was_playing {
            not_playing_polls += 1;
        }

        // Left the game (the log no longer says "playing" for two quick polls).
        if was_playing && not_playing_polls >= 2 {
            left_game = true;
            break;
        }
        // Roblox closed entirely.
        if seen_process && empty_polls >= 2 {
            left_game = was_playing;
            break;
        }
        // Roblox never started, or the user is just sitting in the Roblox app.
        if (!seen_process && started.elapsed() > Duration::from_secs(150))
            || (seen_process && !was_playing && started.elapsed() > Duration::from_secs(1200))
        {
            left_game = false;
            break;
        }
    }

    if left_game && settings.auto_close_roblox {
        let closed = kill_all_roblox_processes();
        log(&format!(
            "companion: left game, closed {closed} Roblox process(es)"
        ));
        thread::sleep(Duration::from_millis(120));
    }
    // Reopen the launcher first - that is what the player is waiting for - and only then
    // tear the overlay down. The overlay hides itself as soon as Roblox is not in front.
    // A server hop kills the old client on purpose and immediately starts the next one, so
    // the "the game ended" path must not race it and put the launcher in front of it.
    if crate::server_hop::hop_in_progress() {
        log("companion: server hop in flight - not reopening the launcher");
    } else {
        log("companion: reopening launcher");
        relaunch_launcher();
    }
    overlay.stop_and_wait();
}
