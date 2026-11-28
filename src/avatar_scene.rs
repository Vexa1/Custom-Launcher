//! Local per-asset layering for the Avatar Editor's 3D viewport.
//!
//! The viewport draws one composite that Roblox rendered: the body plus every worn item, in a
//! single OBJ. Before this module, an equip/unequip only became visible after the whole
//! `set-wearing-assets` round trip *and* a fresh composite render and download - seconds of
//! waiting for something the user already clicked.
//!
//! Here the composite is split into per-asset groups (see [`owners_from_textures`]) and the
//! viewport keeps its own idea of what is worn. Toggling is a visibility flip: an item that is
//! already in the composite comes back on the same frame, and an item the composite never had
//! is drawn from its own decoded mesh as soon as that mesh is in memory. Nothing here ever
//! asks for a new composite render - `base_reloads` must stay at zero.
//!
//! No egui and no network, so `vm-tests/` drives these rules directly.

use std::collections::{HashMap, HashSet};

/// Owner id of the body's own geometry (the part of the composite nobody can unequip).
pub const BASE: u64 = 0;

/// What the viewport currently has for one asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshState {
    /// The item's own mesh is decoded and can be drawn over the composite.
    Ready,
    /// A fetch is on the wire.
    Loading,
    /// There is no separate mesh for this item; it will never be drawn on its own.
    Unavailable,
}

/// The viewport's local scene: the worn set, the meshes it has, and what still needs fetching.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    worn: HashSet<u64>,
    meshes: HashMap<u64, MeshState>,
    /// How many times a fresh composite render was requested. Equipping never asks for one -
    /// that is the whole point of this module - and the app asserts it at runtime.
    pub base_reloads: u32,
}

impl Scene {
    /// Replaces the worn set. Returns whether anything changed (the caller repaints).
    ///
    /// Hiding is free: the composite group for a removed asset simply stops being drawn, on
    /// this very call. Equipping is free too when the item is already in the composite, which
    /// is why the item id is enough here - the renderer asks [`Scene::shows`] per group.
    pub fn apply_worn(&mut self, worn: &[u64]) -> bool {
        let next: HashSet<u64> = worn.iter().copied().filter(|id| *id != BASE).collect();
        if next == self.worn {
            return false;
        }
        self.worn = next;
        true
    }

    pub fn worn(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.worn.iter().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Should this group of the composite be drawn? `owner` is [`BASE`] or an asset id.
    ///
    /// Unworn items keep their geometry in the model (removing triangles would need a rebuild)
    /// and are skipped at draw time instead, which is what makes unequipping instant and
    /// re-equipping a previously worn item instant as well.
    pub fn shows(&self, owner: u64) -> bool {
        owner == BASE || self.worn.contains(&owner)
    }

    /// Should the item's own decoded mesh be drawn over the composite?
    pub fn shows_mesh(&self, asset_id: u64) -> bool {
        self.worn.contains(&asset_id) && self.meshes.get(&asset_id) == Some(&MeshState::Ready)
    }

    /// Worn items that have no mesh yet, marking them as loading. The caller fetches exactly
    /// these, so a click causes one request per never-seen item - never a composite reload.
    pub fn queue(&mut self, cached: impl Fn(u64) -> bool) -> Vec<u64> {
        let mut out = Vec::new();
        for &id in &self.worn {
            match self.meshes.get(&id) {
                Some(MeshState::Ready) => continue,
                Some(MeshState::Loading) | Some(MeshState::Unavailable) => continue,
                None => {}
            }
            if cached(id) {
                self.meshes.insert(id, MeshState::Ready);
                continue;
            }
            self.meshes.insert(id, MeshState::Loading);
            out.push(id);
        }
        out.sort_unstable();
        out
    }

    /// Starts a fetch ahead of the click (the pointer resting on a card). `false` when the
    /// mesh is already decoded, already on its way, or known not to exist.
    pub fn prefetch(&mut self, asset_id: u64) -> bool {
        if asset_id == BASE || self.meshes.contains_key(&asset_id) {
            return false;
        }
        self.meshes.insert(asset_id, MeshState::Loading);
        true
    }

    /// An asset's mesh arrived. `false` when it is no longer worn, so the caller can drop it
    /// from the cache without a repaint.
    pub fn mesh_ready(&mut self, asset_id: u64) -> bool {
        self.meshes.insert(asset_id, MeshState::Ready);
        self.worn.contains(&asset_id)
    }

    /// An asset has no separate mesh (or it failed to download). It is never asked for again
    /// in this session, which is what stops a failing item from re-fetching every frame.
    pub fn mesh_unavailable(&mut self, asset_id: u64) {
        self.meshes.insert(asset_id, MeshState::Unavailable);
    }

    /// A cheap fingerprint of what the scene draws right now: the worn ids and which of them
    /// have their own mesh ready. The raster cache mixes it into its key, so an equip or
    /// unequip re-renders the *picture* without touching the network.
    pub fn stamp(&self) -> u64 {
        fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
            for b in bytes {
                hash = (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for id in self.worn() {
            hash = fnv(hash, &id.to_le_bytes());
            let ready = self.meshes.get(&id) == Some(&MeshState::Ready);
            hash = fnv(hash, &[u8::from(ready)]);
        }
        hash
    }

}

/// Whether an asset type is geometry the viewport has to draw, as opposed to an animation,
/// an emote or a body scale setting.
pub fn is_worn_geometry(asset_type_id: u64) -> bool {
    matches!(asset_type_id, 8 | 19 | 41..=47 | 64..=72 | 76 | 77)
}

/// The asset each composite material belongs to, in the model's texture order.
///
/// Roblox draws a worn item with its own texture atlas, so a material's `map_Kd` hash is a
/// per-asset boundary inside the composite even though the OBJ names every material
/// `Part12Mtl`. Materials whose texture belongs to no worn asset (the body, and anything
/// vertex-coloured) stay [`BASE`]. An item sharing a texture with another picks the first
/// match, and an unmapped texture never hides body geometry by accident.
pub fn owners_from_textures(
    texture_hashes: &[String],
    asset_textures: &HashMap<u64, Vec<String>>,
) -> Vec<u64> {
    let mut first_owner: HashMap<&str, u64> = HashMap::new();
    let mut ids: Vec<&u64> = asset_textures.keys().collect();
    ids.sort_unstable();
    for id in ids {
        for hash in &asset_textures[id] {
            first_owner.entry(hash.as_str()).or_insert(*id);
        }
    }
    texture_hashes
        .iter()
        .map(|hash| first_owner.get(hash.as_str()).copied().unwrap_or(BASE))
        .collect()
}

/// Asset id for one triangle of the composite, given its material index.
pub fn triangle_owner(material_owners: &[u64], texture_index: Option<usize>) -> u64 {
    texture_index
        .and_then(|index| material_owners.get(index).copied())
        .unwrap_or(BASE)
}
