use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::super::decoder::{
    mac_roman_byte, mac_roman_char, mac_turkish_byte, mac_turkish_char,
    read_u16_be, read_u32_be, read_u24_be, search_records,
};

// Language 18 on a Macintosh subtable means MacOS Turkish rather than MacRoman.
const MAC_LANG_TURKISH: u16 = 18;

// What a subtable's codes stand for. Other records (Mac scripts besides Roman, Windows ShiftJIS
// through Johab, custom encodings) are not keyed by Unicode and are skipped.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Encoding {
    Unicode,
    // A Windows symbol font maps U+F020..U+F0FF, which also answers for the codes 0x20..0xFF.
    Symbol,
    MacRoman { turkish: bool },
}

struct Subtable {
    encoding: Encoding,
    format: u16,
    off: usize,
}

impl Subtable {
    // Lower tiers answer first: full Unicode, then BMP, then byte tables, then symbol and Mac.
    fn tier(&self) -> u8 {
        match (self.encoding, self.format) {
            (Encoding::Unicode, 10 | 12 | 13) => 0,
            (Encoding::Unicode, 4) => 1,
            (Encoding::Unicode, _) => 2,
            (Encoding::Symbol, _) => 3,
            (Encoding::MacRoman { .. }, _) => 4,
        }
    }

    fn glyph_id(&self, cmap: &[u8], codepoint: u32) -> Option<u16> {
        let at = |code| lookup(cmap, self.format, self.off, code);
        match self.encoding {
            Encoding::Unicode => at(codepoint),
            Encoding::Symbol => at(codepoint).or_else(|| if codepoint <= 0xFF { at(0xF000 + codepoint) } else { None }),
            Encoding::MacRoman { turkish } => {
                let byte = if turkish { mac_turkish_byte(codepoint) } else { mac_roman_byte(codepoint) };
                at(u32::from(byte?))
            }
        }
    }
}

fn subtables(cmap: &[u8]) -> impl Iterator<Item = Subtable> + '_ {
    let num_tables = read_u16_be(cmap, 2).map_or(0, usize::from);
    (0..num_tables)
        .map_while(move |i| {
            let rec = 4 + i * 8;
            Some((read_u16_be(cmap, rec)?, read_u16_be(cmap, rec + 2)?, read_u32_be(cmap, rec + 4)? as usize))
        })
        .filter_map(move |(platform, encoding, off)| {
            let format = read_u16_be(cmap, off)?;
            let encoding = match (platform, encoding) {
                (0 | 2, _) | (3, 1 | 10) => Encoding::Unicode,
                (3, 0) => Encoding::Symbol,
                (1, 0) => Encoding::MacRoman {
                    turkish: matches!(format, 0 | 4 | 6) && read_u16_be(cmap, off + 4) == Some(MAC_LANG_TURKISH),
                },
                _ => return None,
            };
            Some(Subtable { encoding, format, off })
        })
}

fn lookup(cmap: &[u8], format: u16, off: usize, code: u32) -> Option<u16> {
    match format {
        12 | 13 => segmented_lookup(cmap, off, code, format),
        10 => format10_lookup(cmap, off, code),
        4 => format4_lookup(cmap, off, u16::try_from(code).ok()?),
        6 => format6_lookup(cmap, off, u16::try_from(code).ok()?),
        0 => format0_lookup(cmap, off, u8::try_from(code).ok()?),
        _ => None,
    }
}

// The subtables a code is looked up in, best tier first and each once however many records name it,
// so the first to map a code answers for it. Built once, it serves every lookup in the cmap it read.
pub struct CmapLookup(Vec<Subtable>);

impl CmapLookup {
    pub fn new(cmap: &[u8]) -> CmapLookup {
        let mut seen = BTreeSet::new();
        let mut subtables: Vec<Subtable> = subtables(cmap).filter(|s| seen.insert((s.encoding, s.off))).collect();
        subtables.sort_by_key(Subtable::tier);
        CmapLookup(subtables)
    }

    pub fn glyph_id(&self, cmap: &[u8], codepoint: u32) -> Option<u16> {
        self.0.iter().find_map(|sub| sub.glyph_id(cmap, codepoint))
    }
}

pub fn cmap_glyph_id(cmap: &[u8], codepoint: u32) -> Option<u16> {
    CmapLookup::new(cmap).glyph_id(cmap, codepoint)
}

