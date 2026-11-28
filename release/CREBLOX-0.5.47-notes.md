## Quick update (existing installs)

- **CREBLOX-0.5.47-update.zip** — 553491 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.47-update.zip — SHA-256 `25767adb5a584cd82c220af07af95da27ec95ea7874c38919359dcddb4eca8cb`

## Full build (fresh installs)

- **CREBLOX-0.5.47-full.zip** — 893392 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.47-full.zip — SHA-256 `4bb80d220e18a0550c96ee82321eb35bd3ce4c92f1017857dd43b10ca3fec283`

## What changed

### Party Overlay: the two tests the HUD was missing

- **Loopback, real sockets.** A host listens, two guests dial it, and the real `Status` frames drive the HUD. A guest watches the host go from `Launching` (grey tile) to `InGame` in that exact place *and* job (green tile); the same roster read against a different job stays grey; then a member leaves the party and their tile turns red for the 30 s grace window and is gone after it. Nothing in the chain is a fixture: the invite code, the handshake, the roster broadcast and the tracker are the shipping code.
- **Tray click, real file.** With `overlay-panels-<id>.json` on disk holding the party card, one Party Overlay click rewrites it without the card and leaves `game_active` and the rest of the spec intact. The duplicate delivery of that same physical click - macOS hands one menu action to every reader - flips nothing and does not touch the file; a second real click after the debounce window brings the card back.

### Simpler, pinned membership

`party_hud::tiles` no longer takes the local user id and no longer filters. Every member in the snapshot gets a tile, your own included: the strip is the whole party, and your own square is green like everyone else's while you are in the server the strip is compared against. A test pins that, including the "not in a server yet, so nothing is green" case.

Full gate: 351 tests, macOS and Windows (warnings denied) target checks clean.

