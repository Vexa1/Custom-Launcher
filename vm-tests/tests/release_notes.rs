//! Pairing each release download with its own checksum.
//!
//! A release body lists two macOS archives, each with a size and a SHA-256. The notes used to
//! print the checksum and then wrap the link onto the next line, and pairing the link with the
//! text *after* it handed the update zip the full build's checksum: every in-app update then
//! failed with "Update zip failed SHA-256 verification". These tests pin both layouts.

use creblox_vm_tests::release_links;

const UPDATE_SHA: &str = "be125bd7c17ae8eeb61dea4feb283a8f4c8d72e407085a89ffe1975abda3ada6";
const FULL_SHA: &str = "51db9abbb39d22d2773dc733116ca9e255d743aeb88c08d7be6988e9bb083784";
const UPDATE_URL: &str =
    "https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.39-update.zip";
const FULL_URL: &str =
    "https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.39-full.zip";

#[test]
fn a_checksum_printed_above_the_link_belongs_to_that_link() {
    let notes = format!(
        "- **CREBLOX-0.5.39-update.zip** — 536,651 bytes · SHA-256 `{UPDATE_SHA}`\n  {UPDATE_URL}\n\n\
         - **CREBLOX-0.5.39-full.zip** — 876,360 bytes · SHA-256 `{FULL_SHA}`\n  {FULL_URL}\n"
    );
    assert_eq!(
        release_links::notes_zip(&notes, false),
        Some((UPDATE_URL.to_string(), Some(UPDATE_SHA.to_string())))
    );
    assert_eq!(
        release_links::zip_checksum(&notes, FULL_URL, false),
        Some(FULL_SHA.to_string())
    );
}

#[test]
fn a_checksum_printed_after_the_link_still_works() {
    let notes = format!(
        "- **CREBLOX-0.5.39-update.zip** — 536,651 bytes — {UPDATE_URL} — SHA-256 `{UPDATE_SHA}`\n\
         - **CREBLOX-0.5.39-full.zip** — 876,360 bytes — {FULL_URL} — SHA-256 `{FULL_SHA}`\n"
    );
    assert_eq!(
        release_links::notes_zip(&notes, false),
        Some((UPDATE_URL.to_string(), Some(UPDATE_SHA.to_string())))
    );
    assert_eq!(
        release_links::zip_checksum(&notes, FULL_URL, true),
        Some(FULL_SHA.to_string())
    );
}

#[test]
fn an_archive_never_borrows_its_neighbours_checksum() {
    // Only the full build is printed: the update archive must come back without a checksum
    // rather than being verified against the other download's hash.
    let notes = format!(
        "- **CREBLOX-0.5.39-full.zip** — 876,360 bytes · SHA-256 `{FULL_SHA}`\n  {FULL_URL}\n"
    );
    assert_eq!(release_links::zip_checksum(&notes, UPDATE_URL, true), None);
    assert_eq!(release_links::pick_zip_url(&notes, false), Some(FULL_URL.to_string()));
}
