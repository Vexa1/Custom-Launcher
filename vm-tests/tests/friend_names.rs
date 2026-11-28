//! The Home Friends strip's names.
//!
//! The bug this guards: `GET friends.roblox.com/v1/users/{id}/friends` started answering rows
//! with only an `id`, so every bubble rendered "Player" even though headshots and presences
//! arrived. `src/friend_names.rs` is the real module, included by path; these tests feed it the
//! exact payload shapes the loader sees, with no network in the loop.

use creblox_vm_tests::friend_names::{self, FriendName, NameBook, NAME_BATCH};
use serde_json::json;

fn rows_of(payload: &serde_json::Value) -> Vec<FriendName> {
    friend_names::parse_rows(payload)
}

#[test]
fn an_id_only_friends_payload_resolves_to_real_names() {
    // What the endpoint answers now: ids, nothing else.
    let friends = json!({
        "data": [{ "id": 11 }, { "id": 22 }, { "id": 33 }]
    });
    // What `POST users.roblox.com/v1/users` answers for those ids in one batch.
    let users = json!({
        "data": [
            { "id": 11, "name": "builderman", "displayName": "Builder Man", "hasVerifiedBadge": true },
            { "id": 22, "name": "Roblox", "displayName": "" },
            { "id": 33, "name": "shedletsky", "displayName": "  Shedletsky  " }
        ]
    });
    let mut book = NameBook::default();
    let rows = friend_names::resolve(&friends, Some(&users), &mut book);

    assert_eq!(rows.len(), 3, "every id is still a friend");
    let labels: Vec<&str> = rows
        .iter()
        .map(|row| friend_names::label(&row.display, &row.name))
        .collect();
    assert_eq!(labels, ["Builder Man", "Roblox", "Shedletsky"]);
    for row in &rows {
        assert!(row.is_named(), "{} is still nameless", row.id);
        assert_ne!(
            friend_names::label(&row.display, &row.name),
            friend_names::FALLBACK,
            "a bubble settled on the placeholder"
        );
    }
    // displayName is kept separate from the username, so the tooltip can show `@builderman`.
    assert_eq!(rows[0].name, "builderman");
    assert_eq!(rows[0].display, "Builder Man");
    assert!(rows[0].verified, "the verified badge travels with the batch");
    assert!(!rows[1].verified);
    assert_eq!(rows[1].display, "", "an empty displayName stays empty");
    assert_eq!(rows[2].display, "Shedletsky", "the batch is trimmed");
}

#[test]
fn a_presence_refresh_reuses_the_learned_names() {
    let friends = json!({ "data": [{ "id": 11 }, { "id": 22 }] });
    let users = json!({
        "data": [
            { "id": 11, "name": "builderman", "displayName": "Builder Man" },
            { "id": 22, "name": "Roblox", "displayName": "Roblox" }
        ]
    });
    let mut book = NameBook::default();
    friend_names::resolve(&friends, Some(&users), &mut book);

    // The next poll rebuilds the same friends from ids only, with no batch to spend.
    let again = friend_names::resolve(&friends, None, &mut book);
    let labels: Vec<&str> = again
        .iter()
        .map(|row| friend_names::label(&row.display, &row.name))
        .collect();
    assert_eq!(labels, ["Builder Man", "Roblox"]);
}

#[test]
fn only_a_friend_the_batch_never_answered_stays_on_the_placeholder() {
    let friends = json!({ "data": [{ "id": 11 }, { "id": 99 }] });
    let users = json!({
        "data": [{ "id": 11, "name": "builderman", "displayName": "Builder Man" }]
    });
    let mut book = NameBook::default();
    let rows = friend_names::resolve(&friends, Some(&users), &mut book);

    assert_eq!(friend_names::label(&rows[0].display, &rows[0].name), "Builder Man");
    assert!(!rows[1].is_named(), "id 99 was not answered");
    assert_eq!(
        friend_names::label(&rows[1].display, &rows[1].name),
        friend_names::FALLBACK,
        "the placeholder is the in-flight state, not a final one"
    );

    // When the batch answers later, the same book fills it.
    let later = json!({ "data": [{ "id": 99, "name": "late", "displayName": "Late Bloomer" }] });
    for row in &rows_of(&later) {
        book.remember(row);
    }
    let mut retry = rows[1].clone();
    assert!(book.fill(&mut retry));
    assert_eq!(friend_names::label(&retry.display, &retry.name), "Late Bloomer");
}

