use crate::theme::{accent_color, card_frame, compact, lerp_color, pal};
use crate::types::{Action, CardPreview, Game};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2};
use std::collections::HashMap;

/// Id of the per-frame "the page is being flung" flag published by `Creblox::ui`.
pub fn fast_scroll_id() -> egui::Id {
    egui::Id::NULL.with("creblox_fast_scroll")
}

/// True while the page is moving fast (see `crate::scroll`).
///
/// A fling drags cards under a resting pointer, so during one every card looks hovered for
/// a frame. Card code asks this before animating a hover, building a tooltip or reporting
/// `Action::HoverGame`, which is what keeps a fast scroll from doing preview work per card.
pub fn fast_scrolling(ui: &egui::Ui) -> bool {
    ui.ctx()
        .data(|d| d.get_temp::<bool>(fast_scroll_id()))
        .unwrap_or(false)
}

/// Where the downloaded thumbnail textures live between the app and the cards.
///
/// The app uploads decoded pixels a few per frame and publishes the whole map here once it
/// has changed; a card looks its image URL up and paints that texture. Keying by URL (not
/// by universe id) means one entry serves both the wide cover and the icon of a card.
pub type ThumbTextures = std::sync::Arc<std::collections::HashMap<String, egui::TextureHandle>>;

fn thumb_tex_id() -> egui::Id {
    egui::Id::NULL.with("creblox-thumb-textures")
}

fn thumb_owned_id() -> egui::Id {
    egui::Id::NULL.with("creblox-thumb-owned")
}

pub fn publish_thumb_textures(ctx: &egui::Context, textures: ThumbTextures) {
    ctx.data_mut(|d| d.insert_temp(thumb_tex_id(), textures));
}

pub fn thumb_texture(ctx: &egui::Context, url: &str) -> Option<egui::TextureHandle> {
    ctx.data(|d| {
        d.get_temp::<ThumbTextures>(thumb_tex_id())
            .and_then(|map| map.get(url).cloned())
    })
}

/// Publishes the set of image URLs the launcher downloads itself.
///
/// A URL in this set is never handed back to egui's image loader: the card shows its
/// placeholder until our texture arrives, so nothing is fetched twice.
pub fn publish_thumb_owned(
    ctx: &egui::Context,
    owned: std::sync::Arc<std::collections::HashSet<String>>,
) {
    ctx.data_mut(|d| d.insert_temp(thumb_owned_id(), owned));
}

fn thumb_owned(ctx: &egui::Context, url: &str) -> bool {
    ctx.data(|d| {
        d.get_temp::<std::sync::Arc<std::collections::HashSet<String>>>(thumb_owned_id())
            .is_some_and(|set| set.contains(url))
    })
}

/// A quiet placeholder for a value that has not arrived yet.
///
/// It pulses very slowly with the frame clock, so a profile that is still filling in reads
/// as "on its way" rather than as an empty field, and it keeps asking for frames while it
/// is on screen (nothing else on the page is animating at that point).
pub fn skeleton(ui: &mut egui::Ui, size: Vec2, radius: u8) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = pal(ui);
    let t = ui.input(|i| i.time) as f32;
    let pulse = 0.5 + 0.5 * (t * 2.0).sin();
    let fill = p.profile_border.linear_multiply(0.45 + 0.3 * pulse);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius), fill);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(40));
    resp
}

