#!/usr/bin/env python3
# The Universal Shaping Engine's class of each character it can meet (use_tables.rs), from Unicode's properties and
# Microsoft's additions to them, by the derivation its specification charts under "Character classification".

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import trie
import ucd
from categories import emit_consts

HERE = os.path.dirname(os.path.abspath(__file__))
GRAMMAR = os.path.join(HERE, "..", "..", "data", "grammars", "use.grammar")
OUT = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "use_tables.rs")

# The specification's scripts, which the shaper gives this engine.
USE_SCRIPTS = {
    "Adlam", "Ahom", "Balinese", "Batak", "Beria_Erfe", "Bhaiksuki", "Brahmi", "Buginese", "Buhid",
    "Chakma", "Cham", "Chorasmian", "Cypro_Minoan", "Dives_Akuru", "Dogra", "Duployan",
    "Egyptian_Hieroglyphs", "Elymaic", "Garay", "Grantha", "Gunjala_Gondi", "Gurung_Khema",
    "Hanifi_Rohingya", "Hanunoo", "Javanese", "Jurchen", "Kaithi", "Kawi", "Kayah_Li", "Kharoshthi",
    "Khitan_Small_Script", "Khojki", "Khudawadi", "Kirat_Rai", "Lepcha", "Limbu", "Mahajani",
    "Makasar", "Mandaic", "Manichaean", "Marchen", "Masaram_Gondi", "Medefaidrin", "Meetei_Mayek",
    "Miao", "Modi", "Mongolian", "Multani", "Nag_Mundari", "Nandinagari", "Newa", "Nko",
    "Nyiakeng_Puachue_Hmong", "Ol_Onal", "Old_Sogdian", "Old_Uyghur", "Pahawh_Hmong", "Phags_Pa",
    "Proto_Cuneiform", "Psalter_Pahlavi", "Rejang", "Saurashtra", "Seal", "Sharada", "Siddham",
    "Sidetic", "Sinhala", "Sogdian", "Soyombo", "Sundanese", "Sunuwar", "Syloti_Nagri", "Tagalog",
    "Tagbanwa", "Tai_Le", "Tai_Tham", "Tai_Viet", "Tai_Yo", "Takri", "Tangsa", "Tibetan",
    "Tifinagh", "Tirhuta", "Todhri", "Tolong_Siki", "Toto", "Tulu_Tigalari", "Vithkuqi", "Wancho",
    "Yezidi", "Zanabazar_Square",
}
# The Indic shaper's scripts come here too when a font files them under their '3' tags.
INDIC_SCRIPTS = {"Devanagari", "Bengali", "Gurmukhi", "Gujarati", "Oriya", "Tamil", "Telugu", "Kannada", "Malayalam"}

# The chart's classes by Indic_Syllabic_Category, Microsoft's hieroglyph values among them. As use.grammar reads
# them: IND arrives as O, ZWJ as CGJ, and the invisible stacker and symbol modifiers keep classes of their own.
SYLLABIC = {
    "B": ["Number", "Consonant", "Consonant_Head_Letter", "Tone_Letter", "Vowel_Independent"],
    "CGJ": ["Joiner"],
    "CM": ["Nukta", "Gemination_Mark", "Consonant_Killer"],
    "CS": ["Consonant_With_Stacker"],
    # Unicode's Consonant_Initial_Postfixed "behave like a Consonant_Succeeding_Repha".
    "F": ["Consonant_Succeeding_Repha", "Consonant_Initial_Postfixed"],
    # Microsoft's data gives Lepcha's ran a final modifier value of its own.
    "FM": ["Syllable_Modifier", "Consonant_Final_Modifier"],
    "G": ["Hieroglyph"],
    "GB": ["Consonant_Placeholder"],
    "H": ["Virama"],
    "HM": ["Hieroglyph_Modifier"],
    "HN": ["Number_Joiner"],
    "HR": ["Hieroglyph_Mirror"],
    "IS": ["Invisible_Stacker"],
    "J": ["Hieroglyph_Joiner"],
    "N": ["Brahmi_Joining_Number"],
    "O": ["Consonant_Dead", "Modifying_Letter"],
    "R": ["Consonant_Preceding_Repha", "Consonant_Prefixed"],
    "RK": ["Reordering_Killer"],
    "SB": ["Hieroglyph_Mark_Begin", "Hieroglyph_Segment_Begin"],
    "SE": ["Hieroglyph_Mark_End", "Hieroglyph_Segment_End"],
    "SM": ["Symbol_Modifier"],
    "V": ["Pure_Killer"],
    "VM": ["Tone_Mark", "Cantillation_Mark", "Register_Shifter", "Visarga"],
    "ZWNJ": ["Non_Joiner"],
}
CLASS_OF = {value: cls for cls, values in SYLLABIC.items() for value in values}
# Values the chart makes a base when the character is a letter, and otherwise this class.
UNLESS_LETTER = {"Avagraha": "O", "Bindu": "VM", "Consonant_Final": "F", "Consonant_Medial": "M",
                 "Consonant_Subjoined": "SUB", "Vowel": "V", "Vowel_Dependent": "V"}
