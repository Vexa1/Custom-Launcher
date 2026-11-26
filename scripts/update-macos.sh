#!/usr/bin/env bash
# One-command CREBLOX update: build this folder and install it over the copy you
# already have (settings and logins stay in ~/Library/Application Support/CREBLOX).
#
# Run it with:   bash scripts/update-macos.sh
# or double-click  Update CREBLOX.command  in the extracted update folder.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

# Version currently installed (for the "old -> new" line below).
OLD_VERSION=""
for app in "/Applications/CREBLOX.app" "$HOME/Applications/CREBLOX.app"; do
    if [[ -d "$app" ]]; then
        OLD_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
            "$app/Contents/Info.plist" 2>/dev/null || true)"
        break
    fi
done

echo "==> Updating CREBLOX ${OLD_VERSION:+$OLD_VERSION -> }$VERSION"
echo "    Building (this takes a few minutes the first time)..."
echo

./scripts/build-macos-app.sh

# Open the freshly installed app.
if [[ -f dist/.installed_path ]] && [[ -d "$(cat dist/.installed_path)" ]]; then
    INSTALLED="$(cat dist/.installed_path)"
else
    INSTALLED="dist/CREBLOX.app"
fi

echo
echo "==> CREBLOX $VERSION is installed: $INSTALLED"
open "$INSTALLED" >/dev/null 2>&1 || true
