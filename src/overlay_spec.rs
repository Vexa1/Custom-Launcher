//! The JSON panels the overlay window draws, assembled without egui, a Roblox session or any
//! platform API.
//!
//! [`crate::game_overlay`] feeds this from the launcher's live state once per tick and writes
//! the result to `overlay-panels-<id>.json`; the overlay window parses exactly these objects
//! (`overlay_window::reload_spec`). Keeping the assembly here is what makes the transition that
//! used to break - "no server yet" to "server resolved, region known, live ping measured" -
//! testable in `vm-tests`, where the GUI crate cannot be linked.

use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct ServerPanel {
    pub corner: u8,
    /// 0 keeps the panel until the game closes.
    pub duration_secs: u32,
    pub show_ip: bool,
    pub show_job_id: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatsPanel {
    pub corner: u8,
    pub cpu: bool,
    pub ram: bool,
    pub ping: bool,
    pub session: bool,
    pub clock: bool,
    pub fps_cap: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlagsPanel {
    pub corner: u8,
    pub max_lines: usize,
    pub preset: String,
    /// Pre-rendered `Key = value` lines of every FastFlag CREBLOX writes.
    pub flags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GamePanel {
    pub corner: u8,
}

/// Which panels are switched on, and where they sit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Panels<'a> {
    pub server: Option<&'a ServerPanel>,
    pub stats: Option<&'a StatsPanel>,
    pub flags: Option<&'a FlagsPanel>,
    pub game: Option<&'a GamePanel>,
}

#[derive(Clone, Copy, Debug)]
pub struct LiveServer<'a> {
    pub ip: &'a str,
    pub job_id: &'a str,
    pub place_id: u64,
}

/// Everything the panels say that is not configuration: the resolved server, its region, the
/// live ping and the sampled numbers.
#[derive(Clone, Debug)]
pub struct Live<'a> {
    /// The server the player is in. `None` while the player log has not named one yet, which
    /// is what the HUD shows as `Server info unavailable`.
    pub server: Option<LiveServer<'a>>,
    /// Seconds since that server was resolved: the panel's own age and the `In server` timer.
    pub server_age_secs: u64,
    /// Seconds since this session started, for the `Session` row.
    pub session_secs: u64,
    /// Seconds since the overlay started: how long `Detecting server...` is shown.
    pub detect_secs: u64,
    /// Geolocated region of `server.ip`, e.g. `Sydney, AU`. `None` while locating.
    pub region: Option<&'a str>,
    /// Live RTT against the game server. `None` is the HUD's `-- ms`.
    pub ping_ms: Option<u32>,
    /// `(game name, players online)` once the place was looked up.
    pub game: Option<(&'a str, Option<u64>)>,
    pub cpu_pct: Option<f32>,
    pub ram_mb: Option<f32>,
    pub clock: String,
    /// The position editor's plainly-marked sample HUD, for when no game is running.
    pub edit_preview: bool,
}

/// Builds the `panels` array the overlay window draws.
pub fn build(panels: &Panels, live: &Live) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();

    // The server panel is drawn even before the server is known. With only this overlay
    // enabled (the default) an unresolved server used to mean *no panels at all*, so the
    // overlay window was on screen but completely empty - which looked exactly like "the
    // overlay does not work".
    if let Some(sp) = panels.server {
        match &live.server {
            Some(s) => {
                let visible = sp.duration_secs == 0
                    || live.server_age_secs < sp.duration_secs as u64;
                if visible {
                    let title = match live.region {
                        Some(loc) => format!("Server: {loc}"),
                        None => "Server: locating...".to_string(),
                    };
                    let mut lines = Vec::new();
                    if sp.show_ip {
                        lines.push(s.ip.to_string());
                    }
                    if sp.show_job_id && !s.job_id.is_empty() {
                        lines.push(format!("Job ID: {}", s.job_id));
                    }
                    out.push(json!({
                        "id": "server",
                        "corner": sp.corner,
                        "title": title,
                        "lines": lines
                    }));
                }
            }
            None => {
                // No duration here: this shows until a server is detected, so there is
                // always something on screen as soon as the game starts. If the log never
                // yields an address, say so instead of spinning on "Detecting..." forever.
                let text = if live.detect_secs < 20 {
                    "Detecting server..."
                } else {
                    "Server info unavailable"
                };
                out.push(json!({
                    "id": "server",
                    "corner": sp.corner,
                    "title": "CREBLOX overlay",
                    "lines": [text]
                }));
            }
        }
    }

    if live.edit_preview && live.server.is_none() {
        let corner = panels.server.map(|s| s.corner).unwrap_or(3);
        out.push(json!({
            "id": "preview",
            "corner": corner,
            "title": "CREBLOX overlay · preview",
            "lines": [
                "Region: Preview",
                "Ping: 42 ms",
                "CPU: 12%",
                "RAM: 640 MB",
                "Flags: Competitive (6)"
            ]
        }));
    }

    if let (Some(gp), Some(s)) = (panels.game, &live.server) {
        let title = match &live.game {
            Some((name, _)) => shorten(name, 36),
            None => "Game".to_string(),
        };
        let mut lines = Vec::new();
        if s.place_id != 0 {
            lines.push(format!("Place ID {}", s.place_id));
        }
        if let Some((_, Some(playing))) = &live.game {
            lines.push(format!("Playing  {playing}"));
        }
        lines.push(format!("In server {}", fmt_secs(live.server_age_secs)));
        out.push(json!({"id": "game", "corner": gp.corner, "title": title, "lines": lines}));
    }

    if let Some(st) = panels.stats {
        let mut lines: Vec<String> = Vec::new();
        if st.cpu || st.ram {
            if st.cpu {
                lines.push(match live.cpu_pct {
                    Some(cpu) => format!("CPU      {cpu:.0}%"),
                    None => "CPU      --".to_string(),
                });
            }
            if st.ram {
                lines.push(match live.ram_mb {
                    Some(mb) => format!("RAM      {mb:.0} MB"),
                    None => "RAM      --".to_string(),
                });
            }
        }
        if st.ping {
            lines.push(ping_row(live.ping_ms));
        }
        if let Some(fps) = st.fps_cap {
            lines.push(format!("FPS cap  {fps}"));
        }
        if st.session {
            lines.push(format!("Session  {}", fmt_secs(live.session_secs)));
        }
        if st.clock {
            lines.push(format!("Time     {}", live.clock));
        }
        if !lines.is_empty() {
            out.push(json!({
                "id": "stats",
                "corner": st.corner,
                "title": "Roblox stats",
                "lines": lines
            }));
        }
    }

    if let Some(fl) = panels.flags {
        let total = fl.flags.len();
        let mut lines: Vec<String> = fl.flags.iter().take(fl.max_lines).cloned().collect();
        if total > fl.max_lines {
            lines.push(format!("+{} more", total - fl.max_lines));
        }
        if lines.is_empty() {
            lines.push("No FastFlags are active".to_string());
        }
        out.push(json!({
            "id": "flags",
            "corner": fl.corner,
            "title": format!("FastFlags {} ({total})", fl.preset),
            "lines": lines
        }));
    }

    out
}

