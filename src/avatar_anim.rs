//! Avatar animation: R15/R6 limb skeleton, hierarchical forward kinematics and the keyframed
//! clips the Avatar Editor plays when an animation item is clicked.
//!
//! Deliberately free of egui and the network: `vm-tests/` includes this file by path and drives
//! the joint math, the clips and the player state. `avatar_3d` only consumes the transforms.

use std::f32::consts::{PI, TAU};

/// Parts of a standard R15 rig. R6 models (single-segment arms/legs) classify into the upper
/// segments, so the same hierarchy animates both rigs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    LowerTorso,
    UpperTorso,
    Head,
    LeftUpperArm,
    LeftLowerArm,
    LeftHand,
    RightUpperArm,
    RightLowerArm,
    RightHand,
    LeftUpperLeg,
    LeftLowerLeg,
    LeftFoot,
    RightUpperLeg,
    RightLowerLeg,
    RightFoot,
}

pub const PART_COUNT: usize = 15;

impl Part {
    /// Parent-before-child order: a transform pass can fill parents first.
    pub const ALL: [Part; PART_COUNT] = [
        Part::LowerTorso,
        Part::UpperTorso,
        Part::Head,
        Part::LeftUpperArm,
        Part::LeftLowerArm,
        Part::LeftHand,
        Part::RightUpperArm,
        Part::RightLowerArm,
        Part::RightHand,
        Part::LeftUpperLeg,
        Part::LeftLowerLeg,
        Part::LeftFoot,
        Part::RightUpperLeg,
        Part::RightLowerLeg,
        Part::RightFoot,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn parent(self) -> Option<Part> {
        match self {
            Part::LowerTorso => None,
            Part::UpperTorso | Part::LeftUpperArm | Part::RightUpperArm | Part::LeftUpperLeg
            | Part::RightUpperLeg => Some(Part::LowerTorso),
            Part::Head => Some(Part::UpperTorso),
            Part::LeftLowerArm => Some(Part::LeftUpperArm),
            Part::RightLowerArm => Some(Part::RightUpperArm),
            Part::LeftHand => Some(Part::LeftLowerArm),
            Part::RightHand => Some(Part::RightLowerArm),
            Part::LeftLowerLeg => Some(Part::LeftUpperLeg),
            Part::RightLowerLeg => Some(Part::RightUpperLeg),
            Part::LeftFoot => Some(Part::LeftLowerLeg),
            Part::RightFoot => Some(Part::RightLowerLeg),
        }
    }

    /// Material/object names from Roblox's avatar OBJ (R15, R6 and layered clothing names).
    pub fn from_name(name: &str) -> Option<Part> {
        let n: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if n.is_empty() {
            return None;
        }
        // Longest, most specific names first: "leftupperarm" before "leftarm".
        const NAMES: &[(&str, Part)] = &[
            ("leftupperarm", Part::LeftUpperArm),
            ("rightupperarm", Part::RightUpperArm),
            ("leftlowerarm", Part::LeftLowerArm),
            ("rightlowerarm", Part::RightLowerArm),
            ("leftforearm", Part::LeftLowerArm),
            ("rightforearm", Part::RightLowerArm),
            ("leftupperleg", Part::LeftUpperLeg),
            ("rightupperleg", Part::RightUpperLeg),
            ("leftlowerleg", Part::LeftLowerLeg),
            ("rightlowerleg", Part::RightLowerLeg),
            ("lefthand", Part::LeftHand),
            ("righthand", Part::RightHand),
            ("leftfoot", Part::LeftFoot),
            ("rightfoot", Part::RightFoot),
            ("leftarm", Part::LeftUpperArm),
            ("rightarm", Part::RightUpperArm),
            ("leftleg", Part::LeftUpperLeg),
            ("rightleg", Part::RightUpperLeg),
            ("lowertorso", Part::LowerTorso),
            ("uppertorso", Part::UpperTorso),
            ("torso", Part::LowerTorso),
            ("head", Part::Head),
        ];
        NAMES
            .iter()
            .find(|(needle, _)| n.contains(needle))
            .map(|(_, p)| *p)
    }

    /// Geometric fallback for meshes whose materials carry no body-part names: the centroid's
    /// height band and side decide the limb, using the model's overall bounds.
    pub fn from_position(p: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Part {
        let h = (max[1] - min[1]).max(1e-4);
        let frac = (p[1] - min[1]) / h;
        let cx = (min[0] + max[0]) * 0.5;
        let half = ((max[0] - min[0]) * 0.5).max(1e-4);
        let nx = (p[0] - cx) / half;
        let left = nx < 0.0;

        if frac > 0.86 {
            return Part::Head;
        }
        if nx.abs() > 0.58 && (0.28..0.84).contains(&frac) {
            let side_upper = if left {
                Part::LeftUpperArm
            } else {
                Part::RightUpperArm
            };
            let side_lower = if left {
                Part::LeftLowerArm
            } else {
                Part::RightLowerArm
            };
            let side_hand = if left { Part::LeftHand } else { Part::RightHand };
            return if frac > 0.56 {
                side_upper
            } else if frac > 0.40 {
                side_lower
            } else {
                side_hand
            };
        }
        if frac > 0.55 {
            return Part::UpperTorso;
        }
        if frac > 0.42 {
            return Part::LowerTorso;
        }
        let side_upper = if left {
            Part::LeftUpperLeg
        } else {
            Part::RightUpperLeg
        };
        let side_lower = if left {
            Part::LeftLowerLeg
        } else {
            Part::RightLowerLeg
        };
        let side_foot = if left { Part::LeftFoot } else { Part::RightFoot };
        if frac > 0.20 {
            side_upper
        } else if frac > 0.07 {
            side_lower
        } else {
            side_foot
        }
    }
}

/// A 3x4 affine transform stored column-major in a 4x4 array (the last row is always 0,0,0,1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f32; 16]);

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);

