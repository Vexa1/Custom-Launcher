//! Stale-while-revalidate for the game lists.
//!
//! Home, Charts and Game Details all re-fetch their data on a timer. The rule this module
//! encodes is that a refresh is *silent*: whatever is already on screen stays on screen, a
//! skeleton is only for the first load of an empty page, and a refresh that comes back empty or
//! fails leaves the visible list exactly as it was. The incoming data is merged by
//! `universe_id`, so a card keeps its id - and therefore its cached texture - and only its
//! numbers change.

use crate::types::Game;
use std::collections::HashMap;

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

    /// A refresh is running behind data that is already on screen.
    pub fn is_silent(&self) -> bool {
        self.phase() == Phase::Silent
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
/// likes, formatted labels, name); ids that are new are appended. Nothing is removed, so a
/// narrower remote list never makes cards disappear, and no card changes id - which is what
/// keeps its thumbnail texture.
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

/// Applies a refresh result to a visible list.
///
/// `None` (the fetch failed) and `Some(empty)` (it answered with nothing) both leave the list
/// exactly as it was: stale is better than a blank row that flickers back in a second later.
pub fn apply_refresh(old: &mut Vec<Game>, fresh: Option<Vec<Game>>) -> MergeStats {
    match fresh {
        Some(fresh) if !fresh.is_empty() => merge_games(old, fresh),
        _ => MergeStats {
            updated: 0,
            added: 0,
            kept: old.len(),
        },
    }
}

/// Merges a fresh page of sorts into the ones on screen, matching sorts by title and games by
/// `universe_id`. Rows keep their position (nothing jumps while it is being read), a sort the
/// app has never shown is appended, and a sort missing from the answer keeps its cards.
pub fn merge_sorts(old: &mut Vec<crate::types::Sort>, fresh: Vec<crate::types::Sort>) -> MergeStats {
    let mut stats = MergeStats::default();
    let mut added_sorts: Vec<crate::types::Sort> = Vec::new();
    for sort in fresh {
        if let Some(existing) = old.iter_mut().find(|shown| shown.title == sort.title) {
            let merged = merge_games(&mut existing.games, sort.games);
            stats.updated += merged.updated;
            stats.added += merged.added;
        } else {
            stats.added += sort.games.len();
            added_sorts.push(sort);
        }
    }
    old.extend(added_sorts);
    stats.kept = old.len();
    stats
}
