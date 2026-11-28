## Quick update (existing installs)

- **CREBLOX-0.5.60-update.zip** — 564512 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.60-update.zip — SHA-256 `4894742404d67fc1bc354824ad8ca8f58befdb6649996a5cadaabfd1f94562a0`

## Full build (fresh installs)

- **CREBLOX-0.5.60-full.zip** — 904545 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.60-full.zip — SHA-256 `f54151a78cd5105d4f860645c0948fe5cf1e4d0a2525b0781e489dfc38efca25`

## What changed

### The newer-file merge reads the map once

`save_settings_to` cloned the existing settings map twice just to ask its schema version. It now moves one map out of the file read and gets the version from it in place (`settings_json::version_of(&Map)`), so a save against a newer file does no copying of the file's contents at all.

### The banner is readable in the light theme too

The "Settings file from a newer CREBLOX" title was drawn in the accent orange, which is 7.3:1 on the dark card and **2.5:1 on the light one** - below the 4.5:1 the rest of the UI holds itself to. Text now asks for `theme::warning_ink(dark)`: the accent orange on the dark card, and a darker amber (`ORANGE_INK`, 5.0:1 on white) on the light one. `ORANGE` stays the fill/glow colour it was.

The palette is no longer only checked by eye. `vm-tests` compiles the real `src/theme.rs` (the `eframe` stand-in now re-exports real egui, and the font-candidate lists gained an empty non-macOS/non-Windows fallback so the file builds headless), which means **the theme's own unit tests run in the VM** as well.

### The tests

- `the_banner_title_is_readable_in_the_dark_theme` - `warning_ink(true)` is `ORANGE`, 7.3:1 on the dark card.
- `the_banner_title_is_readable_in_the_light_theme` - `warning_ink(false)` is `ORANGE_INK`, 5.0:1 on the light card, and asserts the accent orange would fail there.
- Both use the real `Palette::of(..)` and the real `shots::contrast_ratio`, so changing either colour or either card background without re-checking fails the suite.

382 tests in the VM suite (376 before, plus these two and the theme module's four that now run); macOS and Windows (warnings denied) target checks clean, no new warnings.

