#!/usr/bin/env python3
# Writes cff-subrs.otf, cff-cid.otf and cff-expert.otf into the directory given; the fixture README
# says what each holds.
import os
import sys

from fontTools.cffLib import (
    CFFFontSet, TopDict, TopDictIndex, GlobalSubrsIndex, SubrsIndex, PrivateDict,
    CharStrings, FDArrayIndex, FontDict, FDSelect, IndexedStrings,
)
from fontTools.misc.psCharStrings import T2CharString
from fontTools.ttLib import TTFont, newTable

UPM = 1000


def enc(v):
    if -107 <= v <= 107:
        return bytes([v + 139])
    if 108 <= v <= 1131:
        v -= 108
        return bytes([(v >> 8) + 247, v & 0xFF])
    if -1131 <= v <= -108:
        v = -v - 108
        return bytes([(v >> 8) + 251, v & 0xFF])
    return bytes([28, (v >> 8) & 0xFF, v & 0xFF])


def nums(*vs):
    return b"".join(enc(v) for v in vs)


RMOVETO, RLINETO, HSTEM, ENDCHAR, CALLSUBR, RETURN, CALLGSUBR = 21, 5, 1, 14, 10, 11, 29


def box(x0, y0, x1, y1):
    return nums(x0, y0) + bytes([RMOVETO]) + nums(x1 - x0, 0, 0, y1 - y0, x0 - x1, 0) + bytes([RLINETO])


def shell(glyph_order, cmap):
    font = TTFont(recalcTimestamp=False)
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
    hmtx.metrics = {name: (500, 0) for name in glyph_order}
    font["hmtx"] = hmtx
    maxp = newTable("maxp")
    maxp.tableVersion = 0x00005000
    maxp.numGlyphs = len(glyph_order)
    font["maxp"] = maxp
    post = newTable("post")
    post.formatType = 3.0
    post.italicAngle = post.underlinePosition = post.underlineThickness = 0
    post.isFixedPitch = post.minMemType42 = post.maxMemType42 = post.minMemType1 = post.maxMemType1 = 0
    font["post"] = post
    from fontTools.ttLib.tables._c_m_a_p import cmap_format_4
    sub = cmap_format_4(4)
    sub.platformID, sub.platEncID, sub.language = 3, 1, 0
    sub.cmap = cmap
    table = newTable("cmap")
    table.tableVersion = 0
    table.tables = [sub]
    font["cmap"] = table
    return font


def font_set(font, top):
    fs = CFFFontSet()
    fs.major, fs.minor = 1, 0
    fs.otFont = font
    fs.fontNames = ["DaegunCff"]
    fs.topDictIndex = TopDictIndex()
    fs.strings = IndexedStrings()
    fs.GlobalSubrs = top.GlobalSubrs
    fs.topDictIndex.append(top)
    return fs


def subrs_font(out):
    # A to F share their box through a local subr; G to K through a global one. Aacute seacs after
    # a hint, Agrave from inside a subroutine, Acircumflex bare.
    custom = [f"g{k:02}" for k in range(12)]
    order = [".notdef", "space", "A", "acute", "grave", "circumflex", "Aacute", "Agrave", "Acircumflex"] + custom
    cmap = {0x20: "space", 0x41: "A", 0xB4: "acute", 0x60: "grave", 0x5E: "circumflex",
            0xC1: "Aacute", 0xC0: "Agrave", 0xC2: "Acircumflex"}
    cmap.update({0x61 + k: name for k, name in enumerate(custom)})
    font = shell(order, cmap)

    gsubrs = GlobalSubrsIndex()
    gsubrs.append(T2CharString(bytecode=box(100, 0, 400, 300) + bytes([RETURN])))
    private = PrivateDict()
    private.defaultWidthX, private.nominalWidthX = 0, 0
    lsubrs = SubrsIndex()
    lsubrs.append(T2CharString(bytecode=box(50, 0, 450, 700) + bytes([RETURN])))
    lsubrs.append(T2CharString(bytecode=nums(0, 200, 65, 193) + bytes([ENDCHAR])))
    private.Subrs = lsubrs

    top = TopDict()
    top.GlobalSubrs = gsubrs
    top.charset = order
    top.FontMatrix = [1 / UPM, 0, 0, 1 / UPM, 0, 0]
    top.FontBBox = [-10, -20, 990, 980]
    top.UnderlinePosition, top.UnderlineThickness = -103, 51
    top.version, top.Notice, top.FullName, top.FamilyName, top.Weight = "1.000", "Daegun CFF fixture", "Daegun Cff", "DaegunCff", "Regular"
    top.Private = private
    cs = CharStrings(None, order, gsubrs, private, None, None)
    programs = {
        ".notdef": bytes([ENDCHAR]),
        "space": bytes([ENDCHAR]),
        "A": nums(-107) + bytes([CALLSUBR, ENDCHAR]),
        "acute": box(200, 750, 300, 900) + bytes([ENDCHAR]),
        "grave": box(150, 750, 250, 900) + bytes([ENDCHAR]),
        "circumflex": box(100, 750, 350, 850) + bytes([ENDCHAR]),
        "Aacute": nums(10, 20) + bytes([HSTEM]) + nums(0, 200, 65, 194) + bytes([ENDCHAR]),
        "Agrave": nums(-106) + bytes([CALLSUBR]),
        "Acircumflex": nums(0, 200, 65, 195) + bytes([ENDCHAR]),
    }
    for k, name in enumerate(custom):
        program = nums(-107) + bytes([CALLGSUBR if k % 2 else CALLSUBR]) + box(500 + 10 * k, 0, 520 + 10 * k, 40) + bytes([ENDCHAR])
        programs[name] = program
    for name in order:
        cs[name] = T2CharString(bytecode=programs[name], private=private, globalSubrs=gsubrs)
    top.CharStrings = cs
    font["CFF "] = newTable("CFF ")
    font["CFF "].cff = font_set(font, top)
    font.save(out)


