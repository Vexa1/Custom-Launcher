## What changed

### A macOS client and a Windows client now prove they work in the same party, on every build

A party is OS-agnostic by construction: one WebSocket carrying JSON, a 22-byte invite in base62, `u64` ids and a 16-bit party size. Nothing enforced that, so this release adds `vm-tests/tests/party_cross_platform.rs`, and `scripts/verify.sh` now runs it by name before the full suite.

Each simulated client carries what really differs between the platforms - the game's install path, the log line format, the headshot URLs - and the suite checks that none of it reaches the wire or the panels file.

- **macOS hosts, Windows joins.** The Windows client decodes the macOS `CRB-...` invite; both rosters match field for field; `Loading` is grey on both, both in the same place and job is green on both, and the two `overlay-panels-*.json` files carry an identical `party` panel. A launch to a new game reaches the Windows client with the same `place_id`, `job_id` and `by` (what its `RequestGameJob` handler opens a client for); a launch for the server you are already in is correctly ignored. When the Windows client is killed without a `Leave`, the macOS HUD turns its tile red and keeps its own green.
- **Windows hosts, macOS joins.** Ready Check started on Windows and answered on macOS, a suggestion and a vote crossing both ways, a Party Server Hop request heard exactly once, and a host that ends the party telling the macOS guest immediately - measured at **1.1 ms**, not a timeout on a dead socket.
- **The wire audit.** Party size is now a fixed 16-bit number (`wire_size`), not a `usize`: the JSON a macOS host writes no longer depends on that process's pointer width. Ids were already `u64`, the invite payload was already fixed bytes read big-endian, and the status strings are ASCII, so `server` / `loading` / `offline` mean the same thing on both platforms.

Full gate: 356 tests, the macOS and Windows (warnings denied) target checks clean.
