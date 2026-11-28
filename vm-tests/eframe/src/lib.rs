//! Stand-in for the GUI crate.
//!
//! `party.rs` asks egui for a `Context` and calls `request_repaint()` on it. Rather than
//! re-declaring the little it needs, the stand-in re-exports the real crate: the harness can
//! then also compile modules that use egui types (`theme`, the settings palette) for real.

pub mod egui {
    pub use ::egui::*;
}
