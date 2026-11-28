//! The JSON panels the overlay window draws, assembled without egui, a Roblox session or any
//! platform API.
//!
//! [`crate::game_overlay`] feeds this from the launcher's live state once per tick and writes
//! the result to `overlay-panels-<id>.json`; the overlay window parses exactly these objects
//! (`overlay_window::reload_spec`). Keeping the assembly here is what makes the transition that
//! used to break - "no server yet" to "server resolved, region known, live ping measured" -
//! testable in `vm-tests`, where the GUI crate cannot be linked.

use serde_json::{Value, json};
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
    /// Seconds since that server was resolved: the panel's own age and how long
    /// `Detecting server...` is shown. The `In server` row is ticked by the overlay.
    pub server_age_secs: u64,
    /// Unix-ms anchor the `In server` and `Session` rows count from. It is written to the
    /// panels file as `joined_at_unix_ms` and is the only thing the launcher supplies: the
    /// overlay window computes the elapsed time itself, so the clock never waits for the
    /// file to be rewritten and never jumps by however long a tick happened to take.
    pub joined_at_unix_ms: u64,
    /// Seconds since the overlay started: how long `Detecting server...` is shown.
    pub detect_secs: u64,
    /// Geolocated region of `server.ip`, e.g. `Sydney, AU`. `None` while locating.
    pub region: Option<&'a str>,
    /// Live RTT against the game server. `None` is the HUD's `-- ms`.
    pub ping_ms: Option<u32>,
    /// The probe path that came back empty (`tcp`, `room list`, ...), drawn next to `-- ms`
    /// so the HUD itself says what failed instead of leaving that to `log.txt`.
    pub ping_path: Option<&'a str>,
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
                let visible =
                    sp.duration_secs == 0 || live.server_age_secs < sp.duration_secs as u64;
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
        // A token, not a time: `overlay_window` fills it from the anchor below on every
        // frame it draws, so the row ticks on the second even when nothing rewrites this file.
        lines.push(format!("In server {ELAPSED}"));
        out.push(json!({
            "id": "game",
            "corner": gp.corner,
            "title": title,
            "lines": lines,
            "joined_at_unix_ms": live.joined_at_unix_ms
        }));
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
            lines.push(ping_row(live.ping_ms, live.ping_path));
        }
        if let Some(fps) = st.fps_cap {
            lines.push(format!("FPS cap  {fps}"));
        }
        if st.session {
            lines.push(format!("Session  {ELAPSED}"));
        }
        if st.clock {
            lines.push(format!("Time     {}", live.clock));
        }
        if !lines.is_empty() {
            let mut panel = json!({
                "id": "stats",
                "corner": st.corner,
                "title": "Roblox stats",
                "lines": lines
            });
            if st.session {
                panel["joined_at_unix_ms"] = json!(live.joined_at_unix_ms);
            }
            out.push(panel);
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
    /// The party member strip.
    Party,
}

/// The on/off state of the switches the tray owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudToggles {
    pub server: bool,
    pub stats: bool,
    pub crosshair: bool,
    pub party: bool,
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
            TrayToggle::Party => Self {
                party: !self.party,
                ..self
            },
        }
    }
}

/// The party card: a strip of member tiles, each a rounded square with a 2 px outline in the
/// colour of that member's state.
///
/// Lives here, not in the overlay window, so `vm-tests` can assert the exact bytes the HUD
/// receives for a member who is in the server, one who is loading and one who dropped.
pub fn party_panel(corner: u8, tiles: &[crate::party_hud::Tile]) -> Value {
    let tiles: Vec<Value> = tiles
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "initials": t.initials,
                "status": t.status.key(),
                "headshot": t.headshot,
            })
        })
        .collect();
    json!({
        "id": "party",
        "corner": corner.min(3),
        "title": format!("Party {}", tiles.len()),
        "tiles": tiles
    })
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

