//! The level-to-action decision for macOS memory pressure.
//!
//! The dispatch source only records a level; this is the pure part that decides whether a
//! frame should flush the image caches (the tray-tuck path), deep-trim and hand pages back, or
//! do nothing. Covered here so the reaction is testable without libdispatch or a Mac.

use std::time::{Duration, Instant};

use creblox_vm_tests::memory_pressure::{action, Action, Level, DEBOUNCE};

#[test]
fn a_warning_flushes_and_a_critical_deep_trims() {
    let now = Instant::now();
    assert_eq!(action(Level::Warn, None, false, now), Action::Flush);
    assert_eq!(action(Level::Critical, None, false, now), Action::DeepFlush);
    assert_eq!(action(Level::None, None, false, now), Action::None);
}

#[test]
fn a_hidden_launcher_does_nothing() {
    // `tuck_into_menu_bar` already ran the flush, so pressure in the menu bar has nothing to
    // free and must not wake a window just to trim it again.
    let now = Instant::now();
    assert_eq!(action(Level::Warn, None, true, now), Action::None);
    assert_eq!(action(Level::Critical, None, true, now), Action::None);
}

#[test]
fn the_same_level_is_debounced() {
    let now = Instant::now();
    let just_flushed = Some((now - Duration::from_secs(5), Level::Warn));
    assert_eq!(action(Level::Warn, just_flushed, false, now), Action::None);

    let waited = Some((now - DEBOUNCE, Level::Warn));
    assert_eq!(action(Level::Warn, waited, false, now), Action::Flush);
}

#[test]
fn an_escalation_is_never_swallowed_by_the_debounce() {
    let now = Instant::now();
    // Warned and flushed one second ago; the system is now critical. The deeper trim must
    // still run, otherwise the one moment that matters most is the one that is ignored.
    let just_warned = Some((now - Duration::from_secs(1), Level::Warn));
    assert_eq!(
        action(Level::Critical, just_warned, false, now),
        Action::DeepFlush
    );

    // The reverse is not an escalation: critical just handled, a plain warn waits.
    let just_critical = Some((now - Duration::from_secs(1), Level::Critical));
    assert_eq!(action(Level::Warn, just_critical, false, now), Action::None);
    // ...and it is due again once the window has passed.
    let old_critical = Some((now - DEBOUNCE - Duration::from_secs(1), Level::Critical));
    assert_eq!(action(Level::Warn, old_critical, false, now), Action::Flush);
}

#[test]
fn a_repeated_critical_waits_out_the_debounce() {
    let now = Instant::now();
    let just_deep = Some((now - Duration::from_secs(2), Level::Critical));
    assert_eq!(action(Level::Critical, just_deep, false, now), Action::None);
    let waited = Some((now - DEBOUNCE, Level::Critical));
    assert_eq!(
        action(Level::Critical, waited, false, now),
        Action::DeepFlush
    );
}