// Glyph 0 is .notdef, so every lookup reads it as unmapped and a later subtable can still answer.
fn notdef_is_miss(gid: u16) -> Option<u16> {
    if gid == 0 { None } else { Some(gid) }
}

fn format6_lookup(cmap: &[u8], base: usize, cp: u16) -> Option<u16> {
    let first = read_u16_be(cmap, base + 6)?;
    let count = read_u16_be(cmap, base + 8)?;
    if cp < first { return None; }
    let idx = (cp - first) as usize;
    if idx >= count as usize { return None; }
    read_u16_be(cmap, base + 10 + idx * 2).and_then(notdef_is_miss)
}

fn format10_lookup(cmap: &[u8], base: usize, codepoint: u32) -> Option<u16> {
    let first = read_u32_be(cmap, base + 12)?;
    let count = read_u32_be(cmap, base + 16)?;
    let idx = codepoint.checked_sub(first)?;
    if idx >= count {
        return None;
    }
    let at = base.checked_add(20)?.checked_add((idx as usize).checked_mul(2)?)?;
    read_u16_be(cmap, at).and_then(notdef_is_miss)
}

fn format0_lookup(cmap: &[u8], base: usize, cp: u8) -> Option<u16> {
    notdef_is_miss(cmap.get(base + 6 + cp as usize).copied().unwrap_or(0) as u16)
}

fn segmented_lookup(cmap: &[u8], base: usize, codepoint: u32, format: u16) -> Option<u16> {
    if base + 16 > cmap.len() { return None; }
    let num_groups  = read_u32_be(cmap, base + 12)? as usize;
    let groups_base = base + 16;
    if groups_base + num_groups * 12 > cmap.len() { return None; }

    let cand = match search_records(num_groups, codepoint, |i| read_u32_be(cmap, groups_base + i * 12)) {
        Some(Ok(i)) => i,
        Some(Err(0)) | None => return None,
        Some(Err(i)) => i - 1,
    };
    let off = groups_base + cand * 12;
    let (Some(start), Some(end)) = (read_u32_be(cmap, off), read_u32_be(cmap, off + 4)) else { return None };
    if codepoint < start || codepoint > end { return None; }
    let glyph = read_u32_be(cmap, off + 8)? as u64;
    let gid = if format == 13 { glyph } else { glyph + (codepoint - start) as u64 };
    if gid > 0xFFFF { None } else { notdef_is_miss(gid as u16) }
}

pub enum UvsLookup {
    Explicit(u16),
    UseDefault,
}

pub fn cmap_variation_glyph_id(cmap: &[u8], base: u32, selector: u32) -> Option<UvsLookup> {
    let base14 = find_format14_subtable(cmap)?;
    let (default_off, non_default_off) = find_var_selector_record(cmap, base14, selector)?;

    if non_default_off != 0
        && let Some(gid) = lookup_non_default_uvs(cmap, base14 + non_default_off as usize, base) {
            return Some(UvsLookup::Explicit(gid));
        }
    if default_off != 0 && lookup_default_uvs(cmap, base14 + default_off as usize, base) {
        return Some(UvsLookup::UseDefault);
    }
    None
}

fn find_format14_subtable(cmap: &[u8]) -> Option<usize> {
    if cmap.len() < 4 { return None; }
    let num_tables = read_u16_be(cmap, 2)? as usize;
    for i in 0..num_tables {
        let rec = 4 + i * 8;
        if rec + 8 > cmap.len() { break; }
        let platform_id = read_u16_be(cmap, rec)?;
        let encoding_id = read_u16_be(cmap, rec + 2)?;
        if platform_id != 0 || encoding_id != 5 { continue; }
        let off = read_u32_be(cmap, rec + 4)? as usize;
        if off + 2 <= cmap.len() && read_u16_be(cmap, off) == Some(14) {
            return Some(off);
        }
    }
    None
}

fn find_var_selector_record(cmap: &[u8], base14: usize, selector: u32) -> Option<(u32, u32)> {
    let num_records = read_u32_be(cmap, base14 + 6)? as usize;
    let records_off  = base14 + 10;
    let hit = search_records(num_records, selector, |i| read_u24_be(cmap, records_off + i * 11))?.ok()?;
    let rec = records_off + hit * 11;
    let default_off     = read_u32_be(cmap, rec + 3)?;
    let non_default_off = read_u32_be(cmap, rec + 7)?;
    Some((default_off, non_default_off))
}