/// The panel positions the editor reported, or the legacy single offset folded into the
/// server card.
///
/// Before per-panel positions there was one offset for the whole stack. An old settings file
/// still loads, and its saved spot moves the card that used to own the stack instead of being
/// dropped on the floor.
pub fn parse_panel_offsets(v: &Value, legacy_x: i16, legacy_y: i16) -> Vec<PanelOffset> {
    match v.as_array() {
        Some(arr) => arr
            .iter()
            .filter_map(|p| {
                Some(PanelOffset {
                    id: p["id"].as_str()?.to_string(),
                    x: p["x"].as_i64().unwrap_or(0).clamp(-8000, 8000) as i16,
                    y: p["y"].as_i64().unwrap_or(0).clamp(-8000, 8000) as i16,
                })
            })
            .collect(),
        None if legacy_x != 0 || legacy_y != 0 => vec![PanelOffset {
            id: "server".to_string(),
            x: legacy_x,
            y: legacy_y,
        }],
        None => Vec::new(),
    }
}

/// The `Ping` row: the live game-server RTT, or `-- ms` with the path that failed.
///
/// `Ping     -- ms (tcp)` is the same fact `log.txt` records, shown where the player is
/// already looking: which probe was last tried and left without a number.
pub fn ping_row(ping_ms: Option<u32>, path: Option<&str>) -> String {
    match (ping_ms, path) {
        (Some(ms), _) => format!("Ping     {ms} ms"),
        (None, Some(path)) => format!("Ping     -- ms ({path})"),
        (None, None) => "Ping     -- ms".to_string(),
    }
}

/// Writes the panels file through a temporary file and a rename, so the overlay window can
/// never read a half-written spec while switching Spaces. Returns whether the new bytes were
/// installed.
///
/// Lives here rather than in `game_overlay` so the integration test can prove the file it
/// renders came from this exact writer, not from a test-only copy of it.
pub fn write_atomic(path: &std::path::Path, text: &str) -> bool {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_err() {
        return false;
    }
    std::fs::rename(&tmp, path).is_ok()
}

/// One HUD panel's position, in points from its own corner anchor.
///
/// Every panel moves on its own: the server card, the stats card and the rest each carry one
/// of these in `settings.json` and in `overlay-panels-<id>.json`. `0, 0` is the panel's corner
/// slot, which is already stacked against the other panels in that corner.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PanelOffset {
    pub id: String,
    pub x: i16,
    pub y: i16,
}

/// Writes every panel's saved `(x, y)` into the panels JSON the overlay window reads.
///
/// A panel with no saved entry is written as `0, 0`, so the overlay never has to guess. The
/// values are clamped to the range the editor can produce - a hand-edited settings file can
/// then never park a card a million points off-screen.
pub fn apply_panel_offsets(panels: &mut [Value], offsets: &[PanelOffset]) {
    for panel in panels {
        let id = panel["id"].as_str().unwrap_or_default().to_string();
        let found = offsets.iter().rev().find(|o| o.id == id);
        let (x, y) = found
            .map(|o| (o.x.clamp(-8000, 8000), o.y.clamp(-8000, 8000)))
            .unwrap_or((0, 0));
        panel["offset_x"] = json!(x);
        panel["offset_y"] = json!(y);
    }
}

/// The custom image's box on screen, in points from the game window's top-left corner.
///
/// The editor owns this arithmetic so the drag a user performs in the overlay and the position
/// the launcher stores can never disagree: one formula, run in `vm-tests` too. Corners are
/// `0` top-left, `1` top-right, `2` bottom-left, `3` bottom-right, the same order the settings
/// page lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ImageBox {
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
}

impl ImageBox {
    /// Never smaller than this: a 4 px image is impossible to grab again.
    pub const MIN: i16 = 24;
    /// The gap between the image and the window edge in its default corner slot.
    pub const MARGIN: f32 = 16.0;

