use alloc::vec::Vec;
use super::super::super::decoder::{bin_search_header, midpoint_search};
use super::super::super::format::aat::Lookup;

// A lookup's entries whose glyphs the subset keeps, renumbered by `remap`, with their values.
pub(crate) fn kept_entries(lookup: &Lookup, remap: impl Fn(u16) -> Option<u16>) -> Vec<(u16, u16)> {
    lookup.pairs().into_iter().filter_map(|(g, v)| Some((remap(g)?, v))).collect()
}

// nUnits counts every unit searched, the 0xFFFF one the lookup chapter has end a search among them.
fn bin_srch(out: &mut Vec<u8>, unit_size: usize, n_units: usize) -> Option<()> {
    for v in [unit_size, n_units].into_iter().chain(bin_search_header(n_units, unit_size)) {
        out.extend_from_slice(&u16::try_from(v).ok()?.to_be_bytes());
    }
    Some(())
}

fn push(out: &mut Vec<u8>, words: &[u16]) {
    words.iter().for_each(|w| out.extend_from_slice(&w.to_be_bytes()));
}

fn consecutive(a: &(u16, u16), b: &(u16, u16)) -> bool {
    a.0.checked_add(1) == Some(b.0)
}

// Segments of consecutive glyphs sharing one value.
fn format2(sorted: &[(u16, u16)]) -> Option<Vec<u8>> {
    let segments: Vec<&[(u16, u16)]> = sorted.chunk_by(|a, b| consecutive(a, b) && a.1 == b.1).collect();
    let mut out = alloc::vec![0, 2];
    bin_srch(&mut out, 6, segments.len() + 1)?;
    for s in &segments {
        push(&mut out, &[s[s.len() - 1].0, s[0].0, s[0].1]);
    }
    push(&mut out, &[0xFFFF, 0xFFFF, 0]);
    Some(out)
}

// Segments of consecutive glyphs, each pointing at its own array of values.
fn format4(sorted: &[(u16, u16)]) -> Option<Vec<u8>> {
    let runs: Vec<&[(u16, u16)]> = sorted.chunk_by(consecutive).collect();
    let mut out = alloc::vec![0, 4];
    bin_srch(&mut out, 6, runs.len() + 1)?;
    let mut values_at = out.len() + 6 * (runs.len() + 1);
    for r in &runs {
        push(&mut out, &[r[r.len() - 1].0, r[0].0, u16::try_from(values_at).ok()?]);
        values_at += 2 * r.len();
    }
    push(&mut out, &[0xFFFF, 0xFFFF, 0]);
    for r in &runs {
        r.iter().for_each(|&(_, v)| push(&mut out, &[v]));
    }
    Some(out)
}

fn format6(sorted: &[(u16, u16)]) -> Option<Vec<u8>> {
    let mut out = alloc::vec![0, 6];
    bin_srch(&mut out, 4, sorted.len() + 1)?;
    sorted.iter().for_each(|&(g, v)| push(&mut out, &[g, v]));
    push(&mut out, &[0xFFFF, 0]);
    Some(out)
}

fn format6_long(sorted: &[(u16, u32)]) -> Option<Vec<u8>> {
    let mut out = alloc::vec![0, 6];
    bin_srch(&mut out, 6, sorted.len())?;
    for &(g, v) in sorted.iter().chain([&(0xFFFF, 0)]) {
        push(&mut out, &[g]);
        out.extend_from_slice(&v.to_be_bytes());
    }
    Some(out)
}

// One array from the first glyph to the last, so only for glyphs with no gaps between them.
fn format8(sorted: &[(u16, u16)]) -> Option<Vec<u8>> {
    let first = sorted.first().map_or(0, |e| e.0);
    if sorted.last().is_some_and(|l| usize::from(l.0 - first) + 1 != sorted.len()) { return None; }
    let mut out = alloc::vec![0, 8];
    push(&mut out, &[first, u16::try_from(sorted.len()).ok()?]);
    sorted.iter().for_each(|&(_, v)| push(&mut out, &[v]));
    Some(out)
}

