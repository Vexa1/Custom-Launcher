//! The HUD's position editor: every panel moves on its own, and the bottom bar's readout,
//! hint and buttons never collide.
//!
//! `src/overlay_window.rs` needs a real window, so the fixture here is `vm-tests/src/hud.rs`,
//! which mirrors the shipping layout from the same tokens and the same JSON. The assertions
//! are the part that fails a build; the PNG is what gets looked at.

use creblox_vm_tests::overlay_spec::{
    FlagsPanel, GamePanel, Live, LiveServer, PanelOffset, Panels, ServerPanel, StatsPanel,
    apply_panel_offsets, build, parse_panel_offsets, spec_json, unix_ms_now,
};
use creblox_vm_tests::{hud, shots};

fn spec_with_panel_positions() -> serde_json::Value {
    let now_ms = unix_ms_now();
    let server = ServerPanel {
        corner: 3,
        duration_secs: 0,
        show_ip: true,
        show_job_id: true,
    };
    let stats = StatsPanel {
        corner: 1,
        cpu: true,
        ram: true,
        ping: true,
        session: true,
        clock: false,
        fps_cap: Some(60),
    };
    let flags = FlagsPanel {
        corner: 0,
        max_lines: 4,
        preset: "Competitive".to_string(),
        flags: vec![
            "DFIntTaskSchedulerTargetFps = 240".to_string(),
            "FFlagDebugGraphicsPreferD3D11 = True".to_string(),
            "FIntRenderShadowIntensity = 0".to_string(),
            "FFlagDisablePostFx = True".to_string(),
            "+2 more".to_string(),
        ],
    };
    let game = GamePanel { corner: 2 };
    let live = Live {
        server: Some(LiveServer {
            ip: "203.0.113.24",
            job_id: "a1b2c3d4-5e6f-7a8b-9c0d-1e2f3a4b5c6d",
            place_id: 920587237,
        }),
        server_age_secs: 12,
        joined_at_unix_ms: now_ms.saturating_sub(65_000),
        detect_secs: 65,
        region: Some("Sydney, AU"),
        ping_ms: Some(38),
        ping_path: None,
        game: Some(("Adopt Me!", Some(312_884))),
        cpu_pct: Some(12.0),
        ram_mb: Some(640.0),
        clock: "09:00".to_string(),
        edit_preview: false,
    };
    let mut panels = build(
        &Panels {
            server: Some(&server),
            stats: Some(&stats),
            flags: Some(&flags),
            game: Some(&game),
        },
        &live,
    );
    // Deliberately different per-panel positions: this is what "independent boxes" means.
    apply_panel_offsets(
        &mut panels,
        &[
            PanelOffset {
                id: "server".to_string(),
                x: 0,
                y: 0,
            },
            PanelOffset {
                id: "stats".to_string(),
                x: 0,
                y: 0,
            },
            PanelOffset {
                id: "flags".to_string(),
                x: 40,
                y: 80,
            },
            PanelOffset {
                id: "game".to_string(),
                x: 120,
                y: -40,
            },
        ],
    );
    spec_json(0.9, true, true, panels)
}

