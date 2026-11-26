//! macOS menu-bar (status item) tray, login item and single-instance support.
//!
//! The status item and its menu are created on the main thread, inside
//! `TrayController::new`, which eframe calls while starting the AppKit event
//! loop. Menu clicks are forwarded to the app over the existing `Msg` channel.

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
        _ => id
            .strip_prefix(ITEM_ACCOUNT_PREFIX)
            .and_then(|slot| slot.parse::<usize>().ok())
            .filter(|slot| *slot < ACCOUNT_SLOTS)
            .map(TrayEvent::SwitchAccountSlot),
    }
}

pub struct TrayController {
    icon: Mutex<Option<TrayIcon>>,
    show_item: MenuItem,
    hide_item: MenuItem,
    page_items: Vec<MenuItem>,
    status_item: MenuItem,
    server_item: MenuItem,
    copy_job_item: MenuItem,
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
                            let _ = tx.send(Msg::TrayAction(action));
                            fired = true;
                        }
                    }
                    while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = ev
                        {
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
                out.push(action);
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

static REOPEN_WATCHER: OnceLock<()> = OnceLock::new();

/// Reopens the launcher when the Dock icon is clicked while no window is on screen.
///
/// macOS reports a Dock click as an app activation, not as an egui event, and eframe stops
/// ticking once the window is ordered out - so the in-`update()` checks could never run in
/// exactly the state they exist for (running Dock dot, no window). That is the "clicking the
/// Dock icon does nothing after leaving a game" bug: the watchdog was sound but asleep.
/// This thread is the only thing guaranteed to be alive then, and it costs one `isActive`
/// call plus one window-list walk every 700 ms.
pub fn start_reopen_watcher(tx: Sender<Msg>, ctx: egui::Context) {
    if REOPEN_WATCHER.set(()).is_err() {
        return;
    }
    let spawned = thread::Builder::new()
        .name("creblox-reopen".to_string())
        .spawn(move || {
            let mut idle_checks = 0u8;
            let mut last_restore: Option<std::time::Instant> = None;
            loop {
                thread::sleep(Duration::from_millis(700));
                // "No visible window" alone is not enough: hiding to the menu bar also
                // leaves no window, and the app is inactive then. Requiring an active app
                // means the user really did click the Dock icon / the app in Finder.
                let needs_reopen = app_is_active() && !any_window_visible();
                if !needs_reopen {
                    idle_checks = 0;
                    continue;
                }
                idle_checks = idle_checks.saturating_add(1);
                // Two consecutive checks (~1.4 s): activation lags behind a hide, and a
                // single sample could snatch the window back the instant it was hidden.
                if idle_checks < 2 {
                    continue;
                }
                if last_restore.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
                    continue;
                }
                last_restore = Some(std::time::Instant::now());
                crate::auth::log("dock: reopen requested while the app had no visible window");
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
