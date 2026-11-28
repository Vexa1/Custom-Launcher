## What changed

- The README now documents the whole app, not just its early feature set: party rooms and the
  party overlay, server hop, the four-tier ping probe, memory-pressure handling, automatic
  refresh policy, scroll pacing, thumbnail lanes and stale-while-revalidate merging.
- Added the modules that were missing from the directory map (`party`, `party_server`,
  `party_hud`, `server_hop`, `net_probe`, `overlay_spec`, `settings`, `settings_json`,
  `game_log`, `chart_feed`, `inventory`, `memory_pressure`, `refresh_policy`, `scroll`,
  `window_lifecycle`, `release_links`, `sha256`, `zip_update`) and the two missing pages
  (`page_inventory`, `ui/party`).
- `settings.json` is documented field by field, including the overlay picture box, panel
  offsets, party strip, crosshair and server-browser filters, plus the `schema` number and the
  rule that a newer file is merged into rather than overwritten.
- The verification section now lists the stand-in drift, cross-platform party, overlay ping
  end-to-end and both-palette screenshot checks, and troubleshooting covers party and hop.
