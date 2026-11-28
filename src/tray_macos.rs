//! macOS menu-bar (status item) tray, login item and single-instance support.
//!
//! The status item and its menu are created on the main thread, inside
//! `TrayController::new`, which eframe calls while starting the AppKit event
//! loop. Menu clicks are forwarded to the app over the existing `Msg` channel.

use std::cell::RefCell;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui;
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::types::Msg;

const ITEM_SHOW: &str = "creblox.show";
const ITEM_HIDE: &str = "creblox.hide";
const ITEM_HOME: &str = "creblox.home";
const ITEM_PROFILE: &str = "creblox.profile";
const ITEM_AVATAR: &str = "creblox.avatar";
const ITEM_NOTIFICATIONS: &str = "creblox.notifications";
const ITEM_SETTINGS: &str = "creblox.settings";
const ITEM_FASTFLAGS: &str = "creblox.fastflags";
const ITEM_TOGGLE_SERVER: &str = "creblox.overlay.server";
const ITEM_TOGGLE_STATS: &str = "creblox.overlay.stats";
const ITEM_TOGGLE_CROSSHAIR: &str = "creblox.overlay.crosshair";
const ITEM_EDIT_POSITION: &str = "creblox.overlay.edit";
const ITEM_COPY_JOB: &str = "creblox.copy_job";
const ITEM_CLOSE_ROBLOX: &str = "creblox.close_roblox";
const ITEM_ACCOUNT_PREFIX: &str = "creblox.account.";
const ITEM_PRESET_QUALITY: &str = "creblox.preset.quality";
const ITEM_PRESET_COMPETITIVE: &str = "creblox.preset.competitive";
/// Fixed menu slots: the menu is built once at startup, so the switcher shows at most this
/// many accounts instead of rebuilding native menu items whenever the list changes.
const ACCOUNT_SLOTS: usize = 4;
const ITEM_DISCORD: &str = "creblox.discord";
const ITEM_UPDATES: &str = "creblox.updates";
const ITEM_HOP_RANDOM: &str = "creblox.hop.random";
const ITEM_HOP_LEAST: &str = "creblox.hop.least";
const ITEM_HOP_PING: &str = "creblox.hop.ping";
const ITEM_QUIT: &str = "creblox.quit";

const LAUNCH_AGENT_LABEL: &str = "com.creblox.launcher";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    ShowAndFocus,
    HideToTray,
    OpenHome,
    OpenProfile,
    OpenAvatar,
    OpenNotifications,
    OpenSettings,
    OpenFastFlags,
    ToggleServerOverlay,
    ToggleStatsOverlay,
    ToggleCrosshair,
    /// Drag the HUD on screen and confirm where it lands.
    EditOverlayPosition,
    CopyJobId,
    CloseRoblox,
    /// Move to a different public server of the place being played.
    ServerHop(crate::server_hop::ServerHopMode),
    /// Index into `TrayState.accounts`; the app resolves it, because it owns the list.
    SwitchAccountSlot(usize),
    ApplyQualityPreset,
    ApplyCompetitivePreset,
    ToggleDiscordRpc,
    CheckUpdates,
    QuitApp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    pub enabled: bool,
    pub window_visible: bool,
    pub status_line: String,
    pub tooltip: String,
    pub discord_rpc_enabled: bool,
    /// Signed in: the page shortcuts (Home, Profile, …) can be used.
    pub signed_in: bool,
    /// "Server: Dallas, US" while connected, empty otherwise.
    pub server_line: String,
    /// Roblox Job ID of the current server, empty when unknown.
    pub job_id: String,
    /// A live game session with a known place and job: the Server Hop submenu is usable.
    pub hop_enabled: bool,
    /// A Roblox player process is alive, so "Close Roblox" has something to close.
    pub roblox_running: bool,
    /// Saved accounts for the switcher, active one prefixed with a check mark.
    pub accounts: Vec<String>,
    pub overlay_server: bool,
    pub overlay_stats: bool,
    pub overlay_crosshair: bool,
    /// The position editor is open on screen.
    pub overlay_editing: bool,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            enabled: true,
            window_visible: true,
            status_line: "Idle".to_string(),
            tooltip: "CREBLOX".to_string(),
            discord_rpc_enabled: true,
            signed_in: false,
            server_line: String::new(),
            job_id: String::new(),
            hop_enabled: false,
            roblox_running: false,
            accounts: Vec::new(),
            overlay_server: true,
            overlay_stats: false,
            overlay_crosshair: false,
            overlay_editing: false,
        }
    }
}

struct TrayShared {
    desired: TrayState,
    shutdown: bool,
}

/// Maps a menu item id to the message the app understands.
fn map_menu_id(id: &str) -> Option<TrayEvent> {
    match id {
        ITEM_SHOW => Some(TrayEvent::ShowAndFocus),
        ITEM_HIDE => Some(TrayEvent::HideToTray),
        ITEM_HOME => Some(TrayEvent::OpenHome),
        ITEM_PROFILE => Some(TrayEvent::OpenProfile),
        ITEM_AVATAR => Some(TrayEvent::OpenAvatar),
        ITEM_NOTIFICATIONS => Some(TrayEvent::OpenNotifications),
        ITEM_SETTINGS => Some(TrayEvent::OpenSettings),
        ITEM_FASTFLAGS => Some(TrayEvent::OpenFastFlags),
        ITEM_TOGGLE_SERVER => Some(TrayEvent::ToggleServerOverlay),
        ITEM_TOGGLE_STATS => Some(TrayEvent::ToggleStatsOverlay),
        ITEM_TOGGLE_CROSSHAIR => Some(TrayEvent::ToggleCrosshair),
        ITEM_EDIT_POSITION => Some(TrayEvent::EditOverlayPosition),
        ITEM_COPY_JOB => Some(TrayEvent::CopyJobId),
        ITEM_CLOSE_ROBLOX => Some(TrayEvent::CloseRoblox),
        ITEM_PRESET_QUALITY => Some(TrayEvent::ApplyQualityPreset),
        ITEM_PRESET_COMPETITIVE => Some(TrayEvent::ApplyCompetitivePreset),
        ITEM_DISCORD => Some(TrayEvent::ToggleDiscordRpc),
        ITEM_UPDATES => Some(TrayEvent::CheckUpdates),
        ITEM_QUIT => Some(TrayEvent::QuitApp),
        ITEM_HOP_RANDOM => Some(TrayEvent::ServerHop(crate::server_hop::ServerHopMode::Random)),
        ITEM_HOP_LEAST => Some(TrayEvent::ServerHop(
            crate::server_hop::ServerHopMode::LeastPlayers,
        )),
        ITEM_HOP_PING => Some(TrayEvent::ServerHop(
            crate::server_hop::ServerHopMode::LowestPing,
        )),
        _ => id
            .strip_prefix(ITEM_ACCOUNT_PREFIX)
            .and_then(|slot| slot.parse::<usize>().ok())
            .filter(|slot| *slot < ACCOUNT_SLOTS)
            .map(TrayEvent::SwitchAccountSlot),
    }
}

