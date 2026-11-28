use crate::app::Creblox;
use crate::theme::{RED, card_frame, pal};
use crate::types::{Action, Game, Msg};
use crate::ui::widgets::*;
use eframe::egui::{self, RichText, Sense};

/// Stable pseudo-random ordering. The same salt and id always sort the same way, so the
/// feed does not jump around while the user scrolls; "Shuffle" just changes the salt.
fn feed_key(id: u64, salt: u64) -> u64 {
    let mut x = id ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// FNV-1a step, used to tell whether the feed cache is still valid.
fn mix(mut h: u64, v: u64) -> u64 {
    h ^= v;
    h.wrapping_mul(0x100_0000_01b3)
}

/// Width of a filter pill (the rounded box you type in) and its height.
const VALUE_BOX_W: f32 = 64.0;
const PILL_H: f32 = 26.0;
/// What the rails span. Only the thumb is bounded by these; the filters keep the typed value.
const PLAYERS_RAIL_MAX: u64 = 100_000;
const VISITS_RAIL_MAX: u64 = 100_000_000;
/// The en dash between the two player pills, the gap between filter groups, and Clear's width.
const DASH_W: f32 = 14.0;
const GAP: f32 = 8.0;
const CLEAR_W: f32 = 58.0;
/// Fixed caption column: keeps the rails starting at the same offset in their group.
const LABEL_W: f32 = 52.0;

/// One filter of the bar: a fixed slice of the row, its children laid out left to right.
fn filter_group(ui: &mut egui::Ui, width: f32, content: impl FnOnce(&mut egui::Ui)) {
    let height = ui.spacing().interact_size.y.max(22.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            // The bar sets its own 6 px gaps, so the default spacing would double them.
            ui.spacing_mut().item_spacing.x = 0.0;
            content(ui);
        },
    );
}

/// The slider trough. `widget_inactive` is almost the card colour, which made the rails
/// disappear against the filter bar; this is a step lighter so the track reads at a glance.
fn rail_color(p: &crate::theme::Palette) -> egui::Color32 {
    let base = p.widget_hovered;
    egui::Color32::from_rgb(
        base.r().saturating_add(16),
        base.g().saturating_add(16),
        base.b().saturating_add(18),
    )
}

/// A filter caption painted in a box the same height as a slider row, so its baseline
/// lines up with the handle and the value box instead of sitting a few pixels low. The box is
/// a fixed width (`LABEL_W`) so all three rails are the same length, whatever the text.
fn filter_label(ui: &mut egui::Ui, text: &str, width: f32) {
    let color = pal(ui).muted;
    let font = egui::FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            width.max(galley.size().x.ceil()),
            ui.spacing().interact_size.y.max(20.0),
        ),
        Sense::hover(),
    );
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, color);
}

/// The en dash between the minimum and maximum pills, centred in its own slice so the two
/// pills keep a fixed distance whatever the group width.
fn filter_sep(ui: &mut egui::Ui) {
    let color = pal(ui).muted;
    let galley = ui
        .painter()
        .layout_no_wrap("\u{2013}".to_string(), egui::FontId::proportional(12.0), color);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(DASH_W, ui.spacing().interact_size.y.max(20.0)),
        Sense::hover(),
    );
    let pos = egui::pos2(
        rect.center().x - galley.size().x * 0.5,
        rect.center().y - galley.size().y * 0.5,
    );
    ui.painter().galley(pos, galley, color);
}

/// One filter pill: a rounded dark box with its value (or its hint) centred horizontally *and*
/// vertically. The pill paints its own frame and hosts a frameless text box inside a centred,
/// justified child, so a two-character value sits in the middle of the box instead of the
/// top-left corner. The buffer is the caller's `String`, so typing is never fought by a redraw.
fn filter_pill(ui: &mut egui::Ui, text: &mut String, hint: &str, hover: &str) -> egui::Response {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(VALUE_BOX_W, PILL_H), Sense::hover());
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(9),
        p.surface_bg,
        egui::Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    // 6 px of padding left and right, 1 px top and bottom: the child gets the rest.
    let inner = rect.shrink2(egui::vec2(6.0, 1.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::LeftToRight,
            )),
    );
    let resp = child.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            // One align for both axes: the value sits dead centre of the pill.
            .align(egui::Align2::CENTER_CENTER)
            .margin(egui::Margin::ZERO)
            .desired_width(inner.width()),
    );
    resp.on_hover_text(hover)
}

