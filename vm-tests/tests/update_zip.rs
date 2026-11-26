//! The Windows self-update reads a release zip and picks the right download. Both halves live in
//! host-independent modules (`src/release_links.rs`, `src/zip_update.rs`), so this suite runs the
//! real code on the Linux VM where the GUI crate does not build.

use creblox_vm_tests::{release_links, zip_update};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const WINDOWS_URL: &str =
    "https://github.com/Vexa1/Custom-Launcher/releases/download/v0.5.4/CREBLOX-0.5.4-windows-x64.zip";
const MAC_UPDATE_URL: &str =
    "https://github.com/Vexa1/Custom-Launcher/releases/download/v0.5.4/creblox-update-0.5.4-macos.zip";
const MAC_FULL_URL: &str =
    "https://github.com/Vexa1/Custom-Launcher/releases/download/v0.5.4/creblox-0.5.4-macos-full.zip";
const MAC_HASH: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
const WIN_HASH: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// The notes print 64-hex tokens; a fixture that drifts from that length would make every
/// checksum assertion below pass or fail for the wrong reason.
#[test]
fn the_checksum_fixtures_are_64_hex_characters() {
    for hash in [MAC_HASH, WIN_HASH] {
        assert_eq!(hash.len(), 64, "not a SHA-256 token: {hash}");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "not hex: {hash}");
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("creblox-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch folder");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        zip.start_file(*name, options).expect("start entry");
        zip.write_all(data).expect("write entry");
    }
    zip.finish().expect("finish zip");
}