/// Applies the HUD half of a menu click on the thread that read it.
///
/// The status item keeps working while the launcher is hidden (`NSApp hide`) and egui is not
/// drawing frames, so a click that only became a `Msg` was written to `settings.json` and the
/// overlay kept drawing the old panels until the game was restarted. Flipping the setting here
/// hands it to the overlay loop, which is a thread of this process and rewrites the panels file
/// without any window.
/// Returns `false` when this was a second copy of a click that was already applied, so the
/// caller does not forward that copy (and log it) again.
fn apply_hud_action(action: &TrayEvent) -> bool {
    use crate::overlay_spec::TrayToggle;
    let toggle = match action {
        TrayEvent::ToggleServerOverlay => TrayToggle::Server,
        TrayEvent::ToggleStatsOverlay => TrayToggle::Stats,
        TrayEvent::ToggleCrosshair => TrayToggle::Crosshair,
        // Not a HUD switch: always belongs to the window.
        _ => return true,
    };
    let (now, applied) = crate::game_overlay::apply_tray_toggle(toggle);
    if !applied {
        // `muda` hands one physical click to every reader of its menu channel; the copy that
        // arrives second must not flip the switch back or queue a second message.
        return false;
    }
    // The tick next to the item is an AppKit object: `muda` items are `Rc`-based, so they
    // stay on the main thread and the new state is applied there, even while eframe is idle.
    dispatch_main(move || sync_hud_checks(now));
    true
}

thread_local! {
    /// The three HUD check items, registered when the menu is built (main thread).
    static HUD_ITEMS: RefCell<Option<(CheckMenuItem, CheckMenuItem, CheckMenuItem)>> =
        const { RefCell::new(None) };
}

/// Sets the check marks from the state the click just produced. Runs on the main thread.
fn sync_hud_checks(state: crate::overlay_spec::HudToggles) {
    let _ = HUD_ITEMS.try_with(|items| {
        if let Some((server, stats, crosshair)) = items.borrow().as_ref() {
            server.set_checked(state.server);
            stats.set_checked(state.stats);
            crosshair.set_checked(state.crosshair);
        }
    });
}

