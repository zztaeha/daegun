#!/usr/bin/env python3

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import trie

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "..", "data", "unicode")
OUT = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "indic_tables.rs")

CATEGORIES = [
    "X", "C", "V", "N", "H", "ZWNJ", "ZWJ", "M", "SM", "A",
    "PLACEHOLDER", "DOTTEDCIRCLE", "RS", "MPst", "Repha", "Ra", "CM", "Symbol", "CS",
    "VAbv", "VBlw", "VPre", "VPst", "VS", "MW", "MY", "MR", "MH", "ML",
    "PT", "As", "Robatic", "Xgroup", "Ygroup",
    "SMPst",
]
CATEGORY_INDEX = {name: i for i, name in enumerate(CATEGORIES)}

POSITIONS = [
    "START", "RA_TO_BECOME_REPH", "PRE_M", "PRE_C",
    "BASE_C", "AFTER_MAIN", "ABOVE_C",
    "BEFORE_SUB", "BELOW_C", "AFTER_SUB",
    "BEFORE_POST", "POST_C", "AFTER_POST",
    "SMVD", "END",
]
POSITION_INDEX = {name: i for i, name in enumerate(POSITIONS)}

FROM_SYLLABIC = {
    "Avagraha": "Symbol",
    "Bindu": "SM",
    "Brahmi_Joining_Number": "PLACEHOLDER",
    "Cantillation_Mark": "A",
    "Consonant": "C",
    "Consonant_Dead": "C",
    "Consonant_Final": "CM",
    "Consonant_Head_Letter": "C",
    "Consonant_Initial_Postfixed": "C",
    "Consonant_Killer": "Xgroup",
    "Consonant_Medial": "CM",
    "Consonant_Placeholder": "PLACEHOLDER",
    "Consonant_Preceding_Repha": "Repha",
    "Consonant_Prefixed": "X",
    "Consonant_Subjoined": "CM",
    "Consonant_Succeeding_Repha": "Robatic",
    "Consonant_With_Stacker": "CS",
    "Gemination_Mark": "SM",
    "Invisible_Stacker": "H",
    "Joiner": "ZWJ",
    "Modifying_Letter": "X",
    "Non_Joiner": "ZWNJ",
    "Nukta": "N",
    "Number": "PLACEHOLDER",
    "Number_Joiner": "PLACEHOLDER",
    "Other": "X",
    "Pure_Killer": "M",
    "Register_Shifter": "Robatic",
    "Syllable_Modifier": "SM",
    "Tone_Letter": "X",
    "Tone_Mark": "SM",
    "Virama": "H",
    "Visarga": "SM",
    "Vowel": "V",
    "Vowel_Dependent": "M",
    "Vowel_Independent": "V",
}

def cps(*items):
    out = set()
    for item in items:
        if isinstance(item, tuple):
            out.update(range(item[0], item[1] + 1))
        else:
            out.add(item)
    return out

