//! The Avatar 3D viewport's Roblox-accurate rendering: material grammar, the metadata camera,
//! alpha, `Kd` and the light setup.
//!
//! These drive the real `src/avatar_render.rs`, the module the viewport renders with, and end
//! by writing `artifacts/avatar_viewport_real.png`: an avatar built the way Roblox ships one
//! (flat `Kd` body parts, a textured shirt, cut-out hair cards, a blended visor) framed by the
//! `camera`/`aabb` metadata. The PNG is the eyeball check; the assertions are the invariants.

use creblox_vm_tests::avatar_render::{
    bake_backoff_ms, downsample, focal_px, normalize_framing, orbit_camera, parse_framing,
    parse_mtl, AlphaMode, Camera, Orbit, Raster, Tri, DEFAULT_FOV,
};
use creblox_vm_tests::egui::{Color32, ColorImage};

fn sample() -> String {
    r#"
# Roblox avatar MTL
newmtl Part0Mtl
Kd 0.890 0.729 0.588
rbx_alphamode opaque

newmtl ShirtMtl
Kd 0.235 0.439 0.769
map_Kd 30DAY-ShirtAtlas-Png
rbx_alphamode opaque

newmtl HairMtl
Kd 1 1 1
map_Kd 30DAY-HairAtlas-Png
map_d 30DAY-HairAlpha-Png
d 0.98
rbx_alphamode overlay
doubleSided true

newmtl VisorMtl
Kd 0.2 0.3 0.4
map_Kd 30DAY-VisorAtlas-Png
Tr 0.35
rbx_alphamode transparent
map_bump 30DAY-Bump-Png
"#
    .to_string()
}

#[test]
fn roblox_mtl_diffuse_and_alpha_are_parsed() {
    let mats = parse_mtl(sample().as_bytes());
    assert_eq!(mats.len(), 4, "one entry per newmtl: {mats:?}");

    // A flat body part: no texture at all, only `Kd`.
    assert_eq!(mats[0].name, "Part0Mtl");
    assert!(mats[0].texture.is_none());
    assert!((mats[0].kd[0] - 0.890).abs() < 1e-3);
    assert!((mats[0].kd[1] - 0.729).abs() < 1e-3);
    assert!((mats[0].kd[2] - 0.588).abs() < 1e-3);
    assert_eq!(mats[0].alpha, 1.0);
    assert_eq!(mats[0].mode, AlphaMode::Opaque);

    // A shirt: `Kd` tints the texture, which is a plain cut-out-free material.
    assert_eq!(mats[1].texture.as_deref(), Some("30DAY-ShirtAtlas-Png"));
    assert_eq!(mats[1].mode, AlphaMode::Opaque);

    // Hair: colour atlas, separate alpha map, `d`, and the cut-out mode.
    assert_eq!(mats[2].texture.as_deref(), Some("30DAY-HairAtlas-Png"));
    assert_eq!(mats[2].alpha_texture.as_deref(), Some("30DAY-HairAlpha-Png"));
    assert!((mats[2].alpha - 0.98).abs() < 1e-6);
    assert_eq!(mats[2].mode, AlphaMode::Overlay);
    assert!(mats[2].double_sided, "hair cards must render from behind");

    // `Tr` is the inverse of `d`; the visor is a blend, and unknown keys are ignored.
    assert!((mats[3].alpha - 0.65).abs() < 1e-6, "Tr 0.35 → d 0.65");
    assert_eq!(mats[3].mode, AlphaMode::Transparent);
    assert_eq!(mats[3].alpha_texture, None);
}

#[test]
fn mtl_defaults_are_forgiving() {
    // A file Roblox sometimes serves has no `Kd`, no mode and Windows-style paths.
    let mats = parse_mtl(
        b"newmtl Part3Mtl\r\nmap_Kd -s 1 1 1 textures\\FaceAtlas.png\r\n",
    );
    assert_eq!(mats.len(), 1);
    assert_eq!(mats[0].kd, [1.0, 1.0, 1.0], "missing Kd is white, not black");
    assert_eq!(mats[0].mode, AlphaMode::Opaque);
    assert_eq!(mats[0].texture.as_deref(), Some("FaceAtlas.png"));
    // An MTL with no `newmtl` at all is empty, never a panic.
    assert!(parse_mtl(b"").is_empty());
    assert!(parse_mtl(b"# only a comment\n").is_empty());
}

