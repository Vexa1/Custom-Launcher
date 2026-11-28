//! Headless screenshots and layout inspection for the UI pages.
//!
//! The images land in `vm-tests/artifacts/` (gitignored) so a change to a page can be looked at
//! instead of guessed at. The assertions here are the part that fails a build: off-screen or
//! collapsed widgets, illegal overlaps, clipped text, undersized hit targets and low contrast.

use creblox_vm_tests::{screens, shots};

const SIZES: [(f32, f32); 2] = [(1280.0, 800.0), (1440.0, 900.0)];

#[test]
fn every_screen_renders_and_lays_out_cleanly() {
    let dir = shots::artifact_dir();
    let mut harness = shots::Harness::new();
    let mut failures: Vec<String> = Vec::new();
    let mut written: Vec<std::path::PathBuf> = Vec::new();

    // Both themes are rendered and checked: the light palette is a different set of colours on
    // the same layout, and a label that reads on the dark card can vanish on the white one.
    for (name, page) in screens::all() {
        for light in [false, true] {
            screens::set_light(light);
            for (w, h) in SIZES {
            let theme = if light { "light-" } else { "" };
            let tag = format!("{name}-{theme}{}x{}", w as u32, h as u32);
            let (shot, probe) = harness.render(&tag, egui::vec2(w, h), 1.5, page);

            let path = shot.save(&dir);
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if bytes < 4_096 {
                failures.push(format!("{tag}: PNG is only {bytes} bytes"));
            }
            if shot.width != (w * 1.5) as usize || shot.height != (h * 1.5) as usize {
                failures.push(format!(
                    "{tag}: rendered {}x{}, expected {}x{}",
                    shot.width,
                    shot.height,
                    (w * 1.5) as usize,
                    (h * 1.5) as usize
                ));
            }
            // A blank frame would pass every geometric rule above, so the picture itself is
            // checked too: text, borders and accents must put real colours on the canvas.
            let colors = shot.distinct_colors();
            if colors < 60 {
                failures.push(format!(
                    "{tag}: only {colors} distinct colours - the frame looks blank"
                ));
            }
            if probe.widgets.len() < 6 {
                failures.push(format!(
                    "{tag}: {} registered widgets, expected a full page",
                    probe.widgets.len()
                ));
            }

            for problem in probe.problems(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, h),
            )) {
                failures.push(format!("{tag}: {problem}"));
            }
            written.push(path);
            }
        }
    }
    screens::set_light(false);

    println!("screenshots written:");
    for path in &written {
        println!("  {}", path.display());
    }

    assert!(
        failures.is_empty(),
        "{} layout problem(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The party strip: one member in this exact server (green), one still loading (grey) and one
/// whose connection dropped (red), rendered from the same panels JSON the launcher writes.
#[test]
fn party_hud_shows_three_states() {
    use creblox_vm_tests::party_hud::{Tile, TileStatus};

    let dir = shots::artifact_dir();
    let mut harness = shots::Harness::new();
    let tiles = vec![
        Tile {
            id: 1,
            name: "Maya".to_string(),
            initials: "M".to_string(),
            status: TileStatus::InServer,
            headshot: String::new(),
        },
        Tile {
            id: 2,
            name: "Theo".to_string(),
            initials: "T".to_string(),
            status: TileStatus::Loading,
            headshot: String::new(),
        },
        Tile {
            id: 3,
            name: "Ada".to_string(),
            initials: "A".to_string(),
            status: TileStatus::Disconnected,
            headshot: String::new(),
        },
    ];
    let spec = creblox_vm_tests::overlay_spec::spec_json(
        1.0,
        true,
        false,
        vec![creblox_vm_tests::overlay_spec::party_panel(3, &tiles)],
    );
    let (shot, probe) = harness.render(
        "overlay_party_hud",
        egui::vec2(520.0, 300.0),
        2.0,
        move |ui, probe| creblox_vm_tests::hud::draw(ui, probe, &spec, 0),
    );
    let path = shot.save(&dir);
    println!("party hud: {}", path.display());
    assert_eq!((shot.width, shot.height), (1040, 600));
    assert!(
        shot.distinct_colors() >= 60,
        "only {} distinct colours - the card looks blank",
        shot.distinct_colors()
    );
    let problems = probe.problems(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(520.0, 300.0),
    ));
    assert!(problems.is_empty(), "{} party HUD problem(s):\n{}", problems.len(), problems.join("\n"));
}

/// The Charts filter bar on its own, twice the size, at the exact path the release notes point
/// at: `artifacts/charts_filters.png`. Every pill's text has to sit in the middle of its box on
/// both axes - the bug this guards is "55", "Min" and "0" hugging the top-left corner.
#[test]
fn charts_filter_bar_pills_are_centred() {
    let dir = shots::artifact_dir();
    let mut harness = shots::Harness::new();
    let (shot, probe) = harness.render(
        "charts_filters",
        egui::vec2(1030.0, 150.0),
        2.0,
        screens::charts_filters,
    );
    let path = shot.save(&dir);
    println!("charts filter bar: {}", path.display());

    assert_eq!((shot.width, shot.height), (2060, 300));
    let mut failures: Vec<String> = Vec::new();
    for id in [
        "charts.f.players.min",
        "charts.f.players.max",
        "charts.f.like",
        "charts.f.visits",
    ] {
        let Some(pill) = probe.widgets.iter().find(|w| w.id == id) else {
            failures.push(format!("{id}: pill was never drawn"));
            continue;
        };
        let text_id = format!("{id}.text");
        let Some(text) = probe.widgets.iter().find(|w| w.id == text_id) else {
            failures.push(format!("{text_id}: no text in the pill"));
            continue;
        };
        let dx = pill.rect.center().x - text.rect.center().x;
        let dy = pill.rect.center().y - text.rect.center().y;
        if dx.abs() > 0.5 || dy.abs() > 0.5 {
            failures.push(format!(
                "{id}: text is {dx:.2}px across and {dy:.2}px down from the centre"
            ));
        }
        if !pill.rect.contains_rect(text.rect) {
            failures.push(format!("{id}: text spills out of its pill"));
        }
    }
    failures.extend(probe.problems(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1030.0, 150.0),
    )));
    assert!(
        failures.is_empty(),
        "{} filter-bar problem(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