OVERRIDES = [
    ("Ra", cps(
        0x0930,  # DEVANAGARI LETTER RA
        0x09B0,  # BENGALI LETTER RA
        0x09F0,  # BENGALI LETTER RA WITH MIDDLE DIAGONAL
        0x0A30,  # GURMUKHI LETTER RA
        0x0AB0,  # GUJARATI LETTER RA
        0x0B30,  # ORIYA LETTER RA
        0x0BB0,  # TAMIL LETTER RA
        0x0C30,  # TELUGU LETTER RA
        0x0CB0,  # KANNADA LETTER RA
        0x0D30,  # MALAYALAM LETTER RA
        0x1004,  # MYANMAR LETTER NGA
        0x101B,  # MYANMAR LETTER RA
        0x105A,  # MYANMAR LETTER MON NGA
        0x179A,  # KHMER LETTER RO
    )),

    ("VS", cps((0xFE00, 0xFE0F))),

    ("VPre", cps(
        0x1031,          # MYANMAR VOWEL SIGN E
        0x1084,          # MYANMAR VOWEL SIGN SHAN E
        (0x17C1, 0x17C3),  # KHMER VOWEL SIGN E, AE, AI
    )),
    ("VAbv", cps(
        0x102D, 0x102E,  # MYANMAR VOWEL SIGN I, II
        (0x1033, 0x1035),  # MYANMAR VOWEL SIGN MON II, MON O, E ABOVE
        (0x1071, 0x1074),  # MYANMAR VOWEL SIGN GEBA KAREN I .. KAYAH II
        0x1085, 0x1086,  # MYANMAR VOWEL SIGN SHAN E ABOVE, SHAN FINAL Y
        0x109D,          # MYANMAR VOWEL SIGN AITON AI
        (0x17B7, 0x17BA),  # KHMER VOWEL SIGN I, II, Y, YY
        0x17BE,          # KHMER VOWEL SIGN OE
        0xA9E5,          # MYANMAR SIGN SHAN SAW
    )),
    ("VBlw", cps(
        0x102F, 0x1030,  # MYANMAR VOWEL SIGN U, UU
        0x1058, 0x1059,  # MYANMAR VOWEL SIGN VOCALIC L, LL
        (0x17BB, 0x17BD),  # KHMER VOWEL SIGN U, UU, UA
    )),
    ("VPst", cps(
        0x102B, 0x102C,  # MYANMAR VOWEL SIGN TALL AA, AA
        0x1056, 0x1057,  # MYANMAR VOWEL SIGN VOCALIC R, RR
        0x1062,          # MYANMAR VOWEL SIGN SGAW KAREN EU
        0x1067, 0x1068,  # MYANMAR VOWEL SIGN WESTERN PWO KAREN EU, UE
        0x1083,          # MYANMAR VOWEL SIGN SHAN AA
        0x17B6,          # KHMER VOWEL SIGN AA
        0x17BF, 0x17C0,  # KHMER VOWEL SIGN YA, IE
        0x17C4, 0x17C5,  # KHMER VOWEL SIGN OO, AU
    )),

    ("MW", cps(0x103D, 0x1082)),  # MEDIAL WA, SHAN MEDIAL WA
    ("MY", cps(0x103B, 0x105E, 0x105F)),  # MEDIAL YA, MON MEDIAL NA, MON MEDIAL MA
    ("MR", cps(0x103C)),  # MEDIAL RA
    ("MH", cps(0x103E, 0x1060)),  # MEDIAL HA, MON MEDIAL LA
    ("ML", cps(0x1060)),  # MON MEDIAL LA
    ("As", cps(0x103A)),  # MYANMAR SIGN ASAT, which kills the vowel of its consonant
    ("PT", cps(
        0x1063, 0x1064,  # MYANMAR TONE MARK SGAW KAREN HATHI, KE PHO
        (0x1069, 0x106D),  # MYANMAR SIGN WESTERN PWO KAREN TONE-1 .. TONE-5
        0xAA7B,          # MYANMAR SIGN PAO KAREN TONE
    )),

    ("Xgroup", cps(
        0x17C6,          # KHMER SIGN NIKAHIT
        0x17CB,          # KHMER SIGN BANTOC
        (0x17CE, 0x17D1),  # KHMER SIGN KAKABAT, AHSDA, SAMYOK SANNYA, VIRIAM
    )),
    ("Ygroup", cps(
        0x17C7, 0x17C8,  # KHMER SIGN REAHMUK, YUUKALEAPINTU
        0x17D3,          # KHMER SIGN BATHAMASAT
        0x17DD,          # KHMER SIGN ATTHACAN
    )),

    ("Symbol", cps(
        (0x1CE9, 0x1CEC),  # VEDIC SIGN ANUSVARA ANTARGOMUKHA .. VAMAGOMUKHA WITH TAIL
        (0x1CEE, 0x1CF1),  # VEDIC SIGN HEXIFORM LONG ANUSVARA .. ANUSVARA UBHAYATO MUKHA
        (0xA8F2, 0xA8F7),  # DEVANAGARI SIGN SPACING CANDRABINDU .. SIGN CANDRABINDU AVAGRAHA
    )),
    ("A", cps(
        0x1032, 0x1036,    # MYANMAR VOWEL SIGN AI, SIGN ANUSVARA
        (0x1CE2, 0x1CE8),  # VEDIC SIGN VISARGA SVARITA .. VISARGA ANUDATTA WITH TAIL
        0x1CED,            # VEDIC SIGN TIRYAK
    )),

    ("PLACEHOLDER", cps(
        0x09FC,  # BENGALI LETTER VEDIC ANUSVARA
        0x0C80,  # TELUGU SIGN COMBINING ANUSVARA ABOVE
        0x0D04,  # MALAYALAM LETTER VEDIC ANUSVARA
        0x104A,  # MYANMAR SIGN LITTLE SECTION
        0x17D9,  # KHMER SIGN PHNAEK MUAN
        0x2015,  # HORIZONTAL BAR
        0x2022,  # BULLET
        (0x25FB, 0x25FE),  # WHITE/BLACK MEDIUM SQUARE and SMALL SQUARE
    )),
    ("DOTTEDCIRCLE", cps(0x25CC)),

    ("C", cps(
        0x0A72, 0x0A73,  # GURMUKHI IRI, URA – bases in their own right
        0x104E,          # MYANMAR SYMBOL AFOREMENTIONED
    )),
    ("N", cps(
        0x0AFB,  # GUJARATI SIGN SHADDA
        0x0B55,  # ORIYA SIGN OVERLINE
        0x1037,  # MYANMAR SIGN DOT BELOW
        0xAA7C, 0xAA7D,  # MYANMAR SIGN TAI LAING TONE-2, TONE-5
    )),
    ("SM", cps(
        0x0953, 0x0954,  # DEVANAGARI GRAVE and ACUTE ACCENT
        0x109C,          # MYANMAR VOWEL SIGN AITON A
    )),
    ("MPst", cps(0x0A40)),  # GURMUKHI VOWEL SIGN II, which follows its consonant
    ("M", cps(0x0A51)),     # GURMUKHI SIGN UDAAT
    ("CM", cps(0x0A75)),    # GURMUKHI SIGN YAKASH

    ("X", cps(
        0x11300, 0x11305, 0x11306, 0x11307,
        0x11338, 0x11339, 0x1133D, 0x1133E, 0x1133F,
    )),
]

