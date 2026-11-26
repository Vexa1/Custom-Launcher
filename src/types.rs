use crate::avatar_3d;
use crate::video_preview;
use eframe::egui::{self, Color32};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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
    /// Show the server region popup on top of the Roblox game window.
    pub overlay_enabled: bool,
    /// 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
    pub overlay_corner: u8,
    /// Seconds before the overlay hides itself; 0 = stay until the game closes.
    pub overlay_duration_secs: u8,
    pub overlay_opacity: f32,
    pub overlay_show_ip: bool,
    pub overlay_show_job_id: bool,
    /// Show the "Connected server" badge in the launcher window.
    pub show_server_badge: bool,
    /// Bring the launcher to the front when you leave a game.
    pub return_to_launcher_on_leave: bool,
    /// Minimize the launcher when a game is launched.
    pub minimize_on_game_launch: bool,
    /// Quit the launcher window while a game runs (a tiny headless helper reopens it).
    pub close_launcher_during_game: bool,
    pub overlay_stats_enabled: bool,
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
            overlay_stats_enabled: false,
            overlay_stats_corner: 1,
            overlay_stats_cpu: true,
            overlay_stats_ram: true,
            overlay_stats_ping: true,
            overlay_stats_session: true,
            overlay_stats_clock: false,
            overlay_stats_fps_cap: true,
            overlay_flags_enabled: false,
            overlay_flags_corner: 0,
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Game {
    pub universe_id: u64,
    pub place_id: u64,
    pub name: String,
    pub creator: String,
    pub creator_verified: bool,
    pub playing: u64,
    pub up: u64,
    pub down: u64,
    pub description: String,
}

