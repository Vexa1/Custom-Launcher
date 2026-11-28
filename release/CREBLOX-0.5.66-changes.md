## What changed

- **Design tokens.** The dark palette now uses the specified `#0B0C10` canvas with three
  elevations (`#12141A`, `#171A22`, `#1E222C`), a 7% hairline and the `#00B06B` accent;
  secondary text is brighter in both themes so helper text reads at a glance. Radii are
  `8 px` controls, `12 px` cards/modals and fully rounded pills; the light theme was
  rebalanced to the same elevation steps.
- **New type and spacing steps.** `TEXT_TITLE` (28 px), `TEXT_BODY` (13 px), `TEXT_SMALL`
  (12 px) and `SPACE_8` (32 px) join the scale; helper text is now 12 px instead of 11 px.
- **New controls in `src/ui/widgets.rs`.** `toggle_switch` (40x22, painter-drawn, 160 ms
  knob, snaps when animations are off, keyboard focusable with Space/Enter) and
  `segmented` (equal-width, accent-filled selection, focus ring). Settings uses real switches
  for animations, the party overlay and Discord presence, and a segmented control for the
  party HUD position instead of a checkbox and a dropdown.
- Settings' page title uses the shared page header again and the login card title takes the
  28 px title role, so no screen drifts from the type scale.
