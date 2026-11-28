//! The screens the headless renderer draws.
//!
//! These mirror the shipped pages — `src/ui/page_home.rs`, `page_charts.rs`, `page_game.rs`,
//! `ui/party.rs`, `page_settings.rs`, `updater.rs` and `src/game_overlay.rs` — at the level the
//! Linux VM can run: the palette, spacing, corner radii and widget shapes come from
//! `src/theme.rs`, but the *page functions themselves* need `crate::app::Creblox`, eframe and the
//! OS APIs, so the GUI crate cannot be linked on this host. A change to a page's layout should be
//! mirrored here so the screenshot test still shows what the page looks like.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Vec2};

use crate::inventory::Category;
use crate::shots::Probe;

// Palette: the same values as `Palette::of(..)` in `src/theme.rs`, read through the functions
// below so the gallery can be rendered in both themes. The names stay upper case because every
// page below reads them the same way the app reads its palette.
thread_local! {
    static LIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the fixtures are drawing the light palette right now.
pub fn light() -> bool {
    LIGHT.with(|on| on.get())
}

/// Switches the palette the fixtures draw with; the caller sets it before each render.
pub fn set_light(on: bool) {
    LIGHT.with(|c| c.set(on));
}

/// The app draws accent *text* through `theme::text_ink`; the fixtures do the same at the same
/// places, so a picture shows the colour the page would really paint.
fn ink(color: Color32) -> Color32 {
    crate::theme::ink(color, !light())
}

fn palette() -> crate::theme::Palette {
    crate::theme::Palette::of(!light())
}

#[allow(non_snake_case)]
pub fn BG() -> Color32 {
    palette().bg
}
#[allow(non_snake_case)]
pub fn CARD() -> Color32 {
    palette().card_bg
}
#[allow(non_snake_case)]
pub fn SURFACE() -> Color32 {
    palette().surface_bg
}
#[allow(non_snake_case)]
pub fn HOVER() -> Color32 {
    palette().hover_bg
}
#[allow(non_snake_case)]
pub fn SELECTED() -> Color32 {
    palette().selected_bg
}
#[allow(non_snake_case)]
pub fn TEXT() -> Color32 {
    palette().text
}
#[allow(non_snake_case)]
pub fn MUTED() -> Color32 {
    palette().muted
}
/// `theme.rs` hairlines are white at 6%; opaque here so the rasterizer has no extra blending to
/// hide behind.The light theme gets the same 7% black hairline made opaque.
#[allow(non_snake_case)]
pub fn BORDER() -> Color32 {
    if light() {
        Color32::from_rgb(0xD7, 0xDB, 0xE2)
    } else {
        Color32::from_rgb(0x2A, 0x2F, 0x3C)
    }
}
// Accents: `GREEN`/`BLUE`/`ORANGE`/`RED` in `src/theme.rs` - the same in both themes.
pub use crate::theme::{BLUE, GREEN, ON_ACCENT, ORANGE, RED};
/// `page_charts::rail_color`: one step off the slider default, so the filter rails read against
/// the bar behind them.
#[allow(non_snake_case)]
pub fn RAIL() -> Color32 {
    if light() {
        Color32::from_rgb(0xC4, 0xCA, 0xD5)
    } else {
        Color32::from_rgb(0x33, 0x38, 0x4A)
    }
}

const RADIUS: u8 = 8;
/// `palette.profile_border` at the loading pulse: a section that has not arrived.
#[allow(non_snake_case)]
pub fn SKELETON() -> Color32 {
    if light() {
        Color32::from_rgb(0xE6, 0xE9, 0xEE)
    } else {
        Color32::from_rgb(0x26, 0x2B, 0x38)
    }
}

/// Every rendered screen, in the order the gallery is written.
pub fn all() -> Vec<(&'static str, fn(&mut egui::Ui, &mut Probe))> {
    vec![
        ("home", home),
        ("charts", charts),
        ("game", game_details),
        ("party", party),
        ("settings", settings),
        ("settings-animations", settings_animations),
        ("settings-updates", settings_updates),
        ("settings-auto-update", settings_auto_update),
        ("updater", updater),
        ("overlay", overlay),
        ("inventory", inventory),
        ("inventory-private", inventory_private),
        ("sidebar", sidebar),
        ("profile", profile),
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
        .fill(CARD())
        .stroke(Stroke::new(1.0, BORDER()))
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
        TEXT(),
        BG(),
    );
}

fn body(ui: &mut egui::Ui, probe: &mut Probe, id: impl Into<String>, text: &str, color: Color32) {
    probe.label(
        ui,
        id,
        text,
        egui::FontId::proportional(13.0),
        color,
        CARD(),
    );
}

fn meta(ui: &mut egui::Ui, probe: &mut Probe, id: impl Into<String>, text: &str) {
    probe.label(
        ui,
        id,
        text,
        egui::FontId::proportional(11.0),
        MUTED(),
        CARD(),
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
        .fill(CARD())
        .inner_margin(egui::Margin::symmetric(14, 18))
        .show(ui, |ui| {
            ui.set_width(148.0);
            ui.vertical(|ui| {
                probe.label(
                    ui,
                    "rail.logo",
                    "CREBLOX",
                    egui::FontId::proportional(15.0),
                    ink(GREEN),
                    CARD(),
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
                        if selected { SELECTED() } else { CARD() },
                        if selected { TEXT() } else { MUTED() },
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
        .fill(SURFACE())
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
            ui.vertical(|ui| {
                probe.label(
                    ui,
                    format!("{id}.value"),
                    value,
                    egui::FontId::proportional(15.0),
                    TEXT(),
                    SURFACE(),
                );
                probe.label(
                    ui,
                    format!("{id}.label"),
                    label,
                    egui::FontId::proportional(11.0),
                    MUTED(),
                    SURFACE(),
                );
            });
        });
}

/// A placeholder bar, the same shape the profile page uses for a section that has not
/// arrived: a rounded fill of the border tone, pulsing slightly in the real page.
fn skeleton(ui: &mut egui::Ui, probe: &mut Probe, id: impl Into<String>, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), SKELETON());
    probe.widget(id, rect, false);
}

/// Player Profiles on frame 1: the header, the avatar and the buttons come from the user
/// record the click carried, and only the sections that are still in flight are skeletons.
pub fn profile(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "profile", |ui, probe| {
        heading(ui, probe, "profile.title", "Profile", 22.0);
        ui.add_space(10.0);

        card_min(ui, probe, "profile.header", 980.0, 0.0, |ui, probe| {
            ui.horizontal_top(|ui| {
                ui.add_space(4.0);
                avatar(ui, probe, "profile.header.avatar", 148.0, SURFACE());
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        heading(ui, probe, "profile.header.name", "Vexa", 24.0);
                        ui.add_space(8.0);
                        probe.button(
                            ui,
                            "profile.header.verified",
                            "verified",
                            Vec2::new(64.0, 20.0),
                            SURFACE(),
                            ink(BLUE),
                        );
                        ui.add_space(8.0);
                        probe.button(
                            ui,
                            "profile.header.presence",
                            "In Game - Adopt Me!",
                            Vec2::new(148.0, 22.0),
                            SURFACE(),
                            ink(GREEN),
                        );
                        // The actions are part of the frame-1 shell: the user record the
                        // click carried already knows the place, so Join Game is live.
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                probe.button(
                                    ui,
                                    "profile.action.join",
                                    "Join Game",
                                    Vec2::new(104.0, 30.0),
                                    GREEN,
                                    ON_ACCENT,
                                );
                                ui.add_space(6.0);
                                probe.button(
                                    ui,
                                    "profile.action.add",
                                    "Add Friend",
                                    Vec2::new(96.0, 30.0),
                                    SURFACE(),
                                    TEXT(),
                                );
                                ui.add_space(6.0);
                                probe.button(
                                    ui,
                                    "profile.action.more",
                                    "...",
                                    Vec2::new(32.0, 30.0),
                                    SURFACE(),
                                    TEXT(),
                                );
                            },
                        );
                    });
                    ui.add_space(6.0);
                    meta(ui, probe, "profile.header.handle", "@vexatheshadow  -  ID 169567434");
                    ui.add_space(10.0);
                    // Counters that have not answered yet keep the pill footprint.
                    ui.horizontal(|ui| {
                        for (id, w) in [
                            ("profile.count.friends", 76.0),
                            ("profile.count.followers", 76.0),
                            ("profile.count.following", 76.0),
                        ] {
                            skeleton(ui, probe, id, Vec2::new(w, 28.0));
                            ui.add_space(8.0);
                        }
                    });
                    ui.add_space(12.0);
                    // The bio request is still running: two placeholder lines, never
                    // "this user has no bio".
                    skeleton(ui, probe, "profile.bio.line1", Vec2::new(430.0, 12.0));
                    ui.add_space(6.0);
                    skeleton(ui, probe, "profile.bio.line2", Vec2::new(260.0, 12.0));
                });
            });
        });

        ui.add_space(16.0);
        heading(ui, probe, "profile.section.wearing", "Currently Wearing", 16.5);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for i in 0..3 {
                skeleton(
                    ui,
                    probe,
                    format!("profile.wearing.{i}"),
                    Vec2::new(150.0, 200.0),
                );
                ui.add_space(12.0);
            }
        });

        ui.add_space(18.0);
        heading(ui, probe, "profile.section.badges", "Badges", 16.5);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for i in 0..4 {
                skeleton(ui, probe, format!("profile.badges.{i}"), Vec2::new(134.0, 168.0));
                ui.add_space(12.0);
            }
        });
    });
}