    pub fn translation(t: [f32; 3]) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.0[12] = t[0];
        m.0[13] = t[1];
        m.0[14] = t[2];
        m
    }

    /// Rotation around X (pitch: forward/back swing), then Y (yaw), then Z (roll: side raise).
    pub fn rotation(rx: f32, ry: f32, rz: f32) -> Mat4 {
        let (sx, cx) = rx.sin_cos();
        let (sy, cy) = ry.sin_cos();
        let (sz, cz) = rz.sin_cos();
        // Rz * Ry * Rx, column-major.
        Mat4([
            cz * cy,
            sz * cy,
            -sy,
            0.0,
            cz * sy * sx - sz * cx,
            sz * sy * sx + cz * cx,
            cy * sx,
            0.0,
            cz * sy * cx + sz * sx,
            sz * sy * cx - cz * sx,
            cy * cx,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    pub fn mul(a: Mat4, b: Mat4) -> Mat4 {
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += a.0[k * 4 + row] * b.0[col * 4 + k];
                }
                out[col * 4 + row] = sum;
            }
        }
        Mat4(out)
    }

    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        [
            self.0[0] * p[0] + self.0[4] * p[1] + self.0[8] * p[2] + self.0[12],
            self.0[1] * p[0] + self.0[5] * p[1] + self.0[9] * p[2] + self.0[13],
            self.0[2] * p[0] + self.0[6] * p[1] + self.0[10] * p[2] + self.0[14],
        ]
    }

    /// Normals ignore translation.
    pub fn transform_dir(&self, v: [f32; 3]) -> [f32; 3] {
        [
            self.0[0] * v[0] + self.0[4] * v[1] + self.0[8] * v[2],
            self.0[1] * v[0] + self.0[5] * v[1] + self.0[9] * v[2],
            self.0[2] * v[0] + self.0[6] * v[1] + self.0[10] * v[2],
        ]
    }
}

/// Per-joint local rotation in radians (pitch, yaw, roll).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub rot: [[f32; 3]; PART_COUNT],
}

impl Default for Pose {
    fn default() -> Self {
        Self::identity()
    }
}

impl Pose {
    pub fn identity() -> Pose {
        Pose {
            rot: [[0.0; 3]; PART_COUNT],
        }
    }

