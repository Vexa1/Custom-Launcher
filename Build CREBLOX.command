#!/bin/bash
# Double-click this file in Finder to BUILD + INSTALL CREBLOX (only needed when the code changes).
# Afterwards open CREBLOX from Launchpad / Spotlight / the Dock like any other app.
cd "$(dirname "$0")" || exit 1
export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:$PATH"

pause_and_exit() {
    echo
    read -n 1 -s -r -p "Press any key to close this window..."
    echo
    exit "${1:-0}"
}

if ! xcode-select -p >/dev/null 2>&1; then
    echo "Xcode Command Line Tools are required."
    echo "Starting the installer now - finish it, then double-click this file again."
    xcode-select --install >/dev/null 2>&1 || true
    pause_and_exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust is not installed."
    echo "Install it from https://rustup.rs (one line in Terminal), then double-click this file again."
    pause_and_exit 1
fi

./scripts/build-macos-app.sh || pause_and_exit 1
if [ -f dist/.installed_path ] && [ -d "$(cat dist/.installed_path)" ]; then
    open "$(cat dist/.installed_path)"
else
    open "dist/CREBLOX.app"
fi
pause_and_exit 0
