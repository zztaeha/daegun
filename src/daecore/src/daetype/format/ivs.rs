use alloc::string::String;
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be, read_i16_be, records_fit};

pub struct ItemVariationStore {
    pub regions:    Vec<Vec<RegionAxis>>,
    pub ivd_data:   Vec<Ivd>,
    pub axis_count: usize,
}

pub struct RegionAxis { pub start: f64, pub peak: f64, pub end: f64 }

#[derive(Default)]
pub struct Ivd {
    pub region_indices: Vec<usize>,
    deltas: Vec<i32>,
}

impl Ivd {
    pub fn row(&self, inner: usize) -> Option<&[i32]> {
        let width = self.region_indices.len();
        if width == 0 { return None; }
        let start = inner.checked_mul(width)?;
        self.deltas.get(start..start.checked_add(width)?)
    }

    pub fn rows(&self) -> usize {
        self.deltas.len().checked_div(self.region_indices.len()).unwrap_or(0)
    }
}

pub fn parse_item_variation_store(buf: &[u8], base: usize) -> Result<ItemVariationStore, String> {
    // The base can come from C. Inside the buffer, the few bytes added to it below cannot overflow.
    if base >= buf.len() {
        return Err("IVS: base past the end".into());
    }
    if read_u16_be(buf, base) != Some(1) {
        return Err("IVS: unknown format".into());
    }
    let region_list_off = read_u32_be(buf, base + 2).ok_or("IVS: header truncated")? as usize;
    let ivd_count       = read_u16_be(buf, base + 6).ok_or("IVS: header truncated")? as usize;

    let region_list_base = base.checked_add(region_list_off).ok_or("IVS: region list offset overflows")?;
    let axis_count       = read_u16_be(buf, region_list_base).ok_or("IVS: region list truncated")? as usize;
    let region_count     = read_u16_be(buf, region_list_base + 2).ok_or("IVS: region list truncated")? as usize;

    let stride = axis_count.checked_mul(6).ok_or("IVS: region stride overflows")?;
    let regions_at = region_list_base.checked_add(4).ok_or("IVS: region list base overflows")?;
    if !records_fit(regions_at, region_count, stride, buf.len()) {
        return Err("IVS: region list does not fit the table".into());
    }

    let mut regions: Vec<Vec<RegionAxis>> = Vec::with_capacity(region_count);
    for i in 0..region_count {
        let r_off = regions_at + i * stride;
        let mut axes = Vec::with_capacity(axis_count);
        for j in 0..axis_count {
            let start = read_i16_be(buf, r_off + j * 6).ok_or("IVS: region axis truncated")?;
            let peak  = read_i16_be(buf, r_off + j * 6 + 2).ok_or("IVS: region axis truncated")?;
            let end   = read_i16_be(buf, r_off + j * 6 + 4).ok_or("IVS: region axis truncated")?;
            axes.push(RegionAxis {
                start: start as f64 / 16384.0,
                peak:  peak  as f64 / 16384.0,
                end:   end   as f64 / 16384.0,
            });
        }
        regions.push(axes);
    }

    if !records_fit(base + 8, ivd_count, 4, buf.len()) {
        return Err("IVS: IVD offset table does not fit".into());
    }
    let mut ivd_offsets: Vec<usize> = Vec::with_capacity(ivd_count);
    for i in 0..ivd_count {
        let off = read_u32_be(buf, base + 8 + i * 4).ok_or("IVS: IVD offset table truncated")?;
        ivd_offsets.push(off as usize);
    }

    // Offsets may repeat, so each copy is held: four values per byte from the store's start, which is
    // all its offsets reach, with rows budgeted apart, as an empty row costs none.
    const MAX_DELTA_ROWS: usize = 1_000_000;
    const VALUES_PER_BYTE: usize = 4;
    const MIN_VALUES: usize = 1 << 16;
    let mut rows_left = MAX_DELTA_ROWS;
    let mut values_left = buf.len().saturating_sub(base).saturating_mul(VALUES_PER_BYTE).max(MIN_VALUES);

    let mut ivd_data: Vec<Ivd> = Vec::with_capacity(ivd_count.min(256));
    for off in ivd_offsets {
        // A NULL offset is data with no variation, whatever inner index names it.
        if off == 0 {
            ivd_data.push(Ivd::default());
            continue;
        }
        let ivd      = base.checked_add(off).ok_or("IVS: IVD offset overflows")?;
        let items    = read_u16_be(buf, ivd).ok_or("IVS: IVD header truncated")? as usize;
        let word_cnt = read_u16_be(buf, ivd + 2).ok_or("IVS: IVD header truncated")? as usize;
        let reg_cnt  = read_u16_be(buf, ivd + 4).ok_or("IVS: IVD header truncated")? as usize;
        rows_left = rows_left
            .checked_sub(items)
            .ok_or("IVS: delta row budget exhausted")?;
        values_left = items
            .checked_mul(reg_cnt)
            .and_then(|deltas| deltas.checked_add(reg_cnt))
            .and_then(|values| values_left.checked_sub(values))
            .ok_or("IVS: value budget exhausted")?;
        if !records_fit(ivd + 6, reg_cnt, 2, buf.len()) {
            return Err("IVS: IVD region index array does not fit".into());
        }
        let mut reg_idxs = Vec::with_capacity(reg_cnt);
        for j in 0..reg_cnt {
            let idx = read_u16_be(buf, ivd + 6 + j * 2).ok_or("IVS: IVD region index truncated")?;
            reg_idxs.push(idx as usize);
        }
        let long_words  = (word_cnt & 0x8000) != 0;
        let wc          = word_cnt & 0x7FFF;
        if wc > reg_cnt {
            return Err("IVS: IVD wordDeltaCount exceeds regionIndexCount".into());
        }
        let wide_size   = if long_words { 4 } else { 2 };
        let narrow_size = if long_words { 2 } else { 1 };
        let row_bytes   = wc * wide_size + (reg_cnt - wc) * narrow_size;
        let data_start  = ivd + 6 + reg_cnt * 2;

        let mut deltas: Vec<i32> = Vec::with_capacity(
            items.min(256).saturating_mul(reg_cnt).min(buf.len()),
        );
        for r in 0..items {
            let row_off = r.checked_mul(row_bytes).and_then(|d| data_start.checked_add(d))
                .ok_or("IVS: delta row offset overflows")?;
            let row = buf.get(row_off..row_off.checked_add(row_bytes).ok_or("IVS: delta row truncated")?)
                .ok_or("IVS: delta row truncated")?;
            let (wide, narrow) = row.split_at_checked(wc * wide_size).ok_or("IVS: delta row truncated")?;
            if long_words {
                deltas.extend(wide.as_chunks::<4>().0.iter().map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])));
                deltas.extend(narrow.as_chunks::<2>().0.iter().map(|c| i16::from_be_bytes([c[0], c[1]]) as i32));
            } else {
                deltas.extend(wide.as_chunks::<2>().0.iter().map(|c| i16::from_be_bytes([c[0], c[1]]) as i32));
                deltas.extend(narrow.iter().map(|&b| b as i8 as i32));
            }
        }
        ivd_data.push(Ivd { region_indices: reg_idxs, deltas });
    }

    Ok(ItemVariationStore { regions, ivd_data, axis_count })
}

