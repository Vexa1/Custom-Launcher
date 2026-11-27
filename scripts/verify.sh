#!/usr/bin/env bash
# Developer self-check. Run before calling any change done.
#
#   0. `vm-tests/` renders every UI page to a PNG with a real egui and checks the layout.
#   1. `vm-tests/` executes the real party and server-hop code on the build host, which is
#      where the logic can actually run without a window or a Roblox session.
#   2. The macOS target check proves the whole GUI crate still compiles for the release
#      target after a change, including every `#[cfg(test)]` module and `#[cfg(macos)]` path.
#   3. The Windows target check does the same for the x64 build, with warnings denied: every
#      `#[cfg(windows)]` path is compiled on every developer machine, not only on Windows.
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

# Fail fast and unambiguously if a stand-in no longer matches the real declaration, before the
# rest of the suite can pass against a stale shape.
echo "== vm-tests: stand-ins still match the launcher's types and signatures"
cargo test --locked --manifest-path vm-tests/Cargo.toml --test stub_drift

echo "== vm-tests: real party + server-hop code on this host"
cargo test --locked --manifest-path vm-tests/Cargo.toml

# Headless UI screenshots: real egui, a software rasterizer, PNGs in vm-tests/artifacts/.
# The same test is the layout check (clipping, overlap, text fit, hit targets, contrast).
# A UI change is not done until these images have been looked at - see AGENTS.md.
echo "== vm-tests: UI screenshots rendered to vm-tests/artifacts/"
cargo test --locked --manifest-path vm-tests/Cargo.toml --test shots

echo "== creblox: compiles for the macOS release target"
cargo check --locked --target x86_64-apple-darwin

# Same trick as above for Windows: the C parts of the Windows build (ring, mimalloc) need a
# cross cc, and zig's mingw-w64 backend is one. On a Windows host these variables are unset,
# the paths do not exist, and the native build is what counts.
if [[ -z "${CC_x86_64_pc_windows_gnu:-}" && -x /tmp/cc-win ]]; then
    export CC_x86_64_pc_windows_gnu=/tmp/cc-win
    if [[ -x /tmp/zig-linux-x86_64-0.13.0/zig ]]; then
        export AR_x86_64_pc_windows_gnu="/tmp/zig-linux-x86_64-0.13.0/zig ar"
    fi
fi
WINDOWS_STD="$(rustc --print sysroot)/lib/rustlib/x86_64-pc-windows-gnu"
if [[ -d "$WINDOWS_STD" ]]; then
    echo "== creblox: compiles for the Windows x64 target (warnings denied)"
    # `-D warnings`: a warning on this target is a Windows-only path that nobody on a Mac
    # would otherwise compile, so it fails here instead of in the Windows CI job.
    RUSTFLAGS="-D warnings" cargo check --locked --target x86_64-pc-windows-gnu
else
    echo "== creblox: Windows target skipped (x86_64-pc-windows-gnu std not installed)"
fi

echo
echo "All checks passed. On macOS, also run 'cargo test --locked' to execute the in-crate"
echo "#[cfg(test)] suites (layout, updater, auth, theme, log parsing) - they need the"
echo "window/OS APIs that only exist there and cannot run in this VM."
