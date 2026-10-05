use super::*;

pub mod bytes {
    pub use crate::daecore::daetype::decoder::{
        read_i16_be, read_offset24, read_u16_be, read_u24_be, read_u32_be, records_fit,
        search_records, window, write_i16_be, write_offset24, write_u16_be, write_u32_be,
    };
}

pub use crate::daecore::daetype::decoder::build_ttf as build_font;

pub mod format {
    pub use crate::daecore::daetype::format::aat::{class, state, Entry, Lookup, StateTable};
    pub use crate::daecore::daetype::format::ankr::{control_point, version as ankr_version, Ankr};
    pub use crate::daecore::daetype::format::coverage::coverage_index;
    pub use crate::daecore::daetype::subsetter::otl::parse_coverage as coverage_glyphs;
    pub use crate::daecore::daetype::format::feature_variations::FeatureVariations;
    pub use crate::daecore::daetype::format::ivs::{
        compute_ivs_delta_f64, delta_set_index_map_lookup, parse_delta_set_index_map,
        parse_item_variation_store, precompute_region_scalars, ItemVariationStore, Ivd, RegionAxis,
    };
    pub use crate::daecore::daetype::format::round::ot_round;
}

impl Font {
    pub fn table(&self, tag: &str) -> Option<&[u8]> {
        self.cache.table_map.get(tag).map(|t| t.as_slice())
    }

    pub fn table_tags(&self) -> Vec<&str> {
        self.cache.table_map.keys().map(String::as_str).collect()
    }

    pub fn has_table(&self, tag: &str) -> bool {
        self.cache.table_map.contains_key(tag)
    }

    pub fn instance_tables(
        &self,
        axes: &[(&str, f64)],
    ) -> Option<alloc::collections::BTreeMap<String, alloc::borrow::Cow<'_, [u8]>>> {
        let stored = || {
            self.cache.table_map.iter().map(|(tag, data)| (tag.clone(), alloc::borrow::Cow::Borrowed(data.as_slice())))
                .collect()
        };
        // A font whose fvar is missing or does not parse is static to the instancer, and a static font is
        // its own instance. Its CFF2 is converted to CFF only to be drawn or subset.
        if self.is_static() {
            return Some(stored());
        }
        let canonical = crate::daecore::cache::canonical_axes(axes);
        Some(
            crate::daecore::daetype::instancer::instance_tables_from_map(&self.cache.table_map, &canonical)
                .unwrap_or_else(|_| stored()),
        )
    }
}
