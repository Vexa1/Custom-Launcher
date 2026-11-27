//! Headless input test for the Games filter boxes.
//!
//! The bug this guards against: the box's text used to be rebuilt from the slider value on
//! every frame, so characters vanished as they were typed. The real `FilterBox` from
//! `src/chart_feed.rs` is driven here through actual egui frames with real text events, the
//! same way `page_charts.rs` drives it, and the buffer has to survive.

use creblox_vm_tests::chart_feed::{FilterBox, filter_text, parse_filter_amount};
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
