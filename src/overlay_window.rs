//! The in-game overlay window (pure Rust).
//!
//! Started as `creblox --overlay-window <spec.json>` by [`crate::game_overlay`]. It opens one
//! transparent, borderless, click-through window that is kept exactly on top of the Roblox
//! game window - also in a fullscreen Space - and draws the overlay panels and the crosshair
//! into it with egui. The panel contents come from the JSON spec file that the launcher (or the
//! headless companion) rewrites about once a second:
//!
//! ```json
//! { "opacity": 0.9, "quit": false,
//!   "panels": [ { "id": "server", "corner": 3, "title": "...", "lines": ["..."] } ],
//!   "crosshair": { "style": 3, "size": 18, "thickness": 2, "gap": 4,
//!                  "color": [255, 60, 60], "opacity": 0.9, "offset_x": 0, "offset_y": 0 } }
//! ```
//! Corners: 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, pos2, vec2};
use serde_json::Value;

// ---------------------------------------------------------------------------
// CoreGraphics: find the Roblox game window
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *const c_void;
    fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut CGRect) -> u8;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    static kCGWindowOwnerName: *const c_void;
    static kCGWindowBounds: *const c_void;
    static kCGWindowLayer: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFNumberGetValue(number: *const c_void, kind: i32, out: *mut c_void) -> u8;
    fn CFRelease(obj: *const c_void);
}

const EXCLUDE_DESKTOP_ELEMENTS: u32 = 16;
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT32: i32 = 3;

/// Bounds (in points, origin top-left of the primary display) of the largest Roblox window.
///
/// Every window is listed, not only the ones on the current Space: Roblox in fullscreen
/// lives on its own Space, and window layers differ between windowed, borderless and
/// fullscreen modes, so a layer-0 on-screen search can come back empty while the game is
/// right there. CREBLOX's own windows are skipped.
fn find_roblox_window() -> Option<CGRect> {
    unsafe {
        let list = CGWindowListCopyWindowInfo(EXCLUDE_DESKTOP_ELEMENTS, 0);
        if list.is_null() {
            return None;
        }
        let mut best: Option<(CGRect, i32, f64)> = None;
        let count = CFArrayGetCount(list);
        for i in 0..count {
            let dict = CFArrayGetValueAtIndex(list, i);
            if dict.is_null() {
                continue;
            }
            let name = CFDictionaryGetValue(dict, kCGWindowOwnerName);
            if name.is_null() {
                continue;
            }
            let mut buf = [0 as c_char; 128];
            if CFStringGetCString(
                name,
                buf.as_mut_ptr(),
                buf.len() as isize,
                CF_STRING_ENCODING_UTF8,
            ) == 0
            {
                continue;
            }
            let owner = CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .to_ascii_lowercase();
            if !owner.contains("roblox") {
                continue;
            }
            let layer_ref = CFDictionaryGetValue(dict, kCGWindowLayer);
            let mut layer: i32 = 0;
            if !layer_ref.is_null() {
                CFNumberGetValue(layer_ref, CF_NUMBER_SINT32, (&mut layer as *mut i32).cast());
            }
            // Floating helper windows (splash / update prompts) are small and layered;
            // the game window is the big one, so prefer normal-level windows only when
            // several candidates are the same size.
            if layer != 0 && layer != 3 {
                continue;
            }
            let bounds = CFDictionaryGetValue(dict, kCGWindowBounds);
            if bounds.is_null() {
                continue;
            }
            let mut rect = CGRect::default();
            if CGRectMakeWithDictionaryRepresentation(bounds, &mut rect) == 0 {
                continue;
            }
            if rect.w < 200.0 || rect.h < 150.0 {
                continue;
            }
            let area = rect.w * rect.h;
            let better = match best {
                None => true,
                Some((_, best_layer, best_area)) => {
                    if (layer == 0) != (best_layer == 0) {
                        layer == 0
                    } else {
                        area > best_area
                    }
                }
            };
            if better {
                best = Some((rect, layer, area));
            }
        }
        CFRelease(list);
        best.map(|(rect, _, _)| rect)
    }
}

/// Bounds of the main display, used when the Roblox window itself cannot be located.
///
/// Getting another app's windows out of `CGWindowListCopyWindowInfo` needs Screen
/// Recording permission on modern macOS; without it the list can come back without
/// Roblox in it. Falling back to the display means the overlays still appear (covering
/// the game screen) instead of the whole overlay silently drawing nothing.
fn main_display_bounds() -> Option<CGRect> {
    unsafe {
        let rect = CGDisplayBounds(CGMainDisplayID());
        (rect.w > 0.0 && rect.h > 0.0).then_some(rect)
    }
}

