## Quick update (existing installs)

- **CREBLOX-0.5.45-update.zip** — 546618 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.45-update.zip — SHA-256 `0a03ee6ea689fac98d3d8675fcf08f4adba5dffbfd276250f47e415a1fd86338`

## Full build (fresh installs)

- **CREBLOX-0.5.45-full.zip** — 886369 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.45-full.zip — SHA-256 `f885202c65d58a2d517d6a8b2d5c833e67c287175b26e37c7f3885a811f87df7`

## What changed

### CREBLOX reacts to macOS memory pressure

A libdispatch memory-pressure source is created on the main queue when the launcher starts. Its handler does no work: it records the level macOS reported (`normal`, `warn`, `critical`) and asks egui for a frame. The next `App::poll` picks the level up and runs the flush there, because a texture may not disappear inside a dispatch callback.

- **warn** runs `flush_image_caches()` - the exact path a tray tuck uses, dropping thumbnails, decoded pixels, video frames, the 3D avatar scene and the profile shelf. Everything is fetched again on demand.
- **critical** does that, then hands the freed pages back with `trim_process_memory()`, re-checks the 192 MB budget, and says so in the status line.
- **hidden** in the menu bar asks for nothing: `tuck_into_menu_bar` already flushed, so there is nothing left to free and no reason to wake a window.

The decision is debounced for 30 seconds per level so a level that keeps flipping cannot thrash the texture pool, but an escalation from warn to critical is never swallowed - that is the moment the deeper trim matters most.

### Tests

New `vm-tests/tests/memory_pressure.rs` covers the whole decision: warn flushes, critical deep-trims, a hidden launcher does nothing, the same level inside 30 s is ignored and due again after it, an escalation inside the window still deep-trims, and a repeated critical waits its turn. Full gate: `bash scripts/verify.sh` green, no new warnings.

