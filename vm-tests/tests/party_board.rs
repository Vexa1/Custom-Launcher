//! Ready Check and Game Vote state, without sockets.
//!
//! The board is what the host broadcasts and what the party bar renders, so its transitions
//! are pinned here: a check that runs for ten seconds, one answer per member, at most three
//! suggested games, one vote per member, and a tally that never reshuffles on a repaint.

use creblox_vm_tests::party::{PartyBoard, PartySuggestion};

fn game(place_id: u64, name: &str) -> PartySuggestion {
    PartySuggestion {
        place_id,
        universe_id: place_id * 10,
        name: name.to_string(),
        by: 1,
    }
}

#[test]
fn a_ready_check_runs_for_ten_seconds_and_then_clears_itself() {
    let mut board = PartyBoard::default();
    assert!(!board.ready_check_active());
    assert_eq!(board.ready_seconds_left(0), 0);

    board.start_ready_check(7, 1_000);
    assert!(board.ready_check_active());
    assert_eq!(board.ready_by, 7);
    assert_eq!(board.ready_seconds_left(1_000), 10);
    assert_eq!(board.ready_seconds_left(1_001), 10);
    assert_eq!(board.ready_seconds_left(10_000), 1);
    assert_eq!(board.ready_seconds_left(10_999), 1);
    assert_eq!(board.ready_seconds_left(11_000), 0);
    assert_eq!(board.ready_seconds_left(99_000), 0);

    assert!(!board.expire(10_999), "still running");
    assert!(board.expire(11_000), "the deadline clears it");
    assert!(!board.ready_check_active());
    assert_eq!(board.ready_by, 0);
    assert_eq!(board.ready_count(), (0, 0));
    assert!(!board.expire(12_000), "expiring twice is a no-op");
}

#[test]
fn each_member_answers_once_and_can_flip_their_answer() {
    let mut board = PartyBoard::default();
    board.start_ready_check(7, 0);
    assert_eq!(board.is_ready(8), None);

    assert!(board.set_ready(8, true));
    assert_eq!(board.is_ready(8), Some(true));
    assert!(!board.set_ready(8, true), "same answer changes nothing");
    assert_eq!(board.ready_count(), (1, 1));

    assert!(board.set_ready(8, false));
    assert_eq!(board.is_ready(8), Some(false));
    assert_eq!(board.ready_count(), (0, 1));

    assert!(board.set_ready(9, true));
    assert!(board.set_ready(10, true));
    assert_eq!(board.ready_count(), (2, 3));

    // A new check asks everybody again instead of carrying old answers over.
    board.start_ready_check(9, 5_000);
    assert_eq!(board.ready_count(), (0, 0));
    assert_eq!(board.ready_by, 9);
}

#[test]
fn a_poll_takes_three_games_and_refuses_duplicates() {
    let mut board = PartyBoard::default();
    assert!(board.suggest(game(1, "Doors")));
    assert!(board.suggest(game(2, "Bedwars")));
    assert!(board.suggest(game(3, "Deepwoken")));
    assert_eq!(board.suggestions.len(), PartyBoard::MAX_SUGGESTIONS);
    assert!(!board.suggest(game(4, "Adopt Me")), "the poll is full");
    assert!(!board.suggest(game(1, "Doors again")), "already on the poll");
    assert!(!board.suggest(game(0, "no place")), "a game needs a place");
    assert_eq!(board.suggestions.len(), 3);
}

#[test]
fn votes_are_one_per_member_and_only_for_suggested_games() {
    let mut board = PartyBoard::default();
    board.suggest(game(1, "Doors"));
    board.suggest(game(2, "Bedwars"));

    assert!(!board.vote(8, 99), "not on the poll");
    assert_eq!(board.my_vote(8), None);

    assert!(board.vote(8, 1));
    assert_eq!(board.my_vote(8), Some(1));
    assert_eq!(board.votes_for(1), 1);
    assert!(!board.vote(8, 1), "same vote changes nothing");

    assert!(board.vote(8, 2), "a member can change their mind");
    assert_eq!(board.my_vote(8), Some(2));
    assert_eq!(board.votes_for(1), 0);
    assert_eq!(board.votes_for(2), 1);
    assert_eq!(board.votes.len(), 1, "still one vote per member");

    assert!(board.vote(9, 2));
    assert_eq!(board.votes_for(2), 2);
    assert_eq!(board.votes.len(), 2);
}

#[test]
fn the_tally_sorts_by_votes_and_ties_keep_suggestion_order() {
    let mut board = PartyBoard::default();
    board.suggest(game(1, "Doors"));
    board.suggest(game(2, "Bedwars"));
    board.suggest(game(3, "Deepwoken"));

    assert!(board.winner().is_none(), "nobody has voted yet");
    assert_eq!(board.tally(), vec![(1, 0), (2, 0), (3, 0)]);

    board.vote(8, 3);
    board.vote(9, 3);
    board.vote(10, 1);
    assert_eq!(board.tally(), vec![(3, 2), (1, 1), (2, 0)]);
    assert_eq!(board.winner().map(|g| g.name.as_str()), Some("Deepwoken"));

    // A tie goes to the game that was suggested first, so the winner cannot flip between
    // two identical tallies.
    board.vote(11, 1);
    assert_eq!(board.tally(), vec![(1, 2), (3, 2), (2, 0)]);
    assert_eq!(board.winner().map(|g| g.name.as_str()), Some("Doors"));
}

#[test]
fn a_board_survives_a_json_round_trip() {
    let mut board = PartyBoard::default();
    board.start_ready_check(7, 1_000);
    board.set_ready(8, true);
    board.suggest(game(123, "Doors"));
    board.vote(8, 123);

    let json = serde_json::to_string(&board).unwrap();
    let back: PartyBoard = serde_json::from_str(&json).unwrap();
    assert_eq!(back, board);

    // A older peer that has no board field yet still reads a frame that contains one: serde
    // ignores unknown keys, which is what keeps 0.5.5 clients in the room.
    let legacy: Result<LegacyMembers, _> = serde_json::from_str(
        r#"{"t":"members","host":7,"max":8,"members":[],"board":{"ready_until_ms":1}}"#,
    );
    assert_eq!(legacy.unwrap().host, 7);
}

#[derive(serde::Deserialize)]
struct LegacyMembers {
    host: u64,
    #[allow(dead_code)]
    max: usize,
}
