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

#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, pos2, vec2};
use serde_json::Value;

use crate::overlay_spec;

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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFNumberGetValue(number: *const c_void, kind: i32, out: *mut c_void) -> u8;
    fn CFRelease(obj: *const c_void);
}

#[cfg(target_os = "macos")]
const EXCLUDE_DESKTOP_ELEMENTS: u32 = 16;
#[cfg(target_os = "macos")]
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
#[cfg(target_os = "macos")]
const CF_NUMBER_SINT32: i32 = 3;

/// Bounds (in points, origin top-left of the primary display) of the largest Roblox window.
///
/// Every window is listed, not only the ones on the current Space: Roblox in fullscreen
/// lives on its own Space, and window layers differ between windowed, borderless and
/// fullscreen modes, so a layer-0 on-screen search can come back empty while the game is
/// right there. CREBLOX's own windows are skipped.
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
fn main_display_bounds() -> Option<CGRect> {
    unsafe {
        let rect = CGDisplayBounds(CGMainDisplayID());
        (rect.w > 0.0 && rect.h > 0.0).then_some(rect)
    }
}

// ---------------------------------------------------------------------------
// Win32: find the Roblox window and drive our own overlay window
// ---------------------------------------------------------------------------
//
// Same two questions the macOS path answers with CoreGraphics/AppKit: where is Roblox's
// game window, and how does this process float a window above it without ever taking
// focus? Everything here goes through the Win32 calls winit/eframe already use, so the
// overlay needs no extra runtime and no external tool.

#[cfg(windows)]
mod win32 {
    use super::CGRect;
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HWND, LPARAM, RECT, TRUE};
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetSystemMetrics, GetWindowLongPtrW,
        GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        HWND_TOPMOST, IsIconic, IsWindowVisible, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE,
        SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_EX_TRANSPARENT,
    };

    /// The title winit gives the overlay's only top-level window.
    const OVERLAY_TITLE: &str = "CREBLOX Overlay";

    fn scale_for(dpi: u32) -> f64 {
        if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
    }

    /// Base name of a process image, lower-case: `("robloxplayerbeta.exe")`.
    ///
    /// `QueryFullProcessImageNameW` reads the path through a query-only handle, so it works
    /// on a normal user account and on protected games without any elevation.
    fn process_name(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if handle as usize == 0 {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(handle);
            if ok == 0 || len == 0 {
                return None;
            }
            let path = OsString::from_wide(&buf[..len as usize]);
            let text = path.to_string_lossy().replace('/', "\\");
            Some(text.rsplit('\\').next().unwrap_or_default().to_ascii_lowercase())
        }
    }

    /// Window rectangle in egui points, excluding the invisible resize border.
    fn window_rect(hwnd: HWND) -> Option<CGRect> {
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        // The DWM frame bounds are what the user sees; `GetWindowRect` includes the
        // invisible border Windows 10 adds to every resizable window.
        let mut ok = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                &mut rect as *mut RECT as *mut c_void,
                std::mem::size_of::<RECT>() as u32,
            )
        };
        if ok != 0 {
            ok = unsafe { GetWindowRect(hwnd, &mut rect) };
        }
        if ok == 0 {
            return None;
        }
        let scale = scale_for(unsafe { GetDpiForWindow(hwnd) });
        Some(CGRect {
            x: rect.left as f64 / scale,
            y: rect.top as f64 / scale,
            w: (rect.right - rect.left) as f64 / scale,
            h: (rect.bottom - rect.top) as f64 / scale,
        })
    }

    struct Pick {
        best: Option<(CGRect, f64)>,
        own_pid: u32,
    }

    unsafe extern "system" fn pick_roblox(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let pick = unsafe { &mut *(lparam as *mut Pick) };
        unsafe {
            // Minimised windows have a bogus rect; the overlay must never cover the desktop
            // because the game was minimized.
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return TRUE;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || pid == pick.own_pid {
                return TRUE;
            }
            let Some(name) = process_name(pid) else {
                return TRUE;
            };
            if !name.starts_with("robloxplayer") {
                return TRUE;
            }
            let Some(rect) = window_rect(hwnd) else {
                return TRUE;
            };
            if rect.w <= 1.0 || rect.h <= 1.0 {
                return TRUE;
            }
            let area = rect.w * rect.h;
            if pick.best.as_ref().is_none_or(|(_, best)| area > *best) {
                pick.best = Some((rect, area));
            }
        }
        TRUE
    }

    pub fn find_roblox_window() -> Option<CGRect> {
        let mut pick = Pick {
            best: None,
            own_pid: std::process::id(),
        };
        unsafe { EnumWindows(Some(pick_roblox), &mut pick as *mut Pick as LPARAM) };
        pick.best.map(|(rect, _)| rect)
    }

    pub fn main_display_bounds() -> Option<CGRect> {
        let scale = scale_for(unsafe { GetDpiForSystem() });
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        if w <= 0 || h <= 0 {
            return None;
        }
        Some(CGRect {
            x: 0.0,
            y: 0.0,
            w: w as f64 / scale,
            h: h as f64 / scale,
        })
    }

    pub fn frontmost_application() -> Option<(String, i32)> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd as usize == 0 {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return None;
        }
        process_name(pid).map(|name| (name, pid as i32))
    }

    struct Own {
        pid: u32,
        title: Vec<u16>,
        titled: HWND,
        first: HWND,
    }

    unsafe extern "system" fn pick_own(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let own = unsafe { &mut *(lparam as *mut Own) };
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != own.pid {
                return TRUE;
            }
            if own.first as usize == 0 {
                own.first = hwnd;
            }
            if own.titled as usize == 0 {
                let len = GetWindowTextLengthW(hwnd);
                if len > 0 {
                    let mut buf = vec![0u16; len as usize + 1];
                    let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                    if copied > 0 && buf[..copied as usize] == own.title[..] {
                        own.titled = hwnd;
                    }
                }
            }
        }
        TRUE
    }

    /// This process's overlay window, found by title and never by a guessed handle: winit
    /// owns the HWND and would not survive us caching it across a recreate.
    fn own_window() -> Option<HWND> {
        let mut own = Own {
            pid: std::process::id(),
            title: OVERLAY_TITLE.encode_utf16().collect(),
            titled: 0 as HWND,
            first: 0 as HWND,
        };
        unsafe { EnumWindows(Some(pick_own), &mut own as *mut Own as LPARAM) };
        let found = if own.titled as usize != 0 {
            own.titled
        } else {
            own.first
        };
        (found as usize != 0).then_some(found)
    }

    /// Show or hide the overlay without destroying it, and without activating it.
    pub fn set_overlay_visible(visible: bool) {
        unsafe {
            let Some(hwnd) = own_window() else {
                return;
            };
            if visible {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            } else {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    /// Topmost, tool-window (no taskbar button), never focusable, click-through unless the
    /// position editor asked for input. Returns false until the window exists.
    pub fn tweak_overlay_window(interactive: bool) -> bool {
        unsafe {
            let Some(hwnd) = own_window() else {
                return false;
            };
            let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            style |= WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
            if interactive {
                style &= !WS_EX_TRANSPARENT;
            } else {
                style |= WS_EX_TRANSPARENT;
            }
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        true
    }
}

#[cfg(windows)]
fn find_roblox_window() -> Option<CGRect> {
    win32::find_roblox_window()
}

#[cfg(windows)]
fn main_display_bounds() -> Option<CGRect> {
    win32::main_display_bounds()
}

/// Nothing to do on Windows: a plain Win32 window has no Dock icon and no activation
/// policy; `tweak_overlay_window` keeps it out of the taskbar and out of focus instead.
#[cfg(windows)]
fn set_accessory_policy() {}

#[cfg(windows)]
fn frontmost_application() -> Option<(String, i32)> {
    win32::frontmost_application()
}

#[cfg(windows)]
fn set_overlay_visible(visible: bool) {
    win32::set_overlay_visible(visible);
}

#[cfg(windows)]
fn tweak_overlay_window(interactive: bool) -> bool {
    win32::tweak_overlay_window(interactive)
}

// ---------------------------------------------------------------------------
// AppKit tweaks through the Objective-C runtime (no extra crates)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}

#[cfg(target_os = "macos")]
type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
#[cfg(target_os = "macos")]
type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, isize) -> *mut c_void;
#[cfg(target_os = "macos")]
type Msg1F = unsafe extern "C" fn(*mut c_void, *mut c_void, f64) -> *mut c_void;

#[cfg(target_os = "macos")]
unsafe fn msg0(obj: *mut c_void, sel: &str) -> *mut c_void {
    let Ok(name) = CString::new(sel) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let f: Msg0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel_registerName(name.as_ptr()))
    }
}

