#!/usr/bin/env bash
# Write the GitHub release body for the current (or given) version from the packaged archives:
#
#   release/CREBLOX-<version>-notes.md
#
# The downloads section is generated here, never typed by hand: each archive's size and SHA-256
# are read from the zip itself, and each archive's link and checksum share one line. That is the
# format every CREBLOX updater can parse - 0.5.39 printed the checksum above the wrapped link and
# in-app updates failed SHA-256 verification.
#
# An optional hand-written `release/CREBLOX-<version>-changes.md` is appended as the "What
# changed" section. Then `vm-tests/tests/release_notes_generated.rs` re-reads every notes file
# and checks each download against its archive, so a hash can never drift from the bytes.
#
# Usage: scripts/release-notes.sh [version]
#        CREBLOX_SKIP_NOTES_CHECK=1 scripts/release-notes.sh   # write without running the test
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
[[ -n "$VERSION" ]] || { echo "Could not read version from Cargo.toml" >&2; exit 1; }

UPDATE="release/CREBLOX-${VERSION}-update.zip"
FULL="release/CREBLOX-${VERSION}-full.zip"
NOTES="release/CREBLOX-${VERSION}-notes.md"
CHANGES="release/CREBLOX-${VERSION}-changes.md"
BASE="https://github.com/Vexa1/Custom-Launcher/raw/main/release"

for zip in "$UPDATE" "$FULL"; do
    [[ -f "$zip" ]] || { echo "Missing $zip - run scripts/package-release.sh first" >&2; exit 1; }
done

update_sha="$(shasum -a 256 "$UPDATE" | cut -d' ' -f1)"
full_sha="$(shasum -a 256 "$FULL" | cut -d' ' -f1)"
update_size="$(wc -c < "$UPDATE" | tr -d ' ')"
full_size="$(wc -c < "$FULL" | tr -d ' ')"

{
    echo "## Quick update (existing installs)"
    echo
    echo "- **CREBLOX-${VERSION}-update.zip** — ${update_size} bytes — ${BASE}/CREBLOX-${VERSION}-update.zip — SHA-256 \`${update_sha}\`"
    echo
    echo "## Full build (fresh installs)"
    echo
    echo "- **CREBLOX-${VERSION}-full.zip** — ${full_size} bytes — ${BASE}/CREBLOX-${VERSION}-full.zip — SHA-256 \`${full_sha}\`"
    echo
    if [[ -f "$CHANGES" ]]; then
        cat "$CHANGES"
        echo
    else
        printf '## What changed\n\n_Write `%s` before publishing: this section is hand-written._\n' "$CHANGES"
        echo
    fi
} > "$NOTES"

echo "Wrote $NOTES"
echo "  update: $update_size bytes  $update_sha"
echo "  full:   $full_size bytes  $full_sha"

if [[ "${CREBLOX_SKIP_NOTES_CHECK:-0}" == "1" ]]; then
    echo "Skipping the notes self-check (CREBLOX_SKIP_NOTES_CHECK=1)." >&2
else
    # The check parses the notes exactly as the app does and compares every printed hash and
    # size with the archive on disk.
    cargo test --locked --manifest-path vm-tests/Cargo.toml --test release_notes_generated
fi
