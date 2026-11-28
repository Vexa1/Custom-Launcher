## What changed

### The HUD editor opens where the HUD actually is

Opening the editor used to let the spec change the layout under you: the editor's own preview card appears when no server is resolved, that card takes a slot in its corner, and every other card in that corner moved down the moment you opened the editor. The overlay now remembers where each card was drawn during live gameplay and re-anchors the editor to those exact rects on entry, so nothing moves until you drag it or press Reset. Both entry points - the tray item and the new **Edit HUD positions…** button in Settings -> Overlay HUD - go through the same code path.

### The custom image is a real element now

- **Drag it.** Anywhere inside the picture moves it; the camera icon changes to a grab hand.
- **Resize it from four corners.** An 8x8 px accent square is drawn on each corner inside a 24 pt target. The opposite corner stays put, the size never goes below 24x24, the box is clamped inside the game view, and holding **Shift** keeps the picture's aspect ratio.
- **Read it while you drag.** The toolbar shows `image  412, 236   320 × 128` for the picture (and the panel readout for a card), and it now clears the corner handles by 16 pt, so a handle can never end up under the Reset / Confirm pills.
- **It is saved exactly.** Confirm writes the box into `settings.json` (`overlay_image_offset_x/y/w/h`) and into the panels file the overlay reads, and live gameplay draws the picture at those points without stretching it. Reset puts it back in its corner slot at the height-percentage size, and a picture you never touched keeps following the corner/size sliders.

### Verification

- `vm-tests/tests/overlay_spec.rs`: five tests for the shared `ImageBox` arithmetic - default corner sizing, a saved box winning over the percentage, a hand-edited 3x3 box being ignored, dragging clamped to the view, each corner resizing from the opposite one with a 24 pt floor, and Shift's aspect lock.
- `vm-tests/tests/overlay_edit.rs`: the picture is rendered at `(412, 236)` and `320 × 128`, its four handles sit exactly on the corners at >= 24 pt with 8 px squares, and the toolbar overlaps neither the picture nor any handle.
- New artifact `vm-tests/artifacts/overlay_edit_image_resize.png`, rendered and looked at: dashed frame, four aligned handles, readout with the size, no toolbar overlap.
- Full gate: 363 tests, macOS and Windows (warnings denied) target checks clean.
