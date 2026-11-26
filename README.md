# CREBLOX — custom Roblox launcher in pure Rust

**Repository:** `Vexa1/Custom-Launcher` · **Version:** 0.4.19
**Target platform:** macOS **10.15.7 (Catalina)** and newer, x86_64 Intel first-class (Apple Silicon via a universal build).
**Windows** builds are supported from the same source tree.

> ### Names and identifiers
>
> The product is **CREBLOX**, and since 0.4.15 the machine identifiers say so too: crate and
> binary `creblox`, bundle id `com.creblox.app`, app bundle `CREBLOX.app`, data folder
> `~/Library/Application Support/CREBLOX` (Windows: `%LOCALAPPDATA%\creblox`).
>
> Installs from before that rename kept their data under `Aurora` / `aurora`.
> `auth::data_dir` migrates those folders once, newest first, so saved sessions, accounts,
> FastFlags and playtime survive. The migration is one-way — do not re-introduce the old
> paths, and do not expect a second migration to run.
>
> The *file* names inside the data folder (`settings.json`, `accounts.dat`, …) are still a
> frozen contract; see *§7 invariant 6*.

---

## 1. Overview

CREBLOX is a native desktop launcher for Roblox that replaces the website with a fast,
keyboard-and-trackpad-friendly shell: browse charts and search, see friends' presence, launch
games into the installed Roblox client, tune the client's FastFlags, and keep a small in-game
HUD (server region, CPU/RAM/ping, active flags, crosshair) on top of the game window.

It is written in **pure Rust** with no Electron/WebView, no JavaScript runtime and no C++
application layer:

* one process, no bundled Chromium, no DOM;
* immediate-mode UI (`egui`) that redraws at the display's refresh rate;
* the Roblox client is never modified, re-signed or re-downloaded — CREBLOX speaks the same
  `roblox-player:` deep link the website uses and hands it to the **installed** Roblox app
  through LaunchServices;
* aggressive resource discipline (RAM-budgeted texture cache, background threads for every
  network call, an idle animation pacer when the window is hidden).

The point of the Catalina target is that a 2012–2015 MacBook must still get a working, smooth
launcher: OpenGL instead of Metal-only code paths, no Big Sur+ AppKit selectors, no
dependencies that link against newer `libSystem` symbols.

### Compatibility matrix

| | Status |
|---|---|
| macOS 10.15.7 Catalina (Intel) | **Primary target.** Deployment target pinned to `10.15`. |
| macOS 11–15 (Intel / Apple Silicon) | Supported (universal `.app`). |
| Windows 10/11 x64 | Supported (separate code paths in `tray_windows.rs`, `roblox_process.rs`). |
| Linux | **Not supported.** `tray`, `roblox_process` and font paths are `cfg(windows)`/`cfg(macos)` only. |

---

## 2. Feature reference

Everything listed here exists in the code today. File references point at the implementation.

### 2.1 Roblox sign-in and accounts

| Feature | How it works |
|---|---|
| **Official Quick Login** (`ui/page_login.rs`, `auth.rs`) | Requests a one-time login code from Roblox's official quick-login endpoint, renders it as a QR code and as a copyable code, polls for completion, then stores the `.ROBLOSECURITY` cookie. No password is ever handled or stored. |
| **Encrypted session at rest** (`auth.rs:466-535`) | The cookie is encrypted with AES-256-GCM (`aes-gcm`) using a random key in `session.key` (mode `0600`, created with `OpenOptionsExt`). Plaintext cookies never hit the disk. |
| **Multi-account** (`accounts.dat`, `app.rs:1075-1170`) | Every signed-in account is kept (id, name, display name, headshot, encrypted cookie) so the top-bar switcher can swap without re-authenticating. Removing an account only drops its stored record. |
| **Session refresh** (`sync_auth_session`) | Periodically re-validates the cookie and refreshes profile/Robux, silently recovering from expired sessions with a "session expired" banner instead of a crash. |
| **Account menu** (`app.rs:3960-4130`) | Chevron menu with account list (current account marked), *Add account*, Profile, Avatar, Settings, theme toggle, Sign out. |

### 2.2 Game discovery

