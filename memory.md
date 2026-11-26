# memory.md — how CREBLOX is released

This is the written-down memory of the release process for **`Vexa1/Custom-Launcher`** (the
CREBLOX launcher). It exists so that a human or another AI agent can cut a release without
re-deriving any of it. Read this before touching `Cargo.toml`, `release/`, or a GitHub release.

`README.md` and `scripts/package-release-macos.sh` used to call the release contract
`RELEASING.md`; that file was never written. **This file is now that contract.**

- Project: native Rust + eframe/egui macOS launcher (GPL-3.0-only), crate and binary `creblox`.
- Branch that ships: `main`. Releases are tagged `v<version>` directly on `main`.
- Current version at the time of writing: **0.4.29** (`Cargo.toml:3`, `README.md:3`).

---

## 1. The shape of a CREBLOX release (read this first)

There are **no GitHub release assets**. Every live release has `"assets": []`. What actually ships is:

| Piece | Where it lives | Who reads it |
|---|---|---|
| `release/CREBLOX-<ver>-update.zip` | committed to this repo, served over raw | The **in-app updater** (`src/updater.rs`) and the "quick update" path |
| `release/CREBLOX-<ver>-full.zip` | committed to this repo, served over raw | People doing a fresh install |
| Release body | GitHub release for tag `v<ver>` | The updater **parses it**; users read it |

Download URLs therefore look like this and must be used verbatim:

```
https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-<ver>-update.zip
https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-<ver>-full.zip
```

"Uploading a release" = **commit the two zips to `release/`, push `main`, then create/update the
GitHub release whose body links them and prints their byte size and SHA-256.** The zips are
tracked on purpose — `release/*.zip` is deliberately *not* in `.gitignore`, and the comment at the
bottom of `.gitignore` says so. Do not "fix" that.

Because the downloads are raw URLs on `main`, **the pushed commit is what users download**. Never
force-push, amend, or delete a published `release/*.zip`; if a bad archive shipped, publish a new
version instead.

---

## 2. Golden rules

1. **Never touch user data.** Everything CREBLOX owns lives in
   `~/Library/Application Support/CREBLOX/` (`session.dat`, `session.key`, `accounts.dat`,
   `settings.json`, `fastflags.json`, `playtime.json`, `log.txt`). No release step may read, move,
   delete or migrate it. The 0.4.15 folder rename is the only sanctioned migration and it is
   one-way.
2. **Version == tag.** `Cargo.toml` `version` and the release tag must match exactly
   (`0.4.29` ↔ `v0.4.29`). The updater only offers releases whose semver is newer
   (`src/updater.rs:157`), and the in-app installer compares `Cargo.toml` versions.
3. **Never re-upload or move a published archive.** Publish a new version instead.
4. **The update link comes first.** The updater takes the *first* `.zip` link it finds in the body
   (`src/updater.rs:186`). See §5 for the exact selection rules.
5. **Never publish an archive from a check-only tree** — see §7, the real link gate exists because
   0.4.25 shipped an undefined symbol.
6. **No secrets, ever.** No `.ROBLOSECURITY` values, `rbx-authentication-ticket`, `session.key`,
   `accounts.dat`, private-server links, IP addresses, personal paths (`/Users/...`) or tokens in
   the repo, the archives or the notes. This was audited and cleaned; keep it that way — details
   in §9.
7. **Only the newest release is `make_latest: true`.** Older releases are updated with
   `make_latest: "false"` so the updater can never latch onto an old tag.

---

## 3. Release runbook (step by step)

Run from the repository root.

### 3.1 Bump the version

```bash
# Cargo.toml:3  version = "0.4.29"  ->  "0.4.30"
# README.md:3   **Version:** 0.4.29 ->  0.4.30
cargo check            # refreshes the creblox entry in Cargo.lock
git diff --stat Cargo.toml Cargo.lock README.md
```

`Cargo.lock` must be committed with the bump: `scripts/package-release.sh` copies `Cargo.lock`
into both archives, and `Update CREBLOX.command` rebuilds from it.

### 3.2 Write the release notes

Follow the format contract in §5. Keep the `### Downloads` block last, update archive first, each
line carrying its size and SHA-256.

