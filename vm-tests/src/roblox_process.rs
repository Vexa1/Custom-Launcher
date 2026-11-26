//! Stand-in for `src/roblox_process.rs`: nothing is launched or killed in the VM, so the
//! tests can drive `server_hop::run_hop` and the party launch handler without a client.

use std::sync::Mutex;

static LAUNCHES: Mutex<Vec<(u64, Option<String>, String)>> = Mutex::new(Vec::new());

pub fn kill_all_roblox_processes() -> usize {
    0
}

pub fn launch_authenticated_server(
    place_id: u64,
    job_id: Option<&str>,
    cookie: &str,
) -> Result<String, String> {
    if let Ok(mut launches) = LAUNCHES.lock() {
        launches.push((place_id, job_id.map(str::to_string), cookie.to_string()));
    }
    Ok("joined the new server".to_string())
}

/// Every launch recorded since the last call, in order: `(place id, job id, cookie)`.
pub fn take_launches() -> Vec<(u64, Option<String>, String)> {
    LAUNCHES
        .lock()
        .map(|mut launches| std::mem::take(&mut *launches))
        .unwrap_or_default()
}
