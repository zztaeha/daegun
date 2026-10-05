use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::decoder::{read_i16_be, read_u16_be, read_u32_be, write_i16_be, write_u16_be, write_u32_be};
use super::super::format::ivs::{compute_ivs_delta_f64, parse_item_variation_store, precompute_region_scalars};
use super::super::format::round::ot_round;

// GDEF's ligature carets of format 3 take their delta from GDEF's own store. The instance bakes
// each at its location as format 1 and drops the store, which nothing can read without fvar.
pub(crate) fn bake_gdef(gdef: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    if read_u16_be(gdef, 0)? != 1 || read_u16_be(gdef, 2)? < 3 {
        return None;
    }
    let mut carets = Vec::new();
    if let Some(list) = offset(gdef, 0, 8) {
        let mut seen = BTreeSet::new();
        for i in 0..records(gdef, list + 2, 2) {
            let Some(lig) = offset(gdef, list, 4 + 2 * i).filter(|&l| seen.insert(l)) else { continue };
            carets.extend((0..records(gdef, lig, 2)).filter_map(|j| offset(gdef, lig, 2 + 2 * j)));
        }
    }
    bake(gdef, 14, carets, location)
}

// BASE 1.1 does the same for its coordinates of format 3. Shared scripts, value lists and extents
// are walked once.
pub(crate) fn bake_base(base: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    if read_u16_be(base, 0)? != 1 || read_u16_be(base, 2)? < 1 {
        return None;
    }
    let mut coords = Vec::new();
    let mut seen = BTreeSet::new();
    for axis in [4, 6].into_iter().filter_map(|field| offset(base, 0, field)) {
        let Some(list) = offset(base, axis, 2) else { continue };
        for i in 0..records(base, list, 6) {
            let Some(script) = offset(base, list, 2 + 6 * i + 4).filter(|&s| seen.insert(s)) else { continue };
            if let Some(values) = offset(base, script, 0).filter(|&v| seen.insert(v)) {
                coords.extend((0..records(base, values + 2, 2)).filter_map(|j| offset(base, values, 4 + 2 * j)));
            }
            let langs = (0..records(base, script + 4, 6)).filter_map(|k| offset(base, script, 6 + 6 * k + 4));
            for min_max in offset(base, script, 2).into_iter().chain(langs).collect::<Vec<_>>() {
                if !seen.insert(min_max) {
                    continue;
                }
                coords.extend([offset(base, min_max, 0), offset(base, min_max, 2)].into_iter().flatten());
                for k in 0..records(base, min_max + 4, 8) {
                    coords.extend([offset(base, min_max, 10 + 8 * k), offset(base, min_max, 12 + 8 * k)].into_iter().flatten());
                }
            }
        }
    }
    bake(base, 8, coords, location)
}

// A non-NULL Offset16 at `from + field`, resolved against `from`.
fn offset(table: &[u8], from: usize, field: usize) -> Option<usize> {
    read_u16_be(table, from + field).filter(|&r| r != 0).map(|r| from + usize::from(r))
}

// The count stated at `at`, cut to the records of `size` bytes that fit after it.
fn records(table: &[u8], at: usize, size: usize) -> usize {
    let stated = read_u16_be(table, at).map_or(0, usize::from);
    stated.min(table.len().saturating_sub(at + 2) / size)
}

// Each value at an offset in `values` laid out as format, coordinate and device offset. Patching
// reads the source and writes the copy, so a value named twice is baked once.
fn bake(table: &[u8], store_field: usize, values: Vec<usize>, location: &[f64]) -> Option<Vec<u8>> {
    let store_at = read_u32_be(table, store_field)? as usize;
    if store_at == 0 {
        return None;
    }
    let mut out = table.to_vec();
    write_u32_be(&mut out, store_field, 0);
    let Ok(store) = parse_item_variation_store(table, store_at) else { return Some(out) };
    let scalars = precompute_region_scalars(&store, location);
    for at in values {
        if read_u16_be(table, at) != Some(3) {
            continue;
        }
        let (Some(coordinate), Some(device)) = (read_i16_be(table, at + 2), read_u16_be(table, at + 4)) else { continue };
        let device = at + usize::from(device);
        if device == at || read_u16_be(table, device + 4) != Some(0x8000) {
            continue;
        }
        let (Some(outer), Some(inner)) = (read_u16_be(table, device), read_u16_be(table, device + 2)) else { continue };
        let value = f64::from(coordinate) + compute_ivs_delta_f64(&store, outer.into(), inner.into(), &scalars);
        write_u16_be(&mut out, at, 1);
        write_i16_be(&mut out, at + 2, ot_round(value).clamp(i16::MIN.into(), i16::MAX.into()) as i16);
    }
    Some(out)
}

pub(crate) fn strip_colr_var_store(colr: &[u8]) -> Option<Vec<u8>> {
    if colr.len() < 34 { return None; }
    let version = read_u16_be(colr, 0)?;
    if version != 1 || read_u32_be(colr, 30)? == 0 { return None; }
    let mut out = colr.to_vec();
    write_u32_be(&mut out, 26, 0);
    write_u32_be(&mut out, 30, 0);
    Some(out)
}
