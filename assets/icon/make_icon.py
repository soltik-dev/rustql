#!/usr/bin/env python3
"""Genera el icono de RustQL (icon.png, 256 px) con Pillow.

Un cilindro de base de datos con el azul de acento de la interfaz sobre el
fondo oscuro del tema. Se dibuja a 4x y se reduce para suavizar los bordes.

    python3 assets/icon/make_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 256
SCALE = 4
BG = (19, 24, 36, 255)        # fondo del tema oscuro
RIM = (38, 48, 70, 255)
ACCENT = (76, 141, 255, 255)  # theme::accent (oscuro)
ACCENT_TOP = (140, 184, 255, 255)
ACCENT_DARK = (45, 98, 200, 255)


def main() -> None:
    s = SIZE * SCALE
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    def px(v: float) -> int:
        return round(v * SCALE)

    # Fondo: cuadrado de esquinas redondeadas con un borde fino
    d.rounded_rectangle([px(8), px(8), px(248), px(248)], radius=px(52), fill=RIM)
    d.rounded_rectangle([px(11), px(11), px(245), px(245)], radius=px(49), fill=BG)

    # Cilindro de tres discos, de abajo arriba
    left, right = px(62), px(194)
    ry = px(21)          # semieje vertical de las elipses
    disc_h = px(34)      # alto del canto de cada disco
    gap = px(11)
    top0 = px(66)
    for i in reversed(range(3)):
        top = top0 + i * (disc_h + gap)
        bottom = top + disc_h
        # Canto: rectángulo + media elipse inferior
        d.ellipse([left, bottom - ry, right, bottom + ry], fill=ACCENT_DARK)
        d.rectangle([left, top, right, bottom], fill=ACCENT_DARK)
        d.ellipse([left, bottom - ry - px(5), right, bottom + ry - px(5)], fill=ACCENT)
        d.rectangle([left, top, right, bottom - px(5)], fill=ACCENT)
        # Tapa
        d.ellipse([left, top - ry, right, top + ry], fill=ACCENT_TOP)

    out = Path(__file__).with_name("icon.png")
    img.resize((SIZE, SIZE), Image.LANCZOS).save(out, optimize=True)
    print(out)


if __name__ == "__main__":
    main()
