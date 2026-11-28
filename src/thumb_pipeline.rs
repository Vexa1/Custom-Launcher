//! Thumbnail scheduling: what gets asked for, in what order, and how much at a time.
//!
//! The thumbnail path used to be implicit: a card drew an `imageUrl` and egui's own loader
//! fetched it whenever it felt like it. That made the order the network worked in an
//! accident of layout, and cards scrolled past still cost bandwidth. This module owns the
//! order instead:
//!
//! * two lanes - cards on screen and cards in the lookahead band - so a request for a row
//!   the user can see always beats a request for a row two rows below;
//! * LIFO inside each lane, so the last card handed in (the one that just scrolled into
//!   view) is served first;
//! * pruning against the set of cards still on screen, so an id that has scrolled away is
//!   dropped before its request is ever sent;
//! * batch sizes that match Roblox's thumbnail endpoints (`50`-`100` ids per call).
//!
//! Everything is in RAM. No path in this module can write to disk.

use std::collections::HashSet;

/// Ids per `thumbnails.roblox.com` call. The endpoints accept 100 universe or asset ids and
/// one call is one round trip, so a grid row of cards costs a fraction of a request.
pub const MAX_BATCH: usize = 100;

/// Textures uploaded per frame. Uploading a whole batch in one frame is a visible hitch; a
/// handful per frame is not, and the queue is drained in a few frames either way.
pub const UPLOADS_PER_FRAME: usize = 6;

/// Concurrent CDN downloads. Ten keeps the shared agent's connection pool busy without
/// opening a socket per image.
pub const WORKERS: usize = 10;

/// Rows built above and below the viewport, so a flick scroll finds its thumbnails already
/// on their way.
pub const LOOKAHEAD_ROWS: usize = 2;

/// Decoded texture budget. A 256x144 RGBA image is ~147 KB, so this is ~28 MB of RAM, well
/// inside the launcher's soft cap, and older cards simply reload if they come back.
pub const TEXTURE_CAP: usize = 192;

/// The size a card's wide thumbnail is decoded to. The grid card is ~220 px wide, so this
/// covers a 2x display without holding the 384x216 download in memory.
pub const WIDE_TARGET: [u32; 2] = [256, 144];

/// The size an icon is decoded to; icon chips are drawn at 16-40 px.
pub const ICON_TARGET: [u32; 2] = [64, 64];

/// A two-lane LIFO queue of thumbnail ids.
///
/// `visible` is what the user is looking at; `lookahead` is the band just outside the
/// viewport. `pop_batch` always empties `visible` first, newest first, and only then starts
/// on the lookahead band.
#[derive(Debug, Default, Clone)]
pub struct ThumbQueue {
    visible: Vec<u64>,
    lookahead: Vec<u64>,
    queued: HashSet<u64>,
}

impl ThumbQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cards inside the viewport. Call in draw order; the last one handed in wins.
    pub fn push_visible(&mut self, ids: impl IntoIterator<Item = u64>) {
        for id in ids {
            if id != 0 && self.queued.insert(id) {
                self.visible.push(id);
            }
        }
    }

    /// Cards in the lookahead band, above or below the viewport.
    pub fn push_lookahead(&mut self, ids: impl IntoIterator<Item = u64>) {
        for id in ids {
            if id != 0 && self.queued.insert(id) {
                self.lookahead.push(id);
            }
        }
    }

    /// Drops everything that is no longer on screen. Returns how many ids were cancelled.
    ///
    /// Called before every batch is taken, so an id the user scrolled past never becomes a
    /// request.
    pub fn prune(&mut self, alive: &HashSet<u64>) -> usize {
        let before = self.visible.len() + self.lookahead.len();
        self.visible.retain(|id| alive.contains(id));
        self.lookahead.retain(|id| alive.contains(id));
        self.queued.retain(|id| alive.contains(id));
        before - (self.visible.len() + self.lookahead.len())
    }

    /// Takes up to `max` ids: the newest on-screen ones, then the newest lookahead ones.
    pub fn pop_batch(&mut self, max: usize) -> Vec<u64> {
        let mut out = Vec::new();
        while out.len() < max {
            let next = self.visible.pop().or_else(|| self.lookahead.pop());
            let Some(id) = next else { break };
            // A pruned id is still in `queued` only if it is alive; this keeps the two lanes
            // and the dedupe set in step whatever order they were touched in.
            if self.queued.remove(&id) {
                out.push(id);
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.visible.len() + self.lookahead.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.visible.clear();
        self.lookahead.clear();
        self.queued.clear();
    }
}

/// Splits ids into request-sized chunks, largest first, with no id repeated.
///
/// A `250`-id row set becomes `100 + 100 + 50` calls, which is the whole point: a scroll
/// that reveals three rows costs one call per 100 cards instead of one per card.
pub fn chunks(ids: &[u64], max: usize) -> Vec<Vec<u64>> {
    let max = max.max(1).min(MAX_BATCH);
    let mut seen = HashSet::new();
    let mut unique: Vec<u64> = Vec::with_capacity(ids.len());
    for id in ids {
        if *id != 0 && seen.insert(*id) {
            unique.push(*id);
        }
    }
    unique.chunks(max).map(<[u64]>::to_vec).collect()
}

/// The size Roblox is asked for, per kind of thumbnail.
///
/// Cards draw a 16:9 tile ~220 px wide, so `384x216` is sharp at 2x and half the bytes of
/// the old `480x270`. Icons are chips, not covers: `150x150` is already more than a 32 px
/// chip can show.
pub const WIDE_REQUEST: &str = "384x216";
pub const ICON_REQUEST: &str = "150x150";