### 3.3 Package the archives

On a **Mac with Xcode Command Line Tools** (the normal path):

```bash
scripts/package-release.sh
```

It refuses to run anywhere else, because it first calls `scripts/verify-release-link.sh`, which
runs a real `cargo build --release --target x86_64-apple-darwin` (linker included) and exits
non-zero on failure. It writes the two zips into the **repository root**, prints both SHA-256s and
byte sizes, and reminds you the update URL goes first.

On a **host with no macOS SDK** (Linux CI, cloud sandbox) the link cannot be proven. The only
sanctioned way past the gate is:

```bash
CREBLOX_SKIP_LINK_GATE=1 scripts/package-release.sh
```

That prints a loud warning on stderr every time: *"the x86_64 Catalina link was NOT verified"*.
Archives made this way are **source-only and unverified** — they must be rebuilt once on macOS
before being published, or the release must state plainly that it was not link-verified. Never
remove the warning, never make the gate silent, and never present a skipped gate as verified.

What the script produces (`scripts/package-release.sh`):

| Archive | Contents | Must **not** contain |
|---|---|---|
| `creblox-<ver>-macos-full.zip` → folder `CREBLOX <ver>` | `assets/ scripts/ src/ Cargo.toml Cargo.lock build.rs README.md LICENSE .gitignore`, both `.command` files, `creblox-update-remove.txt` | build output, `dist/`, `.DS_Store` |
| `creblox-update-<ver>-macos.zip` → folder `CREBLOX <ver> update` | `scripts/ src/ Cargo.toml Cargo.lock`, `Update CREBLOX.command`, `creblox-update-remove.txt` | `build.rs`, `assets/` |

`build.rs` is the marker `Update CREBLOX.command` uses to tell a complete project from an overlay,
so an update must never carry it — the script hard-fails if it does.

Rename to the published convention and move into `release/`:

```bash
mv creblox-<ver>-macos-full.zip   release/CREBLOX-<ver>-full.zip
mv creblox-update-<ver>-macos.zip release/CREBLOX-<ver>-update.zip
shasum -a 256 release/CREBLOX-<ver>-{update,full}.zip
wc -c        release/CREBLOX-<ver>-{update,full}.zip
```

The SHA-256 and the size printed here are the two numbers that go into the notes. Copy them
exactly; the updater verifies the downloaded archive against the hash on the same line.

### 3.4 Commit and push (this is the actual "upload")

```bash
git add Cargo.toml Cargo.lock README.md release/CREBLOX-<ver>-update.zip release/CREBLOX-<ver>-full.zip
git commit -m "release: <ver> archives"
```

Commit-message style already in the history: `release: 0.4.28 archives`,
`release: 0.4.28 - macOS sessions keep their server, stats on by default`,
`release: gate packaging on a real x86_64 Catalina link`.

Pushing is what makes the raw URLs live. **In the cloud sandbox, plain `git push` fails auth** —
use the `cloud_git` push operation, and fetch/pull with `cloud_git` too.

```bash
# sanity before pushing: nothing sensitive, nothing large and unexpected
git status --short
git ls-files -s release | wc -l
```

### 3.5 Create the GitHub release

Tag `v<ver>`, target `main`, title `CREBLOX <ver> — <short human summary>`.

Via the GitHub integration tool (`GITHUB_UPDATE_A_RELEASE` updates an existing one; a create call
makes the tag):

- `owner: Vexa1`, `repo: Custom-Launcher`
- `tag_name: v<ver>`, `target_commitish: main`
- `name: CREBLOX <ver> — …`
- `body:` the notes from §3.2
- `make_latest: "true"` **only for this new release**

Notes on the tooling:

- Up to 8 release updates per request; batch them.
- `GITHUB_UPLOAD_RELEASE_ASSET` rejects local paths ("file key does not belong to this project or
  session"), and this project does not use release assets anyway. Do not try to attach the zips to
  the release; link them from `release/` on `main` as in §1.
- Markdown links inside the body are fine — the updater strips trailing punctuation.

### 3.6 Verify the release (do not skip)

```bash
V=0.4.30
# 1. the raw downloads resolve, with the exact byte size you printed into the notes
for k in update full; do
  curl -s -o /dev/null -w "%{http_code} %{size_download}  $k\n" -L \
    "https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-$V-$k.zip"
done

# 2. the newest release is the one you just made, and it is not a draft
curl -s https://api.github.com/repos/Vexa1/Custom-Launcher/releases/latest \
  | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['tag_name'],d['draft'],repr(d['name']))"

# 3. an in-app update would pick the small archive and the right hash
curl -s https://api.github.com/repos/Vexa1/Custom-Launcher/releases/latest \
  | python3 -c "
import json,sys,re
b=json.load(sys.stdin)['body']
u=next(t.strip('()') for t in b.split() if t.startswith('https://') and '.zip' in t)
print('first zip:', u)
print('hash on that line:', re.search(r'[0-9a-f]{64}', b.split(u)[1].split(chr(10))[0]).group(0))"
```

The last check must show `...-update.zip` first and the hash printed for *that* file.

Also confirm the release is public: the repository `Vexa1/Custom-Launcher` is **public**
(flipped during the audit), so anonymous `curl`/raw downloads must return `200` without a token.
If they 404, the repo went private again and every in-app update silently breaks.

---

## 4. Where the moving parts are

| Path | Role |
|---|---|
| `Cargo.toml:3` | Version — must equal the tag |
| `README.md:3` | Version line shown to users |
| `scripts/package-release.sh` | Builds both archives; link gate; prints sizes + hashes |
| `scripts/verify-release-link.sh` | Fail-closed real Catalina link (`cc`/`ld`), x86_64 slice check |
| `scripts/package-release-macos.sh` | Optional: packages `dist/CREBLOX.app` as `dist/creblox-<ver>-macos-app.zip` with `ditto` |
| `scripts/build-macos-app.sh` | Builds `dist/CREBLOX.app`; icon, `Info.plist`, `LSMinimumSystemVersion 10.15` |
| `scripts/update-macos.sh` | The build-and-install-over-the-old-copy path used by `Update CREBLOX.command` |
| `Update CREBLOX.command` | Finds the user's source folder (pointer in `~/Library/Application Support/CREBLOX/source-folder`), copies changed files, honours `creblox-update-remove.txt`, rebuilds |
| `Build CREBLOX.command` | Fresh install from the full archive |
| `creblox-update-remove.txt` | One repo-relative path per line: files an update deletes from the user's folder |
| `src/updater.rs` | Release parsing, archive selection, SHA-256 verification, installer launch |
| `release/*.zip` | Published archives, tracked on purpose |

---

## 5. Release-notes contract (the updater parses these — do not improvise)

Format, in this order:

```markdown
<one sentence: what this release does for the user>

### Added        (omit when empty)
- …
### Fixed
- …
### Improved
- …
### Downloads
- **Update only — used by the in-app updater**: [CREBLOX-<ver>-update.zip](https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-<ver>-update.zip) — 346,994 B, `8514633084a63f5cfe6b315b1fc9cd6e58afeebe985afe2e6500b9d94484b5a8`
- **Full — fresh install**: [CREBLOX-<ver>-full.zip](https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-<ver>-full.zip) — 677,777 B, `f18200d1d8a2cbbfdfda5c993a1703e5720a44770c896271a1c6c20674494f2c`

<short manual install/update instruction, ending with: user data in ~/Library/Application Support/CREBLOX is never touched>
```

How the parser reads it (`src/updater.rs`):

- `find_zip_url` (`:186`): first token containing `https://` whose path ends in `.zip`, trailing
  `) ] } > . , ; \` " '` stripped. It **prefers a filename containing `update`** (and not `full`),
  otherwise the first zip found. Notes may use bare URLs or Markdown links.
- `find_sha256_for` (`:223`): the 64-hex token **on the same line as the chosen URL**, else the
  first 64-hex token after it, else the first in the body. This is why the hash must sit on the
  download's own line.
- `parse_github_release_update` (`:231`): drafts ignored; tag must be semver-newer than the running
  version; body is the notes; `assets` are scanned for a hotfix JSON only.
- `install_update_zip` (`:636`) / `find_update_root` (`:567`): the extracted folder is located by
  its contents (a `Cargo.toml` plus `scripts/update-macos.sh`), not by an assumed folder name.

Writing rules:

- User-facing, past tense, no internals: no file:line, no "I/we", no "as requested", no apology,
  no "verified here / not compiled here", no test-run narration.
- Say what changed *for the user*; keep at most one line of root cause where it changes their
  behaviour.
- Never edit a published hash or URL. Never re-order the downloads above the summary.
- Title shape: `CREBLOX <ver> — <what is different now>`. All 31 releases follow this.
- Releases older than the newest get `make_latest: false`; their bodies are historical, not
  rewritten on every release.

---

## 6. macOS / Catalina constraints

- **Target: macOS 10.15 Catalina, Intel.** `x86_64-apple-darwin`, `MACOSX_DEPLOYMENT_TARGET=10.15`.
  The app must contain an x86_64 slice; an Apple Silicon host build is arm64-only even when the
  cross target links, which is why `package-release-macos.sh` re-checks `lipo -archs` on the
  packaged binary.
- **OpenGL renderer (`glow`), not wgpu.** `eframe` is pinned with `default-features = false` +
  `glow` deliberately; do not add wgpu "for speed".
- **`-C target-cpu=native`** is used for local builds (`build-macos-app.sh`); set
  `CREBLOX_PORTABLE=1` for a generic binary. The gate script unsets `RUSTFLAGS` before its probe
  build because `native` is invalid for a cross target.
- `cargo check`/`clippy` never run the linker. A check-only tree can hide a missing symbol
  (0.4.25: `_dispatch_get_main_queue`, a C macro used as a function). That is the entire reason
  `verify-release-link.sh` exists — never weaken it.
- If you ever attach the `.app` bundle itself as an asset, it must be created with
  `ditto -c -k --sequesterRsrc --keepParent`; a Finder/`zip` archive drops resource forks and
  xattrs and breaks the ad-hoc signature.

Typical cross-check command when no Mac is available (type-check only, **not** a release proof):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CC_x86_64_apple_darwin=/tmp/cc-darwin \
       AR_x86_64_apple_darwin="/tmp/zig-linux-x86_64-0.13.0/zig ar" \
       CFLAGS_x86_64_apple_darwin="-I/tmp/cc-include" \
       CARGO_TARGET_DIR=/tmp/tgt MACOSX_DEPLOYMENT_TARGET=10.15
cargo check --target x86_64-apple-darwin --all-targets
cargo clippy --target x86_64-apple-darwin --all-targets
```

---

## 7. Security and privacy rules for releases (audited 2026-11-26)

The repository was audited end to end; it came back clean and must stay that way.

- **Never** commit or print: `.ROBLOSECURITY` values, `rbx-authentication-ticket` values,
  `session.dat`/`session.key`, `accounts.dat`, private-server link codes, GitHub/AWS/OpenAI
  tokens, private keys, `.pem`/`.p12`, Discord webhooks, or `/Users/<name>` paths.
  The cookie/ticket *names* may appear in code (`src/api.rs`, `src/avatar_3d.rs:436`,
  `src/roblox_process.rs:1445`) — the values may not.
- **`log.txt` must never contain an IP address, cookie or ticket.** It is regularly pasted into bug
  reports. Two lines were fixed for this: `src/game_overlay.rs` (~`:919`, logs
  `job … place …`) and `src/roblox_process.rs` (~`:1304`, logs `server place …`). Do not
  reintroduce `s.ip`/`s.server_ip` into a log format string.
- `src/auth.rs:125` is the only writer of `log.txt` (400 KB rotation). Keep it free of secrets.
- `.gitignore` blocks `*.log`, `*.dat`, `*.key`, `session.dat`, `settings.json`,
  `fastflags.json`, `*.pem`, `*.p12`, `.env*`, `dist/`, OS/editor artifacts and the local scratch
  paths (`/files/`, `/work-*/`, `/portcheck.sh`, `/freebuff-saved-files.zip`, `/*.patch`).
  `release/*.zip` stays tracked. Verify with:
  `git check-ignore -v release/CREBLOX-<ver>-full.zip` (must print nothing).
- History is clean: no credential or personal path was ever committed on the available history, so
  **no `git filter-repo`/BFG run is needed**. The clone used for the audit had 27 commits; if you
  need a full-history claim, clone the real thing with full depth first.
`freebuff-saved-files.zip` and `creblox-0.4.18.patch` were untracked on purpose and are now
  ignored local files: still on disk for whoever has the checkout, still reachable in history,
  but no longer part of the tree. The zip is a 3.6 MB snapshot of the predecessor project with
  no sessions, settings, accounts, cookies or user paths; the other three `creblox-*.patch`
  files were never tracked. Never `git add -f` any of them back.

---

## 8. Known traps and oddities

- **`RELEASING.md` does not exist** although `README.md:699` and `scripts/package-release-macos.sh`
  referenced it. This file replaces it. If you see another reference, point it here.
- **0.4.8–0.4.17 archives are hosted externally** on `files.catbox.moe`. Those links must stay
  exactly as published; they are historical notes and not reproducible from this repo.
- **0.4.0 and 0.4.0-full are two separate releases.** `v0.4.0` is the rename release (update
  package, SHA `3a31e7b6…`, data migration from `Aurora`); `v0.4.0-full` is the complete project.
  Both are historical and must keep `make_latest: false`.
- **The 0.4.14–0.4.17 bodies no longer contain the old "private repository / add a read-only
  token" instructions** — the repository is public. Do not reintroduce token setup steps.
- `release/` holds every archive from 0.4.18 onwards, so the repo grows ~1 MB per release. That is
  accepted; do not prune published archives.
- `Update CREBLOX.command` treats a folder without `build.rs` as an overlay. If an update ever
  ships `build.rs` it will be mistaken for a full project and the in-place update misbehaves.
- The dormant in-app update subsystem in `src/updater.rs` is live again as of 0.4.28/0.4.29 — it
  picks the compact archive. Treat changes there as release-critical and re-run the §3.6 parser
  check afterwards.

---

## 9. Agent-specific notes (cloud / other chatbots)

- GitHub is reachable through integration tools: `GITHUB_GET_A_REPOSITORY`,
  `GITHUB_UPDATE_A_REPOSITORY`, `GITHUB_UPDATE_A_RELEASE` (≤8 per request),
  `GITHUB_DELETE_A_RELEASE`, `GITHUB_UPLOAD_RELEASE_ASSET` (unusable here — rejects local paths).
  Release ids differ per release; fetch them instead of remembering them:
  `curl -s "https://api.github.com/repos/Vexa1/Custom-Launcher/releases?per_page=100"`.
- Pushing: `cloud_git` (operation `push`, branch `main`) in the Freebuff cloud sandbox; plain
  `git push`/`git fetch` there fails authentication. Never force-push; never rewrite published
  history.
- No macOS SDK in the cloud sandbox: you can `cargo check`/`clippy` for `x86_64-apple-darwin`
  (zig as `cc`/`ar`, see §6) but you cannot link, run, or screenshot the app. Say so plainly
  instead of implying the app was tested.
- Release work must end with the §3.6 checks actually run, and with the exact commands and their
  output reported — a release that "looks right" is not a verified release.

---

## 10. One-page checklist

```
[ ] Cargo.toml version bumped; Cargo.lock refreshed; README.md:3 bumped
[ ] notes written: summary + Added/Fixed/Improved + Downloads (update first, size + SHA on its line)
[ ] archives built; link gate NOT skipped (or explicitly marked unverified)
[ ] zips moved to release/CREBLOX-<ver>-{update,full}.zip; hashes and sizes recorded
[ ] committed "release: <ver> archives" and pushed to main via cloud_git
[ ] GitHub release v<ver> created on main, make_latest true only for this one
[ ] both raw URLs return 200 with the exact byte sizes
[ ] /releases/latest returns v<ver>, not draft
[ ] first zip in the body is the update archive, hash on the same line matches
[ ] older releases untouched except make_latest: false
[ ] no secret, no IP, no personal path in the tree or the notes
```
