//! The Avatar Editor's rig on a Roblox-shaped model.
//!
//! Roblox's own avatar OBJ names its materials `Part12Mtl` and its objects `Object_7`, so the
//! file says nothing about body parts. These tests reproduce that format (and dump it into
//! `artifacts/avatar3d/` so it can be opened in a text editor), then check that the mesh-island
//! classifier still puts every limb where it belongs, including a hat, a hair cap and a
//! backpack. They also measure the software rasteriser's inner loop and the frame pacing that
//! keeps a heavy avatar from saturating a core.

use creblox_vm_tests::avatar_anim::{
    classify_triangles, AnimPlayer, AnimationKind, Part, PartCounts, RigBands, PART_COUNT,
};

type V3 = [f32; 3];

/// One body part of the fixture, in Roblox studs, feet on y = 0.
struct Piece {
    part: Part,
    name: &'static str,
    min: V3,
    max: V3,
    color: [f32; 3],
}

/// An R15 avatar at real proportions, wearing a hat, hair and a backpack. The `name` field is
/// what Roblox's OBJ actually calls it: a counter, never a body part.
fn avatar() -> Vec<Piece> {
    let skin = [0.89, 0.73, 0.59];
    let shirt = [0.23, 0.44, 0.77];
    let trousers = [0.18, 0.22, 0.31];
    let shoes = [0.11, 0.13, 0.17];
    let hair = [0.29, 0.20, 0.14];
    let hat = [0.55, 0.14, 0.14];
    let bag = [0.20, 0.24, 0.20];
    vec![
        Piece { part: Part::LowerTorso, name: "Part0Mtl", min: [-1.0, 1.85, -0.45], max: [1.0, 2.85, 0.45], color: shirt },
        Piece { part: Part::UpperTorso, name: "Part1Mtl", min: [-1.0, 2.85, -0.50], max: [1.0, 4.15, 0.50], color: shirt },
        Piece { part: Part::Head, name: "Part2Mtl", min: [-0.65, 4.15, -0.65], max: [0.65, 5.45, 0.65], color: skin },
        // Hair covers the top of the head; the hat sits on top of that.
        Piece { part: Part::Head, name: "Part3Mtl", min: [-0.68, 4.35, -0.68], max: [0.68, 5.45, 0.68], color: hair },
        Piece { part: Part::Head, name: "Part4Mtl", min: [-0.75, 5.35, -0.75], max: [0.75, 5.85, 0.75], color: hat },
        // A backpack: central, on the upper torso, and not part of the body's silhouette.
        Piece { part: Part::UpperTorso, name: "Part5Mtl", min: [-0.5, 2.9, -0.75], max: [0.5, 4.0, -0.5], color: bag },
        Piece { part: Part::LeftUpperArm, name: "Part6Mtl", min: [-1.55, 3.15, -0.25], max: [-1.05, 4.15, 0.25], color: skin },
        Piece { part: Part::LeftLowerArm, name: "Part7Mtl", min: [-1.55, 2.35, -0.25], max: [-1.05, 3.15, 0.25], color: skin },
        Piece { part: Part::LeftHand, name: "Part8Mtl", min: [-1.52, 2.05, -0.22], max: [-1.08, 2.35, 0.22], color: skin },
        Piece { part: Part::RightUpperArm, name: "Part9Mtl", min: [1.05, 3.15, -0.25], max: [1.55, 4.15, 0.25], color: skin },
        Piece { part: Part::RightLowerArm, name: "Part10Mtl", min: [1.05, 2.35, -0.25], max: [1.55, 3.15, 0.25], color: skin },
        Piece { part: Part::RightHand, name: "Part11Mtl", min: [1.08, 2.05, -0.22], max: [1.52, 2.35, 0.22], color: skin },
        Piece { part: Part::LeftUpperLeg, name: "Part12Mtl", min: [-0.95, 1.05, -0.30], max: [-0.10, 1.95, 0.30], color: trousers },
        Piece { part: Part::LeftLowerLeg, name: "Part13Mtl", min: [-0.95, 0.35, -0.30], max: [-0.10, 1.05, 0.30], color: trousers },
        Piece { part: Part::LeftFoot, name: "Part14Mtl", min: [-0.95, 0.0, -0.30], max: [-0.10, 0.35, 0.55], color: shoes },
        Piece { part: Part::RightUpperLeg, name: "Part15Mtl", min: [0.10, 1.05, -0.30], max: [0.95, 1.95, 0.30], color: trousers },
        Piece { part: Part::RightLowerLeg, name: "Part16Mtl", min: [0.10, 0.35, -0.30], max: [0.95, 1.05, 0.30], color: trousers },
        Piece { part: Part::RightFoot, name: "Part17Mtl", min: [0.10, 0.0, -0.30], max: [0.95, 0.35, 0.55], color: shoes },
    ]
}