def load_ranges(name):
    out = {}
    path = os.path.join(DATA, name)
    if not os.path.exists(path):
        sys.exit("missing {} – see this file's docstring".format(path))
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            rng, value = (p.strip() for p in line.split(";"))
            lo, hi = rng.split("..") if ".." in rng else (rng, rng)
            for cp in range(int(lo, 16), int(hi, 16) + 1):
                out[cp] = value
    return out

def load_blocks():
    out = []
    with open(os.path.join(DATA, "Blocks.txt"), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            rng, name = (p.strip() for p in line.split(";"))
            lo, hi = rng.split("..")
            out.append((int(lo, 16), int(hi, 16), name))
    return out

def load_general_categories():
    out, pending = {}, None
    for line in open(os.path.join(DATA, "UnicodeData.txt"), encoding="utf-8"):
        fields = line.split(";")
        if len(fields) < 3:
            continue
        cp, name, gc = int(fields[0], 16), fields[1], fields[2]
        if name.endswith(", First>"):
            pending = (cp, gc)
        elif name.endswith(", Last>") and pending:
            out.update(dict.fromkeys(range(pending[0], cp + 1), pending[1]))
            pending = None
        else:
            out[cp] = gc
    return out

def build():
    syllabic = load_ranges("IndicSyllabicCategory.txt")
    positional = load_ranges("IndicPositionalCategory.txt")
    blocks = load_blocks()

    def block_of(cp):
        for lo, hi, name in blocks:
            if lo <= cp <= hi:
                return name
        return "?"

    interesting = set(syllabic) | set(positional)
    for _, group in OVERRIDES:
        interesting |= group

    general = load_general_categories()

    categories = {}
    for cp in interesting:
        categories[cp] = FROM_SYLLABIC.get(syllabic.get(cp, "Other"), "X")
        if categories[cp] == "SM" and general.get(cp, "").startswith("N"):
            categories[cp] = "SMPst"
    for name, group in OVERRIDES:
        for cp in group:
            categories[cp] = name

    positions = {cp: position_of(cp, categories[cp], positional.get(cp, "NA"), block_of(cp))
                 for cp in interesting}

    return categories, positions

MATRA_POSITIONS = {
    "Devanagari":          {"Left": "PRE_M", "Top": "AFTER_SUB", "Bottom": "AFTER_SUB", "Right": "AFTER_SUB"},
    "Devanagari Extended": {"Top": "AFTER_SUB"},
    "Bengali":             {"Left": "PRE_M", "Bottom": "AFTER_SUB", "Right": "AFTER_POST",
                            "Left_And_Right": "AFTER_POST"},
    "Gujarati":            {"Left": "PRE_M", "Top": "AFTER_SUB", "Bottom": "AFTER_POST",
                            "Right": "AFTER_POST", "Top_And_Right": "AFTER_POST"},
    "Gurmukhi":            {"Left": "PRE_M", "Top": "AFTER_POST", "Bottom": "AFTER_POST",
                            "Right": "AFTER_POST"},
    "Oriya":               {"Left": "PRE_M", "Top": "AFTER_MAIN", "Bottom": "AFTER_SUB",
                            "Right": "AFTER_POST", "Left_And_Right": "AFTER_POST",
                            "Top_And_Right": "AFTER_POST", "Top_And_Left": "AFTER_MAIN",
                            "Top_And_Left_And_Right": "AFTER_POST"},
    "Tamil":               {"Left": "PRE_M", "Top": "AFTER_SUB", "Right": "AFTER_POST",
                            "Left_And_Right": "AFTER_POST"},
    "Telugu":              {"Top": "BEFORE_SUB", "Bottom": "BEFORE_SUB", "Right": "BEFORE_SUB",
                            "Top_And_Bottom": "BEFORE_SUB"},
    "Kannada":             {"Top": "BEFORE_SUB", "Bottom": "BEFORE_SUB", "Right": "AFTER_SUB",
                            "Top_And_Right": "AFTER_SUB"},
    "Malayalam":           {"Left": "PRE_M", "Top": "AFTER_SUB", "Bottom": "AFTER_POST",
                            "Right": "AFTER_POST", "Left_And_Right": "AFTER_POST"},
}

def position_of(cp, category, positional, block):
    if cp == 0x0D4E:
        return "END"
    if category in ("C", "Ra", "PLACEHOLDER", "DOTTEDCIRCLE", "V", "CS", "Repha"):
        return "BASE_C"
    if category in ("Xgroup", "Ygroup"):
        return "END"
    if category == "VS":
        return "END"
    if category in ("SM", "SMPst", "A", "Symbol"):
        if category == "SM" and block == "Oriya" and positional == "Top":
            return "BEFORE_SUB"
        return "SMVD"
    if category == "H":
        return {"Bottom": "BELOW_C", "Top": "ABOVE_C"}.get(positional, "END")
    if category == "CM":
        return "BASE_C" if block == "Gurmukhi" else "END"
    if category == "VPre":
        return "PRE_C"
    if category == "VAbv":
        return "ABOVE_C"
    if category == "VBlw":
        return "BELOW_C"
    if category == "VPst":
        return "POST_C"

    if category in ("M", "MPst"):
        exceptions = {
            0x0A51: "BELOW_C",   # GURMUKHI SIGN UDAAT, which is written under the base
            0x0C43: "AFTER_SUB", # TELUGU VOWEL SIGN VOCALIC R
            0x0C44: "AFTER_SUB", # TELUGU VOWEL SIGN VOCALIC RR
            0x0CBE: "BEFORE_SUB",# KANNADA VOWEL SIGN AA
            0x0CC0: "BEFORE_SUB",# KANNADA VOWEL SIGN II
            0x0CC1: "BEFORE_SUB",# KANNADA VOWEL SIGN U
            0x0CC2: "BEFORE_SUB",# KANNADA VOWEL SIGN UU
        }
        if cp in exceptions:
            return exceptions[cp]
        return MATRA_POSITIONS.get(block, {}).get(positional, "AFTER_SUB")

    return "END"

def emit(categories, positions):
    both = {}
    for cp in categories:
        both[cp] = (CATEGORY_INDEX[categories[cp]], POSITION_INDEX[positions[cp]])

    default = (CATEGORY_INDEX["X"], POSITION_INDEX["END"])
    ranges = []
    for cp in sorted(both):
        value = both[cp]
        if value == default:
            continue
        if ranges and ranges[-1][1] == cp - 1 and ranges[-1][2] == value:
            ranges[-1][1] = cp
        else:
            ranges.append([cp, cp, value])

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("// Generated by scripts/tools/gen/indic-table.py. Do not edit.\n")
        f.write("//\n")
        f.write("// The syllabic category and matra position of every codepoint the Indic, Khmer\n")
        f.write("// and Myanmar shapers may meet. A codepoint in no range is `X` at position `END`,\n")
        f.write("// which is what everything outside these scripts is.\n")
        f.write("//\n")
        f.write("// Derived in part from the Unicode Character Database. Copyright (c) Unicode,\n")
        f.write("// Inc.; the license notice is reproduced in full at the top of\n")
        f.write("// src/daecore/src/daeshaper/generated/unicode_tables.rs.\n")
        f.write("\n")
        cats = [(lo, hi, cat) for lo, hi, (cat, _) in ranges]
        poss = [(lo, hi, pos) for lo, hi, (_, pos) in ranges]
        stats = trie.emit(
            f, "INDIC_CATEGORY", [("category", "u8"), ("position", "u8")], [cats, poss],
            [CATEGORY_INDEX["X"], POSITION_INDEX["END"]],
            "// A codepoint's Indic category and position, behind a three-stage trie.\n"
            "//\n"
            "// One descent answers both, where two values in one sorted range list would be bisected\n"
            "// eleven deep, once per character.\n",
        )
        print("  indic trie      {} records, {} mid blocks, {} leaf blocks".format(*stats))

    return len(ranges)

def emit_vowel_constraints():
    path = os.path.join(DATA, "IndicShapingInvalidCluster.txt")
    if not os.path.exists(path):
        sys.exit(
            "missing {} – fetch it from Microsoft's font-tools:\n"
            "  curl -sS -o {} \\\n"
            "    https://raw.githubusercontent.com/microsoft/font-tools/master/USE/"
            "IndicShapingInvalidCluster.txt".format(path, path)
        )

    pairs, triples = [], []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip().rstrip(";").strip()
            if not line:
                continue
            parts = [int(p, 16) for p in line.split()]
            if len(parts) == 2:
                pairs.append(tuple(parts))
            elif len(parts) == 3:
                triples.append(tuple(parts))

    pairs.sort()
    triples.sort()
    out = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "vowel_constraints.rs")
    with open(out, "w", encoding="utf-8") as f:
        f.write("// Generated by scripts/tools/gen/indic-table.py. Do not edit.\n")
        f.write("//\n")
        f.write("// Vowel pairs that are written the same way as some other single vowel. A dotted\n")
        f.write("// circle goes between them, so the reader can see the sequence is not the vowel it\n")
        f.write("// resembles and the font does not compose it into one.\n\n")
        f.write("// Sorted, so a lookup can bisect.\n")
        f.write("pub(crate) static INVALID_VOWEL_PAIRS: &[(u32, u32)] = &[\n")
        for a, b in pairs:
            f.write("    (0x{:04X}, 0x{:04X}),\n".format(a, b))
        f.write("];\n\n")
        f.write("// The same thing over three characters. Checked first, because a sequence that\n")
        f.write("// matches here is not also the pair its first two characters form – and the circle\n")
        f.write("// belongs before the last character rather than in the middle.\n")
        f.write("pub(crate) static INVALID_VOWEL_TRIPLES: &[(u32, u32, u32)] = &[\n")
        for a, b, c in triples:
            f.write("    (0x{:04X}, 0x{:04X}, 0x{:04X}),\n".format(a, b, c))
        f.write("];\n")

    print("  {} invalid vowel pairs, {} triples".format(len(pairs), len(triples)))
    return len(pairs) + len(triples)

def main():
    categories, positions = build()
    count = emit(categories, positions)
    print("wrote {}".format(OUT))
    print("  {} ranges over {} codepoints".format(count, len(categories)))

    emit_vowel_constraints()

if __name__ == "__main__":
    main()