pub fn home(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "home", |ui, probe| {
        heading(ui, probe, "home.title", "Home", 22.0);
        ui.add_space(10.0);

        card(ui, probe, "home.profile", 980.0, |ui, probe| {
            ui.horizontal(|ui| {
                avatar(ui, probe, "home.profile.avatar", 56.0, HOVER());
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    probe.label(
                        ui,
                        "home.profile.name",
                        "Vexa",
                        egui::FontId::proportional(17.0),
                        TEXT(),
                        CARD(),
                    );
                    body(ui, probe, "home.profile.status", "Playing Adopt Me!", ink(GREEN));
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
                        avatar(ui, probe, id, 32.0, HOVER());
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            body(ui, probe, id, name, TEXT());
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
                        ui.painter().rect_filled(rect, CornerRadius::same(6), SURFACE());
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            title.chars().next().unwrap_or('?').to_string(),
                            egui::FontId::proportional(28.0),
                            MUTED(),
                        );
                        probe.widget(format!("{id}.thumb"), rect, false);
                        ui.add_space(8.0);
                        body(ui, probe, format!("{id}.title"), title, TEXT());
                        meta(ui, probe, format!("{id}.meta"), creator);
                        body(
                            ui,
                            probe,
                            format!("{id}.playing"),
                            &format!("{playing} playing"),
                            ink(GREEN),
                        );
                    });
                }
            });
            ui.add_space(10.0);
        }
    });
}

/// `page_charts::filter_pill`: a rounded box with its value (or hint) centred in both
/// directions. The pill and the text it paints are recorded, so the shot test can measure the
/// centring instead of trusting it.
fn pill(ui: &mut egui::Ui, probe: &mut Probe, id: &'static str, value: &str, hint: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(64.0, 26.0), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(9),
        SURFACE(),
        Stroke::new(1.0, BORDER()),
        egui::StrokeKind::Inside,
    );
    let color = if hint { MUTED() } else { TEXT() };
    let galley = ui
        .painter()
        .layout_no_wrap(value.to_string(), egui::FontId::proportional(13.0), color);
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley.clone(), color);
    probe.text(format!("{id}.text"), Rect::from_min_size(pos, galley.size()), galley.size());
    probe.contrast(format!("{id}.text"), crate::theme::ink(color, !light()), SURFACE());
    probe.widget(id, rect, false);
}

