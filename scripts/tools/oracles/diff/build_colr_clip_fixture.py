#!/usr/bin/env python3
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

UPM = 1000

def box(x0, y0, x1, y1):
    pen = TTGlyphPen(None)
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()
    return pen.glyph()

# One contour of 4,000 points, and composites of 16 copies, 64,000 points each, which maxp can count.
def big():
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0))
    for i in range(1, 4000):
        pen.lineTo((i % 400, (i // 400) * 70))
    pen.closePath()
    return pen.glyph()

def copies(dx, glyphs):
    pen = TTGlyphPen(glyphs)
    for k in range(16):
        pen.addComponent("big", (1, 0, 0, 1, dx + k, 0))
    return pen.glyph(dropImpliedOnCurves=False)

def solid(index):
    return {"Format": 2, "PaletteIndex": index, "Alpha": 1.0}

def gradient(stops):
    line = {"Extend": 0, "ColorStop": [{"StopOffset": i / (stops - 1), "PaletteIndex": i % 2, "Alpha": 1.0}
                                       for i in range(stops)]}
    return {"Format": 4, "ColorLine": line, "x0": 0, "y0": 0, "x1": 600, "y1": 0, "x2": 0, "y2": 600}

def glyph(name, paint):
    return {"Format": 10, "Glyph": name, "Paint": paint}

# The innermost composite clears two solids, which draw nothing but, under CLEAR, keep their layer open.
def composites(levels):
    if levels == 1:
        return {"Format": 32, "SourcePaint": solid(0), "CompositeMode": 0, "BackdropPaint": solid(1)}
    return {"Format": 32, "SourcePaint": composites(levels - 1), "CompositeMode": 3,
            "BackdropPaint": glyph("other", solid(1))}

def main(out_path):
    copy_names = [f"copies{k}" for k in range(17)]
    order = [".notdef", "clip", "inner", "other", "nested", "composite", "deep", "big", *copy_names, "past",
             "repeated", "foreground", "spread", "layered"]
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord("N"): "nested", ord("C"): "composite", ord("D"): "deep", ord("P"): "past",
                          ord("R"): "repeated", ord("F"): "foreground", ord("S"): "spread",
                          ord("L"): "layered"})
    base = {"big": big()}
    glyphs = {
        ".notdef": TTGlyphPen(None).glyph(),
        "clip": box(100, 0, 600, 700),
        "inner": box(0, 200, 700, 500),
        "other": box(300, -100, 400, 800),
        "nested": box(100, 0, 600, 700),
        "composite": box(100, 0, 600, 700),
        "deep": box(100, 0, 600, 700),
        "big": base["big"],
        "past": box(100, 0, 600, 700),
        "repeated": box(100, 0, 600, 700),
        "foreground": box(100, 0, 600, 700),
        "spread": box(100, 0, 600, 700),
        "layered": box(100, 0, 600, 700),
    }
    glyphs.update({name: copies(10 * k, base) for k, name in enumerate(copy_names)})
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({name: (0 if name == ".notdef" else 700, 0) for name in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunColrClip", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()
    fb.setupCPAL([[(0.8, 0.1, 0.1, 1.0), (0.1, 0.1, 0.8, 1.0)]])
    fb.setupCOLR(
        {
            # A glyph painted through another glyph, so the outer one is a clip.
            "nested": glyph("clip", glyph("inner", solid(0))),
            # A composite inside a glyph: two fills under one clip.
            "composite": glyph("clip", {
                "Format": 32,
                "SourcePaint": glyph("inner", solid(0)),
                "CompositeMode": 3,
                "BackdropPaint": glyph("other", solid(1)),
            }),
            # 64 composites, as deep as daegun parses a paint graph.
            "deep": composites(64),
            # COLR v0: 17 composites of 64,000 points are past a scene's 1,048,576; one named eight times
            # is held once.
            "past": [(name, 0) for name in copy_names],
            "repeated": [("copies0", 0)] * 8,
            # A v0 layer in the text color, which COLR v0 writes as palette index 0xFFFF.
            "foreground": [("inner", 0xFFFF), ("other", 1)],
            # 255 layers sharing one gradient of 4,000 stops: a small table that names 1,020,000 stops.
            "spread": {"Format": 1, "Layers": [glyph("inner", gradient(4000)) for _ in range(255)]},
            # Layers whose first is itself a PaintColrLayers, as fontTools' layer reuse writes them.
            "layered": {"Format": 1, "Layers": [{"Format": 1, "Layers": [glyph("inner", solid(0)), glyph("other", solid(1))]},
                                                glyph("clip", solid(1))]},
        },
    )
    fb.save(out_path)
    print(f"wrote {out_path}")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "colr-clip.ttf")
