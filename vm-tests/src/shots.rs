//! Headless egui rendering for layout inspection (dev-only, never shipped).
//!
//! `creblox`'s own page functions cannot be linked here: `src/ui/*` needs `crate::app::Creblox`,
//! eframe and the macOS/Windows APIs the Linux VM does not have, so the GUI crate does not build
//! on this host at all. What this module does instead is run the *real* egui: the same 0.37 the
//! app uses lays out widgets, shapes and rasterizes glyphs, tessellates them exactly like eframe
//! does, and the meshes are drawn here by a small software triangle rasterizer. The screens in
//! `screens.rs` mirror the shipped pages (palette, spacing and widget shapes from
//! `src/theme.rs`); the layout checks below run on whatever the screens register.
//!
//! Nothing here is referenced by the `creblox` package or copied into a release archive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, Pos2, RawInput, Rect, TextureId, Vec2};

/// Where screenshots go. Override with `CREBLOX_SHOT_DIR` (for example on CI).
pub fn artifact_dir() -> PathBuf {
    std::env::var_os("CREBLOX_SHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts"))
}

/// A rendered frame: RGBA8, top-left origin.
pub struct Shot {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Shot {
    /// Writes `artifacts/<name>.png` and returns the path.
    pub fn save(&self, dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create the screenshot folder");
        let path = dir.join(format!("{}.png", self.name));
        image::save_buffer(
            &path,
            &self.pixels,
            self.width as u32,
            self.height as u32,
            image::ColorType::Rgba8,
        )
        .expect("write the screenshot");
        path
    }

    /// Distinct colours in the frame. A rasterizer that silently drew nothing would otherwise
    /// look like a perfectly uniform, perfectly "valid" screenshot.
    pub fn distinct_colors(&self) -> usize {
        let mut seen: HashMap<[u8; 3], ()> = HashMap::new();
        for px in self.pixels.chunks_exact(4) {
            seen.insert([px[0], px[1], px[2]], ());
        }
        seen.len()
    }
}

struct Texture {
    size: [usize; 2],
    /// Premultiplied sRGB, 0..=1 — the same space as egui's `Color32`.
    pixels: Vec<[f32; 4]>,
}

/// View of one screen, filled in by the fixture while it builds the frame.
#[derive(Default)]
pub struct Probe {
    pub widgets: Vec<Widget>,
    contrasts: Vec<(String, f32)>,
}

#[derive(Clone, Debug)]
pub struct Widget {
    pub id: String,
    pub rect: Rect,
    pub interactive: bool,
    /// Measured size of the text this widget draws, when it draws text.
    pub text: Option<Vec2>,
}

/// Minimum square hit target, WCAG 2.5.8.
pub const MIN_HIT: f32 = 24.0;

impl Probe {
    pub fn widget(&mut self, id: impl Into<String>, rect: Rect, interactive: bool) {
        self.widgets.push(Widget {
            id: id.into(),
            rect,
            interactive,
            text: None,
        });
    }

    pub fn text(&mut self, id: impl Into<String>, rect: Rect, measured: Vec2) {
        self.widgets.push(Widget {
            id: id.into(),
            rect,
            interactive: false,
            text: Some(measured),
        });
    }

    /// Records a contrast pair so a low-contrast label fails the suite instead of being spotted
    /// (or missed) by eye.
    pub fn contrast(&mut self, id: impl Into<String>, fg: Color32, bg: Color32) {
        self.contrasts
            .push((id.into(), contrast_ratio(fg, bg)));
    }