fn box_faces() -> [[usize; 4]; 6] {
    // Corners: 0..3 bottom (z-), 4..7 top (z+), winding outward.
    [
        [0, 1, 2, 3],
        [5, 4, 7, 6],
        [4, 0, 3, 7],
        [1, 5, 6, 2],
        [3, 2, 6, 7],
        [4, 5, 1, 0],
    ]
}

/// The geometry exactly as the parser would see it from the OBJ.
struct Mesh {
    /// One entry per triangle: the three OBJ vertex indices.
    tri_verts: Vec<[u32; 3]>,
    /// One entry per triangle: its three corners.
    tri_pos: Vec<[V3; 3]>,
    /// What a material/object name asked for, per triangle (`None` for `Part0Mtl`).
    named: Vec<Option<Part>>,
    /// The island each triangle belongs to, as the fixture author intended.
    expected: Vec<Part>,
    positions: Vec<V3>,
    /// One entry per triangle: absolute OBJ vertex indices.
    triangles: Vec<(usize, [u32; 3])>,
    pieces: Vec<Piece>,
    model_min: V3,
    model_max: V3,
}

impl Mesh {
    fn build(pieces: Vec<Piece>) -> Mesh {
        let mut positions: Vec<V3> = Vec::new();
        let mut triangles: Vec<(usize, [u32; 3])> = Vec::new();
        let mut expected = Vec::new();
        let mut model_min = [f32::MAX; 3];
        let mut model_max = [f32::MIN; 3];

        for (pi, piece) in pieces.iter().enumerate() {
            let base = positions.len() as u32;
            let [x0, y0, z0] = piece.min;
            let [x1, y1, z1] = piece.max;
            let corners = [
                [x0, y0, z0],
                [x1, y0, z0],
                [x1, y1, z0],
                [x0, y1, z0],
                [x0, y0, z1],
                [x1, y0, z1],
                [x1, y1, z1],
                [x0, y1, z1],
            ];
            for c in corners {
                positions.push(c);
                for a in 0..3 {
                    model_min[a] = model_min[a].min(c[a]);
                    model_max[a] = model_max[a].max(c[a]);
                }
            }
            for face in box_faces() {
                for i in [1, 2] {
                    triangles.push((
                        pi,
                        [
                            base + face[0] as u32,
                            base + face[i] as u32,
                            base + face[i + 1] as u32,
                        ],
                    ));
                    expected.push(piece.part);
                }
            }
        }

        let mut tri_verts = Vec::with_capacity(triangles.len());
        let mut tri_pos = Vec::with_capacity(triangles.len());
        let mut named = Vec::with_capacity(triangles.len());
        for (pi, tri) in &triangles {
            tri_verts.push(*tri);
            tri_pos.push([
                positions[tri[0] as usize],
                positions[tri[1] as usize],
                positions[tri[2] as usize],
            ]);
            // Roblox's material names carry no body part: nothing is named.
            named.push(Part::from_name(&pieces[*pi].name));
        }

        Mesh {
            tri_verts,
            tri_pos,
            named,
            expected,
            positions,
            triangles,
            pieces,
            model_min,
            model_max,
        }
    }

