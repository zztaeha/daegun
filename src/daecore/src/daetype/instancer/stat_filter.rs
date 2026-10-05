use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_u32_be, write_u16_be, write_u32_be};

fn read_fixed(data: &[u8], off: usize) -> Option<f64> {
    read_u32_be(data, off).map(|v| v as i32 as f64 / 65536.0)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1.0 / 65536.0
}

fn axis_value_len(stat: &[u8], at: usize) -> Option<usize> {
    Some(match read_u16_be(stat, at)? {
        1 => 12,
        2 => 20,
        3 => 16,
        4 => 8 + read_u16_be(stat, at + 2)? as usize * 6,
        _ => return None,
    })
}

fn describes(
    stat: &[u8], at: usize, location: &BTreeMap<u16, f64>, combo_left: &mut usize,
) -> Option<bool> {
    let matches_axis = |axis_index: u16, value: f64| match location.get(&axis_index) {
        Some(&coord) => close(coord, value),
        None => true,
    };
    Some(match read_u16_be(stat, at)? {
        1 => matches_axis(read_u16_be(stat, at + 2)?, read_fixed(stat, at + 8)?),
        2 => {
            let axis_index = read_u16_be(stat, at + 2)?;
            let (min, max) = (read_fixed(stat, at + 12)?, read_fixed(stat, at + 16)?);
            match location.get(&axis_index) {
                Some(&coord) => coord >= min - 1.0 / 65536.0 && coord <= max + 1.0 / 65536.0,
                None => true,
            }
        }
        3 => matches_axis(read_u16_be(stat, at + 2)?, read_fixed(stat, at + 8)?),
        4 => {
            let count = read_u16_be(stat, at + 2)? as usize;
            *combo_left = combo_left.checked_sub(count)?;
            let mut all = count > 0;
            for j in 0..count {
                let rec = at + 8 + j * 6;
                if !matches_axis(read_u16_be(stat, rec)?, read_fixed(stat, rec + 2)?) {
                    all = false;
                    break;
                }
            }
            all
        }
        _ => false,
    })
}

// A static font names one value per axis, so where several tables matched the instance's
// coordinate, the one the spec picks stays: a format 1 or 3 over a format 2 whose range it touches,
// unless that range starts at its own nominal value; of format 2 ranges, the higher where they
// touch, unless the lower's nominal is the touching value and the higher's above it.
fn choose_per_axis(stat: &[u8], matched: &[usize], location: &BTreeMap<u16, f64>) -> Vec<usize> {
    let single_axis = |at: usize| {
        let format = read_u16_be(stat, at)?;
        let older_sibling = read_u16_be(stat, at + 4)? & 0x0001 != 0;
        let axis = read_u16_be(stat, at + 2)?;
        (matches!(format, 1..=3) && !older_sibling && location.contains_key(&axis)).then_some((format, axis))
    };
    let mut chosen: BTreeMap<u16, usize> = BTreeMap::new();
    let mut out: Vec<usize> = matched.iter().copied().filter(|&at| single_axis(at).is_none()).collect();
    for &at in matched {
        let Some((format, axis)) = single_axis(at) else { continue };
        let coord = location[&axis];
        let Some(&held) = chosen.get(&axis) else {
            chosen.insert(axis, at);
            continue;
        };
        let range = |at: usize| Some((read_fixed(stat, at + 8)?, read_fixed(stat, at + 12)?, read_fixed(stat, at + 16)?));
        let held_format = read_u16_be(stat, held).unwrap_or(0);
        let take = match (held_format == 2, format == 2) {
            (false, false) => false,
            (true, false) => !range(held).is_some_and(|(nominal, min, _)| close(min, coord) && close(nominal, min)),
            (false, true) => range(at).is_some_and(|(nominal, min, _)| close(min, coord) && close(nominal, min)),
            (true, true) => match (range(held), range(at)) {
                (Some(a), Some(b)) => prefer_range(a, b, coord),
                _ => false,
            },
        };
        if take {
            chosen.insert(axis, at);
        }
    }
    out.extend(chosen.into_values());
    out.sort_unstable();
    out
}

