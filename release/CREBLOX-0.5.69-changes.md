## What changed

- **Every macOS target is compile-checked before a release can be packaged.** The 0.5.63
  failure (a download that then died in the local build) came from packaging with
  `CREBLOX_SKIP_LINK_GATE=1` and only a plain `cargo check`. `scripts/verify.sh` and
  `scripts/package-release.sh` now run
  `cargo check --locked --target x86_64-apple-darwin --all-targets`, so the app, its test
  target and every extra binary are checked for the release OS; the skip flag only skips the
  final Mach-O link. At 0.5.68 the check is already clean, so there was no leftover 0.5.63
  error to repair.
- **Release notes are real markdown now.** The Launcher Updates card renders `##` headings,
  bullets, `**bold**`, inline code and `[label](url)` links as formatted text instead of
  dumping the raw body. `.zip` URLs, checksums and `SHA-256` lines move into a collapsible
  **Details** drawer, with their markers stripped.
- **Build failures are readable in one click.** When the install error is exit 101 / "build
  failed", a **View Error Log** button appears next to **Download in Browser** and expands the
  captured compiler output immediately. The installer log is now kept after a failed install
  (previously it was discarded with the progress state), so the failing command, file, line and
  rustc message are all there; the message itself now points at the error log.
- New `vm-tests/tests/update_card.rs` covers headings/bullets, inline spans, the Details split,
  unclosed markers, and that only exit 101 counts as a build failure.
- New `settings-updates` screenshot renders the formatted notes, the Details drawer and the
  expanded compiler log from the shipping code. The preview harness now aliases the app's bold
  font family, which previously drew section headings as tofu boxes.
