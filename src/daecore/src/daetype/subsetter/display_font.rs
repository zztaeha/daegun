use alloc::collections::BTreeSet;
use alloc::string::ToString;
use alloc::vec::Vec;
use super::super::decoder::{
    bin_search_header, encode_name, lang_tags, name_records, read_u16_be, read_u32_be, write_name_table,
    write_u16_be, NameKey,
};
use super::super::stat::has_data;
use super::cmap::{
    cmap_is_symbol, sanitizes, symbol_code, within, CharMap, CharTables, CmapLookup, Sequence, Sequences,
};
use super::os2_ranges::prune_unicode_ranges;
use super::otl::FeatureParams;

pub(crate) fn is_variation_selector(c: u32) -> bool {
    matches!(c, 0x180B..=0x180D | 0x180F | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

// A source's mappings and the sequences it answers, DMAP's before cmap's, which glyph subsets filter: one read serves
// every subset of the source. Variation subtables past their bound count as listing nothing; `room`, the two tables'
// subset budget, bounds what a subset's sequences may take too.
#[derive(Default)]
pub(crate) struct CmapEntries {
    mappings: Vec<(u32, u32)>,
    sequences: Sequences,
    room: usize,
}

impl CmapEntries {
    pub(crate) fn read(tables: CharTables) -> CmapEntries {
        let map = CharMap::new(tables);
        CmapEntries {
            mappings: map.entries(tables),
            sequences: map.sequences(tables).unwrap_or_default(),
            room: super::subset_budget(tables.cmap.len() + tables.dmap.len()),
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.mappings) + heap_of(&self.sequences.defaults) + heap_of(&self.sequences.glyphs)
    }

    // Every mapping and sequence whose glyph the subset keeps, renumbered; a default sequence stays while its base
    // is mapped. Mapped codes can split each default range, so past `room` ranges the subset keeps no sequence.
    pub(crate) fn kept(&self, new_gid: impl Fn(u32) -> Option<u32>) -> (Vec<(u32, u32)>, Sequences) {
        let mappings: Vec<(u32, u32)> = self.mappings.iter().filter_map(|&(cp, g)| Some((cp, new_gid(g)?))).collect();
        let runs: Vec<(u32, u32)> =
            mappings.chunk_by(|a, b| b.0 == a.0 + 1).map(|run| (run[0].0, run[run.len() - 1].0)).collect();
        let mut sequences = Sequences::default();
        let mut room = self.room;
        for selector in self.sequences.selectors() {
            let defaults: Vec<(u32, u32)> = self.sequences.defaults_of(selector).collect();
            if within(&defaults, &runs, selector, &mut room, &mut sequences.defaults).is_none() {
                return (mappings, Sequences::default());
            }
        }
        sequences.glyphs =
            self.sequences.glyphs.iter().filter_map(|&(vs, base, g)| Some((vs, base, new_gid(g)?))).collect();
        (mappings, sequences)
    }
}

// What a text subset's cmap holds, in the source's glyph ids.
#[derive(Clone, Copy)]
pub struct TextCmap<'a> {
    pub mappings: &'a [(u32, u32)],
    pub sequences: &'a [Sequence],
}

// Text's mappings and sequences in the subset's glyph ids, those it lost left out.
pub(crate) fn remap_text(text: TextCmap, new_gid: impl Fn(u32) -> Option<u32>) -> (Vec<(u32, u32)>, Sequences) {
    let TextCmap { mappings, sequences } = text;
    let mappings = mappings.iter().filter_map(|&(cp, g)| Some((cp, new_gid(g)?))).collect();
    let sequences = sequences.iter()
        .filter_map(|&(vs, base, g)| Some((vs, base, match g { Some(g) => Some(new_gid(g)?), None => None })));
    (mappings, Sequences::from_listed(sequences))
}

pub(crate) struct DisplayTables {
    pub cmap: Vec<u8>,
    pub name: Vec<u8>,
    pub os2: Option<Vec<u8>>,
    pub dmap: Option<Vec<u8>>,
}

// A subset's cmap (`mappings` and `sequences` in the source's encoding), name (the source's, or a minimal one) and
// OS/2, which follows the cmap; and a DMAP, 4 bytes where nothing is left, if the source's reads.
pub(crate) fn display_tables<'a>(
    source: CharTables, mappings: &[(u32, u32)], sequences: &Sequences, name: Option<&[u8]>, os2: Option<&[u8]>,
    subset: impl Fn(&str) -> Option<&'a [u8]>, mut names: BTreeSet<u16>,
) -> DisplayTables {
    let symbol = cmap_is_symbol(source.cmap) && CmapLookup::new(source.cmap).symbol();
    let cmap = build_cmap(mappings, sequences, symbol);
    let dmap = sanitizes(source.dmap).then(|| {
        let dmap = CmapLookup::new(source.dmap);
        let own: Vec<(u32, u32)> =
            mappings.iter().copied().filter(|&(cp, _)| dmap.glyph_id(source.dmap, cp).is_some()).collect();
        // DMAP's searches are held as the source's were, and the sequences' ranges besides; past them it keeps none.
        let mut work = super::subset_budget(source.cmap.len() + source.dmap.len()).saturating_add(sequences.len());
        let own_sequences = sequences.answered_by(&dmap, source.dmap, &mut work).unwrap_or_default();
        match own.is_empty() && own_sequences.is_empty() {
            true => alloc::vec![0; 4],
            false => build_cmap(&own, &own_sequences, false),
        }
    });
    names.extend(names_in_use(subset));
    let name = name.and_then(|n| kept_names(n, &names)).unwrap_or_else(|| build_name_table(os2));
    let os2 = os2.filter(|t| t.len() >= 68).map(|t| {
        let mut t = t.to_vec();
        let codes = mappings.iter().map(|&(cp, _)| cp);
        let char_index = |cp: Option<u32>| cp.map_or(0xFFFF, |cp| cp.min(0xFFFF) as u16);
        write_u16_be(&mut t, 64, char_index(codes.clone().min()));
        write_u16_be(&mut t, 66, char_index(codes.clone().max()));
        prune_unicode_ranges(&mut t, codes);
        t
    });
    DisplayTables { cmap, name, os2, dmap }
}

