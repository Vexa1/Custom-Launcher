//! The release-notes parser and build-failure detection used by the Launcher Updates card.
//!
//! These are the real functions from `src/ui/update_card.rs`; they are pure, so what the card
//! renders can be checked here without a window.

use creblox_vm_tests::update_card::{Block, is_build_failure, parse_notes, spans};

#[test]
fn headings_bullets_and_paragraphs_lose_their_markers() {
    let notes = "## What changed\n\n- **Fixed** the overlay drag\n- Arrow keys work\n\nPlain closing line.";
    let parsed = parse_notes(notes);
    assert_eq!(
        parsed.blocks,
        vec![
            Block::Heading("What changed".to_string()),
            Block::Bullet("**Fixed** the overlay drag".to_string()),
            Block::Bullet("Arrow keys work".to_string()),
            Block::Paragraph("Plain closing line.".to_string()),
        ]
    );
    assert!(parsed.details.is_empty());
}

#[test]
fn inline_bold_code_and_links_are_stripped_into_plain_spans() {
    let runs = spans("**Fixed** the `overlay` and [release notes](https://example.com/x)");
    assert_eq!(runs[0].text, "Fixed");
    assert!(runs[0].bold);
    assert!(!runs[0].code);
    assert!(runs.iter().any(|s| s.text == "overlay" && s.code));
    assert!(runs.last().unwrap().text.ends_with("and release notes"));
    let joined: String = runs.iter().map(|s| s.text.as_str()).collect();
    assert!(!joined.contains("**"));
    assert!(!joined.contains('`'));
    assert!(!joined.contains("https://"));
}

#[test]
fn download_links_and_hashes_move_into_the_details_list() {
    let notes = "## Quick update\n\n- **CREBLOX-0.5.69-update.zip** — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.69-update.zip\n- SHA-256: `768b0f92ee77f646b486ba9570cc8649650a1a532bddb7c4efce260b9ee7b52c`\n\n## What changed\n\n- Real notes stay";
    let parsed = parse_notes(notes);
    assert_eq!(parsed.details.len(), 2);
    assert!(parsed.details[0].contains(".zip"));
    assert!(parsed.details[1].contains("SHA-256"));
    // The drawer stores plain text: no leftover markdown markers around the raw values.
    for line in &parsed.details {
        assert!(!line.contains("**"), "raw bold marker in {line}");
        assert!(!line.contains('`'), "raw backtick in {line}");
    }
    // Nothing raw is left in the formatted body.
    let body = format!("{:?}", parsed.blocks);
    assert!(!body.contains("https://"));
    assert!(!body.contains("768b0f92"));
    assert!(parsed
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Heading(t) if t == "What changed")));
}

#[test]
fn only_compiler_exit_101_counts_as_a_build_failure() {
    assert!(is_build_failure(
        "The build failed (exit 101). Open Details for the output."
    ));
    assert!(is_build_failure("cargo build failed"));
    assert!(!is_build_failure(
        "The update zip does not contain an app source folder."
    ));
    assert!(!is_build_failure("Update zip failed SHA-256 verification."));
}

#[test]
fn an_unclosed_marker_stays_literal_instead_of_swallowing_the_line() {
    let runs = spans("code `here and **bold");
    let joined: String = runs.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(joined, "code here and bold");
    assert!(runs.iter().any(|s| s.code));
    assert!(runs.iter().any(|s| s.bold));
}
