//! Roblox-accurate rendering core for the Avatar Editor's 3D viewport.
//!
//! Roblox's own `avatar-3d` viewer is a thin client over the same assets we download: an OBJ,
//! an MTL and a texture set, rendered with the camera the thumbnail service reports. This
//! module is the part of that viewer that decides how it looks, kept free of eframe and the
//! network so `vm-tests/` can parse materials and render a real viewport PNG.
//!
//! Four things it gets right that a hand-rolled approximation gets wrong:
//!
//! * **Alpha.** Roblox hair, eyebrows, faces and layered clothing are cut-out textures whose
//!   shape is only in the PNG's alpha channel, and the MTL can scale it with `d`, `Tr` or
//!   `map_d`. Texels under the cutout threshold are dropped; the rest blend over what is
//!   behind them, so hair is strands rather than a slab and a face decal does not turn into a
//!   black rectangle.
//! * **`Kd` × `map_Kd`.** Body parts are flat `Kd` materials, clothing and faces are textured,
//!   and Roblox tints a texture with its `Kd`. Both are multiplied, never one replacing the
//!   other.
//! * **Framing.** The thumbnail metadata carries `camera` (`position`, `direction`, `fov`) and
//!   `aabb`. The default view is derived from those, not from a guessed distance, so the
//!   avatar sits in the frame exactly as it does on the website.
//! * **Lighting and edges.** A neutral key, a cool fill and a rim light over a hemisphere
//!   ambient, rendered at twice the display size and box-filtered down, so skin and dark hair
//!   keep their shape and edges are clean.

use crate::egui::{Color32, ColorImage};

/// One texture: the colour and the alpha Roblox shipped, unmodified.
pub type Tex = ColorImage;

/// How a material's alpha is interpreted. Roblox writes this as `rbx_alphamode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    /// Alpha is a cut-out: below the threshold is invisible, above it is solid.
    Overlay,
    /// Alpha is a blend weight.
    Transparent,
}

/// One `newmtl` block, which is all the MTL really says.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// `Kd` diffuse colour, whites when the file omits it.
    pub kd: [f32; 3],
    /// `map_Kd`: the colour texture hash.
    pub texture: Option<String>,
    /// `map_d`: an alpha texture hash, multiplied into the colour texture's alpha.
    pub alpha_texture: Option<String>,
    /// `d` (or `Tr`, which is its inverse). 1.0 when absent.
    pub alpha: f32,
    pub mode: AlphaMode,
    /// `doubleSided`-style hints some exporters add; Roblox leaves faces single sided.
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: String::new(),
            kd: [1.0, 1.0, 1.0],
            texture: None,
            alpha_texture: None,
            alpha: 1.0,
            mode: AlphaMode::Opaque,
            double_sided: false,
        }
    }
}

/// Parses a Roblox `.mtl`.
///
/// Roblox writes `Kd`, `map_Kd`, optionally `map_d`, `d`/`Tr`, and its own `rbx_alphamode`
/// and `rbx_alphaedge` hints. Unknown keys (`Ka`, `Ks`, `Ns`, `illum`, `map_bump`) are ignored
/// rather than fatal: a stricter parser would reject real files.
pub fn parse_mtl(bytes: &[u8]) -> Vec<Material> {
    let text = String::from_utf8_lossy(bytes);
    let mut out: Vec<Material> = Vec::new();
    let mut current: Option<Material> = None;

    // A material's fields arrive in any order, so each one is applied to the block being read
    // and the block is pushed when the next `newmtl` (or the end of the file) arrives.
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();
        match key.to_ascii_lowercase().as_str() {
            "newmtl" => {
                if let Some(done) = current.take() {
                    out.push(done);
                }
                current = Some(Material {
                    name: rest.join(" "),
                    ..Material::default()
                });
            }
            "kd" => {
                if let Some(m) = current.as_mut() {
                    if rest.len() >= 3 {
                        m.kd = [
                            rest[0].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
                            rest[1].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
                            rest[2].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
                        ];
                    }
                }
            }
            "d" => {
                if let Some(m) = current.as_mut() {
                    if let Some(v) = rest.first().and_then(|v| v.parse::<f32>().ok()) {
                        m.alpha = v.clamp(0.0, 1.0);
                    }
                }
            }
            // `-halo`-style options can precede the filename; the hash is the last token.
            "tr" => {
                if let Some(m) = current.as_mut() {
                    if let Some(v) = rest
                        .iter()
                        .rev()
                        .find_map(|v| v.parse::<f32>().ok())
                    {
                        m.alpha = (1.0 - v).clamp(0.0, 1.0);
                    }
                }
            }
            "map_kd" => {
                if let Some(m) = current.as_mut() {
                    if let Some(name) = rest.last() {
                        m.texture = Some(strip_options(name).to_string());
                    }
                }
            }
            "map_d" => {
                if let Some(m) = current.as_mut() {
                    if let Some(name) = rest.last() {
                        m.alpha_texture = Some(strip_options(name).to_string());
                    }
                }
            }
            "rbx_alphamode" => {
                if let Some(m) = current.as_mut() {
                    m.mode = match rest.first().copied().unwrap_or("").to_ascii_lowercase().as_str()
                    {
                        "transparent" => AlphaMode::Transparent,
                        "overlay" | "cutout" => AlphaMode::Overlay,
                        _ => AlphaMode::Opaque,
                    };
                }
            }
            "rbx_doublesided" | "doublesided" => {
                if let Some(m) = current.as_mut() {
                    let v = rest.first().copied().unwrap_or("true").to_ascii_lowercase();
                    m.double_sided = matches!(v.as_str(), "true" | "1" | "on");
                }
            }
            _ => {}
        }
    }
    if let Some(done) = current.take() {
        out.push(done);
    }
    out
}

