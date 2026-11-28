//! The Player Inventory Viewer: one modal, five lazy tabs, a culled grid.
//!
//! The rules that matter for performance live here and in [`crate::inventory`]:
//! * a tab is fetched only when it is opened, and only if it is not cached (LRU, 8 tabs);
//! * the grid is `ScrollArea::show_rows`, so only the visible rows are laid out, whatever
//!   the inventory size;
//! * thumbnails are started for visible cards only, three per frame, so a scroll cannot
//!   enqueue thousands of texture uploads at once. The launcher's RAM budget
//!   ([`crate::app`]'s `enforce_ram_cache_budget`) still owns eviction.
//!
//! A private inventory is a terminal state: the error is cached, so the tab never retries.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

use crate::inventory::{
    Category, FetchError, InventoryItem, Page, TabCache, TabEntry, ThumbKind, needs_fetch,
};
use crate::theme::{font_meta, pal};
use crate::types::Action;

/// Thumbnails per metadata request and image batch. The grid only asks for the cards the
/// viewport actually shows, so one `thumbnails/v1` call covers a screenful regardless of how
/// far the inventory was scrolled.
pub const THUMBS_PER_BATCH: usize = 24;
/// Decoded thumbnails kept alive. Beyond this the oldest texture is dropped; scrolling back
/// re-fetches, which is cheaper than an unbounded texture atlas on a RAM-budgeted launcher.
pub const THUMB_TEXTURE_CAP: usize = 320;
/// Tabs kept in memory (most recent last). One user's five tabs fit with room for a second.
pub const TAB_CACHE_CAPACITY: usize = 8;

const CARD_W: f32 = 132.0;
const CARD_H: f32 = 178.0;
const THUMB: f32 = 108.0;
const GAP: f32 = 8.0;

type FetchResult = Result<Page, FetchError>;

/// One open inventory. Lives in `Creblox` and is drawn as a modal over every page.
pub struct InventoryView {
    pub user_id: u64,
    pub user_name: String,
    pub is_self: bool,
    active: Category,
    cache: TabCache,
    /// `(user, category, is a load-more, channel)` of the request in flight.
    in_flight: Option<(u64, Category, bool, Receiver<FetchResult>)>,
    /// When a rate-limited tab may be tried again.
    retry_at: Option<Instant>,
    /// Decoded thumbnails, keyed by asset id.
    thumbs: HashMap<u64, egui::TextureHandle>,
    /// Ids already requested (in flight, done or failed): an asset is asked for once.
    thumb_requested: HashSet<u64>,
    /// Insertion order, so the texture cap can drop the oldest one.
    thumb_order: Vec<u64>,
    /// The thumbnail batch in flight, if any.
    thumb_rx: Option<Receiver<Vec<(u64, egui::ColorImage)>>>,
    cookie: Option<String>,
    pub open: bool,
}

impl InventoryView {
    pub fn new(user_id: u64, user_name: String, is_self: bool, cookie: Option<String>) -> Self {
        Self {
            user_id,
            user_name,
            is_self,
            active: Category::Accessories,
            cache: TabCache::new(TAB_CACHE_CAPACITY),
            in_flight: None,
            retry_at: None,
            thumbs: HashMap::new(),
            thumb_requested: HashSet::new(),
            thumb_order: Vec::new(),
            thumb_rx: None,
            cookie,
            open: true,
        }
    }

    /// Uploads the thumbnails a worker finished decoding, and keeps the atlas bounded.
    fn poll_thumbs(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.thumb_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(batch) => {
                for (id, image) in batch {
                    let texture = ctx.load_texture(
                        format!("creblox-inventory-thumb-{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    if self.thumbs.insert(id, texture).is_none() {
                        self.thumb_order.push(id);
                    }
                }
                while self.thumb_order.len() > THUMB_TEXTURE_CAP {
                    let oldest = self.thumb_order.remove(0);
                    self.thumbs.remove(&oldest);
                }
                ctx.request_repaint();
            }
            Err(TryRecvError::Empty) => {
                self.thumb_rx = Some(rx);
                ctx.request_repaint_after(Duration::from_millis(60));
            }
            Err(TryRecvError::Disconnected) => {}
        }
    }

