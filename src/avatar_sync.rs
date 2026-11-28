//! Optimistic equip/unequip state for the Avatar Editor, and the mesh cache policy that
//! keeps re-equipping an item instant.
//!
//! The editor used to POST `set-wearing-assets` on every click and wait for the server before
//! touching anything the user could see. Here the desired set is local and immediate; a
//! debounced single request syncs it, a monotonic revision lets late responses and stale 3D
//! fetches be dropped, and a failure restores the last confirmed set.
//!
//! Deliberately free of egui and the network so `vm-tests/` can drive it.

use crate::types::AvatarAsset;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The asset type a name refers to, for inventory items that only carry a display name.
pub fn asset_type_id_from_name(name: &str) -> u64 {
    match name {
        "TShirt" | "T-Shirt" => 2,
        "Hat" => 8,
        "Shirt" => 11,
        "Pants" => 12,
        "Head" => 17,
        "Face" => 18,
        "Gear" => 19,
        "Torso" => 27,
        "RightArm" | "Right Arm" => 28,
        "LeftArm" | "Left Arm" => 29,
        "LeftLeg" | "Left Leg" => 30,
        "RightLeg" | "Right Leg" => 31,
        "HairAccessory" | "Hair Accessory" => 41,
        "FaceAccessory" | "Face Accessory" => 42,
        "NeckAccessory" | "Neck Accessory" => 43,
        "ShoulderAccessory" | "Shoulder Accessory" => 44,
        "FrontAccessory" | "Front Accessory" => 45,
        "BackAccessory" | "Back Accessory" => 46,
        "WaistAccessory" | "Waist Accessory" => 47,
        "ClimbAnimation" => 48,
        "DeathAnimation" => 49,
        "FallAnimation" => 50,
        "IdleAnimation" => 51,
        "JumpAnimation" => 52,
        "RunAnimation" => 53,
        "SwimAnimation" => 54,
        "WalkAnimation" => 55,
        "PoseAnimation" => 56,
        "EmoteAnimation" => 61,
        "TShirtAccessory" => 64,
        "ShirtAccessory" => 65,
        "PantsAccessory" => 66,
        "JacketAccessory" => 67,
        "SweaterAccessory" => 68,
        "ShortsAccessory" => 69,
        "LeftShoeAccessory" => 70,
        "RightShoeAccessory" => 71,
        "DressSkirtAccessory" => 72,
        "EyebrowAccessory" => 76,
        "EyelashAccessory" => 77,
        "MoodAnimation" => 78,
        "DynamicHead" => 79,
        "FaceMakeup" => 88,
        "LipMakeup" => 89,
        "EyeMakeup" => 90,
        _ => 0,
    }
}

pub fn is_single_slot_asset(asset_type_id: u64) -> bool {
    matches!(
        asset_type_id,
        2 | 11
            | 12
            | 17
            | 18
            | 19
            | 27
            | 28
            | 29
            | 30
            | 31
            | 48
            | 49
            | 50
            | 51
            | 52
            | 53
            | 54
            | 55
            | 56
            | 78
            | 79
    )
}

pub fn compute_toggled_assets(
    wearing: &[AvatarAsset],
    item: &AvatarAsset,
) -> (Vec<AvatarAsset>, bool) {
    if wearing.iter().any(|w| w.id == item.id) {
        let next = wearing
            .iter()
            .filter(|w| w.id != item.id)
            .cloned()
            .collect();
        return (next, false);
    }
    let single = if item.asset_type_id != 0 {
        is_single_slot_asset(item.asset_type_id)
    } else {
        is_single_slot_asset(asset_type_id_from_name(&item.asset_type_name))
    };
    let mut next: Vec<AvatarAsset> = if single {
        wearing
            .iter()
            .filter(|w| {
                if item.asset_type_id != 0 && w.asset_type_id != 0 {
                    w.asset_type_id != item.asset_type_id
                } else {
                    !w.asset_type_name
                        .eq_ignore_ascii_case(&item.asset_type_name)
                }
            })
            .cloned()
            .collect()
    } else {
        wearing.to_vec()
    };
    next.push(item.clone());
    (next, true)
}

/// How long the editor waits after the last click before sending one *background* request.
///
/// Nothing the user sees waits for it: the 3D viewport is already showing the click, and the
/// request is a silent save that never triggers a re-render of the composite.
pub const EQUIP_DEBOUNCE: Duration = Duration::from_millis(400);

/// The editor's equip state machine.
#[derive(Debug, Default)]
pub struct AvatarSync {
    /// Monotonic counter: every equip/unequip bumps it, and any response tagged with an older
    /// value is discarded instead of overwriting newer clicks.
    pub revision: u64,
    /// The last wearing set Roblox confirmed.
    pub confirmed: Vec<AvatarAsset>,
    /// The set the user has clicked their way to. This is what the UI draws.
    pub desired: Vec<AvatarAsset>,
    pub error: Option<String>,
    in_flight: Option<u64>,
    due_at: Option<Instant>,
}

