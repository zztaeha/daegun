use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be};
use super::super::super::format::aat::Lookup;
use super::super::otl::remap_gid;
use super::lookup::build_aat_lookup;

const HEADER_LEN: usize = 16;

// Ids past the font's last glyph are placeholders morx passes between subtables (Zapfino's 32000
// to 32003) or the deleted 0xFFFF: they keep their ids, still past the subset's, and count as present.
pub(crate) fn remap_aat(active: &GlyphSet, gid_map: &[u16], num_glyphs: u16, g: u16) -> Option<u16> {
    if g >= num_glyphs { Some(g) } else { remap_gid(active, gid_map, g) }
}

pub(crate) fn present(active: &GlyphSet, num_glyphs: u16, g: u16) -> bool {
    g >= num_glyphs || active.contains(&g)
}

pub(crate) fn assemble(parts: &StateParts, header_len: usize, trailing: &[&[u8]]) -> Option<Vec<u8>> {
    let class_table = build_aat_lookup(&parts.classes)?;
    let class_off = header_len;
    let state_off = class_off + class_table.len();
    let entry_off = state_off + parts.state.len();

    let mut out = alloc::vec![0u8; header_len];
    for (i, v) in [parts.n_classes, class_off as u32, state_off as u32, entry_off as u32].iter().enumerate() {
        out.get_mut(i * 4..i * 4 + 4)?.copy_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&class_table);
    out.extend_from_slice(parts.state);
    out.extend_from_slice(parts.entry);
    for t in trailing { out.extend_from_slice(t); }
    Some(out)
}

pub(crate) struct StateParts<'a> {
    pub(crate) n_classes: u32,
    pub(crate) classes: Vec<(u16, u16)>,
    pub(crate) state: &'a [u8],
    pub(crate) entry: &'a [u8],
}

pub(crate) fn state_parts<'a>(
    data: &'a [u8], extra_starts: &[usize], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<StateParts<'a>> {
    let n_classes = read_u32_be(data, 0)?;
    let class_off = read_u32_be(data, 4)? as usize;
    let state_off = read_u32_be(data, 8)? as usize;
    let entry_off = read_u32_be(data, 12)? as usize;
    if n_classes == 0 || n_classes > 0xFFFF { return None; }

    let mut starts: Vec<usize> = alloc::vec![class_off, state_off, entry_off];
    starts.extend_from_slice(extra_starts);
    let end_of = |from: usize| starts.iter().copied().chain([data.len()])
        .filter(|&o| o > from).min().unwrap_or(data.len());

    let state = data.get(state_off..end_of(state_off))?;
    let entry = data.get(entry_off..end_of(entry_off))?;
    if state.is_empty() || entry.is_empty() { return None; }

    let lookup = Lookup::parse(data.get(class_off..end_of(class_off))?, num_glyphs)?;
    let classes: Vec<(u16, u16)> = lookup.entries().into_iter()
        .filter_map(|(g, class)| remap_aat(active, gid_map, num_glyphs, g).map(|ng| (ng, class)))
        .collect();

    Some(StateParts { n_classes, classes, state, entry })
}

pub(crate) fn entry_table<'a>(data: &'a [u8], extra_starts: &[usize]) -> Option<&'a [u8]> {
    let entry_off = read_u32_be(data, 12)? as usize;
    let mut starts: Vec<usize> = alloc::vec![
        read_u32_be(data, 4)? as usize, read_u32_be(data, 8)? as usize, entry_off,
    ];
    starts.extend_from_slice(extra_starts);
    let end = starts.iter().copied().chain([data.len()])
        .filter(|&o| o > entry_off).min().unwrap_or(data.len());
    data.get(entry_off..end)
}

// The entries a run can reach: from the two start states, through the classes active glyphs fall
// in and the four the shaper makes itself (end of text, out of bounds, deleted glyph, end of line).
pub(crate) fn reachable_entries(
    data: &[u8], extra_starts: &[usize], extra_words: usize, active: &GlyphSet, num_glyphs: u16,
) -> Option<BTreeSet<u16>> {
    let n_classes = read_u32_be(data, 0)? as usize;
    let class_off = read_u32_be(data, 4)? as usize;
    let state_off = read_u32_be(data, 8)? as usize;
    if n_classes == 0 || n_classes > 0xFFFF { return None; }
    let entry = entry_table(data, extra_starts)?;
    let mut starts: Vec<usize> = alloc::vec![class_off, state_off, read_u32_be(data, 12)? as usize];
    starts.extend_from_slice(extra_starts);
    let state_end = starts.iter().copied().chain([data.len()]).filter(|&o| o > state_off).min().unwrap_or(data.len());
    let n_states = state_end.saturating_sub(state_off) / (2 * n_classes);
    let class_end = starts.iter().copied().chain([data.len()]).filter(|&o| o > class_off).min().unwrap_or(data.len());

    let lookup = Lookup::parse(data.get(class_off..class_end)?, num_glyphs)?;
    let mut classes: BTreeSet<u16> = (0..4).collect();
    classes.extend(lookup.entries().into_iter().filter(|&(g, _)| present(active, num_glyphs, g)).map(|(_, c)| c));
    classes.retain(|&c| usize::from(c) < n_classes);

    let (mut seen, mut reached) = (alloc::vec![false; n_states], BTreeSet::new());
    let mut queue: Vec<usize> = (0..n_states.min(2)).collect();
    queue.iter().for_each(|&s| seen[s] = true);
    while let Some(s) = queue.pop() {
        for &c in &classes {
            let Some(e) = read_u16_be(data, state_off + 2 * (s * n_classes + usize::from(c))) else { continue };
            reached.insert(e);
            let Some(next) = read_u16_be(entry, usize::from(e) * (4 + 2 * extra_words)).map(usize::from) else { continue };
            if next < n_states && !seen[next] {
                seen[next] = true;
                queue.push(next);
            }
        }
    }
    Some(reached)
}

pub(crate) fn entries_of(entry: &[u8], extra_words: usize) -> Vec<(u16, u16, u16)> {
    let stride = 4 + 2 * extra_words;
    (0..entry.len() / stride)
        .filter_map(|i| {
            let at = i * stride;
            Some((
                read_u16_be(entry, at + 2)?,
                if extra_words >= 1 { read_u16_be(entry, at + 4)? } else { 0 },
                if extra_words >= 2 { read_u16_be(entry, at + 6)? } else { 0 },
            ))
        })
        .collect()
}

pub(crate) fn subset_state_table(
    data: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let parts = state_parts(data, &[], active, gid_map, num_glyphs)?;
    assemble(&parts, HEADER_LEN, &[])
}