impl Creblox {
    /// A continuous feed of every game CREBLOX knows about. It keeps pulling more games
    /// as you scroll, so it never runs out of page before the catalogue does.
    pub fn page_charts(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let p = pal(ui);
        let syncing = Self::is_sync_active(&self.sorts_sync_started);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new("Games").size(26.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let refresh = egui::Button::new(RichText::new("Refresh").size(13.0).color(p.text))
                    .fill(p.surface_bg)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(16)
                    .min_size(egui::vec2(84.0, 28.0));
                if ui
                    .add_enabled(!syncing, refresh)
                    .on_hover_text("Games auto-sync in background (click to sync now)")
                    .clicked()
                {
                    acts.push(Action::RefreshGames);
                }
                if syncing {
                    ui.add(egui::Spinner::new().size(13.0));
                }
                // Fixed width for both captions: the button keeps its exact place when it
                // changes between "Shuffle" and "Reshuffle".
                let shuffle = egui::Button::new(
                    RichText::new(if self.feed_shuffled {
                        "Reshuffle"
                    } else {
                        "Shuffle"
                    })
                    .size(13.0)
                    .color(p.text),
                )
                .fill(p.surface_bg)
                .stroke(egui::Stroke::new(1.0, p.border))
                .corner_radius(16)
                .min_size(egui::vec2(98.0, 28.0));
                if ui
                    .add(shuffle)
                    .on_hover_text("Show the games in a different random order")
                    .clicked()
                {
                    // Only ever on demand: the feed keeps its natural order by players
                    // until this is pressed.
                    self.feed_shuffled = true;
                    self.feed_salt = self.feed_salt.wrapping_add(0x9E37_79B9_7F4A_7C15);
                }
            });
        });
        if self.charts_freshness.is_silent() {
            small_text(ui, "Updating players and ranks\u{2026}");
        }
        ui.add_space(10.0);

        // Filters: Players is one wide group - a minimum rail, then a minimum pill and a
        // maximum pill - followed by Like % and Visits as single slots, with Clear pinned to
        // the right while any filter is set. Each pill's buffer is app state: it is never
        // rebuilt from the number while the caret is in it, which is what makes typing a
        // value stick.
        let mut min_players = self.chart_min_players;
        let mut min_rating = self.chart_min_rating;
        let mut min_visits = self.chart_min_visits;
        let mut max_players = self.chart_max_players;
        let any_filter =
            min_players > 0 || max_players.is_some() || min_rating > 0 || min_visits > 0;
        let mut players_box = std::mem::take(&mut self.chart_players_box);
        let mut max_players_box = std::mem::take(&mut self.chart_max_players_box);
        let mut like_box = std::mem::take(&mut self.chart_like_box);
        let mut visits_box = std::mem::take(&mut self.chart_visits_box);
        card_frame(ui, 10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                let clear_w = if any_filter { CLEAR_W } else { 0.0 };
                // Four slots, and Players owns two of them so its two pills fit beside the
                // rail instead of squeezing into one.
                let slot = ((ui.available_width() - clear_w - GAP * 4.0) / 4.0)
                    .floor()
                    .max(138.0);
                let players_w = slot * 2.0 + GAP;
                let rail = rail_color(&p);

                filter_group(ui, players_w, |ui| {
                    filter_label(ui, "Players", LABEL_W);
                    ui.add_space(6.0);
                    let rail_w =
                        (ui.available_width() - VALUE_BOX_W * 2.0 - DASH_W - 18.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    // The rail is a copy: egui clamps and snaps whatever value it is given,
                    // which would otherwise rewrite a typed `10` to 0 and `500K` to 100K.
                    let mut rail_pos =
                        crate::chart_feed::rail_position(min_players, PLAYERS_RAIL_MAX);
                    let slider_changed = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = rail;
                            ui.visuals_mut().widgets.hovered.bg_fill = rail;
                            ui.visuals_mut().widgets.active.bg_fill = rail;
                            ui.add(
                                egui::Slider::new(&mut rail_pos, 0.0..=PLAYERS_RAIL_MAX as f64)
                                    .step_by(50.0)
                                    .show_value(false),
                            )
                            .on_hover_text("Minimum players in game")
                        })
                        .inner
                        .changed();
                    if slider_changed {
                        min_players = rail_pos as u64;
                    }
                    ui.add_space(6.0);
                    let edit = filter_pill(
                        ui,
                        &mut players_box.text,
                        "Min",
                        "Type an exact minimum number of players (blank = any)",
                    );
                    // Type-to-filter: the parsed number applies immediately, the characters
                    // stay exactly as typed.
                    if edit.changed() {
                        if let Some(v) = players_box.edited_value() {
                            min_players = v;
                        }
                    }
                    if slider_changed {
                        players_box.sync_from_slider(min_players as u64, edit.has_focus());
                    }
                    if edit.lost_focus() {
                        players_box.normalize(min_players as u64);
                    }
                    ui.add_space(6.0);
                    filter_sep(ui);
                    ui.add_space(6.0);
                    let max_edit = filter_pill(
                        ui,
                        &mut max_players_box.text,
                        "Max",
                        "Type an exact maximum number of players (blank or 0 = no limit)",
                    );
                    // An empty box, \"Max\" or 0 means no upper limit at all.
                    if max_edit.changed() {
                        if let Some(ceiling) = max_players_box.edited_ceiling() {
                            max_players = ceiling;
                        }
                    }
                    if max_edit.lost_focus() {
                        match max_players {
                            Some(v) => max_players_box.normalize(v as u64),
                            None => max_players_box.text.clear(),
                        }
                    }
                });

                filter_group(ui, slot, |ui| {
                    filter_label(ui, "Like %", LABEL_W);
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - VALUE_BOX_W - 6.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    let slider_changed = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = rail;
                            ui.visuals_mut().widgets.hovered.bg_fill = rail;
                            ui.visuals_mut().widgets.active.bg_fill = rail;
                            ui.add(egui::Slider::new(&mut min_rating, 0..=100u8).show_value(false))
                                .on_hover_text("Minimum like ratio")
                        })
                        .inner
                        .changed();
                    ui.add_space(6.0);
                    let edit = filter_pill(
                        ui,
                        &mut like_box.text,
                        "Min",
                        "Type an exact minimum like ratio (blank = any)",
                    );
                    if edit.changed() {
                        if let Some(v) = like_box.edited_value() {
                            min_rating = v.min(100) as u8;
                        }
                    }
                    if slider_changed {
                        like_box.sync_from_slider(min_rating as u64, edit.has_focus());
                    }
                    if edit.lost_focus() {
                        like_box.normalize(min_rating as u64);
                    }
                });

                filter_group(ui, slot, |ui| {
                    filter_label(ui, "Visits", LABEL_W);
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - VALUE_BOX_W - 6.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    // Same display-only copy as the Players rail: a typed `55` stays 55.
                    let mut rail_pos =
                        crate::chart_feed::rail_position(min_visits, VISITS_RAIL_MAX);
                    let slider_changed = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = rail;
                            ui.visuals_mut().widgets.hovered.bg_fill = rail;
                            ui.visuals_mut().widgets.active.bg_fill = rail;
                            ui.add(
                                egui::Slider::new(&mut rail_pos, 0.0..=VISITS_RAIL_MAX as f64)
                                    .step_by(100_000.0)
                                    .show_value(false),
                            )
                            .on_hover_text("Minimum visits")
                        })
                        .inner
                        .changed();
                    if slider_changed {
                        min_visits = rail_pos as u64;
                    }
                    ui.add_space(6.0);
                    let edit = filter_pill(
                        ui,
                        &mut visits_box.text,
                        "Min",
                        "Type an exact minimum number of visits (blank = any)",
                    );
                    if edit.changed() {
                        if let Some(v) = visits_box.edited_value() {
                            min_visits = v;
                        }
                    }
                    if slider_changed {
                        visits_box.sync_from_slider(min_visits, edit.has_focus());
                    }
                    if edit.lost_focus() {
                        visits_box.normalize(min_visits);
                    }
                });

                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if any_filter {
                            let clear = egui::Button::new(
                                RichText::new("Clear").size(12.0).color(p.muted),
                            )
                            .fill(p.surface_bg)
                            .stroke(egui::Stroke::new(1.0, p.border))
                            .corner_radius(12)
                            .min_size(egui::vec2(CLEAR_W, 24.0));
                            if ui.add(clear).on_hover_text("Clear all filters").clicked() {
                                min_players = 0;
                                max_players = None;
                                min_rating = 0;
                                min_visits = 0;
                                players_box.normalize(0);
                                max_players_box.text.clear();
                                like_box.normalize(0);
                                visits_box.normalize(0);
                            }
                        }
                    },
                );
            });
        });
        self.chart_players_box = players_box;
        self.chart_max_players_box = max_players_box;
        self.chart_like_box = like_box;
        self.chart_visits_box = visits_box;
        if self.chart_min_players != min_players {
            self.chart_min_players = min_players;
        }
        if self.chart_max_players != max_players {
            self.chart_max_players = max_players;
        }
        if self.chart_min_rating != min_rating {
            self.chart_min_rating = min_rating;
        }
        if self.chart_min_visits != min_visits {
            self.chart_min_visits = min_visits;
        }

        let min_players = self.chart_min_players;
        let max_players = self.chart_max_players;
        let min_rating = self.chart_min_rating;
        let min_visits = self.chart_min_visits;
        ui.add_space(10.0);

        let mut total = 0usize;
        let mut first_id = 0u64;
        let mut last_id = 0u64;
        if let Some(Ok(sorts)) = &self.sorts {
            for sort in sorts {
                for game in &sort.games {
                    if total == 0 {
                        first_id = game.universe_id;
                    }
                    last_id = game.universe_id;
                    total += 1;
                }
            }
        }

        // Rebuild the filtered, ordered pool only when an input actually changed. This
        // used to sort and clone the whole catalogue on every drawn frame.
        let mut sig = 0xcbf2_9ce4_8422_2325u64;
        sig = mix(sig, total as u64);
        sig = mix(sig, first_id);
        sig = mix(sig, last_id);
        sig = mix(sig, self.feed_salt);
        sig = mix(sig, self.feed_shuffled as u64);
        sig = mix(sig, min_players as u64);
        // 0 already means "no ceiling" in the hash, so a set ceiling is biased by one.
        sig = mix(sig, max_players.map_or(0, |v| v.wrapping_add(1)));
        sig = mix(sig, min_rating as u64);
        sig = mix(sig, min_visits);
        sig = mix(sig, self.game_visits.len() as u64);
        if sig != self.feed_sig {
            self.feed_sig = sig;
            self.rebuild_feed_pool(min_players, max_players, min_rating, min_visits);
        }

        let matched = self.feed_pool.len();
        match &self.sorts {
            None => {
                if self.charts_freshness.skeleton(0) {
                    ui.spinner();
                }
            }
            Some(Err(e)) => {
                ui.colored_label(RED, e);
            }
            Some(Ok(_)) => {
                if matched == 0 {
                    ui.label(
                        RichText::new("No games match those filters yet.")
                            .size(13.0)
                            .color(p.muted),
                    );
                } else {
                    // The grid lays out every row's height but only builds the cards that are
                    // in the viewport, so 100 games and 10,000 games cost the same per frame.
                    let grid = {
                        let preview = self.current_card_preview();
                        wide_grid_info(
                            ui,
                            &self.feed_pool,
                            &self.wide,
                            preview,
                            &self.playtime,
                            acts,
                        )
                    };
                    ui.add_space(6.0);

                    let in_flight = self.chart_feed.in_flight;
                    let error = self.chart_feed.error.clone();
                    let wait = self.chart_feed.wait_left(std::time::Instant::now());
                    let more = self.chart_feed.next.is_some();
                    let loaded = self.chart_feed.loaded.max(matched);
                    let mut retry = false;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if in_flight {
                            // A quiet skeleton row: the page is still usable while it loads.
                            ui.add(egui::Spinner::new().size(12.0));
                            small_text(ui, "Loading more games\u{2026}");
                        } else if let Some(error) = &error {
                            let note = if wait > 0 {
                                format!("{error} ({wait}s)")
                            } else {
                                error.clone()
                            };
                            small_text(ui, note);
                            if ui
                                .link(RichText::new("Retry").size(12.0).color(p.text))
                                .clicked()
                            {
                                retry = true;
                            }
                        } else if more {
                            small_text(ui, format!("{loaded} games loaded \u{2014} keep scrolling"));
                        } else {
                            small_text(ui, format!("End of chart \u{00b7} {loaded} games"));
                        }
                    });
                    if retry {
                        acts.push(Action::ChartRetry);
                    }

                    // Two rows early, so the next page is usually in place before its cards
                    // could be seen. `request_chart_more` drops the call when a page is
                    // already in flight or the cursor is exhausted.
                    if grid.near_end {
                        self.request_chart_more();
                    }
                }
                if min_visits > 0 && self.game_visits.len() < total {
                    small_text(
                        ui,
                        format!(
                            "Visit counts loading\u{2026} {}/{} known",
                            self.game_visits.len().min(total),
                            total
                        ),
                    );
                }
            }
        }

        ui.add_space(20.0);
    }

    /// Deduplicates every chart into one pool, applies the filters, orders it and stores
    /// it. Results are cached in `feed_pool` so drawing is a plain slice read.
    fn rebuild_feed_pool(
        &mut self,
        min_players: u64,
        max_players: Option<u64>,
        min_rating: u8,
        min_visits: u64,
    ) {
        let mut pool: Vec<Game> = Vec::new();
        let mut missing_visits: Vec<u64> = Vec::new();
        if let Some(Ok(sorts)) = &self.sorts {
            let mut refs: Vec<&Game> = Vec::new();
            let mut seen = std::collections::HashSet::with_capacity(
                sorts.iter().map(|s| s.games.len()).sum::<usize>(),
            );
            for sort in sorts {
                for game in &sort.games {
                    if seen.insert(game.universe_id) {
                        refs.push(game);
                    }
                }
            }
            if self.feed_shuffled {
                let salt = self.feed_salt;
                refs.sort_unstable_by_key(|g| feed_key(g.universe_id, salt));
            } else {
                refs.sort_unstable_by(|a, b| b.playing.cmp(&a.playing));
            }
            refs.retain(|g| {
                if !crate::chart_feed::in_player_range(g.playing, min_players, max_players) {
                    return false;
                }
                let votes = g.up + g.down;
                if votes > 0 && (g.up * 100 / votes) < min_rating as u64 {
                    return false;
                }
                if min_visits > 0
                    && self.game_visits.get(&g.universe_id).copied().unwrap_or(0) < min_visits
                {
                    return false;
                }
                true
            });
            for g in &refs {
                if !self.game_visits.contains_key(&g.universe_id)
                    && !self.visits_requested.contains(&g.universe_id)
                {
                    missing_visits.push(g.universe_id);
                    if missing_visits.len() >= 150 {
                        break;
                    }
                }
            }
            pool.reserve(refs.len());
            pool.extend(refs.into_iter().cloned());
        }
        self.feed_pool = pool;

        // Visit counts are fetched in the background, one batch per rebuild.
        if !missing_visits.is_empty() {
            for id in &missing_visits {
                self.visits_requested.insert(*id);
            }
            self.spawn(move || Msg::GameVisits(crate::api::fetch_game_visits(&missing_visits)));
        }
    }
}
