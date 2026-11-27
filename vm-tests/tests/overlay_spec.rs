//! The overlay HUD must stop saying `Server info unavailable` / `Ping -- ms` the moment the
//! watcher resolves the server it is in.
//!
//! `src/game_overlay.rs` collects the live state and hands it to `src/overlay_spec.rs`, which
//! builds the exact `panels` payload the overlay window parses. These tests drive that builder
//! through the transition that used to freeze the HUD: first with no server (the snapshot the
//! overlay window opened with), then with the resolved server, its region and a measured RTT.

use creblox_vm_tests::overlay_spec::{
    self, HudToggles, Live, LiveServer, Panels, ServerPanel, StatsPanel, TrayToggle,
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

/// The whole file the overlay window reads for one HUD state: the panels those switches
/// select (exactly as `OverlayConfig::from_settings` builds them) plus the crosshair entry.
fn spec_for(toggles: HudToggles, crosshair: bool) -> String {
    let server = server_cfg();
    let stats = stats_cfg();
    let panels = overlay_spec::build(
        &Panels {
            server: toggles.server.then_some(&server),
            stats: toggles.stats.then_some(&stats),
            flags: None,
            game: None,
        },
        &resolved(),
    );
    let mut spec = overlay_spec::spec_json(0.9, true, 0, 0, false, panels);
    overlay_spec::set_crosshair(
        &mut spec,
        crosshair.then(|| {
            serde_json::json!({
                "style": 3,
                "size": 18,
                "thickness": 2,
                "gap": 4,
                "color": [255, 60, 60],
                "opacity": 0.9,
                "offset_x": 0,
                "offset_y": 0,
                "image": ""
            })
        }),
    );
    spec.to_string()
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

/// Every tray check item must change the payload the overlay reads, with no restart: the menu
/// hands the click to `apply_tray_toggle`, which flips one field and wakes the writer.
#[test]
fn every_tray_toggle_rewrites_the_panels_spec() {
    let all_on = HudToggles {
        server: true,
        stats: true,
        crosshair: true,
    };
    let before = spec_for(all_on, all_on.crosshair);
    assert!(before.contains("\"id\":\"server\""), "{before}");
    assert!(before.contains("\"id\":\"stats\""), "{before}");
    assert!(before.contains("\"crosshair\""), "{before}");

    // Server region overlay off: its panel is gone, the others are untouched.
    let no_server = spec_for(all_on.flipped(TrayToggle::Server), all_on.crosshair);
    assert!(!no_server.contains("\"id\":\"server\""), "{no_server}");
    assert!(no_server.contains("\"id\":\"stats\""), "{no_server}");
    assert!(no_server.contains("\"crosshair\""), "{no_server}");
    assert_ne!(before, no_server, "the overlay must see new bytes");

    // Stats overlay off: only that panel disappears.
    let no_stats = spec_for(all_on.flipped(TrayToggle::Stats), all_on.crosshair);
    assert!(!no_stats.contains("\"id\":\"stats\""), "{no_stats}");
    assert!(no_stats.contains("\"id\":\"server\""), "{no_stats}");
    assert_ne!(before, no_stats);

    // Crosshair off: the key is removed, not blanked - that is what stops the drawing.
    let no_crosshair = spec_for(all_on, false);
    assert!(!no_crosshair.contains("crosshair"), "{no_crosshair}");
    assert!(no_crosshair.contains("\"id\":\"server\""), "{no_crosshair}");
    assert!(no_crosshair.contains("\"id\":\"stats\""), "{no_crosshair}");
    assert_ne!(before, no_crosshair);

    // Clicking every item again restores exactly the file the overlay opened with.
    let round_trip = all_on
        .flipped(TrayToggle::Server)
        .flipped(TrayToggle::Stats)
        .flipped(TrayToggle::Server)
        .flipped(TrayToggle::Stats);
    assert_eq!(round_trip, all_on);
    let crosshair_back = all_on.flipped(TrayToggle::Crosshair).flipped(TrayToggle::Crosshair);
    assert_eq!(spec_for(crosshair_back, crosshair_back.crosshair), before);
}
