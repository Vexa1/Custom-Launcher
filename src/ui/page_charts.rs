use crate::app::Creblox;
use crate::theme::{RED, card_frame, pal};
use crate::types::{Action, Game, Msg};
use crate::ui::widgets::*;
use eframe::egui::{self, DragValue, RichText, Sense};

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

/// Short "1.2M" style number for the filter labels.
fn compact(n: u64) -> String {
    match n {
        0 => String::new(),
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// Filter boxes show nothing for zero (blank means "any") so typing starts cleanly
/// instead of inserting characters into a formatted word.
fn parse_filter_number(raw: &str) -> Option<f64> {
    let raw = raw.trim().replace(',', "");
    if raw.is_empty() || raw.eq_ignore_ascii_case("any") {
        Some(0.0)
    } else {
        raw.parse().ok()
    }
}

/// FNV-1a step, used to tell whether the feed cache is still valid.
fn mix(mut h: u64, v: u64) -> u64 {
    h ^= v;
    h.wrapping_mul(0x100_0000_01b3)
}

/// Width of a filter's value box; the rail takes whatever is left in its group.
const VALUE_BOX_W: f32 = 64.0;
/// Fixed caption column: keeps the three rails starting at the same offset in their group.
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

/// Draws the hint inside a value box that is showing nothing, so an unset filter still looks
/// like an editable field. It disappears as soon as the box has a value or the caret is in it.
fn hint_when_empty(ui: &egui::Ui, resp: &egui::Response, empty: bool, text: &str) {
    if !empty || resp.has_focus() {
        return;
    }
    let rect = resp.rect.shrink2(egui::vec2(7.0, 2.0));
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(10.5),
        pal(ui).muted,
    );
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
        ui.add_space(10.0);

        // Filters: one horizontal line, each with a bar and exactly one box for an exact
        // number.
        let mut min_players = self.chart_min_players;
        let mut min_rating = self.chart_min_rating;
        let mut min_visits = self.chart_min_visits;
        // Filters: one balanced line of three equal groups, with Clear pinned to the right.
        // Every group is label / 6 px / rail / 6 px / value box, so the three rails start and
        // end in the same place instead of drifting with the label widths.
        let any_filter = min_players > 0 || min_rating > 0 || min_visits > 0;
        card_frame(ui, 10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                let clear_w = if any_filter { 58.0 } else { 0.0 };
                let group_w = ((ui.available_width() - clear_w - 24.0) / 3.0)
                    .floor()
                    .max(228.0);
                let rail = rail_color(&p);

                filter_group(ui, group_w, |ui| {
                    filter_label(ui, "Players", LABEL_W);
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - VALUE_BOX_W - 6.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    ui.scope(|ui| {
                        ui.visuals_mut().widgets.inactive.bg_fill = rail;
                        ui.visuals_mut().widgets.hovered.bg_fill = rail;
                        ui.visuals_mut().widgets.active.bg_fill = rail;
                        ui.add(
                            egui::Slider::new(&mut min_players, 0..=100_000u32)
                                .step_by(50.0)
                                .show_value(false),
                        )
                        .on_hover_text("Minimum players in game");
                    });
                    ui.add_space(6.0);
                    let value = ui.add_sized(
                        [VALUE_BOX_W, 20.0],
                        DragValue::new(&mut min_players)
                            .speed(50.0)
                            .range(0..=u32::MAX)
                            .custom_formatter(|v, _| compact(v as u64))
                            .custom_parser(parse_filter_number),
                    );
                    hint_when_empty(ui, &value, min_players == 0, "0");
                    value.on_hover_text("Type an exact minimum number of players (blank = any)");
                });

                filter_group(ui, group_w, |ui| {
                    filter_label(ui, "Like %", LABEL_W);
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - VALUE_BOX_W - 6.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    ui.scope(|ui| {
                        ui.visuals_mut().widgets.inactive.bg_fill = rail;
                        ui.visuals_mut().widgets.hovered.bg_fill = rail;
                        ui.visuals_mut().widgets.active.bg_fill = rail;
                        ui.add(egui::Slider::new(&mut min_rating, 0..=100u8).show_value(false))
                            .on_hover_text("Minimum like ratio");
                    });
                    ui.add_space(6.0);
                    let value = ui.add_sized(
                        [VALUE_BOX_W, 20.0],
                        DragValue::new(&mut min_rating)
                            .speed(1.0)
                            .range(0..=100u8)
                            .custom_formatter(|v, _| {
                                if v < 0.5 {
                                    String::new()
                                } else {
                                    format!("{v:.0}%")
                                }
                            })
                            .custom_parser(parse_filter_number),
                    );
                    hint_when_empty(ui, &value, min_rating == 0, "Min");
                    value.on_hover_text("Type an exact minimum like ratio (blank = any)");
                });

                filter_group(ui, group_w, |ui| {
                    filter_label(ui, "Visits", LABEL_W);
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - VALUE_BOX_W - 6.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    ui.scope(|ui| {
                        ui.visuals_mut().widgets.inactive.bg_fill = rail;
                        ui.visuals_mut().widgets.hovered.bg_fill = rail;
                        ui.visuals_mut().widgets.active.bg_fill = rail;
                        ui.add(
                            egui::Slider::new(&mut min_visits, 0..=100_000_000u64)
                                .step_by(100_000.0)
                                .show_value(false),
                        )
                        .on_hover_text("Minimum visits");
                    });
                    ui.add_space(6.0);
                    let value = ui.add_sized(
                        [VALUE_BOX_W, 20.0],
                        DragValue::new(&mut min_visits)
                            .speed(100_000.0)
                            .range(0..=u64::MAX)
                            .custom_formatter(|v, _| compact(v as u64))
                            .custom_parser(parse_filter_number),
                    );
                    hint_when_empty(ui, &value, min_visits == 0, "0");
                    value.on_hover_text("Type an exact minimum number of visits (blank = any)");
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
                            .min_size(egui::vec2(58.0, 24.0));
                            if ui
                                .add(clear)
                                .on_hover_text("Clear all filters")
                                .clicked()
                            {
                                min_players = 0;
                                min_rating = 0;
                                min_visits = 0;
                            }
                        }
                    },
                );
            });
        });
        if self.chart_min_players != min_players {
            self.chart_min_players = min_players;
        }
        if self.chart_min_rating != min_rating {
            self.chart_min_rating = min_rating;
        }
        if self.chart_min_visits != min_visits {
            self.chart_min_visits = min_visits;
        }

        let min_players = self.chart_min_players;
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
        sig = mix(sig, min_rating as u64);
        sig = mix(sig, min_visits);
        sig = mix(sig, self.game_visits.len() as u64);
        if sig != self.feed_sig {
            self.feed_sig = sig;
            self.rebuild_feed_pool(min_players, min_rating, min_visits);
        }

        let matched = self.feed_pool.len();
        match &self.sorts {
            None => {
                ui.spinner();
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
    fn rebuild_feed_pool(&mut self, min_players: u32, min_rating: u8, min_visits: u64) {
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
                if g.playing < min_players as u64 {
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
