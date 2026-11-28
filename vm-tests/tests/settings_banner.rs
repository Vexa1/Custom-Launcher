//! The "settings file from a newer build" banner has to be readable in both themes.
//!
//! The palette and the ink both come straight out of `src/theme.rs`, so changing either without
//! re-checking the contrast fails here rather than in a screenshot nobody took in light mode.

use creblox_vm_tests::shots::contrast_ratio;
use creblox_vm_tests::theme::{ORANGE, ORANGE_INK, Palette, warning_ink};

/// WCAG AA for normal-size text.
const MIN_RATIO: f32 = 4.5;

#[test]
fn the_banner_title_is_readable_in_the_dark_theme() {
    let palette = Palette::of(true);
    let ink = warning_ink(true);
    assert_eq!(ink, ORANGE);
    let ratio = contrast_ratio(ink, palette.card_bg);
    assert!(
        ratio >= MIN_RATIO,
        "banner title on the dark card: {ratio:.2}:1"
    );
}

#[test]
fn the_banner_title_is_readable_in_the_light_theme() {
    let palette = Palette::of(false);
    let ink = warning_ink(false);
    assert_eq!(ink, ORANGE_INK);
    let ratio = contrast_ratio(ink, palette.card_bg);
    assert!(
        ratio >= MIN_RATIO,
        "banner title on the light card: {ratio:.2}:1"
    );
    // Why the light theme cannot reuse the accent: it is a fill colour, not ink.
    assert!(
        contrast_ratio(ORANGE, palette.card_bg) < MIN_RATIO,
        "the accent orange should not be used as light-theme text"
    );
}
