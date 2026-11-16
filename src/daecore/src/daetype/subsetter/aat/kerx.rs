use crate::daecore::daeshaper::ot::kerx::acts;
use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::super::decoder::{bin_search_header, midpoint_search, read_u16_be, read_u32_be, read_i16_be};
use super::super::super::format::aat::{tuple_kern, Lookup};
use super::super::remap::format0_pairs;
use super::lookup::{build_class_lookup, build_long_lookup};
use super::state::{assemble, charge, remap_aat, state_parts};

const SUBTABLE_HEADER: usize = 12;
const FORMAT0_HEADER: usize = 16;
const STATE_PLUS_ONE_OFFSET: usize = 20;
const FORMAT0_PAIR: usize = 6;

// A value is read through its tuple, from the subtable's start, so the subset needs none.
fn subset_format0(
    body: &[u8], reach: &[u8], tuples: u32, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    let n_pairs = (read_u32_be(body, 0)? as usize).min(body.len().saturating_sub(FORMAT0_HEADER) / FORMAT0_PAIR);
    let key_at = |i: usize| read_u32_be(body, FORMAT0_HEADER + i * FORMAT0_PAIR);
    let remap = |g: u32| remap_aat(active, gid_map, num_glyphs, g as u16);

    // Each pair with the value the shaper's search finds in the font, as `kern`'s are.
    let (mut kept, mut lefts) = (Vec::new(), BTreeSet::new());
    for i in 0..n_pairs {
        let Some(key) = key_at(i) else { break };
        let Some(l) = remap(key >> 16) else { continue };
        lefts.insert(l);
        let Some(r) = remap(key & 0xFFFF) else { continue };
        let Some(found) = midpoint_search(n_pairs, |j| key_at(j).map(|k| key.cmp(&k))) else { continue };
        let Some(value) = read_i16_be(body, FORMAT0_HEADER + found * FORMAT0_PAIR + 4) else { break };
        kept.push((l, r, tuple_kern(reach, 0, value.into(), tuples) as i16));
    }
    let kept = format0_pairs(kept, lefts);

    let n = kept.len();
    let mut out = Vec::with_capacity(FORMAT0_HEADER + n * FORMAT0_PAIR);
    for v in [n].into_iter().chain(bin_search_header(n, FORMAT0_PAIR)) {
        out.extend_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    for (l, r, value) in kept {
        out.extend_from_slice(&l.to_be_bytes());
        out.extend_from_slice(&r.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }
    Some(out)
}

// A class lookup's kept glyphs, each one it lists with the value its search gives or 0, rebuilt at its width
// (formats 2 and 6 kern between the glyphs it lists), and the highest value kept. A few bytes can name every glyph,
// so its reads spend `work`.
fn rebuild_classes(
    sub: &[u8], at: usize, long: bool, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<(Vec<u8>, Option<u32>)> {
    let lookup = Lookup::parse(sub.get(at..)?, num_glyphs).ok()?;
    charge(work, lookup.reads())?;
    let remap = |g| remap_aat(active, gid_map, num_glyphs, g);
    if long {
        let kept: Vec<(u16, u32)> =
            lookup.listed_entries_u32().into_iter().filter_map(|(g, v)| Some((remap(g)?, v))).collect();
        let highest = kept.iter().map(|&(_, v)| v).max();
        return Some((build_long_lookup(&kept, lookup.wide_unit())?, highest));
    }
    let kept: Vec<(u16, u16)> =
        lookup.listed_entries().into_iter().filter_map(|(g, v)| Some((remap(g)?, v))).collect();
    let highest = kept.iter().map(|&(_, v)| u32::from(v)).max();
    Some((build_class_lookup(&kept, lookup.wide_unit())?, highest))
}

// Formats 2 and 6 read the value their two classes sum to, as an index or format 2's byte offset, wherever it lands
// in what the subtable reads, 0 past that, then through its tuple. A subset writes each value its kept classes reach
// as the font reads it.
fn sums(first: Option<u32>, second: Option<u32>) -> usize {
    first.zip(second).map_or(0, |(a, b)| (a as usize).saturating_add(b as usize).saturating_add(1))
}

// Formats 2 and 6 measure their offsets from the subtable's start, header included; the state formats measure
// from the state table after it.
fn subset_format2(
    sub: &[u8], tuples: u32, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<Vec<u8>> {
    let field = |i: usize| read_u32_be(sub, SUBTABLE_HEADER + 4 * i).map(|v| v as usize);
    let (row_width, array) = (field(0)?, field(3)?);
    let (left, left_max) = rebuild_classes(sub, field(1)?, false, active, gid_map, num_glyphs, work)?;
    let (right, right_max) = rebuild_classes(sub, field(2)?, false, active, gid_map, num_glyphs, work)?;
    let value = |i: usize| {
        let v = array.checked_add(2 * i).and_then(|at| read_i16_be(sub, at));
        v.map_or(0, |v| tuple_kern(sub, 0, v.into(), tuples) as i16)
    };

    let new_left = SUBTABLE_HEADER + 16;
    let new_right = new_left + left.len();
    let new_array = new_right + right.len();
    // A byte offset: the values reached run to half the largest sum.
    let n = sums(left_max, right_max).div_ceil(2);
    let mut out = Vec::with_capacity(new_array - SUBTABLE_HEADER + 2 * n);
    for v in [row_width, new_left, new_right, new_array] {
        out.extend_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    out.extend_from_slice(&left);
    out.extend_from_slice(&right);
    (0..n).for_each(|i| out.extend_from_slice(&value(i).to_be_bytes()));
    Some(out)
}

fn subset_format1(
    body: &[u8], own: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<Vec<u8>> {
    let value_off = read_u32_be(body, 16)? as usize;
    let parts = state_parts(body, &[value_off, own], (1, acts), active, gid_map, num_glyphs, work)?;
    let values = body.get(value_off..)?;

    let mut out = assemble(&parts, STATE_PLUS_ONE_OFFSET, &[values])?;
    let at = (out.len() - values.len()) as u32;
    out.get_mut(16..20)?.copy_from_slice(&at.to_be_bytes());
    Some(out)
}

fn subset_format4(
    body: &[u8], own: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<Vec<u8>> {
    const ACTION_OFFSET: u32 = 0x00FF_FFFF;
    let flags = read_u32_be(body, 16)?;
    let table_off = (flags & ACTION_OFFSET) as usize;
    let parts = state_parts(body, &[table_off, own], (1, acts), active, gid_map, num_glyphs, work)?;
    let table = body.get(table_off..)?;

    let mut out = assemble(&parts, STATE_PLUS_ONE_OFFSET, &[table])?;
    let at = (out.len() - table.len()) as u32;
    if at & !ACTION_OFFSET != 0 { return None; }
    out.get_mut(16..20)?.copy_from_slice(&((flags & !ACTION_OFFSET) | at).to_be_bytes());
    Some(out)
}

// A long value's sum, or its byte offset, past 32 bits reads 0 in the font and the subset alike, so the array stops
// there; short of it the array can still pass the budget, which is checked before it's built.
fn subset_format6(
    sub: &[u8], tuples: u32, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, budget: usize, work: &mut usize,
) -> Option<Vec<u8>> {
    const VALUES_ARE_LONG: u32 = 1;
    let long = read_u32_be(sub, SUBTABLE_HEADER)? & VALUES_ARE_LONG != 0;
    let field = |i: usize| read_u32_be(sub, SUBTABLE_HEADER + 8 + 4 * i).map(|v| v as usize);
    let (array, vector) = (field(2)?, field(3)?);
    let (rows, row_max) = rebuild_classes(sub, field(0)?, long, active, gid_map, num_glyphs, work)?;
    let (cols, col_max) = rebuild_classes(sub, field(1)?, long, active, gid_map, num_glyphs, work)?;
    let width = if long { 4 } else { 2 };
    let n = if long { sums(row_max, col_max).min(u32::MAX as usize / 4 + 1) } else { sums(row_max, col_max) };
    if n.checked_mul(width)? > budget { return None; }
    let value = |i: usize| {
        let at = array.checked_add(width * i);
        let v = if long {
            at.and_then(|at| read_u32_be(sub, at)).map(|v| v as i32)
        } else {
            at.and_then(|at| read_i16_be(sub, at)).map(i32::from)
        };
        v.map_or(0, |v| tuple_kern(sub, vector, v, tuples))
    };

    let new_rows = SUBTABLE_HEADER + 24;
    let new_cols = new_rows + rows.len();
    let new_array = new_cols + cols.len();
    let mut out = Vec::with_capacity(new_array - SUBTABLE_HEADER + width * n);
    out.extend_from_slice(sub.get(SUBTABLE_HEADER..SUBTABLE_HEADER + 8)?);
    for v in [new_rows, new_cols, new_array, 0] {
        out.extend_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    out.extend_from_slice(&rows);
    out.extend_from_slice(&cols);
    for i in 0..n {
        if long {
            out.extend_from_slice(&value(i).to_be_bytes());
        } else {
            out.extend_from_slice(&(value(i) as i16).to_be_bytes());
        }
    }
    Some(out)
}

pub fn subset_kerx(
    kerx: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    let version = read_u16_be(kerx, 0)?;
    let n_tables = read_u32_be(kerx, 4)?;
    let budget = super::super::subset_budget(kerx.len());
    // What reading the subtables' machines and class lookups may spend together, 64 for each unit of the budget.
    let mut work = budget.saturating_mul(64);

    // A subtable that cannot be read ends the walk and one that cannot be rebuilt is left out, as
    // the shaper treats them.
    let mut out = kerx.get(..8)?.to_vec();
    let mut kept = 0u32;
    let mut at = 8usize;
    for _ in 0..n_tables {
        let Some(length) = read_u32_be(kerx, at).map(|v| v as usize).filter(|&l| l >= SUBTABLE_HEADER) else { break };
        let Some(sub) = at.checked_add(length).and_then(|end| kerx.get(at..end)) else { break };
        let reach = sub;
        at += length;
        // tupleCount is read from kerx version 4 on; before, the field holds no count.
        let (coverage, tuple_count) = (read_u32_be(sub, 4)?, if version >= 4 { read_u32_be(sub, 8)? } else { 0 });
        let (body, own) = (&reach[SUBTABLE_HEADER..], length - SUBTABLE_HEADER);

        let (rebuilt, tuples) = match coverage & 0xFF {
            0 => (subset_format0(body, reach, tuple_count, active, gid_map, num_glyphs), 0),
            1 => (subset_format1(body, own, active, gid_map, num_glyphs, &mut work), tuple_count),
            2 => (subset_format2(reach, tuple_count, active, gid_map, num_glyphs, &mut work), 0),
            4 => (subset_format4(body, own, active, gid_map, num_glyphs, &mut work), tuple_count),
            6 => (subset_format6(reach, tuple_count, active, gid_map, num_glyphs, budget, &mut work), 0),
            _ => (None, tuple_count),
        };
        let Some(new_body) = rebuilt else { continue };
        out.extend_from_slice(&u32::try_from(SUBTABLE_HEADER + new_body.len()).ok()?.to_be_bytes());
        out.extend_from_slice(&sub[4..8]);
        out.extend_from_slice(&tuples.to_be_bytes());
        out.extend_from_slice(&new_body);
        kept += 1;
        if out.len() > budget { return None; }
    }
    // A kerx left with no subtable stays: any kerx the sanitizer passes counts as data, which keeps `kern` and the
    // fallback down as in the font.
    out.get_mut(4..8)?.copy_from_slice(&kept.to_be_bytes());
    // Version 3 and later end with a coverage bitfield offset per subtable; 0xFFFFFFFF means none.
    if version >= 3 {
        (0..kept).for_each(|_| out.extend_from_slice(&u32::MAX.to_be_bytes()));
    }
    Some(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/kerx.rs"]
mod tests;
