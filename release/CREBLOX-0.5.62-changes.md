# CREBLOX 0.5.62

## The pages themselves use the light-theme ink now

0.5.61 taught the screenshot harness to draw the light palette and to darken accent text. That
was only true of the fixtures: the shipped pages still painted the bright accent colours as text,
so on a light card the logo, "N playing" counts, ping times, party states and error lines were
2.5-3:1 against white.

Every accent used as *text* now goes through `theme::text_ink(ui, colour)`, which returns the
accent unchanged on the dark palette and the matching ink (`GREEN_INK`, `BLUE_INK`, `ORANGE_INK`,
`RED_INK`) on the light one. Fills, strokes, dots and progress bars keep the bright accent, which
is what it is for.

Changed:

- Home: profile status and the per-card "playing" counts.
- Game details: favorite label, owned-pass price, server occupancy badge and ping.
- Charts, Add friend, Profile, Login, Party, Settings, the account menu and notifications: error
  lines, presence labels, status badges and the "Launcher vX" line.
- Filled accent buttons use `ON_ACCENT` instead of white or the page background: the four green
  Join/Play style buttons, the avatar tab pills, the avatar body-colour pills and the "Silent: On"
  toggle.
- The shot fixtures call the same `theme::ink` at the same places, so the light pictures now show
  what the app really draws.

Both themes still pass the harness: 44 pictures, geometry, clipping, hit targets and contrast.
