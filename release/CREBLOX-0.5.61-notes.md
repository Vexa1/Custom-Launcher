## Quick update (existing installs)

- **CREBLOX-0.5.61-update.zip** — 564686 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.61-update.zip — SHA-256 `089eae5e0e1eff252c59a81fae98f859e9c0918f025b8ea1d7c492f16afe4c14`

## Full build (fresh installs)

- **CREBLOX-0.5.61-full.zip** — 904719 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.61-full.zip — SHA-256 `1a729d099c6aba83406be5c5aa47fb68a630fad911f650babef3b0355283bd94`

# CREBLOX 0.5.61

## The screenshot harness now renders the light theme too

Every page in `vm-tests` was only ever drawn with the dark palette, so the light theme - and the
contrast of its text - was never checked outside a single ratio test. The gallery now renders all
eleven pages twice, in both themes, and the contrast rules run on both sets of pictures.

Drawing the light pass at once found 116 unreadable text pairs that the dark-only harness could
not see:

- accent colours used as text (green, blue, orange, red) are fill colours: 2.5-3:1 on white,
  so the logo, playing counts, online/ping labels and party states all fell under 4.5:1;
- secondary text was 4.08:1 on the light surface card;
- the four green "Play", "Invite", "Ready" and "Launch Doors" buttons used the page background as
  their ink, which only worked while that background was near-black.

## What changed

- `theme::ink(color, dark)` is the text form of an accent: the accent itself on the dark palette,
  a darker matching ink (`GREEN_INK`, `BLUE_INK`, `ORANGE_INK`, `RED_INK`) on the light one.
  `warning_ink` now goes through it, and `ORANGE_INK` was darkened to 5.8:1 so it also reads on
  the light surface card.
- The light palette's `muted` text is darker (`#5A616D`), 5.3:1 on the light surface.
- Filled accent buttons use `ON_ACCENT` as their ink, the value `theme.rs` documents for them.
- The shot harness applies `theme::ink` to every label and button caption before it draws, so the
  contrast that is asserted is the colour that is actually painted.
- New artefacts per page: `<page>-light-<WxH>.png` beside the existing dark ones.

Both themes pass: 22 pictures per theme, 44 in all, geometry, clipping, hit targets and contrast.

