//! Per-game playtime, saved to `playtime.json` in the app data folder.
//!
//! Time is only added when a session actually ends, which is what the launcher screen
//! shows: "Played 3h 12m". Nothing here runs while you are playing - the start time is an
//! `Instant` and the single addition happens on the way out, so tracking costs nothing.

use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::auth::data_dir;

#[derive(Clone, Copy, Debug, Default)]
pub struct GamePlay {
    /// Total seconds played, all sessions.
    pub seconds: u64,
    /// Unix timestamp of the last session (0 = never).
    pub last_played: u64,
}

pub fn load() -> HashMap<u64, GamePlay> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string(data_dir().join("playtime.json")) else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return out;
    };
    let Some(obj) = v.as_object() else {
        return out;
    };
    out.reserve(obj.len());
    for (key, entry) in obj {
        let Ok(universe_id) = key.parse::<u64>() else {
            continue;
        };
        out.insert(
            universe_id,
            GamePlay {
                seconds: entry["seconds"].as_u64().unwrap_or(0),
                last_played: entry["last_played"].as_u64().unwrap_or(0),
            },
        );
    }
    out
}

pub fn save(map: &HashMap<u64, GamePlay>) {
    let mut obj = serde_json::Map::with_capacity(map.len());
    for (id, p) in map {
        obj.insert(
            id.to_string(),
            json!({ "seconds": p.seconds, "last_played": p.last_played }),
        );
    }
    let text = Value::Object(obj).to_string();
    // Written next to the real file and renamed, so a crash never leaves half a file behind.
    let path = data_dir().join("playtime.json");
    let tmp = data_dir().join("playtime.json.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "0m", "7m", "2h 05m", "13h 40m" - hours and minutes only, as asked.
pub fn format_duration(secs: u64) -> String {
    let total_minutes = secs / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}
