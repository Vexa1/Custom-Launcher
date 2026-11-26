#!/usr/bin/env bash
# Developer self-check. Run before calling any change done.
#
#   1. `vm-tests/` executes the real party and server-hop code on the build host, which is
#      where the logic can actually run without a window or a Roblox session.
#   2. The macOS target check proves the whole GUI crate still compiles for the release
#      target after a change, including every `#[cfg(test)]` module and `#[cfg(macos)]` path.
#
# Neither step is part of the shipped app: `vm-tests/` is a separate package that the launcher
# does not depend on, and `scripts/package-release.sh` copies an explicit file list that does
# not include it. Nothing here adds flags, threads or logging to a release build.
set -euo pipefail

cd "$(dirname "$0")/.."

# A Linux sandbox has no Apple toolchain; the C parts of the macOS build (ring, mimalloc) are
# compiled through the zig-based cc shim this project's VM uses. On a real Mac these variables
# are unset, the paths do not exist, and this block does nothing.
if [[ -z "${CC_x86_64_apple_darwin:-}" && -x /tmp/cc-darwin ]]; then
    export CC_x86_64_apple_darwin=/tmp/cc-darwin
    export CFLAGS_x86_64_apple_darwin="-I/tmp/cc-include"
    if [[ -x /tmp/zig-linux-x86_64-0.13.0/zig ]]; then
        export AR_x86_64_apple_darwin="/tmp/zig-linux-x86_64-0.13.0/zig ar"
    fi
fi

echo "== vm-tests: real party + server-hop code on this host"
cargo test --locked --manifest-path vm-tests/Cargo.toml

echo "== creblox: compiles for the macOS release target"
cargo check --locked --target x86_64-apple-darwin

echo
echo "Both passed. On macOS, also run 'cargo test --locked' to execute the in-crate"
echo "#[cfg(test)] suites (layout, updater, auth, theme, log parsing) - they need the"
echo "window/OS APIs that only exist there and cannot run in this VM."
