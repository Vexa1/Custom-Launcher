## What changed

### CREBLOX reacts to macOS memory pressure

A libdispatch memory-pressure source is created on the main queue when the launcher starts. Its handler does no work: it records the level macOS reported (`normal`, `warn`, `critical`) and asks egui for a frame. The next `App::poll` picks the level up and runs the flush there, because a texture may not disappear inside a dispatch callback.

- **warn** runs `flush_image_caches()` - the exact path a tray tuck uses, dropping thumbnails, decoded pixels, video frames, the 3D avatar scene and the profile shelf. Everything is fetched again on demand.
- **critical** does that, then hands the freed pages back with `trim_process_memory()`, re-checks the 192 MB budget, and says so in the status line.
- **hidden** in the menu bar asks for nothing: `tuck_into_menu_bar` already flushed, so there is nothing left to free and no reason to wake a window.

The decision is debounced for 30 seconds per level so a level that keeps flipping cannot thrash the texture pool, but an escalation from warn to critical is never swallowed - that is the moment the deeper trim matters most.

### Tests

New `vm-tests/tests/memory_pressure.rs` covers the whole decision: warn flushes, critical deep-trims, a hidden launcher does nothing, the same level inside 30 s is ignored and due again after it, an escalation inside the window still deep-trims, and a repeated critical waits its turn. Full gate: `bash scripts/verify.sh` green, no new warnings.
