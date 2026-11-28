//! `settings.json`: reading, writing and the migrations between versions.
//!
//! This is the whole mapping between `Settings` and the file, kept out of `auth`'s account and
//! session code so `vm-tests` can compile it and round-trip a real `Settings` in the VM.

use crate::overlay_spec::{SavedImageBox, parse_panel_offsets};
use crate::settings::{BgMode, Settings, TextColorMode};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

/// The `settings.json` layout this build writes.
///
/// 1 was every file written before the `schema` field existed; it kept a dragged picture's
/// position in the offset sliders. 2 is this layout: the position has its own fields and the
/// offsets are sliders again. A future migration reads this number instead of guessing from a
/// missing key.
pub const SETTINGS_VERSION: u64 = 2;

/// The version in a settings object, read without copying it.
fn version_of(object: &serde_json::Map<String, Value>) -> u64 {
    object
        .get("schema")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .unwrap_or(1)
}

/// The version of a settings object. A file without the field - or with a nonsense one - is
/// version 1, which is what every file written before the field existed really is.
pub fn settings_version(v: &Value) -> u64 {
    v.as_object().map(version_of).unwrap_or(1)
}

/// The `settings.json` already in `dir`, if there is a readable one.
fn existing_object(dir: &Path) -> Option<serde_json::Map<String, Value>> {
    let text = fs::read_to_string(dir.join("settings.json")).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.as_object().cloned()
}

/// The version of the `settings.json` already in `dir`, if there is a readable one.
pub fn existing_version(dir: &Path) -> Option<u64> {
    existing_object(dir).map(|o| version_of(&o))
}

fn read_opt_rgb(v: &Value, key: &str) -> Option<[u8; 3]> {
    let arr = v[key].as_array()?;
    if arr.len() != 3 {
        return None;
    }
    Some([
        arr[0].as_u64()?.min(255) as u8,
        arr[1].as_u64()?.min(255) as u8,
        arr[2].as_u64()?.min(255) as u8,
    ])
}

fn opt_u32(v: &Value, key: &str, def: Option<u32>) -> Option<u32> {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().map(|n| n.min(u32::MAX as u64) as u32),
        // An explicit `null` means "no limit", so starting from a value clears it.
        Some(Value::Null) => None,
        _ => def,
    }
}

fn opt_rgb_json(c: Option<[u8; 3]>) -> Value {
    match c {
        Some([r, g, b]) => serde_json::json!([r, g, b]),
        None => Value::Null,
    }
}

