//! The overlay HUD must stop saying `Server info unavailable` / `Ping -- ms` the moment the
//! watcher resolves the server it is in.
//!
//! `src/game_overlay.rs` collects the live state and hands it to `src/overlay_spec.rs`, which
//! builds the exact `panels` payload the overlay window parses. These tests drive that builder
//! through the transition that used to freeze the HUD: first with no server (the snapshot the
//! overlay window opened with), then with the resolved server, its region and a measured RTT.

use creblox_vm_tests::overlay_spec::{
    self, Live, LiveServer, Panels, ServerPanel, StatsPanel,
};

fn server_cfg() -> ServerPanel {
    ServerPanel {
        corner: 2,
        duration_secs: 0,
        show_ip: true,
        show_job_id: true,
    }
}

fn stats_cfg() -> StatsPanel {
    StatsPanel {
        corner: 3,
        cpu: true,
        ram: true,
        ping: true,
        session: true,
        clock: false,
        fps_cap: None,
    }
}

/// Before the player log names a server: nothing resolved, no region, no measurement.
fn unresolved() -> Live<'static> {
    Live {
        server: None,
        server_age_secs: 0,
        session_secs: 30,
        detect_secs: 30,
        region: None,
        ping_ms: None,
        game: None,
        cpu_pct: None,
        ram_mb: None,
        clock: "12:00".to_string(),
        edit_preview: false,
    }
}

/// The same session five seconds later, once `watcher: server place 142823291` fired.
fn resolved() -> Live<'static> {
    Live {
        server: Some(LiveServer {
            ip: "203.0.113.7",
            job_id: "job-abc",
            place_id: 142_823_291,
        }),
        server_age_secs: 5,
        session_secs: 35,
        detect_secs: 35,
        region: Some("Sydney, AU"),
        ping_ms: Some(35),
        game: None,
        cpu_pct: Some(12.0),
        ram_mb: Some(640.0),
        clock: "12:00".to_string(),
        edit_preview: false,
    }
}

fn spec(live: &Live) -> String {
    let server = server_cfg();
    let stats = stats_cfg();
    let panels = overlay_spec::build(
        &Panels {
            server: Some(&server),
            stats: Some(&stats),
            flags: None,
            game: None,
        },
        live,
    );
    overlay_spec::spec_json(0.9, true, 0, 0, false, panels).to_string()
}

#[test]
fn unresolved_then_resolved_server_and_ping_rewrites_the_panels() {
    let before = spec(&unresolved());
    assert!(
        before.contains("Server info unavailable"),
        "the HUD starts without a server: {before}"
    );
    assert!(before.contains("Ping     -- ms"), "{before}");
    assert!(!before.contains("Sydney"), "{before}");

    let after = spec(&resolved());
    assert!(after.contains("Server: Sydney, AU"), "{after}");
    assert!(after.contains("203.0.113.7"), "the resolved address: {after}");
    assert!(after.contains("Job ID: job-abc"), "{after}");
    assert!(after.contains("Ping     35 ms"), "{after}");
    assert!(!after.contains("unavailable"), "{after}");
    assert!(!after.contains("-- ms"), "no row may stay blank: {after}");
    assert!(after.contains("CPU      12%"), "{after}");
    assert!(after.contains("RAM      640 MB"), "{after}");

    assert_ne!(before, after, "the payload the overlay reads must change");
}

#[test]
fn a_resolved_server_is_shown_before_its_region_and_ping_arrive() {
    // `set_live_server` lands the address first; the geolocation and probe answer a beat later.
    // The panel must switch to the server immediately and only the numbers wait.
    let mut live = resolved();
    live.region = None;
    live.ping_ms = None;
    let text = spec(&live);
    assert!(text.contains("Server: locating..."), "{text}");
    assert!(!text.contains("unavailable"), "{text}");
    assert!(text.contains("Ping     -- ms"), "{text}");

    let with_region = spec(&resolved());
    assert!(with_region.contains("Server: Sydney, AU"), "{with_region}");
    assert!(with_region.contains("Ping     35 ms"), "{with_region}");
}

#[test]
fn the_server_panel_expires_only_after_its_configured_duration() {
    let mut cfg = server_cfg();
    cfg.duration_secs = 10;
    let mut live = resolved();
    let stats = stats_cfg();

    live.server_age_secs = 9;
    let shown = overlay_spec::build(
        &Panels {
            server: Some(&cfg),
            stats: Some(&stats),
            flags: None,
            game: None,
        },
        &live,
    );
    assert!(shown.iter().any(|p| p["id"] == "server"));

    live.server_age_secs = 11;
    let hidden = overlay_spec::build(
        &Panels {
            server: Some(&cfg),
            stats: Some(&stats),
            flags: None,
            game: None,
        },
        &live,
    );
    assert!(!hidden.iter().any(|p| p["id"] == "server"));
    // The ping row lives in its own panel and must keep updating.
    assert!(hidden
        .iter()
        .any(|p| p["lines"].to_string().contains("Ping     35 ms")));
}