    /// The size to draw: the saved box when it has one, otherwise the height percentage kept
    /// at the picture's own aspect ratio.
    pub fn size(texture: (f32, f32), pct: u8, view_h: f32, saved: (u16, u16)) -> (f32, f32) {
        if saved.0 >= Self::MIN as u16 && saved.1 >= Self::MIN as u16 {
            return (saved.0 as f32, saved.1 as f32);
        }
        let (tw, th) = texture;
        if tw <= 0.0 || th <= 0.0 {
            return (0.0, 0.0);
        }
        let target_h = (view_h * (pct as f32 / 100.0)).clamp(8.0, view_h.max(8.0));
        (tw * (target_h / th), target_h)
    }

    /// Where the image sits before anyone drags it: its corner slot plus the saved offsets.
    pub fn placed(
        corner: u8,
        size: (f32, f32),
        view: (f32, f32),
        offset: (f32, f32),
    ) -> Self {
        let base_x = if corner % 2 == 1 {
            view.0 - size.0 - Self::MARGIN
        } else {
            Self::MARGIN
        };
        let base_y = if corner < 2 {
            Self::MARGIN
        } else {
            view.1 - size.1 - Self::MARGIN
        };
        Self {
            x: (base_x + offset.0).round() as i16,
            y: (base_y + offset.1).round() as i16,
            w: size.0.round().max(Self::MIN as f32) as i16,
            h: size.1.round().max(Self::MIN as f32) as i16,
        }
        .clamped(view)
    }

    /// Keeps the whole box inside the view, shrinking only when it is larger than the view.
    pub fn clamped(self, view: (f32, f32)) -> Self {
        let mut w = self.w.max(Self::MIN);
        let mut h = self.h.max(Self::MIN);
        w = w.min((view.0 as i16).max(Self::MIN));
        h = h.min((view.1 as i16).max(Self::MIN));
        let x = self.x.clamp(0, (view.0 as i16 - w).max(0));
        let y = self.y.clamp(0, (view.1 as i16 - h).max(0));
        Self { x, y, w, h }
    }

    /// Drags the whole box, keeping it fully on screen.
    pub fn moved(self, dx: f32, dy: f32, view: (f32, f32)) -> Self {
        Self {
            x: self.x.saturating_add(dx.round() as i16),
            y: self.y.saturating_add(dy.round() as i16),
            ..self
        }
        .clamped(view)
    }

    /// Drags one corner. The opposite corner stays put, the box never goes below
    /// [`ImageBox::MIN`] and `lock` keeps the aspect ratio it started with by deriving the
    /// height from the width.
    pub fn resized(self, corner: u8, dx: f32, dy: f32, view: (f32, f32), lock: bool) -> Self {
        let (mut x, mut y, mut w, mut h) = (self.x, self.y, self.w, self.h);
        let (dx, dy) = (dx.round() as i16, dy.round() as i16);
        if corner == 0 || corner == 2 {
            x = x.saturating_add(dx);
            w = w.saturating_sub(dx);
        } else {
            w = w.saturating_add(dx);
        }
        if corner < 2 {
            y = y.saturating_add(dy);
            h = h.saturating_sub(dy);
        } else {
            h = h.saturating_add(dy);
        }
        if lock && self.h > 0 {
            let aspect = self.w as f32 / self.h as f32;
            h = (w as f32 / aspect).round() as i16;
            if corner < 2 {
                y = self.y + self.h - h;
            }
        }
        // Below the minimum, keep the fixed edge and grow back from it.
        if w < Self::MIN {
            if corner == 0 || corner == 2 {
                x = self.x + self.w - Self::MIN;
            }
            w = Self::MIN;
        }
        if h < Self::MIN {
            if corner < 2 {
                y = self.y + self.h - Self::MIN;
            }
            h = Self::MIN;
        }
        Self { x, y, w, h }.clamped(view)
    }
}

/// Which part of the custom image a press landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImagePart {
    /// The picture itself: a drag moves the whole box.
    Body,
    /// A corner grip: `0` top-left, `1` top-right, `2` bottom-left, `3` bottom-right.
    Corner(u8),
}

/// The grab target around each corner, in points. Twice the 8 px square the editor draws,
/// which is the smallest hit target the app allows anywhere else.
pub const IMAGE_GRIP: f32 = 24.0;

