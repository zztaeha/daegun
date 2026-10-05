use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::lookup_list;
use super::generic::{self, schemas, Devices};

fn pass(gpos: &[u8], active: &GlyphSet, gid_map: &[u16], mark_sets: u16, devices: Devices) -> Option<Vec<u8>> {
    let subtable: lookup_list::SubtableSubsetter = &|_, schema, buf, off, active, gid_map, work| {
        generic::subset_subtable(buf, off, schema?, active, gid_map, devices, work)
    };
    lookup_list::subset_lookup_table(gpos, 9, active, gid_map, mark_sets, subtable, &schemas::gpos_schema_for_type)
}

// GPOS with its Device and VariationIndex tables, or without them when they will not fit. A lookup
// keeps its mark filtering set only if it is one of the `mark_sets` the subset's GDEF holds.
pub fn subset_gpos(gpos: &[u8], active: &GlyphSet, gid_map: &[u16], mark_sets: u16) -> Option<Vec<u8>> {
    pass(gpos, active, gid_map, mark_sets, Devices::Keep)
        .or_else(|| pass(gpos, active, gid_map, mark_sets, Devices::Strip))
}

// Every glyph's GPOS at an instance, each VariationIndex's delta in the value it adjusts, written
// out afresh for a record that must gain a value to hold one.
pub(crate) fn instance_gpos(
    gpos: &[u8], n_glyphs: u16, mark_sets: u16, vary: &dyn Fn(u16, u16) -> i32,
) -> Option<Vec<u8>> {
    let mut active = GlyphSet::new();
    (0..n_glyphs).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u16> = (0..n_glyphs).collect();
    pass(gpos, &active, &gid_map, mark_sets, Devices::Vary(vary))
}
