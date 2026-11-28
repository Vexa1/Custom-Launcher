//! What a macOS memory-pressure report should do to the launcher's caches.
//!
//! macOS posts a dispatch source when the system is short of memory. That callback may not
//! flush anything itself - a flush drops textures, and egui must not see its texture list
//! change mid-frame - so it only records a level and wakes egui; a later frame asks
//! [`action`] what to do. The decision is pure arithmetic, so `vm-tests` covers it on a host
//! with no libdispatch, and the platform piece lives in `memory_pressure_macos.rs`.

use std::time::{Duration, Instant};

/// How long one flush is trusted before the same level may flush again.
pub const DEBOUNCE: Duration = Duration::from_secs(30);

/// What macOS last reported.
///
/// Ordered so an escalation (`Warn` -> `Critical`) is visibly greater than what was already
/// acted on.
// `Warn` and `Critical` only ever come from the macOS dispatch source, so on a target without
// it the compiler would call them unconstructed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    #[default]
    None = 0,
    Warn = 1,
    Critical = 2,
}

/// What the frame should do about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    #[default]
    None,
    /// Drop every rebuildable image cache: the same path a tray tuck uses.
    Flush,
    /// Drop them, hand the pages back to the OS and re-check the budget.
    DeepFlush,
}

/// What a reported level should do now.
///
/// A hidden launcher has already been flushed by `tuck_into_menu_bar`, so pressure while it
/// lives in the menu bar asks for nothing; the pending level stays queued and is considered
/// again the next time the window is shown. The debounce keeps a level that keeps flipping
/// from thrashing the texture pool, but an escalation is never swallowed by it: a `Critical`
/// that follows a `Warn` inside the window still gets its deeper trim.
pub fn action(
    level: Level,
    last: Option<(Instant, Level)>,
    hidden: bool,
    now: Instant,
) -> Action {
    if hidden || level == Level::None {
        return Action::None;
    }
    if let Some((at, acted)) = last {
        let escalated = level > acted;
        let waited = now.saturating_duration_since(at) >= DEBOUNCE;
        if !escalated && !waited {
            return Action::None;
        }
    }
    match level {
        Level::Critical => Action::DeepFlush,
        Level::Warn => Action::Flush,
        Level::None => Action::None,
    }
}
