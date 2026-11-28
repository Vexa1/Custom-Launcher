// src/avatar_3d.rs
// Real-time 3D Roblox Avatar parser, loader, and GPU-accelerated egui renderer

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use crate::avatar_anim::{
    accessory_triangle_mask, classify_triangles, island_ids, AnimPlayer, AnimationKind, Mat4, Part,
    PartCounts, Pose, Skeleton, PART_COUNT,
};
use crate::avatar_scene::{self, Scene};
use crate::avatar_sync::MeshStore;
use std::collections::HashMap;
use serde_json::Value;
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};

/// Roblox CDN sharding algorithm: assets starting with 30DAY- or raw hex hashes
/// must be downloaded from t{sub}.rbxcdn.com where sub = (hash bytes XOR 31) % 8.
pub fn rbx_cdn_url(hash: &str) -> String {
    let mut acc: u32 = 31;
    for b in hash.bytes() {
        acc ^= b as u32;
    }
    let sub = acc % 8;
    format!("https://t{sub}.rbxcdn.com/{hash}")
}

/// Download raw asset bytes from Roblox CDN, automatically decompressing gzip if present.
pub fn download_rbx_asset(hash: &str) -> Result<Vec<u8>, String> {
    let url = rbx_cdn_url(hash);
    let resp = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .set("Referer", "https://www.roblox.com/")
        .call()
        .map_err(|e| format!("CDN fetch failed for {url}: {e}"))?;

    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Read error: {e}"))?;

    // Check for GZip magic header (0x1f 0x8b)
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        use flate2::read::GzDecoder;
        let mut gz = GzDecoder::new(&bytes[..]);
        let mut decompressed = Vec::new();
        gz.read_to_end(&mut decompressed)
            .map_err(|e| format!("Gzip decode failed: {e}"))?;
        Ok(decompressed)
    } else {
        Ok(bytes)
    }
}

/// Single 3D vertex with position, normal, UV coordinates, and color
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AvatarVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: Color32,
}

/// Single 3D triangle
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AvatarTriangle {
    pub v: [AvatarVertex; 3],
    pub texture_index: Option<usize>,
    pub base_color: Color32,
    pub normal: [f32; 3],
    /// Rig part this triangle belongs to (`Part` index). Drives the per-limb transform during
    /// an animation.
    pub part: u8,
    /// OBJ vertex index of each corner (`u32::MAX` when the face did not name one). Triangles
    /// that share a vertex belong to the same mesh island, which is how the rig is found.
    pub vidx: [u32; 3],
}

/// Parsed 3D model with normalized coordinates
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Avatar3DModel {
    pub triangles: Vec<AvatarTriangle>,
    pub center: [f32; 3],
    pub scale: f32,
    pub texture_hashes: Vec<String>,
    /// Content hash of the OBJ this model was built from (changes whenever the avatar changes).
    pub obj_hash: String,
    /// Joint pivots derived from each limb's bounding box.
    pub skeleton: Skeleton,
    /// How the triangles split across the rig: logged and written next to the dumped model.
    pub part_counts: PartCounts,
}

impl Avatar3DModel {
    /// `LowerTorso 96 UpperTorso 144 ... unclassified 0` for the log and the dump.
    pub fn part_report(&self) -> String {
        format!(
            "{} triangles: {}",
            self.triangles.len(),
            self.part_counts.report()
        )
    }
}

/// Loaded texture data ready for egui
pub struct LoadedTexture {
    pub hash: String,
    pub image: egui::ColorImage,
    #[allow(dead_code)]
    pub handle: Option<egui::TextureHandle>,
}

/// What is cached for one rendered avatar look: the parsed mesh plus every decoded texture.
type CachedAvatar = (Avatar3DModel, Vec<(String, egui::ColorImage)>);

/// Decoded avatars, keyed by the mesh and texture hashes Roblox served. Toggling an item off
/// and back on, or stepping back to a recently seen look, therefore repaints from memory
/// instead of downloading and decoding the same OBJ and textures again.
fn avatar_cache() -> &'static Mutex<MeshStore<CachedAvatar>> {
    static CACHE: OnceLock<Mutex<MeshStore<CachedAvatar>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(MeshStore::new(128 * 1024 * 1024)))
}

fn avatar_cache_key(obj_hash: &str, tex_hashes: &[String]) -> String {
    format!("{obj_hash}|{}", tex_hashes.join(","))
}

fn avatar_payload_bytes(model: &Avatar3DModel, images: &[(String, egui::ColorImage)]) -> usize {
    let mesh = model.triangles.len() * std::mem::size_of::<AvatarTriangle>();
    let textures: usize = images.iter().map(|(_, i)| i.pixels.len() * 4).sum();
    mesh + textures
}

fn textures_from_images(images: &[(String, egui::ColorImage)]) -> Vec<LoadedTexture> {
    images
        .iter()
        .map(|(hash, image)| LoadedTexture {
            hash: hash.clone(),
            image: image.clone(),
            handle: None,
        })
        .collect()
}

/// Decode one downloaded texture into the form the rasteriser samples.
///
/// This runs on the loader thread: an avatar carries a dozen or more textures, and decoding
/// them all inside the frame that receives the model was a visible hitch.
pub fn decode_texture(hash: String, bytes: &[u8]) -> Option<LoadedTexture> {
    let img = image::load_from_memory(bytes).ok()?;
    let size = [img.width() as _, img.height() as _];
    let mut rgba = img.to_rgba8();
    // Roblox's avatar textures can carry alpha (clothing cut-outs, faded decals). CREBLOX
    // stacks the layers by depth, so a transparent texel would let whatever is behind show
    // through the clothes. Bake the alpha out.
    for px in rgba.pixels_mut() {
        px.0[3] = 255;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_flat_samples().as_slice());
    Some(LoadedTexture {
        hash,
        image,
        handle: None,
    })
}

/// One item's own decoded mesh, ready to be drawn on top of the avatar composite.
///
/// `assets-thumbnail-3d` renders the item on a standard body, so [`overlay_from_model`] strips
/// the body out and keeps only the item's islands. Positions stay in that render's own space;
/// `origin` and `scale` map them into the same normalised space the composite is drawn in,
/// which is what lets a hat downloaded a second ago sit on a model downloaded an hour ago.
pub struct OverlayMesh {
    pub asset_id: u64,
    pub origin: [f32; 3],
    pub scale: f32,
    pub triangles: Vec<AvatarTriangle>,
    pub texture_hashes: Vec<String>,
    pub textures: Vec<LoadedTexture>,
}