| Feature | How it works |
|---|---|
| **Home feed** (`ui/page_home.rs`) | Friends strip (presence dot, "Playing <game>", right-click → *Join game*), first Explore sort as a 6-across grid, "Continue" row from recent games, Favorites row, then every remaining sort as a horizontal row. |
| **Charts** (`ui/page_charts.rs`) | Topic chips over a full-width grid of the selected sort, with an empty state and a manual refresh control. |
| **Search** (`ui/page_search.rs`, `api.rs`) | Accepts a game name, a bare Place ID or a pasted Roblox game URL; resolves whichever it is and shows results as cards. |
| **Game details** (`ui/page_game.rs`) | Media carousel (image thumbnails plus real **animated video previews**), creator + verified badge, maturity, rating and player counts, Play / Favorite / vote, private-server entry, server list with sorting (ping/players), region lookup, game passes, badges and social tabs. |
| **Video previews** (`video_preview.rs`, `app.rs:2646-2746`) | Game previews are VP8-in-WebM; they are demuxed with `matroska-demuxer`, decoded with `vp8-decoder` on a worker thread, downscaled to RGBA and streamed into an egui texture on hover. Includes a monotonic frame clock and looping. |
| **Thumbnails + RAM budget** (`app.rs:3234-3300`) | Thumbnails are requested in batches on background threads and the texture cache is trimmed against a configurable session budget (`enforce_ram_cache_budget`). |
| **Manual game link** (`app.rs:2963-3160`) | "+ Game URL" opens a modal that resolves a pasted link/Place ID and offers *Open*, *Play* or *Favorite*. |
| **Playtime** (`playtime.rs`) | Per-universe total seconds and last-played timestamp, added once when a session ends (zero cost while playing). |

### 2.3 Launching Roblox

| Feature | How it works |
|---|---|
| **Authenticated launch** (`roblox_process.rs:1100-1175`) | `POST https://auth.roblox.com/v1/authentication-ticket` with CSRF handling → read `rbx-authentication-ticket` → build the `roblox-player:1+launchmode:play+gameinfo:…+placelauncherurl:…` URI → hand it to the Roblox app. Runs on a worker thread; the UI shows "Getting launch ticket…". |
| **Server / private-server join** | `RequestGameJob` and `RequestPrivateGame` PlaceLauncher URLs (`build_place_launcher_url`). |
| **Bundle-aware process hand-off** (`roblox_process.rs:1066-1090`) | Resolves the Roblox bundle from the player binary that was actually found and runs `/usr/bin/open -a <bundle> <uri>`, so a browser that grabbed the `roblox-player:` scheme cannot intercept the launch, and Roblox installed in `~/Applications` still works. |
| **Install discovery** (`find_player`) | `/Applications/Roblox.app`, `~/Applications/Roblox.app`, then `~/Library/Application Support/Roblox/Versions/*/RobloxPlayer.app` (newest wins). |
| **Deep links** (`main.rs:57-66`) | If CREBLOX is started with a `roblox-player:` argument it launches and exits — this is what makes it usable as a scheme handler. |
| **Session tracking** (`roblox_process.rs:800-1050`) | Watches Roblox's own log to distinguish *tray only*, *playing*, *back in app* and *app only*; drives the overlay lifetime, the "hide launcher during game" behaviour, auto-close and Discord presence. |
| **Auto-close Roblox** | Optional: when you leave a game, the leftover background Roblox tray app is closed (and restored later). |
| **Hide / minimize / close during game** | Three independent settings: minimize on launch, close the launcher window entirely while playing (a headless `--companion` helper reopens it), and return the launcher to the front when you leave. |

### 2.4 FastFlags and graphics tuning

| Feature | How it works |
|---|---|
| **Presets** (`fastflags.rs:146-176`) | *Competitive* (Metal on macOS / D3D11 on Windows, Voxel lighting, no shadows, low-quality textures, no post-FX, no grass, telemetry off) and *Quality* (automatic renderer, Future lighting, telemetry off). |
| **Toggles** | Renderer (Auto / Metal / OpenGL on macOS), lighting mode (Default / Voxel / ShadowMap / Future), shadows, texture quality, post-FX, grass, telemetry, in-game FPS counter, MSAA (1/2/4/8), quality level override (1–21), FPS cap (`DFIntTaskSchedulerTargetFps`). |
| **Custom flags** | A free-form `key → JSON value` map with typed parsing (`true`, `false`, `null`, integers, floats, strings) in the FastFlags editor. |
| **Import / export** (`fastflags.rs`) | Export the effective flags as JSON; import a JSON blob back into the custom map. |
| **Where flags are written** (`fastflags.rs:489-533`) | macOS: `~/Library/Preferences/com.roblox.Roblox/ClientAppSettings.json`, and additionally `RobloxPlayer.app/Contents/MacOS/ClientSettings/ClientAppSettings.json` for older clients. Windows: `ClientSettings/ClientAppSettings.json` in the active version folder. A write that fails on one target is tolerated as long as one succeeds. |
| **Managed keys** (`BUILTIN_MANAGED_KEYS`) | Turning a toggle off *removes* its keys instead of writing `false`, so Roblox's own defaults return. |
| **Apply on launch** (`sync_saved_fastflags_before_launch`) | Flags are re-synced immediately before every launch so editing them in Settings takes effect on the next join without restarting CREBLOX. |

