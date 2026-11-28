//! The user-facing settings struct and its two colour-mode enums.
//!
//! Kept free of `egui` and of the window types so `vm-tests` can compile the real value and
//! drive the real `settings.json` round trip in a headless VM.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BgMode {
    #[default]
    ThemeDefault,
    SolidColor,
    Wallpaper,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextColorMode {
    #[default]
    Auto,
    White,
    Green,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub dark: bool,
    pub bg_mode: BgMode,
    pub bg_color: [u8; 3],
    pub bg_wallpaper: String,
    pub bg_overlay: f32,
    pub text_mode: TextColorMode,
    pub borders_color: Option<[u8; 3]>,
    pub profile_color: Option<[u8; 3]>,
    pub friends_color: Option<[u8; 3]>,
    pub games_border_color: Option<[u8; 3]>,
    pub text_border_color: Option<[u8; 3]>,
    pub target_fps: Option<u32>,
    pub ram_limit_mb: Option<u32>,
    pub cpu_limit_pct: Option<u8>,
    pub glass_edition: bool,
    pub glass_opacity: f32,
    pub glass_highlight: f32,
    pub corner_roundness: u8,
    pub discord_rpc_enabled: bool,
    pub discord_rpc_show_idle: bool,
    pub discord_rpc_app_id: String,
    pub tray_enabled: bool,
    pub tray_minimize_on_close: bool,
    pub start_with_windows: bool,
    pub start_minimized_to_tray: bool,
    pub auto_close_roblox: bool,
    /// Launch a separate Roblox copy per Play instead of reusing the running app.
    pub multi_instance: bool,
    pub notifications_silent: bool,
    pub auto_check_updates: bool,
    /// Download an available update automatically; installing still needs a click.
    pub auto_download_updates: bool,
    /// Silently re-fetch game lists, player counts and friends activity in the background.
    /// Off means data is only fetched on load, tab change, filter change or an explicit refresh.
    pub auto_refresh: bool,
    /// Show the server region popup on top of the Roblox game window.
    pub overlay_enabled: bool,
    /// 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
    pub overlay_corner: u8,
    /// Seconds before the overlay hides itself; 0 = stay until the game closes.
    pub overlay_duration_secs: u8,
    pub overlay_opacity: f32,
    pub overlay_show_ip: bool,
    pub overlay_show_job_id: bool,
    /// Where the user dragged each in-game HUD panel, in points from that panel's corner
    /// anchor. Written by the tray's "Edit Overlay Position" mode; a missing panel sits in
    /// its corner slot, which is already stacked clear of the others.
    pub overlay_panels: Vec<crate::overlay_spec::PanelOffset>,
    /// Show the "Connected server" badge in the launcher window.
    pub show_server_badge: bool,
    /// Bring the launcher to the front when you leave a game.
    pub return_to_launcher_on_leave: bool,
    /// Minimize the launcher when a game is launched.
    pub minimize_on_game_launch: bool,
    /// Quit the launcher window while a game runs (a tiny headless helper reopens it).
    pub close_launcher_during_game: bool,
    pub overlay_stats_enabled: bool,
    /// The party member strip: who is in the party and which server they are in.
    pub party_overlay: bool,
    pub party_overlay_corner: u8,
    pub overlay_stats_corner: u8,
    pub overlay_stats_cpu: bool,
    pub overlay_stats_ram: bool,
    pub overlay_stats_ping: bool,
    pub overlay_stats_session: bool,
    pub overlay_stats_clock: bool,
    pub overlay_stats_fps_cap: bool,
    pub overlay_flags_enabled: bool,
    pub overlay_flags_corner: u8,
    pub overlay_flags_max_lines: u8,
    /// Show the game name / players / place ID overlay in game.
    pub overlay_game_enabled: bool,
    pub overlay_game_corner: u8,
    /// Optional picture (PNG / JPG / WEBP) drawn inside the game.
    pub overlay_image_enabled: bool,
    pub overlay_image_path: String,
    /// 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
    pub overlay_image_corner: u8,
    /// Picture height as a percentage of the game window height.
    pub overlay_image_size: u8,
    pub overlay_image_opacity: f32,
    pub overlay_image_offset_x: i16,
    pub overlay_image_offset_y: i16,
    /// The box the HUD editor saved, in points from the game area's top-left. `w`/`h` at 0
    /// mean "use the corner, height and offsets above", so a picture that was never dragged
    /// keeps following its size slider. `x`/`y` are only read while `w`/`h` are set, so the
    /// offset sliders below keep their own meaning.
    pub overlay_image_x: i16,
    pub overlay_image_y: i16,
    pub overlay_image_w: u16,
    pub overlay_image_h: u16,
    /// Draw a crosshair in the middle of the Roblox window.
    pub crosshair_enabled: bool,
    /// 0 dot, 1 cross, 2 circle, 3 cross + dot.
    pub crosshair_style: u8,
    pub crosshair_size: u8,
    pub crosshair_thickness: u8,
    pub crosshair_gap: u8,
    pub crosshair_color: [u8; 3],
    pub crosshair_opacity: f32,
    pub crosshair_offset_x: i8,
    pub crosshair_offset_y: i8,
    /// Master switch: off means no moving interface at all.
    pub animations_enabled: bool,
    /// Hover fades, selection rings and the sliding nav indicator (needs the master switch).
    pub hover_animations: bool,
    /// Animated login backdrop and appearance previews.
    pub animated_backgrounds: bool,
    /// Play game preview videos (uses RAM, CPU and data).
    pub video_previews: bool,
    /// PNG drawn as the crosshair instead of the shapes (empty = shapes).
    pub crosshair_image: String,
    /// Server browser quick filters - these stack with each other.
    pub server_filter_low_ping: bool,
    pub server_filter_few_players: bool,
    pub server_filter_good_fps: bool,
    /// Optional ranges for the server browser.
    pub server_min_players: Option<u32>,
    pub server_max_players: Option<u32>,
    pub server_max_ping: Option<u32>,
    /// Show the people in each server (fetched on demand, never for every server at once).
    pub server_show_players: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: false,
            bg_mode: BgMode::ThemeDefault,
            bg_color: [22, 30, 46],
            bg_wallpaper: String::new(),
            bg_overlay: 0.42,
            text_mode: TextColorMode::Auto,
            borders_color: None,
            profile_color: None,
            friends_color: None,
            games_border_color: None,
            text_border_color: None,
            target_fps: None,
            ram_limit_mb: None,
            cpu_limit_pct: None,
            glass_edition: false,
            glass_opacity: 0.56,
            glass_highlight: 0.72,
            corner_roundness: 14,
            discord_rpc_enabled: true,
            discord_rpc_show_idle: true,
            discord_rpc_app_id: String::new(),
            tray_enabled: true,
            tray_minimize_on_close: true,
            start_with_windows: false,
            start_minimized_to_tray: true,
            auto_close_roblox: true,
            multi_instance: false,
            notifications_silent: false,
            auto_check_updates: true,
            auto_download_updates: false,
            auto_refresh: true,
            overlay_enabled: true,
            overlay_corner: 3,
            overlay_duration_secs: 10,
            overlay_opacity: 0.92,
            overlay_show_ip: true,
            overlay_show_job_id: false,
            show_server_badge: true,
            return_to_launcher_on_leave: true,
            minimize_on_game_launch: false,
            close_launcher_during_game: false,
            // On by default: the server panel already defaults on, and a default install
            // that shows a region but no CPU/RAM/ping reads as "the stats are broken".
            overlay_stats_enabled: true,
            party_overlay: true,
            party_overlay_corner: 2,
            overlay_stats_corner: 1,
            overlay_stats_cpu: true,
            overlay_stats_ram: true,
            overlay_stats_ping: true,
            overlay_stats_session: true,
            overlay_stats_clock: false,
            overlay_stats_fps_cap: true,
            overlay_flags_enabled: false,
            overlay_flags_corner: 0,
            overlay_panels: Vec::new(),
            overlay_flags_max_lines: 12,
            overlay_game_enabled: false,
            overlay_game_corner: 2,
            overlay_image_enabled: false,
            overlay_image_path: String::new(),
            overlay_image_corner: 0,
            overlay_image_size: 25,
            overlay_image_opacity: 0.9,
            overlay_image_offset_x: 0,
            overlay_image_offset_y: 0,
            overlay_image_x: 0,
            overlay_image_y: 0,
            overlay_image_w: 0,
            overlay_image_h: 0,
            crosshair_enabled: false,
            crosshair_style: 3,
            crosshair_size: 18,
            crosshair_thickness: 2,
            crosshair_gap: 4,
            crosshair_color: [255, 60, 60],
            crosshair_opacity: 0.9,
            crosshair_offset_x: 0,
            crosshair_offset_y: 0,
            animations_enabled: true,
            hover_animations: true,
            animated_backgrounds: true,
            video_previews: true,
            crosshair_image: String::new(),
            server_filter_low_ping: false,
            server_filter_few_players: false,
            server_filter_good_fps: false,
            server_min_players: None,
            server_max_players: None,
            server_max_ping: None,
            server_show_players: true,
        }
    }
}
