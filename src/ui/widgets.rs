use crate::theme::{accent_color, card_frame, compact, lerp_color, pal};
use crate::types::{Action, CardPreview, Game};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2};
use std::collections::HashMap;

pub fn avatar_glass_frame(ui: &egui::Ui, pad: i8) -> egui::Frame {
    card_frame(ui, pad)
}

pub fn thumb_styled_with_video(
    ui: &mut egui::Ui,
    url: Option<&String>,
    video_tex: Option<&egui::TextureHandle>,
    is_video_loading: bool,
    size: Vec2,
    radius: u8,
    bordered: bool,
) -> egui::Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let r = CornerRadius::same(radius);
        ui.painter().rect_filled(rect, r, p.placeholder);
        if let Some(tex) = video_tex {
            egui::Image::new(tex)
                .corner_radius(radius)
                .paint_at(ui, rect);

            let badge_w = 68.0;
            let badge_h = 18.0;
            let badge_rect = egui::Rect::from_min_size(
                egui::pos2(rect.max.x - badge_w - 6.0, rect.max.y - badge_h - 6.0),
                egui::vec2(badge_w, badge_h),
            );
            ui.painter().rect_filled(
                badge_rect,
                CornerRadius::same(5),
                Color32::from_black_alpha(190),
            );
            let icon_rect = egui::Rect::from_center_size(
                badge_rect.center() - egui::vec2(25.0, 0.0),
                egui::vec2(10.0, 10.0),
            );
            egui::Image::new(egui::include_image!("../../assets/icon_play.png"))
                .tint(Color32::WHITE)
                .paint_at(ui, icon_rect);
            ui.painter().text(
                badge_rect.center() + egui::vec2(6.0, 0.0),
                egui::Align2::CENTER_CENTER,
                "PREVIEW",
                egui::FontId::new(10.0, egui::FontFamily::Name("Bold".into())),
                Color32::WHITE,
            );
        } else {
            if let Some(u) = url.filter(|u| !u.is_empty()) {
                egui::Image::new(u.as_str())
                    .corner_radius(radius)
                    .paint_at(ui, rect);
            }
            if is_video_loading {
                let badge_w = 48.0;
                let badge_h = 14.0;
                let badge_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.max.x - badge_w - 5.0, rect.max.y - badge_h - 5.0),
                    egui::vec2(badge_w, badge_h),
                );
                ui.painter().rect_filled(
                    badge_rect,
                    CornerRadius::same(4),
                    Color32::from_black_alpha(175),
                );
                ui.painter().text(
                    badge_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Loading",
                    egui::FontId::new(8.5, egui::FontFamily::Proportional),
                    Color32::from_rgb(225, 225, 225),
                );
            }
        }
        if bordered {
            ui.painter().rect_stroke(
                rect,
                r,
                egui::Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
        }
        if resp.hovered() && video_tex.is_none() {
            ui.painter()
                .rect_filled(rect, r, Color32::from_black_alpha(24));
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn thumb_styled(
    ui: &mut egui::Ui,
    url: Option<&String>,
    size: Vec2,
    radius: u8,
    bordered: bool,
) -> egui::Response {
    thumb_styled_with_video(ui, url, None, false, size, radius, bordered)
}

pub fn thumb(ui: &mut egui::Ui, url: Option<&String>, size: Vec2, radius: u8) -> egui::Response {
    thumb_styled(ui, url, size, radius, false)
}

pub fn small_text(ui: &mut egui::Ui, text: impl AsRef<str>) {
    ui.add(
        egui::Label::new(
            RichText::new(text.as_ref())
                .font(crate::theme::font_meta())
                .color(crate::theme::tokens(ui).muted),
        )
        .truncate(),
    );
}

pub fn title_text(ui: &mut egui::Ui, text: &str, size: f32) -> egui::Response {
    let p = pal(ui);
    let font_id = egui::FontId::new(size, egui::FontFamily::Name("Bold".into()));
    let resp = ui
        .add(
            egui::Label::new(RichText::new(text).font(font_id).color(p.text).strong())
                .truncate()
                .sense(Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // The tooltip is the label text itself: only build it while the pointer is over the
    // label, instead of allocating a copy of every card title on every frame.
    if resp.hovered() {
        resp.on_hover_text(text)
    } else {
        resp
    }
}

pub fn game_stats_row(
    ui: &mut egui::Ui,
    g: &Game,
    icon_size: f32,
    text_size: f32,
    compact_stats: bool,
) {
    let p = pal(ui);
    let custom_bg = ui.visuals().panel_fill == Color32::TRANSPARENT;
    let dark = ui.visuals().dark_mode;
    let (stat_color, sub_color) = if custom_bg {
        (p.text, p.muted)
    } else if dark {
        (
            Color32::from_rgb(226, 231, 240),
            Color32::from_rgb(190, 198, 210),
        )
    } else {
        (Color32::from_rgb(45, 50, 58), Color32::from_rgb(80, 84, 92))
    };

    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.add(
            egui::Image::new(egui::include_image!("../../assets/icon_like.png"))
                .fit_to_exact_size(egui::vec2(icon_size, icon_size))
                .tint(stat_color),
        );
        // Tabular figures: the rating and player count must not shift the line as they
        // change while the feed scrolls.
        ui.label(
            RichText::new(g.rating())
                .font(crate::theme::font_tabular(text_size))
                .color(stat_color),
        );
        let tail = if g.playing > 0 {
            if compact_stats {
                format!("  {}", compact(g.playing))
            } else {
                format!("  {} playing", compact(g.playing))
            }
        } else if !compact_stats {
            "  Rating".to_string()
        } else {
            String::new()
        };
        if !tail.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(tail)
                        .font(crate::theme::font_meta())
                        .color(sub_color),
                )
                .truncate(),
            );
        }
    });
}