### 2.5 In-game overlay (separate process)

| Feature | How it works |
|---|---|
| **Transparent overlay window** (`main.rs:36-42`, `overlay_window.rs`) | A second copy of the binary runs with `--overlay-window <spec.json>`: borderless, transparent, click-through, always-on-top, non-activating window positioned over the Roblox window (including fullscreen Spaces). |
| **Server panel** (`game_overlay.rs`) | Corner-selectable panel showing server region, IP and job id, with auto-hide duration. |
| **Stats panel** | Live CPU, RAM, ping (ICMP to the game server), session timer, clock and the active FPS cap. |
| **FastFlags panel** | Read-only view of the flags currently applied to the client. |
| **Crosshair** | Style, size, gap, thickness, colour and opacity, drawn with a dark outline for visibility on bright scenes. |
| **Driver** | `game_overlay.rs` writes a JSON spec roughly once a second from a background thread and keeps the overlay process alive, restarting it if it dies mid-session. Works identically from the launcher or from `--companion`. |

### 2.6 Shell, menu bar and integrations

| Feature | How it works |
|---|---|
| **Menu-bar item** (`tray_macos.rs`) | Native `NSStatusItem` via `tray-icon`, drawn as a **template image** so it is black on a light menu bar and white on a dark one. Left click brings the launcher forward; right click gives Open/Hide, the page shortcuts, a live status line, the server line, Copy Job ID, Close Roblox, the three overlay toggles, Discord Rich Presence and Quit. The **update check** entry is dormant. See *System tray and live sync*. |
| **Single instance** (`tray_macos.rs:610-640`) | `.instance.lock` in the data folder; a second launch focuses the existing window instead of starting a rival process. |
| **Launch at login** (`set_startup`) | Writes a login item/LaunchAgent for the resolved bundle path. |
| **Discord Rich Presence** (`discord_rpc.rs`) | A hand-written Discord IPC client (Unix socket / named pipe, 8-byte frame header, `SET_ACTIVITY`), so no `discord-rich-presence` dependency. Shows the game being played; default app id `1005469189907173486`, overridable in Settings. |
| **Notifications** (`ui/notifications.rs`, `auth.rs:730-745`) | Roblox notifications, friend requests and system alerts, with unread badges on the nav rail, top-centre toast popups, a "silent" mode and a `seen_notifications.json` ledger. The page also shows two status cards (Roblox API health, background Roblox processes). |
| **Updates** | Manual, from GitHub Releases: download `CREBLOX-<version>-update.zip` and double-click **`Update CREBLOX.command`**, which rebuilds your source folder in place and keeps settings, logins, playtime and FastFlags. See *Updating an existing install* under building. Nothing in the app contacts the network to look for updates. |
| **Live `hotfix.json`** (`updater.rs`) | Dormant with the rest of the update subsystem: no client fetches it. |
| **Theme packs** (`ui/page_settings.rs:37-110`) | Light, Dark, Mint, Rose, Ocean, Sand, plus fully custom: background (theme default / solid colour / wallpaper with drag-and-drop image and dim overlay), text colour mode, per-surface border colours, corner roundness, and the "Liquid Glass" edition (opacity + highlight), all persisted. |
| **Animations toggle** | Global: `animation_time = 0` in egui plus the page cross-fade is skipped. |
| **Logging** (`auth.rs:55-80`) | `log.txt` in the data folder; the UI can open the folder from Settings. |

---

## 3. Architecture and directory map

