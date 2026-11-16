#!/usr/bin/env python3
# The class and position of each character the Indic, Khmer and Myanmar shapers read (indic_tables.rs), from
# Unicode's Indic properties and Microsoft's script development specifications, and the vowel sequences Microsoft
# lists as invalid (vowel_constraints.rs).

import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import trie
import ucd
from categories import INDIC_CLASSES, INDIC_POSITIONS, emit_consts

GENERATED = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "src", "daecore", "src",
                         "daeshaper", "generated")
OUT = os.path.join(GENERATED, "indic_tables.rs")

# The scripts these shapers serve; Unicode's Indic properties for any other's characters belong to the Universal
# Shaping Engine.
SERVED = {"Deva", "Beng", "Guru", "Gujr", "Orya", "Taml", "Telu", "Knda", "Mlym", "Mymr", "Khmr"}
INDIC = SERVED - {"Mymr", "Khmr"}

# The Indic_Syllabic_Category values each class takes; any other is X.
SYLLABIC = {
    "C": ["Consonant", "Consonant_Dead", "Consonant_Head_Letter", "Consonant_Initial_Postfixed"],
    "V": ["Vowel", "Vowel_Independent"],
    "N": ["Nukta"],
    "H": ["Virama", "Invisible_Stacker"],
    "ZWNJ": ["Non_Joiner"],
    "ZWJ": ["Joiner"],
    "M": ["Vowel_Dependent", "Pure_Killer"],
    "SM": ["Bindu", "Visarga", "Syllable_Modifier", "Gemination_Mark", "Tone_Mark"],
    "A": ["Cantillation_Mark"],
    "Placeholder": ["Consonant_Placeholder", "Number", "Number_Joiner", "Brahmi_Joining_Number"],
    "Repha": ["Consonant_Preceding_Repha"],
    "Medial": ["Consonant_Medial", "Consonant_Subjoined", "Consonant_Final"],
    "Stacker": ["Consonant_With_Stacker"],
    "Symbol": ["Avagraha"],
    "RegShift": ["Register_Shifter"],
    "Robat": ["Consonant_Succeeding_Repha"],
    "AbvS": ["Consonant_Killer"],
}
CLASS_OF = {value: name for name, values in SYLLABIC.items() for value in values}

# Classes a specification's class chart gives outright, where Unicode's properties say less or other.
CHARTED = {
    # Myanmar's: kinzi's letters, the asat, the medials (Mon's medial la apart), the Pwo tones, the dot below, the
    # anusvara class, its consonants and generic bases, and digit zero, a cluster of its own.
    "Ra": [0x1004, 0x101B, 0x105A],
    "As": [0x103A],
    "MY": [0x103B, 0x105E, 0x105F], "MR": [0x103C], "MW": [0x103D, 0x1082], "MH": [0x103E], "ML": [0x1060],
    "PT": [0x1063, 0x1064, *range(0x1069, 0x106E), 0xAA7B],
    "N": [0x1037],
    "A": [0x1032, 0x1036],
    "SM": [0x109C],
    "C": [0x104E],
    "X": [0x1040],
    "Placeholder": [0x2015, 0x2022, *range(0x25FB, 0x25FF)],
    # Khmer's: the register shifters, the robat, and the signs above and after the base, bathamasat above its number
    # among them.
    "RegShift": [0x17C9, 0x17CA],
    "Robat": [0x17CC],
    "AbvS": [0x17C6, 0x17CB, *range(0x17CD, 0x17D2), 0x17D3, 0x17DD],
    "PstS": [0x17C7, 0x17C8],
    "VS": list(range(0xFE00, 0xFE10)),
    "DottedCircle": [0x25CC],
}

