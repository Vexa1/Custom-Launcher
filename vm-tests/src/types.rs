//! The parts of the launcher's `types.rs` the included modules touch.

use crate::party::PartyEvent;

#[derive(Clone, Debug)]
pub enum Msg {
    Party(PartyEvent),
}

/// Same shape as `src/types.rs`.
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarAsset {
    pub id: u64,
    pub name: String,
    pub asset_type_id: u64,
    pub asset_type_name: String,
    pub thumb: String,
    pub meta: Option<serde_json::Value>,
    pub creator_name: String,
    pub creator_verified: bool,
    pub price: Option<u64>,
}

/// Same shape as `src/types.rs`.
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
    pub players_text: String,
    pub rating_text: String,
}

/// Same shape as `src/types.rs`.
#[derive(Clone, Debug)]
pub struct Sort {
    pub title: String,
    pub games: Vec<Game>,
}

/// Same shape as `src/types.rs`.
#[derive(Clone, Debug)]
pub struct ServerItem {
    pub id: String,
    pub playing: u64,
    pub max_players: u64,
    pub ping: Option<u64>,
    pub fps: Option<f64>,
}

/// Same shape as `src/types.rs`.
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

/// Same shape as `src/types.rs`.
#[derive(Clone, Debug, Default)]
pub struct PlayerBadge {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub awarded_date: String,
}

/// Same shape as `src/types.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProfileSections {
    pub info: bool,
    pub friends: bool,
    pub followers: bool,
    pub following: bool,
    pub avatar: bool,
    pub wearing: bool,
    pub badges: bool,
    pub creations: bool,
    pub favorites: bool,
}

/// Same shape as `src/types.rs`.
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
    pub sections: ProfileSections,
}
