Do not read the same file more than once per task unless it has been modified. If a file is too large or the fix is unclear after one read, stop and ask me which function to look at instead of re-reading

# Every fix ships: a change is not done until it is released

Every fix or update you implement must be **fully released** before you finish your turn.
Never leave fixes sitting only in local commits: bump the version (`Cargo.toml`, `README.md`,
`Cargo.lock`), package the release archives, push `main`, publish the GitHub Release with the
macOS archives linked and their byte size + SHA-256 in the notes, and verify every release
check - see the macOS-only release gate below - before calling the work finished. Committed but
unreleased work is unfinished work, and it is never the end of a turn.

# Automated VM verification (zero release overhead)

Before finishing any change, run `scripts/verify.sh` and fix what it reports:

- `cargo test --locked --manifest-path vm-tests/Cargo.toml` — the real `src/party.rs` and
  `src/server_hop.rs`, compiled for this host against the stand-ins in `vm-tests/`, plus the
  loopback end-to-end tests in `vm-tests/tests/` and the UI screenshot/layout suite.
- `cargo check --locked --target x86_64-apple-darwin` — the full GUI crate for the release
  target, including all `#[cfg(test)]` and `#[cfg(macos)]` code.

When a change touches logic that can run without a window (party, server hop, log parsing,
update release parsing, ranking, codecs), add or update a test in `vm-tests/tests/` or its
`#[cfg(test)]` block in the same change. `cargo test --locked` on the GUI crate only runs on
macOS; in this Linux VM it cannot compile, so do not treat a host `cargo test` failure there
as a regression in the app.

Release builds stay clean by construction: `vm-tests/` is a separate package with its own
`[workspace]` that `creblox` does not depend on, all test code sits in `#[cfg(test)]` or
`vm-tests/tests/`, and `scripts/package-release.sh` copies an explicit file list that excludes
it. Never add runtime test flags, background test threads or debug logging to shipping code to
make a check pass.

# UI changes: render the pages and look at them

A change to `src/ui/*`, `src/app.rs`, `src/theme.rs`, `src/game_overlay.rs` or the overlay HUD
(`src/overlay_window.rs`) is not finished when the tests pass. It is finished when the rendered
pages have been looked at.

```bash
cargo test --manifest-path vm-tests/Cargo.toml --test shots
# writes vm-tests/artifacts/<page>-<1280x800|1440x900>.png
```

1. Run it. It fails on layout problems: a widget clipped by the window, collapsed to zero size,
   overlapping another one, text wider than its box, a hit target under 24x24, or contrast below
   4.5:1. Fix those first.
2. Look at the PNGs — Home, Charts, Game details, Party, Settings, Updater, overlay HUD. Check
   spacing, alignment, icons, contrast and anything cut off or cramped. Fix what you see.
3. If a page's structure changed, mirror it in `vm-tests/src/screens.rs`. The GUI crate cannot be
   linked on Linux (`src/ui/*` needs `crate::app::Creblox`, eframe and the OS APIs), so those
   fixtures are what the screenshots show; stale fixtures hide real regressions.
4. Say what you looked at and what you did not. Passing geometry checks are not an eyeball check.

`vm-tests/artifacts/` is gitignored, and none of this is part of a release build.

# Releases: macOS only until Windows is turned back on

**Windows is paused on purpose.** Until the user explicitly says to bring it back, a release is
the macOS archives and nothing else:

- Do not dispatch `.github/workflows/build-windows.yml`, wait on it, or gate a release on it.
- Do not attach or require `CREBLOX-<ver>-windows-x64.zip`, and do not print a size or SHA-256
  for it in the notes. A release without it is complete, not broken.
- Do not describe a release as blocked, incomplete, or in progress because the Windows job has
  not run. Those sentences must disappear from the workflow, not just from your head.

The Windows code, its tests, and the workflow itself stay in the tree and keep working: turning
Windows back on is a dispatch, not a port. `scripts/verify.sh` still type-checks the Windows
target, because that catches `#[cfg(windows)]` breakage on any developer machine; it is a compile
check, not a release gate.

Every fix above still ends in a release: bump, package, push, publish, verify - in the same turn,
not "later". `memory.md` is the step-by-step runbook; this is the gate that decides whether a
release exists.

A release is complete only when all of these are true:

1. `Cargo.toml` and `README.md` carry the new version, and `Cargo.lock` is committed with it.
2. `release/CREBLOX-<ver>-update.zip` and `release/CREBLOX-<ver>-full.zip` are committed,
   pushed, and their byte size + SHA-256 are printed in the notes.
3. No `build-windows` run was required, and no Windows asset is attached or mentioned.
4. The release is published (not a draft) and is the repository's latest.

While Windows is paused, Windows users get no update: the Windows updater treats a release with
no Windows zip as "no update" (`release_links::is_windows_zip_for`), never as an invitation to
install the macOS archive. That is the accepted behaviour, so do not add a macOS fallback for it.

## Turning Windows back on (the contract, kept for that day)

When the user asks for Windows releases again, restore these as release requirements:

- The `build-windows` workflow finished **green** on that tag and attached
  `CREBLOX-<ver>-windows-x64.zip` to the release, with its size and SHA-256 in the notes.
- The release's `assets[]` lists `CREBLOX-<ver>-update.zip` first and
  `CREBLOX-<ver>-windows-x64.zip` second, both with a non-zero size.
- Never call a release done while the Windows job is running or its zip is missing. Re-run the
  job (`workflow_dispatch`, input `release_tag: v<ver>`) or fix the build.

The workflow must keep compiling `creblox.exe` with the GUI subsystem
(`#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]` in
`src/main.rs`, checked as PE subsystem 2 in the job), running `cargo test` on the Windows
runner, and packaging exactly `CREBLOX-<ver>-windows-x64.zip`.

Only a dispatched run attaches an asset (`workflow_dispatch`, input `release_tag: v<ver>`). A
tag push builds and tests but never uploads: two attaching runs would race each other's upload
and notes edit, and the notes would end up quoting a hash for bytes that a later upload had
already replaced. After any release, the hash printed for the Windows line must equal the
attached asset's `digest` — check it, do not assume the run and the notes agree.

# Windows self-update: the rules the tests enforce

- The release parser takes the asset named exactly `CREBLOX-<version>-windows-x64.zip`
  (`release_links::is_windows_zip_for`); a zip for another version, product or architecture is
  ignored, never installed.
- The archive's SHA-256 is verified with `src/sha256.rs` **before** anything is written: no
  external tool, no PowerShell, no script. A release asset without a published hash is refused.
- Extraction is `zip`-crate-only into `%LOCALAPPDATA%\creblox\updates\`; only `creblox.exe` is
  taken out and its PE header must be x64 + GUI.
- The running exe is renamed to `creblox.exe.old` with retries, the new one is renamed into
  place (copying when the target is on another volume), the process reopens it with
  `CREATE_NO_WINDOW`, and `cleanup_old_binary()` deletes the leftovers on the next start.
