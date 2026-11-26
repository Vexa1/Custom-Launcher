use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense};
use serde_json::Value;

use crate::auth::load_settings;
use crate::theme::{apply_theme, install_fonts, theme_accent};

pub const CREBLOX_REPO: &str = "Vexa1/Custom-Launcher";
pub const CREBLOX_RELEASES_URL: &str = "https://github.com/Vexa1/Custom-Launcher/releases";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const UPDATE_TOKEN_SERVICE: &str = "CREBLOX Update Token";

/// Optional fine-grained GitHub token (Contents: Read) for the private repository.
/// It is kept in the macOS Keychain, never in settings.json.
#[cfg(target_os = "macos")]
pub fn github_update_token() -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", UPDATE_TOKEN_SERVICE, "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

#[cfg(not(target_os = "macos"))]
pub fn github_update_token() -> Option<String> {
    None
}

pub fn has_github_update_token() -> bool {
    github_update_token().is_some()
}

#[cfg(target_os = "macos")]
pub fn save_github_update_token(token: &str) -> Result<(), String> {
    let out = Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-s",
            UPDATE_TOKEN_SERVICE,
            "-a",
            "creblox",
            "-w",
            token,
            "-U",
        ])
        .output()
        .map_err(|e| format!("Could not reach the macOS Keychain: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err("Could not save the GitHub token in the macOS Keychain.".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn save_github_update_token(_token: &str) -> Result<(), String> {
    Err("GitHub update tokens are only supported on macOS.".to_string())
}

#[cfg(target_os = "macos")]
pub fn clear_github_update_token() -> Result<(), String> {
    let out = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-s",
            UPDATE_TOKEN_SERVICE,
            "-a",
            "creblox",
        ])
        .output()
        .map_err(|e| format!("Could not reach the macOS Keychain: {e}"))?;
    // Deleting an item that is already gone is fine.
    if out.status.success() || String::from_utf8_lossy(&out.stderr).contains("could not be found") {
        Ok(())
    } else {
        Err("Could not remove the GitHub token from the macOS Keychain.".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn clear_github_update_token() -> Result<(), String> {
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateKind {
    Patch,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrebloxRelease {
    pub version: String,
    pub tag_name: String,
    pub kind: UpdateKind,
    pub title: String,
    pub notes: String,
    pub html_url: String,
    pub exe_url: Option<String>,
    pub exe_name: Option<String>,
    pub exe_size: u64,
    /// macOS source-update zip, taken from a release asset or the release body.
    pub zip_url: Option<String>,
    pub zip_name: Option<String>,
    pub zip_size: u64,
    pub zip_sha256: Option<String>,
    pub hotfix_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveHotfix {
    pub fastflag_overrides: HashMap<String, Value>,
    pub auto_close_roblox: Option<bool>,
    pub status_note: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UpdaterArgs {
    pub target_exe: PathBuf,
    pub from_ver: String,
    pub to_ver: String,
    pub exe_url: String,
    pub exe_size: u64,
}

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next().unwrap_or("").trim();
    if core.is_empty() {
        return None;
    }
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer_version(remote: &str, current: &str) -> bool {
    match (parse_semver(remote), parse_semver(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

pub fn classify_update_kind(remote: &str, current: &str, title: &str, notes: &str) -> UpdateKind {
    let lower_meta = format!("{title}\n{notes}").to_ascii_lowercase();
    if lower_meta.contains("[full]") || lower_meta.contains("[restart]") {
        return UpdateKind::Full;
    }
    if lower_meta.contains("[patch]") || lower_meta.contains("[hotfix]") {
        return UpdateKind::Patch;
    }
    match (parse_semver(remote), parse_semver(current)) {
        (Some((rmaj, rmin, rpat)), Some((cmaj, cmin, cpat))) => {
            if rmaj == cmaj && rmin == cmin && rpat > cpat {
                UpdateKind::Patch
            } else {
                UpdateKind::Full
            }
        }
        _ => UpdateKind::Full,
    }
}

/// Finds the first HTTPS `.zip` link in release notes. Release bodies use both bare
/// URLs and Markdown links, so trailing punctuation is stripped.
fn find_zip_url(text: &str) -> Option<String> {
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
        // The body publishes the update overlay next to the full project zip.
        // Prefer the overlay so an in-app update downloads ~347 KB, not ~677 KB.
        let file = path.rsplit('/').next().unwrap_or(path);
        if file.contains("update") && !file.contains("full") {
            return Some(candidate.to_string());
        }
        if first.is_none() {
            first = Some(candidate.to_string());
        }
    }
    first
}

/// The release body carries the SHA-256 of the update zip; accept any 64-hex token.
fn find_sha256(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .find(|token| token.len() == 64)
        .map(|token| token.to_ascii_lowercase())
}

/// The SHA-256 printed for the archive at `url`: the token on the same line,
/// otherwise the first one after the link. The body lists several downloads, so
/// taking the first hash in the body would verify the overlay against the full
/// zip's checksum once the overlay is preferred.
fn find_sha256_for(text: &str, url: &str) -> Option<String> {
    let tail = text.get(text.find(url)? + url.len()..)?;
    let line = tail.split('\n').next().unwrap_or(tail);
    find_sha256(line)
        .or_else(|| find_sha256(tail))
        .or_else(|| find_sha256(text))
}

pub fn parse_github_release_update(v: &Value, current_version: &str) -> Option<CrebloxRelease> {
    if v["draft"].as_bool().unwrap_or(false) {
        return None;
    }
    let tag_name = v["tag_name"].as_str()?.trim().to_string();
    if !is_newer_version(&tag_name, current_version) {
        return None;
    }
    let normalized_ver = parse_semver(&tag_name)
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .unwrap_or_else(|| tag_name.trim_start_matches(['v', 'V']).to_string());
    let title = v["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&tag_name)
        .to_string();
    let notes = v["body"].as_str().map(str::trim).unwrap_or("").to_string();
    let html_url = v["html_url"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(CREBLOX_RELEASES_URL)
        .to_string();
    let kind = classify_update_kind(&tag_name, current_version, &title, &notes);

    let mut chosen_exe: Option<(String, String, u64, bool)> = None;
    let mut chosen_zip: Option<(String, String, u64, bool)> = None;
    let mut hotfix_url: Option<String> = None;
    if let Some(assets) = v["assets"].as_array() {
        for a in assets {
            let Some(name) = a["name"].as_str() else {
                continue;
            };
            let Some(url) = a["browser_download_url"].as_str() else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower == "hotfix.json" || lower.ends_with(".hotfix.json") {
                hotfix_url = Some(url.to_string());
                continue;
            }
            if lower.ends_with(".zip") {
                let preferred = lower.contains("creblox") || lower.contains("macos");
                let size = a["size"].as_u64().unwrap_or(0);
                let replace = match &chosen_zip {
                    None => true,
                    Some((_, _, _, was_pref)) => !was_pref && preferred,
                };
                if replace {
                    chosen_zip = Some((url.to_string(), name.to_string(), size, preferred));
                }
                continue;
            }
            // macOS builds are distributed as an app bundle / zip from the
            // Releases page, not as a bare Windows .exe, so only Windows looks
            // for an in-place binary asset here.
            if cfg!(target_os = "macos") {
                continue;
            }
            if !lower.ends_with(".exe") {
                continue;
            }
            let preferred = lower.contains("creblox");
            let size = a["size"].as_u64().unwrap_or(0);
            let replace = match &chosen_exe {
                None => true,
                Some((_, _, _, was_pref)) => !was_pref && preferred,
            };
            if replace {
                chosen_exe = Some((url.to_string(), name.to_string(), size, preferred));
            }
        }
    }

    let (exe_url, exe_name, exe_size) = match chosen_exe {
        Some((u, n, sz, _)) => (Some(u), Some(n), sz),
        None => (None, None, 0),
    };
    // The current release flow publishes an update zip link in the release body
    // (GitHub assets need a paid storage key), so fall back to scanning the body.
    let has_asset_zip = chosen_zip.is_some();
    let (zip_url, zip_name, zip_size) = match chosen_zip {
        Some((u, n, sz, _)) => (Some(u), Some(n), sz),
        None => match find_zip_url(&notes) {
            Some(u) => {
                let name = u
                    .rsplit('/')
                    .next()
                    .unwrap_or("creblox-update.zip")
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("creblox-update.zip")
                    .to_string();
                (Some(u), Some(name), 0)
            }
            None => (None, None, 0),
        },
    };
    let zip_sha256 = match (&zip_url, has_asset_zip) {
        (Some(url), false) => find_sha256_for(&notes, url),
        _ => None,
    };

    Some(CrebloxRelease {
        version: normalized_ver,
        tag_name,
        kind,
        title,
        notes,
        html_url,
        exe_url,
        exe_name,
        exe_size,
        zip_url,
        zip_name,
        zip_size,
        zip_sha256,
        hotfix_url,
    })
}

pub fn check_creblox_update(token: Option<&str>) -> Result<Option<CrebloxRelease>, String> {
    if CREBLOX_REPO.is_empty() {
        return Ok(None);
    }
    let url = format!("https://api.github.com/repos/{CREBLOX_REPO}/releases/latest");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout_read(Duration::from_secs(12))
        .build();
    let mut req = agent
        .get(&url)
        .set("User-Agent", concat!("creblox/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json");
    if let Some(token) = token {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => {
            return Err(if token.is_none() {
                "The update repository is private. Add a read-only GitHub token in Settings → Launcher Updates.".to_string()
            } else {
                "This GitHub token cannot see the update repository, or it has no releases."
                    .to_string()
            });
        }
        Err(ureq::Error::Status(401, _)) => {
            return Err(
                "GitHub rejected the update token. Replace it in Settings → Launcher Updates."
                    .to_string(),
            );
        }
        Err(ureq::Error::Status(403, _)) => {
            return Err("GitHub blocked or rate-limited the update check. Add a GitHub token in Settings → Launcher Updates, or try again later.".to_string());
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(format!("GitHub returned HTTP {code}"));
        }
        Err(e) => return Err(format!("Could not reach update server: {e}")),
    };
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(parse_github_release_update(
        &json,
        env!("CARGO_PKG_VERSION"),
    ))
}

pub fn parse_live_hotfix_str(text: &str) -> Result<LiveHotfix, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Invalid hotfix JSON: {e}"))?;
    let mut fastflag_overrides = HashMap::new();
    if let Some(obj) = v["fastflags"].as_object() {
        for (k, val) in obj {
            fastflag_overrides.insert(k.clone(), val.clone());
        }
    }
    let auto_close_roblox = v["auto_close_roblox"].as_bool();
    let status_note = v["note"]
        .as_str()
        .or_else(|| v["message"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(LiveHotfix {
        fastflag_overrides,
        auto_close_roblox,
        status_note,
    })
}

pub fn download_hotfix(url: &str) -> Result<LiveHotfix, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(6))
        .timeout_read(Duration::from_secs(12))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", concat!("creblox/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("Failed fetching hotfix: {e}"))?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    parse_live_hotfix_str(&text)
}

pub fn updates_dir() -> PathBuf {
    crate::auth::data_dir().join("updates")
}

pub fn downloaded_update_path(version: &str) -> PathBuf {
    updates_dir().join(format!("CREBLOX-{version}-update.zip"))
}

fn sha256_of(path: &Path) -> Result<String, String> {
    let out = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| {
            Command::new("shasum")
                .args(["-a", "256"])
                .arg(path)
                .output()
        })
        .map_err(|e| format!("Could not run shasum: {e}"))?;
    if !out.status.success() {
        return Err("Could not checksum the downloaded update.".to_string());
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| "Could not read the downloaded update checksum.".to_string())
}

/// Downloads the macOS source-update zip from the release body, verifies its zip
/// signature and (when the release publishes one) its SHA-256.
pub fn download_update_zip(
    release: &CrebloxRelease,
    downloaded: &AtomicU64,
    total: &AtomicU64,
    ctx: &egui::Context,
) -> Result<PathBuf, String> {
    let url = release
        .zip_url
        .as_deref()
        .ok_or_else(|| "This release has no update zip link.".to_string())?;
    fs::create_dir_all(updates_dir())
        .map_err(|e| format!("Could not create the update folder: {e}"))?;
    let dest = downloaded_update_path(&release.version);
    let part = dest.with_extension("zip.part");
    let _ = fs::remove_file(&part);

    downloaded.store(0, Ordering::Relaxed);
    total.store(release.zip_size, Ordering::Relaxed);
    ctx.request_repaint();

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(120))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", concat!("creblox/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("Update download failed: {e}"))?;

    if let Some(len) = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok())
    {
        if len > 0 {
            total.store(len, Ordering::Relaxed);
        }
    }

    let mut file =
        fs::File::create(&part).map_err(|e| format!("Could not write the update file: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            let _ = fs::remove_file(&part);
            format!("Update download interrupted: {e}")
        })?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| {
            let _ = fs::remove_file(&part);
            format!("Could not save the update file: {e}")
        })?;
        written += n as u64;
        downloaded.store(written, Ordering::Relaxed);
        ctx.request_repaint();
    }
    file.flush()
        .map_err(|e| format!("Could not finish the update download: {e}"))?;
    drop(file);

    let mut magic = [0u8; 4];
    let valid_zip = fs::File::open(&part)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic[..2] == b"PK";
    if !valid_zip {
        let _ = fs::remove_file(&part);
        return Err("Downloaded file is not a valid update zip.".to_string());
    }
    if let Some(expected) = &release.zip_sha256 {
        let actual = sha256_of(&part)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&part);
            return Err("Update zip failed SHA-256 verification.".to_string());
        }
    }
    fs::rename(&part, &dest).map_err(|e| format!("Could not store the update file: {e}"))?;
    Ok(dest)
}

pub fn source_folder() -> Option<PathBuf> {
    let raw = fs::read_to_string(crate::auth::data_dir().join("source-folder")).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.join("Cargo.toml").is_file() && path.join("scripts/update-macos.sh").is_file() {
        Some(path)
    } else {
        None
    }
}

fn is_update_root(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("scripts/update-macos.sh").is_file()
}

fn find_update_root(dir: &Path, depth: u8) -> Option<PathBuf> {
    if is_update_root(dir) {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_update_root(&path, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

fn extract_update_zip(zip: &Path, dest: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let out = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(zip)
        .arg(dest)
        .output();
    #[cfg(not(target_os = "macos"))]
    let out = Command::new("unzip")
        .args(["-q", "-o"])
        .arg(zip)
        .arg("-d")
        .arg(dest)
        .output();

    let out = out.map_err(|e| format!("Could not extract the update zip: {e}"))?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "Could not extract the update zip.".to_string()
        } else {
            format!("Could not extract the update zip: {detail}")
        });
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn launch_installer(script: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let path = script.to_string_lossy().into_owned();
        let opened = Command::new("/usr/bin/open")
            .args(["-a", "Terminal", &path])
            .status();
        if opened.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    Command::new("/bin/bash")
        .arg(script)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start the update installer: {e}"))
}

/// Extracts an update zip, copies it over the source folder and launches the normal
/// macOS update script (which rebuilds, quits the old app and reopens the new one).
pub fn install_update_zip(zip: &Path, version: &str) -> Result<(), String> {
    let source = source_folder().ok_or_else(|| {
        "Could not find the app source folder. Use Download in Browser / Releases once.".to_string()
    })?;
    let staging = updates_dir().join(format!("extract-{version}"));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("Could not clear the old update files: {e}"))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|e| format!("Could not create the update staging folder: {e}"))?;
    extract_update_zip(zip, &staging)?;
    let root = find_update_root(&staging, 2)
        .ok_or_else(|| "The update zip does not contain an app source folder.".to_string())?;

    let script = updates_dir().join(format!("install-{version}.sh"));
    let log_path = crate::auth::data_dir().join("update.log");
    // Output goes to the terminal *and* the log. Redirecting everything into the log made
    // the installer look frozen for the whole build, and a failure closed the window with
    // nothing on screen - the user could not tell waiting from broken.
    let body = format!(
        "#!/bin/bash\n\
         set -euo pipefail\n\
         export PATH=\"$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin\"\n\
         LOG={log}\n\
         SRC={source}\n\
         FROM={root}\n\
         {{\n\
         echo \"==> Installing CREBLOX {version}\"\n\
         echo \"    Building now; a copy of this output goes to: $LOG\"\n\
         if ! command -v cargo >/dev/null 2>&1; then\n\
         echo \"error: cargo was not found, so CREBLOX cannot be rebuilt.\" >&2\n\
         echo \"       Install Rust from https://rustup.rs and run the update again.\" >&2\n\
         exit 1\n\
         fi\n\
         if ! cargo --version >/dev/null 2>&1; then\n\
         echo \"error: cargo is installed but will not run (no default toolchain).\" >&2\n\
         echo \"       Run: rustup default stable\" >&2\n\
         exit 1\n\
         fi\n\
         if [ ! -d \"$FROM\" ]; then\n\
         echo \"error: the downloaded update files are missing: $FROM\" >&2\n\
         echo \"       Download the update again from the release page.\" >&2\n\
         exit 1\n\
         fi\n\
         if [ ! -d \"$SRC\" ]; then\n\
         echo \"error: the CREBLOX source folder is missing: $SRC\" >&2\n\
         echo \"       Install CREBLOX from the full archive once, then update again.\" >&2\n\
         exit 1\n\
         fi\n\
         if [ ! -f \"$SRC/Cargo.toml\" ] || [ ! -f \"$SRC/scripts/update-macos.sh\" ]; then\n\
         echo \"error: $SRC is missing Cargo.toml or scripts/update-macos.sh,\" >&2\n\
         echo \"       so it is not a CREBLOX source folder.\" >&2\n\
         exit 1\n\
         fi\n\
         if [ ! -w \"$SRC\" ] || [ ! -w \"$SRC/scripts\" ] || [ ! -w \"$SRC/Cargo.toml\" ]; then\n\
         echo \"error: the CREBLOX source folder is not writable: $SRC\" >&2\n\
         echo \"       The update replaces src/, scripts/ and Cargo.toml there.\" >&2\n\
         echo \"       Move the folder into your home folder, or fix its permissions.\" >&2\n\
         exit 1\n\
         fi\n\
         cp -R \"$FROM\"/. \"$SRC\"/\n\
         cd \"$SRC\"\n\
         bash scripts/update-macos.sh\n\
         }} 2>&1 | tee -a \"$LOG\" || status=$?\n\
         if [ \"${{status:-0}}\" -ne 0 ]; then\n\
         echo\n\
         echo \"Update failed (exit $status). Full log: $LOG\"\n\
         read -n 1 -s -r -p \"Press any key to close this window...\" || true\n\
         fi\n",
        log = shell_quote(&log_path),
        root = shell_quote(&root),
        source = shell_quote(&source),
        version = version,
    );
    fs::write(&script, body).map_err(|e| format!("Could not write the update installer: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&script, fs::Permissions::from_mode(0o755));
    }
    launch_installer(&script)
}

fn companion_path(exe: &Path, suffix: &str) -> Result<PathBuf, String> {
    let dir = exe
        .parent()
        .ok_or_else(|| "Could not determine application folder.".to_string())?;
    let fname = exe
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Could not determine executable name.".to_string())?;
    Ok(dir.join(format!("{fname}{suffix}")))
}

fn platform_missing_binary_msg() -> String {
    if cfg!(target_os = "macos") {
        "This release has no macOS download attached. Use Open Release to update.".to_string()
    } else {
        "This release has no Windows .exe download attached.".to_string()
    }
}

/// Relaunch the freshly installed build. On macOS, reopen the enclosing .app
/// bundle so the Dock icon and menu bar keep working.
fn spawn_relaunch(target: &Path) -> std::io::Result<std::process::Child> {
    #[cfg(target_os = "macos")]
    {
        if let Some(bundle) = target
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
        {
            return Command::new("/usr/bin/open").arg("-a").arg(bundle).spawn();
        }
    }
    Command::new(target).spawn()
}

fn temp_updater_path() -> PathBuf {
    let name = if cfg!(windows) {
        "creblox_updater.exe"
    } else {
        "creblox_updater"
    };
    std::env::temp_dir().join(name)
}

pub fn cleanup_old_binary() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    for suffix in [".old", ".new"] {
        if let Ok(path) = companion_path(&exe, suffix) {
            if path.exists() && fs::remove_file(&path).is_err() {
                let path_bg = path.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(550));
                    let _ = fs::remove_file(path_bg);
                });
            }
        }
    }
    let tmp_updater = temp_updater_path();
    if exe != tmp_updater && tmp_updater.exists() && fs::remove_file(&tmp_updater).is_err() {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(700));
            let _ = fs::remove_file(tmp_updater);
        });
    }
}

