//! Which name a friend bubble shows, and how names are resolved.
//!
//! `GET friends.roblox.com/v1/users/{id}/friends` used to carry `name`/`displayName` on every
//! row. It now often answers `{"data":[{"id":123}]}`, which left every Home bubble labelled
//! "Player" even though the ids, headshots and presences all arrived. The loader fills those
//! names from `POST users.roblox.com/v1/users` (100 ids per request) and remembers what it
//! learned, so a later presence refresh cannot blank a name back to the placeholder.
//!
//! Everything here is pure: `api.rs` owns the requests, this owns the parsing, the batching,
//! the in-memory book and the label rule.

use serde_json::Value;
use std::collections::HashMap;

/// Ids per `POST users.roblox.com/v1/users` request; that endpoint's limit.
pub const NAME_BATCH: usize = 100;

/// What a bubble says while the batch name lookup is still in flight and nothing else is known.
pub const FALLBACK: &str = "Player";

/// One friend (or one `POST /v1/users` row) before presence and headshot are attached.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FriendName {
    pub id: u64,
    pub display: String,
    pub name: String,
    pub verified: bool,
}

impl FriendName {
    /// Whether the batch has answered for this friend yet.
    pub fn is_named(&self) -> bool {
        !self.display.is_empty() || !self.name.is_empty()
    }
}

/// The label rule, shared by the bubble and the tooltip.
pub fn label<'a>(display: &'a str, name: &'a str) -> &'a str {
    if !display.trim().is_empty() {
        display
    } else if !name.trim().is_empty() {
        name
    } else {
        FALLBACK
    }
}

/// Rows of a `{"data":[...]}` payload.
///
/// `/friends` and `POST /v1/users` share the shape (`id`, sometimes `name`, `displayName`,
/// `hasVerifiedBadge`), so one parser reads both. Ids that are missing, zero or repeated are
/// dropped, because a repeated id costs a download and a zero id names nobody.
pub fn parse_rows(v: &Value) -> Vec<FriendName> {
    let mut out: Vec<FriendName> = Vec::new();
    for item in v["data"].as_array().into_iter().flatten() {
        let Some(id) = item["id"].as_u64().filter(|id| *id > 0) else {
            continue;
        };
        if out.iter().any(|row| row.id == id) {
            continue;
        }
        out.push(FriendName {
            id,
            display: item["displayName"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string(),
            name: item["name"].as_str().unwrap_or_default().trim().to_string(),
            verified: item["hasVerifiedBadge"].as_bool().unwrap_or(false),
        });
    }
    out
}

/// Chunks of at most [`NAME_BATCH`] ids, deduped, zeros dropped, order kept.
pub fn batches(ids: &[u64]) -> Vec<Vec<u64>> {
    let mut unique: Vec<u64> = Vec::with_capacity(ids.len());
    for id in ids {
        if *id > 0 && !unique.contains(id) {
            unique.push(*id);
        }
    }
    unique.chunks(NAME_BATCH).map(|c| c.to_vec()).collect()
}

/// One remembered name, with the badge that came with it.
#[derive(Clone, Debug, Default)]
struct NameEntry {
    display: String,
    name: String,
    verified: bool,
}

/// `user id -> (display name, username, verified)`, filled as names are learned and kept for the
/// life of the process. This is what stops a presence refresh from flashing "Player" again.
#[derive(Debug, Default, Clone)]
pub struct NameBook {
    by_id: HashMap<u64, NameEntry>,
}

impl NameBook {
    /// Records what a row knows. Returns `true` when it taught the book something new.
    pub fn remember(&mut self, row: &FriendName) -> bool {
        if !row.is_named() && !row.verified {
            return false;
        }
        let entry = self.by_id.entry(row.id).or_default();
        let mut changed = false;
        if !row.display.is_empty() && entry.display != row.display {
            entry.display = row.display.clone();
            changed = true;
        }
        if !row.name.is_empty() && entry.name != row.name {
            entry.name = row.name.clone();
            changed = true;
        }
        if row.verified && !entry.verified {
            entry.verified = true;
            changed = true;
        }
        changed
    }

    pub fn look(&self, id: u64) -> Option<(&str, &str, bool)> {
        self.by_id
            .get(&id)
            .map(|entry| (entry.display.as_str(), entry.name.as_str(), entry.verified))
    }

    /// Fills whatever the row is missing from the book. `true` if anything changed.
    pub fn fill(&self, row: &mut FriendName) -> bool {
        let Some((display, name, verified)) = self.look(row.id) else {
            return false;
        };
        let mut changed = false;
        if row.display.is_empty() && !display.is_empty() {
            row.display = display.to_string();
            changed = true;
        }
        if row.name.is_empty() && !name.is_empty() {
            row.name = name.to_string();
            changed = true;
        }
        if verified && !row.verified {
            row.verified = true;
            changed = true;
        }
        changed
    }
}

/// The loader's name step, with no network in it.
///
/// Takes the `/friends` rows (often id-only), fills them from the book, then fills whatever the
/// `POST /v1/users` answer taught it and remembers that for the next refresh. Rows the batch did
/// not answer for stay unnamed, so the caller can retry them.
pub fn resolve(
    friends_payload: &Value,
    users_payload: Option<&Value>,
    book: &mut NameBook,
) -> Vec<FriendName> {
    let mut rows = parse_rows(friends_payload);
    for row in &rows {
        book.remember(row);
    }
    for row in &mut rows {
        book.fill(row);
    }
    if let Some(users) = users_payload {
        for row in &parse_rows(users) {
            book.remember(row);
        }
        for row in &mut rows {
            book.fill(row);
        }
    }
    rows
}
