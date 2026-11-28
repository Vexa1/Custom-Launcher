//! Stale-while-revalidate: a refresh must never blank a populated page.
//!
//! `src/live_refresh.rs` is the real module, included by path. These tests model what Home,
//! Charts and Game Details hold - a visible `Vec<Game>` plus a thumbnail cache keyed by
//! `universe_id` - and check that a background refresh leaves both intact while it is in
//! flight, then merges the new numbers in place.

use creblox_vm_tests::live_refresh::{self, Freshness, Phase};
use creblox_vm_tests::types::{Game, Sort};
use std::collections::{HashMap, HashSet};

fn game(id: u64, playing: u64) -> Game {
    Game {
        universe_id: id,
        place_id: id * 10,
        name: format!("Game {id}"),
        creator: "Builder".to_string(),
        creator_verified: false,
        playing,
        up: playing / 10,
        down: playing / 100,
        description: String::new(),
        players_text: format!("{playing} playing"),
        rating_text: "90%".to_string(),
    }
}

/// The launcher's thumbnail pool: `universe_id -> image URL`, exactly what `request_thumbs`
/// consults before spending a request.
fn thumbs(games: &[Game]) -> HashMap<u64, String> {
    games
        .iter()
        .map(|g| (g.universe_id, format!("https://cdn.example/{}.png", g.universe_id)))
        .collect()
}

/// Mirrors `request_thumbs`: only ids without a URL are fetched.
fn would_fetch(cache: &HashMap<u64, String>, games: &[Game]) -> Vec<u64> {
    games
        .iter()
        .map(|g| g.universe_id)
        .filter(|id| !cache.contains_key(id))
        .collect()
}

