//! The two controls whose state has to be legible: a switch and a segmented picker.
//!
//! They live outside `widgets.rs` and depend only on `crate::theme`, so the Linux screenshot
//! fixtures and their tests compile and draw the same code the app ships. The colour rules are
//! separate functions for the same reason: a test can assert the exact value a disabled control
//! paints instead of guessing at a shape.

use crate::theme::{
    ON_ACCENT, Palette, RADIUS_CONTROL, RADIUS_PILL, TEXT_BODY, accent_color, lerp_color,
    motion_enabled, pal,
};
use eframe::egui::{self, Color32, CornerRadius, Sense};

/// The track fill of a switch. A disabled track is the neutral inactive fill, never the accent,
/// so a sub-toggle under an off master can never read as active.
pub fn switch_track_color(
    p: &Palette,
    accent: Color32,
    enabled: bool,
    hovered: bool,
    t: f32,
) -> Color32 {
    if !enabled {
        // No accent at all: a disabled switch must not look like a quiet "on", even when its
        // stored value is true. The muted label and grey knob carry the rest of the state.
        return p.widget_inactive;
    }
    let rest = if hovered {
        lerp_color(p.widget_inactive, p.widget_hovered, 0.6)
    } else {
        p.widget_inactive
    };
    lerp_color(rest, accent, t)
}

/// The knob fill of a switch: muted grey while disabled, and otherwise the one of white/grey
/// that stays visible on the track. A white knob on the light theme's near-white off-track is
/// invisible, so the knob follows the track's luminance instead of always being white.
pub fn switch_knob_color(p: &Palette, enabled: bool, track: Color32) -> Color32 {
    if !enabled {
        return p.muted.gamma_multiply(0.75);
    }
    let luminance = 0.2126 * track.r() as f32 + 0.7152 * track.g() as f32 + 0.0722 * track.b() as f32;
    if luminance > 200.0 {
        p.muted
    } else {
        Color32::WHITE
    }
}

/// The accent a segmented control fills its active segment with. Disabled keeps 45%, which is
/// clearly dimmer than the enabled fill without vanishing into the track.
pub fn segment_accent(accent: Color32, enabled: bool) -> Color32 {
    if enabled {
        accent
    } else {
        accent.gamma_multiply(0.45)
    }
}

/// A segment's fill: the (possibly dimmed) accent when active, the hover wash when usable, and
/// nothing at all otherwise.
pub fn segment_fill(
    p: &Palette,
    accent: Color32,
    enabled: bool,
    active: bool,
    hovered: bool,
) -> Color32 {
    if active {
        accent
    } else if enabled && hovered {
        p.hover_bg
    } else {
        Color32::TRANSPARENT
    }
}

/// A segment's text: muted while disabled, on-accent on the active segment, body text elsewhere.
pub fn segment_text_color(p: &Palette, enabled: bool, active: bool) -> Color32 {
    if !enabled {
        p.muted
    } else if active {
        ON_ACCENT
    } else {
        p.text
    }
}

/// A switch: 40x22 track with an 18 px knob and its label on the left.
///
/// The knob interpolates over 160 ms while motion is enabled and snaps when it is not. egui
/// stops requesting repaints as soon as the animation settles, so an idle switch costs
/// nothing; the whole control is one allocation and two shapes.
pub fn toggle_switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let p = pal(ui);
    let accent = accent_color(ui);
    let enabled = ui.is_enabled();
    let row_w = ui.available_width();
    let label_color = if enabled { p.text } else { p.muted };
    let galley = ui.painter().layout(
        label.to_owned(),
        egui::FontId::proportional(TEXT_BODY),
        label_color,
        (row_w - 52.0).max(120.0),
    );
    let height = galley.size().y.max(24.0);
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(row_w, height), Sense::click());
    if enabled
        && (response.clicked()
            || (response.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter)
                })))
    {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, *on, label)
    });
    if ui.is_rect_visible(rect) {
        let t = if motion_enabled(ui.ctx()) {
            ui.ctx()
                .animate_bool_with_time(response.id.with("knob"), *on, 0.16)
        } else if *on {
            1.0
        } else {
            0.0
        };
        let track = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 40.0, rect.center().y - 11.0),
            egui::vec2(40.0, 22.0),
        );
        let track_color = switch_track_color(&p, accent, enabled, response.hovered(), t);
        ui.painter()
            .rect_filled(track, CornerRadius::same(RADIUS_PILL), track_color);
        let knob_x = egui::lerp((track.left() + 11.0)..=(track.right() - 11.0), t);
        ui.painter().circle_filled(
            egui::pos2(knob_x, track.center().y),
            8.0,
            switch_knob_color(&p, enabled, track_color),
        );
        if enabled && response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                CornerRadius::same(RADIUS_CONTROL),
                egui::Stroke::new(1.5, accent),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().galley(
            egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
            galley,
            label_color,
        );
    }
    response
}

/// An equal-width segmented control over `options`. Returns true when the selection changed.
///
/// One track, one accent fill on the active segment and a focus ring for Tab navigation; no
/// animation, so switching a mode costs a single frame. Left/Right move the selection while a
/// segment has focus, clamped at both ends.
pub fn segmented(
    ui: &mut egui::Ui,
    id_salt: &str,
    options: &[&str],
    selected: &mut usize,
) -> bool {
    if options.is_empty() {
        return false;
    }
    let enabled = ui.is_enabled();
    let p = pal(ui);
    let accent = segment_accent(accent_color(ui), enabled);
    let mut width = ui.available_width();
    if !width.is_finite() || width < 160.0 {
        width = 320.0;
    }
    let height = 28.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let pill = CornerRadius::same(RADIUS_PILL);
    ui.painter().rect_filled(rect, pill, p.surface_bg);
    ui.painter().rect_stroke(
        rect,
        pill,
        egui::Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    let seg_w = width / options.len() as f32;
    let mut changed = false;
    for (i, option) in options.iter().enumerate() {
        let seg = egui::Rect::from_min_size(
            egui::pos2(rect.left() + seg_w * i as f32, rect.top()),
            egui::vec2(seg_w, height),
        )
        .shrink(2.0);
        let response = ui.interact(seg, ui.make_persistent_id((id_salt, i)), Sense::click());
        let active = *selected == i;
        ui.painter().rect_filled(
            seg,
            pill,
            segment_fill(&p, accent, enabled, active, response.hovered()),
        );
        let color = segment_text_color(&p, enabled, active);
        let galley = ui.painter().layout_no_wrap(
            (*option).to_owned(),
            egui::FontId::proportional(TEXT_BODY),
            color,
        );
        ui.painter()
            .galley(seg.center() - galley.size() * 0.5, galley, color);
        if enabled && response.clicked() && !active {
            *selected = i;
            changed = true;
        }
        // Left/Right move the selection while a segment has focus, the same way a native
        // segmented control behaves. Clamped, so the ends do not wrap past the options.
        if enabled && response.has_focus() {
            let step = ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowLeft) {
                    Some(-1i32)
                } else if i.key_pressed(egui::Key::ArrowRight) {
                    Some(1i32)
                } else {
                    None
                }
            });
            if let Some(step) = step {
                let last = options.len() as i32 - 1;
                let next = (*selected as i32 + step).clamp(0, last) as usize;
                if next != *selected {
                    *selected = next;
                    changed = true;
                }
            }
        }
        if enabled && response.has_focus() {
            ui.painter().rect_stroke(
                seg,
                pill,
                egui::Stroke::new(1.5, accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    changed
}
