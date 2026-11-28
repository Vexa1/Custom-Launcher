//! The profile shelf: the frame-1 shell, the bounded LRU, stale-while-revalidate and the
//! section-by-section assembly.

use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use creblox_vm_tests::profile_cache::{
    apply, all_sections, is_complete, seed, ProfileCache, ProfileUpdate, CAPACITY, HOVER_DWELL,
    TTL,
};
use creblox_vm_tests::profile_cache::FriendStream;
use creblox_vm_tests::types::{AvatarAsset, FriendProfile, Game, ProfileSections, User};

fn user(id: u64) -> User {
    User {
        id,
        name: format!("user{id}"),
        display: format!("User {id}"),
        headshot: format!("rbxthumb://headshot/{id}"),
        presence: 2,
        location: "Adopt Me!".to_string(),
        place_id: Some(920587237),
        verified: false,
    }
}

fn asset(id: u64) -> AvatarAsset {
    AvatarAsset {
        id,
        name: format!("Item {id}"),
        asset_type_id: 8,
        asset_type_name: "Hat".to_string(),
        thumb: String::new(),
        meta: None,
        creator_name: String::new(),
        creator_verified: false,
        price: None,
    }
}

fn game(universe: u64, place: u64, name: &str) -> Game {
    Game {
        universe_id: universe,
        place_id: place,
        name: name.to_string(),
        creator: "Someone".to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// What a full shelf costs, since the RAM budget charges these bytes.

#[test]
fn a_stored_profile_is_charged_its_real_bytes_and_clear_takes_it_back_to_zero() {
    let mut cache = ProfileCache::new();
    assert_eq!(cache.bytes(), 0, "an empty shelf costs nothing");

    let mut p = complete(7);
    p.description = "a description that is definitely longer than nothing".to_string();
    p.experiences = vec![game(1, 10, "Adopt Me!"), game(2, 20, "Blox Fruits")];
    p.favorites = vec![game(3, 30, "Doors")];
    p.currently_wearing = vec![asset(100), asset(101)];

    let expected_profile = p.tracked_bytes();
    cache.insert(p.clone());
    let charged = cache.bytes();

    // The shelf charges the profile's tracked heap plus a small fixed LRU entry.
    assert!(
        charged >= expected_profile,
        "charged {charged} but the profile alone is {expected_profile}"
    );
    assert!(
        charged <= expected_profile + 1024,
        "charged {charged}, far more than the profile's {expected_profile} plus an LRU entry"
    );
    // The regression this pins: the budget used to charge the shelf zero, so a populated
    // profile with its games and avatar items vanished from the total. One stored profile is
    // already kilobytes, so a zero (or a fixed per-entry stub) fails here.
    assert!(
        charged > 1024,
        "a populated profile is kilobytes, not a zero-byte stub: {charged}"
    );

    // Real bytes follow the strings, so the readout cannot drift into a fixed guess.
    let mut longer = p.clone();
    longer.description.push_str(&"x".repeat(4096));
    let mut other = ProfileCache::new();
    other.insert(longer);
    assert!(
        other.bytes() >= charged + 4096,
        "a 4 KB longer description must cost at least 4 KB more"
    );

    cache.clear();
    assert_eq!(cache.bytes(), 0, "the tray flush and a cleared shelf cost zero");
}

fn complete(id: u64) -> FriendProfile {
    let mut p = seed(&user(id));
    p.description = "hi".to_string();
    p.created = "2019-04-02".to_string();
    p.friends = 132;
    p.followers = 9;
    p.following = 4;
    p.avatar = "rbxthumb://avatar".to_string();
    p.sections = all_sections();
    p
}

// ---------------------------------------------------------------------------
// Frame one: the seed is already a drawable profile.
// ---------------------------------------------------------------------------

#[test]
fn the_first_frame_is_already_a_profile() {
    let u = user(42);
    let p = seed(&u);

    // Everything the header draws with, and everything the buttons need, is there.
    assert_eq!(p.user.id, 42);
    assert_eq!(p.user.name, "user42");
    assert_eq!(p.user.display, "User 42");
    assert_eq!(p.user.headshot, "rbxthumb://headshot/42");
    assert_eq!(p.user.presence, 2);
    assert_eq!(p.user.place_id, Some(920587237));
    assert_eq!(p.user.location, "Adopt Me!");

    // And nothing claims a section that has not been fetched.
    assert_eq!(p.sections, ProfileSections::default());
    assert!(!is_complete(p.sections));
    assert_eq!(p.friends, 0);
    assert!(p.badges.is_empty());
    assert!(p.currently_wearing.is_empty());
}

#[test]
fn a_seed_never_looks_like_a_complete_profile() {
    // The page keys its skeletons off these flags, so a seeded header must not pass for a
    // loaded profile even though every field is a valid (empty) value.
    let p = seed(&user(1));
    for flag in [
        p.sections.info,
        p.sections.friends,
        p.sections.followers,
        p.sections.following,
        p.sections.avatar,
        p.sections.wearing,
        p.sections.badges,
        p.sections.creations,
        p.sections.favorites,
    ] {
        assert!(!flag);
    }
}

// ---------------------------------------------------------------------------
// The shelf.
// ---------------------------------------------------------------------------

#[test]
fn a_revisit_is_served_from_memory() {
    let mut cache = ProfileCache::new();
    cache.insert(complete(7));

    let hit = cache.get(7).expect("cached profile");
    assert_eq!(hit.friends, 132);
    assert!(cache.is_fresh(7), "a just-fetched profile needs no network");
    assert!(!cache.needs_refresh(7));
}

#[test]
fn a_profile_past_the_ttl_is_still_painted_and_refreshed_in_the_background() {
    let mut cache = ProfileCache::new();
    let old = Instant::now() - TTL - Duration::from_secs(1);
    cache.insert_at(complete(8), old);

    // Stale-while-revalidate: the data is there to paint...
    assert!(cache.get(8).is_some(), "a stale profile is still served");
    // ...and the caller is told to refresh it.
    assert!(cache.needs_refresh(8));
    assert!(!cache.is_fresh(8));
}

#[test]
fn a_profile_just_inside_the_ttl_is_not_refetched() {
    let mut cache = ProfileCache::new();
    let recent = Instant::now() - TTL + Duration::from_secs(20);
    cache.insert_at(complete(9), recent);
    assert!(!cache.needs_refresh(9));
}

#[test]
fn a_partial_profile_is_always_worth_refetching() {
    let mut cache = ProfileCache::new();
    // Inserted a millisecond ago, but the badges request failed, so it stays incomplete.
    let mut p = complete(10);
    p.sections.badges = false;
    cache.insert(p);
    assert!(cache.needs_refresh(10));
}

#[test]
fn the_shelf_drops_the_least_recently_used() {
    let mut cache = ProfileCache::new();
    for id in 1..=CAPACITY as u64 {
        cache.insert(complete(id));
    }
    // Touch the oldest so it is no longer the least recently used.
    assert!(cache.get(1).is_some());
    cache.insert(complete(CAPACITY as u64 + 1));

    assert!(cache.get(1).is_some(), "a just-used profile survives");
    assert!(cache.get(2).is_none(), "the least recently used one goes");
    assert!(cache.get(CAPACITY as u64 + 1).is_some());
}

#[test]
fn removing_a_profile_forces_the_next_open_to_fetch() {
    let mut cache = ProfileCache::new();
    cache.insert(complete(11));
    assert!(!cache.needs_refresh(11));
    cache.remove(11);
    assert!(cache.needs_refresh(11));
    assert!(cache.get(11).is_none());
}

// ---------------------------------------------------------------------------
// Streaming assembly: sections land in whatever order the network returns them.
// ---------------------------------------------------------------------------

fn stream_for(id: u64) -> (FriendStream, std::sync::mpsc::Sender<ProfileUpdate>) {
    let (tx, rx) = channel();
    (FriendStream::new(id, seed(&user(id)), rx), tx)
}

#[test]
fn sections_arrive_one_at_a_time() {
    let (mut stream, tx) = stream_for(20);

    // The fast endpoints answer first; the header was already drawable before any of them.
    tx.send(ProfileUpdate::Badges(Vec::new())).unwrap();
    tx.send(ProfileUpdate::Friends(132)).unwrap();
    let (applied, finished) = stream.drain();
    assert_eq!(applied, 2);
    assert!(!finished, "the stream is still running");
    assert!(stream.profile.sections.badges);
    assert!(stream.profile.sections.friends);
    assert_eq!(stream.profile.friends, 132);
    // Sections that have not answered keep their skeleton.
    assert!(!stream.profile.sections.info);
    assert!(!stream.profile.sections.wearing);
    assert!(!is_complete(stream.profile.sections));

    // The rest land later, and the last one closes the stream.
    tx.send(ProfileUpdate::Info {
        name: "user20".to_string(),
        display: "User 20".to_string(),
        description: "hello".to_string(),
        created: "2018-01-01".to_string(),
        verified: true,
    })
    .unwrap();
    tx.send(ProfileUpdate::Wearing(vec![asset(1), asset(2)])).unwrap();
    tx.send(ProfileUpdate::Done).unwrap();
    let (applied, finished) = stream.drain();
    assert_eq!(applied, 2);
    assert!(finished);
    assert_eq!(stream.profile.description, "hello");
    assert_eq!(stream.profile.created, "2018-01-01");
    assert!(stream.profile.verified);
    assert!(stream.profile.user.verified);
    assert_eq!(stream.profile.currently_wearing.len(), 2);
}

#[test]
fn the_order_of_arrival_does_not_change_the_profile() {
    let updates = || {
        vec![
            ProfileUpdate::Info {
                name: "user21".to_string(),
                display: "User 21".to_string(),
                description: "bio".to_string(),
                created: "2020-02-02".to_string(),
                verified: false,
            },
            ProfileUpdate::Friends(1),
            ProfileUpdate::Followers(2),
            ProfileUpdate::Following(3),
            ProfileUpdate::Avatar("rbxthumb://avatar21".to_string()),
            ProfileUpdate::Wearing(vec![asset(5)]),
            ProfileUpdate::Badges(Vec::new()),
            ProfileUpdate::Creations(vec![game(10, 11, "Mine")]),
            ProfileUpdate::Favorites(vec![game(12, 13, "Liked")]),
        ]
    };

    let mut straight = seed(&user(21));
    for u in updates() {
        apply(&mut straight, u);
    }

    let mut reversed = seed(&user(21));
    for u in updates().into_iter().rev() {
        apply(&mut reversed, u);
    }

    assert!(is_complete(straight.sections));
    assert!(is_complete(reversed.sections));
    assert_eq!(straight.friends, reversed.friends);
    assert_eq!(straight.followers, reversed.followers);
    assert_eq!(straight.following, reversed.following);
    assert_eq!(straight.description, reversed.description);
    assert_eq!(straight.avatar, reversed.avatar);
    assert_eq!(straight.experiences.len(), reversed.experiences.len());
    assert_eq!(straight.favorites.len(), reversed.favorites.len());
    assert_eq!(straight.currently_wearing.len(), reversed.currently_wearing.len());
}

#[test]
fn a_failed_section_leaves_a_skeleton_not_an_empty_claim() {
    let (mut stream, tx) = stream_for(22);
    tx.send(ProfileUpdate::Failed("badges: HTTP 429".to_string())).unwrap();
    tx.send(ProfileUpdate::Badges(Vec::new())).ok();
    tx.send(ProfileUpdate::Done).unwrap();
    stream.drain();

    assert!(stream.finished);
    assert_eq!(stream.err.as_deref(), Some("badges: HTTP 429"));
    assert!(stream.profile.sections.badges, "the badge list did arrive");

    // A profile that lost a section completely stays incomplete, so it is refetched.
    let mut cache = ProfileCache::new();
    cache.insert(stream.profile);
    assert!(cache.needs_refresh(22));
}

#[test]
fn a_closed_stream_finishes_instead_of_hanging() {
    let (mut stream, tx) = stream_for(23);
    tx.send(ProfileUpdate::Followers(5)).unwrap();
    drop(tx);
    let (applied, finished) = stream.drain();
    assert_eq!(applied, 1);
    assert!(finished, "a dropped sender ends the stream");
    assert_eq!(stream.profile.followers, 5);
    // Draining a finished stream is a no-op, so the caller can keep calling it.
    assert_eq!(stream.drain(), (0, true));
}

#[test]
fn presence_arriving_late_updates_the_header_and_reveals_the_game() {
    let mut profile = seed(&user(24));
    profile.user.presence = 0;
    profile.user.place_id = None;

    apply(
        &mut profile,
        ProfileUpdate::Presence {
            presence: 2,
            location: "Jailbreak".to_string(),
            place_id: Some(606849621),
        },
    );
    apply(
        &mut profile,
        ProfileUpdate::Playing(Some(game(1, 606849621, "Jailbreak"))),
    );

    assert_eq!(profile.user.presence, 2);
    assert_eq!(profile.user.location, "Jailbreak");
    assert_eq!(profile.user.place_id, Some(606849621));
    let playing = profile.playing_game.expect("the currently-playing card");
    assert_eq!(playing.place_id, 606849621, "Join Game uses this place");
}

#[test]
fn the_hover_dwell_is_short_enough_to_feel_instant_and_long_enough_to_ignore_a_sweep() {
    assert_eq!(HOVER_DWELL, Duration::from_millis(120));
    assert!(TTL >= Duration::from_secs(60), "a profile is worth keeping");
}