/// `page_charts::filter_sep`: the en dash between the two player pills.
fn dash(ui: &mut egui::Ui, probe: &mut Probe) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 22.0), Sense::hover());
    let galley =
        ui.painter()
            .layout_no_wrap("\u{2013}".to_string(), egui::FontId::proportional(12.0), MUTED());
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley.clone(), MUTED());
    probe.text("charts.f.players.dash", Rect::from_min_size(pos, galley.size()), galley.size());
    probe.contrast("charts.f.players.dash", MUTED(), CARD());
}

/// The Games filter bar: Players (minimum rail, minimum pill, maximum pill), Like % and Visits,
/// with Clear pinned right while a filter is set.
fn charts_filter_bar(ui: &mut egui::Ui, probe: &mut Probe) {
    let mut players = 0u32;
    let mut rating = 45u8;
    let mut visits = 0u64;
    card(ui, probe, "charts.filters", 966.0, |ui, probe| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            // Like % carries a typed value, so the Clear button is part of the picture.
            let any = rating > 0;
            let slot = ((ui.available_width() - 58.0 - 32.0) / 4.0).floor();
            let players_w = slot * 2.0 + 8.0;

            ui.allocate_ui_with_layout(
                Vec2::new(players_w, 26.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(
                        Vec2::new(52.0, 26.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            probe.label(
                                ui,
                                "charts.f.players.label",
                                "Players",
                                egui::FontId::proportional(12.5),
                                MUTED(),
                                CARD(),
                            );
                        },
                    );
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - 64.0 * 2.0 - 14.0 - 18.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    let rail = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = RAIL();
                            ui.visuals_mut().widgets.hovered.bg_fill = RAIL();
                            ui.visuals_mut().widgets.active.bg_fill = RAIL();
                            ui.add_sized(
                                Vec2::new(rail_w, 24.0),
                                egui::Slider::new(&mut players, 0..=100_000u32)
                                    .step_by(50.0)
                                    .show_value(false),
                            )
                        })
                        .inner;
                    probe.widget("charts.f.players.rail", rail.rect, true);
                    ui.add_space(6.0);
                    pill(ui, probe, "charts.f.players.min", "10", false);
                    ui.add_space(6.0);
                    dash(ui, probe);
                    ui.add_space(6.0);
                    pill(ui, probe, "charts.f.players.max", "500K", false);
                },
            );

            {
                let (id, label, value) = ("like", "Like %", "45");
                ui.allocate_ui_with_layout(
                    Vec2::new(slot, 26.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.allocate_ui_with_layout(
                            Vec2::new(52.0, 26.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                probe.label(
                                    ui,
                                    format!("charts.f.{id}.label"),
                                    label,
                                    egui::FontId::proportional(12.5),
                                    MUTED(),
                                    CARD(),
                                );
                            },
                        );
                        ui.add_space(6.0);
                        let rail_w = (ui.available_width() - 64.0 - 6.0).max(72.0);
                        ui.spacing_mut().slider_width = rail_w;
                        let rail = ui
                            .scope(|ui| {
                                ui.visuals_mut().widgets.inactive.bg_fill = RAIL();
                                ui.visuals_mut().widgets.hovered.bg_fill = RAIL();
                                ui.visuals_mut().widgets.active.bg_fill = RAIL();
                                ui.add_sized(
                                    Vec2::new(rail_w, 24.0),
                                    egui::Slider::new(&mut rating, 0..=100u8).show_value(false),
                                )
                            })
                            .inner;
                        probe.widget(format!("charts.f.{id}.rail"), rail.rect, true);
                        ui.add_space(6.0);
                        pill(ui, probe, "charts.f.like", value, false);
                    },
                );
            }

            ui.allocate_ui_with_layout(
                Vec2::new(slot, 26.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(
                        Vec2::new(52.0, 26.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            probe.label(
                                ui,
                                "charts.f.visits.label",
                                "Visits",
                                egui::FontId::proportional(12.5),
                                MUTED(),
                                CARD(),
                            );
                        },
                    );
                    ui.add_space(6.0);
                    let rail_w = (ui.available_width() - 64.0 - 6.0).max(72.0);
                    ui.spacing_mut().slider_width = rail_w;
                    let rail = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = RAIL();
                            ui.visuals_mut().widgets.hovered.bg_fill = RAIL();
                            ui.visuals_mut().widgets.active.bg_fill = RAIL();
                            ui.add_sized(
                                Vec2::new(rail_w, 24.0),
                                egui::Slider::new(&mut visits, 0..=100_000_000u64)
                                    .step_by(100_000.0)
                                    .show_value(false),
                            )
                        })
                        .inner;
                    probe.widget("charts.f.visits.rail", rail.rect, true);
                    ui.add_space(6.0);
                    pill(ui, probe, "charts.f.visits", "0", true);
                },
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if any {
                    probe.button(
                        ui,
                        "charts.f.clear",
                        "Clear",
                        Vec2::new(58.0, 24.0),
                        SURFACE(),
                        MUTED(),
                    );
                }
            });
        });
    });
}

/// Just the filter bar, at twice the size, for the close-up in `artifacts/charts_filters.png`.
pub fn charts_filters(ui: &mut egui::Ui, probe: &mut Probe) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.vertical(|ui| charts_filter_bar(ui, probe));
        });
}

