## What changed

### The settings round trip is tested with real shapes, not defaults

0.5.54 proved a `Settings` value survives `settings.json`, but its fixture was almost entirely default values, so a lossy encoding of a field the defaults never exercise could still slip through: an optional colour that came back `None`, a limit that came back rounded, a panel offset that came back empty, a path that lost its non-ASCII bytes.

`vm-tests/tests/settings_file.rs` now has a second fixture that is non-default in every shape the file carries, and asserts the whole `Settings` still compares equal after writing and reading it back:

- **Optional colours** - `borders_color`, `profile_color`, `friends_color`, `text_border_color` set to four distinct triples, `games_border_color` deliberately `None` so `null` is checked as a value and not as a dropped key.
- **Optional limits** - `target_fps 144`, `ram_limit_mb 3072`, `cpu_limit_pct 65`, `server_min_players 2`, `server_max_players 24`, `server_max_ping 150`, with unset neighbours left unset.
- **Overlay panel offsets** - three `PanelOffset` entries (`server -37,214`, `stats 12,-8`, `party 0,41`) including negative values, checked in the file as well as after loading.
- **Unicode paths** - a wallpaper in `壁紙/салют ünïcode.png` and a picture named `логотип 🖼️ ロゴ.png` in a `十字` folder, byte-for-byte after the round trip.
- **The rest of the range** - `BgMode::SolidColor`, `TextColorMode::White`, `dark`, a crosshair with a non-default style/size/thickness/gap/colour and negative offset, `overlay_duration_secs 250`, `overlay_flags_max_lines 7`, a Discord app id, and booleans flipped from their defaults.

The fixture also asserts it differs from `Settings::default()`, so it cannot decay back into a defaults-only test.

The whole-value assertion already earned its place: `ram_limit_mb 6144` came back as `4096` because the loader caps RAM at 128-4096 (the same range `telemetry.rs` reports). That is a documented normalisation, not a lost field, so the fixture uses an in-range `3072` and the strict comparison stays.

macOS and Windows (warnings denied) target checks clean; no new warnings.
