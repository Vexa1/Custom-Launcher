use crate::types::{Settings, User};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
#[cfg(windows)]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(windows)]
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

/// Folder name this app used before the CREBLOX rename. Only the migration
/// below reads it; nothing else may depend on it.
#[cfg(target_os = "macos")]
const LEGACY_APP_FOLDER: &str = "Aurora";

/// The pre-0.4 login item was registered under the old bundle identity, so it
/// would point at an app that no longer exists. Start at login is re-enabled
/// from Settings after the rename. Once per process only: `data_dir()` is called
/// for every log line, so this must not touch the disk each time.
#[cfg(target_os = "macos")]
fn drop_legacy_launch_agent() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let legacy = PathBuf::from(home)
                .join("Library")
                .join("LaunchAgents")
                .join("com.aurora.launcher.plist");
            if legacy.exists() {
                let _ = fs::remove_file(&legacy);
            }
        }
    });
}

/// Per-user application data directory.
///
/// macOS: `~/Library/Application Support/CREBLOX`
/// Windows: `%LOCALAPPDATA%\creblox`
#[cfg(target_os = "macos")]
pub fn data_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let base = PathBuf::from(home)
        .join("Library")
        .join("Application Support");
    let dir = base.join("CREBLOX");
    // One-time migration from the previous app names, newest first, so nobody
    // loses a saved session, accounts, playtime or settings. Renaming the
    // folder is the whole point: every file below assumes this path.
    //
    // The trigger is "no user data here yet", not "folder missing": the build
    // script writes CREBLOX/source-folder before the app ever launches, so the
    // folder can exist while the real settings are still in the old one.
    let already_migrated = dir.join("settings.json").exists() || dir.join("accounts.dat").exists();
    if !already_migrated {
        for legacy_name in [LEGACY_APP_FOLDER, "RustBlox"] {
            let legacy = base.join(legacy_name);
            if !legacy.is_dir() {
                continue;
            }
            // Moving is instant and keeps the 0700 permissions; copying is the
            // fallback when the new folder already exists (see above) or the
            // folders live on different volumes.
            if !dir.exists() && fs::rename(&legacy, &dir).is_ok() {
                break;
            }
            let _ = fs::create_dir_all(&dir);
            if let Ok(entries) = fs::read_dir(&legacy) {
                for entry in entries.flatten() {
                    let from = entry.path();
                    let to = dir.join(entry.file_name());
                    if from.is_file() && !to.exists() {
                        let _ = fs::copy(&from, &to);
                    }
                }
            }
            break;
        }
    }
    drop_legacy_launch_agent();
    let _ = fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    dir
}

#[cfg(not(target_os = "macos"))]
pub fn data_dir() -> PathBuf {
    let base = env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(base).join("creblox");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, without pulling in a date crate.
fn log_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}Z")
}

pub fn log(msg: &str) {
    let path = data_dir().join("log.txt");
    // Keep the file small enough to stay readable on the Logs page: once it grows past
    // ~400 KB only the newest lines are kept.
    if fs::metadata(&path).is_ok_and(|m| m.len() > 400_000) {
        if let Ok(text) = fs::read_to_string(&path) {
            let mut keep: Vec<&str> = text.lines().rev().take(1_000).collect();
            keep.reverse();
            let _ = fs::write(&path, keep.join("\n"));
        }
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {msg}", log_stamp());
    }
}

/// Last `max_lines` lines of a log file, newest last.
pub fn log_text(file: &str, max_lines: usize) -> String {
    let text = fs::read_to_string(data_dir().join(file)).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

pub fn clear_log(file: &str) {
    let _ = fs::write(data_dir().join(file), "");
}

pub fn read_lines(file: &str) -> Vec<String> {
    fs::read_to_string(data_dir().join(file))
        .map(|t| {
            t.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn write_lines(file: &str, lines: &[String]) {
    let _ = fs::write(data_dir().join(file), lines.join("\n"));
}

pub fn read_ids(file: &str) -> Vec<u64> {
    read_lines(file)
        .iter()
        .filter_map(|l| l.parse().ok())
        .collect()
}

pub fn write_ids(file: &str, ids: &[u64]) {
    let lines: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    write_lines(file, &lines);
}

pub fn load_settings() -> Settings {
    crate::settings_json::load_settings_from(&data_dir())
}

pub fn save_settings(s: &Settings) {
    // A settings.json written by a newer build keeps the keys this build does not know: the save
    // merges this build's own keys into it instead of replacing the file.
    match crate::settings_json::save_settings_to(&data_dir(), s) {
        crate::settings_json::SaveOutcome::Written => {}
        crate::settings_json::SaveOutcome::MergedIntoNewer => log(
            "settings.json was written by a newer CREBLOX; saved this build's settings into it \
             and left its own alone",
        ),
        crate::settings_json::SaveOutcome::Failed => log("settings.json could not be written"),
    }
}

/// The schema version on disk when the settings file comes from a **newer build**, if it does.
///
/// Read once per process: the Settings page asks on every frame, and the answer cannot change
/// without the file being replaced under a running app.
pub fn newer_settings_file() -> Option<u64> {
    static VERSION: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *VERSION.get_or_init(|| {
        crate::settings_json::existing_version(&data_dir())
            .filter(|v| *v > crate::settings_json::SETTINGS_VERSION)
    })
}

#[cfg(windows)]
pub fn dpapi_transform(input: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    if input.is_empty() {
        return None;
    }
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(input.len()).ok()?,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &in_blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        } else {
            CryptUnprotectData(
                &in_blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 || out_blob.pbData.is_null() {
            return None;
        }
        let result = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec();
        LocalFree(out_blob.pbData as _);
        Some(result)
    }
}

/// Per-install 256-bit key, created once and stored with owner-only (0600)
/// permissions inside the private application data folder.
#[cfg(target_os = "macos")]
fn mac_key() -> Option<[u8; 32]> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = data_dir().join("session.key");
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Some(key);
        }
    }
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).ok()?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    let mut file = opts.open(&path).ok()?;
    file.write_all(&key).ok()?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Some(key)
}