/// `map_Kd -s 1 1 1 30DAY-…-Png` → the hash.
fn strip_options(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

/// A texture texel with its alpha, as the rasteriser needs it.
pub fn texel(tex: &Tex, u: f32, v: f32) -> [u8; 4] {
    let [tw, th] = tex.size;
    if tw == 0 || th == 0 {
        return [255, 255, 255, 255];
    }
    // Wrap, then sample bilinearly: a 150 px Roblox texture magnified over a 700 px viewport
    // is the normal case, and nearest-neighbour turns every face into visible blocks.
    let x = (u - u.floor()) * tw as f32 - 0.5;
    let y = (v - v.floor()) * th as f32 - 0.5;
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let at = |ix: i32, iy: i32| -> [u8; 4] {
        let px = ix.rem_euclid(tw as i32) as usize;
        let py = iy.rem_euclid(th as i32) as usize;
        let c = tex.pixels[py * tw + px];
        [c.r(), c.g(), c.b(), c.a()]
    };
    let (x0, y0) = (x0 as i32, y0 as i32);
    let p00 = at(x0, y0);
    let p10 = at(x0 + 1, y0);
    let p01 = at(x0, y0 + 1);
    let p11 = at(x0 + 1, y0 + 1);
    let mut out = [0u8; 4];
    for i in 0..4 {
        let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
        let bottom = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
        out[i] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Below this a texel is not drawn at all (hair strands, cut-out faces).
pub const ALPHA_CUTOUT: f32 = 0.35;

/// The camera the metadata asked for, flattened into what a rasteriser needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov: f32,
    pub focal: f32,
}

/// What the `avatar-3d` metadata says about framing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    /// Vertical field of view in degrees, as the service reports it.
    pub fov: f32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}

/// Default vertical FOV when the metadata does not carry one. Roblox frames the whole avatar
/// from the feet to the top of the head in a square thumbnail; this is the angle that does it
/// at the default distance.
pub const DEFAULT_FOV: f32 = 30.0;

fn vec3(node: &serde_json::Value) -> Option<[f32; 3]> {
    match node {
        serde_json::Value::Array(items) => {
            if items.len() < 3 {
                return None;
            }
            Some([
                items[0].as_f64()? as f32,
                items[1].as_f64()? as f32,
                items[2].as_f64()? as f32,
            ])
        }
        serde_json::Value::Object(map) => {
            // Roblox writes vectors as {"x":…,"y":…,"z":…}; older renders use "X"/"Y"/"Z".
            let get = |names: [&str; 2]| -> Option<f32> {
                names
                    .iter()
                    .find_map(|n| map.get(*n).and_then(|v| v.as_f64()))
                    .map(|v| v as f32)
            };
            Some([get(["x", "X"])?, get(["y", "Y"])?, get(["z", "Z"])?])
        }
        _ => None,
    }
}

/// Reads `camera` and `aabb` out of the `avatar-3d` manifest.
///
/// The two are independent: a manifest with a camera but no AABB still frames correctly, and
/// one with neither returns `None` so the caller keeps its own fallback framing.
pub fn parse_framing(json: &serde_json::Value) -> Option<Framing> {
    let cam = &json["camera"];
    let eye = vec3(&cam["position"]);
    let dir = vec3(&cam["direction"]);
    let aabb_min = vec3(&json["aabb"]["min"]);
    let aabb_max = vec3(&json["aabb"]["max"]);

    let fov = cam["fov"]
        .as_f64()
        .map(|v| v as f32)
        .or_else(|| {
            // Some renders report the focal length instead.
            cam["focalLength"].as_f64().map(|f| {
                (2.0 * (f as f32).atan().to_degrees()).clamp(5.0, 90.0)
            })
        })
        .filter(|f| *f > 1.0 && *f < 170.0)
        .unwrap_or(DEFAULT_FOV);

    let target = match (aabb_min, aabb_max) {
        (Some(lo), Some(hi)) => [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ],
        _ => *eye.as_ref()?,
    };
    let (eye, dir) = (eye?, dir?);
    let target = if aabb_min.is_some() && aabb_max.is_some() {
        target
    } else {
        // No AABB: the direction is all we have, so look along it from the reported position.
        [eye[0] + dir[0], eye[1] + dir[1], eye[2] + dir[2]]
    };

    Some(Framing {
        eye,
        target,
        fov,
        aabb_min: aabb_min.unwrap_or(target),
        aabb_max: aabb_max.unwrap_or(target),
    })
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The orbit the user drags: an angle around the AABB centre, in the metadata's own units.
///
/// `yaw`/`pitch` are applied *on top of* the framing the service reported, so the default view
/// is Roblox's and dragging starts from there instead of jumping to a different camera.
pub fn orbit_camera(framing: Option<&Framing>, orbit: Orbit, height: f32) -> Camera {
    let (yaw, pitch, zoom) = (orbit.yaw, orbit.pitch, orbit.zoom);
    let (target, base_eye, fov) = match framing {
        Some(f) => {
            let centre = [
                (f.aabb_min[0] + f.aabb_max[0]) * 0.5,
                (f.aabb_min[1] + f.aabb_max[1]) * 0.5,
                (f.aabb_min[2] + f.aabb_max[2]) * 0.5,
            ];
            // Use the AABB centre, not the camera's own target: orbiting anything else makes
            // the avatar swing away from the pointer.
            let centre = if (f.eye[0] - centre[0]).abs() + (f.eye[1] - centre[1]).abs() > 1e-4 {
                centre
            } else {
                f.target
            };
            (centre, f.eye, f.fov)
        }
        None => {
            // No metadata: frame a 2-unit-tall avatar the way a square thumbnail does, with a
            // little headroom above the tallest hair.
            let dist = height / (2.0 * (DEFAULT_FOV.to_radians() * 0.5).tan()) * 1.14;
            ([0.0, 0.0, 0.0], [0.0, 0.0, dist], DEFAULT_FOV)
        }
    };

    let offset = [base_eye[0] - target[0], base_eye[1] - target[1], base_eye[2] - target[2]];
    let base = norm(offset);
    let dist = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2])
        .sqrt()
        .max(1e-3);
    let base_yaw = base[0].atan2(base[2]);
    let base_pitch = base[1].clamp(-1.0, 1.0).asin();

    let r = dist / zoom.clamp(0.35, 3.0);
    let y = base_yaw + yaw;
    let p = (base_pitch + pitch).clamp(-1.45, 1.45);
    let eye = [
        target[0] + r * p.cos() * y.sin(),
        target[1] + r * p.sin(),
        target[2] + r * p.cos() * y.cos(),
    ];
    Camera {
        eye,
        target,
        fov,
        // Filled in by the rasteriser, which knows the pixel height.
        focal: 0.0,
    }
}