/// A minimal PE image: DOS stub, `PE\0\0`, the given machine and subsystem, PE32+ magic.
/// `verify_launcher` reads exactly these fields, so a fixture that skips them is no launcher.
fn pe_image(size: usize, machine: u16, subsystem: u16) -> Vec<u8> {
    let mut data = vec![0u8; size.max(0x200)];
    data[0] = b'M';
    data[1] = b'Z';
    data[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    let pe = 0x80;
    data[pe..pe + 4].copy_from_slice(b"PE\0\0");
    data[pe + 4..pe + 6].copy_from_slice(&machine.to_le_bytes());
    data[pe + 20..pe + 22].copy_from_slice(&0xf0u16.to_le_bytes());
    data[pe + 24..pe + 26].copy_from_slice(&0x20bu16.to_le_bytes());
    data[pe + 24 + 0x44..pe + 24 + 0x46].copy_from_slice(&subsystem.to_le_bytes());
    data
}

const X64_GUI: (u16, u16) = (0x8664, 2);

fn launcher_bytes(size: usize) -> Vec<u8> {
    pe_image(size, X64_GUI.0, X64_GUI.1)
}

fn release_notes() -> String {
    format!(
        "In-place update (about 347 KB): {MAC_UPDATE_URL}\nsha256 {MAC_HASH}\n\n\
         Full build (fresh install): {MAC_FULL_URL}\n\n\
         Windows x64: {WINDOWS_URL}\nsha256 {WIN_HASH}"
    )
}

#[test]
fn extracts_the_launcher_and_leaves_the_rest_alone() {
    let scratch = Scratch::new("extract");
    let zip_path = scratch.path().join("update.zip");
    let payload = launcher_bytes(70 * 1024);
    write_zip(
        &zip_path,
        &[
            ("CREBLOX 0.5.4/LICENSE", b"mit"),
            ("CREBLOX 0.5.4/creblox.exe", &payload),
            ("CREBLOX 0.5.4/README.md", b"readme"),
        ],
    );
    let dest = scratch.path().join("out");

    let exe = zip_update::extract_launcher(&zip_path, &dest).expect("extracts");

    assert_eq!(exe, dest.join("creblox.exe"));
    assert_eq!(fs::read(&exe).expect("read exe"), payload);
    assert!(!dest.join("README.md").exists(), "only the launcher comes out");
    assert_eq!(
        zip_update::verify_launcher(&exe).expect("verifies"),
        payload.len() as u64
    );
}

#[test]
fn a_zip_without_the_launcher_is_rejected() {
    let scratch = Scratch::new("missing");
    let zip_path = scratch.path().join("update.zip");
    write_zip(&zip_path, &[("CREBLOX 0.5.4/README.md", b"readme")]);

    let err = zip_update::extract_launcher(&zip_path, &scratch.path().join("out")).unwrap_err();

    assert!(err.contains("creblox.exe"), "unexpected error: {err}");
}

#[test]
fn archive_paths_are_flattened_into_the_destination() {
    let scratch = Scratch::new("escape");
    let zip_path = scratch.path().join("update.zip");
    write_zip(&zip_path, &[("../../../creblox.exe", b"MZ")]);
    let dest = scratch.path().join("out");

    // Only the entry's file name is used, so the launcher lands inside `dest` and nothing is
    // written up the tree.
    let exe = zip_update::extract_launcher(&zip_path, &dest).expect("extracts inside dest");
    assert_eq!(exe, dest.join("creblox.exe"));
    assert!(!scratch.path().join("creblox.exe").exists());
    assert!(!scratch.path().parent().unwrap().join("creblox.exe").exists());
}

#[test]
fn an_archive_of_only_other_files_extracts_nothing() {
    let scratch = Scratch::new("other");
    let zip_path = scratch.path().join("update.zip");
    write_zip(&zip_path, &[("../../../evil.exe", b"MZ")]);

    let err = zip_update::extract_launcher(&zip_path, &scratch.path().join("out")).unwrap_err();

    assert!(err.contains("creblox.exe"), "unexpected error: {err}");
    assert!(!scratch.path().join("evil.exe").exists());
}

#[test]
fn a_truncated_launcher_is_rejected_before_the_swap() {
    let scratch = Scratch::new("truncated");
    let zip_path = scratch.path().join("update.zip");
    write_zip(&zip_path, &[("creblox.exe", b"MZ")]);

    let exe = zip_update::extract_launcher(&zip_path, &scratch.path().join("out")).expect("extracts");
    let err = zip_update::verify_launcher(&exe).unwrap_err();

    assert!(err.contains("too small"), "unexpected error: {err}");
}

#[test]
fn a_launcher_without_a_pe_header_is_rejected() {
    let scratch = Scratch::new("notpe");
    let file = scratch.path().join("creblox.exe");
    fs::write(&file, vec![0u8; 70 * 1024]).expect("write stub");

    let err = zip_update::verify_launcher(&file).unwrap_err();

    assert!(err.contains("Windows executable"), "unexpected error: {err}");
}

#[test]
fn a_console_build_is_rejected() {
    let scratch = Scratch::new("console");
    let file = scratch.path().join("creblox.exe");
    fs::write(&file, pe_image(70 * 1024, 0x8664, 3)).expect("write console stub");

    let err = zip_update::verify_launcher(&file).unwrap_err();

    assert!(err.contains("GUI"), "unexpected error: {err}");
}

#[test]
fn a_32_bit_build_is_rejected() {
    let scratch = Scratch::new("x86");
    let file = scratch.path().join("creblox.exe");
    fs::write(&file, pe_image(70 * 1024, 0x014c, 2)).expect("write x86 stub");

    let err = zip_update::verify_launcher(&file).unwrap_err();

    assert!(err.contains("x64"), "unexpected error: {err}");
}

#[test]
fn a_pe_header_pointing_outside_the_file_is_rejected() {
    let scratch = Scratch::new("badpe");
    let file = scratch.path().join("creblox.exe");
    let mut data = launcher_bytes(70 * 1024);
    data[0x3c..0x40].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
    fs::write(&file, data).expect("write stub");

    let err = zip_update::verify_launcher(&file).unwrap_err();

    assert!(err.contains("Windows executable"), "unexpected error: {err}");
}

#[test]
fn the_windows_zip_is_picked_on_windows_and_the_overlay_on_macos() {
    let notes = release_notes();

    assert_eq!(
        release_links::pick_zip_url(&notes, true).as_deref(),
        Some(WINDOWS_URL)
    );
    assert_eq!(
        release_links::pick_zip_url(&notes, false).as_deref(),
        Some(MAC_UPDATE_URL)
    );
}

#[test]
fn without_a_platform_build_the_first_zip_link_is_used() {
    let notes = format!("Full build: {MAC_FULL_URL}\nUpdate: {MAC_UPDATE_URL}");

    assert_eq!(
        release_links::pick_zip_url(&notes, true).as_deref(),
        Some(MAC_FULL_URL),
        "Windows falls back to the first link instead of inventing one"
    );
    assert_eq!(
        release_links::pick_zip_url("no links here", true),
        None,
        "a body without downloads has nothing to offer"
    );
}

#[test]
fn trailing_punctuation_is_not_part_of_the_url() {
    let notes = format!("Windows build ({WINDOWS_URL}).");

    assert_eq!(
        release_links::pick_zip_url(&notes, true).as_deref(),
        Some(WINDOWS_URL)
    );
}

#[test]
fn asset_names_are_chosen_per_platform() {
    let windows = "CREBLOX-0.5.4-windows-x64.zip";
    let macos = "creblox-0.5.4-macos-full.zip";

    assert!(release_links::prefers_zip_asset(windows, true));
    assert!(!release_links::prefers_zip_asset(windows, false));
    assert!(release_links::prefers_zip_asset(macos, false));
    assert!(!release_links::prefers_zip_asset(macos, true));
}

#[test]
fn an_asset_zip_is_checked_against_its_own_checksum() {
    let notes = release_notes();

    assert_eq!(
        release_links::find_sha256_near(&notes, WINDOWS_URL).as_deref(),
        Some(WIN_HASH)
    );
    assert_eq!(
        release_links::find_sha256_near(&notes, MAC_UPDATE_URL).as_deref(),
        Some(MAC_HASH)
    );
    // A URL the notes never mention has no checksum, so the updater cannot verify it against
    // another download's hash and leaves it unchecked.
    assert_eq!(
        release_links::find_sha256_near(&notes, "https://example.com/other.zip"),
        None
    );
}

#[test]
fn an_asset_zip_never_borrows_another_downloads_hash() {
    let notes = release_notes();

    assert_eq!(
        release_links::zip_checksum(&notes, WINDOWS_URL, true).as_deref(),
        Some(WIN_HASH)
    );
    // The notes' first checksum belongs to the macOS overlay; an asset URL that is absent from
    // them must not inherit it, so the download stays unverified instead of failing wrongly.
    assert_eq!(
        release_links::zip_checksum(&notes, "https://example.com/unknown.zip", true),
        None
    );
}

#[test]
fn a_body_linked_zip_may_print_its_hash_anywhere() {
    let notes = format!("sha256 {MAC_HASH}\n\nIn-place update: {MAC_UPDATE_URL}");

    assert_eq!(
        release_links::zip_checksum(&notes, MAC_UPDATE_URL, false).as_deref(),
        Some(MAC_HASH)
    );
}

#[test]
fn the_hash_may_sit_on_the_same_line_as_the_link() {
    let notes = format!("Windows x64: {WINDOWS_URL} sha256 {WIN_HASH}");

    assert_eq!(
        release_links::find_sha256_near(&notes, WINDOWS_URL).as_deref(),
        Some(WIN_HASH)
    );
}