/// Encrypt/decrypt local session data with AES-256-GCM on macOS (the DPAPI
/// equivalent). Output format: `RBX1` magic, 12-byte nonce, ciphertext+tag.
#[cfg(target_os = "macos")]
pub fn dpapi_transform(input: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    if input.is_empty() {
        return None;
    }
    let key = mac_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    if encrypt {
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).ok()?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), input)
            .ok()?;
        let mut out = Vec::with_capacity(16 + ciphertext.len());
        out.extend_from_slice(b"RBX1");
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Some(out)
    } else {
        if input.len() < 17 || &input[..4] != b"RBX1" {
            return None;
        }
        cipher
            .decrypt(Nonce::from_slice(&input[4..16]), &input[16..])
            .ok()
    }
}

pub fn save_cookie(cookie: &str) -> Result<(), String> {
    let encrypted = dpapi_transform(cookie.as_bytes(), true)
        .ok_or_else(|| "Local session encryption failed".to_string())?;
    fs::write(data_dir().join("session.dat"), encrypted).map_err(|e| e.to_string())
}

pub fn load_cookie() -> Option<String> {
    let encrypted = fs::read(data_dir().join("session.dat")).ok()?;
    let decrypted = dpapi_transform(&encrypted, false)?;
    String::from_utf8(decrypted).ok()
}

pub fn clear_cookie() {
    let path = data_dir().join("session.dat");
    if let Ok(meta) = fs::metadata(&path) {
        let len = usize::try_from(meta.len()).unwrap_or(0).min(1024 * 64);
        if len > 0 {
            let _ = fs::write(&path, vec![0u8; len]);
        }
    }
    let _ = fs::remove_file(path);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAccount {
    pub id: u64,
    pub name: String,
    pub display: String,
    pub headshot: String,
    pub verified: bool,
    pub cookie: String,
    pub last_used_ms: u64,
}

impl StoredAccount {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "display": self.display,
            "headshot": self.headshot,
            "verified": self.verified,
            "cookie": self.cookie,
            "last_used_ms": self.last_used_ms,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        let id = v.get("id")?.as_u64()?;
        let name = v.get("name")?.as_str()?.to_string();
        let cookie = v.get("cookie")?.as_str()?.to_string();
        if id == 0 || cookie.trim().is_empty() {
            return None;
        }
        let display = v
            .get("display")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&name)
            .to_string();
        Some(Self {
            id,
            name,
            display,
            headshot: v
                .get("headshot")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            verified: v.get("verified").and_then(|x| x.as_bool()).unwrap_or(false),
            cookie,
            last_used_ms: v.get("last_used_ms").and_then(|x| x.as_u64()).unwrap_or(0),
        })
    }
}

pub fn load_accounts() -> Vec<StoredAccount> {
    let Ok(encrypted) = fs::read(data_dir().join("accounts.dat")) else {
        return Vec::new();
    };
    let Some(decrypted) = dpapi_transform(&encrypted, false) else {
        return Vec::new();
    };
    let Ok(text) = String::from_utf8(decrypted) else {
        return Vec::new();
    };
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    arr.iter().filter_map(StoredAccount::from_json).collect()
}