# Classes the chart splits by where the character is drawn.
SPLIT = {"CM", "F", "FM", "M", "SM", "V", "VM"}
# Code points the chart names outright.
BASE_OTHER = {0x2015, 0x2022, *range(0x25FB, 0x25FF)}
GRAPHEME_JOINER = 0x034F
# Tai Tham's sakot subjoins a consonant after vowels as well as before, where the chart's halant can't stand.
SAKOT = 0x1A60


def category_numbers():
    numbers, section = {}, None
    for line in open(GRAMMAR, encoding="utf-8"):
        line = line.split("#", 1)[0].strip()
        if line.endswith(":") and " " not in line:
            section = line[:-1]
        elif section == "categories" and "=" in line:
            name, value = (part.strip() for part in line.split("=", 1))
            numbers[name] = int(value)
    return numbers


def side(where):
    # A part drawn left of the base reorders before it, as every position with one is VPre in the chart; otherwise
    # above wins over below, and below over right.
    if "Left" in where:
        return "Pre"
    if "Top" in where:
        return "Abv"
    return "Blw" if "Bottom" in where or where == "Overstruck" else "Pst"


def classes():
    syllabic = ucd.by_code_point("IndicSyllabicCategory.txt")
    syllabic.update(ucd.by_code_point("IndicSyllabicCategory-Additional.txt"))
    positional = ucd.by_code_point("IndicPositionalCategory.txt")
    positional.update(ucd.by_code_point("IndicPositionalCategory-Additional.txt"))
    ignorable = ucd.by_code_point("DerivedCoreProperties.txt", "Default_Ignorable_Code_Point")
    joining = ucd.by_code_point("ArabicShaping.txt", column=2)
    general, _, decomposed = ucd.unicode_data()[:3]
    with open(ucd.path("UnicodeData.txt"), encoding="utf-8") as f:
        name = {int(line.split(";")[0], 16): line.split(";")[1] for line in f}
    long_name = {f[1]: f[2] for f in ucd.rows("PropertyValueAliases.txt") if f[0] == "sc"}
    script = ucd.by_code_point("Scripts.txt")
    extensions = {c: {long_name[s] for s in v.split()} for c, v in ucd.by_code_point("ScriptExtensions.txt").items()}
    # Common, Inherited and unassigned code points join their neighbors' runs, so this engine's may hold them.
    served = USE_SCRIPTS | INDIC_SCRIPTS | {"Common", "Inherited", "Unknown"}

    # Sinhala's al-lakuna ends two of the vowels the spec decomposes before reading clusters, so it must follow a
    # vowel as a halant can't: any virama Unicode's decompositions place after a dependent vowel's first part.
    vowel_viramas = {part for c, parts in decomposed.items() if syllabic.get(c) == "Vowel_Dependent"
                     for part in parts[1:] if syllabic.get(part) == "Virama"}

    def classify(c):
        isc, gc = syllabic.get(c), general.get(c)
        # The chart's VS, and the variation selectors it leaves out, arrive as CGJ as use.grammar reads them.
        if c == GRAPHEME_JOINER or "VARIATION SELECTOR" in name.get(c, ""):
            return "CGJ"
        if c in BASE_OTHER:
            return "GB"
        if c == SAKOT:
            return "Sakot"
        if c in vowel_viramas:
            return "VowelVirama"
        cls = CLASS_OF.get(isc)
        if isc in UNLESS_LETTER:
            cls = "B" if gc == "Lo" else UNLESS_LETTER[isc]
        if cls is None and joining.get(c) in ("C", "D", "L", "R"):
            cls = "B"
        # Where the chart says nothing: whatever is invisible takes no mark, as its word joiner doesn't, and a mark
        # without a syllabic category modifies the cluster it follows, by where it's drawn, rather than breaking it.
        if cls is None and c in ignorable:
            cls = "WJ"
        if cls is None and gc in ("Mn", "Mc") and c in positional:
            cls = "VM"
        if cls in SPLIT:
            cls += side(positional.get(c, ""))
        return cls

    found = {}
    for c in range(0x110000):
        if extensions.get(c, {script.get(c, "Unknown")}) & served:
            cls = classify(c)
            if cls and cls != "O":
                found[c] = cls
    return found


