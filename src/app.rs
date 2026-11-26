use eframe::egui::{self, RichText, Sense};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use crate::api::*;
use crate::auth::*;
use crate::avatar_3d::{self, Avatar3DState, load_avatar_3d_data_ex};
use crate::discord_rpc::{DiscordActivity, DiscordRpcClient, DiscordRpcConfig};
use crate::fastflags::{FastFlagsConfig, load_fastflags, save_fastflags, sync_client_fastflags};
use crate::roblox_process::*;
use crate::theme::*;
use crate::tray::{
    TrayController, TrayEvent, TrayState, is_startup_enabled, sync_startup_path_if_enabled,
};
use crate::types::*;
use crate::ui::menu::*;
use crate::ui::widgets::*;
use crate::updater::*;
use crate::video_preview;

pub struct Creblox {
    pub ctx: egui::Context,
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    pub sid: String,

    pub page: Page,
    pub back: Page,
    pub status: String,
    pub settings: Settings,
    /// Last value seen by the Settings page's "did anything change" check. Comparing
    /// against this avoids deep-cloning the whole struct on every frame the page is open.
    pub settings_snapshot: Settings,
    /// Cached tails of the two log files shown on the Logs page, refreshed at most once a
    /// second instead of being re-read from disk on every frame.
    pub logs_view: Option<(String, String)>,
    pub logs_view_at: Option<Instant>,
    pub player_path: Option<PathBuf>,
    pub handler: String,

    pub cookie: Option<String>,
    pub quick_login: Option<QuickLoginState>,

    pub search: String,
    pub searched: String,
    pub searching: bool,
    pub results: Option<Result<Vec<Game>, String>>,
    pub manual_game_open: bool,
    pub manual_game_input: String,
    pub manual_game_loading: bool,
    pub manual_game_focus_requested: bool,
    pub manual_game_err: Option<String>,
    pub active_private_server: Option<(u64, u64, String)>,

    pub sorts: Option<Result<Vec<Sort>, String>>,
    pub chart: usize,
    /// Which settings section is open (0 accounts ... 7 updates). Runtime only.
    pub settings_section: u8,
    /// Chart filters: minimum players online and minimum like percentage.
    pub chart_min_players: u32,
    pub chart_min_rating: u8,
    /// How many games each Home row shows; grows as you keep asking for more.
    /// Charts feed: natural order until the user asks for a shuffle.
    pub feed_shuffled: bool,
    /// Order used by the friends list that the home page "+" opens.
    pub friends_sort: FriendsSort,
    /// Games feed cache. Rebuilt only when a filter (or the data) changes, so drawing the
    /// feed never sorts or clones the catalogue again.
    pub feed_pool: Vec<Game>,
    pub feed_sig: u64,
    /// Recently loaded profiles by user id, so going back to a profile is instant.
    pub profile_cache: HashMap<u64, (FriendProfile, Instant)>,
    /// Set when a section button is clicked, consumed by the page scroll area.
    pub scroll_to_top: bool,
    /// How many random games the charts feed currently shows (grows as you scroll).
    pub feed_rows: usize,
    /// Seed of the random game order; "Shuffle" changes it.
    pub feed_salt: u64,
    /// Minimum visit count filter for the games feed.
    pub chart_min_visits: u64,
    /// Visit counts per universe id, fetched in the background.
    pub game_visits: HashMap<u64, u64>,
    pub visits_requested: HashSet<u64>,

    pub recent_ids: Vec<u64>, // place ids
    pub recent: Vec<Game>,
    pub fav_ids: Vec<u64>, // place ids
    pub favs: Vec<Game>,

    pub wide: HashMap<u64, String>,
    pub icon: HashMap<u64, String>,
    pub requested: HashMap<u64, Instant>,
    pub game_video_ids: HashMap<u64, Option<u64>>,
    pub video_id_requested: HashSet<u64>,
    pub decoded_videos: HashMap<u64, Arc<video_preview::DecodedPreviewVideo>>,
    pub video_lru: Vec<u64>,
    pub video_decode_requested: HashSet<u64>,
    pub hovered_game_id: Option<u64>,
    pub hover_start: Option<Instant>,
    pub hover_play_start: Option<Instant>,
    pub active_video_texture: Option<(u64, u64, egui::TextureHandle)>,

    pub sorts_sync_started: Option<Instant>,
    pub saved_sync_started: Option<Instant>,
    pub auth_sync_started: Option<Instant>,
    pub last_sorts_sync: Option<Instant>,
    pub last_saved_sync: Option<Instant>,
    pub last_auth_sync: Option<Instant>,
    /// Only a refresh the user asked for may show the home loading spinner;
    /// background auto-syncs stay invisible.
    pub home_sync_spinner: bool,
    pub just_launched: Vec<(u64, Instant)>,
    pub pending_favs: HashMap<u64, (bool, Instant)>,
    pub was_focused: bool,
    pub auto_close_flag: Arc<AtomicBool>,
    pub watcher_stop: Arc<AtomicBool>,
    pub last_roblox_launch_ms: Arc<AtomicU64>,

    pub friend_names: Vec<String>,
    pub friends: Vec<User>,
    pub friends_err: Option<String>,
    pub adding_friend: bool,
    pub new_friend: String,
    pub friend_searched: String,
    pub friend_searching: bool,
    pub friend_search_loading_more: bool,
    pub friend_search_cursor: Option<String>,
    pub friend_search_verified: HashSet<u64>,
    pub friend_search_results: Option<Result<Vec<User>, String>>,
    pub friend_sent: HashSet<u64>,
    pub social_tab: SocialTab,
    pub social_lists: HashMap<SocialTab, Vec<User>>,
    pub social_cursors: HashMap<SocialTab, Option<String>>,
    pub social_loading: HashSet<SocialTab>,
    pub social_loading_more: HashSet<SocialTab>,
    pub social_errs: HashMap<SocialTab, String>,
    pub social_responding: HashSet<u64>,
    pub toasts: Vec<ToastNotification>,
    pub next_toast_id: u64,
    pub known_friend_requests: Option<HashSet<u64>>,
    pub last_requests_poll: Option<Instant>,
    pub update_checked: bool,
    pub notification_history: Vec<NotificationItem>,
    pub roblox_update_available: Option<(String, String)>,
    pub seen_notifications: SeenNotifications,
    pub notified_kill_pids: HashSet<u32>,
    pub last_autokill_toast: Option<Instant>,
    pub app_update_available: Option<CrebloxRelease>,
    pub app_update_checking: bool,
    pub app_update_checked: bool,
    pub app_update_status: Option<String>,
    pub app_update_downloading: bool,
    pub app_update_progress: Option<(Arc<AtomicU64>, Arc<AtomicU64>)>,
    pub app_update_zip: Option<PathBuf>,
    pub app_update_installing: bool,
    pub app_update_error: Option<String>,
    pub update_token_input: String,
    pub update_token_saved: bool,
    pub app_patch_applied: Option<String>,
    pub app_staged_patch: Option<PathBuf>,

    pub username: String,
    pub me: Option<Profile>,
    pub me_err: Option<String>,

    pub my_profile: Option<FriendProfile>,
    pub my_profile_loading: bool,
    pub my_profile_err: Option<String>,
    pub profile_bio_expanded: bool,
    pub profile_editing: bool,
    pub profile_edit_bio: String,
    pub profile_saving_bio: bool,
    pub profile_save_err: Option<String>,

    pub friend_profile: Option<FriendProfile>,
    pub friend_loading: bool,
    pub friend_err: Option<String>,

    pub avatar_state: Option<AvatarState>,
    pub avatar_palette: Vec<PaletteColor>,
    pub avatar_loading: bool,
    pub avatar_pending: u32,
    pub avatar_seq: u64,
    pub avatar_err: Option<String>,
    pub avatar_tab: AvatarTab,
    pub avatar_search: String,
    pub avatar_color_target: BodyColorTarget,
    pub avatar_items: HashMap<AvatarTab, Vec<AvatarAsset>>,
    pub avatar_cursors: HashMap<AvatarTab, Option<String>>,
    pub avatar_items_loading: Option<AvatarTab>,
    pub avatar_items_err: Option<String>,
    pub avatar_outfits: Option<Vec<OutfitItem>>,
    pub avatar_outfits_page: u32,
    pub avatar_outfits_more: bool,
    pub avatar_outfits_loading: bool,
    pub avatar_outfits_err: Option<String>,
    pub avatar_body_part_filter: BodyPartFilter,
    pub avatar_outfit_filter: OutfitFilter,
    pub avatar_save_outfit_open: bool,
    pub avatar_save_outfit_name: String,
    pub avatar_outfit_saving: bool,
    pub avatar_3d: Avatar3DState,

    pub game_tab: GameTab,
    pub game_extra: Option<GameExtra>,
    /// Last loaded details per universe. Reopening a game paints these immediately and
    /// only revalidates when they are older than the TTL, instead of blanking the page.
    pub game_extra_cache: HashMap<u64, (GameExtra, Instant)>,
    pub game_media_idx: usize,
    pub game_passes_loading: bool,
    pub game_passes_err: Option<String>,
    pub game_servers_loading: bool,
    pub game_servers_err: Option<String>,
    pub server_sort: ServerSort,
    pub server_exclude_full: bool,

    pub wallpaper_tex: Option<(String, egui::TextureHandle)>,
    pub wallpaper_loading: Option<String>,
    pub wallpaper_err: Option<String>,
    pub settings_advanced_open: bool,
    pub last_ram_trim_check: Option<Instant>,
    pub discord_rpc: DiscordRpcClient,
    pub app_started_epoch_secs: u64,
    pub active_roblox_place_id: Option<u64>,
    pub active_roblox_started_at: Option<u64>,
    /// Playtime per place ID, loaded once at startup and only written when a game ends.
    pub playtime: HashMap<u64, crate::playtime::GamePlay>,
    /// `(place id, start)` of the session being timed, if any.
    pub playtime_session: Option<(u64, Instant)>,
    pub server_info: Option<crate::roblox_process::ServerInfo>,
    pub server_locations: HashMap<String, String>,
    pub server_location_pending: HashSet<String>,
    pub game_overlay: crate::game_overlay::OverlayManager,
    pub pending_discord_launch: Option<(u64, Option<Game>, Instant)>,
    pub discord_resolved_games: HashMap<u64, (Game, Option<String>)>,
    pub discord_resolving_places: HashSet<u64>,
    pub tray: TrayController,
    pub window_visible: bool,
    /// Any Roblox process is running (watcher report). Unlike `active_roblox_place_id`
    /// this does not depend on parsing the player log, so it is what keeps the overlay
    /// helper alive (and responding to tray/settings changes) during a whole session.
    pub roblox_running: bool,
    pub tray_minimize_balloon_shown: bool,
    pub allow_close_exit: bool,
    /// CREBLOX hid its own window because a game was launched (it keeps running in the menu bar).
    pub hidden_for_game: bool,
    pub hidden_at: Option<Instant>,
    /// While hidden: whether the app has been inactive at least once since hiding. A click
    /// on the Dock icon flips inactive -> active, which reopens the window; the menu-bar
    /// click that hid it leaves the app active, so that alone does not undo the hide.
    pub hidden_seen_inactive: bool,
    pub last_hidden_check: Option<Instant>,
    /// Watchdog for "the launcher says it is on but no window is on screen".
    pub last_window_check: Option<Instant>,
    /// Checks the real AppKit window state after a restore, not just our own flag.
    pub last_visibility_check: Option<Instant>,
    pub last_overlay_check: Option<Instant>,
    /// The tray's "Edit Overlay Position" mode is open: the HUD is draggable and the
    /// overlay process is waiting for a Confirm / Esc.
    pub overlay_editing: bool,
    /// Last look at the editor's answer file.
    pub last_overlay_edit_check: Option<Instant>,
    /// Frames to wait after showing the window before raising it over Roblox.
    pub raise_frames: u8,
    pub pending_start_in_tray_frames: u8,
    pub startup_err: Option<String>,

    pub saved_accounts: Vec<StoredAccount>,
    pub fastflags: FastFlagsConfig,
    pub ff_new_key: String,
    pub ff_new_val: String,
    pub ff_json_buf: String,
    pub ff_json_open: bool,
    pub ff_err: Option<String>,
}

