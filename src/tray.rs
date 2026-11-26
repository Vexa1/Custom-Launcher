//! Platform tray/startup backend selection.
//!
//! * Windows: native notification-area icon + `Run` registry entry.
//! * macOS: menu-bar status item + `LaunchAgents` entry.

#[cfg(windows)]
#[path = "tray_windows.rs"]
mod platform;
#[cfg(windows)]
pub use platform::*;

#[cfg(target_os = "macos")]
#[path = "tray_macos.rs"]
mod platform;
#[cfg(target_os = "macos")]
pub use platform::*;
