//! The party HUD's tiles and the tray's Party switch.
//!
//! Real `src/party_hud.rs`, real `src/party.rs` types, real `src/overlay_spec.rs`: what is
//! checked here is exactly what the launcher writes into `overlay-panels-<id>.json`.

use std::time::{Duration, Instant};

use creblox_vm_tests::overlay_spec::{self, HudToggles, TrayToggle};
use creblox_vm_tests::party::{
    PartyBoard, PartyMember, PartySnapshot, PartyStatus, PartyUser,
};
use creblox_vm_tests::party_hud::{
    initials, status_for, tiles, Tile, TileStatus, Tracker, GRACE, GREEN, GREY, RED,
};

fn member(
    id: u64,
    name: &str,
    status: PartyStatus,
    place_id: u64,
    job_id: &str,
) -> PartyMember {
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
    let tiles = tiles(&snap, 77, job, Some(1));
    assert_eq!(tiles.len(), 3);
    assert_eq!(tiles[0].status, TileStatus::InServer);
    assert_eq!(tiles[1].status, TileStatus::Loading);
    assert_eq!(tiles[2].status, TileStatus::Loading);
    assert_eq!(tiles[1].name, "Theo");
    assert_eq!(tiles[1].initials, "T");
    assert!(tiles[1].headshot.ends_with("2.png"));
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
    let first = tracker.update(Some(&both), 77, job, Some(1), now);
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|t| t.status == TileStatus::InServer));

    // Theo's socket dropped: the host's member list no longer has him.
    let alone = snapshot(
        "CRB-1",
        vec![member(1, "Maya", PartyStatus::InGame, 77, job)],
    );
    let after = tracker.update(Some(&alone), 77, job, Some(1), now + Duration::from_secs(2));
    assert_eq!(after.len(), 2, "the dropped member is still shown");
    assert_eq!(after[0].status, TileStatus::InServer);
    assert_eq!(after[1].id, 2);
    assert_eq!(after[1].status, TileStatus::Disconnected);
    assert_eq!(after[1].name, "Theo");

    // Past the grace window he is gone for good, so the tile list (and the headshot cache it
    // drives) only holds people who are actually in the party.
    let later = tracker.update(
        Some(&alone),
        77,
        job,
        Some(1),
        now + GRACE + Duration::from_secs(1),
    );
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
    assert_eq!(tracker.update(Some(&snap), 77, job, Some(1), now).len(), 1);
    assert!(tracker.update(None, 77, job, Some(1), now).is_empty());
}

#[test]
fn a_new_party_does_not_inherit_the_old_partys_tiles() {
    let mut tracker = Tracker::new();
    let now = Instant::now();
    let first = snapshot(
        "CRB-1",
        vec![member(1, "Maya", PartyStatus::InGame, 77, "job-a")],
    );
    tracker.update(Some(&first), 77, "job-a", Some(1), now);
    let second = snapshot(
        "CRB-2",
        vec![member(4, "Ada", PartyStatus::Ready, 0, "")],
    );
    let tiles = tracker.update(Some(&second), 77, "job-a", Some(1), now);
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
    let tiles = tiles(&snap, 77, job, Some(1));
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
    assert!(off.flipped(TrayToggle::Party).party, "a second click restores it");
    assert!(
        all_on.flipped(TrayToggle::Stats).party,
        "another switch must not touch the party"
    );

    // The switch removes the panel from the spec, exactly as `apply_tray_toggle` rebuilds it:
    // the card is only written when the party switch is on and the party has members.
    let tiles = vec![Tile {
        id: 1,
        name: "Maya".to_string(),
        initials: "M".to_string(),
        status: TileStatus::InServer,
        headshot: String::new(),
    }];
    let with = overlay_spec::spec_json(
        0.9,
        true,
        false,
        vec![overlay_spec::party_panel(3, &tiles)],
    );
    let without = overlay_spec::spec_json(0.9, true, false, Vec::new());
    assert!(with.to_string().contains("party"));
    assert!(!without.to_string().contains("party"));
}
