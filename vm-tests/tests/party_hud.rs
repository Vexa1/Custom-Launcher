//! The party HUD's tiles, the tray's Party switch, and the panels file a click leaves behind.
//!
//! Real `src/party_hud.rs`, real `src/party.rs` types, real `src/overlay_spec.rs`: what is
//! checked here is exactly what the launcher writes into `overlay-panels-<id>.json`.

use std::time::{Duration, Instant};

use serde_json::json;

use creblox_vm_tests::overlay_spec::{self, ClickDebounce, HudToggles, TrayToggle};
use creblox_vm_tests::party::{PartyBoard, PartyMember, PartySnapshot, PartyStatus, PartyUser};
use creblox_vm_tests::party_hud::{
    initials, status_for, tiles, Tile, TileStatus, Tracker, GRACE, GREEN, GREY, RED,
};

fn member(id: u64, name: &str, status: PartyStatus, place_id: u64, job_id: &str) -> PartyMember {
    PartyMember {
        user: PartyUser {
            id,
            name: name.to_string(),
            display_name: name.to_string(),
            headshot_url: format!("https://example.invalid/{id}.png"),
        },
        status,
        place_id,
        job_id: job_id.to_string(),
    }
}

fn snapshot(code: &str, members: Vec<PartyMember>) -> PartySnapshot {
    PartySnapshot {
        code: code.to_string(),
        invite: String::new(),
        host: members.first().map(|m| m.user.id).unwrap_or(0),
        max: 8,
        members,
        connected: true,
        board: PartyBoard::default(),
    }
}

fn tile(id: u64, name: &str, status: TileStatus) -> Tile {
    Tile {
        id,
        name: name.to_string(),
        initials: initials(name),
        status,
        headshot: String::new(),
    }
}

#[test]
fn the_outline_colour_follows_the_member_state() {
    let my_place = 920587237;
    let my_job = "6f9a2c1e-0b7d-4e11-9f7a-1d2c3b4a5e6f";

    // Exactly this server: same place *and* same job.
    assert_eq!(
        status_for(PartyStatus::InGame, my_place, my_job, my_place, my_job),
        TileStatus::InServer
    );
    // Same game, a different server: still loading in, never green.
    assert_eq!(
        status_for(PartyStatus::InGame, my_place, "other-job", my_place, my_job),
        TileStatus::Loading
    );
    assert_eq!(
        status_for(PartyStatus::InGame, 111, my_job, my_place, my_job),
        TileStatus::Loading
    );
    // Launching, and waiting in the lobby before a launch, are both "loading".
    assert_eq!(
        status_for(PartyStatus::Launching, my_place, my_job, my_place, my_job),
        TileStatus::Loading
    );
    assert_eq!(
        status_for(PartyStatus::Ready, 0, "", my_place, my_job),
        TileStatus::Loading
    );
    // With no server of our own there is nothing to match against.
    assert_eq!(
        status_for(PartyStatus::InGame, 0, "", 0, ""),
        TileStatus::Loading
    );

    assert_eq!(TileStatus::InServer.rgb(), GREEN);
    assert_eq!(TileStatus::Loading.rgb(), GREY);
    assert_eq!(TileStatus::Disconnected.rgb(), RED);
    assert_eq!(TileStatus::InServer.key(), "server");
    assert_eq!(TileStatus::Loading.key(), "loading");
    assert_eq!(TileStatus::Disconnected.key(), "offline");
}

#[test]
fn initials_come_from_the_name_and_never_go_blank() {
    assert_eq!(initials("Maya"), "M");
    assert_eq!(initials("sarah the great"), "ST");
    assert_eq!(initials("  "), "?");
    assert_eq!(initials("42"), "4");
}

#[test]
fn a_snapshot_becomes_one_tile_per_member() {
    let job = "job-a";
    let snap = snapshot(
        "CRB-1",
        vec![
            member(1, "Maya", PartyStatus::InGame, 77, job),
            member(2, "Theo", PartyStatus::Launching, 77, job),
            member(3, "Ada", PartyStatus::Ready, 0, ""),
        ],
    );
    let tiles = tiles(&snap, 77, job);
    assert_eq!(tiles.len(), 3);
    assert_eq!(tiles[0].status, TileStatus::InServer);
    assert_eq!(tiles[1].status, TileStatus::Loading);
    assert_eq!(tiles[2].status, TileStatus::Loading);
    assert_eq!(tiles[1].name, "Theo");
    assert_eq!(tiles[1].initials, "T");
    assert!(tiles[1].headshot.ends_with("2.png"));
}