```
src/
  main.rs            Entry point + CLI dispatch:
                       --overlay-window <spec>  in-game overlay process
                       --companion              headless session watcher
                       --update-* / quiet apply updater helper modes
                       roblox-player:<uri>      deep-link launch & exit
                     Builds the eframe ViewportBuilder (size, min size, icon,
                     Renderer::Glow) and the App.

  app.rs             The application shell and all state. ~5k lines.
                       * `Creblox` struct: settings, session,
                         caches, page state, worker channels (Msg queue),
                         tray/overlay handles.
                       * `impl eframe::App::ui` — layout order: left nav rail,
                         top bar, status bar, CentralPanel with the page
                         ScrollArea, then modals/toasts, then `Action` handling.
                       * `poll()` drains background worker messages; returns
                         "needs repaint".
                       * `spawn(job)` — the ONLY sanctioned way to do I/O.
                       * Launch helpers, account switching, updates, overlay and
                         tray synchronisation, RAM budget, page transition.

  theme.rs           Design system. `Palette` (semantic colours derived from
                     settings/presets), `apply_theme()` (visuals, corner radii,
                     widget states, animation time), `pal(ui)` thread-local palette
                     cache, `TYPE` typography scale, `SPACE` spacing scale,
                     `NAV_RAIL_WIDTH`/`NAV_ITEM_SIZE`, `on_accent()`,
                     `robux_gold()`, `stat_colors()`, `card_frame()`,
                     `install_fonts()` (macOS/Windows system font chains:
                     SF/Apple Symbols/Arial + Bold/Black families).

  ui/
    mod.rs              module list
    widgets.rs          Shared widgets: game card, wide/icon grid + rows,
                        horizontal-wheel bridge, avatar bubble, thumbnails with
                        video badge, stat row, section headers, refresh control
    menu.rs             Left nav rail item (icon, hover/selected animation,
                        unread badge, silent marker)
    page_home.rs        Home feed
    page_charts.rs      Charts
    page_search.rs      Search results
    page_game.rs        Game detail (media, play, servers, passes, badges)
    page_profile.rs     Own/friend profile, bio editing, stats, badge grids
    page_avatar.rs      Avatar editor: 2D preview + 3D viewer, inventory tabs,
                        body colours, outfits
    page_add_friend.rs  Friend search, requests, followers/following
    page_login.rs       Quick Login card, QR rendering, Rec Room-style backdrop
    page_notifications.rs Notification feed + status cards
    page_settings.rs    Every setting (~2.6k lines): theme, FastFlags editor,
                        performance, overlay, Discord, tray, updates, Roblox
                        install, data folder

  api.rs             All Roblox HTTP: JSON GET/POST with CSRF + cookie, games,
                     sorts, thumbnails, video ids, friends, presence, profiles,
                     avatar catalog/outfits, badges, passes, servers, votes,
                     favourites, notifications, user search, link resolution,
                     and `lookup_server_location`.
  auth.rs            Data dir, settings load/save, logging, AES-GCM cookie
                     storage, saved accounts, notification ledger, HOME paths.
  roblox_process.rs  Roblox discovery, launching, process enumeration (Windows
                     toolhelp/NtQueryInformationProcess, macOS `ps`), Roblox log
                     parsing (session state machine), FPS flag application,
                     roblox-player handler status/registration.
  fastflags.rs       FastFlags model, presets, JSON import/export, ClientAppSettings
                     generation, per-OS sync, `build_client_app_settings`.
  game_overlay.rs    Overlay manager: spec generation, overlay process lifecycle,
                     server region/ping sampling.
  overlay_window.rs  The overlay GUI process itself (transparent click-through
                     viewport, AppKit tweaks through the Objective-C runtime,
                     window-follow logic, crosshair/panel painters).
  companion.rs       `--companion`: headless watcher that runs overlays and
                     reopens the launcher when the game ends.
  updater.rs         Update check, release parsing, SHA-256 verification,
                     download/install (patch & full), update window, hotfix.json,
                     CLI helpers, macOS .app swap and Windows binary swap.
  tray.rs            Thin cross-platform facade over the tray implementations.
  tray_macos.rs      NSStatusItem, menu items, activation, single-instance lock,
                     login item management.
  tray_windows.rs    Win32 tray icon + menu.
  discord_rpc.rs     Discord IPC client (frames, connect/retry, activity payload).
  playtime.rs        playtime.json read/write.
  video_preview.rs   VP8/WebM decode → RGBA frames with downscaling.
  avatar_3d.rs       Avatar 3D loader (OBJ/MTL/textures over HTTPS) and a CPU
                     rasteriser with a real z-buffer, cached per (yaw,pitch,zoom).
  telemetry.rs       RAM sampling, self-imposed RAM/CPU limits, the
                     window-visibility key used by the idle animation pacer.
  types.rs           Data model: Game, Profile, User, Friend, Settings (+ Default),
                     Page/Action/Msg enums, NavIcon, AvatarTab, SocialTab, etc.
  platform.rs        OS-specific UI strings ("menu bar" vs "system tray").
```

