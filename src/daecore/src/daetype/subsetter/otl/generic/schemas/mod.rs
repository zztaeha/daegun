pub mod context;
pub mod gsub;
pub mod gpos;
pub mod gdef;

use super::schema::Schema;

// A subtable of a type that defines format 1 only: another number is not read as format 1.
fn format_1(schema: Schema) -> Schema {
    Schema::FormatSwitch(0, alloc::vec![(1, schema)])
}

pub(crate) fn gsub_schema_for_type(effective_type: u16) -> Option<Schema> {
    match effective_type {
        1 => Some(gsub::single_subst_schema()),
        2 => Some(gsub::multiple_subst_schema()),
        3 => Some(gsub::alternate_subst_schema()),
        4 => Some(gsub::ligature_subst_schema()),
        5 => Some(gsub::context_subst_schema()),
        6 => Some(gsub::chain_context_subst_schema()),
        _ => None,
    }
}

pub(crate) fn gpos_schema_for_type(effective_type: u16) -> Option<Schema> {
    match effective_type {
        1 => Some(gpos::single_pos_schema()),
        2 => Some(gpos::pair_pos_schema()),
        3 => Some(gpos::cursive_pos_schema()),
        4 | 6 => Some(gpos::mark_attach_schema()),
        5 => Some(gpos::mark_lig_pos_schema()),
        7 => Some(gpos::context_pos_schema()),
        8 => Some(gpos::chain_context_pos_schema()),
        _ => None,
    }
}