fn lookup_non_default_uvs(cmap: &[u8], base: usize, codepoint: u32) -> Option<u16> {
    let num_mappings = read_u32_be(cmap, base)? as usize;
    let map_start    = base + 4;
    let hit = search_records(num_mappings, codepoint, |i| read_u24_be(cmap, map_start + i * 5))?.ok()?;
    read_u16_be(cmap, map_start + hit * 5 + 3).and_then(notdef_is_miss)
}

fn lookup_default_uvs(cmap: &[u8], base: usize, codepoint: u32) -> bool {
    let num_ranges  = match read_u32_be(cmap, base) { Some(v) => v as usize, None => return false };
    let range_start = base + 4;
    let cand = match search_records(num_ranges, codepoint, |i| read_u24_be(cmap, range_start + i * 4)) {
        Some(Ok(_)) => return true,
        Some(Err(0)) | None => return false,
        Some(Err(i)) => i - 1,
    };
    let rec = range_start + cand * 4;
    let start = match read_u24_be(cmap, rec) { Some(v) => v, None => return false };
    let additional = match cmap.get(rec + 3) { Some(&b) => b as u32, None => return false };
    codepoint >= start && codepoint <= start + additional
}

// A Windows symbol font: a (3,0) subtable and nothing keyed by Unicode.
pub(crate) fn cmap_is_symbol(cmap: &[u8]) -> bool {
    let encodings: Vec<Encoding> = subtables(cmap).map(|s| s.encoding).collect();
    encodings.contains(&Encoding::Symbol) && !encodings.contains(&Encoding::Unicode)
}

// A Unicode variation sequence: selector, base, and its glyph, None where it takes the base's own.
pub type Sequence = (u32, u32, Option<u16>);

// Every sequence the format 14 subtable lists that `keep` takes, default ranges counted out, up to
// `cap` of them read.
pub(crate) fn cmap_variation_sequences(cmap: &[u8], cap: usize, keep: impl Fn(&Sequence) -> bool) -> Option<Vec<Sequence>> {
    let Some(base14) = find_format14_subtable(cmap) else { return Some(Vec::new()) };
    let records = read_u32_be(cmap, base14 + 6)? as usize;
    let (mut read, mut out) = (0usize, Vec::new());
    let mut take = |sequence: Sequence| -> Option<()> {
        read += 1;
        if read > cap { return None; }
        if keep(&sequence) { out.push(sequence); }
        Some(())
    };
    for i in 0..records {
        let rec = base14 + 10 + i * 11;
        let selector = read_u24_be(cmap, rec)?;
        let (default, non_default) = (read_u32_be(cmap, rec + 3)? as usize, read_u32_be(cmap, rec + 7)? as usize);
        if default != 0 {
            let at = base14 + default;
            for r in 0..read_u32_be(cmap, at)? as usize {
                let (start, more) = (read_u24_be(cmap, at + 4 + r * 4)?, *cmap.get(at + 7 + r * 4)?);
                for base in start..=start + u32::from(more) {
                    take((selector, base, None))?;
                }
            }
        }
        if non_default != 0 {
            let at = base14 + non_default;
            for m in 0..read_u32_be(cmap, at)? as usize {
                let (base, gid) = (read_u24_be(cmap, at + 4 + m * 5)?, read_u16_be(cmap, at + 7 + m * 5)?);
                take((selector, base, Some(gid)))?;
            }
        }
    }
    Some(out)
}

// Every mapping of every subtable keyed by Unicode, in Unicode terms, lower tiers first and each
// subtable in its own order, until `push` refuses one or the work runs out.
fn each_unicode_mapping(cmap: &[u8], cap: usize, mut push: impl FnMut(u32, u16) -> bool) -> Option<()> {
    let mut work = cap.saturating_mul(4);
    for sub in &CmapLookup::new(cmap).0 {
        let (format, off) = (sub.format, sub.off);
        match sub.encoding {
            Encoding::Unicode => each_mapping(cmap, format, off, &mut work, &mut push)?,
            Encoding::Symbol => {
                each_mapping(cmap, format, off, &mut work, &mut push)?;
                each_mapping(cmap, format, off, &mut work, |code, gid| {
                    !(0xF000..=0xF0FF).contains(&code) || push(code - 0xF000, gid)
                })?;
            }
            Encoding::MacRoman { turkish } => each_mapping(cmap, format, off, &mut work, |code, gid| {
                let Ok(byte) = u8::try_from(code) else { return true };
                push(if turkish { mac_turkish_char(byte) } else { mac_roman_char(byte) } as u32, gid)
            })?,
        }
    }
    Some(())
}

