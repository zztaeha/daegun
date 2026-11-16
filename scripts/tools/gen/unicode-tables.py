#!/usr/bin/env python3

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import trie
import ucd
from rows import wrapped
from categories import emit_consts

def read_license():
    path = os.path.join(ucd.DATA, "UnicodeLicense.txt")
    if not os.path.exists(path):
        sys.exit(
            "missing {}\n  curl -sS -o {} https://www.unicode.org/license.txt".format(path, path)
        )
    with open(path, encoding="utf-8") as f:
        return f.read().strip()
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "unicode_tables.rs")

# The general categories in the tables' numbering, with the names the API gives them.
GENERAL_CATEGORIES = [
    ("Cn", "Unassigned"), ("Cc", "Control"), ("Cf", "Format"), ("Co", "PrivateUse"), ("Cs", "Surrogate"),
    ("Ll", "LowercaseLetter"), ("Lm", "ModifierLetter"), ("Lo", "OtherLetter"), ("Lt", "TitlecaseLetter"),
    ("Lu", "UppercaseLetter"), ("Mc", "SpacingMark"), ("Me", "EnclosingMark"), ("Mn", "NonspacingMark"),
    ("Nd", "DecimalNumber"), ("Nl", "LetterNumber"), ("No", "OtherNumber"), ("Pc", "ConnectPunctuation"),
    ("Pd", "DashPunctuation"), ("Pe", "ClosePunctuation"), ("Pf", "FinalPunctuation"), ("Pi", "InitialPunctuation"),
    ("Po", "OtherPunctuation"), ("Ps", "OpenPunctuation"), ("Sc", "CurrencySymbol"), ("Sk", "ModifierSymbol"),
    ("Sm", "MathSymbol"), ("So", "OtherSymbol"), ("Zl", "LineSeparator"), ("Zp", "ParagraphSeparator"),
    ("Zs", "SpaceSeparator"),
]
# Each category's rustdoc, by its short name.
GC_DOCS = {
    "Cn": "no character assigned", "Cc": "a C0 or C1 control code", "Cf": "a format control, which draws nothing",
    "Co": "a private use character", "Cs": "a surrogate code point", "Ll": "a lowercase letter",
    "Lm": "a modifier letter", "Lo": "another letter, such as one of a script without case",
    "Lt": "a titlecase digraph", "Lu": "an uppercase letter",
    "Mc": "a spacing mark, which takes room of its own", "Me": "an enclosing mark", "Mn": "a nonspacing mark",
    "Nd": "a decimal digit", "Nl": "a letterlike number, such as a Roman numeral",
    "No": "another number, such as a fraction or a superscript digit", "Pc": "a connector, such as the underscore",
    "Pd": "a dash or hyphen", "Pe": "a closing bracket", "Pf": "a closing quotation mark",
    "Pi": "an opening quotation mark", "Po": "other punctuation", "Ps": "an opening bracket",
    "Sc": "a currency symbol", "Sk": "a modifier symbol, such as a spacing accent", "Sm": "a math symbol",
    "So": "another symbol", "Zl": "the line separator", "Zp": "the paragraph separator", "Zs": "a space",
}
GC_INDEX = {short: i for i, (short, _) in enumerate(GENERAL_CATEGORIES)}

# The Arabic shaper's state table has a column for each joining type below 6; transparent ones skip it.
JOINING_TYPES = [
    ("U", 0, "NonJoining"), ("L", 1, "LeftJoining"), ("R", 2, "RightJoining"), ("D", 3, "DualJoining"),
    ("ALAPH", 4, "Alaph"), ("DALATH RISH", 5, "DalathRish"), ("T", 7, "Transparent"),
]
JT = {key: value for key, value, _ in JOINING_TYPES}


