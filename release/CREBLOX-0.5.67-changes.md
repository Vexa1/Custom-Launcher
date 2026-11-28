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
