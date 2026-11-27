//! Avatar animation: hierarchical joint transforms, keyframed clips, zero-CPU playback state,
//! and a rendered mid-jump PNG in `vm-tests/artifacts/`.

use creblox_vm_tests::avatar_anim::{
    AnimationKind, AnimPlayer, Part, Pose, Skeleton, PART_COUNT,
};
use std::f32::consts::FRAC_PI_2;

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn child_joints_follow_their_parent_without_detaching() {
    // Right arm chain: shoulder, elbow, wrist.
    let mut pivots = [[0.0f32; 3]; PART_COUNT];
    pivots[Part::UpperTorso.index()] = [0.0, 1.6, 0.0];
    pivots[Part::RightUpperArm.index()] = [0.7, 1.6, 0.0];
    pivots[Part::RightLowerArm.index()] = [0.7, 1.1, 0.0];
    pivots[Part::RightHand.index()] = [0.7, 0.75, 0.0];
    pivots[Part::LowerTorso.index()] = [0.0, 1.0, 0.0];
    let skel = Skeleton::new(pivots);

    let rest_shoulder = pivots[Part::RightUpperArm.index()];
    let rest_elbow = pivots[Part::RightLowerArm.index()];
    let rest_wrist = pivots[Part::RightHand.index()];

    let mut pose = Pose::identity();
    pose.set(Part::RightUpperArm, 0.0, 0.0, 1.1);
    let t = skel.transforms(&pose);

    let shoulder = t[Part::RightUpperArm.index()].transform_point(rest_shoulder);
    let elbow = t[Part::RightUpperArm.index()].transform_point(rest_elbow);
    assert!((dist(shoulder, elbow) - dist(rest_shoulder, rest_elbow)).abs() < 1e-4);

    // The lower arm is still socketed at the (moved) elbow: its pivot maps to the same point.
    let elbow_by_child = t[Part::RightLowerArm.index()].transform_point(rest_elbow);
    assert!(dist(elbow, elbow_by_child) < 1e-3, "{elbow:?} vs {elbow_by_child:?}");

    // Both joints bend the hand further than either alone.
    let mut both = Pose::identity();
    both.set(Part::RightUpperArm, 0.0, 0.0, 0.5);
    both.set(Part::RightLowerArm, 0.0, 0.0, 0.5);
    let t2 = skel.transforms(&both);
    let wrist = t2[Part::RightHand.index()].transform_point(rest_wrist);
    let shoulder2 = t2[Part::RightUpperArm.index()].transform_point(rest_shoulder);
    // Two bent joints fold the hand closer to the shoulder than a straight arm.
    assert!(
        dist(wrist, shoulder2) < dist(rest_wrist, rest_shoulder) - 0.01,
        "wrist {wrist:?} shoulder {shoulder2:?}"
    );
}

#[test]
fn a_pose_rotates_a_limb_about_its_own_pivot() {
    let mut pivots = [[0.0f32; 3]; PART_COUNT];
    pivots[Part::LowerTorso.index()] = [0.0, 1.0, 0.0];
    pivots[Part::LeftUpperLeg.index()] = [-0.2, 1.0, 0.0];
    pivots[Part::LeftLowerLeg.index()] = [-0.2, 0.55, 0.0];
    pivots[Part::LeftFoot.index()] = [-0.2, 0.1, 0.0];
    let skel = Skeleton::new(pivots);

    let mut pose = Pose::identity();
    pose.set(Part::LeftUpperLeg, FRAC_PI_2, 0.0, 0.0);
    let t = skel.transforms(&pose);

    let pivot = t[Part::LeftUpperLeg.index()].transform_point(pivots[Part::LeftUpperLeg.index()]);
    assert!(dist(pivot, pivots[Part::LeftUpperLeg.index()]) < 1e-4);
    let foot = t[Part::LeftFoot.index()].transform_point(pivots[Part::LeftFoot.index()]);
    // Rotating +90 degrees about X swings the leg's lower end forward/back, not down.
    assert!((foot[1] - 1.0).abs() < 0.35);
}

#[test]
fn jump_keys_crouch_launch_and_land_in_a_loop() {
    let keys = AnimationKind::Jump.keys();
    assert!(keys.len() >= 8);
    let start = creblox_vm_tests::avatar_anim::eval(AnimationKind::Jump, 0.0);
    let crouch = creblox_vm_tests::avatar_anim::eval(AnimationKind::Jump, 0.12);
    let air = creblox_vm_tests::avatar_anim::eval(AnimationKind::Jump, 0.45);
    let end = creblox_vm_tests::avatar_anim::eval(AnimationKind::Jump, 0.999);

    // Crouch: knees bent and arms drawn back.
    let knee = crouch.get(Part::LeftLowerLeg)[0];
    assert!(knee > 0.5, "expected a bent knee while crouching, got {knee}");
    assert!(crouch.get(Part::LeftUpperArm)[0] < -1.0);
    // Airborne: knees tucked and arms thrown up above the shoulders.
    assert!(air.get(Part::LeftUpperArm)[0] < -2.0);
    assert!(air.get(Part::LeftUpperLeg)[0] < -0.4);
    // The loop ends where it started (standing), so it can wrap without a snap.
    assert!(dist(start.get(Part::LeftUpperArm), end.get(Part::LeftUpperArm)) < 0.15);
}

