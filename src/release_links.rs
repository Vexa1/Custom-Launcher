//! Choosing which download in a release's notes or assets belongs to this platform, and the
//! SHA-256 printed for it.
//!
//! Kept free of egui, HTTP and macOS so `vm-tests` compiles this exact file and runs it on any
//! host, including the Linux VM where the GUI crate does not build.

/// Any 64-hex token: that is how the notes print checksums.
pub fn find_sha256(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .find(|token| token.len() == 64)
        .map(|token| token.to_ascii_lowercase())
}

/// The SHA-256 printed for the archive at `url`.
///
/// The notes print a download's file name, its size and its checksum in one bullet, and where
/// the link wraps onto the next line the checksum ends up above it. Anchoring on the link alone
/// therefore paired the update zip with the *next* archive's checksum, and every update failed
/// verification. So the archive's file name is tried first, then the link's own line, then the
/// lines around it - never a body-wide search, because a release lists several downloads and
/// only this one's checksum may be used.
pub fn find_sha256_near(text: &str, url: &str) -> Option<String> {
    let file = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("");
    if !file.is_empty() {
        if let Some(sum) = bullet_sha256(text, file) {
            return Some(sum);
        }
    }
    let start = text.find(url)?;
    let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    // The link's own line plus the two after it: a checksum printed under the link still
    // belongs to it, and the window is too small to reach another download's bullet.
    let mut end = start;
    for _ in 0..3 {
        match text[end..].find('\n') {
            Some(i) => end += i + 1,
            None => {
                end = text.len();
                break;
            }
        }
    }
    find_sha256(&text[line_start..end])
}

/// The checksum in the bullet that names `file`: its line, the line above it when the checksum
/// was printed first, and at most three lines after it. Stops at a blank line or a new bullet.
fn bullet_sha256(text: &str, file: &str) -> Option<String> {
    for (at, _) in text.match_indices(file) {
        // The line that names the archive starts its bullet, so it holds the size and, in the
        // notes we write, the checksum: no line above it is ever read, because that is another
        // download's link and checksum.
        let start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let mut end = at;
        let mut lines = 0;
        while lines < 4 {
            match text[end..].find('\n') {
                Some(i) => {
                    let line = &text[end..end + i];
                    if lines > 0 && (line.trim().is_empty() || line.trim_start().starts_with("- "))
                    {
                        break;
                    }
                    end += i + 1;
                    lines += 1;
                }
                None => {
                    end = text.len();
                    break;
                }
            }
        }
        if let Some(sum) = find_sha256(&text[start..end]) {
            return Some(sum);
        }
    }
    None
}

/// The checksum the updater should verify the zip at `zip_url` against.
///
/// `from_asset` says the zip came from a release asset rather than a link written into the
/// notes. An asset is only ever checked against the hash printed for its own URL: the notes list
/// several downloads, and a body-wide search would hand the Windows build the macOS overlay's
/// checksum and reject a good update. A hand-written body link keeps the older fallback, because
/// its hash may be printed anywhere in the notes.
pub fn zip_checksum(notes: &str, zip_url: &str, from_asset: bool) -> Option<String> {
    let near = find_sha256_near(notes, zip_url);
    if from_asset {
        near
    } else {
        near.or_else(|| find_sha256(notes))
    }
}

/// A macOS release archive: `CREBLOX-<version>-update.zip` (or the full project zip), never the
/// Windows build.
///
/// `CREBLOX-<version>-windows-x64.zip` also contains "creblox", so the Windows build is excluded
/// by name and by the `.zip` requirement, or a Mac would try to install an archive it cannot run.
pub fn is_macos_zip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip")
        && !lower.contains("windows")
        && (lower.contains("creblox") || lower.contains("macos"))
}

/// A Windows release archive: a CREBLOX zip that names the platform and the architecture.
///
/// Deliberately stricter than "contains windows": the Windows installer swaps an exe out of
/// whatever it downloads, so a helper zip that happened to carry the word must never qualify.
pub fn is_windows_zip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("creblox-")
        && lower.ends_with(".zip")
        && lower.contains("windows")
        && lower.contains("x64")
}

/// The one file name a release must use for its Windows build.
pub fn windows_zip_name(version: &str) -> String {
    format!("CREBLOX-{version}-windows-x64.zip")
}

/// Whether `name` is exactly the Windows build for `version`.
///
/// This is what the release parser asks for, which is why the version is compared and not just
/// the shape: an archive built for another version (or another product) is not this update, and
/// the installer only ever swaps a file it is sure about. A release that renames the asset fails
/// here instead of installing something unexpected.
pub fn is_windows_zip_for(name: &str, version: &str) -> bool {
    name.eq_ignore_ascii_case(&windows_zip_name(version))
}

/// One release asset as GitHub lists it: file name, download URL and size in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZipAsset<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub size: u64,
}

/// The release asset this platform may install, scanning in the order GitHub lists them.
///
/// The Windows job attaches `CREBLOX-<version>-windows-x64.zip` next to the macOS archives, and
/// clients that shipped before that change install the first zip asset they see. So the foreign
/// build is skipped here, not merely ordered last: a Mac that took the Windows zip would try to
/// swap in an .exe, and a Windows client that took the macOS overlay would unzip a source tree
/// instead of a launcher. `None` means this release has no build for this platform, so the caller
/// falls back to a zip link written into the notes.
///
/// With `version` the Windows match is the exact `CREBLOX-<version>-windows-x64.zip` name from
/// the release; without it the structurally stricter [`is_windows_zip`] rule applies.
pub fn pick_zip_asset<'a>(
    assets: &[ZipAsset<'a>],
    windows: bool,
    version: Option<&str>,
) -> Option<ZipAsset<'a>> {
    assets.iter().copied().find(|asset| {
        if !windows {
            return is_macos_zip(asset.name);
        }
        match version {
            Some(version) => is_windows_zip_for(asset.name, version),
            None => is_windows_zip(asset.name),
        }
    })
}

/// The zip link in a release body that this platform can install.
///
/// macOS publishes the small update overlay next to the full project zip and wants the overlay,
/// so an in-app update downloads ~347 KB instead of ~677 KB. Windows wants its own x64 build.
/// A link for the other platform is skipped and never used as a fallback: there is no archive in
/// a body that both systems could install, so falling back would only download a broken update.
pub fn pick_zip_url(text: &str, windows: bool) -> Option<String> {
    let mut macos_other: Option<String> = None;
    for token in text.split_whitespace() {
        let Some(start) = token.find("https://") else {
            continue;
        };
        let candidate =
            token[start..].trim_end_matches([')', ']', '}', '>', '.', ',', ';', '`', '"', '\'']);
        let lower = candidate.to_ascii_lowercase();
        let path = lower.split(['?', '#']).next().unwrap_or("");
        let file = path.rsplit('/').next().unwrap_or(path);
        if windows {
            if is_windows_zip(file) {
                return Some(candidate.to_string());
            }
        } else if is_macos_zip(file) {
            if file.contains("update") && !file.contains("full") {
                return Some(candidate.to_string());
            }
            if macos_other.is_none() {
                macos_other = Some(candidate.to_string());
            }
        }
    }
    if windows {
        None
    } else {
        macos_other
    }
}

/// The body-listed archive for this platform and the SHA-256 its download must match, exactly as
/// the release parser applies them. `None` means the notes have no archive this platform can
/// install, which is how a tag without a Windows build reads as "no update".
pub fn notes_zip(notes: &str, windows: bool) -> Option<(String, Option<String>)> {
    let url = pick_zip_url(notes, windows)?;
    let sum = zip_checksum(notes, &url, false);
    Some((url, sum))
}
