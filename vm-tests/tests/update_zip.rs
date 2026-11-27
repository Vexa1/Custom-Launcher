//! The Windows self-update reads a release zip and picks the right download. Both halves live in
//! host-independent modules (`src/release_links.rs`, `src/zip_update.rs`), so this suite runs the
//! real code on the Linux VM where the GUI crate does not build.

use creblox_vm_tests::{release_links, sha256, zip_update};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The asset name the `build-windows` job attaches for 0.5.6, URL included: the parser asks
/// for exactly this name, so the fixture drifts the moment the workflow does.
const WIN_056_NAME: &str = "CREBLOX-0.5.6-windows-x64.zip";
const WIN_056_URL: &str =
    "https://github.com/Vexa1/Custom-Launcher/releases/download/v0.5.6/CREBLOX-0.5.6-windows-x64.zip";

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

/// SHA-256 of some bytes, through the streaming API the updater uses for a download.
fn sha_hex(data: &[u8]) -> String {
    let mut hasher = sha256::Sha256::new();
    hasher.update(data);
    hasher.finish_hex()
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

/// The Windows CI job points this at the exe it has just built, so the swap checks run against a
/// real link product instead of a fixture. A dev box has no Windows build, so the test is a no-op
/// there - and CI fails loudly if it forgets to set the variable, rather than passing quietly.
#[test]
fn the_built_launcher_passes_its_own_check() {
    let Some(path) = std::env::var_os("CREBLOX_LAUNCHER") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must set CREBLOX_LAUNCHER to the creblox.exe it just built"
        );
        return;
    };

    let path = PathBuf::from(path);
    let size = zip_update::verify_launcher(&path)
        .unwrap_or_else(|e| panic!("the built launcher must pass the swap checks: {e}"));

    assert_eq!(size, fs::metadata(&path).expect("launcher metadata").len());
    assert!(size > zip_update::MIN_LAUNCHER_BYTES);
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
fn a_body_without_this_platforms_archive_offers_no_update() {
    let macos_only = format!("Full build: {MAC_FULL_URL}\nUpdate: {MAC_UPDATE_URL}");

    assert_eq!(
        release_links::pick_zip_url(&macos_only, true),
        None,
        "Windows never falls back to a macOS overlay"
    );
    assert_eq!(
        release_links::notes_zip(&macos_only, true),
        None,
        "no Windows archive means no Windows update, not a foreign one"
    );
    assert_eq!(
        release_links::pick_zip_url("no links here", true),
        None,
        "a body without downloads has nothing to offer"
    );
}

#[test]
fn a_multi_os_release_body_gives_each_platform_only_its_own_archive() {
    // The shape the Windows job appends: both archives listed, each with its own SHA-256, the
    // macOS update first so pre-0.5.5 installs (which take the first zip) keep working.
    let notes = release_notes();

    let macos = release_links::notes_zip(&notes, false).expect("a macOS archive");
    assert_eq!(macos.0, MAC_UPDATE_URL);
    assert_eq!(macos.1.as_deref(), Some(MAC_HASH));
    assert!(
        !macos.0.to_ascii_lowercase().contains("windows") && !macos.0.ends_with(".exe"),
        "a Mac must never be handed the Windows build: {}",
        macos.0
    );

    let windows = release_links::notes_zip(&notes, true).expect("a Windows archive");
    assert_eq!(windows.0, WINDOWS_URL);
    assert_eq!(windows.1.as_deref(), Some(WIN_HASH));
    assert!(
        windows.0.to_ascii_lowercase().contains("windows"),
        "Windows must only accept its own x64 build: {}",
        windows.0
    );

    assert_ne!(macos.0, windows.0, "the two platforms get different files");
    assert_ne!(macos.1, windows.1, "each archive keeps its own checksum");
}

#[test]
fn the_macos_archive_wins_even_when_a_windows_zip_is_listed_first() {
    let notes = format!(
        "Windows x64: {WINDOWS_URL} sha256 {WIN_HASH}\n\n\
         macOS update: {MAC_UPDATE_URL} sha256 {MAC_HASH}"
    );

    assert_eq!(
        release_links::notes_zip(&notes, false).map(|(url, _)| url).as_deref(),
        Some(MAC_UPDATE_URL)
    );
    assert_eq!(
        release_links::notes_zip(&notes, true).map(|(url, _)| url).as_deref(),
        Some(WINDOWS_URL)
    );
}

#[test]
fn a_foreign_archive_is_never_a_notes_fallback() {
    let windows_only = format!("Windows x64: {WINDOWS_URL}\nsha256 {WIN_HASH}");
    let macos_only = format!("Update: {MAC_UPDATE_URL}\nsha256 {MAC_HASH}");

    assert_eq!(
        release_links::notes_zip(&windows_only, false),
        None,
        "a macOS build must not download the Windows zip"
    );
    assert_eq!(
        release_links::notes_zip(&macos_only, true),
        None,
        "a Windows build must not download the macOS overlay"
    );
}