/// Resolves a press at `pos` (window space) against the image box `(x, y, w, h)`.
///
/// Corners are checked first and always win over the body, so a handle consumes the drag it
/// started even though it overlaps the picture. `None` means the press missed the image and
/// the selection frame around it.
pub fn image_part(rect: (f32, f32, f32, f32), pos: (f32, f32)) -> Option<ImagePart> {
    let (x, y, w, h) = rect;
    let half = IMAGE_GRIP * 0.5;
    for (i, (cx, cy)) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
        .into_iter()
        .enumerate()
    {
        if (pos.0 - cx).abs() <= half && (pos.1 - cy).abs() <= half {
            return Some(ImagePart::Corner(i as u8));
        }
    }
    (pos.0 >= x && pos.0 <= x + w && pos.1 >= y && pos.1 <= y + h).then_some(ImagePart::Body)
}

/// Applies one frame of a drag: the body moves, a corner resizes from the opposite edge.
///
/// Shared by the overlay window and the pointer simulation in `vm-tests`, so the drag the
/// user performs and the drag the test asserts can never follow different arithmetic.
pub fn drag_image(
    box_: ImageBox,
    part: ImagePart,
    dx: f32,
    dy: f32,
    view: (f32, f32),
    lock: bool,
) -> ImageBox {
    match part {
        ImagePart::Body => box_.moved(dx, dy, view),
        ImagePart::Corner(i) => box_.resized(i, dx, dy, view, lock),
    }
}

/// One pointer drag on the custom image, resolved once at press.
///
/// The overlay window and the pointer simulation both keep this, so "which part did I grab"
/// and "how far did the pointer move this frame" cannot drift apart. The grab is held even
/// when the pointer leaves the grip, and each motion reports only its own delta - applying the
/// total drag on every frame would make the box accelerate away from the cursor.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImageDrag {
    part: Option<ImagePart>,
    last: Option<(f32, f32)>,
}

impl ImageDrag {
    /// Grabs the image at `pos` inside `rect`; false means the press missed it.
    pub fn press(&mut self, rect: (f32, f32, f32, f32), pos: (f32, f32)) -> bool {
        self.part = image_part(rect, pos);
        self.last = self.part.map(|_| pos);
        self.part.is_some()
    }

    /// The pointer moved to `pos`: the delta since the previous frame, once. `None` when no
    /// drag is held, so a motion without a press never moves anything.
    pub fn motion(&mut self, pos: (f32, f32)) -> Option<(f32, f32)> {
        self.part?;
        let last = self.last.replace(pos)?;
        Some((pos.0 - last.0, pos.1 - last.1))
    }

    /// What the held drag grabbed, if anything.
    pub fn part(&self) -> Option<ImagePart> {
        self.part
    }

    /// Ends the drag; the next press decides a new part.
    pub fn release(&mut self) {
        self.part = None;
        self.last = None;
    }
}

/// The custom image's saved box as it appears in `settings.json`.
///
/// Both sides of the file go through this type: the loader reads it (including the 0.5.51 file
/// that kept the position in the offset sliders) and the saver writes it, so the migration and
/// the round trip are the same code the tests exercise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SavedImageBox {
    pub x: i16,
    pub y: i16,
    /// `0` means no saved box: the picture follows its corner, height and offsets.
    pub w: u16,
    pub h: u16,
}

impl SavedImageBox {
    /// Reads the four fields out of a settings object of a given file version.
    ///
    /// Version 1 stored a dragged position in the offset sliders, so a box without `x`/`y` is
    /// read from them once. From version 2 the position is only ever in `x`/`y`, and a missing
    /// one means `0` - the offsets are sliders again, in every case.
    pub fn from_settings_at(v: &Value, version: u64) -> Self {
        Self::read(
            v,
            ("overlay_image_x", "overlay_image_y"),
            ("overlay_image_offset_x", "overlay_image_offset_y"),
            ("overlay_image_w", "overlay_image_h"),
            version < 2,
        )
    }