pub fn charts(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "charts", |ui, probe| {
        ui.horizontal(|ui| {
            heading(ui, probe, "charts.title", "Games", 26.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                probe.button(
                    ui,
                    "charts.shuffle",
                    "Reshuffle",
                    Vec2::new(98.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
                probe.button(
                    ui,
                    "charts.refresh",
                    "Refresh",
                    Vec2::new(84.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
            });
        });
        ui.add_space(10.0);

        charts_filter_bar(ui, probe);
        ui.add_space(10.0);

        // The culled card grid: thumbnails, name and pre-formatted stats.
        // Five fixed-width cards per row keep the fixture inside the content column at both
        // capture sizes; the real grid derives its width and column count from the viewport.
        let card_w = 170.0;
        let games = [
            ("Adopt Me!", "Uplift Games", "92% \u{00b7} 312K playing"),
            ("Blox Fruits", "Gamer Robot", "88% \u{00b7} 188K playing"),
            ("Tower of Hell", "YXCeptional", "95% \u{00b7} 96K playing"),
            ("BedWars", "Easy.gg", "90% \u{00b7} 74K playing"),
            ("Doors", "LSPLASH", "94% \u{00b7} 51K playing"),
            ("Bloxburg", "Coeptus", "89% \u{00b7} 43K playing"),
            ("Jailbreak", "Badimo", "86% \u{00b7} 39K playing"),
            ("Brookhaven", "Wolfpaq", "81% \u{00b7} 35K playing"),
            ("Piggy", "MiniToon", "91% \u{00b7} 21K playing"),
            ("Arsenal", "ROLVe", "93% \u{00b7} 18K playing"),
        ];
        for (row, chunk) in games.chunks(5).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (column, (name, creator, stats)) in chunk.iter().enumerate() {
                    chart_card(ui, probe, row * 5 + column, card_w, name, creator, stats);
                }
            });
            ui.add_space(8.0);
        }
        meta(
            ui,
            probe,
            "charts.footer",
            "1,250 games loaded \u{2014} keep scrolling",
        );
    });
}

/// One charts card: 16:9 thumbnail slot, name, creator and the pre-formatted stats line.
fn chart_card(
    ui: &mut egui::Ui,
    probe: &mut Probe,
    idx: usize,
    w: f32,
    name: &str,
    creator: &str,
    stats: &str,
) {
    let frame = egui::Frame::NONE
        .fill(CARD())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(6);
    let inner = frame.show(ui, |ui| {
        ui.set_width(w);
        ui.vertical(|ui| {
            let (thumb, _) =
                ui.allocate_exact_size(Vec2::new(w, (w * 9.0 / 16.0).floor()), Sense::hover());
            ui.painter().rect_filled(thumb, CornerRadius::same(6), SURFACE());
            ui.painter().circle_filled(
                thumb.center(),
                14.0,
                Color32::from_rgb(0x2E, 0x34, 0x44),
            );
            probe.widget(format!("charts.card.{idx}.thumb"), thumb, false);
            ui.add_space(6.0);
            probe.label(
                ui,
                format!("charts.card.{idx}.name"),
                name,
                egui::FontId::proportional(13.0),
                TEXT(),
                CARD(),
            );
            probe.label(
                ui,
                format!("charts.card.{idx}.creator"),
                creator,
                egui::FontId::proportional(11.0),
                MUTED(),
                CARD(),
            );
            probe.label(
                ui,
                format!("charts.card.{idx}.stats"),
                stats,
                egui::FontId::proportional(11.0),
                MUTED(),
                CARD(),
            );
        });
    });
    probe.widget(format!("charts.card.{idx}"), inner.response.rect, false);
}

pub fn game_details(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "game", |ui, probe| {
        probe.button(ui, "game.back", "Back to charts", Vec2::new(140.0, 32.0), CARD(), MUTED());
        ui.add_space(12.0);
        card(ui, probe, "game.hero", 980.0, |ui, probe| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(320.0, 180.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(10), SURFACE());
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "A",
                    egui::FontId::proportional(64.0),
                    MUTED(),
                );
                probe.widget("game.thumb", rect, false);
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    heading(ui, probe, "game.title", "Adopt Me!", 22.0);
                    body(ui, probe, "game.creator", "by Uplift Games", MUTED());
                    ui.add_space(10.0);
                    probe.button(
                        ui,
                        "game.play",
                        "Play",
                        Vec2::new(220.0, 40.0),
                        GREEN,
                        ON_ACCENT,
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
                    body(ui, probe, "game.server.region", region, TEXT());
                    ui.add_space(24.0);
                    probe.label(
                        ui,
                        "game.server.ping",
                        *ping,
                        egui::FontId::proportional(13.0),
                        ink(if i == 0 { GREEN } else { MUTED() }),
                        CARD(),
                    );
                    ui.add_space(24.0);
                    meta(ui, probe, "game.server.players", players);
                    ui.add_space(24.0);
                    probe.button(
                        ui,
                        "game.server.join",
                        "Join",
                        Vec2::new(84.0, 30.0),
                        SURFACE(),
                        TEXT(),
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
                        avatar(ui, probe, id, 28.0, HOVER());
                        let _ = i;
                    }
                    ui.add_space(8.0);
                    body(ui, probe, "party.pill.count", "3 in party", TEXT());
                    ui.add_space(10.0);
                    probe.button(ui, "party.pill.invite", "Invite", Vec2::new(78.0, 30.0), GREEN, ON_ACCENT);
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
                    TEXT(),
                    CARD(),
                );
                ui.add_space(8.0);
                probe.button(
                    ui,
                    "party.code.copy",
                    "Copy code",
                    Vec2::new(120.0, 32.0),
                    SURFACE(),
                    TEXT(),
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
                ("Rin", "In Adopt Me!", MUTED()),
            ] {
                ui.horizontal(|ui| {
                    avatar(ui, probe, "party.member.row", 32.0, HOVER());
                    ui.add_space(10.0);
                    body(ui, probe, "party.member.name", name, TEXT());
                    ui.add_space(16.0);
                    probe.label(
                        ui,
                        "party.member.state",
                        state,
                        egui::FontId::proportional(13.0),
                        ink(color),
                        CARD(),
                    );
                    ui.add_space(16.0);
                    probe.button(
                        ui,
                        "party.member.kick",
                        "Remove",
                        Vec2::new(90.0, 30.0),
                        SURFACE(),
                        MUTED(),
                    );
                });
                ui.add_space(10.0);
            }
        });
        // The late-joiner slot check (`party_slot_row` in `src/ui/party.rs`): a member whose
        // join was refused because the party's server filled up.
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            probe.label(
                ui,
                "party.slot.text",
                "Adopt Me!'s server is full - no room for you.",
                egui::FontId::proportional(11.5),
                MUTED(),
                BG(),
            );
            ui.add_space(8.0);
            probe.button(
                ui,
                "party.slot.ask",
                "Ask host to hop",
                Vec2::new(122.0, 24.0),
                SURFACE(),
                MUTED(),
            );
        });
        ui.add_space(12.0);
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
                    SURFACE(),
                    TEXT(),
                );
                body(ui, probe, "party.board.count", "2/3 ready · 8s", TEXT());
                probe.button(
                    ui,
                    "party.board.yes",
                    "Ready",
                    Vec2::new(64.0, 28.0),
                    GREEN,
                    ON_ACCENT,
                );
                probe.button(
                    ui,
                    "party.board.no",
                    "Not Ready",
                    Vec2::new(86.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
                ui.add_space(6.0);
                probe.button(
                    ui,
                    "party.board.doors",
                    "Doors (2)",
                    Vec2::new(92.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
                probe.button(
                    ui,
                    "party.board.bedwars",
                    "Bed Wars (1)",
                    Vec2::new(112.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
                probe.button(
                    ui,
                    "party.board.launch",
                    "Launch Doors",
                    Vec2::new(118.0, 28.0),
                    GREEN,
                    ON_ACCENT,
                );
            });
        });
    });
}

