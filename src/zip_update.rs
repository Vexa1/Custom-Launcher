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

/// How much of the file is read to reach the PE headers. The DOS stub points at the PE header,
/// and every real launcher keeps it well inside this window.
const HEADER_PROBE: usize = 64 * 1024;
const PE32_PLUS_MAGIC: u16 = 0x20b;
const X64_MACHINE: u16 = 0x8664;
const GUI_SUBSYSTEM: u16 = 2;

/// Checks that `path` could be this launcher before it replaces a working install, and returns
/// its size.
///
/// Beyond the size sanity check, the PE header has to say x64 GUI: the release only ships an
/// x64 build, and a console-subsystem image would pop a terminal window open on every start.
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
    let mut head = vec![0u8; (size as usize).min(HEADER_PROBE)];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .map_err(|e| format!("Could not read the new executable: {e}"))?;
    check_pe_header(&head)?;
    Ok(size)
}

/// Rejects anything that is not a 64-bit GUI image: DOS magic, `PE\0\0`, machine, optional
/// header magic and subsystem, in that order, each with the reason a user can act on.
fn check_pe_header(head: &[u8]) -> Result<(), String> {
    let not_exe = "The update zip does not contain a Windows executable.";
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return Err(not_exe.to_string());
    }
    let pe = u32::from_le_bytes(head[0x3c..0x40].try_into().unwrap()) as usize;
    let optional = pe + 24;
    if pe < 0x40 || optional + 0x44 + 2 > head.len() || &head[pe..pe + 4] != b"PE\0\0" {
        return Err(not_exe.to_string());
    }
    let machine = u16::from_le_bytes(head[pe + 4..pe + 6].try_into().unwrap());
    if machine != X64_MACHINE {
        return Err(format!(
            "The new executable is not an x64 Windows build (machine 0x{machine:04x})."
        ));
    }
    let magic = u16::from_le_bytes(head[optional..optional + 2].try_into().unwrap());
    if magic != PE32_PLUS_MAGIC {
        return Err("The new executable is not a 64-bit Windows build.".to_string());
    }
    let subsystem = u16::from_le_bytes(head[optional + 0x44..optional + 0x46].try_into().unwrap());
    if subsystem != GUI_SUBSYSTEM {
        return Err(format!(
            "The new executable is not a Windows GUI build (subsystem {subsystem}), so it would open a console window."
        ));
    }
    Ok(())
}
