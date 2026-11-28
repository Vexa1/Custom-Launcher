//! Two texture budgets in one cache, and only the hero picture on screen being fetched.
//!
//! `src/thumb_pipeline.rs` is the real module, included by path. These tests model the launcher
//! uploading card thumbnails and Game Details gallery pictures into the same `thumb_tex` map:
//! the gallery has its own, smaller budget, and overflowing it must never drop a card.

use creblox_vm_tests::thumb_pipeline::{self, TextureBudget};

fn card(i: usize) -> String {
    format!("https://cdn.example/card-{i}.png")
}

fn hero(i: usize) -> String {
    format!("https://cdn.example/hero-{i}.png")
}

#[test]
fn gallery_pictures_never_evict_card_thumbnails() {
    let mut budget = TextureBudget::default();
    for i in 0..thumb_pipeline::TEXTURE_CAP {
        budget.insert_card(&card(i));
    }
    assert!(
        budget.evictions().is_empty(),
        "a full card cache drops nothing on its own"
    );

    // Open several game pages: the gallery budget overflows, the cards stay put.
    for i in 0..thumb_pipeline::GALLERY_TEXTURE_CAP + 3 {
        budget.insert_gallery(&hero(i));
    }
    let dropped = budget.evictions();
    assert_eq!(
        dropped,
        vec![hero(0), hero(1), hero(2)],
        "the oldest pictures are the ones that go"
    );
    assert!(
        dropped.iter().all(|url| !url.contains("card-")),
        "no card thumbnail was evicted by the gallery"
    );

    // The card lane is still full, so one more card evicts a card - the oldest one.
    budget.insert_card(&card(999));
    assert_eq!(budget.evictions(), vec![card(0)]);
}

#[test]
fn a_texture_uploaded_twice_takes_one_slot() {
    let mut budget = TextureBudget::default();
    for _ in 0..5 {
        budget.insert_gallery("hero.png");
    }
    for i in 0..thumb_pipeline::GALLERY_TEXTURE_CAP {
        budget.insert_gallery(&hero(i));
    }
    // The duplicate never took a slot, so only the first real picture overflows.
    assert_eq!(budget.evictions(), vec!["hero.png".to_string()]);
}

#[test]
fn the_hero_asks_for_the_picture_it_is_showing() {
    let media = vec!["a.png".to_string(), "b.png".to_string(), "c.png".to_string()];
    assert_eq!(
        thumb_pipeline::shown_media(&media, 0).map(String::as_str),
        Some("a.png")
    );
    // The index is wrapped, exactly as the hero wraps it when stepping past the last picture.
    assert_eq!(
        thumb_pipeline::shown_media(&media, 4).map(String::as_str),
        Some("b.png")
    );
    assert_eq!(thumb_pipeline::shown_media(&[], 0), None);
    assert_eq!(thumb_pipeline::shown_media(&vec![String::new()], 0), None);
}

#[test]
fn pass_pictures_have_their_own_lane_too() {
    let mut budget = TextureBudget::default();
    for i in 0..thumb_pipeline::TEXTURE_CAP {
        budget.insert_card(&card(i));
    }
    for i in 0..thumb_pipeline::GALLERY_TEXTURE_CAP {
        budget.insert_gallery(&hero(i));
    }
    for i in 0..thumb_pipeline::PASS_TEXTURE_CAP + 2 {
        budget.insert_pass(&format!("pass-{i}.png"));
    }
    // Opening stores overflows only the pass lane: no card and no gallery picture moves.
    assert_eq!(
        budget.evictions(),
        vec!["pass-0.png".to_string(), "pass-1.png".to_string()]
    );
}
