#!/usr/bin/env bash
# Package dist/CREBLOX.app as dist/creblox-<version>-macos-app.zip for a GitHub release.
#
# Two guards keep a non-Catalina build from shipping:
#   1. scripts/verify-release-link.sh must link the x86_64 Catalina target, so a
#      check-only tree can never reach the zip.
#   2. the packaged app's own binary must contain an x86_64 slice. An Apple
#      Silicon host build is arm64-only even when the x86_64 target links.
#
# memory.md requires this exact asset name and requires the archive to be made
# with `ditto`: a Finder/`zip` archive drops resource forks and xattrs and breaks
# the ad-hoc code signature, which makes the updater refuse to install it.
#
# Run from anywhere on macOS: scripts/package-release-macos.sh
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [[ -z "$VERSION" ]]; then
    echo "error: could not read the version from Cargo.toml" >&2
    exit 1
fi

echo "==> Packaging CREBLOX $VERSION"

# 1. Real link first, before spending minutes on the bundle build.
scripts/verify-release-link.sh

NO_INSTALL=1 scripts/build-macos-app.sh

APP="dist/CREBLOX.app"
if [[ ! -d "$APP" ]]; then
    echo "error: $APP is missing" >&2
    exit 1
fi

# 2. Catalina is Intel-only. The gate above proves the x86_64 target links, but
#    `cargo build --release` in build-macos-app.sh builds the host CPU, so on an
#    Apple Silicon Mac the bundle would still be arm64-only.
BIN="$APP/Contents/MacOS/creblox"
APP_ARCHS="$(lipo -archs "$BIN" 2>/dev/null || true)"
case " $APP_ARCHS " in
    *" x86_64 "*) ;;
    *)
        echo "error: $BIN has no x86_64 slice (architectures: ${APP_ARCHS:-unknown})." >&2
        echo "       Catalina cannot run this app, so it is not packaged." >&2
        echo "       Build and package on an Intel Mac, or produce a universal binary." >&2
        exit 1
        ;;
    esac
echo "==> App architectures: $APP_ARCHS"

ZIP="dist/creblox-${VERSION}-macos-app.zip"
rm -f "$ZIP"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

echo
echo "Wrote: $ZIP"
echo "Attach exactly this file to the v$VERSION release (GitHub computes its"
echo "sha256 digest itself, so no checksum file is needed):"
shasum -a 256 "$ZIP" | awk '{ print "  sha256: " $1 }'
