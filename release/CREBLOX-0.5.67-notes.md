## Quick update (existing installs)

- **CREBLOX-0.5.67-update.zip** — 569058 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.67-update.zip — SHA-256 `4c32391abe2643b6fb986a7d216655b6982e82443614b875a556ffcee9d8140b`

## Full build (fresh installs)

- **CREBLOX-0.5.67-full.zip** — 914098 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.67-full.zip — SHA-256 `f908e7677b3d21347a4b0239e7b8be4a00bfff0ecac1dbe754b24e3cd93fd766`

## What changed

- **Disabled controls now look disabled.** `toggle_switch` draws a muted track, a grey knob
  and dimmed label when its UI is disabled (the animation sub-toggles under an off master),
  and ignores clicks, Space and Enter. `segmented` dims its accent to 45%, greys the labels
  and drops hover/focus while disabled.
- **Keyboard on the segmented control.** Once a segment has focus, Left and Right move the
  selection (clamped at both ends) and report a change, so the segmented picker is usable
  without the mouse.
- **Custom overlay images can be dragged and resized in Edit Mode.** The overlay window was
  created with `with_mouse_passthrough(true)`, and winit kept that state, silently overriding
  the AppKit `setIgnoresMouseEvents:NO` that edit mode performs - so every press fell through
  the picture. Passthrough is now sent as `ViewportCommand::MousePassthrough` whenever the
  editor opens or closes (with the native tweak kept for level and Spaces).
- The picture is one hitbox resolved at press: the body moves it, and the four 24 px corner
  grips win over the body, so a handle can no longer lose its drag to hit-test ordering. The
  live box also survives the launcher rewriting the spec while the editor is open, and Confirm
  writes the same `x`/`y`/`w`/`h` box the editor was showing.
- New `vm-tests/tests/overlay_image_edit.rs` drives real `PointerButton`/`PointerMoved` events
  through an egui context: a body drag moves by the exact delta, a bottom-right drag resizes
  from the opposite corner, corners beat the body, and the confirmed box round-trips through
  the panels JSON.