pub fn save_accounts(accounts: &[StoredAccount]) -> Result<(), String> {
    let path = data_dir().join("accounts.dat");
    if accounts.is_empty() {
        if let Ok(meta) = fs::metadata(&path) {
            let len = usize::try_from(meta.len()).unwrap_or(0).min(1024 * 64);
            if len > 0 {
                let _ = fs::write(&path, vec![0u8; len]);
            }
        }
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    let arr: Vec<Value> = accounts.iter().map(StoredAccount::to_json).collect();
    let text = serde_json::to_string(&Value::Array(arr)).map_err(|e| e.to_string())?;
    let encrypted = dpapi_transform(text.as_bytes(), true)
        .ok_or_else(|| "Local session encryption failed".to_string())?;
    fs::write(path, encrypted).map_err(|e| e.to_string())
}

pub fn upsert_stored_account(
    accounts: &mut Vec<StoredAccount>,
    user: &User,
    cookie: &str,
    last_used_ms: u64,
) {
    if cookie.trim().is_empty() || user.id == 0 {
        return;
    }
    let display = if user.display.trim().is_empty() {
        user.name.clone()
    } else {
        user.display.clone()
    };
    if let Some(existing) = accounts
        .iter_mut()
        .find(|a| a.id == user.id || a.cookie == cookie)
    {
        existing.id = user.id;
        existing.name = user.name.clone();
        existing.display = display;
        existing.headshot = user.headshot.clone();
        existing.verified = user.verified;
        existing.cookie = cookie.to_string();
        existing.last_used_ms = last_used_ms;
    } else {
        accounts.push(StoredAccount {
            id: user.id,
            name: user.name.clone(),
            display,
            headshot: user.headshot.clone(),
            verified: user.verified,
            cookie: cookie.to_string(),
            last_used_ms,
        });
    }
}

pub fn remove_stored_account(
    accounts: &mut Vec<StoredAccount>,
    user_id: u64,
) -> Option<StoredAccount> {
    let pos = accounts.iter().position(|a| a.id == user_id)?;
    Some(accounts.remove(pos))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeenNotifications {
    pub update_uploads: HashSet<String>,
    pub friend_requests_by_user: HashMap<u64, HashSet<u64>>,
}

impl SeenNotifications {
    pub fn from_json_str(text: &str) -> Self {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return Self::default();
        };
        let update_uploads = v["update_uploads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        let mut friend_requests_by_user: HashMap<u64, HashSet<u64>> = HashMap::new();
        if let Some(obj) = v["friend_requests_by_user"].as_object() {
            for (k, arr) in obj {
                let Ok(owner_id) = k.parse::<u64>() else {
                    continue;
                };
                let ids: HashSet<u64> = arr
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_u64())
                    .filter(|&id| id != 0)
                    .collect();
                if !ids.is_empty() {
                    friend_requests_by_user.insert(owner_id, ids);
                }
            }
        }
        Self {
            update_uploads,
            friend_requests_by_user,
        }
    }

    pub fn to_json_value(&self) -> Value {
        let mut updates: Vec<&str> = self.update_uploads.iter().map(|s| s.as_str()).collect();
        updates.sort_unstable();
        let mut fr_map = serde_json::Map::new();
        for (&owner_id, set) in &self.friend_requests_by_user {
            if set.is_empty() {
                continue;
            }
            let mut ids: Vec<u64> = set.iter().copied().collect();
            ids.sort_unstable();
            fr_map.insert(owner_id.to_string(), json!(ids));
        }
        json!({
            "update_uploads": updates,
            "friend_requests_by_user": Value::Object(fr_map),
        })
    }
}

pub fn load_seen_notifications() -> SeenNotifications {
    let path = data_dir().join("seen_notifications.json");
    let Ok(text) = fs::read_to_string(path) else {
        return SeenNotifications::default();
    };
    SeenNotifications::from_json_str(&text)
}

pub fn save_seen_notifications(seen: &SeenNotifications) {
    if let Ok(text) = serde_json::to_string_pretty(&seen.to_json_value()) {
        let _ = fs::write(data_dir().join("seen_notifications.json"), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_user(id: u64, name: &str) -> User {
        User {
            id,
            name: name.to_string(),
            display: format!("{name} Display"),
            headshot: "https://example.com/a.png".to_string(),
            verified: true,
            ..Default::default()
        }
    }

    #[test]
    fn upsert_and_remove_accounts_behaves_cleanly() {
        let mut vault = Vec::new();
        let u1 = sample_user(101, "alice");
        let u2 = sample_user(202, "bob");

        upsert_stored_account(&mut vault, &u1, "cookie_1", 10);
        upsert_stored_account(&mut vault, &u2, "cookie_2", 20);
        assert_eq!(vault.len(), 2);

        let mut u1_updated = u1.clone();
        u1_updated.name = "alice_new".to_string();
        upsert_stored_account(&mut vault, &u1_updated, "cookie_1_new", 30);
        assert_eq!(vault.len(), 2);
        assert_eq!(vault[0].name, "alice_new");
        assert_eq!(vault[0].cookie, "cookie_1_new");

        let removed = remove_stored_account(&mut vault, 101);
        assert!(removed.is_some());
        assert_eq!(vault.len(), 1);
        assert_eq!(vault[0].id, 202);
    }

    #[test]
    fn seen_notifications_roundtrip_and_dedup() {
        let mut seen = SeenNotifications::default();
        assert!(seen.update_uploads.insert("version-abc123".to_string()));
        assert!(!seen.update_uploads.insert("version-abc123".to_string()));
        let reqs = seen.friend_requests_by_user.entry(42).or_default();
        reqs.insert(1001);
        reqs.insert(1002);

        let serialized = seen.to_json_value().to_string();
        let parsed = SeenNotifications::from_json_str(&serialized);
        assert_eq!(parsed, seen);
        assert!(parsed.update_uploads.contains("version-abc123"));
        assert!(parsed.friend_requests_by_user[&42].contains(&1001));
        assert!(parsed.friend_requests_by_user[&42].contains(&1002));
        assert!(!parsed.friend_requests_by_user[&42].contains(&1003));
    }
}
