//! The in-game HUD, laid out from the real `overlay-panels-<id>.json` (dev-only).
//!
//! `src/overlay_window.rs` needs a macOS/Windows window, CoreGraphics and winit, so it cannot
//! be linked in this VM. This module mirrors its panel layout exactly from the shipping design
//! tokens in that file (surface, hairline, divider, label/value colours and fonts, margins,
//! row heights, corner placement and stacking), and draws whatever panels JSON it is handed -
//! so the integration test renders the bytes the launcher actually wrote, not a fixture.
//!
//! Nothing here is part of a release build.

use std::sync::Arc;

use egui::epaint::Galley;
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Vec2};
use serde_json::Value;

use crate::shots::Probe;

// `src/overlay_window.rs`, "HUD design tokens".
const SURFACE: Color32 = Color32::from_rgb(14, 15, 18);
const LABEL: Color32 = Color32::from_rgb(0x8E, 0x93, 0x9E);
const VALUE: Color32 = Color32::from_rgb(0xF3, 0xF4, 0xF6);
const HAIRLINE: Color32 = Color32::from_rgb(0x2A, 0x2F, 0x3C);
const DIVIDER: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x33);
const DOT_OK: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);
const DOT_WARN: Color32 = Color32::from_rgb(0xF5, 0xB0, 0x4B);
const DOT_BAD: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);
const DOT_NEUTRAL: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);

const MARGIN: f32 = 16.0;
const GAP: f32 = 8.0;
const PAD: f32 = 12.0;
const ROW_H: f32 = 17.0;

// `overlay_window::paint_edit`: the draggable-card outline and the active-card accent.
const ACCENT: Color32 = Color32::from_rgb(120, 170, 255);

fn outline() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 150)
}

/// The stand-in picture's pixel size, used for the default (percentage) sizing.
const FIXTURE_IMAGE: (f32, f32) = (160.0, 64.0);

/// A deterministic stand-in for a user's PNG: a dark plate with an accent diagonal and a
/// lighter band, so a screenshot shows the picture's real rect and its edges are visible.
/// The stand-in picture, drawn with plain shapes: the software rasterizer used by these
/// screenshots does not sample the launcher's own textures, and what the artifact has to show
/// is the picture's rect and its selection frame, not its pixels.
fn paint_fixture(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(0x1B, 0x22, 0x33));
    let bars: i32 = 4;
    for i in 0..bars {
        let t = i as f32 / bars as f32;
        let bar = Rect::from_min_size(
            Pos2::new(rect.left() + 10.0, rect.top() + 12.0 + t * (rect.height() - 30.0)),
            Vec2::new((rect.width() - 20.0) * (0.35 + 0.6 * t), 8.0),
        );
        painter.rect_filled(
            bar,
            CornerRadius::same(2),
            Color32::from_rgb(
                (0x40 + 40 * i) as u8,
                (0x5C + 30 * i) as u8,
                (0x9A + 20 * i) as u8,
            ),
        );
    }
    let band = Rect::from_min_size(
        Pos2::new(rect.left() + 10.0, rect.bottom() - 18.0),
        Vec2::new(rect.width() - 20.0, 8.0),
    );
    painter.rect_filled(band, CornerRadius::same(2), Color32::from_rgb(0xE8, 0x98, 0x4B));
    painter.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0, Color32::from_rgb(0x39, 0x43, 0x59)),
        egui::StrokeKind::Inside,
    );
}

/// A dashed rectangle, exactly the pattern the shipping editor paints.
fn dashed_rect(painter: &egui::Painter, frame: Rect, color: Color32, px: f32) {
    let dash = 6.0;
    let stroke = Stroke::new(px, color);
    let mut x = frame.left();
    while x < frame.right() {
        let end = (x + dash).min(frame.right());
        painter.line_segment(
            [Pos2::new(x, frame.top()), Pos2::new(end, frame.top())],
            stroke,
        );
        painter.line_segment(
            [Pos2::new(x, frame.bottom()), Pos2::new(end, frame.bottom())],
            stroke,
        );
        x += dash * 2.0;
    }
    let mut y = frame.top();
    while y < frame.bottom() {
        let end = (y + dash).min(frame.bottom());
        painter.line_segment(
            [Pos2::new(frame.left(), y), Pos2::new(frame.left(), end)],
            stroke,
        );
        painter.line_segment(
            [Pos2::new(frame.right(), y), Pos2::new(frame.right(), end)],
            stroke,
        );
        y += dash * 2.0;
    }
}

