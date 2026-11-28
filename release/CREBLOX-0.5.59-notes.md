## Quick update (existing installs)

- **CREBLOX-0.5.59-update.zip** — 564056 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.59-update.zip — SHA-256 `fababa5450f4c62bb7cf7ed159afa77f581d2f727727f6b59f82f098932ef933`

## Full build (fresh installs)

- **CREBLOX-0.5.59-full.zip** — 904089 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.59-full.zip — SHA-256 `6c826f85fa604f2df9ca1e4c34d5bb6611f5d729891a0be53d740b1da194f4ee`

## What changed

### A newer settings file is merged into, not refused

0.5.58 made a newer build's `settings.json` read-only: correct for the newer file, but it meant downgrading, changing something and finding the change gone - with only a log line to explain it.

Saves now **merge** instead. When the file on disk has a newer schema, this build's own keys are written into it and everything else is kept:

- **The file keeps its schema** - a `schema: 9` file stays version 9, so the newer build still recognises its own layout.
- **Unknown keys survive** - `some_future_key` and every other setting this build has never heard of is still there, byte-for-byte.
- **This build's keys update normally** - the picture's box, the HUD panels, the theme and the rest are written as usual, so changing something in the older build does take effect.
- **`SaveOutcome`** - `save_settings_to` now reports `Written`, `MergedIntoNewer` or `Failed`, and `auth::save_settings` logs which happened.

### The Settings page says so

The state is no longer only in `log.txt`: when a newer file is present, the Settings page shows a card at the top - **"Settings file from a newer CREBLOX (v9)"** in orange, with "Changes you make here are saved into it; its other settings are left untouched." underneath. The version is read once per process (`auth::newer_settings_file`), so the page does not touch the disk on every frame.

### The tests

- **Merge, not replace** - a `schema: 9` file with an unknown key is loaded, `dark` and the picture's `x` are changed, saved, and the file still has `schema: 9`, `some_future_key`, the new `dark` and the new `x`; loading it back gives exactly the value that was saved.
- **This build's own files still write** - `SaveOutcome::Written`, `schema: 2`, and a second save writes again.
- `vm-tests/src/screens.rs` mirrors the new card, and the rendered Settings page was checked at 1280x800 and 1440x900: title orange, body muted, nothing clipped.

376 tests; macOS and Windows (warnings denied) target checks clean, no new warnings.