fn stream_download_exe(
    exe_url: &str,
    dest_path: &Path,
    expected_size: u64,
    downloaded: &AtomicU64,
    total: &AtomicU64,
    cancelled: Option<&AtomicBool>,
    ctx: &egui::Context,
) -> Result<(), String> {
    downloaded.store(0, Ordering::Relaxed);
    total.store(expected_size, Ordering::Relaxed);
    ctx.request_repaint();

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent
        .get(exe_url)
        .set("User-Agent", concat!("creblox/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;

    if let Some(len) = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok())
    {
        if len > 0 {
            total.store(len, Ordering::Relaxed);
        }
    }

    let mut file = fs::File::create(dest_path).map_err(|e| {
        format!(
            "Cannot write to {} ({e}). Move the app to a writable folder or update manually.",
            dest_path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "app folder".to_string())
        )
    })?;

    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut bytes_written: u64 = 0;
    let mut header = [0u8; 2];
    let mut header_len = 0usize;

    loop {
        if cancelled.is_some_and(|c| c.load(Ordering::Relaxed)) {
            drop(file);
            let _ = fs::remove_file(dest_path);
            return Err("Update cancelled.".to_string());
        }
        let n = reader.read(&mut buf).map_err(|e| {
            let _ = fs::remove_file(dest_path);
            format!("Download interrupted: {e}")
        })?;
        if n == 0 {
            break;
        }
        if header_len < 2 {
            let take = (2 - header_len).min(n);
            header[header_len..header_len + take].copy_from_slice(&buf[..take]);
            header_len += take;
        }
        file.write_all(&buf[..n]).map_err(|e| {
            let _ = fs::remove_file(dest_path);
            format!("Failed writing update file: {e}")
        })?;
        bytes_written += n as u64;
        downloaded.store(bytes_written, Ordering::Relaxed);
        ctx.request_repaint();
    }
    file.flush().map_err(|e| {
        let _ = fs::remove_file(dest_path);
        format!("Failed flushing update file: {e}")
    })?;
    drop(file);

    let looks_like_binary = if cfg!(windows) {
        bytes_written >= 64 * 1024 && header == *b"MZ"
    } else {
        bytes_written >= 64 * 1024
    };
    if !looks_like_binary {
        let _ = fs::remove_file(dest_path);
        return Err("Downloaded file is not a valid executable.".to_string());
    }
    Ok(())
}