# Characters whose use in text these classes follow, each for the reason given.
USAGE = {
    # Gurmukhi's vowel bearers open syllables and carry vowel signs as letters do.
    "C": [0x0A72, 0x0A73],
    # Gujarati's shadda doubles its consonant and is typed right after it, where a nukta goes.
    "N": [0x0AFB],
    # Devanagari's grave and acute accents close a syllable as its tone marks do.
    "SM": [0x0953, 0x0954],
    # Gurmukhi's ii follows its consonant; its udaat is written under the base and typed with the vowel signs.
    "MPost": [0x0A40],
    "M": [0x0A51],
    # Spacing letters that stand alone and carry the vedic signs after them.
    "Placeholder": [0x09FC, 0x0C80, 0x0D04],
}


def scripts_of():
    alias = {f[2]: f[1] for f in ucd.rows("PropertyValueAliases.txt") if f[0] == "sc"}
    script = {c: alias[v] for c, v in ucd.by_code_point("Scripts.txt").items()}
    extensions = {c: set(v.split()) for c, v in ucd.by_code_point("ScriptExtensions.txt").items()}
    return lambda c: extensions.get(c, {script.get(c, "Zzzz")})


def names():
    with open(ucd.path("UnicodeData.txt"), encoding="utf-8") as f:
        return {int(line.split(";")[0], 16): line.split(";")[1] for line in f}


def classes():
    syllabic = ucd.by_code_point("IndicSyllabicCategory.txt")
    positional = ucd.by_code_point("IndicPositionalCategory.txt")
    general = ucd.unicode_data()[0]
    of_script = scripts_of()
    name = names()

    # A character Unicode's Indic properties list takes its class when its script is one these shapers serve, or
    # when it's common to all: the joiners, generic bases and modifiers written as digits.
    found = {}
    for c in set(syllabic) | set(positional):
        scripts = of_script(c)
        if scripts & SERVED or scripts <= {"Zyyy", "Zinh"} and c in syllabic:
            found[c] = CLASS_OF.get(syllabic.get(c), "X")
            if found[c] == "SM" and general.get(c, "").startswith("N"):
                found[c] = "SMDigit"
    # Myanmar and Khmer class their vowels by where they're written, a split vowel by its last part but Khmer's oe,
    # which the Khmer specification counts above.
    for c in [c for c in found if syllabic.get(c) == "Vowel_Dependent" and of_script(c) & {"Mymr", "Khmr"}]:
        where = positional.get(c, "")
        found[c] = "VPre" if where == "Left" else "VPst" if where.endswith("Right") else \
            "VBlw" if where == "Bottom" else "VAbv"
    # The vedic blocks' spacing signs Unicode's Indic properties leave out, or call a bindu, are bases for the signs
    # after them; every other combining sign of Vedic Extensions is a vedic sign.
    for c, n in name.items():
        if re.match(r"(VEDIC|DEVANAGARI) SIGN ", n) and general.get(c) == "Lo" \
                and syllabic.get(c) in (None, "Bindu") and of_script(c) & INDIC:
            found[c] = "Symbol"
        elif 0x1CD0 <= c <= 0x1CFF and general.get(c) == "Mn" and c not in syllabic:
            found[c] = "A"
    # The Devanagari specification's Ra, each script's letter that forms a reph; Assamese ra among Bengali's.
    for c, n in name.items():
        if of_script(c) & INDIC and re.fullmatch(r"\w+ LETTER RA( WITH MIDDLE DIAGONAL)?", n):
            found[c] = "Ra"
    found[0x179A] = "Ra"
    for listed in (CHARTED, USAGE):
        for cls, codes in listed.items():
            for c in codes:
                found[c] = cls
    return found, positional