    pub fn set(&mut self, part: Part, rx: f32, ry: f32, rz: f32) {
        self.rot[part.index()] = [rx, ry, rz];
    }

    #[allow(dead_code)] // read by `vm-tests/tests/avatar_anim.rs`
    pub fn get(&self, part: Part) -> [f32; 3] {
        self.rot[part.index()]
    }
}

/// Joint pivots in the model's own coordinates, computed from each part's bounding box.
#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    pub pivots: [[f32; 3]; PART_COUNT],
}

impl Skeleton {
    pub fn new(pivots: [[f32; 3]; PART_COUNT]) -> Skeleton {
        Skeleton { pivots }
    }

    #[allow(dead_code)] // read by `vm-tests/tests/avatar_anim.rs`
    pub fn pivot(&self, part: Part) -> [f32; 3] {
        self.pivots[part.index()]
    }

    /// Joint pivots from each part's rest bounding box: the centre of the joint-facing face
    /// (hips, waist, neck, shoulders, elbows, wrists, hips, knees, ankles). Missing parts
    /// (R6 has no lower arms/legs) get a pivot just below their parent's.
    pub fn from_part_bounds(
        bounds: &[Option<([f32; 3], [f32; 3])>; PART_COUNT],
        center: [f32; 3],
        extent_y: f32,
    ) -> Skeleton {
        let mut pivots = [[center[0], center[1], center[2]]; PART_COUNT];
        for part in Part::ALL {
            let i = part.index();
            let Some((min, max)) = bounds[i] else {
                let parent = part.parent().map(|p| p.index()).unwrap_or(i);
                pivots[i] = [center[0], pivots[parent][1] - extent_y * 0.08, center[2]];
                continue;
            };
            let x = (min[0] + max[0]) * 0.5;
            let z = (min[2] + max[2]) * 0.5;
            let y = match part {
                // The body bends at the hips and at the waist.
                Part::LowerTorso | Part::UpperTorso => min[1],
                // Every other joint sits at the top of its own segment.
                _ => max[1],
            };
            pivots[i] = [x, y, z];
        }
        Skeleton::new(pivots)
    }

    /// Hierarchical forward kinematics: each joint rotates about its own pivot, already
    /// carried along by its parent's transform, so a child never separates from its socket.
    pub fn transforms(&self, pose: &Pose) -> [Mat4; PART_COUNT] {
        let mut out = [Mat4::IDENTITY; PART_COUNT];
        for part in Part::ALL {
            let p = self.pivots[part.index()];
            let [rx, ry, rz] = pose.rot[part.index()];
            let local = Mat4::mul(
                Mat4::mul(Mat4::translation(p), Mat4::rotation(rx, ry, rz)),
                Mat4::translation([-p[0], -p[1], -p[2]]),
            );
            out[part.index()] = match part.parent() {
                None => local,
                Some(parent) => Mat4::mul(out[parent.index()], local),
            };
        }
        out
    }
}

impl Default for Skeleton {
    fn default() -> Self {
        Self::new([[0.0; 3]; PART_COUNT])
    }
}

/// The animation clips the editor can preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationKind {
    Idle,
    Walk,
    Run,
    Jump,
    Fall,
    Swim,
    Climb,
    Emote,
}

impl AnimationKind {
    /// Roblox asset type ids: 48 Climb, 50 Fall, 51 Idle, 52 Jump, 53 Run, 54 Swim, 55 Walk,
    /// 61 Emote. Names are the fallback for items that only carry a display type.
    pub fn from_asset(asset_type_id: u64, type_name: &str) -> Option<AnimationKind> {
        match asset_type_id {
            48 => return Some(AnimationKind::Climb),
            50 => return Some(AnimationKind::Fall),
            51 => return Some(AnimationKind::Idle),
            52 => return Some(AnimationKind::Jump),
            53 => return Some(AnimationKind::Run),
            54 => return Some(AnimationKind::Swim),
            55 => return Some(AnimationKind::Walk),
            61 => return Some(AnimationKind::Emote),
            _ => {}
        }
        let n = type_name.to_ascii_lowercase();
        let n = n.trim_end_matches("animation");
        let kind = if n.contains("climb") {
            AnimationKind::Climb
        } else if n.contains("fall") {
            AnimationKind::Fall
        } else if n.contains("idle") {
            AnimationKind::Idle
        } else if n.contains("jump") {
            AnimationKind::Jump
        } else if n.contains("run") || n.contains("sprint") {
            AnimationKind::Run
        } else if n.contains("swim") {
            AnimationKind::Swim
        } else if n.contains("walk") {
            AnimationKind::Walk
        } else if n.contains("emote") || n.contains("dance") || n.contains("wave") {
            AnimationKind::Emote
        } else {
            return None;
        };
        Some(kind)
    }

