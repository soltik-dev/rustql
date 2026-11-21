#!/usr/bin/env python3
"""Generates the RustQL icons with Pillow: icon.png (256 px, the window and
Linux icon), icon.ico (Windows) and icon.icns (macOS, drawn at 1024 px).

A database cylinder in the interface's accent blue over the dark theme's
background. Drawn at 4x and scaled down to smooth the edges.

    python3 assets/icon/make_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

# Design grid: every coordinate below is in units of a 256 px icon.
DESIGN = 256
SUPERSAMPLE = 4
BG = (19, 24, 36, 255)        # dark theme background
RIM = (38, 48, 70, 255)
ACCENT = (76, 141, 255, 255)  # theme::accent (dark)
ACCENT_TOP = (140, 184, 255, 255)
ACCENT_DARK = (45, 98, 200, 255)


def render(size: int) -> Image.Image:
    """The icon at `size` px."""
    scale = size / DESIGN * SUPERSAMPLE
    s = round(DESIGN * scale)
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    def px(v: float) -> int:
        return round(v * scale)

    # Background: rounded square with a thin border
    d.rounded_rectangle([px(8), px(8), px(248), px(248)], radius=px(52), fill=RIM)
    d.rounded_rectangle([px(11), px(11), px(245), px(245)], radius=px(49), fill=BG)

    # Cylinder of three discs, bottom to top
    left, right = px(62), px(194)
    ry = px(21)          # vertical semi-axis of the ellipses
    disc_h = px(34)      # height of each disc's side
    gap = px(11)
    top0 = px(66)
    for i in reversed(range(3)):
        top = top0 + i * (disc_h + gap)
        bottom = top + disc_h
        # Side: rectangle + lower half ellipse
        d.ellipse([left, bottom - ry, right, bottom + ry], fill=ACCENT_DARK)
        d.rectangle([left, top, right, bottom], fill=ACCENT_DARK)
        d.ellipse([left, bottom - ry - px(5), right, bottom + ry - px(5)], fill=ACCENT)
        d.rectangle([left, top, right, bottom - px(5)], fill=ACCENT)
        # Lid
        d.ellipse([left, top - ry, right, top + ry], fill=ACCENT_TOP)

    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    here = Path(__file__).parent
    icon = render(256)
    icon.save(here / "icon.png", optimize=True)
    icon.save(here / "icon.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    render(1024).save(here / "icon.icns")
    for name in ("icon.png", "icon.ico", "icon.icns"):
        print(here / name)


if __name__ == "__main__":
    main()