def write(found, numbers):
    runs = []
    for c in sorted(found):
        if runs and runs[-1][1] == c - 1 and runs[-1][2] == numbers[found[c]]:
            runs[-1][1] = c
        else:
            runs.append([c, c, numbers[found[c]]])
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("// Generated by scripts/tools/gen/use-table.py – do not edit.\n")
        f.write("//\n")
        f.write("// The Universal Shaping Engine's class of every character it can meet; any other is O,\n")
        f.write("// the grammar's catch-all.\n")
        f.write("//\n")
        f.write("// Derived in part from the Unicode Character Database. Copyright (c) Unicode,\n")
        f.write("// Inc.; the license notice is reproduced in full at the top of\n")
        f.write("// src/daecore/src/daeshaper/generated/unicode_tables.rs.\n")
        f.write("//\n")
        f.write("// Derived in part from Microsoft's Universal Shaping Engine overrides, under this notice,\n")
        f.write("// reproduced verbatim from scripts/data/unicode/MicrosoftLicense.txt:\n")
        f.write("//\n")
        with open(ucd.path("MicrosoftLicense.txt"), encoding="utf-8") as notice:
            for line in notice.read().strip().splitlines():
                f.write("// {}\n".format(line) if line.strip() else "//\n")
        f.write("\n")
        f.write("use crate::daecore::daeshaper::unicode::{Script, Trie};\n\n")
        f.write("// The scripts the Universal Shaping Engine shapes.\n")
        f.write("pub(crate) static USE_SCRIPTS: &[Script] = &[\n")
        for s in sorted(USE_SCRIPTS):
            f.write("    Script::{},\n".format(s.upper()))
        f.write("];\n\n")
        emit_consts(f, "category", sorted((v, k) for k, v in numbers.items()),
                    "the grammar's categories; the passes name a subset")
        stats = trie.emit(
            f, "USE_CATEGORY", "UseCategoryRecord", [("category", "u8")], [runs], [0],
            "// The USE category of a codepoint. Absent is 0 (O), which is also the trie's\n"
            "// default, so the whole of Unicode outside these scripts shares one leaf block.\n",
        )
        print("  use trie        {} records, {} mid blocks, {} leaf blocks".format(*stats))
    return len(runs)


def main():
    numbers = category_numbers()
    found = classes()
    unknown = set(found.values()) - set(numbers)
    if unknown:
        sys.exit("use.grammar has no category for {}".format(", ".join(sorted(unknown))))
    print("  {} ranges over {} codepoints".format(write(found, numbers), len(found)))


if __name__ == "__main__":
    main()
