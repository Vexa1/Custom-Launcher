//! Keeping a card's current thumbnail on screen while its replacement is decoded.
//!
//! Thumbnails are cached by image URL, so a refresh that hands a game a new URL would leave its
//! card looking at a URL with no texture yet - a grey box until the new image is decoded and
//! uploaded. This module records that the new URL replaces the old one and resolves a URL to the
//! newest texture the card can actually paint. The old handle keeps drawing at full brightness
//! until the replacement exists, and then the swap happens on one frame because the new URL's
//! own texture is found first. Nothing here ever sets a card back to "no image".

use std::collections::HashMap;

/// New image URL -> the URL it replaced.
pub type Aliases = HashMap<String, String>;

/// How far a replacement chain is followed. A URL replaced again and again in a long session
/// still resolves, and a cycle can only cost this many lookups.
const MAX_HOPS: usize = 8;

/// Records that `fresh` replaces `old`.
pub fn record(aliases: &mut Aliases, fresh: &str, old: &str) {
    if fresh != old {
        aliases.insert(fresh.to_string(), old.to_string());
    }
}

/// Records the replacements between two gallery media lists, index by index, and returns how
/// many were recorded.
///
/// A revalidated Game Details page usually answers with the same pictures; when one URL does
/// change, the hero that is showing that index keeps painting the version it already has until
/// the new image is decoded, instead of falling back to a placeholder.
pub fn record_media(aliases: &mut Aliases, fresh: &[String], old: &[String]) -> usize {
    let mut recorded = 0;
    for (fresh, previous) in fresh.iter().zip(old.iter()) {
        if !fresh.is_empty() && fresh != previous {
            record(aliases, fresh, previous);
            recorded += 1;
        }
    }
    recorded
}

/// Records the replacements between two lists of keyed image URLs, e.g. a game's pass id and
/// its picture, and returns how many changed.
///
/// A pass that is new, or whose picture did not change, records nothing: only the pictures the
/// store is already showing get an old-texture bridge.
pub fn record_keyed(
    aliases: &mut Aliases,
    fresh: &[(u64, String)],
    old: &[(u64, String)],
) -> usize {
    let mut recorded = 0;
    for (key, fresh_url) in fresh {
        let Some((_, old_url)) = old.iter().find(|(seen, _)| seen == key) else {
            continue;
        };
        if !fresh_url.trim().is_empty() && fresh_url != old_url {
            record(aliases, fresh_url, old_url);
            recorded += 1;
        }
    }
    recorded
}

/// The texture to paint for `url`: its own if it has been uploaded, otherwise the newest one
/// along its replacement chain. `None` only when nothing in the chain was ever uploaded.
pub fn find<'a, T>(
    textures: &'a HashMap<String, T>,
    aliases: &'a Aliases,
    url: &str,
) -> Option<&'a T> {
    if let Some(texture) = textures.get(url) {
        return Some(texture);
    }
    let mut link = aliases.get(url);
    for _ in 0..MAX_HOPS {
        let previous = link?;
        if let Some(texture) = textures.get(previous) {
            return Some(texture);
        }
        link = aliases.get(previous);
    }
    None
}

/// What the cards may paint: every uploaded texture plus a replacement alias for each replaced
/// URL whose own texture is not uploaded yet. Built fresh on the frame the pool changed, so the
/// swap to a decoded image lands in a single frame rather than passing through a placeholder.
pub fn published<T: Clone>(
    textures: &HashMap<String, T>,
    aliases: &Aliases,
) -> HashMap<String, T> {
    let mut map = textures.clone();
    for fresh in aliases.keys() {
        if map.contains_key(fresh) {
            continue;
        }
        if let Some(texture) = find(textures, aliases, fresh) {
            map.insert(fresh.clone(), texture.clone());
        }
    }
    map
}