pub fn parse_delta_set_index_map(buf: &[u8], base: usize) -> Result<Vec<(u32, u32)>, String> {
    let fmt       = *buf.get(base).ok_or("IVS: delta set index map truncated")?;
    if fmt > 1 {
        return Err("IVS: unknown delta set index map format".into());
    }
    let entry_fmt = *buf.get(base + 1).ok_or("IVS: delta set index map truncated")?;
    let map_count: usize = if fmt == 0 {
        read_u16_be(buf, base + 2).ok_or("IVS: delta set index map truncated")? as usize
    } else {
        read_u32_be(buf, base + 2).ok_or("IVS: delta set index map truncated")? as usize
    };
    let inner_bits = (entry_fmt & 0x0F) as usize + 1;
    let entry_size = ((entry_fmt >> 4) & 0x3) as usize + 1;
    let inner_mask = (1usize << inner_bits) - 1;
    let data_off   = if fmt == 0 { 4 } else { 6 };

    let need = map_count.checked_mul(entry_size)
        .and_then(|n| n.checked_add(base + data_off))
        .ok_or("IVS: delta set index map too large")?;
    if need > buf.len() { return Err("IVS: delta set index map truncated".into()); }

    let entries = buf.get(base + data_off..need).ok_or("IVS: delta set index map truncated")?;
    let map: Vec<(u32, u32)> = entries
        .chunks_exact(entry_size)
        .map(|e| {
            let val = e.iter().fold(0usize, |acc, &b| (acc << 8) | b as usize);
            ((val >> inner_bits) as u32, (val & inner_mask) as u32)
        })
        .collect();

    Ok(map)
}