#[test]
fn names_that_arrive_with_the_friends_are_remembered_too() {
    let friends = json!({
        "data": [{ "id": 7, "name": "old_style", "displayName": "Old Style" }]
    });
    let mut book = NameBook::default();
    let rows = friend_names::resolve(&friends, None, &mut book);
    assert_eq!(rows[0].display, "Old Style");

    let id_only = json!({ "data": [{ "id": 7 }] });
    let again = friend_names::resolve(&id_only, None, &mut book);
    assert_eq!(again[0].display, "Old Style");
    assert_eq!(again[0].name, "old_style");
}

#[test]
fn the_book_never_forgets_and_reports_repeats() {
    let mut book = NameBook::default();
    let row = FriendName {
        id: 5,
        display: "Five".to_string(),
        name: "five".to_string(),
        verified: false,
    };
    assert!(book.remember(&row), "the first sighting teaches the book");
    assert!(!book.remember(&row), "the same row again changes nothing");
    // A blank row cannot erase what is known.
    assert!(!book.remember(&FriendName {
        id: 5,
        ..Default::default()
    }));
    assert_eq!(book.look(5), Some(("Five", "five", false)));
    assert_eq!(book.look(6), None);

    // A book entry fills only the blanks of a row; a display name present in the row wins.
    let mut partial = FriendName {
        id: 5,
        display: String::new(),
        name: "typed".to_string(),
        verified: false,
    };
    assert!(book.fill(&mut partial));
    assert_eq!(partial.display, "Five");
    assert_eq!(partial.name, "typed", "a known username is not overwritten");
}

#[test]
fn ids_go_out_in_batches_of_a_hundred() {
    let ids: Vec<u64> = (1..=250).collect();
    let batches = friend_names::batches(&ids);
    assert_eq!(
        batches.iter().map(|b| b.len()).collect::<Vec<_>>(),
        [NAME_BATCH, NAME_BATCH, 50]
    );
    assert_eq!(batches[0][0], 1);
    assert_eq!(batches[2][49], 250);

    // Repeats and zero collapse: one friend costs one id, and id 0 names nobody.
    let messy = [0, 4, 4, 9, 0, 4];
    assert_eq!(friend_names::batches(&messy), vec![vec![4, 9]]);
    assert!(friend_names::batches(&[]).is_empty());
    assert!(friend_names::batches(&[0]).is_empty());
}

#[test]
fn the_row_parser_skips_junk_and_keeps_order() {
    let payload = json!({
        "data": [
            { "id": 3, "name": "  spaced  ", "displayName": " Spaced  Name " },
            { "id": 0, "name": "nobody" },
            { "name": "no id at all" },
            { "id": 3, "name": "duplicate" },
            { "id": 4, "displayName": "Only A Display Name" },
            "not a row",
            { "id": 8, "name": null, "displayName": null }
        ]
    });
    let rows = rows_of(&payload);
    assert_eq!(rows.len(), 3, "3, 4 and 8 survive; 0, no-id and the dup are dropped");
    assert_eq!(rows[0].id, 3);
    assert_eq!(rows[0].name, "spaced");
    assert_eq!(rows[0].display, "Spaced  Name");
    assert_eq!(rows[1].id, 4);
    assert_eq!(
        friend_names::label(&rows[1].display, &rows[1].name),
        "Only A Display Name"
    );
    assert_eq!(rows[2].id, 8);
    assert!(!rows[2].is_named());
    // A missing or empty payload is an empty list, never a panic.
    assert!(rows_of(&json!({})).is_empty());
    assert!(rows_of(&json!({ "data": [] })).is_empty());
}

#[test]
fn the_label_rule_is_display_then_username_then_placeholder() {
    assert_eq!(friend_names::label("Builder Man", "builderman"), "Builder Man");
    assert_eq!(friend_names::label("", "builderman"), "builderman");
    assert_eq!(friend_names::label("   ", "builderman"), "builderman");
    assert_eq!(friend_names::label("", ""), "Player");
    assert_eq!(friend_names::label("  ", "  "), "Player");
}
