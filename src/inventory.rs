//! The player inventory: Roblox's inventory endpoints, its JSON, the privacy/rate-limit
//! policy and the per-tab cache.
//!
//! Everything here is free of egui, a session and any I/O, so the rules that decide what a
//! tab shows - and what it must *not* re-fetch - run in `vm-tests` where the GUI crate cannot
//! be linked. `api.rs` performs the requests, `ui/page_inventory.rs` draws the result.

use serde_json::Value;

/// One tab of the inventory viewer.
///
/// Roblox's inventory is per asset type, not per tab, so a tab owns a short list of type IDs
/// and merges their pages. Tabs are fetched only when opened; nothing is prefetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Accessories,
    Clothing,
    FacesHeads,
    GearAnimations,
    BadgesPasses,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Accessories,
        Category::Clothing,
        Category::FacesHeads,
        Category::GearAnimations,
        Category::BadgesPasses,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Accessories => "Accessories",
            Category::Clothing => "Clothing",
            Category::FacesHeads => "Faces & Heads",
            Category::GearAnimations => "Gear & Animations",
            Category::BadgesPasses => "Badges & Passes",
        }
    }

    /// The `assetTypeId`s this tab asks for, in the order Roblox lists them.
    pub fn asset_types(self) -> &'static [u32] {
        match self {
            // Hat, hair, face, neck, shoulder, front, back, waist.
            Category::Accessories => &[8, 41, 42, 43, 44, 45, 46, 47],
            // Classic shirts, classic pants, T-shirts.
            Category::Clothing => &[11, 12, 2],
            // Face, head, dynamic head.
            Category::FacesHeads => &[18, 17, 79],
            // Gear, emotes, and the animation asset types Roblox keeps in the inventory.
            Category::GearAnimations => &[19, 61, 51, 53, 55, 48, 50, 52, 54],
            // Badges live on their own endpoint; game passes are asset type 34.
            Category::BadgesPasses => &[21, 34],
        }
    }

    /// Badges are not served by the inventory endpoint at all.
    pub fn uses_badges(self) -> bool {
        matches!(self, Category::BadgesPasses)
    }
}

/// One row of an inventory page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryItem {
    pub id: u64,
    pub name: String,
    pub asset_type: u32,
    /// `true` for a badge: it opens on the badge page, not the catalog.
    pub is_badge: bool,
}

impl InventoryItem {
    /// Short uppercase type badge shown on the card (`HAT`, `SHIRT`, `EMOTE`...).
    pub fn type_label(&self) -> &'static str {
        if self.is_badge {
            return "BADGE";
        }
        type_label(self.asset_type)
    }

    /// The page "Open on Roblox" goes to.
    pub fn page_url(&self) -> String {
        if self.is_badge {
            format!("https://www.roblox.com/badges/{}", self.id)
        } else {
            format!("https://www.roblox.com/catalog/{}", self.id)
        }
    }
}

/// A human label for an asset type id, as used by the card badge.
pub fn type_label(asset_type: u32) -> &'static str {
    match asset_type {
        2 => "T-SHIRT",
        8 => "HAT",
        11 => "SHIRT",
        12 => "PANTS",
        17 => "HEAD",
        18 => "FACE",
        19 => "GEAR",
        21 => "BADGE",
        34 => "PASS",
        41 => "HAIR",
        42 => "FACE",
        43 => "NECK",
        44 => "SHOULDER",
        45 => "FRONT",
        46 => "BACK",
        47 => "WAIST",
        48 | 50 | 51 | 52 | 53 | 54 | 55 => "ANIMATION",
        61 => "EMOTE",
        79 => "DYNAMIC HEAD",
        _ => "ASSET",
    }
}

/// Why a tab has nothing to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// Roblox refuses to show this inventory, with or without a cookie. Permanent: never retry.
    Private,
    /// HTTP 429. The tab backs off for `retry_after_secs` and tries again once, visibly.
    RateLimited { retry_after_secs: u64 },
    /// Anything else, with a message fit for the status line.
    Other(String),
}

impl FetchError {
    /// Whether spending another request could change the answer.
    pub fn retryable(&self) -> bool {
        matches!(self, FetchError::RateLimited { .. })
    }
}

/// One page of a tab: the items and how to ask for the next one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<InventoryItem>,
    /// Next cursor per asset type, in [`Category::asset_types`] order (`None` = done).
    pub cursors: Vec<Option<String>>,
    /// Every type has been walked to its last page.
    pub complete: bool,
}

/// The URL for one inventory page of one asset type.
pub fn inventory_url(user_id: u64, asset_type: u32, cursor: Option<&str>) -> String {
    let mut url = format!(
        "https://inventory.roblox.com/v2/users/{user_id}/inventory/{asset_type}?limit=50&sortOrder=Desc"
    );
    if let Some(cursor) = cursor.filter(|c| !c.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&percent_encode(cursor));
    }
    url
}

/// The URL for one page of a user's badges (badges are not in the inventory API).
pub fn badges_url(user_id: u64, cursor: Option<&str>) -> String {
    let mut url = format!("https://badges.roblox.com/v1/users/{user_id}/badges?limit=50&sortOrder=Desc");
    if let Some(cursor) = cursor.filter(|c| !c.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&percent_encode(cursor));
    }
    url
}

/// The thumbnail request for a batch of assets, `150x150` as the grid draws them.
pub fn thumbnails_url(ids: &[u64]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let list = ids
        .iter()
        .take(50)
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Some(format!(
        "https://thumbnails.roblox.com/v1/assets?assetIds={list}&size=150x150&format=Png&isCircular=false"
    ))
}

