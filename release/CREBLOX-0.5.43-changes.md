## What changed

### Every image decoder is capped now

The thumbnail lanes were capped at 2560x2560 per decode; the inventory icon fetch and the 3D avatar texture loader were not, so a single huge picture on those paths could still allocate past the 192 MB budget before any accounting saw it. Both now read at most 12 MB, read the picture's header size, and refuse anything past the cap through the same `decode_fits` gate (logged, with the URL or texture hash). There is no decoder left that can allocate unbounded pixels.

### `Automatic background refresh` switch

Settings -> Tray/Launcher now has **Automatic background refresh** with the subtitle *Silently update game lists, player counts, and friends activity in the background*. Turning it off stops every periodic fetch: Home rows, Continue and Favourites, Charts, friends requests and auth refresh all go through the new `refresh_policy::should_refresh` gate, which answers `false` whenever the switch is off however long the timers have been running. Data still loads on page open, tab change, filter change and the explicit Refresh buttons.

### Background merges no longer move cards

A Charts revalidation now patches the rows in place: sorts are matched by title and games by `universe_id`, so counts update where the card sits and no row is re-ranked under the pointer. Only an explicit refresh (or the first load) applies a new ranking. Home rows already merged in place; that behaviour is unchanged.

### Tests

New `vm-tests/tests/refresh_policy.rs`: with the switch off, every cadence the launcher uses (3 s to 90 s) starts zero fetches, including after a day of idling; with it on the same ticks do fetch, and an in-flight fetch still blocks a second. Merge cases assert card order, chart row order and texture keys survive a background answer, that newcomers are not inserted mid-row, and that a Home refresh appends rather than reorders. Full gate: `bash scripts/verify.sh` green, no new warnings.