    pub fn label(self) -> &'static str {
        match self {
            AnimationKind::Idle => "Idle",
            AnimationKind::Walk => "Walk",
            AnimationKind::Run => "Run",
            AnimationKind::Jump => "Jump",
            AnimationKind::Fall => "Fall",
            AnimationKind::Swim => "Swim",
            AnimationKind::Climb => "Climb",
            AnimationKind::Emote => "Emote",
        }
    }

    /// Loop length in seconds.
    pub fn duration(self) -> f32 {
        match self {
            AnimationKind::Idle => 4.0,
            AnimationKind::Walk => 1.0,
            AnimationKind::Run => 0.7,
            AnimationKind::Jump => 1.15,
            AnimationKind::Fall => 1.4,
            AnimationKind::Swim => 1.6,
            AnimationKind::Climb => 1.5,
            AnimationKind::Emote => 2.4,
        }
    }

    /// Samples the clip's curve into keyframes; interpolation between them is smoothstep.
    pub fn keys(self) -> Vec<Keyframe> {
        let steps = match self {
            AnimationKind::Jump | AnimationKind::Emote => 12,
            _ => 16,
        };
        (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                Keyframe {
                    t,
                    pose: sample(self, t),
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct Keyframe {
    /// Position in the loop, 0..=1.
    pub t: f32,
    pub pose: Pose,
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn lerp_pose(a: &Pose, b: &Pose, f: f32) -> Pose {
    let mut out = Pose::identity();
    for part in Part::ALL {
        let i = part.index();
        for c in 0..3 {
            out.rot[i][c] = a.rot[i][c] + (b.rot[i][c] - a.rot[i][c]) * f;
        }
    }
    out
}

/// Evaluates a clip at `phase` (0..=1, wrapped) with smoothstep between adjacent keys.
pub fn eval_keys(keys: &[Keyframe], phase: f32) -> Pose {
    if keys.is_empty() {
        return Pose::identity();
    }
    let phase = phase.rem_euclid(1.0);
    let mut prev = &keys[0];
    for key in keys.iter().skip(1) {
        if phase <= key.t {
            let span = (key.t - prev.t).max(1e-6);
            let f = smoothstep((phase - prev.t) / span);
            return lerp_pose(&prev.pose, &key.pose, f);
        }
        prev = key;
    }
    // Past the last key: carry the loop's final pose back toward the first.
    let last = keys.last().unwrap();
    let first = &keys[0];
    let span = (1.0 - last.t).max(1e-6);
    let f = smoothstep((phase - last.t) / span);
    lerp_pose(&last.pose, &first.pose, f)
}

pub fn eval(kind: AnimationKind, phase: f32) -> Pose {
    eval_keys(&kind.keys(), phase)
}

/// The analytic curves the keyframes are sampled from.
fn sample(kind: AnimationKind, t: f32) -> Pose {
    let mut p = Pose::identity();
    let a = t * TAU;
    match kind {
        AnimationKind::Idle => {
            // Breathing chest, tiny head sway, relaxed arms drifting in and out.
            p.set(Part::UpperTorso, 0.03 * (a).sin(), 0.02 * (a * 0.5).sin(), 0.0);
            p.set(Part::Head, 0.02 * (a + 0.7).sin(), 0.07 * (a * 0.5).sin(), 0.03 * (a * 0.5).cos());
            p.set(Part::LeftUpperArm, 0.04 * (a + 0.4).sin(), 0.0, -0.05 - 0.03 * a.sin());
            p.set(Part::RightUpperArm, 0.04 * (a + 0.4).sin(), 0.0, 0.05 + 0.03 * a.sin());
            p.set(Part::LeftLowerArm, 0.06, 0.0, 0.0);
            p.set(Part::RightLowerArm, 0.06, 0.0, 0.0);
        }
        AnimationKind::Walk => {
            let stride = 0.55 * a.sin();
            p.set(Part::LeftUpperLeg, stride, 0.0, 0.0);
            p.set(Part::RightUpperLeg, -stride, 0.0, 0.0);
            p.set(Part::LeftLowerLeg, 0.65 * a.sin().max(0.0), 0.0, 0.0);
            p.set(Part::RightLowerLeg, 0.65 * (-a).sin().max(0.0), 0.0, 0.0);
            p.set(Part::LeftUpperArm, -0.45 * a.sin(), 0.0, -0.06);
            p.set(Part::RightUpperArm, 0.45 * a.sin(), 0.0, 0.06);
            p.set(Part::LeftLowerArm, 0.25 + 0.1 * a.sin(), 0.0, 0.0);
            p.set(Part::RightLowerArm, 0.25 - 0.1 * a.sin(), 0.0, 0.0);
            p.set(Part::LowerTorso, 0.0, 0.07 * a.sin(), 0.0);
            p.set(Part::UpperTorso, 0.03, -0.05 * a.sin(), 0.02 * a.cos());
            p.set(Part::Head, 0.0, 0.04 * a.sin(), 0.0);
        }
        AnimationKind::Run => {
            let stride = 0.95 * a.sin();
            p.set(Part::LeftUpperLeg, stride, 0.0, 0.0);
            p.set(Part::RightUpperLeg, -stride, 0.0, 0.0);
            p.set(Part::LeftLowerLeg, 1.1 * a.sin().max(0.0), 0.0, 0.0);
            p.set(Part::RightLowerLeg, 1.1 * (-a).sin().max(0.0), 0.0, 0.0);
            // Forward lean of the whole body, arms bent and pumping.
            p.set(Part::LowerTorso, -0.22, 0.10 * a.sin(), 0.0);
            p.set(Part::UpperTorso, -0.08, -0.08 * a.sin(), 0.0);
            p.set(Part::Head, 0.12, 0.05 * a.sin(), 0.0);
            p.set(Part::LeftUpperArm, -0.85 * a.sin(), 0.0, -0.12);
            p.set(Part::RightUpperArm, 0.85 * a.sin(), 0.0, 0.12);
            p.set(Part::LeftLowerArm, 1.25 + 0.2 * a.sin(), 0.0, 0.0);
            p.set(Part::RightLowerArm, 1.25 - 0.2 * a.sin(), 0.0, 0.0);
        }
        AnimationKind::Jump => {
            // Crouch -> launch -> airborne tuck -> reach for the ground -> land -> recover.
            let crouch = if t < 0.12 {
                t / 0.12
            } else if t < 0.28 {
                1.0 - (t - 0.12) / 0.16
            } else {
                0.0
            };
            let air = if (0.22..0.72).contains(&t) { 1.0 } else { 0.0 };
            // A short bump that returns to the standing pose by the end, so the loop is seamless.
            let land = if t > 0.82 {
                (PI * (t - 0.82) / 0.18).sin()
            } else {
                0.0
            };
            let bend = 0.9 * crouch + 0.55 * air + 0.7 * land;
            p.set(Part::LowerTorso, -0.18 * crouch - 0.08 * air + 0.22 * land, 0.0, 0.0);
            p.set(Part::LeftUpperLeg, -0.75 * bend, 0.0, 0.05);
            p.set(Part::RightUpperLeg, -0.75 * bend, 0.0, -0.05);
            p.set(Part::LeftLowerLeg, 1.25 * bend, 0.0, 0.0);
            p.set(Part::RightLowerLeg, 1.25 * bend, 0.0, 0.0);
            p.set(Part::LeftFoot, -0.35 * bend, 0.0, 0.0);
            p.set(Part::RightFoot, -0.35 * bend, 0.0, 0.0);
            p.set(
                Part::LeftUpperArm,
                -1.9 * crouch - 2.6 * air,
                0.0,
                -0.15 - 0.45 * air,
            );
            p.set(
                Part::RightUpperArm,
                -1.9 * crouch - 2.6 * air,
                0.0,
                0.15 + 0.45 * air,
            );
            p.set(Part::LeftLowerArm, 0.5 * (crouch + air), 0.0, 0.0);
            p.set(Part::RightLowerArm, 0.5 * (crouch + air), 0.0, 0.0);
            p.set(Part::Head, 0.1 * crouch - 0.15 * air, 0.0, 0.0);
        }
        AnimationKind::Fall => {
            // Mid-air balance: arms up and out, legs apart, slight flail.
            let flail = (a * 2.0).sin();
            p.set(Part::LowerTorso, 0.12, 0.10 * (a).sin(), 0.0);
            p.set(Part::UpperTorso, 0.05, -0.06 * (a).sin(), 0.04 * flail);
            p.set(Part::LeftUpperArm, -2.3 + 0.25 * flail, 0.0, -0.85);
            p.set(Part::RightUpperArm, -2.3 - 0.25 * flail, 0.0, 0.85);
            p.set(Part::LeftLowerArm, 0.5 + 0.35 * flail, 0.0, 0.0);
            p.set(Part::RightLowerArm, 0.5 - 0.35 * flail, 0.0, 0.0);
            p.set(Part::LeftUpperLeg, 0.25, 0.0, 0.22);
            p.set(Part::RightUpperLeg, -0.35, 0.0, -0.22);
            p.set(Part::LeftLowerLeg, 0.55, 0.0, 0.0);
            p.set(Part::RightLowerLeg, 0.35, 0.0, 0.0);
            p.set(Part::Head, -0.18, 0.10 * (a).sin(), 0.0);
        }
        AnimationKind::Swim => {
            // Breaststroke: arms sweep out and together, legs kick in alternation.
            let stroke = (a - PI * 0.5).sin();
            let kick = (a * 2.0).sin();
            p.set(Part::LowerTorso, -0.55, 0.0, 0.0);
            p.set(Part::UpperTorso, -0.20, 0.05 * (a).sin(), 0.0);
            p.set(Part::Head, 0.35, 0.0, 0.0);
            p.set(Part::LeftUpperArm, -1.6 + 0.5 * stroke, 0.0, -0.9 * stroke - 0.2);
            p.set(Part::RightUpperArm, -1.6 + 0.5 * stroke, 0.0, 0.9 * stroke + 0.2);
            p.set(Part::LeftLowerArm, 0.4 + 0.3 * stroke, 0.0, 0.0);
            p.set(Part::RightLowerArm, 0.4 + 0.3 * stroke, 0.0, 0.0);
            p.set(Part::LeftUpperLeg, 0.2 + 0.35 * kick, 0.0, 0.12);
            p.set(Part::RightUpperLeg, 0.2 - 0.35 * kick, 0.0, -0.12);
            p.set(Part::LeftLowerLeg, 0.5 - 0.3 * kick, 0.0, 0.0);
            p.set(Part::RightLowerLeg, 0.5 + 0.3 * kick, 0.0, 0.0);
        }
        AnimationKind::Climb => {
            // Alternating overhead reach, body close to the wall, legs stepping up.
            let reach = a.sin();
            p.set(Part::LowerTorso, -0.10, 0.0, 0.0);
            p.set(Part::UpperTorso, 0.08, 0.06 * reach, 0.0);
            p.set(Part::Head, 0.22, 0.0, 0.0);
            p.set(Part::LeftUpperArm, -2.7 - 0.4 * reach, 0.0, -0.25);
            p.set(Part::RightUpperArm, -2.7 + 0.4 * reach, 0.0, 0.25);
            p.set(Part::LeftLowerArm, 0.45 + 0.35 * reach, 0.0, 0.0);
            p.set(Part::RightLowerArm, 0.45 - 0.35 * reach, 0.0, 0.0);
            p.set(Part::LeftUpperLeg, -0.55 * reach, 0.0, 0.08);
            p.set(Part::RightUpperLeg, 0.55 * reach, 0.0, -0.08);
            p.set(Part::LeftLowerLeg, 0.65 + 0.35 * reach, 0.0, 0.0);
            p.set(Part::RightLowerLeg, 0.65 - 0.35 * reach, 0.0, 0.0);
        }
        AnimationKind::Emote => {
            // A wave with the right arm plus a little dance sway.
            let wave = (a * 2.0).sin();
            let sway = a.sin();
            p.set(Part::LowerTorso, 0.05 * sway, 0.10 * sway, 0.0);
            p.set(Part::UpperTorso, 0.03, -0.08 * sway, 0.05 * sway);
            p.set(Part::Head, 0.05, 0.12 * sway, 0.08 * sway);
            p.set(Part::RightUpperArm, -2.4, 0.0, 0.55 + 0.35 * wave);
            p.set(Part::RightLowerArm, 0.3, 0.0, 0.9 * wave);
            p.set(Part::RightHand, 0.0, 0.0, 0.4 * wave);
            p.set(Part::LeftUpperArm, 0.1 * sway, 0.0, -0.12);
            p.set(Part::LeftLowerArm, 0.25, 0.0, 0.0);
            p.set(Part::LeftUpperLeg, 0.06 * sway, 0.0, 0.0);
            p.set(Part::RightUpperLeg, -0.06 * sway, 0.0, 0.0);
        }
    }
    p
}

/// Playback state for the viewport: which clip, whether it is running, and where in the loop.
///
/// When `playing` is false nothing advances and the renderer is handed the rest pose, so a
/// stopped preview costs exactly nothing per frame.
#[derive(Clone, Debug, Default)]
pub struct AnimPlayer {
    pub kind: Option<AnimationKind>,
    playing: bool,
    time: f32,
}

impl AnimPlayer {
    pub fn play(&mut self, kind: AnimationKind) {
        self.kind = Some(kind);
        self.playing = true;
        self.time = 0.0;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.time = 0.0;
    }

    /// Pause/resume from the viewport pill. With no clip chosen yet it starts the idle loop.
    pub fn toggle(&mut self) {
        if self.playing {
            self.stop();
        } else {
            let kind = self.kind.unwrap_or(AnimationKind::Idle);
            self.kind = Some(kind);
            self.playing = true;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing && self.kind.is_some()
    }

    pub fn kind(&self) -> Option<AnimationKind> {
        self.kind
    }

    #[allow(dead_code)] // read by `vm-tests/tests/avatar_anim.rs`
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Advances the loop. Returns `false` when stopped (no work, and the caller must not
    /// schedule another frame).
    pub fn advance(&mut self, dt: f32) -> bool {
        if !self.is_playing() {
            return false;
        }
        let duration = self.kind.unwrap_or(AnimationKind::Idle).duration();
        self.time = (self.time + dt.clamp(0.0, 0.1)) % duration;
        true
    }

    pub fn phase(&self) -> f32 {
        if !self.is_playing() {
            return 0.0;
        }
        let duration = self.kind.unwrap_or(AnimationKind::Idle).duration();
        (self.time / duration).rem_euclid(1.0)
    }

    /// The pose handed to the renderer: rest pose when stopped.
    pub fn pose(&self) -> Pose {
        match self.kind {
            Some(kind) if self.playing => eval(kind, self.phase()),
            _ => Pose::identity(),
        }
    }

    /// Quantised animation frame for the raster cache key: 30 steps per second of clip time.
    pub fn frame_index(&self) -> u32 {
        if !self.is_playing() {
            return 0;
        }
        (self.time * 30.0).floor() as u32
    }

    /// The viewport schedules frames only while this is true.
    pub fn needs_repaint(&self) -> bool {
        self.is_playing()
    }
}
