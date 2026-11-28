//! The overlay HUD must stop saying `Server info unavailable` / `Ping -- ms` the moment the
//! watcher resolves the server it is in.
//!
//! `src/game_overlay.rs` collects the live state and hands it to `src/overlay_spec.rs`, which
//! builds the exact `panels` payload the overlay window parses. These tests drive that builder
//! through the transition that used to freeze the HUD: first with no server (the snapshot the
//! overlay window opened with), then with the resolved server, its region and a measured RTT.

use creblox_vm_tests::overlay_spec::{
    self, GamePanel, HudToggles, Live, LiveServer, Panels, ServerPanel, StatsPanel, TrayToggle,
};

/// A fixed Unix-ms anchor. Every timer assertion below is arithmetic on this, never on the
/// wall clock, so a slow CI machine cannot change what the HUD is expected to say.
const ANCHOR: u64 = 1_700_000_000_123;

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
    let mut spec = overlay_spec::spec_json(0.9, true, false, panels);
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
        joined_at_unix_ms: ANCHOR,
        detect_secs: 30,
        region: None,
        ping_ms: None,
        ping_path: None,
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
        joined_at_unix_ms: ANCHOR,
        detect_secs: 35,
        region: Some("Sydney, AU"),
        ping_ms: Some(35),
        ping_path: None,
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
    overlay_spec::spec_json(0.9, true, false, panels).to_string()
}

/// The launcher writes an anchor and a token, never a formatted clock: the overlay window is
/// what makes the seconds move, so a frozen panels file cannot freeze the row.
#[test]
fn the_timer_rows_are_anchors_the_overlay_ticks_itself() {
    let game = GamePanel { corner: 2 };
    let mut live = resolved();
    live.joined_at_unix_ms = ANCHOR;
    let panels = overlay_spec::build(
        &Panels {
            server: Some(&server_cfg()),
            stats: Some(&stats_cfg()),
            flags: None,
            game: Some(&game),
        },
        &live,
    );
    let game_panel = panels
        .iter()
        .find(|p| p["id"] == "game")
        .expect("the game panel");
    let stats_panel = panels
        .iter()
        .find(|p| p["id"] == "stats")
        .expect("the stats panel");

    for panel in [game_panel, stats_panel] {
        assert_eq!(
            panel["joined_at_unix_ms"].as_u64(),
            Some(ANCHOR),
            "the panel carries the raw timestamp: {panel}"
        );
    }
    let game_lines = game_panel["lines"].to_string();
    assert!(
        game_lines.contains(&format!("In server {}", overlay_spec::ELAPSED)),
        "{game_lines}"
    );
    let stats_lines = stats_panel["lines"].to_string();
    assert!(
        stats_lines.contains(&format!("Session  {}", overlay_spec::ELAPSED)),
        "{stats_lines}"
    );
    // Nothing time-shaped went into the file: 0:05 for the server, 0:35 for the session.
    assert!(!game_lines.contains("0:05"), "{game_lines}");
    assert!(!stats_lines.contains("0:35"), "{stats_lines}");
}

/// Ten seconds of frames, sampled four times a second: the row is stable inside a second and
/// changes exactly on the boundary, so the clock neither freezes nor skips.
#[test]
fn the_timer_advances_exactly_one_second_at_a_time() {
    let line = format!("Session  {}", overlay_spec::ELAPSED);
    let mut seen: Vec<String> = Vec::new();
    for quarter in 0..44u64 {
        let now = ANCHOR + quarter * 250;
        let text = overlay_spec::with_elapsed(&line, ANCHOR, now);
        assert_eq!(
            overlay_spec::elapsed_secs(ANCHOR, now),
            quarter / 4,
            "whole seconds at {quarter} ms"
        );
        if seen.last() != Some(&text) {
            seen.push(text);
        }
    }
    assert_eq!(
        seen,
        vec![
            "Session  00:00",
            "Session  00:01",
            "Session  00:02",
            "Session  00:03",
            "Session  00:04",
            "Session  00:05",
            "Session  00:06",
            "Session  00:07",
            "Session  00:08",
            "Session  00:09",
            "Session  00:10",
        ],
        "one new value per whole second, in order"
    );

    // A whole session, minute rollover included: every second is rendered once, in order.
    for secs in 0..=3_600u64 {
        let text = overlay_spec::with_elapsed(&line, ANCHOR, ANCHOR + secs * 1000);
        assert_eq!(text, format!("Session  {}", overlay_spec::fmt_secs(secs)));
    }
    assert_eq!(overlay_spec::fmt_secs(599), "09:59");
    assert_eq!(overlay_spec::fmt_secs(600), "10:00");
    assert_eq!(overlay_spec::fmt_secs(3599), "59:59");
    assert_eq!(overlay_spec::fmt_secs(3600), "1:00:00");

    // A row without the token is left exactly as it was.
    assert_eq!(
        overlay_spec::with_elapsed("Ping     35 ms", ANCHOR, ANCHOR + 90_000),
        "Ping     35 ms"
    );
}

