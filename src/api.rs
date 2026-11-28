use eframe::egui::{self, Color32};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::profile_cache::ProfileUpdate;
use crate::types::{
    AvatarAsset, AvatarScales, AvatarState, AvatarTab, BodyColors, FriendProfile, Game, GameExtra,
    GamePassItem, Msg, OmniHome, OutfitItem, PaletteColor, PlayerBadge, Profile, ResolvedGameLink,
    SavedSyncData, ServerItem, Sort, User, UserSearchPage,
};
use crate::chart_feed::{ChartError, SortsPage, game_from_explore, parse_sorts_page};
use crate::video_preview;

pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .user_agent("creblox/0.1")
            .build()
    })
}

pub fn get_json(url: &str) -> Result<Value, String> {
    auth_get_json(url, None)
}

pub fn auth_get_json(url: &str, cookie: Option<&str>) -> Result<Value, String> {
    let mut req = agent().get(url);
    if let Some(c) = cookie {
        req = req.set("Cookie", &format!(".ROBLOSECURITY={c}"));
    }
    req.call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())
}

/// One worker-thread fetch of an inventory tab.
///
/// A tab owns several asset types (Roblox's inventory API is per type), so this walks them
/// and merges their pages. `pages_per_type` bounds the work of one call: the viewer asks for
/// one page per type when a tab opens and one more per type when the grid is scrolled to the
/// end. A private inventory aborts immediately - retrying it can never succeed - and a 429 is
/// handed back as [`FetchError::RateLimited`] for the viewer to back off on.
pub fn fetch_inventory_tab(
    user_id: u64,
    category: crate::inventory::Category,
    cursors: Vec<Option<String>>,
    cookie: Option<&str>,
    pages_per_type: usize,
) -> Result<crate::inventory::Page, crate::inventory::FetchError> {
    use crate::inventory::{Page, badges_url, inventory_url, parse_badges, parse_inventory};
    let types = category.asset_types();
    let mut out = Page {
        items: Vec::new(),
        cursors: vec![None; types.len()],
        complete: false,
    };
    let pages_wanted = pages_per_type.max(1);

    // Badges are served by `badges.roblox.com`, not by the inventory endpoint. Asset type 21
    // stays in the category so its cursor lines up with the tab's type list.
    if category.uses_badges() {
        let slot = types.iter().position(|t| *t == 21);
        let mut cursor = slot.and_then(|i| cursors.get(i).cloned().flatten());
        let mut pages = 0;
        loop {
            let value = inventory_get(&badges_url(user_id, cursor.as_deref()), cookie)?;
            let page = parse_badges(&value)?;
            out.items.extend(page.items);
            cursor = page.cursors.into_iter().next().flatten();
            pages += 1;
            if cursor.is_none() || pages >= pages_wanted {
                break;
            }
        }
        if let Some(i) = slot {
            out.cursors[i] = cursor;
        }
    }

    for (i, asset_type) in types.iter().enumerate() {
        if category.uses_badges() && *asset_type == 21 {
            continue; // already walked above
        }
        let mut cursor = cursors.get(i).cloned().flatten();
        let mut pages = 0;
        loop {
            let value = inventory_get(
                &inventory_url(user_id, *asset_type, cursor.as_deref()),
                cookie,
            )?;
            let page = parse_inventory(&value, *asset_type)?;
            out.items.extend(page.items);
            cursor = page.cursors.into_iter().next().flatten();
            pages += 1;
            if cursor.is_none() || pages >= pages_wanted {
                break;
            }
        }
        out.cursors[i] = cursor;
    }

    out.complete = out.cursors.iter().all(Option::is_none);
    // A tab walks several types, so the same asset can come back twice; keep the first sight.
    let mut seen = HashSet::new();
    out.items.retain(|item| seen.insert(item.id));
    Ok(out)
}

/// Downloads the `150x150` thumbnails for a batch of inventory items.
///
/// The thumbnail endpoints are metadata, not images: one request per kind (assets, badge icons,
/// game pass icons) answers with `targetId` + `imageUrl`, and the images themselves are then
/// fetched in parallel. Anything that fails is simply left out, so the card keeps its
/// placeholder instead of drawing a broken-image glyph.
pub fn fetch_inventory_thumbnails(
    items: Vec<(u64, crate::inventory::ThumbKind)>,
) -> Vec<(u64, egui::ColorImage)> {
    use crate::inventory::{ThumbKind, parse_thumbnail_batch, thumbnail_batch_url};
    use std::collections::HashMap;
    use std::io::Read;

    let mut by_kind: HashMap<ThumbKind, Vec<u64>> = HashMap::new();
    for (id, kind) in items {
        if id != 0 {
            by_kind.entry(kind).or_default().push(id);
        }
    }

    let mut urls: Vec<(u64, String)> = Vec::new();
    for (kind, ids) in by_kind {
        for chunk in ids.chunks(crate::thumb_pipeline::MAX_BATCH) {
            let Some(url) = thumbnail_batch_url(kind, chunk) else {
                continue;
            };
            let json = agent()
                .get(&url)
                .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
                .call()
                .ok()
                .and_then(|r| r.into_json::<Value>().ok());
            if let Some(json) = json {
                urls.extend(parse_thumbnail_batch(&json));
            }
        }
    }

    let download = |url: &str| -> Option<egui::ColorImage> {
        let mut bytes = Vec::new();
        agent()
            .get(url)
            .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
            .call()
            .ok()?
            .into_reader()
            // Same caps as the game covers: an icon is drawn at 150x150 and must not be able
            // to allocate a budget-breaking picture.
            .take(crate::thumb_pipeline::MAX_IMAGE_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        if !crate::api::image_dimensions_fit(&bytes) {
            return None;
        }
        let image = image::load_from_memory(&bytes).ok()?;
        let rgba = image.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        Some(egui::ColorImage::from_rgba_unmultiplied(
            size,
            rgba.as_flat_samples().as_slice(),
        ))
    };

    // The same pool as the game covers: independent images, one connection each.
    let mut out: Vec<(u64, egui::ColorImage)> = Vec::new();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (id, url) in urls {
            handles.push(s.spawn(move || (id, download(&url))));
            if handles.len() >= crate::thumb_pipeline::WORKERS {
                for handle in handles.drain(..) {
                    if let Ok((id, Some(image))) = handle.join() {
                        out.push((id, image));
                    }
                }
            }
        }
        for handle in handles.drain(..) {
            if let Ok((id, Some(image))) = handle.join() {
                out.push((id, image));
            }
        }
    });
    out.sort_by_key(|(id, _)| *id);
    out
}

/// One Roblox GET with the inventory policy attached: 403 becomes `Private`, 429 carries its
/// `Retry-After` when Roblox sends one, everything else keeps its status.
fn inventory_get(url: &str, cookie: Option<&str>) -> Result<Value, crate::inventory::FetchError> {
    use crate::inventory::{FetchError, classify_error};
    let mut req = agent().get(url);
    if let Some(c) = cookie {
        req = req.set("Cookie", &format!(".ROBLOSECURITY={c}"));
    }
    match req.call() {
        Ok(resp) => resp
            .into_json()
            .map_err(|e| FetchError::Other(e.to_string())),
        Err(ureq::Error::Status(code, resp)) => {
            let retry_after = resp
                .header("Retry-After")
                .or_else(|| resp.header("x-ratelimit-reset"))
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5)
                .clamp(1, 120);
            let body = resp.into_string().unwrap_or_default();
            Err(match classify_error(code, &body) {
                FetchError::RateLimited { .. } => FetchError::RateLimited {
                    retry_after_secs: retry_after,
                },
                other => other,
            })
        }
        Err(e) => Err(FetchError::Other(e.to_string())),
    }
}

/// Looks up a rough "City, Region, Country" for a server IP address.
pub fn lookup_server_location(ip: &str) -> Option<String> {
    fn join(parts: [Option<&str>; 3]) -> Option<String> {
        let mut out: Vec<String> = Vec::new();
        for p in parts.into_iter().flatten() {
            let p = p.trim();
            if !p.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(p)) {
                out.push(p.to_string());
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out.join(", "))
        }
    }
    if let Ok(v) = get_json(&format!("https://ipinfo.io/{ip}/json")) {
        if let Some(s) = join([
            v["city"].as_str(),
            v["region"].as_str(),
            v["country"].as_str(),
        ]) {
            return Some(s);
        }
    }
    if let Ok(v) = get_json(&format!(
        "http://ip-api.com/json/{ip}?fields=status,country,regionName,city"
    )) {
        let hit = (v["status"].as_str() == Some("success"))
            .then(|| {
                join([
                    v["city"].as_str(),
                    v["regionName"].as_str(),
                    v["country"].as_str(),
                ])
            })
            .flatten();
        if let Some(s) = hit {
            return Some(s);
        }
    }
    // Last resort: ipinfo.io throttles unauthenticated traffic and ip-api.com answers 429
    // under load. Both used to fail with nothing written anywhere, so a whole session could
    // sit on "Unknown location" with no way to tell why.
    match get_json(&format!("https://ipapi.co/{ip}/json/")) {
        Ok(v) => {
            let joined = join([
                v["city"].as_str(),
                v["region"].as_str(),
                v["country_name"].as_str(),
            ]);
            if joined.is_none() {
                crate::auth::log(&format!(
                    "geoip: ipapi.co returned no location for {ip} ({})",
                    v["reason"].as_str().unwrap_or("no reason given")
                ));
            }
            joined
        }
        Err(e) => {
            crate::auth::log(&format!("geoip: every endpoint failed for {ip}: {e}"));
            None
        }
    }
}

pub fn post_json(url: &str, body: Value) -> Result<Value, String> {
    agent()
        .post(url)
        .send_json(body)
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())
}

pub fn build_json_req(
    method: &str,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    csrf: Option<&str>,
) -> ureq::Request {
    let mut req = agent()
        .request(method, url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    if let Some(c) = cookie {
        req = req.set("Cookie", &format!(".ROBLOSECURITY={c}"));
    }
    if let Some(r) = referer {
        req = req.set("Referer", r);
    }
    if let Some(t) = csrf {
        req = req.set("x-csrf-token", t);
    }
    req
}

pub fn parse_ureq_err(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            if let Ok(v) = resp.into_json::<Value>() {
                if let Some(msg) = v["errors"][0]["message"].as_str() {
                    return msg.to_string();
                }
                if let Some(msg) = v["message"].as_str() {
                    return msg.to_string();
                }
            }
            format!("HTTP {code}")
        }
        e => e.to_string(),
    }
}

/// CSRF tokens are per session and valid for many requests. Caching one per cookie turns
/// every authenticated POST (presence, friend actions, avatar changes) from two round
/// trips — 403, then retry — into one.
fn csrf_tokens() -> &'static Mutex<HashMap<String, String>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn csrf_token_for(cookie: &str) -> Option<String> {
    csrf_tokens().lock().ok()?.get(cookie).cloned()
}

fn remember_csrf(cookie: Option<&str>, token: &str) {
    let Some(c) = cookie else { return };
    if let Ok(mut tokens) = csrf_tokens().lock() {
        if tokens.len() > 8 {
            tokens.clear();
        }
        tokens.insert(c.to_string(), token.to_string());
    }
}

pub fn send_csrf(
    method: &str,
    url: &str,
    body: Value,
    cookie: Option<&str>,
    referer: Option<&str>,
) -> Result<ureq::Response, String> {
    if let Some(token) = cookie.and_then(csrf_token_for) {
        match build_json_req(method, url, cookie, referer, Some(&token)).send_json(body.clone()) {
            Ok(resp) => return Ok(resp),
            Err(ureq::Error::Status(403, resp)) => {
                // The token rotated: use the fresh one from this response and retry once.
                if let Some(fresh) = resp.header("x-csrf-token").map(str::to_string) {
                    remember_csrf(cookie, &fresh);
                    return build_json_req(method, url, cookie, referer, Some(&fresh))
                        .send_json(body)
                        .map_err(parse_ureq_err);
                }
            }
            Err(e) => return Err(parse_ureq_err(e)),
        }
    }
    match build_json_req(method, url, cookie, referer, None).send_json(body.clone()) {
        Ok(resp) => Ok(resp),
        Err(ureq::Error::Status(403, resp)) => {
            if let Some(token) = resp.header("x-csrf-token").map(str::to_string) {
                remember_csrf(cookie, &token);
                build_json_req(method, url, cookie, referer, Some(&token))
                    .send_json(body)
                    .map_err(parse_ureq_err)
            } else {
                Err("403 Forbidden (missing CSRF token)".to_string())
            }
        }
        Err(e) => Err(parse_ureq_err(e)),
    }
}

pub fn post_csrf(
    url: &str,
    body: Value,
    cookie: Option<&str>,
    referer: Option<&str>,
) -> Result<ureq::Response, String> {
    send_csrf("POST", url, body, cookie, referer)
}

pub fn patch_csrf(
    url: &str,
    body: Value,
    cookie: Option<&str>,
    referer: Option<&str>,
) -> Result<ureq::Response, String> {
    send_csrf("PATCH", url, body, cookie, referer)
}

