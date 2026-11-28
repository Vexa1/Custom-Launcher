## What changed

- **The hands-free updater now runs on both shipping platforms.** The automatic download/install branch was gated to macOS; it now follows any release that carries this platform's zip, so a Windows build performs the same download → verify → swap → relaunch without a click when Windows releases return.
- **The full pipeline has explicit stages and tests.** `auto_update` tracks `Checking → Downloading → Installing → Restarting` (plus `WaitingForLobby` and `Failed`), the launcher drives every transition itself, and the Settings card shows the current stage. The check answer arriving after a release was found no longer resets a pending install. New VM tests cover the complete zero-click sequence, a download finishing mid-match, and failure/retry.
- **The gameplay guard is stronger.** Detection and download completion now also consult the Roblox session banner when the place id could not be parsed, so a session whose log did not yield a place id cannot be interrupted. The per-frame path still uses only the cheap place-id check.
- **No "click to install" in automatic mode.** A completed download reports "installing..." and starts the build immediately; the manual "ready to install" wording remains only when Automatic updates is off.

This completes the automatic-update work first shipped in 0.5.70; 0.5.71 added the Intel macOS link check and release-asset attachment. No manual step remains between finding a release and relaunching into it.
