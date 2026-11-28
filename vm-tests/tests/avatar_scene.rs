//! The Avatar Editor's 3D viewport follows the click, not `set-wearing-assets`.
//!
//! These tests drive the local scene the viewport renders from: which asset each group of the
//! Roblox composite belongs to, what happens on an unequip (a hidden group, no request) and on
//! an equip (a group that was already there, or one mesh fetched on its own). The requirement
//! they encode is that a toggle is a visibility flip on the frame it is clicked - never a
//! composite re-render, and never a wait for the save.

use creblox_vm_tests::avatar_anim::{accessory_triangle_mask, classify_triangles, island_ids, Part};
use creblox_vm_tests::avatar_scene::{is_worn_geometry, owners_from_textures, triangle_owner, Scene, BASE};
use std::collections::HashMap;
use std::time::Instant;

/// A composite that already carries two accessories: the body plus a hat and hair, both on the
/// head, each with its own texture atlas.
fn worn_scene() -> Scene {
    let mut scene = Scene::default();
    scene.apply_worn(&[111, 222]);
    assert_eq!(scene.queue(|_| false), vec![111, 222], "first look fetches both");
    scene.mesh_ready(111);
    scene.mesh_ready(222);
    scene
}

#[test]
fn unequipping_hides_the_mesh_group_on_the_same_call() {
    let mut scene = worn_scene();
    assert!(scene.shows(111), "the hat's group is drawn while it is worn");

    let started = Instant::now();
    scene.apply_worn(&[222]);
    let elapsed = started.elapsed();

    // The group is gone from the scene immediately: nothing was fetched, nothing was queued,
    // and no composite re-render was asked for.
    assert!(!scene.shows(111), "the unequipped hat must not be drawn");
    assert!(scene.shows(222), "the hair is untouched");
    assert!(!scene.shows_mesh(111));
    assert!(scene.queue(|_| false).is_empty(), "no network for an unequip");
    assert_eq!(scene.base_reloads, 0, "unequipping must not reload the avatar");
    assert!(
        elapsed.as_micros() < 1_000,
        "the hide took {elapsed:?}, which is not instant"
    );
}

#[test]
fn the_body_is_never_hidden() {
    let mut scene = worn_scene();
    scene.apply_worn(&[]);
    assert!(scene.shows(BASE), "the body has no asset id and is never removed");
    assert!(!scene.shows(111));
}

#[test]
fn equipping_a_cached_item_shows_it_immediately() {
    let mut scene = worn_scene();
    scene.apply_worn(&[]);
    assert!(!scene.shows(111), "the hat's geometry is still there, just hidden");

    let started = Instant::now();
    scene.apply_worn(&[111]);
    let elapsed = started.elapsed();

    // Geometry the composite already has costs nothing: no request, no reload, same frame.
    assert!(scene.shows(111), "the hat is back on the avatar");
    assert!(scene.queue(|_| false).is_empty(), "nothing to fetch");
    assert_eq!(scene.base_reloads, 0);
    assert!(
        elapsed.as_micros() < 1_000,
        "re-equipping took {elapsed:?}, which is not instant"
    );
}

#[test]
fn equipping_a_decoded_item_needs_no_save_before_it_is_drawn() {
    let mut scene = Scene::default();
    // Hovering the card fetched it, and it was decoded before the click landed.
    assert!(scene.prefetch(333), "the hover starts one fetch");
    assert!(!scene.prefetch(333), "and only one");
    scene.mesh_ready(333);
    // The click that wears it draws it on the spot: no save, no fetch.
    scene.apply_worn(&[333]);
    assert!(scene.shows_mesh(333), "a cached item is on the avatar at once");
    assert!(scene.queue(|_| false).is_empty());
    assert_eq!(scene.base_reloads, 0);
    assert_eq!(scene.stamp(), scene.stamp(), "the drawn picture is stable");
}

