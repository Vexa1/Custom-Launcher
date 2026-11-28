//! The one switch that stops every periodic fetch, and what a background merge may change.
//!
//! `refresh_policy::should_refresh` is the gate the app asks before every timer-driven fetch,
//! and `live_refresh` is the only thing a background answer is allowed to touch. These cases
//! pin both: with the switch off no elapsed tick fetches anything, and with it on a merge only
//! changes numbers - every card keeps its position and therefore its cached texture.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use creblox_vm_tests::live_refresh::{apply_refresh, patch_games, patch_sorts};
use creblox_vm_tests::refresh_policy::should_refresh;
use creblox_vm_tests::types::{Game, Sort};

fn game(universe: u64, playing: u64) -> Game {
    Game {
        universe_id: universe,
        place_id: universe * 10,
        name: format!("Game {universe}"),
        creator: "Someone".to_string(),
        playing,
        up: playing / 2,
        down: playing / 10,
        players_text: format!("{playing} playing"),
        rating_text: "90%".to_string(),
        ..Default::default()
    }
}

fn ids(games: &[Game]) -> Vec<u64> {
    games.iter().map(|g| g.universe_id).collect()
}

// ---------------------------------------------------------------------------
// The switch: off means nothing periodic is fetched, however long it has been.

#[test]
fn with_auto_refresh_off_elapsed_ticks_start_no_fetch() {
    let now = Instant::now();
    let day_ago = Some(now - Duration::from_secs(24 * 60 * 60));
    // Every interval the launcher uses: focus, on-discovery and background cadences.
    for interval in [
        Duration::from_secs(3),
        Duration::from_secs(6),
        Duration::from_secs(20),
        Duration::from_secs(25),
        Duration::from_secs(60),
        Duration::from_secs(90),
    ] {
        assert!(
            !should_refresh(false, day_ago, None, interval, now),
            "a {interval:?} timer must not fetch while the switch is off"
        );
        assert!(
            !should_refresh(false, None, None, interval, now),
            "not even a first tick"
        );
    }
    // A fetch that is still running is irrelevant when nothing may start.
    assert!(!should_refresh(
        false,
        day_ago,
        Some(now - Duration::from_secs(1)),
        Duration::from_secs(1),
        now,
    ));
}

#[test]
fn with_auto_refresh_on_the_same_ticks_do_fetch() {
    let now = Instant::now();
    let day_ago = Some(now - Duration::from_secs(24 * 60 * 60));
    assert!(should_refresh(true, day_ago, None, Duration::from_secs(20), now));
    assert!(should_refresh(true, None, None, Duration::from_secs(20), now));
    // Inside the interval, or while a fetch runs, it waits.
    assert!(!should_refresh(
        true,
        Some(now - Duration::from_secs(2)),
        None,
        Duration::from_secs(20),
        now,
    ));
    assert!(!should_refresh(
        true,
        day_ago,
        Some(now - Duration::from_secs(5)),
        Duration::from_secs(1),
        now,
    ));
}

// ---------------------------------------------------------------------------
// A background merge: numbers change, nothing moves, nothing loses its texture.

#[test]
fn a_background_refresh_keeps_every_card_in_its_slot() {
    let mut shown = vec![game(1, 10), game(2, 20), game(3, 30)];
    // The texture cache is keyed by the URL chosen for a universe id; a reorder would not
    // change it, but a dropped/inserted card would lose its slot.
    let mut textures: HashMap<u64, String> = ids(&shown)
        .into_iter()
        .map(|id| (id, format!("https://rbxthumb/{id}")))
        .collect();
    let before = textures.clone();

    // The answer is re-ordered and carries a newcomer: neither may move a visible card.
    let fresh = vec![game(3, 3_000), game(2, 2_000), game(9, 9_000)];
    let stats = patch_games(&mut shown, &fresh);

    assert_eq!(ids(&shown), vec![1, 2, 3], "order must not change");
    assert_eq!(stats.updated, 2);
    assert_eq!(shown[0].playing, 10, "a card the answer omits keeps its number");
    assert_eq!(shown[1].playing, 2_000);
    assert_eq!(shown[2].playing, 3_000);
    assert!(
        !shown.iter().any(|g| g.universe_id == 9),
        "no card is inserted into a visible row by a background answer"
    );
    assert_eq!(textures, before, "every texture key survives the merge");

    // Rebuilding the id -> URL map from the merged row gives the same keys.
    textures = ids(&shown)
        .into_iter()
        .map(|id| (id, format!("https://rbxthumb/{id}")))
        .collect();
    assert_eq!(textures, before);
}

#[test]
fn a_background_refresh_keeps_chart_rows_in_order() {
    let mut shown = vec![
        Sort {
            title: "Top Trending".to_string(),
            games: vec![game(1, 100), game(2, 200), game(3, 300)],
        },
        Sort {
            title: "Top Playing".to_string(),
            games: vec![game(4, 400)],
        },
    ];
    let fresh = vec![
        Sort {
            title: "Top Trending".to_string(),
            games: vec![game(3, 3_000), game(1, 1_000), game(7, 7_000)],
        },
        Sort {
            title: "Top Playing".to_string(),
        games: vec![game(4, 4_000)],
        },
    ];
    let stats = patch_sorts(&mut shown, &fresh);

    assert_eq!(shown[0].title, "Top Trending");
    assert_eq!(shown[1].title, "Top Playing");
    assert_eq!(ids(&shown[0].games), vec![1, 2, 3], "chart cards stay put");
    assert_eq!(shown[0].games[0].playing, 1_000);
    assert_eq!(shown[0].games[1].playing, 200);
    assert_eq!(shown[0].games[2].playing, 3_000);
    assert_eq!(shown[1].games[0].playing, 4_000);
    assert_eq!(stats.updated, 3, "two in the first row, one in the second");
}

#[test]
fn a_home_row_refresh_keeps_the_shown_order_and_appends_newcomers() {
    let mut row = vec![game(1, 10), game(2, 20)];
    let fresh = vec![game(2, 200), game(5, 500), game(1, 100)];
    apply_refresh(&mut row, Some(fresh));

    // Existing cards keep their positions; a newly returned game lands at the end.
    assert_eq!(ids(&row), vec![1, 2, 5]);
    assert_eq!(row[0].playing, 100);
    assert_eq!(row[1].playing, 200);
    assert_eq!(row[2].playing, 500);
}