/// A HUD switch the tray's check items own. The menu item and the panel the overlay draws
/// are the same switch, and a click has to change both `settings.json` and the JSON the HUD
/// reads. Keeping that mapping here (no egui, no `Settings`) is what lets `vm-tests` prove a
/// click removes or restores the panel in the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayToggle {
    Server,
    Stats,
    Crosshair,
}

/// The on/off state of the switches the tray owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudToggles {
    pub server: bool,
    pub stats: bool,
    pub crosshair: bool,
}

impl HudToggles {
    /// The state the menu shows after the user clicks `toggle`.
    pub fn flipped(self, toggle: TrayToggle) -> Self {
        match toggle {
            TrayToggle::Server => Self {
                server: !self.server,
                ..self
            },
            TrayToggle::Stats => Self {
                stats: !self.stats,
                ..self
            },
            TrayToggle::Crosshair => Self {
                crosshair: !self.crosshair,
                ..self
            },
        }
    }
}

/// How close two copies of one click can be and still be the same physical click. Two real
/// clicks on one check item cannot be this close: the menu closes after the first one.
pub const CLICK_DEBOUNCE: Duration = Duration::from_millis(150);

/// Drops a duplicate delivery of one tray click.
///
/// macOS hands a status-menu action to every reader of the global menu channel (the tray
/// thread and the window's drain both read it), so one click can arrive twice within
/// milliseconds. Flipping on the second copy would undo the first - the switch and the HUD
/// would blink back to where they were - so a click is accepted once per window.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClickDebounce {
    last: Option<(TrayToggle, Instant)>,
}

impl ClickDebounce {
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// `true` for the first sighting of a physical click, `false` for its duplicate.
    pub fn accept(&mut self, toggle: TrayToggle, at: Instant) -> bool {
        if let Some((last, when)) = self.last {
            if last == toggle && at.saturating_duration_since(when) < CLICK_DEBOUNCE {
                return false;
            }
        }
        self.last = Some((toggle, at));
        true
    }
}

/// Adds or removes the `crosshair` entry of a spec, exactly as the tray's Crosshair item does.
///
/// The overlay treats a missing key as "no crosshair", so removing it - not writing an empty
/// object - is what stops it being drawn on the next reload.
pub fn set_crosshair(spec: &mut Value, crosshair: Option<Value>) {
    match crosshair {
        Some(c) => spec["crosshair"] = c,
        None => {
            if let Some(obj) = spec.as_object_mut() {
                obj.remove("crosshair");
            }
        }
    }
}

/// The `Ping` row: the live game-server RTT, or `-- ms` while there is no measurement.
pub fn ping_row(ping_ms: Option<u32>) -> String {
    match ping_ms {
        Some(ms) => format!("Ping     {ms} ms"),
        None => "Ping     -- ms".to_string(),
    }
}

/// The whole file the overlay window reads.
pub fn spec_json(
    opacity: f32,
    game_active: bool,
    offset_x: i16,
    offset_y: i16,
    edit: bool,
    panels: Vec<Value>,
) -> Value {
    json!({
        "opacity": opacity,
        "game_active": game_active,
        "offset_x": offset_x,
        "offset_y": offset_y,
        "edit": edit,
        "panels": panels
    })
}

/// `m:ss`, or `h:mm:ss` once the session passes an hour.
pub fn fmt_secs(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Trims a label to `max` characters with a trailing ellipsis.
pub fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