// Format 10's `unit`-byte values from the first glyph to the last. A subset keeps glyph order, so every glyph between
// kept ones comes from the font's array with its value and none keeps the fill.
fn format10<V: Copy + Into<u32>>(sorted: &[(u16, V)], unit: usize) -> Option<Vec<u8>> {
    let first = sorted.first().map_or(0, |e| e.0);
    let count = sorted.last().map_or(0, |l| usize::from(l.0 - first) + 1);
    let mut values = alloc::vec![1u32; count];
    sorted.iter().for_each(|&(g, v)| values[usize::from(g - first)] = v.into());
    let mut out = alloc::vec![0, 10];
    push(&mut out, &[u16::try_from(unit).ok()?, first, u16::try_from(count).ok()?]);
    // A unit wider than a value's 4 bytes takes zeros before them.
    values.iter().for_each(|&v| {
        out.extend(core::iter::repeat_n(0, unit.saturating_sub(4)));
        out.extend_from_slice(&v.to_be_bytes()[4 - unit.min(4)..]);
    });
    Some(out)
}

fn sorted<V: Copy>(entries: &[(u16, V)]) -> Vec<(u16, V)> {
    let mut sorted: Vec<(u16, V)> = entries.iter().copied().filter(|&(g, _)| g != 0xFFFF).collect();
    sorted.sort_by_key(|&(g, _)| g);
    sorted.dedup_by_key(|&mut (g, _)| g);
    sorted
}

// The smallest format that holds `entries`, the first listed on a tie. A glyph named twice keeps its
// first value; no entries is a valid empty lookup.
pub(crate) fn build_aat_lookup(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    let sorted = sorted(entries);
    [format2(&sorted), format4(&sorted), format6(&sorted), format8(&sorted)].into_iter().flatten().min_by_key(Vec::len)
}

// A class lookup rebuilt in format 10 where its font's is, of the same width, as no other format reads classes and
// collects glyphs alike. A glyph named twice keeps both units (`format6_twice`).
pub(crate) fn build_class_lookup(entries: &[(u16, u16)], wide_unit: Option<usize>) -> Option<Vec<u8>> {
    let mut glyphs: Vec<u16> = entries.iter().map(|&(g, _)| g).collect();
    glyphs.sort_unstable();
    match wide_unit {
        Some(unit) => format10(&sorted(entries), unit),
        None if glyphs.windows(2).any(|w| w[0] == w[1]) => format6_twice(entries),
        None => build_aat_lookup(entries),
    }
}

// Format 6 with a glyph's two classes in two units, the class it reads first: the start set collects both units,
// and the one the search lands on takes the class read.
fn format6_twice(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    let mut units: Vec<(u16, u16)> = entries.iter().copied().filter(|&(g, _)| g != 0xFFFF).collect();
    units.sort_by_key(|&(g, _)| g);
    for i in 1..units.len() {
        let g = units[i].0;
        if units[i - 1].0 == g && midpoint_search(units.len(), |k| Some(g.cmp(&units[k].0))) == Some(i) {
            let read = units[i - 1].1;
            units[i - 1].1 = units[i].1;
            units[i].1 = read;
        }
    }
    format6(&units)
}

// The same, from the formats whose size depends on the glyphs alone, so values that are offsets past
// the lookup can be worked out from a first build with any values.
pub(crate) fn build_aat_offset_lookup(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    let sorted = sorted(entries);
    [format4(&sorted), format6(&sorted), format8(&sorted)].into_iter().flatten().min_by_key(Vec::len)
}

// kerx format 6's long rows and columns, 32-bit lookups rebuilt as `build_class_lookup` rebuilds a class lookup.
pub(crate) fn build_long_lookup(entries: &[(u16, u32)], wide_unit: Option<usize>) -> Option<Vec<u8>> {
    match wide_unit {
        Some(unit) => format10(&sorted(entries), unit),
        None => format6_long(&sorted(entries)),
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/lookup.rs"]
mod tests;