#[test]
fn run_leans_forward_and_alternates_its_limbs() {
    let phase = creblox_vm_tests::avatar_anim::eval(AnimationKind::Run, 0.25);
    assert!(
        phase.get(Part::LowerTorso)[0] < -0.1,
        "a sprint should lean the torso forward"
    );
    let left_leg = phase.get(Part::LeftUpperLeg)[0];
    let right_leg = phase.get(Part::RightUpperLeg)[0];
    assert!(left_leg * right_leg < 0.0, "legs must be opposed");
    let left_arm = phase.get(Part::LeftUpperArm)[0];
    let right_arm = phase.get(Part::RightUpperArm)[0];
    assert!(left_arm * right_arm < 0.0, "arms must pump in opposition");
    // Knees are bent for a sprint, not straight.
    assert!(phase.get(Part::LeftLowerArm)[0].abs() > 0.9);
}

#[test]
fn idle_is_a_gentle_breathing_loop() {
    let a = creblox_vm_tests::avatar_anim::eval(AnimationKind::Idle, 0.0);
    let b = creblox_vm_tests::avatar_anim::eval(AnimationKind::Idle, 0.5);
    assert_ne!(a, b);
    // Subtle: no joint swings anywhere near the amplitude of a run.
    for part in Part::ALL {
        for c in 0..3 {
            assert!(
                a.get(part)[c].abs() < 0.2,
                "idle should stay calm, {part:?} moved {}",
                a.get(part)[c]
            );
        }
    }
    let wrap = creblox_vm_tests::avatar_anim::eval(AnimationKind::Idle, 1.0);
    assert!(dist(a.get(Part::Head), wrap.get(Part::Head)) < 0.05);
}

#[test]
fn every_clip_type_is_recognised_from_asset_ids_and_names() {
    let cases = [
        (48, "ClimbAnimation", AnimationKind::Climb),
        (50, "FallAnimation", AnimationKind::Fall),
        (51, "IdleAnimation", AnimationKind::Idle),
        (52, "JumpAnimation", AnimationKind::Jump),
        (53, "RunAnimation", AnimationKind::Run),
        (54, "SwimAnimation", AnimationKind::Swim),
        (55, "WalkAnimation", AnimationKind::Walk),
        (61, "EmoteAnimation", AnimationKind::Emote),
    ];
    for (id, name, want) in cases {
        assert_eq!(AnimationKind::from_asset(id, name), Some(want));
        assert_eq!(AnimationKind::from_asset(0, name), Some(want));
    }
    assert_eq!(AnimationKind::from_asset(11, "Shirt"), None);
}

#[test]
fn part_names_and_fallbacks_classify_the_rig() {
    assert_eq!(Part::from_name("UpperTorso"), Some(Part::UpperTorso));
    assert_eq!(Part::from_name("RightLowerArm"), Some(Part::RightLowerArm));
    assert_eq!(Part::from_name("LeftUpperLeg_01"), Some(Part::LeftUpperLeg));
    assert_eq!(Part::from_name("Left Arm"), Some(Part::LeftUpperArm));
    assert_eq!(Part::from_name("Hair"), None);

    let min = [-0.8, 0.0, -0.3];
    let max = [0.8, 2.0, 0.3];
    assert_eq!(Part::from_position([0.0, 1.95, 0.0], min, max), Part::Head);
    assert_eq!(
        Part::from_position([0.0, 1.4, 0.0], min, max),
        Part::UpperTorso
    );
    assert_eq!(
        Part::from_position([-0.7, 1.3, 0.0], min, max),
        Part::LeftUpperArm
    );
    assert_eq!(
        Part::from_position([0.7, 0.2, 0.0], min, max),
        Part::RightLowerLeg
    );
}

#[test]
fn stopped_playback_does_no_work() {
    let mut player = AnimPlayer::default();
    assert!(!player.is_playing());
    assert!(!player.needs_repaint());
    assert!(!player.advance(0.5), "a stopped player never advances");
    assert_eq!(player.pose(), Pose::identity());

    player.play(AnimationKind::Run);
    assert!(player.needs_repaint());
    assert!(player.advance(0.25));
    assert!(player.time() > 0.0);
    let posed = player.pose();
    assert_ne!(posed, Pose::identity());

    player.stop();
    assert!(!player.needs_repaint());
    assert_eq!(player.pose(), Pose::identity());
    assert_eq!(player.frame_index(), 0);
}

