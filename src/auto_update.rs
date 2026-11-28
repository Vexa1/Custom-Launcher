//! The automatic-update scheduler: how often to check, when a match defers an install, and
//! which tag already failed.
//!
//! Kept free of egui, HTTP and settings I/O so `vm-tests` can drive the real rules headlessly.
//! `app.rs` owns one of these while the launcher runs; `Settings::auto_update` is what turns it
//! on, and because that flag lives in `settings.json` it survives the restart into the build
//! the loop just installed.

use std::time::{Duration, Instant};

/// How often the latest GitHub release is checked while automatic updates are on.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// What a freshly detected release should make the launcher do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Download and install now (the player is not in a match).
    Install,
    /// Remember the tag and install as soon as the player is back in the lobby.
    Defer,
    /// This exact tag already failed this session; do not loop on it.
    Skip,
}

/// The automatic-update state that is not in `settings.json`.
pub struct AutoUpdate {
    enabled: bool,
    last_check: Option<Instant>,
    check_running: bool,
    failed_tag: Option<String>,
    deferred: Option<String>,
}

impl AutoUpdate {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_check: None,
            check_running: false,
            failed_tag: None,
            deferred: None,
        }
    }

    /// Follows the settings switch. Turning it on checks immediately instead of waiting out
    /// the remainder of a 30-second window that never started.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.check_running = false;
        self.last_check = None;
        if !enabled {
            self.deferred = None;
        }
    }

    /// True when a check may start now: enabled, not already checking, and 30 seconds have
    /// passed since the last one (or no check has run yet).
    pub fn due(&self, now: Instant) -> bool {
        if !self.enabled || self.check_running {
            return false;
        }
        match self.last_check {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= CHECK_INTERVAL,
        }
    }

    /// How long until the next check is due, for the frame timer. `None` means no check is
    /// armed (off, or one is already running).
    pub fn time_until_due(&self, now: Instant) -> Option<Duration> {
        if !self.enabled || self.check_running {
            return None;
        }
        Some(match self.last_check {
            None => Duration::ZERO,
            Some(last) => CHECK_INTERVAL.saturating_sub(now.saturating_duration_since(last)),
        })
    }

    /// A check left the launcher; the 30 seconds start here, not when the answer arrives, so a
    /// slow request cannot stack another one on top.
    pub fn begin_check(&mut self, now: Instant) {
        self.check_running = true;
        self.last_check = Some(now);
    }

    pub fn finish_check(&mut self) {
        self.check_running = false;
    }

    /// Applies the gameplay guard to a detected release and returns what to do next.
    ///
    /// A tag that already failed is skipped for the rest of the session; a newer tag is a
    /// different tag, so it is tried normally. While a match runs the tag is deferred, and a
    /// release seen again after the match is what triggers the install.
    pub fn on_release(&mut self, tag: &str, in_game: bool) -> Decision {
        if self.failed_tag.as_deref() == Some(tag) {
            return Decision::Skip;
        }
        if in_game {
            self.deferred = Some(tag.to_string());
            Decision::Defer
        } else {
            self.deferred = None;
            Decision::Install
        }
    }

    /// The tag waiting for the match to end, if any. Taking it clears the deferral.
    pub fn take_deferred(&mut self) -> Option<String> {
        self.deferred.take()
    }

    /// Defers a release whose download finished while a match was running.
    pub fn defer(&mut self, tag: &str) {
        self.deferred = Some(tag.to_string());
    }

    /// Remembers a tag whose download or build failed, so the 30-second loop cannot retry it
    /// all session. A newer release is a different tag and is still tried.
    pub fn on_failure(&mut self, tag: &str) {
        self.failed_tag = Some(tag.to_string());
        self.check_running = false;
    }

    /// The user pressed Install / Retry: forget the failure and check again on the next tick.
    pub fn retry(&mut self) {
        self.failed_tag = None;
        self.deferred = None;
        self.last_check = None;
        self.check_running = false;
    }

    /// A manual install press re-arms the same tag without touching the 30-second clock.
    pub fn clear_failure(&mut self) {
        self.failed_tag = None;
    }
}

impl Default for AutoUpdate {
    fn default() -> Self {
        Self::new(false)
    }
}
