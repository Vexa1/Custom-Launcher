//! Cursor-following state for the Game Charts feed (`ui/page_charts.rs`).
//!
//! Roblox's explore endpoint returns a handful of "sorts", each carrying its own games, plus a
//! `nextSortsPageToken`. The charts page used to stop after three such pages; this module is
//! what lets it keep walking the cursor for as long as one comes back, without ever asking for
//! the same page twice, repeating a `universe_id`, or hammering a rate-limited endpoint.
//!
//! It is deliberately free of egui and of the network: `api.rs` owns the HTTP calls and hands
//! the decoded page here, which is what makes the pagination, dedupe and back-off rules
//! testable in `vm-tests/`.

use crate::types::{Game, Sort};
use serde_json::Value;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// One `explore-api/v1/get-sorts` page: its sorts and the token for the next one.
#[derive(Debug, Default, Clone)]
pub struct SortsPage {
    pub sorts: Vec<Sort>,
    pub next: Option<String>,
}

/// A failed charts page. Only a `429` is retryable, and its wait comes from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartError {
    pub message: String,
    pub rate_limited: bool,
    pub retry_after_secs: u64,
}

impl ChartError {
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            rate_limited: false,
            retry_after_secs: 0,
        }
    }

    pub fn rate_limited(retry_after_secs: u64) -> Self {
        let secs = retry_after_secs.clamp(1, 120);
        Self {
            message: format!("Roblox is limiting requests. Retrying in {secs}s..."),
            rate_limited: true,
            retry_after_secs: secs,
        }
    }
}

/// "1.2M" style player count, computed once per game instead of on every drawn frame.
pub fn compact_count(n: u64) -> String {
    match n {
        0 => String::new(),
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// The text a filter box shows for a value: blank at zero, so the hint is what the user sees.
pub fn filter_text(n: u64) -> String {
    if n == 0 {
        String::new()
    } else {
        compact_count(n)
    }
}

/// Parses a typed filter amount: digits, thousands separators, a trailing `%`, and shorthand
/// suffixes (`1.5k`, `2m`, `1b`). Returns `None` for text that is not a number yet, which is
/// what lets a half-typed box keep its characters instead of being wiped.
pub fn parse_filter_amount(raw: &str) -> Option<u64> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '_' | ' ' | '\u{00a0}' | '%' | '$'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let mut number = cleaned;
    let multiplier = match number.chars().last()?.to_ascii_lowercase() {
        'k' => {
            number.pop();
            1e3
        }
        'm' => {
            number.pop();
            1e6
        }
        'b' => {
            number.pop();
            1e9
        }
        _ => 1.0,
    };
    if number.is_empty() {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier).round() as u64)
}

/// One filter's text buffer.
///
/// The buffer is owned by the app and lives across frames, which is the whole point: a box the
/// user is typing in must never be rewritten from the slider underneath it. Only an explicit
/// slider move (while the box is unfocused) or the caret leaving the box changes the text.
#[derive(Debug, Default, Clone)]
pub struct FilterBox {
    pub text: String,
}

impl FilterBox {
    /// What the box currently means. `None` = not a number (yet), leave the filter alone.
    pub fn edited_value(&self) -> Option<u64> {
        if self.text.trim().is_empty() {
            return Some(0);
        }
        parse_filter_amount(&self.text)
    }

    /// What a *ceiling* box (\"Max Players\") means. An empty box, `Max` or `0` is no upper
    /// limit at all: `Some(None)`. `None` means the text is not a number yet, so the filter is
    /// left alone - the same rule that lets a half-typed minimum keep its characters.
    pub fn edited_ceiling(&self) -> Option<Option<u64>> {
        let text = self.text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("max") {
            return Some(None);
        }
        match parse_filter_amount(text)? {
            0 => Some(None),
            n => Some(Some(n)),
        }
    }

    /// Mirrors a slider value into the text. A focused box is left exactly as the user typed it.
    pub fn sync_from_slider(&mut self, value: u64, focused: bool) {
        if !focused {
            self.text = filter_text(value);
        }
    }

    /// Formats the box once the user is done with it.
    pub fn normalize(&mut self, value: u64) {
        self.text = filter_text(value);
    }
}

/// Fills a game's card labels. Cards read these instead of formatting numbers per frame.
pub fn fill_labels(g: &mut Game) {
    let votes = g.up + g.down;
    g.rating_text = if votes == 0 {
        "--".to_string()
    } else {
        format!("{}%", g.up * 100 / votes)
    };
    g.players_text = compact_count(g.playing);
}

pub fn game_from_explore(v: &Value) -> Option<Game> {
    if v["isSponsored"].as_bool() == Some(true) {
        return None;
    }
    let mut game = Game {
        universe_id: v["universeId"].as_u64()?,
        place_id: v["rootPlaceId"].as_u64()?,
        name: v["name"].as_str()?.to_string(),
        creator: v["creatorName"].as_str().unwrap_or("").to_string(),
        creator_verified: v["creatorHasVerifiedBadge"].as_bool().unwrap_or(false),
        playing: v["playerCount"].as_u64().unwrap_or(0),
        up: v["totalUpVotes"].as_u64().unwrap_or(0),
        down: v["totalDownVotes"].as_u64().unwrap_or(0),
        description: v["description"].as_str().unwrap_or("").to_string(),
        ..Game::default()
    };
    fill_labels(&mut game);
    Some(game)
}