/// Min and max corner over a set of triangles.
fn model_bounds(tris: &[[[f32; 3]; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for tri in tris {
        for p in tri {
            for a in 0..3 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
    }
    (min, max)
}

/// Keeps only the item's triangles from "the item worn on a standard body".
///
/// Each rig part of that body is a single mesh island, so whatever else is classified onto a
/// part - a hat and a hair cap on the head, boots over the feet - is the item. Nothing is
/// kept when the split finds no extra island, so a clothing item that merges with the body is
/// never drawn as a second avatar.
pub fn overlay_from_model(
    asset_id: u64,
    model: Avatar3DModel,
    textures: Vec<LoadedTexture>,
) -> Result<OverlayMesh, String> {
    let tri_verts: Vec<[u32; 3]> = model.triangles.iter().map(|t| t.vidx).collect();
    let islands = island_ids(&tri_verts);
    let parts: Vec<Part> = model
        .triangles
        .iter()
        .map(|t| {
            Part::ALL
                .get(t.part as usize)
                .copied()
                .unwrap_or(Part::LowerTorso)
        })
        .collect();
    let tri_pos: Vec<[[f32; 3]; 3]> = model
        .triangles
        .iter()
        .map(|t| [t.v[0].pos, t.v[1].pos, t.v[2].pos])
        .collect();
    let bounds = model_bounds(&tri_pos);
    let mask = accessory_triangle_mask(&parts, &islands, &tri_pos);
    let triangles: Vec<AvatarTriangle> = model
        .triangles
        .iter()
        .zip(mask.iter())
        .filter(|(_, keep)| **keep)
        .map(|(t, _)| t.clone())
        .collect();
    if triangles.is_empty() {
        return Err("this item has no mesh of its own".to_string());
    }
    // Guards against a mis-split: a fragment spanning most of the body, or half of the whole
    // render, is a body part that slipped through rather than the item. Drawing that would
    // put a second body on the avatar, so it is refused and the item waits for the composite.
    let kept = model_bounds(
        &triangles
            .iter()
            .map(|t| [t.v[0].pos, t.v[1].pos, t.v[2].pos])
            .collect::<Vec<_>>(),
    );
    let full_h = (bounds.1[1] - bounds.0[1]).max(1e-4);
    if (kept.1[1] - kept.0[1]) / full_h > 0.6 || triangles.len() * 2 > model.triangles.len() {
        return Err("could not separate this item from the body".to_string());
    }
    Ok(OverlayMesh {
        asset_id,
        origin: model.center,
        scale: model.scale,
        triangles,
        texture_hashes: model.texture_hashes,
        textures,
    })
}

/// The 3D preview of one asset: `assets-thumbnail-3d` reports a render URL once Roblox has
/// drawn it. Nothing is polled: a preview that is not ready yet simply reports so, and the
/// viewport falls back to the composite.
fn asset_3d_manifest(asset_id: u64) -> Result<Value, String> {
    let url = format!("https://thumbnails.roblox.com/v1/assets-thumbnail-3d?assetId={asset_id}");
    let json: Value = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
        .set("Referer", "https://www.roblox.com/")
        .call()
        .map_err(|e| format!("asset 3D API: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let image_url = json["imageUrl"]
        .as_str()
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            format!(
                "3D preview is {} yet",
                json["state"].as_str().unwrap_or("not ready")
            )
        })?;
    let manifest: Value = ureq::get(image_url)
        .set("User-Agent", "Mozilla/5.0")
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(manifest)
}

fn manifest_textures(manifest: &Value) -> Vec<String> {
    manifest["textures"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The texture hashes an asset's own render uses.
///
/// This is how the composite is split by asset: Roblox draws each worn item with its own
/// texture atlas, so a material whose `map_Kd` is one of these belongs to that item. One small
/// manifest request per worn item, and unequipping it becomes a local visibility flip.
pub fn fetch_asset_texture_hashes(asset_id: u64) -> Result<Vec<String>, String> {
    let manifest = asset_3d_manifest(asset_id)?;
    let hashes = manifest_textures(&manifest);
    if hashes.is_empty() {
        return Err("no textures in the asset render".to_string());
    }
    Ok(hashes)
}

/// Downloads one asset's own mesh and decodes it, ready to draw immediately.
pub fn fetch_asset_overlay(asset_id: u64) -> Result<OverlayMesh, String> {
    let manifest = asset_3d_manifest(asset_id)?;
    let obj_hash = manifest["obj"]
        .as_str()
        .filter(|h| !h.is_empty())
        .ok_or("no obj in the asset render")?
        .to_string();
    let mtl_hash = manifest["mtl"].as_str().map(|s| s.to_string());
    let tex_hashes = manifest_textures(&manifest);

    let (obj, mtl, textures) = std::thread::scope(|s| {
        let obj_job = s.spawn(|| download_rbx_asset(&obj_hash));
        let mtl_job = s.spawn(|| mtl_hash.as_deref().and_then(|h| download_rbx_asset(h).ok()));
        let tex_jobs: Vec<_> = tex_hashes
            .iter()
            .map(|h| {
                s.spawn(move || {
                    download_rbx_asset(h)
                        .ok()
                        .and_then(|bytes| decode_texture(h.clone(), &bytes))
                })
            })
            .collect();
        let obj = obj_job.join().unwrap_or_else(|_| Err("OBJ thread failed".to_string()));
        let mtl = mtl_job.join().unwrap_or(None);
        let textures: Vec<LoadedTexture> = tex_jobs
            .into_iter()
            .filter_map(|j| j.join().ok().flatten())
            .collect();
        (obj, mtl, textures)
    });
    let model = parse_avatar_3d(&obj?, mtl.as_deref(), &tex_hashes)?;
    overlay_from_model(asset_id, model, textures)
}

/// State container for the 3D avatar viewport
pub struct Avatar3DState {
    pub is_3d: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub model: Option<Avatar3DModel>,
    pub textures: Vec<LoadedTexture>,
    /// Which asset each material of the composite belongs to (`avatar_scene::BASE` for the
    /// body), in the model's texture order.
    pub material_owners: Vec<u64>,
    /// Texture hashes learned for each worn asset, used to split the composite by asset.
    pub asset_textures: HashMap<u64, Vec<String>>,
    /// Decoded meshes of individual items, drawn over the composite while the composite does
    /// not have them yet. Equipping an item that is already here costs no network at all.
    pub overlays: HashMap<u64, OverlayMesh>,
    /// What the viewport shows, independent of the composite.
    pub scene: Scene,
    /// Decoded textures waiting to be adopted, a couple per frame (see `scroll::adopt_budget`).
    pub pending_textures: Vec<LoadedTexture>,
    pub loading: bool,
    pub err: Option<String>,
    pub loaded_user_id: Option<u64>,
    /// A reload was requested while one was already running; run it right after.
    pub reload_queued: bool,
    /// The next load follows an avatar edit: wait until Roblox serves the *new* model.
    pub expect_change: bool,
    /// Click-to-play animation preview. Costs nothing while stopped.
    pub anim: AnimPlayer,
}

/// Radians of rotation per dragged pixel.
const DRAG_SENSITIVITY: f32 = 0.015;
/// Flip either to -1.0 to invert that drag axis.
const DRAG_DIR_X: f32 = 1.0;
const DRAG_DIR_Y: f32 = 1.0;

const DEFAULT_YAW: f32 = 0.0;
const DEFAULT_PITCH: f32 = 0.05;

impl Default for Avatar3DState {
    fn default() -> Self {
        Self {
            is_3d: true,
            yaw: DEFAULT_YAW, // Character faces forward directly at camera
            pitch: DEFAULT_PITCH,
            zoom: 1.0,
            model: None,
            textures: Vec::new(),
            material_owners: Vec::new(),
            asset_textures: HashMap::new(),
            overlays: HashMap::new(),
            scene: Scene::default(),
            pending_textures: Vec::new(),
            loading: false,
            err: None,
            loaded_user_id: None,
            reload_queued: false,
            expect_change: false,
            anim: AnimPlayer::default(),
        }
    }
}

/// Material definition parsed from MTL file
struct MaterialDef {
    name: String,
    texture_hash: Option<String>,
    color: Color32,
}

/// Parse OBJ and MTL bytes into an Avatar3DModel
pub fn parse_avatar_3d(
    obj_bytes: &[u8],
    mtl_bytes: Option<&[u8]>,
    manifest_textures: &[String],
) -> Result<Avatar3DModel, String> {
    let obj_str = String::from_utf8_lossy(obj_bytes);

    // 1. Parse MTL if available
    let mut materials: Vec<MaterialDef> = Vec::new();
    if let Some(mb) = mtl_bytes {
        let mtl_str = String::from_utf8_lossy(mb);
        let mut cur_name = String::new();
        let mut cur_tex = None;
        let mut cur_col = Color32::from_rgb(210, 210, 215);

        for line in mtl_str.lines() {
            let line = line.trim();
            if line.starts_with("newmtl ") {
                if !cur_name.is_empty() {
                    materials.push(MaterialDef {
                        name: cur_name.clone(),
                        texture_hash: cur_tex.take(),
                        color: cur_col,
                    });
                }
                cur_name = line["newmtl ".len()..].trim().to_string();
                cur_col = Color32::from_rgb(210, 210, 215);
            } else if line.starts_with("Kd ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let r = parts[1].parse::<f32>().unwrap_or(0.8);
                    let g = parts[2].parse::<f32>().unwrap_or(0.8);
                    let b = parts[3].parse::<f32>().unwrap_or(0.8);
                    cur_col = Color32::from_rgb(
                        (r.clamp(0.0, 1.0) * 255.0) as u8,
                        (g.clamp(0.0, 1.0) * 255.0) as u8,
                        (b.clamp(0.0, 1.0) * 255.0) as u8,
                    );
                }
            } else if line.starts_with("map_Kd ") || line.starts_with("map_Ka ") {
                let tex = line.split_whitespace().nth(1).unwrap_or("").trim();
                if !tex.is_empty() {
                    cur_tex = Some(tex.to_string());
                }
            }
        }
        if !cur_name.is_empty() {
            materials.push(MaterialDef {
                name: cur_name,
                texture_hash: cur_tex,
                color: cur_col,
            });
        }
    }

    // Build unique texture hash list
    let mut texture_hashes = manifest_textures.to_vec();
    for m in &materials {
        if let Some(h) = &m.texture_hash {
            if !texture_hashes.contains(h) {
                texture_hashes.push(h.clone());
            }
        }
    }

    // 2. Parse OBJ file
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<Color32> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();

    let mut current_mtl_idx = 0usize;
    let mut current_part: Option<Part> = None;
    let mut object_part: Option<Part> = None;
    let mut triangles: Vec<AvatarTriangle> = Vec::new();
    let mut triangle_parts: Vec<Option<Part>> = Vec::new();

    let mut min_pos = [f32::MAX, f32::MAX, f32::MAX];
    let mut max_pos = [f32::MIN, f32::MIN, f32::MIN];

    for line in obj_str.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with("v ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let x = parts[1].parse::<f32>().unwrap_or(0.0);
                let y = parts[2].parse::<f32>().unwrap_or(0.0);
                let z = parts[3].parse::<f32>().unwrap_or(0.0);
                positions.push([x, y, z]);

                min_pos[0] = min_pos[0].min(x);
                min_pos[1] = min_pos[1].min(y);
                min_pos[2] = min_pos[2].min(z);
                max_pos[0] = max_pos[0].max(x);
                max_pos[1] = max_pos[1].max(y);
                max_pos[2] = max_pos[2].max(z);

                if parts.len() >= 7 {
                    let r = parts[4].parse::<f32>().unwrap_or(1.0);
                    let g = parts[5].parse::<f32>().unwrap_or(1.0);
                    let b = parts[6].parse::<f32>().unwrap_or(1.0);
                    colors.push(Color32::from_rgb(
                        (r.clamp(0.0, 1.0) * 255.0) as u8,
                        (g.clamp(0.0, 1.0) * 255.0) as u8,
                        (b.clamp(0.0, 1.0) * 255.0) as u8,
                    ));
                } else {
                    colors.push(Color32::WHITE);
                }
            }
        } else if line.starts_with("vt ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let u = parts[1].parse::<f32>().unwrap_or(0.0);
                let v = parts[2].parse::<f32>().unwrap_or(0.0);
                uvs.push([u, v]);
            }
        } else if line.starts_with("vn ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let nx = parts[1].parse::<f32>().unwrap_or(0.0);
                let ny = parts[2].parse::<f32>().unwrap_or(0.0);
                let nz = parts[3].parse::<f32>().unwrap_or(0.0);
                normals.push([nx, ny, nz]);
            }
        } else if line.starts_with("usemtl ") {
            let mtl_name = line["usemtl ".len()..].trim();
            if let Some(pos) = materials.iter().position(|m| m.name == mtl_name) {
                current_mtl_idx = pos;
            }
            // Roblox's avatar OBJ names its materials after the R15/R6 body part.
            current_part = Part::from_name(mtl_name).or(object_part);
        } else if line.starts_with("o ") || line.starts_with("g ") {
            let name = line[2..].trim();
            object_part = Part::from_name(name);
        } else if line.starts_with("f ") {
            let face_tokens: Vec<&str> = line.split_whitespace().skip(1).collect();
            if face_tokens.len() < 3 {
                continue;
            }

            let parse_vertex = |tok: &str| -> AvatarVertex {
                let sub: Vec<&str> = tok.split('/').collect();
                let v_idx = sub
                    .get(0)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1);
                let vt_idx = sub.get(1).and_then(|s| s.parse::<usize>().ok());
                let vn_idx = sub.get(2).and_then(|s| s.parse::<usize>().ok());

                let pos = if v_idx > 0 && v_idx <= positions.len() {
                    positions[v_idx - 1]
                } else {
                    [0.0, 0.0, 0.0]
                };

                let col = if v_idx > 0 && v_idx <= colors.len() {
                    colors[v_idx - 1]
                } else {
                    Color32::WHITE
                };

                let uv = if let Some(idx) = vt_idx {
                    if idx > 0 && idx <= uvs.len() {
                        uvs[idx - 1]
                    } else {
                        [0.0, 0.0]
                    }
                } else {
                    [0.0, 0.0]
                };

                let norm = if let Some(idx) = vn_idx {
                    if idx > 0 && idx <= normals.len() {
                        normals[idx - 1]
                    } else {
                        [0.0, 0.0, 0.0]
                    }
                } else {
                    [0.0, 0.0, 0.0]
                };

                AvatarVertex {
                    pos,
                    normal: norm,
                    uv,
                    color: col,
                }
            };

            // The raw OBJ index of each corner: two triangles sharing one are one mesh
            // island, which is what the rig classification groups on.
            let vert_index = |tok: &str| -> u32 {
                tok.split('/')
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|i| if i > 0 { (i - 1) as u32 } else { u32::MAX })
                    .unwrap_or(u32::MAX)
            };

            let v0 = parse_vertex(face_tokens[0]);
            let (tex_idx, base_col) = if let Some(m) = materials.get(current_mtl_idx) {
                let tidx = m
                    .texture_hash
                    .as_ref()
                    .and_then(|h| texture_hashes.iter().position(|th| th == h));
                (tidx, m.color)
            } else {
                (None, Color32::from_rgb(220, 220, 225))
            };

            for i in 1..face_tokens.len() - 1 {
                let mut v0_tri = v0.clone();
                let mut v1 = parse_vertex(face_tokens[i]);
                let mut v2 = parse_vertex(face_tokens[i + 1]);

                // Calculate face normal
                let edge1 = [
                    v1.pos[0] - v0_tri.pos[0],
                    v1.pos[1] - v0_tri.pos[1],
                    v1.pos[2] - v0_tri.pos[2],
                ];
                let edge2 = [
                    v2.pos[0] - v0_tri.pos[0],
                    v2.pos[1] - v0_tri.pos[1],
                    v2.pos[2] - v0_tri.pos[2],
                ];
                let mut fnorm = [
                    edge1[1] * edge2[2] - edge1[2] * edge2[1],
                    edge1[2] * edge2[0] - edge1[0] * edge2[2],
                    edge1[0] * edge2[1] - edge1[1] * edge2[0],
                ];
                let len = (fnorm[0] * fnorm[0] + fnorm[1] * fnorm[1] + fnorm[2] * fnorm[2]).sqrt();
                if len > 1e-6 {
                    fnorm[0] /= len;
                    fnorm[1] /= len;
                    fnorm[2] /= len;
                }

                for vert in [&mut v0_tri, &mut v1, &mut v2] {
                    let nlen2 = vert.normal[0] * vert.normal[0]
                        + vert.normal[1] * vert.normal[1]
                        + vert.normal[2] * vert.normal[2];
                    if nlen2 <= 1e-6 {
                        vert.normal = fnorm;
                    }
                }

                triangle_parts.push(current_part);
                triangles.push(AvatarTriangle {
                    v: [v0_tri, v1, v2],
                    texture_index: tex_idx,
                    base_color: base_col,
                    normal: fnorm,
                    part: current_part.map(|p| p.index() as u8).unwrap_or(255),
                    vidx: [
                        vert_index(face_tokens[0]),
                        vert_index(face_tokens[i]),
                        vert_index(face_tokens[i + 1]),
                    ],
                });
            }
        }
    }

    if positions.is_empty() || triangles.is_empty() {
        return Err("No vertices or triangles found in avatar OBJ".to_string());
    }

    let center = [
        (min_pos[0] + max_pos[0]) * 0.5,
        (min_pos[1] + max_pos[1]) * 0.5,
        (min_pos[2] + max_pos[2]) * 0.5,
    ];

    let extent_y = (max_pos[1] - min_pos[1]).max(1.0);
    let scale = 2.0 / extent_y;

    // Roblox's OBJ does not name body parts (`Part12Mtl`, `Object_7`), so the islands are
    // placed on the rig from their own bounds; a name in the file still wins where present.
    let tri_verts: Vec<[u32; 3]> = triangles.iter().map(|t| t.vidx).collect();
    let tri_pos: Vec<[[f32; 3]; 3]> = triangles
        .iter()
        .map(|t| [t.v[0].pos, t.v[1].pos, t.v[2].pos])
        .collect();
    let parts = classify_triangles(&tri_verts, &tri_pos, &triangle_parts, min_pos, max_pos);
    for (tri, part) in triangles.iter_mut().zip(parts.iter()) {
        tri.part = part.index() as u8;
    }
    let part_counts = PartCounts::from_parts(&parts);
    let skeleton = build_skeleton(&triangles, center, extent_y);

    Ok(Avatar3DModel {
        triangles,
        center,
        scale,
        texture_hashes,
        obj_hash: String::new(),
        skeleton,
        part_counts,
    })
}

