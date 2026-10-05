use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be, write_u16_be};
use super::cmap::{cmap_entries_kept, cmap_is_symbol, cmap_variation_sequences, Sequence};

pub(crate) fn is_variation_selector(c: u32) -> bool {
    matches!(c, 0x180B..=0x180D | 0x180F | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

// Every mapping and sequence of a source cmap whose glyph the subset keeps, renumbered; a default
// sequence stays while its base is mapped. A cmap past the cap counts as mapping nothing.
pub(crate) fn kept_cmap(cmap: &[u8], new_gid: impl Fn(u16) -> Option<u16>) -> (Vec<(u32, u16)>, Vec<Sequence>) {
    const CAP: usize = 200_000;
    let mappings: Vec<(u32, u16)> = cmap_entries_kept(cmap, CAP, |g| new_gid(g).is_some()).unwrap_or_default()
        .into_iter().filter_map(|(cp, g)| Some((cp, new_gid(g)?))).collect();
    let mapped = |cp: u32| mappings.binary_search_by_key(&cp, |&(c, _)| c).is_ok();
    let kept = |&(_, base, glyph): &Sequence| glyph.map_or_else(|| mapped(base), |g| new_gid(g).is_some());
    let sequences = cmap_variation_sequences(cmap, CAP, kept).unwrap_or_default().into_iter()
        .map(|(vs, base, glyph)| (vs, base, glyph.and_then(&new_gid)))
        .collect();
    (mappings, sequences)
}

// What a text subset's cmap holds, in the source's glyph ids.
#[derive(Clone, Copy)]
pub struct TextCmap<'a> {
    pub mappings: &'a [(u32, u16)],
    pub sequences: &'a [Sequence],
}

// Text's mappings and sequences in the subset's glyph ids, those it lost left out.
pub(crate) fn remap_text(text: TextCmap, new_gid: impl Fn(u16) -> Option<u16>) -> (Vec<(u32, u16)>, Vec<Sequence>) {
    let TextCmap { mappings, sequences } = text;
    let mappings = mappings.iter().filter_map(|&(cp, g)| Some((cp, new_gid(g)?))).collect();
    let sequences = sequences.iter()
        .filter_map(|&(vs, base, g)| Some((vs, base, match g { Some(g) => Some(new_gid(g)?), None => None })))
        .collect();
    (mappings, sequences)
}

// A subset's cmap (`mappings` and `sequences` in the source's encoding), name (the source's, or a
// minimal one where it has none) and OS/2, whose first and last characters follow the cmap.
pub(crate) fn display_tables<'a>(
    source_cmap: Option<&[u8]>, mappings: &[(u32, u16)], sequences: &[Sequence], name: Option<&[u8]>, os2: Option<&[u8]>,
    subset: impl Fn(&str) -> Option<&'a [u8]>,
) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>) {
    let cmap = build_cmap(mappings, sequences, source_cmap.is_some_and(cmap_is_symbol));
    let name = name.and_then(|n| kept_names(n, &names_in_use(subset))).unwrap_or_else(|| build_name_table(os2));
    let os2 = os2.filter(|t| t.len() >= 68).map(|t| {
        let mut t = t.to_vec();
        let codes = mappings.iter().map(|&(cp, _)| cp);
        if let (Some(first), Some(last)) = (codes.clone().min(), codes.max()) {
            write_u16_be(&mut t, 64, first.min(0xFFFF) as u16);
            write_u16_be(&mut t, 66, last.min(0xFFFF) as u16);
        }
        t
    });
    (cmap, name, os2)
}

