//! The screens the headless renderer draws.
//!
//! These mirror the shipped pages — `src/ui/page_home.rs`, `page_charts.rs`, `page_game.rs`,
//! `ui/party.rs`, `page_settings.rs`, `updater.rs` and `src/game_overlay.rs` — at the level the
//! Linux VM can run: the palette, spacing, corner radii and widget shapes come from
//! `src/theme.rs`, but the *page functions themselves* need `crate::app::Creblox`, eframe and the
//! OS APIs, so the GUI crate cannot be linked on this host. A change to a page's layout should be
//! mirrored here so the screenshot test still shows what the page looks like.

use egui::{Color32, CornerRadius, Stroke, Vec2};

use crate::shots::Probe;

// Palette: `Palette::of(true)` in `src/theme.rs` (Precision Obsidian).
pub const BG: Color32 = Color32::from_rgb(0x0B, 0x0D, 0x11);
pub const CARD: Color32 = Color32::from_rgb(0x13, 0x16, 0x1E);
pub const SURFACE: Color32 = Color32::from_rgb(0x19, 0x1D, 0x27);
pub const HOVER: Color32 = Color32::from_rgb(0x23, 0x28, 0x38);
pub const SELECTED: Color32 = Color32::from_rgb(0x1E, 0x22, 0x30);
pub const TEXT: Color32 = Color32::from_rgb(0xF1, 0xF3, 0xF6);
pub const MUTED: Color32 = Color32::from_rgb(0x8E, 0x93, 0x9E);
/// `theme.rs` hairlines are white at 6%; opaque here so the rasterizer has no extra blending to
/// hide behind.
pub const BORDER: Color32 = Color32::from_rgb(0x2A, 0x2F, 0x3C);
// Accents: `GREEN`/`BLUE`/`ORANGE`/`RED` in `src/theme.rs`.
pub const GREEN: Color32 = Color32::from_rgb(0x00, 0xB0, 0x6F);
pub const BLUE: Color32 = Color32::from_rgb(0x00, 0xA2, 0xFF);
pub const ORANGE: Color32 = Color32::from_rgb(0xF6, 0x88, 0x02);
pub const RED: Color32 = Color32::from_rgb(0xD2, 0x32, 0x32);

const RADIUS: u8 = 8;

/// Every rendered screen, in the order the gallery is written.
pub fn all() -> Vec<(&'static str, fn(&mut egui::Ui, &mut Probe))> {
    vec![
        ("home", home),
        ("charts", charts),
        ("game", game_details),
        ("party", party),
        ("settings", settings),
        ("updater", updater),
        ("overlay", overlay),
    ]
}

fn card(
    ui: &mut egui::Ui,
    probe: &mut Probe,
    id: impl Into<String>,
    width: f32,
    content: impl FnOnce(&mut egui::Ui, &mut Probe),
) {
    card_min(ui, probe, id, width, 0.0, content);
}

/// A card with a floor on its height, so the cards of one row share a bottom edge.
fn card_min(
    ui: &mut egui::Ui,
    probe: &mut Probe,
    id: impl Into<String>,
    width: f32,
    min_height: f32,
    content: impl FnOnce(&mut egui::Ui, &mut Probe),
) {
    let frame = egui::Frame::NONE
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(14);
    let inner = frame.show(ui, |ui| {
        ui.set_width(width);
        if min_height > 0.0 {
            ui.set_min_height(min_height);
        }
        // Frame::show inherits the parent's layout; a card is always a column, even inside a
        // row of cards.
        ui.vertical(|ui| content(ui, probe));
    });
    probe.widget(id, inner.response.rect, false);
}

fn heading(ui: &mut egui::Ui, probe: &mut Probe, id: &'static str, text: &str, size: f32) {
    probe.label(
        ui,
        id,
        text,
        egui::FontId::proportional(size),
        TEXT,
        BG,
    );
}

fn body(ui: &mut egui::Ui, probe: &mut Probe, id: impl Into<String>, text: &str, color: Color32) {
    probe.label(
        ui,
        id,
        text,
        egui::FontId::proportional(13.0),
        color,
        CARD,
    );
}

fn meta(ui: &mut egui::Ui, probe: &mut Probe, id: impl Into<String>, text: &str) {
    probe.label(
        ui,
        id,
        text,
        egui::FontId::proportional(11.0),
        MUTED,
        CARD,
    );
}

