## Quick update (existing installs)

- **CREBLOX-0.5.70-update.zip** — 579725 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.70-update.zip — SHA-256 `8348c598427f0e0170a6a6769e8d265812e0d6e6798c8f79f1e65b390e308402`

## Full build (fresh installs)

- **CREBLOX-0.5.70-full.zip** — 925036 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.70-full.zip — SHA-256 `24d6ded51bc125460c8359bf2656437e60393616ed57113d3e0116d0cdb9227b`

## What changed

- **macOS link fixed.** `dispatch_get_main_queue()` is a C macro over the exported `_dispatch_main_q` object, not a function. 0.5.69 declared it as one, so Apple's `ld` failed with `_dispatch_get_main_queue, referenced from creblox::memory_pressure_macos::start`. The memory-pressure source now uses the real libSystem symbol, matching the tray's existing dispatch code.
- **No more compile-error releases.** `scripts/verify.sh` refuses any `fn dispatch_get_main_queue` in `src/`, and a new `verify-macos` GitHub Actions workflow runs `cargo build --release --locked` with `MACOSX_DEPLOYMENT_TARGET=10.15` on `macos-latest`, so Apple's linker checks every push.
- **Four dead-code warnings removed** (`sync_to_version_dir` on macOS, the two unused tray startup-command helpers, and `CREATE_NO_WINDOW` off Windows).
- **Automatic updates (off by default).** Settings → Launcher Updates gains an `auto_update` switch, stored in `settings.json`, so it survives restarts. While it is on, CREBLOX checks GitHub's latest release every 30 seconds with `If-None-Match`/ETag (an unchanged release answers 304 without downloading), then downloads, verifies SHA-256, rebuilds and restarts inside the app with the progress bar, without a click.
- **Gameplay is protected.** A release found during a Roblox match is remembered and installed as soon as the player is back in the lobby; a download that finishes mid-match waits too.
- **No failure loop.** A tag whose download or build fails is remembered for the session, so the 30-second loop cannot rebuild it forever. A newer tag is still tried automatically, and Install & Restart retries the failed tag immediately.

