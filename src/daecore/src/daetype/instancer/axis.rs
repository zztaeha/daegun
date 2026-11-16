use alloc::vec::Vec;
use super::super::decoder::{read_i16_be, read_u16_be, read_u32_be, records_fit};
use super::super::format::round::ot_round;
use super::super::format::ivs::{
    delta_set_index_map_lookup, parse_delta_set_index_map, round_fixed, store_delta, store_scalars,
    DeltaSetIndex, ItemVariationStore,
};
use crate::daecore::daetype::Tables;

// An axis tag as a location names it: the one `Tag`, compared as a number in every map keyed on locations.
pub type AxisTag = crate::daecore::Tag;

const ONE: i64 = 65_536;

// One axis's default normalization in 16.16, as the variations overview requires: converted, clamped to the axis,
// divided and clamped to [-1, 1]. Generic, so f32 and f64 callers alike convert what they hold.
pub(crate) fn normalize_axis<V: Into<f64>>(value: V, min: V, def: V, max: V) -> i32 {
    let value = value.into();
    if value.is_nan() {
        return 0;
    }
    let (min, def, max) = (to_fixed(min.into()), to_fixed(def.into()), to_fixed(max.into()));
    // An fvar whose default lies outside its range stretches the range to it, so the default still maps to 0.
    let (min, max) = (min.min(def), max.max(def));
    let value = to_fixed(value).clamp(min, max);
    let normalized = match value.cmp(&def) {
        core::cmp::Ordering::Less => -div(def - value, def - min),
        core::cmp::Ordering::Greater => div(value - def, max - def),
        core::cmp::Ordering::Equal => 0,
    };
    normalized.clamp(-ONE, ONE) as i32
}

// The spec's conversion to 16.16: the fraction times 65,536, a half rounding up.
fn to_fixed(v: f64) -> i64 {
    i64::from(ot_round(v * 65_536.0))
}

// a / b in 16.16 for a ≥ 0 and b > 0, rounded to the nearest, a half up.
fn div(a: i64, b: i64) -> i64 {
    (a * ONE * 2 + b) / (b * 2)
}

// a × b in 16.16, rounded to the nearest, a half up.
fn mul(a: i64, b: i64) -> i64 {
    (a * b + ONE / 2) >> 16
}

// A subset pins every axis where shaping and drawing read the location, so an instance matches the font there.
pub(crate) fn pinned_location(table_map: &Tables, axis_values: &[(AxisTag, f64)]) -> Vec<f64> {
    super::compute_location(table_map, axis_values).unwrap_or_default()
}

// 16.16 to 2.14: add 2 and shift right by 2, sign extending, as the overview's step 5 says.
pub(crate) fn to_f2dot14(v: i32) -> i32 {
    v.saturating_add(2) >> 2
}

// avar's segment maps on 16.16 coordinates, then the result in 2.14 and, for version 2, its deltas added there.
pub(crate) fn apply_avar(avar: Option<&Avar>, location: &mut [i32]) {
    if let Some(avar) = avar {
        for (v, map) in location.iter_mut().zip(&avar.maps) {
            *v = map_fixed(map, *v);
        }
    }
    location.iter_mut().for_each(|v| *v = to_f2dot14(*v));
    if let Some(Avar { v2: Some(v2), .. }) = avar {
        v2.add_deltas(location);
    }
}

// Each axis's (from, to) pairs in 16.16, those the spec says may be ignored left out, and version 2's index map and
// store.
pub(crate) struct Avar {
    maps: Vec<Vec<(i64, i64)>>,
    v2: Option<Avar2>,
}

struct Avar2 {
    map: Vec<DeltaSetIndex>,
    store: Option<ItemVariationStore>,
}

impl Avar {
    // A major version other than 1 or 2 is skipped whole. A segment map that runs past the table ends the maps
    // there; version 2's index map or store that doesn't read is absent.
    pub(crate) fn read(avar: &[u8]) -> Option<Self> {
        let major = read_u16_be(avar, 0)?;
        if !matches!(major, 1 | 2) {
            return None;
        }
        let mut at = 8;
        let mut maps = Vec::new();
        let mut whole = true;
        for _ in 0..read_u16_be(avar, 6)? {
            let Some(count) = read_u16_be(avar, at).map(usize::from).filter(|&n| records_fit(at + 2, n, 4, avar.len()))
            else {
                whole = false;
                break;
            };
            let pair = |j: usize| Some((read_i16_be(avar, at + 2 + 4 * j)?, read_i16_be(avar, at + 4 + 4 * j)?));
            maps.push(segment_map((0..count).filter_map(pair)));
            at += 2 + 4 * count;
        }
        let v2 = (major == 2 && whole).then(|| {
            let offset = |i: usize| read_u32_be(avar, at + i).map(|o| o as usize).filter(|&o| o != 0);
            let map = offset(0).and_then(|o| parse_delta_set_index_map(avar, o).ok()).unwrap_or_default();
            let store = offset(4).and_then(|o| ItemVariationStore::parse(avar, o).ok());
            Avar2 { map, store }
        });
        Some(Avar { maps, v2 })
    }
}

// A map's pairs in 16.16, each that doesn't rise past the last kept left out (avar, "Table formats"). A map without
// -1 to -1, 0 to 0 and 1 to 1 modifies nothing, so it's left empty.
fn segment_map(pairs: impl Iterator<Item = (i16, i16)>) -> Vec<(i64, i64)> {
    let mut kept: Vec<(i64, i64)> = Vec::new();
    for (from, to) in pairs.map(|(f, t)| (i64::from(f) * 4, i64::from(t) * 4)) {
        if kept.last().is_none_or(|&(f, t)| from > f && to >= t) {
            kept.push((from, to));
        }
    }
    let required = [(-ONE, -ONE), (0, 0), (ONE, ONE)];
    if required.iter().all(|pair| kept.contains(pair)) { kept } else { Vec::new() }
}

// One axis's map on a 16.16 coordinate, as the variations overview's steps run: the first pair from at or past it
// gives its value, or the segment up to it interpolates in 16.16.
fn map_fixed(map: &[(i64, i64)], value: i32) -> i32 {
    let v = i64::from(value);
    let Some(end) = map.iter().position(|&(from, _)| from >= v) else { return value };
    let (from, to) = map[end];
    if from == v || end == 0 {
        return to as i32;
    }
    let (start_from, start_to) = map[end - 1];
    let ratio = div(v - start_from, from - start_from);
    (start_to + mul(ratio, to - start_to)).clamp(-ONE, ONE) as i32
}

impl Avar2 {
    // The avar 2 draft: each axis's delta read from the store at the mapped 2.14 coordinates, all before any is added,
    // rounded and added in 2.14 and clamped to [-1, 1]. Without an index map an axis's index is its own.
    fn add_deltas(&self, location: &mut [i32]) {
        let Some(store) = &self.store else { return };
        let coords: Vec<f64> = location.iter().map(|&v| f64::from(v) / 16384.0).collect();
        let scalars = store_scalars(store, &coords);
        for (i, v) in location.iter_mut().enumerate() {
            let (outer, inner) = delta_set_index_map_lookup(&self.map, i).into();
            let delta = store_delta(store, outer, inner, &scalars);
            *v = v.saturating_add(round_away(delta)).clamp(-16_384, 16_384);
        }
    }
}

// A 16.16 delta to the nearest whole 2.14 step, a half away from zero.
fn round_away(v: i64) -> i32 {
    if v < 0 { -round_fixed(-v) } else { round_fixed(v) }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/axis.rs"]
mod tests;
