#!/usr/bin/env bash
# Build both GitHub release packages for the current Cargo.toml version:
#   creblox-<version>-macos-full.zip   complete project (fresh install)
#   creblox-update-<version>-macos.zip small in-place source overlay
#
# The quick update deliberately does NOT contain build.rs, assets or the full
# build command. Update CREBLOX.command uses build.rs to tell a complete
# project apart from an overlay, so copying those files would break in-place
# updates.
#
# Release notes must list the quick update zip URL before the full build URL.
# The in-app updater downloads the first zip link it finds.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[[ -n "$VERSION" ]] || { echo "Could not read version from Cargo.toml" >&2; exit 1; }

# --- Release link gate ----------------------------------------------------
# These archives are what users download. Never write them from a tree that was
# only `cargo check`ed: prove the real Catalina link runs cc/ld first and abort
# here if it fails (0.4.25 shipped an undefined symbol this way).
#
# CREBLOX_SKIP_LINK_GATE=1 is the only way past it, for producing source
# archives on a host that cannot link Mach-O at all (no macOS SDK). It says so
# on stderr every time, so an unverified archive is never a silent one.
if [[ "${CREBLOX_SKIP_LINK_GATE:-0}" == "1" ]]; then
    printf '\nwarning: CREBLOX_SKIP_LINK_GATE=1 - the x86_64 Catalina link was NOT verified.\n' >&2
    printf '         These archives must still be built once on macOS before publishing.\n\n' >&2
else
    scripts/verify-release-link.sh
fi

FULL_ZIP="creblox-${VERSION}-macos-full.zip"
QUICK_ZIP="creblox-update-${VERSION}-macos.zip"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

# --- Complete project -----------------------------------------------------
FULL="$STAGE/CREBLOX $VERSION"
mkdir -p "$FULL"
cp -R assets scripts src "$FULL/"
cp Cargo.toml Cargo.lock build.rs README.md LICENSE .gitignore \
    "Build CREBLOX.command" "Update CREBLOX.command" \
    creblox-update-remove.txt "$FULL/"
find "$FULL" -name '.DS_Store' -delete
# build.rs is what makes this a complete project rather than an overlay.
[[ -f "$FULL/build.rs" ]] || { echo "Full package is missing build.rs" >&2; exit 1; }
[[ -f "$FULL/assets/logo.png" ]] || { echo "Full package is missing assets" >&2; exit 1; }

# --- Small in-place update ------------------------------------------------
QUICK="$STAGE/CREBLOX $VERSION update"
mkdir -p "$QUICK"
cp -R scripts src "$QUICK/"
cp Cargo.toml Cargo.lock "Update CREBLOX.command" \
    creblox-update-remove.txt "$QUICK/"
find "$QUICK" -name '.DS_Store' -delete
if [[ -e "$QUICK/build.rs" ]]; then
    echo "Quick update must not contain build.rs" >&2
    exit 1
fi

rm -f "$FULL_ZIP" "$QUICK_ZIP"
(
    cd "$STAGE"
    zip -r -X -q "$OLDPWD/$FULL_ZIP" "CREBLOX $VERSION"
    zip -r -X -q "$OLDPWD/$QUICK_ZIP" "CREBLOX $VERSION update"
)

full_sha="$(shasum -a 256 "$FULL_ZIP" | cut -d' ' -f1)"
quick_sha="$(shasum -a 256 "$QUICK_ZIP" | cut -d' ' -f1)"
full_size="$(wc -c < "$FULL_ZIP" | tr -d ' ')"
quick_size="$(wc -c < "$QUICK_ZIP" | tr -d ' ')"

echo
echo "Full build: $FULL_ZIP"
echo "  SHA-256: $full_sha"
echo "  Size:    $full_size bytes"
echo "Quick update: $QUICK_ZIP"
echo "  SHA-256: $quick_sha"
echo "  Size:    $quick_size bytes"
echo
echo "Publish both. Put the quick update URL and SHA first in the release"
echo "notes; the in-app updater selects the first zip link."