pub fn extract_roblosecurity(resp: &ureq::Response) -> Option<String> {
    for header in resp.all("set-cookie") {
        if let Some(rest) = header.trim_start().strip_prefix(".ROBLOSECURITY=") {
            let value = rest.split(';').next().unwrap_or("").trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

pub fn enc(s: &str) -> String {
    let mut out = String::new();
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

pub fn join(ids: &[u64]) -> String {
    ids.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn format_iso_date(iso: &str) -> String {
    let date = iso.split('T').next().unwrap_or("");
    let mut parts = date.split('-');
    if let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) {
        if let (Ok(yi), Ok(mi), Ok(di)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) {
            return format!("{mi}/{di}/{yi}");
        }
    }
    "--".to_string()
}

pub fn format_commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

static SID_SEQ: AtomicU64 = AtomicU64::new(1);

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

pub fn new_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SID_SEQ.fetch_add(1, Ordering::Relaxed);
    let a = mix64((nanos as u64) ^ seq.wrapping_mul(0x517cc1b727220a95));
    let b = mix64(((nanos >> 64) as u64) ^ seq ^ 0xa0761d6478bd642f);
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

pub fn val_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
}

pub fn fetch_omni_home(sid: &str, cookie: &str) -> Result<OmniHome, String> {
    let body = serde_json::json!({
        "pageType": "Home",
        "sessionId": sid,
        "supportedTreatmentTypes": ["SortlessGrid", "Carousel"],
    });
    let resp = post_csrf(
        "https://apis.roblox.com/discovery-api/omni-recommendation",
        body,
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    let v: Value = resp.into_json().map_err(|e| e.to_string())?;
    let mut continue_unis = None;
    let mut topics = Vec::new();
    for s in v["sorts"].as_array().into_iter().flatten() {
        let topic = str_of(&s["topic"]);
        let treatment = str_of(&s["treatmentType"]);
        let topic_id = val_u64(&s["topicId"]).unwrap_or(0);
        let lower_topic = topic.to_ascii_lowercase();
        if treatment.eq_ignore_ascii_case("Sponsored") || lower_topic.contains("sponsored") {
            continue;
        }
        let mut row_unis = Vec::new();
        let mut seen = HashSet::new();
        for item in s["recommendationList"].as_array().into_iter().flatten() {
            if item["contentType"].as_str() != Some("Game") {
                continue;
            }
            if item["contentMetadata"]["isSponsored"].as_bool() == Some(true) {
                continue;
            }
            if let Some(uid) = val_u64(&item["contentId"]) {
                if uid > 0 && seen.insert(uid) {
                    row_unis.push(uid);
                    if row_unis.len() >= 24 {
                        break;
                    }
                }
            }
        }
        let is_continue = topic.eq_ignore_ascii_case("Continue") || topic_id == 100000003;
        if is_continue {
            if continue_unis.is_none() {
                continue_unis = Some(row_unis);
            }
        } else if !topic.is_empty() && !row_unis.is_empty() {
            topics.push((topic, row_unis));
        }
    }
    Ok(OmniHome {
        continue_unis,
        topics,
    })
}

pub fn fetch_sorts(sid: &str, cookie: Option<&str>) -> Result<SortsPage, String> {
    let mut out = Vec::new();
    let mut next_token: Option<String> = None;
    let mut explore_err = None;
    // The opening sync walks a few pages so the feed has plenty to show before the first
    // scroll; everything after that is prefetched on demand by `fetch_sorts_more`.
    for page in 0..3 {
        let mut url = format!(
            "https://apis.roblox.com/explore-api/v1/get-sorts?sessionId={sid}&device=computer&country=all"
        );
        if let Some(token) = &next_token {
            url.push_str(&format!("&sortsPageToken={}", enc(token)));
        }
        let v = match auth_get_json(&url, cookie) {
            Ok(v) => v,
            Err(e) if page == 0 => {
                explore_err = Some(e);
                break;
            }
            Err(_) => break,
        };
        let decoded = parse_sorts_page(&v);
        for sort in decoded.sorts {
            if !out
                .iter()
                .any(|x: &Sort| x.title.eq_ignore_ascii_case(&sort.title))
            {
                out.push(sort);
            }
        }
        next_token = decoded.next;
        if next_token.is_none() {
            break;
        }
    }

    if let Some(c) = cookie {
        if let Ok(omni) = fetch_omni_home(sid, c) {
            let mut extra_topics = Vec::new();
            let mut needed_ids = Vec::new();
            let mut seen_ids = HashSet::new();
            for (title, unis) in omni.topics {
                if out
                    .iter()
                    .any(|x: &Sort| x.title.eq_ignore_ascii_case(&title))
                    || extra_topics
                        .iter()
                        .any(|(t, _): &(String, Vec<u64>)| t.eq_ignore_ascii_case(&title))
                {
                    continue;
                }
                for &uid in &unis {
                    if seen_ids.insert(uid) {
                        needed_ids.push(uid);
                    }
                }
                extra_topics.push((title, unis));
            }
            if !needed_ids.is_empty() {
                let by_uid: HashMap<u64, Game> = games_by_universe(&needed_ids)
                    .into_iter()
                    .map(|g| (g.universe_id, g))
                    .collect();
                for (title, unis) in extra_topics {
                    let games: Vec<Game> = unis
                        .into_iter()
                        .filter_map(|u| by_uid.get(&u).cloned())
                        .collect();
                    if !games.is_empty() {
                        out.push(Sort { title, games });
                    }
                }
            }
        }
    }

    if out.is_empty() {
        if let Some(e) = explore_err {
            return Err(e);
        }
    }
    Ok(SortsPage {
        sorts: out,
        next: next_token,
    })
}

/// One more page of the explore cursor, for the charts feed's prefetch.
///
/// A `429` is handed back as [`ChartError::rate_limited`] with the server's own wait, so the
/// page can show a quiet notice and a Retry link instead of retrying in a loop.
pub fn fetch_sorts_more(
    sid: &str,
    cookie: Option<&str>,
    token: &str,
) -> Result<SortsPage, ChartError> {
    let url = format!(
        "https://apis.roblox.com/explore-api/v1/get-sorts?sessionId={sid}&device=computer&country=all&sortsPageToken={}",
        enc(token)
    );
    let mut req = agent().get(&url);
    if let Some(c) = cookie {
        req = req.set("Cookie", &format!(".ROBLOSECURITY={c}"));
    }
    let v = match req.call() {
        Ok(resp) => resp
            .into_json::<Value>()
            .map_err(|e| ChartError::other(e.to_string()))?,
        Err(ureq::Error::Status(429, resp)) => {
            return Err(ChartError::rate_limited(retry_after_secs(&resp)))
        }
        Err(ureq::Error::Status(code, _)) => return Err(ChartError::other(format!("HTTP {code}"))),
        Err(e) => return Err(ChartError::other(e.to_string())),
    };
    Ok(parse_sorts_page(&v))
}

/// The server's own wait, in seconds, from a throttled response.
fn retry_after_secs(resp: &ureq::Response) -> u64 {
    resp.header("Retry-After")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .or_else(|| {
            resp.header("x-ratelimit-reset")
                .and_then(|v| v.trim().parse::<u64>().ok())
        })
        .unwrap_or(5)
}

pub fn search_games(q: &str, sid: &str) -> Result<Vec<Game>, String> {
    let v = get_json(&format!(
        "https://apis.roblox.com/search-api/omni-search?searchQuery={}&sessionId={sid}&pageType=all",
        enc(q)
    ))?;
    let mut out = Vec::new();
    for group in v["searchResults"].as_array().into_iter().flatten() {
        if group["contentGroupType"].as_str() != Some("Game") {
            continue;
        }
        out.extend(
            group["contents"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(game_from_explore),
        );
    }
    Ok(out)
}

pub fn place_universe_cache() -> &'static Mutex<HashMap<u64, u64>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn remember_place_universe(place_id: u64, universe_id: u64) {
    if place_id > 0 && universe_id > 0 {
        if let Ok(mut map) = place_universe_cache().lock() {
            map.insert(place_id, universe_id);
        }
    }
}

pub fn cached_universe_of_place(place_id: u64) -> Option<u64> {
    place_universe_cache()
        .lock()
        .ok()
        .and_then(|map| map.get(&place_id).copied())
}

pub fn games_by_universe(ids: &[u64]) -> Vec<Game> {
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for &id in ids {
        if id > 0 && seen.insert(id) {
            unique.push(id);
        }
    }
    let mut by_id: HashMap<u64, Game> = HashMap::new();
    // Every chunk's details + votes requests are independent: fetch them all at once.
    let fetched: Vec<(Option<Value>, Option<Value>)> = thread::scope(|s| {
        let jobs: Vec<_> = unique
            .chunks(50)
            .map(|chunk| {
                let list = join(&chunk);
                let list2 = list.clone();
                let games = s.spawn(move || {
                    get_json(&format!(
                        "https://games.roblox.com/v1/games?universeIds={list}"
                    ))
                    .ok()
                });
                let votes = s.spawn(move || {
                    get_json(&format!(
                        "https://games.roblox.com/v1/games/votes?universeIds={list2}"
                    ))
                    .ok()
                });
                (games, votes)
            })
            .collect();
        jobs.into_iter()
            .map(|(g, v)| (g.join().ok().flatten(), v.join().ok().flatten()))
            .collect()
    });
    for (games_json, votes_json) in fetched {
        if let Some(v) = games_json {
            for g in v["data"].as_array().into_iter().flatten() {
                let Some(id) = g["id"].as_u64() else { continue };
                let place_id = g["rootPlaceId"].as_u64().unwrap_or(0);
                remember_place_universe(place_id, id);
                let mut game = Game {
                    universe_id: id,
                    place_id,
                    name: str_of(&g["name"]),
                    creator: str_of(&g["creator"]["name"]),
                    creator_verified: g["creator"]["hasVerifiedBadge"]
                        .as_bool()
                        .unwrap_or(false),
                    playing: g["playing"].as_u64().unwrap_or(0),
                    up: 0,
                    down: 0,
                    description: str_of(&g["description"]),
                    ..Game::default()
                };
                // Votes arrive in a second response; the label is refreshed after they do.
                crate::chart_feed::fill_labels(&mut game);
                by_id.insert(id, game);
            }
        }
        if let Some(v) = votes_json {
            for x in v["data"].as_array().into_iter().flatten() {
                if let Some(game) = x["id"].as_u64().and_then(|id| by_id.get_mut(&id)) {
                    game.up = x["upVotes"].as_u64().unwrap_or(0);
                    game.down = x["downVotes"].as_u64().unwrap_or(0);
                }
            }
        }
    }
    ids.iter().filter_map(|id| by_id.get(id).cloned()).collect()
}

pub fn universe_of_place(place: u64) -> Option<u64> {
    if place == 0 {
        return None;
    }
    if let Some(uid) = cached_universe_of_place(place) {
        return Some(uid);
    }
    let uid = get_json(&format!(
        "https://apis.roblox.com/universes/v1/places/{place}/universe"
    ))
    .ok()?["universeId"]
        .as_u64()?;
    remember_place_universe(place, uid);
    Some(uid)
}

pub fn games_by_places(places: &[u64]) -> Vec<Game> {
    let unis: Vec<u64> = places
        .iter()
        .filter_map(|p| universe_of_place(*p))
        .collect();
    games_by_universe(&unis)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedGameLink {
    pub place_id: Option<u64>,
    pub private_server_code: Option<String>,
    pub share_code: Option<String>,
    pub share_type: Option<String>,
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extract_query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            Some((
                url_decode(k).trim().to_ascii_lowercase(),
                url_decode(v).trim().to_string(),
            ))
        })
        .collect()
}

fn trim_link_wrappers(raw: &str) -> &str {
    let mut s = raw.trim();
    loop {
        let next = s
            .strip_prefix('<')
            .and_then(|v| v.strip_suffix('>'))
            .or_else(|| s.strip_prefix('"').and_then(|v| v.strip_suffix('"')))
            .or_else(|| s.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')));
        match next {
            Some(inner) => s = inner.trim(),
            None => return s,
        }
    }
}

fn is_supported_roblox_link(s: &str) -> bool {
    if s.is_empty() || s.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("roblox://") || lower.starts_with("roblox-player:") {
        return true;
    }
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let host = without_scheme.split(['/', '?', '#']).next().unwrap_or("");
    host == "roblox.com"
        || host.ends_with(".roblox.com")
        || host == "ro.blox.com"
        || host.ends_with(".ro.blox.com")
}

pub fn parse_game_link(input: &str) -> Option<ParsedGameLink> {
    let cleaned = trim_link_wrappers(input);
    if cleaned.is_empty() {
        return None;
    }
    if let Ok(place_id) = cleaned.parse::<u64>() {
        return (place_id > 0).then(|| ParsedGameLink {
            place_id: Some(place_id),
            ..Default::default()
        });
    }
    if !is_supported_roblox_link(cleaned) {
        return None;
    }

    let lower = cleaned.to_ascii_lowercase();
    if lower.starts_with("roblox-player:") {
        for part in cleaned.split('+') {
            if let Some((k, v)) = part.split_once(':') {
                if k.eq_ignore_ascii_case("placelauncherurl") {
                    let decoded = url_decode(v);
                    if let Some(parsed) = parse_game_link(&decoded) {
                        return Some(parsed);
                    }
                }
            }
        }
        return None;
    }

    let before_frag = cleaned.split('#').next().unwrap_or(cleaned);
    let (path_part, query_opt) = before_frag
        .split_once('?')
        .map_or((before_frag, None), |(p, q)| (p, Some(q)));

    let mut out = ParsedGameLink::default();
    let mut query_sources: Vec<&str> = Vec::new();
    let mut is_share_path = false;

    if let Some(after_scheme) = lower.strip_prefix("roblox://") {
        let orig_after = &path_part[cleaned.len().min("roblox://".len())..];
        if !orig_after.contains('/') && orig_after.contains('=') {
            query_sources.push(orig_after);
        } else if after_scheme == "share" || after_scheme.ends_with("/share") {
            is_share_path = true;
        }
    } else {
        let after_scheme = path_part
            .strip_prefix("https://")
            .or_else(|| path_part.strip_prefix("http://"))
            .or_else(|| path_part.strip_prefix("HTTPS://"))
            .or_else(|| path_part.strip_prefix("HTTP://"))
            .unwrap_or(path_part);
        if let Some((_, rel_path)) = after_scheme.split_once('/') {
            let segs: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
            for (idx, seg) in segs.iter().enumerate() {
                if seg.eq_ignore_ascii_case("games") {
                    if let Some(id) = segs
                        .get(idx + 1)
                        .and_then(|s| s.parse::<u64>().ok())
                        .filter(|&id| id > 0)
                    {
                        out.place_id = Some(id);
                    }
                } else if seg.eq_ignore_ascii_case("share")
                    || seg.eq_ignore_ascii_case("share-links")
                {
                    is_share_path = true;
                }
            }
        }
    }

    if let Some(q) = query_opt {
        query_sources.push(q);
    }

    for q in query_sources {
        for (k, v) in extract_query_pairs(q) {
            if v.is_empty() {
                continue;
            }
            match k.as_str() {
                "placeid" => {
                    if let Ok(id) = v.parse::<u64>() {
                        if id > 0 {
                            out.place_id = Some(id);
                        }
                    }
                }
                "privateserverlinkcode" | "linkcode" => {
                    out.private_server_code = Some(v);
                }
                "code" if is_share_path => {
                    out.share_code = Some(v);
                }
                "type" if is_share_path => {
                    out.share_type = Some(v);
                }
                "af_dp" | "af_web_dp" => {
                    if out.place_id.is_none() {
                        if let Some(nested) = parse_game_link(&v) {
                            out.place_id = nested.place_id;
                            if out.private_server_code.is_none() {
                                out.private_server_code = nested.private_server_code;
                            }
                            if out.share_code.is_none() {
                                out.share_code = nested.share_code;
                                out.share_type = nested.share_type;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if out.place_id.is_some() || out.share_code.is_some() {
        Some(out)
    } else {
        None
    }
}

fn extract_share_link_target(v: &Value) -> Option<(u64, Option<u64>, Option<String>)> {
    for key in [
        "privateServerInviteData",
        "experienceInviteData",
        "experienceDetailsInviteData",
        "notificationExperienceInviteData",
    ] {
        let data = &v[key];
        if data.is_null() {
            continue;
        }
        let place_id = data["placeId"].as_u64().filter(|&id| id > 0).unwrap_or(0);
        let universe_id = data["universeId"].as_u64().filter(|&id| id > 0);
        let link_code = data["linkCode"]
            .as_str()
            .or_else(|| data["privateServerLinkCode"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if place_id > 0 || universe_id.is_some() {
            return Some((place_id, universe_id, link_code));
        }
    }
    None
}

fn resolve_share_link(
    code: &str,
    share_type: Option<&str>,
    cookie: Option<&str>,
) -> Result<(u64, Option<u64>, Option<String>), String> {
    let mut candidate_types: Vec<&str> = Vec::new();
    if let Some(t) = share_type.map(str::trim).filter(|s| !s.is_empty()) {
        candidate_types.push(t);
    }
    for fallback in ["Server", "ExperienceInvite", "ExperienceDetails"] {
        if !candidate_types
            .iter()
            .any(|t| t.eq_ignore_ascii_case(fallback))
        {
            candidate_types.push(fallback);
        }
    }

    for link_type in candidate_types {
        if let Ok(resp) = post_csrf(
            "https://apis.roblox.com/sharelinks/v1/resolve-link",
            serde_json::json!({
                "linkId": code,
                "linkType": link_type,
            }),
            cookie,
            Some("https://www.roblox.com/"),
        ) {
            if let Ok(v) = resp.into_json::<Value>() {
                if let Some(target) = extract_share_link_target(&v) {
                    return Ok(target);
                }
            }
        }
    }

    Err(
        "Could not resolve that Roblox share link. Try pasting the full roblox.com/games/<id>/... URL or Place ID."
            .to_string(),
    )
}

pub fn resolve_game_link(input: &str, cookie: Option<&str>) -> Result<ResolvedGameLink, String> {
    let parsed = parse_game_link(input).ok_or_else(|| {
        "Enter a valid Roblox game URL or numeric Place ID (e.g. https://www.roblox.com/games/1818/...)."
            .to_string()
    })?;

    let mut place_id = parsed.place_id.unwrap_or(0);
    let mut universe_id = None;
    let mut private_server_code = parsed.private_server_code;

    if place_id == 0 {
        if let Some(code) = parsed.share_code.as_deref() {
            let (pid, uid, scode) = resolve_share_link(code, parsed.share_type.as_deref(), cookie)?;
            place_id = pid;
            universe_id = uid;
            if private_server_code.is_none() {
                private_server_code = scode;
            }
        }
    }

    let game = if place_id > 0 {
        games_by_places(&[place_id])
            .into_iter()
            .next()
            .or_else(|| universe_id.and_then(|uid| games_by_universe(&[uid]).into_iter().next()))
    } else {
        universe_id.and_then(|uid| games_by_universe(&[uid]).into_iter().next())
    }
    .ok_or_else(|| "No Roblox game found for that URL or Place ID.".to_string())?;

    let target_place_id = if place_id > 0 {
        place_id
    } else {
        game.place_id
    };
    Ok(ResolvedGameLink {
        game,
        place_id: target_place_id,
        private_server_code,
    })
}

pub fn resolve_game_and_icon_by_place(place_id: u64) -> Option<(Game, Option<String>)> {
    if place_id == 0 {
        return None;
    }
    let game = games_by_places(&[place_id]).into_iter().next()?;
    let icon_url = get_json(&format!(
        "https://thumbnails.roblox.com/v1/games/icons?universeIds={}&returnPolicy=PlaceHolder&size=256x256&format=Png&isCircular=false",
        game.universe_id
    ))
    .ok()
    .and_then(|v| {
        v["data"]
            .as_array()?
            .first()
            .and_then(|item| item["imageUrl"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    Some((game, icon_url))
}

pub fn fetch_user_favorite_games(uid: u64, cookie: &str) -> Result<Vec<u64>, String> {
    let v = auth_get_json(
        &format!("https://games.roblox.com/v2/users/{uid}/favorite/games?sortOrder=Desc&limit=25"),
        Some(cookie),
    )?;
    let mut unis = Vec::new();
    let mut seen = HashSet::new();
    for item in v["data"].as_array().into_iter().flatten() {
        if let Some(id) = item["id"].as_u64() {
            if let Some(pid) = item["rootPlace"]["id"].as_u64() {
                remember_place_universe(pid, id);
            }
            if id > 0 && seen.insert(id) {
                unis.push(id);
            }
        }
    }
    Ok(unis)
}

pub fn fetch_saved_synced(
    cookie: &str,
    uid_opt: Option<u64>,
    local_recent_places: Vec<u64>,
    local_fav_places: Vec<u64>,
) -> SavedSyncData {
    let uid = uid_opt.or_else(|| {
        auth_get_json(
            "https://users.roblox.com/v1/users/authenticated",
            Some(cookie),
        )
        .ok()
        .and_then(|v| v["id"].as_u64())
    });

    let (omni_continue, remote_fav_unis) = thread::scope(|s| {
        let h_omni = s.spawn(|| {
            fetch_omni_home(&new_session_id(), cookie)
                .ok()
                .and_then(|o| o.continue_unis)
        });
        let favs = uid.and_then(|id| fetch_user_favorite_games(id, cookie).ok());
        (h_omni.join().ok().flatten(), favs)
    });

    let local_recent_unis: Vec<u64> = if omni_continue.as_ref().is_some_and(|v| !v.is_empty()) {
        local_recent_places
            .iter()
            .take(8)
            .filter_map(|p| universe_of_place(*p))
            .collect()
    } else {
        local_recent_places
            .iter()
            .filter_map(|p| universe_of_place(*p))
            .collect()
    };

    let local_fav_unis: Vec<u64> = if remote_fav_unis.is_none() {
        local_fav_places
            .iter()
            .filter_map(|p| universe_of_place(*p))
            .collect()
    } else {
        Vec::new()
    };

    let mut all_unis = Vec::new();
    let mut seen = HashSet::new();
    for &u in omni_continue
        .iter()
        .flatten()
        .chain(remote_fav_unis.iter().flatten())
        .chain(local_recent_unis.iter())
        .chain(local_fav_unis.iter())
    {
        if u > 0 && seen.insert(u) {
            all_unis.push(u);
        }
    }

    let by_uid: HashMap<u64, Game> = if all_unis.is_empty() {
        HashMap::new()
    } else {
        games_by_universe(&all_unis)
            .into_iter()
            .map(|g| (g.universe_id, g))
            .collect()
    };

    let resolve_list = |unis: &[u64]| -> Option<Vec<Game>> {
        let resolved: Vec<Game> = unis.iter().filter_map(|u| by_uid.get(u).cloned()).collect();
        if !unis.is_empty() && resolved.is_empty() {
            None
        } else {
            Some(resolved)
        }
    };

    let remote_recent = omni_continue.as_deref().and_then(resolve_list);
    let fallback_recent = if local_recent_unis.is_empty() {
        None
    } else {
        resolve_list(&local_recent_unis)
    };
    let remote_favs = remote_fav_unis.as_deref().and_then(resolve_list);
    let fallback_favs = if remote_fav_unis.is_none() && !local_fav_unis.is_empty() {
        resolve_list(&local_fav_unis)
    } else {
        None
    };

    SavedSyncData {
        remote_recent,
        fallback_recent,
        remote_favs,
        fallback_favs,
    }
}

pub fn fetch_game_extra(universe_id: u64, cookie: Option<&str>) -> GameExtra {
    let mut ex = GameExtra {
        universe_id,
        verified_creator: false,
        favorites: 0,
        visits: 0,
        max_players: 0,
        created: "--".to_string(),
        updated: "--".to_string(),
        genre: "All".to_string(),
        subgenre: String::new(),
        maturity_short: String::new(),
        maturity_detail: String::new(),
        voice_chat: None,
        camera: None,
        is_favorited_api: None,
        can_vote: false,
        user_vote: None,
        media: Vec::new(),
        preview_video_id: None,
        passes: Vec::new(),
        passes_loaded: false,
        servers: Vec::new(),
        servers_cursor: None,
        servers_loaded: false,
    };

    fn joined(r: std::thread::Result<Result<Value, String>>) -> Result<Value, String> {
        r.unwrap_or_else(|_| Err("request thread failed".to_string()))
    }
    let (games_res, age_res, voice_res, votes_res, media_res) = thread::scope(|s| {
        let h_games = s.spawn(|| {
            auth_get_json(
                &format!("https://games.roblox.com/v1/games?universeIds={universe_id}"),
                cookie,
            )
        });
        let h_age = s.spawn(|| {
            post_json(
                "https://apis.roblox.com/experience-guidelines-api/experience-guidelines/get-age-recommendation",
                serde_json::json!({ "universeId": universe_id }),
            )
        });
        let h_voice = s.spawn(|| {
            cookie.map(|c| {
                auth_get_json(
                    &format!("https://voice.roblox.com/v1/settings/universe/{universe_id}"),
                    Some(c),
                )
            })
        });
        let h_votes = s.spawn(|| {
            cookie.map(|c| {
                auth_get_json(
                    &format!("https://games.roblox.com/v1/games/{universe_id}/votes/user"),
                    Some(c),
                )
            })
        });
        let h_media = s.spawn(|| {
            get_json(&format!(
                "https://games.roblox.com/v2/games/{universe_id}/media"
            ))
        });
        (
            joined(h_games.join()),
            joined(h_age.join()),
            h_voice.join().ok().flatten(),
            h_votes.join().ok().flatten(),
            joined(h_media.join()),
        )
    });

    if let Ok(v) = games_res {
        let g = &v["data"][0];
        if g.is_object() {
            ex.verified_creator = g["creator"]["hasVerifiedBadge"].as_bool().unwrap_or(false);
            ex.favorites = g["favoritedCount"].as_u64().unwrap_or(0);
            ex.visits = g["visits"].as_u64().unwrap_or(0);
            ex.max_players = g["maxPlayers"].as_u64().unwrap_or(0);
            ex.created = format_iso_date(&str_of(&g["created"]));
            ex.updated = format_iso_date(&str_of(&g["updated"]));
            let gl1 = str_of(&g["genre_l1"]);
            let gl2 = str_of(&g["genre_l2"]);
            let fallback_g = str_of(&g["genre"]);
            ex.genre = if !gl1.is_empty() {
                gl1
            } else if !fallback_g.is_empty() {
                fallback_g
            } else {
                "All".to_string()
            };
            ex.subgenre = gl2;
            ex.is_favorited_api = g["isFavoritedByUser"].as_bool();
        }
    }

    if let Ok(v) = age_res {
        let age = &v["ageRecommendationDetails"]["summary"]["ageRecommendation"];
        let short = str_of(&age["displayNameWithHeaderShort"]);
        let name = str_of(&age["displayName"]);
        ex.maturity_short = if !short.is_empty() {
            short
        } else if !name.is_empty() {
            format!("Maturity: {name}")
        } else {
            String::new()
        };
        let descriptors: Vec<String> = v["ageRecommendationDetails"]["descriptorUsages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["contains"].as_bool() == Some(true))
            .filter_map(|d| {
                let s = str_of(&d["descriptor"]["displayName"]);
                (!s.is_empty()).then_some(s)
            })
            .collect();
        ex.maturity_detail = descriptors.join(", ");
    }

    if let Some(Ok(v)) = voice_res {
        ex.voice_chat = v["isUniverseEnabledForVoice"].as_bool();
        ex.camera = v["isUniverseEnabledForCamera"].as_bool();
    }
    if let Some(Ok(v)) = votes_res {
        ex.can_vote = v["canVote"].as_bool().unwrap_or(false);
        ex.user_vote = v["userVote"].as_bool();
    }

    if let Ok(v) = media_res {
        for m in v["data"].as_array().into_iter().flatten() {
            if m["approved"].as_bool() != Some(false) {
                let is_video = m["assetTypeId"].as_u64() == Some(86)
                    || m["assetType"].as_str() == Some("GamePreviewVideo");
                if is_video && ex.preview_video_id.is_none() {
                    ex.preview_video_id = m["videoId"]
                        .as_u64()
                        .or_else(|| m["videoId"].as_str().and_then(|s| s.parse::<u64>().ok()));
                }
            }
        }
        let img_ids: Vec<u64> = v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["approved"].as_bool() != Some(false))
            .filter_map(|m| m["imageId"].as_u64())
            .take(8)
            .collect();
        if !img_ids.is_empty() {
            if let Ok(t) = get_json(&format!(
                "https://thumbnails.roblox.com/v1/assets?assetIds={}&size=480x270&format=Png&isCircular=false",
                join(&img_ids)
            )) {
                let by_id: HashMap<u64, String> = t["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| {
                        let id = d["targetId"].as_u64()?;
                        let url = str_of(&d["imageUrl"]);
                        (!url.is_empty()).then_some((id, url))
                    })
                    .collect();
                for id in img_ids {
                    if let Some(url) = by_id.get(&id) {
                        ex.media.push(url.clone());
                    }
                }
            }
        }
    }

    ex
}

pub fn fetch_game_preview_video_id(universe_id: u64) -> Option<u64> {
    let v = get_json(&format!(
        "https://games.roblox.com/v2/games/{universe_id}/media"
    ))
    .ok()?;
    for m in v["data"].as_array().into_iter().flatten() {
        if m["approved"].as_bool() != Some(false) {
            let is_video = m["assetTypeId"].as_u64() == Some(86)
                || m["assetType"].as_str() == Some("GamePreviewVideo");
            if is_video {
                if let Some(vid) = m["videoId"]
                    .as_u64()
                    .or_else(|| m["videoId"].as_str().and_then(|s| s.parse::<u64>().ok()))
                {
                    if vid > 0 {
                        return Some(vid);
                    }
                }
            }
        }
    }
    None
}

pub fn fetch_and_decode_preview_video(
    video_id: u64,
    max_width: usize,
    cookie: Option<&str>,
) -> Result<video_preview::DecodedPreviewVideo, String> {
    // 1. Try v2/assetId endpoint which provides the signed CDN location URL
    let meta_url = format!("https://assetdelivery.roblox.com/v2/assetId/{video_id}");
    let direct_url = if let Ok(v2_json) = auth_get_json(&meta_url, cookie) {
        v2_json["locations"][0]["location"]
            .as_str()
            .map(str::to_string)
    } else {
        None
    };

    let target_url = direct_url
        .unwrap_or_else(|| format!("https://assetdelivery.roblox.com/v1/asset/?id={video_id}"));

    let resp = agent()
        .get(&target_url)
        .call()
        .map_err(|e| format!("Download video error: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Read video error: {e}"))?;
    video_preview::decode_webm_vp8(&bytes, max_width)
}

pub fn fetch_game_passes(
    universe_id: u64,
    cookie: Option<&str>,
) -> Result<Vec<GamePassItem>, String> {
    let v = auth_get_json(
        &format!(
            "https://apis.roblox.com/game-passes/v1/universes/{universe_id}/game-passes?passView=Full&pageSize=50"
        ),
        cookie,
    )?;
    let mut icon_ids = Vec::new();
    let mut raw: Vec<(GamePassItem, u64)> = v["gamePasses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|gp| {
            let id = gp["id"].as_u64()?;
            let name = {
                let dn = str_of(&gp["displayName"]);
                if dn.is_empty() {
                    str_of(&gp["name"])
                } else {
                    dn
                }
            };
            let description = str_of(&gp["displayDescription"]);
            let price = gp["price"].as_u64();
            let is_for_sale = gp["isForSale"].as_bool().unwrap_or(false);
            let is_owned = gp["isOwned"].as_bool().unwrap_or(false);
            let icon_id = gp["displayIconImageAssetId"].as_u64().unwrap_or(0);
            if icon_id > 0 {
                icon_ids.push(icon_id);
            }
            Some((
                GamePassItem {
                    id,
                    name,
                    description,
                    price,
                    is_for_sale,
                    is_owned,
                    thumb: String::new(),
                },
                icon_id,
            ))
        })
        .collect();
    let thumbs = fetch_asset_thumbs(&icon_ids);
    let mut passes = Vec::with_capacity(raw.len());
    for (mut item, icon_id) in raw.drain(..) {
        if let Some(url) = thumbs.get(&icon_id) {
            item.thumb = url.clone();
        }
        passes.push(item);
    }
    Ok(passes)
}

pub fn fetch_game_servers(
    place_id: u64,
    cursor: Option<&str>,
    cookie: Option<&str>,
) -> Result<(Vec<ServerItem>, Option<String>), String> {
    let mut url = format!(
        "https://games.roblox.com/v1/games/{place_id}/servers/Public?sortOrder=Desc&limit=25"
    );
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&enc(c));
    }
    let v = auth_get_json(&url, cookie)?;
    let next = v["nextPageCursor"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let servers: Vec<ServerItem> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_str()?.to_string();
            Some(ServerItem {
                id,
                playing: s["playing"].as_u64().unwrap_or(0),
                max_players: s["maxPlayers"].as_u64().unwrap_or(0),
                ping: s["ping"].as_u64(),
                fps: s["fps"].as_f64(),
            })
        })
        .collect();
    Ok((servers, next))
}

/// The public-server pool for one place, several pages deep.
///
/// `sort_order` is the API's own value - `"Asc"` (fewest players first) or `"Desc"` - and
/// the caller picks it, because the hop modes rank differently and the panel keeps its own
/// ordering. Servers are not filtered here: the caller knows the job id it must avoid.
pub fn fetch_public_server_pool(
    place_id: u64,
    sort_order: &str,
    pages: usize,
    cookie: Option<&str>,
) -> Result<Vec<ServerItem>, String> {
    let order = if sort_order.eq_ignore_ascii_case("asc") {
        "Asc"
    } else {
        "Desc"
    };
    let mut out: Vec<ServerItem> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..pages.clamp(1, 3) {
        let (mut servers, next) =
            fetch_server_page_in_order(place_id, order, cursor.as_deref(), 100, cookie)?;
        out.append(&mut servers);
        cursor = next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
}

/// One page of a place's public room list, with the cursor for its next page.
///
/// `limit` is clamped to the API's maximum of 100. The ping fallback reads page one first -
/// the server the player is in is almost always there - and asks for a second page only when
/// the first carried a cursor and had no match, which keeps the request rate low enough for
/// Roblox's limiter.
pub fn fetch_server_page(
    place_id: u64,
    cursor: Option<&str>,
    limit: usize,
    cookie: Option<&str>,
) -> Result<(Vec<ServerItem>, Option<String>), String> {
    fetch_server_page_in_order(place_id, "Desc", cursor, limit, cookie)
}

fn fetch_server_page_in_order(
    place_id: u64,
    order: &str,
    cursor: Option<&str>,
    limit: usize,
    cookie: Option<&str>,
) -> Result<(Vec<ServerItem>, Option<String>), String> {
    let mut url = format!(
        "https://games.roblox.com/v1/games/{place_id}/servers/Public?sortOrder={order}&limit={}",
        limit.clamp(1, 100)
    );
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&enc(c));
    }
    let v = auth_get_json(&url, cookie)?;
    let mut out: Vec<ServerItem> = Vec::new();
    for s in v["data"].as_array().into_iter().flatten() {
        let Some(id) = s["id"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(ServerItem {
            id: id.to_string(),
            playing: s["playing"].as_u64().unwrap_or(0),
            max_players: s["maxPlayers"].as_u64().unwrap_or(0),
            ping: s["ping"].as_u64(),
            fps: s["fps"].as_f64(),
        });
    }
    let next = v["nextPageCursor"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok((out, next))
}

pub use crate::party_server::{PartyServer, PartyServerError};

/// Pick one public server with room for the whole party.
///
/// Thin wrapper over [`crate::party_server::select_for_party`] that supplies the real public
/// server listing: page one first, then the wider sweep when page one has nothing.
pub fn find_server_for_party(
    place_id: u64,
    party_size: usize,
    cookie: Option<&str>,
) -> Result<PartyServer, PartyServerError> {
    crate::party_server::select_for_party(party_size, |pages| {
        fetch_public_server_pool(place_id, "Desc", pages, cookie)
    })
}

/// Whether the server a party is heading to still has a free seat for `needed` members.
///
/// Reads the place's public room list, which is the only published source that reports
/// how full a specific job is. Two pages cover the busy end where party hosts land.
pub fn party_slot_state(
    place_id: u64,
    job_id: &str,
    needed: usize,
    cookie: Option<&str>,
) -> crate::party_server::SlotState {
    match fetch_public_server_pool(place_id, "Desc", 2, cookie) {
        Ok(servers) => crate::party_server::slot_state(&servers, job_id, needed),
        Err(_) => crate::party_server::SlotState::Unknown,
    }
}

pub fn vote_game(universe_id: u64, vote: Option<bool>, cookie: &str) -> Result<(), String> {
    patch_csrf(
        &format!("https://games.roblox.com/v1/games/{universe_id}/user-votes"),
        serde_json::json!({ "vote": vote }),
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    Ok(())
}

pub fn favorite_game_api(universe_id: u64, is_fav: bool, cookie: &str) -> Result<(), String> {
    post_csrf(
        &format!("https://games.roblox.com/v1/games/{universe_id}/favorites"),
        serde_json::json!({ "isFavorited": is_fav }),
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    Ok(())
}

pub fn fetch_thumbs(ids: Vec<u64>) -> Msg {
    let mut wide = HashMap::new();
    let mut icon = HashMap::new();
    // Wide thumbnails and icons for every chunk of MAX_BATCH ids, all requested at once.
    // 100 ids per call is the endpoint's limit: a scroll that reveals a full grid costs a
    // couple of requests, not one per card.
    let results: Vec<(Option<Value>, Option<Value>)> = thread::scope(|s| {
        let jobs: Vec<_> = crate::thumb_pipeline::chunks(&ids, crate::thumb_pipeline::MAX_BATCH)
            .into_iter()
            .map(|chunk| {
                let list = join(&chunk);
                let list2 = list.clone();
                let w = s.spawn(move || {
                    get_json(&format!(
                        "https://thumbnails.roblox.com/v1/games/multiget/thumbnails?universeIds={list}&countPerUniverse=1&defaults=true&size={}&format=Png&isCircular=false",
                        crate::thumb_pipeline::WIDE_REQUEST
                    ))
                    .ok()
                });
                let i = s.spawn(move || {
                    get_json(&format!(
                        "https://thumbnails.roblox.com/v1/games/icons?universeIds={list2}&returnPolicy=PlaceHolder&size={}&format=Png&isCircular=false",
                        crate::thumb_pipeline::ICON_REQUEST
                    ))
                    .ok()
                });
                (w, i)
            })
            .collect();
        jobs.into_iter()
            .map(|(w, i)| (w.join().ok().flatten(), i.join().ok().flatten()))
            .collect()
    });
    for (wide_json, icon_json) in results {
        if let Some(v) = wide_json {
            for d in v["data"].as_array().into_iter().flatten() {
                if let (Some(id), Some(url)) = (
                    d["universeId"].as_u64(),
                    d["thumbnails"][0]["imageUrl"].as_str(),
                ) {
                    wide.insert(id, url.to_string());
                }
            }
        }
        if let Some(v) = icon_json {
            for d in v["data"].as_array().into_iter().flatten() {
                if let (Some(id), Some(url)) = (d["targetId"].as_u64(), d["imageUrl"].as_str()) {
                    icon.insert(id, url.to_string());
                }
            }
        }
    }
    Msg::Thumbs { wide, icon }
}

/// Downloads and decodes a batch of already-resolved thumbnail URLs.
///
/// Called on a worker thread. It fans out over [`crate::thumb_pipeline::WORKERS`] scoped
/// threads that all share [`agent`], so the connection to `tr.rbxcdn.com` is reused (TLS
/// handshake once per connection, not once per image). Each image is decoded *and* cropped
/// to the card's exact pixel size here, so the UI thread only ever uploads ready RGBA - it
/// never decodes, and nothing is written to disk.
pub fn download_thumbnails(items: Vec<(String, [u32; 2])>) -> Vec<(String, egui::ColorImage)> {
    decode_images(items, true)
}

/// Downloads Game Details gallery pictures, scaled to the hero's pixel budget.
///
/// Unlike a card thumbnail these are not cropped to a fixed aspect: a gallery picture keeps
/// its own shape, exactly as egui's image loader used to paint it, so the hero does not start
/// framing images differently.
pub fn download_gallery_images(urls: Vec<String>) -> Vec<(String, egui::ColorImage)> {
    let target = crate::thumb_pipeline::GALLERY_TARGET;
    decode_images(urls.into_iter().map(|url| (url, target)).collect(), false)
}

/// Whether an encoded image's header says it is small enough to decode inside the budget.
///
/// Every image path in the app goes through this before calling into `image`: the RAM budget
/// counts bytes after a decode, so the decoder's own full-size allocation is the one place a
/// single picture could still dwarf the ceiling. A header that cannot be parsed returns `true`
/// and leaves the download byte cap as the only bound, which is what the app did before.
pub fn image_dimensions_fit(bytes: &[u8]) -> bool {
    let dimensions = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok());
    match dimensions {
        Some((width, height)) => crate::thumb_pipeline::decode_fits(width, height),
        None => true,
    }
}

fn decode_images(
    items: Vec<(String, [u32; 2])>,
    crop: bool,
) -> Vec<(String, egui::ColorImage)> {
    use std::io::Read;

    let download = |url: &str, target: [u32; 2]| -> Option<egui::ColorImage> {
        let mut bytes = Vec::new();
        agent()
            .get(url)
            .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
            .call()
            .ok()?
            .into_reader()
            // A body larger than any thumbnail we would draw is not worth reading into RAM.
            .take(crate::thumb_pipeline::MAX_IMAGE_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        // The decoder allocates w*h*4 before the resize below, so a picture that would blow
        // the whole RAM budget in one worker is refused here.
        if !image_dimensions_fit(&bytes) {
            crate::auth::log(&format!("thumb: refused an oversized decode for {url}"));
            return None;
        }
        let image = image::load_from_memory(&bytes).ok()?;
        // Downscale once, on the worker: the texture the UI uploads is never bigger than the
        // size it is drawn at. A card thumbnail fills its tile; a gallery picture is fitted.
        let image = if crop {
            image.resize_to_fill(
                target[0].max(1),
                target[1].max(1),
                image::imageops::FilterType::Triangle,
            )
        } else {
            image.resize(
                target[0].max(1),
                target[1].max(1),
                image::imageops::FilterType::Triangle,
            )
        };
        let rgba = image.to_rgba8();
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_flat_samples().as_slice(),
        ))
    };

    let mut out: Vec<(String, egui::ColorImage)> = Vec::new();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (url, target) in items {
            handles.push(s.spawn(move || {
                let image = download(&url, target);
                (url, image)
            }));
            if handles.len() >= crate::thumb_pipeline::WORKERS {
                for handle in handles.drain(..) {
                    if let Ok((url, Some(image))) = handle.join() {
                        out.push((url, image));
                    }
                }
            }
        }
        for handle in handles.drain(..) {
            if let Ok((url, Some(image))) = handle.join() {
                out.push((url, image));
            }
        }
    });
    out
}

/// The resolved friend names of this session, kept for the life of the process.
///
/// `/friends` now often answers ids only, and presence refreshes rebuild those users from ids.
/// Remembering `id -> (displayName, username)` once means a bubble never has to fall back to
/// "Player" twice.
fn name_book() -> &'static std::sync::Mutex<crate::friend_names::NameBook> {
    static BOOK: std::sync::OnceLock<std::sync::Mutex<crate::friend_names::NameBook>> =
        std::sync::OnceLock::new();
    BOOK.get_or_init(|| std::sync::Mutex::new(crate::friend_names::NameBook::default()))
}

fn friend_row(u: &User) -> crate::friend_names::FriendName {
    crate::friend_names::FriendName {
        id: u.id,
        display: u.display.clone(),
        name: u.name.clone(),
        verified: u.verified,
    }
}

/// Copies whatever a name row knows into a user, never overwriting a name with a blank.
fn apply_name(u: &mut User, row: &crate::friend_names::FriendName) {
    if u.name.is_empty() && !row.name.is_empty() {
        u.name = row.name.clone();
    }
    if u.display.is_empty() && !row.display.is_empty() {
        u.display = row.display.clone();
    }
    if row.verified {
        u.verified = true;
    }
}

/// `POST users.roblox.com/v1/users` for every id, 100 per request, merged into one payload.
fn fetch_users_payload(ids: &[u64], cookie: Option<&str>) -> Option<Value> {
    let chunks = crate::friend_names::batches(ids);
    if chunks.is_empty() {
        return None;
    }
    let mut data: Vec<Value> = Vec::new();
    for chunk in chunks {
        let Ok(resp) = post_csrf(
            "https://users.roblox.com/v1/users",
            serde_json::json!({ "userIds": chunk, "excludeBannedUsers": false }),
            cookie,
            None,
        ) else {
            continue;
        };
        let Ok(v) = resp.into_json::<Value>() else {
            continue;
        };
        data.extend(v["data"].as_array().cloned().into_iter().flatten());
    }
    if data.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "data": data }))
    }
}

/// One user row, before headshots and presence.
fn user_from_row(row: crate::friend_names::FriendName) -> User {
    User {
        id: row.id,
        name: row.name,
        display: row.display,
        headshot: String::new(),
        presence: 0,
        location: String::new(),
        place_id: None,
        verified: row.verified,
    }
}

/// The friends loader's name half: parse the rows (often ids only), ask `POST /v1/users` about
/// every id in one batch of up to 100, and let `friend_names::resolve` fill and remember them.
fn resolve_friend_rows(v: &Value, cookie: Option<&str>) -> Vec<crate::friend_names::FriendName> {
    let ids: Vec<u64> = crate::friend_names::parse_rows(v)
        .iter()
        .map(|row| row.id)
        .collect();
    let payload = fetch_users_payload(&ids, cookie);
    let mut book = name_book().lock().unwrap();
    crate::friend_names::resolve(v, payload.as_ref(), &mut book)
}

/// Fills display names and usernames for a batch of users.
///
/// The in-session book answers first, so a presence refresh gets its names back with no
/// request at all; only the users still nameless afterwards cost one batched call (100 ids per
/// request, authenticated so restricted accounts resolve too). Results are remembered.
pub fn fill_names(users: &mut [User], cookie: Option<&str>) {
    if users.is_empty() {
        return;
    }
    let mut rows: Vec<(usize, crate::friend_names::FriendName)> = users
        .iter()
        .enumerate()
        .map(|(i, u)| (i, friend_row(u)))
        .collect();
    {
        let book = name_book().lock().unwrap();
        for (_, row) in rows.iter_mut() {
            book.fill(row);
        }
    }
    let missing: Vec<u64> = rows
        .iter()
        .filter(|(_, row)| !row.is_named())
        .map(|(_, row)| row.id)
        .collect();
    if !missing.is_empty() {
        if let Some(payload) = fetch_users_payload(&missing, cookie) {
            let fresh = crate::friend_names::parse_rows(&payload);
            let mut book = name_book().lock().unwrap();
            for row in &fresh {
                book.remember(row);
            }
            for (_, row) in rows.iter_mut() {
                book.fill(row);
            }
        }
    }
    for (i, row) in rows {
        apply_name(&mut users[i], &row);
    }
}

pub fn enrich_users(users: &mut [User], cookie: Option<&str>) {
    if users.is_empty() {
        return;
    }
    // Names first: `/friends` can answer ids only, and a bubble must never settle on "Player".
    fill_names(users, cookie);
    for chunk in users.chunks_mut(50) {
        let ids: Vec<u64> = chunk.iter().map(|u| u.id).collect();
        if let Ok(h) = get_json(&format!(
            "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={}&size=150x150&format=Png&isCircular=false",
            join(&ids)
        )) {
            let map: HashMap<u64, String> = h["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| Some((d["targetId"].as_u64()?, str_of(&d["imageUrl"]))))
                .collect();
            for u in chunk.iter_mut() {
                u.headshot = map.get(&u.id).cloned().unwrap_or_default();
            }
        }
        if let Some(c) = cookie {
            if let Ok(resp) = post_csrf(
                "https://presence.roblox.com/v1/presence/users",
                serde_json::json!({ "userIds": ids }),
                Some(c),
                None,
            ) {
                if let Ok(p) = resp.into_json::<Value>() {
                    for item in p["userPresences"].as_array().into_iter().flatten() {
                        let Some(uid) = item["userId"].as_u64() else {
                            continue;
                        };
                        if let Some(u) = chunk.iter_mut().find(|u| u.id == uid) {
                            let kind = item["userPresenceType"].as_u64().unwrap_or(0) as u8;
                            u.presence = if kind == 4 { 0 } else { kind };
                            u.location = str_of(&item["lastLocation"]);
                            u.place_id = item["placeId"]
                                .as_u64()
                                .or_else(|| item["rootPlaceId"].as_u64());
                        }
                    }
                }
            }
        }
    }
}

/// Presence only, in one request. A profile already loads name, badge and a 420 px
/// headshot, so `enrich_users` there would spend two extra round trips on data that is
/// thrown away.
fn enrich_presence(user: &mut User, cookie: Option<&str>) {
    let Some(c) = cookie else { return };
    let Ok(resp) = post_csrf(
        "https://presence.roblox.com/v1/presence/users",
        serde_json::json!({ "userIds": [user.id] }),
        Some(c),
        None,
    ) else {
        return;
    };
    let Ok(p) = resp.into_json::<Value>() else {
        return;
    };
    let Some(item) = p["userPresences"].as_array().and_then(|a| a.first()) else {
        return;
    };
    let kind = item["userPresenceType"].as_u64().unwrap_or(0) as u8;
    user.presence = if kind == 4 { 0 } else { kind };
    user.location = str_of(&item["lastLocation"]);
    user.place_id = item["placeId"]
        .as_u64()
        .or_else(|| item["rootPlaceId"].as_u64());
}

/// Where one user is playing right now, for the party's presence fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyPresence {
    pub in_game: bool,
    pub place_id: u64,
    pub job_id: String,
    pub universe_id: u64,
    pub game_name: String,
}

/// One presence poll for a single user id. The party uses this when a direct peer-to-peer
/// socket cannot be opened; the cookie stays here, and nothing about it reaches another user.
pub fn party_presence(user_id: u64, cookie: &str) -> Option<PartyPresence> {
    let resp = post_csrf(
        "https://presence.roblox.com/v1/presence/users",
        serde_json::json!({ "userIds": [user_id] }),
        Some(cookie),
        None,
    )
    .ok()?;
    let value: Value = resp.into_json().ok()?;
    presence_of(&value)
}

/// Pure half of [`party_presence`]: maps Roblox's presence JSON, no network.
fn presence_of(value: &Value) -> Option<PartyPresence> {
    let item = value["userPresences"].as_array()?.first()?;
    let place_id = item["placeId"]
        .as_u64()
        .or_else(|| item["rootPlaceId"].as_u64())
        .unwrap_or(0);
    Some(PartyPresence {
        in_game: item["userPresenceType"].as_u64().unwrap_or(0) == 2,
        place_id,
        job_id: str_of(&item["gameId"]),
        universe_id: item["universeId"].as_u64().unwrap_or(0),
        game_name: str_of(&item["lastLocation"]),
    })
}

#[cfg(test)]
mod party_presence_tests {
    use super::*;

    #[test]
    fn a_game_session_carries_the_server_id() {
        let v = serde_json::json!({
            "userPresences": [{
                "userPresenceType": 2,
                "placeId": 920587237,
                "rootPlaceId": 920587237,
                "gameId": "job-abc",
                "universeId": 4242,
                "lastLocation": "Adopt Me!"
            }]
        });
        let spot = presence_of(&v).expect("a presence row");
        assert!(spot.in_game);
        assert_eq!(spot.place_id, 920_587_237);
        assert_eq!(spot.job_id, "job-abc");
        assert_eq!(spot.universe_id, 4242);
        assert_eq!(spot.game_name, "Adopt Me!");
    }

    #[test]
    fn being_online_is_not_being_in_a_game() {
        let v = serde_json::json!({
            "userPresences": [{ "userPresenceType": 1, "placeId": 0 }]
        });
        let spot = presence_of(&v).expect("a presence row");
        assert!(!spot.in_game);
        assert!(spot.job_id.is_empty());
        assert!(presence_of(&serde_json::json!({ "userPresences": [] })).is_none());
    }
}

pub fn fetch_users_by_ids(ids: &[u64], cookie: Option<&str>) -> Result<Vec<User>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut users = Vec::new();
    for chunk in ids.chunks(100) {
        let v = post_json(
            "https://users.roblox.com/v1/users",
            serde_json::json!({ "userIds": chunk, "excludeBannedUsers": true }),
        )?;
        for u in v["data"].as_array().into_iter().flatten() {
            if let Some(id) = u["id"].as_u64() {
                users.push(User {
                    id,
                    name: str_of(&u["name"]),
                    display: str_of(&u["displayName"]),
                    headshot: String::new(),
                    presence: 0,
                    location: String::new(),
                    place_id: None,
                    verified: u["hasVerifiedBadge"].as_bool().unwrap_or(false),
                });
            }
        }
    }
    enrich_users(&mut users, cookie);
    Ok(users)
}

/// Single user lookup by ID. One request, no search index, so it answers instantly.
#[allow(dead_code)]
pub fn lookup_user_by_id(id: u64) -> Result<Option<User>, String> {
    let v = match get_json(&format!("https://users.roblox.com/v1/users/{id}")) {
        Ok(v) => v,
        Err(e) => {
            if e.contains("404") || e.to_lowercase().contains("not found") {
                return Ok(None);
            }
            return Err(e);
        }
    };
    let name = str_of(&v["name"]);
    if name.is_empty() && v["id"].as_u64().is_none() {
        return Ok(None);
    }
    let disp = str_of(&v["displayName"]);
    let verified = v["hasVerifiedBadge"].as_bool() == Some(true);
    let mut users = vec![User {
        id,
        name: name.clone(),
        display: if disp.is_empty() { name } else { disp },
        headshot: String::new(),
        presence: 0,
        location: String::new(),
        place_id: None,
        verified,
    }];
    enrich_users(&mut users, None);
    Ok(users.into_iter().next())
}

pub fn search_users_api(
    query: &str,
    cursor: Option<&str>,
    sid: &str,
    cookie: Option<&str>,
) -> Result<UserSearchPage, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(UserSearchPage {
            users: Vec::new(),
            verified: HashSet::new(),
            next_cursor: None,
        });
    }

    let mut users: Vec<User> = Vec::new();
    let mut verified: HashSet<u64> = HashSet::new();
    let mut next_cursor: Option<String> = None;

    let mut omni_url = format!(
        "https://apis.roblox.com/search-api/omni-search?searchQuery={}&sessionId={}&verticalType=user",
        enc(q),
        enc(sid)
    );
    if let Some(tok) = cursor.filter(|s| !s.is_empty()) {
        omni_url.push_str("&pageToken=");
        omni_url.push_str(&enc(tok));
    }

    if let Ok(v) = auth_get_json(&omni_url, cookie) {
        for group in v["searchResults"].as_array().into_iter().flatten() {
            if group["contentGroupType"].as_str() != Some("User") {
                continue;
            }
            for item in group["contents"].as_array().into_iter().flatten() {
                let Some(id) = item["contentId"].as_u64().or_else(|| item["id"].as_u64()) else {
                    continue;
                };
                let name = str_of(&item["username"]);
                let disp = str_of(&item["displayName"]);
                let is_v = item["hasVerifiedBadge"].as_bool() == Some(true);
                if is_v {
                    verified.insert(id);
                }
                if !users.iter().any(|existing| existing.id == id) {
                    users.push(User {
                        id,
                        name: name.clone(),
                        display: if disp.is_empty() { name } else { disp },
                        headshot: String::new(),
                        presence: 0,
                        location: String::new(),
                        place_id: None,
                        verified: is_v,
                    });
                }
            }
        }
        next_cursor = v["nextPageToken"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);
    }

    if users.is_empty() && cursor.is_none() {
        let v1_url = format!(
            "https://users.roblox.com/v1/users/search?keyword={}&limit=25",
            enc(q)
        );
        let mut req = agent()
            .get(&v1_url)
            .set("Referer", "https://www.roblox.com/");
        if let Some(c) = cookie {
            req = req.set("Cookie", &format!(".ROBLOSECURITY={c}"));
        }
        let v: Value = req
            .call()
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())?;
        for u in v["data"].as_array().into_iter().flatten() {
            let Some(id) = u["id"].as_u64() else { continue };
            let name = str_of(&u["name"]);
            let disp = str_of(&u["displayName"]);
            let is_v = u["hasVerifiedBadge"].as_bool() == Some(true);
            if is_v {
                verified.insert(id);
            }
            if !users.iter().any(|existing| existing.id == id) {
                users.push(User {
                    id,
                    name: name.clone(),
                    display: if disp.is_empty() { name } else { disp },
                    headshot: String::new(),
                    presence: 0,
                    location: String::new(),
                    place_id: None,
                    verified: is_v,
                });
            }
        }
    }

    enrich_users(&mut users, cookie);
    Ok(UserSearchPage {
        users,
        verified,
        next_cursor,
    })
}

pub fn send_friend_request_api(user_id: u64, cookie: &str) -> Result<(), String> {
    post_csrf(
        &format!("https://friends.roblox.com/v1/users/{user_id}/request-friendship"),
        serde_json::json!({ "friendshipOriginSourceType": 0 }),
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    Ok(())
}

pub fn respond_friend_request_api(user_id: u64, accept: bool, cookie: &str) -> Result<(), String> {
    let endpoint = if accept {
        "accept-friend-request"
    } else {
        "decline-friend-request"
    };
    post_csrf(
        &format!("https://friends.roblox.com/v1/users/{user_id}/{endpoint}"),
        serde_json::json!({}),
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    Ok(())
}

fn parse_social_user_page(
    v: &Value,
    cookie: Option<&str>,
) -> Result<(Vec<User>, Option<String>), String> {
    let mut users: Vec<User> = Vec::new();
    let mut seen = HashSet::new();
    let mut missing_ids = Vec::new();

    for u in v["data"].as_array().into_iter().flatten() {
        let Some(id) = u["id"].as_u64() else { continue };
        if !seen.insert(id) {
            continue;
        }
        let name = str_of(&u["name"]);
        let disp = str_of(&u["displayName"]);
        if name.is_empty() {
            missing_ids.push(id);
        }
        users.push(User {
            id,
            name: name.clone(),
            display: if disp.is_empty() { name } else { disp },
            headshot: String::new(),
            presence: 0,
            location: String::new(),
            place_id: None,
            verified: u["hasVerifiedBadge"].as_bool().unwrap_or(false),
        });
    }

    if !missing_ids.is_empty() {
        if let Ok(resolved) = fetch_users_by_ids(&missing_ids, cookie) {
            let map: HashMap<u64, User> = resolved.into_iter().map(|u| (u.id, u)).collect();
            for u in &mut users {
                if let Some(full) = map.get(&u.id) {
                    *u = full.clone();
                }
            }
        }
    }

    enrich_users(&mut users, cookie);
    let next_cursor = v["nextPageCursor"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    Ok((users, next_cursor))
}

pub fn fetch_friend_requests_api(
    cursor: Option<&str>,
    cookie: &str,
) -> Result<(Vec<User>, Option<String>), String> {
    let mut url =
        "https://friends.roblox.com/v1/my/friends/requests?sortOrder=Desc&limit=100".to_string();
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&enc(c));
    }
    let v = auth_get_json(&url, Some(cookie))?;
    parse_social_user_page(&v, Some(cookie))
}

pub fn fetch_follow_list_api(
    user_id: u64,
    following: bool,
    cursor: Option<&str>,
    cookie: Option<&str>,
) -> Result<(Vec<User>, Option<String>), String> {
    let kind = if following { "followings" } else { "followers" };
    let mut url =
        format!("https://friends.roblox.com/v1/users/{user_id}/{kind}?limit=100&sortOrder=Desc");
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&enc(c));
    }
    let v = auth_get_json(&url, cookie)?;
    parse_social_user_page(&v, cookie)
}

pub fn build_profile(mut user: User) -> Profile {
    enrich_users(std::slice::from_mut(&mut user), None);
    let friends = get_json(&format!(
        "https://friends.roblox.com/v1/users/{}/friends/count",
        user.id
    ))
    .ok()
    .and_then(|v| v["count"].as_u64())
    .unwrap_or(0);
    let avatar = get_json(&format!(
        "https://thumbnails.roblox.com/v1/users/avatar?userIds={}&size=720x720&format=Png&isCircular=false",
        user.id
    ))
    .ok()
    .map(|v| str_of(&v["data"][0]["imageUrl"]))
    .unwrap_or_default();
    Profile {
        user,
        friends,
        avatar,
        robux: None,
    }
}

pub fn fetch_user_currently_wearing(user_id: u64, cookie: Option<&str>) -> Vec<AvatarAsset> {
    let Ok(v) = get_json(&format!(
        "https://avatar.roblox.com/v1/users/{user_id}/avatar"
    )) else {
        return Vec::new();
    };

    let mut items: Vec<AvatarAsset> = Vec::new();
    let mut seen_ids = HashSet::new();

    if let Some(assets) = v["assets"].as_array() {
        for a in assets {
            let Some(id) = a["id"].as_u64() else { continue };
            if !seen_ids.insert(id) {
                continue;
            }
            let mut name = str_of(&a["name"]);
            let type_name = str_of(&a["assetType"]["name"]);
            let type_id = a["assetType"]["id"].as_u64().unwrap_or(0);

            if type_name.ends_with("Animation") {
                if let Some(base) = name.split(" - ").next() {
                    name = base.to_string();
                }
            }

            items.push(AvatarAsset {
                id,
                name,
                asset_type_id: type_id,
                asset_type_name: type_name,
                thumb: String::new(),
                meta: None,
                creator_name: String::new(),
                creator_verified: false,
                price: None,
            });
        }
    }

    if let Some(emotes) = v["emotes"].as_array() {
        for e in emotes {
            let Some(id) = e["assetId"].as_u64() else {
                continue;
            };
            if !seen_ids.insert(id) {
                continue;
            }
            let name = str_of(&e["assetName"]);
            items.push(AvatarAsset {
                id,
                name,
                asset_type_id: 61,
                asset_type_name: "Emote".to_string(),
                thumb: String::new(),
                meta: None,
                creator_name: String::new(),
                creator_verified: false,
                price: None,
            });
        }
    }

    let mut deduped: Vec<AvatarAsset> = Vec::new();
    let mut seen_names = HashSet::new();
    for item in items {
        if item.asset_type_name.ends_with("Animation") {
            if !seen_names.insert(item.name.clone()) {
                continue;
            }
        }
        deduped.push(item);
    }

    let ids: Vec<u64> = deduped.iter().map(|a| a.id).collect();
    let thumbs = fetch_asset_thumbs(&ids);
    for a in &mut deduped {
        a.thumb = thumbs.get(&a.id).cloned().unwrap_or_default();
    }

    enrich_assets(&mut deduped, cookie);

    deduped
}

pub fn update_user_description(cookie: &str, description: &str) -> Result<String, String> {
    let body = serde_json::json!({ "description": description });
    let resp = post_csrf(
        "https://users.roblox.com/v1/users/description",
        body,
        Some(cookie),
        Some("https://www.roblox.com/"),
    )?;
    if resp.status() == 200 {
        Ok(description.to_string())
    } else {
        Err(format!("HTTP error {}", resp.status()))
    }
}

fn fetch_profile_favorites(user_id: u64, cookie: Option<&str>) -> Vec<Game> {
    let url = format!(
        "https://games.roblox.com/v2/users/{user_id}/favorite/games?accessFilter=2&sortOrder=Desc&limit=12"
    );
    let json = if let Some(c) = cookie {
        auth_get_json(&url, Some(c)).or_else(|_| get_json(&url))
    } else {
        get_json(&url)
    };
    let Ok(v) = json else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut fav_ids = Vec::new();
    for item in v["data"].as_array().into_iter().flatten() {
        if let Some(id) = item["id"].as_u64() {
            if let Some(pid) = item["rootPlace"]["id"].as_u64() {
                remember_place_universe(pid, id);
            }
            if id > 0 && seen.insert(id) {
                fav_ids.push(id);
            }
        }
    }
    if fav_ids.is_empty() {
        Vec::new()
    } else {
        games_by_universe(&fav_ids)
    }
}

fn fetch_user_badges(user_id: u64) -> Vec<PlayerBadge> {
    let Ok(v) = get_json(&format!(
        "https://badges.roblox.com/v1/users/{user_id}/badges?limit=12&sortOrder=Desc"
    )) else {
        return Vec::new();
    };
    let mut badges: Vec<PlayerBadge> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            let id = b["id"].as_u64()?;
            Some(PlayerBadge {
                id,
                name: str_of(&b["name"]),
                description: str_of(&b["description"]),
                icon: String::new(),
                awarded_date: str_of(&b["awardedDate"]).chars().take(10).collect(),
            })
        })
        .collect();
    let ids: Vec<u64> = badges.iter().map(|b| b.id).collect();
    if !ids.is_empty() {
        if let Ok(t) = get_json(&format!(
            "https://thumbnails.roblox.com/v1/badges/icons?badgeIds={}&size=150x150&format=Png&isCircular=false",
            join(&ids)
        )) {
            let map: HashMap<u64, String> = t["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| Some((d["targetId"].as_u64()?, str_of(&d["imageUrl"]))))
                .collect();
            for b in &mut badges {
                if let Some(url) = map.get(&b.id) {
                    b.icon = url.clone();
                }
            }
        }
    }
    badges
}

