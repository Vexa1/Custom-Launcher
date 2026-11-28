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

    /// `LowerTorso` and friends: the names used in the log and the dump file.
    pub fn label(self) -> &'static str {
        match self {
            Part::LowerTorso => "LowerTorso",
            Part::UpperTorso => "UpperTorso",
            Part::Head => "Head",
            Part::LeftUpperArm => "LeftUpperArm",
            Part::LeftLowerArm => "LeftLowerArm",
            Part::LeftHand => "LeftHand",
            Part::RightUpperArm => "RightUpperArm",
            Part::RightLowerArm => "RightLowerArm",
            Part::RightHand => "RightHand",
            Part::LeftUpperLeg => "LeftUpperLeg",
            Part::LeftLowerLeg => "LeftLowerLeg",
            Part::LeftFoot => "LeftFoot",
            Part::RightUpperLeg => "RightUpperLeg",
            Part::RightLowerLeg => "RightLowerLeg",
            Part::RightFoot => "RightFoot",
        }
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
    ///
    /// Roblox's own OBJ writer does not actually emit body-part names: its materials are
    /// `Part12Mtl`-style counters and its objects are `Object_7`, so on the models the launcher
    /// downloads this returns `None` and [`classify_triangles`] places the mesh instead. The
    /// table still covers the exports that *do* name parts (Studio "Export Selection", R6's
    /// `Left Arm`/`Right Leg`, layered-clothing part names).
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

    /// Places a whole mesh island from its own bounds: the head and anything riding on it
    /// (hat, hair, face accessory) is called by where it sits, everything else by its centroid.
    pub fn from_bounds(
        min: [f32; 3],
        max: [f32; 3],
        model_min: [f32; 3],
        model_max: [f32; 3],
        bands: RigBands,
    ) -> Part {
        let h = (model_max[1] - model_min[1]).max(1e-4);
        let top = (max[1] - model_min[1]) / h;
        let cx = (model_min[0] + model_max[0]) * 0.5;
        let half = ((model_max[0] - model_min[0]) * 0.5).max(1e-4);
        let centre_x = ((min[0] + max[0]) * 0.5 - cx) / half;
        // A hat hangs *above* the head and a hair cap covers its top, so both are decided by
        // how high the island reaches rather than by where its centre is.
        if top > bands.head + 0.05 && centre_x.abs() < 0.55 {
            return Part::Head;
        }
        Part::from_position_with(
            [
                (min[0] + max[0]) * 0.5,
                (min[1] + max[1]) * 0.5,
                (min[2] + max[2]) * 0.5,
            ],
            model_min,
            model_max,
            bands,
        )
    }

    /// Geometric fallback for meshes whose materials carry no body-part names: the centroid's
    /// height band and side decide the limb, using the model's overall bounds.
    #[allow(dead_code)] // the default-band form is read by `vm-tests/tests/avatar_anim.rs`
    pub fn from_position(p: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Part {
        Self::from_position_with(p, min, max, RigBands::DEFAULT)
    }

    /// [`Part::from_position`] against explicit joint heights.
    pub fn from_position_with(
        p: [f32; 3],
        min: [f32; 3],
        max: [f32; 3],
        bands: RigBands,
    ) -> Part {
        let h = (max[1] - min[1]).max(1e-4);
        let frac = (p[1] - min[1]) / h;
        let cx = (min[0] + max[0]) * 0.5;
        let half = ((max[0] - min[0]) * 0.5).max(1e-4);
        let nx = (p[0] - cx) / half;
        let left = nx < 0.0;

        if frac > bands.head {
            return Part::Head;
        }
        // Arms hang outside the torso's silhouette, between the hips and the shoulders; their
        // height inside that span decides the segment, not the height inside the whole body.
        // Below the hips that far out is a foot, not a hand.
        if nx.abs() > 0.58 && frac > bands.hip * 0.9 && frac < bands.shoulder {
            let upper = if left {
                Part::LeftUpperArm
            } else {
                Part::RightUpperArm
            };
            let lower = if left {
                Part::LeftLowerArm
            } else {
                Part::RightLowerArm
            };
            let hand = if left { Part::LeftHand } else { Part::RightHand };
            return if frac > bands.elbow {
                upper
            } else if frac > bands.wrist {
                lower
            } else {
                hand
            };
        }
        if frac > bands.waist {
            return Part::UpperTorso;
        }
        if frac > bands.hip {
            return Part::LowerTorso;
        }
        let upper = if left {
            Part::LeftUpperLeg
        } else {
            Part::RightUpperLeg
        };
        let lower = if left {
            Part::LeftLowerLeg
        } else {
            Part::RightLowerLeg
        };
        let foot = if left { Part::LeftFoot } else { Part::RightFoot };
        if frac > bands.knee {
            upper
        } else if frac > bands.ankle {
            lower
        } else {
            foot
        }
    }
}

