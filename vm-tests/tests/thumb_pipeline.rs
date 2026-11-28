//! Thumbnail scheduling: LIFO visible-first order, pruning, batch sizes and the lookahead
//! band the grid builds around the viewport.

use std::collections::HashSet;

use creblox_vm_tests::chart_feed::visible_rows;
use creblox_vm_tests::thumb_pipeline::{
    chunks, ThumbQueue, ICON_REQUEST, ICON_TARGET, LOOKAHEAD_ROWS, MAX_BATCH, TEXTURE_CAP,
    UPLOADS_PER_FRAME, WIDE_REQUEST, WIDE_TARGET, WORKERS,
};

// ---------------------------------------------------------------------------
// Order: what the user is looking at goes first, newest first.
// ---------------------------------------------------------------------------

#[test]
fn the_newest_visible_card_is_served_first() {
    let mut q = ThumbQueue::new();
    // Drawn top to bottom, as a grid hands its cards in.
    q.push_visible([10, 11, 12, 13]);
    let batch = q.pop_batch(1);
    assert_eq!(batch, vec![13], "LIFO: the last card handed in wins");
    assert_eq!(q.len(), 3);
    assert_eq!(q.pop_batch(2), vec![12, 11]);
    assert_eq!(q.pop_batch(10), vec![10]);
    assert!(q.is_empty());
}

#[test]
fn an_on_screen_card_always_beats_a_lookahead_one() {
    let mut q = ThumbQueue::new();
    // The lookahead band is pushed after the visible row (it is drawn below it), yet the
    // visible card still has to come out first: the user can see that one.
    q.push_visible([1]);
    q.push_lookahead([90, 91, 92]);
    assert_eq!(q.pop_batch(1), vec![1]);
    assert_eq!(q.pop_batch(3), vec![92, 91, 90], "lookahead is LIFO too");
}

#[test]
fn a_card_is_queued_once_even_if_it_reports_every_frame() {
    let mut q = ThumbQueue::new();
    for _ in 0..40 {
        // A card draws on every frame and reports itself every frame.
        q.push_visible([7]);
        q.push_lookahead([8]);
    }
    assert_eq!(q.len(), 2);
    let mut all = q.pop_batch(MAX_BATCH);
    all.sort();
    assert_eq!(all, vec![7, 8]);
}

#[test]
fn zero_ids_are_never_requested() {
    let mut q = ThumbQueue::new();
    q.push_visible([0, 0, 5]);
    q.push_lookahead([0]);
    assert_eq!(q.pop_batch(10), vec![5]);
}

// ---------------------------------------------------------------------------
// Pruning: an id scrolled far out of the viewport never becomes a request.
// ---------------------------------------------------------------------------

#[test]
fn cards_that_scrolled_away_are_cancelled_before_they_are_sent() {
    let mut q = ThumbQueue::new();
    q.push_visible([1, 2, 3, 4]);
    q.push_lookahead([5, 6]);

    // The user flicked two screens: only 4 and 6 are still near the viewport.
    let alive: HashSet<u64> = [4, 6].into_iter().collect();
    let dropped = q.prune(&alive);
    assert_eq!(dropped, 4);
    assert_eq!(q.len(), 2);
    let mut left = q.pop_batch(MAX_BATCH);
    left.sort();
    assert_eq!(left, vec![4, 6]);
}

#[test]
fn pruning_keeps_the_order_of_what_survives() {
    let mut q = ThumbQueue::new();
    q.push_visible([1, 2, 3]);
    let alive: HashSet<u64> = [1, 3].into_iter().collect();
    q.prune(&alive);
    assert_eq!(q.pop_batch(5), vec![3, 1], "still newest first");
}

#[test]
fn a_pruned_id_can_be_queued_again_when_it_scrolls_back() {
    let mut q = ThumbQueue::new();
    q.push_visible([9]);
    q.prune(&HashSet::new());
    assert!(q.is_empty());
    q.push_visible([9]);
    assert_eq!(q.pop_batch(1), vec![9]);
}