pub struct TrayController {
    icon: Mutex<Option<TrayIcon>>,
    show_item: MenuItem,
    hide_item: MenuItem,
    page_items: Vec<MenuItem>,
    status_item: MenuItem,
    server_item: MenuItem,
    copy_job_item: MenuItem,
    /// The Server Hop submenu: grayed out until a game session is live.
    hop_menu: Option<Submenu>,
    hop_items: Vec<MenuItem>,
    close_roblox_item: MenuItem,
    account_items: Vec<MenuItem>,
    preset_quality_item: MenuItem,
    preset_competitive_item: MenuItem,
    server_overlay_item: CheckMenuItem,
    stats_overlay_item: CheckMenuItem,
    crosshair_item: CheckMenuItem,
    edit_item: MenuItem,
    discord_item: CheckMenuItem,
    shared: Arc<Mutex<TrayShared>>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl TrayController {
    pub fn new(initial: TrayState, tx: Sender<Msg>, ctx: egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(TrayShared {
            desired: initial.clone(),
            shutdown: false,
        }));

        let show_item = MenuItem::with_id(ITEM_SHOW, "Open CREBLOX", true, None);
        let hide_item = MenuItem::with_id(ITEM_HIDE, "Hide to Menu Bar", true, None);
        let home_item = MenuItem::with_id(ITEM_HOME, "Home", true, None);
        let profile_item = MenuItem::with_id(ITEM_PROFILE, "Profile", true, None);
        let avatar_item = MenuItem::with_id(ITEM_AVATAR, "Avatar", true, None);
        let notifications_item = MenuItem::with_id(ITEM_NOTIFICATIONS, "Notifications", true, None);
        let settings_item = MenuItem::with_id(ITEM_SETTINGS, "Settings…", true, None);
        let fastflags_item = MenuItem::with_id(ITEM_FASTFLAGS, "FastFlags…", true, None);
        let status_item =
            MenuItem::with_id("creblox.status", initial.status_line.clone(), false, None);
        let server_item = MenuItem::with_id("creblox.server", "Server: not connected", false, None);
        let copy_job_item = MenuItem::with_id(
            ITEM_COPY_JOB,
            "Copy Job ID",
            !initial.job_id.is_empty(),
            None,
        );
        // Killing the leftover Roblox app used to require opening Settings; it belongs in
        // the menu that is reachable while the game owns the screen.
        let close_roblox_item = MenuItem::with_id(
            ITEM_CLOSE_ROBLOX,
            "Close Roblox",
            initial.roblox_running,
            None,
        );
        // Server Hop: three fixed slots, enabled together with their parent submenu. Built
        // once at startup like the account switcher, because native menu items are not
        // rebuilt while the app runs.
        let hop_random_item = MenuItem::with_id(
            ITEM_HOP_RANDOM,
            "Quick Hop (Any Server)",
            initial.hop_enabled,
            None,
        );
        let hop_least_item = MenuItem::with_id(
            ITEM_HOP_LEAST,
            "Least Players",
            initial.hop_enabled,
            None,
        );
        let hop_ping_item = MenuItem::with_id(
            ITEM_HOP_PING,
            "Lowest Ping",
            initial.hop_enabled,
            None,
        );
        let hop_menu = Submenu::with_items(
            "Server Hop",
            initial.hop_enabled,
            &[&hop_random_item, &hop_least_item, &hop_ping_item],
        )
        .ok();
        let account_items: Vec<MenuItem> = (0..ACCOUNT_SLOTS)
            .map(|slot| {
                MenuItem::with_id(
                    format!("{ITEM_ACCOUNT_PREFIX}{slot}"),
                    "No saved account",
                    false,
                    None,
                )
            })
            .collect();
        let preset_quality_item = MenuItem::with_id(ITEM_PRESET_QUALITY, "Quality", true, None);
        let preset_competitive_item =
            MenuItem::with_id(ITEM_PRESET_COMPETITIVE, "Max FPS", true, None);
        let server_overlay_item = CheckMenuItem::with_id(
            ITEM_TOGGLE_SERVER,
            "Server region overlay",
            true,
            initial.overlay_server,
            None,
        );
        let stats_overlay_item = CheckMenuItem::with_id(
            ITEM_TOGGLE_STATS,
            "Stats overlay",
            true,
            initial.overlay_stats,
            None,
        );
        let crosshair_item = CheckMenuItem::with_id(
            ITEM_TOGGLE_CROSSHAIR,
            "Crosshair",
            true,
            initial.overlay_crosshair,
            None,
        );
        let edit_item = MenuItem::with_id(
            ITEM_EDIT_POSITION,
            "Edit Overlay Position…",
            true,
            None,
        );
        let discord_item = CheckMenuItem::with_id(
            ITEM_DISCORD,
            "Discord Rich Presence",
            true,
            initial.discord_rpc_enabled,
            None,
        );
        let updates_item = MenuItem::with_id(ITEM_UPDATES, "Check for Updates…", true, None);
        let quit_item = MenuItem::with_id(ITEM_QUIT, "Quit CREBLOX", true, None);
        let sep1 = PredefinedMenuItem::separator();
        let sep2 = PredefinedMenuItem::separator();
        let sep3 = PredefinedMenuItem::separator();
        let sep4 = PredefinedMenuItem::separator();

        let icon = (|| -> Option<TrayIcon> {
            let account_refs: Vec<&dyn IsMenuItem> = account_items
                .iter()
                .map(|item| item as &dyn IsMenuItem)
                .collect();
            let account_menu = Submenu::with_items("Switch Account", true, &account_refs).ok()?;
            let preset_refs: [&dyn IsMenuItem; 2] =
                [&preset_quality_item, &preset_competitive_item];
            let presets_menu =
                Submenu::with_items("FastFlags Preset", true, &preset_refs).ok()?;
            let hop_menu_ref: &dyn IsMenuItem = hop_menu.as_ref()?;
            let menu = Menu::with_items(&[
                &show_item,
                &hide_item,
                &sep1,
                &home_item,
                &profile_item,
                &avatar_item,
                &notifications_item,
                &settings_item,
                &fastflags_item,
                &account_menu,
                &presets_menu,
                &sep2,
                &status_item,
                &server_item,
                &copy_job_item,
                &close_roblox_item,
                hop_menu_ref,
                &sep3,
                &server_overlay_item,
                &stats_overlay_item,
                &crosshair_item,
                &edit_item,
                &discord_item,
                &updates_item,
                &sep4,
                &quit_item,
            ])
            .ok()?;
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("CREBLOX")
                // Catalina draws the status item itself: a template image is recoloured
                // black on a light menu bar and white on a dark one, which a plain colour
                // icon cannot do.
                .with_icon_templated(load_tray_icon()?)
                .build()
                .ok()
        })();

        if let Some(icon) = &icon {
            let _ = icon.set_visible(initial.enabled);
        }

        let shared_bg = Arc::clone(&shared);
        let join = thread::Builder::new()
            .name("creblox-tray".to_string())
            .spawn(move || {
                loop {
                    let shutdown = shared_bg.lock().map(|g| g.shutdown).unwrap_or(true);
                    if shutdown {
                        break;
                    }
                    let mut fired = false;
                    while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                        if let Some(action) = map_menu_id(ev.id().as_ref()) {
                            // A duplicate copy of a HUD click is dropped here, so the window
                            // never sees the same toggle twice.
                            if apply_hud_action(&action) {
                                // Quit and Open must not wait for a frame. While the launcher
                                // lives in the menu bar eframe may not tick at all, so a queued
                                // message would sit unread and the click would look dead.
                                if matches!(action, TrayEvent::QuitApp) {
                                    quit_now();
                                }
                                if matches!(action, TrayEvent::ShowAndFocus) {
                                    dispatch_main(wake_launcher);
                                }
                                let _ = tx.send(Msg::TrayAction(action));
                                fired = true;
                            }
                        }
                    }
                    while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = ev
                        {
                            dispatch_main(wake_launcher);
                            let _ = tx.send(Msg::TrayAction(TrayEvent::ShowAndFocus));
                            fired = true;
                        }
                    }
                    if fired {
                        ctx.request_repaint();
                    }
                    thread::sleep(Duration::from_millis(200));
                }
            })
            .ok();

        HUD_ITEMS.with(|slot| {
            *slot.borrow_mut() = Some((
                server_overlay_item.clone(),
                stats_overlay_item.clone(),
                crosshair_item.clone(),
            ));
        });

        Self {
            icon: Mutex::new(icon),
            show_item,
            hide_item,
            page_items: vec![
                home_item,
                profile_item,
                avatar_item,
                notifications_item,
                settings_item,
                fastflags_item,
            ],
            status_item,
            server_item,
            copy_job_item,
            hop_menu,
            hop_items: vec![hop_random_item, hop_least_item, hop_ping_item],
            close_roblox_item,
            account_items,
            preset_quality_item,
            preset_competitive_item,
            server_overlay_item,
            stats_overlay_item,
            crosshair_item,
            edit_item,
            discord_item,
            shared,
            join: Mutex::new(join),
        }
    }

    /// Drains pending menu clicks / status-item clicks on the calling (main) thread.
    ///
    /// The background thread normally forwards them, but AppKit delivers menu actions on
    /// the main thread's run loop; reading the same channel from the UI thread as well
    /// makes sure a click is never missed.
    pub fn drain_events(&self) -> Vec<TrayEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if let Some(action) = map_menu_id(ev.id().as_ref()) {
                // The main thread may win the race for a click; apply it here too so the HUD
                // never depends on which reader of the menu channel got there first. A
                // duplicate copy is not forwarded: the running HUD already has that state.
                if apply_hud_action(&action) {
                    // The main thread reads the same channel: act here too, so a click never
                    // depends on which reader won the race.
                    if matches!(action, TrayEvent::QuitApp) {
                        quit_now();
                    }
                    if matches!(action, TrayEvent::ShowAndFocus) {
                        wake_launcher();
                    }
                    out.push(action);
                }
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                out.push(TrayEvent::ShowAndFocus);
            }
        }
        out
    }

    pub fn sync(&self, mut next: TrayState) {
        next.tooltip = clamp_tray_tooltip(&next.tooltip);
        let changed = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            if guard.shutdown || guard.desired == next {
                false
            } else {
                guard.desired = next.clone();
                true
            }
        };
        if !changed {
            return;
        }
        if let Ok(guard) = self.icon.lock() {
            if let Some(icon) = guard.as_ref() {
                let _ = icon.set_visible(next.enabled);
                let _ = icon.set_tooltip(Some(next.tooltip.as_str()));
            }
        }
        self.discord_item.set_checked(next.discord_rpc_enabled);
        self.discord_item.set_enabled(next.enabled);
        self.server_overlay_item.set_checked(next.overlay_server);
        self.server_overlay_item.set_enabled(next.enabled);
        self.stats_overlay_item.set_checked(next.overlay_stats);
        self.stats_overlay_item.set_enabled(next.enabled);
        self.crosshair_item.set_checked(next.overlay_crosshair);
        self.crosshair_item.set_enabled(next.enabled);
        // Nothing to position when every overlay is off, and the item turns into a status
        // line while the editor is open so the menu explains what is on screen.
        let any_overlay = next.overlay_server || next.overlay_stats || next.overlay_crosshair;
        self.edit_item
            .set_enabled(next.enabled && any_overlay && !next.overlay_editing);
        if next.overlay_editing {
            self.edit_item.set_text("Editing Overlay…");
        } else {
            self.edit_item.set_text("Edit Overlay Position…");
        }
        for item in &self.page_items {
            item.set_enabled(next.enabled && next.signed_in);
        }
        self.status_item.set_text(next.status_line.clone());
        self.server_item.set_text(if next.server_line.is_empty() {
            "Server: not connected".to_string()
        } else {
            next.server_line.clone()
        });
        self.copy_job_item.set_enabled(!next.job_id.is_empty());
        // A hop is only offered mid-game: idle, or a session whose server has not parsed
        // yet, leaves the submenu grayed out instead of hiding it (a menu that moves is
        // harder to use than one that is visibly unavailable).
        let hop_ready = next.enabled && next.hop_enabled;
        for item in &self.hop_items {
            item.set_enabled(hop_ready);
        }
        if let Some(menu) = &self.hop_menu {
            menu.set_enabled(hop_ready);
        }
        self.close_roblox_item
            .set_enabled(next.enabled && next.roblox_running);
        for (slot, item) in self.account_items.iter().enumerate() {
            match next.accounts.get(slot) {
                Some(name) => {
                    item.set_text(name.clone());
                    item.set_enabled(next.enabled && next.signed_in);
                }
                None => {
                    item.set_text("No saved account");
                    item.set_enabled(false);
                }
            }
        }
        self.preset_quality_item.set_enabled(next.enabled);
        self.preset_competitive_item.set_enabled(next.enabled);
        self.hide_item
            .set_enabled(next.enabled && next.window_visible);
        // Always usable: with Roblox fullscreen the window can be "visible" yet behind the
        // game, and clicking this raises CREBLOX over it.
        self.show_item.set_enabled(next.enabled);
        if next.window_visible {
            self.show_item.set_text("Open CREBLOX");
        } else {
            self.show_item
                .set_text(format!("Open CREBLOX — {}", next.status_line));
        }
    }

    /// Whether the status item actually exists. When it does not, hiding the window would
    /// leave a running app with no way back in, so callers keep the window instead.
    pub fn available(&self) -> bool {
        self.icon.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn show_balloon(&self, title: &str, body: &str) {
        let enabled = self
            .shared
            .lock()
            .map(|g| !g.shutdown && g.desired.enabled)
            .unwrap_or(false);
        if !enabled {
            return;
        }
        let script = format!(
            "display notification {} with title {}",
            applescript_string(body),
            applescript_string(title)
        );
        let _ = Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .spawn();
    }

    pub fn shutdown(&self) {
        {
            if let Ok(mut guard) = self.shared.lock() {
                guard.shutdown = true;
                guard.desired.enabled = false;
            }
        }
        if let Ok(mut slot) = self.icon.lock() {
            slot.take();
        }
        if let Ok(mut slot) = self.join.lock() {
            if let Some(handle) = slot.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for TrayController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn applescript_string(raw: &str) -> String {
    let escaped = raw
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ");
    format!("\"{escaped}\"")
}

/// Builds the menu-bar image as a **template**: the mark's brightness becomes alpha and
/// the RGB becomes black, so AppKit renders it black on a light menu bar and white on a
/// dark one.
///
/// `assets/logo.png` is a dark opaque square with a cyan "C". Used directly it showed up as
/// a dark block on a Dark Mode menu bar and as a mismatched rectangle on a light one; as a
/// template it reads like a normal macOS status item on Catalina either way. The background
/// is subtracted rather than used as-is, otherwise its (non-zero) luminance would paint a
/// faint grey tile behind the mark.
fn load_tray_icon() -> Option<Icon> {
    const BACKGROUND_LUMA: i32 = 60;
    let img = image::load_from_memory(include_bytes!("../assets/logo.png"))
        .ok()?
        .to_rgba8();
    let resized = image::imageops::resize(&img, 22, 22, image::imageops::FilterType::Lanczos3);
    let (w, h) = resized.dimensions();
    let mut rgba = resized.into_raw();
    for px in rgba.chunks_exact_mut(4) {
        let luma = px[0].max(px[1]).max(px[2]) as i32;
        let alpha = ((luma - BACKGROUND_LUMA).max(0) * 255 / (255 - BACKGROUND_LUMA)).min(255);
        px[0] = 0;
        px[1] = 0;
        px[2] = 0;
        px[3] = alpha as u8;
    }
    Icon::from_rgba(rgba, w, h).ok()
}

// ---------------------------------------------------------------------------
// Bring the launcher to the front, even while Roblox is fullscreen
// ---------------------------------------------------------------------------

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn objc_msgSend();
}

/// Makes CREBLOX the active app. Without this, clicking the menu bar while Roblox
/// owns the screen (especially a fullscreen Space) can leave the launcher behind
/// the game; `activateIgnoringOtherApps` switches to CREBLOX's Space.
pub fn activate_app() {
    use std::ffi::CString;
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;

    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let Ok(sel_shared) = CString::new("sharedApplication") else {
            return;
        };
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(sel_shared.as_ptr()));
        if app.is_null() {
            return;
        }
        let Ok(sel_activate) = CString::new("activateIgnoringOtherApps:") else {
            return;
        };
        let activate: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        activate(app, sel_registerName(sel_activate.as_ptr()), 1);
        // A hidden (`Cmd+H`) or accessory-policy app is not restored by activation alone:
        // the Dock icon stays visible, the window stays gone and every click looks dead.
        let Ok(sel_policy) = CString::new("setActivationPolicy:") else {
            return;
        };
        activate(app, sel_registerName(sel_policy.as_ptr()), 0); // Regular: back in the Dock
        let Ok(sel_unhide) = CString::new("unhide:") else {
            return;
        };
        activate(app, sel_registerName(sel_unhide.as_ptr()), 0);

        // Activation alone can leave the launcher window hidden behind Roblox (or still
        // ordered out after "hide to menu bar"), so raise every window explicitly.
        let Ok(sel_windows) = CString::new("windows") else {
            return;
        };
        let windows: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let list = windows(app, sel_registerName(sel_windows.as_ptr()));
        if list.is_null() {
            return;
        }
        let Ok(sel_count) = CString::new("count") else {
            return;
        };
        let count = windows(list, sel_registerName(sel_count.as_ptr())) as usize;
        let Ok(sel_at) = CString::new("objectAtIndex:") else {
            return;
        };
        let Ok(sel_front) = CString::new("makeKeyAndOrderFront:") else {
            return;
        };
        let Ok(sel_deminiaturize) = CString::new("deminiaturize:") else {
            return;
        };
        let Ok(sel_can_key) = CString::new("canBecomeKeyWindow") else {
            return;
        };
        type MsgBool = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8;
        let can_key: MsgBool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let object_at: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        for i in 0..count {
            let window = object_at(list, sel_registerName(sel_at.as_ptr()), i as isize);
            if window.is_null() {
                continue;
            }
            // Skip AppKit's own helper windows (status item, menus): they cannot become
            // key, and raising them does not bring the launcher back.
            if can_key(window, sel_registerName(sel_can_key.as_ptr())) == 0 {
                continue;
            }
            activate(window, sel_registerName(sel_deminiaturize.as_ptr()), 0);
            activate(window, sel_registerName(sel_front.as_ptr()), 0);
        }
    }
}

