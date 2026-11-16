// What Unicode says of a character, for the shaper and the text tools: one generated record per code point
// (`scripts/tools/gen/unicode-tables.py`) and the few facts the standard states as ranges.
pub mod bidi;
pub mod linebreak;
pub mod segment;
pub mod itemize;
#[cfg(feature = "dictionaries")]
mod words;

mod canonical;
mod marks;
mod scripts;
mod spaces;
mod trie;
mod vertical;

use super::generated::unicode_tables as t;

pub use super::generated::unicode_tables::{GeneralCategory, VerticalOrientation};
pub(crate) use super::generated::unicode_tables::JoiningType;
pub(crate) use canonical::{compose, composes_second, decompose, hangul_jamo, hangul_syllable, hebrew_presentation_form};
pub(crate) use marks::mark_class;
pub use scripts::{Script, ScriptDirection};
pub(crate) use scripts::horizontal_direction;
pub(crate) use spaces::{space_fallback, SpaceFallback};
pub(crate) use trie::Trie;
pub use vertical::{vertical_form, vertical_orientation};
pub(crate) use vertical::is_upright;

// Drawn for a missing base. UAX #14's LB28a names it beside the AK and AS classes, as it stands in for a missing
// consonant when a cluster is shown alone.
pub(crate) const DOTTED_CIRCLE: char = '\u{25CC}';

// ASCII's records, copied out of the trie as the crate builds: most text is ASCII, and one load beats the trie's
// chain of four.
static ASCII: [t::Props; 128] = {
    let mut records = [*t::PROPS.get(0); 128];
    let mut cp = 1;
    while cp < records.len() {
        records[cp] = *t::PROPS.get(cp as u32);
        cp += 1;
    }
    records
};

#[inline]
pub(crate) fn props(c: char) -> &'static t::Props {
    match ASCII.get(c as usize) {
        Some(record) => record,
        None => t::PROPS.get(u32::from(c)),
    }
}

// Whether `c`'s record has `flag`, one of `t::flag`; each flag but the first two stands before a table's search.
fn flagged(c: char, flag: u8) -> bool {
    props(c).flags & flag != 0
}

// The generator numbers the categories in order of their UCD short names (Cn, the default, first), so each major
// class is one span of them.
impl GeneralCategory {
    pub(crate) fn is_mark(self) -> bool {
        (GeneralCategory::SpacingMark as u8..=GeneralCategory::NonspacingMark as u8).contains(&(self as u8))
    }

    pub(crate) fn is_letter(self) -> bool {
        (GeneralCategory::LowercaseLetter as u8..=GeneralCategory::UppercaseLetter as u8).contains(&(self as u8))
    }
}

/// A character's General_Category.
///
/// ```
/// use daegun::unicode::{general_category, GeneralCategory};
///
/// assert_eq!(general_category('A'), GeneralCategory::UppercaseLetter);
/// assert_eq!(general_category('\u{301}'), GeneralCategory::NonspacingMark);
/// ```
pub fn general_category(c: char) -> GeneralCategory {
    props(c).general_category
}

/// A character's Script property: [`Script::COMMON`] for one many scripts share, such as a digit or a space, and
/// [`Script::INHERITED`] for a mark that takes its base's.
///
/// ```
/// use daegun::unicode::{script, Script};
///
/// assert_eq!(script('ب'), Script::ARABIC);
/// assert_eq!(script('1'), Script::COMMON);
/// ```
pub fn script(c: char) -> Script {
    Script(props(c).script)
}

// The scripts Script_Extensions gives `c` (UAX #24), where they are more than its own.
pub(crate) fn script_extensions(c: char) -> Option<&'static [Script]> {
    if !flagged(c, t::flag::SCRIPT_EXTENSIONS) {
        return None;
    }
    let cp = u32::from(c);
    let at = t::SCRIPT_EXTENSIONS.partition_point(|&(_, last, _)| last < cp);
    let &(_, _, set) = t::SCRIPT_EXTENSIONS.get(at).filter(|&&(first, _, _)| first <= cp)?;
    t::SCRIPT_EXTENSION_SETS.get(usize::from(set)).copied()
}

// Bidi_Mirroring_Glyph (UAX #9's L4): the character drawn for `c` in right-to-left text.
pub(crate) fn mirrored(c: char) -> Option<char> {
    if !flagged(c, t::flag::HAS_MIRROR) {
        return None;
    }
    let at = t::MIRRORING.binary_search_by_key(&u32::from(c), |&(from, _)| from).ok()?;
    char::from_u32(t::MIRRORING[at].1)
}

pub(crate) fn is_default_ignorable(c: char) -> bool {
    flagged(c, t::flag::DEFAULT_IGNORABLE)
}

pub(crate) fn is_extended_pictographic(c: char) -> bool {
    flagged(c, t::flag::EXTENDED_PICTOGRAPHIC)
}

pub(crate) fn joining_type(c: char) -> JoiningType {
    props(c).joining_type
}

// Standardized and ideographic variation selectors. Mongolian's free ones aren't among them: the Arabic model hands
// each the form of its base.
pub(crate) fn is_variation_selector(c: u32) -> bool {
    matches!(c, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

pub(crate) fn is_free_variation_selector(c: u32) -> bool {
    matches!(c, 0x180B..=0x180D | 0x180F)
}

pub(crate) fn is_regional_indicator(c: u32) -> bool {
    matches!(c, 0x1F1E6..=0x1F1FF)
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/mod.rs"]
mod tests;