/// A skeleton row of cards, for a section whose items have not arrived yet.
pub fn skeleton_cards(ui: &mut egui::Ui, count: usize, size: Vec2) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for _ in 0..count {
            skeleton(ui, size, 12);
        }
    });
}

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
                match thumb_texture(ui.ctx(), u) {
                    // Downloaded, decoded and uploaded by our own worker pool.
                    Some(tex) => {
                        egui::Image::new(&tex)
                            .corner_radius(radius)
                            .paint_at(ui, rect);
                    }
                    // Ours, on its way: keep the placeholder. Falling through to egui's
                    // loader here would download the same image a second time.
                    None if thumb_owned(ui.ctx(), u) => {}
                    // Not ours (a page that still uses the URL path): let egui load it.
                    None => {
                        egui::Image::new(u.as_str())
                            .corner_radius(radius)
                            .paint_at(ui, rect);
                    }
                }
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
        // Pre-formatted when the game arrived; the fallback covers hand-built games.
        let rating = if g.rating_text.is_empty() {
            g.rating()
        } else {
            g.rating_text.clone()
        };
        ui.label(
            RichText::new(rating)
                .font(crate::theme::font_tabular(text_size))
                .color(stat_color),
        );
        let count = if g.players_text.is_empty() {
            compact(g.playing)
        } else {
            g.players_text.clone()
        };
        let tail = if g.playing > 0 {
            if compact_stats {
                format!("  {count}")
            } else {
                format!("  {count} playing")
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
    // Read the fling flag once for the whole card: both the hover animation and the hover
    // report below need it, and this runs for every card on screen.
    let flinging = fast_scrolling(ui);
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
            // No hover animation at all while a fling is in progress: each one would keep
            // requesting repaints for a card the pointer only crossed.
            let hover_t = if flinging {
                0.0
            } else {
                ui.ctx().animate_bool(hover_id.with("anim"), prev_hover)
            };
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
    // While the page is flung, a card passing under the pointer is not being hovered: the
    // user is scrolling past it. Reporting it would start a preview lookup per card.
    let is_hovered =
        (pointer_in_card || card_resp.inner.0 || card_resp.response.hovered()) && !flinging;
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

/// What a drawn grid reports back to its page.
#[derive(Clone, Copy, Debug, Default)]
pub struct GridView {
    /// The grid's end is within about two rows of the viewport: fetch the next page now.
    pub near_end: bool,
}

pub fn wide_grid_with_width(
    ui: &mut egui::Ui,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    w: f32,
    acts: &mut Vec<Action>,
) -> GridView {
    let gap = 14.0;
    let avail = ui.available_width();
    let cols = (((avail + gap + 0.5) / (w + gap)).floor() as usize).max(1);
    // Every row has the same height, so remember it and reserve the exact same space for
    // rows that are scrolled out of view instead of building (and animating) their cards.
    // This is what keeps long feeds and the home page smooth while scrolling.
    let row_id = ui.scope_id().with(("wide_grid_row_h", cols, w.to_bits()));
    let cached_h = ui.ctx().data(|d| d.get_temp::<f32>(row_id));
    let clip = ui.clip_rect();
    let gap = 8.0;
    // Before the first row has been measured, use the card's known image height plus its text
    // rows: an unmeasured grid must not build every card in a 10,000-game feed on its first
    // frame. The estimate is replaced by the measured height immediately after.
    let row_h = cached_h.unwrap_or((w * 9.0 / 16.0).floor() + 52.0);
    let top_row = ui.cursor().top();
    let row_count = games.chunks(cols).count();
    let visible = crate::chart_feed::visible_rows(
        clip.top(),
        clip.bottom(),
        top_row,
        row_h,
        gap,
        row_count,
    );
    let mut measured = cached_h;
    for (idx, row) in games.chunks(cols).enumerate() {
        if idx > 0 {
            ui.add_space(gap);
        }
        if !visible.contains(&idx) {
            ui.add_space(row_h);
            continue;
        }
        // A row inside the viewport is what the user is looking at; the rows either side
        // are the lookahead band. Both are reported, and the queue serves the first lane
        // first.
        let row_top = ui.cursor().top();
        let on_screen = row_top < clip.bottom() && row_top + row_h > clip.top();
        for g in row {
            acts.push(Action::ThumbNeeded(g.universe_id, on_screen));
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
    // The cursor now sits at the bottom of the grid's content. When that is no further than a
    // couple of rows below the viewport, the next page is worth starting now.
    let row_h = measured.or(cached_h).unwrap_or(0.0);
    let content_bottom = ui.cursor().top();
    GridView {
        near_end: row_h > 0.0 && content_bottom <= clip.bottom() + row_h * 2.5,
    }
}

pub fn wide_grid(
    ui: &mut egui::Ui,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) -> GridView {
    let w = grid_card_width(ui);
    wide_grid_with_width(ui, games, wide, preview, playtime, w, acts)
}

/// A grid that also says whether its end is close enough to prefetch the next page.
pub fn wide_grid_info(
    ui: &mut egui::Ui,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) -> GridView {
    wide_grid(ui, games, wide, preview, playtime, acts)
}

/// Turns a vertical mouse wheel into sideways movement for the horizontal rows below.
///
/// egui only scrolls a horizontal-only area with a real trackpad swipe or shift+wheel, and
/// plenty of people have a plain mouse. This reads the wheel inside the visible part of the
/// row and scrolls that row instead, and swallows the delta so the page itself does not
/// jump at the same time.
pub fn wide_row(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    games: &[Game],
    wide: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) {
    let w = grid_card_width(ui);
    // Card height is deterministic (see `game_card`), so a card scrolled out of view can be
    // reserved as a plain rectangle. Laying it out instead costs a title + stats text layout
    // and a thumbnail request per off-screen card, on every frame - which is exactly what
    // made these rows drag compared with the grid on the charts page.
    let card_h = (w * 9.0 / 16.0).floor() + 52.0;
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let clip = ui.clip_rect();
                for g in games {
                    let left = ui.cursor().min.x;
                    if left >= clip.right() || left + w <= clip.left() {
                        ui.allocate_exact_size(egui::vec2(w, card_h), Sense::hover());
                        continue;
                    }
                    wide_card(ui, g, wide, preview, playtime, w, acts);
                }
            });
        });
}

