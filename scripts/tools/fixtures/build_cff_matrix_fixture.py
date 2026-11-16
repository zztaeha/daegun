#!/usr/bin/env python3
# Writes cff-matrix.otf and cff-matrix-name.otf into the directory given: FontMatrix cases, a CID font's Top DICT and
# FDs inheriting, concatenating, out of range, and a name-keyed font's scale.
import os
import sys

from fontTools.cffLib import FDArrayIndex, FDSelect, FontDict, GlobalSubrsIndex, PrivateDict, TopDict, CharStrings
from fontTools.misc.psCharStrings import T2CharString

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from build_cff_subset_fixtures import ENDCHAR, box, font_set, shell
from fontTools.ttLib import newTable

FD_MATRICES = [None, [0.5, 0, 0, 0.5, 10, 20], [2, 0, 0, 2, 0, 0], [1e-12, 0, 0, 1, 0, 0]]


def program(k):
    return box(100 + 10 * k, 0, 400, 300) + bytes([ENDCHAR])


def cid_font(out):
    order = [".notdef"] + [f"cid{k:05}" for k in range(1, len(FD_MATRICES) + 1)]
    font = shell(order, {0x41 + k - 1: order[k] for k in range(1, len(order))})
    gsubrs = GlobalSubrsIndex()
    top = TopDict()
    top.GlobalSubrs = gsubrs
    top.charset = order
    top.ROS = ("Daegun", "Matrix", 0)
    top.CIDCount = len(order)
    top.FontMatrix = [0.001, 0, 0.000167, 0.001, 0, 0]
    top.FontBBox = [0, 0, 900, 900]
    fd_array = FDArrayIndex()
    privates = []
    for i, matrix in enumerate(FD_MATRICES):
        private = PrivateDict()
        private.defaultWidthX, private.nominalWidthX = 0, 0
        privates.append(private)
        fd = FontDict()
        fd.FontName = f"DaegunMatrix-FD{i}"
        if matrix:
            fd.FontMatrix = matrix
        fd.Private = private
        fd_array.append(fd)
    top.FDArray = fd_array
    select = FDSelect(format=3)
    select.gidArray = [0] + list(range(len(FD_MATRICES)))
    top.FDSelect = select
    cs = CharStrings(None, order, gsubrs, None, select, fd_array)
    for k, name in enumerate(order):
        bytecode = bytes([ENDCHAR]) if k == 0 else program(k)
        cs[name] = T2CharString(bytecode=bytecode, private=privates[select.gidArray[k]], globalSubrs=gsubrs)
    top.CharStrings = cs
    font["CFF "] = newTable("CFF ")
    font["CFF "].cff = font_set(font, top)
    font.save(out)


def name_font(out):
    order = [".notdef", "A"]
    font = shell(order, {0x41: "A"})
    gsubrs = GlobalSubrsIndex()
    private = PrivateDict()
    private.defaultWidthX, private.nominalWidthX = 0, 0
    top = TopDict()
    top.GlobalSubrs = gsubrs
    top.charset = order
    top.FontMatrix = [0.0005, 0, 0, 0.0005, 3, 4]
    top.FontBBox = [0, 0, 900, 900]
    top.Private = private
    cs = CharStrings(None, order, gsubrs, private, None, None)
    for k, name in enumerate(order):
        bytecode = bytes([ENDCHAR]) if k == 0 else program(k)
        cs[name] = T2CharString(bytecode=bytecode, private=private, globalSubrs=gsubrs)
    top.CharStrings = cs
    font["CFF "] = newTable("CFF ")
    font["CFF "].cff = font_set(font, top)
    font.save(out)


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    cid_font(os.path.join(out, "cff-matrix.otf"))
    name_font(os.path.join(out, "cff-matrix-name.otf"))
    print(f"wrote cff-matrix.otf and cff-matrix-name.otf into {out}")
