#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_u32_be, read_i16_be};
use super::super::format::round::ot_round;
use crate::daecore::daetype::TableBytes;

// The spec's normalization: 16.16 throughout, as FreeType does it, where HarfBuzz and fontTools use
// floats and can land one 2.14 step away.
pub(crate) fn to_fixed(v: f64) -> i32 {
    (v * 65536.0).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

pub(crate) fn normalize_axis(value: i32, min: i32, def: i32, max: i32) -> i32 {
    let value = value.clamp(min.min(max), max.max(min));
    if value < def && def > min {
        div_fix(value - def, def - min).max(-0x10000)
    } else if value > def && max > def {
        div_fix(value - def, max - def).min(0x10000)
    } else {
        0
    }
}

// FT_DivFix and FT_MulDiv: the quotient rounded to nearest, ties away from zero.
fn div_fix(a: i32, b: i32) -> i32 {
    mul_div(a, 0x10000, b)
}

fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return 0;
    }
    let (n, d) = (i64::from(a) * i64::from(b), i64::from(c));
    let q = (n.abs() + d.abs() / 2) / d.abs();
    (if (n < 0) != (d < 0) { -q } else { q }).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

// 16.16 to 2.14: add 2 and shift right by 2, sign extending.
pub(crate) fn to_f2dot14(v: i32) -> i32 {
    v.saturating_add(2) >> 2
}

// avar's segment maps on 16.16 coordinates, then, for version 2, its deltas on the 2.14 result.
// Coordinates come back in 2.14.
pub fn apply_avar_all(table_map: &BTreeMap<String, TableBytes>, location: &mut [i32]) {
    let Some(avar) = table_map.get("avar") else {
        location.iter_mut().for_each(|v| *v = to_f2dot14(*v));
        return;
    };
    let major = read_u16_be(avar, 0);
    let axis_count = read_u16_be(avar, 6).map_or(0, usize::from);
    let mut pos = 8usize;
    if matches!(major, Some(1 | 2)) {
        for i in 0..axis_count {
            let Some(count) = read_u16_be(avar, pos).map(usize::from) else { break };
            pos += 2;
            if let Some(v) = location.get_mut(i) {
                *v = segment_map(avar, pos, count, *v);
            }
            pos += count * 4;
        }
    }
    location.iter_mut().for_each(|v| *v = to_f2dot14(*v));
    if major == Some(2) {
        apply_avar2(avar, pos, location);
    }
}

fn apply_avar2(avar: &[u8], pos: usize, location: &mut [i32]) {
    use super::super::format::ivs::{compute_ivs_delta_f64, delta_set_index_map_lookup, parse_delta_set_index_map,
        parse_item_variation_store, precompute_region_scalars};
    let (Some(map_off), Some(store_off)) = (read_u32_be(avar, pos), read_u32_be(avar, pos + 4)) else { return };
    if store_off == 0 { return; }
    let Ok(store) = parse_item_variation_store(avar, store_off as usize) else { return };
    let map = match map_off {
        0 => None,
        off => match parse_delta_set_index_map(avar, off as usize) {
            Ok(m) => Some(m),
            Err(_) => return,
        },
    };
    let snapshot: Vec<f64> = location.iter().map(|&v| f64::from(v) / 16384.0).collect();
    let scalars = precompute_region_scalars(&store, &snapshot);
    for (i, v) in location.iter_mut().enumerate() {
        let (outer, inner) = map.as_ref().map_or((0, i), |m| delta_set_index_map_lookup(m, i));
        let delta = ot_round(compute_ivs_delta_f64(&store, outer, inner, &scalars));
        *v = v.saturating_add(delta).clamp(-16384, 16384);
    }
}

fn segment_map(avar: &[u8], pos: usize, count: usize, value: i32) -> i32 {
    let pair = |j: usize| -> Option<(i32, i32)> {
        Some((i32::from(read_i16_be(avar, pos + j * 4)?) << 2, i32::from(read_i16_be(avar, pos + j * 4 + 2)?) << 2))
    };
    for j in 1..count {
        let (Some((from0, to0)), Some((from1, to1))) = (pair(j - 1), pair(j)) else { return value };
        if value < from1 {
            return mul_div(value - from0, to1 - to0, from1 - from0) + to0;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    // FreeType's answers; the float path gives 229 at wght 407 and 9830 at 700.
    #[test]
    fn normalization_follows_the_spec_in_fixed_point() {
        let axis = |v: f64| to_f2dot14(normalize_axis(to_fixed(v), to_fixed(100.0), to_fixed(400.0), to_fixed(900.0)));
        assert_eq!([axis(407.0), axis(700.0), axis(100.0), axis(900.0), axis(400.0), axis(250.0)], [230, 9831, -16384, 16384, 0, -8192]);
        // Halfway between two 2.14 steps rounds up on either side of zero.
        assert_eq!([-6, -2, 2, 6].map(to_f2dot14), [-1, 0, 1, 2]);
    }
}
