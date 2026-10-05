use alloc::vec::Vec;

// nUnits counts the 0xFFFF terminator in the segment formats and not in format 6, as Apple's own
// fonts write them; readers take either.
fn bin_srch(out: &mut Vec<u8>, unit_size: usize, n_units: usize) -> Option<()> {
    let (search_range, selector) = match n_units {
        0 => (0, 0),
        n => {
            let selector = (usize::BITS - 1 - n.leading_zeros()) as usize;
            (unit_size << selector, selector)
        }
    };
    let range_shift = (unit_size * n_units).checked_sub(search_range)?;
    for v in [unit_size, n_units, search_range, selector, range_shift] {
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
    bin_srch(&mut out, 4, sorted.len())?;
    sorted.iter().for_each(|&(g, v)| push(&mut out, &[g, v]));
    push(&mut out, &[0xFFFF, 0]);
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

fn sorted(entries: &[(u16, u16)]) -> Vec<(u16, u16)> {
    let mut sorted: Vec<(u16, u16)> = entries.iter().copied().filter(|&(g, _)| g != 0xFFFF).collect();
    sorted.sort_by_key(|&(g, _)| g);
    sorted.dedup_by_key(|&mut (g, _)| g);
    sorted
}

// The smallest format that holds `entries`, the first listed on a tie, as fontTools writes them. A
// glyph named twice keeps its first value; no entries is a valid empty lookup.
pub(crate) fn build_aat_lookup(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    let sorted = sorted(entries);
    [format2(&sorted), format4(&sorted), format6(&sorted), format8(&sorted)].into_iter().flatten().min_by_key(Vec::len)
}

// The same, from the formats whose size depends on the glyphs alone, so values that are offsets past
// the lookup can be worked out from a first build with any values.
pub(crate) fn build_aat_offset_lookup(entries: &[(u16, u16)]) -> Option<Vec<u8>> {
    let sorted = sorted(entries);
    [format4(&sorted), format6(&sorted), format8(&sorted)].into_iter().flatten().min_by_key(Vec::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daecore::daetype::format::aat::Lookup;

    fn read_back(table: &[u8], entries: &[(u16, u16)]) {
        let lookup = Lookup::parse(table, u16::MAX).expect("the lookup parses");
        assert_eq!(lookup.entries(), entries, "format {}", table[1]);
        for &(g, v) in entries {
            assert_eq!(lookup.value(g), Some(v));
        }
    }

    #[test]
    fn each_shape_takes_its_smallest_format_and_reads_back() {
        let runs: Vec<(u16, u16)> = (10..400).map(|g| (g, 1 + g / 100)).collect();
        let dense: Vec<(u16, u16)> = (10..400).map(|g| (g, g * 3)).collect();
        let pieces: Vec<(u16, u16)> = (0..40).flat_map(|r| (r * 100..r * 100 + 20).map(|g| (g, g ^ 0x55))).collect();
        let scattered: Vec<(u16, u16)> = (0..300).map(|i| (i * 7, i)).collect();
        for (entries, format) in [(runs, 2), (pieces, 4), (scattered, 6), (dense, 8), (Vec::new(), 8)] {
            let table = build_aat_lookup(&entries).expect("a lookup");
            assert_eq!(table[1], format);
            read_back(&table, &entries);
        }
    }

    // fontTools never writes format 4, so a lookup whose pieces each run with their own values falls
    // back to format 6, whose searchRange past 16,383 units does not fit in 16 bits.
    #[test]
    fn a_lookup_past_format_6s_header_still_reads_back() {
        let entries: Vec<(u16, u16)> = (0..2_000u16).flat_map(|r| (r * 12..r * 12 + 10).map(|g| (g, g))).collect();
        assert!(format6(&entries).is_none(), "format 6 wrote a searchRange past 16 bits");
        let table = build_aat_lookup(&entries).expect("a lookup");
        assert_eq!(table[1], 4);
        read_back(&table, &entries);
    }

    #[test]
    fn segments_end_with_the_terminator_the_spec_requires() {
        let table = format6(&[(3, 9), (5, 1)]).expect("format 6");
        assert_eq!(&table[table.len() - 4..], [0xFF, 0xFF, 0, 0]);
        assert_eq!(&table[2..12], [0, 4, 0, 2, 0, 8, 0, 1, 0, 0]);
        let table = format2(&[(3, 9), (4, 9)]).expect("format 2");
        assert_eq!(&table[2..12], [0, 6, 0, 2, 0, 12, 0, 1, 0, 0]);
        assert_eq!(&table[table.len() - 6..], [0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
    }

    #[test]
    fn an_offset_lookup_is_sized_by_its_glyphs_alone() {
        let glyphs: Vec<(u16, u16)> = (0..50).map(|i| (i * 2, 0)).collect();
        let valued: Vec<(u16, u16)> = glyphs.iter().map(|&(g, _)| (g, 1_000 + g)).collect();
        assert_eq!(build_aat_offset_lookup(&glyphs).map(|t| t.len()), build_aat_offset_lookup(&valued).map(|t| t.len()));
    }
}
