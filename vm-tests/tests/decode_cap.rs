//! The cap on what one thumbnail decode may allocate.
//!
//! The RAM budget counts bytes after a decode, so the decoder's own full-size allocation is
//! the one path that can exceed the cap in a single worker. `decode_fits` is the gate the
//! download worker applies before calling into `image`.

use creblox_vm_tests::thumb_pipeline::{decode_fits, MAX_DECODE_PIXELS, MAX_IMAGE_BYTES};

#[test]
fn every_thumbnail_lane_fits_under_the_decode_cap() {
    // The lanes: card tiles, pass tiles and the gallery hero.
    assert!(decode_fits(256, 144));
    assert!(decode_fits(256, 256));
    assert!(decode_fits(960, 540));
    // A source picture a few times the target size is still welcome.
    assert!(decode_fits(1920, 1080));
    assert!(decode_fits(2560, 2560));
}

#[test]
fn an_oversized_picture_is_refused_before_it_allocates() {
    assert!(!decode_fits(4096, 4096));
    assert!(!decode_fits(20_000, 20_000));
    assert!(!decode_fits(u32::MAX, u32::MAX), "the check must not overflow");
    // 2560x2561 is just past the cap.
    assert!(!decode_fits(2560, 2561));
}

#[test]
fn the_caps_are_smaller_than_a_budget_blowout() {
    // Worst case one decode can allocate, in RGBA bytes.
    assert_eq!(MAX_DECODE_PIXELS * 4, 26_214_400);
    // The download cap keeps a body from being read into RAM without end.
    assert!(MAX_IMAGE_BYTES <= 16 * 1024 * 1024);
    assert!(MAX_IMAGE_BYTES >= 4 * 1024 * 1024);
}