    /// Reads the same box out of `overlay-panels-<id>.json`'s `image` object, which uses the
    /// short keys.
    pub fn from_panels_image(v: &Value) -> Self {
        Self::read(v, ("x", "y"), ("offset_x", "offset_y"), ("w", "h"), true)
    }

    fn read(
        v: &Value,
        pos: (&str, &str),
        legacy: (&str, &str),
        dims: (&str, &str),
        legacy_offsets: bool,
    ) -> Self {
        let dim = |key: &str| {
            v[key]
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|n| *n >= ImageBox::MIN as u16)
                .unwrap_or(0)
        };
        let (w, h) = (dim(dims.0), dim(dims.1));
        let has_box = w > 0 && h > 0;
        // The box's own position, or - for a version 1 file, which had no position fields -
        // the offsets it stored the dragged position in. Without a box the offsets are the
        // corner sliders' values and are not the picture's position.
        let axis = |pos: &str, legacy: &str| -> i16 {
            let stored = v[pos].as_i64().or_else(|| {
                (legacy_offsets && has_box)
                    .then(|| v[legacy].as_i64())
                    .flatten()
            });
            stored.unwrap_or(0).clamp(0, 8000) as i16
        };
        Self {
            x: axis(pos.0, legacy.0),
            y: axis(pos.1, legacy.1),
            w,
            h,
        }
    }

    /// True when the editor has frozen a box, i.e. the corner and height sliders no longer apply.
    pub fn is_saved(&self) -> bool {
        self.w >= ImageBox::MIN as u16 && self.h >= ImageBox::MIN as u16
    }

    /// The four `settings.json` keys.
    pub fn insert_into(&self, out: &mut serde_json::Map<String, Value>) {
        out.insert("overlay_image_x".to_string(), json!(self.x));
        out.insert("overlay_image_y".to_string(), json!(self.y));
        out.insert("overlay_image_w".to_string(), json!(self.w));
        out.insert("overlay_image_h".to_string(), json!(self.h));
    }

}

/// The whole file the overlay window reads.
pub fn spec_json(opacity: f32, game_active: bool, edit: bool, panels: Vec<Value>) -> Value {
    json!({
        "opacity": opacity,
        "game_active": game_active,
        "edit": edit,
        "panels": panels
    })
}

/// Where a live timer's value belongs in a HUD line.
///
/// The launcher writes this token instead of a time; the overlay window replaces it from the
/// panel's `joined_at_unix_ms` while it draws. A formatted time in the file would only move
/// when the launcher happened to rewrite it - which is how the timer froze and then jumped.
pub const ELAPSED: &str = "{elapsed}";

/// Milliseconds since the Unix epoch: the anchor every panel timer counts from.
pub fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Whole seconds since `started_at_ms`. A clock that moved backwards reads as zero rather
/// than wrapping, and the anchor is never older than the session it belongs to.
pub fn elapsed_secs(started_at_ms: u64, now_ms: u64) -> u64 {
    now_ms.saturating_sub(started_at_ms) / 1000
}

/// Milliseconds until the next whole second of the timer, always `1..=1000`.
///
/// The overlay wakes once per boundary with this. A fixed 500 ms cadence would draw the same
/// second twice and still show the change up to half a second late; a second from the
/// boundary lands on it every time and costs one frame per second in between.
pub fn ms_until_next_second(started_at_ms: u64, now_ms: u64) -> u64 {
    1000 - now_ms.saturating_sub(started_at_ms) % 1000
}

/// Whether a HUD line carries a live timer.
pub fn has_elapsed(line: &str) -> bool {
    line.contains(ELAPSED)
}

/// A HUD line with [`ELAPSED`] replaced by the live `m:ss` / `h:mm:ss` value.
pub fn with_elapsed(line: &str, started_at_ms: u64, now_ms: u64) -> String {
    if !has_elapsed(line) {
        return line.to_string();
    }
    line.replace(
        ELAPSED,
        &fmt_secs(elapsed_secs(started_at_ms, now_ms)),
    )
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
