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

    for (name, page) in screens::all() {
        for (w, h) in SIZES {
            let tag = format!("{name}-{}x{}", w as u32, h as u32);
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