/// Moves a framing from studs into the model's normalised space, where the avatar is 2 units
/// tall and centred on the origin. Scaling and translating a camera is angle-preserving, so
/// the FOV and the orbit still match what the service reported.
pub fn normalize_framing(f: &Framing, center: [f32; 3], scale: f32) -> Framing {
    let map = |p: [f32; 3]| {
        [
            (p[0] - center[0]) * scale,
            (p[1] - center[1]) * scale,
            (p[2] - center[2]) * scale,
        ]
    };
    Framing {
        eye: map(f.eye),
        target: map(f.target),
        fov: f.fov,
        aabb_min: map(f.aabb_min),
        aabb_max: map(f.aabb_max),
    }
}

/// Horizontal/vertical focal length in pixels for a vertical FOV.
pub fn focal_px(fov_deg: f32, height_px: f32) -> f32 {
    (height_px * 0.5) / (fov_deg.to_radians() * 0.5).tan().max(1e-4)
}

/// Three-point studio lighting, the way Roblox's web viewer lights an avatar.
///
/// A soft neutral key from the upper front-left, a cool fill from the lower right and a rim
/// from behind, over a hemisphere ambient that keeps dark hair from going flat black.
pub fn shade(normal: [f32; 3], view: [f32; 3]) -> [f32; 3] {
    let n = norm(normal);
    // Key light: slightly warm, high and to the left, the classic product-shot angle.
    let key = norm([-0.45, 0.72, 0.62]);
    // Fill: cool and low from the right, a third of the key's strength.
    let fill = norm([0.72, 0.05, 0.55]);
    // Rim: from behind and above, to separate dark hair from the backdrop.
    let rim = norm([0.15, 0.45, -0.88]);
    let back = (-dot(n, norm(view))).max(0.0);

    let ambient = 0.34 + 0.13 * (n[1] * 0.5 + 0.5);
    let mut out = [ambient, ambient, ambient * 1.02];
    let kd = dot(n, key).max(0.0);
    out[0] += kd * 0.66;
    out[1] += kd * 0.64;
    out[2] += kd * 0.60;
    let fd = dot(n, fill).max(0.0);
    out[0] += fd * 0.16;
    out[1] += fd * 0.19;
    out[2] += fd * 0.24;
    let rd = dot(n, rim).max(0.0) * back;
    out[0] += rd * 0.22;
    out[1] += rd * 0.23;
    out[2] += rd * 0.26;
    out
}

