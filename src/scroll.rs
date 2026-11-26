//! Deciding when the user is flinging a page.
//!
//! While the content moves fast under a resting pointer, every card that sweeps past looks
//! "hovered" for a frame or two. Acting on that costs a hover animation, a tooltip and a
//! preview lookup per card, which is exactly the work a scroll frame should not do. This
//! keeps one decaying estimate of how hard the page is moving so the drawing code can skip
//! that work and resume the moment the page settles.
//!
//! It also holds the small per-frame budgets that keep a burst of work from landing in a
//! single frame, such as adopting an avatar model's decoded textures.
//!
//! Deliberately free of egui so the rules are testable on the build host
//! (`vm-tests/tests/scroll_pace.rs`); `Creblox::ui` feeds it the frame's scroll delta.

/// Travel in points per second above which a frame counts as a fling rather than a scroll.
///
/// A trackpad flick on a 120 Hz display arrives as deltas of 30-100 points per frame
/// (3600-12000 pt/s); a deliberate two-finger scroll sits around 5-15 points per frame at
/// that rate (600-1800 pt/s). The value sits between the two so a normal scroll keeps its
/// hover effects.
pub const FAST_SCROLL_PTS_PER_SEC: f32 = 3_000.0;

/// How long the estimate takes to fall back to calm once the deltas stop, in seconds.
///
/// While a fling coasts, egui keeps reporting deltas, so those frames read as fast on their
/// own; this only bridges the gaps between them and lets hover previews start about a third
/// of a second after the page comes to rest, instead of freezing them for seconds.
const DRAIN_TAU_SECS: f32 = 0.25;

/// Slowest frame we divide by, so a stalled frame cannot report an enormous speed.
const MIN_FRAME_SECS: f32 = 1.0 / 240.0;

/// Longest frame we account for, so returning from a sleep does not drain the whole estimate.
const MAX_FRAME_SECS: f32 = 0.1;

/// How many decoded avatar textures may move into the live set in one frame.
///
/// A model arrives with a dozen or more of them. Decoding happens on the loader thread, but
/// adopting them all at once still lands the whole batch in the frame the model arrives.
pub const AVATAR_TEXTURES_PER_FRAME: usize = 2;

/// How many of the `pending` items to adopt this frame: the budget, or all that are left.
pub fn adopt_budget(pending: usize) -> usize {
    pending.min(AVATAR_TEXTURES_PER_FRAME)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScrollPace {
    energy: f32,
}

impl ScrollPace {
    pub fn new() -> Self {
        Self { energy: 0.0 }
    }

    /// Feed one frame's finger travel in points and the frame time in seconds.
    pub fn update(&mut self, delta_pts: f32, dt: f32) {
        let dt = if dt.is_finite() { dt } else { 0.0 };
        let dt = dt.clamp(0.0, MAX_FRAME_SECS);
        // The estimate decays on its own, so a flick keeps counting for as long as it
        // coasts even if one frame happens to report a small delta.
        self.energy *= (-dt / DRAIN_TAU_SECS).exp();
        if self.energy < 1.0 {
            self.energy = 0.0;
        }
        let speed = delta_pts.abs() / dt.max(MIN_FRAME_SECS);
        if speed.is_finite() {
            self.energy = self.energy.max(speed);
        }
    }

    /// True while the page is moving fast enough to skip hover work.
    pub fn is_fast(&self) -> bool {
        self.energy > FAST_SCROLL_PTS_PER_SEC
    }

    /// Forget the fling immediately: a page or tab change means nothing is scrolling.
    pub fn reset(&mut self) {
        self.energy = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ_120: f32 = 1.0 / 120.0;

    #[test]
    fn deliberate_scroll_keeps_hover_effects() {
        let mut pace = ScrollPace::new();
        // ~12 points per frame at 120 Hz is 1440 pt/s: a normal two-finger scroll.
        for _ in 0..10 {
            pace.update(12.0, HZ_120);
        }
        assert!(!pace.is_fast());
    }

    #[test]
    fn flick_counts_as_fast() {
        let mut pace = ScrollPace::new();
        pace.update(60.0, HZ_120);
        assert!(pace.is_fast());
    }

    #[test]
    fn a_coasting_flick_stays_fast() {
        let mut pace = ScrollPace::new();
        pace.update(60.0, HZ_120);
        // 100 ms of frames with no delta while the fling continues: still moving fast.
        for _ in 0..12 {
            pace.update(0.0, HZ_120);
        }
        assert!(pace.is_fast());
    }

    #[test]
    fn a_settled_page_is_calm_again() {
        let mut pace = ScrollPace::new();
        pace.update(60.0, HZ_120);
        for _ in 0..360 {
            pace.update(0.0, HZ_120);
        }
        assert!(!pace.is_fast());
    }

    #[test]
    fn avatar_texture_adoption_is_budgeted() {
        assert_eq!(adopt_budget(0), 0);
        assert_eq!(adopt_budget(1), 1);
        assert_eq!(
            adopt_budget(AVATAR_TEXTURES_PER_FRAME),
            AVATAR_TEXTURES_PER_FRAME
        );
        // A whole model's worth never lands in one frame.
        assert_eq!(adopt_budget(40), AVATAR_TEXTURES_PER_FRAME);
    }

    #[test]
    fn reset_and_bad_input_are_calm() {
        let mut pace = ScrollPace::new();
        pace.update(60.0, HZ_120);
        pace.reset();
        assert!(!pace.is_fast());

        let mut pace = ScrollPace::new();
        pace.update(f32::NAN, HZ_120);
        pace.update(0.0, 0.0);
        assert!(!pace.is_fast());
    }
}