# The matra positions by script, from the Devanagari specification's four affinities: before the first half form,
# after the main consonant, after subjoined consonants, and after post-base ones. Each script's below- and post-base
# forms are drawn against its matras where its fonts put them, so a script may move a matra across them.
MATRAS = {
    "Deva": {"Left": "PRE_MATRA", "Top": "AFTER_BELOW", "Bottom": "AFTER_BELOW", "Right": "AFTER_BELOW"},
    "Beng": {"Left": "PRE_MATRA", "Bottom": "AFTER_BELOW", "Right": "AFTER_POST", "Left_And_Right": "AFTER_POST"},
    "Guru": {"Left": "PRE_MATRA", "Top": "AFTER_POST", "Bottom": "AFTER_POST", "Right": "AFTER_POST"},
    "Gujr": {"Left": "PRE_MATRA", "Top": "AFTER_BELOW", "Bottom": "AFTER_POST", "Right": "AFTER_POST",
             "Top_And_Right": "AFTER_POST"},
    "Orya": {"Left": "PRE_MATRA", "Top": "AFTER_BASE", "Bottom": "AFTER_BELOW", "Right": "AFTER_POST",
             "Left_And_Right": "AFTER_POST", "Top_And_Right": "AFTER_POST", "Top_And_Left": "AFTER_BASE",
             "Top_And_Left_And_Right": "AFTER_POST"},
    "Taml": {"Left": "PRE_MATRA", "Top": "AFTER_BELOW", "Right": "AFTER_POST", "Left_And_Right": "AFTER_POST"},
    "Telu": {"Top": "BEFORE_BELOW", "Bottom": "BEFORE_BELOW", "Right": "BEFORE_BELOW",
             "Top_And_Bottom": "BEFORE_BELOW"},
    "Knda": {"Top": "BEFORE_BELOW", "Bottom": "BEFORE_BELOW", "Right": "AFTER_BELOW", "Top_And_Right": "AFTER_BELOW"},
    "Mlym": {"Left": "PRE_MATRA", "Top": "AFTER_BELOW", "Bottom": "AFTER_POST", "Right": "AFTER_POST",
             "Left_And_Right": "AFTER_POST"},
}
# Matras whose fonts draw them against another consonant than their script's others: Telugu's vocalic r and rr after
# the below-base forms, and Kannada's aa, ii, u and uu before them; Gurmukhi's udaat goes under the base.
MATRA_EXCEPTIONS = {
    0x0C43: "AFTER_BELOW", 0x0C44: "AFTER_BELOW",
    0x0CBE: "BEFORE_BELOW", 0x0CC0: "BEFORE_BELOW", 0x0CC1: "BEFORE_BELOW", 0x0CC2: "BEFORE_BELOW",
    0x0A51: "BELOW",
}
# Each class's position, where its class alone gives one.
BY_CLASS = {
    "BASE": ["C", "Ra", "V", "Placeholder", "DottedCircle", "Stacker", "Repha"],
    "SIGNS": ["SM", "SMDigit", "A", "Symbol"],
    "PRE_BASE": ["VPre"], "ABOVE": ["VAbv"], "BELOW": ["VBlw"], "POST": ["VPst"],
}
POSITION_BY_CLASS = {cls: position for position, classes in BY_CLASS.items() for cls in classes}


def position(c, cls, where, scripts):
    if c == 0x0D4E:
        return "LAST"  # Malayalam's dot reph, which is already where it's drawn
    if cls == "SM" and where == "Top" and "Orya" in scripts:
        return "BEFORE_BELOW"
    if cls == "H":
        return {"Bottom": "BELOW", "Top": "ABOVE"}.get(where, "LAST")
    if cls == "Medial":
        return "BASE" if "Guru" in scripts else "LAST"
    if cls in ("M", "MPost"):
        script = next(iter(scripts & INDIC), None)
        return MATRA_EXCEPTIONS.get(c) or MATRAS.get(script, {}).get(where, "AFTER_BELOW")
    return POSITION_BY_CLASS.get(cls, "LAST")


def table():
    found, positional = classes()
    of_script = scripts_of()
    return {c: (cls, position(c, cls, positional.get(c, ""), of_script(c))) for c, cls in found.items()}


