//! The launcher's window lifecycle as the menu bar sees it.
//!
//! `src/window_lifecycle.rs` is the whole decision table, included here by path, so these
//! tests run where the GUI crate cannot be linked. They are the contract:
//!
//! * starting a game tucks the window into the menu bar (`hidden_to_tray = true`),
//! * a finished game never brings it back,
//! * "Open CREBLOX" restores it (Regular policy, window shown and focused),
//! * the red close button is a normal quit and never a tray hide.
//!
//! Every test takes `LOCK`: `apply` publishes the state to a process-wide flag that the macOS
//! tray thread reads, and cargo runs tests in parallel threads of one process.

use std::sync::Mutex;

use creblox_vm_tests::window_lifecycle::{Action, Event, Lifecycle, hidden_to_tray, set_hidden_to_tray};

static LOCK: Mutex<()> = Mutex::new(());

/// A fresh launcher with its window on screen. The published flag is process-wide, so it is
/// reset too: cargo runs these tests in parallel threads of one process.
fn lifecycle() -> Lifecycle {
    set_hidden_to_tray(false);
    Lifecycle::new()
}

#[test]
fn game_started_tucks_the_window_into_the_menu_bar() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut w = lifecycle();
    assert!(!w.hidden_to_tray(), "a fresh launcher has its window");
    assert!(
        !hidden_to_tray(),
        "a reopen request is honoured while the window is on screen"
    );

    assert_eq!(
        w.apply(Event::GameStarted),
        Some(Action::TuckIntoMenuBar),
        "a live session is what hides the window"
    );
    assert!(w.hidden_to_tray(), "the launcher now lives in the menu bar");
    assert!(
        hidden_to_tray(),
        "the tray thread and the Dock handler must see the same state"
    );
    assert!(
        hidden_to_tray(),
        "no Dock click or finished game may undo a game tuck"
    );

    // The watcher can report the same session twice; a second tuck is a no-op, not a second
    // window transition.
    assert_eq!(w.apply(Event::GameStarted), None);
    assert!(w.hidden_to_tray());
}

#[test]
fn game_ended_keeps_the_launcher_in_the_menu_bar() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut w = lifecycle();
    w.apply(Event::GameStarted);

    assert_eq!(
        w.apply(Event::GameEnded),
        None,
        "a finished game is not a request to reopen the window"
    );
    assert!(
        w.hidden_to_tray(),
        "the launcher stays tucked in until Open CREBLOX"
    );
    assert!(hidden_to_tray());

    // However many times the session end is reported (process list, player log, app closed),
    // the window stays where the player left it.
    assert_eq!(w.apply(Event::GameEnded), None);
    assert!(w.hidden_to_tray());
}

#[test]
fn open_launcher_restores_the_window() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut w = lifecycle();
    w.apply(Event::GameStarted);

    assert_eq!(
        w.apply(Event::OpenLauncher),
        Some(Action::ShowLauncher),
        "the menu item is the way back"
    );
    assert!(!w.hidden_to_tray(), "the Dock icon and window come back");
    assert!(!hidden_to_tray());
    assert!(!hidden_to_tray(), "and so do the Dock handler and the watcher");

    // Asking again just raises the window it already shows.
    assert_eq!(w.apply(Event::OpenLauncher), Some(Action::ShowLauncher));
    assert!(!w.hidden_to_tray());
}

#[test]
fn close_clicked_is_a_quit_and_never_a_tray_hide() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut w = lifecycle();
    assert_eq!(w.apply(Event::CloseClicked), Some(Action::Quit));
    assert!(!w.hidden_to_tray(), "the red button never hides the window");

    // Even mid-session the red button means quit; it cannot leave a hidden app behind.
    w.apply(Event::GameStarted);
    assert_eq!(w.apply(Event::CloseClicked), Some(Action::Quit));
    assert!(w.hidden_to_tray(), "quitting does not change the tucked state");
}

#[test]
fn a_second_game_tucks_again_after_the_window_came_back() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut w = lifecycle();
    assert_eq!(w.apply(Event::GameStarted), Some(Action::TuckIntoMenuBar));
    assert_eq!(w.apply(Event::GameEnded), None);
    assert_eq!(w.apply(Event::OpenLauncher), Some(Action::ShowLauncher));
    assert!(!w.hidden_to_tray());

    // The next join hides the window again, exactly like the first one.
    assert_eq!(w.apply(Event::GameStarted), Some(Action::TuckIntoMenuBar));
    assert!(w.hidden_to_tray());
}