// The name IDs a subset's tables refer to: GSUB and GPOS feature parameters (stylistic set and
// character variant labels, the size feature's subfamily) and CPAL's palette and entry labels.
fn names_in_use<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> BTreeSet<u16> {
    let mut ids = BTreeSet::new();
    for layout in ["GSUB", "GPOS"].into_iter().filter_map(&table) {
        let Some(list) = read_u16_be(layout, 6).map(usize::from) else { continue };
        for f in 0..read_u16_be(layout, list).map_or(0, usize::from) {
            let rec = list + 2 + f * 6;
            let (Some(tag), Some(feature)) = (layout.get(rec..rec + 4), read_u16_be(layout, rec + 4)) else { break };
            let feature = list + usize::from(feature);
            let Some(params) = read_u16_be(layout, feature).filter(|&p| p != 0).map(|p| feature + usize::from(p)) else { continue };
            let at = |k: usize| read_u16_be(layout, params + k);
            match tag {
                b"size" => ids.extend(at(4)),
                [b's', b's', ..] => ids.extend(at(2)),
                [b'c', b'v', ..] => {
                    ids.extend([at(2), at(4), at(6)].into_iter().flatten());
                    if let (Some(n), Some(first)) = (at(8), at(10)) {
                        ids.extend((0..n).map_while(|k| first.checked_add(k)));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(cpal) = table("CPAL").filter(|c| read_u16_be(c, 0).is_some_and(|v| v >= 1)) {
        let (entries, palettes) = (read_u16_be(cpal, 2).unwrap_or(0), read_u16_be(cpal, 4).unwrap_or(0));
        let tail = 12 + 2 * usize::from(palettes);
        for (at, count) in [(tail + 4, palettes), (tail + 8, entries)] {
            let Some(labels) = read_u32_be(cpal, at).filter(|&o| o != 0) else { continue };
            ids.extend((0..usize::from(count)).filter_map(|k| read_u16_be(cpal, labels as usize + 2 * k)).filter(|&id| id != 0xFFFF));
        }
    }
    ids
}

// Names 0 to 6, as fontTools keeps by default, and those the subset's tables refer to. A Mac record
// goes where a Unicode or Windows one holds the same name, and stays where it is the only one.
fn kept_names<'n>(name: &'n [u8], in_use: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let format = read_u16_be(name, 0)?;
    let count = usize::from(read_u16_be(name, 2)?);
    let storage = usize::from(read_u16_be(name, 4)?);
    let string = |len: usize, off: usize| name.get(storage + off..storage + off + len);
    let record = |i: usize| name.get(6 + i * 12..18 + i * 12);
    let platform = |rec: &[u8]| u16::from_be_bytes([rec[0], rec[1]]);
    let id_of = |rec: &[u8]| u16::from_be_bytes([rec[6], rec[7]]);
    let unicode: BTreeSet<u16> = (0..count).map_while(record).filter(|r| platform(r) != 1).map(id_of).collect();
    let mut records: Vec<([u8; 8], &[u8])> = Vec::new();
    for i in 0..count {
        let rec = record(i)?;
        let id = id_of(rec);
        if (id <= 6 || in_use.contains(&id)) && !(platform(rec) == 1 && unicode.contains(&id)) {
            let (len, off) = (u16::from_be_bytes([rec[8], rec[9]]), u16::from_be_bytes([rec[10], rec[11]]));
            records.push((rec[..8].try_into().ok()?, string(usize::from(len), usize::from(off))?));
        }
    }
    let mut tags: Vec<&[u8]> = Vec::new();
    if format == 1 {
        let at = 6 + count * 12;
        for k in 0..usize::from(read_u16_be(name, at)?) {
            let (len, off) = (read_u16_be(name, at + 2 + k * 4)?, read_u16_be(name, at + 4 + k * 4)?);
            tags.push(string(usize::from(len), usize::from(off))?);
        }
    }
    if records.is_empty() { return None; }

    let header = 6 + records.len() * 12 + if format == 1 { 2 + tags.len() * 4 } else { 0 };
    let mut out = Vec::with_capacity(header);
    for v in [format, records.len() as u16, u16::try_from(header).ok()?] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    // Each distinct string is stored once; past 64 KB of them the table cannot be addressed.
    let mut strings: Vec<u8> = Vec::new();
    let mut placed: BTreeMap<&'n [u8], u16> = BTreeMap::new();
    let mut place = |bytes: &'n [u8]| -> Option<[u8; 4]> {
        let off = match placed.get(bytes) {
            Some(&off) => off,
            None => {
                let off = u16::try_from(strings.len()).ok()?;
                strings.extend_from_slice(bytes);
                off
            }
        };
        placed.insert(bytes, off);
        let len = u16::try_from(bytes.len()).ok()?;
        let [a, b] = len.to_be_bytes();
        let [c, d] = off.to_be_bytes();
        Some([a, b, c, d])
    };
    for (head, bytes) in &records {
        out.extend_from_slice(head);
        out.extend_from_slice(&place(bytes)?);
    }
    if format == 1 {
        out.extend_from_slice(&(tags.len() as u16).to_be_bytes());
        for bytes in &tags {
            out.extend_from_slice(&place(bytes)?);
        }
    }
    out.extend(strings);
    Some(out)
}

// Format 4 for the BMP, format 12 once a code lies past it or format 4 overflows, format 14 for the
// sequences, records sorted. A symbol font keeps one (3,0) subtable, its codes 0x20 to 0xFF at U+F0xx.
pub fn build_cmap(mappings: &[(u32, u16)], sequences: &[Sequence], symbol: bool) -> Vec<u8> {
    let mut sorted: Vec<(u32, u16)> = mappings.iter()
        .map(|&(cp, g)| if symbol && cp <= 0xFF { (0xF000 + cp, g) } else { (cp, g) })
        .collect();
    sorted.sort_by_key(|&(cp, _)| cp);
    sorted.dedup_by_key(|&mut (cp, _)| cp);

    let bmp = format4(&sorted);
    if symbol && let Some(bmp) = &bmp && sorted.last().is_none_or(|&(cp, _)| cp <= 0xFFFF) {
        return cmap_of(&[((3, 0), bmp)]);
    }
    let wide = (bmp.is_none() || sorted.last().is_some_and(|&(cp, _)| cp > 0xFFFF)).then(|| format12(&sorted));
    let uvs = (!sequences.is_empty()).then(|| format14(sequences));
    let mut records: Vec<((u16, u16), &[u8])> = Vec::new();
    for (platform, bmp_id, wide_id) in [(0, 3, 4), (3, 1, 10)] {
        if let Some(b) = &bmp { records.push(((platform, bmp_id), b)); }
        if let Some(w) = &wide { records.push(((platform, wide_id), w)); }
        if platform == 0 && let Some(u) = &uvs { records.push(((0, 5), u)); }
    }
    cmap_of(&records)
}

// The cmap header and its subtables, one copy of each however many records name it.
fn cmap_of<B: AsRef<[u8]>>(records: &[((u16, u16), B)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(records.len() as u16).to_be_bytes());
    let mut body: Vec<u8> = Vec::new();
    let mut placed: Vec<(&[u8], u32)> = Vec::new();
    let header_len = 4 + 8 * records.len();
    for ((platform, encoding), sub) in records {
        let sub = sub.as_ref();
        let off = match placed.iter().find(|(b, _)| *b == sub) {
            Some(&(_, off)) => off,
            None => {
                let off = (header_len + body.len()) as u32;
                body.extend_from_slice(sub);
                placed.push((sub, off));
                off
            }
        };
        out.extend_from_slice(&platform.to_be_bytes());
        out.extend_from_slice(&encoding.to_be_bytes());
        out.extend_from_slice(&off.to_be_bytes());
    }
    out.extend(body);
    out
}

// Each run of consecutive codes as delta segments or one glyph array, whichever is smaller, and U+FFFF
// in the final segment the spec requires. None when the subtable would pass 64 KB.
fn format4(sorted: &[(u32, u16)]) -> Option<Vec<u8>> {
    struct Seg { start: u16, end: u16, delta: u16, glyphs: Option<Vec<u16>> }
    let bmp: Vec<(u16, u16)> = sorted.iter().filter(|&&(cp, _)| cp < 0xFFFF).map(|&(cp, g)| (cp as u16, g)).collect();
    let mut segs: Vec<Seg> = Vec::new();
    let mut i = 0;
    while i < bmp.len() {
        let mut j = i + 1;
        while j < bmp.len() && bmp[j].0 == bmp[j - 1].0 + 1 { j += 1; }
        let run = &bmp[i..j];
        let delta = |&(cp, g): &(u16, u16)| g.wrapping_sub(cp);
        let pieces = 1 + run.windows(2).filter(|w| delta(&w[0]) != delta(&w[1])).count();
        if pieces == 1 || pieces * 8 <= 8 + 2 * run.len() {
            let mut k = 0;
            while k < run.len() {
                let mut m = k + 1;
                while m < run.len() && delta(&run[m]) == delta(&run[k]) { m += 1; }
                segs.push(Seg { start: run[k].0, end: run[m - 1].0, delta: delta(&run[k]), glyphs: None });
                k = m;
            }
        } else {
            segs.push(Seg { start: run[0].0, end: run[run.len() - 1].0, delta: 0, glyphs: Some(run.iter().map(|&(_, g)| g).collect()) });
        }
        i = j;
    }
    let last = sorted.last().filter(|&&(cp, _)| cp == 0xFFFF).map_or(1, |&(_, g)| g.wrapping_sub(0xFFFF));
    segs.push(Seg { start: 0xFFFF, end: 0xFFFF, delta: last, glyphs: None });

    let n = segs.len();
    let array_len: usize = segs.iter().filter_map(|s| s.glyphs.as_ref()).map(Vec::len).sum();
    let len = 16 + 8 * n + 2 * array_len;
    let length = u16::try_from(len).ok()?;
    let entry_selector = n.ilog2();
    let search_range = 2u16 << entry_selector;
    let mut out = Vec::with_capacity(len);
    for v in [4, length, 0, (2 * n) as u16, search_range, entry_selector as u16, (2 * n) as u16 - search_range] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    segs.iter().for_each(|s| out.extend_from_slice(&s.end.to_be_bytes()));
    out.extend_from_slice(&[0, 0]);
    segs.iter().for_each(|s| out.extend_from_slice(&s.start.to_be_bytes()));
    segs.iter().for_each(|s| out.extend_from_slice(&s.delta.to_be_bytes()));
    // An idRangeOffset counts from its own slot to the segment's first entry in the glyph array.
    let mut array_at = 0usize;
    for (k, s) in segs.iter().enumerate() {
        let offset = match &s.glyphs {
            Some(g) => { let at = 2 * (n - k) + 2 * array_at; array_at += g.len(); at }
            None => 0,
        };
        out.extend_from_slice(&u16::try_from(offset).ok()?.to_be_bytes());
    }
    segs.iter().filter_map(|s| s.glyphs.as_ref()).flatten().for_each(|g| out.extend_from_slice(&g.to_be_bytes()));
    Some(out)
}

// Groups over runs where code and glyph advance together, BMP included, so the subtables agree
// wherever they overlap.
fn format12(sorted: &[(u32, u16)]) -> Vec<u8> {
    let mut groups: Vec<(u32, u32, u16)> = Vec::new();
    for &(cp, gid) in sorted {
        match groups.last_mut() {
            Some(g) if cp == g.1 + 1 && u32::from(gid) == u32::from(g.2) + (g.1 - g.0) + 1 => g.1 = cp,
            _ => groups.push((cp, cp, gid)),
        }
    }
    let len = 16 + groups.len() * 12;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&12u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    for v in [len as u32, 0, groups.len() as u32] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (start, end, gid) in groups {
        for v in [start, end, u32::from(gid)] {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    out
}

// One record per selector, its default bases as ranges and its other sequences as mappings.
fn format14(sequences: &[Sequence]) -> Vec<u8> {
    // A sequence listed both ways keeps its own glyph, which a lookup finds first.
    let mut sorted = sequences.to_vec();
    sorted.sort_unstable_by_key(|&(vs, base, glyph)| (vs, base, glyph.is_none(), glyph));
    sorted.dedup_by_key(|&mut (vs, base, _)| (vs, base));
    let mut selectors: Vec<u32> = sorted.iter().map(|&(vs, _, _)| vs).collect();
    selectors.dedup();

    let u24 = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
    let mut tables: Vec<u8> = Vec::new();
    let mut records: Vec<u8> = Vec::new();
    let header = 10 + 11 * selectors.len();
    for vs in selectors {
        let of = sorted.iter().filter(|&&(v, _, _)| v == vs);
        let mut ranges: Vec<(u32, u8)> = Vec::new();
        for base in of.clone().filter(|s| s.2.is_none()).map(|s| s.1) {
            match ranges.last_mut() {
                Some((start, more)) if base == *start + u32::from(*more) + 1 && *more < 255 => *more += 1,
                _ => ranges.push((base, 0)),
            }
        }
        let mappings: Vec<(u32, u16)> = of.filter_map(|&(_, base, g)| Some((base, g?))).collect();
        records.extend(u24(vs));
        for (count, entry) in [(ranges.len(), 4), (mappings.len(), 5)] {
            if count == 0 {
                records.extend_from_slice(&0u32.to_be_bytes());
                continue;
            }
            records.extend_from_slice(&((header + tables.len()) as u32).to_be_bytes());
            tables.extend_from_slice(&(count as u32).to_be_bytes());
            if entry == 4 {
                ranges.iter().for_each(|&(start, more)| { tables.extend(u24(start)); tables.push(more); });
            } else {
                mappings.iter().for_each(|&(base, g)| { tables.extend(u24(base)); tables.extend_from_slice(&g.to_be_bytes()); });
            }
        }
    }
    let mut out = Vec::with_capacity(header + tables.len());
    out.extend_from_slice(&14u16.to_be_bytes());
    out.extend_from_slice(&((header + tables.len()) as u32).to_be_bytes());
    out.extend_from_slice(&((records.len() / 11) as u32).to_be_bytes());
    out.extend(records);
    out.extend(tables);
    out
}

fn utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

// For a source with no name table: a family of its own, styled as OS/2's fsSelection says.
fn build_name_table(os2: Option<&[u8]>) -> Vec<u8> {
    let selection = os2.and_then(|t| read_u16_be(t, 62)).unwrap_or(0);
    let style = match (selection & 0x20 != 0, selection & 0x01 != 0) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    };
    let family = "DaegunSubset";
    let ps_name = format!("{family}-{}", style.replace(' ', ""));
    let records: [(u16, String); 5] = [
        (1, family.to_string()),
        (2, style.to_string()),
        (3, format!("{ps_name};daegun-subset")),
        (4, format!("{family} {style}")),
        (6, ps_name),
    ];

    let strings: Vec<Vec<u8>> = records.iter().map(|(_, v)| utf16be(v)).collect();
    let header_len = 6;
    let record_len = 12;
    let string_storage_offset = header_len + records.len() * record_len;
    let mut buf = vec![0u8; string_storage_offset];
    write_u16_be(&mut buf, 2, records.len() as u16);
    write_u16_be(&mut buf, 4, string_storage_offset as u16);
    let mut str_off = 0usize;
    for (i, ((name_id, _), s)) in records.iter().zip(&strings).enumerate() {
        let off = header_len + i * record_len;
        for (k, v) in [3u16, 1, 0x0409, *name_id, s.len() as u16, str_off as u16].into_iter().enumerate() {
            write_u16_be(&mut buf, off + 2 * k, v);
        }
        str_off += s.len();
    }
    strings.iter().for_each(|s| buf.extend_from_slice(s));
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::cmap::{cmap_glyph_id, cmap_variation_glyph_id, UvsLookup};

    fn records(cmap: &[u8]) -> Vec<(u16, u16, u16)> {
        (0..usize::from(read_u16_be(cmap, 2).expect("count")))
            .map(|i| {
                let off = read_u32_be(cmap, 8 + 8 * i).expect("offset") as usize;
                (read_u16_be(cmap, 4 + 8 * i).expect("platform"), read_u16_be(cmap, 6 + 8 * i).expect("encoding"), read_u16_be(cmap, off).expect("format"))
            })
            .collect()
    }

    fn maps_all(cmap: &[u8], mappings: &[(u32, u16)]) {
        for &(cp, gid) in mappings {
            assert_eq!(cmap_glyph_id(cmap, cp), Some(gid), "U+{cp:X}");
        }
    }

    // Glyphs advancing with their codes share one segment; glyphs that do not share a glyph array;
    // codes too scattered for format 4 are left to format 12 alone.
    #[test]
    fn format_4_holds_runs_and_gives_way_to_format_12() {
        let runs: Vec<(u32, u16)> = (0x4E00..0x4E00 + 10_000).map(|cp| (cp, (cp - 0x4D00) as u16)).collect();
        let cmap = build_cmap(&runs, &[], false);
        assert!(cmap.len() < 100, "one run took {} bytes", cmap.len());
        maps_all(&cmap, &runs);
        let shuffled: Vec<(u32, u16)> = (0..9_000u32).map(|i| (0x4E00 + i, ((i * 7_919) % 9_000 + 1) as u16)).collect();
        let cmap = build_cmap(&shuffled, &[], false);
        assert_eq!(records(&cmap), [(0, 3, 4), (3, 1, 4)]);
        maps_all(&cmap, &shuffled);
        let scattered: Vec<(u32, u16)> = (0..9_000u32).map(|i| (0x100 + 3 * i, (i + 1) as u16)).collect();
        let cmap = build_cmap(&scattered, &[], false);
        assert_eq!(records(&cmap), [(0, 4, 12), (3, 10, 12)]);
        maps_all(&cmap, &scattered);
        assert_eq!(cmap_glyph_id(&cmap, 0x101), None);
    }

    // U+FFFF mapped lives in the final segment the spec requires, ending codes rising throughout.
    #[test]
    fn u_ffff_is_mapped_by_the_final_segment() {
        let mappings = [(0x41, 3), (0xFFFF, 5)];
        let cmap = build_cmap(&mappings, &[], false);
        maps_all(&cmap, &mappings);
        let sub = read_u32_be(&cmap, 8).expect("offset") as usize;
        let n = usize::from(read_u16_be(&cmap, sub + 6).expect("segCountX2")) / 2;
        let ends: Vec<u16> = (0..n).map(|i| read_u16_be(&cmap, sub + 14 + 2 * i).expect("endCode")).collect();
        assert!(ends.windows(2).all(|w| w[0] < w[1]) && ends.last() == Some(&0xFFFF), "{ends:x?}");
    }

    #[test]
    fn records_run_in_platform_and_encoding_order() {
        let cmap = build_cmap(&[(0x41, 1), (0x1D400, 2)], &[(0xFE00, 0x41, None)], false);
        assert_eq!(records(&cmap), [(0, 3, 4), (0, 4, 12), (0, 5, 14), (3, 1, 4), (3, 10, 12)]);
    }

    #[test]
    fn variation_sequences_read_back_as_written() {
        let sequences = [(0xFE00, 0x41, None), (0xFE00, 0x42, None), (0xFE00, 0x43, Some(9)), (0xE0100, 0x845B, Some(7))];
        let cmap = build_cmap(&[(0x41, 1), (0x42, 2), (0x43, 3), (0x845B, 4)], &sequences, false);
        assert!(matches!(cmap_variation_glyph_id(&cmap, 0x41, 0xFE00), Some(UvsLookup::UseDefault)));
        assert!(matches!(cmap_variation_glyph_id(&cmap, 0x43, 0xFE00), Some(UvsLookup::Explicit(9))));
        assert!(matches!(cmap_variation_glyph_id(&cmap, 0x845B, 0xE0100), Some(UvsLookup::Explicit(7))));
        assert!(cmap_variation_glyph_id(&cmap, 0x44, 0xFE00).is_none());
        assert_eq!(cmap_variation_sequences(&cmap, 100, |_| true), Some(sequences.to_vec()));
    }

    #[test]
    fn a_symbol_font_keeps_one_symbol_subtable() {
        let cmap = build_cmap(&[(0x41, 3), (0xF042, 4)], &[], true);
        assert_eq!(records(&cmap), [(3, 0, 4)]);
        maps_all(&cmap, &[(0xF041, 3), (0x41, 3), (0x42, 4)]);
    }

    fn name_table(records: &[(u16, u16, &str)]) -> Vec<u8> {
        let mut strings: Vec<u8> = Vec::new();
        let mut out = [0u16, records.len() as u16, 6 + 12 * records.len() as u16].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        for &(platform, id, text) in records {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            let (encoding, language) = if platform == 1 { (0, 0) } else { (1, 0x409) };
            for v in [platform, encoding, language, id, bytes.len() as u16, strings.len() as u16] { out.extend(v.to_be_bytes()); }
            strings.extend(bytes);
        }
        out.extend(strings);
        out
    }

    // ss01 names 300; cv01 names 301 to 303 and its two parameters 310 and 311; CPAL's palette 400
    // and its first entry 401, its second none.
    #[test]
    fn the_names_tables_refer_to_are_found() {
        let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let gsub = [u16s(&[1, 0, 10, 12, 0, 0, 2]), b"ss01".to_vec(), u16s(&[14]), b"cv01".to_vec(), u16s(&[22]),
                    u16s(&[4, 0, 0, 300, 4, 0, 0, 301, 302, 303, 2, 310])].concat();
        let cpal = u16s(&[1, 2, 1, 2, 0, 0, 0, 0, 0, 0, 26, 0, 28, 400, 401, 0xFFFF]);
        let ids = names_in_use(|tag| match tag { "GSUB" => Some(&gsub[..]), "CPAL" => Some(&cpal[..]), _ => None });
        assert_eq!(ids.into_iter().collect::<Vec<_>>(), [300, 301, 302, 303, 310, 311, 400, 401]);
    }

    // IDs 0 to 6 and those a kept table names stay; a Mac record goes where a Windows one says the same.
    #[test]
    fn a_subset_keeps_the_names_it_needs() {
        let name = name_table(&[(1, 1, "Fam"), (3, 1, "Fam"), (1, 9, "Designer"), (3, 13, "License"), (3, 300, "Swash set"), (1, 301, "Mac only")]);
        let kept = kept_names(&name, &[300, 301].into_iter().collect()).expect("a name table");
        let ids: Vec<(u16, u16)> = (0..usize::from(read_u16_be(&kept, 2).expect("count")))
            .map(|i| (read_u16_be(&kept, 6 + 12 * i).expect("platform"), read_u16_be(&kept, 12 + 12 * i).expect("id")))
            .collect();
        assert_eq!(ids, [(3, 1), (3, 300), (1, 301)]);
    }
}
