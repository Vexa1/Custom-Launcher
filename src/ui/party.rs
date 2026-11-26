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
            ui.label(RichText::new(err).size(11.5).color(crate::theme::RED));
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

        if let Some(err) = self.party_error.clone() {
            ui.label(RichText::new(err).size(11.5).color(crate::theme::RED));
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