    /// A text label plus its measured size, recorded in one call.
    pub fn label(
        &mut self,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        text: impl Into<String>,
        font: egui::FontId,
        color: Color32,
        bg: Color32,
    ) -> egui::Response {
        let id = id.into();
        let text = text.into();
        let measured = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), color))
            .size();
        let resp = ui.add(egui::Label::new(egui::RichText::new(&text).font(font).color(color)));
        self.text(id.as_str(), resp.rect, measured);
        self.contrast(id, color, bg);
        resp
    }

    /// A button with a real hit target; records the rect and its text size.
    pub fn button(
        &mut self,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        text: &str,
        size: Vec2,
        fill: Color32,
        fg: Color32,
    ) -> egui::Response {
        let id = id.into();
        let font = egui::FontId::proportional(13.0);
        let measured = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), fg))
            .size();
        let resp = ui.add_sized(
            size,
            egui::Button::new(egui::RichText::new(text).font(font).color(fg)).fill(fill),
        );
        self.text(id.as_str(), resp.rect, measured);
        self.widget(id.as_str(), resp.rect, true);
        self.contrast(id, fg, fill);
        resp
    }

    /// Everything the layout rules object to, as sentences the test prints as-is.
    pub fn problems(&self, screen: Rect) -> Vec<String> {
        let mut out = Vec::new();
        for w in &self.widgets {
            if !w.rect.min.x.is_finite()
                || !w.rect.min.y.is_finite()
                || !w.rect.max.x.is_finite()
                || !w.rect.max.y.is_finite()
            {
                out.push(format!("{}: rect is not finite ({:?})", w.id, w.rect));
                continue;
            }
            if w.rect.width() < 0.5 || w.rect.height() < 0.5 {
                out.push(format!(
                    "{}: collapsed to {:.1}x{:.1}",
                    w.id,
                    w.rect.width(),
                    w.rect.height()
                ));
            }
            if !screen.expand(0.5).contains_rect(w.rect) {
                out.push(format!(
                    "{}: clipped by the window (rect {:?} vs screen {:?})",
                    w.id, w.rect, screen
                ));
            }
            if let Some(text) = w.text {
                if text.x > w.rect.width() + 0.5 || text.y > w.rect.height() + 0.5 {
                    out.push(format!(
                        "{}: text {:.1}x{:.1} does not fit its {:.1}x{:.1} box",
                        w.id,
                        text.x,
                        text.y,
                        w.rect.width(),
                        w.rect.height()
                    ));
                }
            }
            if w.interactive && (w.rect.width() < MIN_HIT || w.rect.height() < MIN_HIT) {
                out.push(format!(
                    "{}: hit target {:.1}x{:.1} is under {MIN_HIT:.0}px",
                    w.id,
                    w.rect.width(),
                    w.rect.height()
                ));
            }
        }

        for (i, a) in self.widgets.iter().enumerate() {
            for b in self.widgets.iter().skip(i + 1) {
                if a.id == b.id {
                    continue;
                }
                // A label inside its button, or a card inside the page, is nesting, not a
                // collision: only partially overlapping boxes are illegal.
                //
                // A control anchored to its parent's edge - the editor's corner handles sit
                // half on the picture - is nesting by id even though it straddles the border.
                let nested = a.rect.expand(0.5).contains_rect(b.rect)
                    || b.rect.expand(0.5).contains_rect(a.rect)
                    || b.id.starts_with(&format!("{}.", a.id))
                    || a.id.starts_with(&format!("{}.", b.id));
                if nested {
                    continue;
                }
                let overlap = a.rect.intersect(b.rect);
                if overlap.width() > 1.0 && overlap.height() > 1.0 {
                    out.push(format!(
                        "{} overlaps {} by {:.1}x{:.1}",
                        a.id,
                        b.id,
                        overlap.width(),
                        overlap.height()
                    ));
                }
            }
        }

        for (id, ratio) in &self.contrasts {
            if *ratio < 4.5 {
                out.push(format!("{id}: contrast {ratio:.2}:1 is under 4.5:1"));
            }
        }
        out
    }
}

