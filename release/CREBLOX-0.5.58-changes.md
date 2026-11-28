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
