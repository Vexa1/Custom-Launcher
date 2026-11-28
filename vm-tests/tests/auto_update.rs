//! The 30-second automatic-update loop: cadence, the gameplay guard, failed-tag protection,
//! and the fact that the switch survives the restart the loop itself performs.
//!
//! `src/auto_update.rs` is pure, so these cases drive the real scheduler the launcher uses.

use std::time::{Duration, Instant};

use creblox_vm_tests::auto_update::{AutoUpdate, CHECK_INTERVAL, Decision};
use creblox_vm_tests::settings::Settings;
use creblox_vm_tests::settings_json::{load_settings_from, save_settings_to};

#[test]
fn disabled_never_checks_however_long_the_app_runs() {
    let now = Instant::now();
    let mut updater = AutoUpdate::new(false);
    assert!(!updater.due(now));
    assert!(!updater.due(now + Duration::from_secs(24 * 60 * 60)));
    assert!(updater.time_until_due(now).is_none());

    // Turning the switch on arms an immediate check instead of waiting out a window that
    // never started.
    updater.set_enabled(true);
    assert!(updater.due(now));
}

#[test]
fn a_check_runs_at_thirty_second_intervals() {
    let now = Instant::now();
    let mut updater = AutoUpdate::new(true);
    assert!(updater.due(now), "the first frame checks at once");
    assert_eq!(updater.time_until_due(now), Some(Duration::ZERO));

    updater.begin_check(now);
    assert!(!updater.due(now + Duration::from_secs(1)));
    updater.finish_check();
    assert_eq!(
        updater.time_until_due(now + Duration::from_secs(20)),
        Some(Duration::from_secs(10))
    );
    assert!(!updater.due(now + CHECK_INTERVAL - Duration::from_millis(1)));
    assert!(updater.due(now + CHECK_INTERVAL), "the 30-second tick is due");

    // A check still running never stacks a second one, however long its answer takes.
    updater.begin_check(now + CHECK_INTERVAL);
    assert!(!updater.due(now + CHECK_INTERVAL + Duration::from_secs(90)));
}

#[test]
fn a_release_found_during_a_match_waits_for_the_lobby() {
    let mut updater = AutoUpdate::new(true);
    assert_eq!(
        updater.on_release("v0.5.71", true),
        Decision::Defer,
        "installing mid-match would interrupt gameplay"
    );
    // The same release seen again while still playing stays deferred, not skipped.
    assert_eq!(updater.on_release("v0.5.71", true), Decision::Defer);
    // Back in the lobby the remembered tag is what triggers the install.
    assert_eq!(updater.take_deferred().as_deref(), Some("v0.5.71"));
    assert_eq!(updater.take_deferred(), None, "taken exactly once");

    assert_eq!(updater.on_release("v0.5.72", false), Decision::Install);
}

#[test]
fn a_download_that_finishes_in_game_also_defers_the_install() {
    let mut updater = AutoUpdate::new(true);
    updater.defer("v0.5.71");
    assert_eq!(updater.take_deferred().as_deref(), Some("v0.5.71"));
}

#[test]
fn a_failed_tag_is_not_retried_all_session_but_a_newer_one_is() {
    let mut updater = AutoUpdate::new(true);
    updater.on_failure("v0.5.71");

    // Every 30-second tick sees the same latest tag: skip it, no rebuild loop.
    for _ in 0..4 {
        assert_eq!(updater.on_release("v0.5.71", false), Decision::Skip);
    }
    // v0.5.71+1 is a different tag, so the loop tries it automatically.
    assert_eq!(updater.on_release("v0.5.72", false), Decision::Install);

    // And the card's Install & Restart re-arms the failed tag at once.
    updater.on_failure("v0.5.72");
    assert_eq!(updater.on_release("v0.5.72", false), Decision::Skip);
    updater.retry();
    assert_eq!(updater.on_release("v0.5.72", false), Decision::Install);
}

#[test]
fn the_switch_survives_the_restart_it_performs() {
    let dir = std::env::temp_dir().join(format!(
        "creblox-auto-update-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp settings directory");

    let mut settings = Settings::default();
    settings.auto_update = true;
    assert_eq!(
        save_settings_to(&dir, &settings),
        creblox_vm_tests::settings_json::SaveOutcome::Written
    );

    // A fresh launch reads settings.json and the loop is armed without another click.
    let loaded = load_settings_from(&dir);
    assert!(loaded.auto_update, "auto_update must persist in settings.json");
    let updater = AutoUpdate::new(loaded.auto_update);
    assert!(updater.due(Instant::now()));

    settings.auto_update = false;
    save_settings_to(&dir, &settings);
    assert!(!load_settings_from(&dir).auto_update);

    let _ = std::fs::remove_dir_all(&dir);
}
