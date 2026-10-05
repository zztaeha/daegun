use alloc::collections::BTreeMap;
use alloc::string::String;
use super::decoder::{read_i16_be, read_u16_be, read_u32_be};
use crate::daecore::daetype::TableBytes;

// 0.0 is the registry's "normal", and the only track asked for: a shaper with no interface for
// choosing one has nothing else it could honestly select.
const NORMAL_TRACK: f64 = 0.0;

pub const DEFAULT_POINT_SIZE: f64 = 12.0;

fn fixed(data: &[u8], at: usize) -> Option<f64> {
    read_u32_be(data, at).map(|v| f64::from(v as i32) / 65536.0)
}

pub fn tracking(
    table_map: &BTreeMap<String, TableBytes>,
    ptem: f64,
    horizontal: bool,
) -> f64 {
    let Some(trak) = table_map.get("trak") else { return 0.0 };
    let at = if horizontal { 6 } else { 8 };
    let Some(data) = read_u16_be(trak, at).map(usize::from).filter(|&o| o != 0) else {
        return 0.0;
    };
    track_data(trak, data, ptem)
}

fn track_data(trak: &[u8], data: usize, ptem: f64) -> f64 {
    let n_tracks = read_u16_be(trak, data).map_or(0, usize::from);
    let n_sizes = read_u16_be(trak, data + 2).map_or(0, usize::from);
    let sizes = read_u32_be(trak, data + 4).unwrap_or(0) as usize;
    if n_tracks == 0 || n_sizes == 0 {
        return 0.0;
    }

    let track_of = |k: usize| fixed(trak, data + 8 + k * 8).unwrap_or(0.0);
    let value_of = |k: usize| {
        let values = read_u16_be(trak, data + 8 + k * 8 + 6).map_or(0, usize::from);
        per_size(trak, sizes, values, n_sizes, ptem)
    };

    if n_tracks == 1 {
        return value_of(0);
    }

    let mut lo = 0usize;
    let mut hi = n_tracks - 1;
    while lo + 1 < n_tracks && track_of(lo + 1) <= NORMAL_TRACK {
        lo += 1;
    }
    while hi > 0 && track_of(hi - 1) >= NORMAL_TRACK {
        hi -= 1;
    }
    if lo == hi {
        return value_of(lo);
    }

    let (t0, t1) = (track_of(lo), track_of(hi));
    let span = t1 - t0;
    let interp = if span == 0.0 { 0.0 } else { (NORMAL_TRACK - t0) / span };
    let (a, b) = (value_of(lo), value_of(hi));
    a + interp * (b - a)
}

fn per_size(trak: &[u8], sizes: usize, values: usize, n_sizes: usize, ptem: f64) -> f64 {
    let size_at = |i: usize| fixed(trak, sizes + i * 4).unwrap_or(0.0);
    let value_at = |i: usize| read_i16_be(trak, values + i * 2).map_or(0.0, f64::from);

    for i in 0..n_sizes {
        let s1 = size_at(i);
        if s1 < ptem {
            continue;
        }
        if i == 0 {
            return value_at(0);
        }
        let s0 = size_at(i - 1);
        let (v0, v1) = (value_at(i - 1), value_at(i));
        return v0 + (ptem - s0) / (s1 - s0) * (v1 - v0);
    }
    value_at(n_sizes - 1)
}
