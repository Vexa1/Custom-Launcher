#![recursion_limit = "1024"]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod app;
mod auth;
mod avatar_anim;
/// egui from the one place this crate gets it, so shared modules can name its types
/// without a direct `egui` dependency.
pub use eframe::egui;

mod avatar_render;
mod avatar_scene;
mod avatar_sync;
mod chart_feed;
mod avatar_3d;
mod companion;
mod discord_rpc;
mod fastflags;
mod game_log;
mod inventory;
mod sha256;
mod game_overlay;
mod net_probe;
mod overlay_spec;
mod overlay_window;
mod party;
mod party_server;
mod platform;
mod playtime;
mod profile_cache;
mod release_links;
mod roblox_process;
mod scroll;
mod server_hop;
#[allow(dead_code)]
mod telemetry;
mod theme;
mod thumb_pipeline;
mod tray;
mod types;
mod ui;
mod window_lifecycle;
mod updater;
mod video_preview;

#[cfg(windows)]
mod zip_update;

use std::env;


use app::Creblox;
use auth::log;
use roblox_process::launch_from_uri;

// A frame of this UI allocates a lot of short-lived vertex, text and string
// buffers. mimalloc handles that pattern with less CPU and less fragmentation
// than the system allocator, on both macOS and Windows.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = env::args().collect();
    // In-game overlay window (transparent, click-through, follows the Roblox window).
    if args
        .get(1)
        .is_some_and(|a| a == game_overlay::OVERLAY_WINDOW_FLAG)
    {
        if let Some(spec) = args.get(2) {
            // Never swallow this: if the window cannot be created the whole overlay is gone,
            // and the launcher would only see "the child exited" with no reason.
            if let Err(e) = overlay_window::run(std::path::PathBuf::from(spec)) {
                log(&format!("overlay: window failed to start: {e}"));
            }
        }
        return Ok(());
    }
    // Headless helper that outlives the launcher window while a game is running.
    if args.get(1).is_some_and(|a| a == companion::COMPANION_FLAG) {
        companion::run();
        return Ok(());
    }
    if updater::try_run_quiet_apply_cli(&args) {
        return Ok(());
    }
    if let Some(u_args) = updater::try_parse_updater_cli(&args) {
        return updater::run_updater_window(u_args);
    }
    updater::cleanup_old_binary();
    // Started by a roblox-player: link from the website? Launch the game and exit.
    if let Some(uri) = args.get(1) {
        if uri.starts_with("roblox-player:") {
            if let Err(e) = launch_from_uri(uri, false) {
                log(&format!("Launch failed: {e}"));
            }
            return Ok(());
        }
        // A party invite link opens the launcher with the code already in the join box.
        if uri.starts_with("creblox://party/") {
            party::set_pending_invite(uri);
        }
    }
    // Windows: the `creblox:` scheme has to exist in the per-user registry for a link to
    // reach this process at all. Best effort, no elevation, and harmless to repeat.
    #[cfg(windows)]
    if let Err(e) = roblox_process::register_invite_protocol() {
        log(&format!("party: could not register the creblox: link handler: {e}"));
    }

    log(&format!(
        "app: CREBLOX v{} starting",
        env!("CARGO_PKG_VERSION")
    ));
    let boot_settings = auth::load_settings();
    let start_in_tray = tray::should_start_in_tray(&args, boot_settings.tray_enabled);
    // A restarted-after-update instance must not defer to the copy that is still closing.
    let relaunched = args.iter().any(|a| a == "--updated");
    if !relaunched
        && !start_in_tray
        && boot_settings.tray_enabled
        && tray::try_focus_existing_instance()
    {
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([900.0, 600.0]);
    if start_in_tray {
        viewport = viewport.with_visible(false);
    }
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/logo.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let result = eframe::run_native(
        "CREBLOX",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let mut app = Creblox::new(cc, start_in_tray);
            app.apply_startup_invite();
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = &result {
        // A bundled app has no terminal: log it and tell the user, instead of the icon
        // bouncing and the app closing with no explanation.
        log(&format!("app: failed to start: {e}"));
        #[cfg(target_os = "macos")]
        {
            let message = e
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', " ");
            let script = format!(
                "display alert \"CREBLOX could not start\" message \"{message}\" as critical"
            );
            let _ = std::process::Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .status();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::roblox_process::{
        LogSessionState, extract_active_place_id_from_log, inspect_player_log_text,
        parse_crash_handler_attachment,
    };
    use std::path::PathBuf;

    #[test]
    fn parses_crash_handler_pid_and_log() {
        let cmd = r#"RobloxCrashHandler.exe --attachment=attachment_0.747_Player_E6E63_last.log=C:\Users\Test User\AppData\Local\Roblox\logs\0.747.0_Player_E6E63_last.log --database=C:\logs --annotation=PID=4208 --annotation=PlatformId=2"#;
        let parsed = parse_crash_handler_attachment(cmd);
        assert_eq!(
            parsed,
            Some((
                4208,
                PathBuf::from(
                    r"C:\Users\Test User\AppData\Local\Roblox\logs\0.747.0_Player_E6E63_last.log"
                )
            ))
        );
    }

    #[test]
    fn detects_roblox_log_lifecycle_states() {
        let tray_log = "AppState/TrayMode\nInfo [FLog::Systray] Systray initialized in sync mode\nInfo [FLog::WndProcessCheck] waitForNewPlayerProcess";
        assert_eq!(
            inspect_player_log_text(tray_log, tray_log.len() as u64),
            (LogSessionState::TrayOnly, true)
        );

        let playing_teleport = "[FLog::SingleSurfaceApp] launchUGCGame: (stage:Native).\n[FLog::SessionTransitionFSM] Entered play session.\n[FLog::SingleSurfaceApp] setStage: (stage:UGCGame)\n[FLog::SessionTransitionFSM] Teleported.\n[FLog::SessionTransitionFSM] Entered play session.";
        assert_eq!(
            inspect_player_log_text(playing_teleport, playing_teleport.len() as u64),
            (LogSessionState::Playing, false)
        );

        let finished_play = format!(
            "{playing_teleport}\n[FLog::SingleSurfaceApp] returnToLuaApp: (stage:UGCGame).\n[FLog::SingleSurfaceApp] setStage: (stage:LuaApp)\n[FLog::SessionTransitionFSM] Entered app session."
        );
        assert_eq!(
            inspect_player_log_text(&finished_play, finished_play.len() as u64),
            (LogSessionState::AppAfterPlay, false)
        );

        let resumed_play =
            format!("{finished_play}\n[FLog::SessionTransitionFSM] Entered play session.");
        assert_eq!(
            inspect_player_log_text(&resumed_play, resumed_play.len() as u64),
            (LogSessionState::Playing, false)
        );

        let app_only = "[FLog::SingleSurfaceApp] setStage: (stage:LuaApp)\n[FLog::SessionTransitionFSM] Entered app session.";
        assert_eq!(
            inspect_player_log_text(app_only, app_only.len() as u64),
            (LogSessionState::AppOnly, false)
        );

        // Some Roblox builds only write returnToLuaApp when the player leaves; without the
        // FSM marker this must still end the session instead of staying on Playing forever.
        let return_only = "[FLog::SingleSurfaceApp] launchUGCGame: (stage:Native).\n\
[FLog::SessionTransitionFSM] Entered play session.\n\
[FLog::SingleSurfaceApp] returnToLuaApp: (stage:UGCGame).";
        assert_eq!(
            inspect_player_log_text(return_only, 40_000),
            (LogSessionState::AppAfterPlay, false)
        );
    }

    #[test]
    fn extracts_latest_joined_place_id_from_player_log() {
        let log = "2026-06-16T12:28:19Z,28.4,a9,6 [FLog::Output] ! Joining game 'cf9da149-3ee6' place 1818 at 128.116.1.1\n\
                   2026-06-16T12:35:00Z,42.1,b1,6 [FLog::Output] ! Joining game '11111111-2222' place 2753915549 at 128.116.2.2";
        assert_eq!(extract_active_place_id_from_log(log), Some(2753915549));
        assert_eq!(extract_active_place_id_from_log("random text"), None);
    }
}