// The name IDs a subset's layout tables and STAT refer to: GSUB and GPOS feature parameters (stylistic set and
// character variant labels, the size feature's subfamily), and STAT's.
fn names_in_use<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> BTreeSet<u16> {
    let mut ids = BTreeSet::new();
    // Character variants' label ranges, first to last: many records may name one feature, so each label is added once.
    let mut labels: Vec<(u16, u16)> = Vec::new();
    for layout in ["GSUB", "GPOS"].into_iter().filter_map(&table) {
        let Some(([_, list, _], _)) = super::otl::layout_lists(layout) else { continue };
        for f in 0..read_u16_be(layout, list).map_or(0, usize::from) {
            let rec = list + 2 + f * 6;
            let (Some(tag), Some(feature)) = (layout.get(rec..rec + 4), read_u16_be(layout, rec + 4)) else { break };
            let feature = list + usize::from(feature);
            let Some(params) = read_u16_be(layout, feature).filter(|&p| p != 0).map(|p| feature + usize::from(p))
            else {
                continue;
            };
            let at = |k: usize| read_u16_be(layout, params + k);
            match FeatureParams::of(tag) {
                Some(FeatureParams::Size) => ids.extend(at(4)),
                Some(FeatureParams::StylisticSet) => ids.extend(at(2)),
                Some(FeatureParams::CharacterVariant) => {
                    ids.extend([at(2), at(4), at(6)].into_iter().flatten());
                    if let (Some(n @ 1..), Some(first)) = (at(8), at(10)) {
                        labels.push((first, first.saturating_add(n - 1)));
                    }
                }
                None => {}
            }
        }
    }
    labels.sort_unstable();
    let mut next = 0;
    for (first, last) in labels {
        ids.extend((u32::from(first).max(next)..=u32::from(last)).map(|id| id as u16));
        next = next.max(u32::from(last) + 1);
    }
    // Every design axis's, each axis value's the instance kept, and the elided fallback, its axis records
    // designAxisSize apart, as the STAT chapter lays them out; one narrower than a record holds none.
    if let Some(stat) = table("STAT").filter(|stat| has_data(stat)) {
        let count = |at: usize| read_u16_be(stat, at).map_or(0, usize::from);
        let offset = |at: usize| read_u32_be(stat, at).map_or(0, |v| v as usize);
        let axes = if count(4) < 8 { 0 } else { count(6) };
        ids.extend((0..axes).filter_map(|i| read_u16_be(stat, offset(8).saturating_add(count(4) * i + 4))));
        let values = offset(14);
        for i in 0..count(12) {
            let rel = read_u16_be(stat, values.saturating_add(2 * i));
            let Some(at) = rel.map(|rel| values + usize::from(rel)) else { continue };
            if matches!(read_u16_be(stat, at), Some(1..=4)) {
                ids.extend(read_u16_be(stat, at + 6));
            }
        }
        ids.extend(read_u16_be(stat, 18));
    }
    ids
}

