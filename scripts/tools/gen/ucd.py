# The Unicode Character Database as every generator reads it, from scripts/data/unicode.

import os
import sys

VERSION = (18, 0, 0)
DATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "data", "unicode")

def path(name):
    p = os.path.join(DATA, name)
    if not os.path.exists(p):
        sys.exit("missing {} – fetch it from https://www.unicode.org/Public/{}.{}.{}/ucd/ (some sit in its auxiliary/"
                 " and emoji/ folders), or USE's overrides from Microsoft's font-tools (USE/)".format(p, *VERSION))
    return p

def rows(name):
    with open(path(name), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if line:
                yield [p.strip() for p in line.split(";")]

def ranges(name, wanted=None, column=1):
    out = []
    for fields in rows(name):
        if len(fields) > column and (wanted is None or fields[column] == wanted):
            lo, _, hi = fields[0].partition("..")
            out.append((int(lo, 16), int(hi or lo, 16), fields[column]))
    return out

def by_code_point(name, wanted=None, column=1):
    return {cp: value for lo, hi, value in ranges(name, wanted, column) for cp in range(lo, hi + 1)}

# General category, combining class and canonical decompositions of one or two code points, by code point. A pair
# of names ending ", First>" and ", Last>" stands for every code point between them.
def unicode_data():
    gc, ccc, decomp, first = {}, {}, {}, None
    with open(path("UnicodeData.txt"), encoding="utf-8") as f:
        for line in f:
            fields = line.rstrip("\n").split(";")
            if len(fields) < 6:
                continue
            cp, name, cat = int(fields[0], 16), fields[1], fields[2]
            klass = int(fields[3]) if fields[3] else 0
            dec = fields[5].strip()
            if name.endswith(", First>"):
                first = (cp, cat, klass)
                continue
            if name.endswith(", Last>") and first is not None:
                start, cat, klass = first
                for c in range(start, cp + 1):
                    gc[c] = cat
                    if klass:
                        ccc[c] = klass
                first = None
                continue
            gc[cp] = cat
            if klass:
                ccc[cp] = klass
            if dec and not dec.startswith("<"):
                parts = [int(x, 16) for x in dec.split()]
                if len(parts) in (1, 2):
                    decomp[cp] = parts
    return gc, ccc, decomp
