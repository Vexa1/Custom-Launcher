//! The real `src/ui/controls.rs`, with motion off, painted through egui and poked with keys.
//!
//! These are the shipping widgets, not a fixture copy: `controls.rs` depends only on
//! `crate::theme`, so the Linux test crate includes it by path. That makes it possible to assert
//! the exact colours a disabled switch/segment paints and to prove Left/Right move the selection.

use creblox_vm_tests::controls;
use creblox_vm_tests::egui::{
    self, Color32, Event, Key, Modifiers, Pos2, RawInput, Rect, Shape, Vec2,
};
use creblox_vm_tests::theme;

const SIZE: (f32, f32) = (640.0, 400.0);
const OPTIONS: [&str; 4] = ["Top left", "Top right", "Bottom left", "Bottom right"];

fn input(events: Vec<Event>) -> RawInput {
    let mut input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(SIZE.0, SIZE.1),
        )),
        events,
        ..Default::default()
    };
    input.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        },
    );
    input
}

/// One frame with animations off, which is also what makes the knob/segment snap to its state
/// instead of easing towards it during the assertion.
fn frame<T>(
    ctx: &mut egui::Context,
    events: Vec<Event>,
    body: impl FnOnce(&mut egui::Ui) -> T,
) -> (T, Vec<egui::epaint::ClippedShape>) {
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::NULL.with("creblox_motion_on"), false);
    });
    // The app's `apply_theme` sets the accent on the selection stroke; do the same here so the
    // controls paint the shipped green rather than egui's default selection colour.
    ctx.all_styles_mut(|s| {
        s.visuals.selection.stroke.color = theme::GREEN;
    });
    let mut slot = None;
    let mut body = Some(body);
    let mut out = ctx.run_ui(input(events), |ui| {
        slot = Some(body.take().expect("frame body runs once")(ui));
    });
    out.textures_delta.clear();
    (slot.expect("frame body ran"), std::mem::take(&mut out.shapes))
}

fn key(k: Key) -> Event {
    Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::default(),
    }
}

/// Every filled rectangle and circle the frame painted, flattened out of nested shape lists.
/// egui fades a disabled `Ui` by multiplying the painter opacity, so the shape carries the
/// premultiplied colour (`#52..FF` at 0.5 becomes `#29..80`). Compare the actual painted value.
fn faded(color: Color32, opacity: f32) -> Color32 {
    Color32::from_rgba_premultiplied(
        (color.r() as f32 * opacity).round() as u8,
        (color.g() as f32 * opacity).round() as u8,
        (color.b() as f32 * opacity).round() as u8,
        (color.a() as f32 * opacity).round() as u8,
    )
}

fn painted(shapes: &[egui::epaint::ClippedShape]) -> (Vec<(Rect, Color32)>, Vec<(Pos2, f32, Color32)>) {
    let mut rects = Vec::new();
    let mut circles = Vec::new();
    fn walk(
        shape: &Shape,
        rects: &mut Vec<(Rect, Color32)>,
        circles: &mut Vec<(Pos2, f32, Color32)>,
    ) {
        match shape {
            Shape::Rect(r) => rects.push((r.rect, r.fill)),
            Shape::Circle(c) => circles.push((c.center, c.radius, c.fill)),
            Shape::Vec(v) => {
                for s in v {
                    walk(s, rects, circles);
                }
            }
            _ => {}
        }
    }
    for clipped in shapes {
        walk(&clipped.shape, &mut rects, &mut circles);
    }
    (rects, circles)
}

#[test]
fn a_disabled_switch_that_is_on_paints_a_dim_track_and_a_grey_knob() {
    let mut ctx = egui::Context::default();
    let mut on = true;
    let mut palette = None;
    let mut accent = None;
    let mut opacity = 1.0;
    let (_, shapes) = frame(&mut ctx, Vec::new(), |ui| {
        ui.add_enabled_ui(false, |ui| {
            palette = Some(theme::pal(ui));
            accent = Some(theme::accent_color(ui));
            opacity = ui.opacity();
            controls::toggle_switch(ui, &mut on, "Hover fades, rings and the sliding nav bar");
        });
    });
    let (p, accent) = (palette.unwrap(), accent.unwrap());
    let (rects, circles) = painted(&shapes);

    let track = rects
        .iter()
        .find(|(r, _)| (r.width() - 40.0).abs() < 0.5 && (r.height() - 22.0).abs() < 0.5)
        .map(|(_, fill)| *fill)
        .expect("the switch track was not painted");
    let knob = circles
        .iter()
        .find(|(_, radius, _)| (*radius - 8.0).abs() < 0.5)
        .map(|(_, _, fill)| *fill)
        .expect("the switch knob was not painted");

    assert_eq!(
        track,
        faded(p.widget_inactive, opacity),
        "a disabled on-switch must paint the neutral track"
    );
    assert_ne!(
        track,
        faded(accent, opacity),
        "a disabled on-switch must not paint the live accent"
    );
    assert_eq!(
        knob,
        faded(controls::switch_knob_color(&p, false, track), opacity)
    );
    assert_ne!(
        knob,
        faded(Color32::WHITE, opacity),
        "a disabled knob must not be white"
    );
    // And visibly different from the same switch while enabled, which the next test covers.
    assert_ne!(track, controls::switch_track_color(&p, accent, true, false, 1.0));
}