/// One HUD line split into a muted micro-label and a tabular value, as `overlay_window` does.
struct Row {
    /// Already tracked (uppercase with U+2009), like `Panel::rows` in the shipping window.
    label: String,
    value: String,
}

fn tracked(raw: &str) -> String {
    let upper = raw.trim().to_uppercase();
    let mut out = String::with_capacity(upper.len() * 2);
    for (i, ch) in upper.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out
}

fn split_row(line: &str) -> Row {
    let line = line.trim();
    if let Some((label, value)) = line.split_once(':') {
        let label = label.trim();
        if !label.is_empty() && !label.bytes().any(|b| b.is_ascii_digit()) {
            return Row {
                label: tracked(label),
                value: value.trim().to_string(),
            };
        }
    }
    Row {
        label: String::new(),
        value: line.to_string(),
    }
}

/// The 5 px health dot: ping/latency hue by number, neutral for the server row, none elsewhere.
fn tone(label: &str, value: &str) -> Option<Color32> {
    let label = label.to_ascii_lowercase();
    if label.contains("ping") || label.contains("latency") {
        let digits: String = value
            .chars()
            .filter(char::is_ascii_digit)
            .take(5)
            .collect();
        let ms = digits.parse::<u32>().ok()?;
        return Some(if ms < 60 {
            DOT_OK
        } else if ms < 120 {
            DOT_WARN
        } else {
            DOT_BAD
        });
    }
    if label.contains("server") || label.contains("region") {
        return Some(DOT_NEUTRAL);
    }
    None
}

/// Draws the HUD `spec` over a game-like backdrop and registers every text box, so the layout
/// rules (`Probe::problems`) can reject a clipped or overlapping HUD.
pub fn draw(ui: &mut egui::Ui, probe: &mut Probe, spec: &Value, now_ms: u64) {
    draw_impl(ui, probe, spec, now_ms, None)
}

/// The position editor, mirrored from `overlay_window::paint_edit`: a dashed box around every
/// card, an accent box on `active` and the bottom bar with that card's live `(x, y)`, the drag
/// hint and Reset / Confirm.
pub fn draw_edit(ui: &mut egui::Ui, probe: &mut Probe, spec: &Value, now_ms: u64, active: &str) {
    draw_impl(ui, probe, spec, now_ms, Some(active))
}

