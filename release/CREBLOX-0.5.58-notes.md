## Quick update (existing installs)

- **CREBLOX-0.5.58-update.zip** — 563110 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.58-update.zip — SHA-256 `9d23d3793c4bd5ca260e5863b9e8ed81b6107423f8f5c24735414db360c04aa1`

## Full build (fresh installs)

- **CREBLOX-0.5.58-full.zip** — 903143 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.58-full.zip — SHA-256 `8dfc1202d98e7a93de29b086be02cfc7c2dbaf610d4a9cb0078092a760fb338b`

## What changed

### A settings file from a newer build is read-only

0.5.57 added `"schema"` and, for a file from a newer build, read what it understood and rewrote it as version 2. That is destructive in the one direction that matters: the newer build opened next would find every setting it added gone, silently reset to defaults, with the older build's stale values in their place.

A file whose schema is newer than this build's is now **read-only**:

- **Load** - everything this build understands is read as before, so the window, the overlay and the picture behave as the file says.
- **Save** - `settings_json::save_settings_to` refuses to write when the existing file's version is higher than `SETTINGS_VERSION`, and returns `false` instead. `auth::save_settings` logs `settings.json was written by a NEWER CREBLOX; leaving it untouched` rather than rewriting it.
- **Nothing else changes** - this build's own files, version 1 files and unreadable files all still save exactly as before.

The trade is deliberate and one-way: an older build cannot persist changes while a newer file is present, but it can no longer delete the newer build's settings. Downgrade, open, change something, and the file your newer build wrote is still byte-for-byte what it was.

### The tests

Two tests updated or added in `vm-tests/tests/settings_file.rs`, 376 in total:

- **A newer schema is never rewritten** - a `schema: 9` file with an unknown key is loaded (dark theme and box `412, 236, 320 x 128`), then saved; the save returns `false`, the file is byte-for-byte identical, `schema` is still `9` and the unknown key is still there.
- **This build's own files still save** - a current-version file writes, carries `schema: 2`, and writes again on a second save.

macOS and Windows (warnings denied) target checks clean; no new warnings.

