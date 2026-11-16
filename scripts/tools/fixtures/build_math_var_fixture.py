#!/usr/bin/env python3
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.otlLib.builder import buildMathTable
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables import otTables as ot
from fontTools.varLib.builder import buildVarData, buildVarDevTable, buildVarRegionList, buildVarStore

UPM = 1000

# One wght axis, 100 to 900 with 400 its default, and two regions: one peaking at 900, one at 100.
AXIS = ("wght", 100, 400, 900)
REGIONS = [{"wght": (0, 1, 1)}, {"wght": (-1, -1, 0)}]

# Each varied value: as stored, then its deltas at 900 and at 100. Its row in the store is its index here.
VARIED = {
    "MathLeading": (100, 40, -20),
    "AxisHeight": (250, 30, -10),
    "italics": (10, 20, -8),
    "accent": (150, -30, 12),
    "kern": (30, 50, -14),
    "kern_height": (500, 100, -40),
    "assembly_italics": (40, 16, -6),
}
ROWS = list(VARIED)

GLYPHS = [".notdef", "x", "parenleft", "parenleft.top", "parenleft.ext", "parenleft.bot"]


def box(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()


def device(name):
    return buildVarDevTable(ROWS.index(name))


def value(record_value, name):
    record = ot.MathValueRecord()
    record.Value = record_value
    record.DeviceTable = device(name)
    return record


def main(out_path):
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(GLYPHS)
    fb.setupCharacterMap({ord("x"): "x", ord("("): "parenleft"})
    glyphs = {}
    for name in GLYPHS:
        pen = TTGlyphPen(None)
        if name != ".notdef":
            box(pen, 50, 0, 450, 700)
        glyphs[name] = pen.glyph()
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (500, 50) for n in GLYPHS})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunMathVar", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()
    tag, lo, default, hi = AXIS
    fb.setupFvar([(tag, lo, default, hi, "Weight")], [])

    region_list = buildVarRegionList(REGIONS, [tag])
    data = buildVarData([0, 1], [list(VARIED[name][1:]) for name in ROWS], optimize=False)
    store = buildVarStore(region_list, [data])
    gdef = ot.GDEF()
    gdef.Version = 0x00010003
    gdef.GlyphClassDef = gdef.AttachList = gdef.LigCaretList = gdef.MarkAttachClassDef = None
    gdef.MarkGlyphSetsDef = None
    gdef.VarStore = store
    fb.font["GDEF"] = newTable("GDEF")
    fb.font["GDEF"].table = gdef

    buildMathTable(
        fb.font,
        constants={"MathLeading": 100, "AxisHeight": 250},
        italicsCorrections={"x": 10},
        topAccentAttachments={"x": 150},
        mathKerns={"x": {"TopRight": ([500], [30, 0])}},
        minConnectorOverlap=20,
        vertGlyphVariants={"parenleft": [("parenleft", 700)]},
        vertGlyphAssembly={
            "parenleft": (
                [("parenleft.bot", 0, 0, 100, 300), ("parenleft.ext", 1, 100, 100, 300),
                 ("parenleft.top", 0, 100, 0, 300)],
                40,
            )
        },
    )
    math = fb.font["MATH"].table
    constants = math.MathConstants
    constants.MathLeading = value(100, "MathLeading")
    constants.AxisHeight = value(250, "AxisHeight")
    info = math.MathGlyphInfo
    info.MathItalicsCorrectionInfo.ItalicsCorrection[0] = value(10, "italics")
    info.MathTopAccentAttachment.TopAccentAttachment[0] = value(150, "accent")
    kern = info.MathKernInfo.MathKernInfoRecords[0].TopRightMathKern
    kern.CorrectionHeight[0] = value(500, "kern_height")
    kern.KernValue[0] = value(30, "kern")
    assembly = math.MathVariants.VertGlyphConstruction[0].GlyphAssembly
    assembly.ItalicsCorrection = value(40, "assembly_italics")
    fb.save(out_path)
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "math-var.ttf")