**State flow.** There is exactly one mutable owner: the app struct. UI code never performs
I/O; it pushes an `Action`. `app.rs` translates actions into either state changes or
`self.spawn(closure)` jobs. Jobs return a `Msg` over an `mpsc` channel; `poll()` drains the
channel at the top of every frame and requests a repaint. This keeps the frame loop free of
network, disk and process work.

**Rendering.** One glow (OpenGL) context, egui immediate mode. All colour, radius, spacing
and typography comes from `theme.rs`; page code reads a palette via `pal(ui)` and never
hard-codes a colour for a themed surface.

---

### System tray and live sync

The menu-bar item is the launcher's second face: with a game fullscreen it is the only way
to reach CREBLOX without leaving the session.

**Menu contents** — built in `TrayController::new` (`tray_macos.rs:148`):

| Item | Source of truth | Behaviour |
|---|---|---|
| Open CREBLOX / Hide to Menu Bar | `TrayState.window_visible` | `restore_from_tray` (`app.rs:4061`) sends `Visible(true)`, `Minimized(false)` and `Focus`, then calls AppKit `activateIgnoringOtherApps:` and re-raises across 8 frames, so a hidden or occluded window returns over a fullscreen game. |
| Page shortcuts (Home, Profile, Avatar, Notifications, Settings, FastFlags) | `TrayState.signed_in` | Disabled while signed out; each restores the window, then switches `Page`. |
| Live status line | `tray_state_snapshot` (`app.rs:3853`) | `Playing: <game> — @user`, `Signed in: @user`, or `Not signed in`. The game name comes from the same place-Id lookup Discord presence uses. |
| Server line + Copy Job ID | `server_info`, `server_locations` | Region when resolved, `locating…` while the lookup runs; Copy Job ID is disabled until a join id exists. |
| Close Roblox | `TrayState.roblox_running` | Calls `kill_all_roblox_processes` (`roblox_process.rs:932`) — the same function Settings uses. Disabled when no player process is alive. macOS menu only; Windows keeps the action in Settings. |
| Server region / Stats / Crosshair | `settings.overlay_enabled`, `overlay_stats_enabled`, `crosshair_enabled` | Checkmarks mirror Settings; a click writes `settings.json` and re-applies the overlay config. |
| Discord Rich Presence | `settings.discord_rpc_enabled` | Checkmark plus an immediate presence refresh. |
| Quit CREBLOX | — | Drops the status item, stops the instance listener and the overlay helper. |

**Live sync in both directions.** A menu click becomes `Msg::TrayAction` on the existing
channel; the handler at `app.rs:2695` applies it exactly like the corresponding Settings
control, persists it, and calls `sync_tray_state` (`app.rs:3907`) so the menu shows the new
value on the same frame. Changes made in the window arrive the same way, because the tray
state is rebuilt from `Settings` plus the live session on every sync. The tray reader thread
calls `ctx.request_repaint()` when it forwards a click, so a minimized or idle launcher wakes
immediately instead of waiting for a mouse move.

**The overlay follows without a restart.** Toggling from the menu runs
`maybe_show_game_overlay` (`app.rs:3943`), which pushes an `OverlayConfig` into
`OverlayManager::start` (`game_overlay.rs:252`) — spawning the helper, or updating the
running one's shared config. The helper rewrites the JSON spec (`game_overlay.rs:350`) and
the overlay window re-reads it when the file's stamp changes, polling at 150 ms
(`overlay_window.rs:543`). The overlay window is click-through and never becomes key, so the
game keeps keyboard and mouse focus.

**Threading.** Every `NSStatusItem` / `NSMenu` mutation happens on the main thread:
`TrayController::new` and `sync` are called from eframe's UI thread. The background thread
only drains `MenuEvent` / `TrayIconEvent` and pushes to the channel; it never touches AppKit.