#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
/// Dock icon or shadow. Returns false while the window does not exist yet.
///
/// `interactive` is the position editor: it is the only state in which the overlay accepts a
/// click, so the window stays strictly click-through for the whole rest of the session.
#[cfg(target_os = "macos")]
fn tweak_overlay_window(interactive: bool) -> bool {
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
            msg1(window, "setLevel:", if interactive { 1001 } else { 1000 });
            msg1(window, "setCollectionBehavior:", 1 | 16 | 256); // all Spaces | stationary | over fullscreen
            // Click-through except while the user is dragging the HUD. Edit mode hands the
            // click straight back on Confirm, so Roblox never loses input for long.
            msg1(window, "setIgnoresMouseEvents:", if interactive { 0 } else { 1 });
            msg1(window, "setHasShadow:", 0);
            msg1(window, "setHidesOnDeactivate:", 0);
            msg1(window, "setCanHide:", 0);
            msg1(window, "setOpaque:", 0);
            if interactive {
                // Return / Esc only reach a key window, and clicking the HUD activates the
                // app - so the editor is usable without any extra permission.
                msg1(window, "makeKeyAndOrderFront:", 0);
            } else {
                msg0(window, "resignKeyWindow");
                // Show up without the app being active; without this the overlay can be
                // created and positioned yet never ordered onto the screen.
                msg0(window, "orderFrontRegardless");
            }
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
    /// Unix-ms anchor of this panel's live timer (`joined_at_unix_ms`), when it has one.
    /// The launcher writes it once per session; the seconds come from the clock here.
    started_at_ms: Option<u64>,
    /// Where this panel is drawn, in points from its own corner anchor. The editor moves
    /// exactly one of these when the user drags one card.
    offset: egui::Vec2,
    /// The launcher's saved position for this panel: what Reset and Esc go back to.
    saved: egui::Vec2,
    /// The party card's member tiles. Empty for every text panel.
    tiles: Vec<crate::party_hud::Tile>,
}

/// One laid-out row, in game-window coordinates. Built once per spec / scale change; the
/// render loop only translates and draws these, so a frame allocates nothing.
struct TileLayout {
    id: u64,
    rect: Rect,
    outline: Color32,
    initials: Arc<egui::Galley>,
    name: Arc<egui::Galley>,
    name_pos: Pos2,
}

struct RowLayout {
    label: Option<Arc<egui::Galley>>,
    label_pos: Pos2,
    value: Arc<egui::Galley>,
    value_pos: Pos2,
    tone: Option<(u8, u8, u8)>,
    dot_pos: Pos2,
}