/// One triangle ready to rasterise, in world space.
pub struct Tri<'a> {
    pub pos: [[f32; 3]; 3],
    pub vertex_normals: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    /// Per-vertex colour: Roblox's OBJ carries RGBA vertex colours and they are multiplied by
    /// the material's `Kd`. Untextured faces use this directly.
    pub color: [Color32; 3],
    pub texture: Option<&'a Tex>,
    /// Material `Kd`, multiplied into the texture as well.
    pub kd: [f32; 3],
    /// Material opacity (`d`) times the alpha texture, if any.
    pub alpha: f32,
    pub mode: AlphaMode,
    pub double_sided: bool,
}

/// The user's drag: an orbit around the framing the service reported.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Orbit {
    pub fn new(yaw: f32, pitch: f32, zoom: f32) -> Self {
        Self { yaw, pitch, zoom }
    }
}

/// The depth-tested, alpha-aware software rasteriser.
pub struct Raster<'a> {
    pub pixels: &'a mut [Color32],
    pub zbuf: &'a mut [f32],
    pub width: usize,
    pub height: usize,
    pub camera: Camera,
    pub focal: f32,
}

impl Raster<'_> {
    /// Draws one triangle, blending it over whatever is already there.
    ///
    /// The depth test is per pixel, so an item can be composited over the body without a
    /// painter's-order guess. Cut-out texels are skipped entirely rather than blended, which
    /// is what makes hair read as strands instead of a translucent slab.
    pub fn triangle(&mut self, tri: &Tri<'_>) {
        let (w, h) = (self.width as f32, self.height as f32);
        let cx = w * 0.5;
        let cy = h * 0.5;
        let cam = self.camera;

        // Camera basis from the eye/target pair, with the world up as the roll reference.
        let fwd = norm([
            cam.target[0] - cam.eye[0],
            cam.target[1] - cam.eye[1],
            cam.target[2] - cam.eye[2],
        ]);
        let world_up = [0.0, 1.0, 0.0];
        let mut right = cross(fwd, world_up);
        if dot(right, right) < 1e-8 {
            right = [1.0, 0.0, 0.0];
        }
        let right = norm(right);
        let up = cross(right, fwd);

        let mut screen = [[0.0f32; 2]; 3];
        let mut depth = [0.0f32; 3];
        let mut world_view = [[0.0f32; 3]; 3];

        for i in 0..3 {
            let rel = [
                tri.pos[i][0] - cam.eye[0],
                tri.pos[i][1] - cam.eye[1],
                tri.pos[i][2] - cam.eye[2],
            ];
            let z = dot(rel, fwd);
            if z < 0.05 {
                return;
            }
            let sx = cx + dot(rel, right) * self.focal / z;
            let sy = cy - dot(rel, up) * self.focal / z;
            screen[i] = [sx, sy];
            depth[i] = z;
            world_view[i] = rel;
        }

        let total = edge(screen[0], screen[1], screen[2][0], screen[2][1]);
        if total.abs() < 1e-6 {
            return;
        }
        // No back-face culling. Roblox's avatar is closed solids plus one-triangle-thick
        // sheets (hair cards, eyebrows, face decals), and the OBJ does not document which way
        // its faces wind; a cull therefore has no safe sign and can only punch holes in hair
        // and faces. The depth buffer already decides what is visible, so both sides are
        // rasterised and the nearest surface wins.

        let mut colors = [[0.0f32; 3]; 3];
        for i in 0..3 {
            // `Kd` × the OBJ's vertex colour, then the studio lighting. A textured face
            // multiplies the sampled texel by this same tint, so a white texture is white and
            // a tinted `Kd` tints the fabric, exactly as Roblox's viewer does it.
            let c = tri.color[i];
            let base = [
                (c.r() as f32 / 255.0) * tri.kd[0],
                (c.g() as f32 / 255.0) * tri.kd[1],
                (c.b() as f32 / 255.0) * tri.kd[2],
            ];
            // A sheet seen from behind has its normal pointing away, which would shade it
            // black; flip it toward the viewer instead, as a two-sided material does.
            let mut normal = tri.vertex_normals[i];
            if tri.double_sided && dot(normal, world_view[i]) > 0.0 {
                normal = [-normal[0], -normal[1], -normal[2]];
            }
            let lit = shade(normal, world_view[i]);
            colors[i] = [
                base[0] * lit[0],
                base[1] * lit[1],
                base[2] * lit[2],
            ];
        }

        let uvs = [
            [tri.uv[0][0], 1.0 - tri.uv[0][1]],
            [tri.uv[1][0], 1.0 - tri.uv[1][1]],
            [tri.uv[2][0], 1.0 - tri.uv[2][1]],
        ];

        let min_x = screen[0][0]
            .min(screen[1][0])
            .min(screen[2][0])
            .floor()
            .max(0.0);
        let max_x = screen[0][0]
            .max(screen[1][0])
            .max(screen[2][0])
            .ceil()
            .min(w - 1.0);
        let min_y = screen[0][1]
            .min(screen[1][1])
            .min(screen[2][1])
            .floor()
            .max(0.0);
        let max_y = screen[0][1]
            .max(screen[1][1])
            .max(screen[2][1])
            .ceil()
            .min(h - 1.0);
        if max_x < min_x || max_y < min_y {
            return;
        }

        for y in (min_y as usize)..=(max_y as usize) {
            let py = y as f32 + 0.5;
            let row = y * self.width;
            for x in (min_x as usize)..=(max_x as usize) {
                let px = x as f32 + 0.5;
                let e0 = edge(screen[1], screen[2], px, py) / total;
                let e1 = edge(screen[2], screen[0], px, py) / total;
                let e2 = edge(screen[0], screen[1], px, py) / total;
                if e0 < 0.0 || e1 < 0.0 || e2 < 0.0 {
                    continue;
                }
                let i0 = e0 / depth[0];
                let i1 = e1 / depth[1];
                let i2 = e2 / depth[2];
                let inv = i0 + i1 + i2;
                if inv.abs() < 1e-9 {
                    continue;
                }
                let z = (i0 * depth[0] + i1 * depth[1] + i2 * depth[2]) / inv;
                let idx = row + x;
                if z >= self.zbuf[idx] {
                    continue;
                }

                let mut rgb = [
                    (i0 * colors[0][0] + i1 * colors[1][0] + i2 * colors[2][0]) / inv,
                    (i0 * colors[0][1] + i1 * colors[1][1] + i2 * colors[2][1]) / inv,
                    (i0 * colors[0][2] + i1 * colors[1][2] + i2 * colors[2][2]) / inv,
                ];
                let mut alpha = tri.alpha;
                if let Some(tex) = tri.texture {
                    let u = (i0 * uvs[0][0] + i1 * uvs[1][0] + i2 * uvs[2][0]) / inv;
                    let v = (i0 * uvs[0][1] + i1 * uvs[1][1] + i2 * uvs[2][1]) / inv;
                    let t = texel(tex, u, v);
                    let ka = match tri.mode {
                        AlphaMode::Opaque => 1.0,
                        AlphaMode::Overlay => {
                            if t[3] as f32 / 255.0 < ALPHA_CUTOUT {
                                continue;
                            }
                            1.0
                        }
                        AlphaMode::Transparent => t[3] as f32 / 255.0,
                    };
                    alpha *= ka;
                    if alpha < ALPHA_CUTOUT {
                        continue;
                    }
                    rgb = [
                        rgb[0] * t[0] as f32 / 255.0,
                        rgb[1] * t[1] as f32 / 255.0,
                        rgb[2] * t[2] as f32 / 255.0,
                    ];
                }
                if alpha < ALPHA_CUTOUT {
                    continue;
                }

                let src = [
                    (rgb[0] * 255.0).clamp(0.0, 255.0),
                    (rgb[1] * 255.0).clamp(0.0, 255.0),
                    (rgb[2] * 255.0).clamp(0.0, 255.0),
                ];
                let dst = self.pixels[idx];
                let (r, g, b) = if alpha >= 0.995 {
                    (src[0], src[1], src[2])
                } else {
                    (
                        src[0] * alpha + dst.r() as f32 * (1.0 - alpha),
                        src[1] * alpha + dst.g() as f32 * (1.0 - alpha),
                        src[2] * alpha + dst.b() as f32 * (1.0 - alpha),
                    )
                };
                self.pixels[idx] = Color32::from_rgb(
                    r.round().clamp(0.0, 255.0) as u8,
                    g.round().clamp(0.0, 255.0) as u8,
                    b.round().clamp(0.0, 255.0) as u8,
                );
                // Only opaque coverage writes depth: a blended strand must not stop what is
                // behind it from being drawn.
                if alpha >= 0.995 {
                    self.zbuf[idx] = z;
                }
            }
        }
    }
}

