## Quick update (existing installs)

- **CREBLOX-0.5.71-update.zip** — 579725 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.71-update.zip — SHA-256 `3b19cdec3575e98de4ff95974599f863adec6d472733002f8a5439c785dc95c2`

## Full build (fresh installs)

- **CREBLOX-0.5.71-full.zip** — 925036 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.71-full.zip — SHA-256 `a0b640caba2aab50ecbbdab0bde46b23a6a5b4ba068c9e4dd4a84749becab692`

## What changed

- **The macOS link check now targets the release CPU.** `verify-macos` installs the `x86_64-apple-darwin` target and runs `cargo build --release --locked --target x86_64-apple-darwin` on `macos-latest`, so Apple's `ld` links the Intel binary users actually install (Catalina 10.15+) instead of the runner's arm64 host build.
- **Release archives are attached automatically.** A new `attach-release-assets` workflow uploads the committed `release/CREBLOX-<version>-{update,full}.zip` files to the matching GitHub release when it is published, skips files already attached, and can backfill a tag manually. On every push to `main` it also fills in the latest release if it has no assets, which gave v0.5.70 its two archives.
- No launcher behaviour changed in this release.