/// The next frame is scheduled for the exact next whole second of the timer: one wake-up per
/// tick, landing on the boundary, and never a wait longer than a second or a zero-length busy
/// loop.
#[test]
fn the_next_repaint_lands_on_the_next_whole_second() {
    for offset in [0u64, 1, 123, 999, 1000, 1001, 61_000, 3_599_877, 7_200_500] {
        let now = ANCHOR + offset;
        let wait = overlay_spec::ms_until_next_second(ANCHOR, now);
        assert!(
            (1..=1000).contains(&wait),
            "the wait is one second at most and never zero (offset {offset}: {wait})"
        );
        assert_eq!(
            (now + wait) % 1000,
            ANCHOR % 1000,
            "the wake-up lands on a whole second (offset {offset})"
        );
        assert_eq!(
            (now + wait - 1) % 1000,
            (ANCHOR + 999) % 1000,
            "and on the next one, not a later one (offset {offset})"
        );
    }
    // A clock that reads behind the anchor is a zero, not an overflow.
    assert_eq!(overlay_spec::ms_until_next_second(ANCHOR, ANCHOR - 5), 1000);
    assert_eq!(overlay_spec::elapsed_secs(ANCHOR, ANCHOR - 5), 0);
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

/// `-- ms` alone does not say what failed. The HUD names the path whose probe came back
/// empty, so the answer is on screen instead of only in `log.txt`.
#[test]
fn a_missing_ping_names_the_path_that_failed() {
    let mut live = resolved();
    live.ping_ms = None;
    live.ping_path = Some("tcp");
    let text = spec(&live);
    assert!(text.contains("Ping     -- ms (tcp)"), "{text}");

    // Even the room-list fallback can miss; the row says so.
    live.ping_path = Some("room list");
    let text = spec(&live);
    assert!(text.contains("Ping     -- ms (room list)"), "{text}");

    // A stale path never rides along with a real measurement.
    live.ping_ms = Some(41);
    let text = spec(&live);
    assert!(text.contains("Ping     41 ms"), "{text}");
    assert!(!text.contains("room list"), "{text}");

    // Before the first probe there is no path to name: the row stays plain.
    live.ping_ms = None;
    live.ping_path = None;
    let text = spec(&live);
    assert!(text.contains("Ping     -- ms\""), "no suffix without a path: {text}");
}

/// One physical menu click can reach the tray thread and the window's drain (macOS delivers a
/// status-menu action to every reader of the menu channel). The second copy must be dropped,
/// or it flips the switch straight back and leaves the HUD where it started.
#[test]
fn a_duplicate_tray_click_is_applied_once() {
    use std::time::{Duration, Instant};

    let mut seen = overlay_spec::ClickDebounce::new();
    let t0 = Instant::now();
    assert!(seen.accept(TrayToggle::Crosshair, t0), "the click itself");
    assert!(
        !seen.accept(TrayToggle::Crosshair, t0 + Duration::from_millis(3)),
        "a second copy of the same click must not flip the switch back"
    );
    assert!(
        seen.accept(TrayToggle::Stats, t0 + Duration::from_millis(4)),
        "a different switch is never a duplicate"
    );
    // A real second click arrives after the menu has closed and been reopened.
    assert!(seen.accept(
        TrayToggle::Crosshair,
        t0 + Duration::from_millis(4) + overlay_spec::CLICK_DEBOUNCE
    ));
}

/// Every tray check item must change the payload the overlay reads, with no restart: the menu
/// hands the click to `apply_tray_toggle`, which flips one field and wakes the writer.
#[test]
fn every_tray_toggle_rewrites_the_panels_spec() {
    let all_on = HudToggles {
        server: true,
        stats: true,
        crosshair: true,
        party: true,
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

// ---------------------------------------------------------------------------------------
// The custom image's box: the geometry the editor drags and settings.json stores.
// ---------------------------------------------------------------------------------------

use creblox_vm_tests::overlay_spec::ImageBox;

#[test]
fn a_new_image_sits_in_its_corner_at_the_size_slider_height() {
    // 160x64 picture, 25% of an 800 pt view = 200 pt tall, 500 wide at its own aspect ratio,
    // in the top-right slot with a 16 pt gap.
    let size = ImageBox::size((160.0, 64.0), 25, 800.0, (0, 0));
    assert_eq!(size, (500.0, 200.0));
    let top_right = ImageBox::placed(1, size, (1280.0, 800.0), (0.0, 0.0));
    assert_eq!((top_right.x, top_right.y), (764, 16));
    let bottom_left = ImageBox::placed(2, size, (1280.0, 800.0), (0.0, 0.0));
    assert_eq!((bottom_left.x, bottom_left.y), (16, 584));
    // The offsets are the fine-tune sliders, so they shift the corner slot.
    let nudged = ImageBox::placed(1, size, (1280.0, 800.0), (-30.0, 12.0));
    assert_eq!((nudged.x, nudged.y), (734, 28));
}

#[test]
fn a_saved_box_wins_over_the_percentage() {
    let size = ImageBox::size((160.0, 64.0), 25, 800.0, (320, 128));
    assert_eq!(size, (320.0, 128.0));
    // A box below the minimum is ignored, so a hand-edited settings file cannot make the
    // picture impossible to grab.
    let tiny = ImageBox::size((160.0, 64.0), 25, 800.0, (3, 3));
    assert_eq!(tiny, (500.0, 200.0));
}

#[test]
fn dragging_the_picture_keeps_it_inside_the_game_view() {
    let view = (1280.0, 800.0);
    let start = ImageBox { x: 400, y: 300, w: 320, h: 128 };
    let moved = start.moved(100.0, 60.0, view);
    assert_eq!((moved.x, moved.y), (500, 360));
    let pinned = start.moved(-9000.0, 9000.0, view);
    assert_eq!(pinned.x, 0);
    assert_eq!(pinned.y, view.1 as i16 - pinned.h);
}

#[test]
fn a_corner_handle_resizes_from_the_opposite_corner_and_never_below_24() {
    let view = (1280.0, 800.0);
    let start = ImageBox { x: 400, y: 300, w: 320, h: 128 };
    // Bottom-right grows: the top-left stays put.
    let grown = start.resized(3, 80.0, 40.0, view, false);
    assert_eq!((grown.x, grown.y, grown.w, grown.h), (400, 300, 400, 168));
    // Top-left shrinks from its own corner: the bottom-right stays put.
    let shrunk = start.resized(0, 60.0, 20.0, view, false);
    assert_eq!((shrunk.x, shrunk.y), (460, 320));
    assert_eq!((shrunk.w, shrunk.h), (260, 108));
    // Dragged far past the opposite corner, the box stops at the minimum size instead of
    // flipping inside out.
    let squashed = start.resized(0, 5000.0, 5000.0, view, false);
    assert_eq!((squashed.w, squashed.h), (ImageBox::MIN, ImageBox::MIN));
    assert_eq!((squashed.x, squashed.y), (400 + 320 - ImageBox::MIN, 300 + 128 - ImageBox::MIN));
}

#[test]
fn shift_locks_the_aspect_ratio_while_resizing() {
    let view = (1280.0, 800.0);
    let start = ImageBox { x: 400, y: 300, w: 320, h: 128 };
    let locked = start.resized(3, 80.0, 0.0, view, true);
    // 2.5:1 stays 2.5:1: width grows by 80, height follows to 160.
    assert_eq!((locked.w, locked.h), (400, 160));
    // An unconstrained drag of the same corner would have stretched it instead.
    let free = start.resized(3, 80.0, 0.0, view, false);
    assert_eq!((free.w, free.h), (400, 128));
}