/// WCAG relative-luminance contrast between two opaque colours.
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn channel(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn lum(c: Color32) -> f32 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (la, lb) = (lum(a), lum(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Offscreen egui: keeps the texture atlas across frames, like eframe's renderer does.
pub struct Harness {
    ctx: egui::Context,
    textures: HashMap<TextureId, Texture>,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        // The app installs a "Bold" family in `theme::install_fonts`; the harness has only the
        // default fonts, so alias it to the proportional chain. Without this every heading and
        // every **bold** run in real widget code (`font_section`, notes spans) draws tofu.
        let mut fonts = egui::FontDefinitions::default();
        if let Some(chain) = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
        {
            fonts
                .families
                .insert(egui::FontFamily::Name("Bold".into()), chain);
        }
        ctx.set_fonts(fonts);
        ctx.all_styles_mut(|style| {
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = crate::screens::BG();
            style.visuals.window_fill = crate::screens::CARD();
            style.visuals.override_text_color = Some(crate::screens::TEXT());
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
            style.spacing.button_padding = Vec2::new(12.0, 7.0);
            style.text_styles = [
                (
                    egui::TextStyle::Small,
                    egui::FontId::proportional(11.0),
                ),
                (egui::TextStyle::Body, egui::FontId::proportional(13.0)),
                (
                    egui::TextStyle::Button,
                    egui::FontId::proportional(13.0),
                ),
                (
                    egui::TextStyle::Heading,
                    egui::FontId::proportional(20.0),
                ),
                (
                    egui::TextStyle::Monospace,
                    egui::FontId::monospace(12.0),
                ),
            ]
            .into();
        });
        Self {
            ctx,
            textures: HashMap::new(),
        }
    }

    /// Lays out one screen at `size` points, rasterizes it at `ppp` and returns the pixels plus
    /// what the fixture registered for inspection.
    pub fn render(
        &mut self,
        name: &str,
        size: Vec2,
        ppp: f32,
        page: impl Fn(&mut egui::Ui, &mut Probe),
    ) -> (Shot, Probe) {
        // The palette the *real* widgets read comes from `ui.visuals().dark_mode`, so a light
        // render has to set the egui theme too - otherwise the fixture paints a white page while
        // the shipping controls still draw their dark-theme colours on top of it.
        let light = crate::screens::light();
        self.ctx.all_styles_mut(|style| {
            style.visuals = if light {
                egui::Visuals::light()
            } else {
                egui::Visuals::dark()
            };
            style.visuals.panel_fill = crate::screens::BG();
            style.visuals.window_fill = crate::screens::CARD();
            style.visuals.override_text_color = Some(crate::screens::TEXT());
            // `accent_color` reads the selection stroke, which `apply_theme` normally sets.
            style.visuals.selection.stroke.color = if light {
                crate::theme::BLUE
            } else {
                crate::theme::GREEN
            };
        });
        crate::theme::reset_palette_cache();
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        // egui 0.37 reads the scale factor from the viewport, not the frame input.
        input.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(ppp),
                ..Default::default()
            },
        );

        // A first, empty pass gives egui its fonts and default spacing; the second is the frame
        // that is saved, so nothing is half-initialized in the picture. The root ui has no
        // background of its own, so the page colour is painted first.
        let warm = self.ctx.run_ui(input.clone(), |ui| {
            ui.painter().rect_filled(
                ui.ctx().viewport_rect(),
                egui::CornerRadius::ZERO,
                crate::screens::BG(),
            );
        });
        self.absorb(warm.textures_delta);

        let mut probe = Probe::default();
        let mut full = self.ctx.run_ui(input, |ui| {
            ui.painter().rect_filled(
                ui.ctx().viewport_rect(),
                egui::CornerRadius::ZERO,
                crate::screens::BG(),
            );
            page(ui, &mut probe);
        });
        let delta = std::mem::take(&mut full.textures_delta);
        self.absorb(delta);
        let primitives = self.ctx.tessellate(std::mem::take(&mut full.shapes), ppp);
        drop(full);
        let pixels = self.rasterize(&primitives, size, ppp, crate::screens::BG());
        (
            Shot {
                name: name.to_string(),
                width: (size.x * ppp).round() as usize,
                height: (size.y * ppp).round() as usize,
                pixels,
            },
            probe,
        )
    }

    fn absorb(&mut self, mut delta: egui::TexturesDelta) {
        for (id, updates) in delta.set.drain() {
            for image in updates {
                let egui::ImageData::Color(color) = &image.image;
                let size = color.size;
                let floats: Vec<[f32; 4]> = color
                    .pixels
                    .iter()
                    .map(|c| {
                        let [r, g, b, a] = c.to_array();
                        [
                            r as f32 / 255.0,
                            g as f32 / 255.0,
                            b as f32 / 255.0,
                            a as f32 / 255.0,
                        ]
                    })
                    .collect();

                if std::env::var_os("CREBLOX_SHOT_DEBUG").is_some() {
                    let mid = (size[1] / 2) * size[0] + size[0] / 2;
                    eprintln!(
                        "texture {id:?} pos={:?} {}x{} corner={:?} middle={:?} opaque={}",
                        image.pos,
                        size[0],
                        size[1],
                        floats.first(),
                        floats.get(mid),
                        floats.iter().filter(|p| p[3] > 0.5).count()
                    );
                }

                match image.pos {
                    None => {
                        self.textures.insert(id, Texture { size, pixels: floats });
                    }
                    Some([x0, y0]) => {
                        // The font atlas grows by patches; blit them into the texture we hold.
                        let Some(texture) = self.textures.get_mut(&id) else {
                            continue;
                        };
                        let w = texture.size[0];
                        for row in 0..size[1] {
                            for col in 0..size[0] {
                                let (x, y) = (x0 + col, y0 + row);
                                if x >= w || y >= texture.size[1] {
                                    continue;
                                }
                                texture.pixels[y * w + x] = floats[row * size[0] + col];
                            }
                        }
                    }
                }
            }
        }
        for id in delta.free.drain() {
            self.textures.remove(&id);
        }
    }

    /// Software rasterizer for egui's tessellated meshes: barycentric interpolation of UV and
    /// vertex colour, bilinear texture sampling, premultiplied source-over blending.
    fn rasterize(
        &self,
        primitives: &[ClippedPrimitive],
        size: Vec2,
        ppp: f32,
        background: Color32,
    ) -> Vec<u8> {
        let w = (size.x * ppp).round() as usize;
        let h = (size.y * ppp).round() as usize;
        let bg = {
            let [r, g, b, a] = background.to_array();
            [
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ]
        };
        let mut canvas = vec![bg; w * h];

        for primitive in primitives {
            let Primitive::Mesh(mesh) = &primitive.primitive else {
                continue;
            };
            let texture = self.textures.get(&mesh.texture_id);
            let clip = primitive.clip_rect;
            let cx0 = (clip.min.x * ppp).floor().max(0.0) as i32;
            let cy0 = (clip.min.y * ppp).floor().max(0.0) as i32;
            let cx1 = (clip.max.x * ppp).ceil().min(w as f32) as i32;
            let cy1 = (clip.max.y * ppp).ceil().min(h as f32) as i32;
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }

            for tri in mesh.indices.chunks_exact(3) {
                let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                let Some(v0) = mesh.vertices.get(i0) else { continue };
                let Some(v1) = mesh.vertices.get(i1) else { continue };
                let Some(v2) = mesh.vertices.get(i2) else { continue };
                let p0 = Pos2::new(v0.pos.x * ppp, v0.pos.y * ppp);
                let p1 = Pos2::new(v1.pos.x * ppp, v1.pos.y * ppp);
                let p2 = Pos2::new(v2.pos.x * ppp, v2.pos.y * ppp);
                let area = edge(p0, p1, p2);
                if area.abs() < 1e-6 {
                    continue;
                }

                let min_x = p0.x.min(p1.x).min(p2.x).floor().max(cx0 as f32) as i32;
                let max_x = p0.x.max(p1.x).max(p2.x).ceil().min(cx1 as f32) as i32;
                let min_y = p0.y.min(p1.y).min(p2.y).floor().max(cy0 as f32) as i32;
                let max_y = p0.y.max(p1.y).max(p2.y).ceil().min(cy1 as f32) as i32;

                // A primitive can reference a texture this frame never uploaded (a picked
                // image, say). Draw it untinted instead of dropping the geometry.
                let white = Texture {
                    size: [1, 1],
                    pixels: vec![[1.0, 1.0, 1.0, 1.0]],
                };
                let texture = texture.unwrap_or(&white);

                let c0 = v0.color.to_array();
                let c1 = v1.color.to_array();
                let c2 = v2.color.to_array();
                let colors = [c0, c1, c2].map(|c| {
                    [
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        c[3] as f32 / 255.0,
                    ]
                });

                for y in min_y..max_y {
                    for x in min_x..max_x {
                        let px = x as f32 + 0.5;
                        let py = y as f32 + 0.5;
                        let w0 = edge(p1, p2, Pos2::new(px, py)) / area;
                        let w1 = edge(p2, p0, Pos2::new(px, py)) / area;
                        let w2 = edge(p0, p1, Pos2::new(px, py)) / area;
                        if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                            continue;
                        }
                        // Both colours are premultiplied, so every channel - alpha
                        // included - is the vertex colour times the texel. Skipping the
                        // alpha product turns each glyph cell into an opaque black box.
                        let uv = Pos2::new(
                            w0 * v0.uv.x + w1 * v1.uv.x + w2 * v2.uv.x,
                            w0 * v0.uv.y + w1 * v1.uv.y + w2 * v2.uv.y,
                        );
                        let tex = sample(texture, uv);
                        let mut src = [0.0f32; 4];
                        for (channel, value) in src.iter_mut().enumerate() {
                            let vertex = w0 * colors[0][channel]
                                + w1 * colors[1][channel]
                                + w2 * colors[2][channel];
                            *value = vertex * tex[channel];
                        }
                        if src[3] <= 0.0 {
                            continue;
                        }
                        let dst = &mut canvas[y as usize * w + x as usize];
                        for (channel, value) in src.iter().enumerate() {
                            dst[channel] = value + dst[channel] * (1.0 - src[3]);
                        }
                    }
                }
            }
        }

        let mut rgba = Vec::with_capacity(w * h * 4);
        for px in canvas {
            for channel in px {
                rgba.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        rgba
    }
}

