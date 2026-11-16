#!/usr/bin/env python3
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.misc.psCharStrings import T2CharString, encodeFixed, encodeIntT2

UPM = 1000

# Each glyph is raw Type 2 bytecode, holding what no pen writes: arithmetic and storage operators, a reserved escape,
# odd stems and masks. The Private DICT's initialRandomSeed is negative, read as its integer's low 32 bits.
def code(*tokens):
    out = bytearray()
    for t in tokens:
        if isinstance(t, bytes):
            out += t
        elif isinstance(t, str):
            out += bytes(T2CharString.opcodes[t])
        elif isinstance(t, float):
            out += encodeFixed(t)
        else:
            out += encodeIntT2(t)
    return bytes(out)

START = (0, 0, "rmoveto")
STEMS8 = tuple([0, 10] + [10, 10] * 7)

GLYPHS = {
    # 200/3 and 2/3 of 300 as 16.16 rounds them, and sqrt(2) and sqrt(3) as its square root rounds them.
    "div": code(*START, 200, 3, "div", 0, "rlineto", 2, 3, "div", 300, "mul", 300, "rlineto", -100, 0, "rlineto",
                "endchar"),
    "sqrt": code(*START, 2, "sqrt", 100, "mul", 0, "rlineto", 0, 3, "sqrt", 100, "mul", "rlineto", -100, 0,
                 "rlineto", "endchar"),
    # A zero divisor gives the largest 16.16 value, with the dividend's sign.
    "divzero": code(*START, 1, 0, "div", 0.001, "mul", 0, "rlineto", 0, 300, "rlineto", "endchar"),
    "absneg": code(*START, -250, "abs", 0, "rlineto", 0, 250, "neg", "neg", "rlineto", -125, "neg", "neg", -125,
                   "abs", "neg", "rlineto", "endchar"),
    "logic": code(*START, 1, 2, "and", 0, 3, "or", "add", 0, "not", "add", 4, 4, "eq", "add", 100, "mul", 0,
                  "rlineto", 0, 300, "rlineto", "endchar"),
    "ifelse": code(*START, 300, 100, 1, 2, "ifelse", 0, "rlineto", 0, 100, 300, 2, 1, "ifelse", "rlineto",
                   "endchar"),
    # get past the 32 slots pushes nothing.
    "storage": code(*START, 250, 3, "put", 125, 31, "put", 3, "get", 0, "rlineto", 0, 31, "get", "rlineto", 0, 32,
                    "get", 0, "rlineto", "endchar"),
    "stack": code(*START, 10, 20, 30, 2, "index", "add", "exch", "drop", "dup", "add", "rlineto", 1, 2, 3, 3, 1,
                  "roll", "sub", "sub", 100, "mul", 300, "rlineto", "endchar"),
    # index past the stack copies its bottom; roll past it fails the glyph.
    "indexpast": code(*START, 150, 250, 5, "index", "add", "rlineto", 0, 300, "rlineto", "endchar"),
    "rollpast": code(*START, 100, 200, 5, 1, "roll", "rlineto", 0, 300, "rlineto", "endchar"),
    # An integer from eq and a whole 16.16 from add both index the stack, by their value.
    "intindex": code(*START, 100, 200, 0, 0, "eq", "index", "drop", "rlineto", 0, 300, "rlineto", "endchar"),
    "fixedindex": code(*START, 100, 200, 0, 1, "add", "index", "drop", "rlineto", 0, 300, "rlineto", "endchar"),
    "random": code(*START, "random", 300, "mul", 0, "rlineto", 0, "random", 300, "mul", "rlineto", "endchar"),
    # A reserved escape clears the stack and drawing goes on.
    "reserved": code(*START, 100, 200, b"\x0c\x08", 300, 0, "rlineto", 0, 300, "rlineto", "endchar"),
    # Eight stems, a mask, then a ninth: the next mask names only the first eight, in one byte.
    "latestems": code(*STEMS8, "hstemhm", "hintmask", b"\xff", 0, 50, "vstemhm", "hintmask", b"\x80", *START, 300, 0,
                      "rlineto", 0, 300, "rlineto", -300, 0, "rlineto", "endchar"),
    # 98 stems, past the 96 a hint mask may name.
    "manystems": code(*([0, 5] + [5, 5] * 22), "hstemhm", *(([5, 5] * 24 + ["hstemhm"]) * 3), *([5, 5] * 3),
                      "hstemhm", "hintmask", b"\xff" * 13, *START, 300, 0, "rlineto", 0, 300, "rlineto", "endchar"),
    # 96 stems, as many as a mask may name: the control.
    "maxstems": code(*([0, 5] + [5, 5] * 22), "hstemhm", *(([5, 5] * 24 + ["hstemhm"]) * 3), *([5, 5]),
                     "hstemhm", "hintmask", b"\xff" * 12, *START, 300, 0, "rlineto", 0, 300, "rlineto", "endchar"),
    # A mask whose byte lies past the charstring's end.
    "shortmask": code(0, 10, 100, 20, "hstemhm", 0, 50, 300, 20, "vstemhm", "hintmask"),
    # Once the count is fixed, a mask's two operands are stems declared late and dropped; the mask still reads its byte.
    "maskargs": code(0, 10, "hstemhm", "hintmask", b"\x80", *START, 10, 20, "hintmask", 300, 0, "rlineto", 0, 300,
                     "rlineto", "endchar"),
    # 98 stems and no mask fail the glyph at its first moveto.
    "manymoves": code(*([0, 5] + [5, 5] * 22), "hstemhm", *(([5, 5] * 24 + ["hstemhm"]) * 3), *([5, 5] * 3),
                      "hstemhm", *START, 300, 0, "rlineto", 0, 300, "rlineto", "endchar"),
    # abs and neg of the one 16.16 value with no opposite give the largest.
    "absmin": code(*START, -32768.0, "abs", 0.01, "mul", 0, "rlineto", 0, -32768.0, "neg", 0.01, "mul", "rlineto",
                   "endchar"),
}

def main(out_path):
    order = [".notdef"] + list(GLYPHS)
    fb = FontBuilder(UPM, isTTF=False)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({0x41 + i: name for i, name in enumerate(order[1:])})
    charstrings = {".notdef": T2CharString(program=["endchar"])}
    charstrings.update({name: T2CharString(bytecode=b) for name, b in GLYPHS.items()})
    fb.setupCFF("DaegunCffOperators", {"FullName": "DaegunCffOperators"}, charstrings,
                {"BlueValues": [-15, 0, 500, 515], "initialRandomSeed": -5})
    fb.setupHorizontalMetrics({name: (500, 0) for name in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunCffOperators", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()
    # fontTools would draw every glyph for the font's box, and runs none of these operators.
    fb.font.recalcBBoxes = False
    fb.save(out_path)
    print(f"wrote {out_path}")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "cff-operators.otf")
