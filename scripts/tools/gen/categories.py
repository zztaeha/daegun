# The classes and positions the Indic, Khmer and Myanmar shapers read: indic-table.py writes them into the table, and
# syllable-machines.py holds each grammar's numbers to them.

# Each class is one of Microsoft's script development specifications or Unicode's Indic properties draw.
INDIC_CLASSES = [
    # The Devanagari specification's: consonant, independent vowel, nukta, halant, the joiners, matra, syllable
    # modifier and anudatta with the vedic signs, its NBSP as any stand-in base, and the circle for a missing one.
    "X", "C", "V", "N", "H", "ZWNJ", "ZWJ", "M", "SM", "A", "Placeholder", "DottedCircle",
    # The consonant that becomes a reph, and Unicode's preceding repha, medial consonant and consonant with stacker.
    "Ra", "Repha", "Medial", "Stacker",
    # A sign standing as a base, Gurmukhi's matra that follows its consonant, a syllable modifier written as a digit,
    # and a variation selector.
    "Symbol", "MPost", "SMDigit", "VS",
    # Vowels by where they're written, as the Myanmar and Khmer specifications class them.
    "VPre", "VAbv", "VBlw", "VPst",
    # Khmer's register shifters, robat, and signs above and after the base.
    "RegShift", "Robat", "AbvS", "PstS",
    # Myanmar's asat, its medials, Mon's medial la apart from medial ha, and the Pwo tones.
    "As", "MY", "MR", "MW", "MH", "ML", "PT",
]

# Where a reordered character sorts in its syllable, first to last: the reph, the pre-base matra and consonants, the
# base, then the matras and consonants above, below and after it by the specifications' affinities, and the signs.
INDIC_POSITIONS = [
    "FIRST", "REPH", "PRE_MATRA", "PRE_BASE",
    "BASE", "AFTER_BASE", "ABOVE",
    "BEFORE_BELOW", "BELOW", "AFTER_BELOW",
    "BEFORE_POST", "POST", "AFTER_POST",
    "SIGNS", "LAST",
]


def rust_name(name):
    # A capital that starts a word gets an underscore before it: VAbv is V_ABV, DottedCircle DOTTED_CIRCLE.
    out = ""
    for i, ch in enumerate(name):
        if i and name[i - 1] != "_" and ch.isupper() and i + 1 < len(name) and name[i + 1].islower():
            out += "_"
        out += ch.upper()
    return out


def emit_consts(f, module, names, reason=None):
    if reason:
        f.write('#[allow(dead_code, reason = "{}")]\n'.format(reason))
    f.write("pub(crate) mod {} {{\n".format(module))
    for value, name in names:
        f.write("    pub(crate) const {}: u8 = {};\n".format(rust_name(name), value))
    f.write("}\n\n")
