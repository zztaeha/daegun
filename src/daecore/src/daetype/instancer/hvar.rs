use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_u32_be, write_u16_be};
use super::super::format::ivs::{parse_item_variation_store, parse_delta_set_index_map, delta_set_index_map_lookup, compute_ivs_delta_f64, precompute_region_scalars};
use super::super::format::round::ot_round;
use crate::daecore::daetype::TableBytes;

// Every glyph its own long metric, with the count the data then holds. A count of 0 has no advance
// to repeat, so that data is kept as it is, with its count.
pub fn expand_metrics(mtx_data: &[u8], num_glyphs: usize, long_metrics: usize) -> (Vec<u8>, usize) {
    if long_metrics >= num_glyphs || long_metrics == 0 {
        return (mtx_data.to_vec(), long_metrics);
    }
    let last_advance = read_u16_be(mtx_data, (long_metrics - 1) * 4).unwrap_or(0);
    let mut out = Vec::with_capacity(num_glyphs * 4);
    for gid in 0..num_glyphs {
        let (advance, lsb) = if gid < long_metrics {
            (
                read_u16_be(mtx_data, gid * 4).unwrap_or(0),
                read_u16_be(mtx_data, gid * 4 + 2).unwrap_or(0),
            )
        } else {
            let at = long_metrics * 4 + (gid - long_metrics) * 2;
            (last_advance, read_u16_be(mtx_data, at).unwrap_or(0))
        };
        out.extend_from_slice(&advance.to_be_bytes());
        out.extend_from_slice(&lsb.to_be_bytes());
    }
    (out, num_glyphs)
}

pub(crate) fn apply_metric_var(
    table_map:  &BTreeMap<String, TableBytes>,
    var_tag:    &str,
    mtx_data:   &mut [u8],
    num_glyphs: usize,
    long_metrics: usize,
    location:   &[f64],
) -> Result<(), String> {
    let var     = table_map.get(var_tag).ok_or_else(|| format!("missing {}", var_tag))?;
    let ivs_off = read_u32_be(var, 4).ok_or_else(|| format!("{}: header truncated", var_tag))? as usize;
    let map_off = read_u32_be(var, 8).ok_or_else(|| format!("{}: header truncated", var_tag))? as usize;

    let store = parse_item_variation_store(var, ivs_off)?;
    let map: Option<Vec<(u32, u32)>> = if map_off != 0 {
        Some(parse_delta_set_index_map(var, map_off)?)
    } else {
        None
    };

    if long_metrics == 0 {
        return Err(format!("{}: long-metrics count is zero", var_tag));
    }

    let region_scalars = precompute_region_scalars(&store, location);

    for gid in 0..long_metrics.min(num_glyphs) {
        let (outer, inner) = match &map {
            Some(m) => delta_set_index_map_lookup(m, gid),
            None => (0, gid),
        };
        let delta  = ot_round(compute_ivs_delta_f64(&store, outer, inner, &region_scalars));
        let aw_off = gid * 4;
        if aw_off + 2 > mtx_data.len() { continue; }
        let aw = read_u16_be(mtx_data, aw_off).unwrap_or(0) as i32;
        write_u16_be(mtx_data, aw_off, aw.saturating_add(delta).clamp(0, 65535) as u16);
    }
    Ok(())
}

// HVAR's lsb and VVAR's tsb mappings, read only where a font has no phantom points to give them
// (CFF2); a NULL mapping means the font varies no bearings.
pub(crate) fn apply_bearing_var(
    table_map: &BTreeMap<String, TableBytes>,
    var_tag: &str,
    mtx_data: &mut [u8],
    num_glyphs: usize,
    long_metrics: usize,
    location: &[f64],
) -> Result<(), String> {
    let Some(var) = table_map.get(var_tag) else { return Ok(()) };
    let map_off = read_u32_be(var, 12).ok_or_else(|| format!("{}: header truncated", var_tag))? as usize;
    if map_off == 0 {
        return Ok(());
    }
    let store = parse_item_variation_store(var, read_u32_be(var, 4).unwrap_or(0) as usize)?;
    let map = parse_delta_set_index_map(var, map_off)?;
    let scalars = precompute_region_scalars(&store, location);
    for gid in 0..num_glyphs {
        let (outer, inner) = delta_set_index_map_lookup(&map, gid);
        let delta = ot_round(compute_ivs_delta_f64(&store, outer, inner, &scalars));
        let at = if gid < long_metrics { gid * 4 + 2 } else { long_metrics * 4 + (gid - long_metrics) * 2 };
        if delta == 0 || at + 2 > mtx_data.len() { continue; }
        let bearing = i32::from(read_u16_be(mtx_data, at).unwrap_or(0) as i16);
        write_u16_be(mtx_data, at, bearing.saturating_add(delta).clamp(-32768, 32767) as i16 as u16);
    }
    Ok(())
}

// VORG at the location: each glyph's origin moved by VVAR's vOrg mapping, written out as a record
// wherever it now differs from the default.
pub(crate) fn vary_vorg(table_map: &BTreeMap<String, TableBytes>, num_glyphs: usize, location: &[f64]) -> Option<Vec<u8>> {
    let (vorg, vvar) = (table_map.get("VORG")?, table_map.get("VVAR")?);
    let map_off = read_u32_be(vvar, 20)? as usize;
    if map_off == 0 {
        return None;
    }
    let store = parse_item_variation_store(vvar, read_u32_be(vvar, 4)? as usize).ok()?;
    let map = parse_delta_set_index_map(vvar, map_off).ok()?;
    let scalars = precompute_region_scalars(&store, location);
    let default = read_u16_be(vorg, 4)? as i16;
    let count = usize::from(read_u16_be(vorg, 6)?).min(vorg.len().saturating_sub(8) / 4);
    let mut origins = alloc::vec![i32::from(default); num_glyphs];
    for i in 0..count {
        let gid = usize::from(read_u16_be(vorg, 8 + 4 * i)?);
        if let Some(o) = origins.get_mut(gid) {
            *o = i32::from(read_u16_be(vorg, 10 + 4 * i)? as i16);
        }
    }
    let mut records = Vec::new();
    for (gid, origin) in origins.iter().enumerate() {
        let (outer, inner) = delta_set_index_map_lookup(&map, gid);
        let y = origin.saturating_add(ot_round(compute_ivs_delta_f64(&store, outer, inner, &scalars))).clamp(-32768, 32767);
        if y != i32::from(default) {
            records.push((gid as u16, y as i16));
        }
    }
    let mut out = Vec::with_capacity(8 + 4 * records.len());
    out.extend_from_slice(&vorg[..4]);
    out.extend_from_slice(&default.to_be_bytes());
    out.extend_from_slice(&u16::try_from(records.len()).ok()?.to_be_bytes());
    for (gid, y) in records {
        out.extend_from_slice(&gid.to_be_bytes());
        out.extend_from_slice(&y.to_be_bytes());
    }
    Some(out)
}
