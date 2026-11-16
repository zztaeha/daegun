use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::decoder::{
    clamp_i16, fitting_count, offset16_in, read_i16_be, read_u16_be, read_u32_be, read_uint, write_i16_be,
    write_u16_be, write_u32_be,
};
use super::super::decoder::resolve_offset16;
use super::super::format::ivs::{round_fixed, store_delta, store_scalars, variation_index, ItemVariationStore};
use super::super::math_table::{construction, kern_heights, Covered, CONSTANTS_LEN};
use super::super::subsetter::otl::gdef::GdefHeader;
use super::super::subsetter::subset_budget;

// GDEF's ligature carets of format 3 take their delta from GDEF's own store. The instance bakes
// each at its location as format 1 and drops the store, which nothing can read without fvar. Ligatures at distinct
// offsets can still overlap, so the carets read are held to the subset budget; past it GDEF stays as stored.
pub(crate) fn bake_gdef(gdef: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    let header = GdefHeader::read(gdef)?;
    if header.var_store == 0 {
        return None;
    }
    let mut carets = Vec::new();
    let list = header.lig_caret_list;
    if list != 0 {
        // 1.4's list counts and reaches its ligatures in 24 bits, past a 32-bit coverage offset.
        let (width, at) = if header.lig_carets_24() { (3, list + 4) } else { (2, list + 2) };
        let count = read_uint(gdef, at, width).unwrap_or(0);
        let ligs = (0..count as usize).map_while(|i| read_uint(gdef, at + width + width * i, width));
        let (mut seen, mut left) = (BTreeSet::new(), subset_budget(gdef.len()));
        for lig in ligs.filter(|&rel| rel != 0).map(|rel| list + rel as usize).filter(|&l| seen.insert(l)) {
            let count = fitting_count(gdef, lig, 2);
            left = left.checked_sub(count)?;
            carets.extend((0..count).filter_map(|j| offset16_in(gdef, lig, 2 + 2 * j)));
        }
    }
    bake(gdef, 14, carets, location)
}

// BASE 1.1 does the same for its coordinates of format 3, and stays as stored where they run past their budget.
pub(crate) fn bake_base(base: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    if read_u16_be(base, 0)? != 1 || read_u16_be(base, 2)? < 1 {
        return None;
    }
    bake(base, 8, super::super::base::base_coordinates(base)?, location)
}

// Each value at an offset in `values` laid out as format, coordinate and device offset, its delta rounded alone as a
// device's is. Patching reads the source and writes the copy, so a value named twice is baked once.
fn bake(table: &[u8], store_field: usize, values: Vec<usize>, location: &[f64]) -> Option<Vec<u8>> {
    let store_at = read_u32_be(table, store_field)? as usize;
    if store_at == 0 {
        return None;
    }
    let mut out = table.to_vec();
    write_u32_be(&mut out, store_field, 0);
    let Ok(store) = ItemVariationStore::parse(table, store_at) else { return Some(out) };
    let scalars = store_scalars(&store, location);
    for at in values {
        if read_u16_be(table, at) != Some(3) {
            continue;
        }
        let Some(coordinate) = read_i16_be(table, at + 2) else { continue };
        let Some((outer, inner)) = offset16_in(table, at, 4).and_then(|device| variation_index(table, device)) else {
            continue;
        };
        let delta = round_fixed(store_delta(&store, outer.into(), inner.into(), &scalars));
        write_u16_be(&mut out, at, 1);
        write_i16_be(&mut out, at + 2, clamp_i16(i32::from(coordinate).saturating_add(delta)));
    }
    Some(out)
}

// MATH's values whose devices are VariationIndex ones, naming GDEF's store: each takes its delta at the location,
// rounded alone, and drops its device, where a device of sizes stays. Without the store a value stays as it is.
pub(crate) fn bake_math(math: &[u8], store: Option<&ItemVariationStore>, location: &[f64]) -> Option<Vec<u8>> {
    let scalars = store.map(|store| (store, store_scalars(store, location)));
    let mut out = None;
    for (record, base) in math_values(math)? {
        let Some(device) = resolve_offset16(math, record + 2, base) else { continue };
        let (Some((outer, inner)), Some(value)) = (variation_index(math, device), read_i16_be(math, record)) else {
            continue;
        };
        let delta = scalars.as_ref().map_or(0, |(store, scalars)| {
            round_fixed(store_delta(store, outer.into(), inner.into(), scalars))
        });
        let out = out.get_or_insert_with(|| math.to_vec());
        write_i16_be(out, record, clamp_i16(i32::from(value).saturating_add(delta)));
        write_u16_be(out, record + 2, 0);
    }
    out
}

// Each MathValueRecord as where it lies and the subtable its device counts from, each MathKern read once however many
// records name it. MathKerns at distinct offsets can still overlap, so their values are held to the subset budget;
// None past it, which leaves MATH as stored.
fn math_values(math: &[u8]) -> Option<BTreeSet<(usize, usize)>> {
    let mut values = BTreeSet::new();
    if let Some(at) = offset16_in(math, 0, 4).filter(|&at| at + CONSTANTS_LEN <= math.len()) {
        values.extend((0..51).map(|i| (at + 8 + 4 * i, at)));
    }
    if let Some(info) = offset16_in(math, 0, 6) {
        for at in [0, 2].into_iter().filter_map(|field| offset16_in(math, info, field)) {
            let Some(records) = Covered::at(math, at, 4) else { continue };
            values.extend((0..records.count(math)).map(|i| (records.records + 4 * i, at)));
        }
        if let Some(at) = offset16_in(math, info, 6)
            && let Some(records) = Covered::at(math, at, 8)
        {
            let corners = (0..4 * records.count(math)).filter_map(|i| offset16_in(math, at, 4 + 2 * i));
            let mut left = subset_budget(math.len());
            for kern in corners.collect::<BTreeSet<_>>() {
                let Some(heights) = kern_heights(math, kern) else { continue };
                left = left.checked_sub(2 * heights + 1)?;
                values.extend((0..2 * heights + 1).map(|k| (kern + 2 + 4 * k, kern)));
            }
        }
    }
    if let Some(variants) = offset16_in(math, 0, 8) {
        for direction in [true, false].into_iter().filter_map(|vertical| Covered::variants(math, variants, vertical)) {
            let constructions = (0..direction.count(math)).filter_map(|i| {
                resolve_offset16(math, direction.records + 2 * i, variants)
            });
            let assemblies = constructions.filter_map(|at| construction(math, at)?.assembly);
            values.extend(assemblies.map(|assembly| (assembly.at, assembly.at)));
        }
    }
    Some(values)
}

pub(crate) fn strip_colr_var_store(colr: &[u8]) -> Option<Vec<u8>> {
    super::super::colr_v1::header(colr)?.var_store?;
    let mut out = colr.to_vec();
    write_u32_be(&mut out, 26, 0);
    write_u32_be(&mut out, 30, 0);
    Some(out)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/strip_var_stores.rs"]
mod tests;