/// How many real windows this process owns (`canBecomeKeyWindow` filters out the status
/// item and menu helper windows). Zero means there is nothing to restore - a headless copy,
/// or a viewport that never got created - so the only way back in is a new launcher.
fn key_window_count() -> usize {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;
    type MsgCount = unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize;
    type MsgBool = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8;

    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return 0;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return 0;
        }
        let windows: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let list = windows(app, sel_registerName(c"windows".as_ptr()));
        if list.is_null() {
            return 0;
        }
        let count: MsgCount = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let n = count(list, sel_registerName(c"count".as_ptr()));
        let object_at: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let bool_msg: MsgBool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let can_key = sel_registerName(c"canBecomeKeyWindow".as_ptr());
        let mut real = 0usize;
        for i in 0..n {
            let window = object_at(list, sel_registerName(c"objectAtIndex:".as_ptr()), i as isize);
            if !window.is_null() && bool_msg(window, can_key) != 0 {
                real += 1;
            }
        }
        real
    }
}

/// Starts a fresh launcher from the enclosing `.app` bundle (`open -n` also works when the
/// bundle is already running, so a duplicate is never a problem).
fn relaunch_app_bundle() {
    let bundle = std::env::current_exe().ok().and_then(|exe| {
        let app = exe.parent()?.parent()?.parent()?.to_path_buf();
        app.extension().is_some_and(|e| e == "app").then_some(app)
    });
    match bundle {
        Some(bundle) => {
            let _ = Command::new("/usr/bin/open").arg("-n").arg(bundle).status();
        }
        None => {
            if let Ok(exe) = std::env::current_exe() {
                let _ = Command::new(exe).spawn();
            }
        }
    }
}

