//! Headless input test for the Games filter boxes.
//!
//! The bug this guards against: the box's text used to be rebuilt from the slider value on
//! every frame, so characters vanished as they were typed. The real `FilterBox` from
//! `src/chart_feed.rs` is driven here through actual egui frames with real text events, the
//! same way `page_charts.rs` drives it, and the buffer has to survive.

use creblox_vm_tests::chart_feed::{
    FilterBox, filter_text, in_player_range, parse_filter_amount, rail_position,
};
use egui::{Event, Pos2, RawInput, Rect, Vec2};

const BOX: Vec2 = Vec2::new(64.0, 22.0);

/// One frame of the filter group, mirroring `page_charts.rs`: the text box, the live parse,
/// the guarded slider sync and the normalize-on-blur.
fn frame(
    ctx: &egui::Context,
    typed: Option<&str>,
    state: &mut FilterBox,
    value: &mut u64,
    request_focus: bool,
    slider_changed: bool,
) -> bool {
    let mut input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 160.0))),
        ..Default::default()
    };
    if let Some(text) = typed {
        input.events.push(Event::Text(text.to_string()));
    }
    let mut focused = false;
    let mut output = ctx.run_ui(input, |ui| {
        let edit = ui.add_sized(
            BOX,
            egui::TextEdit::singleline(&mut state.text).hint_text("0"),
        );
        if request_focus && !edit.has_focus() {
            edit.request_focus();
        }
        focused = edit.has_focus();
        // Type-to-filter applies live but never rewrites the characters.
        if edit.changed() {
            if let Some(v) = state.edited_value() {
                *value = v;
            }
        }
        // Only a real slider move may sync the number back into the text, and never while
        // the caret is in the box.
        if slider_changed {
            state.sync_from_slider(*value, edit.has_focus());
        }
        if edit.lost_focus() {
            state.normalize(*value);
        }
    });
    output.textures_delta.clear();
    focused
}

#[test]
fn typing_500_across_frames_keeps_the_buffer() {
    let ctx = egui::Context::default();
    let mut state = FilterBox::default();
    let mut value = 0u64;

    // Frame 1: click into the box (no characters yet).
    assert!(frame(&ctx, None, &mut state, &mut value, true, false));
    assert_eq!(state.text, "", "an empty box stays empty until something is typed");

    // Frames 2-4: type "5", "0", "0" one key at a time, exactly like a user.
    for (i, key) in ["5", "0", "0"].iter().enumerate() {
        let focused = frame(&ctx, Some(key), &mut state, &mut value, true, false);
        assert!(focused, "the box keeps focus while typing");
        assert_eq!(
            state.text,
            ["5", "50", "500"][i],
            "the buffer must accumulate, not be reset by the slider"
        );
    }
    assert_eq!(value, 500, "the filter threshold follows the typed digits");
}

#[test]
fn a_slider_frame_never_overwrites_a_focused_box() {
    let ctx = egui::Context::default();
    let mut state = FilterBox::default();
    let mut value = 0u64;
    frame(&ctx, None, &mut state, &mut value, true, false);
    for key in ["1", "2", "3"] {
        frame(&ctx, Some(key), &mut state, &mut value, true, false);
    }
    assert_eq!(state.text, "123");
    assert_eq!(value, 123);

    // A slider move while the caret is still in the box: the text is untouched.
    value = 900;
    frame(&ctx, None, &mut state, &mut value, true, true);
    assert_eq!(state.text, "123", "a focused box is never rewritten");

    // Once it is not focused any more, a slider move does mirror the number.
    let mut unfocused = FilterBox {
        text: "123".to_string(),
    };
    unfocused.sync_from_slider(900, true);
    assert_eq!(unfocused.text, "123");
    unfocused.sync_from_slider(900, false);
    assert_eq!(unfocused.text, "900");
}

#[test]
fn blur_normalizes_the_typed_text() {
    let mut box_ = FilterBox {
        text: "1.2k".to_string(),
    };
    assert_eq!(box_.edited_value(), Some(1_200));
    box_.normalize(1_200);
    assert_eq!(box_.text, "1.2K");

    // Zero normalizes to an empty box, which is what shows the hint again.
    box_.normalize(0);
    assert_eq!(box_.text, "");
    assert_eq!(filter_text(0), "");
    assert_eq!(filter_text(45), "45");
}

#[test]
fn typed_amounts_accept_separators_percent_and_shorthand() {
    assert_eq!(parse_filter_amount("500"), Some(500));
    assert_eq!(parse_filter_amount("1,250"), Some(1_250));
    assert_eq!(parse_filter_amount(" 2.5k "), Some(2_500));
    assert_eq!(parse_filter_amount("3m"), Some(3_000_000));
    assert_eq!(parse_filter_amount("45%"), Some(45));
    assert_eq!(parse_filter_amount(""), None);
    assert_eq!(parse_filter_amount("k"), None);
    assert_eq!(parse_filter_amount("abc"), None);
    assert_eq!(parse_filter_amount("-5"), None);

    // A half-typed value must not clear the filter: `edited_value` returns None and the
    // caller keeps the previous threshold.
    let mut state = FilterBox {
        text: "1.2.3".to_string(),
    };
    assert_eq!(state.edited_value(), None);
    state.text = "1.5".to_string();
    assert_eq!(state.edited_value(), Some(2), "shorthand-less decimals round");
}

