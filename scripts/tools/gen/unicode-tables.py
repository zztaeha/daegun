#!/usr/bin/env python3

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import trie

VERSION = (17, 0, 0)

def read_license():
    path = os.path.join(DATA, "UnicodeLicense.txt")
    if not os.path.exists(path):
        sys.exit(
            "missing {}\n  curl -sS -o {} https://www.unicode.org/license.txt".format(path, path)
        )
    with open(path, encoding="utf-8") as f:
        return f.read().strip()
HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "..", "data", "unicode")
OUT = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "unicode_tables.rs")

GC_ORDER = [
    "Cn", "Cc", "Cf", "Co", "Cs", "Ll", "Lm", "Lo", "Lt", "Lu",
    "Mc", "Me", "Mn", "Nd", "Nl", "No", "Pc", "Pd", "Pe", "Pf",
    "Pi", "Po", "Ps", "Sc", "Sk", "Sm", "So", "Zl", "Zp", "Zs",
]
GC_INDEX = {name: i for i, name in enumerate(GC_ORDER)}

JT_U, JT_L, JT_R, JT_D, JT_ALAPH, JT_DALATH_RISH, JT_T = 0, 1, 2, 3, 4, 5, 7
JT_BY_LETTER = {"U": JT_U, "L": JT_L, "R": JT_R, "D": JT_D, "C": JT_D, "T": JT_T}

def data_path(name):
    p = os.path.join(DATA, name)
    if not os.path.exists(p):
        sys.exit("missing {} – see this file's docstring for the fetch command".format(p))
    return p