// Names 0 to 6, the 16 and 17 the name chapter has fonts carry beside 1, 2 and 4, their WWS twins 21 and 22, and those
// the subset's tables refer to. A Mac record goes where a Unicode or Windows one holds its name ID.
fn kept_names(name: &[u8], in_use: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let records: Vec<(NameKey, &[u8])> = name_records(name).collect();
    let unicode: BTreeSet<u16> =
        records.iter().filter(|(key, _)| key.platform != 1).map(|(key, _)| key.name_id).collect();
    let kept: Vec<(NameKey, &[u8])> = records
        .into_iter()
        .filter(|(key, _)| {
            let mac_twin = key.platform == 1 && unicode.contains(&key.name_id);
            (matches!(key.name_id, 0..=6 | 16 | 17 | 21 | 22) || in_use.contains(&key.name_id)) && !mac_twin
        })
        .collect();
    if kept.is_empty() { return None; }
    write_name_table(kept, &lang_tags(name))
}

// Format 4 for the BMP, 12 or 13 past it or where 4 overflows, 14 for sequences, 15 where a glyph takes 24 bits. A
// `symbol` font's cmap stays one (3,0) subtable while it fits format 4, a byte code's U+F0xx answering for it.
pub fn build_cmap(mappings: &[(u32, u32)], sequences: &Sequences, symbol: bool) -> Vec<u8> {
    let mut sorted: Vec<(u32, u32)> = mappings.to_vec();
    sorted.sort_by_key(|&(cp, _)| cp);
    sorted.dedup_by_key(|&mut (cp, _)| cp);
    let uvs = (!sequences.is_empty()).then(|| variations(sequences));
    let wide_glyph = sorted.iter().any(|&(_, g)| g > 0xFFFF);

    if symbol && sorted.last().is_none_or(|&(cp, _)| cp <= 0xFFFF) && !wide_glyph {
        let glyph = |cp: u32| sorted.binary_search_by_key(&cp, |&(c, _)| c).ok().map(|i| sorted[i].1);
        let remapped_alike = |&(cp, g): &(u32, u32)| symbol_code(cp).is_some_and(|moved| glyph(moved) == Some(g));
        let own: Vec<(u32, u32)> = sorted.iter().copied().filter(|m| !remapped_alike(m)).collect();
        if let Some(own) = format4(&own) {
            let mut records: Vec<((u16, u16), &[u8])> = uvs.iter().map(|u| ((0, 5), u.as_slice())).collect();
            records.push(((3, 0), &own));
            return cmap_of(&records);
        }
    }
    let bmp = format4(&sorted);
    let past_bmp = sorted.last().is_some_and(|&(cp, _)| cp > 0xFFFF);
    let wide = (bmp.is_none() || past_bmp || wide_glyph).then(|| wide_subtable(&sorted));
    let mut records: Vec<((u16, u16), &[u8])> = Vec::new();
    if let Some(b) = &bmp { records.extend([((0, 3), b.as_slice()), ((3, 1), b.as_slice())]); }
    if let Some((unicode_id, w)) = &wide {
        records.extend([((0, *unicode_id), w.as_slice()), ((3, 10), w.as_slice())]);
    }
    if let Some(u) = &uvs { records.push(((0, 5), u.as_slice())); }
    // The Unicode and Windows platforms each name a subtable for their own encodings, 8 bytes a record, one copy.
    records.sort_by_key(|&(record, _)| record);
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

// Each run of consecutive codes as delta segments or one glyph array, whichever is smaller, and the final U+FFFF
// segment. None past 64 KB. A glyph past 65,535 is left to format 12, not cut to its low 16 bits here.
fn format4(sorted: &[(u32, u32)]) -> Option<Vec<u8>> {
    struct Seg { start: u16, end: u16, delta: u16, glyphs: Option<Vec<u16>> }
    let narrow = |&(cp, g): &(u32, u32)| Some((u16::try_from(cp).ok()?, u16::try_from(g).ok()?));
    let bmp: Vec<(u16, u16)> = sorted.iter().filter_map(narrow).filter(|&(cp, _)| cp < 0xFFFF).collect();
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
            let glyphs = Some(run.iter().map(|&(_, g)| g).collect());
            segs.push(Seg { start: run[0].0, end: run[run.len() - 1].0, delta: 0, glyphs });
        }
        i = j;
    }
    let last = sorted.last().and_then(narrow).filter(|&(cp, _)| cp == 0xFFFF);
    let last = last.map_or(1, |(_, g)| g.wrapping_sub(0xFFFF));
    segs.push(Seg { start: 0xFFFF, end: 0xFFFF, delta: last, glyphs: None });

    let n = segs.len();
    let array_len: usize = segs.iter().filter_map(|s| s.glyphs.as_ref()).map(Vec::len).sum();
    let len = 16 + 8 * n + 2 * array_len;
    if len > usize::from(u16::MAX) {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    for v in [4, len, 0, 2 * n].into_iter().chain(bin_search_header(n, 2)) {
        out.extend_from_slice(&(v as u16).to_be_bytes());
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

// Format 12 or 13, whichever takes fewer groups, as a fallback font mapping all of Unicode to a few glyphs needs few
// format 13 groups. The BMP is included, to agree with the BMP subtable, and the Unicode encoding comes back too.
fn wide_subtable(sorted: &[(u32, u32)]) -> (u16, Vec<u8>) {
    let groups = |step: u32| {
        let mut groups: Vec<(u32, u32, u32)> = Vec::new();
        for &(cp, gid) in sorted {
            match groups.last_mut() {
                Some(g) if cp == g.1 + 1 && gid == g.2 + step * (g.1 - g.0 + 1) => g.1 = cp,
                _ => groups.push((cp, cp, gid)),
            }
        }
        groups
    };
    let (twelve, thirteen) = (groups(1), groups(0));
    let (format, unicode_id, groups) =
        if thirteen.len() < twelve.len() { (13u16, 6, thirteen) } else { (12, 4, twelve) };
    let len = 16 + groups.len() * 12;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    for v in [len as u32, 0, groups.len() as u32] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (start, end, gid) in groups {
        for v in [start, end, gid] {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    (unicode_id, out)
}

// One record per selector, its default bases as ranges of up to 256 and its other sequences as mappings: format 14,
// or 15 where a glyph takes 24 bits.
fn variations(sequences: &Sequences) -> Vec<u8> {
    let u24 = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
    let wide = sequences.glyphs.iter().any(|&(_, _, g)| g > 0xFFFF);
    let selectors = sequences.selectors();
    let mut tables: Vec<u8> = Vec::new();
    let mut records: Vec<u8> = Vec::new();
    let header = 10 + 11 * selectors.len();
    for vs in selectors {
        let chunks = |(first, last): (u32, u32)| (first..=last).step_by(256).map(move |at| (at, (last - at).min(255)));
        let ranges: Vec<(u32, u32)> = sequences.defaults_of(vs).flat_map(chunks).collect();
        let from = sequences.glyphs.partition_point(|&(s, ..)| s < vs);
        let mappings: Vec<(u32, u32)> =
            sequences.glyphs[from..].iter().take_while(|&&(s, ..)| s == vs).map(|&(_, base, g)| (base, g)).collect();
        records.extend(u24(vs));
        for (count, entry) in [(ranges.len(), 4), (mappings.len(), 5 + usize::from(wide))] {
            if count == 0 {
                records.extend_from_slice(&0u32.to_be_bytes());
                continue;
            }
            records.extend_from_slice(&((header + tables.len()) as u32).to_be_bytes());
            tables.extend_from_slice(&(count as u32).to_be_bytes());
            if entry == 4 {
                ranges.iter().for_each(|&(start, more)| { tables.extend(u24(start)); tables.push(more as u8); });
            } else {
                for &(base, g) in &mappings {
                    tables.extend(u24(base));
                    tables.extend_from_slice(&g.to_be_bytes()[if wide { 1 } else { 2 }..]);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(header + tables.len());
    out.extend_from_slice(&if wide { 15u16 } else { 14 }.to_be_bytes());
    out.extend_from_slice(&((header + tables.len()) as u32).to_be_bytes());
    out.extend_from_slice(&((records.len() / 11) as u32).to_be_bytes());
    out.extend(records);
    out.extend(tables);
    out
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
    let names = [
        (1, family.to_string()),
        (2, style.to_string()),
        (3, format!("{ps_name};daegun-subset")),
        (4, format!("{family} {style}")),
        (6, ps_name),
    ];
    let encoded: Vec<(NameKey, Vec<u8>)> = names
        .into_iter()
        .filter_map(|(name_id, s)| {
            let key = NameKey { platform: 3, encoding: 1, language: 0x0409, name_id };
            Some((key, encode_name(key, &s)?))
        })
        .collect();
    write_name_table(encoded.iter().map(|(key, s)| (*key, s.as_slice())).collect(), &[]).unwrap_or_default()
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/display_font.rs"]
mod tests;