/// Left rail + padded content column, the shell every page shares.
fn shell(
    ui: &mut egui::Ui,
    probe: &mut Probe,
    page_id: &'static str,
    content: impl FnOnce(&mut egui::Ui, &mut Probe),
) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                rail(ui, probe);
                ui.add_space(20.0);
                ui.vertical(|ui| content(ui, probe));
            });
        });
    let _ = page_id;
}

fn rail(ui: &mut egui::Ui, probe: &mut Probe) {
    egui::Frame::NONE
        .fill(CARD)
        .inner_margin(egui::Margin::symmetric(14, 18))
        .show(ui, |ui| {
            ui.set_width(148.0);
            ui.vertical(|ui| {
                probe.label(
                    ui,
                    "rail.logo",
                    "CREBLOX",
                    egui::FontId::proportional(15.0),
                    GREEN,
                    CARD,
                );
                ui.add_space(18.0);
                for (id, label, selected) in [
                    ("rail.home", "Home", true),
                    ("rail.charts", "Charts", false),
                    ("rail.party", "Party", false),
                    ("rail.settings", "Settings", false),
                ] {
                    probe.button(
                        ui,
                        id,
                        label,
                        Vec2::new(148.0, 34.0),
                        if selected { SELECTED } else { CARD },
                        if selected { TEXT } else { MUTED },
                    );
                    ui.add_space(4.0);
                }
            });
        });
}

fn avatar(ui: &mut egui::Ui, probe: &mut Probe, id: &'static str, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
    probe.widget(id, rect, false);
}

fn stat_chip(ui: &mut egui::Ui, probe: &mut Probe, id: &str, value: &str, label: &str) {
    egui::Frame::NONE
        .fill(SURFACE)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
            ui.vertical(|ui| {
                probe.label(
                    ui,
                    format!("{id}.value"),
                    value,
                    egui::FontId::proportional(15.0),
                    TEXT,
                    SURFACE,
                );
                probe.label(
                    ui,
                    format!("{id}.label"),
                    label,
                    egui::FontId::proportional(11.0),
                    MUTED,
                    SURFACE,
                );
            });
        });
}

pub fn home(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "home", |ui, probe| {
        heading(ui, probe, "home.title", "Home", 22.0);
        ui.add_space(10.0);

        card(ui, probe, "home.profile", 980.0, |ui, probe| {
            ui.horizontal(|ui| {
                avatar(ui, probe, "home.profile.avatar", 56.0, HOVER);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    probe.label(
                        ui,
                        "home.profile.name",
                        "Vexa",
                        egui::FontId::proportional(17.0),
                        TEXT,
                        CARD,
                    );
                    body(ui, probe, "home.profile.status", "Playing Adopt Me!", GREEN);
                });
                ui.add_space(24.0);
                for (id, value, label) in [
                    ("home.profile.robux", "12,480", "Robux"),
                    ("home.profile.friends", "132", "Friends"),
                    ("home.profile.days", "18", "Day streak"),
                ] {
                    stat_chip(ui, probe, id, value, label);
                }
            });
        });

        ui.add_space(16.0);
        heading(ui, probe, "home.friends.title", "Friends online", 15.0);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (i, (name, game)) in [
                ("Mika", "Blox Fruits"),
                ("Rin", "Tower of Hell"),
                ("Sol", "Adopt Me!"),
                ("Kai", "BedWars"),
            ]
            .iter()
            .enumerate()
            {
                let id = ["home.friend.1", "home.friend.2", "home.friend.3", "home.friend.4"][i];
                card(ui, probe, id, 206.0, |ui, probe| {
                    ui.horizontal(|ui| {
                        avatar(ui, probe, id, 32.0, HOVER);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            body(ui, probe, id, name, TEXT);
                            meta(ui, probe, id, game);
                        });
                    });
                });
            }
        });

        ui.add_space(16.0);
        heading(ui, probe, "home.games.title", "Jump back in", 15.0);
        ui.add_space(6.0);
        for row in 0..2 {
            ui.horizontal(|ui| {
                for col in 0..3 {
                    let (title, creator, playing) = [
                        ("Adopt Me!", "Uplift Games", "312K"),
                        ("Blox Fruits", "Gamer Robot", "188K"),
                        ("Tower of Hell", "YXCeptional", "96K"),
                        ("BedWars", "Easy.gg", "74K"),
                        ("Doors", "LSPLASH", "51K"),
                        ("Arsenal", "ROLVe", "38K"),
                    ][row * 3 + col];
                    let id = format!("home.game.{row}{col}");
                    card(ui, probe, id.clone(), 300.0, |ui, probe| {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 84.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, CornerRadius::same(6), SURFACE);
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            title.chars().next().unwrap_or('?').to_string(),
                            egui::FontId::proportional(28.0),
                            MUTED,
                        );
                        probe.widget(format!("{id}.thumb"), rect, false);
                        ui.add_space(8.0);
                        body(ui, probe, format!("{id}.title"), title, TEXT);
                        meta(ui, probe, format!("{id}.meta"), creator);
                        body(
                            ui,
                            probe,
                            format!("{id}.playing"),
                            &format!("{playing} playing"),
                            GREEN,
                        );
                    });
                }
            });
            ui.add_space(10.0);
        }
    });
}

