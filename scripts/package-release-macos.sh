#!/usr/bin/env bash
# Package dist/Aurora.app as dist/aurora-<version>-macos.zip for a GitHub release.
#
# RELEASING.md requires this exact asset name and requires the archive to be made
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

echo "==> Packaging Aurora $VERSION"
NO_INSTALL=1 scripts/build-macos-app.sh

APP="dist/Aurora.app"
if [[ ! -d "$APP" ]]; then
    echo "error: $APP is missing" >&2
    exit 1
fi

ZIP="dist/aurora-${VERSION}-macos.zip"
rm -f "$ZIP"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

echo
echo "Wrote: $ZIP"
echo "Attach exactly this file to the v$VERSION release (GitHub computes its"
echo "sha256 digest itself, so no checksum file is needed):"
shasum -a 256 "$ZIP" | awk '{ print "  sha256: " $1 }'