impl Game {
    pub fn rating(&self) -> String {
        let total = self.up + self.down;
        if total == 0 {
            "--".to_string()
        } else {
            format!("{}%", self.up * 100 / total)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sort {
    pub title: String,
    pub games: Vec<Game>,
}

#[derive(Clone, Debug)]
pub struct ResolvedGameLink {
    pub game: Game,
    pub place_id: u64,
    pub private_server_code: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ManualGameIntent {
    Open,
    Favorite,
    Play,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameTab {
    About,
    Store,
    Servers,
}

impl GameTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::About => "About",
            Self::Store => "Store",
            Self::Servers => "Servers",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GamePassItem {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub price: Option<u64>,
    pub is_for_sale: bool,
    pub is_owned: bool,
    pub thumb: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ServerSort {
    #[default]
    Default,
    LowestPing,
    FewestPlayers,
    MostPlayers,
    HighestFps,
}

impl ServerSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::LowestPing => "Lowest Ping",
            Self::FewestPlayers => "Fewest Players",
            Self::MostPlayers => "Most Players",
            Self::HighestFps => "Highest FPS",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ServerItem {
    pub id: String,
    pub playing: u64,
    pub max_players: u64,
    pub ping: Option<u64>,
    pub fps: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct GameExtra {
    pub universe_id: u64,
    pub verified_creator: bool,
    pub favorites: u64,
    pub visits: u64,
    pub max_players: u64,
    pub created: String,
    pub updated: String,
    pub genre: String,
    pub subgenre: String,
    pub maturity_short: String,
    pub maturity_detail: String,
    pub voice_chat: Option<bool>,
    pub camera: Option<bool>,
    pub is_favorited_api: Option<bool>,
    pub can_vote: bool,
    pub user_vote: Option<bool>,
    pub media: Vec<String>,
    pub preview_video_id: Option<u64>,
    pub passes: Vec<GamePassItem>,
    pub passes_loaded: bool,
    pub servers: Vec<ServerItem>,
    pub servers_cursor: Option<String>,
    pub servers_loaded: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub display: String,
    pub headshot: String,
    pub presence: u8,
    pub location: String,
    pub place_id: Option<u64>,
    pub verified: bool,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub user: User,
    pub friends: u64,
    pub avatar: String,
    pub robux: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct AvatarAsset {
    pub id: u64,
    pub name: String,
    pub asset_type_id: u64,
    pub asset_type_name: String,
    pub thumb: String,
    pub meta: Option<Value>,
    pub creator_name: String,
    pub creator_verified: bool,
    pub price: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarScales {
    pub height: f32,
    pub width: f32,
    pub head: f32,
    pub depth: f32,
    pub proportion: f32,
    pub body_type: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyColors {
    pub head: u64,
    pub torso: u64,
    pub right_arm: u64,
    pub left_arm: u64,
    pub right_leg: u64,
    pub left_leg: u64,
}

#[derive(Clone, Debug)]
pub struct OutfitItem {
    pub id: u64,
    pub name: String,
    pub thumb: String,
    pub is_editable: bool,
}

#[derive(Clone, Debug)]
pub struct PaletteColor {
    pub id: u64,
    pub name: String,
    pub color: Color32,
}

#[derive(Clone, Debug)]
pub struct AvatarState {
    pub avatar_type: String,
    pub scales: AvatarScales,
    pub colors: BodyColors,
    pub wearing: Vec<AvatarAsset>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AvatarTab {
    Wearing,
    Outfits,
    Accessories,
    Hair,
    Shirts,
    Pants,
    TShirts,
    Layered,
    Faces,
    BodyParts,
    Animations,
    Body,
}

impl AvatarTab {
    pub const ALL: [Self; 12] = [
        Self::Wearing,
        Self::Outfits,
        Self::Accessories,
        Self::Hair,
        Self::Shirts,
        Self::Pants,
        Self::TShirts,
        Self::Layered,
        Self::Faces,
        Self::BodyParts,
        Self::Animations,
        Self::Body,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wearing => "Wearing",
            Self::Outfits => "Outfits",
            Self::Accessories => "Accessories",
            Self::Hair => "Hair",
            Self::Shirts => "Shirts",
            Self::Pants => "Pants",
            Self::TShirts => "T-Shirts",
            Self::Layered => "Layered",
            Self::Faces => "Faces & Heads",
            Self::BodyParts => "Body Parts",
            Self::Animations => "Animations",
            Self::Body => "Body & Skin",
        }
    }

    pub fn inventory_query(self) -> Option<&'static str> {
        match self {
            Self::Accessories => Some(
                "Hat,FaceAccessory,NeckAccessory,ShoulderAccessory,FrontAccessory,BackAccessory,WaistAccessory",
            ),
            Self::Hair => Some("HairAccessory"),
            Self::Shirts => Some("Shirt"),
            Self::Pants => Some("Pants"),
            Self::TShirts => Some("TShirt"),
            Self::Layered => Some(
                "ShirtAccessory,TShirtAccessory,PantsAccessory,JacketAccessory,SweaterAccessory,ShortsAccessory,LeftShoeAccessory,RightShoeAccessory,DressSkirtAccessory",
            ),
            Self::Faces => Some("Face,DynamicHead"),
            Self::BodyParts => Some("Head,DynamicHead,Torso,LeftArm,RightArm,LeftLeg,RightLeg"),
            Self::Animations => Some(
                "IdleAnimation,WalkAnimation,RunAnimation,JumpAnimation,FallAnimation,ClimbAnimation,SwimAnimation",
            ),
            Self::Wearing | Self::Outfits | Self::Body => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyPartFilter {
    All,
    Head,
    Torso,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl BodyPartFilter {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Head,
        Self::Torso,
        Self::LeftArm,
        Self::RightArm,
        Self::LeftLeg,
        Self::RightLeg,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Head => "Heads",
            Self::Torso => "Torsos",
            Self::LeftArm => "L Arms",
            Self::RightArm => "R Arms",
            Self::LeftLeg => "L Legs",
            Self::RightLeg => "R Legs",
        }
    }

    pub fn matches(self, asset: &AvatarAsset) -> bool {
        let tid = asset.asset_type_id;
        let tname = asset.asset_type_name.as_str();
        match self {
            Self::All => true,
            Self::Head => matches!(tid, 17 | 79) || tname == "Head" || tname == "DynamicHead",
            Self::Torso => tid == 27 || tname == "Torso",
            Self::LeftArm => tid == 29 || tname == "LeftArm" || tname == "Left Arm",
            Self::RightArm => tid == 28 || tname == "RightArm" || tname == "Right Arm",
            Self::LeftLeg => tid == 30 || tname == "LeftLeg" || tname == "Left Leg",
            Self::RightLeg => tid == 31 || tname == "RightLeg" || tname == "Right Leg",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutfitFilter {
    All,
    Saved,
    Characters,
}

impl OutfitFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::Saved, Self::Characters];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Saved => "Saved",
            Self::Characters => "Characters",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyColorTarget {
    All,
    Head,
    Torso,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl BodyColorTarget {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Head,
        Self::Torso,
        Self::LeftArm,
        Self::RightArm,
        Self::LeftLeg,
        Self::RightLeg,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Head => "Head",
            Self::Torso => "Torso",
            Self::LeftArm => "L Arm",
            Self::RightArm => "R Arm",
            Self::LeftLeg => "L Leg",
            Self::RightLeg => "R Leg",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OmniHome {
    pub continue_unis: Option<Vec<u64>>,
    pub topics: Vec<(String, Vec<u64>)>,
}

#[derive(Clone, Debug, Default)]
pub struct SavedSyncData {
    pub remote_recent: Option<Vec<Game>>,
    pub fallback_recent: Option<Vec<Game>>,
    pub remote_favs: Option<Vec<Game>>,
    pub fallback_favs: Option<Vec<Game>>,
}

#[derive(Clone, Debug, Default)]
pub struct UserSearchPage {
    pub users: Vec<User>,
    pub verified: HashSet<u64>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PlayerBadge {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub awarded_date: String,
}

#[derive(Clone, Debug, Default)]
pub struct FriendProfile {
    pub user: User,
    pub description: String,
    pub created: String,
    pub verified: bool,
    pub friends: u64,
    pub followers: u64,
    pub following: u64,
    pub avatar: String,
    pub playing_game: Option<Game>,
    pub experiences: Vec<Game>,
    pub favorites: Vec<Game>,
    pub badges: Vec<PlayerBadge>,
    pub currently_wearing: Vec<AvatarAsset>,
}

#[derive(Clone, Copy)]
pub struct CardPreview<'a> {
    pub universe_id: u64,
    pub texture: Option<&'a egui::TextureHandle>,
    pub is_loading: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavIcon {
    Home,
    Charts,
    Profile,
    Avatar,
    Notifications,
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Home,
    Charts,
    Search,
    Game(Game),
    Friend(User),
    AddFriend,
    Profile,
    Avatar,
    Notifications,
    Settings,
    Login,
}

pub struct QuickLoginState {
    pub code: String,
    pub qr_uri: String,
    pub qr_bytes: Option<Vec<u8>>,
    pub status_text: String,
    pub error: Option<String>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SocialTab {
    #[default]
    Requests,
    Friends,
    Followers,
    Following,
}

impl SocialTab {
    pub const ALL: [SocialTab; 4] = [
        SocialTab::Requests,
        SocialTab::Friends,
        SocialTab::Followers,
        SocialTab::Following,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SocialTab::Requests => "Requests",
            SocialTab::Friends => "Friends",
            SocialTab::Followers => "Followers",
            SocialTab::Following => "Following",
        }
    }
}

/// How the friends tab is ordered when you open it from the plus button.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FriendsSort {
    /// People you can play with right now first, then by name.
    #[default]
    RecentlyActive,
    /// Online / in-game friends first, offline after them.
    OnlineFirst,
    NameAsc,
    NameDesc,
}

impl FriendsSort {
    pub const ALL: [FriendsSort; 4] = [
        FriendsSort::RecentlyActive,
        FriendsSort::OnlineFirst,
        FriendsSort::NameAsc,
        FriendsSort::NameDesc,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RecentlyActive => "Recently active",
            Self::OnlineFirst => "Online first",
            Self::NameAsc => "Name (A\u{2013}Z)",
            Self::NameDesc => "Name (Z\u{2013}A)",
        }
    }
}

/// Server browser region selector. Roblox's public server list does not publish a
/// datacenter, so the regions are latency bands: each one shows the best ping the loaded
/// servers actually report, which is what tells you whether a server is close to you.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ServerRegion {
    #[default]
    Any,
    Nearby,
    Regional,
    LongDistance,
}

impl ServerRegion {
    pub const ALL: [ServerRegion; 4] = [
        ServerRegion::Any,
        ServerRegion::Nearby,
        ServerRegion::Regional,
        ServerRegion::LongDistance,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any region",
            Self::Nearby => "Close to me (\u{2264}60 ms)",
            Self::Regional => "Same region (\u{2264}120 ms)",
            Self::LongDistance => "Long distance (\u{2264}250 ms)",
        }
    }

    /// Ping ceiling that stands for the region, or `None` for "anywhere".
    pub fn cap_ms(self) -> Option<u32> {
        match self {
            Self::Any => None,
            Self::Nearby => Some(60),
            Self::Regional => Some(120),
            Self::LongDistance => Some(250),
        }
    }

    pub fn from_cap(cap: Option<u32>) -> Self {
        match cap {
            None | Some(0) => Self::Any,
            Some(c) if c <= 60 => Self::Nearby,
            Some(c) if c <= 120 => Self::Regional,
            _ => Self::LongDistance,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToastKind {
    FriendRequest,
    UpdateAvailable,
    Success,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationSource {
    RobloxApi,
    Creblox,
    Process,
}

impl NotificationSource {
    pub fn label(self) -> &'static str {
        match self {
            NotificationSource::RobloxApi => "Roblox API",
            NotificationSource::Creblox => "Launcher",
            NotificationSource::Process => "Process",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ToastAction {
    AcceptFriendRequest { uid: u64, uname: String },
    OpenRequests,
    OpenSettings,
    OpenUrl(String),
    KillRoblox,
}

#[derive(Clone, Debug)]
pub struct NotificationItem {
    pub id: u64,
    pub source: NotificationSource,
    pub kind: ToastKind,
    pub title: String,
    pub body: String,
    pub avatar: Option<String>,
    pub primary_action: Option<(String, ToastAction)>,
    pub secondary_action: Option<(String, ToastAction)>,
    pub created_secs: f64,
    pub read: bool,
}

#[derive(Clone, Debug)]
pub struct ToastNotification {
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub body: String,
    pub avatar: Option<String>,
    pub primary_action: Option<(String, ToastAction)>,
    pub secondary_action: Option<(String, ToastAction)>,
    pub created_at: f64,
    pub duration: f64,
    pub paused_at: Option<f64>,
    pub dismissed_at: Option<f64>,
}

pub enum Action {
    Go(Page),
    GoChart(usize),
    RefreshGames,
    HoverGame(u64),
    Open(Game),
    OpenFriend(User),
    OpenAddFriend,
    SearchUsers(String),
    LoadMoreUsers,
    SendFriendRequest(u64, String),
    SelectSocialTab(SocialTab),
    RefreshSocialTab(SocialTab),
    LoadMoreSocialTab(SocialTab),
    AcceptFriendRequest(u64, String),
    DeclineFriendRequest(u64, String),
    Play(Game),
    JoinPlace(u64),
    JoinPrivateServer(u64, String),
    OpenManualGame,
    ToggleFav(Game),
    SelectGameTab(GameTab),
    GameMediaStep(i32),
    GameMediaSelect(usize),
    JoinServer(u64, String),
    VoteGame(u64, Option<bool>),
    RefreshServers(u64, u64),
    LoadMoreServers(u64, u64),
    SelectAvatarTab(AvatarTab),
    RefreshAvatar,
    RefreshAvatarTab(AvatarTab),
    LoadMoreAvatarTab(AvatarTab),
    ToggleWearAsset(AvatarAsset),
    UnequipAccessories,
    WearOutfit(OutfitItem),
    SaveOutfit(String),
    DeleteOutfit(OutfitItem),
    SetAvatarType(String),
    SetBodyColorId(u64),
    ApplyAvatarScales,
    ResetAvatarScales,
    DismissAvatarErr,
    RefreshMyProfile,
    SaveBio(String),
    OpenUrl(String),
    StartQuickLogin,
    AddAccount,
    SwitchAccount(u64),
    RemoveAccount(u64),
    SignOut,
    KillRobloxProcesses,
}

pub enum Msg {
    Sorts(Result<Vec<Sort>, String>),
    /// Visit counts for a batch of universes, used by the games feed filter.
    GameVisits(HashMap<u64, u64>),
    Search(String, Result<Vec<Game>, String>),
    Lookup(Result<ResolvedGameLink, String>),
    ManualGameLookup {
        intent: ManualGameIntent,
        result: Result<ResolvedGameLink, String>,
    },
    Details(Option<Game>),
    SavedSynced(SavedSyncData),
    RecentPlaceResolved(Game),
    Thumbs {
        wide: HashMap<u64, String>,
        icon: HashMap<u64, String>,
    },
    MyProfile(u64, Result<FriendProfile, String>),
    BioSaved(Result<String, String>),
    FriendProfile(u64, Result<FriendProfile, String>),
    FriendSearchResult(bool, Result<UserSearchPage, String>),
    FriendRequestDone(u64, String, Result<(), String>),
    SocialListLoaded {
        tab: SocialTab,
        append: bool,
        res: Result<(Vec<User>, Option<String>), String>,
    },
    FriendRequestResponded {
        uid: u64,
        uname: String,
        accepted: bool,
        res: Result<(), String>,
    },
    GameExtraLoaded(Box<GameExtra>),
    GamePassesLoaded(u64, Result<Vec<GamePassItem>, String>),
    GameServersLoaded(u64, bool, Result<(Vec<ServerItem>, Option<String>), String>),
    GameVoteDone(u64, Option<bool>, Result<(), String>),
    GameFavDone(u64, bool, Result<(), String>),
    AvatarLoaded(Result<(AvatarState, Vec<PaletteColor>), String>),
    AvatarInventory(
        AvatarTab,
        bool,
        Result<(Vec<AvatarAsset>, Option<String>), String>,
    ),
    AvatarOutfits(u32, Result<(Vec<OutfitItem>, bool), String>),
    OutfitActionDone(Result<String, String>),
    AvatarApplied(u64, Result<(AvatarState, String, String, String), String>),
    Avatar3DLoaded(
        u64,
        Result<(avatar_3d::Avatar3DModel, Vec<(String, Vec<u8>)>), String>,
    ),
    AuthSession(Result<(Profile, Vec<User>), String>),
    QuickLoginCreated {
        code: String,
        qr_bytes: Option<Vec<u8>>,
    },
    QuickLoginStatus {
        code: String,
        status: String,
        account: Option<String>,
    },
    QuickLoginDone(Result<String, String>),
    ServerLogoutDone(Result<(), String>),
    LaunchResult(Result<String, String>),
    RobloxAppClosed,
    /// Any Roblox process exists (game, menu app or helpers). Sent only on change so the
    /// overlay window can stay alive while Roblox runs even when the log has not told us
    /// which place is being played yet.
    RobloxProcesses(bool),
    RobloxSessionUpdated(Option<u64>),
    RobloxServerUpdated(Option<crate::roblox_process::ServerInfo>),
    ServerLocationResolved(String, Option<String>),
    DiscordGameResolved(u64, Option<(Game, Option<String>)>),
    TrayAction(crate::tray::TrayEvent),
    RobloxBackgroundDetected {
        pid: u32,
        is_playing: bool,
    },
    RobloxBackgroundKilled {
        pid: u32,
    },
    GameVideoIdResolved(u64, Option<u64>),
    VideoPreviewLoaded(u64, Result<Arc<video_preview::DecodedPreviewVideo>, String>),
    WallpaperPicked(Option<String>),
    WallpaperLoaded(String, Result<egui::ColorImage, String>),
    UpdateCheckResult(bool, Result<Option<(String, String)>, String>),
    AppUpdateCheckResult(bool, Result<Option<crate::updater::CrebloxRelease>, String>),
    AppUpdateDownloaded(String, Result<std::path::PathBuf, String>),
    AppPatchDone(
        String,
        Result<
            (
                Option<crate::updater::LiveHotfix>,
                Option<std::path::PathBuf>,
            ),
            String,
        >,
    ),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_asset(id: u64, type_id: u64, type_name: &str) -> AvatarAsset {
        AvatarAsset {
            id,
            name: type_name.to_string(),
            asset_type_id: type_id,
            asset_type_name: type_name.to_string(),
            thumb: String::new(),
            meta: None,
            creator_name: String::new(),
            creator_verified: false,
            price: None,
        }
    }

    #[test]
    fn body_parts_tab_and_filters_match_expected_slots() {
        let q = AvatarTab::BodyParts.inventory_query().unwrap();
        for token in [
            "Head", "Torso", "LeftArm", "RightArm", "LeftLeg", "RightLeg",
        ] {
            assert!(q.contains(token), "missing {token} in query {q}");
        }

        let head = mk_asset(1, 17, "Head");
        let torso = mk_asset(2, 27, "Torso");
        let r_arm = mk_asset(3, 28, "Right Arm");
        let l_leg = mk_asset(4, 30, "LeftLeg");

        assert!(BodyPartFilter::All.matches(&torso));
        assert!(BodyPartFilter::Head.matches(&head));
        assert!(!BodyPartFilter::Head.matches(&torso));
        assert!(BodyPartFilter::Torso.matches(&torso));
        assert!(BodyPartFilter::RightArm.matches(&r_arm));
        assert!(BodyPartFilter::LeftLeg.matches(&l_leg));
        assert!(!BodyPartFilter::LeftArm.matches(&r_arm));
    }
}
