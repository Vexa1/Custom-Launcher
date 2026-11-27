//! The Game Charts feed's headless rules: cursor pagination, dedupe, back-off and the
//! viewport window the grid builds cards from.
//!
//! `src/chart_feed.rs` is the real module, included by path. Nothing here touches the network:
//! the page decoder is fed the same JSON Roblox returns.

use creblox_vm_tests::chart_feed::{
    ChartError, ChartFeed, compact_count, parse_sorts_page, visible_rows,
};
use creblox_vm_tests::types::{Game, Sort};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::{Duration, Instant};

fn game_json(id: u64, playing: u64, up: u64, down: u64) -> Value {
    json!({
        "universeId": id,
        "rootPlaceId": id * 10,
        "name": format!("Game {id}"),
        "creatorName": "Builder",
        "playerCount": playing,
        "totalUpVotes": up,
        "totalDownVotes": down,
    })
}

fn page_json(title: &str, ids: &[u64], next: Option<&str>) -> Value {
    let games: Vec<Value> = ids.iter().map(|&id| game_json(id, 100, 3, 1)).collect();
    let mut v = json!({
        "sorts": [{"contentType": "Games", "sortDisplayName": title, "games": games}]
    });
    if let Some(token) = next {
        v["nextSortsPageToken"] = json!(token);
    }
    v
}

#[test]
fn a_page_parses_sorts_games_and_its_cursor() {
    let v = json!({
        "sorts": [
            {"contentType": "Games", "sortDisplayName": "Top Trending", "games": [
                game_json(1, 1200, 9, 3),
                {"universeId": 99, "rootPlaceId": 990, "name": "Ad", "isSponsored": true}
            ]},
            {"contentType": "Users", "sortDisplayName": "Not games", "games": []},
            {"contentType": "Games", "sortDisplayName": "Empty", "games": []}
        ],
        "nextSortsPageToken": "  page-2  "
    });
    let page = parse_sorts_page(&v);
    assert_eq!(page.sorts.len(), 1, "only non-empty game sorts survive");
    assert_eq!(page.sorts[0].title, "Top Trending");
    assert_eq!(page.sorts[0].games.len(), 1, "sponsored cards are not games");
    assert_eq!(page.sorts[0].games[0].universe_id, 1);
    assert_eq!(page.sorts[0].games[0].players_text, "1.2K");
    assert_eq!(page.sorts[0].games[0].rating_text, "75%");
    assert_eq!(page.next.as_deref(), Some("page-2"), "the token is trimmed");
}

#[test]
fn the_cursor_walks_past_a_thousand_games_without_duplicates() {
    let mut feed = ChartFeed::default();
    let mut sorts: Vec<Sort> = Vec::new();
    let mut seen = HashSet::new();

    // 25 pages of 50 games: well past the old ~688-game ceiling.
    for page in 0..25u64 {
        let ids: Vec<u64> = (0..50).map(|i| page * 50 + i + 1).collect();
        let token = if page == 24 {
            None
        } else {
            Some(format!("cursor-{}", page + 1))
        };
        let decoded = parse_sorts_page(&page_json("Top Trending", &ids, token.as_deref()));
        let fresh = feed.absorb(&mut sorts, decoded);
        assert!(fresh.iter().all(|id| seen.insert(*id)), "no id repeats");
        feed.in_flight = false;
        if feed.next.is_some() {
            assert!(feed.can_fetch(Instant::now()), "a valid cursor keeps the feed open");
        }
    }

    assert_eq!(feed.loaded, 1_250);
    assert_eq!(feed.pages, 25);
    assert_eq!(seen.len(), 1_250);
    assert!(feed.exhausted, "the last page has no cursor");
    assert!(feed.next.is_none());
    assert!(!feed.can_fetch(Instant::now()), "nothing left to fetch");
    let total: usize = sorts.iter().map(|s| s.games.len()).sum();
    assert_eq!(total, 1_250, "the merged catalogue holds every game once");
}