fn edge(a: [f32; 2], b: [f32; 2], px: f32, py: f32) -> f32 {
    (px - a[0]) * (b[1] - a[1]) - (py - a[1]) * (b[0] - a[0])
}

/// Renders at `samples × samples` the requested size and box-filters down, which is what gives
/// the avatar clean edges without a multisampled framebuffer.
pub fn downsample(img: &ColorImage, width: usize, height: usize, samples: usize) -> ColorImage {
    if samples <= 1 || width == 0 || height == 0 {
        return img.clone();
    }
    let [sw, sh] = img.size;
    let mut pixels = vec![Color32::TRANSPARENT; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut acc = [0u32; 3];
            let mut hit = 0u32;
            for sy in 0..samples {
                for sx in 0..samples {
                    let ix = x * samples + sx;
                    let iy = y * samples + sy;
                    if ix >= sw || iy >= sh {
                        continue;
                    }
                    let c = img.pixels[iy * sw + ix];
                    if c.a() == 0 {
                        continue;
                    }
                    acc[0] += c.r() as u32;
                    acc[1] += c.g() as u32;
                    acc[2] += c.b() as u32;
                    hit += 1;
                }
            }
            if hit == 0 {
                continue;
            }
            pixels[y * width + x] = Color32::from_rgb(
                (acc[0] / hit) as u8,
                (acc[1] / hit) as u8,
                (acc[2] / hit) as u8,
            );
        }
    }
    ColorImage::new([width, height], pixels)
}

/// The wait before each retry while Roblox bakes an edited avatar: 150 ms, then 300 ms, then
/// 500 ms and steady. The service usually answers inside the first second, and the old fixed
/// 1.5 s steps made a finished bake sit unnoticed for a second and a half.
pub fn bake_backoff_ms(attempt: usize) -> u64 {
    match attempt {
        0 => 0,
        1 => 150,
        2 => 300,
        _ => 500,
    }
}

/// How many times the loader asks before giving up on a bake (~10 s at the schedule above).
pub const BAKE_ATTEMPTS: usize = 24;
