use crate::app::Creblox;
use crate::theme::{RED, pal};
use crate::types::Action;
use crate::ui::widgets::*;
use eframe::egui::{self, RichText, Sense};

impl Creblox {
    pub fn page_home(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        // Friends
        let count = self
            .me
            .as_ref()
            .map(|p| p.friends)
            .unwrap_or(self.friends.len() as u64);
        ui.add_space(crate::theme::SPACE_4);
        // Party first: it is the control you reach for before picking a game, and it is a
        // single row when there is no party.
        self.party_bar(ui);
        ui.add_space(crate::theme::SPACE_4);
        // Page header (20 px semibold) with the count in tabular figures, so it does not
        // nudge the title while friends come online.
        ui.horizontal(|ui| {
            crate::theme::page_header(ui, "Friends");
            ui.label(
                RichText::new(count.to_string())
                    .font(crate::theme::font_tabular(crate::theme::TEXT_META))
                    .color(crate::theme::tokens(ui).muted),
            );
        });
        ui.add_space(crate::theme::SPACE_3);

        egui::ScrollArea::horizontal()
            .id_salt("friends")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::theme::SPACE_3;
                    let clip = ui.clip_rect();
                    let add_bubble = bubble(ui, None, "Add Friend", "", 0, true, false)
                        .on_hover_text("Search and add friends on Roblox");
                    if add_bubble.clicked() {
                        acts.push(Action::OpenAddFriend);
                    }
                    for f in &self.friends {
                        // A head that is scrolled out of the strip still costs a text layout
                        // and an avatar texture request every frame. Reserve its cell and
                        // skip it, the same way the game rows cull their cards.
                        let left = ui.cursor().min.x;
                        if left >= clip.right() || left + BUBBLE_W <= clip.left() {
                            ui.allocate_exact_size(egui::vec2(BUBBLE_W, BUBBLE_H), Sense::hover());
                            continue;
                        }
                        let base_sub = if f.presence == 2 && !f.location.is_empty() {
                            f.location.as_str()
                        } else if f.presence == 1 {
                            "Online"
                        } else if f.presence == 3 {
                            "In Studio"
                        } else {
                            ""
                        };
                        // A friend who is already in this party is marked, so the strip says
                        // where they are before you try to invite them again.
                        let in_party = self
                            .party_state
                            .as_ref()
                            .is_some_and(|p| p.members.iter().any(|m| m.user.id == f.id));
                        // Allocated only when the friend really is in the party; the plain
                        // case borrows, because this runs per bubble, per frame.
                        let in_party_sub;
                        let sub: &str = if in_party {
                            in_party_sub = format!("In party - {base_sub}");
                            in_party_sub.as_str()
                        } else {
                            base_sub
                        };
                        // displayName, else the username, else "Player" - and only while the
                        // batch name lookup is still in flight, never as a resting state.
                        let label = crate::friend_names::label(&f.display, &f.name);
                        let mut r = bubble(
                            ui,
                            Some(&f.headshot),
                            label,
                            sub,
                            f.presence,
                            false,
                            f.verified,
                        );
                        // Build the tooltip text only while the pointer is on the bubble:
                        // with a long friends list this used to allocate one formatted
                        // string per friend on every frame.
                        if r.hovered() {
                            // The username is what the profile opens on, so show it when it is
                            // known; a display-only friend still gets a real tooltip.
                            let handle = if f.name.is_empty() { label } else { f.name.as_str() };
                            let tip = if f.name.is_empty() && f.display.is_empty() {
                                format!("User #{}", f.id)
                            } else if f.place_id.is_some() {
                                format!("@{} \u{2014} Playing {}", handle, f.location)
                            } else {
                                format!("@{}", handle)
                            };
                            r = r.on_hover_text(tip);
                        }
                        if r.hovered() {
                            // One action per frame; the app debounces it before warming the
                            // profile, so a sweep across the strip starts nothing.
                            acts.push(Action::HoverFriend(f.id));
                        }
                        if r.clicked() {
                            acts.push(Action::OpenFriend(f.clone()));
                        }
                        r.context_menu(|ui| {
                            if ui.button("View profile").clicked() {
                                acts.push(Action::OpenFriend(f.clone()));
                                ui.close();
                            }
                            if ui.button("View Inventory").clicked() {
                                acts.push(Action::ViewInventory(f.id, f.name.clone()));
                                ui.close();
                            }
                            if let Some(pid) = f.place_id {
                                if ui.button("Join game").clicked() {
                                    acts.push(Action::JoinPlace(pid));
                                    ui.close();
                                }
                            }
                            // Only offered inside a party: the invite is that room's code.
                            if self.party_state.is_some() {
                                if ui.button("Invite to Party").clicked() {
                                    acts.push(Action::InviteToParty(f.clone()));
                                    ui.close();
                                }
                            }
                        });
                    }
                    if self.friends.is_empty() {
                        ui.label(RichText::new("No friends loaded yet.").color(pal(ui).muted));
                    }
                });
            });
        if let Some(e) = &self.friends_err {
            ui.colored_label(crate::theme::text_ink(ui, RED), format!("Could not load friends: {e}"));
        }

        // Continue playing: always above the game sections so the last games you played
        // are one click away without scrolling.
        if !self.recent.is_empty() {
            section(ui, "Continue");
            icon_row(
                ui,
                "continue",
                &self.recent,
                &self.icon,
                self.current_card_preview(),
                &self.playtime,
                acts,
            );
        }

        // Explore sorts
        let mut retry = false;
        match &self.sorts {
            None => {
                ui.add_space(20.0);
                // A skeleton is only honest when nothing has ever loaded; a refresh over a
                // populated page never reaches this arm.
                if self.charts_freshness.skeleton(0) {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading games from Roblox\u{2026}");
                    });
                }
            }
            Some(Err(e)) => {
                ui.add_space(20.0);
                ui.colored_label(crate::theme::text_ink(ui, RED), format!("Could not reach Roblox: {e}"));
                retry = ui.button("Retry").clicked();
            }
            Some(Ok(sorts)) => {
                if let Some(first) = sorts.first() {
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let link = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(format!("{}  \u{203A}", first.title))
                                        .font(crate::theme::font_section())
                                        .color(crate::theme::tokens(ui).text),
                                )
                                .sense(Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if link.clicked() {
                            acts.push(Action::GoChart(0));
                        }
                        // The button never greys out and no hint appears beside it: a
                        // background refresh must not add or remove a row, because that is
                        // what would shift the grid below it.
                        sync_refresh_control(
                            ui,
                            "Topics & played games auto-sync in background (click to sync now)",
                            acts,
                        );
                    });
                    ui.add_space(6.0);
                    wide_grid(
                        ui,
                        &first.games,
                        &self.wide,
                        self.current_card_preview(),
                        &self.playtime,
                        acts,
                    );
                }
            }
        }
        if retry {
            self.sorts = None;
            self.force_refresh_games();
        }

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let p = pal(ui);
            ui.add(
                egui::Image::new(egui::include_image!("../../assets/icon_bookmark.png"))
                    .fit_to_exact_size(egui::vec2(15.0, 15.0))
                    .tint(p.text),
            );
            ui.label(
                RichText::new("Favorites")
                    .font(crate::theme::font_page())
                    .color(crate::theme::tokens(ui).text),
            );
            ui.add_space(8.0);
            // Shared outlined pill: 6 px radius, hairline border, tabular label.
            if crate::theme::value_pill(ui, "+ Add URL", false)
                .on_hover_text("Open, play, or pin a Roblox game manually by URL link or Place ID")
                .clicked()
            {
                acts.push(Action::OpenManualGame);
            }
        });
        ui.add_space(8.0);
        if self.favs.is_empty() {
            ui.label(
                RichText::new("No favorites yet — click + Add URL to pin a game link here.")
                    .size(13.0)
                    .color(pal(ui).muted),
            );
        } else {
            icon_row(
                ui,
                "favorites",
                &self.favs,
                &self.icon,
                self.current_card_preview(),
                &self.playtime,
                acts,
            );
        }

        if let Some(Ok(sorts)) = &self.sorts {
            for (i, s) in sorts.iter().enumerate().skip(1) {
                if section_link(ui, &s.title) {
                    acts.push(Action::GoChart(i));
                }
                wide_row(
                    ui,
                    ("sort_row", i),
                    &s.games,
                    &self.wide,
                    self.current_card_preview(),
                    &self.playtime,
                    acts,
                );
            }
        }
        ui.add_space(20.0);
    }
}