# Default ignorables that fonts draw as spacing glyphs: the Hangul fillers and Duployan's shorthand format controls.
VISIBLE_IGNORABLES = {0x115F, 0x1160, 0x3164, 0xFFA0, 0x1BCA0, 0x1BCA1, 0x1BCA2, 0x1BCA3}

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
            "  Add each to the matching list, and check whether the rules in daeshaper/unicode/ need it"
            " too.".format(
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

def bidi_long_to_short():
    out = {f[2]: f[1] for f in ucd.rows("PropertyValueAliases.txt") if len(f) >= 3 and f[0] == "bc"}
    assert out, "PropertyValueAliases.txt lists no bidi classes – its shape changed"
    return out

def parse_bidi_missing(name):
    long_to_short = bidi_long_to_short()
    out = []
    with open(ucd.path(name), encoding="utf-8") as f:
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

VERTICAL_ORIENTATION = [("R", "Rotated"), ("Tr", "TransformedRotated"), ("Tu", "TransformedUpright"), ("U", "Upright")]
VO_DOCS = {
    "R": "`R`: sideways, turned a quarter clockwise, as Latin letters set.",
    "Tr": "`Tr`: upright in a vertical form where one is drawn, else sideways.",
    "Tu": "`Tu`: upright, in a vertical form where one is drawn.",
    "U": "`U`: upright, as CJK ideographs set.",
}

INDIC_CONJUNCT_BREAK = [None, "Consonant", "Extend", "Linker"]

def named(enum, names):
    return lambda v: "{}::{}".format(enum, names[v])

def joining_type(v):
    names = {value: name for _, value, name in JOINING_TYPES}
    return "JoiningType::{}".format(names[v])

# The bits of a record's `flags`, its yes-or-no properties. Each but the first two stands before a search, which runs
# only where it is set.
FLAGS = ["DEFAULT_IGNORABLE", "EXTENDED_PICTOGRAPHIC", "DECOMPOSES", "COMPOSES_SECOND", "HAS_MIRROR", "BRACKET",
         "SCRIPT_EXTENSIONS"]
FLAG = {name: 1 << i for i, name in enumerate(FLAGS)}

# Hangul's algorithmic compositions: syllables decompose, vowels follow a leading consonant and trailing consonants an
# LV syllable.
HANGUL_SYLLABLES = range(0xAC00, 0xD7A4)
HANGUL_SECONDS = list(range(0x1161, 0x1176)) + list(range(0x11A8, 0x11C3))

# Each field's type, the value of a code point the data leaves out, and how a value is written.
TRIE_FIELDS = [
    ("script", "u16", 0xFFFF, str),
    ("general_category", "GeneralCategory", 0, named("GeneralCategory", [n for _, n in GENERAL_CATEGORIES])),
    ("combining_class", "u8", 0, str),
    ("flags", "u8", 0, str),
    ("joining_type", "JoiningType", JT["U"], joining_type),
    ("grapheme_break", "u8", 0, str),
    ("word_break", "u8", 0, str),
    ("bidi_class", "BidiClass", 0, named("BidiClass", BIDI_CLASS)),
    ("indic_conjunct_break", "u8", 0, str),
    ("line_break", "LineBreakClass", 0, named("LineBreakClass", LINE_BREAK)),
    ("east_asian_width", "u8", 0, str),
    ("vertical_orientation", "VerticalOrientation", 0,
     named("VerticalOrientation", [n for _, n in VERTICAL_ORIENTATION])),
]

def emit_enum(f, name, variants, vis="pub(crate)", derives="Clone, Copy, PartialEq, Eq, Debug", acronyms=False,
              doc=None, docs=None, closed=None, open_set=False):
    if doc:
        f.write("/// {}\n".format(doc))
    if open_set:
        f.write("#[non_exhaustive]\n")
    if closed:
        f.write("#[expect(clippy::exhaustive_enums, reason = \"{}\")]\n".format(closed))
    if acronyms:
        f.write("#[allow(clippy::upper_case_acronyms, reason = \"the standard's own names for its values\")]\n")
    f.write("#[derive({})]\n#[repr(u8)]\n{} enum {} {{\n".format(derives, vis, name))
    for i, (value, variant) in enumerate(variants):
        if docs:
            f.write("    /// {}\n".format(docs[i]))
        f.write("    {} = {},\n".format(variant, value))
    f.write("}\n\n")

def parse_incb(name):
    order = {name: i for i, name in enumerate(INDIC_CONJUNCT_BREAK) if name}
    out = []
    for f in ucd.rows(name):
        if len(f) >= 3 and f[1] == "InCB" and f[2] in order:
            lo, _, hi = f[0].partition("..")
            out.append((int(lo, 16), int(hi or lo, 16), order[f[2]]))
    return out

def parse_bracket_pairs(name):
    return sorted((int(f[0], 16), int(f[1], 16), 1 if f[2] == "o" else 0) for f in ucd.rows(name) if len(f) >= 3)

def main():
    gc_by_cp, ccc_by_cp, decomp = ucd.unicode_data()

    gc_ranges = to_ranges(gc_by_cp, 0, lambda c: GC_INDEX.get(c, 0))
    ccc_ranges = to_ranges(ccc_by_cp, 0, lambda k: k)

    scripts_raw = ucd.ranges("Scripts.txt")
    script_names = sorted({v for _, _, v in scripts_raw})
    script_index = {n: i for i, n in enumerate(script_names)}
    script_ranges = merge_named_ranges(scripts_raw, lambda v: script_index[v], -1)

    iso_by_long = {f[2]: f[1] for f in ucd.rows("PropertyValueAliases.txt") if len(f) >= 3 and f[0] == "sc"}
    missing = [n for n in script_names if n not in iso_by_long]
    if missing:
        sys.exit("no ISO 15924 code for: {}".format(", ".join(missing)))
    script_isos = [iso_by_long[n] for n in script_names]

    # Script_Extensions by script name, adjacent ranges of one set merged. Each set puts its largest script first:
    # a run of these characters alone takes it, so a lone tatweel reads as Arabic, not Adlam.
    long_by_iso = {iso: long for long, iso in iso_by_long.items()}
    script_size = {}
    for lo, hi, name in scripts_raw:
        script_size[name] = script_size.get(name, 0) + hi - lo + 1
    extension_sets, extensions = [], []
    for lo, hi, codes in ucd.ranges("ScriptExtensions.txt"):
        names = tuple(sorted((long_by_iso[c] for c in codes.split()), key=lambda n: (-script_size[n], n)))
        if names not in extension_sets:
            extension_sets.append(names)
        k = extension_sets.index(names)
        if extensions and extensions[-1][1] + 1 == lo and extensions[-1][2] == k:
            extensions[-1] = (extensions[-1][0], hi, k)
        else:
            extensions.append((lo, hi, k))
    assert all(a[1] < b[0] for a, b in zip(extensions, extensions[1:])), "Script_Extensions ranges overlap"

    ignorable_raw = ucd.ranges(
        "DerivedCoreProperties.txt", wanted="Default_Ignorable_Code_Point"
    )
    ignorable_raw = [(cp, cp, v) for lo, hi, v in ignorable_raw for cp in range(lo, hi + 1)
                     if cp not in VISIBLE_IGNORABLES]

    pictographic_raw = ucd.ranges("emoji-data.txt", wanted="Extended_Pictographic")

    joining, joiners = {}, set()
    for parts in ucd.rows("ArabicShaping.txt"):
        if len(parts) < 4:
            continue
        group = parts[3]
        joining[int(parts[0], 16)] = JT[group if group in ("ALAPH", "DALATH RISH") else parts[2].replace("C", "D")]
        if parts[2] not in ("T", "U"):
            joiners.add(int(parts[0], 16))
    # What ArabicShaping.txt leaves out is transparent if a nonspacing or enclosing mark or a format character, else
    # non-joining.
    for cp, gc in gc_by_cp.items():
        if cp not in joining and gc in ("Mn", "Me", "Cf"):
            joining[cp] = JT["T"]
    jt_ranges = to_ranges(joining, JT["U"], lambda v: v)

    script_of = {cp: name for lo, hi, name in scripts_raw for cp in range(lo, hi + 1)}
    arabic_joining = sorted({script_of[cp] for cp in joiners} - {"Common", "Inherited"})
    bidi = {cp: value for lo, hi, value in ucd.ranges("DerivedBidiClass.txt") for cp in range(lo, hi + 1)}
    letters = {}
    for cp, name in script_of.items():
        if gc_by_cp.get(cp, "").startswith("L"):
            tally = letters.setdefault(name, [0, 0])
            tally[bidi.get(cp) in ("R", "AL")] += 1
    right_to_left = sorted(n for n, (ltr, rtl) in letters.items() if rtl > ltr)

    gcb_index = {name: i for i, name in enumerate(GRAPHEME_BREAK)}
    gcb_raw = ucd.ranges("GraphemeBreakProperty.txt")
    check_values("GraphemeBreakProperty.txt", gcb_raw, GRAPHEME_BREAK)
    gcb_ranges = merge_named_ranges(gcb_raw, lambda v: gcb_index.get(v, 0), 0)

    wb_index = {name: i for i, name in enumerate(WORD_BREAK)}
    wb_raw = ucd.ranges("WordBreakProperty.txt")
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
    for lo, hi, value in ucd.ranges("DerivedBidiClass.txt"):
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
    lb_raw = ucd.ranges("LineBreak.txt")
    check_values("LineBreak.txt", lb_raw, LINE_BREAK)
    lb_ranges = merge_named_ranges(lb_raw, lambda v: lb_index.get(v, 0), 0)

    eaw_index = {name: i for i, name in enumerate(EAST_ASIAN_WIDTH)}
    eaw_raw = ucd.ranges("EastAsianWidth.txt")
    check_values("EastAsianWidth.txt", eaw_raw, EAST_ASIAN_WIDTH)
    eaw_ranges = merge_named_ranges(eaw_raw, lambda v: eaw_index.get(v, 0), 0)

    vo_index = {name: i for i, (name, _) in enumerate(VERTICAL_ORIENTATION)}
    vo_raw = ucd.ranges("VerticalOrientation.txt")
    check_values("VerticalOrientation.txt", vo_raw, vo_index)
    vo_ranges = merge_named_ranges(vo_raw, lambda v: vo_index.get(v, 0), 0)

    incb_ranges = merge_named_ranges(parse_incb("DerivedCoreProperties.txt"), lambda v: v, 0)

    mirroring = sorted((int(f[0], 16), int(f[1].split()[0], 16)) for f in ucd.rows("BidiMirroring.txt")
                       if len(f) >= 2 and f[1])

    # Each vertical presentation form stands for the character its `<vertical>` decomposition names, fullwidth where
    # that is ASCII. Two forms name the low line; the wavy one's stands for U+FE4F WAVY LOW LINE.
    stands_for = {}
    for f in ucd.rows("UnicodeData.txt"):
        named = f[5].removeprefix("<vertical> ")
        if named != f[5] and " " not in named:
            named = int(named, 16)
            stands_for[int(f[0], 16)] = named + 0xFEE0 if named < 0x80 else named
    stands_for[0xFE34] = 0xFE4F
    vertical_forms = sorted((c, form) for form, c in stands_for.items())
    assert len({c for c, _ in vertical_forms}) == len(vertical_forms), "two vertical forms stand for one character"

    exclusions = {int(f[0].split()[0], 16) for f in ucd.rows("CompositionExclusions.txt")}

    decomp_pairs = sorted((cp, p) for cp, p in decomp.items() if len(p) == 2)
    decomp_singles = sorted((cp, p[0]) for cp, p in decomp.items() if len(p) == 1)

    comp = []
    for cp, (a, b) in decomp_pairs:
        if cp in exclusions or ccc_by_cp.get(a, 0) != 0:
            continue
        comp.append((a, b, cp))
    comp.sort()

    flags = {}
    def flag(cps, name):
        for cp in cps:
            flags[cp] = flags.get(cp, 0) | FLAG[name]
    flag((cp for cp, _, _ in ignorable_raw), "DEFAULT_IGNORABLE")
    flag((cp for lo, hi, _ in pictographic_raw for cp in range(lo, hi + 1)), "EXTENDED_PICTOGRAPHIC")
    flag(list(decomp) + list(HANGUL_SYLLABLES), "DECOMPOSES")
    flag([b for _, b, _ in comp] + HANGUL_SECONDS, "COMPOSES_SECOND")
    flag([cp for cp, _ in mirroring], "HAS_MIRROR")
    flag([cp for cp, _, _ in bracket_pairs], "BRACKET")
    flag((cp for lo, hi, _ in extensions for cp in range(lo, hi + 1)), "SCRIPT_EXTENSIONS")
    flag_ranges = to_ranges(flags, 0, lambda v: v)

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("// GENERATED FILE – do not edit by hand.\n")
        f.write("// Regenerate with: python3 scripts/tools/gen/unicode-tables.py\n")
        f.write("// Source: Unicode Character Database {}.{}.{}\n".format(*ucd.VERSION))
        f.write("//\n")
        f.write("// The notice below is required by the license the data is used under, and is\n")
        f.write("// reproduced verbatim from scripts/data/unicode/UnicodeLicense.txt. It covers the\n")
        f.write("// Unicode data and the tables derived from it, not daegun's own code, which is\n")
        f.write("// MIT (see LICENSE).\n")
        f.write("//\n")
        for line in read_license().splitlines():
            f.write("//\n" if not line.strip() else "// {}\n".format(line))
        f.write("\n")
        f.write("use crate::daecore::daeshaper::run::Direction;\n")
        f.write("use crate::daecore::daeshaper::unicode::{Script, Trie};\n\n")
        f.write("// C strings, so the C ABI lends one as it is.\n")
        f.write("pub(crate) static SCRIPT_NAMES: &[&core::ffi::CStr] = &[\n")
        for n in script_names:
            f.write('    c"{}",\n'.format(n))
        f.write("];\n\n")

        f.write("impl Script {\n")
        for n in script_names:
            f.write("    /// {}, ISO 15924 `{}`.\n".format(n.replace("_", " "), iso_by_long[n]))
            f.write("    pub const {}: Script = Script({});\n".format(n.upper(), script_index[n]))
        f.write("}\n\n")

        def scripts(comment, name, names):
            f.write(comment)
            f.write("pub(crate) static {}: &[Script] = &[\n".format(name))
            for n in names:
                f.write("    Script::{},\n".format(n.upper()))
            f.write("];\n\n")

        f.write("// Each script's direction: right to left where most of its letters are of Bidi class R or AL,\n")
        f.write("// else left to right.\n")
        f.write("pub(crate) static SCRIPT_DIRECTIONS: &[Direction] = &[\n")
        for n in script_names:
            f.write("    Direction::{},\n".format("RightToLeft" if n in right_to_left else "LeftToRight"))
        f.write("];\n\n")
        scripts("// Scripts with letters that join as Arabic's do, by their joining types.\n",
                "ARABIC_JOINING", arabic_joining)

        f.write("// ISO 15924 short codes, parallel to SCRIPT_NAMES. OpenType tags derive from these.\n")
        f.write("pub(crate) static SCRIPT_ISO_CODES: &[&str] = &[\n")
        for c in script_isos:
            f.write('    "{}",\n'.format(c))
        f.write("];\n\n")
        # A perfect hash of the lowercased codes: the first odd multiplier from the golden ratio's that sends each
        # to a slot of its own among 2,048, read with one multiply and two loads where a binary search took eight.
        keys = [int.from_bytes(c.lower().encode(), "big") for c in script_isos]
        assert len(keys) < 255 and len(set(keys)) == len(keys)
        slot = lambda k, m: (k * m & 0xFFFFFFFF) >> 21
        mul = next(m for m in range(0x9E3779B1, 1 << 32, 2) if len({slot(k, m) for k in keys}) == len(keys))
        slots = [0] * 2048
        for i, k in enumerate(keys):
            slots[slot(k, mul)] = i + 1
        f.write("// Each script's ISO 15924 code lowercased as a big-endian u32, by script id.\n")
        f.write("pub(crate) static SCRIPT_ISO_KEYS: &[u32] = &[\n")
        for k in keys:
            f.write("    0x{:08X},\n".format(k))
        f.write("];\n\n")
        f.write("// Where a lowercased code lands, (code * SCRIPT_ISO_MUL) >> 21: its script id plus one, else 0.\n")
        f.write("pub(crate) const SCRIPT_ISO_MUL: u32 = 0x{:08X};\n".format(mul))
        f.write("pub(crate) static SCRIPT_ISO_SLOTS: [u8; 2048] = [\n")
        for row in range(0, 2048, 24):
            f.write("    " + " ".join("{},".format(v) for v in slots[row:row + 24]) + "\n")
        f.write("];\n\n")

        gc_docs = ["`{}`: {}.".format(short, GC_DOCS[short]) for short, _ in GENERAL_CATEGORIES]
        emit_enum(f, "GeneralCategory", enumerate(n for _, n in GENERAL_CATEGORIES), vis="pub",
                  doc="A character's Unicode General_Category.", docs=gc_docs,
                  closed="Unicode's stability policy fixes the set", derives="Clone, Copy, PartialEq, Eq, Hash, Debug")
        emit_enum(f, "JoiningType", [(value, name) for _, value, name in JOINING_TYPES],
                  derives="Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug")
        emit_enum(f, "BidiClass", enumerate(BIDI_CLASS), acronyms=True)
        emit_enum(f, "LineBreakClass", enumerate(LINE_BREAK), acronyms=True)
        emit_enum(f, "VerticalOrientation", enumerate(n for _, n in VERTICAL_ORIENTATION), vis="pub",
                  derives="Clone, Copy, PartialEq, Eq, Hash, Debug", open_set=True,
                  doc="How a character sets in vertical text, its Unicode Vertical_Orientation (UAX #50).",
                  docs=[VO_DOCS[short] for short, _ in VERTICAL_ORIENTATION])
        emit_consts(f, "grapheme_break", enumerate(GRAPHEME_BREAK), "UAX #29's values; the rules name a subset")
        emit_consts(f, "word_break", enumerate(WORD_BREAK), "UAX #29's values; the rules name a subset")
        emit_consts(f, "indic_conjunct_break", enumerate(INDIC_CONJUNCT_BREAK))
        emit_consts(f, "east_asian_width", enumerate(EAST_ASIAN_WIDTH), "UAX #11's values; LB30 names a subset")
        f.write("// The bits of a record's `flags`.\n")
        f.write("pub(crate) mod flag {\n")
        for i, name in enumerate(FLAGS):
            f.write("    pub(crate) const {}: u8 = 1 << {};\n".format(name, i))
        f.write("}\n\n")

        columns = [
            script_ranges, gc_ranges, ccc_ranges, flag_ranges, jt_ranges, gcb_ranges, wb_ranges, bc_ranges,
            incb_ranges, lb_ranges, eaw_ranges, vo_ranges,
        ]
        trie_stats = trie.emit(
            f, "PROPS", "Props", [(name, ty) for name, ty, _, _ in TRIE_FIELDS],
            [[(lo, hi, fmt(v)) for lo, hi, v in column] for column, (*_, fmt) in zip(columns, TRIE_FIELDS)],
            [fmt(default) for _, _, default, fmt in TRIE_FIELDS],
            "// Every Unicode property the engine reads of a code point, in one record.\n",
        )

        mirror_of = dict(mirroring)
        assert all(mirror_of.get(cp) == paired for cp, paired, _ in bracket_pairs), \
            "a bracket pairs with other than its mirror; BIDI_BRACKETS needs its pairs again"
        f.write("// The brackets UAX #9's N0 pairs, sorted, true where one opens. Each pairs with its mirror.\n")
        f.write("pub(crate) static BIDI_BRACKETS: &[(u32, bool)] = &[\n")
        for cp, _, is_open in bracket_pairs:
            f.write("    (0x{:04X}, {}),\n".format(cp, "true" if is_open else "false"))
        f.write("];\n\n")

        f.write("// Script_Extensions where it is more than a character's own script: sorted ranges, "
                "each naming its set\n")
        f.write("// in SCRIPT_EXTENSION_SETS, whose scripts come largest first.\n")
        f.write("pub(crate) static SCRIPT_EXTENSIONS: &[(u32, u32, u16)] = &[\n")
        for lo, hi, k in extensions:
            f.write("    (0x{:04X}, 0x{:04X}, {}),\n".format(lo, hi, k))
        f.write("];\n\n")
        f.write("pub(crate) static SCRIPT_EXTENSION_SETS: &[&[Script]] = &[\n")
        for names in extension_sets:
            scripts = ["Script::" + n.upper() for n in names]
            one = "    &[{}],".format(", ".join(scripts))
            if len(one) <= 120:
                f.write(one + "\n")
            else:
                f.write("    &[\n{}\n    ],\n".format("\n".join(wrapped(" " * 8, [s + "," for s in scripts]))))
        f.write("];\n\n")

        pair_bmp = [(cp, a, b) for cp, (a, b) in decomp_pairs if cp <= 0xFFFF]
        pair_supp = [(cp, a, b) for cp, (a, b) in decomp_pairs if cp > 0xFFFF]
        assert all(a <= 0xFFFF and b <= 0xFFFF for _, a, b in pair_bmp), \
            "a BMP-keyed pair decomposes to a supplementary codepoint; DECOMPOSE_PAIR_BMP can no longer be u16"

        f.write("// Canonical pair decompositions, composed form and both parts all in the BMP.\n")
        f.write("// Half the width of a `u32` triple, and it holds all but 46 of the rows.\n")
        f.write("pub(crate) static DECOMPOSE_PAIR_BMP: &[(u16, u16, u16)] = &[\n")
        for cp, a, b in pair_bmp:
            f.write("    (0x{:04X}, 0x{:04X}, 0x{:04X}),\n".format(cp, a, b))
        f.write("];\n\n")

        f.write("// The rest, where something does not fit a `u16`.\n")
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

        f.write("// Every other singleton decomposition whose value is in the BMP, as its key is.\n")
        f.write("pub(crate) static DECOMPOSE_SINGLE: &[(u16, u16)] = &[\n")
        for cp, a in rest:
            if a <= 0xFFFF:
                f.write("    (0x{:04X}, 0x{:04X}),\n".format(cp, a))
        f.write("];\n\n")
        f.write("// The few whose value is past the BMP.\n")
        f.write("pub(crate) static DECOMPOSE_SINGLE_ASTRAL: &[(u16, u32)] = &[\n")
        for cp, a in rest:
            if a > 0xFFFF:
                f.write("    (0x{:04X}, 0x{:05X}),\n".format(cp, a))
        f.write("];\n\n")

        compose_bmp = [(a, b, cp) for a, b, cp in comp if max(a, b) <= 0xFFFF]
        compose_supp = [(a, b, cp) for a, b, cp in comp if max(a, b) > 0xFFFF]
        assert all(cp <= 0xFFFF for _, _, cp in compose_bmp), "BMP parts compose past it; COMPOSE_BMP needs u32"
        f.write("// Canonical compositions as `(a, b, composed)`, sorted by their parts, so a search reads one\n")
        f.write("// table rather than an index into the decompositions.\n")
        f.write("pub(crate) static COMPOSE_BMP: &[(u16, u16, u16)] = &[\n")
        for a, b, cp in compose_bmp:
            f.write("    (0x{:04X}, 0x{:04X}, 0x{:04X}),\n".format(a, b, cp))
        f.write("];\n\n")
        f.write("// Those with a part past the BMP.\n")
        f.write("pub(crate) static COMPOSE_SUPP: &[(u32, u32, u32)] = &[\n")
        for a, b, cp in compose_supp:
            f.write("    (0x{:05X}, 0x{:05X}, 0x{:05X}),\n".format(a, b, cp))
        f.write("];\n\n")

        hebrew = sorted((a, b, cp) for cp, (a, b) in decomp_pairs if 0xFB1D <= cp <= 0xFB4F)
        assert len({(a, b) for a, b, _ in hebrew}) == len(hebrew), "two Hebrew presentation forms share their parts"
        f.write("// The Hebrew presentation forms canonical composition excludes, as `(a, b, form)`, sorted by\n")
        f.write("// their parts.\n")
        f.write("pub(crate) static HEBREW_PRESENTATION_FORMS: &[(u16, u16, u16)] = &[\n")
        for a, b, cp in hebrew:
            f.write("    (0x{:04X}, 0x{:04X}, 0x{:04X}),\n".format(a, b, cp))
        f.write("];\n\n")

        f.write("// `(codepoint, its mirror)`, sorted.\n")
        f.write("pub(crate) static MIRRORING: &[(u32, u32)] = &[\n")
        for a, b in mirroring:
            f.write("    (0x{:04X}, 0x{:04X}),\n".format(a, b))
        f.write("];\n\n")

        f.write("// `(character, its vertical presentation form)`, sorted: each form's `<vertical>` decomposition, "
                "fullwidth where\n")
        f.write("// it is ASCII; U+FE4F WAVY LOW LINE stands for the wavy low line's form.\n")
        f.write("pub(crate) const VERTICAL_FORMS: [(char, char); {}] = [\n".format(len(vertical_forms)))
        for c, form in vertical_forms:
            f.write("    ('\\u{{{:04X}}}', '\\u{{{:04X}}}'),\n".format(c, form))
        f.write("];\n")

    print("wrote {}".format(os.path.relpath(OUT)))
    print("  fused trie       {} records, {} mid blocks, {} leaf blocks".format(*trie_stats))
    print("  general category {} ranges".format(len(gc_ranges)))
    print("  combining class  {} ranges".format(len(ccc_ranges)))
    print("  script           {} ranges over {} scripts".format(len(script_ranges), len(script_names)))
    print("  flags            {} ranges".format(len(flag_ranges)))
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
    print("  vertical forms   {} pairs".format(len(vertical_forms)))

if __name__ == "__main__":
    main()
