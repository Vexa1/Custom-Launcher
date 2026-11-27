//! When the launcher's window is on screen, and which events may change that.
//!
//! One rule set, in one place: starting a game tucks CREBLOX into the menu bar, a finished
//! game does **not** bring it back, and only "Open CREBLOX" (or a second launch, or a page
//! shortcut) restores the window. Closing the window is a normal quit.
//!
//! No egui, no AppKit, no threads: the transitions are pure, so `vm-tests` asserts them on a
//! host where the GUI crate cannot be linked. The process-wide flag at the bottom is what the
//! macOS tray thread and the Dock-reopen handler read; the app thread owns the [`Lifecycle`].

use std::sync::atomic::{AtomicBool, Ordering};

/// Something that happened which may change what the launcher window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A Roblox session began (a launch from CREBLOX, or the watcher seeing `Playing`).
    GameStarted,
    /// That session ended. Informational: it never reopens the window.
    GameEnded,
    /// "Open CREBLOX" in the tray menu, a page shortcut, or a second launch asking this
    /// instance to show itself.
    OpenLauncher,
    /// The window's red close button, or a normal quit.
    CloseClicked,
}

/// What the window should do in response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Hide the window and drop the Dock icon: CREBLOX lives in the menu bar from now on.
    TuckIntoMenuBar,
    /// Back to a Regular app: show, raise and focus the launcher window.
    ShowLauncher,
    /// Let the process exit like any other Mac app.
    Quit,
}

/// The launcher's window state, driven only by [`Event`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lifecycle {
    hidden_to_tray: bool,
}

impl Lifecycle {
    pub const fn new() -> Self {
        Self {
            hidden_to_tray: false,
        }
    }

    /// Whether CREBLOX is living in the menu bar with no window, on purpose.
    pub const fn hidden_to_tray(self) -> bool {
        self.hidden_to_tray
    }

    /// Applies `event` and reports what the window should do now.
    ///
    /// `GameEnded` is deliberately a no-op: the player hid the window by starting a game, and
    /// only the player undoes that. `CloseClicked` quits whether or not the window is hidden,
    /// and never turns into a tray hide.
    pub fn apply(&mut self, event: Event) -> Option<Action> {
        let action = match event {
            Event::GameStarted => {
                if self.hidden_to_tray {
                    None
                } else {
                    self.hidden_to_tray = true;
                    Some(Action::TuckIntoMenuBar)
                }
            }
            Event::GameEnded => None,
            Event::OpenLauncher => {
                self.hidden_to_tray = false;
                Some(Action::ShowLauncher)
            }
            Event::CloseClicked => Some(Action::Quit),
        };
        set_hidden_to_tray(self.hidden_to_tray);
        action
    }
}

/// The same flag, readable from any thread (the macOS tray watcher, the Dock handler).
static HIDDEN_TO_TRAY: AtomicBool = AtomicBool::new(false);

/// Whether the launcher was tucked into the menu bar on purpose.
///
/// While this is true no Dock click, app activation or finished game may restore the window:
/// "Open CREBLOX" is the only way back. The app thread publishes it through [`Lifecycle`].
pub fn hidden_to_tray() -> bool {
    HIDDEN_TO_TRAY.load(Ordering::Relaxed)
}

/// Publishes the state for the threads that cannot borrow the app.
pub fn set_hidden_to_tray(hidden: bool) {
    HIDDEN_TO_TRAY.store(hidden, Ordering::Relaxed);
}