fn draw_impl(
    ui: &mut egui::Ui,
    probe: &mut Probe,
    spec: &Value,
    now_ms: u64,
    edit: Option<&str>,
) {
    let area = ui.max_rect();
    let painter = ui.painter().clone();
    for i in 0..160 {
        let t = i as f32 / 159.0;
        let rect = Rect::from_min_size(
            Pos2::new(0.0, area.height() * t),
            Vec2::new(area.width(), area.height() / 159.0 + 1.0),
        );
        painter.rect_filled(
            rect,
            CornerRadius::ZERO,
            Color32::from_rgb(
                (18.0 + 26.0 * t) as u8,
                (26.0 + 30.0 * t) as u8,
                (38.0 + 26.0 * t) as u8,
            ),
        );
    }
    probe.widget("hud.game", area, false);

    // The custom image, placed by the same `ImageBox` arithmetic the shipping window uses.
    let mut image_placed: Option<(crate::overlay_spec::ImageBox, Rect)> = None;
    let ppx = 1.0 / painter.ctx().pixels_per_point().max(0.5);
    if let Some(img) = spec["image"].as_object().filter(|i| !i.is_empty()) {
        let saved = (
            img["w"].as_u64().unwrap_or(0) as u16,
            img["h"].as_u64().unwrap_or(0) as u16,
        );
        let min = crate::overlay_spec::ImageBox::MIN as u16;
        let view = (area.width(), area.height());
        let box_ = if saved.0 >= min && saved.1 >= min {
            crate::overlay_spec::ImageBox {
                // The box's own position; a 0.5.51 spec kept it in the offsets.
                x: img["x"]
                    .as_f64()
                    .or_else(|| img["offset_x"].as_f64())
                    .unwrap_or(0.0) as i16,
                y: img["y"]
                    .as_f64()
                    .or_else(|| img["offset_y"].as_f64())
                    .unwrap_or(0.0) as i16,
                w: saved.0 as i16,
                h: saved.1 as i16,
            }
        } else {
            let size = crate::overlay_spec::ImageBox::size(
                (FIXTURE_IMAGE.0, FIXTURE_IMAGE.1),
                img["size"].as_u64().unwrap_or(25) as u8,
                view.1,
                (0, 0),
            );
            crate::overlay_spec::ImageBox::placed(
                img["corner"].as_u64().unwrap_or(0) as u8,
                size,
                view,
                (
                    img["offset_x"].as_f64().unwrap_or(0.0) as f32,
                    img["offset_y"].as_f64().unwrap_or(0.0) as f32,
                ),
            )
        }
        .clamped(view);
        let rect = Rect::from_min_size(
            Pos2::new(area.left() + box_.x as f32, area.top() + box_.y as f32),
            Vec2::new(box_.w as f32, box_.h as f32),
        );
        paint_fixture(&painter, rect);
        probe.widget("hud.image", rect, false);
        if let Some(active) = edit {
            let is_active = active == "image";
            dashed_rect(
                &painter,
                rect.expand(4.0),
                if is_active { ACCENT } else { outline() },
                ppx,
            );
            if is_active {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.5, ACCENT),
                    egui::StrokeKind::Outside,
                );
            }
            for (i, corner) in [
                rect.left_top(),
                rect.right_top(),
                rect.left_bottom(),
                rect.right_bottom(),
            ]
            .into_iter()
            .enumerate()
            {
                let target = Rect::from_center_size(corner, Vec2::new(24.0, 24.0));
                probe.widget(format!("hud.image.handle.{i}"), target, true);
                let square = Rect::from_center_size(corner, Vec2::new(8.0, 8.0));
                painter.rect_filled(square, CornerRadius::same(1), ACCENT);
                painter.rect_stroke(
                    square,
                    CornerRadius::same(1),
                    Stroke::new(ppx, Color32::from_rgb(12, 14, 20)),
                    egui::StrokeKind::Outside,
                );
                probe.widget(format!("hud.image.grip.{i}"), square, false);
            }
        }
        image_placed = Some((box_, rect));
    }

    let opacity = spec["opacity"].as_f64().unwrap_or(0.9).clamp(0.2, 1.0) as f32;
    let ppp = painter.ctx().pixels_per_point().max(0.5);
    let px = 1.0 / ppp;
    let label_font = FontId::proportional(10.0);
    let value_font = FontId::monospace(12.5);
    let mut used = [0.0_f32; 4];

    let Some(panels) = spec["panels"].as_array() else {
        return;
    };
    // Every card's final rect, for the editor's bar and its active-panel outline.
    let mut placed: Vec<(String, Rect)> = Vec::with_capacity(panels.len());
    for (pi, panel) in panels.iter().enumerate() {
        let title = tracked(panel["title"].as_str().unwrap_or(""));
        // Live rows are formatted here, exactly as `overlay_window::tick_timers` does it: the
        // file carries the panel's `joined_at_unix_ms` anchor and a `{elapsed}` token, never a
        // time that would go stale between launcher ticks. The value font is monospace, so
        // the digits are tabular and the row never jitters as the seconds change.
        let started_at_ms = panel["joined_at_unix_ms"].as_u64();
        let lines: Vec<String> = panel["lines"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(|s| s.as_str())
                    .map(|s| match started_at_ms {
                        Some(start) => crate::overlay_spec::with_elapsed(s, start, now_ms),
                        None => s.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rows: Vec<Row> = lines.iter().map(|l| split_row(l)).collect();
        let title_galley: Option<Arc<Galley>> = (!title.is_empty())
            .then(|| painter.layout_no_wrap(title, label_font.clone(), LABEL));

        // The party card, mirrored from `overlay_window::relayout`/`paint_layout`: a strip of
        // 44 px rounded squares, each with its 2 px status outline and the name under it.
        if let Some(tiles) = panel["tiles"].as_array().filter(|t| !t.is_empty()) {
            const SQUARE: f32 = 44.0;
            const CELL_GAP: f32 = 8.0;
            const NAME_GAP: f32 = 3.0;
            const NAME_H: f32 = 13.0;
            let cols = tiles.len().min(4) as f32;
            let rows_n = tiles.len().div_ceil(4);
            let title_h = title_galley.as_ref().map_or(0.0, |g| g.size().y + GAP);
            let rule_h = if title_galley.is_some() { px + GAP } else { 0.0 };
            let width = cols * SQUARE + (cols - 1.0) * CELL_GAP + PAD * 2.0;
            let height = PAD * 2.0
                + title_h
                + rule_h
                + rows_n as f32 * (SQUARE + NAME_GAP + NAME_H)
                + (rows_n as f32 - 1.0).max(0.0) * CELL_GAP;
            let size = Vec2::new(width, height);
            let corner = panel["corner"].as_u64().unwrap_or(3).min(3) as usize;
            let x = if corner % 2 == 1 {
                area.right() - size.x - MARGIN
            } else {
                area.left() + MARGIN
            };
            let y = if corner < 2 {
                area.top() + MARGIN + used[corner]
            } else {
                area.bottom() - size.y - MARGIN - used[corner]
            };
            used[corner] += size.y + GAP;
            let offset = Vec2::new(
                panel["offset_x"].as_f64().unwrap_or(0.0) as f32,
                panel["offset_y"].as_f64().unwrap_or(0.0) as f32,
            );
            let rect = Rect::from_min_size(Pos2::new(x + offset.x, y + offset.y), size);
            let panel_id = format!("hud.panel.{pi}");
            let panel_key = panel["id"].as_str().unwrap_or_default().to_string();
            placed.push((panel_key.clone(), rect));
            probe.widget(panel_id.clone(), rect, false);
            if let Some(active) = edit {
                let is_active = active == panel_key;
                dashed_rect(
                    &painter,
                    rect.expand(4.0),
                    if is_active { ACCENT } else { outline() },
                    px,
                );
                if is_active {
                    painter.rect_stroke(
                        rect,
                        CornerRadius::same(8),
                        Stroke::new(1.5, ACCENT),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            let alpha = (opacity * 255.0).round() as u8;
            let surface =
                Color32::from_rgba_unmultiplied(SURFACE.r(), SURFACE.g(), SURFACE.b(), alpha);
            painter.rect_filled(rect, CornerRadius::same(8), surface);
            painter.rect_stroke(
                rect,
                CornerRadius::same(8),
                Stroke::new(px, HAIRLINE),
                egui::StrokeKind::Inside,
            );
            let mut cursor_y = rect.top() + PAD;
            if let Some(g) = &title_galley {
                let pos = Pos2::new(rect.left() + PAD, cursor_y);
                painter.galley(pos, g.clone(), LABEL);
                probe.text(
                    format!("{panel_id}.title"),
                    Rect::from_min_size(pos, g.size()),
                    g.size(),
                );
                probe.contrast(format!("{panel_id}.title"), LABEL, surface);
                cursor_y += g.size().y + GAP;
                painter.line_segment(
                    [
                        Pos2::new(rect.left() + PAD, cursor_y),
                        Pos2::new(rect.right() - PAD, cursor_y),
                    ],
                    Stroke::new(px, DIVIDER),
                );
                cursor_y += px + GAP;
            }
            for (i, tile) in tiles.iter().enumerate() {
                let status = crate::party_hud::TileStatus::from_key(
                    tile["status"].as_str().unwrap_or("loading"),
                );
                let (r, g, b) = status.rgb();
                let outline = Color32::from_rgb(r, g, b);
                let col = (i % 4) as f32;
                let row = (i / 4) as f32;
                let cell = Rect::from_min_size(
                    Pos2::new(
                        rect.left() + PAD + col * (SQUARE + CELL_GAP),
                        cursor_y + row * (SQUARE + NAME_GAP + NAME_H + CELL_GAP),
                    ),
                    Vec2::splat(SQUARE),
                );
                painter.rect_filled(
                    cell,
                    CornerRadius::same(8),
                    Color32::from_rgba_unmultiplied(255, 255, 255, 16),
                );
                let initials = painter.layout_no_wrap(
                    tile["initials"].as_str().unwrap_or("?").to_string(),
                    FontId::monospace(15.0),
                    VALUE,
                );
                let ipos = Pos2::new(
                    cell.center().x - initials.size().x / 2.0,
                    cell.center().y - initials.size().y / 2.0,
                );
                painter.galley(ipos, initials.clone(), VALUE);
                probe.text(
                    format!("{panel_id}.initials.{i}"),
                    Rect::from_min_size(ipos, initials.size()),
                    initials.size(),
                );
                painter.rect_stroke(
                    cell,
                    CornerRadius::same(8),
                    Stroke::new(2.0, outline),
                    egui::StrokeKind::Inside,
                );
                let name = painter.layout_no_wrap(
                    crate::overlay_spec::shorten(tile["name"].as_str().unwrap_or(""), 7),
                    FontId::proportional(9.0),
                    LABEL,
                );
                let npos = Pos2::new(
                    cell.center().x - name.size().x / 2.0,
                    cell.bottom() + NAME_GAP,
                );
                painter.galley(npos, name.clone(), LABEL);
                probe.text(
                    format!("{panel_id}.name.{i}"),
                    Rect::from_min_size(npos, name.size()),
                    name.size(),
                );
                probe.contrast(format!("{panel_id}.name.{i}"), LABEL, surface);
            }
            continue;
        }

        struct Measured {
            label: Option<Arc<Galley>>,
            value: Arc<Galley>,
            tone: Option<Color32>,
            height: f32,
        }
        let mut label_w = 0.0_f32;
        let mut value_w = 0.0_f32;
        let mut has_dot = false;
        let mut measured: Vec<Measured> = Vec::with_capacity(rows.len());
        for row in &rows {
            let label = (!row.label.is_empty()).then(|| {
                painter.layout_no_wrap(row.label.clone(), label_font.clone(), LABEL)
            });
            let value = painter.layout_no_wrap(row.value.clone(), value_font.clone(), VALUE);
            label_w = label_w.max(label.as_ref().map_or(0.0, |g| g.size().x));
            value_w = value_w.max(value.size().x);
            let tone = tone(&row.label, &row.value);
            has_dot |= tone.is_some();
            let height = ROW_H
                .max(label.as_ref().map_or(0.0, |g| g.size().y))
                .max(value.size().y)
                + 2.0;
            measured.push(Measured {
                label,
                value,
                tone,
                height,
            });
        }
        let dot_w = if has_dot { 11.0 } else { 0.0 };

        let mut width = label_w + if label_w > 0.0 { 14.0 } else { 0.0 } + dot_w + value_w + PAD * 2.0;
        let mut height = PAD * 2.0;
        if let Some(t) = &title_galley {
            width = width.max(t.size().x + PAD * 2.0);
            height += t.size().y + GAP;
        }
        if !measured.is_empty() {
            height += px + GAP + measured.iter().map(|r| r.height).sum::<f32>();
        }
        let size = Vec2::new(width, height);

        let corner = panel["corner"].as_u64().unwrap_or(3).min(3) as usize;
        let x = if corner % 2 == 1 {
            area.right() - size.x - MARGIN
        } else {
            area.left() + MARGIN
        };
        let y = if corner < 2 {
            area.top() + MARGIN + used[corner]
        } else {
            area.bottom() - size.y - MARGIN - used[corner]
        };
        used[corner] += size.y + GAP;
        // Each card carries its own offset, exactly like `overlay_window::relayout`.
        let offset = Vec2::new(
            panel["offset_x"].as_f64().unwrap_or(0.0) as f32,
            panel["offset_y"].as_f64().unwrap_or(0.0) as f32,
        );
        let rect = Rect::from_min_size(Pos2::new(x + offset.x, y + offset.y), size);
        let panel_id = format!("hud.panel.{pi}");
        let panel_key = panel["id"].as_str().unwrap_or_default().to_string();
        placed.push((panel_key.clone(), rect));
        probe.widget(panel_id.clone(), rect, false);
        if let Some(active) = edit {
            let is_active = active == panel_key;
            dashed_rect(
                &painter,
                rect.expand(4.0),
                if is_active { ACCENT } else { outline() },
                px,
            );
            if is_active {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(8),
                    Stroke::new(1.5, ACCENT),
                    egui::StrokeKind::Outside,
                );
            }
        }

        let alpha = (opacity * 255.0).round() as u8;
        let surface = Color32::from_rgba_unmultiplied(SURFACE.r(), SURFACE.g(), SURFACE.b(), alpha);
        painter.rect_filled(rect, CornerRadius::same(8), surface);
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(px, HAIRLINE),
            egui::StrokeKind::Inside,
        );

        let mut cursor_y = rect.top() + PAD;
        if let Some(g) = &title_galley {
            let pos = Pos2::new(rect.left() + PAD, cursor_y);
            painter.galley(pos, g.clone(), LABEL);
            probe.text(
                format!("{panel_id}.title"),
                Rect::from_min_size(pos, g.size()),
                g.size(),
            );
            probe.contrast(format!("{panel_id}.title"), LABEL, SURFACE);
            cursor_y += g.size().y + GAP;
        }

        if !measured.is_empty() {
            let mut rule_y = cursor_y;
            painter.line_segment(
                [
                    Pos2::new(rect.left() + PAD, rule_y),
                    Pos2::new(rect.right() - PAD, rule_y),
                ],
                Stroke::new(px, DIVIDER),
            );
            cursor_y += px + GAP;
            let count = measured.len();
            for (i, row) in measured.iter().enumerate() {
                let value_pos = Pos2::new(
                    rect.right() - PAD - row.value.size().x,
                    cursor_y + (row.height - row.value.size().y) / 2.0,
                );
                let label_pos = row
                    .label
                    .as_ref()
                    .map(|g| Pos2::new(rect.left() + PAD, cursor_y + (row.height - g.size().y) / 2.0))
                    .unwrap_or(Pos2::new(rect.left() + PAD, value_pos.y));
                if let Some(g) = &row.label {
                    painter.galley(label_pos, g.clone(), LABEL);
                    probe.text(
                        format!("{panel_id}.label.{i}"),
                        Rect::from_min_size(label_pos, g.size()),
                        g.size(),
                    );
                    probe.contrast(format!("{panel_id}.label.{i}"), LABEL, SURFACE);
                }
                if let Some(color) = row.tone {
                    let dot_pos = Pos2::new(value_pos.x - 8.0, cursor_y + row.height / 2.0);
                    painter.circle_filled(dot_pos, 2.5, color);
                    probe.widget(
                        format!("{panel_id}.dot.{i}"),
                        Rect::from_center_size(dot_pos, Vec2::splat(5.0)),
                        false,
                    );
                }
                painter.galley(value_pos, row.value.clone(), VALUE);
                probe.text(
                    format!("{panel_id}.value.{i}"),
                    Rect::from_min_size(value_pos, row.value.size()),
                    row.value.size(),
                );
                probe.contrast(format!("{panel_id}.value.{i}"), VALUE, SURFACE);
                cursor_y += row.height;
                if i + 1 < count {
                    rule_y = cursor_y;
                    painter.line_segment(
                        [
                            Pos2::new(rect.left() + PAD, rule_y),
                            Pos2::new(rect.right() - PAD, rule_y),
                        ],
                        Stroke::new(px, DIVIDER),
                    );
                }
            }
        }
    }

    let Some(active) = edit else {
        return;
    };
    let (panel_rect, readout) = if active == "image" {
        let Some((box_, rect)) = image_placed else {
            return;
        };
        (
            rect,
            format!("image  {}, {}   {} × {}", box_.x, box_.y, box_.w, box_.h),
        )
    } else {
        let Some((_, rect)) = placed.iter().find(|(id, _)| id == active) else {
            return;
        };
        let offset_x = panels
            .iter()
            .find(|p| p["id"].as_str() == Some(active))
            .and_then(|p| p["offset_x"].as_i64())
            .unwrap_or(0);
        let offset_y = panels
            .iter()
            .find(|p| p["id"].as_str() == Some(active))
            .and_then(|p| p["offset_y"].as_i64())
            .unwrap_or(0);
        (*rect, format!("{active}  {offset_x}, {offset_y}"))
    };

    // The same measured bar as `overlay_window::paint_edit`: every part starts where the one
    // before it ends, so the hint can never land on the Reset pill.
    let readout_g = painter.layout_no_wrap(readout.clone(), FontId::monospace(11.5), VALUE);
    let pad = 10.0;
    let gap = 10.0;
    let btn_w = 60.0;
    let confirm_w = 78.0;
    let bar_h = 36.0;
    let label_font = FontId::proportional(10.0);
    let fixed = pad + readout_g.size().x + gap + gap + btn_w + 6.0 + confirm_w + pad;
    let room = (area.width() - 16.0).max(120.0);
    let mut hint = "drag · Enter save · Esc cancel";
    let mut hint_g = painter.layout_no_wrap(hint.to_string(), label_font.clone(), LABEL);
    if fixed + hint_g.size().x > room {
        hint = "drag · Enter save";
        hint_g = painter.layout_no_wrap(hint.to_string(), label_font.clone(), LABEL);
    }
    if fixed + hint_g.size().x > room {
        hint = "drag · Enter";
        hint_g = painter.layout_no_wrap(hint.to_string(), label_font.clone(), LABEL);
    }
    let bar_w = (fixed + hint_g.size().x).min(room);
    // The handles are 24 px targets around the corners, so the bar clears them by 16 px.
    let below = panel_rect.max.y + 16.0;
    let bar_y = if below + bar_h <= area.max.y - 8.0 {
        below
    } else {
        (panel_rect.min.y - 16.0 - bar_h).max(area.min.y + 8.0)
    };
    let bar_x = panel_rect.min.x.clamp(
        area.min.x + 8.0,
        (area.max.x - bar_w - 8.0).max(area.min.x + 8.0),
    );
    let bar = Rect::from_min_size(Pos2::new(bar_x, bar_y), Vec2::new(bar_w, bar_h));
    painter.rect_filled(bar, CornerRadius::same(8), SURFACE);
    painter.rect_stroke(
        bar,
        CornerRadius::same(8),
        Stroke::new(px, HAIRLINE),
        egui::StrokeKind::Inside,
    );
    probe.widget("hud.edit.bar", bar, false);

    let confirm = Rect::from_min_size(
        Pos2::new(bar.right() - pad - confirm_w, bar.top() + 5.0),
        Vec2::new(confirm_w, bar_h - 10.0),
    );
    let reset = Rect::from_min_size(
        Pos2::new(confirm.left() - 6.0 - btn_w, bar.top() + 5.0),
        Vec2::new(btn_w, bar_h - 10.0),
    );
    let readout_pos = Pos2::new(bar.left() + pad, bar.center().y);
    let hint_pos = Pos2::new(readout_pos.x + readout_g.size().x + gap, bar.center().y);
    painter.text(
        readout_pos,
        egui::Align2::LEFT_CENTER,
        &readout,
        FontId::monospace(11.5),
        VALUE,
    );
    painter.text(
        hint_pos,
        egui::Align2::LEFT_CENTER,
        hint,
        label_font,
        LABEL,
    );
    probe.text(
        "hud.edit.readout",
        Rect::from_min_size(
            Pos2::new(readout_pos.x, bar.center().y - readout_g.size().y / 2.0),
            readout_g.size(),
        ),
        readout_g.size(),
    );
    probe.text(
        "hud.edit.hint",
        Rect::from_min_size(
            Pos2::new(hint_pos.x, bar.center().y - hint_g.size().y / 2.0),
            hint_g.size(),
        ),
        hint_g.size(),
    );
    for (rect, label) in [(reset, "Reset"), (confirm, "Confirm")] {
        painter.rect_filled(
            rect,
            CornerRadius::same(6),
            Color32::from_rgba_unmultiplied(255, 255, 255, 16),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(px, HAIRLINE),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.0),
            VALUE,
        );
        let id = format!("hud.edit.{}", label.to_ascii_lowercase());
        let measured = painter
            .layout_no_wrap(label.to_string(), FontId::proportional(11.0), VALUE)
            .size();
        probe.text(id.clone(), rect, measured);
        probe.widget(id, rect, true);
        probe.contrast(format!("hud.edit.{}", label.to_ascii_lowercase()), VALUE, SURFACE);
    }
    probe.contrast("hud.edit.readout", VALUE, SURFACE);
    probe.contrast("hud.edit.hint", LABEL, SURFACE);
}