/// Runs `f` on the macOS main thread, even when the egui event loop is not drawing frames.
///
/// This is the only way to restore a window while eframe is idle: `ctx.request_repaint()`
/// cannot wake a viewport that is ordered out, so a channel message sits unread until the
/// user happens to trigger something else. `dispatch_async_f` has existed since 10.6, so it
/// is safe on 10.15, and it is the same thing `performSelectorOnMainThread:` does.
pub fn dispatch_main<F: FnOnce() + Send + 'static>(f: F) {
    use std::os::raw::c_void;
    unsafe extern "C" {
        // `dispatch_get_main_queue()` is a C **macro** in <dispatch/queue.h> that expands to
        // `&_dispatch_main_q`; it is not an exported function. Declaring it as one compiles
        // (and even passes `cargo check`) but fails at `ld` with
        //   "_dispatch_get_main_queue, referenced from ..."
        // The real libSystem symbol is the global queue object below.
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *mut c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
    }
    unsafe extern "C" fn trampoline<F: FnOnce()>(ctx: *mut c_void) {
        let f = unsafe { Box::from_raw(ctx as *mut F) };
        f();
    }
    let boxed = Box::into_raw(Box::new(f));
    unsafe {
        let queue = std::ptr::addr_of!(_dispatch_main_q) as *mut c_void;
        dispatch_async_f(queue, boxed.cast(), trampoline::<F>);
    }
}

