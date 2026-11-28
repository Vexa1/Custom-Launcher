//! The overlay position editor's custom picture, driven by real pointer events.
//!
//! `src/overlay_window.rs` needs a window, AppKit/Win32 and the egui texture loader, so it
//! cannot be linked here. The parts that choose *what* a press grabbed (`image_part`) and
//! *how* a drag changes the box (`drag_image`) live in the real `src/overlay_spec.rs`; this
//! test feeds `Event::PointerButton` / `Event::PointerMoved` through an `egui::Context` that
//! runs the same press -> drag -> release sequence, then checks the box and the JSON the
//! editor leaves for the launcher.

use creblox_vm_tests::egui::{
    self, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2,
};
use creblox_vm_tests::overlay_spec::{
    ImageBox, ImagePart, SavedImageBox, drag_image, image_part,
};

const VIEW: (f32, f32) = (1280.0, 720.0);

/// What the overlay window keeps between frames: the live box and the part the drag grabbed.
struct Editor {
    box_: ImageBox,
    drag: Option<ImagePart>,
}

impl Editor {
    fn new(box_: ImageBox) -> Self {
        Self { box_: box_, drag: None }
    }

    /// One frame of the editor's image hitbox, with the same rule the shipping window uses.
    fn frame(&mut self, ctx: &mut egui::Context, rect: Rect, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(VIEW.0, VIEW.1),
            )),
            events,
            ..Default::default()
        };
        let this = &mut *self;
        let mut out = ctx.run_ui(input, |ui| {
            let zone = rect.expand(12.0);
            let response = ui.interact(
                zone,
                ui.scope_id().with("overlay-image"),
                egui::Sense::click_and_drag(),
            );
            let part_at = |p: Pos2| {
                image_part(
                    (rect.left(), rect.top(), rect.width(), rect.height()),
                    (p.x, p.y),
                )
            };
            if response.drag_started() {
                // The press position decides, exactly like the window: a corner hit at press
                // keeps the drag even after the pointer leaves the grip.
                this.drag = ui
                    .input(|i| i.pointer.press_origin())
                    .and_then(part_at)
                    .or_else(|| ui.input(|i| i.pointer.interact_pos()).and_then(part_at));
            }
            if response.dragged() {
                if let Some(part) = this.drag {
                    let delta = response.drag_delta();
                    this.box_ = drag_image(this.box_, part, delta.x, delta.y, VIEW, false);
                }
            }
            if !response.dragged() {
                this.drag = None;
            }
        });
        // The harness draws nothing, so the glyph atlas deltas are never uploaded; drop them
        // explicitly instead of letting `FullOutput` panic on an unapplied delta.
        out.textures_delta.clear();
    }
}

fn moved(x: f32, y: f32) -> Event {
    Event::PointerMoved(Pos2::new(x, y))
}

fn button(x: f32, y: f32, pressed: bool) -> Event {
    Event::PointerButton {
        pos: Pos2::new(x, y),
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::default(),
    }
}

fn start() -> ImageBox {
    ImageBox {
        x: 100,
        y: 100,
        w: 300,
        h: 200,
    }
}

#[test]
fn dragging_the_body_moves_the_picture_by_the_pointer_delta() {
    let mut ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.0);
    let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(300.0, 200.0));
    let mut editor = Editor::new(start());

    editor.frame(&mut ctx, rect, vec![moved(220.0, 210.0)]);
    editor.frame(&mut ctx, rect, vec![button(220.0, 210.0, true)]);
    editor.frame(&mut ctx, rect, vec![moved(260.0, 240.0)]);
    editor.frame(&mut ctx, rect, vec![button(260.0, 240.0, false)]);

    // The body press resolves to Body, so only the position changes.
    assert_eq!(
        editor.box_,
        ImageBox {
            x: 140,
            y: 130,
            w: 300,
            h: 200
        }
    );
}

#[test]
fn dragging_the_bottom_right_handle_resizes_without_moving_the_origin() {
    let mut ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.0);
    let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(300.0, 200.0));
    let mut editor = Editor::new(start());

    // The corner sits at (400, 300); a press there must beat the body underneath it.
    editor.frame(&mut ctx, rect, vec![moved(400.0, 300.0)]);
    editor.frame(&mut ctx, rect, vec![button(400.0, 300.0, true)]);
    editor.frame(&mut ctx, rect, vec![moved(440.0, 320.0)]);
    editor.frame(&mut ctx, rect, vec![button(440.0, 320.0, false)]);

    assert_eq!(
        editor.box_,
        ImageBox {
            x: 100,
            y: 100,
            w: 340,
            h: 220
        }
    );
}

#[test]
fn a_corner_press_wins_over_the_body_and_misses_resolve_to_nothing() {
    let rect = (100.0, 100.0, 300.0, 200.0);
    assert_eq!(image_part(rect, (400.0, 300.0)), Some(ImagePart::Corner(3)));
    assert_eq!(image_part(rect, (100.0, 100.0)), Some(ImagePart::Corner(0)));
    assert_eq!(image_part(rect, (250.0, 200.0)), Some(ImagePart::Body));
    assert_eq!(image_part(rect, (10.0, 10.0)), None);
}

#[test]
fn the_confirmed_box_persists_into_the_panels_json() {
    // What Confirm leaves for the launcher: the box's four short keys in
    // `overlay-position.json` / the panels `image` object.
    let edited = ImageBox {
        x: 140,
        y: 130,
        w: 340,
        h: 220,
    };
    let panels_image = serde_json::json!({
        "x": edited.x,
        "y": edited.y,
        "w": edited.w,
        "h": edited.h
    });
    let saved = SavedImageBox::from_panels_image(&panels_image);
    assert_eq!(saved.x, 140);
    assert_eq!(saved.y, 130);
    assert_eq!(saved.w, 340);
    assert_eq!(saved.h, 220);
    assert!(saved.is_saved());

    // And through a real file, the way the launcher reads it back.
    let path = std::env::temp_dir().join(format!(
        "creblox-overlay-image-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, panels_image.to_string()).expect("write panels json");
    let text = std::fs::read_to_string(&path).expect("read panels json");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse panels json");
    assert_eq!(
        SavedImageBox::from_panels_image(&value),
        saved,
        "the editor's box must survive the panels file unchanged"
    );
    let _ = std::fs::remove_file(&path);
}
