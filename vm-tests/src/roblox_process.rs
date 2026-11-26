//! Stand-in for `src/roblox_process.rs`: nothing is launched or killed in the VM, so the
//! tests can drive `server_hop::run_hop` without touching a client.

pub fn kill_all_roblox_processes() -> usize {
    0
}

pub fn launch_authenticated_server(
    _place_id: u64,
    _job_id: Option<&str>,
    _cookie: &str,
) -> Result<String, String> {
    Ok("joined the new server".to_string())
}
