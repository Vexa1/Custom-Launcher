//! Server Hop: move to a different public server of the same experience without leaving
//! the game by hand.
//!
//! The tray offers three modes. Each one lists public servers for the place that is being
//! played, drops the ones that cannot be joined (the current server, full servers, servers
//! seen in the last few minutes), and ranks what is left. Only once a target has been
//! chosen is the running client closed and a fresh `RequestGameJob` launch made, so a
//! failed lookup never costs the player the game they are in.
//!
//! The session context is published by the log watcher (`roblox_process::start_watcher`)
//! and cleared when the session ends; the tray reads [`in_game`] to decide whether to
//! enable the submenu at all.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::types::ServerItem;

/// A server has to be left alone for this long after a hop, so that two quick hops cannot
/// bounce between the same pair of servers.
const COOLDOWN: Duration = Duration::from_secs(180);
/// How long the launcher stays out of the way after a hop starts. A hop is a kill and a
/// relaunch; the old game ends a second before the new one joins, and without this window
/// the "you left the game" path would raise the launcher over the game being joined.
const HOP_WINDOW_SECS: u64 = 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerHopMode {
    /// Any joinable server; no preference beyond "not the one I am on".
    Random,
    /// Fewest players first, then lowest ping.
    LeastPlayers,
    /// Lowest reported ping first; falls back to [`ServerHopMode::Random`] when Roblox
    /// publishes no ping for the experience.
    LowestPing,
}

impl ServerHopMode {
    pub fn label(self) -> &'static str {
        match self {
            ServerHopMode::Random => "Quick Hop",
            ServerHopMode::LeastPlayers => "Least Players",
            ServerHopMode::LowestPing => "Lowest Ping",
        }
    }

    /// `sortOrder` for `games.roblox.com/.../servers/Public`.
    ///
    /// Roblox sorts by population, so `Asc` already hands back the emptiest servers first.
    /// The ping modes ask for `Desc` because a busy page is the only place ping telemetry
    /// is published at all.
    pub fn sort_order(self) -> &'static str {
        match self {
            ServerHopMode::LeastPlayers => "Asc",
            ServerHopMode::Random | ServerHopMode::LowestPing => "Desc",
        }
    }

    /// Pages to fetch: one ascending page is enough to find the emptiest server, while the
    /// ping and random modes want a wide pool to choose from.
    pub fn pages(self) -> usize {
        match self {
            ServerHopMode::LeastPlayers => 1,
            ServerHopMode::Random | ServerHopMode::LowestPing => 3,
        }
    }
}

/// What the watcher knows about the session being played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HopContext {
    pub place_id: u64,
    pub job_id: String,
    /// Universe id when the player log carries one; `0` when it does not (the hop itself
    /// only needs the place, so this never blocks it).
    pub universe_id: u64,
}

fn context_slot() -> &'static Mutex<Option<HopContext>> {
    static CONTEXT: OnceLock<Mutex<Option<HopContext>>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new(None))
}

pub fn context() -> Option<HopContext> {
    context_slot().lock().ok().and_then(|g| g.clone())
}

/// Replace the session context, returning whether it actually changed (so the caller only
/// repaints on a real transition).
pub fn set_context(next: Option<HopContext>) -> bool {
    let Ok(mut guard) = context_slot().lock() else {
        return false;
    };
    if *guard == next {
        return false;
    }
    *guard = next;
    true
}

/// True while a game is being played on a known place and job - the only state in which a
/// hop makes sense. Idle, sitting in the Roblox app, or "server not parsed yet" is false.
pub fn in_game() -> bool {
    context().is_some_and(|c| c.place_id > 0 && !c.job_id.is_empty())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn hop_flag_path() -> PathBuf {
    crate::auth::data_dir().join("hop.flag")
}

/// Mark the launcher as "do not come back yet". Written to the data folder rather than kept
/// in memory because the `--companion` helper is a *separate process* and it is the one that
/// reopens the launcher when a game ends.
pub fn begin_hop_window() {
    let until = now_ms().saturating_add(HOP_WINDOW_SECS * 1000);
    if let Some(dir) = hop_flag_path().parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(hop_flag_path(), until.to_string());
}

pub fn end_hop_window() {
    let _ = fs::remove_file(hop_flag_path());
}

/// Whether a hop is in flight (or just finished) and the launcher should stay hidden.
/// Expires by itself, so a crashed hop cannot leave the launcher unreachable.
pub fn hop_in_progress() -> bool {
    let Ok(text) = fs::read_to_string(hop_flag_path()) else {
        return false;
    };
    match text.trim().parse::<u64>() {
        Ok(until) if now_ms() < until => true,
        _ => {
            end_hop_window();
            false
        }
    }
}

fn cooldown_slot() -> &'static Mutex<Vec<(String, SystemTime)>> {
    static SEEN: OnceLock<Mutex<Vec<(String, SystemTime)>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(Vec::new()))
}