pub fn charts(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "charts", |ui, probe| {
        heading(ui, probe, "charts.title", "Charts", 22.0);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (id, label, active) in [
                ("charts.tab.playing", "Top playing", true),
                ("charts.tab.rated", "Top rated", false),
                ("charts.tab.fav", "Favorites", false),
            ] {
                probe.button(
                    ui,
                    id,
                    label,
                    Vec2::new(118.0, 32.0),
                    if active { SELECTED } else { CARD },
                    if active { TEXT } else { MUTED },
                );
            }
        });
        ui.add_space(14.0);
        card(ui, probe, "charts.list", 980.0, |ui, probe| {
            for (i, (name, creator, count, frac)) in [
                ("Adopt Me!", "Uplift Games", "312,884", 0.94),
                ("Blox Fruits", "Gamer Robot", "188,205", 0.62),
                ("Tower of Hell", "YXCeptional", "96,410", 0.41),
                ("BedWars", "Easy.gg", "74,002", 0.33),
                ("Doors", "LSPLASH", "51,377", 0.24),
            ]
            .iter()
            .enumerate()
            {
                ui.horizontal(|ui| {
                    meta(ui, probe, "charts.rank", &format!("{}", i + 1));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        body(ui, probe, "charts.name", name, TEXT);
                        meta(ui, probe, "charts.creator", creator);
                    });
                    ui.add_space(24.0);
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(300.0, 8.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(4), SURFACE);
                    let fill = egui::Rect::from_min_size(
                        rect.min,
                        Vec2::new(rect.width() * frac, rect.height()),
                    );
                    ui.painter().rect_filled(fill, CornerRadius::same(4), GREEN);
                    probe.widget("charts.bar", rect, false);
                    ui.add_space(16.0);
                    probe.label(
                        ui,
                        "charts.count",
                        *count,
                        egui::FontId::proportional(13.0),
                        TEXT,
                        CARD,
                    );
                });
                ui.add_space(10.0);
            }
        });
    });
}

pub fn game_details(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "game", |ui, probe| {
        probe.button(ui, "game.back", "Back to charts", Vec2::new(140.0, 32.0), CARD, MUTED);
        ui.add_space(12.0);
        card(ui, probe, "game.hero", 980.0, |ui, probe| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(320.0, 180.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(10), SURFACE);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "A",
                    egui::FontId::proportional(64.0),
                    MUTED,
                );
                probe.widget("game.thumb", rect, false);
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    heading(ui, probe, "game.title", "Adopt Me!", 22.0);
                    body(ui, probe, "game.creator", "by Uplift Games", MUTED);
                    ui.add_space(10.0);
                    probe.button(
                        ui,
                        "game.play",
                        "Play",
                        Vec2::new(220.0, 40.0),
                        GREEN,
                        BG,
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        stat_chip(ui, probe, "game.visits", "12.4B", "Visits");
                        stat_chip(ui, probe, "game.favs", "9.8M", "Favorites");
                        stat_chip(ui, probe, "game.online", "312K", "Playing");
                    });
                });
            });
        });
        ui.add_space(14.0);
        heading(ui, probe, "game.servers.title", "Servers", 15.0);
        ui.add_space(6.0);
        card(ui, probe, "game.servers", 980.0, |ui, probe| {
            for (i, (region, ping, players)) in [
                ("Frankfurt", "31 ms", "24/50"),
                ("London", "38 ms", "41/50"),
                ("New York", "92 ms", "12/50"),
            ]
            .iter()
            .enumerate()
            {
                ui.horizontal(|ui| {
                    body(ui, probe, "game.server.region", region, TEXT);
                    ui.add_space(24.0);
                    probe.label(
                        ui,
                        "game.server.ping",
                        *ping,
                        egui::FontId::proportional(13.0),
                        if i == 0 { GREEN } else { MUTED },
                        CARD,
                    );
                    ui.add_space(24.0);
                    meta(ui, probe, "game.server.players", players);
                    ui.add_space(24.0);
                    probe.button(
                        ui,
                        "game.server.join",
                        "Join",
                        Vec2::new(84.0, 30.0),
                        SURFACE,
                        TEXT,
                    );
                });
                ui.add_space(8.0);
            }
        });
    });
}