/// Your own member is part of the strip. That is the decision: the card shows the whole party,
/// and your own square goes green exactly like everyone else's when you are in that server.
#[test]
fn your_own_member_is_in_the_strip() {
    let job = "job-a";
    let snap = snapshot(
        "CRB-1",
        vec![
            member(1, "Maya", PartyStatus::InGame, 77, job),
            member(2, "Theo", PartyStatus::Launching, 77, job),
        ],
    );
    let all = tiles(&snap, 77, job);
    assert_eq!(all.len(), 2, "your own tile is drawn too");
    assert_eq!(all[0].id, 1);
    assert_eq!(
        all[0].status,
        TileStatus::InServer,
        "the player running the HUD is in that server"
    );
    assert_eq!(all[1].status, TileStatus::Loading);

    // Not in a server yet: nothing on the strip can be green.
    let desktop = tiles(&snap, 0, "");
    assert_eq!(desktop.len(), 2);
    assert!(desktop.iter().all(|t| t.status == TileStatus::Loading));
}

#[test]
fn a_member_who_vanishes_while_in_a_match_turns_red_for_the_grace_window() {
    let mut tracker = Tracker::new();
    let job = "job-a";
    let both = snapshot(
        "CRB-1",
        vec![
            member(1, "Maya", PartyStatus::InGame, 77, job),
            member(2, "Theo", PartyStatus::InGame, 77, job),
        ],
    );
    let now = Instant::now();
    let first = tracker.update(Some(&both), 77, job, now);
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|t| t.status == TileStatus::InServer));

    // Theo's socket dropped: the host's member list no longer has him.
    let alone = snapshot(
        "CRB-1",
        vec![member(1, "Maya", PartyStatus::InGame, 77, job)],
    );
    let after = tracker.update(Some(&alone), 77, job, now + Duration::from_secs(2));
    assert_eq!(after.len(), 2, "the dropped member is still shown");
    assert_eq!(after[0].status, TileStatus::InServer);
    assert_eq!(after[1].id, 2);
    assert_eq!(after[1].status, TileStatus::Disconnected);
    assert_eq!(after[1].name, "Theo");

    // Past the grace window he is gone for good, so the tile list (and the headshot cache it
    // drives) only holds people who are actually in the party.
    let later = tracker.update(Some(&alone), 77, job, now + GRACE + Duration::from_secs(1));
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].id, 1);
}

#[test]
fn leaving_the_party_drops_every_tile_at_once() {
    let mut tracker = Tracker::new();
    let job = "job-a";
    let snap = snapshot(
        "CRB-1",
        vec![member(1, "Maya", PartyStatus::InGame, 77, job)],
    );
    let now = Instant::now();
    assert_eq!(tracker.update(Some(&snap), 77, job, now).len(), 1);
    assert!(tracker.update(None, 77, job, now).is_empty());
}

#[test]
fn a_new_party_does_not_inherit_the_old_partys_tiles() {
    let mut tracker = Tracker::new();
    let now = Instant::now();
    let first = snapshot(
        "CRB-1",
        vec![member(1, "Maya", PartyStatus::InGame, 77, "job-a")],
    );
    tracker.update(Some(&first), 77, "job-a", now);
    let second = snapshot("CRB-2", vec![member(4, "Ada", PartyStatus::Ready, 0, "")]);
    let tiles = tracker.update(Some(&second), 77, "job-a", now);
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].id, 4);
}