// The same choice as cmap_glyph_id for every code, so an index built from it answers alike.
pub fn cmap_entries(cmap: &[u8], cap: usize) -> Option<Vec<(u32, u16)>> {
    let mut out = Entries { map: Vec::new(), cap, compact_at: cap };
    each_unicode_mapping(cmap, cap, |code, gid| out.push(code, gid))?;
    out.compact();
    Some(out.map)
}

// As cmap_entries, but only the mappings whose glyph `keep` takes. A code belongs to the first
// subtable mapping it whatever the glyph, marked in a bitset over Unicode rather than sorted out.
pub(crate) fn cmap_entries_kept(cmap: &[u8], cap: usize, keep: impl Fn(u16) -> bool) -> Option<Vec<(u32, u16)>> {
    let mut claimed = alloc::vec![0u64; 0x11_0000 / 64];
    let (mut distinct, mut out) = (0usize, Vec::new());
    each_unicode_mapping(cmap, cap, |code, gid| {
        let Some(word) = claimed.get_mut((code / 64) as usize) else { return true };
        let bit = 1u64 << (code % 64);
        if *word & bit == 0 {
            *word |= bit;
            distinct += 1;
            if keep(gid) { out.push((code, gid)); }
        }
        distinct <= cap
    })?;
    out.sort_unstable();
    Some(out)
}

// Pushes compact (sorted, first kept) past the cap and again a quarter cap later, so the cap
// counts distinct codes and repeats cost amortized time.
struct Entries {
    map: Vec<(u32, u16)>,
    cap: usize,
    compact_at: usize,
}

impl Entries {
    fn push(&mut self, code: u32, gid: u16) -> bool {
        self.map.push((code, gid));
        if self.map.len() > self.compact_at {
            self.compact();
            if self.map.len() > self.cap { return false; }
            self.compact_at = self.cap.max(self.map.len() + self.cap / 4);
        }
        true
    }

    fn compact(&mut self) {
        self.map.sort_by_key(|&(code, _)| code);
        self.map.dedup_by_key(|&mut (code, _)| code);
    }
}

// Every code a subtable maps to a glyph other than 0, in table order, until `f` refuses one, the
// table is malformed or the work runs out.
fn each_mapping(
    cmap: &[u8], format: u16, off: usize, work: &mut usize, mut f: impl FnMut(u32, u16) -> bool,
) -> Option<()> {
    let mut step = |code: u32, gid: u16| -> Option<()> {
        *work = work.checked_sub(1)?;
        (gid == 0 || f(code, gid)).then_some(())
    };
    match format {
        12 | 13 => {
            let groups = read_u32_be(cmap, off + 12)? as usize;
            for g in 0..groups {
                let rec = off + 16 + g * 12;
                let (start, end, first_gid) =
                    (read_u32_be(cmap, rec)?, read_u32_be(cmap, rec + 4)?, read_u32_be(cmap, rec + 8)?);
                if end < start { return None; }
                for code in start..=end {
                    let gid = if format == 13 { u64::from(first_gid) } else { u64::from(first_gid) + u64::from(code - start) };
                    step(code, u16::try_from(gid).unwrap_or(0))?;
                }
            }
        }
        10 => {
            let (first, count) = (read_u32_be(cmap, off + 12)?, read_u32_be(cmap, off + 16)?);
            for i in 0..count {
                let Some(code) = first.checked_add(i) else { break };
                step(code, read_u16_be(cmap, off + 20 + i as usize * 2)?)?;
            }
        }
        4 => {
            let seg_count = usize::from(read_u16_be(cmap, off + 6)? / 2);
            let ends = off + 14;
            let starts = ends + seg_count * 2 + 2;
            let deltas = starts + seg_count * 2;
            let range_offsets = deltas + seg_count * 2;
            for i in 0..seg_count {
                let (end, start, delta, range_offset) = (
                    read_u16_be(cmap, ends + i * 2)?,
                    read_u16_be(cmap, starts + i * 2)?,
                    read_u16_be(cmap, deltas + i * 2)?,
                    read_u16_be(cmap, range_offsets + i * 2)?,
                );
                if start > end { continue; }
                for code in start..=end {
                    let gid = if range_offset == 0 {
                        code.wrapping_add(delta)
                    } else {
                        let at = range_offsets + i * 2 + usize::from(range_offset) + usize::from(code - start) * 2;
                        match read_u16_be(cmap, at) {
                            Some(0) | None => 0,
                            Some(g) => g.wrapping_add(delta),
                        }
                    };
                    step(u32::from(code), gid)?;
                }
            }
        }
        6 => {
            let (first, count) = (read_u16_be(cmap, off + 6)?, read_u16_be(cmap, off + 8)?);
            for i in 0..count {
                let (Some(code), Some(gid)) = (first.checked_add(i), read_u16_be(cmap, off + 10 + usize::from(i) * 2))
                else { break };
                step(u32::from(code), gid)?;
            }
        }
        0 => {
            for byte in 0..=0xFFu8 {
                step(u32::from(byte), format0_lookup(cmap, off, byte).unwrap_or(0))?;
            }
        }
        _ => {}
    }
    Some(())
}