pub fn party(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "party", |ui, probe| {
        heading(ui, probe, "party.title", "Party", 22.0);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            card(ui, probe, "party.pill", 360.0, |ui, probe| {
                ui.horizontal(|ui| {
                    for (i, id) in ["party.member.1", "party.member.2", "party.member.3"]
                        .iter()
                        .enumerate()
                    {
                        avatar(ui, probe, id, 28.0, HOVER);
                        let _ = i;
                    }
                    ui.add_space(8.0);
                    body(ui, probe, "party.pill.count", "3 in party", TEXT);
                    ui.add_space(10.0);
                    probe.button(ui, "party.pill.invite", "Invite", Vec2::new(78.0, 30.0), GREEN, BG);
                });
            });
            card(ui, probe, "party.code.card", 420.0, |ui, probe| {
                meta(ui, probe, "party.code.label", "PARTY CODE");
                ui.add_space(4.0);
                probe.label(
                    ui,
                    "party.code.value",
                    "VEXA-7F2Q",
                    egui::FontId::monospace(20.0),
                    TEXT,
                    CARD,
                );
                ui.add_space(8.0);
                probe.button(
                    ui,
                    "party.code.copy",
                    "Copy code",
                    Vec2::new(120.0, 32.0),
                    SURFACE,
                    TEXT,
                );
            });
        });
        ui.add_space(16.0);
        heading(ui, probe, "party.members.title", "In this party", 15.0);
        ui.add_space(6.0);
        card(ui, probe, "party.members", 980.0, |ui, probe| {
            for (name, state, color) in [
                ("Vexa", "Host · Frankfurt", GREEN),
                ("Mika", "Ready", BLUE),
                ("Rin", "In Adopt Me!", MUTED),
            ] {
                ui.horizontal(|ui| {
                    avatar(ui, probe, "party.member.row", 32.0, HOVER);
                    ui.add_space(10.0);
                    body(ui, probe, "party.member.name", name, TEXT);
                    ui.add_space(16.0);
                    probe.label(
                        ui,
                        "party.member.state",
                        state,
                        egui::FontId::proportional(13.0),
                        color,
                        CARD,
                    );
                    ui.add_space(16.0);
                    probe.button(
                        ui,
                        "party.member.kick",
                        "Remove",
                        Vec2::new(90.0, 30.0),
                        SURFACE,
                        MUTED,
                    );
                });
                ui.add_space(10.0);
            }
        });
        ui.add_space(16.0);
        heading(
            ui,
            probe,
            "party.board.title",
            "Ready check and game vote",
            15.0,
        );
        ui.add_space(6.0);
        card(ui, probe, "party.board", 980.0, |ui, probe| {
            ui.horizontal_wrapped(|ui| {
                probe.button(
                    ui,
                    "party.board.ready",
                    "Ready Check",
                    Vec2::new(112.0, 28.0),
                    SURFACE,
                    TEXT,
                );
                body(ui, probe, "party.board.count", "2/3 ready · 8s", TEXT);
                probe.button(
                    ui,
                    "party.board.yes",
                    "Ready",
                    Vec2::new(64.0, 28.0),
                    GREEN,
                    BG,
                );
                probe.button(
                    ui,
                    "party.board.no",
                    "Not Ready",
                    Vec2::new(86.0, 28.0),
                    SURFACE,
                    TEXT,
                );
                ui.add_space(6.0);
                probe.button(
                    ui,
                    "party.board.doors",
                    "Doors (2)",
                    Vec2::new(92.0, 28.0),
                    SURFACE,
                    TEXT,
                );
                probe.button(
                    ui,
                    "party.board.bedwars",
                    "Bed Wars (1)",
                    Vec2::new(112.0, 28.0),
                    SURFACE,
                    TEXT,
                );
                probe.button(
                    ui,
                    "party.board.launch",
                    "Launch Doors",
                    Vec2::new(118.0, 28.0),
                    GREEN,
                    BG,
                );
            });
        });
    });
}

