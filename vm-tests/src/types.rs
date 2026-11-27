//! The parts of the launcher's `types.rs` the included modules touch.

use crate::party::PartyEvent;

#[derive(Clone, Debug)]
pub enum Msg {
    Party(PartyEvent),
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