**One instance.** A second launch (Finder, Dock, Spotlight) fails the `flock` in
`try_focus_existing_instance`, then asks the running copy over a Unix socket
(`notify_running_instance_show` → `start_instance_listener`, `tray_macos.rs:774`) to show its
window, instead of starting a duplicate that would fight over the status item.

### Updating an existing install (manual)

Updates are pulled from GitHub by hand — that is the supported path today.

1. Open **Releases** and download the *quick update* archive for the version you want
   (`CREBLOX-<version>-update.zip`). The notes list it first with its size and SHA-256;
   check it with `shasum -a 256 CREBLOX-<version>-update.zip`.
2. Double-click **`Update CREBLOX.command`** inside the unzipped folder. It locates your
   source folder (the one holding `build.rs` and `name = "creblox"` in `Cargo.toml`),
   copies the changed files into it, rebuilds, and replaces the installed `.app`.
   `settings.json`, saved accounts, playtime and FastFlags are kept.
3. It remembers the folder in `~/Library/Application Support/CREBLOX/source-folder`. If
   that pointer is missing it searches Desktop, Documents, Downloads and Developer, and
   otherwise asks you to pick the folder.

On a machine that has never run CREBLOX, use the *full build* archive and double-click
**`Build CREBLOX.command`** instead.

The in-app updater is **not** part of this flow. `src/updater.rs`, the update window and
their call sites are dormant: leave them untouched while working on the app, and they will
be deleted in a future release. Old `settings.json` files may still carry the update-token
and `auto_check_updates` keys; they are accepted and ignored.

## 4. Data and configuration paths (macOS)

Everything CREBLOX owns lives in **`~/Library/Application Support/CREBLOX/`** (`auth::data_dir`;
pre-0.4.15 `Aurora` folders are migrated into it on first run):

| File | Contents |
|---|---|
| `settings.json` | Serialized `Settings` (see below). Missing keys fall back to `Default`, so old and new configs both load. |
| `fastflags.json` | Serialized `FastFlagsConfig`: renderer, lighting, toggles, `msaa`, `quality_level`, `custom_flags`. |
| `session.dat` | AES-256-GCM encrypted `.ROBLOSECURITY` cookie. |
| `session.key` | 32-byte AES key, mode `0600`. Deleting it logs the user out. |
| `accounts.dat` | Encrypted saved-account list for the switcher. |
| `playtime.json` | `{ "<universeId>": {"seconds": N, "last_played": <unix>} }`. |
| `favorites.txt` | One Place ID per line. |
| `seen_notifications.json` | Notification ids already shown (drives the unread badge). |
| `log.txt` | Plain-text app log (`auth::log`). |
| `.instance.lock` | Single-instance guard (pid of the running launcher). |
| `original_command.txt` | Windows only: backup of the original `roblox-player` handler command. |

Roblox-side paths CREBLOX reads or writes:

| Path | Why |
|---|---|
| `/Applications/Roblox.app` | Preferred client bundle. |
| `~/Applications/Roblox.app` | User-level install. |
| `~/Library/Application Support/Roblox/Versions/*/RobloxPlayer.app` | Versioned layouts; newest is used. |
| `~/Library/Preferences/com.roblox.Roblox/ClientAppSettings.json` | macOS FastFlags (primary). |
| `<RobloxPlayer.app>/Contents/MacOS/ClientSettings/ClientAppSettings.json` | Legacy FastFlags location (also written). |
| Roblox log directory (resolved at runtime by `roblox_logs_dir()`) | Session state detection. |

### `settings.json` schema (abridged)

Source of truth: `types.rs::Settings`. All fields are optional on read.

| Group | Keys |
|---|---|
| Appearance | `dark`, `bg_mode` (`theme_default`/`solid_color`/`wallpaper`), `bg_color: [r,g,b]`, `bg_wallpaper`, `bg_overlay`, `text_mode`, `borders_color`, `profile_color`, `friends_color`, `games_border_color`, `text_border_color`, `glass_edition`, `glass_opacity`, `glass_highlight`, `corner_roundness`, `animations_enabled` |
| Performance | `target_fps`, `ram_limit_mb`, `cpu_limit_pct` |
| Behaviour | `tray_enabled`, `tray_minimize_on_close`, `start_with_windows` (launch at login), `start_minimized_to_tray`, `auto_close_roblox`, `notifications_silent`, `auto_check_updates` (legacy, ignored), `minimize_on_game_launch`, `close_launcher_during_game`, `return_to_launcher_on_leave`, `show_server_badge` |
| Overlay | `overlay_enabled`, `overlay_corner`, `overlay_duration_secs`, `overlay_opacity`, `overlay_show_ip`, `overlay_show_job_id`, `overlay_stats_enabled`, `overlay_stats_corner`, `overlay_stats_cpu/ram/ping/session/clock/fps_cap`, `overlay_flags_enabled`, `overlay_flags_corner` |
| Integrations | `discord_rpc_enabled`, `discord_rpc_show_idle`, `discord_rpc_app_id` |