#[test]
fn the_toggle_pill_pauses_and_resumes_the_chosen_clip() {
    let mut player = AnimPlayer::default();
    player.toggle(); // nothing chosen yet: starts the idle loop
    assert!(player.is_playing());
    assert_eq!(player.kind(), Some(AnimationKind::Idle));

    player.play(AnimationKind::Walk);
    player.toggle();
    assert!(!player.is_playing());
    player.toggle();
    assert!(player.is_playing());
    assert_eq!(player.kind(), Some(AnimationKind::Walk));
}

// ---------------------------------------------------------------------------------------
// Mid-jump PNG: the same skeleton, pose and projection rules the viewport uses, drawn by a
// small z-buffered rasteriser so the limbs can be checked for connection and proportion.
// ---------------------------------------------------------------------------------------

type V3 = [f32; 3];
type Color = [u8; 3];

fn box_tris(min: V3, max: V3) -> Vec<[V3; 3]> {
    let c = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ];
    let faces = [
        [0, 1, 2, 3], // back
        [5, 4, 7, 6], // front
        [4, 0, 3, 7], // left
        [1, 5, 6, 2], // right
        [3, 2, 6, 7], // top
        [4, 5, 1, 0], // bottom
    ];
    let mut tris = Vec::new();
    for f in faces {
        tris.push([c[f[0]], c[f[1]], c[f[2]]]);
        tris.push([c[f[0]], c[f[2]], c[f[3]]]);
    }
    tris
}

fn avatar_mesh() -> (Vec<(Part, [V3; 3], Color)>, [Option<(V3, V3)>; PART_COUNT]) {
    let skin: Color = [226, 186, 150];
    let face: Color = [242, 206, 172];
    let hair: Color = [74, 50, 36];
    let shirt: Color = [58, 112, 196];
    let trousers: Color = [46, 56, 78];
    let shoes: Color = [28, 32, 42];

    let boxes: Vec<(Part, V3, V3, Color)> = vec![
        (Part::LowerTorso, [-0.40, 0.80, -0.23], [0.40, 1.30, 0.23], shirt),
        (Part::UpperTorso, [-0.50, 1.30, -0.26], [0.50, 1.80, 0.26], shirt),
        (Part::Head, [-0.30, 1.80, -0.30], [0.30, 2.16, 0.30], face),
        // A hair cap makes the head read apart from the similarly coloured arms.
        (Part::Head, [-0.31, 2.09, -0.31], [0.31, 2.22, 0.31], hair),
        (Part::LeftUpperArm, [-0.84, 1.26, -0.15], [-0.52, 1.76, 0.15], skin),
        (Part::LeftLowerArm, [-0.84, 0.82, -0.15], [-0.52, 1.26, 0.15], skin),
        (Part::LeftHand, [-0.82, 0.64, -0.14], [-0.54, 0.82, 0.14], skin),
        (Part::RightUpperArm, [0.52, 1.26, -0.15], [0.84, 1.76, 0.15], skin),
        (Part::RightLowerArm, [0.52, 0.82, -0.15], [0.84, 1.26, 0.15], skin),
        (Part::RightHand, [0.54, 0.64, -0.14], [0.82, 0.82, 0.14], skin),
        (Part::LeftUpperLeg, [-0.36, 0.42, -0.18], [-0.05, 0.80, 0.18], trousers),
        (Part::LeftLowerLeg, [-0.36, 0.10, -0.18], [-0.05, 0.42, 0.18], trousers),
        (Part::LeftFoot, [-0.36, 0.0, -0.18], [-0.05, 0.10, 0.28], shoes),
        (Part::RightUpperLeg, [0.05, 0.42, -0.18], [0.36, 0.80, 0.18], trousers),
        (Part::RightLowerLeg, [0.05, 0.10, -0.18], [0.36, 0.42, 0.18], trousers),
        (Part::RightFoot, [0.05, 0.0, -0.18], [0.36, 0.10, 0.28], shoes),
    ];

    let mut bounds: [Option<(V3, V3)>; PART_COUNT] = [None; PART_COUNT];
    let mut tris = Vec::new();
    for (part, min, max, color) in boxes {
        bounds[part.index()] = Some((min, max));
        for t in box_tris(min, max) {
            tris.push((part, t, color));
        }
    }
    (tris, bounds)
}

