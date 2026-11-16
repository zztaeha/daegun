use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, write_u16_be, write_u32_be};
use super::super::stat::{AxisValue, Stat, ValueKind, OLDER_SIBLING_FONT_ATTRIBUTE};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1.0 / 65536.0
}

// Whether a value holds at the instance's location on every axis it names that the location sets.
fn describes(v: &AxisValue, location: &BTreeMap<u16, f64>) -> bool {
    let near = |axis: &u16, value: &f64| location.get(axis).is_none_or(|&coord| close(coord, *value));
    match &v.kind {
        ValueKind::Single { axis, value } | ValueKind::Linked { axis, value, .. } => near(axis, value),
        ValueKind::Range { axis, min, max, .. } => {
            location.get(axis).is_none_or(|&coord| coord >= min - 1.0 / 65536.0 && coord <= max + 1.0 / 65536.0)
        }
        ValueKind::Combo(records) => !records.is_empty() && records.iter().all(|(axis, value)| near(axis, value)),
    }
}

// A static font names one value per axis, so of matching tables the spec's pick stays: a format 1 or 3 over a touching
// format 2 unless that starts at its nominal; of touching 2s the higher, unless the lower's nominal touches.
fn choose_per_axis<'v>(matched: &'v [AxisValue], location: &BTreeMap<u16, f64>) -> Vec<&'v AxisValue> {
    let single_axis = |v: &AxisValue| match v.kind {
        ValueKind::Single { axis, .. } | ValueKind::Range { axis, .. } | ValueKind::Linked { axis, .. } => {
            (v.flags & OLDER_SIBLING_FONT_ATTRIBUTE == 0 && location.contains_key(&axis)).then_some(axis)
        }
        ValueKind::Combo(_) => None,
    };
    let range = |v: &AxisValue| match v.kind {
        ValueKind::Range { nominal, min, max, .. } => Some((nominal, min, max)),
        _ => None,
    };
    let mut chosen: BTreeMap<u16, &AxisValue> = BTreeMap::new();
    let mut out: Vec<&AxisValue> = matched.iter().filter(|v| single_axis(v).is_none()).collect();
    for v in matched {
        let Some(axis) = single_axis(v) else { continue };
        let coord = location[&axis];
        let Some(&held) = chosen.get(&axis) else {
            chosen.insert(axis, v);
            continue;
        };
        let take = match (range(held), range(v)) {
            (None, None) => false,
            (Some((nominal, min, _)), None) => !(close(min, coord) && close(nominal, min)),
            (None, Some((nominal, min, _))) => close(min, coord) && close(nominal, min),
            (Some(a), Some(b)) => prefer_range(a, b, coord),
        };
        if take {
            chosen.insert(axis, v);
        }
    }
    out.extend(chosen.into_values());
    out.sort_unstable_by_key(|v| v.at);
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

pub(crate) fn filter_stat_to_instance(
    data: &[u8],
    axis_coords: &BTreeMap<crate::daecore::Tag, f64>,
) -> Option<Vec<u8>> {
    let stat = Stat::read(data).ok()?;
    let location: BTreeMap<u16, f64> = stat.axes()
        .zip(0u16..)
        .filter_map(|((tag, _, _), i)| Some((i, *axis_coords.get(&crate::daecore::Tag::new(&tag))?)))
        .collect();
    let matched: Vec<AxisValue> = stat.values().filter(|v| describes(v, &location)).collect();
    let kept: Vec<&[u8]> =
        choose_per_axis(&matched, &location).into_iter().filter_map(|v| data.get(v.at..v.at + v.len)).collect();

    let design_axes = stat.design_axes();
    let header_len = 20usize;
    let axes_at = header_len;
    let offsets_at = axes_at + design_axes.len();
    let values_at = offsets_at + kept.len() * 2;

    let mut out = vec![0u8; values_at];
    // The version and design axis fields stay, the minor version at least 1 for the elided fallback name.
    out[..8].copy_from_slice(data.get(..8)?);
    write_u16_be(&mut out, 2, read_u16_be(data, 2)?.max(1));
    write_u32_be(&mut out, 8, u32::try_from(axes_at).ok()?);
    write_u16_be(&mut out, 12, u16::try_from(kept.len()).ok()?);
    write_u32_be(&mut out, 14, u32::try_from(offsets_at).ok()?);
    write_u16_be(&mut out, 18, stat.elided_fallback().unwrap_or(2));
    out[axes_at..offsets_at].copy_from_slice(design_axes);

    for (i, bytes) in kept.iter().enumerate() {
        let target = out.len() - offsets_at;
        write_u16_be(&mut out, offsets_at + i * 2, u16::try_from(target).ok()?);
        out.extend_from_slice(bytes);
    }

    Some(out)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/stat_filter.rs"]
mod tests;
