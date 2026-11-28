## What changed

### A crashed party member now leaves the roster at once

The host already noticed a socket that ended without a `Leave` frame, but it pruned the peer silently: no new roster was broadcast, so every other guest kept drawing someone who was gone and their HUD never turned that tile red until some unrelated status change happened. The prune now rebroadcasts the roster on the same 25 ms tick, and the keepalive prune does the same.

The keepalive also drops from 15 s to 5 s, which bounds the only remaining case - a peer that vanishes without closing its socket (wifi drop, sleep): that member is stale for at most five seconds instead of fifteen. It is a few tiny frames a minute to the handful of people in a party.

### Test, with the measurement

New loopback test `a_crashed_guest_is_dropped_from_the_roster_immediately`: a host, a watching guest and a victim whose handle is dropped, which shuts its worker down without a `Leave` frame - the same socket ending a killed process produces. The watching guest must see the two-member roster and the red `Disconnected` tile, and the test times it: **27 ms** in this VM, one tick, against a 2 s ceiling.

Full gate: 352 tests, macOS and Windows (warnings denied) target checks clean.