/// The "Max Players" pill: an empty box, `Max` or `0` is no upper limit at all, a number is the
/// ceiling, and text that is not a number yet leaves the filter alone.
#[test]
fn a_ceiling_box_means_no_limit_when_it_is_empty_max_or_zero() {
    let box_with = |text: &str| FilterBox {
        text: text.to_string(),
    };
    assert_eq!(box_with("").edited_ceiling(), Some(None));
    assert_eq!(box_with("   ").edited_ceiling(), Some(None));
    assert_eq!(box_with("Max").edited_ceiling(), Some(None));
    assert_eq!(box_with("max").edited_ceiling(), Some(None));
    assert_eq!(box_with("0").edited_ceiling(), Some(None), "0 is not a ceiling");
    assert_eq!(box_with("500").edited_ceiling(), Some(Some(500)));
    assert_eq!(box_with("5,000").edited_ceiling(), Some(Some(5_000)));
    assert_eq!(box_with("5k").edited_ceiling(), Some(Some(5_000)));
    assert_eq!(box_with("2m").edited_ceiling(), Some(Some(2_000_000)));
    assert_eq!(box_with("abc").edited_ceiling(), None, "half-typed text is left alone");
    assert_eq!(box_with("-").edited_ceiling(), None);
}

/// Small plain numbers are values, not half-typed text: they must parse exactly and come back
/// out of the box unchanged.
#[test]
fn small_plain_numbers_parse_exactly() {
    for (text, value) in [
        ("0", 0u64),
        ("1", 1),
        ("10", 10),
        ("55", 55),
        ("500", 500),
        ("12500", 12_500),
        ("12,500", 12_500),
    ] {
        assert_eq!(parse_filter_amount(text), Some(value), "{text}");
        // Formatting is allowed to shorten (`12,500` -> `12.5K`) or empty out (0 -> ""),
        // never to move the value: the box means the same number after a round trip.
        let reformatted = FilterBox {
            text: filter_text(value),
        };
        assert_eq!(
            reformatted.edited_value(),
            Some(value),
            "{value} survives a format round trip"
        );
        let mut players_box = FilterBox {
            text: text.to_string(),
        };
        assert_eq!(players_box.edited_value(), Some(value), "{text}");
        // Blur only tidies the text: the number it means is untouched.
        players_box.normalize(value);
        assert_eq!(players_box.edited_value(), Some(value), "blur keeps {text} = {value}");
    }
}

/// Suffixed amounts are exact and are never clamped to the 100K the players rail spans.
#[test]
fn large_suffixed_numbers_are_not_clamped() {
    for (text, value) in [
        ("500K", 500_000u64),
        ("500k", 500_000),
        ("2M", 2_000_000),
        ("1B", 1_000_000_000),
        ("90000", 90_000),
        ("100000", 100_000),
        ("250K", 250_000),
    ] {
        assert_eq!(parse_filter_amount(text), Some(value), "{text}");
    }
    let players_box = FilterBox {
        text: "500K".to_string(),
    };
    assert_eq!(players_box.edited_value(), Some(500_000));
    let max_box = FilterBox {
        text: "2M".to_string(),
    };
    assert_eq!(max_box.edited_ceiling(), Some(Some(2_000_000)));
}

/// Only the thumb is bounded by the rail's span; the filter value keeps what was typed.
#[test]
fn the_rail_clamps_its_thumb_but_not_the_filter() {
    assert_eq!(rail_position(10, 100_000), 10.0);
    assert_eq!(rail_position(500_000, 100_000), 100_000.0);
    assert_eq!(rail_position(0, 100_000), 0.0);
    assert_eq!(rail_position(55, 100_000_000), 55.0, "a small visits value stays put");
    assert_eq!(rail_position(u64::MAX, 100_000), 100_000.0);
}

/// The player band: at least the minimum, at most the ceiling, and `None` is no ceiling at all.
#[test]
fn the_player_band_is_inclusive_on_both_ends() {
    assert!(in_player_range(10, 10, None), "the minimum itself passes");
    assert!(!in_player_range(9, 10, None));
    assert!(in_player_range(10, 10, Some(10)));
    assert!(!in_player_range(11, 10, Some(10)));
    assert!(in_player_range(500_000, 10, Some(500_000)), "the ceiling itself passes");
    assert!(!in_player_range(500_001, 10, Some(500_000)));
    assert!(in_player_range(0, 0, None), "no filters matches everything");
    assert!(in_player_range(312_000, 10, Some(500_000)));
    assert!(!in_player_range(2_000_000, 10, Some(500_000)));
}
