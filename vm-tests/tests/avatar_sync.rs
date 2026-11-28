//! The Avatar Editor's equip path: instant local toggles, one debounced request per click
//! burst, stale revision rejection, clean rollback, and decoded-avatar reuse.

use creblox_vm_tests::avatar_sync::{AvatarSync, MeshStore, EQUIP_DEBOUNCE};
use creblox_vm_tests::types::AvatarAsset;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn asset(id: u64, name: &str, type_id: u64) -> AvatarAsset {
    AvatarAsset {
        id,
        name: name.to_string(),
        asset_type_id: type_id,
        asset_type_name: "Hat".to_string(),
        thumb: String::new(),
        meta: None,
        creator_name: String::new(),
        creator_verified: false,
        price: None,
    }
}

#[test]
fn equipping_updates_the_local_set_on_the_same_click() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    let hat = asset(1, "Top Hat", 8);
    sync.seed(&[]);

    assert!(sync.toggle(&hat, now));
    // No server round trip: the local set already contains it, so the UI can paint it now.
    assert_eq!(sync.desired, vec![hat.clone()]);
    assert_eq!(sync.revision, 1);
    assert!(sync.pending());

    assert!(!sync.toggle(&hat, now + Duration::from_millis(10)));
    assert!(sync.desired.is_empty());
    assert_eq!(sync.revision, 2);
}

#[test]
fn a_new_shirt_replaces_the_previous_one_in_the_same_slot() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    sync.toggle(&asset(1, "Blue Shirt", 11), now);
    sync.toggle(&asset(2, "Red Shirt", 11), now + Duration::from_millis(5));

    assert_eq!(
        sync.desired.iter().map(|a| a.id).collect::<Vec<_>>(),
        vec![2]
    );
}

#[test]
fn rapid_clicks_coalesce_into_one_request_after_the_debounce() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    let (hat, shirt) = (asset(1, "Hat", 8), asset(2, "Shirt", 11));

    sync.toggle(&hat, now);
    sync.toggle(&shirt, now + Duration::from_millis(60));
    sync.toggle(&hat, now + Duration::from_millis(120));

    // Still inside the debounce window: nothing is sent yet.
    assert!(sync
        .take_request(now + Duration::from_millis(200))
        .is_none());
    // The save is a background concern, so its wait is only about coalescing clicks.
    assert_eq!(EQUIP_DEBOUNCE, Duration::from_millis(300));

    let (revision, assets) = sync
        .take_request(now + Duration::from_millis(120) + EQUIP_DEBOUNCE)
        .expect("one request for the whole burst");
    assert_eq!(revision, 3);
    assert_eq!(assets, vec![shirt]);

    // While it is on the wire no second request is produced.
    assert!(sync
        .take_request(now + Duration::from_secs(5))
        .is_none());
    assert!(sync.pending());
    assert!(sync.settle(revision, &assets));
    assert!(!sync.pending());
}

#[test]
fn a_response_for_an_older_revision_never_overwrites_newer_clicks() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    let (hat, shirt) = (asset(1, "Hat", 8), asset(2, "Shirt", 11));
    sync.toggle(&hat, now);
    let (old_revision, _) = sync
        .take_request(now + EQUIP_DEBOUNCE)
        .expect("first request");

    // The user clicks again while the first request is still in flight.
    sync.toggle(&shirt, now + EQUIP_DEBOUNCE + Duration::from_millis(20));
    sync.toggle(&hat, now + EQUIP_DEBOUNCE + Duration::from_millis(40));
    assert!(sync.is_stale(old_revision));

    // The old success is ignored for the UI but still records the server set, and the newer
    // clicks are re-queued so they are not lost.
    assert!(!sync.settle(old_revision, &[hat.clone()]));
    assert_eq!(sync.desired, vec![shirt.clone()]);
    let (revision, assets) = sync
        .take_request(Instant::now())
        .expect("newer clicks are sent next");
    assert!(revision > old_revision);
    assert_eq!(assets, vec![shirt]);
}

