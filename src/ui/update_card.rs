//! Release notes and build output for the Launcher Updates card.
//!
//! Theme-only on purpose: the Linux previews include this file, so the formatted notes and the
//! error log in the screenshots are the same code the app draws. The parser is plain Rust and is
//! tested without a window in `vm-tests/tests/update_card.rs`.

use crate::theme::pal;
use eframe::egui::{self, Color32, FontId, TextFormat};

/// One rendered piece of a release-notes body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Bullet(String),
    Paragraph(String),
}

/// A release body split into what the card shows and the raw download material.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub blocks: Vec<Block>,
    /// Raw lines that hold `.zip` URLs or SHA-256 checksums. They are reachable from the
    /// collapsible "Details" drawer instead of being dumped into the notes.
    pub details: Vec<String>,
}

/// One run of inline text with the markdown markers already removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub code: bool,
}

/// A line that belongs in the Details drawer rather than the body.
fn is_detail_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let zip_link = (lower.contains("http://") || lower.contains("https://"))
        && (lower.contains(".zip") || lower.contains("checksum"));
    let sha = lower.contains("sha-256") || lower.contains("sha256");
    let long_hash = line
        .split(|c: char| !c.is_ascii_hexdigit())
        .any(|word| word.len() >= 64);
    zip_link || sha || long_hash
}

/// Splits release notes into formatted blocks and a raw details list.
///
/// `## Heading`, `- bullet` and plain paragraphs are recognised; the markers are removed here so
/// the renderer never sees them. Blank lines only separate blocks.
pub fn parse_notes(notes: &str) -> Notes {
    let mut out = Notes::default();
    for raw in notes.replace("\r\n", "\n").lines() {
        let line = raw.trim();
        if line.is_empty() || line == "---" || line == "***" {
            continue;
        }
        if is_detail_line(line) {
            // The drawer holds the raw destination, not the markdown that surrounded it.
            let plain: String = spans(line).into_iter().map(|span| span.text).collect();
            out.details.push(plain.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let text = rest.trim_start_matches('#').trim();
            if !text.is_empty() {
                out.blocks.push(Block::Heading(text.to_string()));
            }
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("• "))
        {
            let text = rest.trim();
            if !text.is_empty() {
                out.blocks.push(Block::Bullet(text.to_string()));
            }
            continue;
        }
        out.blocks.push(Block::Paragraph(line.to_string()));
    }
    out
}

/// Removes `**bold**`, `` `code` `` and `[label](url)` markers, reporting each run's style.
///
/// Unclosed markers are treated as literal text so a half-written note never swallows the rest
/// of the line.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut code = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let push = |out: &mut Vec<Span>, buf: &mut String, bold: bool, code: bool| {
        if !buf.is_empty() {
            out.push(Span {
                text: std::mem::take(buf),
                bold,
                code,
            });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            push(&mut out, &mut buf, bold, code);
            bold = !bold;
            i += 2;
            continue;
        }
        if c == '`' {
            push(&mut out, &mut buf, bold, code);
            code = !code;
            i += 1;
            continue;
        }
        if c == '[' {
            // `[label](url)`: keep the label, drop the raw URL.
            if let Some(close) = chars[i + 1..].iter().position(|c| *c == ']') {
                let close = i + 1 + close;
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close + 2..].iter().position(|c| *c == ')') {
                        let label: String = chars[i + 1..close].iter().collect();
                        buf.push_str(&label);
                        i = close + 2 + end + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    push(&mut out, &mut buf, bold, code);
    out
}

/// True when an install error is the local compiler failing, not a download or setup problem.
pub fn is_build_failure(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("build failed") || lower.contains("exit 101")
}

fn format(spans: &[Span], size: f32, color: Color32, p: &crate::theme::Palette) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for span in spans {
        let (font_id, color, background) = if span.code {
            (
                FontId::monospace((size - 0.5).max(8.0)),
                color,
                p.surface_bg,
            )
        } else if span.bold {
            (
                FontId::new(size, egui::FontFamily::Name("Bold".into())),
                color,
                Color32::TRANSPARENT,
            )
        } else {
            (FontId::proportional(size), color, Color32::TRANSPARENT)
        };
        job.append(
            &span.text,
            0.0,
            TextFormat {
                font_id,
                color,
                background,
                ..Default::default()
            },
        );
    }
    job
}

/// The formatted release notes, with raw download links and hashes in a Details drawer.
pub fn paint_notes(ui: &mut egui::Ui, notes: &str) {
    let p = pal(ui);
    let parsed = parse_notes(notes);
    if parsed.blocks.is_empty() && parsed.details.is_empty() {
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("creblox_release_notes")
        .max_height(150.0)
        .show(ui, |ui| {
            for block in &parsed.blocks {
                match block {
                    Block::Heading(text) => {
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(text)
                                .font(crate::theme::font_section())
                                .color(p.text),
                        );
                        ui.add_space(2.0);
                    }
                    Block::Bullet(text) => {
                        ui.horizontal_top(|ui| {
                            ui.label(
                                egui::RichText::new("•")
                                    .size(crate::theme::TEXT_BODY)
                                    .color(p.muted),
                            );
                            let job = format(&spans(text), crate::theme::TEXT_BODY, p.text, &p);
                            ui.add(egui::Label::new(job).wrap());
                        });
                        ui.add_space(1.0);
                    }
                    Block::Paragraph(text) => {
                        let job = format(&spans(text), crate::theme::TEXT_BODY, p.text, &p);
                        ui.add(egui::Label::new(job).wrap());
                        ui.add_space(2.0);
                    }
                }
            }
        });
    if !parsed.details.is_empty() {
        egui::CollapsingHeader::new(format!("Details ({})", parsed.details.len()))
            .id_salt("creblox_release_notes_details")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("creblox_release_notes_raw")
                    .max_height(90.0)
                    .show(ui, |ui| {
                        for line in &parsed.details {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(line)
                                        .monospace()
                                        .size(10.5)
                                        .color(p.muted),
                                )
                                .wrap(),
                            );
                        }
                    });
            });
    }
}

/// The captured compiler/installer output, expanded under the error row.
pub fn paint_log(ui: &mut egui::Ui, lines: &[String]) {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.surface_bg)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if lines.is_empty() {
                ui.label(
                    egui::RichText::new("No output was captured. The full log is in update.log.")
                        .size(11.5)
                        .color(p.muted),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("creblox_update_error_log")
                .max_height(170.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in lines {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(line)
                                    .monospace()
                                    .size(11.0)
                                    .color(p.muted),
                            )
                            .wrap(),
                        );
                    }
                });
        });
}
