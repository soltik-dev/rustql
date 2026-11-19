"""Adds Font Awesome 4.7 glyphs (SIL OFL 1.1) to rustql.ttf.

rustql.ttf came from fontello, which takes Font Awesome 4 glyphs with the
same code points and scales them from 1792 to 1000 units per em, lowering
them so the descender sits at -150. This script does the same with the
missing ones, so the result matches the glyphs already there.

Usage (fontTools; Font Awesome 4.7 on Fedora: package fontawesome4-fonts):
    python3 add_fontawesome_glyphs.py /usr/share/fonts/fontawesome/fontawesome-webfont.ttf
"""

import sys
from pathlib import Path

from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

# code point → glyph name in rustql.ttf (its use, in src/ui/icons.rs)
GLYPHS = {
    0xF093: "upload",  # EXPORT
    0xF002: "search",  # SEARCH
    0xF084: "key",  # KEY
    0xF017: "clock",  # CLOCK
    0xF0E2: "undo",  # UNDO
    0xF023: "lock",  # LOCK
    0xF09C: "unlock",  # UNLOCK
    0xF06E: "eye",  # VIEW
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
            print(f"{code:#x} already there, left as is")
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
        print(f"{code:#x} ← {source} as {name}")
    font.setGlyphOrder(order)
    font["glyf"].setGlyphOrder(order)
    font.save(FONT)


if __name__ == "__main__":
    main(sys.argv[1])