// Past the end, the last entry answers. An empty map is no map, as HarfBuzz and FreeType read it.
pub fn delta_set_index_map_lookup(map: &[(u32, u32)], idx: usize) -> (usize, usize) {
    match map.get(idx).or(map.last()) {
        Some(&(o, i)) => (o as usize, i as usize),
        None => (0, idx),
    }
}

pub fn precompute_region_scalars(store: &ItemVariationStore, location: &[f64]) -> Vec<f64> {
    (0..store.regions.len()).map(|reg_idx| region_scalar(store, reg_idx, location)).collect()
}

pub fn compute_ivs_delta_f64(store: &ItemVariationStore, outer_idx: usize, inner_idx: usize, region_scalars: &[f64]) -> f64 {
    let ivd       = match store.ivd_data.get(outer_idx) { Some(v) => v, None => return 0.0 };
    let delta_set = match ivd.row(inner_idx) { Some(v) => v, None => return 0.0 };
    let mut delta = 0.0f64;
    for (i, &reg_idx) in ivd.region_indices.iter().enumerate() {
        let scalar = region_scalars.get(reg_idx).copied().unwrap_or(0.0);
        delta += scalar * delta_set.get(i).copied().unwrap_or(0) as f64;
    }
    delta
}

// What a Device or VariationIndex table adds at a location, from the ItemVariationStore of the table
// that holds it; each (outer, inner) is worked out once, as many tables may name one.
pub(crate) struct DeviceDeltas {
    store: crate::daecore::sync::Shared<ItemVariationStore>,
    scalars: Vec<f64>,
    worked_out: alloc::collections::BTreeMap<(u16, u16), f64>,
}

impl DeviceDeltas {
    pub(crate) fn new(table: &[u8], store_at: usize, location: &[f64]) -> Option<DeviceDeltas> {
        if store_at == 0 || location.iter().all(|&c| c == 0.0) {
            return None;
        }
        DeviceDeltas::from_store(crate::daecore::sync::Shared::new(parse_item_variation_store(table, store_at).ok()?), location)
    }

    // Over a store parsed already, such as the one a font keeps for the shaper.
    pub(crate) fn from_store(store: crate::daecore::sync::Shared<ItemVariationStore>, location: &[f64]) -> Option<DeviceDeltas> {
        if location.iter().all(|&c| c == 0.0) {
            return None;
        }
        let scalars = precompute_region_scalars(&store, location);
        Some(DeviceDeltas { store, scalars, worked_out: alloc::collections::BTreeMap::new() })
    }