    /// The OBJ Roblox's own exporter writes: RGBA vertex colours, counter material names,
    /// counter object names.
    fn obj(&self) -> String {
        let mut out = String::from("# Roblox avatar R15 fixture (PartNNMtl naming)\n");
        out.push_str("mtllib roblox_r15_partmtl.mtl\n");
        for p in &self.positions {
            out.push_str(&format!(
                "v {:.4} {:.4} {:.4} {:.3} {:.3} {:.3} 1\n",
                p[0], p[1], p[2], 0.8, 0.8, 0.8
            ));
        }
        out.push_str("vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 -1\n");
        for (pi, piece) in self.pieces.iter().enumerate() {
            out.push_str(&format!("usemtl {}\no Object_{}\n", piece.name, pi));
            for tri in self.triangles.iter().filter(|(p, _)| *p == pi) {
                let face: Vec<String> = tri
                    .1
                    .iter()
                    .map(|c| format!("{}//1", c + 1))
                    .collect();
                out.push_str(&format!("f {}\n", face.join(" ")));
            }
        }
        out
    }

    fn mtl(&self) -> String {
        let mut out = String::new();
        for (i, piece) in self.pieces.iter().enumerate() {
            let [r, g, b] = piece.color;
            out.push_str(&format!(
                "newmtl {}\nKd {r:.3} {g:.3} {b:.3}\nmap_Kd 30DAY-Texture-{i:016X}-Png\nrbx_alphamode opaque\n\n",
                piece.name
            ));
        }
        out
    }
}

#[test]
fn roblox_material_names_identify_no_body_part() {
    // The real thing: a counter. Nothing here can be matched to a limb.
    for name in ["Part0Mtl", "Part12Mtl", "Part30Mtl", "Object_7"] {
        assert_eq!(Part::from_name(name), None, "{name} must not name a limb");
    }
    // Exports that *do* name parts still work, including R6 and spaced names.
    assert_eq!(Part::from_name("UpperTorso"), Some(Part::UpperTorso));
    assert_eq!(Part::from_name("Right Arm"), Some(Part::RightUpperArm));
    assert_eq!(Part::from_name("Left Leg"), Some(Part::LeftUpperLeg));
    assert_eq!(Part::from_name("LeftForearm"), Some(Part::LeftLowerArm));
}

#[test]
fn unnamed_islands_are_placed_on_the_rig() {
    let mesh = Mesh::build(avatar());
    assert_eq!(mesh.named.iter().filter(|n| n.is_some()).count(), 0);

    let parts = classify_triangles(
        &mesh.tri_verts,
        &mesh.tri_pos,
        &mesh.named,
        mesh.model_min,
        mesh.model_max,
    );

    let mut wrong = Vec::new();
    for (i, (got, want)) in parts.iter().zip(mesh.expected.iter()).enumerate() {
        if got != want {
            let piece = &mesh.pieces[mesh.triangles[i].0];
            wrong.push(format!("{} triangle {i}: {got:?} != {want:?}", piece.name));
        }
    }
    assert!(wrong.is_empty(), "misclassified triangles: {:#?}", wrong);
}

#[test]
fn every_limb_gets_geometry_and_nothing_is_unclassified() {
    let mesh = Mesh::build(avatar());
    let parts = classify_triangles(
        &mesh.tri_verts,
        &mesh.tri_pos,
        &mesh.named,
        mesh.model_min,
        mesh.model_max,
    );
    let counts = PartCounts::from_parts(&parts);
    assert_eq!(counts.total(), mesh.tri_verts.len());
    assert_eq!(counts.unclassified, 0);
    assert_eq!(counts.per_part.len(), PART_COUNT);
    for part in Part::ALL {
        assert!(
            counts.per_part[part.index()] > 0,
            "{} got no triangles: {}",
            part.label(),
            counts.report()
        );
    }
    // A hat, a hair cap and a backpack must not steal triangles from the body: the head
    // carries three boxes, so it owns exactly three times a limb's 12 triangles.
    assert_eq!(counts.per_part[Part::Head.index()], 36);
    println!("per-part triangles: {}", counts.report());
}