fn swap_binary_with_retries(target_exe: &Path, staged_exe: &Path) -> Result<(), String> {
    let backup_path = companion_path(target_exe, ".old")?;
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }

    let mut last_err = None;
    let mut moved_old = false;
    for _ in 0..45 {
        if !target_exe.exists() {
            moved_old = false;
            last_err = None;
            break;
        }
        match fs::rename(target_exe, &backup_path) {
            Ok(()) => {
                moved_old = true;
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(140));
            }
        }
    }
    if let Some(e) = last_err {
        return Err(format!("Could not prepare executable swap: {e}"));
    }

    if let Err(e) = fs::rename(staged_exe, target_exe) {
        if moved_old {
            let _ = fs::rename(&backup_path, target_exe);
        }
        return Err(format!("Could not install updated executable: {e}"));
    }
    Ok(())
}

pub fn apply_small_patch_without_restart(
    release: &CrebloxRelease,
    downloaded: &AtomicU64,
    total: &AtomicU64,
    ctx: &egui::Context,
) -> Result<(Option<LiveHotfix>, Option<PathBuf>), String> {
    let hotfix = match &release.hotfix_url {
        Some(url) => Some(download_hotfix(url)?),
        None => None,
    };
    let staged = if let Some(exe_url) = &release.exe_url {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Could not locate current executable: {e}"))?;
        let staged_path = companion_path(&current_exe, ".staged")?;
        stream_download_exe(
            exe_url,
            &staged_path,
            release.exe_size,
            downloaded,
            total,
            None,
            ctx,
        )?;
        Some(staged_path)
    } else {
        None
    };
    if hotfix.is_none() && staged.is_none() {
        return Err("This patch release has no downloadable patch or hotfix attached.".to_string());
    }
    Ok((hotfix, staged))
}