/// Derives each joint's anatomical pivot from the vertices of the limb it starts.
///
/// The pivot is the centre of the joint-facing face of the part: the hips for the lower torso,
/// the waist for the upper torso, the neck for the head, shoulders for upper arms, elbows,
/// wrists, hips, knees and ankles. Rotating a part about that point keeps it socketed in its
/// parent, which is what stops the limbs from detaching mid-animation.
fn build_skeleton(triangles: &[AvatarTriangle], center: [f32; 3], extent_y: f32) -> Skeleton {
    let mut bounds: [Option<([f32; 3], [f32; 3])>; PART_COUNT] = [None; PART_COUNT];
    for tri in triangles {
        let idx = tri.part as usize;
        if idx >= PART_COUNT {
            continue;
        }
        for v in &tri.v {
            if let Some((min, max)) = &mut bounds[idx] {
                for a in 0..3 {
                    min[a] = min[a].min(v.pos[a]);
                    max[a] = max[a].max(v.pos[a]);
                }
            } else {
                bounds[idx] = Some((v.pos, v.pos));
            }
        }
    }
    Skeleton::from_part_bounds(&bounds, center, extent_y)
}

/// Keeps the avatar the launcher just downloaded next to its log, and logs the limb split.
///
/// The 3D avatar OBJ is only served to a signed-in session, so the material names and the
/// per-part triangle counts can only be seen on a real account. Writing the pair to
/// `<data dir>/avatar3d/` (one look, overwritten each time) makes that inspectable, and the
/// one-line report in the log says whether the rig actually found every limb.
fn save_avatar_dump(user_id: u64, obj: &[u8], mtl: Option<&[u8]>, model: &Avatar3DModel) {
    let report = model.part_report();
    crate::auth::log(&format!("Avatar 3D {user_id}: {report}"));

    let dir = crate::auth::data_dir().join("avatar3d");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Bounded on purpose: one look's worth of downloads, and nothing unreasonable in size.
    if obj.len() <= 16 * 1024 * 1024 {
        let _ = std::fs::write(dir.join("avatar.obj"), obj);
    }
    if let Some(mtl) = mtl.filter(|m| m.len() <= 2 * 1024 * 1024) {
        let _ = std::fs::write(dir.join("avatar.mtl"), mtl);
    }
    let _ = std::fs::write(
        dir.join("avatar_parts.txt"),
        format!(
            "user {user_id}\nobj hash {}\n{}\n\nempty parts: {:?}\n",
            model.obj_hash,
            report,
            model.part_counts.empty()
        ),
    );
}

