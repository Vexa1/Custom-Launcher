//! `theme::ink` turns an accent into text a light card can carry, and leaves the dark palette
//! alone. The pages rely on both halves, so both are pinned here.

use creblox_vm_tests::theme::{self, BLUE, BLUE_INK, GREEN, GREEN_INK, ORANGE, ORANGE_INK, RED, RED_INK};

#[test]
fn dark_keeps_the_accent_and_light_uses_the_ink() {
    for (accent, ink) in [
        (GREEN, GREEN_INK),
        (BLUE, BLUE_INK),
        (ORANGE, ORANGE_INK),
        (RED, RED_INK),
    ] {
        assert_eq!(theme::ink(accent, true), accent, "dark text keeps the accent");
        assert_eq!(theme::ink(accent, false), ink, "light text uses the ink");
        assert!(
            theme::contrast_on_white(ink) >= 4.5,
            "ink {ink:?} is only {:.2}:1 on white",
            theme::contrast_on_white(ink)
        );
        // The light palette's surface card is the other background text lands on.
        let surface = theme::Palette::of(false).surface_bg;
        let ratio = theme::contrast_ratio(ink, surface);
        assert!(ratio >= 4.5, "ink {ink:?} is only {ratio:.2}:1 on the surface card");
    }
}

/// A custom accent (or the login page's orange) is not one of the four named colours; its text
/// form is darkened until it clears the same 4.5:1.
#[test]
fn an_unnamed_accent_is_darkened_until_it_reads() {
    let rr_orange = egui::Color32::from_rgb(0xFF, 0x6B, 0x2B);
    let inked = theme::ink(rr_orange, false);
    assert_ne!(inked, rr_orange);
    assert!(theme::contrast_on_white(inked) >= 4.5);
    assert!(inked.r() > inked.g() && inked.g() > inked.b(), "the hue survives");

    // A colour that already reads on white is handed back untouched.
    let ink = theme::ink(egui::Color32::from_rgb(0x20, 0x24, 0x2C), false);
    assert_eq!(ink, egui::Color32::from_rgb(0x20, 0x24, 0x2C));
}