/// Reads `dir/settings.json`; a missing or unparseable file is a default value.
pub fn load_settings_from(dir: &Path) -> Settings {
    let Ok(text) = fs::read_to_string(dir.join("settings.json")) else {
        return Settings::default();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Settings::default();
    };
    let def = Settings::default();
    // The custom image's saved box. A version 1 file keeps a dragged position in the offsets;
    // from version 2 the position is only in `x`/`y`, so the offsets stay sliders.
    let saved_image = SavedImageBox::from_settings_at(&v, settings_version(&v));
    let bg_mode = match v["bg_mode"].as_str() {
        Some("color") => BgMode::SolidColor,
        Some("wallpaper") => BgMode::Wallpaper,
        _ => BgMode::ThemeDefault,
    };
    let bg_color = read_opt_rgb(&v, "bg_color").unwrap_or(def.bg_color);
    let bg_wallpaper = v["bg_wallpaper"].as_str().unwrap_or("").to_string();
    let bg_overlay = v["bg_overlay"]
        .as_f64()
        .map(|f| (f as f32).clamp(0.0, 0.85))
        .unwrap_or(def.bg_overlay);
    let text_mode = match v["text_mode"].as_str() {
        Some("white") => TextColorMode::White,
        Some("green") => TextColorMode::Green,
        Some("dark") => TextColorMode::Dark,
        _ => TextColorMode::Auto,
    };
    Settings {
        dark: v["dark"].as_bool().unwrap_or(false),
        bg_mode,
        bg_color,
        bg_wallpaper,
        bg_overlay,
        text_mode,
        borders_color: read_opt_rgb(&v, "borders_color"),
        profile_color: read_opt_rgb(&v, "profile_color"),
        friends_color: read_opt_rgb(&v, "friends_color"),
        games_border_color: read_opt_rgb(&v, "games_border_color"),
        text_border_color: read_opt_rgb(&v, "text_border_color"),
        target_fps: v["target_fps"].as_u64().and_then(|n| u32::try_from(n).ok()),
        ram_limit_mb: v["ram_limit_mb"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| n.clamp(128, 4096)),
        cpu_limit_pct: v["cpu_limit_pct"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(10, 100))
            .filter(|&p| p < 100),
        glass_edition: v["glass_edition"].as_bool().unwrap_or(def.glass_edition),
        glass_opacity: v["glass_opacity"]
            .as_f64()
            .map(|f| (f as f32).clamp(0.20, 0.92))
            .unwrap_or(def.glass_opacity),
        glass_highlight: v["glass_highlight"]
            .as_f64()
            .map(|f| (f as f32).clamp(0.0, 1.0))
            .unwrap_or(def.glass_highlight),
        corner_roundness: v["corner_roundness"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(6, 22))
            .unwrap_or(def.corner_roundness),
        discord_rpc_enabled: v["discord_rpc_enabled"]
            .as_bool()
            .unwrap_or(def.discord_rpc_enabled),
        discord_rpc_show_idle: v["discord_rpc_show_idle"]
            .as_bool()
            .unwrap_or(def.discord_rpc_show_idle),
        discord_rpc_app_id: v["discord_rpc_app_id"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string(),
        tray_enabled: v["tray_enabled"].as_bool().unwrap_or(def.tray_enabled),
        tray_minimize_on_close: v["tray_minimize_on_close"]
            .as_bool()
            .unwrap_or(def.tray_minimize_on_close),
        start_with_windows: v["start_with_windows"]
            .as_bool()
            .unwrap_or(def.start_with_windows),
        start_minimized_to_tray: v["start_minimized_to_tray"]
            .as_bool()
            .unwrap_or(def.start_minimized_to_tray),
        auto_close_roblox: v["auto_close_roblox"].as_bool().unwrap_or(true),
        multi_instance: v["multi_instance"].as_bool().unwrap_or(def.multi_instance),
        notifications_silent: v["notifications_silent"].as_bool().unwrap_or(false),
        auto_check_updates: v["auto_check_updates"].as_bool().unwrap_or(true),
        auto_download_updates: v["auto_download_updates"].as_bool().unwrap_or(false),
        auto_refresh: v["auto_refresh"].as_bool().unwrap_or(true),
        overlay_enabled: v["overlay_enabled"]
            .as_bool()
            .unwrap_or(def.overlay_enabled),
        overlay_corner: v["overlay_corner"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.min(3))
            .unwrap_or(def.overlay_corner),
        overlay_duration_secs: v["overlay_duration_secs"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(def.overlay_duration_secs),
        overlay_opacity: v["overlay_opacity"]
            .as_f64()
            .map(|f| (f as f32).clamp(0.2, 1.0))
            .unwrap_or(def.overlay_opacity),
        overlay_show_ip: v["overlay_show_ip"]
            .as_bool()
            .unwrap_or(def.overlay_show_ip),
        overlay_show_job_id: v["overlay_show_job_id"]
            .as_bool()
            .unwrap_or(def.overlay_show_job_id),
        show_server_badge: v["show_server_badge"]
            .as_bool()
            .unwrap_or(def.show_server_badge),
        return_to_launcher_on_leave: v["return_to_launcher_on_leave"]
            .as_bool()
            .unwrap_or(def.return_to_launcher_on_leave),
        minimize_on_game_launch: v["minimize_on_game_launch"]
            .as_bool()
            .unwrap_or(def.minimize_on_game_launch),
        close_launcher_during_game: v["close_launcher_during_game"]
            .as_bool()
            .unwrap_or(def.close_launcher_during_game),
        // Missing in every settings.json written before per-panel positions: an old single
        // offset is folded into the server card instead of being dropped.
        overlay_panels: parse_panel_offsets(
            &v["overlay_panels"],
            v["overlay_offset_x"]
                .as_i64()
                .map(|n| n.clamp(-8000, 8000) as i16)
                .unwrap_or(0),
            v["overlay_offset_y"]
                .as_i64()
                .map(|n| n.clamp(-8000, 8000) as i16)
                .unwrap_or(0),
        ),
        overlay_stats_enabled: v["overlay_stats_enabled"]
            .as_bool()
            .unwrap_or(def.overlay_stats_enabled),
        party_overlay: v["party_overlay"].as_bool().unwrap_or(def.party_overlay),
        party_overlay_corner: v["party_overlay_corner"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.min(3))
            .unwrap_or(def.party_overlay_corner),
        overlay_stats_corner: v["overlay_stats_corner"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.min(3))
            .unwrap_or(def.overlay_stats_corner),
        overlay_stats_cpu: v["overlay_stats_cpu"]
            .as_bool()
            .unwrap_or(def.overlay_stats_cpu),
        overlay_stats_ram: v["overlay_stats_ram"]
            .as_bool()
            .unwrap_or(def.overlay_stats_ram),
        overlay_stats_ping: v["overlay_stats_ping"]
            .as_bool()
            .unwrap_or(def.overlay_stats_ping),
        overlay_stats_session: v["overlay_stats_session"]
            .as_bool()
            .unwrap_or(def.overlay_stats_session),
        overlay_stats_clock: v["overlay_stats_clock"]
            .as_bool()
            .unwrap_or(def.overlay_stats_clock),
        overlay_stats_fps_cap: v["overlay_stats_fps_cap"]
            .as_bool()
            .unwrap_or(def.overlay_stats_fps_cap),
        overlay_flags_enabled: v["overlay_flags_enabled"]
            .as_bool()
            .unwrap_or(def.overlay_flags_enabled),
        overlay_flags_corner: v["overlay_flags_corner"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.min(3))
            .unwrap_or(def.overlay_flags_corner),
        overlay_flags_max_lines: v["overlay_flags_max_lines"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(1, 40))
            .unwrap_or(def.overlay_flags_max_lines),
        overlay_game_enabled: v["overlay_game_enabled"]
            .as_bool()
            .unwrap_or(def.overlay_game_enabled),
        overlay_game_corner: v["overlay_game_corner"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(0, 3))
            .unwrap_or(def.overlay_game_corner),
        overlay_image_enabled: v["overlay_image_enabled"]
            .as_bool()
            .unwrap_or(def.overlay_image_enabled),
        overlay_image_path: v["overlay_image_path"]
            .as_str()
            .unwrap_or(&def.overlay_image_path)
            .to_string(),
        overlay_image_corner: v["overlay_image_corner"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(0, 3))
            .unwrap_or(def.overlay_image_corner),
        overlay_image_size: v["overlay_image_size"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(5, 100))
            .unwrap_or(def.overlay_image_size),
        overlay_image_opacity: v["overlay_image_opacity"]
            .as_f64()
            .map(|f| (f as f32).clamp(0.1, 1.0))
            .unwrap_or(def.overlay_image_opacity),
        overlay_image_offset_x: v["overlay_image_offset_x"]
            .as_i64()
            .map(|n| n.clamp(-2000, 2000) as i16)
            .unwrap_or(def.overlay_image_offset_x),
        overlay_image_offset_y: v["overlay_image_offset_y"]
            .as_i64()
            .map(|n| n.clamp(-2000, 2000) as i16)
            .unwrap_or(def.overlay_image_offset_y),
        // The editor's saved box. Anything under the editor's own minimum is treated as
        // "not set", so a hand-edited 3 px box cannot make the picture impossible to grab.
        // A 0.5.51 file kept the dragged position in the offsets; `from_settings` reads that
        // once, so the picture stays exactly where the user left it.
        overlay_image_x: saved_image.x,
        overlay_image_y: saved_image.y,
        overlay_image_w: saved_image.w,
        overlay_image_h: saved_image.h,
        crosshair_enabled: v["crosshair_enabled"]
            .as_bool()
            .unwrap_or(def.crosshair_enabled),
        crosshair_style: v["crosshair_style"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(0, 3))
            .unwrap_or(def.crosshair_style),
        crosshair_size: v["crosshair_size"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(1, 120))
            .unwrap_or(def.crosshair_size),
        crosshair_thickness: v["crosshair_thickness"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.clamp(1, 120))
            .unwrap_or(def.crosshair_thickness),
        crosshair_gap: v["crosshair_gap"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|n| n.min(60))
            .unwrap_or(def.crosshair_gap),
        crosshair_color: v["crosshair_color"]
            .as_array()
            .filter(|a| a.len() == 3)
            .map(|a| {
                [
                    a[0].as_u64().unwrap_or(255).min(255) as u8,
                    a[1].as_u64().unwrap_or(255).min(255) as u8,
                    a[2].as_u64().unwrap_or(255).min(255) as u8,
                ]
            })
            .unwrap_or(def.crosshair_color),
        crosshair_opacity: v["crosshair_opacity"]
            .as_f64()
            .map(|f| (f as f32).clamp(0.1, 1.0))
            .unwrap_or(def.crosshair_opacity),
        crosshair_offset_x: v["crosshair_offset_x"]
            .as_i64()
            .map(|n| n.clamp(-100, 100) as i8)
            .unwrap_or(def.crosshair_offset_x),
        crosshair_offset_y: v["crosshair_offset_y"]
            .as_i64()
            .map(|n| n.clamp(-100, 100) as i8)
            .unwrap_or(def.crosshair_offset_y),
        animations_enabled: v["animations_enabled"]
            .as_bool()
            .unwrap_or(def.animations_enabled),
        hover_animations: v["hover_animations"]
            .as_bool()
            .unwrap_or(def.hover_animations),
        animated_backgrounds: v["animated_backgrounds"]
            .as_bool()
            .unwrap_or(def.animated_backgrounds),
        video_previews: v["video_previews"].as_bool().unwrap_or(def.video_previews),
        crosshair_image: v["crosshair_image"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or(def.crosshair_image),
        server_filter_low_ping: v["server_filter_low_ping"]
            .as_bool()
            .unwrap_or(def.server_filter_low_ping),
        server_filter_few_players: v["server_filter_few_players"]
            .as_bool()
            .unwrap_or(def.server_filter_few_players),
        server_filter_good_fps: v["server_filter_good_fps"]
            .as_bool()
            .unwrap_or(def.server_filter_good_fps),
        server_min_players: opt_u32(&v, "server_min_players", def.server_min_players),
        server_max_players: opt_u32(&v, "server_max_players", def.server_max_players),
        server_max_ping: opt_u32(&v, "server_max_ping", def.server_max_ping),
        server_show_players: v["server_show_players"]
            .as_bool()
            .unwrap_or(def.server_show_players),
    }
}

/// What a save did to the file that was already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Written as this build's own layout.
    Written,
    /// A newer build's file was updated in place: this build's keys were written into it and
    /// the file kept its schema version and every key this build does not know.
    MergedIntoNewer,
    /// Nothing could be written (an I/O error).
    Failed,
}

/// Writes `s` into `dir/settings.json`.
///
/// A file from a **newer build** holds keys this build has never heard of, so it is never
/// replaced: this build's own keys are merged into it, and its `schema` and unknown keys are
/// kept as they are. The newer build therefore finds its settings intact, with the ones this
/// build owns updated.
pub fn save_settings_to(dir: &Path, s: &Settings) -> SaveOutcome {
    let mode_str = match s.bg_mode {
        BgMode::ThemeDefault => "default",
        BgMode::SolidColor => "color",
        BgMode::Wallpaper => "wallpaper",
    };
    let text_str = match s.text_mode {
        TextColorMode::Auto => "auto",
        TextColorMode::White => "white",
        TextColorMode::Green => "green",
        TextColorMode::Dark => "dark",
    };
    let mut v = serde_json::json!({
        "schema": SETTINGS_VERSION,
        "dark": s.dark,
        "bg_mode": mode_str,
        "bg_color": [s.bg_color[0], s.bg_color[1], s.bg_color[2]],
        "bg_wallpaper": s.bg_wallpaper,
        "bg_overlay": s.bg_overlay,
        "text_mode": text_str,
        "borders_color": opt_rgb_json(s.borders_color),
        "profile_color": opt_rgb_json(s.profile_color),
        "friends_color": opt_rgb_json(s.friends_color),
        "games_border_color": opt_rgb_json(s.games_border_color),
        "text_border_color": opt_rgb_json(s.text_border_color),
        "target_fps": s.target_fps,
        "ram_limit_mb": s.ram_limit_mb,
        "cpu_limit_pct": s.cpu_limit_pct,
        "glass_edition": s.glass_edition,
        "glass_opacity": s.glass_opacity,
        "glass_highlight": s.glass_highlight,
        "corner_roundness": s.corner_roundness,
        "discord_rpc_enabled": s.discord_rpc_enabled,
        "discord_rpc_show_idle": s.discord_rpc_show_idle,
        "discord_rpc_app_id": s.discord_rpc_app_id,
        "tray_enabled": s.tray_enabled,
        "tray_minimize_on_close": s.tray_minimize_on_close,
        "start_with_windows": s.start_with_windows,
        "start_minimized_to_tray": s.start_minimized_to_tray,
        "auto_close_roblox": s.auto_close_roblox,
        "multi_instance": s.multi_instance,
        "notifications_silent": s.notifications_silent,
        "auto_check_updates": s.auto_check_updates,
        "auto_download_updates": s.auto_download_updates,
        "auto_refresh": s.auto_refresh,
        "overlay_enabled": s.overlay_enabled,
        "overlay_corner": s.overlay_corner,
        "overlay_duration_secs": s.overlay_duration_secs,
        "overlay_opacity": s.overlay_opacity,
        "overlay_show_ip": s.overlay_show_ip,
        "overlay_show_job_id": s.overlay_show_job_id,
        "overlay_panels": s
            .overlay_panels
            .iter()
            .map(|p| json!({"id": p.id, "x": p.x, "y": p.y}))
            .collect::<Vec<_>>(),
        "show_server_badge": s.show_server_badge,
        "return_to_launcher_on_leave": s.return_to_launcher_on_leave,
        "minimize_on_game_launch": s.minimize_on_game_launch,
        "close_launcher_during_game": s.close_launcher_during_game,
        "overlay_stats_enabled": s.overlay_stats_enabled,
        "party_overlay": s.party_overlay,
        "party_overlay_corner": s.party_overlay_corner,
        "overlay_stats_corner": s.overlay_stats_corner,
        "overlay_stats_cpu": s.overlay_stats_cpu,
        "overlay_stats_ram": s.overlay_stats_ram,
        "overlay_stats_ping": s.overlay_stats_ping,
        "overlay_stats_session": s.overlay_stats_session,
        "overlay_stats_clock": s.overlay_stats_clock,
        "overlay_stats_fps_cap": s.overlay_stats_fps_cap,
        "overlay_flags_enabled": s.overlay_flags_enabled,
        "overlay_flags_corner": s.overlay_flags_corner,
        "overlay_flags_max_lines": s.overlay_flags_max_lines,
        "overlay_game_enabled": s.overlay_game_enabled,
        "overlay_game_corner": s.overlay_game_corner,
        "overlay_image_enabled": s.overlay_image_enabled,
        "overlay_image_path": s.overlay_image_path,
        "overlay_image_corner": s.overlay_image_corner,
        "overlay_image_size": s.overlay_image_size,
        "overlay_image_opacity": s.overlay_image_opacity,
        "overlay_image_offset_x": s.overlay_image_offset_x,
        "overlay_image_offset_y": s.overlay_image_offset_y,
        "crosshair_enabled": s.crosshair_enabled,
        "crosshair_style": s.crosshair_style,
        "crosshair_size": s.crosshair_size,
        "crosshair_thickness": s.crosshair_thickness,
        "crosshair_gap": s.crosshair_gap,
        "crosshair_color": s.crosshair_color,
        "crosshair_opacity": s.crosshair_opacity,
        "crosshair_offset_x": s.crosshair_offset_x,
        "crosshair_offset_y": s.crosshair_offset_y,
        "animations_enabled": s.animations_enabled,
        "hover_animations": s.hover_animations,
        "animated_backgrounds": s.animated_backgrounds,
        "video_previews": s.video_previews,
        "crosshair_image": s.crosshair_image,
        "server_filter_low_ping": s.server_filter_low_ping,
        "server_filter_few_players": s.server_filter_few_players,
        "server_filter_good_fps": s.server_filter_good_fps,
        "server_min_players": s.server_min_players,
        "server_max_players": s.server_max_players,
        "server_max_ping": s.server_max_ping,
        "server_show_players": s.server_show_players,
    });
    if let Some(obj) = v.as_object_mut() {
        crate::overlay_spec::SavedImageBox {
            x: s.overlay_image_x,
            y: s.overlay_image_y,
            w: s.overlay_image_w,
            h: s.overlay_image_h,
        }
        .insert_into(obj);
    }
    let existing = existing_object(dir);
    let newer_version = existing
        .as_ref()
        .map(version_of)
        .filter(|version| *version > SETTINGS_VERSION);
    let (out, outcome) = match (existing, newer_version) {
        // Keep the newer layout's version and everything this build does not recognise;
        // overwrite only the keys it owns. The map is moved, never copied.
        (Some(mut map), Some(version)) => {
            if let Some(ours) = v.as_object() {
                for (key, value) in ours {
                    map.insert(key.clone(), value.clone());
                }
            }
            map.insert("schema".to_string(), json!(version));
            (Value::Object(map), SaveOutcome::MergedIntoNewer)
        }
        _ => (v, SaveOutcome::Written),
    };
    match serde_json::to_string_pretty(&out) {
        Ok(text) => match fs::write(dir.join("settings.json"), text) {
            Ok(()) => outcome,
            Err(_) => SaveOutcome::Failed,
        },
        Err(_) => SaveOutcome::Failed,
    }
}
