//! Reading the one `creblox.exe` out of a Windows release zip.
//!
//! Pure std plus the `zip` crate: no PowerShell, no external unzip, no helper process, and no
//! directory traversal, because only an entry's file name is ever used. Kept independent of the
//! GUI so `vm-tests` compiles this exact file and runs it on any host.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The file the updater installs.
pub const LAUNCHER_NAME: &str = "creblox.exe";
/// The largest launcher an update may carry. Anything bigger is not CREBLOX, and must never
/// reach the exe swap.
pub const MAX_LAUNCHER_BYTES: u64 = 200 * 1024 * 1024;
/// Below this a file cannot be the real launcher, so a truncated download can do no harm.
pub const MIN_LAUNCHER_BYTES: u64 = 64 * 1024;

/// Copies `creblox.exe` out of `zip` into `dest` and returns its path.
///
/// The archive may nest the launcher in a folder; only the last path segment is compared, and
/// only the file name is used on disk, so `../../creblox.exe` cannot escape `dest`.
pub fn extract_launcher(zip: &Path, dest: &Path) -> Result<PathBuf, String> {
    let file = fs::File::open(zip).map_err(|e| format!("Could not open the update zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("The update zip could not be read: {e}"))?;

    let mut found: Option<(usize, u64)> = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("The update zip is damaged: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().rsplit(['/', '\\']).next().unwrap_or_default();
        if name.eq_ignore_ascii_case(LAUNCHER_NAME) {
            found = Some((index, entry.size()));
            break;
        }
    }
    let Some((index, size)) = found else {
        return Err(format!("The update zip does not contain {LAUNCHER_NAME}."));
    };
    if size == 0 || size > MAX_LAUNCHER_BYTES {
        return Err(format!(
            "The update zip carries an unusable {LAUNCHER_NAME} ({size} bytes)."
        ));
    }

    fs::create_dir_all(dest)
        .map_err(|e| format!("Could not create the update folder: {e}"))?;
    let target = dest.join(LAUNCHER_NAME);
    let mut entry = archive
        .by_index(index)
        .map_err(|e| format!("The update zip is damaged: {e}"))?;
    let mut out = fs::File::create(&target)
        .map_err(|e| format!("Could not write the new executable: {e}"))?;
    let copied = std::io::copy(&mut entry, &mut out)
        .map_err(|e| format!("Could not extract the new executable: {e}"))?;
    out.flush()
        .map_err(|e| format!("Could not finish the new executable: {e}"))?;
    if copied != size {
        let _ = fs::remove_file(&target);
        return Err(format!(
            "The update zip ended in the middle of {LAUNCHER_NAME}."
        ));
    }
    Ok(target)
}

/// Checks that `path` is plausible before it replaces a working launcher, and returns its size.
pub fn verify_launcher(path: &Path) -> Result<u64, String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("Could not read the new executable: {e}"))?
        .len();
    if size < MIN_LAUNCHER_BYTES {
        return Err(format!(
            "The new executable is too small to be CREBLOX ({size} bytes)."
        ));
    }
    if size > MAX_LAUNCHER_BYTES {
        return Err(format!(
            "The new executable is implausibly large ({size} bytes)."
        ));
    }
    let mut magic = [0u8; 2];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| format!("Could not read the new executable: {e}"))?;
    if &magic != b"MZ" {
        return Err("The update zip does not contain a Windows executable.".to_string());
    }
    Ok(size)
}
