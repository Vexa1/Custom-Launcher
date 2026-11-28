## Quick update (existing installs)

- **CREBLOX-0.5.46-update.zip** — 553461 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.46-update.zip — SHA-256 `9b36dfbd1d65cc6e69a6c8e2739d6a205811110cbbd8e406510bf6e86931fe91`

## Full build (fresh installs)

- **CREBLOX-0.5.46-full.zip** — 893362 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.46-full.zip — SHA-256 `555a961b01c717304ef6332416d884782a9818db55daee18e350ec76f5513712`

## What changed

### Party Overlay HUD

When you are in a party and the new **Party Overlay** switch is on, the in-game HUD gains a party card next to the server and stats cards - the same window, the same draggable card in *Edit Overlay Position*, no extra OS window.

- Each member is a 44 px rounded square (8 px corner radius) showing their 60x60 headshot, with two initials while it loads and if it never arrives.
- A 2 px outline says where that member is: **green** `#22C55E` in this exact server (place id *and* job id), **grey** `#9CA3AF` while they launch, load or wait in the lobby, **red** `#EF4444` for the 30 seconds after their connection drops mid-match.
- Names sit under each square, shortened with an ellipsis so they never run into the next tile.

**Zero polling.** Tiles are rebuilt only when a party snapshot arrives or your own server changes. Headshots are fetched three at a time, cached as 60x60 textures only for members currently in the party, and dropped the moment someone leaves. The panels file is only rewritten when a tile actually changes, so an idle party writes nothing and draws nothing new.

### The switch

**Party Overlay** is in the macOS and Windows tray menus next to the other HUD switches, and in Settings → In-Game Overlay. A tray click is applied on the thread that read it: `settings.json` is saved and `overlay-panels-<id>.json` is rewritten immediately, so the card appears or disappears mid-game within one overlay step (100 ms). The launcher's replayed copy of that click only adopts the saved state - it never rewrites the file twice.

### Tests

New `vm-tests/tests/party_hud.rs` (8 cases): the colour for every state including "same place, different job", initials, one tile per member, a vanished member turning red and expiring after 30 s, leaving the party dropping every tile, a new party not inheriting the old room's tiles, the exact panels JSON the HUD parses, and the tray switch flipping only itself. New screenshot `vm-tests/artifacts/overlay_party_hud.png` renders a card with all three states from that same panels JSON, and was inspected: rounded corners, equal 2 px outlines, centred initials, names clear of each other. Full gate: 347 tests, macOS and Windows (warnings denied) target checks clean.