#[test]
fn an_item_the_composite_never_had_fetches_only_itself() {
    let mut scene = Scene::default();
    scene.apply_worn(&[444]);
    // The composite has no group for it yet, so the item's own mesh is the only way to draw
    // it - and that is one request for one asset, not a new avatar render.
    assert_eq!(scene.queue(|_| false), vec![444]);
    assert!(!scene.shows_mesh(444), "nothing to draw until the mesh lands");
    assert_eq!(scene.base_reloads, 0);
    let before = scene.stamp();

    scene.mesh_ready(444);
    assert!(scene.shows_mesh(444), "drawn the moment it is decoded");
    assert_ne!(scene.stamp(), before, "the raster cache sees the change");
    assert_eq!(scene.base_reloads, 0);
}

#[test]
fn an_item_without_a_mesh_is_never_asked_for_twice() {
    let mut scene = Scene::default();
    scene.apply_worn(&[555]);
    assert_eq!(scene.queue(|_| false), vec![555]);
    scene.mesh_unavailable(555);
    // A click on it again must not start the same failing download over and over.
    scene.apply_worn(&[555, 666]);
    assert!(!scene.queue(|_| false).contains(&555));
    assert!(!scene.shows_mesh(555), "an unavailable item is never drawn as an overlay");
}

#[test]
fn a_failed_save_rolls_the_viewport_back_without_a_reload() {
    let mut scene = worn_scene();
    // Clicking unequip hid the hat...
    scene.apply_worn(&[222]);
    assert!(!scene.shows(111));
    // ...then `set-wearing-assets` came back with an error, so the confirmed set is restored.
    scene.apply_worn(&[111, 222]);
    assert!(scene.shows(111), "the hat is back without a re-render");
    assert!(scene.queue(|_| false).is_empty());
    assert_eq!(scene.base_reloads, 0);
}

#[test]
fn a_burst_of_clicks_is_one_scene_change_per_click() {
    let mut scene = Scene::default();
    // Equip three hats over each other: each click is a local flip, so the scene has to keep
    // up frame by frame instead of waiting for the debounced save.
    for (i, id) in [10, 11, 12].iter().enumerate() {
        scene.apply_worn(&[*id]);
        assert!(scene.shows(*id), "click {i} is on the avatar");
        assert_eq!(scene.base_reloads, 0);
    }
}

#[test]
fn the_composite_is_split_by_asset_texture() {
    let mut textures: HashMap<u64, Vec<String>> = HashMap::new();
    textures.insert(900, vec!["HatAtlas".to_string()]);
    textures.insert(901, vec!["HairAtlas".to_string(), "Shared".to_string()]);
    textures.insert(902, vec!["Shared".to_string()]);

    let hashes = [
        "BodyAtlas".to_string(),
        "HatAtlas".to_string(),
        "HairAtlas".to_string(),
        "Shared".to_string(),
        "Unknown".to_string(),
    ];
    let owners = owners_from_textures(&hashes, &textures);

    assert_eq!(owners[1], 900, "the hat's material is the hat's group");
    assert_eq!(owners[2], 901, "the hair's material is the hair's group");
    // A texture two items share cannot be attributed safely, and an atlas nobody claimed is
    // the body's: both stay with the base so no body part ever disappears.
    assert_eq!(owners[0], BASE);
    assert_eq!(owners[3], 901, "first match wins, deterministically");
    assert_eq!(owners[4], BASE);

    // Triangles without a material, or with an index past the map, are body geometry.
    assert_eq!(triangle_owner(&owners, Some(1)), 900);
    assert_eq!(triangle_owner(&owners, None), BASE);
    assert_eq!(triangle_owner(&owners, Some(99)), BASE);
    assert_eq!(triangle_owner(&[], Some(0)), BASE);
}

#[test]
fn only_geometry_items_are_tracked() {
    // Hats, hair, accessories, layered clothing, eyebrows and eyelashes are drawn.
    for id in [8, 19, 41, 42, 43, 44, 45, 46, 47, 64, 65, 66, 67, 68, 69, 70, 71, 72, 76, 77] {
        assert!(is_worn_geometry(id), "asset type {id} is worn geometry");
    }
    // Animations and emotes are not: an equip of one must not fetch a mesh.
    for id in [0, 2, 11, 12, 17, 18, 27, 48, 51, 61, 78, 79] {
        assert!(!is_worn_geometry(id), "asset type {id} is not geometry");
    }
}