/// Hides the launcher **without** freezing the egui event loop.
///
/// `ViewportCommand::Visible(false)` orders the window out, and an ordered-out window gives
/// winit nothing to do: `App::update()` stops being called, so every queued message - "a
/// game ended, come back", "the Dock was clicked" - waits forever. That is exactly the log:
/// `dock: reopen requested while the app had no visible window` twice, and never a single
/// `window: restoring the launcher`. `[NSApp hide:]` hides the app while the window stays
/// ordered in, so the loop keeps ticking, the messages are read, and the Dock icon behaves
/// like any other app's.
pub fn hide_launcher_window() {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return;
        }
        let msg: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        msg(app, sel_registerName(c"hide:".as_ptr()), 0);
    }
    crate::auth::log("window: hidden to the menu bar (NSApp hide, event loop stays alive)");
}

/// Puts the launcher back on screen, from the main thread, without needing `App::update()`.
///
/// Same AppKit sequence Apple's own reopen path performs: Regular policy (back in the Dock),
/// `unhide:`, `deminiaturize:` + `makeKeyAndOrderFront:` per real window, then
/// `activateIgnoringOtherApps:`. Safe to call from a `dispatch_async_f` block.
pub fn show_launcher_window() {
    crate::auth::log("window: restoring the launcher");
    activate_app();
}

/// Whether the whole app is hidden (`[NSApp isHidden]`).
///
/// A hidden app still owns ordered-in windows, so "no visible window" alone cannot detect
/// it - and the watcher has to treat a hidden app as "the launcher is not on screen".
pub fn app_is_hidden() -> bool {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgBool = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8;
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return false;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return false;
        }
        let is_hidden: MsgBool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        is_hidden(app, sel_registerName(c"isHidden".as_ptr())) != 0
    }
}

/// Sets the AppKit activation policy (`0` = Regular/Dock icon, `1` = Accessory/none).
fn set_activation_policy(policy: isize) {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return;
        }
        let set_policy: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        set_policy(app, sel_registerName(c"setActivationPolicy:".as_ptr()), policy);
    }
}

/// Keeps the headless `--companion` copy out of the Dock.
///
/// It has no window and no delegate, so a Regular policy there is a running icon whose dot
/// can never be satisfied: clicking it activates an app with nothing to show. Accessory
/// removes the icon entirely, which is what a background helper should look like.
pub fn set_accessory_policy() {
    set_activation_policy(1);
}

static REPAINT_CTX: OnceLock<egui::Context> = OnceLock::new();

/// Work that has to happen before the process goes away (stopping the overlay helper).
static QUIT_CLEANUP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Registers the cleanup the tray's quit path runs before exiting.
pub fn set_quit_cleanup(cleanup: impl Fn() + Send + Sync + 'static) {
    let _ = QUIT_CLEANUP.set(Box::new(cleanup));
}

/// Quits from the tray callback itself.
///
/// A menu click is delivered on the main thread, which is exactly where AppKit expects a
/// termination to come from. Waiting for `App::update()` to read a queued `QuitApp` message
/// is what made the item look dead while the launcher was tucked away: with no visible
/// window eframe stops ticking, so the message was never read.
pub fn quit_now() -> ! {
    crate::auth::log("tray: quitting from the menu callback");
    if let Some(cleanup) = QUIT_CLEANUP.get() {
        cleanup();
    }
    std::process::exit(0)
}

/// Wakes egui without restoring or focusing the window.
///
/// For a background callback (toolkit-level event, memory pressure) that changed state a
/// frame has to pick up. Unlike [`wake_launcher`] it leaves a tucked-away window alone.
pub fn request_repaint_now() {
    if let Some(ctx) = REPAINT_CTX.get() {
        ctx.request_repaint();
    }
}

/// Brings the launcher back from the tray callback, without waiting for a frame.
///
/// Drops the tucked-in flag first (the Dock watcher and the reopen handler read it), then
/// performs the AppKit restore and wakes egui so `App::update()` syncs the viewport.
pub fn wake_launcher() {
    crate::window_lifecycle::set_hidden_to_tray(false);
    show_launcher_window();
    if let Some(ctx) = REPAINT_CTX.get() {
        ctx.request_repaint();
    }
}

/// Whether CREBLOX is living in the menu bar on purpose (`window_lifecycle`).
///
/// While it is, nothing here may put the window back: not a finished game, not a Dock click,
/// not an app activation. "Open CREBLOX" is the one way out, and it travels through the
/// app's own event channel.
fn tucked_in_menu_bar() -> bool {
    crate::window_lifecycle::hidden_to_tray()
}

/// Adds `applicationShouldHandleReopen:hasVisibleWindows:` to the delegate winit installed.
///
/// winit never implements it, so a Dock click on an ordered-out window is silently dropped:
/// AppKit activates the app, finds nothing to show and stops - the icon keeps its dot and
/// every click looks dead. Adding the method to the *existing* delegate class (rather than
/// replacing the delegate) leaves all of winit's own callbacks untouched, and returning
/// `YES` keeps AppKit's default behaviour as a second chance.
pub fn install_dock_reopen_handler() {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;

    unsafe extern "C" fn reopen(
        _this: *mut c_void,
        _sel: *mut c_void,
        _app: *mut c_void,
        _visible: i8,
    ) -> i8 {
        if tucked_in_menu_bar() {
            // A menu-bar app has no Dock icon, so this is a launch reactivation (Finder, an
            // `open` of the bundle, a second instance). It must not pull the window back.
            crate::auth::log("dock: reopen ignored - CREBLOX stays in the menu bar");
            return 0;
        }
        crate::auth::log("dock: applicationShouldHandleReopen - restoring the launcher");
        activate_app();
        if let Some(ctx) = REPAINT_CTX.get() {
            ctx.request_repaint();
        }
        1
    }

    unsafe extern "C" {
        fn class_addMethod(
            cls: *mut c_void,
            name: *mut c_void,
            imp: *mut c_void,
            types: *const std::ffi::c_char,
        ) -> i8;
        fn object_getClass(obj: *mut c_void) -> *mut c_void;
    }

    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return;
        }
        let delegate = shared(app, sel_registerName(c"delegate".as_ptr()));
        if delegate.is_null() {
            crate::auth::log("dock: no NSApplication delegate yet - relying on the watcher");
            return;
        }
        let delegate_class = object_getClass(delegate);
        if delegate_class.is_null() {
            return;
        }
        let added = class_addMethod(
            delegate_class,
            sel_registerName(c"applicationShouldHandleReopen:hasVisibleWindows:".as_ptr()),
            reopen as *const () as *mut c_void,
            c"c@:@c".as_ptr(),
        );
        // `class_addMethod` returns NO when the class already implements it (an older or
        // patched winit): the watcher thread below still covers that case.
        crate::auth::log(if added != 0 {
            "dock: reopen handler installed"
        } else {
            "dock: reopen handler already present - relying on the watcher"
        });
    }
}