// ---------------------------------------------------------------------------
// AppKit tweaks through the Objective-C runtime (no extra crates)
// ---------------------------------------------------------------------------

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}

type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;
type Msg1F = unsafe extern "C" fn(*mut c_void, *mut c_void, f64) -> *mut c_void;

unsafe fn msg0(obj: *mut c_void, sel: &str) -> *mut c_void {
    let Ok(name) = CString::new(sel) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let f: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel_registerName(name.as_ptr()))
    }
}

unsafe fn msg1(obj: *mut c_void, sel: &str, arg: isize) -> *mut c_void {
    let Ok(name) = CString::new(sel) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let f: Msg1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel_registerName(name.as_ptr()), arg)
    }
}

/// `objc_msgSend` for a `CGFloat` (f64) argument, such as `setAlphaValue:`.
unsafe fn msg1_f(obj: *mut c_void, sel: &str, arg: f64) -> *mut c_void {
    let Ok(name) = CString::new(sel) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let f: Msg1F = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel_registerName(name.as_ptr()), arg)
    }
}

/// Makes the overlay window float above everything - including fullscreen games - without a
/// Dock icon, shadow or mouse handling. Returns false while the window does not exist yet.
fn tweak_overlay_window() -> bool {
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return false;
        }
        let app = msg0(class, "sharedApplication");
        if app.is_null() {
            return false;
        }
        msg1(app, "setActivationPolicy:", 1); // accessory: no Dock icon, never takes focus
        let windows = msg0(app, "windows");
        if windows.is_null() {
            return false;
        }
        let count = msg0(windows, "count") as usize;
        if count == 0 {
            return false;
        }
        for i in 0..count {
            let window = msg1(windows, "objectAtIndex:", i as isize);
            if window.is_null() {
                continue;
            }
            msg1(window, "setLevel:", 1000); // above the fullscreen window level
            msg1(window, "setCollectionBehavior:", 1 | 16 | 256); // all Spaces | stationary | over fullscreen
            msg1(window, "setIgnoresMouseEvents:", 1);
            msg1(window, "setHasShadow:", 0);
            msg1(window, "setHidesOnDeactivate:", 0);
            msg1(window, "setCanHide:", 0);
            msg1(window, "setOpaque:", 0);
            // Show up without the app being active; without this the overlay can be
            // created and positioned yet never ordered onto the screen.
            msg0(window, "orderFrontRegardless");
        }
        true
    }
}

// ---------------------------------------------------------------------------
// The overlay app
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// HUD design tokens
// ---------------------------------------------------------------------------
//
// Deliberately plain: one obsidian surface, one hairline, two text weights. No glows, no
// gradients, no accent colour per metric - the only colour on screen is a 5 px dot whose
// hue carries meaning (ping health) and the crosshair the user chose.

/// `rgba(14, 15, 18, 0.82)` at full opacity; scaled by the user's overlay opacity at draw time.
const SURFACE_RGB: (u8, u8, u8) = (14, 15, 18);
const SURFACE_ALPHA: f32 = 0.82;
/// The 1 px inner stroke that separates the panel from bright or dark scenes.
const HAIRLINE_ALPHA: f32 = 0.09;
/// Row separators sit below the panel hairline so the edge always reads first.
const DIVIDER_ALPHA: f32 = 0.06;
const LABEL_RGB: (u8, u8, u8) = (0x8E, 0x93, 0x9E);
const VALUE_RGB: (u8, u8, u8) = (0xF3, 0xF4, 0xF6);
const DOT_OK_RGB: (u8, u8, u8) = (0x34, 0xD3, 0x99);
const DOT_WARN_RGB: (u8, u8, u8) = (0xF5, 0xB0, 0x4B);
const DOT_BAD_RGB: (u8, u8, u8) = (0xF8, 0x71, 0x71);
const DOT_NEUTRAL_RGB: (u8, u8, u8) = (0x6B, 0x72, 0x80);

/// One telemetry line split into a muted micro-label and a tabular value.
#[derive(Clone, Default)]
struct Row {
    label: String,
    value: String,
}

#[derive(Clone, Default)]
#[allow(dead_code)]
struct Panel {
    id: String,
    corner: u8,
    title: String,
    /// `title` uppercased with thin-space tracking, built once per spec change.
    title_display: String,
    lines: Vec<String>,
    /// `lines` parsed into label/value pairs, also built once per spec change so the
    /// render loop never splits or allocates strings per frame.
    rows: Vec<Row>,
}

/// Uppercase with U+2009 between characters: egui has no letter-spacing, and this is the
/// cheapest way to get the tracked micro-label look without a custom font.
fn tracked(raw: &str) -> String {
    let upper = raw.trim().to_uppercase();
    let mut out = String::with_capacity(upper.len() * 2);
    for (i, ch) in upper.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out
}