def cid_font(out):
    # Glyphs alternate between two FDs, each with its own local Subrs drawing a different stroke.
    order = [".notdef"] + [f"cid{k:05}" for k in [1, 2, 3, 4, 5, 6, 7, 8, 34, 125, 200]]
    font = shell(order, {0x41 + k - 1: order[k] for k in range(1, 12)})
    gsubrs = GlobalSubrsIndex()
    top = TopDict()
    top.GlobalSubrs = gsubrs
    top.charset = order
    top.ROS = ("Daegun", "Fixture", 0)
    top.CIDCount = 201
    top.FontMatrix = [1 / UPM, 0, 0, 1 / UPM, 0, 0]
    top.FontBBox = [0, 0, 900, 900]
    top.Notice = "Daegun CID fixture"
    fd_array = FDArrayIndex()
    privates = []
    for i, stroke in enumerate([(300, 0), (0, 300)]):
        private = PrivateDict()
        private.defaultWidthX, private.nominalWidthX = 0, 0
        subrs = SubrsIndex()
        subrs.append(T2CharString(bytecode=nums(*stroke) + bytes([RLINETO, RETURN])))
        subrs.append(T2CharString(bytecode=nums(100 * (i + 1), 0) + bytes([RLINETO, RETURN])))
        private.Subrs = subrs
        privates.append(private)
        fd = FontDict()
        fd.FontName = f"DaegunCid-FD{i}"
        fd.FontMatrix = [1, 0, 0, 1, 0, 0]
        fd.Private = private
        fd_array.append(fd)
    top.FDArray = fd_array
    select = FDSelect(format=3)
    select.gidArray = [0] + [k % 2 for k in range(1, len(order))]
    top.FDSelect = select
    cs = CharStrings(None, order, gsubrs, None, select, fd_array)
    for k, name in enumerate(order):
        private = privates[select.gidArray[k]]
        if k == 0:
            program = bytes([ENDCHAR])
        elif name in ("cid00034", "cid00125"):
            program = box(100, 0, 300, 200) + bytes([ENDCHAR])
        elif name == "cid00200":
            program = nums(0, 200, 65, 194) + bytes([ENDCHAR])
        else:
            program = nums(10 * k, 10 * k) + bytes([RMOVETO]) + nums(-107) + bytes([CALLSUBR]) + nums(-106) + bytes([CALLSUBR, ENDCHAR])
        cs[name] = T2CharString(bytecode=program, private=private, globalSubrs=gsubrs)
    top.CharStrings = cs
    font["CFF "] = newTable("CFF ")
    font["CFF "].cff = font_set(font, top)
    font.save(out)


def expert_font(out):
    # The first glyphs of the Expert charset, which fontTools writes as Top DICT charset 1.
    order = [".notdef", "space", "exclamsmall", "Hungarumlautsmall", "dollaroldstyle", "dollarsuperior"]
    font = shell(order, {0x20: "space", 0x21: "exclamsmall", 0x22: "Hungarumlautsmall", 0x24: "dollaroldstyle"})
    gsubrs = GlobalSubrsIndex()
    private = PrivateDict()
    private.defaultWidthX, private.nominalWidthX = 0, 0
    top = TopDict()
    top.GlobalSubrs = gsubrs
    top.charset = order
    top.FontMatrix = [1 / UPM, 0, 0, 1 / UPM, 0, 0]
    top.Private = private
    cs = CharStrings(None, order, gsubrs, private, None, None)
    for k, name in enumerate(order):
        program = bytes([ENDCHAR]) if k < 2 else box(10 * k, 0, 100 + 10 * k, 100) + bytes([ENDCHAR])
        cs[name] = T2CharString(bytecode=program, private=private, globalSubrs=gsubrs)
    top.CharStrings = cs
    font["CFF "] = newTable("CFF ")
    font["CFF "].cff = font_set(font, top)
    font.save(out)


if __name__ == "__main__":
    out_dir = sys.argv[1]
    subrs_font(os.path.join(out_dir, "cff-subrs.otf"))
    cid_font(os.path.join(out_dir, "cff-cid.otf"))
    expert_font(os.path.join(out_dir, "cff-expert.otf"))