def write_table(entries):
    index = {name: i for i, name in enumerate(INDIC_CLASSES)}, {name: i for i, name in enumerate(INDIC_POSITIONS)}
    default = ("X", "LAST")
    runs = []
    for c in sorted(c for c, value in entries.items() if value != default):
        if runs and runs[-1][1] == c - 1 and runs[-1][2] == entries[c]:
            runs[-1][1] = c
        else:
            runs.append([c, c, entries[c]])
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("// Generated by scripts/tools/gen/indic-table.py. Do not edit.\n//\n"
                "// The class and position of every character the Indic, Khmer and Myanmar shapers read; any\n"
                "// other is X at LAST. Derived in part from the Unicode Character Database, whose license notice\n"
                "// is at the top of src/daecore/src/daeshaper/generated/unicode_tables.rs.\n\n"
                "use crate::daecore::daeshaper::unicode::Trie;\n\n")
        emit_consts(f, "category", enumerate(INDIC_CLASSES), "every class the table holds; each grammar reads some")
        emit_consts(f, "position", enumerate(INDIC_POSITIONS), "the sorting order; each pass names some")
        columns = [[(lo, hi, index[k][value[k]]) for lo, hi, value in runs] for k in (0, 1)]
        stats = trie.emit(f, "INDIC_CATEGORY", "IndicCategoryRecord", [("category", "u8"), ("position", "u8")],
                          columns, [index[0]["X"], index[1]["LAST"]],
                          "// A character's class and position, in one record so one lookup finds both.\n")
    print("  {} runs over {} characters; trie of {} records, {} mid blocks, {} leaf blocks".format(
        len(runs), len(entries), *stats))


# Microsoft's invalid vowel sequences: two or three characters that read as another vowel, which take a dotted circle
# before their last so a font doesn't compose them.
def write_vowel_constraints():
    sequences = [tuple(int(p, 16) for p in fields[0].split()) for fields in ucd.rows("IndicShapingInvalidCluster.txt")]
    pairs = sorted(s for s in sequences if len(s) == 2)
    # A triple whose first two are a pair is never reached: the pair matches first.
    triples = sorted(s for s in sequences if len(s) == 3 and s[:2] not in set(pairs))
    starts = []
    for c in sorted({s[0] for s in pairs + triples}):
        if starts and starts[-1][1] == c - 1:
            starts[-1][1] = c
        else:
            starts.append([c, c])
    arms = ["0x{:04X}".format(lo) if lo == hi else "0x{:04X}..=0x{:04X}".format(lo, hi) for lo, hi in starts]
    with open(os.path.join(GENERATED, "vowel_constraints.rs"), "w", encoding="utf-8") as f:
        f.write("// Generated by scripts/tools/gen/indic-table.py. Do not edit.\n//\n"
                "// Vowel sequences written the same way as some other vowel. A dotted circle goes before the\n"
                "// last, so the reader can see the sequence isn't the vowel it resembles and the font doesn't\n"
                "// compose it.\n//\n"
                "// From Microsoft's list of invalid Indic clusters, whose license notice is at the top of\n"
                "// src/daecore/src/daeshaper/generated/use_tables.rs.\n\n"
                "// Sorted, so a lookup can bisect.\n"
                "pub(crate) static INVALID_VOWEL_PAIRS: &[(u32, u32)] = &[\n")
        f.writelines("    (0x{:04X}, 0x{:04X}),\n".format(*p) for p in pairs)
        f.write("];\n\n// The same over three characters, none opening with a pair, which would match first.\n"
                "pub(crate) static INVALID_VOWEL_TRIPLES: &[(u32, u32, u32)] = &[\n")
        f.writelines("    (0x{:04X}, 0x{:04X}, 0x{:04X}),\n".format(*t) for t in triples)
        f.write("];\n\n// Whether a sequence opens with `c`. Few characters do, and the rest skip the searches.\n"
                "pub(crate) fn opens_invalid_vowel_sequence(c: u32) -> bool {\n"
                "    matches!(\n        c,\n        " + "\n            | ".join(arms) + "\n    )\n}\n")
    print("  {} invalid vowel pairs, {} triples".format(len(pairs), len(triples)))


def main():
    write_table(table())
    write_vowel_constraints()
    print("wrote {} and vowel_constraints.rs".format(os.path.relpath(OUT)))


if __name__ == "__main__":
    main()
