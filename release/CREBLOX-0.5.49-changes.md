## What changed

### A party member who vanishes without closing their socket is now actually detected

A crashed process ends its socket, so the read notices it. A laptop that sleeps, a wifi drop or a NAT mapping that quietly disappears does not: the socket stays open and still accepts the host's keepalive writes, so the keepalive could never prove anyone was still there. The host now remembers when each peer last sent anything and prunes a peer that has been silent for 5 s, checked on every 25 ms tick - one `Instant` comparison per peer, no new thread, no extra traffic.

To make that signal real, guests ping their host every 2 s instead of every 5 s (a handful of tiny frames a minute, per member). A guest still treats a failed ping as a dead host at once.

### Tests, with the measurements

- New `a_peer_that_goes_silent_without_closing_is_pruned_and_turns_red`: a real handshake over a real socket that sends one `join` frame and then nothing at all, while the socket stays open. The host drops that member and the watching roster (and the HUD tile) goes red after **5.00 s** - the window itself, not a multiple of it. The test holds the socket open the whole time, so this cannot pass through the crash path.
- `a_crashed_guest_is_dropped_from_the_roster_immediately` now requires the drop within **500 ms**; it measures one 25 ms tick.

Full gate: 353 tests, macOS and Windows (warnings denied) target checks clean.
