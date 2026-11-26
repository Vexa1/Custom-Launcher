//! Capacity-aware server selection for a whole party.
//!
//! This is the half of `api::find_server_for_party` that decides, with no sockets and no
//! `egui`, which public server a party of `N` should join: rank the listed rooms, fall back
//! to more pages only when page one has nothing, and explain the failure when no room fits.
//! Keeping it here lets `vm-tests` drive the real policy against mock Roblox server lists.

use crate::types::ServerItem;

/// A public server picked for a whole party.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyServer {
    pub job_id: String,
    pub playing: u64,
    pub max_players: u64,
    pub ping: Option<u64>,
    pub free_slots: u64,
    /// The room has one seat more than the party needs, so a random player cannot take the
    /// last spot while the slower members are still joining.
    pub buffered: bool,
}

/// Why no single server could hold the party. The host is told before anything launches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartyServerError {
    /// Every listed server caps below the party size (single-player or small experiences).
    TooSmall { max_players: u64, party_size: usize },
    /// Servers exist, but none has room for everybody at once.
    NoCapacity {
        party_size: usize,
        best_free: u64,
        listed: usize,
    },
    /// The request itself failed (offline, expired session, rate limit).
    Api(String),
}

impl PartyServerError {
    pub fn message(&self) -> String {
        match self {
            PartyServerError::TooSmall {
                max_players,
                party_size,
            } => format!(
                "This experience allows {max_players} players per server, so a party of {party_size} cannot fit in one"
            ),
            PartyServerError::NoCapacity {
                party_size,
                best_free,
                listed,
            } => format!(
                "No server has room for {party_size} right now (best is {best_free} open slots of {listed} listed)"
            ),
            PartyServerError::Api(e) => format!("Could not list servers: {e}"),
        }
    }
}

/// Pick one public server with room for the whole party.
///
/// Order of preference: a room with a spare seat over one that is exactly big enough (so a
/// random player cannot take the last slot mid-join), then the lowest reported ping, then
/// the healthier lobby.
pub fn pick_party_server(servers: &[ServerItem], party_size: u64) -> Option<PartyServer> {
    let mut candidates: Vec<PartyServer> = servers
        .iter()
        .filter(|s| s.max_players > 0)
        .filter_map(|s| {
            let free = s.max_players.saturating_sub(s.playing);
            if free < party_size {
                return None;
            }
            Some(PartyServer {
                job_id: s.id.clone(),
                playing: s.playing,
                max_players: s.max_players,
                ping: s.ping.filter(|p| *p > 0),
                free_slots: free,
                buffered: free > party_size,
            })
        })
        .collect();
    candidates.sort_by(|a, b| {
        // A spare seat first, then ping, then the livelier room.
        b.buffered
            .cmp(&a.buffered)
            .then_with(|| a.ping.unwrap_or(u64::MAX).cmp(&b.ping.unwrap_or(u64::MAX)))
            .then_with(|| b.playing.cmp(&a.playing))
    });
    candidates.into_iter().next()
}

/// The whole selection policy, with the network injected.
///
/// `fetch(pages)` returns up to `pages` pages of public servers for the place. Page one is
/// asked for first and the wider sweep only happens when page one has nothing the party fits
/// in, which is what keeps the common case to a single request.
pub fn select_for_party<F>(party_size: usize, mut fetch: F) -> Result<PartyServer, PartyServerError>
where
    F: FnMut(usize) -> Result<Vec<ServerItem>, String>,
{
    let size = party_size.max(1) as u64;
    let mut listed = fetch(1).map_err(PartyServerError::Api)?;
    if pick_party_server(&listed, size).is_none() {
        // Only now pay for the extra pages.
        listed = fetch(3).map_err(PartyServerError::Api)?;
    }
    if let Some(server) = pick_party_server(&listed, size) {
        return Ok(server);
    }
    let max_players = listed.iter().map(|s| s.max_players).max().unwrap_or(0);
    if max_players > 0 && max_players < size {
        return Err(PartyServerError::TooSmall {
            max_players,
            party_size: party_size.max(1),
        });
    }
    let best_free = listed
        .iter()
        .filter(|s| s.max_players > 0)
        .map(|s| s.max_players.saturating_sub(s.playing))
        .max()
        .unwrap_or(0);
    Err(PartyServerError::NoCapacity {
        party_size: party_size.max(1),
        best_free,
        listed: listed.len(),
    })
}
