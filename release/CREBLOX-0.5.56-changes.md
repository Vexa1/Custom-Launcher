## What changed

### Out-of-range settings are tested to settle, not oscillate

Every numeric setting is clamped on load, so a hand-edited `settings.json` cannot park a slider outside its range. What was not tested is whether the clamp is a *fixed point*: if the loader normalised a value while the writer kept the number it read, the file would change on every launch and two launches could disagree about the same setting.

`vm-tests/tests/settings_file.rs` now has `out_of_range_values_are_normalised_and_then_stay_put`, which writes a file with values no slider can produce and checks both halves:

- **Normalised on load** - `ram_limit_mb 6144` becomes `4096`, `cpu_limit_pct 5` becomes `10`, `crosshair_offset_x -1000` becomes `-100`, `glass_opacity 3.0` becomes `0.92`, `overlay_image_corner 9` becomes `3`, `overlay_image_size 250` becomes `100`, `overlay_flags_max_lines 99` becomes `40`.
- **The picture's box** - `x -50` becomes `0` and `y 9000` becomes `8000`.
- **Panel offsets** - `server -9000, 9000` becomes `-8000, 8000` and keeps its entry; a neighbouring in-range panel (`stats 5, -5`) is untouched, so the clamp is not rounding everything.
- **Stable across a second pass** - the loaded value is written back and loaded again, and the two `Settings` values must compare equal. The second file is also inspected directly to confirm it holds the clamped numbers, not the hand-edited ones.

372 tests in the VM suite; macOS and Windows (warnings denied) target checks clean, no new warnings.