/// Where the joints sit vertically, as fractions of the model's own height.
///
/// Defaults are measured off a standard R15 avatar (hips at 0.34 of the character, waist at
/// 0.50, jaw at 0.80), which is why the plain constants are not simply evenly spaced. A model
/// wearing a tall hat is *taller* than the character, so [`RigBands::from_islands`] calibrates
/// them against the head the model actually has; without that, a hat pushed the whole torso
/// down into the legs' band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigBands {
    /// Jaw line: everything above is the head or an accessory riding on it.
    pub head: f32,
    pub shoulder: f32,
    pub elbow: f32,
    pub wrist: f32,
    pub waist: f32,
    pub hip: f32,
    pub knee: f32,
    pub ankle: f32,
}

impl RigBands {
    pub const DEFAULT: RigBands = RigBands {
        head: 0.80,
        shoulder: 0.76,
        elbow: 0.55,
        wrist: 0.42,
        waist: 0.50,
        hip: 0.34,
        knee: 0.19,
        ankle: 0.05,
    };

    /// Calibrates the bands against the head the model actually has, given every island's
    /// bounds. The jaw is the lowest point of whatever is up there (the head, plus the hat and
    /// hair above it), so a tall accessory no longer squashes every band below it.
    pub fn from_islands(
        bounds: impl Iterator<Item = ([f32; 3], [f32; 3])>,
        model_min: [f32; 3],
        model_max: [f32; 3],
    ) -> RigBands {
        let h = (model_max[1] - model_min[1]).max(1e-4);
        let cx = (model_min[0] + model_max[0]) * 0.5;
        let half = ((model_max[0] - model_min[0]) * 0.5).max(1e-4);
        let mut jaw = f32::MAX;
        for (min, max) in bounds {
            let top = (max[1] - model_min[1]) / h;
            let centre_x = ((min[0] + max[0]) * 0.5 - cx) / half;
            if top >= 0.86 && centre_x.abs() < 0.55 {
                jaw = jaw.min((min[1] - model_min[1]) / h);
            }
        }
        let head = if jaw.is_finite() && (0.45..0.92).contains(&jaw) {
            jaw
        } else {
            RigBands::DEFAULT.head
        };
        RigBands::from_head(head)
    }

    /// The band layout for a given jaw height. R15 proportions: shoulders just under the jaw,
    /// hips at 0.44 of the jaw height, knees at 0.55 of the hips, arms down to just below the
    /// hips.
    pub fn from_head(head: f32) -> RigBands {
        let head = head.clamp(0.45, 0.92);
        let shoulder = head - 0.04;
        let hip = 0.44 * head;
        let arm_bottom = hip + 0.07;
        let arm = (shoulder - arm_bottom).max(0.05);
        RigBands {
            head,
            shoulder,
            elbow: shoulder - 0.49 * arm,
            wrist: shoulder - 0.85 * arm,
            waist: hip + 0.45 * (shoulder - hip),
            hip,
            knee: 0.55 * hip,
            ankle: 0.16 * hip,
        }
    }
}

/// How many triangles each rig part ended up with, plus the leftovers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartCounts {
    pub per_part: [usize; PART_COUNT],
    pub unclassified: usize,
}

impl PartCounts {
    pub fn from_parts(parts: &[Part]) -> PartCounts {
        let mut counts = PartCounts::default();
        for p in parts {
            counts.per_part[p.index()] += 1;
        }
        counts
    }

    #[allow(dead_code)] // read by the avatar tests
    pub fn total(&self) -> usize {
        self.per_part.iter().sum::<usize>() + self.unclassified
    }