impl Creblox {
    pub fn new(cc: &eframe::CreationContext<'_>, start_in_tray: bool) -> Self {
        // Once an image is on the GPU, egui does not need its CPU-side pixels.
        // Dropping them frees tens of MB with thumbnails, wallpapers and video
        // frames loaded, and nothing in the app reads image data back.
        cc.egui_ctx.options_mut(|o| o.reduce_texture_memory = true);
        let (tx, rx) = channel();
        let sid = new_session_id();
        let cookie = load_cookie();
        let logged_in = cookie.is_some();
        let mut settings = load_settings();
        let startup_live = is_startup_enabled();
        if startup_live {
            sync_startup_path_if_enabled(settings.start_minimized_to_tray);
        }
        if settings.start_with_windows != startup_live {
            settings.start_with_windows = startup_live;
            save_settings(&settings);
        }
        let settings_snapshot = settings.clone();
        let tray = TrayController::new(
            TrayState {
                enabled: settings.tray_enabled,
                window_visible: !start_in_tray,
                status_line: if logged_in {
                    "Starting session...".to_string()
                } else {
                    "Not signed in".to_string()
                },
                tooltip: "CREBLOX".to_string(),
                discord_rpc_enabled: settings.discord_rpc_enabled,
                signed_in: logged_in,
                server_line: String::new(),
                job_id: String::new(),
                roblox_running: false,
                accounts: Vec::new(),
                overlay_server: settings.overlay_enabled,
                overlay_stats: settings.overlay_stats_enabled,
                overlay_crosshair: settings.crosshair_enabled,
                overlay_editing: false,
            },
            tx.clone(),
            cc.egui_ctx.clone(),
        );
        let saved_accounts = load_accounts();
        let fastflags = load_fastflags();
        let _ = sync_client_fastflags(&fastflags, settings.target_fps, &[]);
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, &settings);
        let auto_close_flag = Arc::new(AtomicBool::new(settings.auto_close_roblox));
        let watcher_stop = Arc::new(AtomicBool::new(false));
        let last_roblox_launch_ms = Arc::new(AtomicU64::new(0));
        start_roblox_watcher(
            tx.clone(),
            cc.egui_ctx.clone(),
            Arc::clone(&auto_close_flag),
            Arc::clone(&watcher_stop),
            Arc::clone(&last_roblox_launch_ms),
        );
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            tx,
            rx,
            sid,
            page: if logged_in { Page::Home } else { Page::Login },
            back: Page::Login,
            status: if logged_in {
                String::new()
            } else {
                "Sign in with Quick Login to use the launcher.".to_string()
            },
            settings,
            settings_snapshot,
            logs_view: None,
            logs_view_at: None,
            player_path: find_player(),
            handler: handler_status(),
            cookie,
            quick_login: None,
            search: String::new(),
            searched: String::new(),
            searching: false,
            results: None,
            manual_game_open: false,
            manual_game_input: String::new(),
            manual_game_loading: false,
            manual_game_focus_requested: false,
            manual_game_err: None,
            active_private_server: None,
            sorts: {
                // Instant first paint from the last session; the background sync
                // replaces this with fresh charts a moment later.
                let cached = crate::api::load_sorts_cache();
                if cached.is_empty() {
                    None
                } else {
                    Some(Ok(cached))
                }
            },
            chart: 0,
            settings_section: 0,
            chart_min_players: 0,
            chart_min_rating: 0,
            feed_shuffled: false,
            friends_sort: FriendsSort::RecentlyActive,
            feed_pool: Vec::new(),
            feed_sig: 0,
            profile_cache: HashMap::new(),
            scroll_to_top: false,
            feed_rows: 30,
            feed_salt: 0x5EED_1234_ABCD_0001,
            chart_min_visits: 0,
            game_visits: HashMap::new(),
            visits_requested: HashSet::new(),
            recent_ids: read_ids("recent.txt"),
            recent: Vec::new(),
            fav_ids: read_ids("favorites.txt"),
            favs: Vec::new(),
            wide: HashMap::new(),
            icon: HashMap::new(),
            requested: HashMap::new(),
            game_video_ids: HashMap::new(),
            video_id_requested: HashSet::new(),
            decoded_videos: HashMap::new(),
            video_lru: Vec::new(),
            video_decode_requested: HashSet::new(),
            hovered_game_id: None,
            hover_start: None,
            hover_play_start: None,
            active_video_texture: None,
            sorts_sync_started: None,
            saved_sync_started: None,
            auth_sync_started: None,
            last_sorts_sync: None,
            last_saved_sync: None,
            last_auth_sync: None,
            home_sync_spinner: false,
            just_launched: Vec::new(),
            pending_favs: HashMap::new(),
            was_focused: true,
            auto_close_flag,
            watcher_stop,
            last_roblox_launch_ms,
            friend_names: Vec::new(),
            friends: crate::api::load_friends_cache(),
            friends_err: None,
            adding_friend: false,
            new_friend: String::new(),
            friend_searched: String::new(),
            friend_searching: false,
            friend_search_loading_more: false,
            friend_search_cursor: None,
            friend_search_verified: HashSet::new(),
            friend_search_results: None,
            friend_sent: HashSet::new(),
            social_tab: SocialTab::Requests,
            social_lists: HashMap::new(),
            social_cursors: HashMap::new(),
            social_loading: HashSet::new(),
            social_loading_more: HashSet::new(),
            social_errs: HashMap::new(),
            social_responding: HashSet::new(),
            toasts: Vec::new(),
            next_toast_id: 0,
            known_friend_requests: None,
            last_requests_poll: None,
            update_checked: false,
            notification_history: Vec::new(),
            roblox_update_available: None,
            seen_notifications: load_seen_notifications(),
            notified_kill_pids: HashSet::new(),
            last_autokill_toast: None,
            app_update_available: None,
            app_update_checking: false,
            app_update_checked: false,
            app_update_status: None,
            app_update_downloading: false,
            app_update_progress: None,
            app_update_zip: None,
            app_update_installing: false,
            app_update_error: None,
            update_token_input: String::new(),
            update_token_saved: crate::updater::has_github_update_token(),
            app_patch_applied: None,
            app_staged_patch: None,
            username: String::new(),
            me: None,
            me_err: None,
            my_profile: None,
            my_profile_loading: false,
            my_profile_err: None,
            profile_bio_expanded: false,
            profile_editing: false,
            profile_edit_bio: String::new(),
            profile_saving_bio: false,
            profile_save_err: None,
            friend_profile: None,
            friend_loading: false,
            friend_err: None,
            avatar_state: None,
            avatar_palette: Vec::new(),
            avatar_loading: false,
            avatar_pending: 0,
            avatar_seq: 0,
            avatar_err: None,
            avatar_tab: AvatarTab::Wearing,
            avatar_search: String::new(),
            avatar_color_target: BodyColorTarget::All,
            avatar_items: HashMap::new(),
            avatar_cursors: HashMap::new(),
            avatar_items_loading: None,
            avatar_items_err: None,
            avatar_outfits: None,
            avatar_outfits_page: 1,
            avatar_outfits_more: false,
            avatar_outfits_loading: false,
            avatar_outfits_err: None,
            avatar_body_part_filter: BodyPartFilter::All,
            avatar_outfit_filter: OutfitFilter::All,
            avatar_save_outfit_open: false,
            avatar_save_outfit_name: String::new(),
            avatar_outfit_saving: false,
            avatar_3d: Avatar3DState::default(),
            game_tab: GameTab::About,
            game_extra: None,
            game_extra_cache: HashMap::new(),
            game_media_idx: 0,
            game_passes_loading: false,
            game_passes_err: None,
            game_servers_loading: false,
            game_servers_err: None,
            server_sort: ServerSort::Default,
            server_exclude_full: false,
            wallpaper_tex: None,
            wallpaper_loading: None,
            wallpaper_err: None,
            settings_advanced_open: false,
            last_ram_trim_check: None,
            discord_rpc: DiscordRpcClient::new(),
            app_started_epoch_secs: now_unix_ms() / 1000,
            active_roblox_place_id: None,
            roblox_running: false,
            active_roblox_started_at: None,
            playtime: crate::playtime::load(),
            playtime_session: None,
            server_info: None,
            server_locations: HashMap::new(),
            server_location_pending: HashSet::new(),
            game_overlay: crate::game_overlay::OverlayManager::new(),
            pending_discord_launch: None,
            discord_resolved_games: HashMap::new(),
            discord_resolving_places: HashSet::new(),
            tray,
            window_visible: !start_in_tray,
            tray_minimize_balloon_shown: false,
            allow_close_exit: false,
            hidden_for_game: false,
            hidden_at: None,
            hidden_seen_inactive: false,
            last_hidden_check: None,
            last_window_check: None,
            last_visibility_check: None,
            last_overlay_check: None,
            overlay_editing: false,
            last_overlay_edit_check: None,
            raise_frames: 0,
            pending_start_in_tray_frames: if start_in_tray { 2 } else { 0 },
            startup_err: None,
            saved_accounts,
            fastflags,
            ff_new_key: String::new(),
            ff_new_val: String::new(),
            ff_json_buf: String::new(),
            ff_json_open: false,
            ff_err: None,
        };
        // Let a later launch (Finder, Dock, `open`, Spotlight) ask this instance to show
        // itself instead of the second process exiting with nothing on screen.
        #[cfg(target_os = "macos")]
        crate::tray::start_instance_listener(app.tx.clone(), app.ctx.clone());
        // A Dock click while no window is on screen is an app activation, and eframe stops
        // ticking once the window is ordered out - so this cannot live in `update()`.
        #[cfg(target_os = "macos")]
        crate::tray::start_reopen_watcher(app.tx.clone(), app.ctx.clone());
        // A Dock click arrives as `applicationShouldHandleReopen:hasVisibleWindows:`, which
        // winit does not implement, so it is added to winit's own delegate class here.
        #[cfg(target_os = "macos")]
        crate::tray::install_dock_reopen_handler();
        // Starting hidden only works with a menu-bar icon; without one the process would
        // run with no window and no way to bring it back.
        if start_in_tray && !app.tray.available() {
            crate::auth::log("tray: icon unavailable at startup - showing the window");
            app.restore_from_tray();
        }
        app.apply_runtime_resource_limits(false);
        app.sync_discord_presence();
        app.sync_tray_state();
        if app.settings.bg_mode == BgMode::Wallpaper && !app.settings.bg_wallpaper.trim().is_empty()
        {
            let path = app.settings.bg_wallpaper.clone();
            app.request_wallpaper_load(path);
        }
        if app.cookie.is_some() {
            app.sync_sorts(true);
            app.sync_saved(true);
            app.sync_auth_session(true);
        }
        app
    }

    pub fn spawn(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }

    pub fn is_sync_active(started: &Option<Instant>) -> bool {
        started.is_some_and(|t| t.elapsed() < Duration::from_secs(30))
    }

    pub fn is_stale(last: &Option<Instant>, started: &Option<Instant>, interval: Duration) -> bool {
        if Self::is_sync_active(started) {
            return false;
        }
        match (last, started) {
            (Some(t), _) => t.elapsed() >= interval,
            (None, None) => true,
            (None, Some(s)) => s.elapsed() >= interval,
        }
    }

    pub fn sync_sorts(&mut self, force: bool) {
        if self.cookie.is_none() {
            return;
        }
        if !force && Self::is_sync_active(&self.sorts_sync_started) {
            return;
        }
        self.sid = new_session_id();
        self.sorts_sync_started = Some(Instant::now());
        let sid = self.sid.clone();
        let cookie = self.cookie.clone();
        self.spawn(move || Msg::Sorts(fetch_sorts(&sid, cookie.as_deref())));
    }

    pub fn sync_saved(&mut self, force: bool) {
        let Some(cookie) = self.cookie.clone() else {
            return;
        };
        if !force && Self::is_sync_active(&self.saved_sync_started) {
            return;
        }
        self.saved_sync_started = Some(Instant::now());
        let uid = self.me.as_ref().map(|p| p.user.id);
        let r = self.recent_ids.clone();
        let f = self.fav_ids.clone();
        self.spawn(move || Msg::SavedSynced(fetch_saved_synced(&cookie, uid, r, f)));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_toast(
        &mut self,
        source: NotificationSource,
        kind: ToastKind,
        title: impl Into<String>,
        body: impl Into<String>,
        avatar: Option<String>,
        primary_action: Option<(String, ToastAction)>,
        secondary_action: Option<(String, ToastAction)>,
        duration: f64,
        show_popup: bool,
    ) {
        self.next_toast_id = self.next_toast_id.wrapping_add(1);
        let id = self.next_toast_id;
        let now = self.ctx.input(|i| i.time);
        let title_s = title.into();
        let body_s = body.into();

        if self.notification_history.len() >= 50 {
            self.notification_history.remove(0);
        }
        self.notification_history.push(NotificationItem {
            id,
            source,
            kind,
            title: title_s.clone(),
            body: body_s.clone(),
            avatar: avatar.clone(),
            primary_action: primary_action.clone(),
            secondary_action: secondary_action.clone(),
            created_secs: now,
            read: matches!(self.page, Page::Notifications),
        });

        if show_popup && !self.settings.notifications_silent {
            if self.toasts.len() >= 4 {
                self.toasts.remove(0);
            }
            self.toasts.push(ToastNotification {
                id,
                kind,
                title: title_s,
                body: body_s,
                avatar,
                primary_action,
                secondary_action,
                created_at: now,
                duration,
                paused_at: None,
                dismissed_at: None,
            });
        }
        self.ctx.request_repaint();
    }

    pub fn check_updates(&mut self, manual: bool) {
        let player = self.player_path.clone();
        if manual {
            self.status = "Checking for Roblox updates...".to_string();
        }
        self.spawn(move || Msg::UpdateCheckResult(manual, check_roblox_update(player.as_deref())));
    }

    pub fn install_app_update(&mut self) {
        if self.app_update_downloading || self.app_update_installing {
            return;
        }
        let Some(zip) = self.app_update_zip.clone() else {
            return;
        };
        let Some(version) = self
            .app_update_available
            .as_ref()
            .map(|r| r.version.clone())
        else {
            return;
        };
        self.app_update_error = None;
        self.app_update_status = Some(format!("Starting the v{version} installer..."));
        match crate::updater::install_update_zip(&zip, &version) {
            Ok(()) => {
                self.app_update_installing = true;
                self.status = format!("Installing v{version} in Terminal...");
                self.allow_close_exit = true;
                self.tray.shutdown();
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => {
                self.app_update_error = Some(e.clone());
                self.app_update_status = None;
                self.status = format!("Update install failed: {e}");
            }
        }
    }

    pub fn save_update_token(&mut self) {
        let token = self.update_token_input.trim().to_string();
        if token.is_empty() {
            self.app_update_error = Some("Paste a GitHub token first.".to_string());
            return;
        }
        match crate::updater::save_github_update_token(&token) {
            Ok(()) => {
                self.update_token_input.clear();
                self.update_token_saved = true;
                self.app_update_error = None;
                self.app_update_status =
                    Some("GitHub token saved. Checking releases...".to_string());
                self.check_app_updates(true);
            }
            Err(e) => {
                self.app_update_error = Some(e.clone());
                self.status = format!("Could not save GitHub token: {e}");
            }
        }
    }

    pub fn clear_update_token(&mut self) {
        match crate::updater::clear_github_update_token() {
            Ok(()) => {
                self.update_token_saved = false;
                self.app_update_status = Some(
                    "GitHub token removed. Add one to check private repository releases."
                        .to_string(),
                );
                self.app_update_error = None;
            }
            Err(e) => {
                self.app_update_error = Some(e.clone());
                self.status = format!("Could not remove GitHub token: {e}");
            }
        }
    }

    pub fn check_app_updates(&mut self, manual: bool) {
        if self.app_update_checking || self.app_update_downloading {
            return;
        }
        self.app_update_checking = true;
        self.app_update_error = None;
        if manual {
            self.app_update_status = Some("Checking GitHub releases...".to_string());
        }
        self.spawn(move || {
            let token = crate::updater::github_update_token();
            Msg::AppUpdateCheckResult(manual, check_creblox_update(token.as_deref()))
        });
    }

    pub fn start_app_update_download(&mut self, force_restart: bool) {
        if self.app_update_downloading {
            return;
        }
        let Some(release) = self.app_update_available.clone() else {
            return;
        };
        // macOS releases are source zips linked in the release body: download them
        // inside the app, then let the user press Install & Restart.
        if cfg!(target_os = "macos") && release.zip_url.is_some() {
            let ver = release.version.clone();
            let downloaded = Arc::new(AtomicU64::new(0));
            let total = Arc::new(AtomicU64::new(release.zip_size));
            self.app_update_progress = Some((Arc::clone(&downloaded), Arc::clone(&total)));
            self.app_update_downloading = true;
            self.app_update_error = None;
            self.app_update_zip = None;
            self.app_update_status = Some(format!("Downloading v{ver}..."));
            let ctx = self.ctx.clone();
            self.spawn(move || {
                Msg::AppUpdateDownloaded(
                    ver,
                    crate::updater::download_update_zip(&release, &downloaded, &total, &ctx),
                )
            });
            return;
        }
        if force_restart || release.kind == UpdateKind::Full {
            if release.exe_url.is_none() {
                let _ = open::that(&release.html_url);
                return;
            }
            self.app_update_error = None;
            match launch_updater_process(&release) {
                Ok(()) => {
                    self.app_staged_patch = None;
                    self.allow_close_exit = true;
                    self.tray.shutdown();
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => {
                    self.app_update_error = Some(e.clone());
                    self.status = format!("Updater launch failed: {e}");
                }
            }
            return;
        }

        let ver = release.version.clone();
        let downloaded = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(release.exe_size));
        self.app_update_progress = Some((Arc::clone(&downloaded), Arc::clone(&total)));
        self.app_update_downloading = true;
        self.app_update_error = None;
        self.app_update_status = Some(format!("Applying patch v{ver} in background..."));
        let ctx = self.ctx.clone();
        self.spawn(move || {
            Msg::AppPatchDone(
                ver,
                apply_small_patch_without_restart(&release, &downloaded, &total, &ctx),
            )
        });
    }

    pub fn sync_auth_session(&mut self, force: bool) {
        let Some(cookie) = self.cookie.clone() else {
            return;
        };
        if !force && Self::is_sync_active(&self.auth_sync_started) {
            return;
        }
        self.auth_sync_started = Some(Instant::now());
        self.spawn(move || Msg::AuthSession(fetch_authenticated_session(&cookie)));
    }

    pub fn force_refresh_games(&mut self) {
        if self.cookie.is_none() {
            return;
        }
        self.requested.clear();
        let thumb_ids: Vec<u64> = self
            .recent
            .iter()
            .chain(self.favs.iter())
            .map(|g| g.universe_id)
            .collect();
        self.request_thumbs(thumb_ids);
        self.home_sync_spinner = true;
        self.sync_sorts(true);
        self.sync_saved(true);
        self.sync_auth_session(true);
    }

    fn clear_home_sync_spinner_if_idle(&mut self) {
        if !Self::is_sync_active(&self.sorts_sync_started)
            && !Self::is_sync_active(&self.saved_sync_started)
        {
            self.home_sync_spinner = false;
        }
    }

    pub fn maybe_auto_sync(&mut self, ctx: &egui::Context) {
        if self.cookie.is_none() {
            return;
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        let gained_focus = focused && !self.was_focused;
        self.was_focused = focused;

        // Wake often while the window is in use; rarely while it is hidden / in the background.
        // While hidden, still tick a couple of times a second: the Dock-activation and
        // "game closed" watchdogs only run inside a frame, and a 30 s tick left a hidden
        // launcher looking dead until the app was quit and reopened.
        ctx.request_repaint_after(if !self.window_visible {
            Duration::from_secs(2)
        } else if focused {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(20)
        });

        let on_discovery = matches!(self.page, Page::Home | Page::Charts);
        let saved_interval = if gained_focus {
            Duration::from_secs(3)
        } else if on_discovery {
            Duration::from_secs(20)
        } else {
            Duration::from_secs(60)
        };
        let sorts_interval = if gained_focus {
            Duration::from_secs(6)
        } else if on_discovery {
            Duration::from_secs(25)
        } else {
            Duration::from_secs(90)
        };
        let auth_interval = if gained_focus {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(30)
        };

        if Self::is_stale(
            &self.last_saved_sync,
            &self.saved_sync_started,
            saved_interval,
        ) {
            self.home_sync_spinner = false;
            self.sync_saved(false);
        }
        if Self::is_stale(
            &self.last_sorts_sync,
            &self.sorts_sync_started,
            sorts_interval,
        ) {
            self.home_sync_spinner = false;
            self.sync_sorts(false);
        }
        if Self::is_stale(&self.last_auth_sync, &self.auth_sync_started, auth_interval) {
            self.sync_auth_session(false);
        }
        let req_interval = if gained_focus {
            Duration::from_secs(8)
        } else {
            Duration::from_secs(30)
        };
        let req_stale = self
            .last_requests_poll
            .map_or(true, |t| t.elapsed() >= req_interval);
        if req_stale && !self.social_loading.contains(&SocialTab::Requests) {
            self.last_requests_poll = Some(Instant::now());
            self.load_social_tab(SocialTab::Requests, false);
        }
    }

    pub fn find_known_game_by_place(&self, pid: u64) -> Option<Game> {
        if let Page::Game(g) = &self.page {
            if g.place_id == pid {
                return Some(g.clone());
            }
        }
        for g in self.recent.iter().chain(self.favs.iter()) {
            if g.place_id == pid {
                return Some(g.clone());
            }
        }
        if let Some(Ok(sorts)) = &self.sorts {
            for s in sorts {
                if let Some(g) = s.games.iter().find(|g| g.place_id == pid) {
                    return Some(g.clone());
                }
            }
        }
        if let Some(Ok(res)) = &self.results {
            if let Some(g) = res.iter().find(|g| g.place_id == pid) {
                return Some(g.clone());
            }
        }
        if let Some(g) = self
            .friend_profile
            .as_ref()
            .and_then(|fp| fp.playing_game.as_ref())
            .filter(|g| g.place_id == pid)
        {
            return Some(g.clone());
        }
        if let Some(g) = self
            .my_profile
            .as_ref()
            .and_then(|fp| fp.playing_game.as_ref())
            .filter(|g| g.place_id == pid)
        {
            return Some(g.clone());
        }
        None
    }

    pub fn record_recent_game(&mut self, g: Game) {
        let now = Instant::now();
        self.just_launched
            .retain(|(p, t)| *p != g.place_id && t.elapsed() < Duration::from_secs(600));
        self.just_launched.insert(0, (g.place_id, now));
        self.just_launched.truncate(8);

        self.recent_ids.retain(|p| *p != g.place_id);
        self.recent_ids.insert(0, g.place_id);
        self.recent_ids.truncate(24);
        write_ids("recent.txt", &self.recent_ids);

        self.recent
            .retain(|x| x.universe_id != g.universe_id && x.place_id != g.place_id);
        self.recent.insert(0, g.clone());
        self.recent.truncate(24);
        self.request_thumbs(vec![g.universe_id]);
    }

    pub fn record_recent_place_id(&mut self, pid: u64) {
        if pid == 0 {
            return;
        }
        if let Some(g) = self.find_known_game_by_place(pid) {
            self.record_recent_game(g);
            return;
        }
        let now = Instant::now();
        self.just_launched
            .retain(|(p, t)| *p != pid && t.elapsed() < Duration::from_secs(600));
        self.just_launched.insert(0, (pid, now));
        self.just_launched.truncate(8);

        self.recent_ids.retain(|p| *p != pid);
        self.recent_ids.insert(0, pid);
        self.recent_ids.truncate(24);
        write_ids("recent.txt", &self.recent_ids);

        self.spawn(move || {
            if let Some(g) = games_by_places(&[pid]).into_iter().next() {
                Msg::RecentPlaceResolved(g)
            } else {
                Msg::Details(None)
            }
        });
    }

    pub fn load_avatar(&mut self) {
        if self.avatar_loading {
            return;
        }
        if let Some(cookie) = self.cookie.clone() {
            if let Some(p) = &self.me {
                crate::api::remember_render_urls(&p.avatar, &p.user.headshot);
            }
            self.avatar_loading = true;
            self.avatar_err = None;
            self.spawn(move || Msg::AvatarLoaded(fetch_avatar_session(&cookie)));
        }
    }

    pub fn load_avatar_3d(&mut self) {
        if self.avatar_3d.loading {
            // An edit landed while a load was running: that load may show the old avatar,
            // so run another one as soon as it finishes instead of dropping the request.
            self.avatar_3d.reload_queued = true;
            return;
        }
        let Some(uid) = self.me.as_ref().map(|p| p.user.id) else {
            return;
        };
        let cookie = self.cookie.clone();
        // After an avatar edit, wait for Roblox to serve a model different from the old one.
        let stale_hash = if std::mem::take(&mut self.avatar_3d.expect_change) {
            self.avatar_3d
                .model
                .as_ref()
                .map(|m| m.obj_hash.clone())
                .filter(|h| !h.is_empty())
        } else {
            None
        };
        self.avatar_3d.loading = true;
        self.avatar_3d.err = None;
        self.spawn(move || {
            let res = load_avatar_3d_data_ex(uid, cookie.as_deref(), stale_hash.as_deref());
            Msg::Avatar3DLoaded(uid, res)
        });
    }

    pub fn ensure_avatar_loaded(&mut self) {
        if self.avatar_state.is_none() && !self.avatar_loading {
            self.load_avatar();
        }
        if self.avatar_3d.model.is_none() && !self.avatar_3d.loading {
            self.load_avatar_3d();
        }
        self.ensure_avatar_tab_loaded(self.avatar_tab);
    }

    pub fn ensure_my_profile_loaded(&mut self) {
        if self.my_profile_loading {
            return;
        }
        let Some(me) = self.me.as_ref() else {
            return;
        };
        let uid = me.user.id;
        let user = me.user.clone();
        let cookie = self.cookie.clone();
        // A profile loaded less than a minute ago is painted without any network work.
        if let Some((cached, at)) = self.profile_cache.get(&uid) {
            if at.elapsed() < Duration::from_secs(60) {
                self.my_profile = Some(cached.clone());
                self.my_profile_err = None;
                return;
            }
        }
        self.my_profile_loading = true;
        self.my_profile_err = None;
        let uid = user.id;
        self.spawn(move || Msg::MyProfile(uid, fetch_friend_profile(user, cookie)));
    }

    pub fn load_avatar_tab(&mut self, tab: AvatarTab, append: bool) {
        let Some(cookie) = self.cookie.clone() else {
            return;
        };
        let Some(uid) = self.me.as_ref().map(|p| p.user.id) else {
            return;
        };
        match tab {
            AvatarTab::Outfits => {
                if self.avatar_outfits_loading {
                    return;
                }
                let page = if append {
                    self.avatar_outfits_page + 1
                } else {
                    1
                };
                self.avatar_outfits_loading = true;
                self.avatar_outfits_err = None;
                self.spawn(move || {
                    Msg::AvatarOutfits(page, fetch_avatar_outfits(uid, page, &cookie))
                });
            }
            t if t.inventory_query().is_some() => {
                if self.avatar_items_loading == Some(t) {
                    return;
                }
                let cursor = if append {
                    let Some(c) = self.avatar_cursors.get(&t).and_then(|c| c.clone()) else {
                        return;
                    };
                    Some(c)
                } else {
                    None
                };
                self.avatar_items_loading = Some(t);
                self.avatar_items_err = None;
                self.spawn(move || {
                    Msg::AvatarInventory(
                        t,
                        append,
                        fetch_avatar_inventory(uid, t, &cookie, cursor.as_deref()),
                    )
                });
            }
            _ => {}
        }
    }

    pub fn ensure_avatar_tab_loaded(&mut self, tab: AvatarTab) {
        match tab {
            AvatarTab::Outfits if self.avatar_outfits.is_none() => self.load_avatar_tab(tab, false),
            t if t.inventory_query().is_some() && !self.avatar_items.contains_key(&t) => {
                self.load_avatar_tab(t, false)
            }
            _ => {}
        }
    }

    pub fn cancel_quick_login(&mut self) {
        if let Some(ql) = self.quick_login.take() {
            ql.cancel.store(true, Ordering::Relaxed);
            if !ql.qr_uri.is_empty() {
                self.ctx.forget_image(&ql.qr_uri);
            }
        }
    }

    pub fn begin_quick_login(&mut self) {
        self.cancel_quick_login();
        if !matches!(self.page, Page::Login) {
            self.back = self.page.clone();
        }
        self.page = Page::Login;
        let cancel = start_quick_login(self.tx.clone(), self.ctx.clone());
        self.quick_login = Some(QuickLoginState {
            code: String::new(),
            qr_uri: String::new(),
            qr_bytes: None,
            status_text: "Requesting Quick Login code...".to_string(),
            error: None,
            cancel,
        });
    }

    pub fn reset_session_caches(&mut self) {
        self.username.clear();
        self.me = None;
        self.me_err = None;
        self.friends.clear();
        self.friends_err = None;
        self.friend_names.clear();
        self.adding_friend = false;
        self.new_friend.clear();
        self.friend_searched.clear();
        self.friend_searching = false;
        self.friend_search_loading_more = false;
        self.friend_search_cursor = None;
        self.friend_search_verified.clear();
        self.friend_search_results = None;
        self.friend_sent.clear();
        self.social_tab = SocialTab::Requests;
        self.social_lists.clear();
        self.social_cursors.clear();
        self.social_loading.clear();
        self.social_loading_more.clear();
        self.social_errs.clear();
        self.social_responding.clear();
        self.known_friend_requests = None;
        self.last_requests_poll = None;
        self.notification_history.clear();
        self.search.clear();
        self.searched.clear();
        self.searching = false;
        self.results = None;
        self.manual_game_open = false;
        self.manual_game_input.clear();
        self.manual_game_loading = false;
        self.manual_game_focus_requested = false;
        self.manual_game_err = None;
        self.active_private_server = None;
        self.pending_discord_launch = None;
        self.sorts = None;
        self.recent.clear();
        self.favs.clear();
        self.wide.clear();
        self.icon.clear();
        self.requested.clear();
        self.game_video_ids.clear();
        self.video_id_requested.clear();
        self.decoded_videos.clear();
        self.video_lru.clear();
        self.video_decode_requested.clear();
        self.hovered_game_id = None;
        self.hover_start = None;
        self.hover_play_start = None;
        self.active_video_texture = None;
        self.sorts_sync_started = None;
        self.saved_sync_started = None;
        self.auth_sync_started = None;
        self.last_sorts_sync = None;
        self.last_saved_sync = None;
        self.last_auth_sync = None;
        self.home_sync_spinner = false;
        self.just_launched.clear();
        self.pending_favs.clear();
        self.my_profile = None;
        self.my_profile_loading = false;
        self.my_profile_err = None;
        self.profile_bio_expanded = false;
        self.profile_editing = false;
        self.profile_edit_bio.clear();
        self.profile_saving_bio = false;
        self.profile_save_err = None;
        self.friend_profile = None;
        self.friend_loading = false;
        self.friend_err = None;
        self.avatar_state = None;
        self.avatar_palette.clear();
        self.avatar_loading = false;
        self.avatar_pending = 0;
        self.avatar_seq = 0;
        self.avatar_err = None;
        self.avatar_tab = AvatarTab::Wearing;
        self.avatar_search.clear();
        self.avatar_color_target = BodyColorTarget::All;
        self.avatar_items.clear();
        self.avatar_cursors.clear();
        self.avatar_items_loading = None;
        self.avatar_items_err = None;
        self.avatar_outfits = None;
        self.avatar_outfits_page = 1;
        self.avatar_outfits_more = false;
        self.avatar_outfits_loading = false;
        self.avatar_outfits_err = None;
        self.avatar_body_part_filter = BodyPartFilter::All;
        self.avatar_outfit_filter = OutfitFilter::All;
        self.avatar_save_outfit_open = false;
        self.avatar_save_outfit_name.clear();
        self.avatar_outfit_saving = false;
        self.avatar_3d = Avatar3DState::default();
        self.game_tab = GameTab::About;
        self.game_extra = None;
        self.game_media_idx = 0;
        self.game_passes_loading = false;
        self.game_passes_err = None;
        self.game_servers_loading = false;
        self.game_servers_err = None;
        self.server_sort = ServerSort::Default;
        self.server_exclude_full = false;
        self.feed_pool.clear();
        self.feed_sig = 0;
        self.ctx.forget_all_images();
    }

    pub fn record_active_account(&mut self, prof: &Profile) {
        let Some(cookie) = self.cookie.as_deref() else {
            return;
        };
        upsert_stored_account(&mut self.saved_accounts, &prof.user, cookie, now_unix_ms());
        let _ = save_accounts(&self.saved_accounts);
    }

    pub fn switch_account(&mut self, user_id: u64) {
        let Some(pos) = self.saved_accounts.iter().position(|a| a.id == user_id) else {
            return;
        };
        let target_cookie = self.saved_accounts[pos].cookie.clone();
        let target_name = self.saved_accounts[pos].name.clone();
        if self.cookie.as_deref() == Some(target_cookie.as_str())
            && self.me.as_ref().is_some_and(|p| p.user.id == user_id)
        {
            return;
        }
        self.cancel_quick_login();
        self.saved_accounts[pos].last_used_ms = now_unix_ms();
        let _ = save_accounts(&self.saved_accounts);
        if let Err(e) = save_cookie(&target_cookie) {
            self.status = format!("Switched account, but could not save session: {e}");
        } else {
            self.status = format!("Switched to @{target_name}");
        }
        self.reset_session_caches();
        self.cookie = Some(target_cookie);
        if matches!(self.page, Page::Login | Page::Friend(_)) {
            self.page = Page::Home;
        }
        self.back = Page::Home;
        self.sync_sorts(true);
        self.sync_saved(true);
        self.sync_auth_session(true);
        self.sync_discord_presence();
    }

    pub fn remove_saved_account(&mut self, user_id: u64) {
        let is_active = self.me.as_ref().is_some_and(|p| p.user.id == user_id)
            || self
                .saved_accounts
                .iter()
                .find(|a| a.id == user_id)
                .is_some_and(|a| self.cookie.as_deref() == Some(a.cookie.as_str()));
        let removed = remove_stored_account(&mut self.saved_accounts, user_id);
        let _ = save_accounts(&self.saved_accounts);
        if is_active {
            if let Some(next_id) = self.saved_accounts.first().map(|a| a.id) {
                self.switch_account(next_id);
                if let Some(acc) = removed {
                    self.status = format!(
                        "Removed @{} — switched to @{}",
                        acc.name,
                        self.saved_accounts
                            .first()
                            .map(|a| a.name.as_str())
                            .unwrap_or("")
                    );
                }
            } else {
                self.cancel_quick_login();
                self.cookie = None;
                clear_cookie();
                let _ = fs::remove_file(data_dir().join("username.txt"));
                self.reset_session_caches();
                self.page = Page::Login;
                self.back = Page::Login;
                self.status = "Account removed — sign in with Quick Login to continue.".to_string();
                self.sync_discord_presence();
            }
        } else if let Some(acc) = removed {
            self.status = format!("Removed saved account @{}", acc.name);
        }
    }

    pub fn sign_out(&mut self) {
        self.cancel_quick_login();
        let active_id = self.me.as_ref().map(|p| p.user.id);
        if let Some(old_cookie) = self.cookie.take() {
            if let Some(uid) = active_id {
                remove_stored_account(&mut self.saved_accounts, uid);
            } else if let Some(pos) = self
                .saved_accounts
                .iter()
                .position(|a| a.cookie == old_cookie)
            {
                self.saved_accounts.remove(pos);
            }
            let _ = save_accounts(&self.saved_accounts);
            self.spawn(move || {
                let res = post_csrf(
                    "https://auth.roblox.com/v2/logout",
                    serde_json::json!({}),
                    Some(&old_cookie),
                    Some("https://www.roblox.com/"),
                )
                .map(|_| ());
                Msg::ServerLogoutDone(res)
            });
        }
        if let Some(next_id) = self.saved_accounts.first().map(|a| a.id) {
            self.switch_account(next_id);
            return;
        }
        clear_cookie();
        let _ = fs::remove_file(data_dir().join("username.txt"));
        self.reset_session_caches();
        self.page = Page::Login;
        self.back = Page::Login;
        self.status = "Signed out — sign in with Quick Login to continue.".to_string();
        self.sync_discord_presence();
    }

    pub fn request_thumbs(&mut self, ids: Vec<u64>) {
        let now = Instant::now();
        let retry_missing = Duration::from_secs(10);
        let refresh_existing = Duration::from_secs(120);
        let mut new = Vec::new();
        let mut queued = HashSet::new();
        for id in ids {
            if id == 0 || !queued.insert(id) {
                continue;
            }
            let have_both = self.wide.contains_key(&id) && self.icon.contains_key(&id);
            let min_age = if have_both {
                refresh_existing
            } else {
                retry_missing
            };
            let ready = self
                .requested
                .get(&id)
                .is_none_or(|t| t.elapsed() >= min_age);
            if ready {
                self.requested.insert(id, now);
                new.push(id);
            }
        }
        if !new.is_empty() {
            self.spawn(move || fetch_thumbs(new));
        }
    }

    pub fn start_search(&mut self) {
        let q = self.search.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.searched = q.clone();
        self.searching = true;
        self.results = None;
        self.page = Page::Search;
        if parse_game_link(&q).is_some() {
            // URLs and numeric Place IDs resolve directly to the game page
            let cookie = self.cookie.clone();
            self.spawn(move || Msg::Lookup(resolve_game_link(&q, cookie.as_deref())));
        } else {
            let sid = new_session_id();
            self.sid = sid.clone();
            self.spawn(move || {
                let r = search_games(&q, &sid);
                Msg::Search(q, r)
            });
        }
    }

    pub fn start_manual_game_lookup(&mut self, intent: ManualGameIntent) {
        if self.manual_game_loading || self.cookie.is_none() {
            return;
        }
        let raw = self.manual_game_input.trim().to_string();
        if parse_game_link(&raw).is_none() {
            self.manual_game_err = Some(
                "Enter a valid Roblox game URL or numeric Place ID (e.g. https://www.roblox.com/games/1818/...)."
                    .to_string(),
            );
            return;
        }
        self.manual_game_loading = true;
        self.manual_game_err = None;
        let cookie = self.cookie.clone();
        self.spawn(move || Msg::ManualGameLookup {
            intent,
            result: resolve_game_link(&raw, cookie.as_deref()),
        });
    }

    pub fn poll(&mut self) -> bool {
        let mut had_messages = false;
        while let Ok(msg) = self.rx.try_recv() {
            had_messages = true;
            let logged_in = self.cookie.is_some();
            match msg {
                Msg::GameVisits(v) => {
                    self.game_visits.extend(v);
                }
                Msg::Sorts(r) => {
                    if !logged_in {
                        continue;
                    }
                    self.sorts_sync_started = None;
                    self.last_sorts_sync = Some(Instant::now());
                    self.clear_home_sync_spinner_if_idle();
                    match r {
                        Ok(s) => {
                            let ids: Vec<u64> = s
                                .iter()
                                .flat_map(|sort| sort.games.iter().map(|g| g.universe_id))
                                .collect();
                            crate::api::save_sorts_cache(&s);
                            if !s.is_empty() {
                                if self.chart >= s.len() {
                                    self.chart = 0;
                                }
                                self.sorts = Some(Ok(s));
                            } else if self.sorts.is_none() || matches!(self.sorts, Some(Err(_))) {
                                self.sorts = Some(Ok(s));
                            }
                            self.request_thumbs(ids);
                        }
                        Err(e) => {
                            if self.sorts.is_none() || matches!(self.sorts, Some(Err(_))) {
                                self.sorts = Some(Err(e));
                            }
                        }
                    }
                }
                Msg::Search(q, r) => {
                    if !logged_in {
                        continue;
                    }
                    if q == self.searched {
                        self.searching = false;
                        if let Ok(v) = &r {
                            let ids = v.iter().map(|g| g.universe_id).collect();
                            self.request_thumbs(ids);
                        }
                        self.results = Some(r);
                    }
                }
                Msg::Lookup(r) => {
                    if !logged_in {
                        continue;
                    }
                    self.searching = false;
                    match r {
                        Ok(resolved) => {
                            self.back = Page::Home;
                            self.apply_resolved_game_link(resolved, ManualGameIntent::Open);
                        }
                        Err(e) => self.results = Some(Err(e)),
                    }
                }
                Msg::ManualGameLookup { intent, result } => {
                    if !logged_in {
                        continue;
                    }
                    self.manual_game_loading = false;
                    match result {
                        Ok(resolved) => {
                            self.manual_game_open = false;
                            self.manual_game_err = None;
                            self.manual_game_input.clear();
                            self.apply_resolved_game_link(resolved, intent);
                        }
                        Err(e) => self.manual_game_err = Some(e),
                    }
                }
                Msg::Details(Some(full)) => {
                    if !logged_in {
                        continue;
                    }
                    let mut updated_current = false;
                    if let Page::Game(cur) = &mut self.page {
                        if cur.universe_id == full.universe_id {
                            *cur = full;
                            updated_current = true;
                        }
                    }
                    if updated_current {
                        self.sync_discord_presence();
                    }
                }
                Msg::Details(None) => {}
                Msg::GameExtraLoaded(ex) => {
                    if !logged_in {
                        continue;
                    }
                    self.game_video_ids
                        .insert(ex.universe_id, ex.preview_video_id.clone());
                    let uid = ex.universe_id;
                    self.game_extra_cache
                        .insert(uid, ((*ex).clone(), Instant::now()));
                    if self.game_extra_cache.len() > 8 {
                        if let Some(oldest) = self
                            .game_extra_cache
                            .iter()
                            .min_by_key(|(_, (_, at))| *at)
                            .map(|(id, _)| *id)
                        {
                            self.game_extra_cache.remove(&oldest);
                        }
                    }
                    if let Page::Game(cur) = &self.page {
                        if cur.universe_id == uid {
                            self.game_extra = Some(*ex);
                        }
                    }
                }
                Msg::GameVideoIdResolved(uid, vid_opt) => {
                    self.video_id_requested.remove(&uid);
                    self.game_video_ids.insert(uid, vid_opt);
                    if let Some(vid) = vid_opt {
                        if !self.decoded_videos.contains_key(&vid)
                            && !self.video_decode_requested.contains(&vid)
                        {
                            self.video_decode_requested.insert(vid);
                            let cookie = self.cookie.clone();
                            self.spawn(move || {
                                let res =
                                    fetch_and_decode_preview_video(vid, 320, cookie.as_deref())
                                        .map(Arc::new);
                                Msg::VideoPreviewLoaded(vid, res)
                            });
                        }
                    }
                }
                Msg::VideoPreviewLoaded(vid, res) => {
                    self.video_decode_requested.remove(&vid);
                    if let Ok(video) = res {
                        if !self.decoded_videos.contains_key(&vid) {
                            let max_videos = crate::telemetry::max_video_cache_for_ram_limit(
                                self.settings.ram_limit_mb,
                            );
                            while self.video_lru.len() >= max_videos {
                                if let Some(old_vid) = self.video_lru.pop() {
                                    self.decoded_videos.remove(&old_vid);
                                }
                            }
                            self.video_lru.insert(0, vid);
                            self.decoded_videos.insert(vid, video);
                        }
                        if self.hover_play_start.is_none() {
                            self.hover_play_start = Some(Instant::now());
                        }
                    }
                }
                Msg::WallpaperPicked(path_opt) => {
                    if let Some(path) = path_opt {
                        self.settings.bg_mode = BgMode::Wallpaper;
                        self.settings.bg_wallpaper = path.clone();
                        save_settings(&self.settings);
                        apply_theme(&self.ctx, &self.settings);
                        self.request_wallpaper_load(path);
                    }
                }
                Msg::WallpaperLoaded(path, res) => {
                    if self.wallpaper_loading.as_deref() == Some(path.as_str()) {
                        self.wallpaper_loading = None;
                    }
                    if path == self.settings.bg_wallpaper {
                        match res {
                            Ok(img) => {
                                let tex = self.ctx.load_texture(
                                    "app_wallpaper",
                                    img,
                                    egui::TextureOptions::LINEAR,
                                );
                                self.wallpaper_tex = Some((path, tex));
                                self.wallpaper_err = None;
                            }
                            Err(e) => {
                                self.wallpaper_tex = None;
                                self.wallpaper_err = Some(e);
                            }
                        }
                    }
                }
                Msg::GamePassesLoaded(uid, r) => {
                    if !logged_in {
                        continue;
                    }
                    if matches!(&self.page, Page::Game(cur) if cur.universe_id == uid) {
                        self.game_passes_loading = false;
                        match r {
                            Ok(passes) => {
                                if let Some(ex) = &mut self.game_extra {
                                    ex.passes = passes;
                                    ex.passes_loaded = true;
                                }
                                self.game_passes_err = None;
                            }
                            Err(e) => {
                                self.game_passes_err = Some(e);
                            }
                        }
                    }
                }
                Msg::GameServersLoaded(uid, append, r) => {
                    if !logged_in {
                        continue;
                    }
                    if matches!(&self.page, Page::Game(cur) if cur.universe_id == uid) {
                        self.game_servers_loading = false;
                        match r {
                            Ok((mut list, next_cursor)) => {
                                if let Some(ex) = &mut self.game_extra {
                                    if append {
                                        ex.servers.append(&mut list);
                                    } else {
                                        ex.servers = list;
                                    }
                                    ex.servers_cursor = next_cursor;
                                    ex.servers_loaded = true;
                                }
                                self.game_servers_err = None;
                            }
                            Err(e) => {
                                self.game_servers_err = Some(e);
                            }
                        }
                    }
                }
                Msg::GameVoteDone(uid, vote, r) => {
                    if !logged_in {
                        continue;
                    }
                    match r {
                        Ok(()) => {
                            self.status = match vote {
                                Some(true) => "Upvoted experience.".to_string(),
                                Some(false) => "Downvoted experience.".to_string(),
                                None => "Cleared vote.".to_string(),
                            };
                        }
                        Err(e) => {
                            self.status = format!("Could not submit vote: {e}");
                            let cookie = self.cookie.clone();
                            self.spawn(move || {
                                Msg::GameExtraLoaded(Box::new(fetch_game_extra(
                                    uid,
                                    cookie.as_deref(),
                                )))
                            });
                        }
                    }
                }
                Msg::GameFavDone(uid, is_fav, r) => {
                    if !logged_in {
                        continue;
                    }
                    match r {
                        Ok(()) => {
                            if let Some(ex) = &mut self.game_extra {
                                if ex.universe_id == uid {
                                    ex.is_favorited_api = Some(is_fav);
                                }
                            }
                        }
                        Err(e) => {
                            self.status = format!("Saved locally (Roblox favorite sync: {e})");
                        }
                    }
                }
                Msg::SavedSynced(sync) => {
                    if !logged_in {
                        continue;
                    }
                    self.saved_sync_started = None;
                    self.last_saved_sync = Some(Instant::now());
                    self.clear_home_sync_spinner_if_idle();
                    let mut thumb_ids = Vec::new();

                    self.just_launched
                        .retain(|(_, t)| t.elapsed() < Duration::from_secs(600));
                    let next_recent = if let Some(remote) = sync.remote_recent {
                        if !remote.is_empty() || self.just_launched.is_empty() {
                            let mut merged = Vec::new();
                            let mut fallback_by_pid: HashMap<u64, Game> = self
                                .recent
                                .iter()
                                .chain(sync.fallback_recent.iter().flatten())
                                .map(|g| (g.place_id, g.clone()))
                                .collect();
                            for (pid, _) in &self.just_launched {
                                if !remote.iter().any(|g| g.place_id == *pid)
                                    && !merged.iter().any(|g: &Game| g.place_id == *pid)
                                {
                                    if let Some(g) = fallback_by_pid.remove(pid) {
                                        merged.push(g);
                                    }
                                }
                            }
                            for g in remote {
                                if !merged.iter().any(|x| x.universe_id == g.universe_id) {
                                    merged.push(g);
                                }
                            }
                            merged.truncate(24);
                            Some(merged)
                        } else {
                            sync.fallback_recent
                        }
                    } else {
                        sync.fallback_recent
                    };
                    if let Some(v) = next_recent {
                        thumb_ids.extend(v.iter().map(|g| g.universe_id));
                        let next_ids: Vec<u64> = v
                            .iter()
                            .filter(|g| g.place_id > 0)
                            .map(|g| g.place_id)
                            .collect();
                        if !next_ids.is_empty() || !self.recent.is_empty() {
                            self.recent_ids = next_ids;
                            write_ids("recent.txt", &self.recent_ids);
                        }
                        self.recent = v;
                    }

                    self.pending_favs
                        .retain(|_, (_, t)| t.elapsed() < Duration::from_secs(15));
                    let next_favs = if let Some(mut remote) = sync.remote_favs {
                        for (uid, (want_fav, _)) in &self.pending_favs {
                            if *want_fav {
                                if !remote.iter().any(|g| g.universe_id == *uid) {
                                    if let Some(existing) =
                                        self.favs.iter().find(|g| g.universe_id == *uid)
                                    {
                                        remote.insert(0, existing.clone());
                                    }
                                }
                            } else {
                                remote.retain(|g| g.universe_id != *uid);
                            }
                        }
                        remote.truncate(24);
                        Some(remote)
                    } else {
                        sync.fallback_favs
                    };
                    if let Some(v) = next_favs {
                        thumb_ids.extend(v.iter().map(|g| g.universe_id));
                        let next_ids: Vec<u64> = v
                            .iter()
                            .filter(|g| g.place_id > 0)
                            .map(|g| g.place_id)
                            .collect();
                        self.fav_ids = next_ids;
                        write_ids("favorites.txt", &self.fav_ids);
                        self.favs = v;
                    }

                    self.request_thumbs(thumb_ids);
                }
                Msg::RecentPlaceResolved(g) => {
                    if !logged_in {
                        continue;
                    }
                    self.recent
                        .retain(|x| x.universe_id != g.universe_id && x.place_id != g.place_id);
                    self.recent.insert(0, g.clone());
                    self.recent.truncate(24);
                    self.request_thumbs(vec![g.universe_id]);
                }
                Msg::Thumbs { wide, icon } => {
                    if !logged_in {
                        continue;
                    }
                    for (uid, url) in wide {
                        if let Some(old) = self.wide.get(&uid) {
                            if old != &url {
                                self.ctx.forget_image(old);
                            }
                        }
                        self.wide.insert(uid, url);
                    }
                    for (uid, url) in icon {
                        if let Some(old) = self.icon.get(&uid) {
                            if old != &url {
                                self.ctx.forget_image(old);
                            }
                        }
                        self.icon.insert(uid, url);
                    }
                    self.sync_discord_presence();
                }
                Msg::MyProfile(uid, r) => {
                    // A reply from the account you just switched away from must never paint
                    // over the new account's (empty) profile page.
                    if self.me.as_ref().is_none_or(|p| p.user.id != uid) {
                        continue;
                    }
                    self.my_profile_loading = false;
                    match r {
                        Ok(fp) => {
                            let mut ids: Vec<u64> =
                                fp.experiences.iter().map(|g| g.universe_id).collect();
                            ids.extend(fp.favorites.iter().map(|g| g.universe_id));
                            if let Some(g) = &fp.playing_game {
                                ids.push(g.universe_id);
                            }
                            self.request_thumbs(ids);
                            self.profile_cache.insert(uid, (fp.clone(), Instant::now()));
                            self.my_profile = Some(fp);
                            self.my_profile_err = None;
                        }
                        Err(e) => {
                            self.my_profile = None;
                            self.my_profile_err = Some(e);
                        }
                    }
                }
                Msg::BioSaved(r) => {
                    self.profile_saving_bio = false;
                    match r {
                        Ok(new_bio) => {
                            if let Some(p) = &mut self.my_profile {
                                p.description = new_bio;
                            }
                            self.profile_editing = false;
                            self.profile_save_err = None;
                            self.status = "Bio updated successfully!".to_string();
                        }
                        Err(e) => {
                            self.profile_save_err = Some(e);
                        }
                    }
                }
                Msg::FriendProfile(uid, r) => {
                    if !logged_in {
                        continue;
                    }
                    let on_page = matches!(&self.page, Page::Friend(u) if u.id == uid);
                    match r {
                        Ok(fp) => {
                            // Cache even when another page is open, so a profile fetched
                            // from anywhere is fast the next time it is opened.
                            self.profile_cache.insert(uid, (fp.clone(), Instant::now()));
                            if self.profile_cache.len() > 24 {
                                if let Some(oldest) = self
                                    .profile_cache
                                    .iter()
                                    .min_by_key(|(_, (_, at))| *at)
                                    .map(|(id, _)| *id)
                                {
                                    self.profile_cache.remove(&oldest);
                                }
                            }
                            if on_page {
                                let mut ids: Vec<u64> =
                                    fp.experiences.iter().map(|g| g.universe_id).collect();
                                ids.extend(fp.favorites.iter().map(|g| g.universe_id));
                                if let Some(g) = &fp.playing_game {
                                    ids.push(g.universe_id);
                                }
                                self.request_thumbs(ids);
                                self.friend_loading = false;
                                self.friend_profile = Some(fp);
                                self.friend_err = None;
                            }
                        }
                        Err(e) => {
                            if on_page {
                                self.friend_loading = false;
                                self.friend_profile = None;
                                self.friend_err = Some(e);
                            }
                        }
                    }
                }
                Msg::FriendSearchResult(append, r) => {
                    if !logged_in {
                        continue;
                    }
                    self.friend_searching = false;
                    self.friend_search_loading_more = false;
                    match r {
                        Ok(page) => {
                            self.friend_search_cursor = page.next_cursor;
                            self.friend_search_verified.extend(page.verified);
                            if append {
                                if let Some(Ok(existing)) = &mut self.friend_search_results {
                                    let seen: HashSet<u64> =
                                        existing.iter().map(|u| u.id).collect();
                                    for u in page.users {
                                        if !seen.contains(&u.id) {
                                            existing.push(u);
                                        }
                                    }
                                }
                            } else {
                                self.friend_search_results = Some(Ok(page.users));
                            }
                        }
                        Err(e) => {
                            if !append {
                                self.friend_search_results = Some(Err(e));
                                self.friend_search_cursor = None;
                            } else {
                                self.status = format!("Could not load more players: {e}");
                            }
                        }
                    }
                }
                Msg::FriendRequestDone(uid, uname, r) => {
                    if !logged_in {
                        continue;
                    }
                    match r {
                        Ok(()) => {
                            self.friend_sent.insert(uid);
                            self.status = format!("Friend request sent to @{uname}!");
                        }
                        Err(e) => {
                            self.status = format!("Could not add @{uname}: {e}");
                        }
                    }
                }
                Msg::SocialListLoaded { tab, append, res } => {
                    if !logged_in {
                        continue;
                    }
                    self.social_loading.remove(&tab);
                    self.social_loading_more.remove(&tab);
                    match res {
                        Ok((users, next_cursor)) => {
                            self.social_errs.remove(&tab);
                            if tab == SocialTab::Friends {
                                self.friends = users;
                                self.friends_err = None;
                                if let Some(prof) = &mut self.me {
                                    prof.friends = self.friends.len() as u64;
                                }
                            } else {
                                if !append && tab == SocialTab::Requests {
                                    self.last_requests_poll = Some(Instant::now());
                                    let current_ids: HashSet<u64> =
                                        users.iter().map(|u| u.id).collect();
                                    let viewing_requests = matches!(self.page, Page::AddFriend)
                                        && self.social_tab == SocialTab::Requests;
                                    let account_uid =
                                        self.me.as_ref().map(|p| p.user.id).unwrap_or(0);
                                    let mut notify_single: Option<(u64, String, String, String)> =
                                        None;
                                    let mut notify_group: Option<(usize, String, String)> = None;
                                    let mut changed_seen = false;
                                    {
                                        let seen_set = self
                                            .seen_notifications
                                            .friend_requests_by_user
                                            .entry(account_uid)
                                            .or_default();
                                        let unseen: Vec<&User> = users
                                            .iter()
                                            .filter(|u| !seen_set.contains(&u.id))
                                            .collect();
                                        if !unseen.is_empty() {
                                            if !viewing_requests {
                                                if unseen.len() == 1 {
                                                    let u = unseen[0];
                                                    let disp = if u.display.is_empty() {
                                                        u.name.clone()
                                                    } else {
                                                        u.display.clone()
                                                    };
                                                    notify_single = Some((
                                                        u.id,
                                                        u.name.clone(),
                                                        disp,
                                                        u.headshot.clone(),
                                                    ));
                                                } else {
                                                    let first = unseen[0];
                                                    notify_group = Some((
                                                        unseen.len(),
                                                        first.name.clone(),
                                                        first.headshot.clone(),
                                                    ));
                                                }
                                            }
                                            for u in unseen {
                                                if seen_set.insert(u.id) {
                                                    changed_seen = true;
                                                }
                                            }
                                        }
                                    }
                                    if changed_seen {
                                        save_seen_notifications(&self.seen_notifications);
                                    }
                                    if let Some((uid, uname, disp, headshot)) = notify_single {
                                        self.push_toast(
                                            NotificationSource::RobloxApi,
                                            ToastKind::FriendRequest,
                                            "Friend Request",
                                            format!("{disp} (@{uname}) wants to add you."),
                                            (!headshot.is_empty()).then_some(headshot),
                                            Some((
                                                "Accept".to_string(),
                                                ToastAction::AcceptFriendRequest { uid, uname },
                                            )),
                                            Some(("View".to_string(), ToastAction::OpenRequests)),
                                            8.5,
                                            true,
                                        );
                                    } else if let Some((count, first_name, headshot)) = notify_group
                                    {
                                        self.push_toast(
                                            NotificationSource::RobloxApi,
                                            ToastKind::FriendRequest,
                                            format!("{count} Friend Requests"),
                                            format!(
                                                "@{first_name} and {} more waiting to accept.",
                                                count - 1
                                            ),
                                            (!headshot.is_empty()).then_some(headshot),
                                            Some((
                                                "View Requests".to_string(),
                                                ToastAction::OpenRequests,
                                            )),
                                            None,
                                            8.5,
                                            true,
                                        );
                                    }
                                    self.known_friend_requests = Some(current_ids);
                                }
                                if append {
                                    let entry = self.social_lists.entry(tab).or_default();
                                    let seen: HashSet<u64> = entry.iter().map(|u| u.id).collect();
                                    for u in users {
                                        if !seen.contains(&u.id) {
                                            entry.push(u);
                                        }
                                    }
                                } else {
                                    self.social_lists.insert(tab, users);
                                }
                                self.social_cursors.insert(tab, next_cursor);
                            }
                        }
                        Err(e) => {
                            if !append {
                                self.social_errs.insert(tab, e);
                            } else {
                                self.status = format!("Could not load more {}: {e}", tab.label());
                            }
                        }
                    }
                }
                Msg::FriendRequestResponded {
                    uid,
                    uname,
                    accepted,
                    res,
                } => {
                    if !logged_in {
                        continue;
                    }
                    self.social_responding.remove(&uid);
                    match res {
                        Ok(()) => {
                            if let Some(known) = &mut self.known_friend_requests {
                                known.remove(&uid);
                            }
                            let account_uid = self.me.as_ref().map(|p| p.user.id).unwrap_or(0);
                            if let Some(seen) = self
                                .seen_notifications
                                .friend_requests_by_user
                                .get_mut(&account_uid)
                            {
                                if seen.remove(&uid) {
                                    save_seen_notifications(&self.seen_notifications);
                                }
                            }
                            let mut removed_user = None;
                            if let Some(list) = self.social_lists.get_mut(&SocialTab::Requests) {
                                if let Some(pos) = list.iter().position(|u| u.id == uid) {
                                    removed_user = Some(list.remove(pos));
                                }
                            }
                            if accepted {
                                if let Some(mut u) = removed_user {
                                    if !self.friends.iter().any(|f| f.id == u.id) {
                                        if u.name.is_empty() {
                                            u.name = uname.clone();
                                        }
                                        self.friends.insert(0, u);
                                    }
                                }
                                if let Some(prof) = &mut self.me {
                                    prof.friends = self.friends.len() as u64;
                                }
                                if let Some(fp) = &mut self.my_profile {
                                    fp.friends = self.friends.len() as u64;
                                }
                                self.status = format!("Accepted friend request from @{uname}!");
                            } else {
                                self.status = format!("Declined friend request from @{uname}.");
                            }
                        }
                        Err(e) => {
                            self.status = format!("Could not update request from @{uname}: {e}");
                        }
                    }
                }
                Msg::AvatarLoaded(r) => {
                    if !logged_in {
                        continue;
                    }
                    self.avatar_loading = false;
                    match r {
                        Ok((state, palette)) => {
                            self.avatar_state = Some(state);
                            if !palette.is_empty() {
                                self.avatar_palette = palette;
                            }
                            self.avatar_err = None;
                        }
                        Err(e) => {
                            self.avatar_err = Some(e);
                        }
                    }
                }
                Msg::AvatarInventory(tab, append, r) => {
                    if !logged_in {
                        continue;
                    }
                    if self.avatar_items_loading == Some(tab) {
                        self.avatar_items_loading = None;
                    }
                    match r {
                        Ok((mut items, next_cursor)) => {
                            if append {
                                let entry = self.avatar_items.entry(tab).or_default();
                                entry.append(&mut items);
                            } else {
                                self.avatar_items.insert(tab, items);
                            }
                            self.avatar_cursors.insert(tab, next_cursor);
                            if self.avatar_tab == tab {
                                self.avatar_items_err = None;
                            }
                        }
                        Err(e) => {
                            if self.avatar_tab == tab {
                                self.avatar_items_err = Some(e);
                            }
                        }
                    }
                }
                Msg::AvatarOutfits(page, r) => {
                    if !logged_in {
                        continue;
                    }
                    self.avatar_outfits_loading = false;
                    match r {
                        Ok((mut outfits, has_more)) => {
                            self.avatar_outfits_page = page;
                            self.avatar_outfits_more = has_more;
                            if page > 1 {
                                let list = self.avatar_outfits.get_or_insert_with(Vec::new);
                                list.append(&mut outfits);
                            } else {
                                self.avatar_outfits = Some(outfits);
                            }
                            self.avatar_outfits_err = None;
                        }
                        Err(e) => {
                            self.avatar_outfits_err = Some(e);
                        }
                    }
                }
                Msg::OutfitActionDone(r) => {
                    if !logged_in {
                        continue;
                    }
                    self.avatar_outfit_saving = false;
                    match r {
                        Ok(msg) => {
                            self.status = msg;
                            self.avatar_err = None;
                            self.avatar_save_outfit_open = false;
                            self.avatar_save_outfit_name.clear();
                            self.avatar_outfits = None;
                            self.avatar_outfits_page = 1;
                            self.avatar_outfits_more = false;
                            self.load_avatar_tab(AvatarTab::Outfits, false);
                        }
                        Err(e) => {
                            self.avatar_err = Some(e);
                        }
                    }
                }
                Msg::AvatarApplied(seq, r) => {
                    if !logged_in {
                        continue;
                    }
                    self.avatar_pending = self.avatar_pending.saturating_sub(1);
                    match r {
                        Ok((state, avatar_url, headshot_url, note)) => {
                            if seq == self.avatar_seq {
                                self.avatar_state = Some(state);
                                self.avatar_err = None;
                            }
                            if let Some(prof) = &mut self.me {
                                if !avatar_url.is_empty() {
                                    if !prof.avatar.is_empty() && prof.avatar != avatar_url {
                                        self.ctx.forget_image(&prof.avatar);
                                    }
                                    prof.avatar = avatar_url;
                                }
                                if !headshot_url.is_empty() {
                                    if !prof.user.headshot.is_empty()
                                        && prof.user.headshot != headshot_url
                                    {
                                        self.ctx.forget_image(&prof.user.headshot);
                                    }
                                    prof.user.headshot = headshot_url;
                                }
                            }
                            if seq == self.avatar_seq {
                                self.status = note;
                                self.avatar_3d.loaded_user_id = None;
                                self.avatar_3d.expect_change = true;
                                self.load_avatar_3d();
                            }
                        }
                        Err(e) => {
                            if seq == self.avatar_seq {
                                self.avatar_err = Some(e.clone());
                                self.status = format!("Avatar update failed: {e}");
                                self.load_avatar();
                            }
                        }
                    }
                }
                Msg::Avatar3DLoaded(uid, res) => {
                    self.avatar_3d.loading = false;
                    let run_again = std::mem::take(&mut self.avatar_3d.reload_queued);
                    match res {
                        Ok((model, raw_textures)) => {
                            self.avatar_3d.loaded_user_id = Some(uid);
                            self.avatar_3d.model = Some(model);
                            self.avatar_3d.err = None;
                            self.avatar_3d.textures.clear();
                            for (hash, bytes) in raw_textures {
                                if let Ok(img) = image::load_from_memory(&bytes) {
                                    let size = [img.width() as _, img.height() as _];
                                    let mut rgba = img.to_rgba8();
                                    // Roblox's avatar textures can carry alpha (clothing
                                    // cut-outs, faded decals). CREBLOX stacks the layers by
                                    // depth, so a transparent texel would let whatever is
                                    // behind show through the clothes. Bake the alpha out.
                                    for px in rgba.pixels_mut() {
                                        px.0[3] = 255;
                                    }
                                    let color_img = egui::ColorImage::from_rgba_unmultiplied(
                                        size,
                                        rgba.as_flat_samples().as_slice(),
                                    );
                                    self.avatar_3d.textures.push(avatar_3d::LoadedTexture {
                                        hash,
                                        image: color_img,
                                        handle: None,
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            self.avatar_3d.err = Some(e);
                        }
                    }
                    if run_again {
                        self.avatar_3d.expect_change = true;
                        self.load_avatar_3d();
                    }
                }
                Msg::AuthSession(r) => {
                    if !logged_in {
                        continue;
                    }
                    self.auth_sync_started = None;
                    self.last_auth_sync = Some(Instant::now());
                    match r {
                        Ok((profile, friends)) => {
                            let profile_id = profile.user.id;
                            self.status.clear();
                            if let Some(prev) = &self.me {
                                if prev.user.headshot != profile.user.headshot
                                    && !prev.user.headshot.is_empty()
                                {
                                    self.ctx.forget_image(&prev.user.headshot);
                                }
                            }
                            let prev_shots: HashMap<u64, String> = self
                                .friends
                                .iter()
                                .map(|u| (u.id, u.headshot.clone()))
                                .collect();
                            for f in &friends {
                                if let Some(old) = prev_shots.get(&f.id) {
                                    if old != &f.headshot && !old.is_empty() {
                                        self.ctx.forget_image(old);
                                    }
                                }
                            }
                            self.record_active_account(&profile);
                            self.me = Some(profile);
                            self.me_err = None;
                            crate::api::save_friends_cache(&friends);
                            self.friends = friends;
                            self.friends_err = None;
                            self.sync_tray_state();
                            if self.known_friend_requests.is_none()
                                && !self.social_loading.contains(&SocialTab::Requests)
                            {
                                self.last_requests_poll = Some(Instant::now());
                                self.load_social_tab(SocialTab::Requests, false);
                            }
                            if matches!(self.page, Page::Avatar) {
                                self.ensure_avatar_loaded();
                            }
                            if matches!(self.page, Page::Profile) {
                                // Show this account's cached profile straight away while
                                // the fresh copy loads in the background.
                                if let Some((cached, _)) = self.profile_cache.get(&profile_id) {
                                    self.my_profile = Some(cached.clone());
                                }
                                self.my_profile_loading = false;
                                self.ensure_my_profile_loaded();
                            }
                        }
                        Err(e) => {
                            if e.contains("Session expired")
                                || e.contains("401")
                                || e.contains("403")
                            {
                                self.sign_out();
                                self.status =
                                    format!("Session expired — please sign in again ({e})");
                            } else if self.me.is_none() {
                                self.me_err = Some(e.clone());
                                self.status = e;
                            }
                        }
                    }
                }
                Msg::QuickLoginCreated { code, qr_bytes } => {
                    if let Some(ql) = &mut self.quick_login {
                        ql.qr_uri = format!("bytes://ql_qr_{code}.png");
                        ql.code = code;
                        ql.qr_bytes = qr_bytes;
                        ql.status_text =
                            "Waiting for you to enter this code in Roblox...".to_string();
                    }
                }
                Msg::QuickLoginStatus {
                    code,
                    status,
                    account,
                } => {
                    if let Some(ql) = &mut self.quick_login {
                        if ql.code == code {
                            ql.status_text = match (status.as_str(), account) {
                                ("UserLinked", Some(acc)) => {
                                    format!("Linked to {acc}! Confirm on your device to finish...")
                                }
                                ("UserLinked", None) => {
                                    "Code linked! Confirm on your device to finish...".to_string()
                                }
                                _ => "Waiting for you to enter this code in Roblox...".to_string(),
                            };
                        }
                    }
                }
                Msg::QuickLoginDone(r) => match r {
                    Ok(cookie) => {
                        self.cancel_quick_login();
                        self.reset_session_caches();
                        if let Err(e) = save_cookie(&cookie) {
                            self.status = format!("Signed in, but could not save session: {e}");
                        } else {
                            self.status = "Signed in! Loading your Roblox profile...".to_string();
                        }
                        self.cookie = Some(cookie);
                        self.page = Page::Home;
                        self.back = Page::Home;
                        self.sync_sorts(true);
                        self.sync_saved(true);
                        self.sync_auth_session(true);
                        self.sync_discord_presence();
                    }
                    Err(e) => {
                        if let Some(ql) = &mut self.quick_login {
                            ql.error = Some(e);
                            ql.status_text.clear();
                        }
                    }
                },
                Msg::ServerLogoutDone(r) => {
                    if self.cookie.is_none() {
                        self.status = match r {
                            Ok(()) => "Signed out — session revoked on Roblox.".to_string(),
                            Err(_) => {
                                "Signed out locally (could not reach Roblox to revoke token)."
                                    .to_string()
                            }
                        };
                    }
                }
                Msg::LaunchResult(r) => {
                    if r.is_err() {
                        self.pending_discord_launch = None;
                        if self.active_roblox_place_id.is_none() {
                            self.active_roblox_started_at = None;
                        }
                        self.sync_discord_presence();
                    }
                    if r.is_ok() {
                        // Start the overlays with the game itself instead of waiting for the
                        // watchdog to notice the session.
                        self.maybe_show_game_overlay();
                    }
                    if r.is_ok() && self.settings.close_launcher_during_game {
                        // Take only the window away: CREBLOX itself keeps running so the
                        // menu-bar icon, the overlays and the settings stay available while
                        // the game is on (and it never looks like CREBLOX "quit").
                        self.hidden_for_game = true;
                        self.hidden_at = Some(Instant::now());
                        let ctx = self.ctx.clone();
                        self.hide_to_tray(&ctx, false);
                    } else if r.is_ok() && self.settings.minimize_on_game_launch {
                        self.ctx
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    self.status = match r {
                        Ok(s) => s,
                        Err(e) => format!("Launch error: {e}"),
                    };
                }
                Msg::RobloxServerUpdated(info) => {
                    if self.server_info != info {
                        if let Some(i) = &info {
                            let ip = i.ip.clone();
                            if !self.server_locations.contains_key(&ip)
                                && self.server_location_pending.insert(ip.clone())
                            {
                                self.spawn(move || {
                                    let loc = crate::api::lookup_server_location(&ip);
                                    Msg::ServerLocationResolved(ip, loc)
                                });
                            }
                        }
                        self.server_info = info;
                        // Even without a server (Roblox menu, log not written yet) the
                        // overlay helper stays alive while Roblox runs; it hides itself
                        // through `game_active` instead of being torn down here.
                        self.maybe_show_game_overlay();
                        self.sync_tray_state();
                    }
                }
                Msg::ServerLocationResolved(ip, loc) => {
                    self.server_location_pending.remove(&ip);
                    let text = loc.unwrap_or_else(|| "Unknown location".to_string());
                    self.server_locations.insert(ip.clone(), text.clone());
                    if self.server_info.as_ref().is_some_and(|s| s.ip == ip) {
                        self.maybe_show_game_overlay();
                        self.sync_tray_state();
                        self.push_toast(
                            NotificationSource::Creblox,
                            ToastKind::Success,
                            "Connected to server",
                            text,
                            None,
                            None,
                            None,
                            5.0,
                            true,
                        );
                    }
                }
                Msg::RobloxAppClosed => {
                    self.finish_playtime_session();
                    let was_playing = self.active_roblox_place_id.is_some();
                    self.active_roblox_place_id = None;
                    self.active_roblox_started_at = None;
                    self.pending_discord_launch = None;
                    self.sync_discord_presence();
                    if self.status.contains("Launching")
                        || self.status.contains("Joining")
                        || self.status.contains("Getting launch ticket")
                    {
                        self.status = "Closed Roblox app after session.".to_string();
                    }
                    // The launcher always comes back once Roblox is gone - whether the
                    // window was closed, minimized or just left behind the game.
                    let _ = was_playing;
                    self.return_to_launcher();
                    if logged_in {
                        self.sync_saved(true);
                        self.sync_auth_session(true);
                    }
                }
                Msg::RobloxProcesses(running) => {
                    if self.roblox_running != running {
                        self.roblox_running = running;
                        crate::auth::log(&format!(
                            "watcher: Roblox processes {}",
                            if running { "present" } else { "gone" }
                        ));
                        self.maybe_show_game_overlay();
                        self.sync_tray_state();
                    }
                    // The hidden-window watchdog otherwise waits for its next frame. React
                    // to the actual process-exit event so the launcher comes straight back.
                    if !running && self.hidden_for_game {
                        let hidden_long_enough = self
                            .hidden_at
                            .is_none_or(|t| t.elapsed() >= Duration::from_secs(4));
                        if hidden_long_enough {
                            self.finish_playtime_session();
                            self.return_to_launcher();
                        }
                    }
                }
                Msg::RobloxSessionUpdated(place_opt) => {
                    if self.active_roblox_place_id != place_opt {
                        match place_opt {
                            Some(pid) => {
                                let matching_pending = self
                                    .pending_discord_launch
                                    .as_ref()
                                    .is_some_and(|(p, _, _)| *p == pid);
                                if let Some((p, Some(g), _)) = self.pending_discord_launch.take() {
                                    if p == pid {
                                        let icon = self.icon.get(&g.universe_id).cloned();
                                        self.discord_resolved_games.entry(pid).or_insert((g, icon));
                                    }
                                }
                                self.active_roblox_place_id = Some(pid);
                                crate::auth::log(&format!("session: joined place {pid}"));
                                // Overlays come up for every session, even when the server
                                // IP is never resolved (the old code only started them on a
                                // server update, so they could silently never appear).
                                self.maybe_show_game_overlay();
                                if self.playtime_session.map(|(p, _)| p) != Some(pid) {
                                    self.playtime_session = Some((pid, Instant::now()));
                                }
                                if !matching_pending || self.active_roblox_started_at.is_none() {
                                    self.active_roblox_started_at = Some(now_unix_ms() / 1000);
                                }
                            }
                            None => {
                                // The game session ended (player pressed Leave / closed Roblox).
                                self.finish_playtime_session();
                                let was_playing = self.active_roblox_place_id.is_some();
                                self.active_roblox_place_id = None;
                                self.active_roblox_started_at = None;
                                // Keep the overlay helper but hide it: re-joining from the
                                // Roblox menu must not need a process respawn.
                                self.maybe_show_game_overlay();
                                if was_playing || self.hidden_for_game {
                                    self.return_to_launcher();
                                }
                            }
                        }
                        self.sync_discord_presence();
                    }
                }
                Msg::DiscordGameResolved(pid, resolved) => {
                    self.discord_resolving_places.remove(&pid);
                    if let Some((g, icon_url)) = resolved {
                        self.request_thumbs(vec![g.universe_id]);
                        self.discord_resolved_games.insert(pid, (g, icon_url));
                        self.sync_discord_presence();
                    }
                }
                Msg::RobloxKilled(closed) => {
                    self.status = if closed == 0 {
                        "No running Roblox process to close".to_string()
                    } else {
                        format!("Closed {closed} Roblox process(es)")
                    };
                    self.sync_tray_state();
                }
                Msg::TrayAction(ev) => {
                    crate::auth::log(&format!("tray: {ev:?}"));
                    match ev {
                        TrayEvent::ShowAndFocus => self.restore_from_tray(),
                        TrayEvent::HideToTray => {
                            let ctx = self.ctx.clone();
                            self.hide_to_tray(&ctx, false);
                        }
                        TrayEvent::OpenHome
                        | TrayEvent::OpenProfile
                        | TrayEvent::OpenAvatar
                        | TrayEvent::OpenNotifications
                        | TrayEvent::OpenSettings
                        | TrayEvent::OpenFastFlags => {
                            self.restore_from_tray();
                            if self.cookie.is_some() {
                                self.page = match ev {
                                    TrayEvent::OpenHome => Page::Home,
                                    TrayEvent::OpenProfile => Page::Profile,
                                    TrayEvent::OpenAvatar => Page::Avatar,
                                    TrayEvent::OpenNotifications => Page::Notifications,
                                    _ => Page::Settings,
                                };
                                self.sync_discord_presence();
                            }
                            self.sync_tray_state();
                        }
                        TrayEvent::ToggleServerOverlay => {
                            self.settings.overlay_enabled = !self.settings.overlay_enabled;
                            save_settings(&self.settings);
                            self.maybe_show_game_overlay();
                            self.sync_tray_state();
                        }
                        TrayEvent::ToggleStatsOverlay => {
                            self.settings.overlay_stats_enabled =
                                !self.settings.overlay_stats_enabled;
                            save_settings(&self.settings);
                            self.maybe_show_game_overlay();
                            self.sync_tray_state();
                        }
                        TrayEvent::ToggleCrosshair => {
                            self.settings.crosshair_enabled = !self.settings.crosshair_enabled;
                            save_settings(&self.settings);
                            self.maybe_show_game_overlay();
                            self.sync_tray_state();
                        }
                        TrayEvent::EditOverlayPosition => {
                            // The overlay process owns the pointer and the launcher owns
                            // settings.json, so the answer comes back as one small file
                            // (polled below) instead of two processes writing one config.
                            if !self.overlay_editing {
                                self.overlay_editing = true;
                                self.status =
                                    "Drag the HUD, then Confirm (⏎) — Esc cancels".to_string();
                                self.maybe_show_game_overlay();
                                self.game_overlay.set_edit(true);
                                if !self.game_overlay.is_running() {
                                    // Never leave the menu in "Editing…" with nothing on
                                    // screen to edit: that would look like a hang.
                                    self.overlay_editing = false;
                                    self.status = "Could not open the HUD editor".to_string();
                                }
                                self.sync_tray_state();
                            }
                        }
                        TrayEvent::CopyJobId => {
                            if let Some(info) = self.server_info.clone() {
                                if !info.job_id.is_empty() {
                                    self.ctx.copy_text(info.job_id.clone());
                                    self.status = format!("Copied Job ID {}", info.job_id);
                                }
                            }
                        }
                        TrayEvent::CloseRoblox => {
                            // Deliberately the same call Settings uses, so the tray cannot
                            // close a different set of processes than the UI would - but off
                            // the UI thread: it shells out to kill per process, and a menu
                            // click must never stall the window while that runs.
                            self.status = "Closing Roblox…".to_string();
                            self.roblox_running = false;
                            self.sync_tray_state();
                            let tx = self.tx.clone();
                            let ctx = self.ctx.clone();
                            std::thread::spawn(move || {
                                let closed = crate::roblox_process::kill_all_roblox_processes();
                                let _ = tx.send(Msg::RobloxKilled(closed));
                                ctx.request_repaint();
                            });
                        }
                        TrayEvent::SwitchAccountSlot(slot) => {
                            // The menu only knows slot indexes; the account list lives here,
                            // so a stale menu can never switch to a deleted account.
                            if let Some(account) = self.saved_accounts.get(slot) {
                                let user_id = account.id;
                                self.switch_account(user_id);
                                self.sync_tray_state();
                            }
                        }
                        TrayEvent::ApplyQualityPreset => {
                            // The same calls the Settings buttons make, so the menu cannot
                            // leave FastFlags in a state the UI does not show.
                            self.fastflags.apply_quality_preset();
                            let _ = save_fastflags(&self.fastflags);
                            let _ = crate::fastflags::sync_saved_fastflags_before_launch();
                            self.status = "Applied the Quality FastFlags preset".to_string();
                            self.sync_tray_state();
                        }
                        TrayEvent::ApplyCompetitivePreset => {
                            self.fastflags.apply_competitive_preset();
                            let _ = save_fastflags(&self.fastflags);
                            let _ = crate::fastflags::sync_saved_fastflags_before_launch();
                            self.status = "Applied the Max FPS FastFlags preset".to_string();
                            self.sync_tray_state();
                        }
                        TrayEvent::ToggleDiscordRpc => {
                            self.settings.discord_rpc_enabled = !self.settings.discord_rpc_enabled;
                            save_settings(&self.settings);
                            self.sync_discord_presence();
                            self.sync_tray_state();
                        }
                        TrayEvent::CheckUpdates => {
                            self.restore_from_tray();
                            self.check_app_updates(true);
                            self.sync_tray_state();
                        }
                        TrayEvent::QuitApp => self.quit_app(),
                    }
                }
                Msg::RobloxBackgroundDetected { pid, is_playing } => {
                    if !is_playing
                        && !self.settings.auto_close_roblox
                        && self.notified_kill_pids.insert(pid)
                        && self
                            .last_autokill_toast
                            .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
                    {
                        self.last_autokill_toast = Some(Instant::now());
                        self.push_toast(
                            NotificationSource::Process,
                            ToastKind::Error,
                            "Roblox Running in Background",
                            format!(
                                "{} (PID {pid}) is open in the background.",
                                crate::platform::player_label()
                            ),
                            None,
                            Some(("Kill Process".to_string(), ToastAction::KillRoblox)),
                            None,
                            7.5,
                            true,
                        );
                    }
                }
                Msg::RobloxBackgroundKilled { pid } => {
                    self.status = format!("Closed background Roblox process (PID {pid}).");
                    if self.notified_kill_pids.insert(pid)
                        && self
                            .last_autokill_toast
                            .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
                    {
                        self.last_autokill_toast = Some(Instant::now());
                        self.push_toast(
                            NotificationSource::Process,
                            ToastKind::Success,
                            "Closed Background Roblox",
                            format!("Terminated lingering Roblox process (PID {pid})."),
                            None,
                            None,
                            None,
                            5.0,
                            true,
                        );
                    }
                }
                Msg::UpdateCheckResult(manual, res) => match res {
                    Ok(Some((ver, upload))) => {
                        self.roblox_update_available = Some((ver.clone(), upload.clone()));
                        if manual {
                            self.status = format!("Roblox update available: v{ver}");
                        }
                        if self.seen_notifications.update_uploads.insert(upload) {
                            save_seen_notifications(&self.seen_notifications);
                            self.push_toast(
                                NotificationSource::RobloxApi,
                                ToastKind::UpdateAvailable,
                                "Roblox Update Available",
                                format!(
                                    "Version {ver} is live and newer than your installed Roblox build."
                                ),
                                None,
                                Some((
                                    "Download Update".to_string(),
                                    ToastAction::OpenUrl(
                                        "https://www.roblox.com/download/client".to_string(),
                                    ),
                                )),
                                None,
                                10.0,
                                true,
                            );
                        }
                    }
                    Ok(None) => {
                        self.roblox_update_available = None;
                        if manual {
                            self.status = "Roblox Player is up to date.".to_string();
                        }
                    }
                    Err(e) => {
                        if manual {
                            self.status = format!("Update check failed: {e}");
                        }
                    }
                },
                Msg::AppUpdateCheckResult(manual, res) => {
                    self.app_update_checking = false;
                    match res {
                        Ok(Some(release)) => {
                            let ver = release.version.clone();
                            let key = format!("creblox:{}", release.tag_name);
                            self.app_update_status = Some(format!("Update available: v{ver}"));
                            self.app_update_available = Some(release.clone());
                            let cached_zip =
                                crate::updater::downloaded_update_path(&release.version);
                            self.app_update_zip = cached_zip.is_file().then_some(cached_zip);
                            if self.settings.auto_download_updates
                                && cfg!(target_os = "macos")
                                && release.zip_url.is_some()
                                && self.app_update_zip.is_none()
                            {
                                self.start_app_update_download(false);
                            }
                            if manual && !self.app_update_downloading {
                                self.status = format!("Update v{ver} is available!");
                            }
                            if self.seen_notifications.update_uploads.insert(key) {
                                save_seen_notifications(&self.seen_notifications);
                                self.push_toast(
                                    NotificationSource::Creblox,
                                    ToastKind::UpdateAvailable,
                                    format!("Update v{ver} Available"),
                                    if release.notes.is_empty() {
                                        "New update ready. Click below to update inside Settings."
                                            .to_string()
                                    } else {
                                        release
                                            .notes
                                            .lines()
                                            .find(|l| !l.trim().is_empty())
                                            .unwrap_or("New update ready.")
                                            .trim()
                                            .chars()
                                            .take(90)
                                            .collect()
                                    },
                                    None,
                                    Some((
                                        "Update in Settings".to_string(),
                                        ToastAction::OpenSettings,
                                    )),
                                    None,
                                    10.0,
                                    true,
                                );
                            }
                        }
                        Ok(None) => {
                            self.app_update_available = None;
                            self.app_update_zip = None;
                            if manual {
                                let msg = format!(
                                    "You are on the latest version (v{}).",
                                    env!("CARGO_PKG_VERSION")
                                );
                                self.app_update_status = Some(msg.clone());
                                self.status = msg;
                            } else {
                                self.app_update_status =
                                    Some(format!("Up to date (v{})", env!("CARGO_PKG_VERSION")));
                            }
                        }
                        Err(e) => {
                            self.app_update_error = if manual { Some(e.clone()) } else { None };
                            if manual {
                                self.app_update_status = None;
                                self.status = format!("Update check failed: {e}");
                            }
                        }
                    }
                }
                Msg::AppUpdateDownloaded(ver, res) => {
                    self.app_update_downloading = false;
                    self.app_update_progress = None;
                    match res {
                        Ok(path) => {
                            if matches!(&self.app_update_available, Some(r) if r.version == ver) {
                                self.app_update_zip = Some(path);
                                self.app_update_status =
                                    Some(format!("v{ver} downloaded — ready to install."));
                                self.status =
                                    format!("v{ver} downloaded — click Install & Restart.");
                            }
                        }
                        Err(e) => {
                            self.app_update_error = Some(e.clone());
                            self.app_update_status = None;
                            self.status = format!("Update download failed: {e}");
                        }
                    }
                }
                Msg::AppPatchDone(ver, res) => {
                    self.app_update_downloading = false;
                    match res {
                        Ok((hotfix, staged)) => {
                            let mut note = None;
                            if let Some(hf) = hotfix {
                                let mut ff_changed = false;
                                for (k, v) in hf.fastflag_overrides {
                                    self.fastflags.custom_flags.insert(k, v);
                                    ff_changed = true;
                                }
                                if ff_changed {
                                    let _ = save_fastflags(&self.fastflags);
                                    let _ = sync_client_fastflags(
                                        &self.fastflags,
                                        self.settings.target_fps,
                                        &[],
                                    );
                                }
                                if let Some(auto_kill) = hf.auto_close_roblox {
                                    self.settings.auto_close_roblox = auto_kill;
                                    save_settings(&self.settings);
                                    self.auto_close_flag.store(auto_kill, Ordering::Relaxed);
                                }
                                note = hf.status_note;
                            }
                            if let Some(staged_path) = staged {
                                self.app_staged_patch = Some(staged_path);
                            }
                            self.app_patch_applied = Some(ver.clone());
                            let summary = note.unwrap_or_else(|| {
                                if self.app_staged_patch.is_some() {
                                    format!(
                                        "Patch v{ver} applied — no restart needed (finishes on exit)."
                                    )
                                } else {
                                    format!("Patch v{ver} applied live.")
                                }
                            });
                            self.app_update_status = Some(summary.clone());
                            self.status = summary;
                        }
                        Err(e) => {
                            self.app_update_error = Some(e.clone());
                            self.app_update_status = None;
                            self.status = format!("Patch failed: {e}");
                        }
                    }
                }
            }
        }
        if let Some((_, _, started)) = self.pending_discord_launch {
            if started.elapsed() >= Duration::from_secs(20) {
                self.pending_discord_launch = None;
                self.sync_discord_presence();
            }
        }
        had_messages
    }

    pub fn current_card_preview(&self) -> Option<CardPreview<'_>> {
        let uid = self.hovered_game_id?;
        let vid_opt = self.game_video_ids.get(&uid);
        let is_loading = match vid_opt {
            Some(Some(vid)) => !self.decoded_videos.contains_key(vid),
            None => self.video_id_requested.contains(&uid),
            Some(None) => false,
        };
        let tex = if let Some((active_uid, _, tex)) = &self.active_video_texture {
            if *active_uid == uid { Some(tex) } else { None }
        } else {
            None
        };
        Some(CardPreview {
            universe_id: uid,
            texture: tex,
            is_loading,
        })
    }

    pub fn handle_game_hover(&mut self, hovered: Option<u64>, ctx: &egui::Context) {
        let now = Instant::now();
        if hovered != self.hovered_game_id {
            self.hovered_game_id = hovered;
            self.hover_start = hovered.map(|_| now);
            self.hover_play_start = None;
            self.active_video_texture = None;
        }

        let Some(uid) = self.hovered_game_id else {
            return;
        };
        if !self.settings.video_previews {
            return;
        }

        // If video is already decoded in memory, start playing immediately!
        let has_cached_video = self
            .game_video_ids
            .get(&uid)
            .and_then(|opt| *opt)
            .map(|vid| self.decoded_videos.contains_key(&vid))
            .unwrap_or(false);

        if !has_cached_video {
            let dwell_passed = self
                .hover_start
                .map(|t| now.duration_since(t).as_millis() >= 75)
                .unwrap_or(false);
            if !dwell_passed {
                ctx.request_repaint_after(Duration::from_millis(40));
                return;
            }
        }

        // 1. Resolve video_id if not yet known
        match self.game_video_ids.get(&uid) {
            None => {
                if !self.video_id_requested.contains(&uid) {
                    self.video_id_requested.insert(uid);
                    self.spawn(move || {
                        let vid = fetch_game_preview_video_id(uid);
                        Msg::GameVideoIdResolved(uid, vid)
                    });
                }
                ctx.request_repaint_after(Duration::from_millis(60));
            }
            Some(None) => {
                // Confirmed no video for this game
            }
            Some(Some(vid)) => {
                let vid = *vid;
                // 2. Fetch and decode if not in decoded_videos
                if let Some(video) = self.decoded_videos.get(&vid) {
                    if self.hover_play_start.is_none() {
                        self.hover_play_start = Some(now);
                    }
                    let elapsed_ms = self
                        .hover_play_start
                        .map(|t| t.elapsed().as_millis() as u64)
                        .unwrap_or(0);
                    if let Some(frame) = video.frame_at(elapsed_ms) {
                        let mut need_new_tex = false;
                        if let Some((cur_uid, cur_ts, tex)) = &mut self.active_video_texture {
                            if *cur_uid == uid {
                                if *cur_ts != frame.timestamp_ms {
                                    tex.set(frame.image.clone(), egui::TextureOptions::LINEAR);
                                    *cur_ts = frame.timestamp_ms;
                                }
                            } else {
                                need_new_tex = true;
                            }
                        } else {
                            need_new_tex = true;
                        }

                        if need_new_tex {
                            let tex_name = format!("preview_video_{vid}");
                            let tex = ctx.load_texture(
                                tex_name,
                                frame.image.clone(),
                                egui::TextureOptions::LINEAR,
                            );
                            self.active_video_texture = Some((uid, frame.timestamp_ms, tex));
                        }

                        // Schedule next frame redraw
                        ctx.request_repaint_after(Duration::from_millis(33));
                    }
                } else if !self.video_decode_requested.contains(&vid) {
                    self.video_decode_requested.insert(vid);
                    let cookie = self.cookie.clone();
                    self.spawn(move || {
                        let res = fetch_and_decode_preview_video(vid, 320, cookie.as_deref())
                            .map(Arc::new);
                        Msg::VideoPreviewLoaded(vid, res)
                    });
                    ctx.request_repaint_after(Duration::from_millis(60));
                } else {
                    ctx.request_repaint_after(Duration::from_millis(60));
                }
            }
        }
    }

    pub fn open_game_details(&mut self, g: Game) {
        if !matches!(self.page, Page::Game(_)) {
            self.back = self.page.clone();
        }
        let uid = g.universe_id;
        let cookie = self.cookie.clone();
        self.request_thumbs(vec![uid]);
        self.game_tab = GameTab::About;
        self.game_extra = None;
        self.game_media_idx = 0;
        self.game_passes_loading = false;
        self.game_passes_err = None;
        self.game_servers_loading = false;
        self.game_servers_err = None;
        self.server_sort = ServerSort::Default;
        self.server_exclude_full = false;
        // Reopening a game paints its last details immediately (including the server
        // list) and only revalidates once they are two minutes old.
        let cached = self.game_extra_cache.get(&uid).cloned();
        let fresh = cached
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(120));
        if let Some((ex, _)) = &cached {
            self.game_video_ids.insert(uid, ex.preview_video_id.clone());
            self.game_extra = Some(ex.clone());
        }
        self.page = Page::Game(g);
        self.spawn(move || Msg::Details(games_by_universe(&[uid]).into_iter().next()));
        if !fresh {
            let cookie = cookie.clone();
            self.spawn(move || {
                Msg::GameExtraLoaded(Box::new(fetch_game_extra(uid, cookie.as_deref())))
            });
        }
    }

    pub fn load_game_passes(&mut self, universe_id: u64) {
        if self.game_passes_loading {
            return;
        }
        let cookie = self.cookie.clone();
        self.game_passes_loading = true;
        self.game_passes_err = None;
        self.spawn(move || {
            Msg::GamePassesLoaded(
                universe_id,
                fetch_game_passes(universe_id, cookie.as_deref()),
            )
        });
    }

    pub fn load_social_tab(&mut self, tab: SocialTab, append: bool) {
        if self.social_loading.contains(&tab) || self.social_loading_more.contains(&tab) {
            return;
        }
        let cursor = if append {
            let Some(Some(c)) = self.social_cursors.get(&tab).cloned() else {
                return;
            };
            Some(c)
        } else {
            None
        };
        let Some(cookie) = self.cookie.clone() else {
            return;
        };
        let my_id = self.me.as_ref().map(|p| p.user.id).unwrap_or(0);
        if tab != SocialTab::Requests && my_id == 0 {
            return;
        }
        if append {
            self.social_loading_more.insert(tab);
        } else {
            self.social_loading.insert(tab);
            self.social_errs.remove(&tab);
        }
        self.spawn(move || {
            let res = match tab {
                SocialTab::Requests => fetch_friend_requests_api(cursor.as_deref(), &cookie),
                SocialTab::Friends => fetch_user_friends(my_id, Some(&cookie)).map(|u| (u, None)),
                SocialTab::Followers => {
                    fetch_follow_list_api(my_id, false, cursor.as_deref(), Some(&cookie))
                }
                SocialTab::Following => {
                    fetch_follow_list_api(my_id, true, cursor.as_deref(), Some(&cookie))
                }
            };
            Msg::SocialListLoaded { tab, append, res }
        });
    }

    pub fn load_game_servers(&mut self, universe_id: u64, place_id: u64, append: bool) {
        if self.game_servers_loading {
            return;
        }
        let cursor = if append {
            let Some(c) = self
                .game_extra
                .as_ref()
                .filter(|ex| ex.universe_id == universe_id)
                .and_then(|ex| ex.servers_cursor.clone())
            else {
                return;
            };
            Some(c)
        } else {
            None
        };
        let cookie = self.cookie.clone();
        self.game_servers_loading = true;
        self.game_servers_err = None;
        self.spawn(move || {
            Msg::GameServersLoaded(
                universe_id,
                append,
                fetch_game_servers(place_id, cursor.as_deref(), cookie.as_deref()),
            )
        });
    }

    pub fn launch_place_id(&self, place_id: u64) {
        if let Some(cookie) = self.cookie.clone() {
            self.last_roblox_launch_ms
                .store(now_unix_ms(), Ordering::Relaxed);
            self.spawn(move || Msg::LaunchResult(launch_authenticated(place_id, &cookie)));
        }
    }

    pub fn launch_server(&self, place_id: u64, job_id: String) {
        if let Some(cookie) = self.cookie.clone() {
            self.last_roblox_launch_ms
                .store(now_unix_ms(), Ordering::Relaxed);
            self.spawn(move || {
                Msg::LaunchResult(launch_authenticated_server(
                    place_id,
                    Some(&job_id),
                    &cookie,
                ))
            });
        }
    }

    pub fn launch_private_server(&self, place_id: u64, link_code: String) {
        if let Some(cookie) = self.cookie.clone() {
            self.last_roblox_launch_ms
                .store(now_unix_ms(), Ordering::Relaxed);
            self.spawn(move || {
                Msg::LaunchResult(launch_authenticated_private_server(
                    place_id, &link_code, &cookie,
                ))
            });
        }
    }

    pub fn apply_resolved_game_link(
        &mut self,
        resolved: ResolvedGameLink,
        intent: ManualGameIntent,
    ) {
        self.request_thumbs(vec![resolved.game.universe_id]);
        match intent {
            ManualGameIntent::Open => {
                self.active_private_server = resolved
                    .private_server_code
                    .map(|code| (resolved.game.universe_id, resolved.place_id, code));
                self.open_game_details(resolved.game);
            }
            ManualGameIntent::Favorite => {
                self.add_favorite(&resolved.game);
            }
            ManualGameIntent::Play => {
                self.record_recent_game(resolved.game.clone());
                self.note_discord_game_launch(resolved.place_id, Some(resolved.game.clone()));
                if let Some(code) = resolved.private_server_code {
                    self.status = format!("Joining private server for {}...", resolved.game.name);
                    self.launch_private_server(resolved.place_id, code);
                } else {
                    self.status = format!("Launching {}...", resolved.game.name);
                    self.launch_place_id(resolved.place_id);
                }
            }
        }
    }

    pub fn play(&mut self, g: &Game) {
        if self.cookie.is_none() {
            return;
        }
        self.status = "Getting launch ticket...".to_string();
        self.note_discord_game_launch(g.place_id, Some(g.clone()));
        self.launch_place_id(g.place_id);
        self.record_recent_game(g.clone());
    }

    // ----- pages -----

    pub fn toggle_fav(&mut self, g: &Game) {
        let now_fav = if self.fav_ids.contains(&g.place_id) {
            self.fav_ids.retain(|p| *p != g.place_id);
            self.favs.retain(|x| x.universe_id != g.universe_id);
            self.status = format!("Removed {} from favorites", g.name);
            false
        } else {
            self.fav_ids.push(g.place_id);
            self.favs.retain(|x| x.universe_id != g.universe_id);
            self.favs.insert(0, g.clone());
            self.status = format!("Added {} to favorites", g.name);
            true
        };
        self.pending_favs
            .insert(g.universe_id, (now_fav, Instant::now()));
        if let Some(ex) = &mut self.game_extra {
            if ex.universe_id == g.universe_id {
                ex.is_favorited_api = Some(now_fav);
                ex.favorites = if now_fav {
                    ex.favorites.saturating_add(1)
                } else {
                    ex.favorites.saturating_sub(1)
                };
            }
        }
        write_ids("favorites.txt", &self.fav_ids);
        if let Some(cookie) = self.cookie.clone() {
            let uid = g.universe_id;
            self.spawn(move || {
                Msg::GameFavDone(uid, now_fav, favorite_game_api(uid, now_fav, &cookie))
            });
        }
    }

    pub fn add_favorite(&mut self, g: &Game) {
        if self.fav_ids.contains(&g.place_id) {
            self.status = format!("{} is already in Favorites", g.name);
        } else {
            self.toggle_fav(g);
        }
    }

    pub fn render_manual_game_modal(&mut self, ctx: &egui::Context) {
        if !self.manual_game_open {
            return;
        }
        let screen_rect = ctx.input(|i| i.content_rect());
        if !screen_rect.is_positive() {
            return;
        }
        let mut close_modal =
            ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !self.manual_game_loading;
        let mut trigger_intent: Option<ManualGameIntent> = None;

        egui::Area::new(egui::Id::NULL.with("manual_game_backdrop"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen_rect.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(140));
                if resp.clicked() && !self.manual_game_loading {
                    close_modal = true;
                }
            });

        let modal_w = (screen_rect.width() - 48.0).clamp(320.0, 480.0);
        egui::Area::new(egui::Id::NULL.with("manual_game_modal"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let p = pal(ui);
                egui::Frame::new()
                    .fill(p.card_bg)
                    .stroke(egui::Stroke::new(1.2, p.border))
                    .corner_radius(16)
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Open or Add Game by Link").size(18.0).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let close_btn = ui.add_enabled(
                                        !self.manual_game_loading,
                                        egui::Button::new(
                                            RichText::new("\u{00D7}").size(16.0).color(p.muted),
                                        )
                                        .frame(false),
                                    );
                                    if close_btn.clicked() {
                                        close_modal = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Paste a Roblox game link, private server link, share link, or Place ID.",
                            )
                            .size(12.5)
                            .color(p.muted),
                        );
                        ui.add_space(12.0);

                        egui::Frame::new()
                            .fill(p.surface_bg)
                            .stroke(egui::Stroke::new(1.2, p.text_border))
                            .corner_radius(10)
                            .inner_margin(egui::Margin::symmetric(12, 9))
                            .show(ui, |ui| {
                                ui.set_width((modal_w - 26.0).max(200.0));
                                let resp = ui.add_enabled(
                                    !self.manual_game_loading,
                                    egui::TextEdit::singleline(&mut self.manual_game_input)
                                        .frame(egui::Frame::NONE)
                                        .hint_text(
                                            "https://www.roblox.com/games/2753915549/... or Place ID",
                                        )
                                        .desired_width(ui.available_width()),
                                );
                                if self.manual_game_focus_requested {
                                    resp.request_focus();
                                    self.manual_game_focus_requested = false;
                                }
                                if resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    && !self.manual_game_loading
                                {
                                    trigger_intent = Some(ManualGameIntent::Open);
                                }
                            });

                        ui.add_space(8.0);
                        let parsed = parse_game_link(&self.manual_game_input);
                        if let Some(info) = &parsed {
                            let summary = if let Some(pid) = info.place_id {
                                if info.private_server_code.is_some() {
                                    format!("Detected Place ID {pid} • Private server link")
                                } else {
                                    format!("Detected Place ID {pid}")
                                }
                            } else {
                                "Detected Roblox share link".to_string()
                            };
                            ui.label(RichText::new(summary).size(12.0).color(GREEN));
                        } else if !self.manual_game_input.trim().is_empty() {
                            ui.label(
                                RichText::new(
                                    "Examples: roblox.com/games/<id>/..., roblox://placeId=<id>, or Place ID",
                                )
                                .size(12.0)
                                .color(p.muted),
                            );
                        }

                        if let Some(err) = &self.manual_game_err {
                            ui.add_space(6.0);
                            ui.add(egui::Label::new(RichText::new(err).size(12.5).color(RED)).wrap());
                        }

                        ui.add_space(14.0);
                        let has_private = parsed
                            .as_ref()
                            .is_some_and(|info| info.private_server_code.is_some());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.add_enabled_ui(!self.manual_game_loading, |ui| {
                                let play_label = if has_private {
                                    "Join Private Server"
                                } else {
                                    "Play"
                                };
                                let play_btn = egui::Button::image_and_text(
                                    egui::Image::new(egui::include_image!("../assets/icon_play.png"))
                                        .fit_to_exact_size(egui::vec2(14.0, 14.0))
                                        .tint(egui::Color32::WHITE),
                                    RichText::new(play_label)
                                        .size(13.0)
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(GREEN)
                                .corner_radius(10)
                                .min_size(egui::vec2(0.0, 34.0));
                                if ui.add(play_btn).clicked() {
                                    trigger_intent = Some(ManualGameIntent::Play);
                                }

                                let fav_btn = egui::Button::image_and_text(
                                    egui::Image::new(egui::include_image!(
                                        "../assets/icon_bookmark.png"
                                    ))
                                    .fit_to_exact_size(egui::vec2(13.0, 13.0))
                                    .tint(p.muted),
                                    RichText::new("Put in Favorites")
                                        .size(13.0)
                                        .strong()
                                        .color(p.text),
                                )
                                .fill(p.surface_bg)
                                .stroke(egui::Stroke::new(1.0, p.border))
                                .corner_radius(10)
                                .min_size(egui::vec2(0.0, 34.0));
                                if ui.add(fav_btn).clicked() {
                                    trigger_intent = Some(ManualGameIntent::Favorite);
                                }

                                let open_btn = egui::Button::new(
                                    RichText::new("Open Page")
                                        .size(13.0)
                                        .strong()
                                        .color(p.text),
                                )
                                .fill(p.surface_bg)
                                .stroke(egui::Stroke::new(1.0, p.border))
                                .corner_radius(10)
                                .min_size(egui::vec2(0.0, 34.0));
                                if ui.add(open_btn).clicked() {
                                    trigger_intent = Some(ManualGameIntent::Open);
                                }
                            });
                            if self.manual_game_loading {
                                ui.add_space(4.0);
                                ui.spinner();
                            }
                        });
                    });
            });

        if close_modal {
            self.manual_game_open = false;
            self.manual_game_err = None;
        } else if let Some(intent) = trigger_intent {
            self.start_manual_game_lookup(intent);
        }
    }

    pub fn request_wallpaper_load(&mut self, path: String) {
        let trimmed = path.trim().to_string();
        if trimmed.is_empty() {
            self.wallpaper_tex = None;
            self.wallpaper_loading = None;
            self.wallpaper_err = None;
            return;
        }
        self.wallpaper_loading = Some(trimmed.clone());
        self.wallpaper_err = None;
        self.spawn(move || {
            let res = (|| -> Result<egui::ColorImage, String> {
                let bytes = fs::read(&trimmed).map_err(|e| format!("Could not read image: {e}"))?;
                let mut dyn_img = image::load_from_memory(&bytes)
                    .map_err(|e| format!("Unsupported or invalid image: {e}"))?;
                if dyn_img.width() > 1920 || dyn_img.height() > 1080 {
                    dyn_img = dyn_img.resize(1920, 1080, image::imageops::FilterType::Triangle);
                }
                let rgba = dyn_img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                Ok(egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    rgba.as_raw(),
                ))
            })();
            Msg::WallpaperLoaded(trimmed, res)
        });
    }

    #[cfg(windows)]
    pub fn open_wallpaper_picker(&mut self) {
        self.spawn(|| {
            use std::os::windows::process::CommandExt;
            let script = concat!(
                "Add-Type -AssemblyName System.Windows.Forms;",
                "$d = New-Object System.Windows.Forms.OpenFileDialog;",
                "$d.Title = 'Select Background Wallpaper';",
                "$d.Filter = 'Image Files|*.png;*.jpg;*.jpeg;*.webp;*.bmp|All Files|*.*';",
                "if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { ",
                "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; ",
                "Write-Output $d.FileName }"
            );
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Sta", "-Command", script])
                .creation_flags(0x08000000)
                .output();
            let picked = out.ok().and_then(|o| {
                if !o.status.success() {
                    return None;
                }
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            });
            Msg::WallpaperPicked(picked)
        });
    }

    /// Opens the native macOS file panel and returns a POSIX path.
    #[cfg(target_os = "macos")]
    pub fn open_wallpaper_picker(&mut self) {
        self.spawn(|| {
            let script = "POSIX path of (choose file with prompt \"Select Background Wallpaper\" of type {\"public.image\"})";
            let out = std::process::Command::new("/usr/bin/osascript")
                .args(["-e", script])
                .output();
            let picked = out.ok().and_then(|o| {
                if !o.status.success() {
                    return None;
                }
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            });
            Msg::WallpaperPicked(picked)
        });
    }

    pub fn apply_runtime_resource_limits(&mut self, manual_trim: bool) {
        crate::telemetry::apply_resource_limits(
            self.settings.ram_limit_mb,
            self.settings.cpu_limit_pct,
        );
        self.enforce_ram_cache_budget(manual_trim);
    }

    pub fn enforce_ram_cache_budget(&mut self, manual_trim: bool) {
        let max_videos =
            crate::telemetry::max_video_cache_for_ram_limit(self.settings.ram_limit_mb);
        if manual_trim {
            self.active_video_texture = None;
            self.decoded_videos.clear();
            self.video_lru.clear();
            self.requested.clear();
            self.last_ram_trim_check = Some(Instant::now());
            let _ = crate::telemetry::trim_process_memory();
            if let Some(mb) = crate::telemetry::current_process_ram_mb() {
                self.status = format!("Trimmed launcher RAM ({mb:.0} MB in use)");
            } else {
                self.status = "Trimmed launcher RAM caches".to_string();
            }
            return;
        }

        while self.video_lru.len() > max_videos {
            if let Some(old_vid) = self.video_lru.pop() {
                self.decoded_videos.remove(&old_vid);
            }
        }

        {
            // Always keep CREBLOX under a soft memory cap (default 190 MB; the RAM-limit
            // setting overrides it). Heavy caches are dropped first, then loaded images.
            let cap_mb = self.settings.ram_limit_mb.unwrap_or(190);
            static OVER_CAP_LAST_CHECK: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            let now = Instant::now();
            let due = self
                .last_ram_trim_check
                .map(|t| now.saturating_duration_since(t) >= Duration::from_secs(12))
                .unwrap_or(true);
            if due {
                self.last_ram_trim_check = Some(now);
                if crate::telemetry::current_process_ram_mb()
                    .is_some_and(|used| used > cap_mb as f32)
                {
                    let keep = max_videos.min(1);
                    while self.video_lru.len() > keep {
                        if let Some(old_vid) = self.video_lru.pop() {
                            self.decoded_videos.remove(&old_vid);
                        }
                    }
                    let _ = crate::telemetry::trim_process_memory();
                    // Still over the cap on the next check: drop cached images too
                    // (visible ones simply reload).
                    if OVER_CAP_LAST_CHECK.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        self.ctx.forget_all_images();
                        let _ = crate::telemetry::trim_process_memory();
                    }
                } else {
                    OVER_CAP_LAST_CHECK.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }

    pub fn note_discord_game_launch(&mut self, place_id: u64, game: Option<Game>) {
        if place_id == 0 {
            return;
        }
        let resolved_game = game.or_else(|| self.find_known_game_by_place(place_id));
        if let Some(ref g) = resolved_game {
            self.request_thumbs(vec![g.universe_id]);
        }
        self.pending_discord_launch = Some((place_id, resolved_game, Instant::now()));
        if self.active_roblox_started_at.is_none() {
            self.active_roblox_started_at = Some(now_unix_ms() / 1000);
        }
        self.sync_discord_presence();
    }

    pub fn sync_discord_presence(&mut self) {
        self.sync_tray_state();
        let cfg = DiscordRpcConfig {
            enabled: self.settings.discord_rpc_enabled,
            show_launcher_idle: self.settings.discord_rpc_show_idle,
            app_id: self.settings.discord_rpc_app_id.clone(),
        };
        if !cfg.enabled {
            self.discord_rpc.sync(cfg, None);
            return;
        }

        let active_place = self.active_roblox_place_id.or_else(|| {
            self.pending_discord_launch
                .as_ref()
                .and_then(|(pid, _, started)| {
                    (started.elapsed() < Duration::from_secs(20)).then_some(*pid)
                })
        });

        if let Some(pid) = active_place {
            let cached = self.discord_resolved_games.get(&pid).cloned();
            let known_game = cached
                .as_ref()
                .map(|(g, _)| g.clone())
                .or_else(|| self.find_known_game_by_place(pid))
                .or_else(|| {
                    self.pending_discord_launch
                        .as_ref()
                        .and_then(|(p, g, _)| (*p == pid).then(|| g.clone()).flatten())
                });

            let icon_url = cached.and_then(|(_, u)| u).or_else(|| {
                known_game
                    .as_ref()
                    .and_then(|g| self.icon.get(&g.universe_id).cloned())
            });

            if (known_game.is_none() || icon_url.is_none())
                && self.discord_resolving_places.insert(pid)
            {
                self.spawn(move || {
                    Msg::DiscordGameResolved(pid, resolve_game_and_icon_by_place(pid))
                });
            }

            let details = known_game
                .as_ref()
                .map(|g| g.name.clone())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Playing Roblox".to_string());
            let state = match known_game.as_ref() {
                Some(g) if !g.creator.trim().is_empty() => format!("By {}", g.creator),
                _ if self.active_roblox_place_id.is_none() => "Launching via CREBLOX".to_string(),
                _ => "In-game via CREBLOX".to_string(),
            };
            let large_text = known_game
                .as_ref()
                .map(|g| g.name.clone())
                .filter(|s| !s.trim().is_empty());
            let start_epoch_secs = self
                .active_roblox_started_at
                .unwrap_or_else(|| now_unix_ms() / 1000);

            self.discord_rpc.sync(
                cfg,
                Some(DiscordActivity {
                    details,
                    state,
                    start_epoch_secs,
                    large_image: icon_url,
                    large_text,
                    small_image: None,
                    small_text: None,
                    button_label: Some("Open on Roblox".to_string()),
                    button_url: Some(format!("https://www.roblox.com/games/{pid}")),
                }),
            );
            return;
        }

        if cfg.show_launcher_idle && self.cookie.is_some() {
            let (state, large_image, large_text) = match &self.page {
                Page::Game(g) => (
                    format!("Viewing {}", g.name),
                    self.icon.get(&g.universe_id).cloned(),
                    Some(g.name.clone()),
                ),
                Page::Home => ("Home".to_string(), None, None),
                Page::Charts => ("Charts".to_string(), None, None),
                Page::Search => ("Searching games".to_string(), None, None),
                Page::Avatar => ("Customizing Avatar".to_string(), None, None),
                Page::Profile | Page::Friend(_) => ("Viewing Profile".to_string(), None, None),
                Page::AddFriend => ("Adding Friends".to_string(), None, None),
                Page::Notifications => ("Notifications".to_string(), None, None),
                Page::Settings => ("Settings".to_string(), None, None),
                Page::Login => ("Signing in".to_string(), None, None),
            };
            self.discord_rpc.sync(
                cfg,
                Some(DiscordActivity {
                    details: "Browsing CREBLOX".to_string(),
                    state,
                    start_epoch_secs: self.app_started_epoch_secs,
                    large_image,
                    large_text,
                    small_image: None,
                    small_text: None,
                    button_label: None,
                    button_url: None,
                }),
            );
        } else {
            self.discord_rpc.sync(cfg, None);
        }
    }

    pub fn tray_state_snapshot(&self) -> TrayState {
        let playing_name = self.active_roblox_place_id.map(|pid| {
            self.discord_resolved_games
                .get(&pid)
                .map(|(g, _)| g.name.clone())
                .or_else(|| self.find_known_game_by_place(pid).map(|g| g.name))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Roblox".to_string())
        });
        let user_tag = self.me.as_ref().map(|p| format!("@{}", p.user.name));
        let status_line = match (&playing_name, &user_tag) {
            (Some(game), Some(user)) => format!("Playing: {game} — {user}"),
            (Some(game), None) => format!("Playing: {game}"),
            (None, Some(user)) => format!("Signed in: {user}"),
            (None, None) => "Not signed in".to_string(),
        };
        let tooltip = match (&playing_name, &user_tag) {
            (Some(game), Some(user)) => format!("CREBLOX ({user}) — Playing {game}"),
            (Some(game), None) => format!("CREBLOX — Playing {game}"),
            (None, Some(user)) => format!("CREBLOX — {user}"),
            (None, None) => "CREBLOX".to_string(),
        };
        let server_line = match (&self.server_info, self.active_roblox_place_id) {
            (Some(info), _) => match self.server_locations.get(&info.ip) {
                Some(loc) => format!("Server: {loc}"),
                None => "Server: locating…".to_string(),
            },
            (None, Some(_)) => "Server: detecting…".to_string(),
            (None, None) => String::new(),
        };
        TrayState {
            // The menu-bar icon must be there while a game runs, even when the tray is
            // disabled otherwise: it is the only way back into CREBLOX mid-game.
            enabled: self.settings.tray_enabled || self.active_roblox_place_id.is_some(),
            window_visible: self.window_visible,
            status_line,
            tooltip,
            discord_rpc_enabled: self.settings.discord_rpc_enabled,
            signed_in: self.cookie.is_some(),
            server_line,
            job_id: self
                .server_info
                .as_ref()
                .map(|i| i.job_id.clone())
                .unwrap_or_default(),
            // The menu item is only useful while a player process exists; `roblox_running`
            // is already maintained by the process scan that drives the tray state.
            roblox_running: self.roblox_running || self.active_roblox_place_id.is_some(),
            // The menu has four fixed slots (tray_macos::ACCOUNT_SLOTS); the active account
            // is marked so the switcher reads like a radio group.
            accounts: self
                .saved_accounts
                .iter()
                .take(4)
                .map(|account| {
                    let active = self
                        .me
                        .as_ref()
                        .is_some_and(|profile| profile.user.name == account.name);
                    if active {
                        format!("✓ {}", account.name)
                    } else {
                        account.name.clone()
                    }
                })
                .collect(),
            overlay_server: self.settings.overlay_enabled,
            overlay_stats: self.settings.overlay_stats_enabled,
            overlay_crosshair: self.settings.crosshair_enabled,
            overlay_editing: self.overlay_editing,
        }
    }

    pub fn sync_tray_state(&self) {
        self.tray.sync(self.tray_state_snapshot());
    }

    pub fn hide_to_tray(&mut self, ctx: &egui::Context, show_balloon: bool) {
        // Start a fresh hide: a Dock click only reopens after the app has been inactive
        // since this hide, not because it happened to be inactive while still visible.
        self.hidden_seen_inactive = false;
        // Without a *visible* menu-bar/tray icon, hiding the window leaves a running app
        // with no obvious way back in. The tray icon object can exist but be hidden when
        // tray mode is off, so check that too and minimize instead.
        let tray_reachable = self.tray.available()
            && (self.settings.tray_enabled || self.active_roblox_place_id.is_some());
        if !tray_reachable {
            crate::auth::log("tray: no visible icon - minimizing instead of hiding");
            self.window_visible = true;
            self.hidden_at = None;
            #[cfg(target_os = "macos")]
            crate::tray::set_hidden_for_game(false);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.sync_tray_state();
            return;
        }
        self.window_visible = false;
        self.hidden_at = Some(Instant::now());
        self.hidden_seen_inactive = false;
        // The watcher thread restores a window hidden *for a game* by itself, because
        // nothing inside `update()` runs while the window is ordered out.
        #[cfg(target_os = "macos")]
        crate::tray::set_hidden_for_game(self.hidden_for_game);
        // macOS: never `Visible(false)`. Ordering the window out stops the event loop, so the
        // "the game ended" and "the Dock was clicked" messages were queued and never read
        // (the log shows `dock: reopen requested` twice with no restore after it). AppKit's
        // `hide:` hides the app while the loop keeps running, so those messages arrive.
        #[cfg(target_os = "macos")]
        crate::tray::hide_launcher_window();
        #[cfg(not(target_os = "macos"))]
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.sync_tray_state();
        if show_balloon && self.settings.tray_enabled {
            self.tray.show_balloon(
                "The launcher is still running",
                crate::platform::tray_help_text(),
            );
        }
    }

    /// After leaving a game, put CREBLOX in front (instead of Roblox's own home screen).
    /// Starts (or refreshes) the in-game overlays for the server we are connected to.
    pub fn maybe_show_game_overlay(&mut self) {
        // CREBLOX keeps running (only its window is hidden) while a game is on, so the
        // overlays are always driven from here - also with "close the window while I play".
        let flags = crate::fastflags::load_fastflags();
        let mut cfg = crate::game_overlay::OverlayConfig::from_settings(&self.settings, &flags);
        // The position editor needs the overlay on screen whether or not a game is running,
        // so it overrides both of the "nothing to show" exits below.
        cfg.edit = self.overlay_editing;
        if !cfg.any_enabled() {
            self.game_overlay.stop();
            return;
        }
        // Keep the helper while Roblox runs even if the place is not known yet: the tray
        // and settings then edit the overlays live instead of hitting a dead process.
        if !self.overlay_editing && self.active_roblox_place_id.is_none() && !self.roblox_running {
            self.game_overlay.stop();
            return;
        }
        // A join started from CREBLOX counts as an active session for a few minutes even
        // if the player log cannot be read, so overlays still appear for that launch.
        let launched_recently = now_unix_ms()
            .saturating_sub(self.last_roblox_launch_ms.load(Ordering::Relaxed))
            < 180_000;
        cfg.game_active = self.overlay_editing
            || self.active_roblox_place_id.is_some()
            || (self.roblox_running && launched_recently);
        self.game_overlay.start(cfg);
        let server = self
            .server_info
            .as_ref()
            .map(|i| (i.ip.clone(), i.job_id.clone(), i.place_id));
        self.game_overlay.set_server(server);
    }

    pub fn render_server_badge(&mut self, ctx: &egui::Context) {
        if !self.settings.show_server_badge || self.active_roblox_place_id.is_none() {
            return;
        }
        let Some(info) = self.server_info.clone() else {
            return;
        };
        let location = self
            .server_locations
            .get(&info.ip)
            .cloned()
            .unwrap_or_else(|| "Locating server...".to_string());
        let dark = self.settings.dark;
        let fill = if dark {
            egui::Color32::from_rgba_unmultiplied(18, 22, 30, 238)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 242)
        };
        let text = if dark {
            egui::Color32::from_gray(238)
        } else {
            egui::Color32::from_gray(28)
        };
        let muted = if dark {
            egui::Color32::from_gray(150)
        } else {
            egui::Color32::from_gray(105)
        };
        egui::Area::new(egui::Id::NULL.with("server_badge"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -14.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(fill)
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(egui::Stroke::new(1.0, muted.gamma_multiply(0.35)))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Connected server")
                                .size(11.0)
                                .color(muted),
                        );
                        ui.label(egui::RichText::new(location).strong().color(text));
                        ui.label(
                            egui::RichText::new(&info.ip)
                                .monospace()
                                .size(11.0)
                                .color(muted),
                        );
                        if !info.job_id.is_empty() && ui.small_button("Copy Job ID").clicked() {
                            ui.ctx().copy_text(info.job_id.clone());
                        }
                    });
            });
    }

    /// Adds the finished session to the playtime table. Cheap: one map update and one small
    /// file write per game, never while playing.
    pub fn finish_playtime_session(&mut self) {
        let Some((place, started)) = self.playtime_session.take() else {
            return;
        };
        let secs = started.elapsed().as_secs();
        // Sessions under ten seconds are launches that never got into a game.
        if secs < 10 {
            return;
        }
        let entry = self.playtime.entry(place).or_default();
        entry.seconds = entry.seconds.saturating_add(secs);
        entry.last_played = crate::playtime::now_unix();
        crate::playtime::save(&self.playtime);
    }

    pub fn return_to_launcher(&mut self) {
        // CREBLOX hid itself when the game was launched, so it always comes back - even when
        // "return to the launcher" is off: the window should never go away for good.
        if self.hidden_for_game {
            self.hidden_for_game = false;
            self.restore_from_tray();
            return;
        }
        if !self.settings.return_to_launcher_on_leave {
            return;
        }
        self.restore_from_tray();
    }

    pub fn restore_from_tray(&mut self) {
        crate::auth::log("window: restoring the launcher");
        self.window_visible = true;
        self.hidden_seen_inactive = false;
        #[cfg(target_os = "macos")]
        crate::tray::set_hidden_for_game(false);
        self.pending_start_in_tray_frames = 0;
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // Raise it immediately as well, through AppKit itself. `Visible(true)` can be
        // processed a frame later (or dropped while the app is in the background), and
        // `makeKeyAndOrderFront:` also brings the window in front of a fullscreen Roblox
        // Space. The repeated frames below are kept as a second chance.
        #[cfg(target_os = "macos")]
        crate::tray::show_launcher_window();
        // Raising the window has to happen *after* eframe applied the visibility change,
        // otherwise the window can stay ordered out / behind Roblox. Several frames of
        // re-asserting visibility are cheap and survive a dropped window-server message.
        self.raise_frames = 8;
        self.ctx.request_repaint();
        self.sync_tray_state();
    }

    pub fn quit_app(&mut self) {
        self.allow_close_exit = true;
        self.tray.shutdown();
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        self.ctx.request_repaint();
    }

    pub fn paint_app_background(&self, ctx: &egui::Context) {
        if self.settings.bg_mode == BgMode::ThemeDefault {
            return;
        }
        let screen_rect = ctx.input(|i| i.content_rect());
        if !screen_rect.is_positive() {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::background());
        match self.settings.bg_mode {
            BgMode::ThemeDefault => {}
            BgMode::SolidColor => {
                let [r, g, b] = self.settings.bg_color;
                painter.rect_filled(screen_rect, 0.0, egui::Color32::from_rgb(r, g, b));
            }
            BgMode::Wallpaper => {
                let base = Palette::of(self.settings.dark).bg;
                painter.rect_filled(screen_rect, 0.0, base);
                if let Some((_, tex)) = &self.wallpaper_tex {
                    let img_sz = tex.size_vec2();
                    let uv = if img_sz.x > 0.0 && img_sz.y > 0.0 {
                        let win_aspect = screen_rect.width() / screen_rect.height().max(1.0);
                        let img_aspect = img_sz.x / img_sz.y;
                        if img_aspect > win_aspect {
                            let u_span = (win_aspect / img_aspect).clamp(0.01, 1.0);
                            let u_pad = (1.0 - u_span) * 0.5;
                            egui::Rect::from_min_max(
                                egui::pos2(u_pad, 0.0),
                                egui::pos2(1.0 - u_pad, 1.0),
                            )
                        } else {
                            let v_span = (img_aspect / win_aspect).clamp(0.01, 1.0);
                            let v_pad = (1.0 - v_span) * 0.5;
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, v_pad),
                                egui::pos2(1.0, 1.0 - v_pad),
                            )
                        }
                    } else {
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
                    };
                    painter.image(tex.id(), screen_rect, uv, egui::Color32::WHITE);

                    let alpha = (self.settings.bg_overlay.clamp(0.0, 0.85) * 255.0).round() as u8;
                    if alpha > 0 {
                        let scrim = if self.settings.dark {
                            egui::Color32::from_rgba_unmultiplied(12, 14, 19, alpha)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(248, 250, 252, alpha)
                        };
                        painter.rect_filled(screen_rect, 0.0, scrim);
                    }
                }
            }
        }
    }
}