#[test]
fn a_universe_id_is_rendered_only_once_across_pages_and_sorts() {
    let mut feed = ChartFeed::default();
    let mut sorts = Vec::new();
    feed.absorb(&mut sorts, parse_sorts_page(&page_json("Top Trending", &[1, 2, 3], Some("p2"))));
    feed.absorb(&mut sorts, parse_sorts_page(&page_json("Top Trending", &[3, 4, 5], Some("p3"))));
    // A second sort row that reintroduces an id must not add a card either.
    let other = parse_sorts_page(&page_json("Up & Coming", &[2, 6], None));
    feed.absorb(&mut sorts, other);

    assert_eq!(feed.loaded, 6);
    assert_eq!(sorts.len(), 2, "the existing sort row is reused, not duplicated");
    let ids: Vec<u64> = sorts.iter().flat_map(|s| s.games.iter().map(|g| g.universe_id)).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn repeated_pages_do_not_spin_the_prefetch_forever() {
    let mut feed = ChartFeed::default();
    let mut sorts = Vec::new();
    // The API hands back the same games with a fresh cursor: following it would loop.
    let fresh = feed.absorb(&mut sorts, parse_sorts_page(&page_json("Top", &[7, 8], Some("p2"))));
    assert_eq!(fresh.len(), 2);
    let again = feed.absorb(&mut sorts, parse_sorts_page(&page_json("Top", &[7, 8], Some("p3"))));
    assert!(again.is_empty());
    assert!(feed.exhausted);
    assert!(!feed.can_fetch(Instant::now()));
}

#[test]
fn an_in_flight_page_and_a_429_window_both_hold_the_next_request() {
    let t0 = Instant::now();
    let mut feed = ChartFeed::default();
    let mut sorts = Vec::new();
    feed.absorb(&mut sorts, parse_sorts_page(&page_json("Top", &[1], Some("p2"))));
    assert!(feed.can_fetch(t0));

    feed.in_flight = true;
    assert!(!feed.can_fetch(t0), "the same cursor must not be asked for twice");
    feed.in_flight = false;

    feed.note_rate_limit(0, t0); // the server's value is clamped to a sane minimum
    assert_eq!(feed.wait_left(t0), 1);
    assert!(!feed.can_fetch(t0));
    assert!(!feed.can_fetch(t0 + Duration::from_millis(500)));
    assert!(feed.can_fetch(t0 + Duration::from_secs(1)), "the feed resumes by itself");

    feed.note_rate_limit(90, t0);
    assert_eq!(feed.wait_left(t0), 90);
    assert!(feed.error.as_deref().unwrap().contains("90s"));

    // The manual Retry link clears the window immediately.
    feed.clear_error();
    assert!(feed.can_fetch(t0));
    assert!(feed.error.is_none());
}

#[test]
fn failures_are_kept_as_a_message_without_a_retry_window() {
    let mut feed = ChartFeed::default();
    let mut sorts = Vec::new();
    feed.absorb(&mut sorts, parse_sorts_page(&page_json("Top", &[1], Some("p2"))));
    feed.note_error("HTTP 500");
    assert_eq!(feed.error.as_deref(), Some("HTTP 500"));
    assert!(!feed.rate_limited);
    assert!(feed.can_fetch(Instant::now()), "a hard error may be retried by hand");
    assert!(ChartError::rate_limited(0).retry_after_secs >= 1);
    assert!(!ChartError::other("x").rate_limited);
}

#[test]
fn card_labels_are_formatted_once() {
    assert_eq!(compact_count(999), "999");
    assert_eq!(compact_count(1_200), "1.2K");
    assert_eq!(compact_count(3_400_000), "3.4M");
    assert_eq!(compact_count(0), "");
    let g = &parse_sorts_page(&page_json("Top", &[5], None)).sorts[0].games[0];
    assert_eq!(g.players_text, "100");
    assert_eq!(g.rating_text, "75%");
    // A game with no votes shows the same "--" the details page does.
    let voted: Game = parse_sorts_page(&json!({"sorts": [{"contentType": "Games",
        "sortDisplayName": "T", "games": [game_json(6, 0, 0, 0)]}]}))
    .sorts[0]
        .games[0]
        .clone();
    assert_eq!(voted.rating_text, "--");
    assert_eq!(voted.players_text, "");
}

#[test]
fn only_the_viewport_rows_are_built_in_a_ten_thousand_row_feed() {
    // The grid's card row is ~140 px tall with an 8 px gap inside an 800 px viewport.
    let all = visible_rows(0.0, 800.0, 0.0, 140.0, 8.0, 10_000);
    assert!(all.start == 0);
    assert!(all.len() < 12, "a 10,000-row feed lays out a screenful, not 10,000 rows");

    // Scrolled far down: only rows around the viewport, still a screenful.
    let deep = visible_rows(50_000.0, 50_800.0, 0.0, 140.0, 8.0, 10_000);
    assert!(deep.start > 300, "deep rows start where the viewport is");
    assert!(deep.len() < 12);
    assert!(deep.end <= 10_000);

    // Scrolled past the end, and an empty feed, both produce an empty range.
    assert!(visible_rows(2_000_000.0, 2_000_800.0, 0.0, 140.0, 8.0, 10_000).is_empty());
    assert!(visible_rows(0.0, 800.0, 0.0, 140.0, 8.0, 0).is_empty());
    // A zero-height row (unmeasured) never indexes anything.
    assert!(visible_rows(0.0, 800.0, 0.0, 0.0, 8.0, 10).is_empty());
}
