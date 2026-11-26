//! Stand-in for `src/auth.rs`: the harness has no Roblox session, and its data folder is a
//! scratch directory so a test can never touch the real
//! `~/Library/Application Support/CREBLOX`.

use std::path::PathBuf;

/// `CREBLOX_VM_DATA` lets a test point at its own scratch directory.
pub fn data_dir() -> PathBuf {
    std::env::var_os("CREBLOX_VM_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("creblox-vm-tests"))
}

pub fn log(_line: &str) {}