pub fn launch_updater_process(release: &CrebloxRelease) -> Result<(), String> {
    let exe_url = release
        .exe_url
        .as_deref()
        .ok_or_else(|| platform_missing_binary_msg())?;
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Could not locate current executable: {e}"))?;
    let helper_exe = temp_updater_path();
    fs::copy(&current_exe, &helper_exe)
        .map_err(|e| format!("Could not start updater helper: {e}"))?;

    Command::new(&helper_exe)
        .arg("--updater")
        .arg(&current_exe)
        .arg(env!("CARGO_PKG_VERSION"))
        .arg(&release.version)
        .arg(exe_url)
        .arg(release.exe_size.to_string())
        .spawn()
        .map_err(|e| format!("Could not launch updater window: {e}"))?;
    Ok(())
}

pub fn launch_quiet_apply_on_exit(staged_exe: &Path) {
    if !staged_exe.exists() {
        return;
    }
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    let helper_exe = temp_updater_path();
    if fs::copy(&current_exe, &helper_exe).is_err() {
        return;
    }
    let mut cmd = Command::new(&helper_exe);
    cmd.arg("--quiet-apply").arg(&current_exe).arg(staged_exe);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let _ = cmd.spawn();
}

pub fn try_parse_updater_cli(args: &[String]) -> Option<UpdaterArgs> {
    if args.get(1).map(|s| s.as_str()) != Some("--updater") || args.len() < 7 {
        return None;
    }
    Some(UpdaterArgs {
        target_exe: PathBuf::from(&args[2]),
        from_ver: args[3].clone(),
        to_ver: args[4].clone(),
        exe_url: args[5].clone(),
        exe_size: args[6].parse::<u64>().unwrap_or(0),
    })
}