fn note_hop(job_id: &str) {
    if job_id.trim().is_empty() {
        return;
    }
    let Ok(mut guard) = cooldown_slot().lock() else {
        return;
    };
    let now = SystemTime::now();
    guard.retain(|(_, at)| now.duration_since(*at).unwrap_or_default() < COOLDOWN);
    guard.push((job_id.trim().to_string(), now));
    // Bounded: a long session can hop many times, and the list is only ever scanned.
    if guard.len() > 32 {
        let drop = guard.len() - 32;
        guard.drain(0..drop);
    }
}

fn cooling_snapshot() -> Vec<String> {
    let Ok(mut guard) = cooldown_slot().lock() else {
        return Vec::new();
    };
    let now = SystemTime::now();
    guard.retain(|(_, at)| now.duration_since(*at).unwrap_or_default() < COOLDOWN);
    guard.iter().map(|(id, _)| id.clone()).collect()
}

fn random_pick(pool: &[ServerItem]) -> Option<ServerItem> {
    if pool.is_empty() {
        return None;
    }
    // No RNG dependency for one number: the nanosecond clock is enough to spread picks.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    pool.get(nanos % pool.len()).cloned()
}

/// Filter and rank the API's server list. Pure, so the rules can be exercised without a
/// session: never the current server, never a full or empty one, never one recently left.
pub fn rank(
    mode: ServerHopMode,
    candidates: &[ServerItem],
    current_job: &str,
    cooling: &[String],
) -> Option<ServerItem> {
    let pool: Vec<ServerItem> = candidates
        .iter()
        .filter(|s| {
            !s.id.is_empty()
                && s.id != current_job
                && !cooling.iter().any(|c| c == &s.id)
                && s.playing > 0
                && s.max_players > 0
                && s.playing < s.max_players
        })
        .cloned()
        .collect();
    if pool.is_empty() {
        return None;
    }
    match mode {
        ServerHopMode::LeastPlayers => {
            let mut sorted = pool;
            sorted.sort_by_key(|s| (s.playing, s.ping.unwrap_or(u64::MAX)));
            sorted.into_iter().next()
        }
        ServerHopMode::LowestPing => {
            let mut with_ping: Vec<ServerItem> = pool
                .iter()
                .filter(|s| s.ping.is_some_and(|p| p > 0))
                .cloned()
                .collect();
            if with_ping.is_empty() {
                // Roblox omits `ping` for some experiences; a hop should still work.
                return random_pick(&pool);
            }
            with_ping.sort_by_key(|s| (s.ping.unwrap_or(u64::MAX), s.playing));
            with_ping.into_iter().next()
        }
        ServerHopMode::Random => random_pick(&pool),
    }
}