// Whether range `b` (nominal, min, max) is to be used over `a` at `coord`, both holding it.
fn prefer_range(a: (f64, f64, f64), b: (f64, f64, f64), coord: f64) -> bool {
    let ((_, a_min, a_max), (_, b_min, b_max)) = (a, b);
    if close(a_min, b_min) && close(a_max, b_max) {
        return false;
    }
    if a_min <= b_min && b_max <= a_max {
        return false;
    }
    if b_min <= a_min && a_max <= b_max {
        return true;
    }
    let (lower, higher, b_is_higher) = if b_min > a_min { (a, b, true) } else { (b, a, false) };
    let touching = close(lower.2, coord) && close(higher.1, coord);
    let lower_wins = touching && close(lower.0, coord) && higher.0 > coord;
    b_is_higher != lower_wins
}

pub(crate) fn filter_stat_to_instance(stat: &[u8], axis_coords: &BTreeMap<String, f64>) -> Option<Vec<u8>> {
    if stat.len() < 18 {
        return None;
    }
    let major = read_u16_be(stat, 0)?;
    let minor = read_u16_be(stat, 2)?;
    let design_axis_size = read_u16_be(stat, 4)? as usize;
    let design_axis_count = read_u16_be(stat, 6)? as usize;
    let design_axes_offset = read_u32_be(stat, 8)? as usize;
    let axis_value_count = read_u16_be(stat, 12)? as usize;
    let offsets_array = read_u32_be(stat, 14)? as usize;

    if design_axis_size < 8 {
        return None;
    }
    let design_axes = stat.get(design_axes_offset..)?.get(..design_axis_count.checked_mul(design_axis_size)?)?;

    let mut location: BTreeMap<u16, f64> = BTreeMap::new();
    for i in 0..design_axis_count {
        let rec = i * design_axis_size;
        let tag = String::from_utf8_lossy(design_axes.get(rec..rec + 4)?).to_string();
        if let Some(&coord) = axis_coords.get(&tag) {
            location.insert(u16::try_from(i).ok()?, coord);
        }
    }

    let mut matched: Vec<usize> = Vec::new();
    let mut combo_left = stat.len() / 6;
    for i in 0..axis_value_count {
        let rel = read_u16_be(stat, offsets_array + i * 2)? as usize;
        let at = offsets_array.checked_add(rel)?;
        let Some(len) = axis_value_len(stat, at) else { continue };
        if stat.get(at..).and_then(|s| s.get(..len)).is_none() { continue };
        if describes(stat, at, &location, &mut combo_left).unwrap_or(false) {
            matched.push(at);
        }
    }
    let kept: Vec<&[u8]> = choose_per_axis(stat, &matched, &location)
        .into_iter()
        .filter_map(|at| stat.get(at..at + axis_value_len(stat, at)?))
        .collect();

    let elided_fallback = if minor >= 1 && stat.len() >= 20 { read_u16_be(stat, 18)? } else { 2 };

    let header_len = 20usize;
    let axes_at = header_len;
    let offsets_at = axes_at + design_axes.len();
    let values_at = offsets_at + kept.len() * 2;

    let mut out = vec![0u8; values_at];
    write_u16_be(&mut out, 0, major);
    write_u16_be(&mut out, 2, minor.max(1));
    write_u16_be(&mut out, 4, u16::try_from(design_axis_size).ok()?);
    write_u16_be(&mut out, 6, u16::try_from(design_axis_count).ok()?);
    write_u32_be(&mut out, 8, u32::try_from(axes_at).ok()?);
    write_u16_be(&mut out, 12, u16::try_from(kept.len()).ok()?);
    write_u32_be(&mut out, 14, u32::try_from(offsets_at).ok()?);
    write_u16_be(&mut out, 18, elided_fallback);
    out[axes_at..offsets_at].copy_from_slice(design_axes);

    for (i, bytes) in kept.iter().enumerate() {
        let target = out.len() - offsets_at;
        write_u16_be(&mut out, offsets_at + i * 2, u16::try_from(target).ok()?);
        out.extend_from_slice(bytes);
    }

    Some(out)
}