#[allow(dead_code)]
pub fn verified_badge(ui: &mut egui::Ui, size: f32) -> egui::Response {
    verified_badge_tip(ui, size, "Verified")
}

pub fn verified_badge_tip(ui: &mut egui::Ui, size: f32, tip: &str) -> egui::Response {
    let img = egui::Image::new(egui::include_image!("../../assets/icon_verified.png"))
        .fit_to_exact_size(egui::vec2(size, size));
    ui.add(img).on_hover_text(tip)
}

pub fn section(ui: &mut egui::Ui, text: &str) {
    crate::theme::section_header(ui, text);
}

pub fn section_link(ui: &mut egui::Ui, text: &str) -> bool {
    let t = crate::theme::tokens(ui);
    ui.add_space(crate::theme::SPACE_6);
    let resp = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let l = ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .font(crate::theme::font_section())
                        .color(t.text),
                )
                .sense(Sense::click()),
            );
            let arrow = egui::Image::new(egui::include_image!("../../assets/icon_arrow_right.png"))
                .fit_to_exact_size(egui::vec2(12.0, 12.0))
                .tint(t.muted);
            let a = ui.add(arrow.sense(Sense::click()));
            let clicked = l.clicked() || a.clicked();
            if l.hovered() || a.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            clicked
        })
        .inner;
    ui.add_space(crate::theme::SPACE_2);
    resp
}

pub fn sync_refresh_control(ui: &mut egui::Ui, syncing: bool, tip: &str, acts: &mut Vec<Action>) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let p = pal(ui);
        let btn = egui::Button::new(RichText::new("Refresh").size(13.0).color(p.text))
            .fill(p.surface_bg)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(16);
        if ui.add_enabled(!syncing, btn).on_hover_text(tip).clicked() {
            acts.push(Action::RefreshGames);
        }
        if syncing {
            ui.add(egui::Spinner::new().size(13.0));
        }
    });
}