#[test]
fn renders_a_connected_avatar_mid_stride() {
    let (tris, bounds) = avatar_mesh();
    let center = [0.0, 1.11, 0.0];
    let extent_y = 2.22f32;
    let skel = Skeleton::from_part_bounds(&bounds, center, extent_y);
    // Mid-stride of the run: one leg forward, one back, arms pumping in opposition.
    let pose = creblox_vm_tests::avatar_anim::eval(AnimationKind::Run, 0.30);
    let transforms = skel.transforms(&pose);

    let w = 700usize;
    let h = 700usize;
    let mut px = vec![0u8; w * h * 4];
    // Studio backdrop, same palette as the viewport.
    for y in 0..h {
        let t = y as f32 / h as f32;
        let top = [30.0, 36.0, 48.0];
        let bot = [14.0, 17.0, 24.0];
        for x in 0..w {
            let i = (y * w + x) * 4;
            for c in 0..3 {
                px[i + c] = (top[c] + (bot[c] - top[c]) * t) as u8;
            }
            px[i + 3] = 255;
        }
    }
    let mut depth = vec![f32::INFINITY; w * h];

    let yaw = 0.35f32;
    let pitch = 0.05f32;
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let scale = 2.0 / extent_y;
    let dist = 4.8f32;
    // A touch wider than the live viewport so the jump's raised arms stay inside the frame.
    let aspect = h as f32 * 0.40;
    let cx = w as f32 * 0.5;
    let cyy = h as f32 * 0.5;
    let light = {
        let l = [0.35f32, 0.60, 0.72];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };

    for (part, tri, color) in &tris {
        let m = transforms[part.index()];
        let mut screen = [(0f32, 0f32, 0f32); 3];
        let mut normal = [0f32; 3];
        for (i, p) in tri.iter().enumerate() {
            let wp = m.transform_point(*p);
            let x = (wp[0] - center[0]) * scale;
            let y = (wp[1] - center[1]) * scale;
            let z = (wp[2] - center[2]) * scale;
            let rx = x * cy + z * sy;
            let rz = -x * sy + z * cy;
            let fy = y * cp - rz * sp;
            let fz = y * sp + rz * cp;
            let persp = dist / (dist + fz);
            screen[i] = (
                cx + rx * persp * aspect,
                cyy - fy * persp * aspect + h as f32 * 0.02,
                fz,
            );
        }
        // Face normal in view space, for flat shading.
        let e1 = [
            screen[1].0 - screen[0].0,
            screen[1].1 - screen[0].1,
            screen[1].2 - screen[0].2,
        ];
        let e2 = [
            screen[2].0 - screen[0].0,
            screen[2].1 - screen[0].1,
            screen[2].2 - screen[0].2,
        ];
        normal[0] = e1[1] * e2[2] - e1[2] * e2[1];
        normal[1] = e1[2] * e2[0] - e1[0] * e2[2];
        normal[2] = e1[0] * e2[1] - e1[1] * e2[0];
        let nl = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
            .sqrt()
            .max(1e-6);
        let lambert = ((normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2])
            / nl)
            .abs()
            .clamp(0.0, 1.0);
        let shade = 0.42 + 0.58 * lambert;
        let col = [
            (color[0] as f32 * shade) as u8,
            (color[1] as f32 * shade) as u8,
            (color[2] as f32 * shade) as u8,
        ];

        let minx = screen.iter().map(|s| s.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let maxx = (screen.iter().map(|s| s.0).fold(f32::MIN, f32::max).ceil() as isize)
            .clamp(0, w as isize - 1) as usize;
        let miny = screen.iter().map(|s| s.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let maxy = (screen.iter().map(|s| s.1).fold(f32::MIN, f32::max).ceil() as isize)
            .clamp(0, h as isize - 1) as usize;

        let area = (screen[1].0 - screen[0].0) * (screen[2].1 - screen[0].1)
            - (screen[1].1 - screen[0].1) * (screen[2].0 - screen[0].0);
        if area.abs() < 1e-6 {
            continue;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let pxf = x as f32 + 0.5;
                let pyf = y as f32 + 0.5;
                let w0 = ((screen[1].0 - pxf) * (screen[2].1 - pyf)
                    - (screen[1].1 - pyf) * (screen[2].0 - pxf))
                    / area;
                let w1 = ((screen[2].0 - pxf) * (screen[0].1 - pyf)
                    - (screen[2].1 - pyf) * (screen[0].0 - pxf))
                    / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * screen[0].2 + w1 * screen[1].2 + w2 * screen[2].2;
                let idx = y * w + x;
                if z < depth[idx] {
                    depth[idx] = z;
                    let i = idx * 4;
                    px[i] = col[0];
                    px[i + 1] = col[1];
                    px[i + 2] = col[2];
                }
            }
        }
    }

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("avatar_3d_anim.png");
    let img = image::RgbaImage::from_raw(w as u32, h as u32, px).unwrap();
    img.save(&path).unwrap();
    let len = std::fs::metadata(&path).unwrap().len();
    assert!(len > 10_000, "the mid-animation PNG should not be empty");
}