static REOPEN_WATCHER: OnceLock<()> = OnceLock::new();

/// Reopens the launcher when the app is activated while no window is on screen.
///
/// macOS reports a Dock click as an app activation, not as an egui event, and eframe stops
/// ticking once the window is ordered out - so the in-`update()` checks could never run in
/// exactly the state they exist for (running Dock dot, no window). This thread is the only
/// thing guaranteed to be alive then, and it costs one `isActive` call plus one window-list
/// walk every 400 ms.
///
/// It only ever restores a window the user hid themselves. A launcher tucked into the menu bar
/// for a game is skipped entirely: there, "Open CREBLOX" is the only way back.
pub fn start_reopen_watcher(tx: Sender<Msg>, ctx: egui::Context) {
    if REOPEN_WATCHER.set(()).is_err() {
        return;
    }
    let _ = REPAINT_CTX.set(ctx.clone());
    let spawned = thread::Builder::new()
        .name("creblox-reopen".to_string())
        .spawn(move || {
            let mut idle_checks = 0u8;
            let mut last_restore: Option<std::time::Instant> = None;
            // Never act on the first seconds: the window is created during startup, and a
            // watcher that fires before it exists would restart a healthy process.
            let started = std::time::Instant::now();
            loop {
                // 400 ms keeps "left the game -> launcher back" under a second while the
                // hidden window is doing nothing; the checks are two objc calls.
                thread::sleep(Duration::from_millis(400));
                // Either state means "the launcher is not on screen": an ordered-out window
                // (the old hide) or a hidden app (the new one, whose window stays ordered in).
                let no_window = !any_window_visible() || app_is_hidden();
                if !no_window {
                    idle_checks = 0;
                    continue;
                }
                // Tucked into the menu bar on purpose: a finished game, a Dock click and
                // an app activation are all explicitly not a request to reopen. The watcher
                // exists for a window the *user* hid; a game tuck is undone only by
                // "Open CREBLOX", which arrives through the app's own channel.
                if tucked_in_menu_bar() {
                    idle_checks = 0;
                    continue;
                }
                // "No visible window" alone is not enough: hiding to the menu bar also
                // leaves no window, and the app is inactive then, so requiring an active app
                // means the user really did click the Dock icon or the app in Finder.
                let dock_click = app_is_active();
                if !dock_click {
                    idle_checks = 0;
                    continue;
                }
                idle_checks = idle_checks.saturating_add(1);
                // Two consecutive checks (~0.8 s): activation lags behind a hide, and a
                // single sample could snatch the window back the instant it was hidden.
                if idle_checks < 2 {
                    continue;
                }
                if last_restore.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
                    continue;
                }
                last_restore = Some(std::time::Instant::now());
                // Nothing to restore anywhere: this process owns no window at all. Hand over
                // to a fresh launcher from the bundle (the same thing `--companion` does
                // when it finishes) instead of leaving a Dock dot that can never open.
                if key_window_count() == 0 && started.elapsed() > Duration::from_secs(5) {
                    crate::auth::log("dock: no window in this process - relaunching the launcher");
                    dispatch_main(relaunch_app_bundle);
                    thread::sleep(Duration::from_millis(800));
                    std::process::exit(0);
                }
                crate::auth::log("dock: reopen requested while the app had no visible window");
                // AppKit does the real work, on the main thread, right now. Sending the
                // message alone was the bug: with no visible window eframe never ticks, so
                // `request_repaint()` wakes nothing and the message waits forever.
                dispatch_main(show_launcher_window);
                let _ = tx.send(Msg::TrayAction(TrayEvent::ShowAndFocus));
                ctx.request_repaint();
            }
        });
    if let Err(e) = spawned {
        crate::auth::log(&format!("dock: reopen watcher failed to start: {e}"));
    }
}

/// Whether CREBLOX itself is the active (front) app. Used to notice a click on the Dock
/// icon while the window is hidden: AppKit activates the app, but winit never reopens a
/// window that was ordered out, so nothing used to happen.
pub fn app_is_active() -> bool {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    // `isActive` returns BOOL (signed char): read it as such, not as a pointer, so the
    // undefined upper bits of the return register cannot report a false positive.
    type MsgBool = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8;

    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return false;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return false;
        }
        let is_active: MsgBool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        is_active(app, sel_registerName(c"isActive".as_ptr())) != 0
    }
}

/// Whether the launcher has a window that can actually be brought back.
///
/// A minimized window counts: it is still in the Dock. An ordered-out window does not,
/// which is how a dropped `Visible(true)` used to leave a running CREBLOX with no UI.
/// The status-item and menu helper windows are ignored (`canBecomeKeyWindow` is false).
pub fn any_window_visible() -> bool {
    use std::os::raw::c_void;
    type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;
    type MsgCount = unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize;
    type MsgBool = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8;

    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return false;
        }
        let shared: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return false;
        }
        let windows: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let list = windows(app, sel_registerName(c"windows".as_ptr()));
        if list.is_null() {
            return false;
        }
        let count: MsgCount = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let n = count(list, sel_registerName(c"count".as_ptr()));
        let object_at: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let bool_msg: MsgBool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        for i in 0..n {
            let window = object_at(
                list,
                sel_registerName(c"objectAtIndex:".as_ptr()),
                i as isize,
            );
            if window.is_null() {
                continue;
            }
            if bool_msg(window, sel_registerName(c"canBecomeKeyWindow".as_ptr())) == 0 {
                continue;
            }
            let visible = bool_msg(window, sel_registerName(c"isVisible".as_ptr())) != 0;
            let miniaturized = bool_msg(window, sel_registerName(c"isMiniaturized".as_ptr())) != 0;
            if visible || miniaturized {
                return true;
            }
        }
        false
    }
}