/// Fetch avatar 3D data from Roblox thumbnails API and CDN
#[allow(dead_code)]
pub fn load_avatar_3d_data(
    user_id: u64,
    cookie: Option<&str>,
) -> Result<(Avatar3DModel, Vec<LoadedTexture>), String> {
    load_avatar_3d_data_ex(user_id, cookie, None)
}

/// Like [`load_avatar_3d_data`], but when `stale_obj_hash` is given (right after the avatar
/// was edited) it keeps polling until Roblox has re-rendered a model with a *different*
/// mesh. Without this the old, cached model came back and new items never showed up.
pub fn load_avatar_3d_data_ex(
    user_id: u64,
    cookie: Option<&str>,
    stale_obj_hash: Option<&str>,
) -> Result<(Avatar3DModel, Vec<LoadedTexture>), String> {
    let url = format!("https://thumbnails.roblox.com/v1/users/avatar-3d?userId={user_id}");
    let max_attempts = if stale_obj_hash.is_some() { 10 } else { 6 };
    let mut last_err = String::from("Avatar 3D thumbnail is not ready yet");
    let mut manifest_and_hash: Option<(Value, String)> = None;
    for attempt in 0..max_attempts {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        let mut req = ureq::get(&url)
            .set(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
            )
            .set("Referer", "https://www.roblox.com/")
            .set("Cache-Control", "no-cache");
        if let Some(c) = cookie {
            req = req.set("Cookie", &format!(".ROBLOSECURITY={c}"));
        }
        let json: Value = match req
            .call()
            .map_err(|e| e.to_string())
            .and_then(|r| r.into_json().map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("avatar-3d API call error: {e}");
                continue;
            }
        };
        let Some(image_url) = json["imageUrl"].as_str().filter(|u| !u.is_empty()) else {
            let state = json["state"].as_str().unwrap_or("unknown");
            last_err = format!("Avatar 3D thumbnail status: {state}");
            continue; // Pending / not generated yet: try again shortly
        };
        let manifest: Value = match ureq::get(image_url)
            .set("User-Agent", "Mozilla/5.0")
            .call()
            .map_err(|e| e.to_string())
            .and_then(|r| r.into_json().map_err(|e| e.to_string()))
        {
            Ok(m) => m,
            Err(e) => {
                last_err = format!("Manifest download error: {e}");
                continue;
            }
        };
        let hash = manifest["obj"].as_str().unwrap_or("").to_string();
        if hash.is_empty() {
            last_err = "No obj hash in manifest".to_string();
            continue;
        }
        if stale_obj_hash == Some(hash.as_str()) && attempt + 1 < max_attempts {
            last_err = "Roblox is still rendering the updated avatar".to_string();
            continue; // same mesh as before the edit: the thumbnail is stale
        }
        manifest_and_hash = Some((manifest, hash));
        break;
    }
    let (manifest, obj_hash_owned) = manifest_and_hash.ok_or(last_err)?;
    let obj_hash: &str = &obj_hash_owned;
    let mtl_hash = manifest["mtl"].as_str();

    let mut tex_hashes = Vec::new();
    if let Some(arr) = manifest["textures"].as_array() {
        for t in arr {
            if let Some(h) = t.as_str() {
                tex_hashes.push(h.to_string());
            }
        }
    }

    // Reuse an already decoded avatar when this exact look was seen recently.
    if let Ok(mut cache) = avatar_cache().lock() {
        if let Some(hit) = cache.get(&avatar_cache_key(obj_hash, &tex_hashes)) {
            let (model, images) = &*hit;
            return Ok((model.clone(), textures_from_images(images)));
        }
    }

    // Download the OBJ, MTL and every texture in parallel (was one-by-one).
    let (obj_bytes, mtl_bytes, raw_textures) = std::thread::scope(|s| {
        let obj_job = s.spawn(|| download_rbx_asset(obj_hash));
        let mtl_job = s.spawn(|| mtl_hash.and_then(|mh| download_rbx_asset(mh).ok()));
        // Download and decode on the same worker thread: the UI frame that receives the
        // model then has nothing left to do but adopt ready images.
        let tex_jobs: Vec<_> = tex_hashes
            .iter()
            .map(|th| {
                s.spawn(move || {
                    download_rbx_asset(th)
                        .ok()
                        .and_then(|bytes| decode_texture(th.clone(), &bytes))
                })
            })
            .collect();
        let obj = obj_job
            .join()
            .unwrap_or_else(|_| Err("OBJ download thread failed".to_string()));
        let mtl = mtl_job.join().unwrap_or(None);
        let textures: Vec<LoadedTexture> = tex_jobs
            .into_iter()
            .filter_map(|j| j.join().ok().flatten())
            .collect();
        (obj, mtl, textures)
    });
    let obj_bytes = obj_bytes?;

    let mut model = parse_avatar_3d(&obj_bytes, mtl_bytes.as_deref(), &tex_hashes)?;
    model.obj_hash = obj_hash_owned.clone();
    save_avatar_dump(user_id, &obj_bytes, mtl_bytes.as_deref(), &model);

    // Keep the decoded result, bounded by the shared RAM budget, so coming back to this look
    // is a memory copy rather than another round trip.
    let images: Vec<(String, egui::ColorImage)> = raw_textures
        .iter()
        .map(|t| (t.hash.clone(), t.image.clone()))
        .collect();
    let key = avatar_cache_key(&obj_hash_owned, &tex_hashes);
    let bytes = avatar_payload_bytes(&model, &images);
    if let Ok(mut cache) = avatar_cache().lock() {
        cache.put(key, Arc::new((model.clone(), images)), bytes);
    }

    Ok((model, raw_textures))
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, -1.0]
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn rotate_normal(n: [f32; 3], cos_y: f32, sin_y: f32, cos_p: f32, sin_p: f32) -> [f32; 3] {
    let nx = n[0] * cos_y + n[2] * sin_y;
    let ny_pre = n[1];
    let nz_pre = -n[0] * sin_y + n[2] * cos_y;
    let rny = ny_pre * cos_p - nz_pre * sin_p;
    let rnz = ny_pre * sin_p + nz_pre * cos_p;
    normalize3([nx, rny, rnz])
}

