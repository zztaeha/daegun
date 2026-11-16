pub mod context;
pub mod gsub;
pub mod gpos;
pub mod gdef;

use super::schema::Schema;

pub(crate) fn gsub_schema_for_type(effective_type: u16) -> Option<&'static Schema> {
    match effective_type {
        1 => Some(&gsub::SINGLE_SUBST),
        2 => Some(&gsub::MULTIPLE_SUBST),
        3 => Some(&gsub::ALTERNATE_SUBST),
        4 => Some(&gsub::LIGATURE_SUBST),
        5 => Some(&context::CONTEXT),
        6 => Some(&context::CHAIN_CONTEXT),
        _ => None,
    }
}

pub(crate) fn gpos_schema_for_type(effective_type: u16) -> Option<&'static Schema> {
    match effective_type {
        1 => Some(&gpos::SINGLE_POS),
        2 => Some(&gpos::PAIR_POS),
        3 => Some(&gpos::CURSIVE_POS),
        4 => Some(&gpos::MARK_BASE_POS),
        6 => Some(&gpos::MARK_MARK_POS),
        5 => Some(&gpos::MARK_LIG_POS),
        7 => Some(&context::CONTEXT),
        8 => Some(&context::CHAIN_CONTEXT),
        _ => None,
    }
}