def parse_semicolon_ranges(name, wanted=None, value_col=1):
    out = []
    with open(data_path(name), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [p.strip() for p in line.split(";")]
            if len(parts) <= value_col:
                continue
            value = parts[value_col]
            if wanted is not None and value != wanted:
                continue
            rng = parts[0]
            if ".." in rng:
                lo, hi = rng.split("..")
            else:
                lo = hi = rng
            out.append((int(lo, 16), int(hi, 16), value))
    return out

def parse_unicode_data():
    gc = {}
    ccc = {}
    decomp = {}
    pending_first = None

    with open(data_path("UnicodeData.txt"), encoding="utf-8") as f:
        for line in f:
            fields = line.rstrip("\n").split(";")
            if len(fields) < 6:
                continue
            cp = int(fields[0], 16)
            name = fields[1]
            cat = fields[2]
            klass = int(fields[3]) if fields[3] else 0
            dec = fields[5].strip()

            if name.endswith(", First>"):
                pending_first = (cp, cat, klass)
                continue
            if name.endswith(", Last>") and pending_first is not None:
                start, scat, sklass = pending_first
                for c in range(start, cp + 1):
                    gc[c] = scat
                    if sklass:
                        ccc[c] = sklass
                pending_first = None
                continue

            gc[cp] = cat
            if klass:
                ccc[cp] = klass
            if dec and not dec.startswith("<"):
                parts = [int(x, 16) for x in dec.split()]
                if len(parts) in (1, 2):
                    decomp[cp] = parts
    return gc, ccc, decomp

def to_ranges(by_cp, default, encode):
    items = sorted(by_cp.items())
    ranges = []
    for cp, raw in items:
        val = encode(raw)
        if val == default:
            continue
        if ranges and ranges[-1][1] == cp - 1 and ranges[-1][2] == val:
            ranges[-1][1] = cp
        else:
            ranges.append([cp, cp, val])
    return [(a, b, c) for a, b, c in ranges]

def check_values(source, raw, known):
    unknown = sorted({v for _, _, v in raw} - set(known))
    if unknown:
        sys.exit(
            "{}: {} this generator does not number: {}\n"
            "  Add each to the matching list, and check whether the Rust side needs to know about it"
            " too – the numbering is shared with daeshaper/unicode/.".format(
                source, "a value" if len(unknown) == 1 else "values", ", ".join(unknown)))

def merge_named_ranges(raw, encode, default):
    per = {}
    for lo, hi, value in raw:
        v = encode(value)
        if v == default:
            continue
        for cp in range(lo, hi + 1):
            per[cp] = v
    items = sorted(per.items())
    ranges = []
    for cp, v in items:
        if ranges and ranges[-1][1] == cp - 1 and ranges[-1][2] == v:
            ranges[-1][1] = cp
        else:
            ranges.append([cp, cp, v])
    return [(a, b, c) for a, b, c in ranges]

TRIE_FIELDS = [
    ("script", "u16", 0xFFFF),
    ("general_category", "u8", 0),
    ("combining_class", "u8", 0),
    ("default_ignorable", "u8", 0),
    ("extended_pictographic", "u8", 0),
    ("joining_type", "u8", 0xFF),
    ("grapheme_break", "u8", 0),
    ("word_break", "u8", 0),
    ("bidi_class", "u8", 0),
    ("indic_conjunct_break", "u8", 0),
    ("line_break", "u8", 0),
    ("east_asian_width", "u8", 0),
    ("vertical_orientation", "u8", 0),
]

def emit_trie(f, columns):
    records, top, mid, leaf, leaf_bits, mid_bits = trie.build(
        columns, [default for _, _, default in TRIE_FIELDS]
    )

    f.write("""// Every per-codepoint property, in one record.
//
// All thirteen at once for a codepoint, reached by three array indexes rather than a binary search
// through each of thirteen sorted range tables.
#[derive(Clone, Copy)]
pub(crate) struct Props {
""")
    for name, ty, _ in TRIE_FIELDS:
        f.write("    pub(crate) {}: {},\n".format(name, ty))
    f.write("}\n\n")

    f.write("// The distinct property combinations Unicode actually uses. Indexed by `PROPS_LEAF`.\n")
    f.write("pub(crate) static PROPS: &[Props; {}] = &[\n".format(len(records)))
    for rec in records:
        fields = ", ".join("{}: {}".format(n, v) for (n, _, _), v in zip(TRIE_FIELDS, rec))
        f.write("    Props {{ {} }},\n".format(fields))
    f.write("];\n\n")

    f.write("// Block shape, searched for this table's data. The descent reads these, so a\n")
    f.write("// regeneration that lands on a different shape moves the lookup with it.\n")
    f.write("pub(crate) const PROPS_LEAF_BITS: u32 = {};\n".format(leaf_bits))
    f.write("pub(crate) const PROPS_MID_BITS: u32 = {};\n\n".format(mid_bits))

    f.write("// Stage 1: `cp >> {}` to a `PROPS_MID` block.\n".format(leaf_bits + mid_bits))
    trie.emit_flat(f, "PROPS_TOP", top)
    f.write("// Stage 2: a mid block plus `(cp >> {}) & {}` to a `PROPS_LEAF` block.\n"
            .format(leaf_bits, (1 << mid_bits) - 1))
    trie.emit_flat(f, "PROPS_MID", mid)
    f.write("// Stage 3: a leaf block plus `cp & {}` to a `PROPS` record.\n".format((1 << leaf_bits) - 1))
    trie.emit_flat(f, "PROPS_LEAF", leaf)

    return len(records), len(mid) >> mid_bits, len(leaf) >> leaf_bits

def bidi_long_to_short():
    out = {}
    with open(data_path("PropertyValueAliases.txt"), encoding="utf-8") as f:
        for line in f:
            parts = [p.strip() for p in line.split("#", 1)[0].split(";")]
            if len(parts) >= 3 and parts[0] == "bc":
                out[parts[2]] = parts[1]
    assert out, "PropertyValueAliases.txt lists no bidi classes – its shape changed"
    return out

def parse_bidi_missing(name):
    long_to_short = bidi_long_to_short()
    out = []
    with open(data_path(name), encoding="utf-8") as f:
        for line in f:
            if "@missing:" not in line:
                continue
            body = line.split("@missing:", 1)[1].strip()
            rng, _, value = body.partition(";")
            value = value.strip()
            if value not in long_to_short:
                continue
            lo, _, hi = rng.strip().partition("..")
            out.append((int(lo, 16), int(hi or lo, 16), long_to_short[value]))
    return out

GRAPHEME_BREAK = [
    "Other", "CR", "LF", "Control", "Extend", "ZWJ", "Regional_Indicator", "Prepend",
    "SpacingMark", "L", "V", "T", "LV", "LVT",
]
WORD_BREAK = [
    "Other", "CR", "LF", "Newline", "Extend", "ZWJ", "Regional_Indicator", "Format", "Katakana",
    "Hebrew_Letter", "ALetter", "Single_Quote", "Double_Quote", "MidNumLet", "MidLetter", "MidNum",
    "Numeric", "ExtendNumLet", "WSegSpace",
]
BIDI_CLASS = [
    "L", "R", "AL", "EN", "ES", "ET", "AN", "CS", "NSM", "BN", "B", "S", "WS", "ON",
    "LRE", "LRO", "RLE", "RLO", "PDF", "LRI", "RLI", "FSI", "PDI",
]

LINE_BREAK = [
    "XX", "AI", "AK", "AL", "AP", "AS", "B2", "BA", "BB", "BK", "CB", "CJ", "CL", "CM", "CP",
    "CR", "EB", "EM", "EX", "GL", "H2", "H3", "HH", "HL", "HY", "ID", "IN", "IS", "JL", "JT", "JV",
    "LF", "NL", "NS", "NU", "OP", "PO", "PR", "QU", "RI", "SA", "SG", "SP", "SY", "VF", "VI",
    "WJ", "ZW", "ZWJ",
]
EAST_ASIAN_WIDTH = ["N", "A", "F", "H", "Na", "W"]

VERTICAL_ORIENTATION = ["R", "Tr", "Tu", "U"]

def parse_incb(name):
    order = {"Consonant": 1, "Extend": 2, "Linker": 3}
    out = []
    with open(data_path(name), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [x.strip() for x in line.split(";")]
            if len(parts) < 3 or parts[1] != "InCB":
                continue
            code = order.get(parts[2])
            if code is None:
                continue
            lo, _, hi = parts[0].partition("..")
            out.append((int(lo, 16), int(hi or lo, 16), code))
    return out

def parse_bracket_pairs(name):
    out = []
    with open(data_path(name), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [p.strip() for p in line.split(";")]
            if len(parts) < 3:
                continue
            out.append((int(parts[0], 16), int(parts[1], 16), 1 if parts[2] == "o" else 0))
    return sorted(out)

def main():
    gc_by_cp, ccc_by_cp, decomp = parse_unicode_data()

    gc_ranges = to_ranges(gc_by_cp, 0, lambda c: GC_INDEX.get(c, 0))
    ccc_ranges = to_ranges(ccc_by_cp, 0, lambda k: k)

    scripts_raw = parse_semicolon_ranges("Scripts.txt")
    script_names = sorted({v for _, _, v in scripts_raw})
    script_index = {n: i for i, n in enumerate(script_names)}
    script_ranges = merge_named_ranges(scripts_raw, lambda v: script_index[v], -1)

    iso_by_long = {}
    with open(data_path("PropertyValueAliases.txt"), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line.startswith("sc ;"):
                continue
            parts = [p.strip() for p in line.split(";")]
            if len(parts) >= 3:
                iso_by_long[parts[2]] = parts[1]
    missing = [n for n in script_names if n not in iso_by_long]
    if missing:
        sys.exit("no ISO 15924 code for: {}".format(", ".join(missing)))
    script_isos = [iso_by_long[n] for n in script_names]

    ignorable_raw = parse_semicolon_ranges(
        "DerivedCoreProperties.txt", wanted="Default_Ignorable_Code_Point"
    )
    ignorable_ranges = merge_named_ranges(ignorable_raw, lambda _v: 1, -1)

    pictographic_raw = parse_semicolon_ranges("emoji-data.txt", wanted="Extended_Pictographic")
    pictographic_ranges = merge_named_ranges(pictographic_raw, lambda _v: 1, -1)

    joining = {}
    with open(data_path("ArabicShaping.txt"), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [p.strip() for p in line.split(";")]
            if len(parts) < 4:
                continue
            group = parts[3]
            if group == "ALAPH":
                jt = JT_ALAPH
            elif group == "DALATH RISH":
                jt = JT_DALATH_RISH
            else:
                jt = JT_BY_LETTER.get(parts[2], JT_U)
            joining[int(parts[0], 16)] = jt
    jt_ranges = to_ranges(joining, 255, lambda v: v)

    gcb_index = {name: i for i, name in enumerate(GRAPHEME_BREAK)}
    gcb_raw = parse_semicolon_ranges("GraphemeBreakProperty.txt")
    check_values("GraphemeBreakProperty.txt", gcb_raw, GRAPHEME_BREAK)
    gcb_ranges = merge_named_ranges(gcb_raw, lambda v: gcb_index.get(v, 0), 0)

    wb_index = {name: i for i, name in enumerate(WORD_BREAK)}
    wb_raw = parse_semicolon_ranges("WordBreakProperty.txt")
    check_values("WordBreakProperty.txt", wb_raw, WORD_BREAK)
    wb_ranges = merge_named_ranges(wb_raw, lambda v: wb_index.get(v, 0), 0)

    bc_index = {name: i for i, name in enumerate(BIDI_CLASS)}
    bc_long_to_short = bidi_long_to_short()
    bc_per = {}
    for lo, hi, short in parse_bidi_missing("DerivedBidiClass.txt"):
        if short == "L":
            continue  # 0 is the table default; storing it would just bloat the ranges
        for cp in range(lo, hi + 1):
            bc_per[cp] = bc_index[short]
    for lo, hi, value in parse_semicolon_ranges("DerivedBidiClass.txt"):
        short = bc_long_to_short.get(value, value)
        code = bc_index.get(short)
        if code is None:
            continue
        for cp in range(lo, hi + 1):
            if code == 0:
                bc_per.pop(cp, None)
            else:
                bc_per[cp] = code
    bc_ranges = to_ranges(bc_per, 0, lambda v: v)

    bracket_pairs = parse_bracket_pairs("BidiBrackets.txt")

    lb_index = {name: i for i, name in enumerate(LINE_BREAK)}
    lb_raw = parse_semicolon_ranges("LineBreak.txt")
    check_values("LineBreak.txt", lb_raw, LINE_BREAK)
    lb_ranges = merge_named_ranges(lb_raw, lambda v: lb_index.get(v, 0), 0)

    eaw_index = {name: i for i, name in enumerate(EAST_ASIAN_WIDTH)}
    eaw_raw = parse_semicolon_ranges("EastAsianWidth.txt")
    check_values("EastAsianWidth.txt", eaw_raw, EAST_ASIAN_WIDTH)
    eaw_ranges = merge_named_ranges(eaw_raw, lambda v: eaw_index.get(v, 0), 0)

    vo_index = {name: i for i, name in enumerate(VERTICAL_ORIENTATION)}
    vo_raw = parse_semicolon_ranges("VerticalOrientation.txt")
    check_values("VerticalOrientation.txt", vo_raw, VERTICAL_ORIENTATION)
    vo_ranges = merge_named_ranges(vo_raw, lambda v: vo_index.get(v, 0), 0)

    incb_ranges = merge_named_ranges(parse_incb("DerivedCoreProperties.txt"), lambda v: v, 0)

    mirroring = []
    with open(data_path("BidiMirroring.txt"), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [p.strip() for p in line.split(";")]
            if len(parts) < 2 or not parts[1]:
                continue
            mirroring.append((int(parts[0], 16), int(parts[1].split()[0], 16)))
    mirroring.sort()

    exclusions = set()
    with open(data_path("CompositionExclusions.txt"), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if line:
                exclusions.add(int(line.split()[0], 16))

    decomp_pairs = sorted((cp, p) for cp, p in decomp.items() if len(p) == 2)
    decomp_singles = sorted((cp, p[0]) for cp, p in decomp.items() if len(p) == 1)

    comp = []
    for cp, (a, b) in decomp_pairs:
        if cp in exclusions or ccc_by_cp.get(a, 0) != 0:
            continue
        comp.append((a, b, cp))
    comp.sort()

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("// GENERATED FILE – do not edit by hand.\n")
        f.write("// Regenerate with: python3 scripts/tools/gen/unicode-tables.py\n")
        f.write("// Source: Unicode Character Database {}.{}.{}\n".format(*VERSION))
        f.write("//\n")
        f.write("// The notice below is required by the license the data is used under, and is\n")
        f.write("// reproduced verbatim from scripts/data/unicode/UnicodeLicense.txt. It covers the\n")
        f.write("// Unicode data and the tables derived from it, not daegun's own code, which is\n")
        f.write("// MIT (see LICENSE).\n")
        f.write("//\n")
        for line in read_license().splitlines():
            f.write("//\n" if not line.strip() else "// {}\n".format(line))
        f.write("\n")
        f.write("pub(crate) static SCRIPT_NAMES: &[&str] = &[\n")
        for n in script_names:
            f.write('    "{}",\n'.format(n))
        f.write("];\n\n")

        f.write("// The two scripts itemization treats as taking their script from context.\n")
        f.write("// Emitted rather than searched for: `SCRIPT_NAMES` is sorted, so these indices move\n")
        f.write("// whenever Unicode adds a script, and comparing two `u16`s beats the string\n")
        f.write("// comparisons they replace, once per character.\n")
        for name in ("Common", "Inherited"):
            f.write("pub(crate) const SCRIPT_{}: u16 = {};\n".format(name.upper(), script_index[name]))
        f.write("\n")

        f.write("// ISO 15924 short codes, parallel to SCRIPT_NAMES. OpenType tags derive from these.\n")
        f.write("pub(crate) static SCRIPT_ISO_CODES: &[&str] = &[\n")
        for c in script_isos:
            f.write('    "{}",\n'.format(c))
        f.write("];\n\n")

        trie_stats = emit_trie(f, [
            script_ranges, gc_ranges, ccc_ranges, ignorable_ranges, pictographic_ranges,
            jt_ranges, gcb_ranges, wb_ranges, bc_ranges, incb_ranges, lb_ranges,
            eaw_ranges, vo_ranges,
        ])

        f.write("// `(codepoint, paired codepoint, 1 if opening)`, sorted, for UAX #9's N0.\n")
        f.write("pub(crate) static BIDI_BRACKETS: &[(u32, u32, u8)] = &[\n")
        for cp, paired, is_open in bracket_pairs:
            f.write("    (0x{:04X}, 0x{:04X}, {}),\n".format(cp, paired, is_open))
        f.write("];\n\n")

        pair_bmp = [(cp, a, b) for cp, (a, b) in decomp_pairs if cp <= 0xFFFF]
        pair_supp = [(cp, a, b) for cp, (a, b) in decomp_pairs if cp > 0xFFFF]
        assert all(a <= 0xFFFF and b <= 0xFFFF for _, a, b in pair_bmp), \
            "a BMP-keyed pair decomposes to a supplementary codepoint; DECOMPOSE_PAIR_BMP can no longer be u16"
        assert [r[0] for r in pair_bmp] + [r[0] for r in pair_supp] == [cp for cp, _ in decomp_pairs], \
            "DECOMPOSE_PAIR's BMP rows are no longer a sorted prefix; COMPOSE_INDEX would name the wrong entries"

        f.write("// Canonical pair decompositions, composed form and both parts all in the BMP.\n")
        f.write("// Half the width of a `u32` triple, and it holds all but 46 of the rows.\n")
        f.write("pub(crate) static DECOMPOSE_PAIR_BMP: &[(u16, u16, u16)] = &[\n")
        for cp, a, b in pair_bmp:
            f.write("    (0x{:04X}, 0x{:04X}, 0x{:04X}),\n".format(cp, a, b))
        f.write("];\n\n")

        f.write("// The rest, where something does not fit a `u16`. Logically this continues\n")
        f.write("// `DECOMPOSE_PAIR_BMP`: index `n` here is index `DECOMPOSE_PAIR_BMP.len() + n`\n")
        f.write("// to `COMPOSE_INDEX`.\n")
        f.write("pub(crate) static DECOMPOSE_PAIR_SUPP: &[(u32, u32, u32)] = &[\n")
        for cp, a, b in pair_supp:
            f.write("    (0x{:05X}, 0x{:05X}, 0x{:05X}),\n".format(cp, a, b))
        f.write("];\n\n")

        cjk_base, cjk_last = 0x2F800, 0x2FA1D
        cjk = [(cp, a) for cp, a in decomp_singles if cjk_base <= cp <= cjk_last]
        rest = [(cp, a) for cp, a in decomp_singles if not cjk_base <= cp <= cjk_last]
        assert [cp for cp, _ in cjk] == list(range(cjk_base, cjk_last + 1)), \
            "the CJK compatibility block is no longer dense; DECOMPOSE_SINGLE_CJK cannot be indexed by codepoint"
        assert all(cp <= 0xFFFF for cp, _ in rest), \
            "a supplementary singleton decomposition appeared outside the CJK compatibility block"

        f.write("// Singleton decompositions of the CJK compatibility ideographs, indexed by\n")
        f.write("// `codepoint - DECOMPOSE_SINGLE_CJK_BASE`. The block is dense, so keys would be\n")
        f.write("// half the table and every one derivable: without them a bisection is an index.\n")
        f.write("pub(crate) const DECOMPOSE_SINGLE_CJK_BASE: u32 = 0x{:05X};\n".format(cjk_base))
        f.write("pub(crate) static DECOMPOSE_SINGLE_CJK: &[u32; {}] = &[\n".format(len(cjk)))
        for i in range(0, len(cjk), 8):
            f.write("    " + " ".join("0x{:04X},".format(a) for _, a in cjk[i:i + 8]) + "\n")
        f.write("];\n\n")

        f.write("// Every other singleton decomposition. Keys are all BMP; a few values are not.\n")
        f.write("pub(crate) static DECOMPOSE_SINGLE: &[(u32, u32)] = &[\n")
        for cp, a in rest:
            f.write("    (0x{:04X}, 0x{:04X}),\n".format(cp, a))
        f.write("];\n\n")

        where = {(cp, a, b): i for i, (cp, (a, b)) in enumerate(decomp_pairs)}
        index = []
        for a, b, cp in comp:
            assert (cp, a, b) in where, \
                "compose (%04X,%04X)->%04X is not in DECOMPOSE_PAIR" % (a, b, cp)
            index.append(where[(cp, a, b)])
        assert max(index) <= 0xFFFF, "DECOMPOSE_PAIR outgrew a u16 index"

        f.write("// Indices into `DECOMPOSE_PAIR`, ordered by that entry's `(a, b)` so a search by\n")
        f.write("// composition can bisect. Composition *is* decomposition read backwards; storing\n")
        f.write("// the pairs a second time cost 12 bytes an entry to say nothing new.\n")
        f.write("pub(crate) static COMPOSE_INDEX: &[u16; {}] = &[\n".format(len(index)))
        for i in range(0, len(index), 16):
            f.write("    " + " ".join("{},".format(v) for v in index[i:i + 16]) + "\n")
        f.write("];\n\n")

        assert all(a <= 0xFFFF and b <= 0xFFFF for a, b in mirroring), \
            "a mirrored pair left the BMP; MIRRORING can no longer be u16"
        f.write("// `(codepoint, its mirror)`, sorted. Both fit a `u16`: mirroring is a BMP-only\n")
        f.write("// property, which halves the table against a `u32` pair.\n")
        f.write("pub(crate) static MIRRORING: &[(u16, u16)] = &[\n")
        for a, b in mirroring:
            f.write("    (0x{:04X}, 0x{:04X}),\n".format(a, b))
        f.write("];\n")

    print("wrote {}".format(os.path.relpath(OUT)))
    print("  fused trie       {} records, {} mid blocks, {} leaf blocks".format(*trie_stats))
    print("  general category {} ranges".format(len(gc_ranges)))
    print("  combining class  {} ranges".format(len(ccc_ranges)))
    print("  script           {} ranges over {} scripts".format(len(script_ranges), len(script_names)))
    print("  ignorable        {} ranges".format(len(ignorable_ranges)))
    print("  pictographic     {} ranges".format(len(pictographic_ranges)))
    print("  joining type     {} ranges".format(len(jt_ranges)))
    print("  grapheme break   {} ranges".format(len(gcb_ranges)))
    print("  word break       {} ranges".format(len(wb_ranges)))
    print("  bidi class       {} ranges".format(len(bc_ranges)))
    print("  bidi brackets    {} pairs".format(len(bracket_pairs)))
    print("  indic conjunct   {} ranges".format(len(incb_ranges)))
    print("  line break       {} ranges".format(len(lb_ranges)))
    print("  east asian width {} ranges".format(len(eaw_ranges)))
    print("  vertical orient  {} ranges".format(len(vo_ranges)))
    print("  decompose        {} pairs, {} singletons".format(len(decomp_pairs), len(decomp_singles)))
    print("  compose          {} pairs".format(len(comp)))
    print("  mirroring        {} pairs".format(len(mirroring)))

if __name__ == "__main__":
    main()