impl AvatarSync {
    /// Adopts the server's set (first load, or another avatar action). A pending edit wins:
    /// seeding never throws away clicks the user has already made.
    pub fn seed(&mut self, wearing: &[AvatarAsset]) {
        if self.pending() {
            return;
        }
        self.confirmed = wearing.to_vec();
        self.desired = wearing.to_vec();
        self.error = None;
    }

    /// Flips one asset, honouring Roblox's single-slot rules (a new hat replaces the old one).
    /// Returns whether it is now worn. The caller repaints immediately.
    pub fn toggle(&mut self, asset: &AvatarAsset, now: Instant) -> bool {
        let (next, equipped) = compute_toggled_assets(&self.desired, asset);
        self.desired = next;
        self.revision += 1;
        self.due_at = Some(now + EQUIP_DEBOUNCE);
        self.error = None;
        equipped
    }

    /// Drops every asset matching `drop` (the "remove all accessories" button) and schedules
    /// one sync for the result.
    pub fn retain_where(&mut self, keep: impl Fn(&AvatarAsset) -> bool, now: Instant) {
        self.desired.retain(|a| keep(a));
        self.revision += 1;
        self.due_at = Some(now + EQUIP_DEBOUNCE);
        self.error = None;
    }

    /// A request is waiting for its debounce or is on the wire: the viewport shows its pill.
    pub fn pending(&self) -> bool {
        self.in_flight.is_some() || self.due_at.is_some()
    }

    /// One request's worth of work, once the debounce elapsed and nothing is in flight. The
    /// debounce is what turns five rapid clicks into a single `set-wearing-assets` call.
    pub fn take_request(&mut self, now: Instant) -> Option<(u64, Vec<AvatarAsset>)> {
        if self.in_flight.is_some() {
            return None;
        }
        let due = self.due_at?;
        if now < due {
            return None;
        }
        self.due_at = None;
        self.in_flight = Some(self.revision);
        Some((self.revision, self.desired.clone()))
    }

    /// Roblox confirmed a request. `true` means it is still the newest one and the server set
    /// may be adopted; a stale confirmation only updates `confirmed` and queues the newer
    /// clicks that are still waiting.
    pub fn settle(&mut self, revision: u64, server_wearing: &[AvatarAsset]) -> bool {
        if self.in_flight == Some(revision) {
            self.in_flight = None;
        }
        self.confirmed = server_wearing.to_vec();
        if revision == self.revision {
            self.desired = server_wearing.to_vec();
            self.error = None;
            true
        } else {
            self.due_at = Some(Instant::now());
            false
        }
    }

    /// A request failed. For the newest revision the optimistic set is rolled back to the last
    /// confirmed one; an older failure just re-queues the newer clicks.
    pub fn fail(&mut self, revision: u64, message: impl Into<String>, now: Instant) -> bool {
        if self.in_flight == Some(revision) {
            self.in_flight = None;
        }
        if revision != self.revision {
            self.due_at = Some(now);
            return false;
        }
        self.desired = self.confirmed.clone();
        self.revision += 1;
        self.due_at = None;
        self.error = Some(message.into());
        true
    }

    /// Takes the pending error out, so it is shown once (as a toast) and not every frame.
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// Whether a 3D fetch started at `revision` is already out of date.
    pub fn is_stale(&self, revision: u64) -> bool {
        revision != self.revision
    }
}

/// A bounded, reuse-first cache of decoded payloads, keyed by content hash.
///
/// The avatar viewport is re-fetched whenever Roblox re-renders the model; toggling an item
/// back to a recently seen look must not download and decode the same OBJ and textures again.
/// Payloads are reference-counted, so a hit hands out the very same mesh instead of rebuilding
/// it, and the least recently used entry is evicted once the byte budget is exceeded.
#[derive(Debug)]
pub struct MeshStore<P> {
    budget: usize,
    used: usize,
    tick: u64,
    entries: Vec<(String, Arc<P>, usize, u64)>,
}

impl<P> MeshStore<P> {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            tick: 0,
            entries: Vec::new(),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<Arc<P>> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.iter_mut().find(|(k, _, _, _)| k == key)?;
        entry.3 = tick;
        Some(Arc::clone(&entry.1))
    }

    /// Stores a payload, replacing an older entry with the same key and evicting the least
    /// recently used entries until the budget fits. A payload larger than the whole budget is
    /// simply not cached.
    pub fn put(&mut self, key: impl Into<String>, payload: Arc<P>, bytes: usize) {
        let key = key.into();
        if bytes > self.budget {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|(k, _, _, _)| *k == key) {
            self.used = self.used.saturating_sub(self.entries[pos].2);
            self.entries.remove(pos);
        }
        self.tick += 1;
        self.entries.push((key, payload, bytes, self.tick));
        self.used += bytes;
        while self.used > self.budget && !self.entries.is_empty() {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.3)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let (_, _, size, _) = self.entries.remove(oldest);
            self.used = self.used.saturating_sub(size);
        }
    }
}

impl<P> Default for MeshStore<P> {
    fn default() -> Self {
        Self::new(96 * 1024 * 1024)
    }
}