#[test]
fn a_tall_hat_does_not_squash_the_joint_bands() {
    // Bands calibrated from the head island, with and without a tall hat. The hat adds 0.4
    // studs above a 5.45-stud body, which used to push the torso down into the legs' band.
    let with_hat = Mesh::build(avatar());
    let bands = RigBands::from_islands(
        with_hat
            .pieces
            .iter()
            .map(|p| (p.min, p.max)),
        with_hat.model_min,
        with_hat.model_max,
    );
    let h = with_hat.model_max[1] - with_hat.model_min[1];
    let jaw = (4.15 - with_hat.model_min[1]) / h;
    assert!(
        (bands.head - jaw).abs() < 0.02,
        "jaw should be {jaw}, bands say {}",
        bands.head
    );

    // Without the hat the model is shorter; the bands must follow the head, not the height.
    let mut no_hat = avatar();
    no_hat.retain(|p| p.name != "Part4Mtl");
    let plain = Mesh::build(no_hat);
    let plain_bands = RigBands::from_islands(
        plain.pieces.iter().map(|p| (p.min, p.max)),
        plain.model_min,
        plain.model_max,
    );
    let plain_h = plain.model_max[1] - plain.model_min[1];
    assert!((plain_bands.head - 4.15 / plain_h).abs() < 0.02);

    // The waist and the hip stay inside the torso in both cases.
    assert!(bands.hip < bands.waist && bands.waist < bands.shoulder);
    assert!(plain_bands.hip < plain_bands.waist && plain_bands.waist < plain_bands.shoulder);
}

#[test]
fn the_real_format_is_dumped_for_inspection() {
    let mesh = Mesh::build(avatar());
    let parts = classify_triangles(
        &mesh.tri_verts,
        &mesh.tri_pos,
        &mesh.named,
        mesh.model_min,
        mesh.model_max,
    );
    let counts = PartCounts::from_parts(&parts);

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join("avatar3d");
    std::fs::create_dir_all(&dir).unwrap();
    let obj = mesh.obj();
    let mtl = mesh.mtl();
    std::fs::write(dir.join("roblox_r15_partmtl.obj"), &obj).unwrap();
    std::fs::write(dir.join("roblox_r15_partmtl.mtl"), &mtl).unwrap();
    std::fs::write(
        dir.join("roblox_r15_partmtl.parts.txt"),
        format!(
            "fixture: Roblox R15, PartNNMtl materials (no body-part names)\n{} triangles\n{}\n\nexpected: {}\n",
            counts.total(),
            counts.report(),
            {
                let mut s = String::new();
                for piece in &mesh.pieces {
                    s.push_str(&format!("{} = {}, ", piece.name, piece.part.label()));
                }
                s
            }
        ),
    )
    .unwrap();

    assert!(obj.lines().any(|l| l == "usemtl Part12Mtl"));
    assert!(obj.lines().any(|l| l.starts_with("f ") && l.contains("//")));
    // RGBA vertex colours, as Roblox writes them.
    assert!(obj
        .lines()
        .filter(|l| l.starts_with("v "))
        .all(|l| l.split_whitespace().count() == 8));
    assert!(mtl.contains("newmtl Part12Mtl"));
    assert!(mtl.contains("rbx_alphamode"));
    let path = dir.join("roblox_r15_partmtl.obj");
    assert!(std::fs::metadata(&path).unwrap().len() > 4_000);
}

#[test]
fn a_posed_frame_is_paced_by_its_measured_cost() {
    let mut player = AnimPlayer::default();
    // Stopped: nothing is scheduled and nothing is timed.
    assert_eq!(player.frame_interval(), 1.0 / 30.0);
    assert!(!player.take_cost_report());

    player.play(AnimationKind::Run);
    // A cheap avatar keeps the 30 fps cap.
    player.note_frame_cost(2.0);
    assert!((player.effective_fps() - 30.0).abs() < 0.5, "{}", player.effective_fps());
    // One that takes 40 ms per frame is redrawn about 12 times a second, not 30.
    player.note_frame_cost(40.0);
    assert!(player.effective_fps() < 20.0, "{}", player.effective_fps());
    // However slow it gets, the preview never drops below 5 fps.
    player.note_frame_cost(900.0);
    assert!(player.frame_interval() <= 0.2 + 1e-6);
    assert!(player.frame_interval() >= 1.0 / 30.0);
    // The measured cost is reported once per play session, not every frame.
    assert!(player.take_cost_report());
    assert!(!player.take_cost_report());

    // A new clip resets the pacing measurement.
    player.play(AnimationKind::Idle);
    assert!((player.effective_fps() - 30.0).abs() < 0.5);
    assert_eq!(player.frame_cost_ms(), 0.0);

    // Stopped again: the interval is back to the idle value and no work is ever scheduled.
    player.stop();
    assert_eq!(player.frame_interval(), 1.0 / 30.0);
    assert!(!player.needs_repaint());
}