    /// Starts one thumbnail batch for the cards that are on screen right now.
    fn request_thumbs(
        &mut self,
        ctx: &egui::Context,
        visible: &[(u64, ThumbKind)],
    ) {
        if self.thumb_rx.is_some() {
            return;
        }
        let mut batch: Vec<(u64, ThumbKind)> = Vec::new();
        for (id, kind) in visible {
            if *id == 0 || self.thumbs.contains_key(id) {
                continue;
            }
            if !self.thumb_requested.insert(*id) {
                continue;
            }
            batch.push((*id, *kind));
            if batch.len() >= THUMBS_PER_BATCH {
                break;
            }
        }
        if batch.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("creblox-inventory-thumbs".to_string())
            .spawn(move || {
                let _ = tx.send(crate::api::fetch_inventory_thumbnails(batch));
            });
        if spawned.is_ok() {
            self.thumb_rx = Some(rx);
            ctx.request_repaint();
        }
    }

    /// Reads the worker's answer. Called once per frame while the viewer is open.
    pub fn poll(&mut self) {
        let Some((user_id, category, more, rx)) = self.in_flight.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => self.apply_result(user_id, category, more, result),
            Err(TryRecvError::Empty) => {
                self.in_flight = Some((user_id, category, more, rx));
            }
            Err(TryRecvError::Disconnected) => {
                self.apply_result(
                    user_id,
                    category,
                    more,
                    Err(FetchError::Other("request cancelled".to_string())),
                );
            }
        }
    }

    fn apply_result(&mut self, user_id: u64, category: Category, more: bool, result: FetchResult) {
        if user_id != self.user_id {
            return;
        }
        let key = (user_id, category);
        match result {
            Ok(page) => {
                let entry = self.cache.get_mut(key);
                let mut merged = match (more, entry) {
                    (true, Some(existing)) => existing.clone(),
                    _ => TabEntry::default(),
                };
                // A load-more must not repeat what is already on screen.
                let mut seen: HashSet<u64> = merged.items.iter().map(|i| i.id).collect();
                for item in page.items {
                    if seen.insert(item.id) {
                        merged.items.push(item);
                    }
                }
                merged.cursors = page.cursors;
                merged.complete = page.complete;
                merged.error = None;
                self.cache.insert(key, merged);
            }
            Err(err) => {
                if err.retryable() {
                    if let FetchError::RateLimited { retry_after_secs } = err {
                        self.retry_at = Some(Instant::now() + Duration::from_secs(retry_after_secs));
                        // Keep what is already on screen; the tab shows a quiet notice.
                        let mut entry = self
                            .cache
                            .get_mut(key)
                            .cloned()
                            .unwrap_or_default();
                        entry.error = Some(err);
                        self.cache.insert(key, entry);
                    }
                } else {
                    // Private and hard errors are terminal: caching them is what stops a
                    // retry on every frame (and every tab re-open).
                    self.cache.insert(
                        key,
                        TabEntry {
                            error: Some(err),
                            ..TabEntry::default()
                        },
                    );
                }
            }
        }
    }

    /// Starts the active tab's first page, unless it is cached, in flight or backing off.
    fn ensure_active(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let key = (self.user_id, self.active);
        let backing_off = self
            .cache
            .get_mut(key)
            .map(|entry| matches!(entry.error, Some(FetchError::RateLimited { .. })))
            .unwrap_or(false);
        if backing_off {
            match self.retry_at {
                Some(at) if Instant::now() < at => return,
                Some(_) => {
                    // The window has passed: drop the marker and try this tab once more.
                    self.retry_at = None;
                    self.cache.remove(key);
                }
                None => return,
            }
        }
        if needs_fetch(&mut self.cache, key, None) {
            self.start(false);
        }
    }

    /// One worker request for `more` pages of the active tab.
    fn start(&mut self, more: bool) {
        if self.in_flight.is_some() {
            return;
        }
        let key = (self.user_id, self.active);
        let cursors = if more {
            let Some(entry) = self.cache.get_mut(key) else {
                return;
            };
            if entry.complete || entry.error.is_some() {
                return;
            }
            entry.cursors.clone()
        } else {
            Vec::new()
        };
        let (tx, rx) = mpsc::channel();
        let user_id = self.user_id;
        let category = self.active;
        let cookie = self.cookie.clone();
        let spawned = std::thread::Builder::new()
            .name("creblox-inventory".to_string())
            .spawn(move || {
                let result = crate::api::fetch_inventory_tab(
                    user_id,
                    category,
                    cursors,
                    cookie.as_deref(),
                    1,
                );
                let _ = tx.send(result);
            });
        if spawned.is_ok() {
            self.in_flight = Some((user_id, category, more, rx));
        }
    }

    /// Draws the modal and returns what the user clicked.
    pub fn render(&mut self, ctx: &egui::Context) -> Vec<Action> {
        let mut acts: Vec<Action> = Vec::new();
        if !self.open {
            return acts;
        }
        self.poll();
        self.ensure_active();
        if self.in_flight.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if let Some(at) = self.retry_at {
            let wait = at.saturating_duration_since(Instant::now());
            ctx.request_repaint_after(wait.min(Duration::from_millis(500)));
        }

        self.poll_thumbs(ctx);
        let screen = ctx.input(|i| i.content_rect());
        // The scrim belongs on the *middle* layer: above the page, strictly below this
        // foreground modal. Painted on the foreground layer it was ordered against the modal
        // itself, so it dimmed the inventory's own card - and every card inside it.
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::unique("creblox-inventory-scrim"),
        ))
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(96));

        let Self {
            user_name,
            is_self,
            active,
            cache,
            thumbs,
            in_flight,
            open,
            user_id,
            ..
        } = self;
        // Cards the grid actually laid out this frame; their thumbnails are requested after
        // the modal is built, one batch at a time.
        let mut visible: Vec<(u64, ThumbKind)> = Vec::new();
        let key = (*user_id, *active);
        let mut tab_changed = false;
        let mut load_more = false;
        let mut close = false;

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }

        let width = (screen.width() - 96.0).clamp(420.0, 1120.0);
        let height = (screen.height() - 96.0).clamp(320.0, 760.0);
        egui::Area::new(egui::Id::unique("creblox-inventory"))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let p = pal(ui);
                egui::Frame::new()
                    .fill(p.card_bg)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 32.0);
                        ui.set_height(height - 32.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{}'s inventory",
                                    if *is_self { "Your" } else { user_name }
                                ))
                                .size(17.0)
                                .strong()
                                .color(p.text),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Roblox · 50 per page")
                                    .font(font_meta())
                                    .color(p.muted),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button(RichText::new("Close").size(12.0)).clicked() {
                                        close = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            for category in Category::ALL {
                                let selected = category == *active;
                                let text = RichText::new(category.label())
                                    .size(12.0)
                                    .color(if selected { p.text } else { p.muted });
                                let button = egui::Button::new(text)
                                    .fill(if selected { p.selected_bg } else { p.surface_bg })
                                    .stroke(Stroke::new(1.0, if selected { p.border } else { p.surface_bg }))
                                    .corner_radius(7);
                                if ui.add(button).clicked() && !selected {
                                    *active = category;
                                    tab_changed = true;
                                }
                            }
                        });
                        ui.add_space(10.0);

                        let entry = cache.get_mut(key);
                        body(
                            ui,
                            entry,
                            in_flight.is_some(),
                            thumbs,
                            &mut visible,
                            *is_self,
                            &mut acts,
                        );

                        if let Some(entry) = cache.get_mut(key) {
                            if !entry.complete
                                && entry.error.is_none()
                                && !entry.items.is_empty()
                                && in_flight.is_none()
                            {
                                ui.add_space(6.0);
                                if ui
                                    .button(RichText::new("Load more").size(12.0))
                                    .clicked()
                                {
                                    load_more = true;
                                }
                            }
                        }

                    });
            });

        if close {
            *open = false;
            return acts;
        }
        self.request_thumbs(ctx, &visible);
        if tab_changed {
            self.start(false);
            ctx.request_repaint();
        }
        if load_more {
            self.start(true);
            ctx.request_repaint();
        }
        acts
    }

}