### `fastflags.json` schema

```json
{
  "renderer": "auto | d3d11 | vulkan | opengl | metal",
  "lighting": "default | voxel | shadow_map | future",
  "disable_shadows": false,
  "low_quality_textures": false,
  "disable_post_fx": false,
  "disable_grass": false,
  "disable_telemetry": false,
  "show_fps": false,
  "msaa": null,
  "quality_level": null,
  "custom_flags": { "DFIntTaskSchedulerTargetFps": 240 }
}
```

---

## 5. Build and run (macOS 10.15.7)

### Prerequisites

* macOS 10.15.7 or newer, Xcode Command Line Tools (`xcode-select --install`).
* Rust **1.85+** (edition 2024): `curl https://sh.rustup.rs -sSf | sh`.
* No other system libraries: TLS is `rustls` with bundled roots, image decoding is pure Rust.

### Commands

```bash
# type-check the Catalina target (the deployment target is pinned in .cargo/config.toml)
cargo check

# build a release binary
cargo build --release            # -> target/release/creblox

# build + assemble + ad-hoc sign + install dist/CREBLOX.app
scripts/build-macos-app.sh       # NO_INSTALL=1 to skip the install step

# package the release archives (quick update + full build)
scripts/package-release.sh
```

`MACOSX_DEPLOYMENT_TARGET` is pinned to `10.15` in `.cargo/config.toml`, so
`MACOSX_DEPLOYMENT_TARGET=10.15 cargo check` and a bare `cargo check` are equivalent (an
externally set value still wins). Note that `scripts/build-macos-app.sh` builds for the host
CPU (`-C target-cpu=native`) unless `CREBLOX_PORTABLE=1` is set, and it does **not** `lipo`
the slices: a release that must run on Catalina has to be built on an Intel Mac (or built
portable), because an arm64-only bundle cannot launch there.

Bundle fields that matter: `CFBundleIdentifier com.creblox.app`,
`LSMinimumSystemVersion 10.15`, `NSHighResolutionCapable`,
`NSSupportsAutomaticGraphicsSwitching`.

### Verifying from a non-macOS machine (for AI/CI sandboxes)

The crate only compiles for macOS/Windows (`tray`, `roblox_process` and the font tables are
`cfg`-gated). To type-check the macOS code paths from Linux:

```bash
rustup target add x86_64-apple-darwin
# ring's build script assembles Mach-O objects; zig ships a clang that can target darwin
CC_x86_64_apple_darwin=/path/to/zig-cc-darwin \
AR_x86_64_apple_darwin="zig ar" \
cargo check --target x86_64-apple-darwin --all-targets
```

where `zig-cc-darwin` forwards to `zig cc -target x86_64-macos "$@"` and drops the
`--target=x86_64-apple-macosx` argument the `cc` crate injects. This **type-checks** only —
it cannot link AppKit or sign a bundle, so it never replaces a real macOS build.

---

## 6. Troubleshooting (Catalina)

