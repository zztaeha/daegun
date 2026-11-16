#!/usr/bin/env python3
# Writes assets/test-data/unicode-properties.tsv from Unicode's own data files, in hex: `p start end category class
# script` a run alike in all three, `m code mirror`, `d code a b composes` (b - for one character), `s script
# direction`. Run from the engine root once scripts/tools/gen/ucd.py has fetched the files.
import sys
from pathlib import Path

UCD = Path("scripts/data/unicode")
OUT = Path("assets/test-data/unicode-properties.tsv")
SCALARS = [*range(0xD800), *range(0xE000, 0x110000)]
# Hangul syllables decompose by arithmetic rather than by table.
S_BASE, L_BASE, V_BASE, T_BASE, T_COUNT, N_COUNT, S_COUNT = 0xAC00, 0x1100, 0x1161, 0x11A7, 28, 588, 11172


def fields(name):
    for line in (UCD / name).read_text(encoding="utf-8").splitlines():
        line = line.split("#")[0].strip()
        if line:
            yield [f.strip() for f in line.split(";")]


def span(code):
    a, _, b = code.partition("..")
    return range(int(a, 16), int(b or a, 16) + 1)


def main():
    category, ccc, decomposition = {}, {}, {}
    first = None
    for f in fields("UnicodeData.txt"):
        u = int(f[0], 16)
        if f[1].endswith(", First>"):
            first = u
            continue
        for v in range(first if f[1].endswith(", Last>") else u, u + 1):
            category[v], ccc[v] = f[2], int(f[3])
        if f[5] and not f[5].startswith("<"):
            decomposition[u] = [int(x, 16) for x in f[5].split()]
    for s in range(S_COUNT):
        lv = s % T_COUNT == 0
        decomposition[S_BASE + s] = ([L_BASE + s // N_COUNT, V_BASE + s % N_COUNT // T_COUNT] if lv
                                     else [S_BASE + s - s % T_COUNT, T_BASE + s % T_COUNT])

    iso = {f[2]: f[1] for f in fields("PropertyValueAliases.txt") if f[0] == "sc"}
    script = {}
    for f in fields("Scripts.txt"):
        for u in span(f[0]):
            script[u] = iso[f[1]]
    bidi = {}
    for f in fields("DerivedBidiClass.txt"):
        for u in span(f[0]):
            bidi[u] = f[1]

    rows, run = [], None
    for u in SCALARS:
        p = (category.get(u, "Cn"), ccc.get(u, 0), script.get(u, "Zzzz"))
        if run and run[2] == p and run[1] == u - 1:
            run[1] = u
            continue
        if run:
            rows.append(run)
        run = [u, u, p]
    rows.append(run)
    lines = [f"p\t{a:X}\t{b:X}\t{c}\t{k}\t{s}" for a, b, (c, k, s) in rows]

    mirror = {int(f[0], 16): int(f[1], 16) for f in fields("BidiMirroring.txt")}
    lines += [f"m\t{u:X}\t{mirror[u]:X}" for u in SCALARS if u in mirror]

    # A pair composes back unless excluded by name, or its composite or first part isn't a starter.
    excluded = {u for f in fields("CompositionExclusions.txt") for u in span(f[0])}
    for u in SCALARS:
        if u not in decomposition:
            continue
        parts = decomposition[u]
        a, b = parts[0], parts[1] if len(parts) > 1 else None
        hangul = S_BASE <= u < S_BASE + S_COUNT
        composes = b is not None and (hangul or (u not in excluded and ccc.get(u, 0) == 0 and ccc.get(a, 0) == 0))
        lines.append(f"d\t{u:X}\t{a:X}\t{'-' if b is None else f'{b:X}'}\t{int(composes)}")

    # A script runs right to left where its letters are strong right-to-left.
    letters = {}
    for u, s in script.items():
        if category.get(u, "Cn")[0] == "L":
            letters.setdefault(s, []).append(bidi.get(u, "L") in ("R", "AL"))
    for code in iso.values():
        if code == "Zzzz":
            continue
        rtl = letters.get(code, [])
        direction = "R" if rtl and sum(rtl) * 2 > len(rtl) else "L"
        lines.append(f"s\t{code}\t{direction}")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{len(lines)} rows -> {OUT}", file=sys.stderr)


if __name__ == "__main__":
    main()
