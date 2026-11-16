use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::lookup_list;
use super::generic::{self, schemas, Devices, InPlace, Kept, Parsed, Stop};

fn pass(gpos: &[u8], kept: Kept, mark_sets: &[Option<u16>], devices: Devices, held: bool) -> Option<Vec<u8>> {
    let subtable: lookup_list::SubtableSubsetter = &|_, schema, buf, off, work| {
        generic::parse_subtable(buf, off, schema?, kept, devices, work).map(Parsed::Value)
    };
    lookup_list::subset_lookup_table(gpos, 9, mark_sets, subtable, schemas::gpos_schema_for_type, held)
}

// GPOS with its Device and VariationIndex tables, or without them when they will not fit. Its VariationIndex tables
// go unless `varied`, GDEF holding a store for them to name; mark sets and `held` are as `subset_gsub` takes them.
pub fn subset_gpos(
    gpos: &[u8], active: &GlyphSet, gid_map: &[u32], mark_sets: &[Option<u16>], held: bool, varied: bool,
) -> Option<Vec<u8>> {
    let kept = Kept::Subset(active, gid_map);
    let devices = if varied { Devices::Keep } else { Devices::Sizes };
    pass(gpos, kept, mark_sets, devices, held).or_else(|| pass(gpos, kept, mark_sets, Devices::Strip, held))
}

// GPOS at an instance, each VariationIndex's delta added to the value it adjusts: in place, or written
// out afresh, every glyph kept, when a record must gain a value to hold its delta.
pub(crate) fn instance_gpos(
    gpos: &[u8], mark_sets: &[Option<u16>], vary: &impl Fn(u16, u16) -> i32,
) -> Option<Vec<u8>> {
    let mut walk = InPlace::new(gpos, vary);
    let (mut walked, mut offsets) = (BTreeSet::new(), lookup_list::OffsetBudget::for_table(gpos.len()));
    for lookup in lookup_list::lookups(gpos)?.flatten().filter(|&l| walked.insert(l)) {
        for (kind, at) in lookup_list::subtables(gpos, lookup, 9, &mut offsets)? {
            // Value records and anchors are held by lookup types 1 to 6; contexts hold none.
            let Some(schema) = schemas::gpos_schema_for_type(kind).filter(|_| kind <= 6) else { continue };
            match walk.subtable(at, schema) {
                Ok(()) => {}
                Err(Stop::Widen) => return pass(gpos, Kept::All, mark_sets, Devices::Vary(vary), true),
                Err(Stop::Budget) => return None,
            }
        }
    }
    Some(walk.out)
}
