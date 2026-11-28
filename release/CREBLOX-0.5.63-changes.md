# CREBLOX 0.5.63

## The accent text that 0.5.62 missed

0.5.62 wrapped the obvious `colored_label(ACCENT, ...)` and `.color(ACCENT)` sites. This sweep
catches the rest - the ones that arrive through a match, a variable or a helper.

- Notification source labels (`ROBLOX API` / `CREBLOX` / `PROCESS`) and the toast kind glyph now
  draw through `theme::text_ink`, so a green or orange label reads on the light card.
- Toast titles and bodies were already `p.text` / `p.muted`; the toast action buttons already
  picked white or dark ink from the fill.
- The Settings "current value" labels use the theme accent, which can be a custom colour: they
  are inked too.
- Party error lines (which used `crate::theme::RED` fully qualified, so 0.5.62's rewrite missed
  them) are inked.
- The menu's unread badge is a filled dot, not text, and the nav labels already come from the
  palette - nothing to change there.

## `theme::ink` now handles any colour

`ink` used to map only the four named accents and hand anything else back untouched. A custom
accent - or the login page's `#FF6B2B` - is now darkened along its hue until it clears 4.5:1 on
white (`darken_for_light`), and `theme::contrast_ratio` / `contrast_on_white` are public so the
ratio can be asserted. A new `vm-tests/tests/theme_ink.rs` pins both halves: the dark palette
keeps the accent, the light one uses an ink that reads on white and on the surface card.

## Login page buttons

The Rec Room style buttons were white text on orange (2.9:1) in *both* themes. They now use the
same rule as the toast buttons - white on a dark fill, `ON_ACCENT` on a bright one - via a small
`on_fill` helper. The orange itself is only ever a fill there; no orange text existed to ink.

## Dark theme

The 0.5.62 switch from the page background to `ON_ACCENT` on filled green buttons changes dark
mode by two RGB steps (`#0B0D11` to `#061A12`), which is invisible in the 1280x800 shots of Home,
Game details, Party and Settings: the Play, Invite, Ready and Launch Doors captions still read as
near-black on green, and every accent label is unchanged because `ink` returns the accent under
the dark palette.
