#!/usr/bin/env python3
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables.ttProgram import Program

UPM = 1024

CVT = [0, 100, 200, 400, 512]

def box(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()

def glyphs():
    out = {}
    pen = TTGlyphPen(None); out[".notdef"] = pen.glyph()
    for name, (x0, y0, x1, y1) in {
        "A": (37, 0, 337, 703),
        "B": (37, 0, 637, 1003),
        "C": (0, 0, 300, 300),
        "D": (50, 50, 350, 350),
        "E": (37, 0, 337, 703),
        "F": (37, 0, 337, 703),
    }.items():
        pen = TTGlyphPen(None); box(pen, x0, y0, x1, y1); out[name] = pen.glyph()
    return out

FPGM = """
PUSHB[ ] 0
FDEF[ ]
  MDAP[1]
ENDF[ ]
PUSHB[ ] 1
FDEF[ ]
  PUSHB[ ] 2
  MDAP[1]
ENDF[ ]
"""

PREP = """
PUSHW[ ] 511
SCANCTRL[ ]
PUSHB[ ] 4
SCANTYPE[ ]
PUSHB[ ] 0 1
RCVT[ ]
WS[ ]
"""

GLYPH_PROGRAMS = {
    "A": """
        SVTCA[1]
        PUSHB[ ] 0 0
        CALL[ ]
        PUSHB[ ] 1 0
        CALL[ ]
        IUP[0]
        IUP[1]
    """,
    "B": """
        SVTCA[0]
        MPPEM[ ]
        PUSHB[ ] 30
        LT[ ]
        IF[ ]
          PUSHB[ ] 2
          MDAP[1]
        ELSE[ ]
          PUSHB[ ] 2
          MDAP[0]
        EIF[ ]
        IUP[0]
        IUP[1]
    """,
    "C": """
        SVTCA[0]
        PUSHB[ ] 2 1
        LOOPCALL[ ]
        IUP[0]
        IUP[1]
    """,
    "D": "",
    "E": """
        SVTCA[1]
        PUSHB[ ] 0
        FDEF[ ]
          MDAP[0]
        ENDF[ ]
        PUSHB[ ] 0 0
        CALL[ ]
        PUSHB[ ] 1 0
        CALL[ ]
        IUP[0]
        IUP[1]
    """,
    "F": """
        SVTCA[1]
        PUSHB[ ] 0 0
        CALL[ ]
        PUSHB[ ] 1 0
        CALL[ ]
        IUP[0]
        IUP[1]
    """,
}

def main(out_path):
    order = [".notdef", "A", "B", "C", "D", "E", "F"]
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord(c): c for c in "ABCDEF"})
    gs = glyphs()
    fb.setupGlyf(gs)
    metrics = {}
    for name in order:
        g = gs[name]
        width = 0 if name == ".notdef" else 700
        metrics[name] = (width, getattr(g, "xMin", 0) if hasattr(g, "xMin") else 0)
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunHinted", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()

    fb.font["fpgm"] = _program(FPGM)
    fb.font["prep"] = _program(PREP)
    fb.font["cvt "] = _cvt(CVT)
    for name, asm in GLYPH_PROGRAMS.items():
        if not asm.strip():
            continue
        glyph = fb.font["glyf"][name]
        p = Program()
        p.fromAssembly(asm)
        glyph.program = p

    maxp = fb.font["maxp"]
    maxp.maxZones = 2
    maxp.maxTwilightPoints = 16
    maxp.maxStorage = 64
    maxp.maxFunctionDefs = 16
    maxp.maxInstructionDefs = 0
    maxp.maxStackElements = 128
    maxp.maxSizeOfInstructions = 256

    fb.save(out_path)
    print(f"wrote {out_path}")

def _program(asm):
    from fontTools.ttLib import newTable
    t = newTable("fpgm")
    p = Program()
    p.fromAssembly(asm)
    t.program = p
    return t

def _cvt(values):
    from array import array
    from fontTools.ttLib import newTable
    t = newTable("cvt ")
    t.values = array("h", values)  # compile() byteswaps in place, so it needs a real array
    return t

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "hinted.ttf")