/// Find a target, then (and only then) close the running client and join it.
///
/// Runs on a worker thread: it makes two network round trips, kills a process and waits for
/// it to go away. Returns the line to show the user.
pub fn run_hop(mode: ServerHopMode, cookie: &str) -> Result<String, String> {
    let ctx = context().ok_or_else(|| "Server Hop needs an active game session".to_string())?;
    if ctx.place_id == 0 || ctx.job_id.is_empty() {
        return Err("The current server is not known yet - try again in a moment".to_string());
    }

    // 1. Pre-flight. Nothing is closed while this can still fail.
    let candidates = crate::api::fetch_public_server_pool(
        ctx.place_id,
        mode.sort_order(),
        mode.pages(),
        Some(cookie),
    )
    .map_err(|e| format!("Could not list public servers: {e}"))?;
    let cooling = cooling_snapshot();
    let target = rank(mode, &candidates, &ctx.job_id, &cooling).ok_or_else(|| {
        format!(
            "No other joinable public server right now ({} listed)",
            candidates.len()
        )
    })?;

    crate::auth::log(&format!(
        "hop: {} place {} -> {} ({} candidates, {} of {} players)",
        mode.label(),
        ctx.place_id,
        target.id,
        candidates.len(),
        target.playing,
        target.max_players
    ));

    // 2. Keep the launcher out of the way, close the old client, join the new server.
    begin_hop_window();
    let closed = crate::roblox_process::kill_all_roblox_processes();
    std::thread::sleep(Duration::from_millis(700));
    // Both ends go on cooldown: the server just left must not be a target of the next hop.
    note_hop(&ctx.job_id);
    note_hop(&target.id);

    match crate::roblox_process::launch_authenticated_server(ctx.place_id, Some(&target.id), cookie)
    {
        Ok(message) => Ok(format!(
            "{}: {} ({} of {} players)",
            mode.label(),
            message,
            target.playing,
            target.max_players
        )),
        Err(e) => {
            // Nothing is running any more, so let the launcher come back.
            end_hop_window();
            Err(format!(
                "Could not join the new server: {e} (closed {closed} Roblox process(es))"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str, playing: u64, max_players: u64, ping: Option<u64>) -> ServerItem {
        ServerItem {
            id: id.to_string(),
            playing,
            max_players,
            ping,
            fps: None,
        }
    }

    #[test]
    fn current_server_is_never_chosen() {
        let servers = vec![server("current", 5, 20, Some(10))];
        assert!(rank(ServerHopMode::Random, &servers, "current", &[]).is_none());
    }

    #[test]
    fn full_and_empty_servers_are_dropped() {
        let servers = vec![
            server("full", 20, 20, Some(5)),
            server("empty", 0, 20, Some(5)),
            server("ok", 3, 20, Some(50)),
        ];
        let picked = rank(ServerHopMode::LeastPlayers, &servers, "current", &[]).unwrap();
        assert_eq!(picked.id, "ok");
    }

    #[test]
    fn cooldown_servers_are_skipped() {
        let servers = vec![server("a", 2, 10, Some(5)), server("b", 4, 10, Some(9))];
        let picked = rank(
            ServerHopMode::LeastPlayers,
            &servers,
            "current",
            &["a".to_string()],
        )
        .unwrap();
        assert_eq!(picked.id, "b");
    }

    #[test]
    fn least_players_prefers_fewest_then_lowest_ping() {
        let servers = vec![
            server("busy", 15, 20, Some(5)),
            server("quiet-high-ping", 2, 20, Some(90)),
            server("quiet-low-ping", 2, 20, Some(30)),
        ];
        let picked = rank(ServerHopMode::LeastPlayers, &servers, "current", &[]).unwrap();
        assert_eq!(picked.id, "quiet-low-ping");
    }

    #[test]
    fn lowest_ping_ignores_missing_values() {
        let servers = vec![
            server("no-ping", 1, 20, None),
            server("fast", 9, 20, Some(12)),
            server("slow", 2, 20, Some(80)),
        ];
        let picked = rank(ServerHopMode::LowestPing, &servers, "current", &[]).unwrap();
        assert_eq!(picked.id, "fast");
    }

    #[test]
    fn lowest_ping_falls_back_to_random_when_no_ping_is_published() {
        let servers = vec![server("a", 1, 20, None), server("b", 2, 20, None)];
        let picked = rank(ServerHopMode::LowestPing, &servers, "current", &[]).unwrap();
        assert!(picked.id == "a" || picked.id == "b");
    }

    #[test]
    fn every_candidate_filtered_out_means_no_target() {
        let servers = vec![
            server("current", 5, 20, Some(1)),
            server("full", 20, 20, Some(1)),
        ];
        assert!(rank(ServerHopMode::LowestPing, &servers, "current", &[]).is_none());
    }

    #[test]
    fn context_requires_a_place_and_a_job() {
        let _ = set_context(None);
        assert!(!in_game());
        assert!(set_context(Some(HopContext {
            place_id: 0,
            job_id: "j".to_string(),
            universe_id: 1,
        })));
        assert!(!in_game());
        assert!(set_context(Some(HopContext {
            place_id: 1818,
            job_id: "j".to_string(),
            universe_id: 1,
        })));
        assert!(in_game());
        // Same value again is not a change.
        assert!(!set_context(Some(HopContext {
            place_id: 1818,
            job_id: "j".to_string(),
            universe_id: 1,
        })));
        assert!(set_context(None));
        assert!(!in_game());
    }
}
