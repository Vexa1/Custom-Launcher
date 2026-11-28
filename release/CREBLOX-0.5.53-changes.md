## What changed

### The image's saved box is read and written in one place, with tests

0.5.52 put the picture's box in `overlay_image_x/y/w/h` and migrated a 0.5.51 file out of the offsets, but that reading lived inline in both `settings.json`'s loader and the overlay window's spec parser, and nothing tested it.

It is now `overlay_spec::SavedImageBox`, used by all four paths: settings load, settings save, the panels spec, and the overlay window's parser. Same behaviour, one implementation.

- `from_settings` / `from_panels_image` - the long keys of `settings.json` and the short keys of the panels file.
- A saved box (`w`/`h` at 24 or more) wins; a 0.5.51 file with a box but no `x`/`y` is read once from the offsets it wrote.
- Without a box, `x`/`y` are not taken from the offsets at all: those stay the corner sliders' own values.
- `insert_into` writes the four keys, so the save path uses the same definition of the box it reads back.

### The tests

Four new tests in `vm-tests/tests/overlay_spec.rs`, 367 in total:

- A **0.5.51 file** with the position in the offsets and a 320x128 box migrates to `x 412, y 236, 320 x 128` and counts as saved.
- A **round trip** through `settings.json` text keeps the box exactly, leaves the corner, the height slider and the offsets (`30, -20`) untouched, and is idempotent on a second write.
- **No box** (0, 3x3, or missing keys) is not a saved box, and the offsets are never mistaken for a position.
- The **panels file** object with short keys and disagreeing offsets resolves to `x`/`y`, not the offsets.

macOS and Windows (warnings denied) target checks clean; no new warnings.