/// Rounds a coordinate down to the physical pixel grid, so a filled rect and its 1 px inner
/// stroke land on whole device pixels on both Retina (2x) and 1080p 1x Catalina displays.
fn snap_px(v: f32, ppp: f32) -> f32 {
    (v * ppp).round() / ppp
}

/// Snaps a stroked coordinate to the centre of a device pixel: a 1 px line drawn on a pixel
/// boundary straddles two pixels and blurs, one drawn on the centre stays razor sharp.
fn snap_half_px(v: f32, ppp: f32) -> f32 {
    ((v * ppp - 0.5).round() + 0.5) / ppp
}

/// Semantic dot colour for a row, or `None` when the row carries no health meaning.
fn tone_rgba(label: &str, value: &str, alpha: u8) -> Option<Color32> {
    let label = label.to_ascii_lowercase();
    let rgb = if label.contains("ping") || label.contains("latency") {
        let digits: String = value
            .chars()
            .filter(char::is_ascii_digit)
            .take(5)
            .collect();
        let ms = digits.parse::<u32>().ok()?;
        if ms < 60 {
            DOT_OK_RGB
        } else if ms < 120 {
            DOT_WARN_RGB
        } else {
            DOT_BAD_RGB
        }
    } else if label.contains("server") || label.contains("region") {
        DOT_NEUTRAL_RGB
    } else {
        return None;
    };
    Some(Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, alpha))
}

#[derive(Clone)]
struct Crosshair {
    style: u8,
    size: f32,
    thickness: f32,
    gap: f32,
    color: [u8; 3],
    opacity: f32,
    offset_x: f32,
    offset_y: f32,
    /// Absolute path to a PNG crosshair, empty for the drawn shapes.
    image: String,
}

/// A picture drawn in one corner of the game window.
#[derive(Clone)]
struct OverlayImage {
    path: String,
    corner: u8,
    size_pct: u8,
    opacity: f32,
    offset_x: f32,
    offset_y: f32,
}

/// Turns this helper into a background (accessory) app *before* any window exists, so
/// creating the overlay can never steal focus from Roblox. Without this the overlay
/// process could become the frontmost app for a moment, the "is Roblox in front" check
/// then failed, and the overlay hid itself - and never came back.
fn set_accessory_policy() {
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let app = msg0(class, "sharedApplication");
        if app.is_null() {
            return;
        }
        msg1(app, "setActivationPolicy:", 1);
    }
}

/// `(name, pid)` of the application currently in front of everything else (name lower-case),
/// e.g. `("roblox", 1234)`.
fn frontmost_application() -> Option<(String, i32)> {
    unsafe {
        let class = objc_getClass(c"NSWorkspace".as_ptr());
        if class.is_null() {
            return None;
        }
        let workspace = msg0(class, "sharedWorkspace");
        if workspace.is_null() {
            return None;
        }
        let app = msg0(workspace, "frontmostApplication");
        if app.is_null() {
            return None;
        }
        let name = msg0(app, "localizedName");
        if name.is_null() {
            return None;
        }
        let utf8 = msg0(name, "UTF8String") as *const c_char;
        if utf8.is_null() {
            return None;
        }
        let pid = msg0(app, "processIdentifier") as i32;
        Some((
            CStr::from_ptr(utf8).to_string_lossy().to_ascii_lowercase(),
            pid,
        ))
    }
}

/// Shows or hides the overlay window without destroying it.
///
/// The window floats above every app and every Space, so without this it stayed on screen
/// for a moment after switching away from Roblox (or kept sitting over the launcher).
fn set_overlay_visible(visible: bool) {
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let app = msg0(class, "sharedApplication");
        if app.is_null() {
            return;
        }
        let windows = msg0(app, "windows");
        if windows.is_null() {
            return;
        }
        let count = msg0(windows, "count") as usize;
        for i in 0..count {
            let window = msg1(windows, "objectAtIndex:", i as isize);
            if window.is_null() {
                continue;
            }
            if visible {
                msg1(window, "setLevel:", 1000);
                msg1_f(window, "setAlphaValue:", 1.0);
                msg0(window, "orderFrontRegardless");
            } else {
                // Never orderOut: an ordered-out window stops being repainted by macOS, so
                // after switching Spaces the overlay could never find its way back. A fully
                // transparent window keeps its frames and reappears instantly.
                msg1_f(window, "setAlphaValue:", 0.0);
            }
        }
    }
}

