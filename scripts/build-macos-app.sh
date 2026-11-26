#!/usr/bin/env bash
# Build CREBLOX as a native macOS .app bundle (Intel and Apple Silicon).
# Run from anywhere: scripts/build-macos-app.sh
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.15}"

# Compile for the exact CPU in this Mac so LLVM can use its SIMD instructions
# (SSE4/AVX/AVX2 as available). This is a local build for your own machine; set
# CREBLOX_PORTABLE=1 to produce a generic x86-64 binary instead. CREBLOX always
# runs on macOS 10.15+ either way.
if [[ "${CREBLOX_PORTABLE:-0}" != "1" ]]; then
    export RUSTFLAGS="${RUSTFLAGS:-} -C target-cpu=native"
fi

echo "==> Building CREBLOX $VERSION (release, deployment target $MACOSX_DEPLOYMENT_TARGET)"
cargo build --release

APP="dist/CREBLOX.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "target/release/creblox" "$APP/Contents/MacOS/creblox"
chmod +x "$APP/Contents/MacOS/creblox"
cp assets/logo.png "$APP/Contents/Resources/logo.png"

# --- Icon ---------------------------------------------------------------
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
# iconutil only accepts these exact ten names. Generating extra sizes (64x64, …) made
# it fail, which left the app without a real icon in the Dock and Finder.
icon_png() { sips -z "$2" "$2" assets/logo.png --out "$ICONSET/$1" >/dev/null; }
icon_png icon_16x16.png 16
icon_png icon_16x16@2x.png 32
icon_png icon_32x32.png 32
icon_png icon_32x32@2x.png 64
icon_png icon_128x128.png 128
icon_png icon_128x128@2x.png 256
icon_png icon_256x256.png 256
icon_png icon_256x256@2x.png 512
icon_png icon_512x512.png 512
icon_png icon_512x512@2x.png 1024
if iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"; then
    echo "==> Icon built"
else
    echo "==> Warning: iconutil failed; the app keeps the PNG icon instead"
fi
rm -rf "$(dirname "$ICONSET")"

# --- Info.plist ---------------------------------------------------------
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>CREBLOX</string>
    <key>CFBundleDisplayName</key>
    <string>CREBLOX</string>
    <key>CFBundleIdentifier</key>
    <string>com.creblox.app</string>
    <key>CFBundleExecutable</key>
    <string>creblox</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.games</string>
</dict>
</plist>
PLIST

# Ad-hoc signature keeps Gatekeeper and the Keychain happy for a local build.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

echo
echo "Built: $APP"

# --- Install -------------------------------------------------------------
# Installs the app so you can open it like any other Mac app (Launchpad,
# Spotlight, Dock) instead of running this script each time.
# Set NO_INSTALL=1 to only build dist/CREBLOX.app.
if [[ "${NO_INSTALL:-0}" == "1" ]]; then
    echo "Skipping install (NO_INSTALL=1). Launch with: open \"$APP\""
    exit 0
fi

DEST_DIR="/Applications"
if [[ ! -w "$DEST_DIR" ]]; then
    DEST_DIR="$HOME/Applications"
    mkdir -p "$DEST_DIR"
fi
DEST="$DEST_DIR/CREBLOX.app"

# 0.3.x installed as Aurora.app under a different bundle id. Leaving it behind
# would put two launchers in Applications, so the rename removes it.
for OLD_APP in "$DEST_DIR/Aurora.app" "$HOME/Applications/Aurora.app"; do
    if [[ -d "$OLD_APP" ]]; then
        echo "==> Removing the pre-rename app: $OLD_APP"
        osascript -e 'tell application id "com.aurora.app" to quit' >/dev/null 2>&1 || true
        pkill -x aurora >/dev/null 2>&1 || true
        rm -rf "$OLD_APP"
    fi
done

# Quit a running copy first so the files can be replaced cleanly.
if pgrep -x creblox >/dev/null 2>&1; then
    echo "==> Closing the running CREBLOX"
    osascript -e 'tell application id "com.creblox.app" to quit' >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5 6; do
        pgrep -x creblox >/dev/null 2>&1 || break
        sleep 0.5
    done
    pkill -x creblox >/dev/null 2>&1 || true
fi

rm -rf "$DEST"
ditto "$APP" "$DEST"
printf '%s' "$DEST" > dist/.installed_path

# Remember where this source folder lives so the next update zip can find it.
mkdir -p "$HOME/Library/Application Support/CREBLOX" 2>/dev/null || true
printf '%s' "$PWD" > "$HOME/Library/Application Support/CREBLOX/source-folder" 2>/dev/null || true

echo "==> Installed: $DEST"
echo "    Open it from Launchpad / Spotlight, or drag it to your Dock."
echo "    You only need to run this script again when you update the code."