/// The five boxes of the fixture composite, in studs, feet on y = 0: torso, head, hat above
/// the head, and two boots.
const BOXES: [([f32; 3], [f32; 3]); 5] = [
    ([-1.0, 0.0, -0.5], [1.0, 3.0, 0.5]),  // torso
    ([-0.6, 3.0, -0.6], [0.6, 4.2, 0.6]),  // head
    ([-0.8, 4.2, -0.8], [0.8, 4.8, 0.8]),  // hat
    ([-0.9, 0.0, -0.3], [-0.1, 0.9, 0.4]), // left boot
    ([0.1, 0.0, -0.3], [0.9, 0.9, 0.4]),   // right boot
];

/// Builds the given boxes as separate mesh islands, the way Roblox writes them.
fn build_boxes(which: &[usize]) -> (Vec<[u32; 3]>, Vec<[[f32; 3]; 3]>) {
    let faces: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [5, 4, 7, 6],
        [4, 0, 3, 7],
        [1, 5, 6, 2],
        [3, 2, 6, 7],
        [4, 5, 1, 0],
    ];
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut tri_verts: Vec<[u32; 3]> = Vec::new();
    let mut tri_pos: Vec<[[f32; 3]; 3]> = Vec::new();
    for b in which {
        let (min, max) = (BOXES[*b].0, BOXES[*b].1);
        let base = positions.len() as u32;
        for c in [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ] {
            positions.push(c);
        }
        for face in faces {
            for i in [1, 2] {
                let tri = [
                    base + face[0] as u32,
                    base + face[i] as u32,
                    base + face[i + 1] as u32,
                ];
                tri_verts.push(tri);
                tri_pos.push([
                    positions[tri[0] as usize],
                    positions[tri[1] as usize],
                    positions[tri[2] as usize],
                ]);
            }
        }
    }
    (tri_verts, tri_pos)
}

/// The body plus both of its accessories: what `assets-thumbnail-3d` renders for a hat or a
/// pair of boots.
fn composite() -> (Vec<[u32; 3]>, Vec<[[f32; 3]; 3]>, usize, usize) {
    let (tri_verts, tri_pos) = build_boxes(&[0, 1, 2, 3, 4]);
    let total = tri_verts.len();
    (tri_verts, tri_pos, 12, total)
}

#[test]
fn the_items_own_mesh_is_split_from_the_body() {
    let (tri_verts, tri_pos, hat_tris, total) = composite();
    let parts = classify_triangles(&tri_verts, &tri_pos, &vec![None; total], [-1.0, 0.0, -0.8], [1.0, 4.8, 0.8]);
    let islands = island_ids(&tri_verts);
    assert_eq!(islands.len(), total);
    // Five boxes, five islands: nothing welded.
    let distinct: std::collections::HashSet<u32> = islands.iter().copied().collect();
    assert_eq!(distinct.len(), 5);

    let mask = accessory_triangle_mask(&parts, &islands, &tri_pos);
    let kept = mask.iter().filter(|k| **k).count();
    assert_eq!(kept, hat_tris, "only the hat is an accessory island");

    // Every kept triangle is on the head and at or above the hairline, never on the torso or
    // the boots.
    for (i, keep) in mask.iter().enumerate() {
        if *keep {
            let y = tri_pos[i].iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            assert!(y >= 4.2 - 1e-3, "kept triangle {i} starts at {y}");
            assert_eq!(parts[i], Part::Head);
        }
    }
}

#[test]
fn a_body_without_accessories_keeps_every_triangle() {
    // Torso and head only: one island per rig part, so there is nothing the split could call
    // an item and no triangle may be dropped from the avatar.
    let (tri_verts, tri_pos) = build_boxes(&[0, 1]);
    let total = tri_verts.len();
    let parts = classify_triangles(
        &tri_verts,
        &tri_pos,
        &vec![None; total],
        [-1.0, 0.0, -0.5],
        [1.0, 4.2, 0.6],
    );
    let islands = island_ids(&tri_verts);
    let mask = accessory_triangle_mask(&parts, &islands, &tri_pos);
    assert_eq!(mask.len(), total);
    assert!(mask.iter().all(|m| !m), "a plain body has no accessory island");
}