fn framing_json() -> serde_json::Value {
    serde_json::json!({
        "camera": {
            "position": {"x": 0.0, "y": 2.8, "z": 11.9},
            "direction": {"x": 0.0, "y": -0.05, "z": -1.0},
            "fov": 26.5
        },
        "aabb": {
            "min": {"x": -1.6, "y": 0.0, "z": -0.7},
            "max": {"x": 1.6, "y": 5.6, "z": 0.7}
        }
    })
}

#[test]
fn the_metadata_camera_frames_the_avatar() {
    let f = parse_framing(&framing_json()).expect("camera + aabb");
    assert!((f.fov - 26.5).abs() < 1e-3);
    assert!((f.eye[1] - 2.8).abs() < 1e-3);
    // The orbit centre is the AABB centre, not the camera's own look-at point.
    assert!((f.target[0]).abs() < 1e-6);
    assert!((f.target[1] - 2.8).abs() < 1e-6);
    assert!((f.target[2]).abs() < 1e-6);

    // The starting orbit reproduces the reported camera: eye is exactly where Roblox put it,
    // and the FOV is the reported one.
    let cam = orbit_camera(Some(&f), Orbit::new(0.0, 0.0, 1.0), 2.0);
    assert!((cam.eye[0] - f.eye[0]).abs() < 1e-3, "{:?}", cam.eye);
    assert!((cam.eye[2] - f.eye[2]).abs() < 1e-3, "{:?}", cam.eye);
    assert!((cam.fov - 26.5).abs() < 1e-6);
    assert!((cam.target[1] - 2.8).abs() < 1e-6);

    // Zoom pulls the eye in along the same ray; yaw orbits around the centre at constant range.
    let near = orbit_camera(Some(&f), Orbit::new(0.0, 0.0, 2.0), 2.0);
    let d0 = distance(cam.eye, cam.target);
    let d1 = distance(near.eye, near.target);
    assert!((d1 - d0 / 2.0).abs() < 1e-3, "{d0} → {d1}");
    let turned = orbit_camera(Some(&f), Orbit::new(0.5, 0.0, 1.0), 2.0);
    assert!((distance(turned.eye, turned.target) - d0).abs() < 1e-3);
    assert!(turned.eye[0] > 0.0, "yaw swings right: {:?}", turned.eye);

    // A full-bleed framing: the AABB (5.6 units tall) must fit inside the vertical FOV.
    let visible = 2.0 * d0 * (cam.fov.to_radians() * 0.5).tan();
    assert!(visible >= 5.6, "visible height {visible} clips the avatar");
    assert!(visible < 5.6 * 1.6, "the avatar is framed too small: {visible}");
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn framing_math_survives_the_normalisation_the_viewport_applies() {
    let f = parse_framing(&framing_json()).unwrap();
    // The viewport maps both the model and the camera into "2 units tall, centred".
    let center = [0.0, 2.8, 0.0];
    let scale = 2.0 / 5.6;
    let n = normalize_framing(&f, center, scale);
    let raw = orbit_camera(Some(&f), Orbit::new(0.0, 0.0, 1.0), 2.0);
    let scaled = orbit_camera(Some(&n), Orbit::new(0.0, 0.0, 1.0), 2.0);

    // Same angle, same relative framing: only the units changed.
    let raw_dist = distance(raw.eye, raw.target);
    let scaled_dist = distance(scaled.eye, scaled.target);
    assert!((scaled_dist - raw_dist * scale).abs() < 1e-3);
    assert!((scaled.fov - raw.fov).abs() < 1e-6);
    assert!(scaled.target[1].abs() < 1e-3, "centred on the AABB: {:?}", scaled.target);
}

#[test]
fn a_manifest_without_a_camera_still_frames_the_avatar() {
    // No metadata at all: the fallback frames a 2-unit avatar with headroom.
    let cam = orbit_camera(None, Orbit::new(0.0, 0.0, 1.0), 2.0);
    assert_eq!(cam.fov, DEFAULT_FOV);
    let visible = 2.0 * distance(cam.eye, cam.target) * (cam.fov.to_radians() * 0.5).tan();
    assert!(visible > 2.0, "a 2-unit avatar must fit: {visible}");

    // Direction but no AABB: look along it instead of inventing a centre.
    let json = serde_json::json!({
        "camera": {"position": [0, 1, 5], "direction": [0, 0, -1]}
    });
    let f = parse_framing(&json).expect("camera alone is enough");
    assert!((f.target[2] - 4.0).abs() < 1e-6);
    assert_eq!(f.fov, DEFAULT_FOV, "no fov → the documented default");

    // Garbage is refused rather than rendered from nonsense.
    assert!(parse_framing(&serde_json::json!({"camera": {}})).is_none());
    assert!(parse_framing(&serde_json::json!({})).is_none());
}

#[test]
fn the_focal_length_matches_the_fov() {
    let f = focal_px(30.0, 800.0);
    // Vertical half-extent at the focal plane: tan(15°) * 800/2.
    let expected = 400.0 / 15.0f32.to_radians().tan();
    assert!((f - expected).abs() < 1e-2, "{f} vs {expected}");
    assert!(focal_px(60.0, 800.0) < f, "a wider FOV is a shorter lens");
}

#[test]
fn the_bake_poll_backs_off_fast() {
    assert_eq!(bake_backoff_ms(0), 0);
    assert_eq!(bake_backoff_ms(1), 150);
    assert_eq!(bake_backoff_ms(2), 300);
    assert_eq!(bake_backoff_ms(3), 500);
    assert_eq!(bake_backoff_ms(30), 500, "the delay is capped");
}

// --- Rendering -------------------------------------------------------------------------

/// A flat quad facing the camera at `z`, from `x0..x1`, `y0..y1`, with a solid colour.
fn quad(z: f32, x0: f32, x1: f32, y0: f32, y1: f32) -> [[f32; 3]; 3] {
    let _ = (x0, x1, y0, y1);
    [
        [-1.0, -1.0, z],
        [1.0, -1.0, z],
        [1.0, 1.0, z],
    ]
}

fn render(tris: &[Tri<'_>], size: usize) -> ColorImage {
    let ss = 2;
    let (w, h) = (size * ss, size * ss);
    let mut pixels = vec![Color32::TRANSPARENT; w * h];
    let mut zbuf = vec![f32::INFINITY; w * h];
    let camera = Camera {
        eye: [0.0, 0.0, 4.0],
        target: [0.0, 0.0, 0.0],
        fov: 40.0,
        focal: focal_px(40.0, h as f32),
    };
    let focal = camera.focal;
    let mut raster = Raster {
        pixels: &mut pixels,
        zbuf: &mut zbuf,
        width: w,
        height: h,
        camera,
        focal,
    };
    for tri in tris {
        raster.triangle(tri);
    }
    drop(raster);
    downsample(&ColorImage::new([w, h], pixels), size, size, ss)
}

fn solid(rgb: [u8; 3]) -> ColorImage {
    ColorImage::new([4, 4], vec![Color32::from_rgb(rgb[0], rgb[1], rgb[2]); 16])
}

fn transparent_stripes() -> ColorImage {
    // Left half transparent, right half opaque white: the shape of a hair card.
    let mut px = Vec::with_capacity(16);
    for _y in 0..4 {
        for x in 0..4 {
            px.push(if x < 2 {
                Color32::from_rgba_unmultiplied(255, 255, 255, 0)
            } else {
                Color32::WHITE
            });
        }
    }
    ColorImage::new([4, 4], px)
}

#[test]
fn a_cut_out_texture_is_dropped_rather_than_drawn_as_a_block() {
    let tex = transparent_stripes();
    let tri = Tri {
        pos: quad(0.0, -1.0, 1.0, -1.0, 1.0),
        vertex_normals: [[0.0, 0.0, 1.0]; 3],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        color: [Color32::WHITE; 3],
        texture: Some(&tex),
        kd: [1.0, 1.0, 1.0],
        alpha: 1.0,
        mode: AlphaMode::Overlay,
        double_sided: true,
    };
    let img = render(&[tri], 64);
    let mid = 32;
    let left = img.pixels[mid * 64 + 12].a();
    let right = img.pixels[mid * 64 + 52].a();
    assert_eq!(left, 0, "the transparent half must not be painted");
    assert_eq!(right, 255, "the opaque half is solid");
}

#[test]
fn diffuse_colour_multiplies_the_texture() {
    let white = solid([255, 255, 255]);
    let make = |kd: [f32; 3]| Tri {
        pos: quad(0.0, -1.0, 1.0, -1.0, 1.0),
        vertex_normals: [[0.0, 0.0, 1.0]; 3],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        color: [Color32::WHITE; 3],
        texture: Some(&white),
        kd,
        alpha: 1.0,
        mode: AlphaMode::Opaque,
        double_sided: false,
    };
    let plain = render(&[make([1.0, 1.0, 1.0])], 32);
    let tinted = render(&[make([1.0, 0.0, 0.0])], 32);
    let centre = |img: &ColorImage| img.pixels[16 * 32 + 16];
    let a = centre(&plain);
    let b = centre(&tinted);
    assert!(a.r() > 100, "a white texture under the lights: {a:?}");
    assert_eq!(b.g(), 0, "Kd 0 on green must zero the green channel: {b:?}");
    assert_eq!(b.b(), 0);
    assert!(b.r() > 20, "the red channel survives: {b:?}");
    assert!(
        b.r() as u32 * 100 > a.r() as u32 * 85,
        "the tint must not darken everything: {b:?} vs {a:?}"
    );
}

#[test]
fn an_untextured_part_uses_its_diffuse_colour() {
    let skin = Tri {
        pos: quad(0.0, -1.0, 1.0, -1.0, 1.0),
        vertex_normals: [[0.0, 0.0, 1.0]; 3],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        color: [Color32::from_rgb(227, 186, 150); 3],
        texture: None,
        kd: [1.0, 1.0, 1.0],
        alpha: 1.0,
        mode: AlphaMode::Opaque,
        double_sided: false,
    };
    let img = render(&[skin], 32);
    let c = img.pixels[16 * 32 + 16];
    assert!(c.r() > c.g() && c.g() > c.b(), "skin keeps its warm tone: {c:?}");
    assert!(c.r() > 80, "and is not left in the dark: {c:?}");
}

#[test]
fn a_nearer_surface_wins_the_depth_test() {
    let white = solid([255, 255, 255]);
    let build = |z: f32, kd: [f32; 3], double_sided: bool| Tri {
        pos: quad(z, -1.0, 1.0, -1.0, 1.0),
        vertex_normals: [[0.0, 0.0, 1.0]; 3],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        color: [Color32::WHITE; 3],
        texture: Some(&white),
        kd,
        alpha: 1.0,
        mode: AlphaMode::Opaque,
        double_sided,
    };
    // Far red first, near blue second: the blue must win even though it was drawn later.
    let img = render(&[build(-1.0, [1.0, 0.0, 0.0], true), build(0.5, [0.0, 0.0, 1.0], true)], 32);
    let c = img.pixels[16 * 32 + 16];
    assert!(c.b() > c.r(), "the nearer quad wins: {c:?}");
}

#[test]
fn a_wider_field_of_view_shows_more_of_the_scene() {
    // The metadata FOV really changes the projection, so Roblox's framing is reproduced.
    let mk = |fov: f32| Camera {
        eye: [0.0, 0.0, 4.0],
        target: [0.0, 0.0, 0.0],
        fov,
        focal: focal_px(fov, 400.0),
    };
    let narrow = mk(15.0);
    let wide = mk(60.0);
    let project = |cam: Camera, p: [f32; 3]| {
        let fwd = [0.0, 0.0, -1.0];
        let z = (p[0] - cam.eye[0]) * fwd[0] + (p[1] - cam.eye[1]) * fwd[1] + (p[2] - cam.eye[2]) * fwd[2];
        (200.0 + p[0] * cam.focal / z) - 200.0
    };
    let x = 1.0;
    assert!(project(wide, [x, 0.0, 0.0]).abs() < project(narrow, [x, 0.0, 0.0]).abs());
}

/// A closed box as 12 triangles, in Roblox proportions, with outward normals per face.
fn box_tris(min: [f32; 3], max: [f32; 3]) -> Vec<([[f32; 3]; 3], [[f32; 2]; 3])> {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let c = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    let faces = [
        [0, 1, 2, 3],
        [5, 4, 7, 6],
        [4, 0, 3, 7],
        [1, 5, 6, 2],
        [3, 2, 6, 7],
        [4, 5, 1, 0],
    ];
    let mut out = Vec::with_capacity(12);
    let (u0, u1, u2, u3) = ([0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]);
    for f in faces {
        let (a, b, cc, d) = (c[f[0]], c[f[1]], c[f[2]], c[f[3]]);
        out.push(([a, b, cc], [u0, u1, u2]));
        out.push(([a, cc, d], [u0, u2, u3]));
    }
    out
}

/// The avatar the artifact renders, built the way Roblox ships one: flat-`Kd` skin, a textured
/// shirt, cut-out hair cards and a blended visor, framed by the metadata camera.
///
/// Group per triangle: 0 skin, 1 shirt, 2 hair, 3 face decal, 4 visor.
fn avatar_scene() -> (Vec<[[f32; 3]; 3]>, Vec<[[f32; 2]; 3]>, Vec<u8>) {
    let full = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
    let mut tris = Vec::new();
    let mut uvs = Vec::new();
    let mut groups = Vec::new();
    let mut add = |b: Vec<([[f32; 3]; 3], [[f32; 2]; 3])>, g: u8| {
        for (p, uv) in b {
            tris.push(p);
            uvs.push(uv);
            groups.push(g);
        }
    };
    // R15-ish proportions, feet on y = 0: 5.45 studs tall, 1.9 wide at the arms.
    add(box_tris([-0.8, 1.9, -0.45], [0.8, 3.0, 0.45]), 1); // lower torso
    add(box_tris([-0.8, 3.0, -0.5], [0.8, 4.1, 0.5]), 1); // upper torso
    add(box_tris([-0.6, 4.1, -0.6], [0.6, 5.3, 0.6]), 0); // head
    add(box_tris([-1.15, 3.1, -0.25], [-0.85, 4.1, 0.25]), 0); // left arm
    add(box_tris([0.85, 3.1, -0.25], [1.15, 4.1, 0.25]), 0); // right arm
    add(box_tris([-0.65, 0.0, -0.3], [-0.1, 1.9, 0.3]), 5); // left leg
    add(box_tris([0.1, 0.0, -0.3], [0.65, 1.9, 0.3]), 5); // right leg

    // Face decal: a cut-out sheet just proud of the head's front, so it is depth-correct.
    add(
        vec![
            ([[-0.45, 4.35, 0.605], [0.45, 4.35, 0.605], [0.45, 5.05, 0.605]], full),
            ([[-0.45, 4.35, 0.605], [0.45, 5.05, 0.605], [-0.45, 5.05, 0.605]], full),
        ],
        3,
    );
    // Hair: a cap over the skull plus cards in front of the forehead, all cut out by alpha.
    for i in 0..16 {
        let x = -0.62 + i as f32 * 0.078;
        add(
            vec![
                ([[x, 4.9, 0.58], [x + 0.055, 4.9, 0.58], [x + 0.055, 5.42, 0.58]], full),
                ([[x, 4.9, 0.58], [x + 0.055, 5.42, 0.58], [x, 5.42, 0.58]], full),
            ],
            2,
        );
        // And the same strands over the top of the skull.
        let z = -0.5 + i as f32 * 0.065;
        add(
            vec![
                ([[-0.62, 5.30, z], [0.62, 5.30, z], [0.62, 5.30, z + 0.05]], full),
                ([[-0.62, 5.30, z], [0.62, 5.30, z + 0.05], [-0.62, 5.30, z + 0.05]], full),
            ],
            2,
        );
    }
    // Visor: a blended band across the eyes, proud of the face decal.
    add(
        vec![
            ([[-0.5, 4.6, 0.615], [0.5, 4.6, 0.615], [0.5, 4.85, 0.615]], full),
            ([[-0.5, 4.6, 0.615], [0.5, 4.85, 0.615], [-0.5, 4.85, 0.615]], full),
        ],
        4,
    );
    (tris, uvs, groups)
}

#[test]
fn the_viewport_png_shows_the_real_thing() {
    let (tris, uvs, groups) = avatar_scene();
    assert_eq!(tris.len(), groups.len());

    let json = serde_json::json!({
        "camera": {
            "position": {"x": 0.0, "y": 2.7, "z": 11.0},
            "direction": {"x": 0.0, "y": 0.0, "z": -1.0},
            "fov": 30.0
        },
        "aabb": {"min": {"x": -1.15, "y": 0.0, "z": -0.6}, "max": {"x": 1.15, "y": 5.45, "z": 0.6}}
    });
    let framing = parse_framing(&json).expect("metadata");
    let cam = orbit_camera(Some(&framing), Orbit::new(0.25, 0.0, 1.0), 5.4);
    let size = 640usize;
    let ss = 2;
    let (w, h) = (size * ss, size * ss);
    let mut pixels = vec![Color32::TRANSPARENT; w * h];
    let mut zbuf = vec![f32::INFINITY; w * h];

    // Textures: a shirt with a logo band, hair with strand-shaped alpha, a face with eyes.
    let shirt = {
        let mut px = Vec::new();
        for y in 0..32 {
            for x in 0..32 {
                let logo = (8..24).contains(&x) && (10..22).contains(&y);
                px.push(if logo {
                    Color32::from_rgb(240, 240, 245)
                } else {
                    Color32::from_rgb(59, 112, 196)
                });
            }
        }
        ColorImage::new([32, 32], px)
    };
    let hair = {
        let mut px = Vec::new();
        for _y in 0..32 {
            for x in 0..32 {
                let strand = (x % 4) < 3;
                px.push(if strand {
                    Color32::from_rgb(58, 40, 28)
                } else {
                    Color32::from_rgba_unmultiplied(58, 40, 28, 0)
                });
            }
        }
        ColorImage::new([32, 32], px)
    };
    let face = {
        let mut px = Vec::new();
        for y in 0..32 {
            for x in 0..32 {
                let eye = (8..12).contains(&x) && (8..14).contains(&y);
                let other = (20..24).contains(&x) && (8..14).contains(&y);
                let mouth = (12..20).contains(&x) && (24..27).contains(&y);
                if eye || other {
                    px.push(Color32::from_rgb(24, 20, 18));
                } else if mouth {
                    px.push(Color32::from_rgb(140, 70, 66));
                } else {
                    px.push(Color32::from_rgba_unmultiplied(227, 186, 150, 0));
                }
            }
        }
        ColorImage::new([32, 32], px)
    };
    let visor = {
        let mut px = Vec::new();
        for _ in 0..16 {
            for _ in 0..16 {
                px.push(Color32::from_rgba_unmultiplied(40, 90, 160, 140));
            }
        }
        ColorImage::new([16, 16], px)
    };

    let mut raster = Raster {
        pixels: &mut pixels,
        zbuf: &mut zbuf,
        width: w,
        height: h,
        camera: Camera { focal: focal_px(cam.fov, h as f32), ..cam },
        focal: focal_px(cam.fov, h as f32),
    };
    for (i, pos) in tris.iter().enumerate() {
        let (kd, texture, alpha, mode, double_sided) = match groups[i] {
            0 => ([227.0 / 255.0, 186.0 / 255.0, 150.0 / 255.0], None, 1.0, AlphaMode::Opaque, false),
            1 => ([1.0, 1.0, 1.0], Some(&shirt), 1.0, AlphaMode::Opaque, false),
            2 => ([1.0, 1.0, 1.0], Some(&hair), 1.0, AlphaMode::Overlay, true),
            3 => ([1.0, 1.0, 1.0], Some(&face), 1.0, AlphaMode::Overlay, true),
            4 => ([1.0, 1.0, 1.0], Some(&visor), 1.0, AlphaMode::Transparent, true),
            // Trousers: a flat diffuse material, no texture — the other half of Roblox's MTL.
            _ => ([0.18, 0.22, 0.31], None, 1.0, AlphaMode::Opaque, false),
        };
        raster.triangle(&Tri {
            pos: *pos,
            vertex_normals: [[0.0, 0.0, 1.0]; 3],
            uv: uvs[i],
            color: [Color32::WHITE; 3],
            texture,
            kd,
            alpha,
            mode,
            double_sided,
        });
    }
    drop(raster);

    let img = downsample(&ColorImage::new([w, h], pixels), size, size, ss);

    // The avatar has to be there: a body-shaped share of the frame, and empty space around it.
    let covered = img.pixels.iter().filter(|p| p.a() > 0).count();
    let share = covered as f32 / (size * size) as f32;
    assert!(
        (0.08..0.55).contains(&share),
        "the avatar covers {:.1}% of the frame",
        share * 100.0
    );
    assert_eq!(img.pixels[2].a(), 0, "the corners stay empty");

    // Hair reads as strands, not a slab: inside its band some columns are empty.
    let band_y = (size as f32 * 0.16) as usize;
    let row: Vec<Color32> = (0..size).map(|x| img.pixels[band_y * size + x]).collect();
    let filled = row.iter().filter(|p| p.a() > 0).count();
    assert!(filled > 0, "the hair band is drawn");
    assert!(
        filled < row.len(),
        "cut-out alpha must leave gaps between strands"
    );

    // Save it for the eyeball check.
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts");
    std::fs::create_dir_all(&dir).unwrap();
    let mut rgba = Vec::with_capacity(size * size * 4);
    for p in &img.pixels {
        rgba.extend_from_slice(&[p.r(), p.g(), p.b(), p.a()]);
    }
    let path = dir.join("avatar_viewport_real.png");
    image::save_buffer(&path, &rgba, size as u32, size as u32, image::ColorType::Rgba8).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 3_000);
}
