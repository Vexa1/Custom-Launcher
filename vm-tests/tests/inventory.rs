//! The Player Inventory Viewer's headless rules: JSON, privacy, rate limits, tab caching.
//!
//! `src/inventory.rs` is the real module, included by path. The network and the grid are not
//! involved: these are the decisions that must never regress into duplicate API calls or a
//! retry loop against a private inventory.

use creblox_vm_tests::inventory::{
    Category, FetchError, InventoryItem, Page, TabCache, TabEntry, classify_error, inventory_url,
    needs_fetch, parse_badges, parse_inventory, thumbnails_url, type_label,
};

fn sample_page() -> serde_json::Value {
    serde_json::json!({
        "data": [
            {"assetId": 111, "name": "Sparkle Fedora", "assetType": 8, "created": "2024-01-01T00:00:00Z"},
            {"assetId": 222, "name": "Blue Hair", "assetType": 41},
            {"id": 333, "name": "No asset id field", "assetType": 42}
        ],
        "nextPageCursor": "abc+/=="
    })
}

#[test]
fn an_inventory_page_parses_items_and_its_cursor() {
    let page = parse_inventory(&sample_page(), 8).expect("valid page");
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].id, 111);
    assert_eq!(page.items[0].name, "Sparkle Fedora");
    assert_eq!(page.items[0].asset_type, 8);
    assert_eq!(page.items[0].type_label(), "HAT");
    // Items that only carry `id` still come through, and the requested type is the fallback.
    assert_eq!(page.items[2].id, 333);
    assert_eq!(page.items[2].type_label(), "FACE");
    assert_eq!(page.cursors, vec![Some("abc+/==".to_string())]);
    assert!(!page.complete);
    // The cursor is part of the next request, encoded.
    assert!(inventory_url(7, 8, Some("abc+/==")).contains("cursor=abc%2B%2F%3D%3D"));
}

#[test]
fn the_last_page_has_no_cursor() {
    let value = serde_json::json!({"data": [], "nextPageCursor": null});
    let page = parse_inventory(&value, 8).expect("valid page");
    assert!(page.items.is_empty());
    assert!(page.complete);
}

#[test]
fn badges_are_parsed_from_their_own_endpoint() {
    let value = serde_json::json!({
        "data": [
            {"id": 9001, "name": "Welcome to the Club", "awardedDate": "2023-05-05T00:00:00Z"}
        ],
        "nextPageCursor": ""
    });
    let page = parse_badges(&value).expect("valid badges");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, 9001);
    assert_eq!(page.items[0].type_label(), "BADGE");
    assert!(page.items[0].page_url().contains("/badges/9001"));
    assert!(page.complete, "an empty cursor means the last page");
}

#[test]
fn a_hidden_inventory_is_private_and_never_retried() {
    // Roblox answers 403, sometimes with a body that says it in words.
    let err = classify_error(403, "{\"errors\":[{\"message\":\"User's inventory is hidden\"}]}");
    assert_eq!(err, FetchError::Private);
    assert!(!err.retryable());
    assert_eq!(
        classify_error(400, "the user's inventory is hidden"),
        FetchError::Private
    );
    // It is cached as an error, which is what stops the frame loop from retrying it.
    let mut cache = TabCache::new(8);
    let key = (42, Category::Accessories);
    cache.insert(
        key,
        TabEntry {
            error: Some(FetchError::Private),
            ..TabEntry::default()
        },
    );
    assert!(!needs_fetch(&mut cache, key, None));
}

#[test]
fn rate_limits_back_off_without_retrying_immediately() {
    let err = classify_error(429, "");
    assert!(err.retryable());
    assert!(matches!(err, FetchError::RateLimited { retry_after_secs } if retry_after_secs > 0));
    assert_eq!(classify_error(500, "oops"), FetchError::Other("HTTP 500".into()));
    assert!(!classify_error(500, "oops").retryable());
}

#[test]
fn tabs_are_cached_lru_and_a_tab_in_flight_is_not_requested_twice() {
    let mut cache = TabCache::new(2);
    let a = (7, Category::Accessories);
    let b = (7, Category::Clothing);
    let c = (7, Category::FacesHeads);
    assert!(needs_fetch(&mut cache, a, None), "nothing cached yet");
    cache.insert(a, TabEntry::default());
    cache.insert(b, TabEntry::default());
    assert!(!needs_fetch(&mut cache, a, None), "a cached tab is instant");
    assert!(
        !needs_fetch(&mut cache, c, Some(c)),
        "an in-flight tab must not be requested again"
    );
    // Touch A so B becomes the least recently used, then add C: B is evicted.
    assert!(cache.get_mut(a).is_some());
    cache.insert(c, TabEntry::default());
    assert!(cache.get_mut(b).is_none(), "the oldest tab is evicted");
    assert!(cache.get_mut(a).is_some());
    assert!(cache.get_mut(c).is_some());
}

#[test]
fn a_second_page_appends_to_the_tab_in_the_cache() {
    // The viewer's merge rule: a load-more never repeats an item already on screen.
    let mut entry = TabEntry {
        items: vec![InventoryItem {
            id: 1,
            name: "One".into(),
            asset_type: 8,
            is_badge: false,
        }],
        ..TabEntry::default()
    };
    let page = Page {
        items: vec![
            InventoryItem {
                id: 1,
                name: "One".into(),
                asset_type: 8,
                is_badge: false,
            },
            InventoryItem {
                id: 2,
                name: "Two".into(),
                asset_type: 8,
                is_badge: false,
            },
        ],
        cursors: vec![None],
        complete: true,
    };
    let seen: Vec<u64> = entry.items.iter().map(|i| i.id).collect();
    entry
        .items
        .extend(page.items.into_iter().filter(|i| !seen.contains(&i.id)));
    assert_eq!(entry.items.len(), 2);
    assert!(page.complete);
    assert_eq!(type_label(79), "DYNAMIC HEAD");
    assert_eq!(type_label(61), "EMOTE");
    assert!(thumbnails_url(&[1, 2]).unwrap().contains("size=150x150"));
}