/// Three-point studio lighting (warm key + cool fill + Fresnel rim + hemisphere AO).
fn shade_studio_vertex(
    view_pos: [f32; 3],
    raw_normal: [f32; 3],
    face_normal: [f32; 3],
    cos_y: f32,
    sin_y: f32,
    cos_p: f32,
    sin_p: f32,
    untextured_base: Option<Color32>,
) -> Color32 {
    let n_src = if dot3(raw_normal, raw_normal) > 1e-6 {
        raw_normal
    } else {
        face_normal
    };
    let n = rotate_normal(n_src, cos_y, sin_y, cos_p, sin_p);
    let v = normalize3([-view_pos[0], -view_pos[1], -(4.8 + view_pos[2])]);

    // Vectors point from surface toward studio lights (camera sits on -Z).
    let l_key = normalize3([-0.46, 0.68, -0.72]);
    let l_fill = normalize3([0.78, 0.14, -0.50]);
    let l_rim = normalize3([0.44, 0.56, 0.70]);

    // Key light: soft-wrapped diffuse + Blinn-Phong studio softbox specular
    let diff_key = ((dot3(n, l_key) + 0.24) / 1.24).clamp(0.0, 1.0).powf(1.12);
    let h_key = normalize3([l_key[0] + v[0], l_key[1] + v[1], l_key[2] + v[2]]);
    let spec = dot3(n, h_key).max(0.0).powf(16.0) * 0.26;

    // Cool opposite fill light
    let diff_fill = ((dot3(n, l_fill) + 0.34) / 1.34).clamp(0.0, 1.0);

    // Fresnel rim highlight along silhouette edges
    let ndotv = dot3(n, v).clamp(0.0, 1.0);
    let fresnel = (1.0 - ndotv).powf(2.4);
    let rim_gate = ((dot3(n, l_rim) + 0.40) / 1.40).clamp(0.0, 1.0);
    let rim = fresnel * (0.16 + 0.34 * rim_gate);

    // Hemisphere ambient (studio ceiling vs floor bounce) + subtle vertical grounding AO
    let hemi = (n[1] * 0.5 + 0.5).clamp(0.0, 1.0);
    let amb_rgb = [
        0.24 + (0.37 - 0.24) * hemi,
        0.25 + (0.39 - 0.25) * hemi,
        0.30 + (0.44 - 0.30) * hemi,
    ];
    let ao = 0.86 + 0.14 * ((view_pos[1] + 1.0) * 0.5).clamp(0.0, 1.0);

    let key_tint = [1.04f32, 0.99f32, 0.92f32];
    let fill_tint = [0.82f32, 0.91f32, 1.08f32];
    let rim_tint = [0.92f32, 0.96f32, 1.06f32];

    let mut lr =
        amb_rgb[0] * ao + (0.62 * diff_key * key_tint[0]) + (0.24 * diff_fill * fill_tint[0]);
    let mut lg =
        amb_rgb[1] * ao + (0.62 * diff_key * key_tint[1]) + (0.24 * diff_fill * fill_tint[1]);
    let mut lb =
        amb_rgb[2] * ao + (0.62 * diff_key * key_tint[2]) + (0.24 * diff_fill * fill_tint[2]);

    let sr = spec * key_tint[0] + rim * rim_tint[0];
    let sg = spec * key_tint[1] + rim * rim_tint[1];
    let sb = spec * key_tint[2] + rim * rim_tint[2];

    if let Some(base) = untextured_base {
        let br = base.r() as f32 / 255.0;
        let bg = base.g() as f32 / 255.0;
        let bb = base.b() as f32 / 255.0;
        Color32::from_rgba_unmultiplied(
            ((br * lr + sr * 0.72).clamp(0.0, 1.0) * 255.0) as u8,
            ((bg * lg + sg * 0.72).clamp(0.0, 1.0) * 255.0) as u8,
            ((bb * lb + sb * 0.72).clamp(0.0, 1.0) * 255.0) as u8,
            base.a(),
        )
    } else {
        lr = (lr + sr).clamp(0.0, 1.0);
        lg = (lg + sg).clamp(0.0, 1.0);
        lb = (lb + sb).clamp(0.0, 1.0);
        Color32::from_rgba_unmultiplied(
            (lr * 255.0) as u8,
            (lg * 255.0) as u8,
            (lb * 255.0) as u8,
            255,
        )
    }
}

/// Key for the cached, software-rasterised avatar image.
#[derive(PartialEq)]
struct RasterKey {
    yaw: u32,
    pitch: u32,
    zoom: u32,
    width: u32,
    height: u32,
    obj_hash: String,
    tri_count: usize,
    anim_frame: u32,
    /// Which asset groups are drawn and which item meshes are ready.
    scene_stamp: u64,
    textures: Vec<(String, [usize; 2])>,
}

thread_local! {
    static RASTER_CACHE: std::cell::RefCell<Option<(RasterKey, egui::TextureHandle)>> =
        const { std::cell::RefCell::new(None) };
}

