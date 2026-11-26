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

/// A filter caption painted in a box the same height as a slider row, so its baseline
/// lines up with the handle and the value box instead of sitting a few pixels low.
fn filter_label(ui: &mut egui::Ui, text: &str) {
    let color = pal(ui).muted;
    let font = egui::FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            galley.size().x.ceil(),
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
                    self.feed_rows = 30;
                }
            });
        });
        ui.add_space(10.0);

        // Filters: one horizontal line, each with a bar and exactly one box for an exact
        // number.
        let mut min_players = self.chart_min_players;
        let mut min_rating = self.chart_min_rating;
        let mut min_visits = self.chart_min_visits;
        card_frame(ui, 10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                // One unbreakable group per filter: label, rail and value box always wrap
                // together, and every value box is the same width so the three groups line
                // up instead of looking ragged.
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = 132.0;
                    filter_label(ui, "Players");
                    ui.add(
                        egui::Slider::new(&mut min_players, 0..=100_000u32)
                            .step_by(50.0)
                            .show_value(false),
                    )
                    .on_hover_text("Minimum players in game");
                    ui.add_sized(
                        [64.0, 20.0],
                        DragValue::new(&mut min_players)
                            .speed(50.0)
                            .range(0..=u32::MAX)
                            .custom_formatter(|v, _| compact(v as u64))
                            .custom_parser(parse_filter_number),
                    )
                    .on_hover_text("Type an exact minimum number of players (blank = any)");
                });
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = 132.0;
                    filter_label(ui, "Like %");
                    ui.add(egui::Slider::new(&mut min_rating, 0..=100u8).show_value(false))
                        .on_hover_text("Minimum like ratio");
                    ui.add_sized(
                        [64.0, 20.0],
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
                    )
                    .on_hover_text("Type an exact minimum like ratio (blank = any)");
                });
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = 132.0;
                    filter_label(ui, "Visits");
                    ui.add(
                        egui::Slider::new(&mut min_visits, 0..=100_000_000u64)
                            .step_by(100_000.0)
                            .show_value(false),
                    )
                    .on_hover_text("Minimum visits");
                    ui.add_sized(
                        [64.0, 20.0],
                        DragValue::new(&mut min_visits)
                            .speed(100_000.0)
                            .range(0..=u64::MAX)
                            .custom_formatter(|v, _| compact(v as u64))
                            .custom_parser(parse_filter_number),
                    )
                    .on_hover_text("Type an exact minimum number of visits (blank = any)");
                });

                if min_players > 0 || min_rating > 0 || min_visits > 0 {
                    ui.add_space(4.0);
                    if ui.small_button("Clear").clicked() {
                        min_players = 0;
                        min_rating = 0;
                        min_visits = 0;
                    }
                }
            });
        });
        if self.chart_min_players != min_players {
            self.chart_min_players = min_players;
            self.feed_rows = 30;
        }
        if self.chart_min_rating != min_rating {
            self.chart_min_rating = min_rating;
            self.feed_rows = 30;
        }
        if self.chart_min_visits != min_visits {
            self.chart_min_visits = min_visits;
            self.feed_rows = 30;
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
            self.feed_rows = self.feed_rows.max(30);
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
                    let shown_n = self.feed_rows.min(matched);
                    let preview = self.current_card_preview();
                    wide_grid(
                        ui,
                        &self.feed_pool[..shown_n],
                        &self.wide,
                        preview,
                        &self.playtime,
                        acts,
                    );
                    ui.add_space(6.0);
                    if shown_n < matched {
                        small_text(
                            ui,
                            format!(
                                "{shown_n} of {matched} games \u{2014} keep scrolling for more"
                            ),
                        );
                        let (sentinel, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 1.0), Sense::hover());
                        // Grow only while the end of the feed is on screen. This used to
                        // also require a wheel delta in that exact frame, which never
                        // happens once a trackpad fling has settled - the feed stalled.
                        if ui.is_rect_visible(sentinel) {
                            self.feed_rows = (self.feed_rows + 24).min(matched);
                            ui.ctx().request_repaint();
                        }
                    } else {
                        small_text(ui, format!("All {matched} games loaded"));
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