pub fn clamp_tray_tooltip(raw: &str) -> String {
    let trimmed = raw.trim();
    let src = if trimmed.is_empty() {
        "CREBLOX"
    } else {
        trimmed
    };
    // macOS tooltips are short; keep a tidy single line.
    let mut out: String = src.chars().take(96).collect();
    if out.trim().is_empty() {
        out = "CREBLOX".to_string();
    }
    out
}

pub fn should_start_in_tray(args: &[String], tray_enabled: bool) -> bool {
    tray_enabled
        && args.iter().skip(1).any(|a| {
            matches!(
                a.trim().to_ascii_lowercase().as_str(),
                "--tray" | "--minimized" | "--start-minimized"
            )
        })
}

pub fn build_startup_command(exe_path: &Path, start_minimized: bool) -> String {
    if start_minimized {
        format!("\"{}\" --tray", exe_path.display())
    } else {
        format!("\"{}\"", exe_path.display())
    }
}

pub fn parse_startup_command(cmd: &str) -> Option<(String, bool)> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (exe, rest) = if let Some(after_quote) = trimmed.strip_prefix('"') {
        let end = after_quote.find('"')?;
        (&after_quote[..end], &after_quote[end + 1..])
    } else {
        match trimmed.find(char::is_whitespace) {
            Some(idx) => (&trimmed[..idx], &trimmed[idx..]),
            None => (trimmed, ""),
        }
    };
    if exe.is_empty() {
        return None;
    }
    let minimized = rest.split_whitespace().any(|flag| {
        matches!(
            flag.to_ascii_lowercase().as_str(),
            "--tray" | "--minimized" | "--start-minimized"
        )
    });
    Some((exe.to_string(), minimized))
}

fn launch_agent_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(env::var("HOME").ok()?)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist")),
    )
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn launch_agent_plist(exe: &Path, start_minimized: bool) -> String {
    let mut args = format!(
        "        <string>{}</string>\n",
        xml_escape(&exe.display().to_string())
    );
    if start_minimized {
        args.push_str("        <string>--tray</string>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{LAUNCH_AGENT_LABEL}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n{args}    </array>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
    <key>KeepAlive</key>\n\
    <false/>\n\
    <key>ProcessType</key>\n\
    <string>Interactive</string>\n\
</dict>\n\
</plist>\n"
    )
}

pub fn is_startup_enabled() -> bool {
    launch_agent_path().is_some_and(|p| p.exists())
}

pub fn set_startup(enabled: bool, start_minimized: bool) -> Result<(), String> {
    let path = launch_agent_path().ok_or_else(|| "Could not find home folder".to_string())?;
    if !enabled {
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, launch_agent_plist(&exe, start_minimized)).map_err(|e| e.to_string())
}

pub fn sync_startup_path_if_enabled(start_minimized: bool) {
    if !is_startup_enabled() {
        return;
    }
    let Ok(exe) = env::current_exe() else {
        return;
    };
    let Some(path) = launch_agent_path() else {
        return;
    };
    let desired = launch_agent_plist(&exe, start_minimized);
    if fs::read_to_string(&path).ok().as_deref() != Some(desired.as_str()) {
        let _ = fs::write(path, desired);
    }
}

fn instance_socket_path() -> PathBuf {
    crate::auth::data_dir().join(".instance.sock")
}

/// Asks the already-running CREBLOX to show and focus its window. Returns false when
/// nothing is listening (stale socket, crash, older build), so the caller can decide what
/// to do instead of silently exiting.
pub fn notify_running_instance_show() -> bool {
    let path = instance_socket_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    stream.write_all(b"show\n").is_ok() && stream.flush().is_ok()
}

static INSTANCE_LISTENER_STARTED: OnceLock<()> = OnceLock::new();

/// Listens on a local socket so a second launch (Finder, Dock, `open`, Spotlight) can ask
/// this instance to show its window instead of just exiting. This replaces the old
/// AppleScript-only handoff, which needed Accessibility permission and did nothing when the
/// window was hidden - the app looked open but clicking it did nothing.
pub fn start_instance_listener(tx: Sender<Msg>, ctx: egui::Context) {
    if INSTANCE_LISTENER_STARTED.set(()).is_err() {
        return;
    }
    let path = instance_socket_path();
    let _ = fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            crate::auth::log(&format!(
                "instance: could not listen on {}: {e}",
                path.display()
            ));
            return;
        }
    };
    let spawned = thread::Builder::new()
        .name("creblox-instance".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut buf = [0u8; 32];
                let n = stream.read(&mut buf).unwrap_or(0);
                let cmd = String::from_utf8_lossy(&buf[..n]);
                if cmd.trim() == "show" {
                    crate::auth::log("instance: another launch asked us to show the window");
                    let _ = tx.send(Msg::TrayAction(TrayEvent::ShowAndFocus));
                    ctx.request_repaint();
                }
            }
        });
    if let Err(e) = spawned {
        crate::auth::log(&format!("instance: listener thread failed: {e}"));
    }
}

/// A second copy of CREBLOX should hand off to the running instance instead of
/// opening a duplicate window. `flock` gives us a cheap process-lifetime lock; the socket
/// lets the already-running instance actually show itself.
pub fn try_focus_existing_instance() -> bool {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    let path = crate::auth::data_dir().join(".instance.lock");
    let Ok(file) = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
    else {
        return false;
    };
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        // Hold the lock for the rest of the process lifetime.
        std::mem::forget(file);
        return false;
    }
    crate::auth::log("instance: another CREBLOX is running, asking it to show");
    if notify_running_instance_show() {
        return true;
    }
    // Fallbacks for an instance started by an older build (no socket): activating the
    // bundle makes the running app frontmost, and CREBLOX shows its own window when it
    // notices it became active.
    let _ = Command::new("/usr/bin/open")
        .args(["-b", "com.creblox.app"])
        .status();
    let _ = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "tell application \"System Events\" to set frontmost of (first process whose name is \"creblox\") to true",
        ])
        .status();
    // The process is still the only instance: never start a duplicate that would fight
    // over the status item and the overlay process.
    true
}
