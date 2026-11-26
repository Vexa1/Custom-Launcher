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

/// The SHA-256 printed for the archive at `url`: a token on the same line, otherwise the first
/// one after the link.
///
/// A release lists several downloads, so this never looks at the text before the URL and never
/// falls back to a body-wide search: a release-asset zip must be verified against its own
/// checksum, or not at all.
pub fn find_sha256_near(text: &str, url: &str) -> Option<String> {
    let tail = text.get(text.find(url)? + url.len()..)?;
    let line = tail.split('\n').next().unwrap_or(tail);
    find_sha256(line).or_else(|| find_sha256(tail))
}

/// Whether a `.zip` asset name is the one this platform's in-app updater should take. Windows
/// looks for the x64 build; macOS takes the project archive.
pub fn prefers_zip_asset(name: &str, windows: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    if windows {
        lower.contains("windows")
    } else {
        // `CREBLOX-<version>-windows-x64.zip` also contains "creblox", so the Windows build has
        // to be excluded by name or a Mac would download an exe it cannot run.
        !lower.contains("windows") && (lower.contains("creblox") || lower.contains("macos"))
    }
}

/// The first zip link in a release body, preferring this platform's build.
///
/// macOS publishes the small update overlay next to the full project zip and wants the overlay,
/// so an in-app update downloads ~347 KB instead of ~677 KB. Windows wants its own x64 build.
pub fn pick_zip_url(text: &str, windows: bool) -> Option<String> {
    let mut first: Option<String> = None;
    for token in text.split_whitespace() {
        let Some(start) = token.find("https://") else {
            continue;
        };
        let candidate =
            token[start..].trim_end_matches([')', ']', '}', '>', '.', ',', ';', '`', '"', '\'']);
        let lower = candidate.to_ascii_lowercase();
        let path = lower.split(['?', '#']).next().unwrap_or("");
        if !path.ends_with(".zip") {
            continue;
        }
        let file = path.rsplit('/').next().unwrap_or(path);
        let wanted = if windows {
            file.contains("windows")
        } else {
            file.contains("update") && !file.contains("full")
        };
        if wanted {
            return Some(candidate.to_string());
        }
        if first.is_none() {
            first = Some(candidate.to_string());
        }
    }
    first
}
