//! Late-joiner slot check: whether the server a party is heading to still has a seat.
//!
//! The public room list is the only published source that says how full a specific job is.
//! A listed room with no free seat blocks the join; an unlisted one (private, gone, or an
//! API failure) is not evidence of anything, so the join is still attempted.

use creblox_vm_tests::party_server::{slot_state, SlotState};
use creblox_vm_tests::types::ServerItem;

fn room(id: &str, playing: u64, max_players: u64) -> ServerItem {
    ServerItem {
        id: id.to_string(),
        playing,
        max_players,
        ping: Some(40),
        fps: None,
    }
}

#[test]
fn a_full_server_has_no_room_for_a_member() {
    let servers = [room("job-full", 30, 30)];
    assert_eq!(
        slot_state(&servers, "job-full", 1),
        SlotState::Full { free_slots: 0 }
    );
}

#[test]
fn one_free_seat_is_room_for_one_member() {
    let servers = [room("job-a", 29, 30)];
    assert_eq!(slot_state(&servers, "job-a", 1), SlotState::Room);
}

#[test]
fn a_late_joiner_needs_a_seat_like_everybody_else() {
    // One seat left, two members still to join: sending the second would drop them back to
    // the menu with no explanation.
    let servers = [room("job-a", 29, 30)];
    assert_eq!(
        slot_state(&servers, "job-a", 2),
        SlotState::Full { free_slots: 1 }
    );
    assert_eq!(slot_state(&servers, "job-a", 1), SlotState::Room);
}

#[test]
fn an_unlisted_server_is_unknown() {
    // A private server, or one Roblox no longer lists: no data, so no refusal.
    let servers = [room("job-a", 30, 30), room("job-b", 1, 30)];
    assert_eq!(slot_state(&servers, "job-private", 1), SlotState::Unknown);
}

#[test]
fn a_server_without_a_capacity_is_unknown() {
    // Roblox reports 0/0 for rooms it cannot describe; that is not a full server.
    let servers = [room("job-a", 0, 0)];
    assert_eq!(slot_state(&servers, "job-a", 1), SlotState::Unknown);
}

#[test]
fn asking_for_no_members_still_needs_one_seat() {
    let full = [room("job-a", 30, 30)];
    assert_eq!(
        slot_state(&full, "job-a", 0),
        SlotState::Full { free_slots: 0 }
    );
    let open = [room("job-a", 29, 30)];
    assert_eq!(slot_state(&open, "job-a", 0), SlotState::Room);
}

#[test]
fn the_message_says_why_the_join_stopped() {
    assert_eq!(SlotState::Room.message(), "");
    assert_eq!(SlotState::Unknown.message(), "");
    let full = SlotState::Full { free_slots: 0 }.message();
    assert!(full.contains("full") && !full.contains('0'), "{full}");
    let tight = SlotState::Full { free_slots: 1 }.message();
    assert!(tight.contains("1 free slot"), "{tight}");
}