pub fn try_run_quiet_apply_cli(args: &[String]) -> bool {
    if args.get(1).map(|s| s.as_str()) != Some("--quiet-apply") || args.len() < 4 {
        return false;
    }
    let target_exe = PathBuf::from(&args[2]);
    let staged_exe = PathBuf::from(&args[3]);
    if staged_exe.exists() {
        if swap_binary_with_retries(&target_exe, &staged_exe).is_err() {
            let _ = fs::remove_file(&staged_exe);
        }
    }
    true
}

#[derive(Clone, Debug)]
enum UpdaterPhase {
    Downloading,
    Installing,
    Launching,
    Failed(String),
}

struct UpdaterShared {
    phase: UpdaterPhase,
}

struct CREBLOXUpdaterApp {
    args: UpdaterArgs,
    downloaded: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    shared: Arc<Mutex<UpdaterShared>>,
    smooth_progress: f32,
    started_at: Instant,
    worker_started: bool,
}

impl CREBLOXUpdaterApp {
    fn new(args: UpdaterArgs) -> Self {
        let total = args.exe_size;
        Self {
            args,
            downloaded: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(total)),
            cancelled: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(UpdaterShared {
                phase: UpdaterPhase::Downloading,
            })),
            smooth_progress: 0.0,
            started_at: Instant::now(),
            worker_started: false,
        }
    }

    fn start_worker(&mut self, ctx: &egui::Context) {
        self.worker_started = true;
        self.cancelled.store(false, Ordering::Relaxed);
        self.downloaded.store(0, Ordering::Relaxed);
        self.total.store(self.args.exe_size, Ordering::Relaxed);
        if let Ok(mut guard) = self.shared.lock() {
            guard.phase = UpdaterPhase::Downloading;
        }

        let args = self.args.clone();
        let downloaded = Arc::clone(&self.downloaded);
        let total = Arc::clone(&self.total);
        let cancelled = Arc::clone(&self.cancelled);
        let shared = Arc::clone(&self.shared);
        let ctx_bg = ctx.clone();

        thread::spawn(move || {
            let staging = match companion_path(&args.target_exe, ".new") {
                Ok(p) => p,
                Err(e) => {
                    if let Ok(mut g) = shared.lock() {
                        g.phase = UpdaterPhase::Failed(e);
                    }
                    ctx_bg.request_repaint();
                    return;
                }
            };

            if let Err(e) = stream_download_exe(
                &args.exe_url,
                &staging,
                args.exe_size,
                &downloaded,
                &total,
                Some(&cancelled),
                &ctx_bg,
            ) {
                if let Ok(mut g) = shared.lock() {
                    g.phase = UpdaterPhase::Failed(e);
                }
                ctx_bg.request_repaint();
                return;
            }

            if let Ok(mut g) = shared.lock() {
                g.phase = UpdaterPhase::Installing;
            }
            ctx_bg.request_repaint();

            if let Err(e) = swap_binary_with_retries(&args.target_exe, &staging) {
                let _ = fs::remove_file(&staging);
                if let Ok(mut g) = shared.lock() {
                    g.phase = UpdaterPhase::Failed(e);
                }
                ctx_bg.request_repaint();
                return;
            }

            if let Ok(mut g) = shared.lock() {
                g.phase = UpdaterPhase::Launching;
            }
            ctx_bg.request_repaint();
            thread::sleep(Duration::from_millis(420));

            if let Err(e) = spawn_relaunch(&args.target_exe) {
                if let Ok(mut g) = shared.lock() {
                    g.phase =
                        UpdaterPhase::Failed(format!("Update installed, but relaunch failed: {e}"));
                }
                ctx_bg.request_repaint();
                return;
            }

            ctx_bg.send_viewport_cmd(egui::ViewportCommand::Close);
        });
    }
}