/// Decodes one explore page. Sponsored entries and empty sorts are dropped here, so callers
/// never see a card with no games behind it.
pub fn parse_sorts_page(v: &Value) -> SortsPage {
    let mut sorts = Vec::new();
    for s in v["sorts"].as_array().into_iter().flatten() {
        if s["contentType"].as_str() != Some("Games") {
            continue;
        }
        let title = s["sortDisplayName"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        let games: Vec<Game> = s["games"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(game_from_explore)
            .collect();
        if games.is_empty() {
            continue;
        }
        sorts.push(Sort { title, games });
    }
    let next = v["nextSortsPageToken"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    SortsPage { sorts, next }
}

/// The rows of a fixed-height grid that a viewport can actually show.
///
/// `grid_top` is where the grid's first row starts in screen space; row `i` starts at
/// `grid_top + i * (row_h + gap)`. One row of slack is included on each side so a partially
/// visible card is never skipped, which is what keeps a 10,000-row feed cheap: callers build
/// only the rows in the returned range and reserve space for the rest.
pub fn visible_rows(
    clip_top: f32,
    clip_bottom: f32,
    grid_top: f32,
    row_h: f32,
    gap: f32,
    rows: usize,
) -> std::ops::Range<usize> {
    if rows == 0 || !row_h.is_finite() || row_h <= 0.0 {
        return 0..0;
    }
    let step = (row_h + gap).max(1.0);
    let first = (((clip_top - grid_top) / step).floor().max(0.0) as usize).min(rows);
    if first >= rows {
        return rows..rows;
    }
    let visible = ((clip_bottom - clip_top).max(0.0) / step).ceil() as usize;
    // Rows just above the viewport are built too: a flick up has to find its cards, not an
    // empty band that fills in a frame later.
    let start = first.saturating_sub(crate::thumb_pipeline::LOOKAHEAD_ROWS);
    let last = (first + visible + crate::thumb_pipeline::LOOKAHEAD_ROWS).min(rows);
    start..last
}

/// The feed's cursor and its safety rules.
///
/// `absorb` is the only way games enter the catalogue: it drops repeats by `universe_id`, keeps
/// existing sort rows in place (their games are appended) and advances the cursor exactly once
/// per page. `can_fetch` is the in-flight/back-off guard that keeps a fast scroll from firing
/// the same cursor twice.
#[derive(Debug, Default)]
pub struct ChartFeed {
    seen: HashSet<u64>,
    pub next: Option<String>,
    /// Unique games absorbed so far.
    pub loaded: usize,
    /// Pages absorbed so far.
    pub pages: usize,
    /// The API returned no cursor: there is nothing more to ask for.
    pub exhausted: bool,
    pub in_flight: bool,
    pub error: Option<String>,
    pub rate_limited: bool,
    retry_at: Option<Instant>,
}

impl ChartFeed {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Merges the first page of a fresh sync, replacing whatever was loaded before.
    pub fn absorb_initial(&mut self, into: &mut Vec<Sort>, page: SortsPage) {
        self.reset();
        into.clear();
        self.absorb(into, page);
    }

    /// Merges one page into `into` and returns the games that were new.
    ///
    /// A repeated `universe_id` is dropped even when it arrives under a different sort, which is
    /// exactly what the charts grid must not draw twice.
    pub fn absorb(&mut self, into: &mut Vec<Sort>, page: SortsPage) -> Vec<u64> {
        let mut fresh = Vec::new();
        for sort in page.sorts {
            let existing = into
                .iter_mut()
                .find(|s| s.title.eq_ignore_ascii_case(&sort.title));
            let target = match existing {
                Some(sort_row) => sort_row,
                None => {
                    into.push(Sort {
                        title: sort.title,
                        games: Vec::new(),
                    });
                    into.last_mut().expect("just pushed")
                }
            };
            for game in sort.games {
                if self.seen.insert(game.universe_id) {
                    fresh.push(game.universe_id);
                    target.games.push(game);
                }
            }
        }
        // A page that adds nothing but still carries a cursor is not progress: stop rather than
        // walk a cursor that only returns duplicates, which would spin the prefetch forever.
        let grew = !fresh.is_empty();
        self.pages += 1;
        self.loaded = self.seen.len();
        self.error = None;
        self.rate_limited = false;
        self.retry_at = None;
        match page.next {
            Some(token) if !token.is_empty() && grew => self.next = Some(token),
            Some(_) => {
                self.next = None;
                self.exhausted = true;
            }
            None => {
                self.next = None;
                self.exhausted = true;
            }
        }
        fresh
    }

    /// Whether another page may be requested right now.
    pub fn can_fetch(&self, now: Instant) -> bool {
        !self.in_flight
            && !self.exhausted
            && self.next.is_some()
            && self.retry_at.map_or(true, |at| now >= at)
    }

    /// Records a `429` and the server's wait, so nothing is requested until it passes.
    pub fn note_rate_limit(&mut self, retry_after_secs: u64, now: Instant) {
        let error = ChartError::rate_limited(retry_after_secs);
        self.retry_at = Some(now + Duration::from_secs(error.retry_after_secs));
        self.rate_limited = true;
        self.error = Some(error.message);
    }

    pub fn note_error(&mut self, message: impl Into<String>) {
        self.rate_limited = false;
        self.retry_at = None;
        self.error = Some(message.into());
    }

    /// The manual "Retry" link: forgets the error and the back-off window.
    pub fn clear_error(&mut self) {
        self.error = None;
        self.rate_limited = false;
        self.retry_at = None;
    }

    /// Seconds left on a `429` wait, for the footer's countdown.
    pub fn wait_left(&self, now: Instant) -> u64 {
        self.retry_at
            .map(|at| at.saturating_duration_since(now).as_secs().max(1))
            .unwrap_or(0)
    }
}