/// Render the 3D avatar into the viewport.
///
/// egui has no depth buffer: it paints meshes in the order it is handed them. Any
/// per-texture or per-triangle painter's algorithm is therefore approximate, and where the
/// order is wrong the clothes and the body show through each other ("see-through avatar",
/// clashing textures). This rasterises the model on the CPU into one image with a real
/// z-buffer - the nearest surface always wins - and hands egui that single image.
pub fn render_avatar_3d_mesh(
    painter: &Painter,
    model: &Avatar3DModel,
    textures: &[LoadedTexture],
    scene: &Scene,
    material_owners: &[u64],
    overlays: &HashMap<u64, OverlayMesh>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    viewport_rect: Rect,
    textures_ready: bool,
    pose: &Pose,
    anim_frame: u32,
) -> Option<f32> {
    if viewport_rect.width() < 2.0 || viewport_rect.height() < 2.0 {
        return None;
    }
    // Textures are adopted a couple per frame, so mid-load the set is incomplete. Rasterising
    // it would burn a full software render per frame and flicker material colours, so keep
    // showing the last finished image until the set is whole.
    if !textures_ready {
        let previous = RASTER_CACHE.with(|c| c.borrow().as_ref().map(|(_, h)| h.clone()));
        if let Some(handle) = previous {
            painter.image(
                handle.id(),
                viewport_rect,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        return None;
    }
    // Rasterise a little above screen resolution so the GPU's linear filtering smooths the
    // triangle edges out again.
    // One raster pixel per point is plenty: the GPU smooths the edges, and the software
    // rasteriser was doing 9x the work on retina (1.5 x 2.0) for no visible gain.
    let scale = painter.ctx().pixels_per_point().clamp(1.0, 1.5);
    let width = ((viewport_rect.width() * scale).round() as usize).clamp(24, 1600);
    let height = ((viewport_rect.height() * scale).round() as usize).clamp(24, 1600);

    // Quantise the camera so a tiny drag reuses the cached raster instead of
    // re-rasterising the whole model for a fraction of a degree.
    let key = RasterKey {
        yaw: (yaw * 200.0).round() as i32 as u32,
        pitch: (pitch * 200.0).round() as i32 as u32,
        zoom: (zoom * 100.0).round() as i32 as u32,
        width: width as u32,
        height: height as u32,
        obj_hash: model.obj_hash.clone(),
        tri_count: model.triangles.len(),
        anim_frame,
        // Which groups are drawn and which item meshes are ready. An equip or unequip
        // changes this, so the picture is redrawn on the spot - no network involved.
        scene_stamp: scene.stamp(),
        textures: textures
            .iter()
            .map(|t| (t.hash.clone(), t.image.size))
            .collect(),
    };

    let cached = RASTER_CACHE.with(|c| {
        let slot = c.borrow();
        match slot.as_ref() {
            Some((cached_key, handle)) if *cached_key == key => Some(handle.clone()),
            _ => None,
        }
    });

    // A posed frame re-rasterises the whole avatar, so its cost is measured and handed back:
    // the player uses it to space the next frame. Cache hits cost nothing and return None.
    let mut raster_ms = None;
    let handle = match cached {
        Some(handle) => handle,
        None => {
            let started = std::time::Instant::now();
            let image = rasterize_avatar(
                model, textures, scene, material_owners, overlays, yaw, pitch, zoom, width, height,
                pose, anim_frame,
            );
            if anim_frame != 0 {
                raster_ms = Some(started.elapsed().as_secs_f32() * 1000.0);
            }
            let existing = RASTER_CACHE.with(|c| c.borrow().as_ref().map(|(_, h)| h.clone()));
            let handle = match existing {
                Some(mut handle) => {
                    handle.set(image, egui::TextureOptions::LINEAR);
                    handle
                }
                None => painter.ctx().load_texture(
                    "creblox_avatar_3d_raster",
                    image,
                    egui::TextureOptions::LINEAR,
                ),
            };
            RASTER_CACHE.with(|c| *c.borrow_mut() = Some((key, handle.clone())));
            handle
        }
    };

    painter.image(
        handle.id(),
        viewport_rect,
        Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    raster_ms
}

fn edge_fn(a: Pos2, b: Pos2, px: f32, py: f32) -> f32 {
    (px - a.x) * (b.y - a.y) - (py - a.y) * (b.x - a.x)
}

/// Nearest-neighbour texture lookup with wrapping, always opaque (Roblox avatar textures can
/// carry alpha in unused texels; blending that would make the avatar see-through again).
fn sample_texture(img: &egui::ColorImage, u: f32, v: f32) -> Color32 {
    let [tw, th] = img.size;
    if tw == 0 || th == 0 {
        return Color32::WHITE;
    }
    let uu = u - u.floor();
    let vv = v - v.floor();
    let x = ((uu * tw as f32).floor() as usize).min(tw - 1);
    let y = ((vv * th as f32).floor() as usize).min(th - 1);
    let c = img.pixels[y * tw + x];
    Color32::from_rgb(c.r(), c.g(), c.b())
}

/// The rasteriser's target: the picture, a depth buffer and the camera.
///
/// Both the avatar composite and an individual item's mesh go through [`Raster::triangle`], so
/// a separately downloaded hat is depth-tested against the body it lands on instead of being
/// painted over it.
struct Raster<'a> {
    pixels: &'a mut [Color32],
    zbuf: &'a mut [f32],
    width: usize,
    height: usize,
    cx: f32,
    cy: f32,
    aspect_scale: f32,
    cos_y: f32,
    sin_y: f32,
    cos_p: f32,
    sin_p: f32,
    dist: f32,
}

impl Raster<'_> {
    /// Draws one triangle.
    ///
    /// `limb` is the animation's transform for the triangle's rig part, and `origin`/`scale`
    /// map the triangle's own coordinates into the normalised space the camera looks at: the
    /// composite's centre and scale for its own triangles, the item render's for an overlay.
    fn triangle(
        &mut self,
        tri: &AvatarTriangle,
        limb: Option<Mat4>,
        origin: [f32; 3],
        scale: f32,
        lookup: &[Option<&LoadedTexture>],
    ) {
        let hf = self.height as f32;
        let wf = self.width as f32;
        let mut screen = [Pos2::ZERO; 3];
        let mut zs = [0.0f32; 3];
        let mut persp = [0.0f32; 3];
        let mut view = [[0.0f32; 3]; 3];

        for i in 0..3 {
            let v = &tri.v[i];
            let pos = match limb {
                Some(m) => m.transform_point(v.pos),
                None => v.pos,
            };
            let px = (pos[0] - origin[0]) * scale;
            let py = (pos[1] - origin[1]) * scale;
            let pz = (pos[2] - origin[2]) * scale;

            let rx = px * self.cos_y + pz * self.sin_y;
            let rz = -px * self.sin_y + pz * self.cos_y;
            let fy = py * self.cos_p - rz * self.sin_p;
            let fz = py * self.sin_p + rz * self.cos_p;

            let p = self.dist / (self.dist + fz);
            let sx = self.cx + rx * p * self.aspect_scale;
            let sy = self.cy - fy * p * self.aspect_scale + hf * 0.02;

            screen[i] = Pos2::new(sx, sy);
            zs[i] = fz;
            persp[i] = p;
            view[i] = [rx, fy, fz];
        }

        let total = edge_fn(screen[0], screen[1], screen[2].x, screen[2].y);
        if total.abs() < 1e-6 {
            return;
        }
        // Keep the same faces the old 2D cross-product cull kept.
        let cross = (screen[1].x - screen[0].x) * (screen[2].y - screen[0].y)
            - (screen[1].y - screen[0].y) * (screen[2].x - screen[0].x);
        if cross <= 0.0 {
            return;
        }

        let tex = tri
            .texture_index
            .and_then(|idx| lookup.get(idx))
            .copied()
            .flatten();
        let has_tex = tex.is_some();

        let mut colors = [[0.0f32; 3]; 3];
        let normal = match limb {
            Some(m) => m.transform_dir(tri.normal),
            None => tri.normal,
        };
        for i in 0..3 {
            let v = &tri.v[i];
            let vnormal = match limb {
                Some(m) => m.transform_dir(v.normal),
                None => v.normal,
            };
            let base = if has_tex {
                None
            } else if v.color != Color32::WHITE {
                Some(v.color)
            } else {
                Some(tri.base_color)
            };
            let c = shade_studio_vertex(
                view[i], vnormal, normal, self.cos_y, self.sin_y, self.cos_p, self.sin_p, base,
            );
            colors[i] = [c.r() as f32, c.g() as f32, c.b() as f32];
        }

        let uvs = [
            [tri.v[0].uv[0], 1.0 - tri.v[0].uv[1]],
            [tri.v[1].uv[0], 1.0 - tri.v[1].uv[1]],
            [tri.v[2].uv[0], 1.0 - tri.v[2].uv[1]],
        ];

        let min_x = screen[0]
            .x
            .min(screen[1].x)
            .min(screen[2].x)
            .floor()
            .max(0.0);
        let max_x = screen[0]
            .x
            .max(screen[1].x)
            .max(screen[2].x)
            .ceil()
            .min(wf - 1.0);
        let min_y = screen[0]
            .y
            .min(screen[1].y)
            .min(screen[2].y)
            .floor()
            .max(0.0);
        let max_y = screen[0]
            .y
            .max(screen[1].y)
            .max(screen[2].y)
            .ceil()
            .min(hf - 1.0);
        if max_x < min_x || max_y < min_y {
            return;
        }

        for y in (min_y as usize)..=(max_y as usize) {
            let py = y as f32 + 0.5;
            let row = y * self.width;
            for x in (min_x as usize)..=(max_x as usize) {
                let px = x as f32 + 0.5;
                let e0 = edge_fn(screen[1], screen[2], px, py) / total;
                let e1 = edge_fn(screen[2], screen[0], px, py) / total;
                let e2 = edge_fn(screen[0], screen[1], px, py) / total;
                if e0 < 0.0 || e1 < 0.0 || e2 < 0.0 {
                    continue;
                }

                // Perspective-correct barycentric weights.
                let i0 = e0 / persp[0];
                let i1 = e1 / persp[1];
                let i2 = e2 / persp[2];
                let inv = i0 + i1 + i2;
                if inv.abs() < 1e-9 {
                    continue;
                }
                let z = (i0 * zs[0] + i1 * zs[1] + i2 * zs[2]) / inv;
                let idx = row + x;
                if z >= self.zbuf[idx] {
                    continue;
                }
                self.zbuf[idx] = z;

                let lr = (i0 * colors[0][0] + i1 * colors[1][0] + i2 * colors[2][0]) / inv;
                let lg = (i0 * colors[0][1] + i1 * colors[1][1] + i2 * colors[2][1]) / inv;
                let lb = (i0 * colors[0][2] + i1 * colors[1][2] + i2 * colors[2][2]) / inv;

                let (r, g, b) = if let Some(tex) = tex {
                    let u = (i0 * uvs[0][0] + i1 * uvs[1][0] + i2 * uvs[2][0]) / inv;
                    let v = (i0 * uvs[0][1] + i1 * uvs[1][1] + i2 * uvs[2][1]) / inv;
                    let t = sample_texture(&tex.image, u, v);
                    (
                        t.r() as f32 * lr / 255.0,
                        t.g() as f32 * lg / 255.0,
                        t.b() as f32 * lb / 255.0,
                    )
                } else {
                    (lr, lg, lb)
                };

                self.pixels[idx] = Color32::from_rgb(
                    r.round().clamp(0.0, 255.0) as u8,
                    g.round().clamp(0.0, 255.0) as u8,
                    b.round().clamp(0.0, 255.0) as u8,
                );
            }
        }
    }
}

/// Renders the avatar: the composite's visible groups, then any item mesh the composite does
/// not have yet.
///
/// Nothing in here waits for `set-wearing-assets`: an unequip is a group that is no longer
/// drawn, and an equip is either geometry that was already in the model or a decoded mesh the
/// caller has in hand.
#[allow(clippy::too_many_arguments)]
fn rasterize_avatar(
    model: &Avatar3DModel,
    textures: &[LoadedTexture],
    scene: &Scene,
    material_owners: &[u64],
    overlays: &HashMap<u64, OverlayMesh>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    width: usize,
    height: usize,
    pose: &Pose,
    anim_frame: u32,
) -> egui::ColorImage {
    let mut pixels = vec![Color32::TRANSPARENT; width * height];
    let mut zbuf = vec![f32::INFINITY; width * height];

    let hf = height as f32;
    let wf = width as f32;
    let mut raster = Raster {
        pixels: &mut pixels,
        zbuf: &mut zbuf,
        width,
        height,
        cx: wf * 0.5,
        cy: hf * 0.5,
        aspect_scale: hf * 0.49 * zoom,
        cos_y: yaw.cos(),
        sin_y: yaw.sin(),
        cos_p: pitch.cos(),
        sin_p: pitch.sin(),
        dist: 4.8,
    };

    // Striking the pose: one hierarchical FK pass, then each triangle is carried by its limb.
    let transforms = if anim_frame == 0 {
        None
    } else {
        Some(model.skeleton.transforms(pose))
    };

    // Material texture indices point into `texture_hashes`; resolve them by hash so a texture
    // that failed to download cannot shift every later texture onto the wrong body part.
    let lookup: Vec<Option<&LoadedTexture>> = model
        .texture_hashes
        .iter()
        .map(|hash| textures.iter().find(|t| &t.hash == hash))
        .collect();

    for tri in &model.triangles {
        // A group the user has unequipped stays in the model and is simply not drawn, which
        // is what makes the toggle instant and keeps the geometry for a re-equip.
        let owner = avatar_scene::triangle_owner(material_owners, tri.texture_index);
        if !scene.shows(owner) {
            continue;
        }
        let limb = transforms
            .as_ref()
            .and_then(|t| t.get(tri.part as usize))
            .copied();
        raster.triangle(tri, limb, model.center, model.scale, &lookup);
    }

    // Items this composite never had (equipped a moment ago): their own decoded mesh, drawn
    // on the frame it became ready. The shared depth buffer sorts them against the body.
    for (asset_id, overlay) in overlays {
        if !scene.shows_mesh(*asset_id) {
            continue;
        }
        let lookup: Vec<Option<&LoadedTexture>> = overlay
            .texture_hashes
            .iter()
            .map(|hash| overlay.textures.iter().find(|t| &t.hash == hash))
            .collect();
        for tri in &overlay.triangles {
            let limb = transforms
                .as_ref()
                .and_then(|t| t.get(tri.part as usize))
                .copied();
            raster.triangle(tri, limb, overlay.origin, overlay.scale, &lookup);
        }
    }
    drop(raster);

    egui::ColorImage::new([width, height], pixels)
}

/// Render the complete avatar preview viewport with studio lighting, floor shadow,
/// 2D/3D mode switch, camera reset, and click-and-drag 3D rotation grab.
pub fn render_avatar_viewport(
    ui: &mut egui::Ui,
    state: &mut Avatar3DState,
    avatar_2d_url: Option<&str>,
    viewport_size: Vec2,
    dark_mode: bool,
    // The debounced equip sync is still running: show the corner pill.
    syncing: bool,
) {
    let (rect, resp) = ui.allocate_exact_size(viewport_size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // Animation preview: advance one tick and ask for the next frame *only* while a clip is
    // actually playing in the 3D viewport. Stopped, 2D mode or another page = no repaint at
    // all, so the preview costs 0% CPU when idle.
    if state.is_3d && state.model.is_some() && state.anim.needs_repaint() {
        let dt = ui.input(|i| i.stable_dt);
        state.anim.advance(dt);
        // Paced, not every frame: a posed frame is a full software render, and the interval is
        // whatever keeps that inside half of the frame budget (30 fps cap by default).
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32(
            state.anim.frame_interval(),
        ));
    }

    // 1. Studio backdrop gradient
    let top_col = if dark_mode {
        Color32::from_rgb(28, 33, 44)
    } else {
        // Soft pastel studio warm glow matching example photo
        Color32::from_rgb(246, 243, 232)
    };
    let bot_col = if dark_mode {
        Color32::from_rgb(14, 17, 24)
    } else {
        Color32::from_rgb(230, 233, 242)
    };

    let stroke = Stroke::new(
        1.0,
        if dark_mode {
            Color32::from_rgb(45, 52, 68)
        } else {
            Color32::from_rgb(215, 220, 230)
        },
    );

    // Rounded studio viewport frame
    painter.rect_filled(rect, 14.0, bot_col);
    painter.rect_stroke(rect, 14.0, stroke, egui::StrokeKind::Inside);

    // Vertical gradient backdrop
    let mut bg_mesh = egui::Mesh::default();
    bg_mesh.add_rect_with_uv(
        rect.shrink(1.0),
        Rect::from_min_max(egui::epaint::WHITE_UV, egui::epaint::WHITE_UV),
        Color32::WHITE,
    );
    // Tint vertices for studio gradient
    if bg_mesh.vertices.len() >= 4 {
        bg_mesh.vertices[0].color = top_col;
        bg_mesh.vertices[1].color = top_col;
        bg_mesh.vertices[2].color = bot_col;
        bg_mesh.vertices[3].color = bot_col;
    }
    painter.add(Shape::Mesh(bg_mesh.into()));

    // 2. Soft floor contact shadow under avatar feet
    let shadow_center = Pos2::new(rect.center().x, rect.bottom() - 32.0);
    let shadow_radius = Vec2::new(viewport_size.x * 0.38, 14.0);
    painter.add(Shape::ellipse_filled(
        shadow_center,
        shadow_radius,
        if dark_mode {
            Color32::from_black_alpha(80)
        } else {
            Color32::from_black_alpha(35)
        },
    ));

    // 3. Render 3D Model or 2D image
    let has_3d_model = state.model.is_some();

    let textures_ready = state.pending_textures.is_empty();
    let pose = state.anim.pose();
    let anim_frame = state.anim.frame_index();
    if let (true, Some(model)) = (state.is_3d, state.model.as_ref()) {
        let raster_ms = render_avatar_3d_mesh(
            &painter,
            model,
            &state.textures,
            &state.scene,
            &state.material_owners,
            &state.overlays,
            state.yaw,
            state.pitch,
            state.zoom,
            rect,
            textures_ready,
            &pose,
            anim_frame,
        );
        if let Some(ms) = raster_ms {
            state.anim.note_frame_cost(ms);
            if state.anim.take_cost_report() {
                crate::auth::log(&format!(
                    "Avatar animation: posed raster {:.1} ms, preview at {:.0} fps",
                    state.anim.frame_cost_ms(),
                    state.anim.effective_fps()
                ));
            }
        }
    } else {
        // Fallback to high-resolution 2D avatar preview.
        // Roblox's full-body render is square, so painting it into the wide viewport rect
        // stretched it; use the largest centred square inside the same area instead.
        if let Some(url) = avatar_2d_url {
            let side = (viewport_size.x - 16.0)
                .min(viewport_size.y - 24.0)
                .max(32.0);
            let img_rect = Rect::from_center_size(
                Pos2::new(rect.center().x, rect.center().y - 6.0),
                Vec2::new(side, side),
            );
            egui::Image::new(url)
                .corner_radius(10)
                .paint_at(ui, img_rect);
        }
    }

    // 4. Mouse Grab & Drag interaction
    if resp.dragged() {
        let delta = resp.drag_delta();
        // Grab-style drag: the surface under the cursor follows the cursor.
        // (Horizontal was already correct; vertical tilt used to be inverted.)
        state.yaw -= delta.x * DRAG_SENSITIVITY * DRAG_DIR_X;
        state.pitch = (state.pitch - delta.y * DRAG_SENSITIVITY * DRAG_DIR_Y).clamp(-0.75, 0.75);
        // Automatically switch to 3D mode if dragged while in 2D and 3D model is loaded
        if !state.is_3d && has_3d_model {
            state.is_3d = true;
        }
    }

    if resp.hovered() {
        ui.ctx().set_cursor_icon(if resp.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });

        // Zoom via scroll wheel
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            state.zoom = (state.zoom * (1.0 + scroll * 0.0015)).clamp(0.65, 2.2);
        }
    }

    if resp.double_clicked() {
        state.yaw = DEFAULT_YAW;
        state.pitch = DEFAULT_PITCH;
        state.zoom = 1.0;
    }

    // 5. Floating Controls Overlay in top-left (Matching example photo)
    let overlay_pos = Pos2::new(rect.left() + 12.0, rect.top() + 12.0);
    let mut ui_builder = ui.new_child(
        egui::UiBuilder::new().max_rect(Rect::from_min_size(overlay_pos, Vec2::new(120.0, 160.0))),
    );

    ui_builder.vertical(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(0.0, 6.0);

        // 2D / 3D pill button group
        egui::Frame::new()
            .fill(if dark_mode {
                Color32::from_rgba_unmultiplied(18, 22, 30, 220)
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 220)
            })
            .stroke(Stroke::new(
                1.0,
                if dark_mode {
                    Color32::from_rgb(50, 58, 76)
                } else {
                    Color32::from_rgb(205, 212, 224)
                },
            ))
            .corner_radius(16)
            .inner_margin(3)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;

                    // 2D toggle button
                    let is_2d = !state.is_3d;
                    let btn_2d =
                        egui::Button::new(egui::RichText::new("2D").size(11.5).strong().color(
                            if is_2d {
                                Color32::WHITE
                            } else if dark_mode {
                                Color32::from_rgb(160, 170, 190)
                            } else {
                                Color32::from_rgb(80, 90, 110)
                            },
                        ))
                        .fill(if is_2d {
                            Color32::from_rgb(0, 180, 105)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(12)
                        .min_size(Vec2::new(32.0, 24.0));
                    if ui.add(btn_2d).clicked() {
                        state.is_3d = false;
                        state.anim.stop();
                    }

                    // 3D toggle button
                    let is_3d = state.is_3d;
                    let btn_3d =
                        egui::Button::new(egui::RichText::new("3D").size(11.5).strong().color(
                            if is_3d {
                                Color32::WHITE
                            } else if dark_mode {
                                Color32::from_rgb(160, 170, 190)
                            } else {
                                Color32::from_rgb(80, 90, 110)
                            },
                        ))
                        .fill(if is_3d {
                            Color32::from_rgb(0, 180, 105)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(12)
                        .min_size(Vec2::new(32.0, 24.0));
                    if ui.add(btn_3d).clicked() {
                        state.is_3d = true;
                    }
                });
            });

        // Reset camera button ↺
        let reset_btn = egui::Button::new(egui::RichText::new("↺").size(15.0).strong().color(
            if dark_mode {
                Color32::from_rgb(210, 220, 240)
            } else {
                Color32::from_rgb(60, 70, 90)
            },
        ))
        .fill(if dark_mode {
            Color32::from_rgba_unmultiplied(18, 22, 30, 220)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 220)
        })
        .stroke(Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_rgb(50, 58, 76)
            } else {
                Color32::from_rgb(205, 212, 224)
            },
        ))
        .corner_radius(14)
        .min_size(Vec2::new(28.0, 28.0));

        if ui
            .add(reset_btn)
            .on_hover_text("Reset Camera (Yaw & Pitch)")
            .clicked()
        {
            state.yaw = DEFAULT_YAW;
            state.pitch = DEFAULT_PITCH;
            state.zoom = 1.0;
        }

        // Play / Stop the animation preview. Clicking an animation item picks the clip; this
        // pill pauses and resumes it without leaving the viewport.
        let playing = state.anim.is_playing();
        let can_play = state.is_3d && state.model.is_some();
        let pill_fill = if dark_mode {
            Color32::from_rgba_unmultiplied(18, 22, 30, 220)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 220)
        };
        let pill_stroke = Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_rgb(50, 58, 76)
            } else {
                Color32::from_rgb(205, 212, 224)
            },
        );
        let pill_text = if playing {
            Color32::from_rgb(255, 120, 120)
        } else if can_play {
            Color32::from_rgb(0, 190, 120)
        } else if dark_mode {
            Color32::from_rgb(110, 118, 134)
        } else {
            Color32::from_rgb(150, 158, 172)
        };
        let play_btn = egui::Button::new(
            egui::RichText::new(if playing { "❚❚ Stop" } else { "▶ Play" })
                .size(11.5)
                .strong()
                .color(pill_text),
        )
        .fill(pill_fill)
        .stroke(pill_stroke)
        .corner_radius(14)
        .min_size(Vec2::new(28.0, 26.0));
        let tip = match (state.anim.kind(), playing) {
            (Some(kind), true) => format!("Stop the {} preview", kind.label()),
            (Some(kind), false) => format!("Play the {} preview", kind.label()),
            (None, _) => "Click an animation item to preview it here".to_string(),
        };
        let play_resp = ui.add_enabled(can_play, play_btn).on_hover_text(tip);
        if play_resp.clicked() {
            if state.anim.kind().is_some() {
                state.anim.toggle();
            } else {
                state.anim.play(AnimationKind::Idle);
            }
            if state.anim.is_playing() {
                ui.ctx().request_repaint();
            }
        }
    });

    // 6. Loading indicator or status overlay in top-right
    if state.loading {
        let loading_pos = Pos2::new(rect.right() - 84.0, rect.top() + 12.0);
        let mut ui_loading = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(loading_pos, Vec2::new(72.0, 26.0))),
        );
        egui::Frame::new()
            .fill(if dark_mode {
                Color32::from_rgba_unmultiplied(18, 22, 30, 220)
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 220)
            })
            .corner_radius(12)
            .inner_margin(4)
            .show(&mut ui_loading, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new("3D").size(11.0).strong());
                });
            });
    }

    if syncing {
        // Tiny, non-intrusive: the mesh stays on screen and draggable while the debounced
        // `set-wearing-assets` request is on the wire. No full-frame spinner, no blanking.
        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(11.0);
        let text = Color32::from_rgb(226, 231, 240);
        let galley = painter.layout_no_wrap("Syncing...".to_string(), font, text);
        let size = galley.size() + egui::vec2(18.0, 7.0);
        let pill = Rect::from_min_size(
            Pos2::new(
                (rect.max.x - size.x - 10.0).max(rect.min.x + 6.0),
                (rect.max.y - size.y - 10.0).max(rect.min.y + 6.0),
            ),
            size,
        );
        painter.rect_filled(pill, 9.0, Color32::from_rgba_unmultiplied(10, 13, 20, 200));
        painter.galley(pill.min + egui::vec2(9.0, 3.5), galley, text);
    }
}