struct OverlayWindowApp {
    spec_path: PathBuf,
    panels: Vec<Panel>,
    crosshair: Option<Crosshair>,
    opacity: f32,
    bounds: Option<CGRect>,
    applied_bounds: Option<CGRect>,
    last_poll: Instant,
    last_tweak: Option<Instant>,
    /// Size + modification time of the spec we last parsed.
    last_spec_stamp: Option<(u64, Option<std::time::SystemTime>)>,
    misses: u32,
    /// Consecutive polls where the spec file could not be read or parsed.
    missing_specs: u32,
    /// The overlay is only on screen while Roblox is the front app.
    visible: bool,
    /// Decoded crosshair PNG, kept until the path changes.
    crosshair_tex: Option<(String, egui::TextureHandle)>,
    /// Custom image overlay (logo / watermark) and its decoded texture.
    image: Option<OverlayImage>,
    image_tex: Option<(String, egui::TextureHandle)>,
    /// When this process started, used for the "must appear at game start" grace period.
    started: Instant,
    /// True while the Roblox window could not be found and the display is used instead.
    using_fallback: bool,
    /// Cached "is a game running at all" check, refreshed every 1.5 s.
    last_proc_check: Instant,
    roblox_running: bool,
    /// The launcher says a game session is being played (not just the Roblox menu).
    /// Kept true when the spec does not say, so older specs still show.
    game_active: bool,
}

impl OverlayWindowApp {
    fn new(spec_path: PathBuf) -> Self {
        Self {
            spec_path,
            panels: Vec::new(),
            crosshair: None,
            opacity: 0.9,
            bounds: None,
            applied_bounds: None,
            last_poll: Instant::now() - Duration::from_secs(10),
            last_tweak: None,
            last_spec_stamp: None,
            misses: 0,
            missing_specs: 0,
            visible: false,
            crosshair_tex: None,
            image: None,
            image_tex: None,
            started: Instant::now(),
            using_fallback: false,
            last_proc_check: Instant::now() - Duration::from_secs(10),
            roblox_running: false,
            game_active: true,
        }
    }

    /// Loads (or reloads) the image crosshair. Only runs when the configured path changes,
    /// so a still image is decoded exactly once.
    fn ensure_crosshair_texture(
        &mut self,
        ctx: &egui::Context,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        texture_for(ctx, &mut self.crosshair_tex, path, "crosshair-image")
    }

    /// Loads (or reloads) the custom image overlay, again only when the path changes.
    fn ensure_image_texture(
        &mut self,
        ctx: &egui::Context,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        texture_for(ctx, &mut self.image_tex, path, "overlay-image")
    }

