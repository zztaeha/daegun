use super::super::format::ivs::{compute_ivs_delta_f64, ItemVariationStore};
use super::Colrv1Ctx;

const NO_VARIATION_INDEX: u32 = 0xFFFF_FFFF;

// A field's unit, in which deltas apply. The result is not clamped to the stored type: the spec
// lets varied angles, stop offsets, scales and radii leave it.
#[derive(Clone, Copy)]
pub(super) enum VarField {
    Raw,
    F2Dot14,
    Fixed1616,
}

impl VarField {
    fn divisor(self) -> f64 {
        match self {
            VarField::Raw => 1.0,
            VarField::F2Dot14 => 16384.0,
            VarField::Fixed1616 => 65536.0,
        }
    }
}

// Not rounded to the field's unit either, as fontTools and HarfBuzz read a varied value.
pub(super) fn resolve_var_field(
    raw_base: i32, field: VarField, var_index_base: u32, field_position: u32, ctx: &Colrv1Ctx,
) -> f64 {
    (raw_base as f64 + resolve_var_delta(var_index_base, field_position, ctx)) / field.divisor()
}

pub(super) fn resolve_var_angle(
    raw_base: i16, bias: f64, var_index_base: u32, field_position: u32, ctx: &Colrv1Ctx,
) -> f64 {
    let f2dot14 = resolve_var_field(raw_base as i32, VarField::F2Dot14, var_index_base, field_position, ctx);
    (f2dot14 + bias) * 180.0
}

pub(super) fn decode_angle(raw: i16, bias: f64) -> f64 {
    (raw as f64 / 16384.0 + bias) * 180.0
}

fn resolve_var_delta(var_index_base: u32, field_position: u32, ctx: &Colrv1Ctx) -> f64 {
    resolve_delta_raw(ctx.var_store, ctx.var_index_map, ctx.region_scalars, var_index_base, field_position)
}

pub(super) fn resolve_delta_raw(
    var_store: Option<&ItemVariationStore>,
    var_index_map: Option<&[(u32, u32)]>,
    region_scalars: &[f64],
    var_index_base: u32,
    field_position: u32,
) -> f64 {
    if var_index_base == NO_VARIATION_INDEX { return 0.0; }
    let Some(store) = var_store else { return 0.0 };
    // Indices do not wrap past 0xFFFFFFFF; with no map, an index is outer in its high word, inner in
    // its low.
    let Some(idx) = var_index_base.checked_add(field_position) else { return 0.0 };
    let (outer, inner) = match var_index_map {
        Some(map) if !map.is_empty() => super::super::format::ivs::delta_set_index_map_lookup(map, idx as usize),
        _ => ((idx >> 16) as usize, (idx & 0xFFFF) as usize),
    };
    compute_ivs_delta_f64(store, outer, inner, region_scalars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::format::ivs::{parse_item_variation_store, precompute_region_scalars};

    // With no map, an index is outer in its high word and inner in its low: 0x00050000 is row 0 of
    // the sixth data subtable, not row 327,680 of the first.
    #[test]
    fn with_no_map_an_index_splits_into_outer_and_inner() {
        let store = super::super::testing::store(6, -50);
        let ivs = parse_item_variation_store(&store, 0).expect("a store");
        let scalars = precompute_region_scalars(&ivs, &[1.0]);
        assert_eq!(resolve_delta_raw(Some(&ivs), None, &scalars, 0x0005_0000, 0), -50.0);
        assert_eq!(resolve_delta_raw(Some(&ivs), Some(&[]), &scalars, 0x0005_0000, 0), -50.0, "an empty map is no map");
    }
}
