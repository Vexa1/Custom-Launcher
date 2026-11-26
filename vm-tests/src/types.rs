//! The parts of the launcher's `types.rs` the included modules touch.

use crate::party::PartyEvent;

#[derive(Clone, Debug)]
pub enum Msg {
    Party(PartyEvent),
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