pub fn settings(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "settings", |ui, probe| {
        heading(ui, probe, "settings.title", "Settings", 22.0);
        ui.add_space(10.0);
        // Mirrors page_settings' notice for a settings.json written by a newer build. The page
        // shows it only in that case; the fixture shows it so its layout is checked.
        card_min(ui, probe, "settings.newer", 940.0, 58.0, |ui, probe| {
            heading(
                ui,
                probe,
                "settings.newer.title",
                "Settings file from a newer CREBLOX (v9)",
                13.0,
            );
            ui.add_space(4.0);
            body(
                ui,
                probe,
                "settings.newer.body",
                "Changes you make here are saved into it; its other settings are left untouched.",
                MUTED(),
            );
        });
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
                        body(ui, probe, id, label, TEXT());
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
                        body(ui, probe, id, label, TEXT());
                    });
                    ui.add_space(8.0);
                }
                // Mirrors the Settings readout of the tracked image caches, so its type sizes
                // and wrapping are laid out here even though the real page needs macOS.
                body(
                    ui,
                    probe,
                    "settings.cache.total",
                    "Image caches: 0 B / 192 MB",
                    TEXT(),
                );
                ui.add_space(2.0);
                meta(
                    ui,
                    probe,
                    "settings.cache.parts",
                    "0 B textures · 0 B waiting · 0 B video · 0 B avatar · 0 B profiles · process 0 MB",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    probe.widget("settings.auto_refresh", toggle(ui, true), true);
                    ui.add_space(10.0);
                    body(
                        ui,
                        probe,
                        "settings.auto_refresh.label",
                        "Automatic background refresh",
                        TEXT(),
                    );
                });
                meta(
                    ui,
                    probe,
                    "settings.auto_refresh.hint",
                    "Silently update lists, player counts and friends",
                );
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
                ui.painter().rect_filled(rect, CornerRadius::same(12), SURFACE());
                ui.painter().circle_filled(rect.center(), 9.0, GREEN);
                probe.widget("settings.overlay.scale.slider", rect, true);
                ui.add_space(10.0);
                body(ui, probe, "settings.overlay.hint", "Drag to resize the in-game HUD.", MUTED());
            });
            card_min(ui, probe, "settings.updates", 470.0, 160.0, |ui, probe| {
                heading(ui, probe, "settings.updates.title", "Updates", 15.0);
                ui.add_space(6.0);
                body(ui, probe, "settings.updates.version", "Version 0.5.5", TEXT());
                meta(ui, probe, "settings.updates.state", "Up to date as of today");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    probe.button(
                        ui,
                        "settings.updates.check",
                        "Check now",
                        Vec2::new(120.0, 32.0),
                        SURFACE(),
                        TEXT(),
                    );
                    probe.button(
                        ui,
                        "settings.updates.release",
                        "Open release",
                        Vec2::new(130.0, 32.0),
                        CARD(),
                        MUTED(),
                    );
                });
            });
        });
    });
}