/// Streams a profile section by section.
///
/// Every request starts at the same moment on its own thread and reports the instant it
/// resolves, so the page fills in while the slowest endpoint is still running instead of
/// waiting for the whole set. `on` is called from several threads at once, so it has to be
/// cheap and thread safe (it is a channel send into the UI thread).
pub fn stream_friend_profile(
    user: User,
    cookie: Option<String>,
    on: impl Fn(ProfileUpdate) + Send + Sync,
) {
    let id = user.id;
    let cookie_ref = cookie.as_deref();
    let place_id = user.place_id;
    let on = &on;
    thread::scope(|s| {
        // Bio, join date, verified badge and the current names.
        s.spawn(move || {
            match get_json(&format!("https://users.roblox.com/v1/users/{id}")) {
                Ok(info) => on(ProfileUpdate::Info {
                    name: str_of(&info["name"]),
                    display: str_of(&info["displayName"]),
                    description: str_of(&info["description"]),
                    created: str_of(&info["created"]).chars().take(10).collect(),
                    verified: info["hasVerifiedBadge"].as_bool().unwrap_or(false),
                }),
                Err(e) => on(ProfileUpdate::Failed(format!("profile: {e}"))),
            }
        });
        // The three counters are separate endpoints and separate pills, so each lands on
        // its own instead of the row waiting for the last one.
        for endpoint in ["friends", "followers", "followings"] {
            s.spawn(move || {
                let count = get_json(&format!(
                    "https://friends.roblox.com/v1/users/{id}/{endpoint}/count"
                ))
                .ok()
                .and_then(|v| v["count"].as_u64());
                let update = match count {
                    None => ProfileUpdate::Failed(format!("{endpoint}: no count")),
                    Some(n) if endpoint == "friends" => ProfileUpdate::Friends(n),
                    Some(n) if endpoint == "followers" => ProfileUpdate::Followers(n),
                    Some(n) => ProfileUpdate::Following(n),
                };
                on(update);
            });
        }
        // 352x352 is the size the header circle actually draws at 2x; the bigger 720x720
        // copy cost more to download than it ever showed.
        s.spawn(move || {
            let url = get_json(&format!(
                "https://thumbnails.roblox.com/v1/users/avatar?userIds={id}&size=352x352&format=Png&isCircular=false"
            ))
            .ok()
            .map(|v| str_of(&v["data"][0]["imageUrl"]))
            .unwrap_or_default();
            on(ProfileUpdate::Avatar(url));
        });
        // Only when the seed has no headshot (a search result carries one, the user
        // record from a server row may not).
        if user.headshot.is_empty() {
            s.spawn(move || {
                let url = get_json(&format!(
                    "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={id}&size=420x420&format=Png&isCircular=false"
                ))
                .ok()
                .map(|v| str_of(&v["data"][0]["imageUrl"]))
                .unwrap_or_default();
                on(ProfileUpdate::Headshot(url));
            });
        }
        s.spawn(move || on(ProfileUpdate::Wearing(fetch_user_currently_wearing(id, cookie_ref))));
        s.spawn(move || on(ProfileUpdate::Badges(fetch_user_badges(id))));
        s.spawn(move || on(ProfileUpdate::Favorites(fetch_profile_favorites(id, cookie_ref))));
        s.spawn(move || {
            let exp_ids: Vec<u64> = get_json(&format!(
                "https://games.roblox.com/v2/users/{id}/games?accessFilter=2&limit=12"
            ))
            .ok()
            .map(|v| {
                v["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|g| g["id"].as_u64())
                    .collect()
            })
            .unwrap_or_default();
            let games = if exp_ids.is_empty() {
                Vec::new()
            } else {
                games_by_universe(&exp_ids)
            };
            on(ProfileUpdate::Creations(games));
        });
        // Presence is the one section that can reveal a new place, so it runs last and its
        // game is resolved from the place it reports.
        s.spawn(move || {
            let mut presence_user = user.clone();
            enrich_presence(&mut presence_user, cookie_ref);
            on(ProfileUpdate::Presence {
                presence: presence_user.presence,
                location: presence_user.location.clone(),
                place_id: presence_user.place_id,
            });
            let pid = presence_user.place_id.or(place_id);
            if let Some(pid) = pid {
                if let Some(game) = games_by_places(&[pid]).into_iter().next() {
                    on(ProfileUpdate::Playing(Some(game)));
                }
            }
        });
    });
    on(ProfileUpdate::Done);
}

/// The whole profile in one call, assembled from the same stream. Used where the result is
/// cached rather than painted (prefetching, the signed-in account's own page).
pub fn fetch_friend_profile(
    user: User,
    cookie: Option<String>,
) -> Result<FriendProfile, String> {
    let seed = crate::profile_cache::seed(&user);
    let (tx, rx) = std::sync::mpsc::channel();
    stream_friend_profile(user, cookie, |update| {
        let _ = tx.send(update);
    });
    let mut stream = crate::profile_cache::FriendStream::new(seed.user.id, seed, rx);
    stream.drain();
    if !stream.profile.sections.info {
        return Err(stream
            .err
            .unwrap_or_else(|| "Could not load this profile.".to_string()));
    }
    Ok(stream.profile)
}

pub fn fetch_robux_balance(user_id: u64, cookie: &str) -> Option<u64> {
    let urls = [
        format!("https://economy.roblox.com/v1/users/{user_id}/currency"),
        "https://economy.roblox.com/v1/user/currency".to_string(),
    ];
    for url in &urls {
        if let Ok(v) = auth_get_json(url, Some(cookie)) {
            if let Some(robux) = v["robux"].as_u64() {
                return Some(robux);
            }
        }
    }
    None
}

pub fn fetch_user_friends(id: u64, cookie: Option<&str>) -> Result<Vec<User>, String> {
    let v = auth_get_json(
        &format!("https://friends.roblox.com/v1/users/{id}/friends"),
        cookie,
    )?;
    // `/friends` increasingly answers ids only, so every row is name-resolved as a batch here
    // instead of trusting it to carry `name`/`displayName`.
    // Names first - one call per 100 ids, and anything already known comes from the session
    // book without a request - then headshots and presence.
    let mut friends: Vec<User> = resolve_friend_rows(&v, cookie)
        .into_iter()
        .map(user_from_row)
        .collect();
    enrich_users(&mut friends, cookie);

    // Some accounts get an empty list instead; `friends/find` still answers with ids, which the
    // same batch resolve turns into real names.
    if friends.is_empty() {
        if let Ok(found) = auth_get_json(
            &format!("https://friends.roblox.com/v1/users/{id}/friends/find?limit=50"),
            cookie,
        ) {
            let mut list: Vec<User> = resolve_friend_rows(
                &serde_json::json!({ "data": found["PageItems"].clone() }),
                cookie,
            )
            .into_iter()
            .map(user_from_row)
            .collect();
            enrich_users(&mut list, cookie);
            friends = list;
        }
    }

    friends.sort_by_key(|f| match f.presence {
        2 => 0,
        1 => 1,
        3 => 2,
        _ => 3,
    });
    Ok(friends)
}

pub fn fetch_authenticated_session(cookie: &str) -> Result<(Profile, Vec<User>), String> {
    let me_json = auth_get_json(
        "https://users.roblox.com/v1/users/authenticated",
        Some(cookie),
    )?;
    let id = me_json["id"]
        .as_u64()
        .ok_or_else(|| "Session expired or invalid".to_string())?;
    let user = User {
        id,
        name: str_of(&me_json["name"]),
        display: str_of(&me_json["displayName"]),
        headshot: String::new(),
        presence: 1,
        location: String::new(),
        place_id: None,
        verified: me_json["hasVerifiedBadge"].as_bool().unwrap_or(false),
    };
    let mut profile = build_profile(user);
    let (robux, friends) = thread::scope(|s| {
        let h_robux = s.spawn(|| fetch_robux_balance(id, cookie));
        let friends = fetch_user_friends(id, Some(cookie)).unwrap_or_default();
        (h_robux.join().ok().flatten(), friends)
    });
    profile.robux = robux;

    Ok((profile, friends))
}

pub fn parse_hex_color(hex: &str) -> Color32 {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&clean[0..2], 16),
            u8::from_str_radix(&clean[2..4], 16),
            u8::from_str_radix(&clean[4..6], 16),
        ) {
            return Color32::from_rgb(r, g, b);
        }
    }
    Color32::from_rgb(163, 162, 165)
}