    // A VariationIndex's delta; a Device table of sizes adds nothing, applying only at a ppem.
    pub(crate) fn delta(&mut self, table: &[u8], device_at: usize) -> f64 {
        if read_u16_be(table, device_at + 4) != Some(0x8000) {
            return 0.0;
        }
        let (Some(outer), Some(inner)) = (read_u16_be(table, device_at), read_u16_be(table, device_at + 2)) else {
            return 0.0;
        };
        let (store, scalars) = (&self.store, &self.scalars);
        *self
            .worked_out
            .entry((outer, inner))
            .or_insert_with(|| compute_ivs_delta_f64(store, usize::from(outer), usize::from(inner), scalars))
    }
}

fn region_scalar(store: &ItemVariationStore, reg_idx: usize, location: &[f64]) -> f64 {
    let region = match store.regions.get(reg_idx) { Some(r) => r, None => return 0.0 };
    // Most regions are zero at any one location, and the first axis that says so settles it: every
    // factor is finite and not negative, so the product could only stay zero.
    let mut scalar = 1.0;
    for (j, a) in region.iter().take(store.axis_count).enumerate() {
        scalar *= axis_factor(location.get(j).copied().unwrap_or(0.0), a.start, a.peak, a.end);
        if scalar == 0.0 {
            break;
        }
    }
    scalar
}

// One axis's part of a region's scalar, for the IVS and for gvar and cvar tuples alike. The spec, fontTools
// and FreeType ignore an axis whose region is backwards or crosses zero; HarfBuzz answers 0 at a zero coordinate.
pub(crate) fn axis_factor(loc: f64, start: f64, peak: f64, end: f64) -> f64 {
    if peak == 0.0 || loc == peak || start > peak || peak > end || (start < 0.0 && end > 0.0) {
        1.0
    } else if loc < start || loc > end {
        0.0
    } else if loc < peak {
        (loc - start) / (peak - start)
    } else {
        (end - loc) / (end - peak)
    }
}

pub(crate) fn region_scalars(store: &ItemVariationStore, outer_idx: usize, location: &[f64]) -> Option<Vec<f64>> {
    let ivd = store.ivd_data.get(outer_idx)?;
    Some(ivd.region_indices.iter().map(|&reg_idx| region_scalar(store, reg_idx, location)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // A one-axis store whose first data offset is NULL and whose second holds one delta of 100, with
    // other table bytes after it that a NULL read as an offset would land among.
    fn store_with_null_data() -> Vec<u8> {
        let mut t = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 0, 0, 0, 0, 0, 0, 26];
        t.extend([0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0]);
        t.extend([0, 1, 0, 0, 0, 1, 0, 0, 100]);
        t.extend([0x11; 40]);
        t
    }

    #[test]
    fn a_null_data_offset_has_no_variation() {
        let store = parse_item_variation_store(&store_with_null_data(), 0).expect("parses");
        let scalars = precompute_region_scalars(&store, &[1.0]);
        assert_eq!(compute_ivs_delta_f64(&store, 0, 0, &scalars), 0.0);
        assert_eq!(compute_ivs_delta_f64(&store, 1, 0, &scalars), 100.0);
    }

    #[test]
    fn a_store_or_map_of_an_unknown_format_is_refused() {
        let mut t = store_with_null_data();
        t[1] = 2;
        assert!(parse_item_variation_store(&t, 0).is_err());
        assert!(parse_delta_set_index_map(&[7, 0, 0, 0, 0, 1, 0x40], 0).is_err());
    }

    #[test]
    fn an_empty_delta_set_index_map_maps_nothing() {
        let map = parse_delta_set_index_map(&[0, 0, 0, 0], 0).expect("parses");
        assert!(map.is_empty());
        assert_eq!(delta_set_index_map_lookup(&map, 5), (0, 5));
        let one = parse_delta_set_index_map(&[0, 0x17, 0, 1, 0x01, 0x02], 0).expect("parses");
        assert_eq!([0, 9].map(|i| delta_set_index_map_lookup(&one, i)), [(1, 2), (1, 2)]);
    }
}
