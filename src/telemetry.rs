//! Process and resource helpers.
//!
//! The launcher used to draw a live FPS / RAM / CPU dock in the sidebar. Those stats now
//! live in the in-game overlays, so the dock, its sampler and its painters are gone. What
//! remains here is the plumbing the app still needs: the RAM/CPU limits it applies to
//! itself, the current memory reading, and the key the UI uses to tell the idle animation
//! pacer whether the window is on screen.

#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use std::sync::OnceLock;

/// `ctx.data` key the app uses to tell the idle animation pacer whether the window is on screen.
pub const WINDOW_VISIBLE_KEY: &str = "creblox_window_visible";

#[cfg(windows)]
fn read_process_memory_mb() -> Option<f32> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) == 0 {
            return None;
        }
        Some((pmc.WorkingSetSize as f64 / (1024.0 * 1024.0)) as f32)
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn read_process_memory_mb() -> Option<f32> {
    None
}

#[cfg(target_os = "macos")]
fn read_process_memory_mb() -> Option<f32> {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            std::process::id() as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if n <= 0 {
        return None;
    }
    Some((info.pti_resident_size as f64 / (1024.0 * 1024.0)) as f32)
}

pub fn current_process_ram_mb() -> Option<f32> {
    read_process_memory_mb()
}

pub fn max_video_cache_for_ram_limit(ram_limit_mb: Option<u32>) -> usize {
    match ram_limit_mb {
        None => 2,
        Some(mb) if mb <= 320 => 1,
        Some(mb) if mb <= 640 => 2,
        Some(_) => 3,
    }
}

/// Windows CPU-rate control takes hundredths of a percent.
#[cfg(windows)]
fn compute_cpu_rate_param(cpu_limit_pct: Option<u8>) -> Option<u32> {
    cpu_limit_pct
        .map(|pct| pct.clamp(10, 100))
        .filter(|&pct| pct < 100)
        .map(|pct| (pct as u32) * 100)
}

#[cfg(windows)]
pub fn trim_process_memory() -> bool {
    use windows_sys::Win32::System::ProcessStatus::K32EmptyWorkingSet;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe { K32EmptyWorkingSet(GetCurrentProcess()) != 0 }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn trim_process_memory() -> bool {
    false
}

/// Hands freed pages back to the OS now instead of waiting for the allocator to reuse them.
/// This is what makes the RAM limit actually bite on macOS instead of only labelling itself.
#[cfg(target_os = "macos")]
pub fn trim_process_memory() -> bool {
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }
    // A null zone means every zone.
    unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) > 0 }
}

/// CPU limiter on macOS: lower the app's scheduling priority when a cap is set
/// (macOS has no hard per-process CPU quota for unprivileged apps).
#[cfg(target_os = "macos")]
pub fn apply_resource_limits(ram_limit_mb: Option<u32>, cpu_limit_pct: Option<u8>) {
    let nice = match cpu_limit_pct {
        Some(pct) if pct <= 25 => 15,
        Some(pct) if pct <= 50 => 10,
        Some(pct) if pct <= 75 => 5,
        _ => 0,
    };
    // `setpriority` is a syscall; only make it when the level actually changes.
    static LAST_NICE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
    if LAST_NICE.swap(nice, std::sync::atomic::Ordering::Relaxed) == nice {
        return;
    }
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, nice);
    }

    // Enforcement: macOS has no per-process memory quota, so when the limit is exceeded we
    // release allocator pages immediately. Cheap, and it only runs over the cap.
    if let Some(mb) = ram_limit_mb {
        if read_process_memory_mb().is_some_and(|cur| cur > mb as f32) {
            trim_process_memory();
        }
    }
}

#[cfg(windows)]
fn ensure_cpu_limit_job() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    static JOB_HANDLE: OnceLock<Option<isize>> = OnceLock::new();
    let stored = *JOB_HANDLE.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        // Allow spawned RobloxPlayerBeta.exe processes to break away uncapped.
        let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        ext.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const ext).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return None;
        }
        Some(job as isize)
    });
    stored.map(|v| v as windows_sys::Win32::Foundation::HANDLE)
}

#[cfg(windows)]
pub fn apply_resource_limits(ram_limit_mb: Option<u32>, cpu_limit_pct: Option<u8>) {
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
        JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JobObjectCpuRateControlInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Memory::{
        QUOTA_LIMITS_HARDWS_MAX_DISABLE, SetProcessWorkingSetSizeEx,
    };
    use windows_sys::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, NORMAL_PRIORITY_CLASS, SetPriorityClass,
    };

    static LAST_APPLIED: OnceLock<Mutex<Option<(Option<u32>, Option<u8>)>>> = OnceLock::new();
    let norm_ram = ram_limit_mb.map(|mb| mb.clamp(128, 4096));
    let norm_cpu = cpu_limit_pct.map(|p| p.clamp(10, 100)).filter(|&p| p < 100);
    let next = (norm_ram, norm_cpu);

    let cache = LAST_APPLIED.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        if *guard == Some(next) {
            return;
        }
        *guard = Some(next);
    }

    unsafe {
        let proc_h = GetCurrentProcess();
        if let Some(mb) = norm_ram {
            let max_ws = (mb as usize).saturating_mul(1024 * 1024);
            let min_ws = (16 * 1024 * 1024usize).min(max_ws / 4);
            let _ =
                SetProcessWorkingSetSizeEx(proc_h, min_ws, max_ws, QUOTA_LIMITS_HARDWS_MAX_DISABLE);
            if read_process_memory_mb().is_some_and(|cur| cur > mb as f32) {
                let _ = trim_process_memory();
            }
        } else {
            let _ = SetProcessWorkingSetSizeEx(
                proc_h,
                usize::MAX,
                usize::MAX,
                QUOTA_LIMITS_HARDWS_MAX_DISABLE,
            );
        }

        if let Some(job) = ensure_cpu_limit_job() {
            let mut info: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION = std::mem::zeroed();
            if let Some(rate) = compute_cpu_rate_param(norm_cpu) {
                info.ControlFlags =
                    JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
                info.Anonymous.CpuRate = rate;
            }
            let _ = SetInformationJobObject(
                job,
                JobObjectCpuRateControlInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
            );
        }

        let prio = if norm_cpu.is_some_and(|p| p <= 50) {
            BELOW_NORMAL_PRIORITY_CLASS
        } else {
            NORMAL_PRIORITY_CLASS
        };
        let _ = SetPriorityClass(proc_h, prio);
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn apply_resource_limits(_ram_limit_mb: Option<u32>, _cpu_limit_pct: Option<u8>) {}
