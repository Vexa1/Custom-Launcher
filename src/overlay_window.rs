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

#[derive(Clone, Default)]
#[allow(dead_code)]
struct Panel {
    id: String,
    corner: u8,
    title: String,
    lines: Vec<String>,
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
                    .map(|p| Panel {
                        id: p["id"].as_str().unwrap_or("").to_string(),
                        corner: p["corner"].as_u64().unwrap_or(3).min(3) as u8,
                        title: p["title"].as_str().unwrap_or("").to_string(),
                        lines: p["lines"]
                            .as_array()
                            .map(|l| {
                                l.iter()
                                    .filter_map(|s| s.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
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

        let margin = 16.0;
        let gap = 8.0;
        let mut used = [0.0_f32; 4];
        for panel in &self.panels {
            if panel.title.is_empty() && panel.lines.is_empty() {
                continue;
            }
            let title_font = FontId::proportional(13.0);
            let line_font = FontId::monospace(11.5);
            let alpha = |a: f32| (a * self.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
            let title_color = Color32::from_white_alpha(alpha(1.0));
            let line_color = Color32::from_white_alpha(alpha(0.78));

            let title_galley = (!panel.title.is_empty())
                .then(|| painter.layout_no_wrap(panel.title.clone(), title_font, title_color));
            let line_galleys: Vec<_> = panel
                .lines
                .iter()
                .map(|l| painter.layout_no_wrap(l.clone(), line_font.clone(), line_color))
                .collect();

            let pad = 12.0;
            let mut width = 0.0_f32;
            let mut height = pad * 2.0;
            if let Some(g) = &title_galley {
                width = width.max(g.size().x);
                height += g.size().y + 4.0;
            }
            for g in &line_galleys {
                width = width.max(g.size().x);
                height += g.size().y + 2.0;
            }
            let size = vec2(width + pad * 2.0, height);

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

            let rect = Rect::from_min_size(pos2(x, y), size);
            painter.rect_filled(
                rect,
                CornerRadius::same(10),
                Color32::from_rgba_unmultiplied(18, 23, 31, alpha(0.92)),
            );
            let mut cursor = rect.min + vec2(pad, pad);
            if let Some(g) = title_galley {
                let h = g.size().y;
                painter.galley(cursor, g, title_color);
                cursor.y += h + 4.0;
            }
            for g in line_galleys {
                let h = g.size().y;
                painter.galley(cursor, g, line_color);
                cursor.y += h + 2.0;
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
    let color = Color32::from_rgba_unmultiplied(
        ch.color[0],
        ch.color[1],
        ch.color[2],
        (ch.opacity * 255.0).round() as u8,
    );
    // A thin dark outline keeps it visible on bright backgrounds.
    let outline = Color32::from_black_alpha((ch.opacity * 160.0).round() as u8);
    let t = ch.thickness;
    let arm = ch.size / 2.0;
    let gap = ch.gap;
    let draw_cross = |painter: &egui::Painter| {
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let a = pos2(center.x + dx * gap, center.y + dy * gap);
            let b = pos2(center.x + dx * (gap + arm), center.y + dy * (gap + arm));
            painter.line_segment([a, b], Stroke::new(t + 2.0, outline));
            painter.line_segment([a, b], Stroke::new(t, color));
        }
    };
    match ch.style {
        0 => {
            painter.circle_filled(center, t * 1.5 + 1.0, outline);
            painter.circle_filled(center, t * 1.5, color);
        }
        1 => draw_cross(painter),
        2 => {
            painter.circle_stroke(center, arm, Stroke::new(t + 2.0, outline));
            painter.circle_stroke(center, arm, Stroke::new(t, color));
        }
        _ => {
            draw_cross(painter);
            painter.circle_filled(center, t * 1.2 + 1.0, outline);
            painter.circle_filled(center, t * 1.2, color);
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
