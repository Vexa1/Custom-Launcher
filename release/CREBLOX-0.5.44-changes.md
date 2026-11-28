## What changed

### With automatic refresh off, loaded thumbnails are not re-downloaded either

Auditing every periodic fetch turned up one the switch did not cover: `request_thumbs` re-downloads pictures that are already on screen after 120 s, and the pages ask for their visible ids on every frame. That window now comes from `refresh_policy::thumb_refresh_age`, so with the switch off a picture that exists is never fetched again - only ids with no texture at all are requested, because drawing needs those.

Everything else periodic now goes through the same gate or is a user action:

- **Gated:** Home rows, Continue and Favourites, Charts, friends requests, auth refresh (all in `maybe_auto_sync`), and loaded thumbnails.
- **User actions, not timers:** opening a page or game, changing a tab or filter, pressing Refresh, hovering a friend (profile prefetch), equipping an avatar item (debounced save + bake poll), starting a party.
- **Separate switches / local only:** the update check follows *Check for updates*; Discord RPC and the tray are local; the Roblox process watchdogs only observe.

Presence is fetched inside the saved-games sync, so turning the switch off stops it too.

### Tests

`with_auto_refresh_off_a_loaded_thumbnail_is_never_refetched` pins the new window: 120 s of age fetches nothing while off, a day of age fetches nothing, a missing picture still retries on its normal window, and with the switch on the 120 s refresh is due again. Full gate: `bash scripts/verify.sh` green, no new warnings.