/// The Launcher Updates card: formatted release notes with a Details drawer, and a failed
/// local build whose compiler output is one click away.
pub fn settings_updates(ui: &mut egui::Ui, probe: &mut Probe) {
    const NOTES: &str = "## What changed\n\n- **Disabled controls** no longer hint at the accent.\n- Overlay images drag from one press-resolved hitbox.\n- Arrow keys move the segmented picker.\n\n## Downloads\n\n- Quick update: https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.69-update.zip\n- SHA-256: `768b0f92ee77f646b486ba9570cc8649650a1a532bddb7c4efce260b9ee7b52c`";
    const LOG: &[&str] = &[
        "   Compiling creblox v0.5.69",
        "error[E0308]: mismatched types",
        "  --> src/overlay_window.rs:2041:9",
        "   |",
        "41 |     ctx.send_viewport_cmd(command)",
        "   |         ^^^^^^^^^^^^^^^^^^^^^^^^ expected `ViewportCommand`, found `Command`",
        "error: could not compile `creblox` (bin \"creblox\") due to 1 previous error",
    ];
    shell(ui, probe, "settings", |ui, probe| {
        heading(ui, probe, "settings.title", "Settings", 22.0);
        ui.add_space(10.0);
        card_min(ui, probe, "settings.updates.available", 940.0, 0.0, |ui, probe| {
            heading(
                ui,
                probe,
                "settings.updates.available.title",
                "New version ready: v0.5.69",
                14.0,
            );
            ui.add_space(6.0);
            crate::update_card::paint_notes(ui, NOTES);
        });
        ui.add_space(14.0);
        card_min(ui, probe, "settings.updates.error", 940.0, 0.0, |ui, probe| {
            heading(
                ui,
                probe,
                "settings.updates.error.title",
                "Launcher Updates",
                14.0,
            );
            ui.add_space(6.0);
            body(
                ui,
                probe,
                "settings.updates.error.text",
                "The build failed (exit 101). Open the error log for the output.",
                ink(ORANGE),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                probe.button(
                    ui,
                    "settings.updates.error.browser",
                    "Download in Browser",
                    Vec2::new(142.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
                probe.button(
                    ui,
                    "settings.updates.error.log",
                    "Hide Error Log",
                    Vec2::new(104.0, 28.0),
                    SURFACE(),
                    TEXT(),
                );
            });
            ui.add_space(6.0);
            let lines: Vec<String> = LOG.iter().map(|line| line.to_string()).collect();
            crate::update_card::paint_log(ui, &lines);
        });
    });
}

/// The Settings page's Launcher Updates card with Automatic updates on.
///
/// Mirrors the shipped card's three checkboxes (the first is the new 30-second loop switch),
/// so the rendered page shows the toggle's label and its position in the card.
pub fn settings_auto_update(ui: &mut egui::Ui, probe: &mut Probe) {
    shell(ui, probe, "settings", |ui, probe| {
        heading(ui, probe, "settings.title", "Settings", 22.0);
        ui.add_space(10.0);
        card_min(ui, probe, "settings.auto.available", 940.0, 0.0, |ui, probe| {
            heading(
                ui,
                probe,
                "settings.auto.available.title",
                "New version ready: v0.5.72",
                14.0,
            );
            ui.add_space(6.0);
            body(
                ui,
                probe,
                "settings.auto.available.notes",
                "Installs automatically when no Roblox match is running.",
                MUTED(),
            );
        });
        ui.add_space(14.0);
        card_min(ui, probe, "settings.auto.card", 940.0, 0.0, |ui, probe| {
            heading(
                ui,
                probe,
                "settings.auto.title",
                "Launcher Updates",
                14.0,
            );
            ui.add_space(6.0);
            body(
                ui,
                probe,
                "settings.auto.description",
                "Downloads releases inside the app. Installs manually, or automatically when no match is running.",
                MUTED(),
            );
            ui.add_space(6.0);
            let mut auto_update = true;
            ui.checkbox(
                &mut auto_update,
                "Automatic updates (check every 30 seconds, install and restart outside a match)",
            );
            let mut on_startup = true;
            ui.checkbox(
                &mut on_startup,
                "Check for launcher updates on startup",
            );
            let mut download = false;
            ui.checkbox(
                &mut download,
                "Download updates automatically when available (install stays manual)",
            );
            ui.add_space(4.0);
            body(
                ui,
                probe,
                "settings.auto.stage",
                "Checking GitHub for a newer release...",
                MUTED(),
            );
        });
    });
}

/// The Settings page's Animations & Performance card with the master switch off.
///
/// Drawn with the shipping `controls` module rather than a fixture copy, and with motion off,
/// so the screenshot is the real disabled switch, disabled sub-toggles and disabled segmented
/// picker. The main Settings fixture is shorter than the page; this one shows the section the
/// master switch governs.
pub fn settings_animations(ui: &mut egui::Ui, probe: &mut Probe) {
    ui.ctx().data_mut(|d| {
        d.insert_temp(egui::Id::NULL.with("creblox_motion_on"), false);
    });
    shell(ui, probe, "settings", |ui, probe| {
        heading(ui, probe, "settings.title", "Settings", 22.0);
        ui.add_space(10.0);
        card_min(ui, probe, "settings.anim", 940.0, 0.0, |ui, probe| {
            heading(
                ui,
                probe,
                "settings.anim.title",
                "Animations & Performance",
                15.0,
            );
            ui.add_space(6.0);
            let mut master = false;
            probe.widget(
                "settings.anim.master",
                crate::controls::toggle_switch(ui, &mut master, "All animations").rect,
                true,
            );
            ui.add_enabled_ui(master, |ui| {
                ui.indent("anim_details", |ui| {
                    for (id, label) in [
                        (
                            "settings.anim.hover",
                            "Hover fades, rings and the sliding nav bar",
                        ),
                        (
                            "settings.anim.backgrounds",
                            "Animated backgrounds (login backdrop, appearance previews)",
                        ),
                        (
                            "settings.anim.videos",
                            "Game preview videos on hover (uses extra RAM, CPU and data)",
                        ),
                    ] {
                        let mut on = true;
                        probe.widget(
                            id,
                            crate::controls::toggle_switch(ui, &mut on, label).rect,
                            true,
                        );
                    }
                });
            });
            ui.add_space(8.0);
            meta(ui, probe, "settings.anim.position", "Party overlay position");
            ui.add_space(2.0);
            let mut corner = 0usize;
            let before = ui.cursor();
            let width = ui.available_width();
            ui.add_enabled_ui(false, |ui| {
                crate::controls::segmented(
                    ui,
                    "party_overlay_corner",
                    &["Top left", "Top right", "Bottom left", "Bottom right"],
                    &mut corner,
                );
            });
            probe.widget(
                "settings.anim.position.picker",
                Rect::from_min_size(before.min, Vec2::new(width, 28.0)),
                true,
            );
            ui.add_space(8.0);
            meta(
                ui,
                probe,
                "settings.anim.hint",
                "Dimmed while All animations is off: these cannot be changed.",
            );
        });
    });
}

fn toggle(ui: &mut egui::Ui, on: bool) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 24.0), egui::Sense::click());
    let bg = if on { GREEN } else { HOVER() };
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    let knob_x = if on {
        rect.right() - 12.0
    } else {
        rect.left() + 12.0
    };
    ui.painter()
        .circle_filled(egui::Pos2::new(knob_x, rect.center().y), 8.0, BG());
    rect
}

