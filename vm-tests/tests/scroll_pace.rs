//! The fling detector that lets a scroll frame skip hover/preview work, exercised on the
//! build host through the real `src/scroll.rs`.

use creblox_vm_tests::scroll::{ScrollPace, AVATAR_TEXTURES_PER_FRAME, adopt_budget};

const HZ_120: f32 = 1.0 / 120.0;

fn coast(pace: &mut ScrollPace, frames: usize) {
    for _ in 0..frames {
        pace.update(0.0, HZ_120);
    }
}

#[test]
fn a_deliberate_scroll_keeps_hover_effects() {
    let mut pace = ScrollPace::new();
    for _ in 0..10 {
        // 12 points per frame at 120 Hz: 1,440 points per second.
        pace.update(12.0, HZ_120);
    }
    assert!(!pace.is_fast(), "a normal scroll was treated as a fling");
}

#[test]
fn a_flick_is_fast_from_the_first_frame() {
    let mut pace = ScrollPace::new();
    // 60 points in one 120 Hz frame: 7,200 points per second.
    pace.update(60.0, HZ_120);
    assert!(pace.is_fast());
}

#[test]
fn a_fling_keeps_counting_while_it_coasts() {
    let mut pace = ScrollPace::new();
    pace.update(60.0, HZ_120);
    // 100 ms of frames the platform reports as motionless while the fling carries on.
    coast(&mut pace, 12);
    assert!(pace.is_fast(), "the fling stopped counting too early");
}

/// A flick must not leave the page in "scrolling" mode after it stops: the next card the
/// pointer rests on has to get its preview without another input event.
#[test]
fn hover_work_resumes_once_the_page_settles() {
    let mut pace = ScrollPace::new();
    pace.update(60.0, HZ_120);
    // Half a second later nothing is moving any more.
    coast(&mut pace, 60);
    assert!(!pace.is_fast(), "still suppressing hover after settling");
}

#[test]
fn a_page_change_clears_the_fling() {
    let mut pace = ScrollPace::new();
    pace.update(60.0, HZ_120);
    assert!(pace.is_fast());
    pace.reset();
    assert!(!pace.is_fast());
}

#[test]
fn a_model_load_is_adopted_a_few_textures_at_a_time() {
    assert_eq!(adopt_budget(0), 0);
    assert_eq!(adopt_budget(1), 1);
    // However many textures an avatar arrives with, one frame never takes the whole batch.
    for pending in [2usize, 3, 12, 40] {
        assert_eq!(adopt_budget(pending), AVATAR_TEXTURES_PER_FRAME.min(pending));
        assert!(adopt_budget(pending) <= AVATAR_TEXTURES_PER_FRAME);
    }
}

#[test]
fn nonsense_input_cannot_get_stuck() {
    let mut pace = ScrollPace::new();
    pace.update(f32::NAN, HZ_120);
    assert!(!pace.is_fast());

    // A stalled frame must not produce an enormous speed.
    let mut pace = ScrollPace::new();
    pace.update(0.0, 0.0);
    assert!(!pace.is_fast());

    let mut pace = ScrollPace::new();
    pace.update(0.0, -1.0);
    assert!(!pace.is_fast());
}
