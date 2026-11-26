//! Capacity-aware server selection, driven through the real policy in `src/party_server.rs`.
//!
//! These feed the same mock Roblox server lists the API would return and check the three
//! promises the party button makes: never a room that is too small, the best room a page can
//! offer, and a second page only when the first has nothing.

use std::cell::RefCell;

use creblox_vm_tests::party_server::{pick_party_server, select_for_party, PartyServerError};
use creblox_vm_tests::types::ServerItem;

fn server(id: &str, playing: u64, max_players: u64, ping: Option<u64>) -> ServerItem {
    ServerItem {
        id: id.to_string(),
        playing,
        max_players,
        ping,
        fps: Some(60.0),
    }
}

/// Records which page counts the policy asked for.
fn recorder<'a>(
    pages: &'a RefCell<Vec<usize>>,
    by_pages: impl Fn(usize) -> Vec<ServerItem> + 'a,
) -> impl FnMut(usize) -> Result<Vec<ServerItem>, String> + 'a {
    move |count| {
        pages.borrow_mut().push(count);
        Ok(by_pages(count))
    }
}

#[test]
fn a_party_of_four_skips_every_room_with_fewer_than_four_seats() {
    let listed = vec![
        server("two-seats", 2, 4, Some(5)),
        server("three-seats", 3, 6, Some(6)),
        server("roomy", 4, 10, Some(80)),
        server("healthy", 3, 8, Some(30)),
    ];
    let picked = pick_party_server(&listed, 4).expect("a room with four seats");
    assert_eq!(picked.job_id, "healthy");
    assert_eq!(picked.free_slots, 5);
    assert_eq!(picked.playing, 3);
    assert_eq!(picked.max_players, 8);
    assert!(
        picked.buffered,
        "five seats for a party of four keeps a spare"
    );
}

#[test]
fn the_lowest_ping_wins_inside_the_same_buffering_class() {
    let exact = vec![
        server("exact-slow", 4, 8, Some(40)),
        server("exact-fast", 4, 8, Some(7)),
        server("exact-noping", 4, 8, None),
    ];
    assert_eq!(
        pick_party_server(&exact, 4).expect("an exact fit").job_id,
        "exact-fast"
    );

    let buffered = vec![
        server("buffered-slow", 2, 8, Some(200)),
        server("buffered-fast", 2, 8, Some(20)),
    ];
    assert_eq!(
        pick_party_server(&buffered, 4)
            .expect("a buffered room")
            .job_id,
        "buffered-fast"
    );
}

/// The documented policy: a spare seat beats an exactly-big-enough room even at worse ping,
/// because a random player cannot take the last slot while the party is still joining.
#[test]
fn a_spare_seat_is_preferred_over_a_tighter_room() {
    let listed = vec![
        server("exact", 4, 8, Some(5)),
        server("spare-seat", 3, 8, Some(900)),
    ];
    let picked = pick_party_server(&listed, 4).expect("a room");
    assert_eq!(picked.job_id, "spare-seat");
    assert!(picked.buffered);
}

#[test]
fn the_livelier_room_wins_when_ping_and_buffering_tie() {
    let listed = vec![
        server("empty", 0, 10, Some(50)),
        server("busy", 5, 10, Some(50)),
    ];
    assert_eq!(
        pick_party_server(&listed, 4).expect("a room").job_id,
        "busy"
    );
}

#[test]
fn page_one_is_enough_when_it_has_a_room() {
    let pages = RefCell::new(Vec::new());
    let picked = select_for_party(
        4,
        recorder(&pages, |_| vec![server("first-page", 4, 8, Some(20))]),
    )
    .expect("a room on page one");
    assert_eq!(picked.job_id, "first-page");
    assert_eq!(*pages.borrow(), vec![1], "one request is all a fit needs");
}

#[test]
fn a_second_page_is_fetched_when_page_one_has_no_fit() {
    let pages = RefCell::new(Vec::new());
    let picked = select_for_party(
        4,
        recorder(&pages, |count| {
            if count == 1 {
                // Full and too-small lobbies only: exactly the case a cursor is for.
                vec![
                    server("full", 8, 8, Some(10)),
                    server("small", 2, 4, Some(11)),
                    server("tight", 6, 8, Some(12)),
                ]
            } else {
                vec![
                    server("full", 8, 8, Some(10)),
                    server("small", 2, 4, Some(11)),
                    server("tight", 6, 8, Some(12)),
                    server("second-page", 2, 12, Some(150)),
                ]
            }
        }),
    )
    .expect("a room from the wider sweep");
    assert_eq!(picked.job_id, "second-page");
    assert_eq!(
        *pages.borrow(),
        vec![1, 3],
        "page one first, then the sweep that reaches the cursor's pages"
    );
}

#[test]
fn a_game_that_can_never_hold_the_party_says_so() {
    let pages = RefCell::new(Vec::new());
    let error = select_for_party(
        4,
        recorder(&pages, |_| vec![server("duel", 0, 2, Some(20))]),
    )
    .expect_err("nothing can hold four");
    assert_eq!(
        error,
        PartyServerError::TooSmall {
            max_players: 2,
            party_size: 4
        }
    );
    assert!(error.message().contains("2 players"), "{}", error.message());
}

#[test]
fn a_full_game_reports_the_best_free_seats() {
    let error = select_for_party(4, |_| {
        Ok(vec![
            server("busy", 6, 8, Some(20)),
            server("busier", 5, 8, Some(30)),
        ])
    })
    .expect_err("three seats is not four");
    assert_eq!(
        error,
        PartyServerError::NoCapacity {
            party_size: 4,
            best_free: 3,
            listed: 2
        }
    );
}

#[test]
fn a_listing_failure_is_passed_through_untouched() {
    let pages = RefCell::new(Vec::new());
    let error = select_for_party(4, |count| {
        pages.borrow_mut().push(count);
        Err("403 Forbidden".to_string())
    })
    .expect_err("the request failed");
    assert_eq!(error, PartyServerError::Api("403 Forbidden".to_string()));
    assert_eq!(
        *pages.borrow(),
        vec![1],
        "a failure never triggers the sweep"
    );
}

#[test]
fn servers_without_a_player_cap_are_ignored() {
    let listed = vec![server("unknown-cap", 0, 0, Some(5))];
    assert!(pick_party_server(&listed, 4).is_none());
    let error = select_for_party(4, move |_| Ok(listed.clone())).expect_err("no cap, no room");
    assert_eq!(
        error,
        PartyServerError::NoCapacity {
            party_size: 4,
            best_free: 0,
            listed: 1
        }
    );
}

#[test]
fn a_party_of_one_still_avoids_the_single_seat_room() {
    let listed = vec![
        server("last-seat", 7, 8, Some(5)),
        server("open", 1, 8, Some(60)),
    ];
    let picked = pick_party_server(&listed, 1).expect("a room");
    assert_eq!(picked.job_id, "open", "a last seat can be taken mid-join");
    assert_eq!(picked.free_slots, 7);
}

/// The policy only ships if the API wrapper actually calls it with the real listing.
#[test]
fn find_server_for_party_is_wired_to_the_tested_policy() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/api.rs"))
        .expect("the launcher's api.rs");
    let start = source
        .find("pub fn find_server_for_party(")
        .expect("find_server_for_party exists");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("the function ends")];
    assert!(
        body.contains("party_server::select_for_party(party_size,"),
        "find_server_for_party must delegate to the tested policy:\n{body}"
    );
    assert!(
        body.contains("fetch_public_server_pool(place_id, \"Desc\", pages, cookie)"),
        "the real public listing must be what the policy sees:\n{body}"
    );
}