/// A compact total-playtime pill, kept inside the thumbnail so it cannot affect card
/// layout or spill into neighbouring cards.
fn paint_playtime_badge(ui: &egui::Ui, rect: egui::Rect, seconds: u64) {
    let text = crate::playtime::format_duration(seconds);
    let font = egui::FontId::proportional(10.5);
    let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
    let pad_x = 5.0;
    let pad_y = 2.0;
    let badge_w = galley.size().x + pad_x * 2.0;
    // Very old games can have huge totals; skip rather than clip or overflow the image.
    if badge_w > rect.width() - 10.0 {
        return;
    }
    let badge_h = galley.size().y + pad_y * 2.0;
    let badge = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 5.0, rect.bottom() - badge_h - 5.0),
        egui::vec2(badge_w, badge_h),
    );
    let painter = ui.painter();
    painter.rect_filled(badge, CornerRadius::same(4), Color32::from_black_alpha(200));
    painter.galley(
        egui::pos2(
            badge.left() + pad_x,
            badge.center().y - galley.size().y * 0.5,
        ),
        galley,
        Color32::WHITE,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn game_card(
    ui: &mut egui::Ui,
    g: &Game,
    thumb_url: Option<&String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    thumb_size: Vec2,
    title_size: f32,
    badge_size: f32,
    stat_icon_size: f32,
    stat_text_size: f32,
    compact_stats: bool,
    acts: &mut Vec<Action>,
) {
    let w = thumb_size.x;
    // Title line + one metadata line, on the 4 px grid: no reserved empty space below.
    let card_h = thumb_size.y + 52.0;
    let hover_id = ui.scope_id().with(("game_card_hover", g.universe_id));
    let card_resp = ui.allocate_ui_with_layout(
        egui::vec2(w, card_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(w);
            ui.set_max_width(w);
            // Reset item spacing locally so card height stays deterministic regardless of grid row spacing
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);

            let (video_tex, is_loading) = match preview.filter(|p| p.universe_id == g.universe_id) {
                Some(p) => (p.texture, p.is_loading),
                None => (None, false),
            };

            let a =
                thumb_styled_with_video(ui, thumb_url, video_tex, is_loading, thumb_size, 8, false);
            let gp = pal(ui);
            let prev_hover = ui
                .ctx()
                .data(|d| d.get_temp::<bool>(hover_id))
                .unwrap_or(false);
            let hover_t = ui.ctx().animate_bool(hover_id.with("anim"), prev_hover);
            let border_col = lerp_color(gp.games_border, accent_color(ui), hover_t * 0.75);
            if hover_t > 0.01 {
                ui.painter().rect_filled(
                    a.rect,
                    CornerRadius::same(8),
                    Color32::from_white_alpha((hover_t * 8.0).round() as u8),
                );
            }
            // A hairline that changes colour on hover, never thickness: a growing stroke
            // reads as a glow and shifts the perceived edge.
            ui.painter().rect_stroke(
                a.rect,
                CornerRadius::same(8),
                egui::Stroke::new(1.0, border_col),
                egui::StrokeKind::Inside,
            );
            if let Some(seconds) = playtime
                .get(&g.place_id)
                .map(|p| p.seconds)
                .filter(|seconds| *seconds >= 60)
            {
                paint_playtime_badge(ui, a.rect, seconds);
            }
            ui.add_space(3.0);

            let max_title_w = if g.creator_verified {
                (w - badge_size - 6.0).max(40.0)
            } else {
                w
            };
            let b = ui
                .horizontal(|ui| {
                    ui.set_min_height(24.0);
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let t = ui
                        .scope(|ui| {
                            ui.set_max_width(max_title_w);
                            title_text(ui, &g.name, title_size)
                        })
                        .inner;
                    if g.creator_verified {
                        verified_badge_tip(ui, badge_size, "Verified Creator");
                    }
                    t
                })
                .inner;

            game_stats_row(ui, g, stat_icon_size, stat_text_size, compact_stats);

            (a.hovered() || b.hovered(), a.clicked() || b.clicked())
        },
    );

    let card_rect = card_resp.response.rect;
    let pointer_in_card = ui.rect_contains_pointer(card_rect);
    let is_hovered = pointer_in_card || card_resp.inner.0 || card_resp.response.hovered();
    let is_clicked =
        card_resp.inner.1 || (pointer_in_card && ui.input(|i| i.pointer.primary_clicked()));
    ui.ctx().data_mut(|d| {
        d.insert_temp(hover_id, is_hovered);
    });

    if is_hovered {
        acts.push(Action::HoverGame(g.universe_id));
    }
    if is_clicked {
        acts.push(Action::Open(g.clone()));
    }
}

pub fn wide_card(
    ui: &mut egui::Ui,
    g: &Game,
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    w: f32,
    acts: &mut Vec<Action>,
) {
    let h = (w * 9.0 / 16.0).floor();
    game_card(
        ui,
        g,
        wide.get(&g.universe_id),
        preview,
        playtime,
        egui::vec2(w, h),
        13.0,
        13.0,
        12.0,
        11.0,
        false,
        acts,
    );
}

