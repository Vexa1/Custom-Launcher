//! Party pill: one compact row, plus a small invite window. No extra windows while idle,
//! no per-frame work beyond what egui already paints for the page.

use crate::app::Creblox;
use crate::party::PartyStatus;
use crate::theme::{SPACE_2, pal};
use crate::ui::widgets::{small_text, thumb};
use eframe::egui::{self, RichText};

impl Creblox {
    /// Draw the party bar. Called from Home; cheap when there is no party (two buttons).
    pub fn party_bar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        egui::Frame::new()
            .fill(p.card_bg)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| match self.party_state.clone() {
                Some(party) => self.party_room_row(ui, &party, p.card_bg, p.muted, p.text),
                None => self.party_idle_row(ui, p.muted, p.text),
            });
        self.party_invite_window(ui.ctx());
    }

    fn party_idle_row(&mut self, ui: &mut egui::Ui, muted: egui::Color32, text: egui::Color32) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Party").strong().color(text));
            small_text(ui, "Play with friends in the same server");
            ui.add_space(SPACE_2);
            if ui.button("Create Party").clicked() {
                self.party_create();
            }
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.party_join_code)
                    .hint_text("CRB-...")
                    .desired_width(170.0),
            );
            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let joinable = !self.party_join_code.trim().is_empty();
            if (ui
                .add_enabled(joinable, egui::Button::new("Join"))
                .clicked()
                || submitted)
                && joinable
            {
                let code = self.party_join_code.clone();
                self.party_join(code);
            }
        });
        if let Some(err) = self.party_error.clone() {
            ui.label(
                RichText::new(err)
                    .size(11.5)
                    .color(crate::theme::text_ink(ui, crate::theme::RED)),
            );
        } else {
            ui.label(
                RichText::new(
                    "A party is a temporary room: it disappears when the last member leaves.",
                )
                .size(11.0)
                .color(muted),
            );
        }
    }

    fn party_room_row(
        &mut self,
        ui: &mut egui::Ui,
        party: &crate::party::PartySnapshot,
        _card: egui::Color32,
        muted: egui::Color32,
        text: egui::Color32,
    ) {
        let me = self.me.as_ref().map(|p| p.user.id).unwrap_or(0);
        let is_host = party.is_host(me);
        let host_target = party.host_target();
        let mut kick: Option<u64> = None;
        let mut promote: Option<u64> = None;
        let mut join_target: Option<(u64, String)> = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Party").strong().color(text));
            ui.label(
                RichText::new(&party.code)
                    .monospace()
                    .size(12.0)
                    .color(text),
            );
            if ui.small_button("Copy Invite").clicked() {
                self.party_copy_invite();
            }
            ui.label(
                RichText::new(format!("{}/{}", party.members.len(), party.max))
                    .size(11.5)
                    .color(muted),
            );
            if !party.connected {
                ui.label(RichText::new("reconnecting...").size(11.0).color(muted));
            }

            ui.add_space(SPACE_2);
            // Applied after the list: the menu closure must not borrow `self` while the rows
            // are being drawn.
            let mut open_inventory: Option<(u64, String)> = None;
            for m in &party.members {
                let is_me = m.user.id == me;
                let response = ui
                    .horizontal(|ui| {
                        thumb(ui, Some(&m.user.headshot_url), egui::vec2(18.0, 18.0), 9);
                        ui.label(RichText::new(m.user.label()).size(12.0).color(text));
                        if party.is_host(m.user.id) {
                            ui.label(RichText::new("*").size(12.0).color(muted))
                                .on_hover_text("Party host");
                        }
                        ui.label(status_dot(m.status, muted, text));
                    })
                    .response
                    .on_hover_text(format!("@{} - {}", m.user.name, m.status.label()));
                response.context_menu(|ui| {
                    if ui.button("View Inventory").clicked() {
                        open_inventory = Some((m.user.id, m.user.name.clone()));
                        ui.close();
                    }
                    if m.place_id > 0 && !m.job_id.is_empty() && !is_me {
                        if ui.button("Join their server").clicked() {
                            join_target = Some((m.place_id, m.job_id.clone()));
                            ui.close();
                        }
                    }
                    if is_host && !is_me {
                        if ui.button("Make host").clicked() {
                            promote = Some(m.user.id);
                            ui.close();
                        }
                        if ui.button("Remove from party").clicked() {
                            kick = Some(m.user.id);
                            ui.close();
                        }
                    }
                });
            }

            if let Some((user_id, name)) = open_inventory {
                self.inventory = Some(crate::ui::page_inventory::InventoryView::new(
                    user_id,
                    name,
                    false,
                    self.cookie.clone(),
                ));
            }

            ui.add_space(SPACE_2);
            if !is_host {
                if let Some((place, job)) = host_target.clone() {
                    if ui.button("Join Party's Server").clicked() {
                        join_target = Some((place, job));
                    }
                }
            }
            if ui.small_button("Leave").clicked() {
                self.party_leave();
            }
        });

        self.party_board_row(ui, party, me, is_host, text);
        self.party_slot_row(ui, is_host, muted);

        if let Some(err) = self.party_error.clone() {
            ui.label(
                RichText::new(err)
                    .size(11.5)
                    .color(crate::theme::text_ink(ui, crate::theme::RED)),
            );
        }

        if let Some(id) = kick {
            self.party.kick(id);
        }
        if let Some(id) = promote {
            self.party.promote(id);
        }
        if let Some((place, job)) = join_target {
            self.launch_server(place, job);
        }
    }

    /// The late-joiner slot check: the server the party is heading to has no seat for this
    /// member. The host can move the party to a server with room; anybody else asks.
    fn party_slot_row(&mut self, ui: &mut egui::Ui, is_host: bool, muted: egui::Color32) {
        let full = self.party_full.clone();
        let asked = self.party_hop_from.clone();
        if full.is_none() && asked.is_none() {
            return;
        }
        let mut hop = false;
        let mut ask = false;
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if let Some((launch, free)) = &full {
                let game = if launch.game_name.trim().is_empty() {
                    "The party's server".to_string()
                } else {
                    format!("{}'s server", short(&launch.game_name))
                };
                let detail = if *free == 0 {
                    "is full".to_string()
                } else {
                    format!("has {free} free slot(s)")
                };
                ui.label(
                    RichText::new(format!("{game} {detail} - no room for you."))
                        .size(11.5)
                        .color(muted),
                );
            } else if let Some(name) = &asked {
                ui.label(
                    RichText::new(format!("{name} cannot join: the party's server is full."))
                        .size(11.5)
                        .color(muted),
                );
            }
            if is_host {
                if ui
                    .button("Hop party to a server with room")
                    .on_hover_text("Only servers with a seat per member are pickable")
                    .clicked()
                {
                    hop = true;
                }
            } else if full.is_some()
                && ui
                    .button("Ask host to hop")
                    .on_hover_text("The host picks a server with room for everybody")
                    .clicked()
            {
                ask = true;
            }
        });
        if hop {
            self.hop_to_server(crate::server_hop::ServerHopMode::LeastPlayers);
        } else if ask {
            self.party.request_hop();
            self.status = "Asked the host to hop the party".to_string();
        }
    }

    /// Ready Check and Game Vote: one wrapped row under the member strip. Nothing is drawn
    /// at all when there is no check, no poll and no game page open, so a plain party keeps
    /// its old height.
    fn party_board_row(
        &mut self,
        ui: &mut egui::Ui,
        party: &crate::party::PartySnapshot,
        me: u64,
        is_host: bool,
        text: egui::Color32,
    ) {
        let board = &party.board;
        let open_game = match &self.page {
            crate::types::Page::Game(g) => Some((g.place_id, g.universe_id, g.name.clone())),
            _ => None,
        };
        let can_suggest = open_game.as_ref().is_some_and(|(place, _, _)| {
            board.suggestions.len() < crate::party::PartyBoard::MAX_SUGGESTIONS
                && !board.suggestions.iter().any(|s| s.place_id == *place)
        });
        if !board.ready_check_active() && board.suggestions.is_empty() && !can_suggest {
            return;
        }
        let mut start = false;
        let mut answer: Option<bool> = None;
        let mut vote: Option<u64> = None;
        let mut launch: Option<(u64, u64, String)> = None;

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if is_host {
                if ui
                    .add_enabled(
                        !board.ready_check_active(),
                        egui::Button::new("Ready Check"),
                    )
                    .on_hover_text("Ask everyone to confirm they are ready (10 s)")
                    .clicked()
                {
                    start = true;
                }
            }
            if board.ready_check_active() {
                let (ready, answered) = board.ready_count();
                let secs = board.ready_seconds_left(now_ms());
                ui.label(
                    RichText::new(format!("{ready}/{answered} ready - {secs}s"))
                        .size(11.5)
                        .color(text),
                );
                let mine = board.is_ready(me).unwrap_or(false);
                if ui.selectable_label(mine, "Ready").clicked() {
                    answer = Some(true);
                }
                if ui.selectable_label(!mine, "Not Ready").clicked() {
                    answer = Some(false);
                }
            }

            if !board.suggestions.is_empty() {
                if board.ready_check_active() || is_host {
                    ui.separator();
                }
                for suggestion in &board.suggestions {
                    let votes = board.votes_for(suggestion.place_id);
                    let mine = board.my_vote(me) == Some(suggestion.place_id);
                    if ui
                        .selectable_label(mine, format!("{} ({votes})", short(&suggestion.name)))
                        .on_hover_text("Vote to play this with the party")
                        .clicked()
                    {
                        vote = Some(suggestion.place_id);
                    }
                }
                if is_host {
                    if let Some(winner) = board.winner() {
                        if ui.button(format!("Launch {}", short(&winner.name))).clicked() {
                            launch = Some((
                                winner.place_id,
                                winner.universe_id,
                                winner.name.clone(),
                            ));
                        }
                    }
                }
            }

            // Any member can put the game they are looking at on the poll.
            if let (Some((place_id, universe_id, name)), true) = (open_game, can_suggest) {
                if ui.small_button("Suggest this game").clicked() {
                    self.party.suggest_game(place_id, universe_id, name);
                }
            }
        });

        if start {
            self.party.start_ready_check();
        }
        if let Some(ready) = answer {
            self.party.set_ready(ready);
        }
        if let Some(place_id) = vote {
            self.party.vote(place_id);
        }
        if let Some((place_id, universe_id, name)) = launch {
            self.party_launch(place_id, universe_id, name);
        }
    }

    fn party_invite_window(&mut self, ctx: &egui::Context) {
        if !self.party_invite_open {
            return;
        }
        let Some((short, full)) = self
            .party_state
            .as_ref()
            .map(|p| (p.code.clone(), p.invite.clone()))
        else {
            self.party_invite_open = false;
            return;
        };
        let mut open = true;
        let mut copy = false;
        // While the host is still asking its router for a port there is no reachable
        // endpoint yet, so the window shows the room label and waits.
        let invite = if full.is_empty() { short.clone() } else { full };
        let link = format!("creblox://party/{invite}");
        egui::Window::new("Invite to party")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Send this code, or the link below, to a friend who uses CREBLOX.");
                ui.add_space(SPACE_2);
                ui.label(
                    RichText::new(&short)
                        .monospace()
                        .strong()
                        .size(20.0),
                );
                ui.label(RichText::new(&invite).monospace().size(12.0));
                ui.label(RichText::new(&link).monospace().size(12.0));
                ui.add_space(SPACE_2);
                if ui.button("Copy invite").clicked() {
                    copy = true;
                }
            });
        if copy {
            self.party_copy_invite();
        }
        self.party_invite_open = open;
    }
}

/// Keeps a game name from pushing the board row past the edge of the bar.
fn short(name: &str) -> String {
    const MAX: usize = 16;
    if name.chars().count() <= MAX {
        return name.to_string();
    }
    let mut out: String = name.chars().take(MAX - 3).collect();
    out.push_str("...");
    out
}

/// Wall-clock milliseconds, matching the deadline the host puts in the board.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn status_dot(status: PartyStatus, muted: egui::Color32, text: egui::Color32) -> RichText {
    let (glyph, color) = match status {
        PartyStatus::Ready => ("-", muted),
        PartyStatus::Launching => (">", text),
        PartyStatus::InGame => ("#", text),
    };
    RichText::new(glyph).size(11.0).color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_a_marker() {
        for status in [
            PartyStatus::Ready,
            PartyStatus::Launching,
            PartyStatus::InGame,
        ] {
            assert!(
                !status_dot(status, egui::Color32::WHITE, egui::Color32::BLACK)
                    .text()
                    .is_empty()
            );
        }
    }
}