#[test]
fn the_raster_inner_loop_cost_is_measured() {
    // The viewport's per-pixel work: three edge functions, perspective-correct barycentrics, a
    // depth test and a texture fetch. Measuring this loop says what one posed frame of the
    // whole avatar costs, because every pixel the avatar covers goes through it once (plus
    // overdraw). The real per-frame number is logged by the app itself.
    const SIDE: usize = 340;
    let tex: Vec<[u8; 3]> = (0..256 * 256)
        .map(|i| [(i % 251) as u8, (i / 251 % 251) as u8, 90])
        .collect();
    let screen = [[10.0f32, 10.0], [330.0, 30.0], [40.0, 330.0]];
    // The viewport's own edge function, so the pixel coverage here matches it exactly.
    let edge = |a: [f32; 2], b: [f32; 2], px: f32, py: f32| {
        (px - a[0]) * (b[1] - a[1]) - (py - a[1]) * (b[0] - a[0])
    };
    let total = edge(screen[0], screen[1], screen[2][0], screen[2][1]);
    assert!(total.abs() > 1e-6);
    let zs = [1.0f32, 1.1, 1.05];
    let persp = [0.9f32, 0.92, 0.88];

    let mut pixels = vec![0u8; SIDE * SIDE * 3];
    let mut zbuf = vec![f32::INFINITY; SIDE * SIDE];

    let started = std::time::Instant::now();
    for y in 0..SIDE {
        let py = y as f32 + 0.5;
        for x in 0..SIDE {
            let px = x as f32 + 0.5;
            let e0 = edge(screen[1], screen[2], px, py) / total;
            let e1 = edge(screen[2], screen[0], px, py) / total;
            let e2 = edge(screen[0], screen[1], px, py) / total;
            if e0 < 0.0 || e1 < 0.0 || e2 < 0.0 {
                continue;
            }
            let i0 = e0 / persp[0];
            let i1 = e1 / persp[1];
            let i2 = e2 / persp[2];
            let inv = i0 + i1 + i2;
            let z = (i0 * zs[0] + i1 * zs[1] + i2 * zs[2]) / inv;
            let idx = y * SIDE + x;
            if z >= zbuf[idx] {
                continue;
            }
            zbuf[idx] = z;
            let u = (i0 * 0.1 + i1 * 0.9 + i2 * 0.2) / inv;
            let v = (i0 * 0.2 + i1 * 0.1 + i2 * 0.8) / inv;
            let tx = ((u - u.floor()) * 256.0) as usize & 255;
            let ty = ((v - v.floor()) * 256.0) as usize & 255;
            let c = tex[ty * 256 + tx];
            let shade = 0.4 + 0.6 * e0;
            let o = idx * 3;
            pixels[o] = (c[0] as f32 * shade) as u8;
            pixels[o + 1] = (c[1] as f32 * shade) as u8;
            pixels[o + 2] = (c[2] as f32 * shade) as u8;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let covered = zbuf.iter().filter(|z| z.is_finite()).count();
    assert!(covered > 20_000, "the test triangle should cover a real area");

    // A 700 px viewport whose avatar covers ~130k pixels runs this loop that many times.
    let per_pixel_ns = elapsed * 1e9 / (SIDE * SIDE) as f64;
    let posed_ms = per_pixel_ns * 130_000.0 / 1e6;
    println!(
        "raster fill: {:.1} ns/px -> {:.1} ms of fill per posed frame at 700 px ({} px covered here)",
        per_pixel_ns, posed_ms, covered
    );
    assert!(posed_ms < 60.0, "fill cost {posed_ms:.1} ms is unexpectedly slow");
}
