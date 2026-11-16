use alloc::vec::Vec;
use alloc::collections::{BTreeMap, BTreeSet};
use super::decoder::{fitting_count, offset16_in, read_u16_be, read_i16_be, read_u32_be};
use super::format::ivs::{units, DeviceDeltas};
use super::subsetter::subset_budget;
use crate::daecore::daetype::Tables;
use crate::daecore::Tag;

// Every reader here takes a NULL offset as no table and a count as the records that fit after it.

/// A script's baselines on one axis, from BASE.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
#[doc(alias("BaseScriptInfo"))]
pub struct BaseScript {
    /// The baseline the script's glyphs sit on; None where BASE names none.
    pub default_baseline: Option<Tag>,
    /// Each baseline's position in font units: y for horizontal text, x for vertical.
    pub baselines: BTreeMap<Tag, f64>,
}

// A script's BaseScript on one axis, and the axis table: DFLT's when the script has none, or no tag names one.
fn base_script(base: &[u8], vertical: bool, script_tag: Option<[u8; 4]>) -> Option<(usize, usize)> {
    let axis = offset16_in(base, 0, if vertical { 6 } else { 4 })?;
    let list = offset16_in(base, axis, 2)?;
    let count = fitting_count(base, list, 6);
    let script = |tag: &[u8]| {
        let i = (0..count).find(|&i| base.get(list + 2 + i * 6..list + 6 + i * 6) == Some(tag))?;
        offset16_in(base, list, 6 + i * 6)
    };
    let script = script_tag.and_then(|tag| script(&tag)).or_else(|| script(b"DFLT"))?;
    Some((axis, script))
}

// BASE 1.1's ItemVariationStore at a location in normalized coordinates.
fn deltas(base: &[u8], location: &[f64]) -> Option<DeviceDeltas> {
    if read_u16_be(base, 2)? < 1 { return None; }
    DeviceDeltas::new(base, read_u32_be(base, 8)? as usize, location)
}

// A BaseCoord in design units: formats 1, 2 and 4 (2 with a 24-bit glyph) as stored, the contour point 2 and 4
// name refining it only at a hinted size, and 3 moved by its VariationIndex.
fn coordinate(base: &[u8], at: usize, deltas: &mut Option<DeviceDeltas>) -> Option<f64> {
    let value = f64::from(read_i16_be(base, at + 2)?);
    match read_u16_be(base, at)? {
        1 | 2 | 4 => Some(value),
        3 => {
            let device = usize::from(read_u16_be(base, at + 4)?);
            let delta = match deltas { Some(d) if device != 0 => d.delta(base, at + device), _ => 0 };
            Some(value + units(delta))
        }
        _ => None,
    }
}

// A script's baselines on one axis, in design units at a location in normalized coordinates.
pub fn base_script_info(
    table_map: &Tables, script_tag: Option<[u8; 4]>, vertical: bool, location: &[f64],
) -> Option<BaseScript> {
    let base = table_map.get("BASE")?;
    let (axis, script) = base_script(base, vertical, script_tag)?;

    let tag_list = offset16_in(base, axis, 0)?;
    let tag_count = fitting_count(base, tag_list, 4);
    let tag = |i: usize| {
        let at = tag_list + 2 + i * 4;
        base.get(at..at + 4).and_then(|t| t.try_into().ok()).map(Tag::new)
    };

    let values = offset16_in(base, script, 0)?;
    let default_index = usize::from(read_u16_be(base, values)?);
    let coord_count = fitting_count(base, values + 2, 2);

    let mut deltas = deltas(base, location);
    let mut baselines = BTreeMap::new();
    for i in 0..coord_count.min(tag_count) {
        let Some(at) = offset16_in(base, values, 4 + i * 2) else { continue };
        if let Some(c) = coordinate(base, at, &mut deltas) {
            baselines.insert(tag(i)?, c);
        }
    }
    let default_baseline = if default_index < tag_count { tag(default_index) } else { None };
    Some(BaseScript { default_baseline, baselines })
}

