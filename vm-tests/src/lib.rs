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

// Headless UI rendering (dev-only): real egui, software rasterizer, PNGs in `artifacts/`.
pub mod hud;
pub mod screens;
pub mod shots;

#[path = "../../src/avatar_anim.rs"]
pub mod avatar_anim;

pub use ::egui;

#[path = "../../src/avatar_render.rs"]
pub mod avatar_render;

#[path = "../../src/avatar_scene.rs"]
pub mod avatar_scene;

#[path = "../../src/avatar_sync.rs"]
pub mod avatar_sync;

#[path = "../../src/chart_feed.rs"]
pub mod chart_feed;

#[path = "../../src/friend_names.rs"]
pub mod friend_names;

#[path = "../../src/live_refresh.rs"]
pub mod live_refresh;

#[path = "../../src/thumb_alias.rs"]
pub mod thumb_alias;

#[path = "../../src/thumb_pipeline.rs"]
pub mod thumb_pipeline;

#[path = "../../src/game_log.rs"]
pub mod game_log;

#[path = "../../src/inventory.rs"]
pub mod inventory;

#[path = "../../src/net_probe.rs"]
pub mod net_probe;

#[path = "../../src/overlay_spec.rs"]
pub mod overlay_spec;

#[path = "../../src/profile_cache.rs"]
pub mod profile_cache;

#[path = "../../src/party.rs"]
pub mod party;

#[path = "../../src/party_server.rs"]
pub mod party_server;

#[path = "../../src/scroll.rs"]
pub mod scroll;

#[path = "../../src/server_hop.rs"]
pub mod server_hop;

#[path = "../../src/release_links.rs"]
pub mod release_links;

#[path = "../../src/sha256.rs"]
pub mod sha256;

#[path = "../../src/window_lifecycle.rs"]
pub mod window_lifecycle;

#[path = "../../src/zip_update.rs"]
pub mod zip_update;
