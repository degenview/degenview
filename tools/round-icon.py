#!/usr/bin/env python3
"""Regenerates the rounded app icons Linux and Windows show from the square master.

macOS rounds its own Dock icon at run time (`inset_macos_icon` in src/lib.rs); a Linux launcher
or a Windows taskbar draws the file as it is, so it needs the corners in its alpha channel. Same
corner radius as the macOS code (22.3% of the side); the art fills the canvas, with no margin.

    pip install pillow
    tools/round-icon.py

Reads  assets/brand/DegenView-512.png (the square master, kept for macOS)
Writes assets/brand/DegenView-512-rounded.png (the window icon, the Flatpak and tarball icon)
       assets/brand/DegenView.ico (the Windows executable's icon: 16 to 256 px)
"""
from pathlib import Path

from PIL import Image, ImageDraw

BRAND = Path(__file__).resolve().parent.parent / "assets" / "brand"
RADIUS = 0.223
SUPERSAMPLE = 4
ICO_SIZES = [16, 32, 48, 64, 128, 256]


def rounded(master: Image.Image, side: int) -> Image.Image:
    """`master` scaled to `side` px with rounded corners, antialiased by drawing the mask larger."""
    big = side * SUPERSAMPLE
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=round(big * RADIUS), fill=255
    )
    icon = master.convert("RGBA").resize((side, side), Image.LANCZOS)
    icon.putalpha(mask.resize((side, side), Image.LANCZOS))
    return icon


def main() -> None:
    master = Image.open(BRAND / "DegenView-512.png")
    rounded(master, 512).save(BRAND / "DegenView-512-rounded.png", optimize=True)
    layers = [rounded(master, size) for size in ICO_SIZES]
    layers[-1].save(
        BRAND / "DegenView.ico",
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
        append_images=layers[:-1],
    )


if __name__ == "__main__":
    main()