#[test]
fn the_panel_carries_the_status_the_hud_draws() {
    let job = "job-a";
    let snap = snapshot(
        "CRB-1",
        vec![
            member(1, "Maya", PartyStatus::InGame, 77, job),
            member(2, "Theo", PartyStatus::Launching, 77, job),
            member(3, "Ada", PartyStatus::InGame, 77, "other"),
        ],
    );
    let tiles = tiles(&snap, 77, job);
    let panel = overlay_spec::party_panel(3, &tiles);
    assert_eq!(panel["id"], "party");
    assert_eq!(panel["corner"], 3);
    assert_eq!(panel["title"], "Party 3");
    let arr = panel["tiles"].as_array().expect("tiles array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["status"], "server");
    assert_eq!(arr[1]["status"], "loading");
    assert_eq!(arr[2]["status"], "loading");
    assert_eq!(arr[0]["initials"], "M");
    // The HUD parses exactly these keys back.
    assert!(panel.to_string().contains("\"headshot\":\"https://example.invalid/1.png\""));

    let spec = overlay_spec::spec_json(0.9, true, false, vec![panel]);
    assert!(spec.to_string().contains("\"id\":\"party\""));
}

#[test]
fn the_tray_party_switch_flips_only_itself() {
    let all_on = HudToggles {
        server: true,
        stats: true,
        crosshair: true,
        party: true,
    };
    let off = all_on.flipped(TrayToggle::Party);
    assert!(!off.party);
    assert!(off.server && off.stats && off.crosshair);
    assert!(
        off.flipped(TrayToggle::Party).party,
        "a second click restores it"
    );
    assert!(
        all_on.flipped(TrayToggle::Stats).party,
        "another switch must not touch the party"
    );
}

/// A Party Overlay tray click rewrites the active `overlay-panels-<id>.json`, and the second
/// delivery of that same physical click leaves the file alone.
///
/// macOS hands one status-menu click to every reader of the global menu channel, so a duplicate
/// that flipped the switch again (or wrote the file again) would undo the click and replay it.
/// This drives the shipping pieces that decide that: `ClickDebounce`, `HudToggles::flipped`,
/// `party_panel` and `write_atomic`.
#[test]
fn a_party_tray_click_rewrites_the_panels_file_without_a_duplicate_replay() {
    let dir = std::env::temp_dir().join(format!(
        "creblox-party-toggle-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp folder");
    // The overlay's own file name for this run.
    let path = dir.join("overlay-panels-777.json");

    let tiles = vec![tile(1, "Maya", TileStatus::InServer)];
    let rebuild = |party: bool| {
        let panels = if party {
            vec![overlay_spec::party_panel(3, &tiles)]
        } else {
            Vec::new()
        };
        overlay_spec::spec_json(0.92, true, false, panels)
    };

    // The game is running with the card on screen.
    overlay_spec::write_atomic(&path, &rebuild(true).to_string());
    assert!(std::fs::read_to_string(&path).unwrap().contains("party"));

    let mut state = HudToggles {
        server: true,
        stats: true,
        crosshair: true,
        party: true,
    };
    let mut seen = ClickDebounce::new();
    let t0 = Instant::now();
    let mut applies = 0;

    // Click one: the switch flips and the spec on disk loses the card.
    if seen.accept(TrayToggle::Party, t0) {
        state = state.flipped(TrayToggle::Party);
        overlay_spec::write_atomic(&path, &rebuild(state.party).to_string());
        applies += 1;
    }
    let after_first = std::fs::read_to_string(&path).unwrap();
    assert!(!state.party);
    assert!(
        !after_first.contains("\"id\":\"party\""),
        "the click must remove the card: {after_first}"
    );
    assert!(
        after_first.contains("\"game_active\":true"),
        "the rest of the spec must survive the click: {after_first}"
    );

    // The duplicate delivery, three milliseconds later: no flip, no write.
    if seen.accept(TrayToggle::Party, t0 + Duration::from_millis(3)) {
        state = state.flipped(TrayToggle::Party);
        overlay_spec::write_atomic(&path, &rebuild(state.party).to_string());
        applies += 1;
    }
    assert_eq!(applies, 1, "one physical click, one apply");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        after_first,
        "the duplicate delivery rewrote the file"
    );
    assert!(!state.party, "the duplicate clicked the card back on");

    // A real second click, past the debounce window, restores it.
    assert!(seen.accept(TrayToggle::Party, t0 + Duration::from_millis(200)));
    state = state.flipped(TrayToggle::Party);
    overlay_spec::write_atomic(&path, &rebuild(state.party).to_string());
    assert!(state.party);
    assert!(std::fs::read_to_string(&path).unwrap().contains("party"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// The panel id is the one the overlay window looks for; a rename here would silently drop the
/// card from every HUD.
#[test]
fn the_party_panel_id_is_the_one_the_hud_parses() {
    let panel = overlay_spec::party_panel(0, &[tile(1, "Maya", TileStatus::Loading)]);
    assert_eq!(panel, json!({
        "id": "party",
        "corner": 0,
        "title": "Party 1",
        "tiles": [{
            "id": 1,
            "name": "Maya",
            "initials": "M",
            "status": "loading",
            "headshot": ""
        }]
    }));
}