// A script's extent on one axis in design units at a location: the language's MinMax, else the
// default, narrowed to a feature's record if any; None for a side the font leaves out.
pub fn base_min_max(
    table_map: &Tables, script_tag: Option<[u8; 4]>, language: Option<[u8; 4]>,
    feature: Option<[u8; 4]>, vertical: bool, location: &[f64],
) -> Option<(Option<f64>, Option<f64>)> {
    let base = table_map.get("BASE")?;
    let (_, script) = base_script(base, vertical, script_tag)?;

    let tagged = |at: usize, count: usize, stride: usize, tag: Option<[u8; 4]>| {
        let tag = tag?;
        (0..count).find(|&k| base.get(at + k * stride..at + k * stride + 4) == Some(&tag[..]))
    };
    let min_max = match tagged(script + 6, fitting_count(base, script + 4, 6), 6, language) {
        Some(k) => offset16_in(base, script, 10 + k * 6)?,
        None => offset16_in(base, script, 2)?,
    };
    let [min_field, max_field] = match tagged(min_max + 6, fitting_count(base, min_max + 4, 8), 8, feature) {
        Some(k) => [10 + k * 8, 12 + k * 8],
        None => [0, 2],
    };
    let mut deltas = deltas(base, location);
    let mut side = |field: usize| coordinate(base, offset16_in(base, min_max, field)?, &mut deltas);
    let min = side(min_field);
    Some((min, side(max_field)))
}

// Every BaseCoord the table names on either axis, each once however many records share it: each
// BaseScript's values, its default extent, and its languages' extents with their features'. Tables at distinct
// offsets can still overlap, so the records read are held to the subset budget; None past it.
pub(crate) fn base_coordinates(base: &[u8]) -> Option<Vec<usize>> {
    let mut coords = Vec::new();
    let (mut scripts, mut values, mut min_maxes) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let mut left = subset_budget(base.len());
    let mut fitting = |at: usize, stride: usize| {
        let count = fitting_count(base, at, stride);
        left = left.checked_sub(count)?;
        Some(count)
    };
    for axis in [4, 6].into_iter().filter_map(|field| offset16_in(base, 0, field)) {
        let Some(list) = offset16_in(base, axis, 2) else { continue };
        for i in 0..fitting(list, 6)? {
            let Some(script) = offset16_in(base, list, 6 + 6 * i).filter(|&s| scripts.insert(s)) else { continue };
            if let Some(at) = offset16_in(base, script, 0).filter(|&v| values.insert(v)) {
                coords.extend((0..fitting(at + 2, 2)?).filter_map(|j| offset16_in(base, at, 4 + 2 * j)));
            }
            let langs = (0..fitting(script + 4, 6)?).filter_map(|k| offset16_in(base, script, 10 + 6 * k));
            for min_max in offset16_in(base, script, 2)
                .into_iter()
                .chain(langs)
                .filter(|&m| min_maxes.insert(m))
                .collect::<Vec<_>>()
            {
                coords.extend([0, 2].into_iter().filter_map(|field| offset16_in(base, min_max, field)));
                for k in 0..fitting(min_max + 4, 8)? {
                    coords.extend([10, 12].into_iter().filter_map(|field| offset16_in(base, min_max, field + 8 * k)));
                }
            }
        }
    }
    Some(coords)
}

// Whether no BaseCoord names a glyph, as format 2 does by ID, so BASE stays valid however glyphs are
// renumbered. A format no reader knows may name one; a coordinate past the table names nothing. A table read past
// its budget may name any.
pub fn base_is_glyph_free(base: &[u8]) -> bool {
    base_coordinates(base).is_some_and(|coords| {
        coords.into_iter().all(|at| read_u16_be(base, at).is_none_or(|format| matches!(format, 1 | 3)))
    })
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/base.rs"]
mod tests;
