//! A real `Settings` through the real `settings.json` writer and reader.
//!
//! `settings_json::save_settings_to` and `load_settings_from` are what `auth::save_settings` and
//! `auth::load_settings` call; this crate compiles them straight out of `src/`, so a change that
//! stops writing the picture's box - or stops reading it - fails here instead of at a user's
//! next launch.

use creblox_vm_tests::settings::Settings;
use creblox_vm_tests::settings_json::{
    SETTINGS_VERSION, SaveOutcome, load_settings_from, save_settings_to, settings_version,
};
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

/// One non-default value of every shape the file can carry: the optional colours, the optional
/// numeric limits, the panel offsets, unicode paths and the odd small integers. A lossy encoding
/// of any of them shows up as an inequality rather than as a field quietly falling back to its
/// default.
fn every_shape() -> Settings {
    use creblox_vm_tests::overlay_spec::PanelOffset;
    use creblox_vm_tests::settings::{BgMode, TextColorMode};

    let mut s = picture_at_412_236();

    // Optional colours: three distinct triples plus a fifth left unset.
    s.borders_color = Some([1, 2, 3]);
    s.profile_color = Some([200, 100, 50]);
    s.friends_color = Some([9, 8, 7]);
    s.games_border_color = None;
    s.text_border_color = Some([255, 0, 128]);

    // Optional limits: values, and a neighbouring one still unset.
    s.target_fps = Some(144);
    s.ram_limit_mb = Some(3072);
    s.cpu_limit_pct = Some(65);
    s.auto_update = true;
    s.server_min_players = Some(2);
    s.server_max_players = Some(24);
    s.server_max_ping = Some(150);

    // Per-panel HUD offsets, including a negative one.
    s.overlay_panels = vec![
        PanelOffset {
            id: "server".to_string(),
            x: -37,
            y: 214,
        },
        PanelOffset {
            id: "stats".to_string(),
            x: 12,
            y: -8,
        },
        PanelOffset {
            id: "party".to_string(),
            x: 0,
            y: 41,
        },
    ];

    // Unicode paths, as a macOS wallpaper folder and a picture name really can be.
    s.bg_wallpaper = "/Users/véxa/壁紙/салют ünïcode.png".to_string();
    s.overlay_image_path = "/Users/véxa/Pictures/логотип 🖼️ ロゴ.png".to_string();
    s.crosshair_image = "/Users/véxa/十字/прицел.png".to_string();

    // Enums and the plain values around them.
    s.dark = true;
    s.bg_mode = BgMode::SolidColor;
    s.bg_color = [7, 8, 9];
    s.bg_overlay = 0.55;
    s.text_mode = TextColorMode::White;
    s.glass_opacity = 0.61;
    s.glass_highlight = 0.33;
    s.corner_roundness = 9;
    s.overlay_opacity = 0.75;
    s.overlay_duration_secs = 250;
    s.overlay_flags_max_lines = 7;
    s.overlay_image_enabled = true;
    s.overlay_image_corner = 2;
    s.overlay_image_opacity = 0.55;
    s.crosshair_enabled = true;
    s.crosshair_style = 2;
    s.crosshair_size = 9;
    s.crosshair_thickness = 3;
    s.crosshair_gap = 5;
    s.crosshair_color = [10, 20, 30];
    s.crosshair_opacity = 0.55;
    s.crosshair_offset_x = -7;
    s.crosshair_offset_y = 9;
    s.discord_rpc_app_id = "123456789012345678".to_string();
    s.animations_enabled = false;
    s.auto_refresh = false;
    s.server_filter_low_ping = true;
    s.server_show_players = true;
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
fn every_shape_in_the_file_survives_the_round_trip() {
    let dir = temp_dir("every-shape");
    let saved = every_shape();
    // Each shape must actually differ from what a missing key would load as, or this test would
    // not be able to see it being dropped.
    assert_ne!(saved, Settings::default(), "the fixture is not the defaults");

    save_settings_to(&dir, &saved);
    let text = fs::read_to_string(dir.join("settings.json")).expect("settings.json was written");
    let file: Value = serde_json::from_str(&text).expect("settings.json parses");

    // The shapes are in the file with the values they went in with - `null` is only ever a
    // deliberate "no value", never a dropped one.
    assert_eq!(file["borders_color"], serde_json::json!([1, 2, 3]));
    assert_eq!(file["profile_color"], serde_json::json!([200, 100, 50]));
    assert_eq!(file["games_border_color"], Value::Null);
    assert_eq!(file["target_fps"], serde_json::json!(144));
    assert_eq!(file["ram_limit_mb"], serde_json::json!(3072));
    assert_eq!(file["cpu_limit_pct"], serde_json::json!(65));
    assert_eq!(file["server_max_ping"], serde_json::json!(150));
    assert_eq!(file["bg_wallpaper"], serde_json::json!(saved.bg_wallpaper));
    assert_eq!(
        file["overlay_image_path"],
        serde_json::json!(saved.overlay_image_path)
    );
    assert_eq!(file["overlay_panels"][0]["id"], serde_json::json!("server"));
    assert_eq!(file["overlay_panels"][0]["x"], serde_json::json!(-37));
    assert_eq!(file["overlay_panels"][0]["y"], serde_json::json!(214));
    assert_eq!(file["overlay_panels"][2]["id"], serde_json::json!("party"));

    // And the whole value comes back unchanged, which is what catches a shape the writer kept
    // but the reader narrows, rounds or drops.
    let loaded = load_settings_from(&dir);
    assert_eq!(loaded, saved, "every shape comes back unchanged");

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

    // No `schema` key at all: that is version 1, and it is read as such.
    assert_eq!(settings_version(&file), 1);

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
fn out_of_range_values_are_normalised_and_then_stay_put() {
    use creblox_vm_tests::overlay_spec::PanelOffset;

    let dir = temp_dir("out-of-range");
    // A hand-edited file: every value here is outside the range its slider can produce.
    let raw = serde_json::json!({
        "ram_limit_mb": 6144,
        "cpu_limit_pct": 5,
        "crosshair_offset_x": -1000,
        "glass_opacity": 3.0,
        "overlay_image_corner": 9,
        "overlay_image_size": 250,
        "overlay_flags_max_lines": 99,
        "overlay_image_x": -50,
        "overlay_image_y": 9000,
        "overlay_image_w": 320,
        "overlay_image_h": 128,
        "overlay_panels": [
            {"id": "server", "x": -9000, "y": 9000},
            {"id": "stats", "x": 5, "y": -5},
        ],
    });
    fs::write(
        dir.join("settings.json"),
        serde_json::to_string_pretty(&raw).expect("fixture"),
    )
    .expect("settings.json written");

    // The load normalises each value into the range its editor can produce.
    let first = load_settings_from(&dir);
    assert_eq!(first.ram_limit_mb, Some(4096), "RAM capped at 4096 MB");
    assert_eq!(first.cpu_limit_pct, Some(10), "CPU floor is 10%");
    assert_eq!(first.crosshair_offset_x, -100, "crosshair offset is +-100");
    assert_eq!(first.glass_opacity, 0.92, "glass opacity is 0.20-0.92");
    assert_eq!(first.overlay_image_corner, 3, "corner is 0-3");
    assert_eq!(first.overlay_image_size, 100, "height is 5-100%");
    assert_eq!(first.overlay_flags_max_lines, 40, "flags lines are 1-40");
    assert_eq!(
        (first.overlay_image_x, first.overlay_image_y),
        (0, 8000),
        "the picture's box is clamped into the game area"
    );
    assert_eq!(
        first.overlay_panels,
        vec![
            PanelOffset {
                id: "server".to_string(),
                x: -8000,
                y: 8000,
            },
            PanelOffset {
                id: "stats".to_string(),
                x: 5,
                y: -5,
            },
        ],
        "panel offsets are clamped and keep their entries"
    );

    // The second pass writes the normalised value and reads it back. If a clamp only ran on
    // load while the writer kept the out-of-range number, this is where it would oscillate.
    save_settings_to(&dir, &first);
    let second = load_settings_from(&dir);
    assert_eq!(second, first, "a normalised value is a fixed point");

    // And the file itself now holds the clamped numbers, not the hand-edited ones.
    let text = fs::read_to_string(dir.join("settings.json")).expect("settings.json written");
    let file: Value = serde_json::from_str(&text).expect("settings.json parses");
    assert_eq!(file["ram_limit_mb"], serde_json::json!(4096));
    assert_eq!(file["cpu_limit_pct"], serde_json::json!(10));
    assert_eq!(file["overlay_panels"][0]["x"], serde_json::json!(-8000));
    assert_eq!(file["overlay_panels"][0]["y"], serde_json::json!(8000));
    assert_eq!(file["overlay_image_y"], serde_json::json!(8000));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_file_carries_its_schema_version() {
    let dir = temp_dir("schema-version");
    save_settings_to(&dir, &picture_at_412_236());

    let text = fs::read_to_string(dir.join("settings.json")).expect("settings.json written");
    let file: Value = serde_json::from_str(&text).expect("settings.json parses");
    assert_eq!(
        file["schema"],
        serde_json::json!(SETTINGS_VERSION),
        "every file this build writes names its layout"
    );
    assert_eq!(settings_version(&file), SETTINGS_VERSION);
    assert_eq!(load_settings_from(&dir).overlay_image_w, 320);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_version_2_file_takes_the_position_only_from_its_own_fields() {
    let dir = temp_dir("schema-2");
    // Version 2 has the position in `x`/`y`, so a box without them is at 0 rather than in the
    // offset sliders - the offsets are sliders again, not a hiding place.
    let file = serde_json::json!({
        "schema": 2,
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

    assert_eq!(settings_version(&file), 2);
    let loaded = load_settings_from(&dir);
    assert_eq!(
        (
            loaded.overlay_image_x,
            loaded.overlay_image_y,
            loaded.overlay_image_w,
            loaded.overlay_image_h
        ),
        (0, 0, 320, 128),
        "a version 2 file's offsets are not a position"
    );

    // Saving it back keeps the version and the box, so the file settles.
    save_settings_to(&dir, &loaded);
    assert_eq!(load_settings_from(&dir), loaded);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_newer_schema_still_opens() {
    let dir = temp_dir("schema-newer");
    // A file from a later build: read what this build understands instead of falling back to
    // defaults, and rewrite it as this build's version.
    let file = serde_json::json!({
        "schema": 9,
        "dark": true,
        "overlay_image_x": 412,
        "overlay_image_y": 236,
        "overlay_image_w": 320,
        "overlay_image_h": 128,
        "some_future_key": {"nested": true},
    });
    fs::write(
        dir.join("settings.json"),
        serde_json::to_string(&file).expect("fixture"),
    )
    .expect("settings.json written");

    assert_eq!(settings_version(&file), 9);
    let loaded = load_settings_from(&dir);
    assert!(loaded.dark);
    assert_eq!(
        (
            loaded.overlay_image_x,
            loaded.overlay_image_y,
            loaded.overlay_image_w,
            loaded.overlay_image_h
        ),
        (412, 236, 320, 128)
    );

    // ... and saving writes this build's keys into that file while keeping everything it does
    // not know, so the newer build still finds its own settings.
    let mut changed = loaded;
    changed.overlay_image_x = 99;
    changed.dark = false;
    assert_eq!(
        save_settings_to(&dir, &changed),
        SaveOutcome::MergedIntoNewer,
        "a newer file is merged into, not replaced"
    );
    let after = fs::read_to_string(dir.join("settings.json")).expect("fixture file");
    let merged: Value = serde_json::from_str(&after).expect("settings.json parses");
    assert_eq!(merged["schema"], serde_json::json!(9), "the layout version is kept");
    assert_eq!(
        merged["some_future_key"]["nested"],
        serde_json::json!(true),
        "keys this build does not know survive"
    );
    assert_eq!(merged["overlay_image_x"], serde_json::json!(99), "known keys are written");
    assert_eq!(merged["dark"], serde_json::json!(false));
    assert_eq!(
        load_settings_from(&dir),
        changed,
        "the merged file reads back as this build's settings"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_current_version_file_is_still_written() {
    // The read-only rule is about newer files only: this build's own files save normally.
    let dir = temp_dir("schema-current");
    assert_eq!(
        save_settings_to(&dir, &picture_at_412_236()),
        SaveOutcome::Written
    );

    let text = fs::read_to_string(dir.join("settings.json")).expect("settings.json written");
    let file: Value = serde_json::from_str(&text).expect("settings.json parses");
    assert_eq!(file["schema"], serde_json::json!(SETTINGS_VERSION));
    assert_eq!(
        save_settings_to(&dir, &Settings::default()),
        SaveOutcome::Written,
        "a second save writes"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_file_is_the_default_value() {
    let dir = temp_dir("missing");
    assert_eq!(load_settings_from(&dir), Settings::default());
    let _ = fs::remove_dir_all(&dir);
}
