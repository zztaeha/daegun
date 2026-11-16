use super::super::decoder::{f2dot14, fixed};
use super::super::format::ivs::{round_fixed, units};
use super::{Colrv1Ctx, Deltas};

const NO_VARIATION_INDEX: u32 = 0xFFFF_FFFF;

// A field's unit, in which deltas apply. The result is not clamped to the stored type: the spec
// lets varied angles, stop offsets, scales and radii leave it.
#[derive(Clone, Copy)]
pub(super) enum VarField {
    Raw,
    F2Dot14,
    Fixed1616,
    Alpha,
}

impl VarField {
    fn value(self, raw: f64) -> f64 {
        match self {
            VarField::Raw => raw,
            VarField::F2Dot14 | VarField::Alpha => f2dot14(raw),
            VarField::Fixed1616 => fixed(raw),
        }
    }
}

// Not rounded to the field's unit either: a varied value keeps its fraction, and the sum is taken whole, never
// wrapped to the field's 16 bits.
pub(super) fn resolve_var_field(
    raw_base: i32, field: VarField, var_index_base: u32, field_position: u32, ctx: &Colrv1Ctx,
) -> f64 {
    field.value(raw_base as f64 + resolve_delta(ctx.deltas, var_index_base, field_position))
}

pub(super) fn resolve_var_angle(
    raw_base: i16, bias: f64, var_index_base: u32, field_position: u32, ctx: &Colrv1Ctx,
) -> f64 {
    (resolve_var_field(raw_base as i32, VarField::F2Dot14, var_index_base, field_position, ctx) + bias) * 180.0
}

pub(super) fn decode_angle(raw: i16, bias: f64) -> f64 {
    (f2dot14(raw) + bias) * 180.0
}

// A field's delta rounded to a whole unit of the field, where the varied value is stored.
pub(super) fn resolve_delta(deltas: Deltas, var_index_base: u32, field_position: u32) -> f64 {
    f64::from(round_fixed(resolve_fixed(deltas, var_index_base, field_position)))
}

// A field's delta in its units, unrounded: a clip box rounds its edges outward from it, as the COLR spec asks.
pub(super) fn resolve_units(deltas: Deltas, var_index_base: u32, field_position: u32) -> f64 {
    units(resolve_fixed(deltas, var_index_base, field_position))
}

// A field's index is its base plus its place, none past 0xFFFFFFFF rather than wrapping; with no map it is outer in
// its high word, inner in its low, as the spec's implicit mapping has it.
fn resolve_fixed(deltas: Deltas, var_index_base: u32, field_position: u32) -> i64 {
    if var_index_base == NO_VARIATION_INDEX { return 0; }
    let Some(idx) = var_index_base.checked_add(field_position) else { return 0 };
    let implicit = ((idx >> 16) as usize, (idx & 0xFFFF) as usize);
    match deltas {
        Deltas::Drawn(at) => at.map_or(0, |(view, scalars)| view.delta(idx as usize, implicit, scalars)),
        Deltas::Measured(Some(source), scalars) => source.delta(idx as usize, implicit, scalars),
        Deltas::Measured(None, _) => 0,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/colr_v1/varfield.rs"]
mod tests;