#[test]
fn an_enabled_switch_still_uses_the_accent_and_a_white_knob() {
    let mut ctx = egui::Context::default();
    let mut on = true;
    let mut accent = None;
    let (_, shapes) = frame(&mut ctx, Vec::new(), |ui| {
        accent = Some(theme::accent_color(ui));
        controls::toggle_switch(ui, &mut on, "All animations");
    });
    let accent = accent.unwrap();
    let (rects, circles) = painted(&shapes);
    let track = rects
        .iter()
        .find(|(r, _)| (r.width() - 40.0).abs() < 0.5 && (r.height() - 22.0).abs() < 0.5)
        .map(|(_, fill)| *fill)
        .expect("the switch track was not painted");
    let knob = circles
        .iter()
        .find(|(_, radius, _)| (*radius - 8.0).abs() < 0.5)
        .map(|(_, _, fill)| *fill)
        .expect("the switch knob was not painted");
    assert_eq!(track, accent);
    assert_eq!(knob, Color32::WHITE);
}

#[test]
fn a_disabled_segment_paints_the_dimmed_accent_and_muted_text() {
    let mut ctx = egui::Context::default();
    let mut selected = 1usize;
    let mut palette = None;
    let mut accent = None;
    let mut opacity = 1.0;
    let (_, shapes) = frame(&mut ctx, Vec::new(), |ui| {
        ui.add_enabled_ui(false, |ui| {
            palette = Some(theme::pal(ui));
            accent = Some(theme::accent_color(ui));
            opacity = ui.opacity();
            controls::segmented(ui, "corner", &OPTIONS, &mut selected);
        });
    });
    let (p, accent) = (palette.unwrap(), accent.unwrap());
    let dim = faded(controls::segment_accent(accent, false), opacity);
    let (rects, _) = painted(&shapes);

    assert!(
        rects.iter().any(|(_, fill)| *fill == dim),
        "the active segment must paint the 45% accent while disabled"
    );
    assert!(
        !rects.iter().any(|(_, fill)| *fill == faded(accent, opacity)),
        "a disabled segment must never paint the full accent"
    );
    assert_eq!(controls::segment_text_color(&p, false, true), p.muted);
    assert_ne!(controls::segment_accent(accent, false), accent);
}

#[test]
fn arrow_keys_move_the_segmented_selection_and_stop_at_the_ends() {
    let mut ctx = egui::Context::default();
    let mut selected = 0usize;

    // Frame one gives the first segment focus; no key yet.
    let (_, _) = frame(&mut ctx, Vec::new(), |ui| {
        ui.add_enabled_ui(true, |ui| {
            let id = ui.make_persistent_id(("corner", 0));
            ui.memory_mut(|m| m.request_focus(id));
            controls::segmented(ui, "corner", &OPTIONS, &mut selected);
        });
    });

    let press = |ctx: &mut egui::Context, k: Key, selected: &mut usize| -> bool {
        let (changed, _) = frame(ctx, vec![key(k)], |ui| {
            ui.add_enabled_ui(true, |ui| {
                let id = ui.make_persistent_id(("corner", 0));
                ui.memory_mut(|m| m.request_focus(id));
                controls::segmented(ui, "corner", &OPTIONS, selected)
            })
            .inner
        });
        changed
    };

    assert!(press(&mut ctx, Key::ArrowRight, &mut selected));
    assert_eq!(selected, 1);
    for expected in [2usize, 3] {
        assert!(press(&mut ctx, Key::ArrowRight, &mut selected));
        assert_eq!(selected, expected);
    }
    // Clamped at the last option: the key is used but nothing changes.
    assert!(!press(&mut ctx, Key::ArrowRight, &mut selected));
    assert_eq!(selected, 3);

    assert!(press(&mut ctx, Key::ArrowLeft, &mut selected));
    assert_eq!(selected, 2);
}

#[test]
fn a_disabled_segmented_control_ignores_the_arrow_keys() {
    let mut ctx = egui::Context::default();
    let mut selected = 1usize;
    let (changed, _) = frame(&mut ctx, vec![key(Key::ArrowRight)], |ui| {
        ui.add_enabled_ui(false, |ui| {
            let id = ui.make_persistent_id(("corner", 0));
            ui.memory_mut(|m| m.request_focus(id));
            controls::segmented(ui, "corner", &OPTIONS, &mut selected)
        })
        .inner
    });
    assert!(!changed);
    assert_eq!(selected, 1);
}