#[test]
fn a_refresh_over_populated_home_keeps_cards_and_textures() {
    let mut recent = vec![game(1, 10), game(2, 20), game(3, 30)];
    let visible_before: Vec<u64> = recent.iter().map(|g| g.universe_id).collect();
    let cache = thumbs(&recent);
    let mut freshness = Freshness::default();
    freshness.succeed();
    assert_eq!(freshness.phase(), Phase::Ready);

    // A background refresh starts. Nothing is cleared and no skeleton appears.
    freshness.request();
    assert!(freshness.is_silent());
    assert!(!freshness.skeleton(recent.len()));
    assert_eq!(
        recent.iter().map(|g| g.universe_id).collect::<Vec<_>>(),
        visible_before,
        "the row is untouched while the fetch is in flight"
    );

    // The answer arrives: two games changed, one is new.
    let fresh = vec![game(2, 220), game(1, 110), game(4, 40)];
    let stats = live_refresh::apply_refresh(&mut recent, Some(fresh));
    freshness.succeed();

    assert_eq!(stats.updated, 2);
    assert_eq!(stats.added, 1);
    assert_eq!(recent.len(), 4);
    // Positions are kept: the first card is still the first card.
    assert_eq!(
        recent.iter().map(|g| g.universe_id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    // Numbers are merged in place, formatted labels included.
    assert_eq!(recent[0].playing, 110);
    assert_eq!(recent[0].players_text, "110 playing");
    assert_eq!(recent[1].playing, 220);
    assert_eq!(recent[2].playing, 30, "a game missing from the answer keeps its numbers");
    // No duplicates, ever.
    let ids: HashSet<u64> = recent.iter().map(|g| g.universe_id).collect();
    assert_eq!(ids.len(), recent.len());

    // Textures are keyed by universe_id, so every game that was loaded before the refresh
    // still resolves its image, and only the newcomer costs a download.
    for id in [1, 2, 3] {
        assert!(cache.contains_key(&id), "texture for {id} was dropped");
    }
    assert_eq!(would_fetch(&cache, &recent), vec![4]);
}

#[test]
fn an_empty_or_failed_refresh_never_empties_the_row() {
    let mut recent = vec![game(1, 10), game(2, 20)];
    let mut freshness = Freshness::default();
    freshness.succeed();

    let kept = live_refresh::apply_refresh(&mut recent, None);
    assert_eq!(kept.kept, 2);
    assert_eq!(recent.len(), 2, "a failed fetch leaves the cards alone");

    let kept = live_refresh::apply_refresh(&mut recent, Some(Vec::new()));
    assert_eq!(kept.kept, 2);
    assert_eq!(recent.len(), 2, "an empty answer is not a reason to blank the row");

    freshness.request();
    freshness.fail();
    assert_eq!(
        freshness.phase(),
        Phase::Ready,
        "a failed refresh over good data still has good data to show"
    );
    assert!(!freshness.skeleton(recent.len()));

    // A first-ever load with nothing behind it is the only skeleton.
    let fresh_boot = Freshness::default();
    assert!(fresh_boot.skeleton(0));
    assert_eq!(fresh_boot.phase(), Phase::FirstLoad);
    assert!(!fresh_boot.skeleton(5));
    let mut failed_boot = Freshness::default();
    failed_boot.request();
    failed_boot.fail();
    assert_eq!(failed_boot.phase(), Phase::Failed);
    assert!(!failed_boot.skeleton(0), "an error is drawn as an error, not a spinner");
}

#[test]
fn a_first_load_becomes_silent_once_it_has_data() {
    let mut freshness = Freshness::default();
    freshness.request();
    assert_eq!(freshness.phase(), Phase::FirstLoad);
    assert!(freshness.skeleton(0));

    freshness.succeed();
    assert_eq!(freshness.phase(), Phase::Ready);
    assert!(!freshness.skeleton(0), "data is on screen now");

    freshness.request();
    assert_eq!(freshness.phase(), Phase::Silent);
    assert!(freshness.is_silent());
    freshness.succeed();
    assert_eq!(freshness.phase(), Phase::Ready);

    freshness.reset();
    assert_eq!(freshness.phase(), Phase::FirstLoad, "a logout forgets the data");
}

#[test]
fn a_charts_refresh_merges_ranks_in_place() {
    let mut sorts = vec![
        Sort {
            title: "Top Trending".to_string(),
            games: vec![game(1, 100), game(2, 200)],
        },
        Sort {
            title: "Top Earning".to_string(),
            games: vec![game(3, 300)],
        },
    ];
    let before: Vec<u64> = sorts[0].games.iter().map(|g| g.universe_id).collect();

    // A fresh catalogue: same sorts, changed counts, one new game, one brand-new sort.
    let fresh = vec![
        Sort {
            title: "Top Trending".to_string(),
            games: vec![game(1, 900), game(4, 400)],
        },
        Sort {
            title: "Top Earning".to_string(),
            games: vec![game(3, 330)],
        },
        Sort {
            title: "New And Notable".to_string(),
            games: vec![game(5, 500)],
        },
    ];
    let stats = live_refresh::merge_sorts(&mut sorts, fresh);

    assert_eq!(sorts.len(), 3, "the new sort is appended, none disappear");
    assert_eq!(stats.updated, 2, "1 and 3 took the new numbers");
    assert_eq!(stats.added, 2, "4 and the new sort's 5 are new cards");
    // The row on screen keeps its order, so nothing jumps while the user is reading it.
    assert_eq!(
        sorts[0].games.iter().map(|g| g.universe_id).collect::<Vec<_>>(),
        vec![1, 2, 4]
    );
    assert_eq!(before, vec![1, 2]);
    assert_eq!(sorts[0].games[0].playing, 900);
    assert_eq!(sorts[1].games[0].playing, 330);
    assert_eq!(sorts[2].games[0].universe_id, 5);
}

#[test]
fn game_details_revalidate_without_losing_the_open_game() {
    // What `open_game_details` models: the game on screen, its details, and its hero texture.
    let mut open = game(42, 500);
    let mut details_playing = open.playing;
    let hero = thumbs(std::slice::from_ref(&open));
    let mut freshness = Freshness::default();
    freshness.succeed();

    freshness.request();
    assert!(freshness.is_silent());
    assert_eq!(open.universe_id, 42, "the page does not change game");
    assert!(hero.contains_key(&42), "the hero image is still cached");
    assert_eq!(would_fetch(&hero, std::slice::from_ref(&open)), Vec::<u64>::new());

    // The revalidated numbers land.
    let fresh = game(42, 650);
    details_playing = fresh.playing;
    open = fresh;
    freshness.succeed();

    assert_eq!(details_playing, 650);
    assert_eq!(open.name, "Game 42", "identity does not change");
    assert_eq!(open.universe_id, 42);
    assert_eq!(freshness.phase(), Phase::Ready);
}