/// Twice the signed area of the triangle (a, b, p).
fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Bilinear sample of a premultiplied texture with clamped edges.
fn sample(texture: &Texture, uv: Pos2) -> [f32; 4] {
    let w = texture.size[0];
    let h = texture.size[1];
    if w == 0 || h == 0 {
        return [1.0, 1.0, 1.0, 1.0];
    }
    let x = uv.x * w as f32 - 0.5;
    let y = uv.y * h as f32 - 0.5;
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let clamp = |v: f32, max: usize| v.clamp(0.0, (max - 1) as f32) as usize;
    let texel = |ix: usize, iy: usize| texture.pixels[iy * w + ix];
    let (x0c, x1c) = (clamp(x0, w), clamp(x0 + 1.0, w));
    let (y0c, y1c) = (clamp(y0, h), clamp(y0 + 1.0, h));
    let (t00, t10) = (texel(x0c, y0c), texel(x1c, y0c));
    let (t01, t11) = (texel(x0c, y1c), texel(x1c, y1c));
    let mut out = [0.0f32; 4];
    for c in 0..4 {
        let top = t00[c] * (1.0 - fx) + t10[c] * fx;
        let bottom = t01[c] * (1.0 - fx) + t11[c] * fx;
        out[c] = top * (1.0 - fy) + bottom * fy;
    }
    out
}