    /// Shows/hides the window only when the state actually changes.
    fn update_visibility(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            // The window was out of the window list while hidden: place it again.
            self.applied_bounds = None;
        }
        eprintln!(
            "overlay-window: visible={visible} (display fallback: {})",
            self.using_fallback
        );
        set_overlay_visible(visible);
    }

    /// Reads and applies the spec file. Only called when the file actually changed.
    fn reload_spec(&mut self) {
        let parsed = std::fs::read_to_string(&self.spec_path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(spec) = parsed else {
            // A missing or half-written spec must never kill the overlay: while switching
            // Spaces the file can briefly be unavailable. Only give up after a long run of
            // failed reads (the launcher really is gone then).
            self.missing_specs += 1;
            if self.missing_specs > 20 {
                std::process::exit(0);
            }
            return;
        };
        self.missing_specs = 0;
        if spec["quit"].as_bool().unwrap_or(false) {
            std::process::exit(0);
        }
        self.opacity = spec["opacity"].as_f64().unwrap_or(0.9).clamp(0.2, 1.0) as f32;
        self.game_active = spec["game_active"].as_bool().unwrap_or(true);
        self.panels = spec["panels"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        let title = p["title"].as_str().unwrap_or("").to_string();
                        let lines: Vec<String> = p["lines"]
                            .as_array()
                            .map(|l| {
                                l.iter()
                                    .filter_map(|s| s.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        // Split once, here, not in `paint`: "Server: Dallas, US" becomes a
                        // muted SERVER label with a tabular value, which is what makes the
                        // numbers line up and stop jittering.
                        let rows = lines
                            .iter()
                            .map(|line| match line.split_once(':') {
                                Some((label, value)) if !label.trim().is_empty() => Row {
                                    label: tracked(label),
                                    value: value.trim().to_string(),
                                },
                                _ => Row {
                                    label: String::new(),
                                    value: line.trim().to_string(),
                                },
                            })
                            .collect();
                        Panel {
                            id: p["id"].as_str().unwrap_or("").to_string(),
                            corner: p["corner"].as_u64().unwrap_or(3).min(3) as u8,
                            title_display: tracked(&title),
                            title,
                            rows,
                            lines,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.image = spec
            .get("image")
            .filter(|i| i.is_object())
            .map(|i| OverlayImage {
                path: i["path"].as_str().unwrap_or_default().to_string(),
                corner: i["corner"].as_u64().unwrap_or(0).min(3) as u8,
                size_pct: i["size"].as_u64().unwrap_or(25).clamp(1, 100) as u8,
                opacity: i["opacity"].as_f64().unwrap_or(0.9).clamp(0.05, 1.0) as f32,
                offset_x: i["offset_x"].as_f64().unwrap_or(0.0) as f32,
                offset_y: i["offset_y"].as_f64().unwrap_or(0.0) as f32,
            });
        self.crosshair = spec
            .get("crosshair")
            .filter(|c| c.is_object())
            .map(|c| Crosshair {
                style: c["style"].as_u64().unwrap_or(3).min(3) as u8,
                size: c["size"].as_f64().unwrap_or(18.0).clamp(4.0, 120.0) as f32,
                thickness: c["thickness"].as_f64().unwrap_or(2.0).clamp(1.0, 10.0) as f32,
                gap: c["gap"].as_f64().unwrap_or(4.0).clamp(0.0, 60.0) as f32,
                color: [
                    c["color"][0].as_u64().unwrap_or(255).min(255) as u8,
                    c["color"][1].as_u64().unwrap_or(60).min(255) as u8,
                    c["color"][2].as_u64().unwrap_or(60).min(255) as u8,
                ],
                opacity: c["opacity"].as_f64().unwrap_or(0.9).clamp(0.1, 1.0) as f32,
                offset_x: c["offset_x"].as_f64().unwrap_or(0.0) as f32,
                offset_y: c["offset_y"].as_f64().unwrap_or(0.0) as f32,
                image: c["image"].as_str().unwrap_or_default().to_string(),
            });
        eprintln!(
            "overlay-window: spec loaded ({} panels, image: {})",
            self.panels.len(),
            self.image.as_ref().map(|i| i.path.as_str()).unwrap_or("-")
        );
    }

    /// Re-reads the spec and the Roblox window position (about four times a second).
    fn poll(&mut self, ctx: &egui::Context) {
        if self.last_poll.elapsed() < Duration::from_millis(150) {
            return;
        }
        self.last_poll = Instant::now();

        // Only draw while Roblox is the front app: switching to another window or Space
        // hides the overlay immediately instead of leaving it floating for a few seconds.
        let front = frontmost_application();
        // Is a game running at all? This is what stops the overlay from appearing over the
        // desktop when nothing was joined, and makes it vanish once the game closes.
        if self.last_proc_check.elapsed() >= Duration::from_millis(1500) {
            self.last_proc_check = Instant::now();
            let (procs, _) = crate::roblox_process::enumerate_roblox_processes();
            self.roblox_running = !procs.is_empty();
        }
        // A Roblox window that exists proves the game runs even when the process scan
        // misses it (helper names, sandboxing, permissions).
        let found = find_roblox_window();
        if found.is_some() {
            self.roblox_running = true;
        }
        let self_pid = std::process::id() as i32;
        let roblox_in_front = match &front {
            // Our own overlay window floats above the game and may briefly report itself
            // as frontmost while it is being created: never let that hide the overlay.
            Some((_, pid)) if *pid == self_pid => self.visible,
            Some((name, _)) => name.contains("roblox"),
            // Reading the front app needs Screen Recording permission on macOS; without it
            // fall back to "a game is running" so the overlay still appears.
            None => self.roblox_running,
        };
        // Locate the game window: it decides both where the overlay sits and whether
        // we can trust our idea of what is on screen.
        self.using_fallback = found.is_none();
        self.bounds = found.or_else(main_display_bounds);
        if self.using_fallback && self.applied_bounds.is_none() {
            eprintln!(
                "overlay-window: no Roblox window found, covering the display instead ({front:?})"
            );
        }
        // Visible only while a game runs (not the Roblox menu), and only while it is the
        // front app - with a short grace period so it appears with the game instead of a
        // few seconds later.
        let grace = self.roblox_running && self.started.elapsed() < Duration::from_secs(10);
        self.update_visibility(
            self.game_active && self.roblox_running && (roblox_in_front || grace),
        );

        // Spec file: the launcher rewrites it only when something changed, so when the
        // size and modification time are unchanged there is nothing to read or parse.
        let stamp = std::fs::metadata(&self.spec_path)
            .ok()
            .map(|m| (m.len(), m.modified().ok()));
        if stamp != self.last_spec_stamp {
            self.last_spec_stamp = stamp;
            self.reload_spec();
        }

        match self.bounds {
            Some(b) => {
                self.misses = 0;
                if self.applied_bounds.is_none() {
                    eprintln!(
                        "overlay-window: Roblox window at {:.0},{:.0} {:.0}x{:.0}",
                        b.x, b.y, b.w, b.h
                    );
                }
                if self.applied_bounds != Some(b) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(
                        b.x as f32, b.y as f32,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
                        b.w as f32, b.h as f32,
                    )));
                    self.applied_bounds = Some(b);
                }
            }
            None => {
                self.misses += 1;
                // Stay alive and keep looking: Roblox can take a while to create its
                // window (and fullscreen Spaces can make it appear late). The manager
                // closes this process when the session ends.
                if self.misses == 40 || self.misses == 400 {
                    eprintln!(
                        "overlay-window: no Roblox window found yet ({} checks)",
                        self.misses
                    );
                }
            }
        }

        // Window level / Space behavior: apply until the window exists, then refresh now and then.
        let due = self
            .last_tweak
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(2));
        if due {
            let ok = tweak_overlay_window();
            self.last_tweak = if ok { Some(Instant::now()) } else { None };
            // `update_visibility` only acts on changes. If the very first "visible" poll ran
            // before the NSWindow existed, the alpha stayed 0 forever; now that the window
            // is known to exist, re-assert the current state on every refresh.
            if ok {
                set_overlay_visible(self.visible);
            }
        }
    }

    fn paint(&mut self, ui: &egui::Ui) {
        if !self.visible {
            return;
        }
        let painter = ui.painter();
        let area = ui.max_rect();
        let monitor_h = ui
            .ctx()
            .input(|i| i.viewport().monitor_size.map(|m| m.y))
            .unwrap_or(0.0);
        // Windowed Roblox has a ~28 pt title bar inside its window bounds. With no window
        // (display fallback) or a fullscreen one, the panels use the whole area.
        let title_bar = match self.bounds {
            Some(b) if monitor_h > 0.0 && b.h as f32 >= monitor_h - 2.0 => 0.0,
            Some(_) => 28.0,
            None => 0.0,
        };

        // Image overlay first: it is a background decoration, the text panels go on top.
        if let Some(img) = self.image.clone() {
            if !img.path.is_empty() {
                if let Some(tex) = self.ensure_image_texture(ui.ctx(), &img.path) {
                    paint_overlay_image(painter, area, title_bar, &img, &tex);
                }
            }
        }

        let crosshair_image = self
            .crosshair
            .as_ref()
            .map(|ch| ch.image.clone())
            .unwrap_or_default();
        let crosshair_tex = self.ensure_crosshair_texture(ui.ctx(), &crosshair_image);
        if let Some(ch) = &self.crosshair {
            let center = pos2(
                area.center().x + ch.offset_x,
                area.top() + title_bar + (area.height() - title_bar) / 2.0 + ch.offset_y,
            );
            match crosshair_tex {
                Some(tex) => paint_crosshair_image(painter, center, ch, &tex),
                None => paint_crosshair(painter, center, ch),
            }
        }

        self.paint_panels(painter, area, title_bar);
    }

    /// Draws the telemetry panels.
    ///
    /// Surface, one hairline, two text weights: a tracked 10 pt micro-label on the left, a
    /// tabular 12.5 pt value right-aligned on the same line, hairline row separators, and a
    /// 5 px dot only where a row has health meaning. Everything is snapped to the physical
    /// pixel grid so the 1 px edges stay crisp at 1x and 2x.
    fn paint_panels(&self, painter: &egui::Painter, area: Rect, title_bar: f32) {
        let ppp = painter.ctx().pixels_per_point().max(0.5);
        let px = 1.0 / ppp;
        let margin = 16.0;
        let gap = 8.0;
        let pad = 12.0;
        let row_h = 17.0;
        let label_font = FontId::proportional(10.0);
        let value_font = FontId::monospace(12.5);
        let alpha = |a: f32| (a * self.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        let label_color =
            Color32::from_rgba_unmultiplied(LABEL_RGB.0, LABEL_RGB.1, LABEL_RGB.2, alpha(1.0));
        let value_color =
            Color32::from_rgba_unmultiplied(VALUE_RGB.0, VALUE_RGB.1, VALUE_RGB.2, alpha(1.0));
        let surface = Color32::from_rgba_unmultiplied(
            SURFACE_RGB.0,
            SURFACE_RGB.1,
            SURFACE_RGB.2,
            alpha(SURFACE_ALPHA),
        );
        let hairline = Color32::from_rgba_unmultiplied(255, 255, 255, alpha(HAIRLINE_ALPHA));
        let divider = Color32::from_rgba_unmultiplied(255, 255, 255, alpha(DIVIDER_ALPHA));
        let mut used = [0.0_f32; 4];

        for panel in &self.panels {
            if panel.title_display.is_empty() && panel.rows.is_empty() {
                continue;
            }
            let title_galley = (!panel.title_display.is_empty()).then(|| {
                painter.layout_no_wrap(
                    panel.title_display.clone(),
                    label_font.clone(),
                    label_color,
                )
            });

            let mut label_w = 0.0_f32;
            let mut value_w = 0.0_f32;
            let mut has_dot = false;
            let mut rows = Vec::with_capacity(panel.rows.len());
            for row in &panel.rows {
                let label = (!row.label.is_empty()).then(|| {
                    painter.layout_no_wrap(row.label.clone(), label_font.clone(), label_color)
                });
                let value =
                    painter.layout_no_wrap(row.value.clone(), value_font.clone(), value_color);
                label_w = label_w.max(label.as_ref().map_or(0.0, |g| g.size().x));
                value_w = value_w.max(value.size().x);
                let tone = tone_rgba(&row.label, &row.value, alpha(1.0));
                has_dot |= tone.is_some();
                rows.push((label, value, tone));
            }
            let dot_w = if has_dot { 11.0 } else { 0.0 };

            let mut width = label_w
                + if label_w > 0.0 { 14.0 } else { 0.0 }
                + dot_w
                + value_w
                + pad * 2.0;
            let mut height = pad * 2.0;
            if let Some(t) = &title_galley {
                width = width.max(t.size().x + pad * 2.0);
                height += t.size().y + gap;
            }
            if !rows.is_empty() {
                height += px + gap + row_h * rows.len() as f32;
            }
            let size = vec2(width, height);

            let corner = panel.corner.min(3) as usize;
            let x = if corner % 2 == 1 {
                area.right() - size.x - margin
            } else {
                area.left() + margin
            };
            let y = if corner < 2 {
                area.top() + title_bar + margin + used[corner]
            } else {
                area.bottom() - size.y - margin - used[corner]
            };
            used[corner] += size.y + gap;

            let rect = Rect::from_min_size(pos2(snap_px(x, ppp), snap_px(y, ppp)), size);
            let radius = CornerRadius::same(8);
            painter.rect_filled(rect, radius, surface);
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(px, hairline),
                egui::StrokeKind::Inside,
            );

            let mut cursor_y = rect.top() + pad;
            if let Some(g) = title_galley {
                painter.galley(pos2(rect.left() + pad, cursor_y), g.clone(), label_color);
                cursor_y += g.size().y + gap;
            }
            if !rows.is_empty() {
                let rule = snap_half_px(cursor_y, ppp);
                painter.line_segment(
                    [
                        pos2(rect.left() + pad, rule),
                        pos2(rect.right() - pad, rule),
                    ],
                    Stroke::new(px, divider),
                );
                cursor_y += px + gap;
                for (i, (label, value, tone)) in rows.iter().enumerate() {
                    if let Some(l) = label {
                        let ly = cursor_y + (row_h - l.size().y) / 2.0;
                        painter.galley(pos2(rect.left() + pad, ly), l.clone(), label_color);
                    }
                    let vx = rect.right() - pad - value.size().x;
                    let vy = cursor_y + (row_h - value.size().y) / 2.0;
                    if let Some(color) = tone {
                        // 5 px diameter, no glow: colour is the whole signal.
                        painter.circle_filled(pos2(vx - 8.0, cursor_y + row_h / 2.0), 2.5, *color);
                    }
                    painter.galley(pos2(vx, vy), value.clone(), value_color);
                    cursor_y += row_h;
                    if i + 1 < rows.len() {
                        let rule = snap_half_px(cursor_y, ppp);
                        painter.line_segment(
                            [
                                pos2(rect.left() + pad, rule),
                                pos2(rect.right() - pad, rule),
                            ],
                            Stroke::new(px, divider),
                        );
                    }
                }
            }
        }
    }
}

/// Decodes `path` into an egui texture, reusing `cache` when the path did not change.
fn texture_for(
    ctx: &egui::Context,
    cache: &mut Option<(String, egui::TextureHandle)>,
    path: &str,
    key: &str,
) -> Option<egui::TextureHandle> {
    if path.is_empty() {
        *cache = None;
        return None;
    }
    if let Some((cached, tex)) = cache {
        if cached == path && tex.size()[0] > 0 {
            return Some(tex.clone());
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &img.into_raw());
    let tex = ctx.load_texture(format!("{key}:{path}"), color, egui::TextureOptions::LINEAR);
    *cache = Some((path.to_string(), tex.clone()));
    Some(tex)
}

/// Draws the custom image in its corner, scaled by height and never stretched.
fn paint_overlay_image(
    painter: &egui::Painter,
    area: Rect,
    title_bar: f32,
    img: &OverlayImage,
    tex: &egui::TextureHandle,
) {
    let image_size = tex.size_vec2();
    if image_size.x <= 0.0 || image_size.y <= 0.0 {
        return;
    }
    let view_h = (area.height() - title_bar).max(1.0);
    let target_h = (view_h * (img.size_pct as f32 / 100.0)).clamp(8.0, view_h);
    let scale = target_h / image_size.y;
    let size = vec2(image_size.x * scale, target_h);
    let margin = 16.0;
    let x = if img.corner % 2 == 1 {
        area.right() - size.x - margin
    } else {
        area.left() + margin
    };
    let y = if img.corner < 2 {
        area.top() + title_bar + margin
    } else {
        area.bottom() - size.y - margin
    };
    let rect = Rect::from_min_size(pos2(x + img.offset_x, y + img.offset_y), size);
    let alpha = (img.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    painter.image(
        tex.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::from_white_alpha(alpha),
    );
}

/// Draws a PNG crosshair centred on the aim point, fitted into `ch.size` without stretching.
fn paint_crosshair_image(
    painter: &egui::Painter,
    center: Pos2,
    ch: &Crosshair,
    tex: &egui::TextureHandle,
) {
    let image_size = tex.size_vec2();
    if image_size.x <= 0.0 || image_size.y <= 0.0 {
        return;
    }
    let scale = ch.size / image_size.x.max(image_size.y);
    let size = image_size * scale;
    let rect = Rect::from_center_size(center, size);
    let alpha = (ch.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    painter.image(
        tex.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::from_white_alpha(alpha),
    );
}

fn paint_crosshair(painter: &egui::Painter, center: Pos2, ch: &Crosshair) {
    let ppp = painter.ctx().pixels_per_point().max(0.5);
    let px = 1.0 / ppp;
    let color = Color32::from_rgba_unmultiplied(
        ch.color[0],
        ch.color[1],
        ch.color[2],
        (ch.opacity * 255.0).round() as u8,
    );
    // Exactly one device pixel of dark backing. The old `thickness + 2` was two points, which
    // on a 1x Catalina display smeared into a soft halo around every arm.
    let outline = Color32::from_black_alpha((ch.opacity * 160.0).round() as u8);
    let t = ch.thickness.max(px);
    let outline_t = t + px;
    // Centre the whole shape on a device-pixel centre, then snap each endpoint: a segment
    // that starts on a pixel boundary straddles two rows of pixels and blurs.
    let center = pos2(snap_half_px(center.x, ppp), snap_half_px(center.y, ppp));
    let arm = ch.size / 2.0;
    let gap = ch.gap;
    let draw_cross = |painter: &egui::Painter| {
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let a = pos2(
                snap_half_px(center.x + dx * gap, ppp),
                snap_half_px(center.y + dy * gap, ppp),
            );
            let b = pos2(
                snap_half_px(center.x + dx * (gap + arm), ppp),
                snap_half_px(center.y + dy * (gap + arm), ppp),
            );
            painter.line_segment([a, b], Stroke::new(outline_t, outline));
            painter.line_segment([a, b], Stroke::new(t, color));
        }
    };
    match ch.style {
        0 => {
            painter.circle_filled(center, snap_px(t * 1.5 + px, ppp), outline);
            painter.circle_filled(center, snap_px(t * 1.5, ppp), color);
        }
        1 => draw_cross(painter),
        2 => {
            painter.circle_stroke(center, arm, Stroke::new(outline_t, outline));
            painter.circle_stroke(center, arm, Stroke::new(t, color));
        }
        _ => {
            draw_cross(painter);
            painter.circle_filled(center, snap_px(t * 1.2 + px, ppp), outline);
            painter.circle_filled(center, snap_px(t * 1.2, ppp), color);
        }
    }
}

impl eframe::App for OverlayWindowApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.paint(ui);
        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

/// Entry point for `creblox --overlay-window <spec.json>`.
pub fn run(spec_path: PathBuf) -> eframe::Result<()> {
    eprintln!("overlay-window: starting (spec {})", spec_path.display());
    // Must happen before the window is created, otherwise the overlay briefly becomes the
    // frontmost app, fails its own "Roblox is in front" check and hides itself.
    set_accessory_policy();
    let viewport = egui::ViewportBuilder::default()
        .with_title("CREBLOX Overlay")
        .with_inner_size([400.0, 300.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_resizable(false)
        .with_active(false);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "CREBLOX Overlay",
        options,
        Box::new(move |cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = Color32::TRANSPARENT;
            visuals.window_fill = Color32::TRANSPARENT;
            visuals.extreme_bg_color = Color32::TRANSPARENT;
            cc.egui_ctx.set_visuals(visuals);
            // Keep frames coming even while the overlay is transparent: the visibility check
            // lives in `ui()`, and a window macOS considers idle may stop being redrawn.
            let heartbeat_ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(400));
                    heartbeat_ctx.request_repaint();
                }
            });
            Ok(Box::new(OverlayWindowApp::new(spec_path)))
        }),
    )
}
