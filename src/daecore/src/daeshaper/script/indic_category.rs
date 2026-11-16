use crate::daecore::daeshaper::generated::indic_tables as t;

pub(crate) use t::{category, position};

// Whether the Indic, Khmer and Myanmar grammars can take `c` in a syllable but not as the one that opens it.
pub(crate) fn is_dependent(c: u32) -> bool {
    use category::*;
    !matches!(lookup(c).0, X | C | V | RA | STACKER | SYMBOL | PLACEHOLDER | DOTTED_CIRCLE)
}

pub(crate) fn lookup(c: u32) -> (u8, u8) {
    if c >= 0x0011_0000 {
        return (category::X, position::LAST);
    }
    let rec = t::INDIC_CATEGORY.get(c);
    (rec.category, rec.position)
}
