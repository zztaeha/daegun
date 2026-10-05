#!/usr/bin/env python3
# Builds gvar-points.ttf, a wght axis and a hand-written gvar of cases fontTools cannot write. Every
# glyph is the box 100..500 x 0..700 with advance 600, and each case varies it at wght 900:
#   A  every point +50 in x, phantom 1 +40, phantom 2 +100: advance 660, drawn at 110..510
#   B  point list 0, 0, 1 with x deltas 10, 20, 30: a repeated point takes both, so 130..530
#   C  point list 0, 1, 9 on 4 points: 9 is dropped alone, so 110..530
#   D  nine point numbers on 4 points (0 eight times, then 1), x deltas 1 eight times, then 50: 108..550
#   E  A as a component, phantom 1 +30 and phantom 2 +60: advance 630, lsb 120
import io
import struct
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.sfnt import SFNTReader, SFNTWriter
from fontTools.ttLib.tables._g_l_y_f import Glyph, GlyphComponent

ORDER = [".notdef", "A", "B", "C", "D", "E"]


def box():
    pen = TTGlyphPen(None)
    pen.moveTo((100, 0))
    pen.lineTo((500, 0))
    pen.lineTo((500, 700))
    pen.lineTo((100, 700))
    pen.closePath()
    return pen.glyph()


def composite():
    g = Glyph()
    c = GlyphComponent()
    c.glyphName, c.x, c.y, c.flags = "A", 0, 0, 0
    g.components = [c]
    g.numberOfContours = -1
    return g


def deltas(values):
    # Packed deltas: runs of byte-sized values, each run at most 64 long.
    out = b""
    for i in range(0, len(values), 64):
        run = values[i:i + 64]
        out += bytes([len(run) - 1]) + bytes(v & 0xFF for v in run)
    return out


def tuple_data(points, xs, ys):
    # One tuple at peak wght 1.0 with private point numbers; a count of 0 names every point.
    flags = 0x8000 | 0x2000
    serialized = points + deltas(xs) + deltas(ys)
    header = struct.pack(">HHh", len(serialized), flags, 0x4000)
    return struct.pack(">HH", 1, 4 + len(header)) + header + serialized


CASES = {
    "A": tuple_data(bytes([0]), [50, 50, 50, 50, 40, 100, 0, 0], [0] * 8),
    "B": tuple_data(bytes([3, 0x02, 0, 0, 1]), [10, 20, 30], [0, 0, 0]),
    "C": tuple_data(bytes([3, 0x02, 0, 1, 8]), [10, 30, 99], [0, 0, 0]),
    "D": tuple_data(bytes([9, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 1]), [1] * 8 + [50], [0] * 9),
    "E": tuple_data(bytes([0]), [0, 30, 60, 0, 0], [0] * 5),
}


def gvar_bytes():
    blobs = [CASES.get(name, b"") for name in ORDER]
    offsets, at = [], 0
    for blob in blobs:
        offsets.append(at)
        at += len(blob) + (len(blob) & 1)
    offsets.append(at)
    header_len = 20 + 4 * len(offsets)
    out = struct.pack(">HHHHIHHI", 1, 0, 1, 0, header_len, len(ORDER), 1, header_len)
    out += b"".join(struct.pack(">I", o) for o in offsets)
    for blob in blobs:
        out += blob + (b"\0" if len(blob) & 1 else b"")
    return out


def main(path):
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(ORDER)
    fb.setupCharacterMap({ord(c): c for c in "ABCDE"})
    glyphs = {name: box() for name in ORDER[1:5]}
    glyphs[".notdef"] = TTGlyphPen(None).glyph()
    glyphs["E"] = composite()
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({name: (600, 100 if name != ".notdef" else 0) for name in ORDER})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunGvarPoints", "styleName": "Regular"})
    fb.setupOS2()
    fb.setupPost()
    fb.setupFvar([("wght", 100, 400, 900, "Weight")], [])
    fb.font["head"].flags |= 2
    plain = io.BytesIO()
    fb.save(plain)
    # fontTools would re-encode a gvar it compiles, so the hand-laid bytes go in as they are.
    plain.seek(0)
    reader = SFNTReader(plain)
    tables = {tag: reader[tag] for tag in reader.keys()}
    tables["gvar"] = gvar_bytes()
    with open(path, "wb") as out:
        writer = SFNTWriter(out, len(tables))
        for tag in sorted(tables):
            writer[tag] = tables[tag]
        writer.close()


if __name__ == "__main__":
    main(sys.argv[1])