#[test]
fn every_panel_keeps_its_own_position_in_the_panels_json() {
    let spec = spec_with_panel_positions();
    let panels = spec["panels"].as_array().expect("panels");
    let find = |id: &str| {
        panels
            .iter()
            .find(|p| p["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("panel {id} is in the spec"))
    };
    assert_eq!(find("server")["offset_x"].as_i64(), Some(0));
    assert_eq!(find("stats")["offset_y"].as_i64(), Some(0));
    assert_eq!(find("flags")["offset_x"].as_i64(), Some(40));
    assert_eq!(find("flags")["offset_y"].as_i64(), Some(80));
    assert_eq!(find("game")["offset_x"].as_i64(), Some(120));
    assert_eq!(find("game")["offset_y"].as_i64(), Some(-40));
    assert!(!spec["panels"][0].get("saved").is_some());
}

#[test]
fn a_panel_without_a_saved_position_sits_in_its_corner_slot() {
    let mut panels = spec_with_panel_positions()["panels"]
        .as_array()
        .cloned()
        .expect("panels");
    // Only one card was ever moved; the rest must be written as the corner default.
    apply_panel_offsets(
        &mut panels,
        &[PanelOffset {
            id: "stats".to_string(),
            x: -30,
            y: 12,
        }],
    );
    let stats = panels
        .iter()
        .find(|p| p["id"].as_str() == Some("stats"))
        .expect("stats");
    assert_eq!(stats["offset_x"].as_i64(), Some(-30));
    assert_eq!(stats["offset_y"].as_i64(), Some(12));
    for panel in &panels {
        if panel["id"].as_str() == Some("stats") {
            continue;
        }
        assert_eq!(panel["offset_x"].as_i64(), Some(0), "{panel}");
        assert_eq!(panel["offset_y"].as_i64(), Some(0), "{panel}");
    }
}

#[test]
fn an_old_single_offset_is_folded_into_the_server_card() {
    // settings.json from before per-panel positions: one offset for the whole stack.
    let old = serde_json::json!({});
    let migrated = parse_panel_offsets(&old["overlay_panels"], -120, 40);
    assert_eq!(
        migrated,
        vec![PanelOffset {
            id: "server".to_string(),
            x: -120,
            y: 40,
        }]
    );
    // A file that already has the map wins, even when the legacy keys are absent.
    let saved = serde_json::json!([{"id": "stats", "x": 7, "y": 9}]);
    assert_eq!(
        parse_panel_offsets(&saved, 0, 0),
        vec![PanelOffset {
            id: "stats".to_string(),
            x: 7,
            y: 9,
        }]
    );
    // Nothing saved, nothing migrated: every card stays in its corner.
    assert!(parse_panel_offsets(&serde_json::Value::Null, 0, 0).is_empty());
}

#[test]
fn the_editor_renders_independent_boxes_and_a_clear_toolbar() {
    let spec = spec_with_panel_positions();
    let now_ms = unix_ms_now();
    let viewport = egui::vec2(1280.0, 800.0);
    let mut harness = shots::Harness::new();
    let (shot, probe) = harness.render("overlay_edit_mode", viewport, 1.5, |ui, probe| {
        hud::draw_edit(ui, probe, &spec, now_ms, "stats");
    });

    // One hitbox per panel, one dashed outline each: four cards, four boxes.
    let panels: Vec<_> = probe
        .widgets
        .iter()
        .filter(|w| w.id.starts_with("hud.panel.") && w.id.matches('.').count() == 2)
        .collect();
    assert_eq!(panels.len(), 4, "every panel has its own box");
    for (i, a) in panels.iter().enumerate() {
        for b in panels.iter().skip(i + 1) {
            let overlap = a.rect.intersect(b.rect);
            assert!(
                overlap.width() <= 1.0 || overlap.height() <= 1.0,
                "{} and {} overlap by {:.1}x{:.1}",
                a.id,
                b.id,
                overlap.width(),
                overlap.height()
            );
        }
        // Every line of a card stays inside that card's own surface.
        let prefix = format!("{}.", a.id);
        for w in probe
            .widgets
            .iter()
            .filter(|w| w.id.starts_with(&prefix))
        {
            assert!(
                a.rect.expand(0.5).contains_rect(w.rect),
                "{} spills out of {}: {:?} vs {:?}",
                w.id,
                a.id,
                w.rect,
                a.rect
            );
        }
    }

    // The toolbar holds all four parts, and none of them touches another.
    let ids = [
        "hud.edit.bar",
        "hud.edit.readout",
        "hud.edit.hint",
        "hud.edit.reset",
        "hud.edit.confirm",
    ];
    let rect = |id: &str| {
        probe
            .widgets
            .iter()
            .find(|w| w.id == id)
            .unwrap_or_else(|| panic!("{id} is on the bar"))
            .rect
    };
    for id in ids {
        rect(id);
    }
    let readout = rect("hud.edit.readout");
    let hint = rect("hud.edit.hint");
    let reset = rect("hud.edit.reset");
    let confirm = rect("hud.edit.confirm");
    assert!(
        hint.left() >= readout.right() - 0.5,
        "the hint starts where the readout ends ({:?} vs {:?})",
        hint,
        readout
    );
    assert!(
        reset.left() >= hint.right() - 0.5,
        "Reset starts after the hint ({:?} vs {:?})",
        reset,
        hint
    );
    assert!(
        confirm.left() >= reset.right() - 0.5,
        "Confirm starts after Reset ({:?} vs {:?})",
        confirm,
        reset
    );
    assert!(confirm.width() >= 24.0 && reset.width() >= 24.0);
    assert!(
        readout.width() > 0.0 && hint.width() > 0.0,
        "the bar's text is measured, not guessed"
    );

    let problems = probe.problems(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport));
    assert!(
        problems.is_empty(),
        "the editor has layout problems:\n{}",
        problems.join("\n")
    );
    let path = shot.save(&shots::artifact_dir());
    assert_eq!(path.file_name().unwrap(), "overlay_edit_mode.png");
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    assert!(bytes > 4_096, "the PNG is only {bytes} bytes: {path:?}");
    assert!(
        shot.distinct_colors() >= 60,
        "the frame looks blank ({} colours)",
        shot.distinct_colors()
    );
    println!("overlay_edit_mode.png: {} ({bytes} bytes)", path.display());
}

/// The custom image is an element of the editor in its own right: it is drawn at the saved box
/// from `settings.json` / the panels file, and it has a dashed frame plus four corner handles.
#[test]
fn the_custom_image_moves_and_resizes_from_its_saved_box() {
    let viewport = egui::vec2(1280.0, 800.0);
    let mut spec = spec_with_panel_positions();
    // A picture the editor already dragged to a non-default spot and resized: these are the
    // points the live HUD draws it at, not a corner slot.
    // The offsets deliberately disagree with the box: with a saved box they are the corner
    // fine-tune sliders and must not move the picture.
    spec["image"] = serde_json::json!({
        "path": "/tmp/creblox-logo.png",
        "corner": 0,
        "size": 25,
        "opacity": 0.95,
        "offset_x": 30,
        "offset_y": -20,
        "x": 412,
        "y": 236,
        "w": 320,
        "h": 128
    });

    let dir = shots::artifact_dir();
    let mut harness = shots::Harness::new();
    let (shot, probe) = harness.render(
        "overlay_edit_image_resize",
        viewport,
        1.5,
        move |ui, probe| hud::draw_edit(ui, probe, &spec, 0, "image"),
    );

    let find = |id: &str| {
        probe
            .widgets
            .iter()
            .find(|w| w.id == id)
            .unwrap_or_else(|| panic!("{id} was drawn"))
            .rect
    };

    // Exactly the saved box, not a corner or a percentage.
    let image = find("hud.image");
    assert!(
        (image.left() - 412.0).abs() <= 0.5
            && (image.top() - 236.0).abs() <= 0.5
            && (image.width() - 320.0).abs() <= 0.5
            && (image.height() - 128.0).abs() <= 0.5,
        "the picture is not at its saved box: {image:?}"
    );
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, viewport).contains_rect(image),
        "the picture hangs outside the game view: {image:?}"
    );

    // Four 24 px targets, each centred on a corner, each holding an 8 px square.
    let corners = [
        image.left_top(),
        image.right_top(),
        image.left_bottom(),
        image.right_bottom(),
    ];
    for (i, corner) in corners.iter().enumerate() {
        let target = find(&format!("hud.image.handle.{i}"));
        assert!(
            (target.center() - *corner).length() <= 0.5,
            "handle {i} is not on its corner: {target:?} vs {corner:?}"
        );
        assert!(
            target.width() >= 24.0 && target.height() >= 24.0,
            "handle {i} is too small to grab: {target:?}"
        );
        let grip = find(&format!("hud.image.grip.{i}"));
        assert!(
            (grip.width() - 8.0).abs() <= 0.5 && (grip.center() - *corner).length() <= 0.5,
            "the drawn square for handle {i} is {grip:?}"
        );
    }

    // The toolbar anchors to the picture and never lands on it.
    let bar = find("hud.edit.bar");
    let overlap = bar.intersect(image);
    assert!(
        overlap.width() <= 0.5 || overlap.height() <= 0.5,
        "the toolbar covers the picture by {:.1}x{:.1}",
        overlap.width(),
        overlap.height()
    );
    for (i, _) in corners.iter().enumerate() {
        let handle = find(&format!("hud.image.handle.{i}"));
        assert!(
            !bar.intersects(handle),
            "the toolbar sits on handle {i}: {bar:?} vs {handle:?}"
        );
    }
    let readout = find("hud.edit.readout");
    assert!(
        bar.contains_rect(readout),
        "the readout is inside the bar: {readout:?} vs {bar:?}"
    );
    // The image readout carries its size as well as its position, so it is wider than the
    // panel readout's "id  x, y".
    assert!(
        readout.width() > 120.0,
        "the readout looks too narrow to include W x H: {readout:?}"
    );

    let problems = probe.problems(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport));
    assert!(
        problems.is_empty(),
        "the editor with an image has layout problems:\n{}",
        problems.join("\n")
    );

    let path = shot.save(&dir);
    assert_eq!(path.file_name().unwrap(), "overlay_edit_image_resize.png");
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    assert!(bytes > 4_096, "the PNG is only {bytes} bytes: {path:?}");
    assert!(
        shot.distinct_colors() >= 60,
        "the frame looks blank ({} colours)",
        shot.distinct_colors()
    );
    println!("overlay_edit_image_resize.png: {} ({bytes} bytes)", path.display());
}