/// The tab's body: a private/error state, a loading line, or the culled grid.
fn body(
    ui: &mut egui::Ui,
    entry: Option<&mut TabEntry>,
    loading: bool,
    thumbs: &HashMap<u64, egui::TextureHandle>,
    visible: &mut Vec<(u64, ThumbKind)>,
    is_self: bool,
    acts: &mut Vec<Action>,
) {
    let p = pal(ui);
    let Some(entry) = entry else {
        centered_note(ui, "Loading inventory...", "Fetching the first page", &p);
        return;
    };
    if let Some(err) = &entry.error {
        match err {
            FetchError::Private => centered_note(
                ui,
                "This player's inventory is private",
                "Roblox only shows an inventory its owner made public.",
                &p,
            ),
            FetchError::RateLimited { .. } => centered_note(
                ui,
                "Roblox is limiting requests",
                "The viewer backs off instead of retrying in a loop.",
                &p,
            ),
            FetchError::Other(e) => {
                centered_note(ui, "Could not load this inventory", e, &p)
            }
        }
        return;
    }
    if entry.items.is_empty() {
        if loading {
            centered_note(ui, "Loading inventory...", "The first page is on its way", &p);
        } else {
            centered_note(
                ui,
                "Nothing in this category",
                "Roblox returned no items for these asset types.",
                &p,
            );
        }
        return;
    }
    grid(ui, &entry.items, thumbs, visible, is_self, acts);
}