pub fn settings(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "settings", |ui, probe| {
        heading(ui, probe, "settings.title", "Settings", 22.0);
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            card_min(ui, probe, "settings.appearance", 470.0, 196.0, |ui, probe| {
                heading(ui, probe, "settings.appearance.title", "Appearance", 15.0);
                ui.add_space(8.0);
                for (id, label, on) in [
                    ("settings.dark", "Dark mode", true),
                    ("settings.glass", "Glass surfaces", false),
                ] {
                    ui.horizontal(|ui| {
                        probe.widget(id, toggle(ui, on), true);
                        ui.add_space(10.0);
                        body(ui, probe, id, label, TEXT);
                    });
                    ui.add_space(8.0);
                }
                ui.add_space(4.0);
                meta(ui, probe, "settings.accent.label", "Accent");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for (id, color) in [
                        ("settings.accent.green", GREEN),
                        ("settings.accent.blue", BLUE),
                        ("settings.accent.orange", ORANGE),
                        ("settings.accent.red", RED),
                    ] {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
                        ui.painter().rect_filled(rect, CornerRadius::same(6), color);
                        probe.widget(id, rect, true);
                    }
                });
            });
            card_min(ui, probe, "settings.launcher", 470.0, 196.0, |ui, probe| {
                heading(ui, probe, "settings.launcher.title", "Launcher", 15.0);
                ui.add_space(8.0);
                for (id, label, on) in [
                    ("settings.tray", "Keep in the tray", true),
                    ("settings.startup", "Start with Windows", false),
                    ("settings.updates", "Check for updates", true),
                ] {
                    ui.horizontal(|ui| {
                        probe.widget(id, toggle(ui, on), true);
                        ui.add_space(10.0);
                        body(ui, probe, id, label, TEXT);
                    });
                    ui.add_space(8.0);
                }
            });
        });
        ui.add_space(14.0);
        ui.horizontal_top(|ui| {
            card_min(ui, probe, "settings.overlay", 470.0, 160.0, |ui, probe| {
                heading(ui, probe, "settings.overlay.title", "Overlay HUD", 15.0);
                ui.add_space(8.0);
                meta(ui, probe, "settings.overlay.scale", "Scale");
                ui.add_space(4.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(440.0, 24.0), egui::Sense::click());
                ui.painter().rect_filled(rect, CornerRadius::same(12), SURFACE);
                ui.painter().circle_filled(rect.center(), 9.0, GREEN);
                probe.widget("settings.overlay.scale.slider", rect, true);
                ui.add_space(10.0);
                body(ui, probe, "settings.overlay.hint", "Drag to resize the in-game HUD.", MUTED);
            });
            card_min(ui, probe, "settings.updates", 470.0, 160.0, |ui, probe| {
                heading(ui, probe, "settings.updates.title", "Updates", 15.0);
                ui.add_space(6.0);
                body(ui, probe, "settings.updates.version", "Version 0.5.5", TEXT);
                meta(ui, probe, "settings.updates.state", "Up to date as of today");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    probe.button(
                        ui,
                        "settings.updates.check",
                        "Check now",
                        Vec2::new(120.0, 32.0),
                        SURFACE,
                        TEXT,
                    );
                    probe.button(
                        ui,
                        "settings.updates.release",
                        "Open release",
                        Vec2::new(130.0, 32.0),
                        CARD,
                        MUTED,
                    );
                });
            });
        });
    });
}