#[test]
fn an_exe_link_is_not_a_macos_archive() {
    let notes = "Download: https://github.com/Vexa1/Custom-Launcher/releases/download/v0.5.5/creblox.exe";

    assert_eq!(release_links::pick_zip_url(notes, false), None);
    assert_eq!(release_links::notes_zip(notes, false), None);
    assert!(!release_links::is_macos_zip("creblox.exe"));
    assert!(!release_links::is_windows_zip("creblox.exe"));
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

    assert!(release_links::is_windows_zip(windows));
    assert!(!release_links::is_windows_zip(macos));
    assert!(release_links::is_macos_zip(macos));
    assert!(!release_links::is_macos_zip(windows));
}

fn asset<'a>(name: &'a str, url: &'a str, size: u64) -> release_links::ZipAsset<'a> {
    release_links::ZipAsset { name, url, size }
}

#[test]
fn a_foreign_zip_asset_is_ignored_not_merely_ordered_last() {
    // The Windows job attaches its zip next to the macOS archives. Clients that shipped before
    // that install the first zip asset they see, so the rule has to be "ignore", never "prefer".
    let release = [
        asset("CREBLOX-0.5.5-windows-x64.zip", WINDOWS_URL, 8_773_620),
        asset("CREBLOX-0.5.5-update.zip", MAC_UPDATE_URL, 398_986),
    ];

    assert_eq!(
        release_links::pick_zip_asset(&release, false, None).map(|a| a.url),
        Some(MAC_UPDATE_URL),
        "a Mac must skip the .exe zip even when it is listed first"
    );
    assert_eq!(
        release_links::pick_zip_asset(&release, true, None).map(|a| a.url),
        Some(WINDOWS_URL),
        "Windows must skip the macOS overlay"
    );
}

#[test]
fn the_first_usable_asset_wins() {
    let release = [
        asset("CREBLOX-0.5.5-update.zip", MAC_UPDATE_URL, 398_986),
        asset("CREBLOX-0.5.5-full.zip", MAC_FULL_URL, 730_944),
    ];

    // The overlay is published first so the in-app update stays small.
    let picked = release_links::pick_zip_asset(&release, false, None).expect("a macOS build");
    assert_eq!(picked.name, "CREBLOX-0.5.5-update.zip");
    assert_eq!(picked.size, 398_986, "the size comes from the asset");
}

#[test]
fn a_release_without_this_platforms_asset_picks_nothing() {
    let mac_only = [asset("CREBLOX-0.5.5-update.zip", MAC_UPDATE_URL, 398_986)];
    let windows_only = [asset("CREBLOX-0.5.5-windows-x64.zip", WINDOWS_URL, 8_773_620)];

    // None is what makes the updater fall back to the notes link instead of installing a build
    // the platform cannot run.
    assert_eq!(release_links::pick_zip_asset(&mac_only, true, None), None);
    assert_eq!(release_links::pick_zip_asset(&windows_only, false, None), None);
    assert_eq!(release_links::pick_zip_asset(&[], false, None), None);
}

