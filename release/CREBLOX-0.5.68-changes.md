## What changed

- **Disabled controls no longer hint at the accent.** A disabled `toggle_switch` now paints the
  neutral inactive track (never green), a grey knob and a muted label, and egui's disabled fade
  takes it further down - a sub-toggle under an off "All animations" master cannot read as on.
  A disabled `segmented` keeps a 45%-dimmed accent so the selected option is still legible, with
  muted labels and no hover or focus. Arrow keys are ignored while disabled.
- **The switch knob stays visible in the light theme.** A white knob on the near-white off-track
  was invisible; the knob now follows the track's luminance (dark knob on light, white on the
  accent). Enabled switches are unchanged.
- **The controls moved to `src/ui/controls.rs`.** It depends only on `theme`, so the Linux
  screenshot fixtures include the real file and render the shipping switch and segmented picker
  instead of a copy. `widgets.rs` re-exports them, so no call site changed.
- **New widget tests** (`vm-tests/tests/widgets_disabled.rs`) assert the exact painted track,
  knob and segment colours with motion off, that an enabled switch still uses the accent, that
  Left/Right move the selection and clamp at both ends, and that a disabled picker ignores them.
- **New `settings-animations` screenshot** of the Settings page's Animations & Performance card
  with the master switch off, drawn by the real controls. The preview harness now also sets the
  egui theme and accent for light renders and drops the palette cache between them, so light
  screenshots no longer keep the dark palette.
- **Overlay image dragging is now one shared state machine.** `overlay_spec::ImageDrag` resolves
  the grabbed part at press and reports only each frame's pointer delta; the editor and the
  pointer simulation both use it, so a drag cannot double-apply its total or lose its corner.
  The simulation now checks two successive moves and that release clears the grab.
