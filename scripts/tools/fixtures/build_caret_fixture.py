#!/usr/bin/env python3
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables import otTables as ot

UPM = 1000

F1_CARETS = [300, 600]          # "ffi": two carets, format 1 (plain coordinates)
F3_CARETS = [250, 500, 750]     # "ffl": three carets, format 3 (coordinate + device)
F2_POINT_INDEX = 1
F2_EXPECTED_X = 420

GLYPHS = {
    ".notdef": None,
    "f": (50, 0, 350, 700),
    "i": (50, 0, 250, 700),
    "l": (50, 0, 250, 700),
    "f_f_i": (20, 0, 900, 700),     # the format-1 ligature
    "f_i": (20, 0, 420, 700),       # the format-2 ligature; x=420 is point 1
    "f_f_l": (20, 0, 900, 700),     # the format-3 ligature
}

def box(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()

def build_glyphs():
    out = {}
    for name, rect in GLYPHS.items():
        pen = TTGlyphPen(None)
        if rect:
            box(pen, *rect)
        out[name] = pen.glyph()
    return out

def caret_coord(value):
    cv = ot.CaretValue()
    cv.Format = 1
    cv.Coordinate = value
    return cv

def caret_point(index):
    cv = ot.CaretValue()
    cv.Format = 2
    cv.CaretValuePoint = index
    return cv

def caret_coord_with_device(value):
    cv = ot.CaretValue()
    cv.Format = 3
    cv.Coordinate = value
    dev = ot.Device()
    dev.StartSize = 0
    dev.EndSize = 0
    dev.DeltaFormat = 1
    dev.DeltaValue = [0]
    cv.DeviceTable = dev
    return cv

def lig_glyph(carets):
    lg = ot.LigGlyph()
    lg.CaretValue = carets
    lg.CaretCount = len(carets)
    return lg

def build_gdef(glyph_order):
    gdef = ot.GDEF()
    gdef.Version = 0x00010000

    classes = ot.GlyphClassDef()
    classes.classDefs = {
        "f": 1, "i": 1, "l": 1,
        "f_f_i": 2, "f_i": 2, "f_f_l": 2,   # 2 = ligature
    }
    gdef.GlyphClassDef = classes

    lcl = ot.LigCaretList()
    cov = ot.Coverage()
    covered = sorted(["f_f_i", "f_i", "f_f_l"], key=glyph_order.index)
    cov.glyphs = covered
    lcl.Coverage = cov

    by_name = {
        "f_f_i": lig_glyph([caret_coord(v) for v in F1_CARETS]),
        "f_i": lig_glyph([caret_point(F2_POINT_INDEX)]),
        "f_f_l": lig_glyph([caret_coord_with_device(v) for v in F3_CARETS]),
    }
    lcl.LigGlyph = [by_name[n] for n in covered]
    lcl.LigGlyphCount = len(covered)
    gdef.LigCaretList = lcl

    gdef.AttachList = None
    gdef.MarkAttachClassDef = None

    table = newTable("GDEF")
    table.table = gdef
    return table

def main(out_path):
    order = list(GLYPHS.keys())
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord("f"): "f", ord("i"): "i", ord("l"): "l"})
    glyphs = build_glyphs()
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (900 if n.count("_") else 400, 20) for n in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunCarets", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()

    ligatures = {("f", "f", "i"): "f_f_i", ("f", "i"): "f_i", ("f", "f", "l"): "f_f_l"}
    from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
    fea = "feature liga {\n"
    for seq, target in ligatures.items():
        fea += f"    sub {' '.join(seq)} by {target};\n"
    fea += "} liga;\n"

    fb.font["GDEF"] = build_gdef(order)
    fb.save(out_path)

    from fontTools.ttLib import TTFont
    font = TTFont(out_path)
    keep = font["GDEF"]
    addOpenTypeFeaturesFromString(font, fea)
    font["GDEF"] = keep
    font.save(out_path)
    print(f"wrote {out_path}")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "carets.ttf")