fn centered_note(ui: &mut egui::Ui, title: &str, detail: &str, p: &crate::theme::Palette) {
    ui.add_space(ui.available_height() * 0.32);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(16.0).strong().color(p.text));
        ui.add_space(4.0);
        ui.label(RichText::new(detail).size(12.0).color(p.muted));
    });
}

/// The virtualised grid: `show_rows` lays out only the rows the viewport shows.
fn grid(
    ui: &mut egui::Ui,
    items: &[InventoryItem],
    thumbs: &HashMap<u64, egui::TextureHandle>,
    visible: &mut Vec<(u64, ThumbKind)>,
    is_self: bool,
    acts: &mut Vec<Action>,
) {
    let p = pal(ui);
    let columns = (((ui.available_width() + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let rows = items.len().div_ceil(columns);
    let row_h = CARD_H + GAP;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_h, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for column in 0..columns {
                        let Some(item) = items.get(row * columns + column) else {
                            break;
                        };
                        // Only the cards actually laid out are asked for a thumbnail.
                        visible.push((item.id, ThumbKind::of(item)));
                        card(ui, item, thumbs.get(&item.id), is_self, &p, acts);
                    }
                });
                ui.add_space(GAP);
            }
        });
}

fn card(
    ui: &mut egui::Ui,
    item: &InventoryItem,
    thumb: Option<&egui::TextureHandle>,
    is_self: bool,
    p: &crate::theme::Palette,
    acts: &mut Vec<Action>,
) {
    let frame = egui::Frame::new()
        .fill(p.surface_bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8));
    let inner = frame.show(ui, |ui| {
        ui.set_width(CARD_W - 16.0);
        ui.vertical(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(Vec2::splat(THUMB), Sense::hover());
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(6),
                p.placeholder,
            );
            match thumb {
                // A decoded texture: no per-card HTTP request, no broken-image glyph.
                Some(tex) => {
                    ui.painter().image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                // Not downloaded yet (or its thumbnail is unavailable): the type, quietly.
                None => {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        item.type_label(),
                        egui::FontId::proportional(11.0),
                        p.muted,
                    );
                }
            }
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new(&item.name)
                        .size(11.5)
                        .color(p.text),
                )
                .truncate(),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(item.type_label())
                    .font(font_meta())
                    .color(p.muted),
            );
        });
    });

    let rect = inner.response.rect;
    if ui.is_rect_visible(rect) {
        let resp = ui
            .interact(rect, ui.scope_id().with(("inv-card", item.id)), Sense::click())
            .on_hover_text(&item.name)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            acts.push(Action::OpenUrl(item.page_url()));
        }
        resp.context_menu(|ui| {
            if ui.button("Open on Roblox").clicked() {
                acts.push(Action::OpenUrl(item.page_url()));
                ui.close();
            }
            if ui.button("Copy Asset ID").clicked() {
                ui.ctx().copy_text(item.id.to_string());
                ui.close();
            }
            if is_self && !item.is_badge {
                if ui.button("Try On").clicked() {
                    acts.push(Action::OpenUrl(format!(
                        "{}{}",
                        item.page_url(),
                        "/#!/?tryOn=1"
                    )));
                    ui.close();
                }
            }
        });
    }
}
