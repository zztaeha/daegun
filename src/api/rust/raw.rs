//! Font data at the byte level, for tools that read or write tables daegun has no call for.

/// Big-endian reads and writes that never panic: a read past the end gives none and a write past it does nothing.
pub mod bytes {
    pub use crate::daecore::daetype::decoder::{
        read_i16_be, read_offset24, read_u16_be, read_u24_be, read_u32_be, records_fit,
        search_records, window, write_i16_be, write_offset24, write_u16_be, write_u32_be,
    };
}

/// Readers for structures several tables share: AAT lookups, state tables and ankr, Coverage, FeatureVariations
/// and the ItemVariationStore.
pub mod format {
    pub use crate::daecore::daetype::format::aat::{class, state, Entry, GlyphValue, Lookup, StateTable};
    pub use crate::daecore::daetype::format::ankr::{control_point, Ankr, AnchorPoint};
    pub use crate::daecore::daetype::format::coverage::coverage_index;
    pub use crate::daecore::daetype::subsetter::otl::parse_coverage as coverage_glyphs;
    pub use crate::daecore::daetype::format::feature_variations::{FeatureVariations, Substitution};
    pub use crate::daecore::daetype::format::ivs::{
        delta_set_index_map_lookup, parse_delta_set_index_map, DeltaSetIndex, ItemVariationStore, Ivd, RegionAxis,
    };
    pub use crate::daecore::daetype::format::round::ot_round;
}

pub use crate::daecore::daetype::outline::outline_glyf_bytes;
pub use crate::daecore::daetype::subsetter::parse_loca;

/// A font file of these tables, its directory in tag order as the spec sorts it, whatever order they come in, and of
/// a tag given twice the last; each padded to four bytes and checksummed, with head's checkSumAdjustment set. No
/// bytes for no tables, which make no font.
///
/// # Errors
///
/// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for what a font's directory can't state: more than 65,535
/// tables, or a table of 4 GiB or more, or one starting 4 GiB or more into the file.
pub fn build_font<'a, V: AsRef<[u8]> + 'a>(
    tables: impl IntoIterator<Item = (&'a crate::daecore::Tag, &'a V)>,
) -> crate::Result<alloc::vec::Vec<u8>> {
    crate::daecore::daetype::decoder::build_ttf(tables)
}