#[test]
fn clearing_drops_every_lane() {
    let mut q = ThumbQueue::new();
    q.push_visible([1, 2]);
    q.push_lookahead([3]);
    q.clear();
    assert!(q.is_empty());
    assert!(q.pop_batch(10).is_empty());
}

// ---------------------------------------------------------------------------
// Batching: 100 ids per call, no repeats, nothing left behind.
// ---------------------------------------------------------------------------

#[test]
fn a_grid_page_becomes_a_handful_of_calls_not_one_per_card() {
    let ids: Vec<u64> = (1..=250).collect();
    let batches = chunks(&ids, MAX_BATCH);
    let sizes: Vec<usize> = batches.iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![100, 100, 50]);
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 250);
}

#[test]
fn every_id_is_requested_exactly_once() {
    let ids: Vec<u64> = vec![1, 2, 2, 3, 0, 3, 4];
    let batches = chunks(&ids, MAX_BATCH);
    let mut flat: Vec<u64> = batches.into_iter().flatten().collect();
    flat.sort();
    assert_eq!(flat, vec![1, 2, 3, 4], "no repeats, no zero ids");
}

#[test]
fn a_batch_is_never_larger_than_the_endpoint_allows() {
    let ids: Vec<u64> = (1..=1000).collect();
    for batch in chunks(&ids, 5000) {
        assert!(
            batch.len() <= MAX_BATCH,
            "asked for {} ids in one call",
            batch.len()
        );
    }
    assert!((50..=100).contains(&MAX_BATCH), "spec: 50-100 ids per call");
}

#[test]
fn the_queue_hands_out_batches_of_at_most_max() {
    let mut q = ThumbQueue::new();
    q.push_visible(1..=250);
    let first = q.pop_batch(MAX_BATCH);
    assert_eq!(first.len(), MAX_BATCH);
    assert_eq!(q.len(), 150);
}

// ---------------------------------------------------------------------------
// The pipeline's shape.
// ---------------------------------------------------------------------------

#[test]
fn a_few_textures_are_uploaded_per_frame() {
    assert!(
        (6..=8).contains(&UPLOADS_PER_FRAME),
        "the spec asks for 6-8 uploads per frame"
    );
    assert!(WORKERS >= 8 && WORKERS <= 12, "8-12 download connections");
    assert!(TEXTURE_CAP >= 64, "the RAM texture budget is worth caching");
}

#[test]
fn cards_download_a_card_sized_image_not_a_full_cover() {
    // 256x144 is the decoded size of a card cover; the request is 384x216, which is sharp
    // at 2x and smaller than the 480x270 it replaced.
    assert_eq!(WIDE_TARGET, [256, 144]);
    assert_eq!(WIDE_REQUEST, "384x216");
    assert_eq!(ICON_TARGET, [64, 64]);
    assert_eq!(ICON_REQUEST, "150x150");
}

#[test]
fn the_grid_builds_two_lookahead_rows_above_and_below() {
    // 100 rows of 100 px, viewport rows 3..6.
    let visible = visible_rows(300.0, 600.0, 0.0, 100.0, 0.0, 100);
    assert_eq!(LOOKAHEAD_ROWS, 2);
    assert!(visible.start <= 1, "rows above the viewport are built too");
    assert!(visible.end >= 8, "rows below the viewport are built too");
    assert!(
        visible.contains(&2) || visible.contains(&3),
        "the viewport itself is always inside the range"
    );
}

#[test]
fn a_grid_at_the_top_still_builds_the_rows_below_it() {
    let visible = visible_rows(0.0, 300.0, 0.0, 100.0, 0.0, 100);
    assert_eq!(visible.start, 0, "there is nothing above row zero");
    assert!(visible.end >= 5);
}

#[test]
fn a_grid_scrolled_to_the_end_does_not_run_past_the_last_row() {
    let visible = visible_rows(9_500.0, 10_000.0, 0.0, 100.0, 0.0, 100);
    assert!(visible.end <= 100);
    assert!(visible.start < 100);
}