#[cfg(test)]
mod tests {
    use super::{Avatar3DState, DEFAULT_PITCH, DEFAULT_YAW, shade_studio_vertex};
    use eframe::egui::Color32;

    #[test]
    fn default_camera_faces_front_and_studio_lighting_has_contrast() {
        let st = Avatar3DState::default();
        assert_eq!(st.yaw, DEFAULT_YAW);
        assert_eq!(st.pitch, DEFAULT_PITCH);

        let key_lit = shade_studio_vertex(
            [0.0, 0.2, -0.3],
            [-0.3, 0.5, -0.8],
            [-0.3, 0.5, -0.8],
            1.0,
            0.0,
            1.0,
            0.0,
            None,
        );
        let shadow_side = shade_studio_vertex(
            [0.3, -0.2, 0.1],
            [0.85, -0.45, 0.25],
            [0.85, -0.45, 0.25],
            1.0,
            0.0,
            1.0,
            0.0,
            None,
        );
        assert!(key_lit.r() > shadow_side.r() + 45);

        let dark_lit = shade_studio_vertex(
            [-0.2, 0.4, -0.4],
            [-0.22, 0.34, -0.91],
            [-0.22, 0.34, -0.91],
            1.0,
            0.0,
            1.0,
            0.0,
            Some(Color32::from_rgb(20, 20, 22)),
        );
        assert!(dark_lit.r() > 50);
    }
}