| Symptom | Cause / fix |
|---|---|
| "CREBLOX is damaged and can't be opened" | The `.app` is ad-hoc signed. Download through a browser so quarantine is applied normally, or `xattr -dr com.apple.quarantine /Applications/CREBLOX.app`. CREBLOX never modifies the Roblox bundle's signature itself, but FastFlags written inside `RobloxPlayer.app` can invalidate Roblox's own signature — delete that `ClientSettings` folder if Roblox refuses to start. |
| Window opens then closes / `Error: ...` on launch | OpenGL context creation failed. eframe uses glow (OpenGL 3.3+); Catalina on any Metal-capable Mac provides OpenGL 4.1. Very old GPUs without 3.3 cannot run the UI. Check `log.txt` in the data folder. |
| Blurry text or wrong hitboxes | HiDPI is handled by egui; if a scaling tool (e.g. a custom `NSHighResolutionCapable=NO` plist edit) forces 1x, restore the shipped plist. |
| Roblox not detected | Re-scan in Settings → Roblox. Install locations supported: `/Applications`, `~/Applications`, versioned folders. |
| Games launch in a browser instead of Roblox | A browser captured `roblox-player:`. CREBLOX launches the discovered bundle explicitly; re-register Roblox with `open -a Roblox` once, or set the default handler in Safari's preferences. |
| FastFlags don't apply | They are written on launch (`sync_saved_fastflags_before_launch`) and on save. Check both `~/Library/Preferences/com.roblox.Roblox/ClientAppSettings.json` and the bundle copy; Roblox updates wipe the bundle copy. |
| TLS/certificate errors | Networking is `rustls` + bundled `webpki-roots` — it does not use Catalina's system root store, so expired system certificates cannot break it. A corporate MITM proxy will. |
| Menu bar icon missing | `tray_enabled` is off, or another instance owns `.instance.lock`. |
| Won't update | Updates are manual: download the archive from **Releases** and run `Update CREBLOX.command` (or `Build CREBLOX.command` on a new machine). There is no in-app update check to debug. If the script cannot find your source folder, check `~/Library/Application Support/CREBLOX/source-folder` and the `Cargo.toml` inside it. |

---

## 7. AI developer context / invariants

Read this before editing. These are not preferences; breaking them breaks Catalina users.

1. **macOS 10.15.7 is the floor.** Never raise `MACOSX_DEPLOYMENT_TARGET`, never call an
   AppKit/Security/UTType API introduced in 11.0+, and never add a dependency whose minimum
   OS is newer. `objc2-app-kit` and `tray-icon` are already at the edge of what Catalina
   allows — check availability annotations before adding more.
2. **glow/OpenGL only.** `wgpu`/Metal-only rendering is deliberately excluded. Do not
   re-enable it "for performance".
3. **Never block the UI thread.** `ui()`, `poll()`, page functions and action handlers must
   not do network requests, file hashing, archive/binary work, `process::Command::status()`
   or sleeps. Use `self.spawn(closure) -> Msg` and handle the result in `poll()`. Process
   enumeration in UI code must use the cached variant.
4. **No `unwrap()`/`expect()` on I/O, network, paths or FFI.** Surface a status string or an
   error card instead. `unwrap()` is acceptable only inside `#[cfg(test)]`.
5. **One theme.** Colours, radii, spacing and type sizes come from `src/theme.rs`
   (`pal(ui)`, `TYPE`, `SPACE`, `on_accent`, `stat_colors`, `card_frame`). No new hard-coded
   hex value for a themed surface, no new magic font size.
6. **Frozen identifiers.** The data-folder *file* names — `settings.json`, `fastflags.json`,
   `playtime.json`, `favorites.txt`, `accounts.dat`, `session.dat`/`session.key` — and the
   FastFlags key names are a data contract: add fields with defaults, never rename or move
   them without a migration. The crate/binary (`creblox`), bundle id (`com.creblox.app`) and
   data folder (`CREBLOX`) were renamed in 0.4.15 behind a one-way migration; do not move
   them back. User-facing strings say CREBLOX.
7. **Preserve launcher features.** FastFlags presets, custom flags, multi-account, overlay,
   tray and deep links all ship. Fix them; don't delete them. The dormant update
   subsystem is the one exception: it is scheduled for deletion, not preservation.
8. **The release contract is `RELEASING.md`.** `Cargo.toml` version == tag, normal release,
   exact asset names, binaries built from the tagged commit, never re-upload an asset.
9. **The macOS binary must contain an x86_64 slice.** Catalina is Intel-only; keep the
   universal build in `scripts/build-macos-app.sh` working.
10. **Test on the target.** `cargo check`/`clippy` for `x86_64-apple-darwin` is the minimum;
    UI changes are unverified until the app has actually been seen on Catalina (or at least
    on the macOS build you ship).
11. **Project house rule (`AGENTS.md`):** don't read the same file twice in one task unless it
    changed. Search with line numbers, then read one window.

---

## 8. License

GPL-3.0-only. See `LICENSE`.

*CREBLOX is an unofficial third-party launcher. It is not affiliated with or endorsed by
Roblox Corporation. It never stores passwords, only the session cookie the official login
flow returns.*