pub fn fetch_asset_thumbs(ids: &[u64]) -> HashMap<u64, String> {
    let mut map = HashMap::new();
    for chunk in ids.chunks(50) {
        if chunk.is_empty() {
            continue;
        }
        if let Ok(v) = get_json(&format!(
            "https://thumbnails.roblox.com/v1/assets?assetIds={}&size=150x150&format=Png&isCircular=false",
            join(chunk)
        )) {
            for d in v["data"].as_array().into_iter().flatten() {
                if let (Some(id), Some(url)) = (d["targetId"].as_u64(), d["imageUrl"].as_str()) {
                    if !url.is_empty() {
                        map.insert(id, url.to_string());
                    }
                }
            }
        }
    }
    map
}

pub fn parse_avatar_state_json(v: &Value) -> AvatarState {
    let avatar_type = match v["playerAvatarType"].as_str() {
        Some("R6") => "R6".to_string(),
        _ => "R15".to_string(),
    };
    let s = &v["scales"];
    let scales = AvatarScales {
        height: s["height"].as_f64().unwrap_or(1.0) as f32,
        width: s["width"].as_f64().unwrap_or(1.0) as f32,
        head: s["head"].as_f64().unwrap_or(1.0) as f32,
        depth: s["depth"].as_f64().unwrap_or(1.0) as f32,
        proportion: s["proportion"].as_f64().unwrap_or(0.0) as f32,
        body_type: s["bodyType"].as_f64().unwrap_or(0.0) as f32,
    };
    let c = &v["bodyColors"];
    let colors = BodyColors {
        head: c["headColorId"].as_u64().unwrap_or(194),
        torso: c["torsoColorId"].as_u64().unwrap_or(23),
        right_arm: c["rightArmColorId"].as_u64().unwrap_or(194),
        left_arm: c["leftArmColorId"].as_u64().unwrap_or(194),
        right_leg: c["rightLegColorId"].as_u64().unwrap_or(102),
        left_leg: c["leftLegColorId"].as_u64().unwrap_or(102),
    };
    let mut wearing: Vec<AvatarAsset> = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let id = a["id"].as_u64()?;
            let name = str_of(&a["name"]);
            let type_name = str_of(&a["assetType"]["name"]);
            let type_id = a["assetType"]["id"]
                .as_u64()
                .unwrap_or_else(|| asset_type_id_from_name(&type_name));
            let meta = if a["meta"].is_null() {
                None
            } else {
                Some(a["meta"].clone())
            };
            Some(AvatarAsset {
                id,
                name,
                asset_type_id: type_id,
                asset_type_name: type_name,
                thumb: String::new(),
                meta,
                creator_name: String::new(),
                creator_verified: false,
                price: None,
            })
        })
        .collect();
    let ids: Vec<u64> = wearing.iter().map(|a| a.id).collect();
    let thumbs = fetch_asset_thumbs(&ids);
    for a in &mut wearing {
        a.thumb = thumbs.get(&a.id).cloned().unwrap_or_default();
    }
    AvatarState {
        avatar_type,
        scales,
        colors,
        wearing,
    }
}

