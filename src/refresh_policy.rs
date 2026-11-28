//! When an automatic background refresh may run.
//!
//! Settings has one switch for all of it: with *Automatic background refresh* off, nothing
//! periodic may re-fetch game lists, player counts or friends activity - not because each timer
//! remembers to check, but because every periodic check goes through [`should_refresh`] first.
//! Pure and free of egui so `vm-tests` runs this exact file.

use std::time::{Duration, Instant};

/// How long one fetch may run before another of the same kind may start.
pub const SYNC_ACTIVE: Duration = Duration::from_secs(30);

/// How old a thumbnail that is already on screen may be before it is fetched again.
///
/// The pages ask for the pictures of whatever is visible on every frame, so "fetch it again"
/// has to be a decision, not a reflex: 120 s keeps counts-and-art fresh while automatic refresh
/// is on, and `Duration::MAX` means a picture that exists is never re-downloaded while it is
/// off. A missing picture is still fetched either way - drawing needs it.
pub fn thumb_refresh_age(auto_refresh: bool) -> Duration {
    if auto_refresh {
        Duration::from_secs(120)
    } else {
        Duration::MAX
    }
}

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
