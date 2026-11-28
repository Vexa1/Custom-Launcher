## What changed

- **The macOS link check now targets the release CPU.** `verify-macos` installs the `x86_64-apple-darwin` target and runs `cargo build --release --locked --target x86_64-apple-darwin` on `macos-latest`, so Apple's `ld` links the Intel binary users actually install (Catalina 10.15+) instead of the runner's arm64 host build.
- **Release archives are attached automatically.** A new `attach-release-assets` workflow uploads the committed `release/CREBLOX-<version>-{update,full}.zip` files to the matching GitHub release when it is published, skips files already attached, and can backfill a tag manually. On every push to `main` it also fills in the latest release if it has no assets, which gave v0.5.70 its two archives.
- No launcher behaviour changed in this release.
