## Quick update (existing installs)

- **CREBLOX-0.5.42-update.zip** — 540442 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.42-update.zip — SHA-256 `f306a29ea847794d015c80a16184f41680ecdea1102acc17196e60903f3706e4`

## Full build (fresh installs)

- **CREBLOX-0.5.42-full.zip** — 880235 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.42-full.zip — SHA-256 `c9b82106d7f44a9494481625204c30f567a0560fe5eb40f2e5b2a454c0938bbb`

## What changed

### A single decode can no longer blow the budget

Every cache is budgeted in bytes, but the decoder allocated its picture before the budget could see it: a 20000x20000 PNG is 1.6 GB of RGBA in one worker, and a burst of four could dwarf the 192 MB ceiling. A thumbnail worker now reads at most 12 MB of body and checks the picture's header size before decoding; anything past 2560x2560 (about 26 MB of RGBA) is refused and logged instead of allocated. The 3D avatar skull was the first suspect, the decoder is the one path around the accounting.

### Settings shows what the process really uses

The image-cache row now prints the tracked total **and** the process's own RSS next to it (`Image caches: 3.1 MB / 192 MB · process 84 MB`). If the tracked number stays low while the process grows, the remaining RAM is eframe's image loader, the driver or the overlay - not the launcher's caches.

### Release notes are generated and checked against the zips

`scripts/release-notes.sh` reads the packaged archives, prints each download's real byte size and SHA-256 on the same line as its link, and runs `vm-tests/tests/release_notes_generated.rs`, which parses the body exactly as the updater does and fails if a printed hash, size or link disagrees with the zip. The 0.5.39 "Update zip failed SHA-256 verification" report cannot come back through a hand-typed body.

### Tests

`decode_cap.rs` pins the per-decode ceiling and proves the lane sizes still fit under it. Full gate: `bash scripts/verify.sh` green, no new warnings.