pub fn updater(ui: &mut egui::Ui, probe: &mut Probe) {
    // The updater window is a small standalone frame, not a page inside the shell.
    ui.vertical_centered(|ui| {
        ui.add_space(90.0);
        card(ui, probe, "updater.card", 520.0, |ui, probe| {
            heading(ui, probe, "updater.title", "Updating CREBLOX", 20.0);
            ui.add_space(4.0);
            body(ui, probe, "updater.stage", "Installing...", TEXT());
            ui.add_space(12.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(492.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(5), SURFACE());
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
                MUTED(),
                CARD(),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                probe.button(
                    ui,
                    "updater.details",
                    "Details",
                    Vec2::new(100.0, 30.0),
                    SURFACE(),
                    TEXT(),
                );
                probe.button(ui, "updater.cancel", "Cancel", Vec2::new(100.0, 30.0), CARD(), MUTED());
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
        .rect_stroke(hud, CornerRadius::same(12), Stroke::new(1.0, BORDER()), egui::StrokeKind::Inside);
    probe.widget("overlay.hud", hud, false);

    ui.scope_builder(egui::UiBuilder::new().max_rect(hud.shrink2(Vec2::new(14.0, 8.0))), |ui| {
        ui.horizontal_centered(|ui| {
            probe.label(
                ui,
                "overlay.brand",
                "CREBLOX",
                egui::FontId::proportional(12.0),
                ink(GREEN),
                CARD(),
            );
            ui.add_space(10.0);
            probe.label(
                ui,
                "overlay.fps",
                "240 FPS",
                egui::FontId::monospace(13.0),
                TEXT(),
                CARD(),
            );
            ui.add_space(10.0);
            probe.label(
                ui,
                "overlay.ping",
                "32 ms",
                egui::FontId::monospace(13.0),
                ink(GREEN),
                CARD(),
            );
            ui.add_space(10.0);
            probe.label(
                ui,
                "overlay.server",
                "Frankfurt",
                egui::FontId::proportional(12.0),
                MUTED(),
                CARD(),
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
            MUTED(),
            CARD(),
        );
    });
}

// ---------------------------------------------------------------------------
// Left navigation rail (`src/app.rs` + `src/ui/menu.rs`)
// ---------------------------------------------------------------------------

/// Shipped widths: the `nav` panel in `app.rs` and the item box in `menu.rs`.
const RAIL_WIDTH: f32 = 92.0;
const NAV_ITEM_WIDTH: f32 = 84.0;
const NAV_ITEM_HEIGHT: f32 = 58.0;

/// Same rule as `menu::fitted_label_font`: the largest size at or under 11 that fits.
fn nav_font(ui: &egui::Ui, label: &str, width: f32) -> egui::FontId {
    let mut size = 11.0f32;
    loop {
        let font = egui::FontId::proportional(size);
        let measured = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), TEXT()))
            .size()
            .x;
        if measured <= width || size <= 8.5 {
            return font;
        }
        size -= 0.5;
    }
}

/// The navigation rail next to a page. A label wider than its item box is the clipping bug
/// that shipped (`Notificat`, `Profil`), and the layout check fails on it.
pub fn sidebar(ui: &mut egui::Ui, probe: &mut Probe) {
    let screen = ui.ctx().viewport_rect();
    ui.painter()
        .rect_filled(screen, CornerRadius::ZERO, BG());

    let rail = egui::Rect::from_min_size(screen.min, Vec2::new(RAIL_WIDTH, screen.height()));
    ui.painter()
        .rect_filled(rail, CornerRadius::ZERO, CARD());
    probe.widget("nav.rail", rail, false);

    let items = [
        ("Home", true),
        ("Charts", false),
        ("Profile", false),
        ("Avatar", false),
        ("Notifications", false),
        ("Settings", false),
    ];
    let mut y = screen.min.y + 10.0;
    for (i, (label, selected)) in items.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(screen.min.x, y),
            Vec2::new(NAV_ITEM_WIDTH, NAV_ITEM_HEIGHT),
        );
        if *selected {
            ui.painter()
                .rect_filled(rect.shrink(3.0), CornerRadius::same(10), SELECTED());
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 2.0, rect.center().y - 11.0),
                    egui::pos2(rect.left() + 5.0, rect.center().y + 11.0),
                ),
                CornerRadius::same(2),
                GREEN,
            );
        }
        // The shipped rail paints a 20 px icon here; the slot is what the layout needs.
        let icon = egui::Rect::from_center_size(
            rect.center() - Vec2::new(0.0, 8.0),
            Vec2::splat(20.0),
        );
        ui.painter().rect_filled(
            icon,
            CornerRadius::same(5),
            if *selected { HOVER() } else { SURFACE() },
        );
        let font = nav_font(ui, label, rect.width() - 6.0);
        let color = if *selected { TEXT() } else { MUTED() };
        let measured = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap((*label).to_owned(), font.clone(), color))
            .size();
        ui.painter().text(
            rect.center() + Vec2::new(0.0, 15.0),
            egui::Align2::CENTER_CENTER,
            *label,
            font,
            color,
        );
        probe.text(
            format!("nav.label.{i}"),
            egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.center().y + 15.0),
                Vec2::new(NAV_ITEM_WIDTH - 6.0, measured.y),
            ),
            measured,
        );
        probe.contrast(format!("nav.label.{i}"), crate::theme::ink(color, !light()), CARD());
        probe.widget(format!("nav.item.{i}"), rect, true);
        y += NAV_ITEM_HEIGHT;
    }

    // A page beside the rail, so the screenshot shows the rail against real content.
    let page = egui::Rect::from_min_max(
        egui::pos2(rail.right() + 24.0, screen.top() + 24.0),
        egui::pos2(screen.right() - 24.0, screen.bottom() - 24.0),
    );
    ui.painter()
        .rect_filled(page, CornerRadius::same(RADIUS), CARD());
    probe.widget("page.content", page, false);
    probe.text(
        "page.title",
        egui::Rect::from_min_size(page.min + Vec2::new(20.0, 20.0), Vec2::new(220.0, 22.0)),
        Vec2::new(64.0, 20.0),
    );
    ui.painter().text(
        page.min + Vec2::new(20.0, 30.0),
        egui::Align2::LEFT_CENTER,
        "Home",
        egui::FontId::proportional(18.0),
        TEXT(),
    );
}

// ---------------------------------------------------------------------------
// Player Inventory Viewer (`src/ui/page_inventory.rs`)
// ---------------------------------------------------------------------------

