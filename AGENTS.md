Do not read the same file more than once per task unless it has been modified. If a file is too large or the fix is unclear after one read, stop and ask me which function to look at instead of re-reading

# Automated VM verification (zero release overhead)

Before finishing any change, run `scripts/verify.sh` and fix what it reports:

- `cargo test --locked --manifest-path vm-tests/Cargo.toml` — the real `src/party.rs` and
  `src/server_hop.rs`, compiled for this host against the stand-ins in `vm-tests/`, plus the
  loopback end-to-end tests in `vm-tests/tests/`.
- `cargo check --locked --target x86_64-apple-darwin` — the full GUI crate for the release
  target, including all `#[cfg(test)]` and `#[cfg(macos)]` code.

When a change touches logic that can run without a window (party, server hop, log parsing,
update release parsing, ranking, codecs), add or update a test in `vm-tests/tests/` or its
`#[cfg(test)]` block in the same change. `cargo test --locked` on the GUI crate only runs on
macOS; in this Linux VM it cannot compile, so do not treat a host `cargo test` failure there
as a regression in the app.

Release builds stay clean by construction: `vm-tests/` is a separate package with its own
`[workspace]` that `creblox` does not depend on, all test code sits in `#[cfg(test)]` or
`vm-tests/tests/`, and `scripts/package-release.sh` copies an explicit file list that excludes
it. Never add runtime test flags, background test threads or debug logging to shipping code to
make a check pass.
