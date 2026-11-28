//! When an automatic background refresh may run.
//!
//! Settings has one switch for all of it: with *Automatic background refresh* off, nothing
//! periodic may re-fetch game lists, player counts or friends activity - not because each timer
//! remembers to check, but because every periodic check goes through [`should_refresh`] first.
//! Pure and free of egui so `vm-tests` runs this exact file.

use std::time::{Duration, Instant};

/// How long one fetch may run before another of the same kind may start.
pub const SYNC_ACTIVE: Duration = Duration::from_secs(30);

/// Whether a periodic list/statistics refresh may start now.
///
/// `enabled` is the Settings switch: when it is off this always answers `false`, however long
/// ago the last answer was. A fetch that is still running blocks another one, and a recent
/// answer waits out its interval.
pub fn should_refresh(
    enabled: bool,
    last: Option<Instant>,
    started: Option<Instant>,
    interval: Duration,
    now: Instant,
) -> bool {
    if !enabled {
        return false;
    }
    if started.is_some_and(|t| now.saturating_duration_since(t) < SYNC_ACTIVE) {
        return false;
    }
    last.map_or(true, |t| now.saturating_duration_since(t) >= interval)
}
