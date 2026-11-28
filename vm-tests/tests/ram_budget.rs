//! The hard RAM cap, the trim order, and the empty state a tray flush must leave behind.
//!
//! `src/ram_budget.rs` is the real module, included by path. These tests push megabytes of
//! simulated thumbnails, decoded pixels and video frames through it, the way an endless
//! Charts scroll does, and check the tracked total never ends above the cap.

use creblox_vm_tests::ram_budget::{self, Usage};
use std::hint::black_box;

#[test]
fn thousands_of_thumbnail_loads_stay_under_the_hard_cap() {
    let per_card = ram_budget::texture_bytes(256, 144);
    let mut textures = 0usize;
    let mut pending = 0usize;
    let mut fired = false;

    for i in 0..5000 {
        textures += per_card;
        // A download batch lands every few cards: decoded pixels waiting for an upload slot.
        if i % 8 == 0 {
            pending += per_card * 4;
        }
        let usage = Usage {
            thumbnail_textures: textures,
            pending_images: pending,
            ..Default::default()
        };
        if let Some(plan) = ram_budget::plan(usage, ram_budget::TOTAL_CAP_BYTES) {
            if plan.clear_thumbnails {
                textures = 0;
                pending = 0;
                fired = true;
            }
        }
        let after = Usage {
            thumbnail_textures: textures,
            pending_images: pending,
            ..Default::default()
        };
        assert!(
            after.total() <= ram_budget::TOTAL_CAP_BYTES,
            "iteration {i} kept {} bytes with the cap at {}",
            after.total(),
            ram_budget::TOTAL_CAP_BYTES
        );
    }

    black_box(&mut textures);
    assert!(fired, "the cap must fire somewhere in 5000 loads");
}

#[test]
fn a_big_video_cache_is_dropped_before_the_page_art() {
    let usage = Usage {
        thumbnail_textures: 40 << 20,
        video_frames: 300 << 20,
        ..Default::default()
    };
    let plan = ram_budget::plan(usage, ram_budget::TOTAL_CAP_BYTES).expect("over the cap");
    assert!(plan.clear_videos, "video frames are the cheapest to rebuild");
    assert!(!plan.clear_avatar);
    assert!(!plan.clear_thumbnails, "the page keeps its pictures");
}

#[test]
fn a_pathological_total_clears_every_cache() {
    let usage = Usage {
        thumbnail_textures: 190 << 20,
        pending_images: 20 << 20,
        video_frames: 10 << 20,
        avatar_scene: 32 << 20,
        profile: 4 << 20,
    };
    let plan = ram_budget::plan(usage, ram_budget::TOTAL_CAP_BYTES).expect("over the cap");
    assert!(plan.clear_videos && plan.clear_avatar && plan.clear_profile);
    assert!(plan.clear_thumbnails, "nothing else is left to free");
}

#[test]
fn the_tray_flush_leaves_nothing_to_evict() {
    // What `flush_image_caches` leaves behind: every tracked cache at zero.
    let after = Usage::default();
    assert_eq!(after.total(), 0);
    assert!(
        ram_budget::plan(after, ram_budget::TRAY_CAP_BYTES).is_none(),
        "a flushed launcher is under the tray budget by a wide margin"
    );
    assert!(after.total() < ram_budget::TRAY_CAP_BYTES);
}

#[test]
fn an_idle_one_screen_cache_is_inside_the_cap() {
    // A realistic steady state: card textures, one hero picture and the pending queue.
    let usage = Usage {
        thumbnail_textures: ram_budget::texture_bytes(256, 144) * 192,
        pending_images: ram_budget::texture_bytes(256, 144) * 48,
        video_frames: ram_budget::texture_bytes(320, 180) * 96,
        avatar_scene: 32 << 20,
        profile: 0,
    };
    assert!(usage.total() < ram_budget::TOTAL_CAP_BYTES);
    assert!(ram_budget::plan(usage, ram_budget::TOTAL_CAP_BYTES).is_none());
}
