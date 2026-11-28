//! A real `Settings` through the real `settings.json` writer and reader.
//!
//! `settings_json::save_settings_to` and `load_settings_from` are what `auth::save_settings` and
//! `auth::load_settings` call; this crate compiles them straight out of `src/`, so a change that
//! stops writing the picture's box - or stops reading it - fails here instead of at a user's
//! next launch.

use creblox_vm_tests::settings::Settings;
use creblox_vm_tests::settings_json::{load_settings_from, save_settings_to};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "creblox-settings-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp settings directory");
    dir
}

/// A settings value whose picture box and whose neighbouring sliders all differ, so a field
/// that is dropped, renamed or swapped cannot pass by coincidence.
fn picture_at_412_236() -> Settings {
    let mut s = Settings::default();
    s.overlay_image_x = 412;
    s.overlay_image_y = 236;
    s.overlay_image_w = 320;
    s.overlay_image_h = 128;
    s.overlay_image_offset_x = 30;
    s.overlay_image_offset_y = -20;
    s.overlay_image_corner = 1;
    s.overlay_image_size = 40;
    s
}

#[test]
fn a_settings_value_round_trips_through_the_settings_file() {
    let dir = temp_dir("round-trip");
    let saved = picture_at_412_236();

    save_settings_to(&dir, &saved);
    let text = fs::read_to_string(dir.join("settings.json")).expect("settings.json was written");
    let file: Value = serde_json::from_str(&text).expect("settings.json parses");

    // The write side: the four keys must be in the file.
    for (key, want) in [
        ("overlay_image_x", 412),
        ("overlay_image_y", 236),
        ("overlay_image_w", 320),
        ("overlay_image_h", 128),
    ] {
        assert_eq!(file[key], serde_json::json!(want), "{key} in settings.json");
    }

    // The read side: a fresh load gives the same value back.
    let loaded = load_settings_from(&dir);
    assert_eq!(
        (
            loaded.overlay_image_x,
            loaded.overlay_image_y,
            loaded.overlay_image_w,
            loaded.overlay_image_h
        ),
        (412, 236, 320, 128),
        "the loader reads the box back"
    );
    // ... and the sliders and corner it was split from are still their own values.
    assert_eq!(
        (loaded.overlay_image_offset_x, loaded.overlay_image_offset_y),
        (30, -20),
        "the offset sliders come back untouched"
    );
    assert_eq!(
        (loaded.overlay_image_corner, loaded.overlay_image_size),
        (1, 40),
        "the corner and height slider come back untouched"
    );
    assert_eq!(loaded, saved, "the whole settings value comes back");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_pre_split_file_opens_its_picture_where_it_was_left() {
    let dir = temp_dir("pre-split");
    // What 0.5.51 wrote: a saved box whose position lived only in the offset sliders.
    let file = serde_json::json!({
        "overlay_image_w": 320,
        "overlay_image_h": 128,
        "overlay_image_offset_x": 412,
        "overlay_image_offset_y": 236,
    });
    fs::write(
        dir.join("settings.json"),
        serde_json::to_string(&file).expect("fixture"),
    )
    .expect("settings.json written");

    let loaded = load_settings_from(&dir);
    assert_eq!(
        (
            loaded.overlay_image_x,
            loaded.overlay_image_y,
            loaded.overlay_image_w,
            loaded.overlay_image_h
        ),
        (412, 236, 320, 128),
        "an older file's box is migrated, not dropped"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_file_is_the_default_value() {
    let dir = temp_dir("missing");
    assert_eq!(load_settings_from(&dir), Settings::default());
    let _ = fs::remove_dir_all(&dir);
}