/// The modal shell: dimmed page, card, title row, the five lazy category tabs.
fn inventory_shell(
    ui: &mut egui::Ui,
    probe: &mut Probe,
    title: &str,
    body: impl FnOnce(&mut egui::Ui, &mut Probe),
) {
    let screen = ui.ctx().viewport_rect();
    ui.painter().rect_filled(screen, CornerRadius::ZERO, BG());
    probe.widget("inventory.backdrop", screen, false);

    let modal = egui::Rect::from_center_size(screen.center(), Vec2::new(1120.0, 720.0));
    ui.painter()
        .rect_filled(modal, CornerRadius::same(12), CARD());
    ui.painter().rect_stroke(
        modal,
        CornerRadius::same(12),
        Stroke::new(1.0, BORDER()),
        egui::StrokeKind::Inside,
    );
    probe.widget("inventory.modal", modal, false);

    ui.scope_builder(
        egui::UiBuilder::new().max_rect(modal.shrink(16.0)),
        |ui| {
            ui.set_width(modal.width() - 32.0);
            ui.horizontal(|ui| {
                probe.label(
                    ui,
                    "inventory.title",
                    title,
                    egui::FontId::proportional(17.0),
                    TEXT(),
                    CARD(),
                );
                ui.add_space(6.0);
                probe.label(
                    ui,
                    "inventory.subtitle",
                    "Roblox · 50 per page",
                    egui::FontId::proportional(10.0),
                    MUTED(),
                    CARD(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    probe.button(
                        ui,
                        "inventory.close",
                        "Close",
                        Vec2::new(58.0, 26.0),
                        SURFACE(),
                        TEXT(),
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (i, category) in Category::ALL.iter().enumerate() {
                    let selected = i == 0;
                    probe.button(
                        ui,
                        format!("inventory.tab.{i}"),
                        category.label(),
                        Vec2::new(150.0, 28.0),
                        if selected { SELECTED() } else { SURFACE() },
                        if selected { TEXT() } else { MUTED() },
                    );
                }
            });
            ui.add_space(10.0);
            body(ui, probe);
        },
    );
}

pub fn inventory(ui: &mut egui::Ui, probe: &mut Probe) {
    inventory_shell(ui, probe, "Your inventory", |ui, probe| {
        let items = [
            ("Sparkle Fedora", "HAT"),
            ("Blue Hair", "HAIR"),
            ("Classic Shirt", "SHIRT"),
            ("Black Pants", "PANTS"),
            ("Round Shades", "FACE"),
            ("Wings", "BACK"),
            ("Sword", "GEAR"),
            ("Wave", "EMOTE"),
            ("Idle", "ANIMATION"),
            ("Run", "ANIMATION"),
            ("Walk", "ANIMATION"),
            ("Neck Fluff", "NECK"),
            ("Beanie", "HAT"),
            ("Backpack", "BACK"),
        ];
        // Same responsive column count as `page_inventory::grid`.
        let columns = (((ui.available_width() + 8.0) / 140.0).floor() as usize).max(1);
        for (row, chunk) in items.chunks(columns).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (column, (name, kind)) in chunk.iter().enumerate() {
                    inventory_card(ui, probe, row * columns + column, name, kind);
                }
            });
            ui.add_space(8.0);
        }
        probe.button(
            ui,
            "inventory.load_more",
            "Load more",
            Vec2::new(96.0, 28.0),
            SURFACE(),
            TEXT(),
        );
    });
}

/// One card: thumbnail slot, name, type badge. The real card paints a decoded 150 px texture
/// and falls back to the type label while one is on its way; the fixture stands in for the
/// texture with a two-tone tile, because it cannot reach Roblox.
fn inventory_card(ui: &mut egui::Ui, probe: &mut Probe, idx: usize, name: &str, kind: &str) {
    let frame = egui::Frame::new()
        .fill(SURFACE())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8));
    let inner = frame.show(ui, |ui| {
        ui.set_width(116.0);
        ui.vertical(|ui| {
            let (thumb, _) = ui.allocate_exact_size(Vec2::splat(108.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(thumb, CornerRadius::same(6), HOVER());
            if idx % 7 == 6 {
                // Not downloaded yet: the real card shows the type label here, never an icon.
                ui.painter().text(
                    thumb.center(),
                    egui::Align2::CENTER_CENTER,
                    kind,
                    egui::FontId::proportional(11.0),
                    MUTED(),
                );
            } else {
                // Stand-in for the decoded thumbnail: a soft two-tone tile.
                let tint = match idx % 5 {
                    0 => BLUE,
                    1 => GREEN,
                    2 => ORANGE,
                    3 => RED,
                    _ => RAIL(),
                };
                ui.painter().rect_filled(
                    thumb.shrink(6.0),
                    CornerRadius::same(5),
                    Color32::from_rgb(0x24, 0x2A, 0x38),
                );
                ui.painter().circle_filled(
                    thumb.center() - Vec2::new(0.0, 8.0),
                    30.0,
                    tint.gamma_multiply(0.55),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_center_size(
                        thumb.center() + Vec2::new(0.0, 30.0),
                        Vec2::new(64.0, 10.0),
                    ),
                    CornerRadius::same(3),
                    Color32::from_rgb(0x36, 0x3D, 0x4E),
                );
            }
            probe.widget(format!("inventory.card.{idx}.thumb"), thumb, false);
            ui.add_space(6.0);
            probe.label(
                ui,
                format!("inventory.card.{idx}.name"),
                name,
                egui::FontId::proportional(11.5),
                TEXT(),
                SURFACE(),
            );
            ui.add_space(2.0);
            probe.label(
                ui,
                format!("inventory.card.{idx}.type"),
                kind,
                egui::FontId::proportional(10.0),
                MUTED(),
                SURFACE(),
            );
        });
    });
    probe.widget(format!("inventory.card.{idx}"), inner.response.rect, false);
}

pub fn inventory_private(ui: &mut egui::Ui, probe: &mut Probe) {
    inventory_shell(ui, probe, "MysteryPlayer's inventory", |ui, probe| {
        ui.add_space(220.0);
        ui.vertical_centered(|ui| {
            probe.label(
                ui,
                "inventory.empty.title",
                "This player's inventory is private",
                egui::FontId::proportional(16.0),
                TEXT(),
                CARD(),
            );
            ui.add_space(4.0);
            probe.label(
                ui,
                "inventory.empty.detail",
                "Roblox only shows an inventory its owner made public.",
                egui::FontId::proportional(12.0),
                MUTED(),
                CARD(),
            );
        });
    });
}