    /// One line for the log: `LowerTorso 96 UpperTorso 144 ... unclassified 0`.
    pub fn report(&self) -> String {
        let mut out = String::new();
        for part in Part::ALL {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!("{} {}", part.label(), self.per_part[part.index()]));
        }
        out.push_str(&format!(" unclassified {}", self.unclassified));
        out
    }

    /// Parts that got no geometry at all: an R6 rig never has lower arms, so this is only a
    /// debugging signal, not an error.
    pub fn empty(&self) -> Vec<Part> {
        Part::ALL
            .iter()
            .copied()
            .filter(|p| self.per_part[p.index()] == 0)
            .collect()
    }
}

/// Assigns every triangle a rig part, working from welded mesh islands rather than one triangle
/// at a time.
///
/// Roblox's avatar OBJ does not name its body parts: the MTL has `newmtl Part12Mtl` entries and
/// the OBJ has `o Object_7`, so material names identify nothing. The geometry still does: each
/// body part and each accessory is its own island, so an island's own bounds say where it goes
/// on the rig. Deciding per island also means a triangle on a seam can no longer be pulled onto
/// a neighbouring limb mid-animation, which is what tore clothing apart.
///
/// `tri_verts` holds the OBJ vertex index of each corner (`u32::MAX` when the file did not give
/// one), `tri_pos` the corners themselves, and `named` the part a material/object name asked for.
/// A part named in the file wins for the whole island; a tall central island (the torso, a
/// welded full body, a shirt wrapping the waist) is split by height so the waist can bend.
pub fn classify_triangles(
    tri_verts: &[[u32; 3]],
    tri_pos: &[[[f32; 3]; 3]],
    named: &[Option<Part>],
    model_min: [f32; 3],
    model_max: [f32; 3],
) -> Vec<Part> {
    let count = tri_pos.len();
    let mut out = vec![Part::LowerTorso; count];
    if count == 0 {
        return out;
    }

    // Union-find over shared vertices: two triangles in the same island share a corner.
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut owner: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (t, verts) in tri_verts.iter().enumerate() {
        for &v in verts {
            if v == u32::MAX {
                continue;
            }
            match owner.get(&v) {
                Some(&first) => {
                    let a = root(&mut parent, first);
                    let b = root(&mut parent, t);
                    if a != b {
                        parent[a] = b;
                    }
                }
                None => {
                    owner.insert(v, t);
                }
            }
        }
    }

    let mut islands: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for t in 0..count {
        let r = root(&mut parent, t);
        islands.entry(r).or_default().push(t);
    }

    let height = (model_max[1] - model_min[1]).max(1e-4);
    let centre_x = (model_min[0] + model_max[0]) * 0.5;
    let half_width = ((model_max[0] - model_min[0]) * 0.5).max(1e-4);

    // Island bounds come first: the joint heights are calibrated from the head this model has.
    let mut shape: Vec<(Vec<usize>, [f32; 3], [f32; 3])> = Vec::with_capacity(islands.len());
    for tris in islands.into_values() {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for &t in &tris {
            for p in &tri_pos[t] {
                for a in 0..3 {
                    min[a] = min[a].min(p[a]);
                    max[a] = max[a].max(p[a]);
                }
            }
        }
        shape.push((tris, min, max));
    }
    let bands = RigBands::from_islands(
        shape.iter().map(|(_, min, max)| (*min, *max)),
        model_min,
        model_max,
    );

    for (tris, min, max) in &shape {
        let (min, max) = (*min, *max);
        // A body-part name in the file is authoritative for the whole island.
        let mut votes = [0usize; PART_COUNT];
        for &t in tris {
            if let Some(part) = named.get(t).copied().flatten() {
                votes[part.index()] += 1;
            }
        }
        if let Some(best) = (0..PART_COUNT).max_by_key(|&i| votes[i]) {
            if votes[best] * 2 > tris.len() {
                let part = Part::ALL[best];
                for &t in tris {
                    out[t] = part;
                }
                continue;
            }
        }

        // The torso (or a welded whole body, or a shirt over the waist) covers several rig
        // parts: split it by height so the hips and the waist can bend independently.
        let tall = (max[1] - min[1]) > height * 0.30;
        let central = (((min[0] + max[0]) * 0.5 - centre_x) / half_width).abs() < 0.40;
        if tall && central {
            for &t in tris {
                let mut tmin = [f32::MAX; 3];
                let mut tmax = [f32::MIN; 3];
                for p in &tri_pos[t] {
                    for a in 0..3 {
                        tmin[a] = tmin[a].min(p[a]);
                        tmax[a] = tmax[a].max(p[a]);
                    }
                }
                // `from_bounds` puts the triangle's height band on the model's scale, which
                // is the split this torso blob needs.
                out[t] = Part::from_bounds(tmin, tmax, model_min, model_max, bands);
            }
        } else {
            let part = Part::from_bounds(min, max, model_min, model_max, bands);
            for &t in tris {
                out[t] = part;
            }
        }
    }

    out
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
///
/// A posed frame is a full software re-rasterisation of the whole avatar, so the player also
/// keeps its own frame pacing: [`AnimPlayer::note_frame_cost`] feeds it the measured cost of the
/// last posed raster and the viewport asks for the next frame only after [`frame_interval`],
/// which keeps the preview inside half of a frame's wall time.
///
/// [`frame_interval`]: AnimPlayer::frame_interval
#[derive(Clone, Debug)]
pub struct AnimPlayer {
    pub kind: Option<AnimationKind>,
    playing: bool,
    time: f32,
    /// Seconds between posed frames. Starts at the 30 fps cap and stretches when a raster is
    /// expensive, so a heavy avatar degrades smoothly instead of saturating the CPU.
    interval: f32,
    /// Smoothed cost of one posed raster, in milliseconds.
    cost_ms: f32,
    /// Whether this play session has already logged its measured frame cost.
    reported: bool,
}

/// The preview redraws at most this often.
const MIN_FRAME_INTERVAL: f32 = 1.0 / 30.0;
/// ...and never less often than this, even if one posed raster is slow.
const MAX_FRAME_INTERVAL: f32 = 0.2;

impl Default for AnimPlayer {
    fn default() -> Self {
        AnimPlayer {
            kind: None,
            playing: false,
            time: 0.0,
            interval: MIN_FRAME_INTERVAL,
            cost_ms: 0.0,
            reported: false,
        }
    }
}

impl AnimPlayer {
    pub fn play(&mut self, kind: AnimationKind) {
        self.kind = Some(kind);
        self.playing = true;
        self.time = 0.0;
        self.interval = MIN_FRAME_INTERVAL;
        self.cost_ms = 0.0;
        self.reported = false;
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

    /// Records how long one posed raster took and paces the preview to stay inside half of
    /// the frame time it spends drawing faces.
    ///
    /// Cheap avatars keep the 30 fps cap; an expensive one is drawn less often instead of
    /// trying to keep up and maxing a core. Nothing is scheduled while stopped, so this never
    /// runs when the preview is idle.
    pub fn note_frame_cost(&mut self, ms: f32) {
        if !ms.is_finite() || ms <= 0.0 {
            return;
        }
        // React to a slow frame at once (so the next one is already spaced out) and recover
        // gently once the cost drops again.
        self.cost_ms = if self.cost_ms <= 0.0 || ms > self.cost_ms {
            ms
        } else {
            self.cost_ms * 0.8 + ms * 0.2
        };
        self.interval = (2.0 * self.cost_ms / 1000.0).clamp(MIN_FRAME_INTERVAL, MAX_FRAME_INTERVAL);
    }

    /// Seconds until the next posed frame may be drawn (the viewport uses
    /// `request_repaint_after`).
    pub fn frame_interval(&self) -> f32 {
        if self.is_playing() {
            self.interval
        } else {
            MIN_FRAME_INTERVAL
        }
    }

    /// The rate the preview is currently being redrawn at, for the log.
    pub fn effective_fps(&self) -> f32 {
        1.0 / self.frame_interval()
    }

    /// Smoothed cost of one posed raster in milliseconds (0 until the first frame is timed).
    pub fn frame_cost_ms(&self) -> f32 {
        self.cost_ms
    }

    /// True once per play session, so the measured cost is logged exactly once.
    pub fn take_cost_report(&mut self) -> bool {
        if self.is_playing() && !self.reported && self.cost_ms > 0.0 {
            self.reported = true;
            true
        } else {
            false
        }
    }
}
/// A compact island id per triangle: two triangles that share an OBJ vertex are in the same
/// island, so accessories and body parts separate even though the file names nothing.
pub fn island_ids(tri_verts: &[[u32; 3]]) -> Vec<u32> {
    let count = tri_verts.len();
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut owner: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (t, verts) in tri_verts.iter().enumerate() {
        for &v in verts {
            if v == u32::MAX {
                continue;
            }
            match owner.get(&v) {
                Some(&first) => {
                    let a = root(&mut parent, first);
                    let b = root(&mut parent, t);
                    if a != b {
                        parent[a] = b;
                    }
                }
                None => {
                    owner.insert(v, t);
                }
            }
        }
    }
    // Compact the roots so an id is dense and stable (and cheap to bucket).
    let mut compact: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(count);
    for t in 0..count {
        let r = root(&mut parent, t);
        let next = compact.len() as u32;
        out.push(*compact.entry(r).or_insert(next));
    }
    out
}

/// Which triangles belong to an item worn on top of the body rather than to the body itself.
///
/// Roblox renders "the avatar" as one composite: the body plus every worn item, with no asset
/// ids anywhere in the file. The geometry still separates: in a composite each rig part of the
/// body is one island, so when a part carries several islands the body's own is the one with
/// the largest bounding volume and the rest are what the user put on it.
///
/// This is a heuristic, so callers must treat the result as "probably the item" and check that
/// what they extracted is small relative to the whole render before drawing it. An empty mask
/// (one island per part) means the split found nothing and the composite should be kept whole.
pub fn accessory_triangle_mask(parts: &[Part], islands: &[u32], tri_pos: &[[[f32; 3]; 3]]) -> Vec<bool> {
    let count = parts.len();
    let mut out = vec![false; count];
    if count == 0 || islands.len() != count || tri_pos.len() != count {
        return out;
    }
    // Bounding volume per island: the body's part encloses more space than the item sitting on
    // it, which separates a head from a hat even when the hat is the denser mesh.
    let mut volume: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    let mut tris: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut bounds: std::collections::HashMap<u32, ([f32; 3], [f32; 3])> =
        std::collections::HashMap::new();
    let mut island_parts: std::collections::HashMap<u32, std::collections::HashSet<u8>> =
        std::collections::HashMap::new();
    for i in 0..count {
        let island = islands[i];
        island_parts
            .entry(island)
            .or_default()
            .insert(parts[i].index() as u8);
        let entry = bounds
            .entry(island)
            .or_insert(([f32::MAX; 3], [f32::MIN; 3]));
        for p in &tri_pos[i] {
            for a in 0..3 {
                entry.0[a] = entry.0[a].min(p[a]);
                entry.1[a] = entry.1[a].max(p[a]);
            }
        }
        *tris.entry(island).or_insert(0) += 1;
    }
    for (island, (min, max)) in &bounds {
        let size = [
            (max[0] - min[0]).max(0.0),
            (max[1] - min[1]).max(0.0),
            (max[2] - min[2]).max(0.0),
        ];
        volume.insert(*island, size[0] * size[1] * size[2]);
    }
    // An island covering several parts is the torso (or a welded whole body): it is the body,
    // never an accessory, because a body part cannot be shoulder-height *and* waist-height.
    let spans = |island: u32| island_parts.get(&island).map_or(0, |p| p.len()) > 1;

    let mut body: std::collections::HashMap<u8, u32> = std::collections::HashMap::new();
    for island in bounds.keys().copied() {
        if spans(island) {
            continue;
        }
        let part = *island_parts
            .get(&island)
            .and_then(|p| p.iter().next())
            .unwrap_or(&0);
        // A part's islands are a small set; the body's is the largest by volume, then by
        // triangle count, then the lowest id so the split is deterministic.
        let better = match body.get(&part) {
            None => true,
            Some(current) => {
                let v = volume.get(&island).copied().unwrap_or(0.0);
                let cv = volume.get(current).copied().unwrap_or(0.0);
                let t = tris.get(&island).copied().unwrap_or(0);
                let ct = tris.get(current).copied().unwrap_or(0);
                (v, t) > (cv, ct) || ((v, t) == (cv, ct) && island < *current)
            }
        };
        if better {
            body.insert(part, island);
        }
    }
    for i in 0..count {
        let part = parts[i].index() as u8;
        out[i] = match body.get(&part) {
            Some(body_island) => *body_island != islands[i],
            // A part with no single-island body of its own (the welded torso) has nothing to
            // compare against, so its geometry stays with the body.
            None => false,
        };
    }
    out
}
