//! Stale-while-revalidate for the game lists.
//!
//! Home, Charts and Game Details all re-fetch their data on a timer. The rule this module
//! encodes is that a refresh is *silent*: whatever is already on screen stays on screen, a
//! skeleton is only for the first load of an empty page, and a refresh that comes back empty or
//! fails leaves the visible list exactly as it was. The incoming data is merged by
//! `universe_id`, so a card keeps its id - and therefore its cached texture - and only its
//! numbers change. Home rows stay at [`HOME_ROW_CAP`] cards however often they refresh, and a
//! chart sort follows the fresh ranking rather than stale positions.

use crate::types::Game;
use std::collections::{HashMap, HashSet};

/// How many cards a Home row (Continue, Favourites) is laid out for.
pub const HOME_ROW_CAP: usize = 24;

/// What a list is doing right now, as far as drawing is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing cached yet and a fetch is running: the only time a skeleton is honest.
    FirstLoad,
    /// Data is on screen and a fetch is running behind it: draw the data, maybe a hint.
    Silent,
    /// Data is on screen and nothing is running.
    Ready,
    /// Nothing has ever loaded and the last attempt failed.
    Failed,
}

/// Tracks one list's refresh state. Cheap to copy, so a page can read it into a local.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Freshness {
    refreshing: bool,
    ready: bool,
    failed: bool,
}

impl Freshness {
    /// A fetch just started. Whatever is drawn stays drawn.
    pub fn request(&mut self) {
        self.refreshing = true;
        self.failed = false;
    }

    /// A fetch answered. From now on a later failure cannot take the page back to a skeleton.
    pub fn succeed(&mut self) {
        self.refreshing = false;
        self.ready = true;
        self.failed = false;
    }

    /// A fetch failed. That only matters when there is nothing to show.
    pub fn fail(&mut self) {
        self.refreshing = false;
        self.failed = !self.ready;
    }

    /// Forgot everything, for a logout or an account switch.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn phase(&self) -> Phase {
        if self.refreshing {
            if self.ready {
                Phase::Silent
            } else {
                Phase::FirstLoad
            }
        } else if self.failed {
            Phase::Failed
        } else if self.ready {
            Phase::Ready
        } else {
            Phase::FirstLoad
        }
    }

    /// Whether to draw a skeleton/spinner for a list of `visible` items. Never over real cards.
    pub fn skeleton(&self, visible: usize) -> bool {
        visible == 0 && self.phase() == Phase::FirstLoad
    }
}

/// What a merge changed, for tests and for logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MergeStats {
    pub updated: usize,
    pub added: usize,
    pub kept: usize,
}

/// Merges `fresh` into `old` in place, by `universe_id`.
///
/// Games already on screen keep their position and take the fresh numbers (player counts,
/// likes, formatted labels, name); ids that are new are appended. Nothing is removed here, so a
/// narrower remote list never makes cards disappear, and no card changes id - which is what
/// keeps its thumbnail texture. The Home rows are capped afterwards by [`apply_refresh`].
pub fn merge_games(old: &mut Vec<Game>, fresh: Vec<Game>) -> MergeStats {
    let mut stats = MergeStats::default();
    let mut by_id: HashMap<u64, Game> = fresh
        .iter()
        .map(|g| (g.universe_id, g.clone()))
        .collect();
    for game in old.iter_mut() {
        if let Some(fresh_game) = by_id.remove(&game.universe_id) {
            *game = fresh_game;
            stats.updated += 1;
        }
    }
    for game in fresh {
        if by_id.remove(&game.universe_id).is_some() {
            old.push(game);
            stats.added += 1;
        }
    }
    stats.kept = old.len();
    stats
}

/// Applies a refresh result to a Home row.
///
/// `None` (the fetch failed) and `Some(empty)` (it answered with nothing) both leave the list
/// exactly as it was: stale is better than a blank row that flickers back in a second later.
/// A merge only ever appends, so the row is then trimmed back to [`HOME_ROW_CAP`] - otherwise a
/// long-lived session would grow it by a card on every refresh.
pub fn apply_refresh(old: &mut Vec<Game>, fresh: Option<Vec<Game>>) -> MergeStats {
    let mut stats = MergeStats {
        updated: 0,
        added: 0,
        kept: old.len(),
    };
    if let Some(fresh) = fresh.filter(|fresh| !fresh.is_empty()) {
        let listed: HashSet<u64> = fresh.iter().map(|g| g.universe_id).collect();
        stats = merge_games(old, fresh);
        trim_to_row_cap(old, &listed);
        stats.kept = old.len();
    }
    stats
}

