//! The self-check for generated release notes.
//!
//! `scripts/release-notes.sh` writes `release/CREBLOX-<version>-notes.md` from the packaged
//! archives. This test re-reads every notes file in `release/`, parses it with the same
//! `release_links` code the updater uses, and compares each printed size and SHA-256 with the
//! zip on disk. A regenerated body whose hash drifted from its bytes, or a hand-edit that moved
//! a checksum away from its link, fails here before anything is published.

use std::fs;
use std::path::{Path, PathBuf};

use creblox_vm_tests::{release_links, sha256};

const BASE: &str = "https://github.com/Vexa1/Custom-Launcher/raw/main/release";

fn notes_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../release");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-notes.md"))
        })
        .collect();
    out.sort();
    out
}

/// `CREBLOX-0.5.42-notes.md` -> `0.5.42`.
fn version_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("CREBLOX-")?
        .strip_suffix("-notes.md")
        .map(str::to_string)
}

#[test]
fn every_generated_notes_file_matches_its_archives() {
    let files = notes_files();
    assert!(
        !files.is_empty(),
        "no release notes found: run scripts/release-notes.sh for a released version"
    );

    for path in files {
        let notes = fs::read_to_string(&path).expect("notes file is readable");
        let version = version_of(&path).expect("notes file is named CREBLOX-<version>-notes.md");

        for kind in ["update", "full"] {
            let zip_name = format!("CREBLOX-{version}-{kind}.zip");
            let zip_path = path.with_file_name(&zip_name);
            let url = format!("{BASE}/{zip_name}");
            assert!(zip_path.exists(), "{} names a missing archive", path.display());

            // The archive's line must carry BOTH the link and the checksum. This is the drift
            // that broke 0.5.39: the hash sat on the line above the wrapped link.
            let line = notes
                .lines()
                .find(|line| line.contains(&zip_name))
                .unwrap_or_else(|| panic!("{} has no line for {zip_name}", path.display()));
            assert!(
                line.contains(&url),
                "{zip_name} must link to {url} on its own line"
            );
            assert!(
                line.contains("SHA-256"),
                "{zip_name} must print its checksum on the same line as its link"
            );

            let size = fs::metadata(&zip_path).expect("archive metadata").len();
            assert!(
                line.contains(&format!("{size} bytes")),
                "{zip_name} should print its real size ({size})"
            );

            let printed = release_links::zip_checksum(&notes, &url, false)
                .unwrap_or_else(|| panic!("{zip_name} has no parsed checksum"));
            let actual = sha256::file_hex(&zip_path).expect("hash the archive");
            assert_eq!(printed, actual, "{zip_name}: printed hash is not the archive's");
        }

        // And the updater picks the quick update, verified against that same hash.
        let update_url = format!("{BASE}/CREBLOX-{version}-update.zip");
        let (picked, hash) = release_links::notes_zip(&notes, false)
            .unwrap_or_else(|| panic!("{} has no installable download", path.display()));
        assert_eq!(picked, update_url, "the update archive must come first");
        assert_eq!(
            hash.as_deref(),
            Some(sha256_of(&path, &version, "update").as_str()),
            "the picked download's checksum must be the update archive's"
        );
    }
}

fn sha256_of(notes: &Path, version: &str, kind: &str) -> String {
    let zip = notes.with_file_name(format!("CREBLOX-{version}-{kind}.zip"));
    sha256::file_hex(&zip).expect("hash the archive")
}
