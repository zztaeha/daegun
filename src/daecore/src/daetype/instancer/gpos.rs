use alloc::vec::Vec;

use super::super::format::ivs::{round_fixed, store_delta, store_scalars};
use super::super::subsetter::{instance_gpos, mark_glyph_set_count};
use super::super::subsetter::otl::gdef::var_store;

// GPOS at `location`, its VariationIndex deltas read from GDEF's store.
pub(crate) fn apply_gpos_var(gpos: &[u8], gdef: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    let store = var_store(gdef)?;
    let scalars = store_scalars(&store, location);
    let vary = |outer: u16, inner: u16| round_fixed(store_delta(&store, outer.into(), inner.into(), &scalars));
    // An instance keeps every glyph, so each mark glyph set keeps its index.
    instance_gpos(gpos, &(0..mark_glyph_set_count(gdef)).map(Some).collect::<Vec<_>>(), &vary)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/gpos.rs"]
mod tests;
