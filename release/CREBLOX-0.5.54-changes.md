## What changed

### `settings.json` is now tested, not just its helper

0.5.53 tested `SavedImageBox` but not the file itself: the loader and saver lived inside `auth.rs`, which needs the window and the account code, so `vm-tests` could not compile them. A future edit could therefore have dropped `overlay_image_x/y/w/h` from `save_settings` while every test still passed.

The whole `Settings` ↔ `settings.json` mapping now lives in `src/settings_json.rs` (`load_settings_from` / `save_settings_to`), with `Settings`, `BgMode` and `TextColorMode` in `src/settings.rs`. `auth::load_settings` and `auth::save_settings` are two-line wrappers over the same functions, and `crate::types` re-exports the moved types, so no call site changed.

### The tests

Three new tests in `vm-tests/tests/settings_file.rs`, running the real shipping code against a real `Settings` value in a temp directory, 370 in total:

- **Round trip**: a picture at `x 412, y 236, 320 x 128` with deliberately different offsets (`30, -20`), corner `1` and height `40` is saved, the file on disk is checked for all four keys with those exact values, and a fresh load returns the *whole* `Settings` unchanged - box, sliders and corner.
- **Older file**: a 0.5.51 file with the position only in the offsets still opens as a box at `412, 236`.
- **Missing file**: no `settings.json` is a default value, not an error.

`vm-tests` gained a `recursion_limit` because the writer is one long `json!` block.

macOS and Windows (warnings denied) target checks clean; no new warnings.
