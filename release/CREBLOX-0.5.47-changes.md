## What changed

### Party Overlay: the two tests the HUD was missing

- **Loopback, real sockets.** A host listens, two guests dial it, and the real `Status` frames drive the HUD. A guest watches the host go from `Launching` (grey tile) to `InGame` in that exact place *and* job (green tile); the same roster read against a different job stays grey; then a member leaves the party and their tile turns red for the 30 s grace window and is gone after it. Nothing in the chain is a fixture: the invite code, the handshake, the roster broadcast and the tracker are the shipping code.
- **Tray click, real file.** With `overlay-panels-<id>.json` on disk holding the party card, one Party Overlay click rewrites it without the card and leaves `game_active` and the rest of the spec intact. The duplicate delivery of that same physical click - macOS hands one menu action to every reader - flips nothing and does not touch the file; a second real click after the debounce window brings the card back.

### Simpler, pinned membership

`party_hud::tiles` no longer takes the local user id and no longer filters. Every member in the snapshot gets a tile, your own included: the strip is the whole party, and your own square is green like everyone else's while you are in the server the strip is compared against. A test pins that, including the "not in a server yet, so nothing is green" case.

Full gate: 351 tests, macOS and Windows (warnings denied) target checks clean.