const UPDATER_BG_BLACK_LIGHT: Color32 = Color32::from_rgb(27, 30, 36);
const UPDATER_BORDER_COLOR: Color32 = Color32::from_rgb(52, 57, 68);
const UPDATER_TRACK_COLOR: Color32 = Color32::from_rgb(44, 49, 59);
const UPDATER_TEXT_COLOR: Color32 = Color32::from_rgb(242, 244, 248);
const UPDATER_MUTED_COLOR: Color32 = Color32::from_rgb(164, 171, 184);

impl eframe::App for CREBLOXUpdaterApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [
            UPDATER_BG_BLACK_LIGHT.r() as f32 / 255.0,
            UPDATER_BG_BLACK_LIGHT.g() as f32 / 255.0,
            UPDATER_BG_BLACK_LIGHT.b() as f32 / 255.0,
            1.0,
        ]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.worker_started {
            self.start_worker(ui.ctx());
        }

        let settings = load_settings();
        let accent = theme_accent(&settings);
        let phase = self
            .shared
            .lock()
            .map(|g| g.phase.clone())
            .unwrap_or(UpdaterPhase::Downloading);

        let screen = ui.ctx().input(|i| i.viewport_rect());
        let dt = ui.ctx().input(|i| i.unstable_dt).min(0.05);
        let t = self.started_at.elapsed().as_secs_f32();

        let target_frac = match &phase {
            UpdaterPhase::Downloading => {
                let done = self.downloaded.load(Ordering::Relaxed);
                let total = self.total.load(Ordering::Relaxed);
                if total > 0 {
                    (done as f32 / total as f32).clamp(0.03, 0.90)
                } else {
                    0.22
                }
            }
            UpdaterPhase::Installing => 0.95,
            UpdaterPhase::Launching => 1.0,
            UpdaterPhase::Failed(_) => self.smooth_progress,
        };
        self.smooth_progress += (target_frac - self.smooth_progress) * (dt * 9.0).clamp(0.0, 1.0);

        let painter = ui.painter();
        painter.rect_filled(screen, CornerRadius::ZERO, UPDATER_BG_BLACK_LIGHT);
        painter.rect_stroke(
            screen.shrink(0.5),
            CornerRadius::ZERO,
            egui::Stroke::new(1.0, UPDATER_BORDER_COLOR),
            egui::StrokeKind::Inside,
        );

        let drag_resp = ui.interact(screen, ui.scope_id().with("updater_drag"), Sense::drag());
        if drag_resp.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(22, 16)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close_btn = ui.add(
                            egui::Button::new(
                                RichText::new("\u{00D7}")
                                    .size(15.0)
                                    .color(UPDATER_MUTED_COLOR),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        );
                        if close_btn
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.cancelled.store(true, Ordering::Relaxed);
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });

                ui.vertical_centered(|ui| {
                    let (logo_wrap, _) =
                        ui.allocate_exact_size(egui::vec2(58.0, 58.0), Sense::hover());
                    let pulse = 0.5 + 0.5 * (t * 2.4).sin();
                    let halo_alpha = (18.0 + pulse * 18.0) as u8;
                    ui.painter().circle_filled(
                        logo_wrap.center(),
                        28.0,
                        Color32::from_rgba_unmultiplied(
                            accent.r(),
                            accent.g(),
                            accent.b(),
                            halo_alpha,
                        ),
                    );
                    let logo_rect =
                        egui::Rect::from_center_size(logo_wrap.center(), egui::vec2(44.0, 44.0));
                    egui::Image::new(egui::include_image!("../assets/logo.png"))
                        .corner_radius(10)
                        .paint_at(ui, logo_rect);

                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Updating launcher")
                            .size(18.0)
                            .strong()
                            .color(UPDATER_TEXT_COLOR),
                    );
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(format!(
                            "v{}  ->  v{}",
                            self.args.from_ver, self.args.to_ver
                        ))
                        .size(12.5)
                        .color(UPDATER_MUTED_COLOR),
                    );

                    ui.add_space(18.0);

                    match &phase {
                        UpdaterPhase::Failed(err) => {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(err)
                                        .size(11.5)
                                        .color(Color32::from_rgb(248, 113, 113)),
                                )
                                .wrap(),
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if ui.button("Retry").clicked() {
                                    self.start_worker(ui.ctx());
                                }
                                if ui.button("Close").clicked() {
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        }
                        _ => {
                            let (bar_rect, _) =
                                ui.allocate_exact_size(egui::vec2(296.0, 7.0), Sense::hover());
                            ui.painter().rect_filled(
                                bar_rect,
                                CornerRadius::same(4),
                                UPDATER_TRACK_COLOR,
                            );
                            let fill_w = (bar_rect.width() * self.smooth_progress).max(8.0);
                            let fill_rect = egui::Rect::from_min_size(
                                bar_rect.min,
                                egui::vec2(fill_w.min(bar_rect.width()), bar_rect.height()),
                            );
                            ui.painter()
                                .rect_filled(fill_rect, CornerRadius::same(4), accent);

                            ui.add_space(10.0);
                            let status_txt = match phase {
                                UpdaterPhase::Downloading => {
                                    let done = self.downloaded.load(Ordering::Relaxed);
                                    let total = self.total.load(Ordering::Relaxed);
                                    if total > 0 {
                                        let pct = ((done as f64 / total as f64) * 100.0).min(100.0);
                                        format!(
                                            "Downloading update · {:.1} / {:.1} MB ({:.0}%)",
                                            done as f64 / (1024.0 * 1024.0),
                                            total as f64 / (1024.0 * 1024.0),
                                            pct
                                        )
                                    } else {
                                        "Downloading update...".to_string()
                                    }
                                }
                                UpdaterPhase::Installing => "Installing update...".to_string(),
                                UpdaterPhase::Launching => "Starting launcher...".to_string(),
                                UpdaterPhase::Failed(_) => String::new(),
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(status_txt)
                                        .size(12.0)
                                        .color(UPDATER_MUTED_COLOR),
                                )
                                .wrap(),
                            );
                        }
                    }
                });
            });

        if !matches!(phase, UpdaterPhase::Failed(_)) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