/// Trims an over-full row back to [`HOME_ROW_CAP`], dropping cards the fresh answer no longer
/// lists before it drops any it does - from the tail each time, so the head of the row is the
/// part that stays put. A just-favourited or just-launched game is in the answer, so it is the
/// stale card behind it that falls off, not the game the user is looking at.
fn trim_to_row_cap(games: &mut Vec<Game>, listed: &HashSet<u64>) {
    while games.len() > HOME_ROW_CAP {
        let victim = games
            .iter()
            .rposition(|g| !listed.contains(&g.universe_id))
            .unwrap_or(games.len() - 1);
        games.remove(victim);
    }
}

/// Updates the numbers of cards already on screen, matching by `universe_id`, without moving
/// any of them and without appending newcomers.
///
/// This is the ranked-grid path: the grid has to follow the fresh numbers, but re-sorting it
/// would drag cards out from under the pointer. A card that is still in the catalogue takes the
/// fresh game at the position it already holds; new games and a genuinely new ranking arrive
/// through `merge_sorts` and the next pool rebuild.
pub fn patch_games(old: &mut [Game], fresh: &[Game]) -> MergeStats {
    let mut stats = MergeStats {
        updated: 0,
        added: 0,
        kept: old.len(),
    };
    let mut by_id: HashMap<u64, &Game> = fresh.iter().map(|g| (g.universe_id, g)).collect();
    for game in old.iter_mut() {
        if let Some(fresh_game) = by_id.remove(&game.universe_id) {
            *game = fresh_game.clone();
            stats.updated += 1;
        }
    }
    stats
}

/// Updates the numbers of the cards already shown in each sort, keeping every position.
///
/// A background revalidation must not re-rank rows under the pointer: sorts are matched by
/// title, games by `universe_id`, and each shown card takes the fresh numbers where it already
/// sits. Newcomers and a genuinely new ranking arrive through [`merge_sorts`], which an
/// explicit refresh (or a first load) still uses.
pub fn patch_sorts(old: &mut [crate::types::Sort], fresh: &[crate::types::Sort]) -> MergeStats {
    let mut stats = MergeStats::default();
    for sort in old.iter_mut() {
        if let Some(new) = fresh.iter().find(|shown| shown.title == sort.title) {
            stats.updated += patch_games(&mut sort.games, &new.games).updated;
        }
        stats.kept += sort.games.len();
    }
    stats
}

/// Merges a fresh page of sorts into the ones on screen, matching sorts by title.
///
/// A sort is a *ranking*, so a sort the answer contains takes the fresh games in the **fresh
/// order**: the row shows the new positions instead of where those cards happened to sit
/// before, and a game that dropped out of the top of the chart leaves it. Textures do not care -
/// they are keyed by `universe_id`, which a reorder never changes. Sorts themselves keep their
/// place on the page, a sort the app has never shown is appended, and a sort missing from the
/// answer keeps its cards.
pub fn merge_sorts(old: &mut Vec<crate::types::Sort>, fresh: Vec<crate::types::Sort>) -> MergeStats {
    let mut stats = MergeStats::default();
    let mut added_sorts: Vec<crate::types::Sort> = Vec::new();
    for mut sort in fresh {
        if let Some(existing) = old.iter_mut().find(|shown| shown.title == sort.title) {
            // A repeated id inside one answer is one card, not two rows at the same rank.
            let mut seen: HashSet<u64> = HashSet::with_capacity(sort.games.len());
            sort.games.retain(|g| seen.insert(g.universe_id));
            let shown: HashSet<u64> = existing.games.iter().map(|g| g.universe_id).collect();
            for game in &sort.games {
                if shown.contains(&game.universe_id) {
                    stats.updated += 1;
                } else {
                    stats.added += 1;
                }
            }
            existing.games = sort.games;
        } else {
            stats.added += sort.games.len();
            added_sorts.push(sort);
        }
    }
    old.extend(added_sorts);
    stats.kept = old.len();
    stats
}
