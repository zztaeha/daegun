use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be, read_i16_be};
use super::super::super::format::aat::Lookup;
use super::lookup::build_aat_lookup;
use super::state::{assemble, remap_aat, state_parts};

const SUBTABLE_HEADER: usize = 12;
const FORMAT0_HEADER: usize = 16;
const STATE_PLUS_ONE_OFFSET: usize = 20;
const FORMAT0_PAIR: usize = 6;

fn subset_format0(body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16) -> Option<Vec<u8>> {
    let n_pairs = read_u32_be(body, 0)? as usize;
    let mut kept: Vec<(u16, u16, i16)> = Vec::new();
    for i in 0..n_pairs {
        let at = FORMAT0_HEADER + i * FORMAT0_PAIR;
        let (Some(left), Some(right), Some(value)) =
            (read_u16_be(body, at), read_u16_be(body, at + 2), read_i16_be(body, at + 4)) else { break };
        let (Some(l), Some(r)) = (remap_aat(active, gid_map, num_glyphs, left), remap_aat(active, gid_map, num_glyphs, right))
        else { continue };
        kept.push((l, r, value));
    }
    kept.sort_unstable_by_key(|&(l, r, _)| (l, r));

    let n = kept.len();
    let (search_range, selector) = match n {
        0 => (0, 0),
        n => {
            let selector = (usize::BITS - 1 - n.leading_zeros()) as usize;
            (FORMAT0_PAIR << selector, selector)
        }
    };
    let mut out = Vec::with_capacity(FORMAT0_HEADER + n * FORMAT0_PAIR);
    for v in [n, search_range, selector, FORMAT0_PAIR * n - search_range] {
        out.extend_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    for (l, r, value) in kept {
        out.extend_from_slice(&l.to_be_bytes());
        out.extend_from_slice(&r.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }
    Some(out)
}

// A class lookup's kept glyphs with their values, rebuilt.
fn rebuild_classes(sub: &[u8], at: usize, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16) -> Option<Vec<u8>> {
    let lookup = Lookup::parse(sub.get(at..)?, num_glyphs)?;
    let kept: Vec<(u16, u16)> = lookup.entries().into_iter()
        .filter_map(|(g, v)| remap_aat(active, gid_map, num_glyphs, g).map(|ng| (ng, v)))
        .collect();
    build_aat_lookup(&kept)
}

// Where a part that starts at `from` ends: at the next part the header names, or the subtable's end.
fn end_of(sub: &[u8], from: usize, starts: &[usize]) -> usize {
    starts.iter().copied().chain([sub.len()]).filter(|&o| o > from).min().unwrap_or(sub.len())
}

// Formats 2 and 6 measure their offsets from the subtable's start, header included; the state
// formats measure from the state table after it.
fn subset_format2(
    sub: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let row_width = read_u32_be(sub, SUBTABLE_HEADER)?;
    let starts: Vec<usize> = (1..4).map(|i| read_u32_be(sub, SUBTABLE_HEADER + i * 4).map(|v| v as usize))
        .collect::<Option<_>>()?;
    let (left_off, right_off, array_off) = (starts[0], starts[1], starts[2]);
    let left = rebuild_classes(sub, left_off, active, gid_map, num_glyphs)?;
    let right = rebuild_classes(sub, right_off, active, gid_map, num_glyphs)?;
    let array = sub.get(array_off..end_of(sub, array_off, &starts))?;

    let new_left = SUBTABLE_HEADER + 16;
    let new_right = new_left + left.len();
    let new_array = new_right + right.len();
    let mut out = Vec::with_capacity(new_array - SUBTABLE_HEADER + array.len());
    for v in [row_width as usize, new_left, new_right, new_array] {
        out.extend_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    out.extend_from_slice(&left);
    out.extend_from_slice(&right);
    out.extend_from_slice(array);
    Some(out)
}

fn subset_format1(
    body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let value_off = read_u32_be(body, 16)? as usize;
    let parts = state_parts(body, &[value_off], active, gid_map, num_glyphs)?;
    let values = body.get(value_off..)?;

    let mut out = assemble(&parts, STATE_PLUS_ONE_OFFSET, &[values])?;
    let at = (out.len() - values.len()) as u32;
    out.get_mut(16..20)?.copy_from_slice(&at.to_be_bytes());
    Some(out)
}

fn subset_format4(
    body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    const ACTION_OFFSET: u32 = 0x00FF_FFFF;
    let flags = read_u32_be(body, 16)?;
    let table_off = (flags & ACTION_OFFSET) as usize;
    let parts = state_parts(body, &[table_off], active, gid_map, num_glyphs)?;
    let table = body.get(table_off..)?;

    let mut out = assemble(&parts, STATE_PLUS_ONE_OFFSET, &[table])?;
    let at = (out.len() - table.len()) as u32;
    if at & !ACTION_OFFSET != 0 { return None; }
    out.get_mut(16..20)?.copy_from_slice(&((flags & !ACTION_OFFSET) | at).to_be_bytes());
    Some(out)
}

fn subset_format6(
    sub: &[u8], tuple_count: u32, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    const VALUES_ARE_LONG: u32 = 1;
    let flags = read_u32_be(sub, SUBTABLE_HEADER)?;
    if flags & VALUES_ARE_LONG != 0 { return None; }

    let fields = if tuple_count >= 1 { 4 } else { 3 };
    let starts: Vec<usize> = (0..fields).map(|i| read_u32_be(sub, SUBTABLE_HEADER + 8 + i * 4).map(|v| v as usize))
        .collect::<Option<_>>()?;
    let rows = rebuild_classes(sub, starts[0], active, gid_map, num_glyphs)?;
    let cols = rebuild_classes(sub, starts[1], active, gid_map, num_glyphs)?;
    let array = sub.get(starts[2]..end_of(sub, starts[2], &starts))?;
    let vectors = match starts.get(3) {
        Some(&v) => Some(sub.get(v..end_of(sub, v, &starts))?),
        None => None,
    };

    let header = 8 + 4 * fields;
    let new_rows = SUBTABLE_HEADER + header;
    let new_cols = new_rows + rows.len();
    let new_array = new_cols + cols.len();
    let new_vectors = new_array + array.len();

    let mut out = sub.get(SUBTABLE_HEADER..SUBTABLE_HEADER + 8)?.to_vec();
    for v in [new_rows, new_cols, new_array, new_vectors].into_iter().take(fields) {
        out.extend_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    out.extend_from_slice(&rows);
    out.extend_from_slice(&cols);
    out.extend_from_slice(array);
    if let Some(v) = vectors { out.extend_from_slice(v); }
    Some(out)
}

pub fn subset_kerx(
    kerx: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let version = read_u16_be(kerx, 0)?;
    let n_tables = read_u32_be(kerx, 4)?;
    let budget = super::super::subset_budget(kerx.len());

    // A subtable that cannot be read ends the walk and one that cannot be rebuilt is left out, as
    // the shaper treats them.
    let mut out = kerx.get(..8)?.to_vec();
    let mut kept = 0u32;
    let mut at = 8usize;
    for _ in 0..n_tables {
        let Some(length) = read_u32_be(kerx, at).map(|v| v as usize).filter(|&l| l >= SUBTABLE_HEADER) else { break };
        let Some(sub) = at.checked_add(length).and_then(|end| kerx.get(at..end)) else { break };
        at += length;
        let (coverage, tuple_count) = (read_u32_be(sub, 4)?, read_u32_be(sub, 8)?);
        let body = &sub[SUBTABLE_HEADER..];

        let rebuilt = match coverage & 0xFF {
            0 if tuple_count == 0 => subset_format0(body, active, gid_map, num_glyphs),
            1 => subset_format1(body, active, gid_map, num_glyphs),
            2 if tuple_count == 0 => subset_format2(sub, active, gid_map, num_glyphs),
            4 => subset_format4(body, active, gid_map, num_glyphs),
            6 => subset_format6(sub, tuple_count, active, gid_map, num_glyphs),
            _ => None,
        };
        let Some(mut new_body) = rebuilt else { continue };
        new_body.resize(new_body.len().next_multiple_of(4), 0);
        out.extend_from_slice(&u32::try_from(SUBTABLE_HEADER + new_body.len()).ok()?.to_be_bytes());
        out.extend_from_slice(&sub[4..SUBTABLE_HEADER]);
        out.extend_from_slice(&new_body);
        kept += 1;
        if out.len() > budget { return None; }
    }
    if kept == 0 { return None; }
    out.get_mut(4..8)?.copy_from_slice(&kept.to_be_bytes());
    // Version 3 and later end with a coverage bitfield offset per subtable; 0xFFFFFFFF means none.
    if version >= 3 {
        (0..kept).for_each(|_| out.extend_from_slice(&u32::MAX.to_be_bytes()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn kerx_of(declared: u32, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let body: Vec<u8> = subtables.iter()
            .flat_map(|(format, b)| [[(12 + b.len()) as u32, *format, 0].map(u32::to_be_bytes).concat(), b.clone()].concat())
            .collect();
        [u16s(&[2, 0]), declared.to_be_bytes().to_vec(), body].concat()
    }

    fn everything(n: u16) -> (GlyphSet, Vec<u16>) {
        let mut active = GlyphSet::new();
        (0..n).for_each(|g| { active.insert(g); });
        (active, (0..n).collect())
    }

    // A format 1 state table over glyphs 1 to 65,533 in one segment, its values after the entries.
    fn state_kerning() -> Vec<u8> {
        let classes = u16s(&[2, 6, 2, 12, 1, 0, 65_533, 1, 4, 0xFFFF, 0xFFFF, 0]);
        let at = 20 + classes.len();
        [[5u32, 20, at as u32, (at + 20) as u32, (at + 24) as u32].map(u32::to_be_bytes).concat(), classes, alloc::vec![0; 20], u16s(&[0, 0, 0xFFFF, 0])].concat()
    }

    // Rebuilt at four bytes a glyph, 100 such subtables would be 26 MB from 8 KB; each stays one segment.
    #[test]
    fn a_segment_stays_a_segment() {
        let t = kerx_of(20, &(0..20).map(|_| (1, state_kerning())).collect::<Vec<_>>());
        let (active, gid_map) = everything(u16::MAX);
        let out = subset_kerx(&t, &active, &gid_map, u16::MAX).expect("a subset kerx");
        assert!(out.len() <= t.len() + 400, "{} bytes of kerx became {}", t.len(), out.len());
    }

    // A table naming one subtable more than it holds keeps the ones it holds.
    #[test]
    fn a_missing_last_subtable_keeps_the_ones_before() {
        let pairs = [[1u32, 6, 0, 0].map(u32::to_be_bytes).concat(), u16s(&[1, 2, 0xFFCE])].concat();
        let t = kerx_of(2, &[(0, pairs)]);
        let (active, gid_map) = everything(3);
        let out = subset_kerx(&t, &active, &gid_map, 3).expect("the readable subtable stays");
        assert_eq!(read_u32_be(&out, 4), Some(1));
    }
}
