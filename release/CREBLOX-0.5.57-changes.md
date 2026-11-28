## What changed

### `settings.json` names its layout

Every migration so far guessed from a **missing key**: the 0.5.52 picture box was recognised by a version 1 file having no `x`/`y`, and that only works once. A second migration of the same file would find the same key absent for an unrelated reason and guess wrong.

The file now carries `"schema": 2`, and the reader uses that number instead of inspecting which keys exist:

- **`settings_json::SETTINGS_VERSION`** is the layout this build writes; **`settings_version`** reads a file's version, treating a missing field - or a nonsense one - as **1**, which is exactly what every file written before the field existed is.
- **Version 1** keeps the old rule: a saved box with no `x`/`y` takes its position once from the offset sliders, so nobody's picture jumps on upgrade.
- **Version 2 and later** take the position only from `x`/`y`. A box without them is at `0, 0`; the offsets are sliders again in every case, never a hiding place for a position.
- **A newer schema** (say a file from a future build) is still read for everything this build understands and rewritten as version 2, instead of falling back to defaults.
- Every save writes the version, so a file that has been opened once is self-describing from then on.

### The tests

Three new tests in `vm-tests/tests/settings_file.rs`, 375 in total:

- **The file names its version** - a saved file has `schema` equal to `SETTINGS_VERSION` and reads back at that version.
- **Version 2 ignores the offsets** - `w/h 320x128` with offsets `412, 236` and no `x`/`y` loads at `0, 0`, then settles after a save and reload.
- **A newer schema still opens** - `schema: 9` with a box and an unknown key loads the dark theme and the box `412, 236, 320x128`, and is rewritten as version 2.
- The existing version 1 test now also asserts the file reads as version 1 and still migrates its position out of the offsets.

macOS and Windows (warnings denied) target checks clean; no new warnings.
