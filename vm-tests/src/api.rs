//! Stand-in for `src/api.rs`: the harness has no Roblox session, so the presence fallback is
//! fed from `set_next` instead of a network call.

use std::sync::Mutex;

static NEXT: Mutex<Option<(u64, String, u64, String)>> = Mutex::new(None);

pub struct PartyPresence {
    pub in_game: bool,
    pub place_id: u64,
    pub job_id: String,
    pub universe_id: u64,
    pub game_name: String,
}

/// What the next poll should report as the host's session.
pub fn set_next(place_id: u64, job_id: &str, universe_id: u64, game_name: &str) {
    if let Ok(mut next) = NEXT.lock() {
        *next = Some((place_id, job_id.to_string(), universe_id, game_name.to_string()));
    }
}

pub fn party_presence(_user_id: u64, cookie: &str) -> Option<PartyPresence> {
    if cookie.is_empty() {
        return None;
    }
    let guard = NEXT.lock().ok()?;
    let (place_id, job_id, universe_id, game_name) = guard.clone()?;
    Some(PartyPresence {
        in_game: true,
        place_id,
        job_id,
        universe_id,
        game_name,
    })
}

/// Stand-in for the public server list; tests feed it with `set_pool`.
pub fn fetch_public_server_pool(
    _place_id: u64,
    _sort_order: &str,
    _pages: usize,
    _cookie: Option<&str>,
) -> Result<Vec<crate::types::ServerItem>, String> {
    Ok(POOL.lock().map(|pool| pool.clone()).unwrap_or_default())
}

static POOL: Mutex<Vec<crate::types::ServerItem>> = Mutex::new(Vec::new());

pub fn set_pool(items: Vec<crate::types::ServerItem>) {
    if let Ok(mut pool) = POOL.lock() {
        *pool = items;
    }
}