#[test]
fn only_zip_assets_are_candidates() {
    let notes = [asset("creblox-0.5.5-notes.txt", "https://example.com/notes.txt", 10)];

    assert_eq!(
        release_links::pick_zip_asset(&notes, false, None),
        None,
        "a name alone is not an updatable archive"
    );
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

// ---------------------------------------------------------------------------
// SHA-256: the check that stands between a downloaded zip and the installed exe.
// ---------------------------------------------------------------------------

#[test]
fn sha256_matches_the_published_vectors() {
    assert_eq!(
        sha_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn a_megabyte_is_hashed_across_blocks() {
    // One million `a`: the classic long-message vector, and the size class of a real release
    // archive, so padding across many blocks is exercised.
    let data = vec![b'a'; 1_000_000];
    assert_eq!(
        sha_hex(&data),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn streaming_in_short_chunks_gives_the_same_digest() {
    let mut hasher = sha256::Sha256::new();
    for byte in b"The file is read in whatever chunks the OS returns" {
        hasher.update(&[*byte]);
    }
    assert_eq!(
        hasher.finish_hex(),
        sha_hex(b"The file is read in whatever chunks the OS returns")
    );
}

#[test]
fn the_file_digest_matches_its_bytes() {
    let scratch = Scratch::new("sha-file");
    let file = scratch.path().join("CREBLOX-0.5.6-windows-x64.zip");
    fs::write(&file, b"abc").expect("write archive");

    assert_eq!(
        sha256::file_hex(&file).expect("hash the file"),
        sha_hex(b"abc")
    );
}

// ---------------------------------------------------------------------------
// Release-asset selection: exactly one Windows zip, matched by name and version.
// ---------------------------------------------------------------------------

#[test]
fn the_windows_asset_must_be_the_exact_release_zip() {
    let release = [
        asset("CREBLOX-0.5.6-update.zip", MAC_UPDATE_URL, 412_856),
        asset("CREBLOX-0.5.6-full.zip", MAC_FULL_URL, 745_330),
        asset(WIN_056_NAME, WIN_056_URL, 8_959_062),
        asset(
            "CREBLOX-0.5.5-windows-x64.zip",
            "https://example.com/old.zip",
            8_773_620,
        ),
    ];

    let picked = release_links::pick_zip_asset(&release, true, Some("0.5.6"))
        .expect("this release has a Windows build");
    assert_eq!(picked.name, WIN_056_NAME);
    assert_eq!(picked.url, WIN_056_URL);
    assert_eq!(picked.size, 8_959_062, "the size comes from the asset");

    // macOS still takes the update overlay, never the Windows exe.
    let mac = release_links::pick_zip_asset(&release, false, Some("0.5.6")).expect("a macOS build");
    assert_eq!(mac.name, "CREBLOX-0.5.6-update.zip");
}

#[test]
fn a_windows_zip_for_another_version_is_not_this_update() {
    let release = [
        asset("CREBLOX-0.5.5-windows-x64.zip", WINDOWS_URL, 8_773_620),
        asset("CREBLOX-0.5.6-windows-x64.zip.bak", WIN_056_URL, 8_959_062),
    ];

    assert_eq!(
        release_links::pick_zip_asset(&release, true, Some("0.5.6")),
        None,
        "a zip built for 0.5.5 must not be installed as 0.5.6"
    );
}

#[test]
fn a_zip_that_is_not_the_launcher_build_is_ignored() {
    for name in [
        "CREBLOX-0.5.6-windows.zip",
        "creblox-windows-helper.zip",
        "other-product-windows-x64.zip",
        "CREBLOX-0.5.6-windows-x64.exe",
    ] {
        let release = [asset(name, "https://example.com/x.zip", 1024)];
        assert_eq!(
            release_links::pick_zip_asset(&release, true, None),
            None,
            "{name} is not the Windows launcher archive"
        );
    }
}

#[test]
fn the_windows_zip_name_is_the_one_ci_publishes() {
    assert_eq!(
        release_links::windows_zip_name("0.5.6"),
        "CREBLOX-0.5.6-windows-x64.zip"
    );
    assert!(release_links::is_windows_zip_for(WIN_056_NAME, "0.5.6"));
    assert!(release_links::is_windows_zip_for(
        "creblox-0.5.6-windows-x64.zip",
        "0.5.6"
    ));
    assert!(!release_links::is_windows_zip_for(WIN_056_NAME, "0.5.5"));
}

#[test]
fn the_appended_asset_lines_give_windows_its_own_hash() {
    // The exact shape the Windows job appends to the notes: each asset URL followed by the
    // `sha256` line for that file.
    let notes = format!(
        "A release.\n\n### Downloads\n- **Update**: [CREBLOX-0.5.6-update.zip](https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.6-update.zip) — 412,856 B, `{MAC_HASH}`\n\n\
         macOS update (release asset): https://github.com/Vexa1/Custom-Launcher/releases/download/v0.5.6/CREBLOX-0.5.6-update.zip\nsha256 {MAC_HASH}\n\
         Windows x64: {WIN_056_URL}\nsha256 {WIN_HASH}"
    );

    assert_eq!(
        release_links::zip_checksum(&notes, WIN_056_URL, true).as_deref(),
        Some(WIN_HASH),
        "the Windows asset is verified against the hash printed under its own URL"
    );
    assert_eq!(
        release_links::notes_zip(&notes, true).map(|(url, _)| url).as_deref(),
        Some(WIN_056_URL),
        "and it is the link the Windows updater would download"
    );
}

// ---------------------------------------------------------------------------
// The archive layout the workflow packages.
// ---------------------------------------------------------------------------

#[test]
fn the_ci_artifact_layout_installs() {
    // `Compress-Archive -Path dist/creblox.exe` puts the launcher at the archive root.
    let scratch = Scratch::new("ci-layout");
    let zip_path = scratch.path().join(WIN_056_NAME);
    let payload = launcher_bytes(90 * 1024);
    write_zip(&zip_path, &[("creblox.exe", &payload)]);

    let dest = scratch.path().join("out");
    let exe = zip_update::extract_launcher(&zip_path, &dest).expect("the CI layout extracts");
    assert_eq!(exe, dest.join("creblox.exe"));
    assert_eq!(
        zip_update::verify_launcher(&exe).expect("the extracted launcher verifies"),
        payload.len() as u64
    );
}
