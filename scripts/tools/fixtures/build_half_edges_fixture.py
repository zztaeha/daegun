#!/usr/bin/env python3
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen

UPM = 1000

# A CFF outline keeps its fractions: roundTolerance 0 writes them as 16.16 operands.
def box(x0, y0, x1, y1):
    pen = T2CharStringPen(500, None, roundTolerance=0)
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()
    return pen.getCharString()

def empty():
    return T2CharStringPen(500, None).getCharString()

def scaled(scale):
    paint = {"Format": 10, "Glyph": "square", "Paint": {"Format": 2, "PaletteIndex": 0, "Alpha": 1.0}}
    return {"Format": 18, "Paint": paint, "scaleX": scale, "scaleY": scale, "centerX": 1, "centerY": 1}

def main(out_path):
    order = [".notdef", "low", "high", "left", "square", "up", "flip"]
    fb = FontBuilder(UPM, isTTF=False)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord("a"): "low", ord("b"): "high", ord("c"): "left", ord("s"): "square", ord("u"): "up",
                          ord("f"): "flip"})
    fb.setupCFF("DaegunHalfEdges", {"FullName": "DaegunHalfEdges"}, {
        ".notdef": empty(),
        "low": box(-2.5, 2.5, 7.5, 10.5),
        "high": box(0.5, -10.5, 9.5, -0.5),
        "left": box(-7.5, -3.5, -1.5, 4.5),
        "square": box(20, 0, 320, 700),
        "up": empty(),
        "flip": empty(),
    }, {})
    fb.setupHorizontalMetrics({name: (500, 0) for name in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunHalfEdges", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()
    fb.setupCPAL([[(0.1, 0.1, 0.8, 1.0)]])
    # The square scaled by a half about (1, 1), either way up, so every edge of its painted box lies on a half.
    fb.setupCOLR({"up": scaled(0.5), "flip": scaled(-0.5)})
    fb.save(out_path)
    print(f"wrote {out_path}")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "half-edges.otf")