/// A picture that has never been dragged still follows its corner, height and offsets, so the
/// settings sliders keep working until the editor freezes a box.
#[test]
fn an_untouched_image_keeps_following_its_corner_slot() {
    let viewport = egui::vec2(1280.0, 800.0);
    let mut spec = spec_with_panel_positions();
    spec["image"] = serde_json::json!({
        "path": "/tmp/creblox-logo.png",
        "corner": 1,
        "size": 25,
        "opacity": 0.95,
        "offset_x": 0,
        "offset_y": 0,
        "w": 0,
        "h": 0
    });
    let mut harness = shots::Harness::new();
    let (_, probe) = harness.render(
        "overlay_edit_image_default",
        viewport,
        1.5,
        move |ui, probe| hud::draw(ui, probe, &spec, 0),
    );
    let image = probe
        .widgets
        .iter()
        .find(|w| w.id == "hud.image")
        .expect("the picture is drawn")
        .rect;
    // Top-right, 25% of the 800 pt view height -> 200 pt tall, 160x64 picture -> 500 wide.
    assert!(
        (image.width() - 500.0).abs() <= 1.0 && (image.height() - 200.0).abs() <= 1.0,
        "the default size is not 25% of the height at the picture's aspect: {image:?}"
    );
    assert!(
        (image.right() - (viewport.x - 16.0)).abs() <= 1.0 && (image.top() - 16.0).abs() <= 1.0,
        "the default picture is not in its top-right slot: {image:?}"
    );
}