pub fn run_updater_window(args: UpdaterArgs) -> eframe::Result<()> {
    let settings = load_settings();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([404.0, 290.0])
        .with_decorations(false)
        .with_transparent(false)
        .with_resizable(false);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/logo.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Updating CREBLOX",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            install_fonts(&cc.egui_ctx);
            apply_theme(&cc.egui_ctx, &settings);
            Ok(Box::new(CREBLOXUpdaterApp::new(args)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parsing_and_comparison_work() {
        assert_eq!(parse_semver("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("0.2.5-beta.1"), Some((0, 2, 5)));
        assert_eq!(parse_semver("1.4"), Some((1, 4, 0)));
        assert_eq!(parse_semver(""), None);

        assert!(is_newer_version("v0.1.1", "0.1.0"));
        assert!(is_newer_version("v0.2.0", "0.1.9"));
        assert!(is_newer_version("1.0.0", "0.99.99"));
        assert!(!is_newer_version("v0.1.0", "0.1.0"));
        assert!(!is_newer_version("v0.0.9", "0.1.0"));

        assert_eq!(
            classify_update_kind("v0.1.1", "0.1.0", "Bug fixes", "Fixed crash"),
            UpdateKind::Patch
        );
        assert_eq!(
            classify_update_kind("v0.2.0", "0.1.0", "New UI", "Redesigned cards"),
            UpdateKind::Full
        );
        assert_eq!(
            classify_update_kind("v0.1.1", "0.1.0", "Update [full]", "Major change"),
            UpdateKind::Full
        );
    }

    #[test]
    fn parses_github_release_and_picks_exe_and_hotfix() {
        let json = serde_json::json!({
            "tag_name": "v0.1.2",
            "name": "Small Bugfix Patch",
            "body": "- Fixed friend sync\n- Live FastFlag hotfix",
            "draft": false,
            "html_url": "https://github.com/gothssmy-design/creblox/releases/tag/v0.1.2",
            "assets": [
                {
                    "name": "hotfix.json",
                    "size": 512,
                    "browser_download_url": "https://example.com/hotfix.json"
                },
                {
                    "name": "creblox.exe",
                    "size": 8500000,
                    "browser_download_url": "https://example.com/creblox.exe"
                }
            ]
        });
        let rel = parse_github_release_update(&json, "0.1.0").expect("should detect newer release");
        assert_eq!(rel.version, "0.1.2");
        assert_eq!(rel.kind, UpdateKind::Patch);
        assert_eq!(rel.exe_name.as_deref(), Some("creblox.exe"));
        assert_eq!(
            rel.exe_url.as_deref(),
            Some("https://example.com/creblox.exe")
        );
        assert_eq!(
            rel.hotfix_url.as_deref(),
            Some("https://example.com/hotfix.json")
        );
        assert_eq!(rel.exe_size, 8_500_000);
        assert!(parse_github_release_update(&json, "0.1.2").is_none());
    }

    #[test]
    fn reads_zip_and_sha_from_release_body() {
        let json = serde_json::json!({
            "tag_name": "v0.4.14-update",
            "name": "CREBLOX 0.4.14 Update",
            "body": "Update zip:\nhttps://files.catbox.moe/abc123.zip\n\nSHA-256: `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`",
            "draft": false,
            "assets": []
        });
        let rel = parse_github_release_update(&json, "0.4.13").expect("newer release");
        assert_eq!(
            rel.zip_url.as_deref(),
            Some("https://files.catbox.moe/abc123.zip")
        );
        assert_eq!(rel.zip_name.as_deref(), Some("abc123.zip"));
        assert_eq!(
            rel.zip_sha256.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn parses_live_hotfix_payload() {
        let raw = r#"{
            "fastflags": {"DFIntTaskSchedulerTargetFps": 240, "FFlagDebugDisableTelemetry": true},
            "auto_close_roblox": true,
            "note": "Applied FPS & telemetry patch live."
        }"#;
        let hf = parse_live_hotfix_str(raw).expect("hotfix json should parse");
        assert_eq!(hf.fastflag_overrides.len(), 2);
        assert_eq!(hf.auto_close_roblox, Some(true));
        assert_eq!(
            hf.status_note.as_deref(),
            Some("Applied FPS & telemetry patch live.")
        );
    }

    #[test]
    fn streams_download_validates_pe_and_swaps_target_exe() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().unwrap();

        let mut fake_pe = vec![0x90u8; 80 * 1024];
        fake_pe[0] = b'M';
        fake_pe[1] = b'Z';
        fake_pe[2..6].copy_from_slice(b"V011");

        let server_payload = fake_pe.clone();
        let server = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req = [0u8; 1024];
                let _ = stream.read(&mut req);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    server_payload.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&server_payload);
            }
        });

        let dir = std::env::temp_dir().join(format!("creblox_updater_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let target_exe = dir.join("creblox.exe");
        let staged_exe = dir.join("creblox.exe.new");
        fs::write(&target_exe, b"OLD_VERSION_BYTES").unwrap();

        let dl = AtomicU64::new(0);
        let total = AtomicU64::new(0);
        let ctx = egui::Context::default();
        let url = format!("http://{addr}/creblox.exe");
        stream_download_exe(
            &url,
            &staged_exe,
            fake_pe.len() as u64,
            &dl,
            &total,
            None,
            &ctx,
        )
        .expect("stream download should succeed");

        assert_eq!(dl.load(Ordering::Relaxed), fake_pe.len() as u64);
        assert_eq!(total.load(Ordering::Relaxed), fake_pe.len() as u64);

        swap_binary_with_retries(&target_exe, &staged_exe).expect("binary swap should succeed");
        let swapped = fs::read(&target_exe).unwrap();
        assert_eq!(swapped.len(), fake_pe.len());
        assert_eq!(&swapped[0..6], b"MZV011");
        let backup = fs::read(dir.join("creblox.exe.old")).unwrap();
        assert_eq!(backup, b"OLD_VERSION_BYTES");

        let _ = server.join();
        let _ = fs::remove_dir_all(&dir);
    }
}
