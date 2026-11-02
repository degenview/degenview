#!/usr/bin/env python3
"""Regenerates the art behind the icons in the macOS disk image's window.

The window shows DegenView.app on the left and an Applications shortcut on the right
(packaging/macos/dmg-settings.py places them); this draws the arrow between them and a hint under
them. The output is committed, so this only runs when the art changes.

    pip install pillow
    tools/make-dmg-background.py

Writes packaging/macos/dmg-background.png     (660x380,  72 dpi)
       packaging/macos/dmg-background@2x.png  (1320x760, 144 dpi)
tools/make-dmg.sh joins the two into one Retina-aware TIFF with `tiffutil -cathidpicheck`, which
refuses files whose size and dpi do not pair up, hence the dpi written here.

The icon labels are drawn by Finder in black (light appearance) or white (dark appearance) and a
disk image cannot choose, so the background is a mid-tone slate that both can be read on.
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).resolve().parent.parent / "packaging" / "macos"
# Points, and the same numbers as dmg-settings.py. Finder ignores a disk image's wish to hide its
# status bar, path bar and tab bar (they are the user's global settings), so with all of them on only
# about 256 of the window's 380 points show: the icons, their labels and the hint stay in the top
# 256, and the lower part of the art only shows for a user with a bare window.
WIDTH, HEIGHT = 660, 380
APP_X, APPLICATIONS_X, ICON_Y = 180, 480, 125  # icon centres
HINT_Y = 232
TOP, BOTTOM = (156, 164, 178), (126, 134, 150)  # the vertical gradient
FOREGROUND = (255, 255, 255)
HINT = "Drag DegenView to Applications"
FONTS = ["/System/Library/Fonts/Helvetica.ttc", "/Library/Fonts/Arial.ttf"]


def font(size: int) -> ImageFont.ImageFont:
    for path in FONTS:
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default(size)


def draw(scale: int) -> Image.Image:
    """The background at `scale` pixels per point."""
    image = Image.new("RGB", (WIDTH * scale, HEIGHT * scale))
    pixels = ImageDraw.Draw(image)
    for y in range(HEIGHT * scale):
        mix = y / (HEIGHT * scale - 1)
        pixels.line(
            [(0, y), (WIDTH * scale, y)],
            fill=tuple(round(a + (b - a) * mix) for a, b in zip(TOP, BOTTOM)),
        )
    # Everything else is drawn on a 4x sheet and scaled down, which antialiases the shapes.
    unit = scale * 4
    sheet = Image.new("RGBA", (WIDTH * unit, HEIGHT * unit))
    ink = ImageDraw.Draw(sheet)
    y = ICON_Y * unit
    start, end = (APP_X + 100) * unit, (APPLICATIONS_X - 100) * unit
    thickness, head = 9 * unit, 22 * unit
    ink.rounded_rectangle(
        (start, y - thickness // 2, end - head // 2, y + thickness // 2),
        radius=thickness // 2,
        fill=FOREGROUND,
    )
    ink.polygon([(end - head, y - head * 0.7), (end, y), (end - head, y + head * 0.7)], fill=FOREGROUND)
    text = font(15 * unit)
    box = ink.textbbox((0, 0), HINT, font=text)
    ink.text(
        ((WIDTH * unit - (box[2] - box[0])) // 2 - box[0], HINT_Y * unit),
        HINT,
        font=text,
        fill=FOREGROUND,
    )
    sheet = sheet.resize(image.size, Image.LANCZOS)
    image.paste(sheet, (0, 0), sheet)
    return image


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    draw(1).save(OUT / "dmg-background.png", dpi=(72, 72), optimize=True)
    draw(2).save(OUT / "dmg-background@2x.png", dpi=(144, 144), optimize=True)
    print(f"wrote {OUT / 'dmg-background.png'} and dmg-background@2x.png")


if __name__ == "__main__":
    main()
