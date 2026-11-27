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
pub fn draw(ui: &mut egui::Ui, probe: &mut Probe, spec: &Value) {
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

    let opacity = spec["opacity"].as_f64().unwrap_or(0.9).clamp(0.2, 1.0) as f32;
    let ppp = painter.ctx().pixels_per_point().max(0.5);
    let px = 1.0 / ppp;
    let label_font = FontId::proportional(10.0);
    let value_font = FontId::monospace(12.5);
    let mut used = [0.0_f32; 4];

    let Some(panels) = spec["panels"].as_array() else {
        return;
    };
    for (pi, panel) in panels.iter().enumerate() {
        let title = tracked(panel["title"].as_str().unwrap_or(""));
        let lines: Vec<String> = panel["lines"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let rows: Vec<Row> = lines.iter().map(|l| split_row(l)).collect();
        let title_galley: Option<Arc<Galley>> = (!title.is_empty())
            .then(|| painter.layout_no_wrap(title, label_font.clone(), LABEL));

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
        let rect = Rect::from_min_size(Pos2::new(x, y), size);
        let panel_id = format!("hud.panel.{pi}");
        probe.widget(panel_id.clone(), rect, false);

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
}
