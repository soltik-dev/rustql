"""Añade a rustql.ttf glifos de Font Awesome 4.7 (SIL OFL 1.1).

rustql.ttf salió de fontello, que toma los glifos de Font Awesome 4 con sus
mismos códigos y los escala de 1792 a 1000 unidades por em, bajándolos para
que el descendente quede en -150. Este script hace lo mismo con los que
faltan, para que el resultado sea coherente con los que ya había.

Uso (fontTools; Font Awesome 4.7 en Fedora: paquete fontawesome4-fonts):
    python3 add_fontawesome_glyphs.py /usr/share/fonts/fontawesome/fontawesome-webfont.ttf
"""

import sys
from pathlib import Path

from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

# código → nombre del glifo en rustql.ttf (el uso, en src/ui/icons.rs)
GLYPHS = {
    0xF093: "upload",  # EXPORT
    0xF002: "search",  # SEARCH
    0xF084: "key",  # KEY
    0xF017: "clock",  # CLOCK
    0xF0E2: "undo",  # UNDO
}

FONT = Path(__file__).with_name("rustql.ttf")


def main(fontawesome_path: str) -> None:
    font = TTFont(FONT)
    fa = TTFont(fontawesome_path)
    scale = font["head"].unitsPerEm / fa["head"].unitsPerEm
    dy = font["hhea"].descent - fa["hhea"].descent * scale

    fa_cmap = fa.getBestCmap()
    order = list(font.getGlyphOrder())
    for code, name in GLYPHS.items():
        if code in font.getBestCmap():
            print(f"{code:#x} ya está, se deja")
            continue
        source = fa_cmap[code]
        pen = TTGlyphPen(None)
        fa.getGlyphSet()[source].draw(TransformPen(pen, (scale, 0, 0, scale, 0, dy)))
        glyph = pen.glyph()
        glyph.recalcBounds(font["glyf"])
        font["glyf"].glyphs[name] = glyph
        advance = round(fa["hmtx"][source][0] * scale)
        font["hmtx"][name] = (advance, glyph.xMin if glyph.numberOfContours else 0)
        order.append(name)
        for table in font["cmap"].tables:
            if table.isUnicode():
                table.cmap[code] = name
        print(f"{code:#x} ← {source} como {name}")
    font.setGlyphOrder(order)
    font["glyf"].setGlyphOrder(order)
    font.save(FONT)


if __name__ == "__main__":
    main(sys.argv[1])