struct PanelLayout {
    /// The panel's id from the spec (`server`, `stats`, `party`, ...), so the editor can line
    /// a card up with where it was before the spec changed.
    id: String,
    rect: Rect,
    title: Option<(Pos2, Arc<egui::Galley>)>,
    rows: Vec<RowLayout>,
    /// Party tiles: one rounded square, its outline colour, and the name under it.
    tiles: Vec<TileLayout>,
    /// Absolute y of every hairline inside the panel (title rule + row separators).
    rules: Vec<f32>,
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

/// Splits one HUD line into a muted label and a tabular value.
///
/// The label side must be digit-free: `Time     12:34` and `Session  1:23:45` carry their
/// colons inside the value, and splitting on them turned the clock into `T I M E 1 2` /
/// `34` and the session into `S E S S I O N 1` / `23:45`. HUD labels are words only, so a
/// digit before the colon means the whole line is a value.
fn split_row(line: &str) -> Row {
    let line = line.trim();
    if let Some((label, value)) = line.split_once(':') {
        let label = label.trim();
        if !label.is_empty() && !label.bytes().any(|b| b.is_ascii_digit()) {
            return Row {
                label: tracked(label),
                value: value.trim().to_string(),
            };
        }
    }
    Row {
        label: String::new(),
        value: line.to_string(),
    }
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

/// A picture drawn in one corner of the game window - or exactly where the editor put it.
#[derive(Clone)]
struct OverlayImage {
    path: String,
    corner: u8,
    size_pct: u8,
    opacity: f32,
    offset_x: f32,
    offset_y: f32,
    /// The editor's saved box in points from the game area's top-left. `w`/`h` at 0 mean the
    /// picture still follows `size_pct` and the corner fine-tune offsets above; the box's
    /// position has its own fields so a dragged picture and the offset sliders cannot fight.
    x: f32,
    y: f32,
    w: u16,
    h: u16,
    /// Where it was drawn this frame (game-window coordinates).
    rect: Option<Rect>,
    /// What Reset and Esc go back to.
    saved: (f32, f32, u16, u16),
}

/// Turns this helper into a background (accessory) app *before* any window exists, so
/// creating the overlay can never steal focus from Roblox. Without this the overlay
/// process could become the frontmost app for a moment, the "is Roblox in front" check
/// then failed, and the overlay hid itself - and never came back.
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
    /// Decoded 60x60 headshots, one per member in the party right now. Dropped as soon as a
    /// member leaves the strip, which is what keeps this to a few kilobytes.
    party_tex: std::collections::HashMap<u64, egui::TextureHandle>,
    /// Tiles whose headshot has a fetch in flight, so a member never queues two.
    party_asked: std::collections::HashSet<u64>,
    /// Fetched pictures waiting to be uploaded as textures.
    party_ready: std::sync::Arc<std::sync::Mutex<Vec<(u64, egui::ColorImage)>>>,
    crosshair: Option<Crosshair>,
    opacity: f32,
    bounds: Option<CGRect>,
    applied_bounds: Option<CGRect>,
    last_poll: Instant,
    last_tweak: Option<Instant>,
    /// The frontmost app and the Roblox window rectangle. Both are OS lookups, so they are
    /// refreshed on their own slower clock instead of on every poll.
    last_lookup: Instant,
    front: Option<(String, i32)>,
    roblox_window: Option<CGRect>,
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
    /// A Roblox player log contains a join banner right now. Checked here, in this process,
    /// so the HUD never depends on the launcher having parsed a place id.
    session_active: bool,
    /// The launcher says a game session is being played (not just the Roblox menu).
    /// Kept true when the spec does not say, so older specs still show.
    game_active: bool,
    /// Interactive position editing is on.
    edit: bool,
    /// The user already confirmed/cancelled; ignore the stale `edit: true` in the spec until
    /// the launcher has caught up, otherwise the next spec reload would re-enter edit mode.
    edit_done: bool,
    /// The panel being dragged right now, by id (`None` when nothing is moving).
    dragging: Option<String>,
    /// This process just entered edit mode: the next editor frame re-anchors every card to
    /// where it was drawn before the spec switched to the editor's preview.
    edit_entered: bool,
    /// Where each card was drawn during live gameplay, by id. The editor starts from these
    /// instead of from the corner stack it would compute for its own spec.
    live_rects: Vec<(String, Rect)>,
    /// Where the custom image was drawn this frame, and which grip is being dragged:
    /// `0..=3` a corner, `4` the whole picture, `None` nothing.
    image_rect: Option<Rect>,
    image_grip: Option<u8>,
    /// The part the current drag grabbed, decided once at press so a corner keeps the drag
    /// even after the pointer leaves its grip.
    image_drag: Option<overlay_spec::ImagePart>,
    /// The image's box and the game area it was fitted into, in points from that area's
    /// top-left - the coordinates the editor and settings.json use.
    image_box: Option<crate::overlay_spec::ImageBox>,
    image_view: (f32, f32),
    /// The panel under the pointer, by id: it gets the accent outline.
    hovering: Option<String>,
    /// The window accepts mouse input (edit mode only).
    interactive: bool,
    /// Last cursor-hit-test state sent to the window; the command is only sent on change.
    passthrough: Option<bool>,
    /// Laid-out panels, in game-window space.
    layout: Vec<PanelLayout>,
    /// `(spec generation, ppp, opacity, width, height, title bar)` the layout was built for.
    layout_stamp: Option<(u64, u32, u32, f32, f32, f32)>,
    /// Bumped on every spec reload so the layout cache is invalidated exactly once.
    spec_gen: u64,
    /// Window-local origin of the layout (non-zero while the window is trimmed to the HUD).
    shift: egui::Vec2,
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
            last_lookup: Instant::now() - Duration::from_secs(10),
            front: None,
            roblox_window: None,
            last_spec_stamp: None,
            misses: 0,
            missing_specs: 0,
            visible: false,
            crosshair_tex: None,
            party_tex: std::collections::HashMap::new(),
            party_asked: std::collections::HashSet::new(),
            party_ready: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            image: None,
            image_tex: None,
            started: Instant::now(),
            using_fallback: false,
            last_proc_check: Instant::now() - Duration::from_secs(10),
            roblox_running: false,
            session_active: false,
            game_active: true,
            edit: false,
            edit_done: false,
            dragging: None,
            edit_entered: false,
            live_rects: Vec::new(),
            image_rect: None,
            image_grip: None,
            image_drag: None,
            image_box: None,
            image_view: (0.0, 0.0),
            hovering: None,
            interactive: false,
            passthrough: None,
            layout: Vec::new(),
            layout_stamp: None,
            spec_gen: 0,
            shift: egui::Vec2::ZERO,
        }
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
        self.spec_gen = self.spec_gen.wrapping_add(1);
        // Positions of the cards the user is moving right now. The launcher rewrites this
        // file while the editor is open, so they are carried across the reload below -
        // snapping to the saved values would drop a card out from under the pointer.
        let live_offsets: Vec<(String, egui::Vec2)> = if self.edit {
            self.panels
                .iter()
                .map(|p| (p.id.clone(), p.offset))
                .collect()
        } else {
            Vec::new()
        };
        // The picture needs the same protection: the launcher rewrites this file while the
        // editor is open, and reading the saved box back would snap it out from under the
        // pointer on every reload.
        let live_image = if self.edit {
            self.image.as_ref().map(|i| (i.x, i.y, i.w, i.h))
        } else {
            None
        };
        let edit = spec["edit"].as_bool().unwrap_or(false);
        if !edit {
            self.edit = false;
            self.edit_done = false;
        } else if !self.edit && !self.edit_done {
            // Entered from the tray or Settings: every card starts exactly where it is on
            // screen right now, not where the editor's own layout would put it.
            self.edit = true;
            self.edit_entered = true;
            self.image_grip = None;
        }
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
                            .map(|line| split_row(line.as_str()))
                            .collect();
                        // The anchor only matters when a row carries the timer token.
                        let started_at_ms = p["joined_at_unix_ms"]
                            .as_u64()
                            .filter(|_| lines.iter().any(|l| overlay_spec::has_elapsed(l)));
                        let offset = vec2(
                            p["offset_x"].as_f64().unwrap_or(0.0) as f32,
                            p["offset_y"].as_f64().unwrap_or(0.0) as f32,
                        );
                        let tiles = p["tiles"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .map(|t| crate::party_hud::Tile {
                                        id: t["id"].as_u64().unwrap_or(0),
                                        name: t["name"].as_str().unwrap_or("").to_string(),
                                        initials: t["initials"].as_str().unwrap_or("?").to_string(),
                                        status: crate::party_hud::TileStatus::from_key(
                                            t["status"].as_str().unwrap_or("loading"),
                                        ),
                                        headshot: t["headshot"].as_str().unwrap_or("").to_string(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        Panel {
                            id: p["id"].as_str().unwrap_or("").to_string(),
                            tiles,
                            corner: p["corner"].as_u64().unwrap_or(3).min(3) as u8,
                            title_display: tracked(&title),
                            title,
                            rows,
                            lines,
                            started_at_ms,
                            offset,
                            saved: offset,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self.edit {
            for panel in &mut self.panels {
                if let Some((_, live)) = live_offsets.iter().find(|(id, _)| *id == panel.id) {
                    panel.offset = *live;
                }
            }
        }
        self.image = spec
            .get("image")
            .filter(|i| i.is_object())
            .map(|i| {
                // The box's four fields, including the 0.5.51 file that kept its position in
                // the offsets - one reader for the launcher's file and this one.
                let saved = crate::overlay_spec::SavedImageBox::from_panels_image(i);
                OverlayImage {
                    path: i["path"].as_str().unwrap_or_default().to_string(),
                    corner: i["corner"].as_u64().unwrap_or(0).min(3) as u8,
                    size_pct: i["size"].as_u64().unwrap_or(25).clamp(1, 100) as u8,
                    opacity: i["opacity"].as_f64().unwrap_or(0.9).clamp(0.05, 1.0) as f32,
                    offset_x: i["offset_x"].as_f64().unwrap_or(0.0) as f32,
                    offset_y: i["offset_y"].as_f64().unwrap_or(0.0) as f32,
                    x: saved.x as f32,
                    y: saved.y as f32,
                    w: saved.w,
                    h: saved.h,
                    rect: None,
                    saved: (saved.x as f32, saved.y as f32, saved.w, saved.h),
                }
            });
        if let (Some((x, y, w, h)), Some(img)) = (live_image, self.image.as_mut()) {
            img.x = x;
            img.y = y;
            img.w = w;
            img.h = h;
        }
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
        // hides the overlay instead of leaving it floating for a few seconds.
        //
        // These two are the expensive half of a poll: `frontmostApplication` goes through
        // AppKit and `CGWindowListCopyWindowInfo` walks every window on the machine. The
        // editor and a drag re-poll at frame rate, so asking about seven times a second
        // (the 150 ms floor below) bought nothing - neither answer changes that fast. They
        // now share a half-second clock, still twice the cadence of a visible HUD frame.
        if self.last_lookup.elapsed() >= Duration::from_millis(500) {
            self.last_lookup = Instant::now();
            self.front = frontmost_application();
            self.roblox_window = find_roblox_window();
        }
        let front = self.front.clone();
        // Is a game running at all? This is what stops the overlay from appearing over the
        // desktop when nothing was joined, and makes it vanish once the game closes.
        if self.last_proc_check.elapsed() >= Duration::from_millis(1500) {
            self.last_proc_check = Instant::now();
            // Cached: the answer is one tick old at most, and a full scan here ran twice
            // per poll (`session_banner_active` below scans too).
            let (procs, _) = crate::roblox_process::enumerate_roblox_processes_cached();
            self.roblox_running = !procs.is_empty();
            // Independent of the launcher: a join banner in the player log is a session.
            self.session_active = crate::roblox_process::session_banner_active();
        }
        // A Roblox window that exists proves the game runs even when the process scan
        // misses it (helper names, sandboxing, permissions).
        let found = self.roblox_window;
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
        // Editing wins over every "should it show" rule: the user asked for it from the tray,
        // possibly with no game running at all.
        // Two independent signals, either of which is enough. `game_active` comes from the
        // launcher and needs the place id to parse; `session_active` is read from the player
        // log here. Requiring only the first was the reason the overlay could stay invisible
        // for a whole session even though the game was running and the address was known.
        let want = if self.edit {
            true
        } else {
            (self.game_active || self.session_active) && self.roblox_running && (roblox_in_front || grace)
        };
        if want != self.visible {
            let reason = format!(
                "overlay: show={want} (edit={}, game_active={}, session={}, roblox={}, in_front={roblox_in_front}, grace={grace}, front={front:?})",
                self.edit, self.game_active, self.session_active, self.roblox_running
            );
            // The helper's stderr goes to overlay.log, but log.txt is the file people read
            // when the HUD does not appear - so the one line that explains the decision goes
            // to both.
            eprintln!("overlay-window: {reason}");
            crate::auth::log(&reason);
        }
        self.update_visibility(want);
        // Mouse input is enabled only for the editor, and taken back the moment it ends.
        if self.interactive != self.edit {
            self.interactive = self.edit;
            tweak_overlay_window(self.interactive);
        }

        // Spec file: the launcher rewrites it only when something changed, so when the
        // size and modification time are unchanged there is nothing to read or parse.
        let stamp = std::fs::metadata(&self.spec_path)
            .ok()
            .map(|m| (m.len(), m.modified().ok()));
        if stamp != self.last_spec_stamp {
            self.last_spec_stamp = stamp;
            self.reload_spec();
        }

        // The layout is built in `paint` (it needs a painter to measure text) and cached, so
        // the union here is at most one frame behind - enough to size a HUD-only window.
        match self.bounds {
            Some(b) => {
                self.misses = 0;
                if self.applied_bounds.is_none() {
                    eprintln!(
                        "overlay-window: Roblox window at {:.0},{:.0} {:.0}x{:.0}",
                        b.x, b.y, b.w, b.h
                    );
                }
                let union = self.layout_union();
                // Text panels only: shrink the window to the HUD itself. On a Catalina Intel
                // GPU that is the difference between compositing a few thousand pixels and a
                // full-screen transparent quad every time the HUD updates. A crosshair or a
                // picture needs the whole game area, and so does the editor.
                let tight = !self.edit
                    && self.crosshair.is_none()
                    && self.image.is_none()
                    && !union.is_negative()
                    && union.width() > 1.0
                    && union.height() > 1.0;
                let (pos, size) = if tight {
                    (
                        pos2(b.x as f32 + union.min.x, b.y as f32 + union.min.y),
                        union.size(),
                    )
                } else {
                    (pos2(b.x as f32, b.y as f32), vec2(b.w as f32, b.h as f32))
                };
                self.shift = if tight {
                    union.min.to_vec2()
                } else {
                    egui::Vec2::ZERO
                };
                let want = CGRect {
                    x: pos.x as f64,
                    y: pos.y as f64,
                    w: size.x as f64,
                    h: size.y as f64,
                };
                if self.applied_bounds != Some(want) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                    self.applied_bounds = Some(want);
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
            let ok = tweak_overlay_window(self.interactive);
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
        if !self.visible && !self.edit {
            return;
        }
        let painter = ui.painter();
        let area = ui.max_rect();
        let title_bar = self.title_bar(ui.ctx());
        // Measures every label and value once, then caches the galleys.
        self.relayout(painter, title_bar);

        // Image overlay first: it is a background decoration, the text panels go on top.
        self.image_rect = None;
        self.image_box = None;
        if let Some(img) = self.image.as_mut().filter(|img| !img.path.is_empty()) {
            let path = img.path.clone();
            if let Some(tex) = texture_for(ui.ctx(), &mut self.image_tex, &path, "overlay-image") {
                let view = (area.width().max(1.0), (area.height() - title_bar).max(1.0));
                let (box_, rect) = image_rect(area, title_bar, img, tex.size_vec2(), view);
                img.rect = Some(rect);
                self.image_rect = Some(rect);
                self.image_box = Some(box_);
                self.image_view = view;
                paint_image_rect(painter, rect, img.opacity, &tex);
            }
        }

        let crosshair_path = self
            .crosshair
            .as_ref()
            .map(|ch| ch.image.as_str())
            .unwrap_or("");
        let crosshair_tex = texture_for(
            ui.ctx(),
            &mut self.crosshair_tex,
            crosshair_path,
            "crosshair-image",
        );
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

        self.pump_party_textures(ui.ctx());
        self.paint_layout(painter);
        if self.edit {
            self.paint_edit(ui);
        }
    }

    /// Title-bar height of the game window (0 when it is fullscreen or unknown).
    fn title_bar(&self, ctx: &egui::Context) -> f32 {
        let monitor_h = ctx
            .input(|i| i.viewport().monitor_size.map(|m| m.y))
            .unwrap_or(0.0);
        match self.bounds {
            Some(b) if monitor_h > 0.0 && b.h as f32 >= monitor_h - 2.0 => 0.0,
            Some(_) => 28.0,
            None => 0.0,
        }
    }

    /// Uploads headshots that finished downloading and asks for the ones still missing.
    ///
    /// Textures are dropped the moment a member leaves the strip, so the cache only ever holds
    /// the people in the party - a few 60x60 pictures, not a history of everyone seen.
    fn pump_party_textures(&mut self, ctx: &egui::Context) {
        if let Ok(mut ready) = self.party_ready.lock() {
            for (id, image) in ready.drain(..) {
                let alive = self
                    .panels
                    .iter()
                    .any(|p| p.tiles.iter().any(|t| t.id == id));
                self.party_asked.remove(&id);
                if alive {
                    let tex = ctx.load_texture(
                        format!("party-{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.party_tex.insert(id, tex);
                }
            }
        }
        let live: Vec<u64> = self
            .panels
            .iter()
            .flat_map(|p| p.tiles.iter().map(|t| t.id))
            .collect();
        self.party_tex.retain(|id, _| live.contains(id));
        self.party_asked.retain(|id| live.contains(id));
        let mut queued = 0;
        for panel in &self.panels {
            for tile in &panel.tiles {
                if queued >= 3 {
                    break;
                }
                if tile.headshot.is_empty()
                    || self.party_tex.contains_key(&tile.id)
                    || self.party_asked.contains(&tile.id)
                {
                    continue;
                }
                self.party_asked.insert(tile.id);
                queued += 1;
                let url = tile.headshot.clone();
                let id = tile.id;
                let ready = std::sync::Arc::clone(&self.party_ready);
                let _ = std::thread::Builder::new()
                    .name("creblox-party-headshot".to_string())
                    .spawn(move || {
                        let fetched = crate::api::download_thumbnails(vec![(url, [60, 60])]);
                        if let Some((_, image)) = fetched.into_iter().next() {
                            if let Ok(mut out) = ready.lock() {
                                out.push((id, image));
                            }
                        }
                    });
            }
        }
    }

    /// Bounding box of every panel, in game-window coordinates.
    fn layout_union(&self) -> Rect {
        self.layout
            .iter()
            .fold(Rect::NOTHING, |acc, p| acc.union(p.rect))
    }

    /// Advances the live timer rows (`In server`, `Session`) from their anchors.
    ///
    /// The launcher writes an anchor, not a clock, so the seconds have to be produced here.
    /// Each row is rewritten only when its whole second changed - one string a second, not a
    /// re-format per frame - and the layout is invalidated once for the new widths.
    ///
    /// Returns the millis until the next whole second, so `ui` can sleep exactly until the
    /// moment the row changes and nowhere in between.
    fn tick_timers(&mut self, now_ms: u64) -> Option<u64> {
        let mut wake: Option<u64> = None;
        let mut changed = false;
        for panel in &mut self.panels {
            let Some(started_at_ms) = panel.started_at_ms else {
                continue;
            };
            let due = overlay_spec::ms_until_next_second(started_at_ms, now_ms);
            wake = Some(wake.map_or(due, |w: u64| w.min(due)));
            for i in 0..panel.rows.len() {
                if !overlay_spec::has_elapsed(&panel.rows[i].value) {
                    continue;
                }
                let text = overlay_spec::with_elapsed(&panel.rows[i].value, started_at_ms, now_ms);
                if text == panel.rows[i].value {
                    continue;
                }
                panel.rows[i].value = text.clone();
                if let Some(line) = panel.lines.get_mut(i) {
                    *line = text;
                }
                changed = true;
            }
        }
        if changed {
            self.spec_gen = self.spec_gen.wrapping_add(1);
        }
        wake
    }

    /// Lays the panels out in game-window coordinates.
    ///
    /// Runs only when the spec, the scale, the opacity or the window size changed. Every
    /// frame after that draws the cached galleys, so painting allocates nothing - the old
    /// version laid out every label and value again on each frame.
    fn relayout(&mut self, painter: &egui::Painter, title_bar: f32) {
        let ppp = painter.ctx().pixels_per_point().max(0.5);
        let size = self
            .bounds
            .map(|b| vec2(b.w as f32, b.h as f32))
            .unwrap_or(vec2(400.0, 300.0));
        let key = (
            self.spec_gen,
            (ppp * 1000.0) as u32,
            (self.opacity * 1000.0) as u32,
            size.x,
            size.y,
            title_bar,
        );
        if self.layout_stamp == Some(key) {
            return;
        }
        self.layout_stamp = Some(key);
        self.layout.clear();

        let px = 1.0 / ppp;
        let margin = 16.0;
        let gap = 8.0;
        let pad = 12.0;
        let row_h: f32 = 17.0;
        let label_font = FontId::proportional(10.0);
        let value_font = FontId::monospace(12.5);
        let alpha = (self.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        let label_color =
            Color32::from_rgba_unmultiplied(LABEL_RGB.0, LABEL_RGB.1, LABEL_RGB.2, alpha);
        let value_color =
            Color32::from_rgba_unmultiplied(VALUE_RGB.0, VALUE_RGB.1, VALUE_RGB.2, alpha);
        let area = Rect::from_min_size(Pos2::ZERO, size);
        let mut used = [0.0_f32; 4];

        for panel in &self.panels {
            if panel.title_display.is_empty() && panel.rows.is_empty() && panel.tiles.is_empty() {
                continue;
            }
            let title = (!panel.title_display.is_empty()).then(|| {
                painter.layout_no_wrap(panel.title_display.clone(), label_font.clone(), label_color)
            });

            // The party card: a row-wrapped strip of member squares. Same card surface,
            // same corner slot and offset as every text panel.
            if !panel.tiles.is_empty() {
                let square: f32 = 44.0;
                let cell_gap: f32 = 8.0;
                let name_gap: f32 = 3.0;
                let name_h: f32 = 13.0;
                let cols = panel.tiles.len().min(4) as f32;
                let rows_n = panel.tiles.len().div_ceil(4);
                let title_h = title.as_ref().map_or(0.0, |g| g.size().y + gap);
                let rule_h = if title.is_some() { px + gap } else { 0.0 };
                let width = cols * square + (cols - 1.0) * cell_gap + pad * 2.0;
                let height = pad * 2.0
                    + title_h
                    + rule_h
                    + rows_n as f32 * (square + name_gap + name_h)
                    + (rows_n as f32 - 1.0).max(0.0) * cell_gap;
                let panel_size = vec2(width, height);
                let corner = panel.corner.min(3) as usize;
                let base_x = if corner % 2 == 1 {
                    area.right() - panel_size.x - margin
                } else {
                    area.left() + margin
                };
                let base_y = if corner < 2 {
                    area.top() + title_bar + margin + used[corner]
                } else {
                    area.bottom() - panel_size.y - margin - used[corner]
                };
                used[corner] += panel_size.y + gap;
                let rect = Rect::from_min_size(
                    pos2(
                        snap_px(base_x + panel.offset.x, ppp),
                        snap_px(base_y + panel.offset.y, ppp),
                    ),
                    panel_size,
                );
                let mut cursor_y = rect.top() + pad;
                let mut rules = Vec::with_capacity(1);
                let title = title.map(|g| {
                    let pos = pos2(rect.left() + pad, cursor_y);
                    cursor_y += g.size().y + gap;
                    rules.push(snap_half_px(cursor_y, ppp));
                    cursor_y += px + gap;
                    (pos, g)
                });
                let name_font = FontId::proportional(9.0);
                let mut out_tiles = Vec::with_capacity(panel.tiles.len());
                for (i, tile) in panel.tiles.iter().enumerate() {
                    let col = (i % 4) as f32;
                    let row = (i / 4) as f32;
                    let x = rect.left() + pad + col * (square + cell_gap);
                    let y = cursor_y + row * (square + name_gap + name_h + cell_gap);
                    let cell = Rect::from_min_size(pos2(x, y), vec2(square, square));
                    let initials = painter.layout_no_wrap(
                        tile.initials.clone(),
                        FontId::monospace(15.0),
                        value_color,
                    );
                    let name = painter.layout_no_wrap(
                        overlay_spec::shorten(&tile.name, 7),
                        name_font.clone(),
                        label_color,
                    );
                    let name_pos = pos2(
                        cell.center().x - name.size().x / 2.0,
                        cell.bottom() + name_gap,
                    );
                    let rgb = tile.status.rgb();
                    out_tiles.push(TileLayout {
                        id: tile.id,
                        rect: cell,
                        outline: Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, alpha),
                        initials,
                        name,
                        name_pos,
                    });
                }
                self.layout.push(PanelLayout {
                    id: panel.id.clone(),
                    rect,
                    title,
                    rows: Vec::new(),
                    rules,
                    tiles: out_tiles,
                });
                continue;
            }

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
                let tone = tone_rgba(&row.label, &row.value, 255).map(|c| (c.r(), c.g(), c.b()));
                has_dot |= tone.is_some();
                // The slot is as tall as the taller text plus a little air. A fixed row
                // height let a sub-label's descenders touch the line below it - and on a
                // fallback font the two rows visibly overlapped.
                let h = row_h
                    .max(label.as_ref().map_or(0.0, |g| g.size().y))
                    .max(value.size().y)
                    + 2.0;
                rows.push((label, value, tone, h));
            }
            let dot_w = if has_dot { 11.0 } else { 0.0 };

            let mut width = label_w
                + if label_w > 0.0 { 14.0 } else { 0.0 }
                + dot_w
                + value_w
                + pad * 2.0;
            let mut height = pad * 2.0;
            if let Some(t) = &title {
                width = width.max(t.size().x + pad * 2.0);
                height += t.size().y + gap;
            }
            if !rows.is_empty() {
                height += px + gap + rows.iter().map(|r| r.3).sum::<f32>();
            }
            let panel_size = vec2(width, height);

            // The corner slot first (already stacked clear of the panels in that corner),
            // then this panel's own offset: the spot the user dragged just this card to.
            let corner = panel.corner.min(3) as usize;
            let base_x = if corner % 2 == 1 {
                area.right() - panel_size.x - margin
            } else {
                area.left() + margin
            };
            let base_y = if corner < 2 {
                area.top() + title_bar + margin + used[corner]
            } else {
                area.bottom() - panel_size.y - margin - used[corner]
            };
            used[corner] += panel_size.y + gap;
            let rect = Rect::from_min_size(
                pos2(
                    snap_px(base_x + panel.offset.x, ppp),
                    snap_px(base_y + panel.offset.y, ppp),
                ),
                panel_size,
            );

            let mut cursor_y = rect.top() + pad;
            let title = title.map(|g| {
                let pos = pos2(rect.left() + pad, cursor_y);
                cursor_y += g.size().y + gap;
                (pos, g)
            });

            let mut rules = Vec::with_capacity(rows.len() + 1);
            let mut out_rows = Vec::with_capacity(rows.len());
            if !rows.is_empty() {
                rules.push(snap_half_px(cursor_y, ppp));
                cursor_y += px + gap;
                let row_count = rows.len();
                for (i, (label, value, tone, h)) in rows.into_iter().enumerate() {
                    let value_pos = pos2(
                        rect.right() - pad - value.size().x,
                        cursor_y + (h - value.size().y) / 2.0,
                    );
                    // A line with no label ("Server info unavailable") is a status line of
                    // its own: at the left edge, on its own row, never sharing a baseline
                    // with the title or the line above it.
                    let label_pos = label
                        .as_ref()
                        .map(|g| pos2(rect.left() + pad, cursor_y + (h - g.size().y) / 2.0))
                        .unwrap_or(pos2(rect.left() + pad, value_pos.y));
                    let dot_pos = pos2(value_pos.x - 8.0, cursor_y + h / 2.0);
                    cursor_y += h;
                    if i + 1 < row_count {
                        rules.push(snap_half_px(cursor_y, ppp));
                    }
                    out_rows.push(RowLayout {
                        label,
                        label_pos,
                        value,
                        value_pos,
                        tone,
                        dot_pos,
                    });
                }
            }
            self.layout.push(PanelLayout {
                id: panel.id.clone(),
                rect,
                title,
                rows: out_rows,
                rules,
                tiles: Vec::new(),
            });
        }
        if !self.edit {
            // The live layout, kept for the moment the user opens the editor: it is the only
            // place the pre-edit positions still exist once the spec switches to its preview.
            self.live_rects = self
                .layout
                .iter()
                .map(|l| (l.id.clone(), l.rect))
                .collect();
        }
    }

    /// Draws the cached layout. Pure translation and fills: no text is laid out here.
    fn paint_layout(&self, painter: &egui::Painter) {
        if self.layout.is_empty() {
            return;
        }
        let ppp = painter.ctx().pixels_per_point().max(0.5);
        let px = 1.0 / ppp;
        let alpha = (self.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        let scaled = |v: f32| (v * self.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        let surface = Color32::from_rgba_unmultiplied(
            SURFACE_RGB.0,
            SURFACE_RGB.1,
            SURFACE_RGB.2,
            scaled(SURFACE_ALPHA),
        );
        let hairline = Color32::from_rgba_unmultiplied(255, 255, 255, scaled(HAIRLINE_ALPHA));
        let divider = Color32::from_rgba_unmultiplied(255, 255, 255, scaled(DIVIDER_ALPHA));
        let label_color =
            Color32::from_rgba_unmultiplied(LABEL_RGB.0, LABEL_RGB.1, LABEL_RGB.2, alpha);
        let value_color =
            Color32::from_rgba_unmultiplied(VALUE_RGB.0, VALUE_RGB.1, VALUE_RGB.2, alpha);
        // Rects already carry their own offsets; the window only needs the origin trim.
        let shift = -self.shift;
        let radius = CornerRadius::same(8);

        for panel in &self.layout {
            let rect = panel.rect.translate(shift);
            painter.rect_filled(rect, radius, surface);
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(px, hairline),
                egui::StrokeKind::Inside,
            );
            if let Some((pos, galley)) = &panel.title {
                painter.galley(*pos + shift, galley.clone(), label_color);
            }
            for y in &panel.rules {
                painter.line_segment(
                    [
                        pos2(rect.left() + 12.0, y + shift.y),
                        pos2(rect.right() - 12.0, y + shift.y),
                    ],
                    Stroke::new(px, divider),
                );
            }
            for tile in &panel.tiles {
                let cell = tile.rect.translate(shift);
                painter.rect_filled(
                    cell,
                    radius,
                    Color32::from_rgba_unmultiplied(255, 255, 255, scaled(16.0)),
                );
                match self.party_tex.get(&tile.id) {
                    Some(tex) => {
                        painter.image(
                            tex.id(),
                            cell,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
                        );
                    }
                    None => {
                        // Initials while the 60x60 headshot loads, and if it never arrives.
                        let pos = pos2(
                            cell.center().x - tile.initials.size().x / 2.0,
                            cell.center().y - tile.initials.size().y / 2.0,
                        );
                        painter.galley(pos, tile.initials.clone(), value_color);
                    }
                }
                // The 2 px status outline: green in this server, grey loading, red gone.
                painter.rect_stroke(
                    cell,
                    radius,
                    Stroke::new(2.0, tile.outline),
                    egui::StrokeKind::Inside,
                );
                painter.galley(tile.name_pos + shift, tile.name.clone(), label_color);
            }
            for row in &panel.rows {
                if let Some(label) = &row.label {
                    painter.galley(row.label_pos + shift, label.clone(), label_color);
                }
                if let Some((r, g, b)) = row.tone {
                    painter.circle_filled(
                        row.dot_pos + shift,
                        2.5,
                        Color32::from_rgba_unmultiplied(r, g, b, alpha),
                    );
                }
                painter.galley(row.value_pos + shift, row.value.clone(), value_color);
            }
        }
    }

    /// The position editor: one dashed outline per panel, an accent border on the panel under
    /// the pointer or being dragged, its live `(x, y)` in the readout, and Reset / Confirm.
    /// Also the only place that moves a panel.
    fn paint_edit(&mut self, ui: &egui::Ui) {
        if self.layout.is_empty() && self.image_rect.is_none() {
            return;
        }
        // The spec the launcher writes for the editor can differ from the live one (a preview
        // card, a panel the game hid). Re-anchor every card once, to where it was drawn before,
        // so opening the editor never moves anything under the pointer.
        if self.edit_entered {
            self.edit_entered = false;
            let mut moved = false;
            for (i, layout) in self.layout.iter().enumerate() {
                let Some((_, live)) = self.live_rects.iter().find(|(id, _)| *id == layout.id)
                else {
                    continue;
                };
                let shift = live.min - layout.rect.min;
                if shift.length() > 0.01 {
                    if let Some(panel) = self.panels.get_mut(i) {
                        panel.offset += shift;
                        moved = true;
                    }
                }
            }
            if moved {
                self.spec_gen = self.spec_gen.wrapping_add(1);
                ui.ctx().request_repaint();
            }
        }
        let ppp = ui.ctx().pixels_per_point().max(0.5);
        let px = 1.0 / ppp;
        let view = ui.max_rect();
        let painter = ui.painter();
        let surface =
            Color32::from_rgba_unmultiplied(SURFACE_RGB.0, SURFACE_RGB.1, SURFACE_RGB.2, 240);
        let hairline = Color32::from_rgba_unmultiplied(255, 255, 255, 60);
        let text_color = Color32::from_rgb(VALUE_RGB.0, VALUE_RGB.1, VALUE_RGB.2);
        let muted = Color32::from_rgb(LABEL_RGB.0, LABEL_RGB.1, LABEL_RGB.2);
        let outline = Color32::from_rgba_unmultiplied(255, 255, 255, 150);
        let accent = Color32::from_rgb(120, 170, 255);

        // One hitbox per panel: dragging a card moves only that card.
        let mut hits: Vec<(usize, String, egui::Response)> = Vec::with_capacity(self.layout.len());
        for (i, panel) in self.layout.iter().enumerate() {
            let id = self
                .panels
                .get(i)
                .map(|p| p.id.clone())
                .unwrap_or_else(|| format!("panel-{i}"));
            let response = ui.interact(
                panel.rect.translate(-self.shift),
                ui.scope_id().with(("creblox-hud-panel", i)),
                egui::Sense::drag(),
            );
            hits.push((i, id, response));
        }
        self.hovering = None;
        self.dragging = None;
        let mut moved = false;
        for (i, id, response) in &hits {
            if response.hovered() || response.dragged() {
                self.hovering = Some(id.clone());
            }
            if response.dragged() {
                self.dragging = Some(id.clone());
                let delta = response.drag_delta();
                if let (Some(b), Some(layout), Some(panel)) =
                    (self.bounds, self.layout.get(*i), self.panels.get_mut(*i))
                {
                    // Clamp this card's own rect, so no panel can leave the game window.
                    let min = layout.rect.min + delta;
                    let size = layout.rect.size();
                    let clamped = pos2(
                        min.x.clamp(0.0, (b.w as f32 - size.x).max(0.0)),
                        min.y.clamp(0.0, (b.h as f32 - size.y).max(0.0)),
                    );
                    panel.offset += delta + (clamped - min);
                }
                moved = true;
            }
        }
        if moved {
            // The layout carries the offsets, so it has to be built again next frame.
            self.spec_gen = self.spec_gen.wrapping_add(1);
            ui.ctx().request_repaint();
        }

        // The custom image: one hitbox for the picture and all four handles. The press
        // position decides what was grabbed (corners first), so a handle always consumes its
        // drag before the body can - per-widget overlapping hitboxes depended on egui's
        // hit-test order and made the picture feel dead wherever another target sat on it.
        let mut image_active = false;
        if let (Some(rect), Some(box_), true) =
            (self.image_rect, self.image_box, self.image.is_some())
        {
            let view = self.image_view;
            let hit = rect.translate(-self.shift);
            let zone = hit.expand(overlay_spec::IMAGE_GRIP / 2.0);
            let response = ui.interact(
                zone,
                ui.scope_id().with("creblox-hud-image"),
                egui::Sense::click_and_drag(),
            );
            let part_at = |p: egui::Pos2| {
                overlay_spec::image_part(
                    (hit.left(), hit.top(), hit.width(), hit.height()),
                    (p.x, p.y),
                )
            };
            let hovered_part = ui.input(|i| i.pointer.interact_pos()).and_then(part_at);
            if response.drag_started() {
                // Resolve against the press origin, not the pointer after the first move.
                self.image_drag = ui
                    .input(|i| i.pointer.press_origin())
                    .and_then(part_at)
                    .or(hovered_part);
            }
            if response.hovered() || response.dragged() {
                image_active = true;
                if let Some(part) = hovered_part {
                    self.image_grip = Some(match part {
                        overlay_spec::ImagePart::Body => 4,
                        overlay_spec::ImagePart::Corner(i) => i,
                    });
                    ui.ctx().set_cursor_icon(match part {
                        overlay_spec::ImagePart::Body => egui::CursorIcon::Grab,
                        overlay_spec::ImagePart::Corner(0 | 3) => {
                            egui::CursorIcon::ResizeNwSe
                        }
                        overlay_spec::ImagePart::Corner(_) => {
                            egui::CursorIcon::ResizeNeSw
                        }
                    });
                }
            }
            if response.dragged()
                && let Some(part) = self.image_drag
            {
                let hold_shift = ui.input(|i| i.modifiers.shift);
                let delta = response.drag_delta();
                let next =
                    overlay_spec::drag_image(box_, part, delta.x, delta.y, view, hold_shift);
                if let Some(img) = self.image.as_mut() {
                    apply_image_box(img, next);
                }
                self.dragging = Some("image".to_string());
                self.spec_gen = self.spec_gen.wrapping_add(1);
                ui.ctx().request_repaint();
            }
            if !response.dragged() {
                self.image_drag = None;
            }
        }
        if image_active {
            self.hovering = Some("image".to_string());
            if self.dragging.is_none() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        } else if self.image_grip.is_some() && self.dragging.is_none() {
            self.image_grip = None;
        }
        match (&self.dragging, &self.hovering) {
            (Some(_), _) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            (None, Some(_)) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            (None, None) => {}
        }

        let active = self.dragging.clone().or_else(|| self.hovering.clone());
        let mut active_rect = view;
        if active.as_deref() == Some("image") {
            if let Some(rect) = self.image_rect {
                active_rect = rect.translate(-self.shift);
            }
        }
        for (i, id, _) in &hits {
            let Some(panel) = self.layout.get(*i) else {
                continue;
            };
            let rect = panel.rect.translate(-self.shift);
            let is_active = active.as_deref() == Some(id.as_str());
            if is_active {
                active_rect = rect;
            }
            self.paint_dashed(
                painter,
                rect.expand(4.0),
                if is_active { accent } else { outline },
                px,
            );
            if is_active {
                // Crisp accent border on the card itself, next to the dashed "draggable" box.
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(8),
                    Stroke::new(1.5, accent),
                    egui::StrokeKind::Outside,
                );
            }
        }

        // The image's selection frame: a dashed box, an accent border when it is active and
        // four 8x8 px accent squares the user can grab.
        if let Some(rect) = self.image_rect {
            let hit = rect.translate(-self.shift);
            let image_is_active = active.as_deref() == Some("image");
            self.paint_dashed(
                painter,
                hit.expand(4.0),
                if image_is_active { accent } else { outline },
                px,
            );
            if image_is_active {
                painter.rect_stroke(
                    hit,
                    CornerRadius::same(4),
                    Stroke::new(1.5, accent),
                    egui::StrokeKind::Outside,
                );
            }
            for corner in [
                hit.left_top(),
                hit.right_top(),
                hit.left_bottom(),
                hit.right_bottom(),
            ] {
                let square = Rect::from_center_size(
                    pos2(snap_px(corner.x, ppp), snap_px(corner.y, ppp)),
                    vec2(8.0, 8.0),
                );
                painter.rect_filled(square, CornerRadius::same(1), accent);
                painter.rect_stroke(
                    square,
                    CornerRadius::same(1),
                    Stroke::new(px, Color32::from_rgb(12, 14, 20)),
                    egui::StrokeKind::Outside,
                );
            }
        }

        // Edge guides for the active card, while it is close to a screen edge.
        let snap = 8.0;
        let guide = Color32::from_rgba_unmultiplied(120, 170, 255, 200);
        let edges = [
            (active_rect.left() - view.left(), view.left(), true),
            (view.right() - active_rect.right(), view.right(), true),
            (active_rect.top() - view.top(), view.top(), false),
            (view.bottom() - active_rect.bottom(), view.bottom(), false),
        ];
        for (distance, at, vertical) in edges {
            if distance.abs() >= snap {
                continue;
            }
            let stroke = Stroke::new(px, guide);
            if vertical {
                painter.line_segment(
                    [pos2(at + px * 0.5, view.top()), pos2(at + px * 0.5, view.bottom())],
                    stroke,
                );
            } else {
                painter.line_segment(
                    [pos2(view.left(), at + px * 0.5), pos2(view.right(), at + px * 0.5)],
                    stroke,
                );
            }
        }

        // Readout: the active card's id and its live position.
        let shown = active
            .as_ref()
            .and_then(|id| self.panels.iter().find(|p| &p.id == id))
            .or_else(|| self.panels.first());
        let (readout, readout_w) = if active.as_deref() == Some("image") {
            match self.image_box {
                Some(b) => {
                    let text = format!("image  {}, {}   {} × {}", b.x, b.y, b.w, b.h);
                    let galley = painter.layout_no_wrap(text, FontId::monospace(11.5), text_color);
                    (galley.text().to_string(), galley.size().x)
                }
                None => (String::new(), 0.0),
            }
        } else {
            match shown {
            Some(p) => {
                let text = format!(
                    "{}  {}, {}",
                    p.id,
                    p.offset.x.round() as i32,
                    p.offset.y.round() as i32
                );
                let galley =
                    painter.layout_no_wrap(text, FontId::monospace(11.5), text_color);
                (galley.text().to_string(), galley.size().x)
            }
            None => (String::new(), 0.0),
            }
        };

        // The bar is as wide as its parts, and each part starts where the measured one before
        // it ends: the readout, the hint, Reset and Confirm can never overlap.
        let pad = 10.0;
        let gap = 10.0;
        let btn_w = 60.0;
        let confirm_w = 78.0;
        // 36 with a 5 px inset leaves a 26 px pill: a real hit target, not a hairline.
        let bar_h = 36.0;
        let label_font = FontId::proportional(10.0);
        let fixed = pad + readout_w + gap + gap + btn_w + 6.0 + confirm_w + pad;
        let room = (view.width() - 16.0).max(120.0);
        // Words, not glyphs: the default egui font has no ↵, so a symbol hint renders as a
        // tofu box on a stock install.
        let mut hint = "drag · Enter save · Esc cancel";
        let mut hint_w = painter
            .layout_no_wrap(hint.to_string(), label_font.clone(), muted)
            .size()
            .x;
        if fixed + hint_w > room {
            hint = "drag · Enter save";
            hint_w = painter
                .layout_no_wrap(hint.to_string(), label_font.clone(), muted)
                .size()
                .x;
        }
        if fixed + hint_w > room {
            hint = "drag · Enter";
            hint_w = painter
                .layout_no_wrap(hint.to_string(), label_font.clone(), muted)
                .size()
                .x;
        }
        let bar_w = (fixed + hint_w).min(room);
        // Clear of the active element's own corner handles (24 px targets, 12 px each side),
        // so the bar can never sit on one.
        let below = active_rect.max.y + 16.0;
        let bar_y = if below + bar_h <= view.max.y - 8.0 {
            below
        } else {
            (active_rect.min.y - 16.0 - bar_h).max(view.min.y + 8.0)
        };
        let bar_x = active_rect.min.x.clamp(
            view.min.x + 8.0,
            (view.max.x - bar_w - 8.0).max(view.min.x + 8.0),
        );
        let bar = Rect::from_min_size(pos2(bar_x, bar_y), vec2(bar_w, bar_h));
        painter.rect_filled(bar, CornerRadius::same(8), surface);
        painter.rect_stroke(
            bar,
            CornerRadius::same(8),
            Stroke::new(px, hairline),
            egui::StrokeKind::Inside,
        );

        let confirm = Rect::from_min_size(
            pos2(bar.right() - pad - confirm_w, bar.top() + 5.0),
            vec2(confirm_w, bar_h - 10.0),
        );
        let reset = Rect::from_min_size(
            pos2(confirm.left() - 6.0 - btn_w, bar.top() + 5.0),
            vec2(btn_w, bar_h - 10.0),
        );
        let readout_pos = pos2(bar.left() + pad, bar.center().y);
        let hint_pos = pos2(readout_pos.x + readout_w + gap, bar.center().y);
        painter.text(
            readout_pos,
            egui::Align2::LEFT_CENTER,
            &readout,
            FontId::monospace(11.5),
            text_color,
        );
        if hint_pos.x + hint_w <= reset.left() - 4.0 {
            painter.text(
                hint_pos,
                egui::Align2::LEFT_CENTER,
                hint,
                label_font,
                muted,
            );
        }
        for (rect, label) in [(reset, "Reset"), (confirm, "Confirm")] {
            painter.rect_filled(
                rect,
                CornerRadius::same(6),
                Color32::from_rgba_unmultiplied(255, 255, 255, 16),
            );
            painter.rect_stroke(
                rect,
                CornerRadius::same(6),
                Stroke::new(px, hairline),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.0),
                text_color,
            );
        }

        let confirm_clicked = ui
            .interact(
                confirm,
                ui.scope_id().with("creblox-hud-confirm"),
                egui::Sense::click(),
            )
            .clicked();
        let reset_clicked = ui
            .interact(
                reset,
                ui.scope_id().with("creblox-hud-reset"),
                egui::Sense::click(),
            )
            .clicked();
        let (enter, esc) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if reset_clicked {
            // Reset is the default layout: every card back in its own corner slot and the
            // picture back to its corner, height percentage and offsets.
            for panel in &mut self.panels {
                panel.offset = egui::Vec2::ZERO;
            }
            if let Some(img) = self.image.as_mut() {
                // Dropping the box is what hands the picture back to its corner slot.
                img.w = 0;
                img.h = 0;
            }
            self.image_grip = None;
            self.spec_gen = self.spec_gen.wrapping_add(1);
        }
        if confirm_clicked || enter {
            self.finish_edit(true);
        } else if esc {
            self.finish_edit(false);
        }
    }

    /// A dashed rectangle: the editor's "this card can be moved" outline.
    fn paint_dashed(&self, painter: &egui::Painter, frame: Rect, color: Color32, px: f32) {
        let dash = 6.0;
        let mut x = frame.left();
        while x < frame.right() {
            let end = (x + dash).min(frame.right());
            painter.line_segment(
                [pos2(x, frame.top()), pos2(end, frame.top())],
                Stroke::new(px, color),
            );
            painter.line_segment(
                [pos2(x, frame.bottom()), pos2(end, frame.bottom())],
                Stroke::new(px, color),
            );
            x += dash * 2.0;
        }
        let mut y = frame.top();
        while y < frame.bottom() {
            let end = (y + dash).min(frame.bottom());
            painter.line_segment(
                [pos2(frame.left(), y), pos2(frame.left(), end)],
                Stroke::new(px, color),
            );
            painter.line_segment(
                [pos2(frame.right(), y), pos2(frame.right(), end)],
                Stroke::new(px, color),
            );
            y += dash * 2.0;
        }
    }

    /// Ends the edit session: saves (or reverts) every panel's position and hands input back
    /// to the game.
    fn finish_edit(&mut self, confirmed: bool) {
        if confirmed {
            for panel in &mut self.panels {
                panel.saved = panel.offset;
            }
            if let Some(img) = self.image.as_mut() {
                img.saved = (img.x, img.y, img.w, img.h);
            }
        } else {
            for panel in &mut self.panels {
                panel.offset = panel.saved;
            }
            if let Some(img) = self.image.as_mut() {
                img.x = img.saved.0;
                img.y = img.saved.1;
                img.w = img.saved.2;
                img.h = img.saved.3;
            }
        }
        self.image_grip = None;
        self.image_drag = None;
        self.spec_gen = self.spec_gen.wrapping_add(1);
        self.write_result(confirmed);
        self.edit = false;
        // The spec still says `edit: true` until the launcher reads the answer; without this
        // the next reload (the session timer ticks every second) would re-open the editor.
        self.edit_done = true;
        self.dragging = None;
        self.hovering = None;
        if self.interactive {
            self.interactive = false;
            tweak_overlay_window(false);
        }
    }

    /// Leaves the answer for the launcher, atomically so a half-written file is never read.
    fn write_result(&self, confirmed: bool) {
        let path = self.spec_path.with_file_name("overlay-position.json");
        let panels: Vec<Value> = self
            .panels
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "x": p.offset.x.round() as i32,
                    "y": p.offset.y.round() as i32
                })
            })
            .collect();
        // Only an image the user actually moved or resized is frozen into a box; one that was
        // left alone keeps following its corner and height slider.
        let image = match &self.image {
            Some(img) if img.w > 0 => self.image_box.map(|b| {
                serde_json::json!({ "x": b.x, "y": b.y, "w": b.w.max(0), "h": b.h.max(0) })
            }),
            _ => None,
        };
        let body = serde_json::json!({
            "state": if confirmed { "confirmed" } else { "cancelled" },
            "panels": panels,
            "image": image
        })
        .to_string();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
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

/// Where the custom image goes: the editor's saved box when there is one, otherwise its
/// corner slot and height percentage, kept at the picture's own aspect ratio.
///
/// Returns the box in game-area coordinates (what the editor and settings use) and the same
/// box in window coordinates (what gets painted).
fn image_rect(
    area: Rect,
    title_bar: f32,
    img: &OverlayImage,
    tex: egui::Vec2,
    view: (f32, f32),
) -> (crate::overlay_spec::ImageBox, Rect) {
    let min = crate::overlay_spec::ImageBox::MIN as u16;
    let box_ = if img.w >= min && img.h >= min {
        crate::overlay_spec::ImageBox {
            x: img.x.round() as i16,
            y: img.y.round() as i16,
            w: img.w as i16,
            h: img.h as i16,
        }
    } else {
        let size = crate::overlay_spec::ImageBox::size((tex.x, tex.y), img.size_pct, view.1, (0, 0));
        crate::overlay_spec::ImageBox::placed(
            img.corner,
            size,
            view,
            (img.offset_x, img.offset_y),
        )
    }
    .clamped(view);
    let rect = Rect::from_min_size(
        pos2(area.left() + box_.x as f32, area.top() + title_bar + box_.y as f32),
        vec2(box_.w as f32, box_.h as f32),
    );
    (box_, rect)
}

/// Paints the image into a rect the editor chose (never stretched beyond it: the texture keeps
/// its own aspect ratio inside the box, centred).
fn paint_image_rect(painter: &egui::Painter, rect: Rect, opacity: f32, tex: &egui::TextureHandle) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let alpha = (opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    painter.image(
        tex.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::from_white_alpha(alpha),
    );
}

/// Writes an edited image box back onto the live picture and marks the layout stale.
fn apply_image_box(img: &mut OverlayImage, box_: crate::overlay_spec::ImageBox) {
    img.x = box_.x as f32;
    img.y = box_.y as f32;
    img.w = box_.w.max(0) as u16;
    img.h = box_.h.max(0) as u16;
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
        // The editor is the only state in which the window may take clicks. Sending this as a
        // viewport command (not only the native tweak) keeps winit's own cursor-hit-test state
        // in step, which is what makes a drag reach egui at all.
        let want_passthrough = !self.interactive;
        if self.passthrough != Some(want_passthrough) {
            self.passthrough = Some(want_passthrough);
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(want_passthrough));
        }
        // Advance the clock before drawing, so the frame that wakes on the boundary already
        // shows the new second instead of painting the old one and sleeping another tick.
        let tick = if self.visible || self.edit {
            self.tick_timers(overlay_spec::unix_ms_now())
        } else {
            None
        };
        self.paint(ui);
        // Event-driven, not a loop: 60 fps only while the user is dragging. With a session
        // timer on screen the wake-up is scheduled for the exact next whole second, so one
        // frame draws each tick and nothing in between; without one, two wake-ups a second
        // while the HUD is up and one while it is hidden. The old build had a 150 ms repaint
        // plus a 400 ms heartbeat thread, i.e. ~9 full transparent composites a second
        // whether anything had changed or not.
        let interval = if self.edit || self.dragging.is_some() {
            Duration::from_millis(16)
        } else if let Some(ms) = tick {
            Duration::from_millis(ms.clamp(1, 1000))
        } else if self.visible {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(1000)
        };
        ctx.request_repaint_after(interval);
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
        // Deliberately **not** `with_mouse_passthrough(true)`: winit keeps that state on the
        // window, so it silently overrode the AppKit `setIgnoresMouseEvents:NO` that edit mode
        // performs and every click fell through the picture. Passthrough is now driven by
        // `ViewportCommand::MousePassthrough` from `ui()`, which the window backend applies.
        // Tool-window style: the HUD must never add a second taskbar button.
        .with_taskbar(false)
        .with_resizable(false);
    // `with_active(false)` used to be set here. It tells winit not to activate the app, but
    // an inactive, accessory-policy process is also the one winit is least willing to order
    // front - and a window that is never ordered front is exactly what "the overlay does not
    // work" looks like. The accessory policy already keeps Roblox's focus intact.
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
            // No heartbeat thread: `ui()` schedules its own wake-ups (500 ms idle, 16 ms
            // while dragging), so an idle overlay costs one frame every two seconds.
            Ok(Box::new(OverlayWindowApp::new(spec_path)))
        }),
    )
}