fn toggle(ui: &mut egui::Ui, on: bool) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 24.0), egui::Sense::click());
    let bg = if on { GREEN } else { HOVER };
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    let knob_x = if on {
        rect.right() - 12.0
    } else {
        rect.left() + 12.0
    };
    ui.painter()
        .circle_filled(egui::Pos2::new(knob_x, rect.center().y), 8.0, BG);
    rect
}

pub fn updater(ui: &mut egui::Ui, probe: &mut Probe) {
    // The updater window is a small standalone frame, not a page inside the shell.
    ui.vertical_centered(|ui| {
        ui.add_space(90.0);
        card(ui, probe, "updater.card", 520.0, |ui, probe| {
            heading(ui, probe, "updater.title", "Updating CREBLOX", 20.0);
            ui.add_space(4.0);
            body(ui, probe, "updater.stage", "Installing...", TEXT);
            ui.add_space(12.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(492.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(5), SURFACE);
            let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * 0.72, 10.0));
            ui.painter().rect_filled(fill, CornerRadius::same(5), GREEN);
            probe.widget("updater.progress", rect, false);
            ui.add_space(8.0);
            meta(ui, probe, "updater.percent", "72%");
            ui.add_space(10.0);
            probe.label(
                ui,
                "updater.log",
                "Replacing C:\\Users\\you\\AppData\\Local\\creblox\\creblox.exe",
                egui::FontId::monospace(11.0),
                MUTED,
                CARD,
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                probe.button(
                    ui,
                    "updater.details",
                    "Details",
                    Vec2::new(100.0, 30.0),
                    SURFACE,
                    TEXT,
                );
                probe.button(ui, "updater.cancel", "Cancel", Vec2::new(100.0, 30.0), CARD, MUTED);
            });
        });
    });
}

pub fn overlay(ui: &mut egui::Ui, probe: &mut Probe) {
    // A game frame behind the HUD, so the overlay's translucency is visible in the screenshot.
    let screen = ui.ctx().viewport_rect();
    let painter = ui.painter();
    // Enough bands that the backdrop reads as a gradient rather than stripes.
    for i in 0..160 {
        let t = i as f32 / 159.0;
        let rect = egui::Rect::from_min_size(
            egui::Pos2::new(0.0, screen.height() * t),
            Vec2::new(screen.width(), screen.height() / 159.0 + 1.0),
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
    probe.widget("overlay.game", screen, false);

    let hud = egui::Rect::from_min_size(egui::Pos2::new(24.0, 24.0), Vec2::new(392.0, 44.0));
    ui.painter().rect_filled(
        hud,
        CornerRadius::same(12),
        Color32::from_rgba_unmultiplied(11, 13, 17, 220),
    );
    ui.painter()
        .rect_stroke(hud, CornerRadius::same(12), Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
    probe.widget("overlay.hud", hud, false);

    ui.scope_builder(egui::UiBuilder::new().max_rect(hud.shrink2(Vec2::new(14.0, 8.0))), |ui| {
        ui.horizontal_centered(|ui| {
            probe.label(
                ui,
                "overlay.brand",
                "CREBLOX",
                egui::FontId::proportional(12.0),
                GREEN,
                CARD,
            );
            ui.add_space(10.0);
            probe.label(
                ui,
                "overlay.fps",
                "240 FPS",
                egui::FontId::monospace(13.0),
                TEXT,
                CARD,
            );
            ui.add_space(10.0);
            probe.label(
                ui,
                "overlay.ping",
                "32 ms",
                egui::FontId::monospace(13.0),
                GREEN,
                CARD,
            );
            ui.add_space(10.0);
            probe.label(
                ui,
                "overlay.server",
                "Frankfurt",
                egui::FontId::proportional(12.0),
                MUTED,
                CARD,
            );
        });
    });

    let badge = egui::Rect::from_min_size(
        egui::Pos2::new(screen.width() - 214.0, screen.height() - 52.0),
        Vec2::new(190.0, 28.0),
    );
    ui.painter().rect_filled(
        badge,
        CornerRadius::same(14),
        Color32::from_rgba_unmultiplied(11, 13, 17, 180),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(badge.shrink2(Vec2::new(12.0, 5.0))), |ui| {
        probe.label(
            ui,
            "overlay.watermark",
            "CREBLOX 0.5.5",
            egui::FontId::proportional(11.0),
            MUTED,
            CARD,
        );
    });
}
