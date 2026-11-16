use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be, Placed};
use super::super::super::format::aat::{end_of, Lookup};
use super::super::remap_gid16;
use super::lookup::{build_aat_lookup, build_aat_offset_lookup, kept_entries};

const DIRECTION_HEADER: usize = 6;
const ACTION_HEADER: usize = 8;
mod action {
    pub(super) const DECOMPOSITION: u16 = 0;
    pub(super) const ADD_GLYPH: u16 = 1;
    pub(super) const CONDITIONAL_ADD_GLYPH: u16 = 2;
    pub(super) const STRETCH: u16 = 3;
    pub(super) const DUCTILE: u16 = 4;
    pub(super) const REPEATED_ADD_GLYPH: u16 = 5;
}

const NO_GLYPH: u16 = 0xFFFF;

// The list at `at` with its glyphs renumbered; each record read spends its bytes from `left`, None once it runs out.
fn subset_actions(
    data: &[u8], at: usize, active: &GlyphSet, gid_map: &[u32], left: &mut Option<usize>,
) -> Option<Vec<u8>> {
    let count = read_u32_be(data, at)?;
    let mut kept: Vec<Vec<u8>> = Vec::new();

    let mut cursor = at + 4;
    for _ in 0..count {
        let action_type = read_u16_be(data, cursor + 2)?;
        let length = read_u32_be(data, cursor + 4)? as usize;
        if length < ACTION_HEADER { return None; }
        let record = data.get(cursor..cursor.checked_add(length)?)?;
        *left = left.and_then(|left| left.checked_sub(length));
        left.as_ref()?;
        cursor += length;

        let remap_at = |out: &mut Vec<u8>, offset: usize| -> Option<()> {
            let g = read_u16_be(record, ACTION_HEADER + offset)?;
            let new = remap_gid16(active, gid_map, g)?;
            out.get_mut(ACTION_HEADER + offset..ACTION_HEADER + offset + 2)?
                .copy_from_slice(&new.to_be_bytes());
            Some(())
        };

        let mut out = record.to_vec();
        let survived = match action_type {
            action::DECOMPOSITION => {
                let n = read_u16_be(record, ACTION_HEADER + 10)? as usize;
                let glyphs_at = ACTION_HEADER + 12;
                if glyphs_at + n * 2 > record.len() { return None; }
                (0..n).try_for_each(|i| remap_at(&mut out, 12 + i * 2)).is_some()
            }
            action::ADD_GLYPH => remap_at(&mut out, 0).is_some(),
            action::CONDITIONAL_ADD_GLYPH => {
                let add = read_u16_be(record, ACTION_HEADER + 4)?;
                let new_add = if add == NO_GLYPH { NO_GLYPH } else {
                    remap_gid16(active, gid_map, add).unwrap_or(NO_GLYPH)
                };
                out.get_mut(ACTION_HEADER + 4..ACTION_HEADER + 6)?.copy_from_slice(&new_add.to_be_bytes());
                remap_at(&mut out, 6).is_some()
            }
            action::REPEATED_ADD_GLYPH => remap_at(&mut out, 2).is_some(),
            action::STRETCH | action::DUCTILE => true,
            _ => false,
        };
        if survived { kept.push(out); }
    }
    if kept.is_empty() { return None; }

    let mut out = (kept.len() as u32).to_be_bytes().to_vec();
    for k in kept { out.extend_from_slice(&k); }
    Some(out)
}

// The kept glyphs a lookup names, with their values; an empty list when it names none.
fn remap_keys(data: &[u8], at: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32)
    -> Option<Vec<(u16, u16)>>
{
    Some(kept_entries(&Lookup::parse(data.get(at..)?, num_glyphs).ok()?, |g| remap_gid16(active, gid_map, g)))
}

// The category table is mort-style: an 8-byte header and a classic state table. Its class array is
// rebuilt in place, which only shrinks it, so the state array and entries keep their offsets.
fn subset_category_table(just: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    const STATE_TABLE: usize = 8;
    let length = usize::from(read_u16_be(just, off)?);
    let mut table = just.get(off..off.checked_add(length)?)?.to_vec();
    let class_at = STATE_TABLE + usize::from(read_u16_be(&table, STATE_TABLE + 2)?);
    let first = read_u16_be(&table, class_at)?;
    let n = usize::from(read_u16_be(&table, class_at + 2)?);
    let kept: Vec<(u16, u8)> = table.get(class_at + 4..class_at + 4 + n)?.iter().enumerate()
        .filter_map(|(i, &class)| {
            let g = first.checked_add(u16::try_from(i).ok()?)?;
            Some((remap_gid16(active, gid_map, g)?, class))
        })
        .collect();

    let new_first = kept.first().map_or(0, |&(g, _)| g);
    let new_n = kept.last().map_or(0, |&(g, _)| usize::from(g - new_first) + 1);
    let mut classes = alloc::vec![1u8; new_n];
    kept.into_iter().for_each(|(g, class)| classes[usize::from(g - new_first)] = class);
    table.get_mut(class_at..class_at + 4)?
        .copy_from_slice(&[new_first.to_be_bytes(), u16::try_from(new_n).ok()?.to_be_bytes()].concat());
    table.get_mut(class_at + 4..class_at + 4 + new_n)?.copy_from_slice(&classes);
    Some(table)
}

