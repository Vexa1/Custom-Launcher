//! Dev-only harness for the launcher's host-independent modules.
//!
//! Each module below is the *real* source file from `src/`, included by path and backed by the
//! small stand-ins in this crate for the parts that need a window, a Roblox session or macOS.
//! `cargo test --manifest-path vm-tests/Cargo.toml` therefore exercises the shipping code in a
//! Linux VM, where the GUI crate itself cannot compile.
//!
//! This package is not referenced by `Cargo.toml` at the repository root and ships in no
//! release artifact.

pub mod api;
pub mod auth;
pub mod roblox_process;
pub mod types;

#[path = "../../src/party.rs"]
pub mod party;

#[path = "../../src/party_server.rs"]
pub mod party_server;

#[path = "../../src/scroll.rs"]
pub mod scroll;

#[path = "../../src/server_hop.rs"]
pub mod server_hop;
