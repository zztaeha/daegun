#!/usr/bin/env python3
import sys

from fontTools.ttLib import TTFont, newTable
from fontTools.cffLib import (
    CFFFontSet, TopDict, TopDictIndex, GlobalSubrsIndex, SubrsIndex, PrivateDict,
    CharStrings, FDArrayIndex, FontDict, FDSelect,
)
from fontTools.misc.psCharStrings import T2CharString

UPM = 1000
GLYPH_ORDER = [".notdef", "cid00001", "cid00002", "cid00003", "cid00004"]
FD_SELECT_GIDS = [0, 0, 0, 1, 1]
ORIGINS = {"cid00001": (10, 10), "cid00002": (20, 20), "cid00003": (30, 30), "cid00004": (40, 40)}

def enc(v):
    assert -107 <= v <= 107, "fixture only needs the one-byte Type2 number encoding"
    return bytes([v + 139])

def build_minimal_sfnt_shell(glyph_order):
    font = TTFont()
    font.sfntVersion = "OTTO"
    font.setGlyphOrder(glyph_order)

    head = newTable("head")
    head.tableVersion, head.fontRevision = 1.0, 1.0
    head.checkSumAdjustment, head.magicNumber, head.flags = 0, 0x5F0F3CF5, 0
    head.unitsPerEm = UPM
    head.created = head.modified = 0
    head.xMin = head.yMin = 0
    head.xMax = head.yMax = UPM
    head.macStyle, head.lowestRecPPEM, head.fontDirectionHint = 0, 6, 2
    head.indexToLocFormat, head.glyphDataFormat = 0, 0
    font["head"] = head

    hhea = newTable("hhea")
    hhea.tableVersion = 0x00010000
    hhea.ascent, hhea.descent, hhea.lineGap = UPM, 0, 0
    hhea.advanceWidthMax = UPM
    hhea.minLeftSideBearing = hhea.minRightSideBearing = 0
    hhea.xMaxExtent = UPM
    hhea.caretSlopeRise, hhea.caretSlopeRun, hhea.caretOffset = 1, 0, 0
    hhea.reserved0 = hhea.reserved1 = hhea.reserved2 = hhea.reserved3 = 0
    hhea.metricDataFormat = 0
    hhea.numberOfHMetrics = len(glyph_order)
    font["hhea"] = hhea

    hmtx = newTable("hmtx")
    hmtx.metrics = {name: (600, 0) for name in glyph_order}
    font["hmtx"] = hmtx

    maxp = newTable("maxp")
    maxp.tableVersion = 0x00005000
    maxp.numGlyphs = len(glyph_order)
    font["maxp"] = maxp

    return font

def build_cff_font_set(font, fdselect_format):
    fontSet = CFFFontSet()
    fontSet.major, fontSet.minor = 1, 0
    fontSet.otFont = font
    fontSet.fontNames = ["TestCID-Regular"]
    fontSet.topDictIndex = TopDictIndex()

    globalSubrs = GlobalSubrsIndex()
    fontSet.GlobalSubrs = globalSubrs

    topDict = TopDict()
    topDict.charset = GLYPH_ORDER
    topDict.ROS = ("Adobe", "Identity", 0)
    topDict.CIDCount = len(GLYPH_ORDER)
    topDict.GlobalSubrs = globalSubrs
    topDict.FontMatrix = [1 / UPM, 0, 0, 1 / UPM, 0, 0]

    call_subr0 = enc(-107) + bytes([10])
    fd_subr_bytecode = [
        enc(100) + enc(0) + bytes([5]) + bytes([11]),  # FD0 subr0: rlineto(100,0); return
        enc(0) + enc(100) + bytes([5]) + bytes([11]),  # FD1 subr0: rlineto(0,100); return
    ]

    fd_privates = []
    for i in range(2):
        priv = PrivateDict()
        priv.defaultWidthX, priv.nominalWidthX = 0, 0
        subrs = SubrsIndex()
        subrs.append(T2CharString(bytecode=fd_subr_bytecode[i]))
        priv.Subrs = subrs
        fd_privates.append(priv)

    fdArray = FDArrayIndex()
    for i, priv in enumerate(fd_privates):
        fd = FontDict()
        fd.FontName = f"TestCID-Regular-{i}"
        fd.FontMatrix = [1, 0, 0, 1, 0, 0]
        fd.Private = priv
        fdArray.append(fd)
    topDict.FDArray = fdArray

    fdSelect = FDSelect(format=fdselect_format)
    fdSelect.gidArray = list(FD_SELECT_GIDS)
    topDict.FDSelect = fdSelect

    charStrings = CharStrings(None, topDict.charset, globalSubrs, None, fdSelect, fdArray)
    for i, name in enumerate(GLYPH_ORDER):
        private = fd_privates[fdSelect.gidArray[i]]
        if name == ".notdef":
            bytecode = bytes([14])  # bare endchar
        else:
            dx, dy = ORIGINS[name]
            bytecode = enc(dx) + enc(dy) + bytes([21]) + call_subr0 + bytes([14])
        charStrings[name] = T2CharString(bytecode=bytecode, private=private, globalSubrs=globalSubrs)
    topDict.CharStrings = charStrings

    fontSet.topDictIndex.append(topDict)
    return fontSet

def main(out_path, fdselect_format):
    font = build_minimal_sfnt_shell(GLYPH_ORDER)
    font["CFF "] = newTable("CFF ")
    font["CFF "].cff = build_cff_font_set(font, fdselect_format)
    font.save(out_path)

if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]))
