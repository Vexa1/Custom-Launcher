//! The macOS side of the memory-pressure reaction: a dispatch source that posts a level.
//!
//! Nothing is flushed here. The source records the level, wakes egui through the tray's shared
//! context, and returns; `App::poll` picks the level up on the next frame and calls into the
//! same `flush_image_caches` path a tray tuck uses. That keeps every texture drop on a frame,
//! where egui expects its texture list to change.

use crate::memory_pressure::Level;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};

/// The level the source last posted.
static LEVEL: AtomicU8 = AtomicU8::new(0);

/// `dispatch_source_type_memorypressure`, an opaque libdispatch object.
#[repr(C)]
struct DispatchSourceTypeS {
    _private: [u8; 0],
}

type DispatchFunction = unsafe extern "C" fn(*mut c_void);

unsafe extern "C" {
    static _dispatch_source_type_memorypressure: DispatchSourceTypeS;
    fn dispatch_source_create(
        kind: *const DispatchSourceTypeS,
        handle: usize,
        mask: usize,
        queue: *mut c_void,
    ) -> *mut c_void;
    fn dispatch_source_set_event_handler_f(
        source: *mut c_void,
        handler: DispatchFunction,
        context: *mut c_void,
    );
    fn dispatch_source_get_data(source: *mut c_void) -> usize;
    fn dispatch_resume(object: *mut c_void);
    // `dispatch_get_main_queue()` is a C macro in <dispatch/queue.h> that expands to
    // `&_dispatch_main_q`; it is not an exported function. Declaring it as one compiles and
    // passes `cargo check`, then fails at `ld` with "_dispatch_get_main_queue, referenced
    // from ...". The queue object below is the real libSystem symbol.
    static _dispatch_main_q: c_void;
}

const NORMAL: usize = 0x01;
const WARN: usize = 0x02;
const CRITICAL: usize = 0x04;

/// Runs on the main queue. Only records and wakes; never touches a texture.
unsafe extern "C" fn on_pressure(source: *mut c_void) {
    let events = unsafe { dispatch_source_get_data(source) };
    let level = if events & CRITICAL != 0 {
        Level::Critical
    } else if events & WARN != 0 {
        Level::Warn
    } else if events & NORMAL != 0 {
        Level::None
    } else {
        return;
    };
    LEVEL.store(level as u8, Ordering::Release);
    crate::auth::log(&format!("memory: macOS reported {level:?} pressure"));
    // The flush belongs to a frame, so ask for one instead of doing the work here.
    crate::tray::request_repaint_now();
}

/// Starts the memory-pressure source on the main queue. Only the first call does anything.
pub fn start() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    unsafe {
        let source = dispatch_source_create(
            std::ptr::addr_of!(_dispatch_source_type_memorypressure),
            0,
            NORMAL | WARN | CRITICAL,
            std::ptr::addr_of!(_dispatch_main_q) as *mut c_void,
        );
        if source.is_null() {
            crate::auth::log("memory: could not create the pressure source");
            return;
        }
        dispatch_source_set_event_handler_f(source, on_pressure, source);
        dispatch_resume(source);
    }
    crate::auth::log("memory: watching macOS memory pressure");
}

/// Takes the pending level, leaving the source ready for the next report.
pub fn take_level() -> Level {
    match LEVEL.swap(0, Ordering::AcqRel) {
        2 => Level::Critical,
        1 => Level::Warn,
        _ => Level::None,
    }
}