/// Percent-encodes a cursor for the query string (cursors can carry `+`, `/`, `=`).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Maps a failed request to the policy the viewer applies.
pub fn classify_error(status: u16, body: &str) -> FetchError {
    let lower = body.to_ascii_lowercase();
    if status == 429 {
        return FetchError::RateLimited {
            retry_after_secs: 5,
        };
    }
    if status == 403
        || lower.contains("inventory is hidden")
        || lower.contains("inventory is private")
    {
        return FetchError::Private;
    }
    FetchError::Other(format!("HTTP {status}"))
}

/// Parses one inventory page. `asset_type` is the type that was requested, used when the
/// payload leaves it out.
pub fn parse_inventory(v: &Value, asset_type: u32) -> Result<Page, FetchError> {
    let data = v["data"]
        .as_array()
        .ok_or_else(|| FetchError::Other("unexpected inventory response".to_string()))?;
    let mut items = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item["assetId"].as_u64().or_else(|| item["id"].as_u64()) else {
            continue;
        };
        items.push(InventoryItem {
            id,
            name: item["name"].as_str().unwrap_or_default().to_string(),
            asset_type: item["assetType"]
                .as_u64()
                .and_then(|t| u32::try_from(t).ok())
                .unwrap_or(asset_type),
            is_badge: false,
        });
    }
    Ok(Page {
        items,
        cursors: vec![next_cursor(v)],
        complete: next_cursor(v).is_none(),
    })
}

/// Parses one page of the badges endpoint.
pub fn parse_badges(v: &Value) -> Result<Page, FetchError> {
    let data = v["data"]
        .as_array()
        .ok_or_else(|| FetchError::Other("unexpected badges response".to_string()))?;
    let items = data
        .iter()
        .filter_map(|badge| {
            Some(InventoryItem {
                id: badge["id"].as_u64()?,
                name: badge["name"].as_str().unwrap_or_default().to_string(),
                asset_type: 21,
                is_badge: true,
            })
        })
        .collect();
    Ok(Page {
        items,
        cursors: vec![next_cursor(v)],
        complete: next_cursor(v).is_none(),
    })
}

fn next_cursor(v: &Value) -> Option<String> {
    v["nextPageCursor"]
        .as_str()
        .filter(|c| !c.trim().is_empty())
        .map(str::to_string)
}

/// A fetched tab, kept hot so switching back to it costs nothing.
#[derive(Clone, Debug, Default)]
pub struct TabEntry {
    pub items: Vec<InventoryItem>,
    /// Cursor per entry of [`Category::asset_types`] (`None` = that type is exhausted).
    pub cursors: Vec<Option<String>>,
    pub complete: bool,
    pub error: Option<FetchError>,
}

/// A small LRU of fetched tabs, keyed by `(user_id, category)`.
///
/// The viewer keeps every user in one session here, but only the few most recently opened
/// tabs: an inventory can be thousands of items and the launcher is RAM-budgeted.
#[derive(Debug)]
pub struct TabCache {
    capacity: usize,
    /// Least recently used first, most recent last.
    entries: Vec<((u64, Category), TabEntry)>,
}

impl TabCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Vec::new(),
        }
    }

    /// The cached tab, marked as used. `None` when it was never fetched.
    pub fn get_mut(&mut self, key: (u64, Category)) -> Option<&mut TabEntry> {
        let index = self.entries.iter().position(|(k, _)| *k == key)?;
        let entry = self.entries.remove(index);
        self.entries.push(entry);
        self.entries.last_mut().map(|(_, e)| e)
    }

    /// Stores (or refreshes) a tab and evicts the least recently used one when full.
    pub fn insert(&mut self, key: (u64, Category), entry: TabEntry) {
        if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
            self.entries.remove(index);
        }
        self.entries.push((key, entry));
        while self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
    }

    /// Drops one tab (used when a `429` backoff has expired and the tab is retried).
    pub fn remove(&mut self, key: (u64, Category)) {
        self.entries.retain(|(k, _)| *k != key);
    }
}

/// Whether clicking this tab should start a request.
///
/// Cached tabs open instantly; a tab already in flight must not be requested twice (that is
/// how a fast tab-switch used to turn into duplicate API calls and a 429).
pub fn needs_fetch(
    cache: &mut TabCache,
    key: (u64, Category),
    in_flight: Option<(u64, Category)>,
) -> bool {
    if in_flight == Some(key) {
        return false;
    }
    cache.get_mut(key).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_category_has_types_and_a_label() {
        for c in Category::ALL {
            assert!(!c.asset_types().is_empty(), "{c:?}");
            assert!(!c.label().is_empty(), "{c:?}");
        }
    }

    #[test]
    fn a_cursor_is_percent_encoded() {
        let url = inventory_url(42, 8, Some("a+b/c=="));
        assert!(url.ends_with("&cursor=a%2Bb%2Fc%3D%3D"), "{url}");
        assert!(inventory_url(42, 8, None).ends_with("sortOrder=Desc"));
    }

    #[test]
    fn thumbnails_are_requested_at_150() {
        let url = thumbnails_url(&[1, 2, 3]).expect("url");
        assert!(url.contains("assetIds=1,2,3"), "{url}");
        assert!(url.contains("size=150x150"), "{url}");
        assert!(thumbnails_url(&[]).is_none());
    }
}