pub fn icon_card(
    ui: &mut egui::Ui,
    g: &Game,
    icons: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    w: f32,
    acts: &mut Vec<Action>,
) {
    game_card(
        ui,
        g,
        icons.get(&g.universe_id),
        preview,
        playtime,
        egui::vec2(w, w),
        12.5,
        12.0,
        12.0,
        11.0,
        true,
        acts,
    );
}

pub fn grid_card_width(ui: &egui::Ui) -> f32 {
    // Never hand out a card wider than the page: the grid lives in a vertical-only
    // scroll area, so an over-wide card is clipped instead of scrolling.
    let avail = ui.available_width().max(150.0);
    ((avail - 5.0 * 14.0) / 6.0)
        .clamp(150.0, 280.0)
        .min(avail)
        .floor()
}

pub fn wide_grid_with_width(
    ui: &mut egui::Ui,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    w: f32,
    acts: &mut Vec<Action>,
) {
    let gap = 14.0;
    let avail = ui.available_width();
    let cols = (((avail + gap + 0.5) / (w + gap)).floor() as usize).max(1);
    // Every row has the same height, so remember it and reserve the exact same space for
    // rows that are scrolled out of view instead of building (and animating) their cards.
    // This is what keeps long feeds and the home page smooth while scrolling.
    let row_id = ui.scope_id().with(("wide_grid_row_h", cols, w.to_bits()));
    let cached_h = ui.ctx().data(|d| d.get_temp::<f32>(row_id));
    let clip = ui.clip_rect();
    let mut measured = cached_h;
    for (idx, row) in games.chunks(cols).enumerate() {
        if idx > 0 {
            ui.add_space(8.0);
        }
        let top = ui.cursor().top();
        if let Some(h) = cached_h {
            if top + h < clip.top() - 40.0 || top > clip.bottom() + 40.0 {
                ui.add_space(h);
                continue;
            }
        }
        // horizontal_top (Align::Min) keeps every card aligned cleanly along its top edge
        let r = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for g in row {
                wide_card(ui, g, wide, preview, playtime, w, acts);
            }
        });
        let h = r.response.rect.height();
        if h > 0.0 {
            measured = Some(h);
        }
    }
    if measured != cached_h {
        if let Some(h) = measured {
            ui.ctx().data_mut(|d| d.insert_temp(row_id, h));
        }
    }
}

pub fn wide_grid(
    ui: &mut egui::Ui,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) {
    let w = grid_card_width(ui);
    wide_grid_with_width(ui, games, wide, preview, playtime, w, acts);
}

/// Turns a vertical mouse wheel into sideways movement for the horizontal rows below.
///
/// egui only scrolls a horizontal-only area with a real trackpad swipe or shift+wheel, and
/// plenty of people have a plain mouse. This reads the wheel inside the visible part of the
/// row and scrolls that row instead, and swallows the delta so the page itself does not
/// jump at the same time.
pub fn wide_row(
    ui: &mut egui::Ui,
    id: &str,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) {
    let w = grid_card_width(ui);
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for g in games {
                    wide_card(ui, g, wide, preview, playtime, w, acts);
                }
            });
        });
}

pub fn icon_row(
    ui: &mut egui::Ui,
    id: &str,
    games: &[Game],
    icons: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) {
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for g in games {
                    icon_card(ui, g, icons, preview, playtime, 128.0, acts);
                }
            });
        });
}

