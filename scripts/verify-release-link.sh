#!/usr/bin/env bash
# Fail-closed release-link gate.
#
# `cargo check` type-checks but never runs `cc`/`ld`, so a symbol that does not
# exist (0.4.25: `_dispatch_get_main_queue`, a C macro rather than a function)
# passes every check and fails only when a user builds. Archives must therefore
# never be produced from a check-only tree: this script runs the real release
# link for the Catalina target and aborts the packaging path when it fails.
#
# Run directly before tagging, or let scripts/package-release.sh call it:
#   scripts/verify-release-link.sh
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET="${CREBLOX_RELEASE_TARGET:-x86_64-apple-darwin}"

fail() {
    printf '\nerror: %s\n' "$1" >&2
    shift || true
    for line in "$@"; do printf '       %s\n' "$line" >&2; done
    printf '\n' >&2
    exit 1
}

# The link pulls in AppKit/CoreText/objc and runs Apple's ld64. A Linux or CI box
# can type-check with a zig shim but cannot prove the link, so it must not
# report success.
if [[ "$(uname -s)" != "Darwin" ]]; then
    fail "the release link can only be verified on macOS (this host is $(uname -s))." \
         "Build and package on a Mac; a type-check alone cannot ship."
fi

command -v cargo >/dev/null 2>&1 \
    || fail "cargo was not found. Install Rust from https://rustup.rs and retry."

if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
        fail "the Rust target $TARGET is not installed." \
             "Run: rustup target add $TARGET"
    fi
fi

if ! xcode-select -p >/dev/null 2>&1; then
    fail "Xcode Command Line Tools are missing." \
         "Run: xcode-select --install"
fi

export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.15}"

# `-C target-cpu=native` (set by scripts/build-macos-app.sh) describes the host
# CPU and is invalid for a cross target, so the probe build starts clean.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS

BIN="target/${TARGET}/release/creblox"
LOG="${TMPDIR:-/tmp}/creblox-release-link.log"

echo "==> Verifying the real release link for $TARGET (runs cc/ld)"
if ! cargo build --release --target "$TARGET" 2>&1 | tee "$LOG"; then
    fail "the $TARGET release link failed, so packaging was aborted." \
         "Never publish archives from a tree that was only checked." \
         "Fix the undefined symbol or framework above and rerun." \
         "Full log: $LOG"
fi

[[ -f "$BIN" ]] \
    || fail "cargo reported success but $BIN is missing; packaging was aborted."

ARCHS="$(lipo -archs "$BIN" 2>/dev/null || true)"
case " $ARCHS " in
    *" x86_64 "*) ;;
    *) fail "$BIN is not an x86_64 binary (architectures: ${ARCHS:-unknown});" \
            "Catalina cannot run it." ;;
esac

printf '\n==> Release link OK: %s (%s)\n' "$BIN" "$ARCHS"
