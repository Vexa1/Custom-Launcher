#!/usr/bin/env python3
"""Normalise the CREBLOX artwork and rebuild the Windows .ico.

`assets/logo.png` is the single source of truth for the in-app logo, the menu-bar
icon and the macOS Dock icon, so it must be a 1024x1024 *fully opaque* PNG: a
transparent edge made the Dock/Finder icon look cut off.

This script only normalises that file (size + opacity) and regenerates
`assets/icon.ico` from it, so re-running it never changes the artwork itself.

Usage: python3 scripts/make-icon.py
"""

from PIL import Image, ImageDraw
import os

SIZE = 1024
OUT_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets"
)


def main():
    png_path = os.path.join(OUT_DIR, "logo.png")
    icon = Image.open(png_path).convert("RGBA")
    if icon.size != (SIZE, SIZE):
        icon = icon.resize((SIZE, SIZE), Image.LANCZOS)

    # The artwork is a rounded square on a white background, so the four corners were
    # white and showed up as bright brackets behind the 30px logo in the app and in the
    # Dock icon. Fill everything outside the rounded square with the artwork's own
    # background colour, which keeps the mark identical and removes the white.
    bg = icon.getpixel((2, icon.height // 2))[:3]
    mask = Image.new("L", icon.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [(0, 0), (icon.width - 1, icon.height - 1)],
        radius=int(icon.width * 0.225),
        fill=255,
    )
    icon = Image.composite(icon, Image.new("RGBA", icon.size, bg + (255,)), mask)

    alpha = icon.getchannel("A")
    if alpha.getextrema()[0] < 255:
        # Fill any transparent edge with the artwork's own background colour.
        bg = icon.getpixel((icon.width // 2, max(4, int(icon.height * 0.04))))[:3]
        base = Image.new("RGBA", icon.size, bg + (255,))
        icon = Image.alpha_composite(base, icon)
        icon = icon.convert("RGBA")

    icon.save(png_path, "PNG")

    ico_path = os.path.join(OUT_DIR, "icon.ico")
    icon.save(
        ico_path,
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )

    print(f"wrote {png_path} and {ico_path}")


if __name__ == "__main__":
    main()