#[test]
fn a_3d_render_from_a_previous_revision_is_discarded() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    sync.toggle(&asset(1, "Hat", 8), now);
    let revision_at_start = sync.revision;
    sync.toggle(&asset(2, "Shirt", 11), now + Duration::from_millis(10));

    assert!(sync.is_stale(revision_at_start));
    assert!(!sync.is_stale(sync.revision));
}

#[test]
fn a_failed_request_rolls_back_to_the_confirmed_set() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    let confirmed = asset(1, "Hat", 8);
    sync.seed(std::slice::from_ref(&confirmed));

    sync.toggle(&asset(2, "Shirt", 11), now);
    let (revision, _) = sync.take_request(now + EQUIP_DEBOUNCE).unwrap();
    assert!(sync.fail(revision, "429 Too Many Requests", Instant::now()));

    assert_eq!(sync.desired, vec![confirmed]);
    assert_eq!(sync.confirmed, vec![asset(1, "Hat", 8)]);
    assert!(!sync.pending());
    assert_eq!(
        sync.take_error().as_deref(),
        Some("429 Too Many Requests")
    );
    assert!(sync.take_error().is_none(), "the toast is shown once");
}

#[test]
fn a_failure_for_an_older_revision_keeps_the_newer_clicks() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    sync.seed(&[]);
    sync.toggle(&asset(1, "Hat", 8), now);
    let (old_revision, _) = sync.take_request(now + EQUIP_DEBOUNCE).unwrap();
    sync.toggle(&asset(2, "Shirt", 11), now + EQUIP_DEBOUNCE);

    assert!(!sync.fail(old_revision, "boom", Instant::now()));
    let expected = vec![asset(1, "Hat", 8), asset(2, "Shirt", 11)];
    assert_eq!(sync.desired, expected);
    let (revision, assets) = sync.take_request(Instant::now()).unwrap();
    assert!(revision > old_revision);
    assert_eq!(assets, expected);
}

#[test]
fn seeding_never_throws_away_pending_clicks() {
    let now = Instant::now();
    let mut sync = AvatarSync::default();
    sync.seed(&[asset(1, "Hat", 8)]);
    sync.toggle(&asset(2, "Shirt", 11), now);

    sync.seed(&[asset(1, "Hat", 8)]);
    assert_eq!(sync.desired, vec![asset(1, "Hat", 8), asset(2, "Shirt", 11)]);
}

#[test]
fn a_cached_avatar_is_reused_instead_of_rebuilt() {
    let mut store: MeshStore<String> = MeshStore::new(64);
    let model = Arc::new("decoded mesh + textures".to_string());
    store.put("obj-hash", Arc::clone(&model), 32);

    let hit = store.get("obj-hash").expect("cache hit");
    assert!(Arc::ptr_eq(&hit, &model), "the same decoded payload is reused");
    assert_eq!(*hit, "decoded mesh + textures");
    assert!(store.get("other-hash").is_none());
}

#[test]
fn the_mesh_cache_stays_within_its_budget() {
    let mut store: MeshStore<String> = MeshStore::new(100);
    store.put("old", Arc::new("a".into()), 60);
    store.put("new", Arc::new("b".into()), 60);

    // The first payload had to go: 60 + 60 > 100.
    assert!(store.get("old").is_none());
    assert!(store.get("new").is_some());

    // A payload larger than the whole budget is not cached at all.
    store.put("huge", Arc::new("c".into()), 200);
    assert!(store.get("huge").is_none());
}

#[test]
fn the_least_recently_used_avatar_is_evicted_first() {
    let mut store: MeshStore<String> = MeshStore::new(100);
    store.put("old", Arc::new("a".into()), 50);
    store.put("recent", Arc::new("b".into()), 50);
    // Using the older entry makes the newer one the eviction candidate.
    assert!(store.get("old").is_some());

    store.put("newest", Arc::new("c".into()), 50);
    assert!(store.get("old").is_some(), "recently used entry stays");
    assert!(store.get("newest").is_some());
    assert!(store.get("recent").is_none());
}