pub fn enrich_assets(assets: &mut [AvatarAsset], cookie: Option<&str>) {
    if assets.is_empty() {
        return;
    }
    for chunk in assets.chunks_mut(30) {
        let items: Vec<Value> = chunk
            .iter()
            .map(|a| serde_json::json!({ "itemType": 1, "id": a.id }))
            .collect();
        if let Ok(resp) = post_csrf(
            "https://catalog.roblox.com/v1/catalog/items/details",
            serde_json::json!({ "items": items }),
            cookie,
            Some("https://www.roblox.com/"),
        ) {
            if let Ok(v) = resp.into_json::<Value>() {
                for item in v["data"].as_array().into_iter().flatten() {
                    let Some(id) = item["id"].as_u64() else {
                        continue;
                    };
                    if let Some(a) = chunk.iter_mut().find(|a| a.id == id) {
                        let cname = str_of(&item["creatorName"]);
                        if !cname.is_empty() {
                            a.creator_name = cname;
                        }
                        a.creator_verified =
                            item["creatorHasVerifiedBadge"].as_bool().unwrap_or(false);
                        a.price = item["price"]
                            .as_u64()
                            .or_else(|| item["lowestPrice"].as_u64());
                        if let Some(n) = item["name"].as_str() {
                            if !n.is_empty() && !a.asset_type_name.ends_with("Animation") {
                                a.name = n.to_string();
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn fetch_avatar_state(cookie: &str) -> Result<AvatarState, String> {
    let v = auth_get_json("https://avatar.roblox.com/v1/avatar", Some(cookie))?;
    let mut state = parse_avatar_state_json(&v);
    enrich_assets(&mut state.wearing, Some(cookie));
    Ok(state)
}

pub fn fetch_avatar_palette() -> Vec<PaletteColor> {
    let Ok(v) = get_json("https://avatar.roblox.com/v1/avatar-rules") else {
        return Vec::new();
    };
    v["bodyColorsPalette"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(PaletteColor {
                id: c["brickColorId"].as_u64()?,
                name: str_of(&c["name"]),
                color: parse_hex_color(&str_of(&c["hexColor"])),
            })
        })
        .collect()
}

pub fn fetch_avatar_session(cookie: &str) -> Result<(AvatarState, Vec<PaletteColor>), String> {
    let state = fetch_avatar_state(cookie)?;
    let palette = fetch_avatar_palette();
    Ok((state, palette))
}

pub fn fetch_avatar_inventory(
    user_id: u64,
    tab: AvatarTab,
    cookie: &str,
    cursor: Option<&str>,
) -> Result<(Vec<AvatarAsset>, Option<String>), String> {
    let Some(types) = tab.inventory_query() else {
        return Ok((Vec::new(), None));
    };
    let mut url = format!(
        "https://inventory.roblox.com/v2/users/{user_id}/inventory?assetTypes={types}&limit=50&sortOrder=Desc"
    );
    if let Some(c) = cursor.filter(|s| !s.is_empty()) {
        url.push_str("&cursor=");
        url.push_str(&enc(c));
    }
    let v = auth_get_json(&url, Some(cookie))?;
    let next_cursor = v["nextPageCursor"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut items: Vec<AvatarAsset> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|it| {
            let id = it["assetId"].as_u64()?;
            let name = str_of(&it["name"]);
            let type_name = str_of(&it["assetType"]);
            let type_id = asset_type_id_from_name(&type_name);
            Some(AvatarAsset {
                id,
                name,
                asset_type_id: type_id,
                asset_type_name: type_name,
                thumb: String::new(),
                meta: None,
                creator_name: String::new(),
                creator_verified: false,
                price: None,
            })
        })
        .collect();
    let ids: Vec<u64> = items.iter().map(|a| a.id).collect();
    let thumbs = fetch_asset_thumbs(&ids);
    for a in &mut items {
        a.thumb = thumbs.get(&a.id).cloned().unwrap_or_default();
    }
    enrich_assets(&mut items, Some(cookie));
    Ok((items, next_cursor))
}

fn parse_outfit_list(v: &Value) -> Vec<OutfitItem> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| {
            Some(OutfitItem {
                id: o["id"].as_u64()?,
                name: str_of(&o["name"]),
                thumb: String::new(),
                is_editable: o["isEditable"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

pub fn fetch_avatar_outfits(
    user_id: u64,
    page: u32,
    cookie: &str,
) -> Result<(Vec<OutfitItem>, bool), String> {
    let mut outfits: Vec<OutfitItem> = Vec::new();
    let has_more;
    if page == 1 {
        let mut seen = HashSet::new();
        if let Ok(editable_v) = auth_get_json(
            &format!(
                "https://avatar.roblox.com/v1/users/{user_id}/outfits?isEditable=true&page=1&itemsPerPage=50"
            ),
            Some(cookie),
        ) {
            for item in parse_outfit_list(&editable_v) {
                if seen.insert(item.id) {
                    outfits.push(item);
                }
            }
        }
        let all_res = auth_get_json(
            &format!("https://avatar.roblox.com/v1/users/{user_id}/outfits?page=1&itemsPerPage=50"),
            Some(cookie),
        );
        match all_res {
            Ok(v) => {
                let all_items = parse_outfit_list(&v);
                has_more = all_items.len() >= 50;
                for item in all_items {
                    if seen.insert(item.id) {
                        outfits.push(item);
                    }
                }
            }
            Err(e) if outfits.is_empty() => return Err(e),
            Err(_) => has_more = false,
        }
    } else {
        let v = auth_get_json(
            &format!(
                "https://avatar.roblox.com/v1/users/{user_id}/outfits?page={page}&itemsPerPage=50"
            ),
            Some(cookie),
        )?;
        outfits = parse_outfit_list(&v);
        has_more = outfits.len() >= 50;
    }
    let ids: Vec<u64> = outfits.iter().map(|o| o.id).collect();
    for chunk in ids.chunks(50) {
        if chunk.is_empty() {
            continue;
        }
        if let Ok(t) = get_json(&format!(
            "https://thumbnails.roblox.com/v1/users/outfits?userOutfitIds={}&size=150x150&format=Png&isCircular=false",
            join(chunk)
        )) {
            let map: HashMap<u64, String> = t["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| Some((d["targetId"].as_u64()?, str_of(&d["imageUrl"]))))
                .collect();
            for o in &mut outfits {
                if let Some(url) = map.get(&o.id) {
                    o.thumb = url.clone();
                }
            }
        }
    }
    Ok((outfits, has_more))
}

pub fn fetch_user_renders(user_id: u64) -> (String, String) {
    let mut avatar_url = String::new();
    let mut headshot_url = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(450));
        }
        if avatar_url.is_empty() {
            if let Ok(v) = get_json(&format!(
                "https://thumbnails.roblox.com/v1/users/avatar?userIds={user_id}&size=720x720&format=Png&isCircular=false"
            )) {
                let state = v["data"][0]["state"].as_str().unwrap_or("");
                let url = str_of(&v["data"][0]["imageUrl"]);
                if state == "Completed" && !url.is_empty() {
                    avatar_url = url;
                }
            }
        }
        if headshot_url.is_empty() {
            if let Ok(v) = get_json(&format!(
                "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={user_id}&size=150x150&format=Png&isCircular=false"
            )) {
                let state = v["data"][0]["state"].as_str().unwrap_or("");
                let url = str_of(&v["data"][0]["imageUrl"]);
                if state == "Completed" && !url.is_empty() {
                    headshot_url = url;
                }
            }
        }
        if !avatar_url.is_empty() && !headshot_url.is_empty() {
            break;
        }
    }
    (avatar_url, headshot_url)
}

pub fn avatar_assets_payload(assets: &[AvatarAsset]) -> Value {
    let arr: Vec<Value> = assets
        .iter()
        .map(|a| {
            let mut obj = serde_json::json!({ "id": a.id });
            if let Some(m) = &a.meta {
                obj["meta"] = m.clone();
            }
            obj
        })
        .collect();
    serde_json::json!({ "assets": arr })
}



// The equip/unequip rules live with the optimistic sync state so they can be tested in
// `vm-tests/`; keep the old paths working for this module's callers.
pub use crate::avatar_sync::asset_type_id_from_name;

pub fn post_wearing_assets(cookie: &str, assets: &[AvatarAsset]) -> Result<(), String> {
    let resp = post_csrf(
        "https://avatar.roblox.com/v2/avatar/set-wearing-assets",
        avatar_assets_payload(assets),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    )?;
    if let Ok(v) = resp.into_json::<Value>() {
        if v["success"].as_bool() == Some(false) {
            return Err("Roblox could not equip one or more selected items.".to_string());
        }
    }
    Ok(())
}

fn last_render_urls() -> &'static Mutex<(String, String)> {
    static LAST: OnceLock<Mutex<(String, String)>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new((String::new(), String::new())))
}

/// Remembers the thumbnail URLs shown on screen so an edit can tell "stale" from "new".
pub fn remember_render_urls(avatar_url: &str, headshot_url: &str) {
    if let Ok(mut g) = last_render_urls().lock() {
        if !avatar_url.is_empty() {
            g.0 = avatar_url.to_string();
        }
        if !headshot_url.is_empty() {
            g.1 = headshot_url.to_string();
        }
    }
}

pub fn finish_avatar_update(
    user_id: u64,
    cookie: &str,
    note: String,
) -> Result<(AvatarState, String, String, String), String> {
    let state = fetch_avatar_state(cookie)?;
    // Roblox keeps serving the previous picture for a few seconds after an edit, so keep
    // asking until the thumbnail URL actually changes (bounded, ~8 s).
    let (old_avatar, old_head) = last_render_urls()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut avatar_url = String::new();
    let mut headshot_url = String::new();
    for attempt in 0..8 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(1000));
        }
        let (a, h) = fetch_user_renders(user_id);
        if !a.is_empty() && a != old_avatar {
            avatar_url = a;
        } else if avatar_url.is_empty() && !a.is_empty() && attempt == 7 {
            avatar_url = a;
        }
        if !h.is_empty() && h != old_head {
            headshot_url = h;
        } else if headshot_url.is_empty() && !h.is_empty() && attempt == 7 {
            headshot_url = h;
        }
        if !avatar_url.is_empty() && !headshot_url.is_empty() {
            break;
        }
    }
    remember_render_urls(&avatar_url, &headshot_url);
    Ok((state, avatar_url, headshot_url, note))
}

pub fn apply_avatar_assets(
    user_id: u64,
    cookie: &str,
    assets: &[AvatarAsset],
    note: String,
) -> Result<(AvatarState, String, String, String), String> {
    post_wearing_assets(cookie, assets)?;
    finish_avatar_update(user_id, cookie, note)
}

pub fn apply_avatar_type(
    user_id: u64,
    cookie: &str,
    avatar_type: &str,
) -> Result<(AvatarState, String, String, String), String> {
    post_csrf(
        "https://avatar.roblox.com/v1/avatar/set-player-avatar-type",
        serde_json::json!({ "playerAvatarType": avatar_type }),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    )?;
    finish_avatar_update(
        user_id,
        cookie,
        format!("Switched avatar rig to {avatar_type}."),
    )
}

pub fn apply_body_colors(
    user_id: u64,
    cookie: &str,
    colors: BodyColors,
) -> Result<(AvatarState, String, String, String), String> {
    post_csrf(
        "https://avatar.roblox.com/v1/avatar/set-body-colors",
        serde_json::json!({
            "headColorId": colors.head,
            "torsoColorId": colors.torso,
            "rightArmColorId": colors.right_arm,
            "leftArmColorId": colors.left_arm,
            "rightLegColorId": colors.right_leg,
            "leftLegColorId": colors.left_leg,
        }),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    )?;
    finish_avatar_update(user_id, cookie, "Updated avatar skin tone.".to_string())
}

pub fn apply_avatar_scales(
    user_id: u64,
    cookie: &str,
    scales: AvatarScales,
) -> Result<(AvatarState, String, String, String), String> {
    post_csrf(
        "https://avatar.roblox.com/v1/avatar/set-scales",
        serde_json::json!({
            "height": (scales.height * 100.0).round() / 100.0,
            "width": (scales.width * 100.0).round() / 100.0,
            "head": (scales.head * 100.0).round() / 100.0,
            "depth": (scales.depth * 100.0).round() / 100.0,
            "proportion": (scales.proportion * 100.0).round() / 100.0,
            "bodyType": (scales.body_type * 100.0).round() / 100.0,
        }),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    )?;
    finish_avatar_update(user_id, cookie, "Updated avatar body scales.".to_string())
}

pub fn apply_wear_outfit(
    user_id: u64,
    cookie: &str,
    outfit: &OutfitItem,
) -> Result<(AvatarState, String, String, String), String> {
    let details = auth_get_json(
        &format!("https://avatar.roblox.com/v1/outfits/{}/details", outfit.id),
        Some(cookie),
    )?;
    let parsed = parse_avatar_state_json(&details);
    let _ = post_csrf(
        "https://avatar.roblox.com/v1/avatar/set-player-avatar-type",
        serde_json::json!({ "playerAvatarType": parsed.avatar_type }),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    );
    let _ = post_csrf(
        "https://avatar.roblox.com/v1/avatar/set-body-colors",
        serde_json::json!({
            "headColorId": parsed.colors.head,
            "torsoColorId": parsed.colors.torso,
            "rightArmColorId": parsed.colors.right_arm,
            "leftArmColorId": parsed.colors.left_arm,
            "rightLegColorId": parsed.colors.right_leg,
            "leftLegColorId": parsed.colors.left_leg,
        }),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    );
    let s = &details["scale"];
    if s.is_object() {
        let _ = post_csrf(
            "https://avatar.roblox.com/v1/avatar/set-scales",
            s.clone(),
            Some(cookie),
            Some("https://www.roblox.com/my/avatar"),
        );
    }
    post_wearing_assets(cookie, &parsed.wearing)?;
    finish_avatar_update(
        user_id,
        cookie,
        format!("Equipped outfit \"{}\".", outfit.name),
    )
}

fn brick_hex_from_palette(id: u64, palette: &[PaletteColor]) -> String {
    let c = palette
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.color)
        .unwrap_or(eframe::egui::Color32::from_rgb(163, 162, 165));
    format!("{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

pub fn create_avatar_outfit(
    cookie: &str,
    name: &str,
    fallback_state: Option<AvatarState>,
    palette: &[PaletteColor],
) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Please enter an outfit name.".to_string());
    }
    let state = match fallback_state {
        Some(s) => s,
        None => fetch_avatar_state(cookie)?,
    };

    let v2 = auth_get_json("https://avatar.roblox.com/v2/avatar/avatar", Some(cookie)).ok();
    let v4_assets = v2
        .as_ref()
        .and_then(|v| v["assets"].as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|a| {
                    let id = a["id"].as_u64()?;
                    let mut obj = serde_json::json!({ "id": id });
                    if let Some(m) = a.get("meta").filter(|m| !m.is_null()) {
                        obj["meta"] = m.clone();
                    }
                    Some(obj)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            state
                .wearing
                .iter()
                .map(|a| {
                    let mut obj = serde_json::json!({ "id": a.id });
                    if let Some(m) = &a.meta {
                        obj["meta"] = m.clone();
                    }
                    obj
                })
                .collect()
        });

    let body_color3s = v2
        .as_ref()
        .and_then(|v| v.get("bodyColor3s").filter(|c| c.is_object()).cloned())
        .unwrap_or_else(|| {
            serde_json::json!({
                "headColor3": brick_hex_from_palette(state.colors.head, palette),
                "torsoColor3": brick_hex_from_palette(state.colors.torso, palette),
                "rightArmColor3": brick_hex_from_palette(state.colors.right_arm, palette),
                "leftArmColor3": brick_hex_from_palette(state.colors.left_arm, palette),
                "rightLegColor3": brick_hex_from_palette(state.colors.right_leg, palette),
                "leftLegColor3": brick_hex_from_palette(state.colors.left_leg, palette),
            })
        });

    let scale_json = v2
        .as_ref()
        .and_then(|v| v.get("scales").filter(|s| s.is_object()).cloned())
        .unwrap_or_else(|| {
            serde_json::json!({
                "height": (state.scales.height * 100.0).round() / 100.0,
                "width": (state.scales.width * 100.0).round() / 100.0,
                "head": (state.scales.head * 100.0).round() / 100.0,
                "depth": (state.scales.depth * 100.0).round() / 100.0,
                "proportion": (state.scales.proportion * 100.0).round() / 100.0,
                "bodyType": (state.scales.body_type * 100.0).round() / 100.0,
            })
        });

    let rig = v2
        .as_ref()
        .and_then(|v| v["playerAvatarType"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(state.avatar_type.as_str());

    let v4_body = serde_json::json!({
        "name": trimmed,
        "outfitType": "Avatar",
        "assets": v4_assets,
        "bodyColor3s": body_color3s,
        "scale": scale_json,
        "playerAvatarType": rig,
    });

    match post_csrf(
        "https://avatar.roblox.com/v4/outfits/create",
        v4_body,
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    ) {
        Ok(_) => Ok(format!("Saved outfit \"{trimmed}\".")),
        Err(v4_err) => {
            let asset_ids: Vec<u64> = state.wearing.iter().map(|a| a.id).collect();
            let v1_body = serde_json::json!({
                "name": trimmed,
                "assetIds": asset_ids,
                "bodyColors": {
                    "headColorId": state.colors.head,
                    "torsoColorId": state.colors.torso,
                    "rightArmColorId": state.colors.right_arm,
                    "leftArmColorId": state.colors.left_arm,
                    "rightLegColorId": state.colors.right_leg,
                    "leftLegColorId": state.colors.left_leg,
                },
                "scale": scale_json,
                "playerAvatarType": state.avatar_type,
            });
            match post_csrf(
                "https://avatar.roblox.com/v1/outfits/create",
                v1_body,
                Some(cookie),
                Some("https://www.roblox.com/my/avatar"),
            ) {
                Ok(_) => Ok(format!("Saved outfit \"{trimmed}\".")),
                Err(_) => Err(v4_err),
            }
        }
    }
}

pub fn delete_avatar_outfit(cookie: &str, outfit: &OutfitItem) -> Result<String, String> {
    post_csrf(
        &format!("https://avatar.roblox.com/v1/outfits/{}/delete", outfit.id),
        serde_json::json!({}),
        Some(cookie),
        Some("https://www.roblox.com/my/avatar"),
    )?;
    Ok(format!("Deleted outfit \"{}\".", outfit.name))
}

pub fn start_quick_login(tx: Sender<Msg>, ctx: egui::Context) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    thread::spawn(move || {
        let send = |m: Msg| {
            let _ = tx.send(m);
            ctx.request_repaint();
        };

        let created = match post_json(
            "https://apis.roblox.com/auth-token-service/v1/login/create",
            serde_json::json!({}),
        ) {
            Ok(v) => v,
            Err(e) => {
                send(Msg::QuickLoginDone(Err(format!(
                    "Could not start Quick Login: {e}"
                ))));
                return;
            }
        };

        let Some(code) = created["code"].as_str().map(str::to_string) else {
            send(Msg::QuickLoginDone(Err(
                "Unexpected response from Roblox Quick Login".to_string(),
            )));
            return;
        };
        let Some(private_key) = created["privateKey"].as_str().map(str::to_string) else {
            send(Msg::QuickLoginDone(Err(
                "Missing private key from Roblox Quick Login".to_string(),
            )));
            return;
        };
        let image_path = str_of(&created["imagePath"]);
        let qr_bytes = if !image_path.is_empty() {
            let qr_url = format!("https://apis.roblox.com/auth-token-service{image_path}");
            agent().get(&qr_url).call().ok().and_then(|resp| {
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(64 * 1024)
                    .read_to_end(&mut buf)
                    .ok()?;
                Some(buf)
            })
        } else {
            None
        };

        send(Msg::QuickLoginCreated {
            code: code.clone(),
            qr_bytes,
        });

        while !flag.load(Ordering::Relaxed) {
            for _ in 0..15 {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(200));
            }
            if flag.load(Ordering::Relaxed) {
                return;
            }

            let resp = match post_csrf(
                "https://apis.roblox.com/auth-token-service/v1/login/status",
                serde_json::json!({ "code": code, "privateKey": private_key }),
                None,
                None,
            ) {
                Ok(r) => r,
                Err(e) => {
                    if e.contains("400") || e.contains("404") {
                        send(Msg::QuickLoginDone(Err(
                            "Quick Login code expired. Click New code to try again.".to_string(),
                        )));
                        return;
                    }
                    continue;
                }
            };

            let Ok(v) = resp.into_json::<Value>() else {
                continue;
            };
            let status = str_of(&v["status"]);
            let account = v["accountName"].as_str().map(str::to_string);

            if status == "Validated" {
                match post_csrf(
                    "https://auth.roblox.com/v2/login",
                    serde_json::json!({
                        "ctype": "AuthToken",
                        "cvalue": code,
                        "password": private_key
                    }),
                    None,
                    None,
                ) {
                    Ok(login_resp) => {
                        if let Some(cookie) = extract_roblosecurity(&login_resp) {
                            send(Msg::QuickLoginDone(Ok(cookie)));
                        } else {
                            send(Msg::QuickLoginDone(Err(
                                "Login approved, but Roblox did not return a session cookie."
                                    .to_string(),
                            )));
                        }
                    }
                    Err(e) => {
                        send(Msg::QuickLoginDone(Err(format!("Login failed: {e}"))));
                    }
                }
                return;
            } else {
                send(Msg::QuickLoginStatus {
                    code: code.clone(),
                    status,
                    account,
                });
            }
        }
    });
    cancel
}

pub fn parse_client_version_update(
    v: &Value,
    local_version_folder: Option<&str>,
) -> Option<(String, String)> {
    let upload = str_of(&v["clientVersionUpload"]);
    let version = str_of(&v["version"]);
    let local = local_version_folder?.trim();
    if upload.is_empty() || local.is_empty() || local.eq_ignore_ascii_case(&upload) {
        return None;
    }
    Some((
        if version.is_empty() {
            upload.clone()
        } else {
            version
        },
        upload,
    ))
}

pub fn check_roblox_update(
    local_player: Option<&std::path::Path>,
) -> Result<Option<(String, String)>, String> {
    let Some(local_folder) = local_player
        .and_then(crate::roblox_process::version_dir_for_player)
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
    else {
        return Ok(None);
    };
    // Roblox publishes a separate client channel per operating system.
    let channel = if cfg!(target_os = "macos") {
        "MacPlayer"
    } else {
        "WindowsPlayer"
    };
    let v = get_json(&format!(
        "https://clientsettings.roblox.com/v2/client-version/{channel}"
    ))?;
    Ok(parse_client_version_update(&v, Some(local_folder.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_roblox_client_update_when_version_differs() {
        let json = serde_json::json!({
            "version": "0.699.0.6990001",
            "clientVersionUpload": "version-new123456"
        });
        assert_eq!(
            parse_client_version_update(&json, Some("version-old999999")),
            Some((
                "0.699.0.6990001".to_string(),
                "version-new123456".to_string()
            ))
        );
        assert_eq!(
            parse_client_version_update(&json, Some("version-new123456")),
            None
        );
        assert_eq!(parse_client_version_update(&json, None), None);
    }

    #[test]
    fn parses_roblox_game_links_and_place_ids() {
        assert_eq!(
            parse_game_link("1818"),
            Some(ParsedGameLink {
                place_id: Some(1818),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link("  <https://www.roblox.com/games/2753915549/Blox-Fruits>  "),
            Some(ParsedGameLink {
                place_id: Some(2753915549),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link(
                "www.roblox.com/en/games/4924922222/Brookhaven-RP?privateServerLinkCode=abc_123"
            ),
            Some(ParsedGameLink {
                place_id: Some(4924922222),
                private_server_code: Some("abc_123".to_string()),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link("https://web.roblox.com/games/start?placeId=1818&launchData=foo"),
            Some(ParsedGameLink {
                place_id: Some(1818),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link("roblox://placeId=920587237"),
            Some(ParsedGameLink {
                place_id: Some(920587237),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link("roblox://experiences/start?placeId=1818&linkCode=secret99"),
            Some(ParsedGameLink {
                place_id: Some(1818),
                private_server_code: Some("secret99".to_string()),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link(
                "https://ro.blox.com/Ebh5?af_dp=roblox%3A%2F%2Fexperiences%2Fstart%3FplaceId%3D1818"
            ),
            Some(ParsedGameLink {
                place_id: Some(1818),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_game_link("https://www.roblox.com/share?code=9f8e7d6c5b4a&type=Server"),
            Some(ParsedGameLink {
                place_id: None,
                private_server_code: None,
                share_code: Some("9f8e7d6c5b4a".to_string()),
                share_type: Some("Server".to_string()),
            })
        );

        assert_eq!(parse_game_link(""), None);
        assert_eq!(parse_game_link("0"), None);
        assert_eq!(parse_game_link("blox fruits"), None);
        assert_eq!(parse_game_link("game 1818"), None);
        assert_eq!(parse_game_link("https://example.com/games/1818"), None);
    }

    #[test]
    fn builds_place_launcher_urls_for_public_job_and_private_links() {
        assert_eq!(
            crate::roblox_process::build_place_launcher_url(1818, None, None),
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame&browserTrackerId=0&placeId=1818&isPlayTogetherGame=false"
        );
        assert_eq!(
            crate::roblox_process::build_place_launcher_url(1818, Some("job-1"), None),
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGameJob&browserTrackerId=0&placeId=1818&gameId=job-1&isPlayTogetherGame=false"
        );
        assert_eq!(
            crate::roblox_process::build_place_launcher_url(1818, None, Some("vip+1")),
            "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestPrivateGame&browserTrackerId=0&placeId=1818&linkCode=vip%2B1&isPlayTogetherGame=false"
        );
    }
}

// ---------------------------------------------------------------------------
// Startup caches
//
// The home and charts pages should have something on screen immediately instead of
// waiting for Roblox's APIs. These snapshots are written whenever fresh data arrives
// and read once at startup; the normal background sync replaces them a moment later.
// ---------------------------------------------------------------------------

fn cache_write(file: &str, text: &str) {
    let path = crate::auth::data_dir().join(file);
    // Skip identical writes: the background sync runs every few seconds.
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text) {
        return;
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn cache_read(file: &str) -> Option<Value> {
    let text = std::fs::read_to_string(crate::auth::data_dir().join(file)).ok()?;
    serde_json::from_str::<Value>(&text).ok()
}

pub fn save_sorts_cache(sorts: &[Sort]) {
    let rows: Vec<Value> = sorts
        .iter()
        .map(|s| {
            let games: Vec<Value> = s
                .games
                .iter()
                .map(|g| {
                    serde_json::json!({
                        "u": g.universe_id,
                        "p": g.place_id,
                        "n": g.name,
                        "c": g.creator,
                        "cv": g.creator_verified,
                        "pl": g.playing,
                        "up": g.up,
                        "dn": g.down,
                    })
                })
                .collect();
            serde_json::json!({ "title": s.title, "games": games })
        })
        .collect();
    if sorts.is_empty() {
        return;
    }
    if let Ok(text) = serde_json::to_string(&rows) {
        cache_write("sorts-cache.json", &text);
    }
}

pub fn load_sorts_cache() -> Vec<Sort> {
    let Some(rows) = cache_read("sorts-cache.json") else {
        return Vec::new();
    };
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let games: Vec<Game> = s["games"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| {
                    {
                        let mut game = Game {
                            universe_id: g["u"].as_u64()?,
                            place_id: g["p"].as_u64().unwrap_or(0),
                            name: g["n"].as_str().unwrap_or_default().to_string(),
                            creator: g["c"].as_str().unwrap_or_default().to_string(),
                            creator_verified: g["cv"].as_bool().unwrap_or(false),
                            playing: g["pl"].as_u64().unwrap_or(0),
                            up: g["up"].as_u64().unwrap_or(0),
                            down: g["dn"].as_u64().unwrap_or(0),
                            description: String::new(),
                            ..Game::default()
                        };
                        crate::chart_feed::fill_labels(&mut game);
                        Some(game)
                    }
                })
                .collect();
            let title = s["title"].as_str().unwrap_or_default().to_string();
            (!games.is_empty()).then_some(Sort { title, games })
        })
        .collect()
}

pub fn save_friends_cache(friends: &[User]) {
    // A failed/partial fetch must never replace a good cache with nameless entries:
    // that is what made every friend show up as "Player" on the next launch.
    if friends.is_empty() || friends.iter().all(|f| f.name.is_empty()) {
        return;
    }
    let rows: Vec<Value> = friends
        .iter()
        .filter(|f| !f.name.is_empty() || !f.display.is_empty())
        .map(|f| {
            serde_json::json!({
                "id": f.id,
                "name": f.name,
                "display": f.display,
                "headshot": f.headshot,
                "presence": f.presence,
                "location": f.location,
                "place": f.place_id,
                "verified": f.verified,
            })
        })
        .collect();
    if let Ok(text) = serde_json::to_string(&rows) {
        cache_write("friends-cache.json", &text);
    }
}

pub fn load_friends_cache() -> Vec<User> {
    let Some(rows) = cache_read("friends-cache.json") else {
        return Vec::new();
    };
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some(User {
                id: f["id"].as_u64()?,
                name: f["name"].as_str().unwrap_or_default().to_string(),
                display: f["display"].as_str().unwrap_or_default().to_string(),
                headshot: f["headshot"].as_str().unwrap_or_default().to_string(),
                presence: f["presence"].as_u64().unwrap_or(0) as u8,
                location: f["location"].as_str().unwrap_or_default().to_string(),
                place_id: f["place"].as_u64(),
                verified: f["verified"].as_bool().unwrap_or(false),
            })
        })
        // Skip nameless leftovers from an old bad fetch: they rendered as "Player".
        .filter(|u: &User| !u.name.is_empty())
        .collect()
}

/// Visit counts for a batch of universes (one request per 50 ids, all in parallel).
pub fn fetch_game_visits(ids: &[u64]) -> HashMap<u64, u64> {
    let mut unique: Vec<u64> = ids.iter().copied().filter(|id| *id > 0).collect();
    unique.sort_unstable();
    unique.dedup();
    let chunks: Vec<Vec<u64>> = unique.chunks(50).map(|c| c.to_vec()).collect();
    let fetched: Vec<Option<Value>> = thread::scope(|s| {
        let jobs: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let list = join(chunk);
                s.spawn(move || {
                    get_json(&format!(
                        "https://games.roblox.com/v1/games?universeIds={list}"
                    ))
                    .ok()
                })
            })
            .collect();
        jobs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let mut out = HashMap::new();
    for v in fetched.into_iter().flatten() {
        for g in v["data"].as_array().into_iter().flatten() {
            if let (Some(id), Some(visits)) = (g["id"].as_u64(), g["visits"].as_u64()) {
                out.insert(id, visits);
            }
        }
    }
    out
}