pub fn icon_row(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    games: &[Game],
    icons: &HashMap<u64, String>,
    preview: Option<CardPreview<'_>>,
    playtime: &HashMap<u64, crate::playtime::GamePlay>,
    acts: &mut Vec<Action>,
) {
    // Same rule as `wide_row`: 128 px square card plus the 52 px title/stats block.
    let card_h = 128.0 + 52.0;
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let clip = ui.clip_rect();
                for g in games {
                    let left = ui.cursor().min.x;
                    if left >= clip.right() || left + 128.0 <= clip.left() {
                        ui.allocate_exact_size(egui::vec2(128.0, card_h), Sense::hover());
                        continue;
                    }
                    icon_card(ui, g, icons, preview, playtime, 128.0, acts);
                }
            });
        });
}

/// Friends strip cell: one presence avatar over two centred text lines.
pub const BUBBLE_W: f32 = 108.0;
pub const BUBBLE_H: f32 = 148.0;
/// Avatar diameter inside a friends bubble.
pub const BUBBLE_AVATAR: f32 = 96.0;

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
        egui::vec2(BUBBLE_W, BUBBLE_H),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.set_width(BUBBLE_W);
            ui.spacing_mut().item_spacing.y = 3.0;
            // 96 px presence avatar: a recognisable face and a comfortable click target.
            // The 0.4.26 token pass halved this to 48, which read as a thumbnail, not a
            // friend.
            let r = thumb_styled(ui, url, egui::vec2(BUBBLE_AVATAR, BUBBLE_AVATAR), 48, false);
            let hover_t = ui
                .ctx()
                .animate_bool(r.id.with("bubble_hover"), r.hovered());
            if plus {
                ui.painter()
                    .circle_filled(r.rect.center(), 46.5, p.friends_card_bg);
            } else if hover_t > 0.01 {
                ui.painter().circle_filled(
                    r.rect.center(),
                    46.5,
                    Color32::from_white_alpha((hover_t * 14.0).round() as u8),
                );
            }
            let ring_col = lerp_color(
                p.friends_border,
                if plus { t.accent } else { t.hairline_strong },
                hover_t * 0.75,
            );
            ui.painter().rect_stroke(
                r.rect,
                CornerRadius::same(48),
                egui::Stroke::new(1.5 + hover_t, ring_col),
                egui::StrokeKind::Inside,
            );
            if plus {
                let c = r.rect.center();
                let st = egui::Stroke::new(2.8, t.muted);
                ui.painter()
                    .line_segment([c - egui::vec2(13.0, 0.0), c + egui::vec2(13.0, 0.0)], st);
                ui.painter()
                    .line_segment([c - egui::vec2(0.0, 13.0), c + egui::vec2(0.0, 13.0)], st);
            } else if presence > 0 {
                // 16 px semantic dot with a 2 px canvas ring, never a coloured border.
                let dot_rgb = match presence {
                    2 => crate::theme::EMERALD_RGB,
                    3 => crate::theme::AMBER_RGB,
                    _ => (0x34, 0xD3, 0x99),
                };
                let center = r.rect.right_bottom() - egui::vec2(12.0, 12.0);
                ui.painter().circle_filled(center, 11.0, p.bg);
                ui.painter().circle_filled(
                    center,
                    8.0,
                    Color32::from_rgb(dot_rgb.0, dot_rgb.1, dot_rgb.2),
                );
            }
            ui.add_space(crate::theme::SPACE_1);
            // Names are centred under the avatar: a truncated `Label` inside a horizontal
            // row fills the cell and left-aligns the text, which made names drift off to
            // the left of their circles. Painting the line ourselves keeps it centred.
            // No copy of the name unless the verified mark has to be appended: this runs
            // for every bubble, every frame.
            let name = if label.is_empty() { "Player" } else { label };
            if verified {
                let mut marked = String::with_capacity(name.len() + 3);
                marked.push_str(name);
                marked.push_str(" ✓");
                bubble_line(ui, &marked, 13.0, p.text);
            } else {
                bubble_line(ui, name, 13.0, p.text);
            }
            if !sub.is_empty() {
                bubble_line(ui, sub, 11.5, p.muted);
            }
            resp = Some(r);
        },
    );
    resp.unwrap_or_else(|| ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()))
}

/// One centred, ellipsised line inside a friends/game bubble cell.
pub fn bubble_line(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    const MAX_W: f32 = BUBBLE_W - 4.0;
    const CELL_W: f32 = BUBBLE_W;
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
