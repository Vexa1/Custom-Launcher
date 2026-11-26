#!/bin/bash
# Double-click this file to install an CREBLOX update.
#
# This zip contains only the files that changed. Double-clicking this file
# finds the CREBLOX folder you already have, copies the new files into it,
# rebuilds, and replaces the installed app. Settings and logins are kept.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SUPPORT="$HOME/Library/Application Support/CREBLOX"
SAVED_POINTER="$SUPPORT/source-folder"
# Where 0.3.x remembered your project folder before the rename.
LEGACY_POINTER="$HOME/Library/Application Support/Aurora/source-folder"

pause_and_exit() {
    echo
    read -n 1 -s -r -p "Press any key to close this window..." || true
    echo
    exit "${1:-0}"
}

is_creblox_source() {
    [[ -n "${1:-}" ]] || return 1
    [[ -f "$1/Cargo.toml" ]] || return 1
    # Accept both names: you are updating a folder that still says aurora.
    grep -Eq '^name *= *"(creblox|aurora)"' "$1/Cargo.toml" 2>/dev/null
}

file_mtime() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        stat -f '%m' "$1" 2>/dev/null || echo 0
    else
        stat -c '%Y' "$1" 2>/dev/null || echo 0
    fi
}

find_sources() {
    local root
    for root in "$HOME/Desktop" "$HOME/Documents" "$HOME/Downloads" "$HOME/Developer"; do
        [[ -d "$root" ]] || continue
        find "$root" -maxdepth 3 -name Cargo.toml -type f 2>/dev/null
    done
}

echo "==> CREBLOX update"

TARGET=""
# An update folder also carries Cargo.toml, so only treat this folder as the
# project itself when it has build.rs (which updates never ship).
if is_creblox_source "$HERE" && [[ -f "$HERE/build.rs" ]]; then
    TARGET="$HERE"
elif [[ -f "$SAVED_POINTER" ]] && is_creblox_source "$(cat "$SAVED_POINTER" 2>/dev/null)"; then
    TARGET="$(cat "$SAVED_POINTER")"
elif [[ -f "$LEGACY_POINTER" ]] && is_creblox_source "$(cat "$LEGACY_POINTER" 2>/dev/null)"; then
    TARGET="$(cat "$LEGACY_POINTER")"
else
    CANDIDATES="$(find_sources | while IFS= read -r file; do
        dir="$(dirname "$file")"
        if [[ -f "$dir/build.rs" ]] && grep -Eq '^name *= *"(creblox|aurora)"' "$file" 2>/dev/null; then
            printf '%s\n' "$dir"
        fi
    done | awk '!seen[$0]++')"

    if [[ -n "$CANDIDATES" ]]; then
        # If several copies exist, use the most recently touched one.
        TARGET="$(printf '%s\n' "$CANDIDATES" | while IFS= read -r dir; do
            printf '%s\t%s\n' "$(file_mtime "$dir/Cargo.toml")" "$dir"
        done | sort -rn | head -1 | cut -f2)"
    fi
fi

if [[ -z "$TARGET" ]]; then
    # Nothing installed to update in place. This download is a complete copy of
    # the project, so build it right here instead of stopping dead.
    echo "    No existing CREBLOX folder found - installing from this download."
    TARGET="$HERE"
fi

echo "    CREBLOX folder: $TARGET"

# --- Copy only the changed files over the existing copy ------------------
if [[ "$TARGET" != "$HERE" ]]; then
    echo "==> Copying updated files"
    (
        cd "$HERE" || exit 1
        find . -type f \
            ! -name '.DS_Store' \
            ! -name 'creblox-update-remove.txt' \
            -print | while IFS= read -r rel; do
                rel="${rel#./}"
                mkdir -p "$TARGET/$(dirname "$rel")"
                cp -f "$HERE/$rel" "$TARGET/$rel"
            done
    )
fi

# --- Drop files this version no longer uses ------------------------------
if [[ -f "$HERE/creblox-update-remove.txt" ]]; then
    while IFS= read -r path; do
        [[ -n "$path" ]] || continue
        if [[ -e "$TARGET/$path" ]]; then
            echo "    removing $path"
            rm -rf "$TARGET/$path"
        fi
    done < "$HERE/creblox-update-remove.txt"
fi

# Remember the folder so later updates can find it without searching.
mkdir -p "$SUPPORT" 2>/dev/null || true
printf '%s' "$TARGET" > "$SAVED_POINTER" 2>/dev/null || true

# --- Build and install ---------------------------------------------------
export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:$PATH"

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

cd "$TARGET" || pause_and_exit 1
bash scripts/update-macos.sh || pause_and_exit 1
pause_and_exit 0