impl eframe::App for Creblox {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // macOS users expect Cmd+Q to really quit, even when the window is set
        // to hide into the menu bar on close.
        #[cfg(target_os = "macos")]
        {
            let quit_shortcut = ui.ctx().input(|i| {
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Q)
            });
            if quit_shortcut {
                self.quit_app();
                return;
            }
        }
        {
            // Publish "is the window on screen" for the idle animation pacer, but only
            // when it actually changed: this runs on every frame and `insert_temp`
            // allocates each time.
            let visible = self.window_visible;
            let id = egui::Id::NULL.with(crate::telemetry::WINDOW_VISIBLE_KEY);
            let unchanged = ui.ctx().data(|d| d.get_temp::<bool>(id)) == Some(visible);
            if !unchanged {
                ui.ctx().data_mut(|d| d.insert_temp(id, visible));
            }
        }
        // The tray thread forwards menu clicks too; draining the same channel here as
        // well means a click can never be lost because that thread was slow.
        for ev in self.tray.drain_events() {
            let _ = self.tx.send(Msg::TrayAction(ev));
        }
        if self.poll() {
            ui.ctx().request_repaint();
        }
        // CREBLOX hid itself for a game. If the game is gone but the watcher's last message was
        // missed, the window would never come back - so check for ourselves now and then.
        if self.hidden_for_game {
            let due = self
                .last_hidden_check
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(2));
            if due {
                self.last_hidden_check = Some(Instant::now());
                let hidden_long_enough = self
                    .hidden_at
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(5));
                if hidden_long_enough {
                    let (procs, _) = crate::roblox_process::enumerate_roblox_processes();
                    if procs.is_empty() {
                        self.finish_playtime_session();
                        self.return_to_launcher();
                    }
                }
            }
        }
        // Remember every inactive moment while CREBLOX is hidden. This used to be sampled
        // only once every three seconds, so a quick click on the Dock/Finder icon could be
        // missed and the window then stayed hidden - which looked like CREBLOX refusing to
        // open. A minimized window is left alone; only a truly hidden window is reopened
        // this way.
        #[cfg(target_os = "macos")]
        if !crate::tray::app_is_active() {
            self.hidden_seen_inactive = true;
        }
        if !self.window_visible {
            let due = self
                .last_window_check
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
            if due {
                self.last_window_check = Some(Instant::now());
                #[cfg(target_os = "macos")]
                {
                    if crate::tray::app_is_active() && self.hidden_seen_inactive {
                        // Active again after being inactive: a Dock/Finder click.
                        self.restore_from_tray();
                    }
                }
            }
        }
        // macOS can drop a Visible(true) while CREBLOX is in the background. The old code
        // then kept `window_visible = true` and never tried again, leaving a running process
        // whose window stayed ordered out: the launcher looked closed, and opening it again
        // only asked that invisible instance to show itself. Check AppKit's real window state
        // and re-apply the restore instead of trusting our own flag.
        #[cfg(target_os = "macos")]
        if self.window_visible
            && self.raise_frames == 0
            && !self.allow_close_exit
            && !ui.ctx().input(|i| i.viewport().close_requested())
        {
            let due = self
                .last_visibility_check
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
            if due {
                self.last_visibility_check = Some(Instant::now());
                if !crate::tray::any_window_visible() {
                    crate::auth::log(
                        "window: AppKit reports no visible window; restoring the launcher",
                    );
                    self.restore_from_tray();
                }
            }
        }
        // If the overlay process died during a game, bring it back: the game is still running
        // and the overlays should be on screen.
        if self.active_roblox_place_id.is_some() || self.roblox_running {
            let due = self
                .last_overlay_check
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5));
            if due {
                self.last_overlay_check = Some(Instant::now());
                if !self.game_overlay.is_running() {
                    self.maybe_show_game_overlay();
                }
            }
        }
        // Position editor: pick up the overlay's answer (Confirm / Esc) and persist it.
        if self.overlay_editing {
            let due = self
                .last_overlay_edit_check
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(250));
            if due {
                self.last_overlay_edit_check = Some(Instant::now());
                if let Some(result) = crate::game_overlay::take_position_result() {
                    self.overlay_editing = false;
                    if result.confirmed {
                        self.settings.overlay_offset_x = result.offset_x;
                        self.settings.overlay_offset_y = result.offset_y;
                        save_settings(&self.settings);
                        self.status = format!(
                            "HUD position saved ({}, {})",
                            result.offset_x, result.offset_y
                        );
                    } else {
                        self.status = "HUD position unchanged".to_string();
                    }
                    self.maybe_show_game_overlay();
                    self.sync_tray_state();
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        if self.raise_frames > 0 {
            self.raise_frames -= 1;
            // Re-assert visibility every frame: one Visible(true) can be dropped while the
            // app is hidden or minimized, which left a running launcher with no window.
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            if self.raise_frames == 0 {
                // Make CREBLOX the active app so the window comes forward even when Roblox
                // owns the screen in a fullscreen Space.
                #[cfg(target_os = "macos")]
                crate::tray::activate_app();
            }
            ui.ctx().request_repaint();
        }
        if self.pending_start_in_tray_frames > 0 {
            self.pending_start_in_tray_frames -= 1;
            if self.pending_start_in_tray_frames == 0 {
                self.hide_to_tray(ui.ctx(), false);
            } else {
                ui.ctx().request_repaint();
            }
        }
        if ui.ctx().input(|i| i.viewport().close_requested())
            && !self.allow_close_exit
            && self.settings.tray_enabled
            && self.settings.tray_minimize_on_close
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let show_tip = !self.tray_minimize_balloon_shown;
            self.tray_minimize_balloon_shown = true;
            self.hide_to_tray(ui.ctx(), show_tip);
        }
        if !self.update_checked {
            self.update_checked = true;
            self.check_updates(false);
        }
        if !self.app_update_checked {
            self.app_update_checked = true;
            if self.settings.auto_check_updates {
                self.check_app_updates(false);
            }
        }
        self.maybe_auto_sync(ui.ctx());
        if matches!(self.page, Page::Settings) {
            let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            });
            if let Some(img_path) = dropped.into_iter().find(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "bmp"
                        )
                    })
                    .unwrap_or(false)
            }) {
                let s = img_path.display().to_string();
                self.settings.bg_mode = BgMode::Wallpaper;
                self.settings.bg_wallpaper = s.clone();
                save_settings(&self.settings);
                apply_theme(ui.ctx(), &self.settings);
                self.request_wallpaper_load(s);
            }
        }
        self.paint_app_background(ui.ctx());
        let logged_in = self.cookie.is_some();
        let p = pal(ui);
        let chrome_fill = panel_chrome_fill(ui, &self.settings);
        let central_fill = if self.settings.bg_mode == BgMode::ThemeDefault {
            p.bg
        } else {
            egui::Color32::TRANSPARENT
        };
        let mut acts: Vec<Action> = Vec::new();
        let mut do_search = false;
        let mut toggle_theme = false;
        let mut refresh_auth = false;

        // Left sidebar (only shown when logged in)
        if logged_in {
            let unread_notifs = self.notification_history.iter().filter(|n| !n.read).count();
            egui::Panel::left("nav")
                .resizable(false)
                .exact_size(78.0)
                .show_separator_line(self.settings.glass_edition)
                .frame(egui::Frame::new().fill(chrome_fill).inner_margin(4))
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    let page = &self.page;
                    if nav_item(ui, NavIcon::Home, "Home", matches!(page, Page::Home)) {
                        acts.push(Action::Go(Page::Home));
                    }
                    if nav_item(ui, NavIcon::Charts, "Charts", matches!(page, Page::Charts)) {
                        acts.push(Action::Go(Page::Charts));
                    }
                    if nav_item(
                        ui,
                        NavIcon::Profile,
                        "Profile",
                        matches!(page, Page::Profile),
                    ) {
                        acts.push(Action::Go(Page::Profile));
                    }
                    if nav_item(ui, NavIcon::Avatar, "Avatar", matches!(page, Page::Avatar)) {
                        acts.push(Action::Go(Page::Avatar));
                    }
                    if nav_item_badged(
                        ui,
                        NavIcon::Notifications,
                        "Notifications",
                        matches!(page, Page::Notifications),
                        unread_notifs,
                        self.settings.notifications_silent,
                    ) {
                        acts.push(Action::Go(Page::Notifications));
                    }
                    if nav_item(
                        ui,
                        NavIcon::Settings,
                        "Settings",
                        matches!(page, Page::Settings),
                    ) {
                        acts.push(Action::Go(Page::Settings));
                    }
                });
        }

        // Top bar
        egui::Panel::top("top")
            .resizable(false)
            .show_separator_line(self.settings.glass_edition)
            .frame(
                egui::Frame::new()
                    .fill(chrome_fill)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // The OS still shows the app name (window title, Dock, menu bar), but the
                    // launcher window itself stays unbranded.
                    if logged_in {
                        ui.add_space(20.0);
                        let url_btn_reserve = 106.0;
                        // Nothing is drawn to the left of the search box any more, so let
                        // the box give way first instead of pushing the trailing account
                        // controls past the right edge at the minimum window width.
                        let free_w =
                            (ui.available_width() - 200.0 - url_btn_reserve).max(170.0);
                        let search_w = (free_w - 20.0).clamp(150.0, 400.0);
                        let search_pad = ((free_w - search_w) * 0.5).max(8.0);
                        ui.add_space(search_pad);

                        egui::Frame::new()
                            .fill(p.surface_bg)
                            .stroke(egui::Stroke::new(1.2, p.text_border))
                            .corner_radius(18)
                            .inner_margin(egui::Margin::symmetric(14, 6))
                            .show(ui, |ui| {
                                ui.set_width(search_w);
                                ui.horizontal_centered(|ui| {
                                    ui.spacing_mut().item_spacing.x = 9.0;
                                    ui.add(
                                        egui::Image::new(egui::include_image!(
                                            "../assets/icon_search.png"
                                        ))
                                        .fit_to_exact_size(egui::vec2(14.0, 14.0))
                                        .tint(p.muted),
                                    );
                                    let edit_w = (search_w - 42.0).max(80.0);
                                    // Fixed height + centred row: the placeholder used to sit
                                    // a pixel or two low next to the magnifier.
                                    let r = ui.add_sized(
                                        egui::vec2(edit_w, 20.0),
                                        egui::TextEdit::singleline(&mut self.search)
                                            .frame(egui::Frame::NONE)
                                            .hint_text("Search games, Place ID, or link"),
                                    );
                                    if r.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        do_search = true;
                                    }
                                    if !self.search.is_empty()
                                        && ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new("\u{00D7}")
                                                        .size(12.0)
                                                        .color(p.muted),
                                                )
                                                .sense(Sense::click()),
                                            )
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                    {
                                        self.search.clear();
                                    }
                                });
                            });
                        ui.add_space(8.0);
                        let url_btn = egui::Button::new(
                            RichText::new("+ Game URL")
                                .size(12.5)
                                .strong()
                                .color(p.text),
                        )
                        .fill(p.surface_bg)
                        .stroke(egui::Stroke::new(1.2, p.text_border))
                        .corner_radius(18)
                        .min_size(egui::vec2(0.0, 30.0));
                        if ui
                            .add(url_btn)
                            .on_hover_text(
                                "Manually open, play, or add a Roblox game by URL link or Place ID",
                            )
                            .clicked()
                        {
                            acts.push(Action::OpenManualGame);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if logged_in {
                            if let Some(prof) = &self.me {
                                let p = pal(ui);
                                let (switch_rect, switch_btn) =
                                    ui.allocate_exact_size(egui::vec2(22.0, 24.0), Sense::click());
                                if switch_btn.hovered() {
                                    ui.painter().rect_filled(switch_rect, 6.0, p.hover_bg);
                                }
                                let c = switch_rect.center();
                                let chev_col = if switch_btn.hovered() {
                                    p.text
                                } else {
                                    p.muted
                                };
                                ui.painter().line_segment(
                                    [egui::pos2(c.x - 4.0, c.y - 1.5), egui::pos2(c.x, c.y + 2.5)],
                                    egui::Stroke::new(1.5, chev_col),
                                );
                                ui.painter().line_segment(
                                    [egui::pos2(c.x, c.y + 2.5), egui::pos2(c.x + 4.0, c.y - 1.5)],
                                    egui::Stroke::new(1.5, chev_col),
                                );
                                let switch_btn = switch_btn.on_hover_text("Accounts & menu");

                                // Avatar, then the name, then the menu chevron: one neat
                                // row with even spacing instead of a name that drifted
                                // left of its picture.
                                let resp = ui
                                    .horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        let h = thumb_styled(
                                            ui,
                                            Some(&prof.user.headshot),
                                            egui::vec2(26.0, 26.0),
                                            13,
                                            true,
                                        );
                                        let n = ui
                                            .horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 5.0;
                                                let l = ui.add(
                                                    egui::Label::new(
                                                        RichText::new(&prof.user.name)
                                                            .size(13.5)
                                                            .strong(),
                                                    )
                                                    .truncate()
                                                    .sense(Sense::click()),
                                                );
                                                if prof.user.verified {
                                                    verified_badge_tip(
                                                        ui,
                                                        13.0,
                                                        "Verified Account",
                                                    );
                                                }
                                                l
                                            })
                                            .inner;
                                        (n.clicked() || h.clicked(), n, h)
                                    })
                                    .inner;
                                if resp.0 {
                                    acts.push(Action::Go(Page::Profile));
                                }
                                let active_uid = prof.user.id;
                                let saved_list = &self.saved_accounts;
                                let dark_mode = self.settings.dark;
                                let menu_item = |ui: &mut egui::Ui,
                                                 label: &str,
                                                 col: Option<egui::Color32>|
                                 -> bool {
                                    let w = ui.available_width();
                                    let (rect, r) =
                                        ui.allocate_exact_size(egui::vec2(w, 26.0), Sense::click());
                                    if r.hovered() {
                                        ui.painter().rect_filled(rect, 5.0, p.hover_bg);
                                    }
                                    ui.painter().text(
                                        egui::pos2(rect.left() + 8.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        label,
                                        egui::FontId::proportional(12.5),
                                        col.unwrap_or(p.text),
                                    );
                                    r.clicked()
                                };
                                let mut menu = |ui: &mut egui::Ui| {
                                    ui.set_width(210.0);
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    for acc in saved_list.iter() {
                                        let is_cur = acc.id == active_uid;
                                        let (rect, r) = ui.allocate_exact_size(
                                            egui::vec2(ui.available_width(), 38.0),
                                            Sense::click(),
                                        );
                                        if r.hovered() && !is_cur {
                                            ui.painter().rect_filled(rect, 6.0, p.hover_bg);
                                        } else if is_cur {
                                            ui.painter().rect_filled(
                                                rect,
                                                6.0,
                                                p.selected_bg.gamma_multiply(0.55),
                                            );
                                        }
                                        let mut row_ui = ui.new_child(
                                            egui::UiBuilder::new()
                                                .max_rect(rect.shrink2(egui::vec2(6.0, 4.0))),
                                        );
                                        row_ui.horizontal(|ui| {
                                            let _ = thumb_styled(
                                                ui,
                                                Some(&acc.headshot),
                                                egui::vec2(24.0, 24.0),
                                                12,
                                                true,
                                            );
                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = -1.0;
                                                ui.label(
                                                    RichText::new(&acc.display).size(12.5).strong(),
                                                );
                                                ui.label(
                                                    RichText::new(format!("@{}", acc.name))
                                                        .size(10.5)
                                                        .color(p.muted),
                                                );
                                            });
                                        });
                                        if is_cur {
                                            ui.painter().circle_filled(
                                                egui::pos2(rect.right() - 12.0, rect.center().y),
                                                3.5,
                                                GREEN,
                                            );
                                        } else if r.clicked() {
                                            acts.push(Action::SwitchAccount(acc.id));
                                            ui.close();
                                        }
                                    }
                                    if menu_item(ui, "+ Add account", Some(p.muted)) {
                                        acts.push(Action::AddAccount);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if menu_item(ui, "Profile", None) {
                                        acts.push(Action::Go(Page::Profile));
                                        ui.close();
                                    }
                                    if menu_item(ui, "Avatar", None) {
                                        acts.push(Action::Go(Page::Avatar));
                                        ui.close();
                                    }
                                    if menu_item(ui, "Settings", None) {
                                        acts.push(Action::Go(Page::Settings));
                                        ui.close();
                                    }
                                    if menu_item(
                                        ui,
                                        if dark_mode { "Light mode" } else { "Dark mode" },
                                        None,
                                    ) {
                                        toggle_theme = true;
                                        ui.close();
                                    }
                                    ui.separator();
                                    if menu_item(ui, "Sign out", Some(RED)) {
                                        acts.push(Action::SignOut);
                                        ui.close();
                                    }
                                };
                                egui::Popup::from_toggle_button_response(&switch_btn)
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                    .show(&mut menu);
                                resp.1.context_menu(&mut menu);
                                resp.2.context_menu(&mut menu);

                                ui.add_space(6.0);
                                let rb_text = match prof.robux {
                                    Some(v) => crate::api::format_commas(v),
                                    None => "--".to_string(),
                                };
                                let rb_gold = if self.settings.dark {
                                    egui::Color32::from_rgb(245, 194, 66)
                                } else {
                                    egui::Color32::from_rgb(184, 126, 8)
                                };
                                let rb_frame = egui::Frame::new()
                                    .fill(p.surface_bg)
                                    .stroke(egui::Stroke::new(1.0, p.border))
                                    .corner_radius(15)
                                    .inner_margin(egui::Margin::symmetric(10, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            let (icon_rect, _) = ui.allocate_exact_size(
                                                egui::vec2(15.0, 15.0),
                                                Sense::hover(),
                                            );
                                            draw_robux_icon(
                                                ui.painter(),
                                                icon_rect.center(),
                                                15.0,
                                                rb_gold,
                                            );
                                            ui.label(
                                                RichText::new(&rb_text)
                                                    .size(12.5)
                                                    .strong()
                                                    .color(p.text),
                                            );
                                        });
                                    });
                                let rb_tip = prof
                                    .robux
                                    .map(|v| {
                                        format!(
                                            "{} Robux available — click to open Transactions",
                                            crate::api::format_commas(v)
                                        )
                                    })
                                    .unwrap_or_else(|| {
                                        "Robux balance unavailable — right-click to refresh"
                                            .to_string()
                                    });
                                let rb_resp = ui
                                    .interact(
                                        rb_frame.response.rect,
                                        ui.scope_id().with("topbar_robux_pill"),
                                        Sense::click(),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(rb_tip);
                                if rb_resp.clicked() {
                                    acts.push(Action::OpenUrl(
                                        "https://www.roblox.com/transactions".to_string(),
                                    ));
                                }
                                rb_resp.context_menu(|ui| {
                                    if ui.button("Open Transactions").clicked() {
                                        acts.push(Action::OpenUrl(
                                            "https://www.roblox.com/transactions".to_string(),
                                        ));
                                        ui.close();
                                    }
                                    if ui.button("Refresh Balance").clicked() {
                                        refresh_auth = true;
                                        ui.close();
                                    }
                                });
                            }
                        } else {
                            let theme_icon = if self.settings.dark { "Light" } else { "Dark" };
                            let tbtn = egui::Button::new(
                                RichText::new(theme_icon).size(12.0).color(p.muted),
                            )
                            .fill(p.surface_bg)
                            .corner_radius(16);
                            if ui.add(tbtn).on_hover_text("Toggle theme").clicked() {
                                toggle_theme = true;
                            }
                        }
                    });
                });
            });

        // Status bar (only shown when there is an active message)
        if !self.status.is_empty() {
            egui::Panel::bottom("status")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(chrome_fill)
                        .inner_margin(egui::Margin::symmetric(20, 5)),
                )
                .show(ui, |ui| {
                    small_text(ui, &self.status);
                });
        }

        // Page content (central panel always last)
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(central_fill)
                    .inner_margin(egui::Margin::symmetric(22, 6)),
            )
            .show(ui, |ui| {
                let mut page_scroll = egui::ScrollArea::vertical()
                    .id_salt("page_scroll")
                    .auto_shrink([false, false]);
                // Clicking the section you are already on takes you back to the top.
                if self.scroll_to_top {
                    page_scroll = page_scroll.vertical_scroll_offset(0.0);
                    self.scroll_to_top = false;
                }
                page_scroll.show(ui, |ui| {
                    if !logged_in {
                        self.page_login(ui, &mut acts);
                    } else {
                        let page_key: (u8, u64) = match &self.page {
                            Page::Home => (0, 0),
                            Page::Charts => (1, self.chart as u64),
                            Page::Search => (2, 0),
                            Page::Game(g) => (3, g.universe_id),
                            Page::Friend(u) => (4, u.id),
                            Page::AddFriend => (5, 0),
                            Page::Profile => (6, 0),
                            Page::Avatar => (7, 0),
                            Page::Notifications => (8, 0),
                            Page::Settings => (9, 0),
                            Page::Login => (10, 0),
                        };
                        let anim_state_id = egui::Id::NULL.with("page_transition_state");
                        let now = ui.input(|i| i.time);
                        let entered_at = ui.ctx().data_mut(|d| {
                            let (prev_k, prev_t) = d
                                .get_temp::<((u8, u64), f64)>(anim_state_id)
                                .unwrap_or((page_key, now));
                            if prev_k != page_key {
                                d.insert_temp(anim_state_id, (page_key, now));
                                now
                            } else {
                                d.insert_temp(anim_state_id, (prev_k, prev_t));
                                prev_t
                            }
                        });
                        // The cross-fade is an animation, so it has to obey the same
                        // setting as every other animation in the app.
                        let raw_t = if self.settings.animations_enabled {
                            ((now - entered_at) as f32 / 0.16).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let ease = 1.0 - (1.0 - raw_t).powi(3);
                        if raw_t < 1.0 {
                            ui.ctx().request_repaint();
                        }
                        if !matches!(self.page, Page::Login) && raw_t < 1.0 {
                            ui.add_space((1.0 - ease) * 6.0);
                            ui.set_opacity(0.45 + 0.55 * ease);
                        }
                        // Take the page out instead of cloning it: the game page carries a
                        // full description, and cloning it every frame was pure churn.
                        let page = std::mem::replace(&mut self.page, Page::Home);
                        match &page {
                            Page::Home => self.page_home(ui, &mut acts),
                            Page::Charts => self.page_charts(ui, &mut acts),
                            Page::Search => self.page_search(ui, &mut acts),
                            Page::Game(g) => self.page_game(ui, g, &mut acts),
                            Page::Friend(u) => self.page_friend(ui, u, &mut acts),
                            Page::AddFriend => self.page_add_friend(ui, &mut acts),
                            Page::Profile => self.page_my_profile(ui, &mut acts),
                            Page::Avatar => self.page_avatar(ui, &mut acts),
                            Page::Notifications => self.page_notifications(ui, &mut acts),
                            Page::Settings => {
                                // Settings changed while a game is running must reach the
                                // live overlay immediately, not only on the next join.
                                // Compare against a snapshot instead of cloning the whole
                                // settings struct (with all its strings) every frame.
                                self.page_settings(ui, &mut acts);
                                if self.settings != self.settings_snapshot {
                                    self.settings_snapshot = self.settings.clone();
                                    self.maybe_show_game_overlay();
                                    self.sync_tray_state();
                                }
                            }
                            Page::Login => self.page_login(ui, &mut acts),
                        }
                        self.page = page;
                    }
                });
            });

        if toggle_theme {
            self.settings.dark = !self.settings.dark;
            save_settings(&self.settings);
            apply_theme(ui.ctx(), &self.settings);
        }
        if refresh_auth {
            self.sync_auth_session(true);
        }
        // The launcher no longer draws the FPS/RAM/CPU dock (those stats live in the
        // in-game overlay), so there is nothing to poll here any more: the RAM budget is
        // simply kept in check.
        self.enforce_ram_cache_budget(false);

        if logged_in {
            self.render_manual_game_modal(ui.ctx());
        }
        self.render_notifications(ui.ctx(), &mut acts);
        self.render_server_badge(ui.ctx());

        let hovered_game = acts.iter().find_map(|a| {
            if let Action::HoverGame(uid) = a {
                Some(*uid)
            } else {
                None
            }
        });
        self.handle_game_hover(hovered_game, ui.ctx());

        // Apply clicks after drawing
        let prev_page_disc = std::mem::discriminant(&self.page);
        let prev_page_game_id = match &self.page {
            Page::Game(g) => Some(g.universe_id),
            _ => None,
        };
        for a in acts {
            match a {
                Action::HoverGame(_) => {}
                Action::StartQuickLogin => self.begin_quick_login(),
                Action::AddAccount => self.begin_quick_login(),
                Action::SwitchAccount(uid) => self.switch_account(uid),
                Action::RemoveAccount(uid) => self.remove_saved_account(uid),
                Action::SignOut => self.sign_out(),
                Action::KillRobloxProcesses => {
                    let killed = kill_all_roblox_processes();
                    if killed > 0 {
                        self.status = format!("Closed {killed} Roblox process(es).");
                    } else {
                        self.status = "No background Roblox processes found.".to_string();
                    }
                }
                Action::OpenUrl(u) => {
                    if let Err(e) = open::that(&u) {
                        self.status = format!("Error: {e}");
                    }
                }
                _ if !self.cookie.is_some() => {}
                Action::Go(p) => {
                    if !matches!(p, Page::Login) {
                        self.cancel_quick_login();
                    }
                    if matches!(p, Page::Avatar) {
                        self.ensure_avatar_loaded();
                    }
                    if matches!(p, Page::Profile) {
                        self.ensure_my_profile_loaded();
                    }
                    let reclicking_home =
                        matches!(p, Page::Home) && matches!(self.page, Page::Home);
                    if !reclicking_home && matches!(p, Page::Home | Page::Charts) {
                        self.home_sync_spinner = false;
                        if Self::is_stale(
                            &self.last_sorts_sync,
                            &self.sorts_sync_started,
                            Duration::from_secs(12),
                        ) {
                            self.sync_sorts(false);
                        }
                        if Self::is_stale(
                            &self.last_saved_sync,
                            &self.saved_sync_started,
                            Duration::from_secs(12),
                        ) {
                            self.sync_saved(false);
                        }
                        if Self::is_stale(
                            &self.last_auth_sync,
                            &self.auth_sync_started,
                            Duration::from_secs(12),
                        ) {
                            self.sync_auth_session(false);
                        }
                    }
                    self.page = p;
                    // Section click always lands at the first position, whether it switches
                    // page or re-opens the one you are already on.
                    self.scroll_to_top = true;
                    if reclicking_home {
                        self.status = "Refreshing home...".to_string();
                        self.force_refresh_games();
                    }
                }
                Action::RefreshGames => self.force_refresh_games(),
                Action::RefreshMyProfile => {
                    self.my_profile = None;
                    self.my_profile_err = None;
                    // "Refresh profile" must bypass the 60 s cache and hit the network.
                    if let Some(me) = &self.me {
                        self.profile_cache.remove(&me.user.id);
                    }
                    self.ensure_my_profile_loaded();
                }
                Action::SaveBio(bio) => {
                    if let Some(cookie) = self.cookie.clone() {
                        self.profile_saving_bio = true;
                        self.profile_save_err = None;
                        let bio_clone = bio.clone();
                        self.spawn(move || {
                            let res = update_user_description(&cookie, &bio_clone);
                            Msg::BioSaved(res)
                        });
                    }
                }
                Action::SelectAvatarTab(tab) => {
                    self.avatar_tab = tab;
                    self.ensure_avatar_tab_loaded(tab);
                }
                Action::RefreshAvatar => {
                    self.load_avatar();
                    self.avatar_3d.loaded_user_id = None;
                    self.load_avatar_3d();
                    if let Some((uid, cookie)) =
                        self.me.as_ref().map(|p| p.user.id).zip(self.cookie.clone())
                    {
                        self.avatar_seq += 1;
                        self.avatar_pending += 1;
                        let seq = self.avatar_seq;
                        self.spawn(move || {
                            Msg::AvatarApplied(
                                seq,
                                finish_avatar_update(
                                    uid,
                                    &cookie,
                                    "Refreshed avatar preview.".to_string(),
                                ),
                            )
                        });
                    }
                }
                Action::RefreshAvatarTab(tab) => match tab {
                    AvatarTab::Wearing | AvatarTab::Body => self.load_avatar(),
                    AvatarTab::Outfits => {
                        self.avatar_outfits = None;
                        self.avatar_outfits_page = 1;
                        self.avatar_outfits_more = false;
                        self.load_avatar_tab(tab, false);
                    }
                    t => {
                        self.avatar_items.remove(&t);
                        self.avatar_cursors.remove(&t);
                        self.load_avatar_tab(t, false);
                    }
                },
                Action::LoadMoreAvatarTab(tab) => {
                    self.load_avatar_tab(tab, true);
                }
                Action::ToggleWearAsset(item) => {
                    if let (Some(uid), Some(cookie)) =
                        (self.me.as_ref().map(|p| p.user.id), self.cookie.clone())
                    {
                        if let Some(st) = &mut self.avatar_state {
                            let (next, equipped) = compute_toggled_assets(&st.wearing, &item);
                            st.wearing = next.clone();
                            let note = if equipped {
                                format!("Equipped \"{}\".", item.name)
                            } else {
                                format!("Removed \"{}\".", item.name)
                            };
                            self.avatar_err = None;
                            self.avatar_seq += 1;
                            self.avatar_pending += 1;
                            let seq = self.avatar_seq;
                            self.status = if equipped {
                                format!("Equipped {} — syncing render...", item.name)
                            } else {
                                format!("Removed {} — syncing render...", item.name)
                            };
                            self.spawn(move || {
                                Msg::AvatarApplied(
                                    seq,
                                    apply_avatar_assets(uid, &cookie, &next, note),
                                )
                            });
                        }
                    }
                }
                Action::UnequipAccessories => {
                    if let (Some(uid), Some(cookie)) =
                        (self.me.as_ref().map(|p| p.user.id), self.cookie.clone())
                    {
                        if let Some(st) = &mut self.avatar_state {
                            let next: Vec<AvatarAsset> = st
                                .wearing
                                .iter()
                                .filter(|a| !matches!(a.asset_type_id, 8 | 41..=47))
                                .cloned()
                                .collect();
                            st.wearing = next.clone();
                            self.avatar_err = None;
                            self.avatar_seq += 1;
                            self.avatar_pending += 1;
                            let seq = self.avatar_seq;
                            self.status =
                                "Removed hats & accessories — syncing render...".to_string();
                            self.spawn(move || {
                                Msg::AvatarApplied(
                                    seq,
                                    apply_avatar_assets(
                                        uid,
                                        &cookie,
                                        &next,
                                        "Removed all hats & accessories.".to_string(),
                                    ),
                                )
                            });
                        }
                    }
                }
                Action::WearOutfit(outfit) => {
                    if let (Some(uid), Some(cookie)) =
                        (self.me.as_ref().map(|p| p.user.id), self.cookie.clone())
                    {
                        self.avatar_err = None;
                        self.avatar_seq += 1;
                        self.avatar_pending += 1;
                        let seq = self.avatar_seq;
                        self.status = format!("Applying outfit \"{}\"...", outfit.name);
                        self.spawn(move || {
                            Msg::AvatarApplied(seq, apply_wear_outfit(uid, &cookie, &outfit))
                        });
                    }
                }
                Action::SaveOutfit(name) => {
                    if let Some(cookie) = self.cookie.clone() {
                        if !self.avatar_outfit_saving {
                            self.avatar_err = None;
                            self.avatar_outfit_saving = true;
                            self.status = format!("Saving outfit \"{}\"...", name.trim());
                            let state = self.avatar_state.clone();
                            let palette = self.avatar_palette.clone();
                            self.spawn(move || {
                                Msg::OutfitActionDone(create_avatar_outfit(
                                    &cookie, &name, state, &palette,
                                ))
                            });
                        }
                    }
                }
                Action::DeleteOutfit(outfit) => {
                    if let Some(cookie) = self.cookie.clone() {
                        if !self.avatar_outfit_saving {
                            self.avatar_err = None;
                            self.avatar_outfit_saving = true;
                            self.status = format!("Deleting outfit \"{}\"...", outfit.name);
                            self.spawn(move || {
                                Msg::OutfitActionDone(delete_avatar_outfit(&cookie, &outfit))
                            });
                        }
                    }
                }
                Action::SetAvatarType(rig) => {
                    if let (Some(uid), Some(cookie)) =
                        (self.me.as_ref().map(|p| p.user.id), self.cookie.clone())
                    {
                        if let Some(st) = &mut self.avatar_state {
                            st.avatar_type = rig.clone();
                        }
                        self.avatar_err = None;
                        self.avatar_seq += 1;
                        self.avatar_pending += 1;
                        let seq = self.avatar_seq;
                        self.status = format!("Switched rig to {rig} — syncing render...");
                        self.spawn(move || {
                            Msg::AvatarApplied(seq, apply_avatar_type(uid, &cookie, &rig))
                        });
                    }
                }
                Action::SetBodyColorId(color_id) => {
                    if let (Some(uid), Some(cookie)) =
                        (self.me.as_ref().map(|p| p.user.id), self.cookie.clone())
                    {
                        if let Some(st) = &mut self.avatar_state {
                            let mut colors = st.colors;
                            match self.avatar_color_target {
                                BodyColorTarget::All => {
                                    colors = BodyColors {
                                        head: color_id,
                                        torso: color_id,
                                        right_arm: color_id,
                                        left_arm: color_id,
                                        right_leg: color_id,
                                        left_leg: color_id,
                                    };
                                }
                                BodyColorTarget::Head => colors.head = color_id,
                                BodyColorTarget::Torso => colors.torso = color_id,
                                BodyColorTarget::LeftArm => colors.left_arm = color_id,
                                BodyColorTarget::RightArm => colors.right_arm = color_id,
                                BodyColorTarget::LeftLeg => colors.left_leg = color_id,
                                BodyColorTarget::RightLeg => colors.right_leg = color_id,
                            }
                            st.colors = colors;
                            self.avatar_err = None;
                            self.avatar_seq += 1;
                            self.avatar_pending += 1;
                            let seq = self.avatar_seq;
                            self.status = "Updating skin tone...".to_string();
                            self.spawn(move || {
                                Msg::AvatarApplied(seq, apply_body_colors(uid, &cookie, colors))
                            });
                        }
                    }
                }
                Action::ApplyAvatarScales => {
                    if let (Some(st), Some(uid), Some(cookie)) = (
                        self.avatar_state.as_ref(),
                        self.me.as_ref().map(|p| p.user.id),
                        self.cookie.clone(),
                    ) {
                        let scales = st.scales;
                        self.avatar_err = None;
                        self.avatar_seq += 1;
                        self.avatar_pending += 1;
                        let seq = self.avatar_seq;
                        self.status = "Updating avatar scales...".to_string();
                        self.spawn(move || {
                            Msg::AvatarApplied(seq, apply_avatar_scales(uid, &cookie, scales))
                        });
                    }
                }
                Action::ResetAvatarScales => {
                    if let (Some(uid), Some(cookie)) =
                        (self.me.as_ref().map(|p| p.user.id), self.cookie.clone())
                    {
                        let scales = AvatarScales {
                            height: 1.0,
                            width: 1.0,
                            head: 1.0,
                            depth: 1.0,
                            proportion: 0.0,
                            body_type: 0.0,
                        };
                        if let Some(st) = &mut self.avatar_state {
                            st.scales = scales;
                        }
                        self.avatar_err = None;
                        self.avatar_seq += 1;
                        self.avatar_pending += 1;
                        let seq = self.avatar_seq;
                        self.status = "Resetting avatar scales...".to_string();
                        self.spawn(move || {
                            Msg::AvatarApplied(seq, apply_avatar_scales(uid, &cookie, scales))
                        });
                    }
                }
                Action::DismissAvatarErr => {
                    self.avatar_err = None;
                }
                Action::GoChart(i) => {
                    self.chart = i;
                    self.page = Page::Charts;
                    if Self::is_stale(
                        &self.last_sorts_sync,
                        &self.sorts_sync_started,
                        Duration::from_secs(12),
                    ) {
                        self.home_sync_spinner = false;
                        self.sync_sorts(false);
                    }
                }
                Action::Open(g) => {
                    if self
                        .active_private_server
                        .as_ref()
                        .is_some_and(|(uid, _, _)| *uid != g.universe_id)
                    {
                        self.active_private_server = None;
                    }
                    self.open_game_details(g);
                }
                Action::SelectGameTab(tab) => {
                    self.game_tab = tab;
                    if let Page::Game(g) = &self.page {
                        let uid = g.universe_id;
                        let pid = g.place_id;
                        let need_passes = tab == GameTab::Store
                            && !self
                                .game_extra
                                .as_ref()
                                .is_some_and(|e| e.universe_id == uid && e.passes_loaded);
                        let need_servers = tab == GameTab::Servers
                            && !self
                                .game_extra
                                .as_ref()
                                .is_some_and(|e| e.universe_id == uid && e.servers_loaded);
                        if need_passes {
                            self.load_game_passes(uid);
                        }
                        if need_servers {
                            self.load_game_servers(uid, pid, false);
                        }
                    }
                }
                Action::GameMediaStep(delta) => {
                    let len = self.game_extra.as_ref().map(|e| e.media.len()).unwrap_or(0);
                    if len > 1 {
                        let n = len as isize;
                        self.game_media_idx = ((self.game_media_idx as isize + delta as isize)
                            .rem_euclid(n)) as usize;
                    }
                }
                Action::GameMediaSelect(idx) => {
                    self.game_media_idx = idx;
                }
                Action::JoinServer(pid, job_id) => {
                    self.status = "Getting launch ticket for server...".to_string();
                    self.record_recent_place_id(pid);
                    self.note_discord_game_launch(pid, None);
                    self.launch_server(pid, job_id);
                }
                Action::VoteGame(uid, vote) => {
                    if let Some(cookie) = self.cookie.clone() {
                        let prev = self
                            .game_extra
                            .as_ref()
                            .filter(|e| e.universe_id == uid)
                            .and_then(|e| e.user_vote);
                        if let Some(ex) = &mut self.game_extra {
                            if ex.universe_id == uid {
                                ex.user_vote = vote;
                            }
                        }
                        if let Page::Game(g) = &mut self.page {
                            if g.universe_id == uid {
                                match (prev, vote) {
                                    (Some(true), Some(false)) => {
                                        g.up = g.up.saturating_sub(1);
                                        g.down = g.down.saturating_add(1);
                                    }
                                    (Some(false), Some(true)) => {
                                        g.down = g.down.saturating_sub(1);
                                        g.up = g.up.saturating_add(1);
                                    }
                                    (None, Some(true)) => g.up = g.up.saturating_add(1),
                                    (None, Some(false)) => g.down = g.down.saturating_add(1),
                                    (Some(true), None) => g.up = g.up.saturating_sub(1),
                                    (Some(false), None) => g.down = g.down.saturating_sub(1),
                                    _ => {}
                                }
                            }
                        }
                        self.spawn(move || {
                            Msg::GameVoteDone(uid, vote, vote_game(uid, vote, &cookie))
                        });
                    }
                }
                Action::RefreshServers(uid, pid) => {
                    // Keep the loaded servers on screen while the new list is fetched:
                    // emptying it collapsed the page and threw the scroll position away.
                    self.load_game_servers(uid, pid, false);
                }
                Action::LoadMoreServers(uid, pid) => {
                    self.load_game_servers(uid, pid, true);
                }
                Action::OpenAddFriend => {
                    if !matches!(self.page, Page::AddFriend) {
                        self.back = self.page.clone();
                    }
                    self.page = Page::AddFriend;
                    if !self.social_lists.contains_key(&SocialTab::Requests)
                        && !self.social_loading.contains(&SocialTab::Requests)
                    {
                        self.load_social_tab(SocialTab::Requests, false);
                    }
                    let tab = self.social_tab;
                    if tab != SocialTab::Requests
                        && tab != SocialTab::Friends
                        && !self.social_lists.contains_key(&tab)
                        && !self.social_loading.contains(&tab)
                    {
                        self.load_social_tab(tab, false);
                    }
                }
                Action::SelectSocialTab(tab) => {
                    self.social_tab = tab;
                    if tab != SocialTab::Friends
                        && !self.social_lists.contains_key(&tab)
                        && !self.social_loading.contains(&tab)
                    {
                        self.load_social_tab(tab, false);
                    }
                }
                Action::RefreshSocialTab(tab) => {
                    self.load_social_tab(tab, false);
                }
                Action::LoadMoreSocialTab(tab) => {
                    self.load_social_tab(tab, true);
                }
                Action::AcceptFriendRequest(uid, uname) => {
                    if self.social_responding.insert(uid) {
                        if let Some(cookie) = self.cookie.clone() {
                            self.status = format!("Accepting @{uname}...");
                            self.spawn(move || Msg::FriendRequestResponded {
                                uid,
                                uname,
                                accepted: true,
                                res: respond_friend_request_api(uid, true, &cookie),
                            });
                        } else {
                            self.social_responding.remove(&uid);
                        }
                    }
                }
                Action::DeclineFriendRequest(uid, uname) => {
                    if self.social_responding.insert(uid) {
                        if let Some(cookie) = self.cookie.clone() {
                            self.status = format!("Declining @{uname}...");
                            self.spawn(move || Msg::FriendRequestResponded {
                                uid,
                                uname,
                                accepted: false,
                                res: respond_friend_request_api(uid, false, &cookie),
                            });
                        } else {
                            self.social_responding.remove(&uid);
                        }
                    }
                }
                Action::SearchUsers(q) => {
                    let q = q.trim().to_string();
                    if !q.is_empty() {
                        let cookie = self.cookie.clone();
                        let sid = self.sid.clone();
                        self.friend_searched = q.clone();
                        self.friend_searching = true;
                        self.friend_search_loading_more = false;
                        self.friend_search_cursor = None;
                        self.friend_search_results = None;
                        self.friend_search_verified.clear();
                        self.spawn(move || {
                            Msg::FriendSearchResult(
                                false,
                                search_users_api(&q, None, &sid, cookie.as_deref()),
                            )
                        });
                    }
                }
                Action::LoadMoreUsers => {
                    if !self.friend_search_loading_more && self.friend_search_cursor.is_some() {
                        let q = self.friend_searched.clone();
                        let cursor = self.friend_search_cursor.clone();
                        let cookie = self.cookie.clone();
                        let sid = self.sid.clone();
                        self.friend_search_loading_more = true;
                        self.spawn(move || {
                            Msg::FriendSearchResult(
                                true,
                                search_users_api(&q, cursor.as_deref(), &sid, cookie.as_deref()),
                            )
                        });
                    }
                }
                Action::SendFriendRequest(uid, uname) => {
                    self.friend_sent.insert(uid);
                    if let Some(cookie) = self.cookie.clone() {
                        self.status = format!("Sending friend request to @{uname}...");
                        self.spawn(move || {
                            Msg::FriendRequestDone(
                                uid,
                                uname,
                                send_friend_request_api(uid, &cookie),
                            )
                        });
                    }
                }
                Action::OpenFriend(user) => {
                    if !matches!(self.page, Page::Friend(_)) {
                        self.back = self.page.clone();
                    }
                    let uid = user.id;
                    let cookie = self.cookie.clone();
                    // Paint a profile we already have immediately. If it is under a minute
                    // old, skip the network entirely: the page opens in a frame.
                    let cached = self.profile_cache.get(&uid).cloned();
                    let fresh = cached
                        .as_ref()
                        .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(60));
                    self.friend_profile = cached.as_ref().map(|(p, _)| p.clone());
                    self.friend_err = None;
                    self.friend_loading = cached.is_none();
                    self.page = Page::Friend(user.clone());
                    if !fresh {
                        self.spawn(move || {
                            Msg::FriendProfile(uid, fetch_friend_profile(user, cookie))
                        });
                    }
                }
                Action::Play(g) => self.play(&g),
                Action::JoinPlace(pid) => {
                    self.status = "Joining friend...".to_string();
                    self.record_recent_place_id(pid);
                    self.note_discord_game_launch(pid, None);
                    self.launch_place_id(pid);
                }
                Action::JoinPrivateServer(pid, code) => {
                    self.status = "Joining private server...".to_string();
                    let known_game = match &self.page {
                        Page::Game(g) if g.place_id == pid => Some(g.clone()),
                        _ => None,
                    };
                    if let Some(ref g) = known_game {
                        self.record_recent_game(g.clone());
                    } else {
                        self.record_recent_place_id(pid);
                    }
                    self.note_discord_game_launch(pid, known_game);
                    self.launch_private_server(pid, code);
                }
                Action::OpenManualGame => {
                    self.manual_game_open = true;
                    self.manual_game_err = None;
                    self.manual_game_loading = false;
                    self.manual_game_focus_requested = true;
                }
                Action::ToggleFav(g) => self.toggle_fav(&g),
            }
        }
        if do_search && self.cookie.is_some() {
            self.start_search();
        }
        let next_page_game_id = match &self.page {
            Page::Game(g) => Some(g.universe_id),
            _ => None,
        };
        if std::mem::discriminant(&self.page) != prev_page_disc
            || next_page_game_id != prev_page_game_id
        {
            self.sync_discord_presence();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.watcher_stop.store(true, Ordering::Relaxed);
        self.auto_close_flag.store(false, Ordering::Relaxed);
        self.tray.shutdown();
        self.discord_rpc.shutdown();
        if self.settings.auto_close_roblox {
            close_tray_roblox_processes();
        }
        if let Some(staged) = self.app_staged_patch.take() {
            launch_quiet_apply_on_exit(&staged);
        }
    }
}
