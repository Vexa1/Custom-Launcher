## Quick update (existing installs)

- **CREBLOX-0.5.52-update.zip** — 560274 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.52-update.zip — SHA-256 `b5967f1199e2860b9ad3da255d0d0e990a39621056fe690609b3b4a718e5b53d`

## Full build (fresh installs)

- **CREBLOX-0.5.52-full.zip** — 900335 bytes — https://github.com/Vexa1/Custom-Launcher/raw/main/release/CREBLOX-0.5.52-full.zip — SHA-256 `573eb2d44b834ebafbf1795546a9b3909372e15973620926cf1b76777c2615f5`

## What changed

### The picture's saved box has its own fields

0.5.51 stored the dragged picture's position in `overlay_image_offset_x/y` - the same two numbers the Settings "Offset X / Offset Y" sliders own. So once you had moved the picture, those sliders moved it again from its frozen spot, and there was no way to tell a saved position from a fine-tune nudge.

The box now has its own fields: `overlay_image_x`, `overlay_image_y`, `overlay_image_w`, `overlay_image_h`. The offsets are sliders again, in every case:

- **A saved box wins.** With `w`/`h` set, the picture is drawn at `x, y` and the offset sliders no longer move it. The Settings page reports `Saved from the HUD editor: 320 x 128 at 412, 236`.
- **No box yet.** With `w`/`h` at 0, nothing changed: corner, height percentage and offsets place the picture, exactly as before the editor existed.
- **Reset** still drops the box (`w`/`h` back to 0) and hands the picture back to its corner.
- **Nothing jumps on upgrade.** A `settings.json` written by 0.5.51 has a saved box but no `x`/`y`; those are read once from the offsets it wrote, so a picture someone had dragged stays exactly where they left it.

The panels file the overlay reads carries `x`/`y` alongside `w`/`h` for the same reason, and a 0.5.51 spec is still understood.

Also removed a leftover `let _ = grip;` in the editor's handle code; the 24 pt handle targets and the 8 px squares are unchanged.

### Verification

- The editor screenshot test now gives the picture a saved box at `412, 236` **and** deliberately different offsets (`30, -20`); it still renders at `412, 236` and `320 × 128`, which is the whole point of the split.
- `vm-tests/artifacts/overlay_edit_image_resize.png` re-rendered and looked at: unchanged frame, handles on the corners, toolbar clear of both.
- Full gate: 363 tests, macOS and Windows (warnings denied) target checks clean.