fn format4_lookup(cmap: &[u8], base: usize, cp: u16) -> Option<u16> {
    if base + 14 > cmap.len() { return None; }
    let seg_count = read_u16_be(cmap, base + 6)? as usize / 2;

    let end_off   = base + 14;
    let start_off = end_off + seg_count * 2 + 2;
    let delta_off = start_off + seg_count * 2;
    let range_off = delta_off + seg_count * 2;

    let i = match search_records(seg_count, cp as u32, |k| read_u16_be(cmap, end_off + k * 2).map(u32::from)) {
        Some(Ok(k)) | Some(Err(k)) => k,
        None => return None,
    };
    if i >= seg_count { return None; }

    let start = read_u16_be(cmap, start_off + i * 2)?;
    if cp < start { return None; }

    let delta        = read_u16_be(cmap, delta_off + i * 2)? as u32;
    let range_offset = read_u16_be(cmap, range_off + i * 2)? as usize;

    if range_offset == 0 {
        notdef_is_miss(((cp as u32 + delta) & 0xFFFF) as u16)
    } else {
        let gid_off = range_off + i * 2 + range_offset + (cp - start) as usize * 2;
        if gid_off + 2 > cmap.len() { return None; }
        let g = read_u16_be(cmap, gid_off).unwrap_or(0);
        if g == 0 { None } else { notdef_is_miss(((g as u32 + delta) & 0xFFFF) as u16) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // Over every fixture's cmap, keeping some glyphs is filtering every mapping: one code, one choice.
    #[test]
    fn kept_entries_are_entries_filtered() {
        let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");
        for rel in ["inter/InterVariable.ttf", "stix-two-math/STIX2Math.otf", "source-han-sans/SourceHanSansJP-VF.otf",
                    "structure/TestCMAPMacTurkish.ttf", "eb-garamond/EBGaramond.ttf", "noto-color-emoji/NotoColorEmoji.ttf"] {
            let bytes = std::fs::read(format!("{fonts}/{rel}")).expect("a fixture");
            let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("it parses");
            let cmap = tables.get("cmap").expect("a cmap");
            let all = cmap_entries(cmap, 200_000).expect("entries");
            assert_eq!(cmap_entries_kept(cmap, 200_000, |_| true).as_ref(), Some(&all), "{rel}");
            let some: Vec<(u32, u16)> = all.iter().copied().filter(|&(_, g)| g % 3 == 0).collect();
            assert_eq!(cmap_entries_kept(cmap, 200_000, |g| g % 3 == 0), Some(some), "{rel}");
        }
    }

    // A cmap whose records are (platform, encoding, index into `subtables`).
    fn cmap(records: &[(u16, u16, usize)], subtables: &[Vec<u8>]) -> Vec<u8> {
        let mut at = 4 + records.len() * 8;
        let offsets: Vec<usize> = subtables.iter().map(|s| { at += s.len(); at - s.len() }).collect();
        let mut out = vec![0, 0];
        out.extend((records.len() as u16).to_be_bytes());
        for &(platform, encoding, i) in records {
            out.extend(platform.to_be_bytes());
            out.extend(encoding.to_be_bytes());
            out.extend((offsets[i] as u32).to_be_bytes());
        }
        subtables.iter().for_each(|s| out.extend(s));
        out
    }

    fn format12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = vec![0, 12, 0, 0];
        out.extend(((16 + groups.len() * 12) as u32).to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend((groups.len() as u32).to_be_bytes());
        for &(start, end, gid) in groups {
            [start, end, gid].iter().for_each(|v| out.extend(v.to_be_bytes()));
        }
        out
    }

    // Segments of (start, end, first glyph), closed by the 0xFFFF segment.
    fn format4(language: u16, segments: &[(u16, u16, u16)]) -> Vec<u8> {
        let mut segs = segments.to_vec();
        segs.push((0xFFFF, 0xFFFF, 0));
        let n = segs.len() as u16;
        let mut out = vec![0, 4];
        out.extend((16 + n * 8).to_be_bytes());
        out.extend(language.to_be_bytes());
        out.extend((n * 2).to_be_bytes());
        out.extend([0; 6]);
        segs.iter().for_each(|s| out.extend(s.1.to_be_bytes()));
        out.extend([0, 0]);
        segs.iter().for_each(|s| out.extend(s.0.to_be_bytes()));
        segs.iter().for_each(|s| out.extend(s.2.wrapping_sub(s.0).to_be_bytes()));
        segs.iter().for_each(|_| out.extend([0, 0]));
        out
    }

    fn format6(language: u16, first: u16, glyphs: &[u16]) -> Vec<u8> {
        let mut out = vec![0, 6];
        out.extend((10 + glyphs.len() as u16 * 2).to_be_bytes());
        out.extend(language.to_be_bytes());
        out.extend(first.to_be_bytes());
        out.extend((glyphs.len() as u16).to_be_bytes());
        glyphs.iter().for_each(|g| out.extend(g.to_be_bytes()));
        out
    }

    fn format0(glyphs: &[(u8, u8)]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, 6, 0, 0];
        let mut array = [0u8; 256];
        glyphs.iter().for_each(|&(code, gid)| array[usize::from(code)] = gid);
        out.extend(array);
        out
    }

    fn indexed(entries: &[(u32, u16)], code: u32) -> Option<u16> {
        entries.binary_search_by_key(&code, |&(c, _)| c).ok().map(|i| entries[i].1)
    }

    // Five records name one format 12 and the format 4 repeats its BMP half: 6,000 raw mappings for
    // 1,000 codes. Without the offset dedup the work runs out; without compaction the cap trips.
    #[test]
    fn repeated_records_and_codes_count_once_against_the_cap() {
        let full = format12(&[(0x20, 0x20 + 499, 1), (0x10000, 0x10000 + 499, 501)]);
        let bmp = format4(0, &[(0x20, 0x20 + 499, 1)]);
        let records = [(0, 3, 1), (0, 4, 0), (0, 6, 0), (3, 1, 1), (3, 10, 0), (0, 4, 0), (0, 4, 0)];
        let table = cmap(&records, &[full, bmp]);

        let entries = cmap_entries(&table, 1000).expect("1,000 distinct codes fit a cap of 1,000");
        assert_eq!(entries.len(), 1000);
        assert_eq!(entries[0], (0x20, 1));
        assert_eq!(entries[999], (0x10000 + 499, 1000));
        assert!(cmap_entries(&table, 999).is_none(), "1,000 distinct codes passed a cap of 999");
    }

    // Inter's layout: two records each for one format 4 and one format 12, the format 4 first. A lookup
    // searches each once, the format 12 first, and its answer stands where the two differ.
    #[test]
    fn a_lookup_searches_each_subtable_once_best_first() {
        let bmp = format4(0, &[(0x41, 0x41, 7)]);
        let full = format12(&[(0x41, 0x41, 9), (0x10000, 0x10000, 10)]);
        let table = cmap(&[(0, 3, 0), (0, 4, 1), (3, 1, 0), (3, 10, 1)], &[bmp, full]);
        let lookup = CmapLookup::new(&table);
        assert_eq!(lookup.0.iter().map(|s| s.format).collect::<Vec<_>>(), [12, 4]);
        assert_eq!(lookup.glyph_id(&table, 0x41), Some(9));
        assert_eq!(cmap_glyph_id(&table, 0x10000), Some(10));
    }

    // Wingdings' layout: the symbol table maps U+F020..U+F0FF, which answers for 0x20..0xFF too.
    // A code the table maps itself still wins over its U+F0xx alias.
    #[test]
    fn a_symbol_table_answers_for_its_byte_codes() {
        let table = cmap(&[(3, 0, 0)], &[format4(0, &[(0x41, 0x41, 500), (0xF020, 0xF0FF, 10)])]);
        assert_eq!(cmap_glyph_id(&table, 0x42), Some(10 + 0x22));
        assert_eq!(cmap_glyph_id(&table, 0xE0), Some(10 + 0xC0));
        assert_eq!(cmap_glyph_id(&table, 0xF041), Some(10 + 0x21));
        assert_eq!(cmap_glyph_id(&table, 0x41), Some(500));
        assert_eq!(cmap_glyph_id(&table, 0x141), None);

        let entries = cmap_entries(&table, 1000).expect("enumerates");
        assert_eq!(indexed(&entries, 0x42), Some(10 + 0x22));
        assert_eq!(indexed(&entries, 0x41), Some(500));
    }

    // A Mac Roman table is keyed by Mac bytes whatever its format: byte 0x80 is Ä, not U+0080.
    #[test]
    fn mac_tables_are_keyed_by_mac_bytes_in_every_format() {
        let roman6 = cmap(&[(1, 0, 0)], &[format6(0, 0x80, &[10, 11])]);
        let roman4 = cmap(&[(1, 0, 0)], &[format4(0, &[(0x80, 0x81, 10)])]);
        for table in [&roman6, &roman4] {
            assert_eq!(cmap_glyph_id(table, 0x80), None, "U+0080 was read as a Mac byte");
            assert_eq!(cmap_glyph_id(table, 'Ä' as u32), Some(10));
            assert_eq!(cmap_glyph_id(table, 'Å' as u32), Some(11));
            assert_eq!(cmap_entries(table, 1000).expect("enumerates"), [(0xC4, 10), (0xC5, 11)]);
        }

        let turkish = cmap(&[(1, 0, 0)], &[format6(MAC_LANG_TURKISH, 0xDA, &[20])]);
        assert_eq!(cmap_glyph_id(&turkish, 'Ğ' as u32), Some(20));
        assert_eq!(cmap_entries(&turkish, 1000).expect("enumerates"), [('Ğ' as u32, 20)]);
    }

    // Mac Japanese and Windows PRC codes are not Unicode, so neither answers for 'A'.
    #[test]
    fn non_unicode_encodings_are_skipped() {
        let table = cmap(&[(1, 1, 0), (3, 3, 1)], &[format0(&[(0x41, 5)]), format4(0, &[(0x41, 0x41, 6)])]);
        assert_eq!(cmap_glyph_id(&table, 'A' as u32), None);
        assert_eq!(cmap_entries(&table, 1000).expect("enumerates"), []);
    }

    // Every encoding at once, the Unicode tables mapping part of what the others do. The index
    // must give what the direct lookup gives for every code.
    #[test]
    fn the_index_agrees_with_the_lookup_everywhere() {
        let table = cmap(
            &[(3, 1, 0), (3, 0, 1), (1, 0, 2), (1, 1, 3), (3, 10, 4), (0, 3, 5)],
            &[
                format4(0, &[(0x41, 0x5A, 1), (0xC0, 0xC5, 40)]),
                format4(0, &[(0x30, 0x30, 70), (0xF020, 0xF0FF, 100)]),
                format6(0, 0x80, &[300, 301, 302, 303, 304]),
                format0(&[(0x42, 9)]),
                format12(&[(0x41, 0x43, 500), (0x1F600, 0x1F64F, 400)]),
                format6(0, 0x2000, &[600, 0, 601]),
            ],
        );
        let entries = cmap_entries(&table, 10_000).expect("enumerates");
        for code in 0..=0x2_0000 {
            assert_eq!(indexed(&entries, code), cmap_glyph_id(&table, code), "U+{code:04X}");
        }
    }

    // HarfBuzz reads a non-default mapping to glyph 0 as no variant, so the base glyph stays.
    #[test]
    fn a_variant_mapped_to_notdef_is_no_variant() {
        let mut uvs = vec![0, 14];
        uvs.extend(30u32.to_be_bytes());
        uvs.extend(1u32.to_be_bytes());
        uvs.extend([0x0E, 0x01, 0x00]);
        uvs.extend(0u32.to_be_bytes());
        uvs.extend(21u32.to_be_bytes());
        uvs.extend(1u32.to_be_bytes());
        uvs.extend([0x00, 0x84, 0x5B, 0, 0]);
        let table = cmap(&[(0, 5, 0)], &[uvs]);
        assert!(cmap_variation_glyph_id(&table, 0x845B, 0xE0100).is_none());
    }
}