// Glyph to action list. A list many glyphs share is rebuilt and written once, and a value of 0 means
// the glyph has no action.
fn subset_postcompensation(
    just: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, left: &mut Option<usize>,
) -> Option<Vec<u8>> {
    let mut lists: BTreeMap<u16, Option<Vec<u8>>> = BTreeMap::new();
    let mut kept: Vec<(u16, u16)> = Vec::new();
    for (g, value) in remap_keys(just, off, active, gid_map, num_glyphs)? {
        if value == 0 { continue; }
        let list = lists.entry(value)
            .or_insert_with(|| subset_actions(just, off.checked_add(usize::from(value))?, active, gid_map, left));
        if list.is_some() { kept.push((g, value)); }
    }
    if kept.is_empty() { return None; }

    let mut actions = Placed::new(build_aat_offset_lookup(&kept)?.len());
    let entries: Vec<(u16, u16)> = kept.iter()
        .map(|&(g, value)| Some((g, actions.place(lists.get(&value)?.as_deref()?)?)))
        .collect::<Option<_>>()?;
    let mut table = build_aat_offset_lookup(&entries)?;
    table.extend_from_slice(&actions.bytes);
    Some(table)
}

struct Direction {
    head: Vec<u8>,
    class_table: Option<Vec<u8>>,
    wdc: Vec<u8>,
    pc: Option<Vec<u8>>,
}

fn subset_direction(
    just: &[u8], at: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, left: &mut Option<usize>,
) -> Option<Direction> {
    let class_off = read_u16_be(just, at)? as usize;
    let wdc_off = read_u16_be(just, at + 2)? as usize;
    let pc_off = read_u16_be(just, at + 4)? as usize;

    let mut head = alloc::vec![0u8; DIRECTION_HEADER];
    head.extend_from_slice(&build_aat_lookup(&remap_keys(just, at + DIRECTION_HEADER, active, gid_map, num_glyphs)?)?);

    let class_table = (class_off != 0).then(|| subset_category_table(just, class_off, active, gid_map)).flatten();
    let wdc = just.get(wdc_off..end_of(&[class_off, pc_off], wdc_off, just.len()))?.to_vec();
    let pc = (pc_off != 0).then(|| subset_postcompensation(just, pc_off, active, gid_map, num_glyphs, left)).flatten();

    Some(Direction { head, class_table, wdc, pc })
}

pub fn subset_just(
    just: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    let version = read_u32_be(just, 0)?;
    let format = read_u16_be(just, 4)?;

    // A list can start inside another, so values reading on through the same records read them again: what both
    // directions read is held to the subset budget, and the table is refused past it.
    let mut built: [Option<Direction>; 2] = [None, None];
    let mut left = Some(super::super::subset_budget(just.len()));
    for (i, slot) in [6usize, 8].into_iter().enumerate() {
        let off = read_u16_be(just, slot)? as usize;
        if off == 0 { continue; }
        built[i] = subset_direction(just, off, active, gid_map, num_glyphs, &mut left);
    }
    left?;
    if built.iter().all(Option::is_none) { return None; }

    // Every offset is 16 bits from the table's start: a layout past that is refused, not wrapped. A
    // part both directions share, such as one width delta table, is written once.
    let mut dir_offsets = [0u16; 2];
    let mut at = 10usize;
    for (i, d) in built.iter().enumerate() {
        let Some(d) = d else { continue };
        dir_offsets[i] = u16::try_from(at).ok()?;
        at += d.head.len();
    }
    let mut parts = Placed::new(at);
    let mut table_offsets = [[0u16; 3]; 2];
    for (i, d) in built.iter().enumerate() {
        let Some(d) = d else { continue };
        for (k, part) in [d.class_table.as_deref(), Some(d.wdc.as_slice()), d.pc.as_deref()].into_iter().enumerate() {
            let Some(part) = part else { continue };
            table_offsets[i][k] = parts.place(part)?;
        }
    }
    let at = at + parts.bytes.len();
    if at > super::super::subset_budget(just.len()) { return None; }

    let mut out = Vec::with_capacity(at);
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&dir_offsets[0].to_be_bytes());
    out.extend_from_slice(&dir_offsets[1].to_be_bytes());
    for (i, d) in built.iter().enumerate() {
        let Some(d) = d else { continue };
        let mut head = d.head.clone();
        for (k, v) in table_offsets[i].iter().enumerate() {
            head.get_mut(k * 2..k * 2 + 2)?.copy_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&head);
    }
    out.extend_from_slice(&parts.bytes);
    Some(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/just.rs"]
mod tests;