// Round avatar with optional status dot and sub-label under it
pub fn bubble(
    ui: &mut egui::Ui,
    url: Option<&String>,
    label: &str,
    sub: &str,
    presence: u8,
    plus: bool,
    verified: bool,
) -> egui::Response {
    let p = pal(ui);
    let t = crate::theme::tokens(ui);
    let mut resp = None;
    ui.allocate_ui_with_layout(
        egui::vec2(72.0, 108.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.set_width(72.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            // 48 px presence avatar: compact enough for a strip, still a comfortable
            // click target and big enough to recognise a face.
            let r = thumb_styled(ui, url, egui::vec2(48.0, 48.0), 24, false);
            let hover_t = ui
                .ctx()
                .animate_bool(r.id.with("bubble_hover"), r.hovered());
            if plus {
                ui.painter()
                    .circle_filled(r.rect.center(), 23.5, p.friends_card_bg);
            } else if hover_t > 0.01 {
                ui.painter().circle_filled(
                    r.rect.center(),
                    23.5,
                    Color32::from_white_alpha((hover_t * 10.0).round() as u8),
                );
            }
            let ring_col = lerp_color(
                p.friends_border,
                if plus { t.accent } else { t.hairline_strong },
                hover_t * 0.75,
            );
            ui.painter().rect_stroke(
                r.rect,
                CornerRadius::same(24),
                egui::Stroke::new(1.0, ring_col),
                egui::StrokeKind::Inside,
            );
            if plus {
                let c = r.rect.center();
                let st = egui::Stroke::new(1.8, t.muted);
                ui.painter()
                    .line_segment([c - egui::vec2(7.0, 0.0), c + egui::vec2(7.0, 0.0)], st);
                ui.painter()
                    .line_segment([c - egui::vec2(0.0, 7.0), c + egui::vec2(0.0, 7.0)], st);
            } else if presence > 0 {
                // 5 px semantic dot with a 1 px canvas ring, never a coloured border.
                let dot_rgb = match presence {
                    2 => crate::theme::EMERALD_RGB,
                    3 => crate::theme::AMBER_RGB,
                    _ => (0x34, 0xD3, 0x99),
                };
                let center = r.rect.right_bottom() - egui::vec2(6.0, 6.0);
                ui.painter().circle_filled(center, 5.5, p.bg);
                crate::theme::paint_status_dot(ui.painter(), center, dot_rgb);
            }
            ui.add_space(crate::theme::SPACE_1);
            // Names are centred under the avatar: a truncated `Label` inside a horizontal
            // row fills the cell and left-aligns the text, which made names drift off to
            // the left of their circles. Painting the line ourselves keeps it centred.
            let mut name = if label.is_empty() {
                "Player".to_string()
            } else {
                label.to_string()
            };
            if verified {
                name.push_str(" ✓");
            }
            bubble_line(ui, &name, 12.0, p.text);
            if !sub.is_empty() {
                bubble_line(ui, sub, 10.5, p.muted);
            }
            resp = Some(r);
        },
    );
    resp.unwrap_or_else(|| ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()))
}

/// One centred, ellipsised line inside a friends/game bubble cell.
pub fn bubble_line(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    const MAX_W: f32 = 68.0;
    const CELL_W: f32 = 72.0;
    let font = egui::FontId::proportional(size);
    let mut galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), color);
    if galley.size().x > MAX_W {
        // Binary search the longest prefix that still fits with an ellipsis. The old loop
        // re-laid out the whole string once per removed character, every drawn frame.
        let chars: Vec<char> = text.chars().collect();
        let mut lo = 0usize;
        let mut hi = chars.len();
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let mut candidate: String = chars[..mid].iter().collect();
            candidate.push('…');
            let shortened = ui.painter().layout_no_wrap(candidate, font.clone(), color);
            if shortened.size().x <= MAX_W {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo > 0 {
            let mut candidate: String = chars[..lo].iter().collect();
            candidate.push('…');
            galley = ui.painter().layout_no_wrap(candidate, font.clone(), color);
        }
    }
    let height = galley.size().y.max(size + 2.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CELL_W, height), Sense::hover());
    let pos = egui::pos2(rect.center().x - galley.size().x * 0.5, rect.top());
    ui.painter().galley(pos, galley, color);
}

fn hex_points(center: egui::Pos2, radius: f32) -> Vec<egui::Pos2> {
    let mut pts = Vec::with_capacity(6);
    for i in 0..6 {
        let angle = -std::f32::consts::FRAC_PI_2 + (i as f32) * std::f32::consts::FRAC_PI_3;
        pts.push(egui::pos2(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    pts
}

/// The real Robux mark: a solid gold hexagon, a white inset gap, a gold inner plate and a
/// white square punched out of the middle.
pub fn draw_robux_icon(painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
    let r = size * 0.47;
    let white = Color32::WHITE;
    painter.add(egui::Shape::convex_polygon(
        hex_points(center, r),
        color,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        hex_points(center, r * 0.85),
        white,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        hex_points(center, r * 0.76),
        color,
        egui::Stroke::NONE,
    ));
    let square = (size * 0.30).max(2.0);
    painter.rect_filled(
        egui::Rect::from_center_size(center, egui::vec2(square, square)),
        (size * 0.03).max(0.5),
        white,
    );
}
