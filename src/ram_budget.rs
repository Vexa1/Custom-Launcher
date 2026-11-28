//! How much RAM the launcher's rebuildable caches may hold, and what to drop when they do not.
//!
//! Card thumbnails, Game Store pictures, gallery pictures, video preview frames and the 3D
//! avatar scene can all be fetched or decoded again, so all of them are budgeted:
//! [`TOTAL_CAP_BYTES`] is a hard ceiling, and [`TRAY_CAP_BYTES`] is what may stay decoded
//! while the window is tucked into the menu bar. The planning here is pure arithmetic, so
//! `vm-tests` can push megabytes through it on a host with no GPU.

/// Hard ceiling for every rebuildable image cache together: uploaded textures, decoded
/// pixels waiting for upload, decoded video frames, the 3D avatar scene and the profile shelf.
pub const TOTAL_CAP_BYTES: usize = 192 * 1024 * 1024;

/// What may stay decoded while the launcher lives in the menu bar. It is dropped when the
/// window is tucked away, so anything above this afterwards is a leak.
pub const TRAY_CAP_BYTES: usize = 40 * 1024 * 1024;

/// Bytes one RGBA image of `width`x`height` takes, on the CPU and on the GPU.
pub const fn texture_bytes(width: usize, height: usize) -> usize {
    width.saturating_mul(height).saturating_mul(4)
}

/// What the tracked caches currently hold, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Uploaded `TextureHandle`s (their pixel size, not the download size).
    pub thumbnail_textures: usize,
    /// Decoded `ColorImage`s waiting for an upload slot.
    pub pending_images: usize,
    /// Decoded VP8 preview frames.
    pub video_frames: usize,
    /// The 3D avatar's meshes and textures.
    pub avatar_scene: usize,
    /// The profile shelf's cloned profiles.
    pub profile: usize,
}

impl Usage {
    pub fn total(self) -> usize {
        self.thumbnail_textures
            .saturating_add(self.pending_images)
            .saturating_add(self.video_frames)
            .saturating_add(self.avatar_scene)
            .saturating_add(self.profile)
    }
}

/// Which caches one enforcement step clears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub clear_videos: bool,
    pub clear_avatar: bool,
    pub clear_profile: bool,
    pub clear_thumbnails: bool,
}

/// What to drop to get under `cap`, or `None` when already inside it.
///
/// Cheapest to rebuild first: video frames (only needed while a preview plays), then the 3D
/// avatar scene, then the profile shelf, and only then every thumbnail texture - which is
/// what a page is actually showing. A step is included only while the running total is still
/// over the cap, so a 10 MB overflow does not throw away the whole page.
pub fn plan(usage: Usage, cap: usize) -> Option<Plan> {
    let mut total = usage.total();
    if total <= cap {
        return None;
    }
    let mut plan = Plan::default();
    if usage.video_frames > 0 {
        plan.clear_videos = true;
        total = total.saturating_sub(usage.video_frames);
    }
    if total > cap && usage.avatar_scene > 0 {
        plan.clear_avatar = true;
        total = total.saturating_sub(usage.avatar_scene);
    }
    if total > cap && usage.profile > 0 {
        plan.clear_profile = true;
        total = total.saturating_sub(usage.profile);
    }
    if total > cap {
        plan.clear_thumbnails = true;
    }
    Some(plan)
}
