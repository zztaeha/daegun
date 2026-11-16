use alloc::collections::btree_map::{BTreeMap, Entry};
use alloc::vec::Vec;
use core::cmp::Ordering::{Equal, Greater, Less};

use super::super::decoder::{
    mac_roman_byte, mac_roman_char, mac_turkish_byte, mac_turkish_char, midpoint_search, read_u16_be, read_u24_be,
    read_u32_be, window,
};
use super::super::format::index::Put;
use super::super::format::sanitize::Sanitizer;

// Language 18 on a Macintosh subtable means MacOS Turkish rather than MacRoman.
const MAC_LANG_TURKISH: u16 = 18;

// A format 12 or 13 group can name codes past Unicode's last. They map nothing, so no lookup answers
// for one, no enumeration lists one, and a cmap lists at most this many codes.
const UNICODE_CODES: u32 = 0x11_0000;

// The encoding the record search takes as any.
const ANY_ENCODING: u16 = 0xFFFF;

// The records a cmap is read through, best first: symbol, 32-bit Unicode before 16-bit as the cmap chapter asks,
// the deprecated Unicode encodings, Mac Roman, then any Mac record at all.
const BEST: [(u16, u16); 11] =
    [(3, 0), (3, 10), (0, 6), (0, 4), (3, 1), (0, 3), (0, 2), (0, 1), (0, 0), (1, 0), (1, ANY_ENCODING)];

// How the one subtable's codes stand for Unicode.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    Unicode,
    // A symbol subtable looks again at U+F000 plus a byte code it lacks.
    Symbol,
    // ASCII as itself and the rest through Mac Roman, or through MacOS Turkish where the language says so.
    Mac { turkish: bool },
    // Any other Mac subtable answers for ASCII alone, which every Mac script encoding shares.
    Ascii,
}

// Where a symbol subtable holds a byte code: the cmap chapter lays its symbols out from U+F020, byte 0x20's place,
// so byte b sits at U+F000 + b.
pub(crate) fn symbol_code(u: u32) -> Option<u32> {
    (u <= 0xFF).then(|| 0xF000 + u)
}

// The cmap's header and records. Each subtable is checked where it's read, one that fails being absent, so a lookup
// checks the few it reads however many records share them.
pub(crate) fn sanitizes(cmap: &[u8]) -> bool {
    let s = Sanitizer::new(cmap);
    let Some(records) = read_u16_be(cmap, 2) else { return false };
    read_u16_be(cmap, 0) == Some(0) && s.array(4, records.into(), 8)
}

fn subtable_sanitizes(cmap: &[u8], off: usize) -> bool {
    let s = &mut Sanitizer::new(cmap);
    let word = |at: usize| read_u16_be(cmap, at).unwrap_or(0);
    let long = |at: usize| read_u32_be(cmap, at).unwrap_or(0);
    if !s.fits(off, 2) {
        return false;
    }
    match word(off) {
        0 => s.fits(off, 262),
        4 => s.fits(off, 14) && s.range(off + 14, 2 + 4 * u32::from(word(off + 6))),
        6 => s.fits(off, 10) && s.array(off + 10, word(off + 8).into(), 2),
        10 => s.fits(off, 20) && s.array(off + 20, long(off + 16), 2),
        12 | 13 => s.fits(off, 16) && s.array(off + 16, long(off + 12), 12),
        format @ (14 | 15) => {
            let records = long(off + 6);
            s.fits(off, 10)
                && s.array(off + 10, records, 11)
                && (0..records as usize).all(|i| {
                    // Default ranges of 4 bytes, then glyph mappings of 5, or 6 in format 15's 24-bit glyphs.
                    let mapping = if format == 15 { 6 } else { 5 };
                    [(off + 13 + 11 * i, 4), (off + 17 + 11 * i, mapping)].into_iter().all(|(field, size)| {
                        let at = off.saturating_add(long(field) as usize);
                        long(field) == 0 || (s.fits(at, 4) && s.array(at + 4, long(at), size))
                    })
                })
        }
        _ => true,
    }
}

// The record the search finds, and the offset it names, none for 0.
fn find_subtable(cmap: &[u8], platform: u16, encoding: u16) -> Option<usize> {
    let records = read_u16_be(cmap, 2).map_or(0, usize::from);
    let i = midpoint_search(records, |i| {
        let (p, e) = (read_u16_be(cmap, 4 + 8 * i)?, read_u16_be(cmap, 6 + 8 * i)?);
        Some(platform.cmp(&p).then(if encoding == ANY_ENCODING { Equal } else { encoding.cmp(&e) }))
    })?;
    read_u32_be(cmap, 8 + 8 * i).filter(|&off| off != 0).map(|off| off as usize)
}

#[derive(Clone, Copy)]
struct Subtable {
    format: u16,
    off: usize,
    segments: usize,
    // Segments in code order, each running forward and ending before the next starts, so any search finds a
    // code's one segment.
    sorted: bool,
}

impl Subtable {
    fn new(cmap: &[u8], off: usize) -> Subtable {
        let format = read_u16_be(cmap, off).unwrap_or(0);
        let segments = match format {
            4 => read_u16_be(cmap, off + 6).map_or(0, |n| usize::from(n / 2)),
            12 | 13 => read_u32_be(cmap, off + 12).map_or(0, |n| n as usize),
            0 | 6 | 10 => 1,
            _ => 0,
        };
        let unsorted = Subtable { format, off, segments, sorted: false };
        let mut last_end: Option<u32> = None;
        let sorted = (0..segments).all(|i| match unsorted.bounds(cmap, i) {
            Some((start, end)) if start <= end && last_end.is_none_or(|last| start > last) => {
                last_end = Some(end);
                true
            }
            _ => false,
        });
        Subtable { sorted, ..unsorted }
    }

    // Segment `i`'s first and last codes as the table states them, either way round.
    fn bounds(&self, cmap: &[u8], i: usize) -> Option<(u32, u32)> {
        let word = |at: usize| read_u16_be(cmap, at).map(u32::from);
        let long = |at: usize| read_u32_be(cmap, at);
        let off = self.off;
        match self.format {
            4 => Some((word(off + 16 + 2 * self.segments + 2 * i)?, word(off + 14 + 2 * i)?)),
            12 | 13 => Some((long(off + 16 + 12 * i)?, long(off + 20 + 12 * i)?)),
            6 => Some((word(off + 6)?, word(off + 6)? + word(off + 8)?.checked_sub(1)?)),
            10 => {
                let first = long(off + 12)?;
                let last = first.wrapping_add(long(off + 16)?.checked_sub(1)?);
                // Indexed by the code less the first, unsigned, so an array past 0xFFFFFFFF wraps to 0.
                Some((if last < first { 0 } else { first }, last))
            }
            0 => Some((0, 0xFF)),
            _ => None,
        }
    }

    // Segment `i` where it runs forward.
    fn segment(&self, cmap: &[u8], i: usize) -> Option<Segment> {
        if let 12 | 13 = self.format {
            let r = window::<12>(cmap, self.off + 16 + 12 * i)?;
            let [start, end, glyph] = [0, 4, 8].map(|k| u32::from_be_bytes([r[k], r[k + 1], r[k + 2], r[k + 3]]));
            let rule = if self.format == 12 { Rule::Sequential(glyph) } else { Rule::Constant(glyph) };
            return (start <= end).then_some(Segment { start, end, rule });
        }
        let (start, end) = self.bounds(cmap, i).filter(|(start, end)| start <= end)?;
        let (off, n) = (self.off, self.segments);
        let rule = match self.format {
            4 => {
                let range_at = off + 16 + 6 * n + 2 * i;
                match (read_u16_be(cmap, off + 16 + 4 * n + 2 * i)?, read_u16_be(cmap, range_at)?) {
                    (delta, 0) => Rule::Delta(delta),
                    (delta, range) => Rule::Array { at: range_at + 2 * usize::from(range / 2), delta },
                }
            }
            12 => Rule::Sequential(read_u32_be(cmap, off + 24 + 12 * i)?),
            13 => Rule::Constant(read_u32_be(cmap, off + 24 + 12 * i)?),
            6 => Rule::Words(off + 10),
            10 => {
                let place = start.wrapping_sub(read_u32_be(cmap, off + 12)?) as usize;
                Rule::Words((off + 20).saturating_add(place.saturating_mul(2)))
            }
            _ => Rule::Bytes(off + 6),
        };
        Some(Segment { start, end, rule })
    }

    // The glyph read for `code`, 0 for none: formats 4, 12 and 13 find the segment by `midpoint_search`,
    // whose comparisons differ where a segment runs backward.
    fn glyph(&self, cmap: &[u8], code: u32) -> u32 {
        let i = match self.format {
            4 => midpoint_search(self.segments, |i| {
                let (start, end) = self.bounds(cmap, i)?;
                Some(if code > end { Greater } else if code < start { Less } else { Equal })
            }),
            12 | 13 => midpoint_search(self.segments, |i| {
                let (start, end) = self.bounds(cmap, i)?;
                Some(if code < start { Less } else if code > end { Greater } else { Equal })
            }),
            _ => Some(0),
        };
        i.and_then(|i| self.segment(cmap, i)).filter(|s| s.holds(code)).map_or(0, |s| s.glyph(cmap, code))
    }

    // Every code the subtable maps to a glyph other than 0, each once. In order, segment by segment, a segment of one
    // rule as runs; out of order, the codes any segment holds, each taking the glyph the search finds.
    fn each(&self, cmap: &[u8], out: &mut impl Put) {
        out.expect(self.segments);
        if !self.sorted {
            let mut held: Vec<(u32, u32)> = (0..self.segments)
                .filter_map(|i| self.segment(cmap, i))
                .filter(|s| s.start < UNICODE_CODES)
                .map(|s| (s.start, s.end.min(UNICODE_CODES - 1)))
                .collect();
            held.sort_unstable();
            for code in merged(held).into_iter().flat_map(|(first, last)| first..=last) {
                let gid = self.glyph(cmap, code);
                if gid != 0 {
                    out.put(code, gid);
                }
            }
            return;
        }
        for i in 0..self.segments {
            let Some(segment) = self.segment(cmap, i) else { continue };
            let end = segment.end.min(UNICODE_CODES - 1);
            if segment.start <= end && segment.runs(end, out) {
                continue;
            }
            for code in segment.start..=end {
                let gid = segment.glyph(cmap, code);
                if gid != 0 {
                    out.put(code, gid);
                }
            }
        }
    }
}

// A run of codes a subtable maps by one rule.
#[derive(Clone, Copy)]
struct Segment {
    start: u32,
    end: u32,
    rule: Rule,
}

#[derive(Clone, Copy)]
enum Rule {
    // The code plus a delta, modulo 65,536: format 4 without a glyph array.
    Delta(u16),
    // Format 4's words from `at`, where the cmap chapter's formula puts the segment's first code: its range offset
    // in words past the offset's own place, wherever in the table that lands. A nonzero glyph plus the delta.
    Array { at: usize, delta: u16 },
    // Words from `at` by the code's place: formats 6 and 10.
    Words(usize),
    // Bytes from `at`: format 0.
    Bytes(usize),
    // The first glyph plus the code's place, wrapping (format 12), or one glyph for every code (format 13).
    Sequential(u32),
    Constant(u32),
}

impl Segment {
    fn holds(&self, code: u32) -> bool {
        (self.start..=self.end).contains(&code)
    }

    // The segment's codes to `end` as runs of glyphs rising by one or the same, glyph 0 left out: false for a rule
    // read code by code.
    fn runs(&self, end: u32, out: &mut impl Put) -> bool {
        let start = self.start;
        match self.rule {
            Rule::Delta(delta) if end <= 0xFFFF => {
                let zero = u32::from(0u16.wrapping_sub(delta));
                let glyph = u32::from((start as u16).wrapping_add(delta));
                if (start..=end).contains(&zero) {
                    (zero > start).then(|| out.put_run(start, zero - 1, glyph, 1));
                    (zero < end).then(|| out.put_run(zero + 1, end, 1, 1));
                } else {
                    out.put_run(start, end, glyph, 1);
                }
            }
            // Glyphs that wrap past u32's last to 0 are read code by code.
            Rule::Sequential(first) if first.checked_add(end - start).is_some() => {
                let (from, glyph) = if first == 0 { (start + 1, 1) } else { (start, first) };
                (from <= end).then(|| out.put_run(from, end, glyph, 1));
            }
            Rule::Constant(glyph) => {
                if glyph != 0 {
                    out.put_run(start, end, glyph, 0);
                }
            }
            _ => return false,
        }
        true
    }

    // The glyph for a code the segment holds, 0 for none.
    fn glyph(&self, cmap: &[u8], code: u32) -> u32 {
        let place = code - self.start;
        match self.rule {
            Rule::Delta(delta) => (code as u16).wrapping_add(delta).into(),
            Rule::Array { at, delta } => read_u16_be(cmap, at + 2 * place as usize)
                .filter(|&g| g != 0)
                .map_or(0, |g| g.wrapping_add(delta).into()),
            Rule::Words(at) => {
                let word = at.saturating_add((place as usize).saturating_mul(2));
                read_u16_be(cmap, word).map_or(0, u32::from)
            }
            Rule::Bytes(at) => cmap.get(at + place as usize).copied().map_or(0, u32::from),
            Rule::Sequential(first) => first.wrapping_add(place),
            Rule::Constant(glyph) => glyph,
        }
    }
}

// A cmap as shaping reads it: one subtable, the first in `BEST`'s order whose check passes, answers for every code,
// and a cmap whose header fails answers for none. Built once, it serves every lookup in the cmap it read.
pub struct CmapLookup {
    best: Option<(Subtable, Reading)>,
    // The variation subtable (0, 5) names, and whether it's format 15, whose glyphs take 24 bits.
    sequences: Option<(usize, bool)>,
}

impl CmapLookup {
    pub fn new(cmap: &[u8]) -> CmapLookup {
        if !sanitizes(cmap) {
            return CmapLookup { best: None, sequences: None };
        }
        let best = BEST.iter().find_map(|&(platform, encoding)| {
            let off = find_subtable(cmap, platform, encoding).filter(|&off| subtable_sanitizes(cmap, off))?;
            let reading = match (platform, encoding) {
                (3, 0) => Reading::Symbol,
                (1, 0) => Reading::Mac {
                    turkish: matches!(read_u16_be(cmap, off), Some(0 | 4 | 6))
                        && read_u16_be(cmap, off + 4) == Some(MAC_LANG_TURKISH),
                },
                (1, _) => Reading::Ascii,
                _ => Reading::Unicode,
            };
            Some((Subtable::new(cmap, off), reading))
        });
        // The variation sequences: of the (0, 5) records, read in order, the first of format 15, the 24-bit twin,
        // else the first of format 14, where its check passes.
        let unicode_variations =
            |i: usize| read_u16_be(cmap, 4 + 8 * i) == Some(0) && read_u16_be(cmap, 6 + 8 * i) == Some(5);
        let subtables = (0..read_u16_be(cmap, 2).map_or(0, usize::from))
            .filter(|&i| unicode_variations(i))
            .filter_map(|i| read_u32_be(cmap, 8 + 8 * i).map(|off| off as usize))
            .filter(|&off| off != 0);
        let first = |format: u16| subtables.clone().find(|&off| read_u16_be(cmap, off) == Some(format));
        let sequences = first(15).map(|off| (off, true)).or_else(|| first(14).map(|off| (off, false)));
        let sequences = sequences.filter(|&(off, _)| subtable_sanitizes(cmap, off));
        CmapLookup { best, sequences }
    }

    // Whether the cmap is read through its symbol subtable.
    pub(crate) fn symbol(&self) -> bool {
        matches!(self.best, Some((_, Reading::Symbol)))
    }

    pub fn glyph_id(&self, cmap: &[u8], codepoint: u32) -> Option<u32> {
        let (sub, reading) = self.best.as_ref().filter(|_| codepoint < UNICODE_CODES)?;
        let at = |code: u32| notdef_is_miss(sub.glyph(cmap, code));
        match *reading {
            Reading::Unicode => at(codepoint),
            Reading::Symbol => at(codepoint).or_else(|| at(symbol_code(codepoint)?)),
            Reading::Mac { turkish: true } => at(mac_turkish_byte(codepoint)?.into()),
            Reading::Mac { .. } if codepoint < 0x80 => at(codepoint),
            Reading::Mac { .. } => at(mac_roman_byte(codepoint).filter(|&b| b >= 0x80)?.into()),
            Reading::Ascii if codepoint < 0x80 => at(codepoint),
            Reading::Ascii => None,
        }
    }

    // Every code the lookup answers for, ascending, with its glyph, so an index built from it answers alike.
    #[cfg(test)]
    pub fn entries(&self, cmap: &[u8]) -> Vec<(u32, u32)> {
        let mut out = Entries { map: Vec::new(), compact_at: UNICODE_CODES as usize };
        self.each_entry(cmap, &mut out);
        out.compact();
        out.map
    }

    // What `entries` lists, unsorted: a code given more than once answers with its first glyph. Never glyph 0.
    pub(crate) fn each_entry(&self, cmap: &[u8], out: &mut impl Put) {
        let Some((sub, reading)) = &self.best else { return };
        match *reading {
            Reading::Unicode => sub.each(cmap, out),
            Reading::Symbol => {
                sub.each(cmap, out);
                for u in 0..=0xFF {
                    if let Some(gid) = notdef_is_miss(sub.glyph(cmap, 0xF000 + u)) {
                        out.put(u, gid);
                    }
                }
            }
            Reading::Mac { turkish } => sub.each(cmap, &mut Codes(|code, gid| match u8::try_from(code) {
                Ok(byte) if turkish => out.put(mac_turkish_char(byte).into(), gid),
                Ok(byte) if byte < 0x80 => out.put(code, gid),
                Ok(byte) => out.put(mac_roman_char(byte).into(), gid),
                Err(_) => {}
            })),
            Reading::Ascii => sub.each(cmap, &mut Codes(|code, gid| if code < 0x80 { out.put(code, gid) })),
        }
    }

    // The variant lookup: the selector's record, then its default ranges before its own glyphs.
    pub fn variation(&self, cmap: &[u8], base: u32, selector: u32) -> Option<UvsLookup> {
        let (at, wide) = self.sequences?;
        let records = read_u32_be(cmap, at + 6)? as usize;
        let i = midpoint_search(records, |i| Some(selector.cmp(&read_u24_be(cmap, at + 10 + 11 * i)?)))?;
        let record = at + 10 + 11 * i;
        let (default, own) = (read_u32_be(cmap, record + 3)? as usize, read_u32_be(cmap, record + 7)? as usize);
        if default != 0 {
            let ranges = at + default;
            let found = midpoint_search(read_u32_be(cmap, ranges)? as usize, |i| {
                let (start, more) = (read_u24_be(cmap, ranges + 4 + 4 * i)?, *cmap.get(ranges + 7 + 4 * i)?);
                Some(if base < start { Less } else if base > start + u32::from(more) { Greater } else { Equal })
            });
            if found.is_some() {
                return Some(UvsLookup::UseDefault);
            }
        }
        let mappings = (own != 0).then_some(at + own)?;
        let count = read_u32_be(cmap, mappings)? as usize;
        let size = 5 + usize::from(wide);
        let i = midpoint_search(count, |i| Some(base.cmp(&read_u24_be(cmap, mappings + 4 + size * i)?)))?;
        mapped_glyph(cmap, mappings + 7 + size * i, wide).and_then(notdef_is_miss).map(UvsLookup::Explicit)
    }

    // The selectors the variation subtable's records name, onto `out`.
    fn selectors(&self, cmap: &[u8], out: &mut Vec<u32>) {
        let Some((at, _)) = self.sequences else { return };
        let records = read_u32_be(cmap, at + 6).map_or(0, |n| n as usize);
        out.extend((0..records).map_while(|i| read_u24_be(cmap, at + 10 + 11 * i)));
    }

    // Where the default ranges and glyph mappings `variation` reads for `selector` lie in the table, 0 for either it
    // doesn't read.
    fn arrays(&self, cmap: &[u8], selector: u32) -> (usize, usize) {
        let field = |at: usize| read_u32_be(cmap, at).map(|v| v as usize);
        let Some((at, _)) = self.sequences else { return (0, 0) };
        let found = field(at + 6).and_then(|records| {
            midpoint_search(records, |i| Some(selector.cmp(&read_u24_be(cmap, at + 10 + 11 * i)?)))
        });
        let record = found.map(|i| at + 10 + 11 * i);
        let placed = |offset: usize| if offset == 0 { 0 } else { at + offset };
        match record.map(|record| (field(record + 3), field(record + 7))) {
            Some((Some(default), Some(own))) => (placed(default), placed(own)),
            _ => (0, 0),
        }
    }

    // What `variation` answers from these arrays, every base at once: the bases its search of the default ranges finds,
    // and those its search of the mappings then finds with a glyph but 0. None once the searches' steps spend `work`.
    fn answers(&self, cmap: &[u8], (ranges, mappings): (usize, usize), work: &mut usize) -> Option<Answers> {
        let mut out = Answers::default();
        let Some((_, wide)) = self.sequences else { return Some(out) };
        if ranges != 0 {
            let Some(count) = read_u32_be(cmap, ranges) else { return Some(out) };
            let range = |i: usize| {
                let (start, more) = (read_u24_be(cmap, ranges + 4 + 4 * i)?, *cmap.get(ranges + 7 + 4 * i)?);
                Some((start, start + u32::from(more)))
            };
            let ranges = found(count as usize, range, work)?;
            out.defaults = merged(ranges.into_iter().map(|(first, last, _)| (first, last)));
        }
        if mappings != 0 {
            let (Some(count), size) = (read_u32_be(cmap, mappings), 5 + usize::from(wide)) else { return Some(out) };
            let base = |i: usize| read_u24_be(cmap, mappings + 4 + size * i).map(|b| (b, b));
            for (base, _, i) in found(count as usize, base, work)? {
                let glyph = mapped_glyph(cmap, mappings + 7 + size * i, wide).and_then(notdef_is_miss);
                if let Some(glyph) = glyph.filter(|_| !holds(&out.defaults, base)) {
                    out.glyphs.push((base, glyph));
                }
            }
        }
        Some(out)
    }
}

// What `lookup` answers for `selector`, worked out once for each pair of arrays its records name, `read` holding
// those worked out.
fn answered<'r>(
    lookup: &CmapLookup, table: &[u8], selector: u32, read: &'r mut BTreeMap<(usize, usize), Answers>,
    work: &mut usize,
) -> Option<&'r Answers> {
    let arrays = lookup.arrays(table, selector);
    if let Entry::Vacant(slot) = read.entry(arrays) {
        slot.insert(lookup.answers(table, arrays, work)?);
    }
    read.get(&arrays)
}

// The keys a `midpoint_search` of `count` entries finds, each entry a range of keys (first, last), with the entry it
// finds them at, in key order. Each step splits the keys it is asked about three ways, so one walk of the steps finds
// them all, each entry read at most once and spending one of `work`; None past it.
fn found(
    count: usize, entry: impl Fn(usize) -> Option<(u32, u32)>, work: &mut usize,
) -> Option<Vec<(u32, u32, usize)>> {
    // A step is a walk of entries lo..hi for the keys first..=last that reach them, or an entry's hit, pushed between
    // its two sides' walks so the keys come out in order.
    let (mut found, mut steps) = (Vec::new(), alloc::vec![Ok((0, count, 0, u32::MAX))]);
    while let Some(step) = steps.pop() {
        let (lo, hi, first, last) = match step {
            Ok(walk) => walk,
            Err(hit) => {
                found.push(hit);
                continue;
            }
        };
        if lo >= hi || first > last {
            continue;
        }
        *work = work.checked_sub(1)?;
        let mid = lo + (hi - lo - 1) / 2;
        // An entry that doesn't read fails the search for every key reaching it.
        let Some((start, end)) = entry(mid) else { continue };
        if last > end {
            steps.push(Ok((mid + 1, hi, first.max(end + 1), last)));
        }
        if first.max(start) <= last.min(end) {
            steps.push(Err((first.max(start), last.min(end), mid)));
        }
        if first < start {
            steps.push(Ok((lo, mid, first, last.min(start - 1))));
        }
    }
    Some(found)
}

// Ranges of codes, sorted by their first, as fewer: those that overlap or touch joined.
fn merged(ranges: impl IntoIterator<Item = (u32, u32)>) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (first, last) in ranges {
        match out.last_mut() {
            Some(prev) if first <= prev.1.saturating_add(1) => prev.1 = prev.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

// Whether sorted, disjoint ranges hold `code`.
fn holds(ranges: &[(u32, u32)], code: u32) -> bool {
    let at = ranges.partition_point(|&(_, last)| last < code);
    ranges.get(at).is_some_and(|&(first, _)| first <= code)
}

// The parts of sorted, disjoint `ranges` within `of`'s, onto `out` with `selector`, each spending one of `work`.
pub(crate) fn within(
    ranges: &[(u32, u32)], of: &[(u32, u32)], selector: u32, work: &mut usize, out: &mut Vec<(u32, u32, u32)>,
) -> Option<()> {
    for &(first, last) in ranges {
        let from = of.partition_point(|&(_, end)| end < first);
        for &(start, end) in of[from..].iter().take_while(|&&(start, _)| start <= last) {
            *work = work.checked_sub(1)?;
            out.push((selector, first.max(start), last.min(end)));
        }
    }
    Some(())
}

// What one table answers for a selector: its default bases as sorted, disjoint ranges, and the bases with a glyph of
// their own, sorted.
#[derive(Default)]
struct Answers {
    defaults: Vec<(u32, u32)>,
    glyphs: Vec<(u32, u32)>,
}

impl Answers {
    fn len(&self) -> usize {
        self.defaults.len() + self.glyphs.len()
    }

    // The bases the table answers for at all, a glyph of their own or the default.
    fn bases(&self) -> Vec<(u32, u32)> {
        let mut all: Vec<(u32, u32)> = self.glyphs.iter().map(|&(base, _)| (base, base)).collect();
        all.extend_from_slice(&self.defaults);
        all.sort_unstable();
        merged(all)
    }
}

// A font's variation sequences as its lookup answers them, by selector then base: the bases taking their default
// glyph, as (selector, first, last) ranges, and those with a glyph of their own, as (selector, base, glyph).
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct Sequences {
    pub(crate) defaults: Vec<(u32, u32, u32)>,
    pub(crate) glyphs: Vec<(u32, u32, u32)>,
}

impl Sequences {
    // Sequences given one at a time: a base listed both ways keeps the default, which a lookup finds first, and one
    // given two glyphs its lower.
    pub(crate) fn from_listed(listed: impl IntoIterator<Item = Sequence>) -> Sequences {
        let (mut defaults, mut glyphs) = (Vec::new(), Vec::new());
        for (selector, base, glyph) in listed {
            match glyph {
                Some(glyph) => glyphs.push((selector, base, glyph)),
                None => defaults.push((selector, base)),
            }
        }
        defaults.sort_unstable();
        defaults.dedup();
        glyphs.sort_unstable();
        glyphs.dedup_by_key(|&mut (selector, base, _)| (selector, base));
        let mut out = Sequences::default();
        for of in defaults.chunk_by(|a, b| a.0 == b.0) {
            let ranges = merged(of.iter().map(|&(_, base)| (base, base)));
            out.defaults.extend(ranges.into_iter().map(|(first, last)| (of[0].0, first, last)));
        }
        out.glyphs = glyphs.into_iter().filter(|&(selector, base, _)| !out.default_holds(selector, base)).collect();
        out
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.defaults.is_empty() && self.glyphs.is_empty()
    }

    // The ranges and glyph mappings held, which their memory and a pass over them take.
    pub(crate) fn len(&self) -> usize {
        self.defaults.len() + self.glyphs.len()
    }

    // `selector`'s default ranges, as (first, last).
    pub(crate) fn defaults_of(&self, selector: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let from = self.defaults.partition_point(|&(s, ..)| s < selector);
        self.defaults[from..].iter().take_while(move |&&(s, ..)| s == selector).map(|&(_, first, last)| (first, last))
    }

    fn default_holds(&self, selector: u32, base: u32) -> bool {
        let at = self.defaults.partition_point(|&(s, _, last)| (s, last) < (selector, base));
        self.defaults.get(at).is_some_and(|&(s, first, _)| s == selector && first <= base)
    }

    // Every selector held, each once, in order.
    pub(crate) fn selectors(&self) -> Vec<u32> {
        let mut all: Vec<u32> = self.defaults.iter().chain(&self.glyphs).map(|&(selector, ..)| selector).collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    // The sequences `lookup` answers for at all, read through its table, each step and range spending one of `work`;
    // None past it.
    pub(crate) fn answered_by(&self, lookup: &CmapLookup, table: &[u8], work: &mut usize) -> Option<Sequences> {
        let (mut out, mut read) = (Sequences::default(), BTreeMap::new());
        for selector in self.selectors() {
            let bases = answered(lookup, table, selector, &mut read, work)?.bases();
            *work = work.checked_sub(bases.len())?;
            let defaults: Vec<(u32, u32)> = self.defaults_of(selector).collect();
            within(&defaults, &bases, selector, work, &mut out.defaults)?;
        }
        out.glyphs = self.glyphs.iter().copied()
            .filter(|&(selector, base, _)| lookup.variation(table, base, selector).is_some())
            .collect();
        Some(out)
    }

    // Each sequence, a default range's one base at a time, in order.
    #[cfg(test)]
    pub(crate) fn listed(&self) -> Vec<Sequence> {
        let defaults = self.defaults.iter().flat_map(|&(s, first, last)| (first..=last).map(move |b| (s, b, None)));
        let mut all: Vec<Sequence> = defaults.chain(self.glyphs.iter().map(|&(s, b, g)| (s, b, Some(g)))).collect();
        all.sort_unstable();
        all
    }
}

// A font's characters as shaping reads them: DMAP, a table of cmap's own structure, answers first, a glyph 0 from it
// falling through to cmap. Either table may be missing or refused.
pub struct CharMap {
    dmap: CmapLookup,
    cmap: CmapLookup,
}

// The bytes a `CharMap` reads, empty for a missing table.
#[derive(Clone, Copy, Default)]
pub struct CharTables<'a> {
    pub cmap: &'a [u8],
    pub dmap: &'a [u8],
}

impl CharMap {
    pub fn new(tables: CharTables) -> CharMap {
        CharMap { dmap: CmapLookup::new(tables.dmap), cmap: CmapLookup::new(tables.cmap) }
    }

    // Whether DMAP reads at all, so a caller without one can leave its bytes unread.
    pub(crate) fn reads_dmap(&self) -> bool {
        self.dmap.best.is_some() || self.dmap.sequences.is_some()
    }

    pub fn glyph_id(&self, tables: CharTables, codepoint: u32) -> Option<u32> {
        self.dmap.glyph_id(tables.dmap, codepoint).or_else(|| self.cmap.glyph_id(tables.cmap, codepoint))
    }

    // DMAP's answer where it has one, else cmap's: a default from either takes the nominal glyph `glyph_id` gives.
    pub fn variation(&self, tables: CharTables, base: u32, selector: u32) -> Option<UvsLookup> {
        let dmap = self.dmap.variation(tables.dmap, base, selector);
        dmap.or_else(|| self.cmap.variation(tables.cmap, base, selector))
    }

    // Every code either table maps, ascending, with the glyph `glyph_id` gives.
    pub fn entries(&self, tables: CharTables) -> Vec<(u32, u32)> {
        let mut out = Entries { map: Vec::new(), compact_at: UNICODE_CODES as usize };
        self.each_entry(tables, &mut out);
        out.compact();
        out.map
    }

    // DMAP's entries before cmap's, so a code both map keeps DMAP's glyph.
    pub(crate) fn each_entry(&self, tables: CharTables, out: &mut impl Put) {
        self.dmap.each_entry(tables.dmap, out);
        self.cmap.each_entry(tables.cmap, out);
    }

    // Every sequence either table lists as `variation` answers it, DMAP's answer first and cmap's for the bases DMAP
    // leaves. Each pair of arrays is searched once however many records name it, but records can name overlapping
    // arrays too, so the steps and the ranges and glyphs answered are held to the two tables' subset budget; None past.
    pub(crate) fn sequences(&self, tables: CharTables) -> Option<Sequences> {
        let mut work = super::subset_budget(tables.cmap.len() + tables.dmap.len());
        let mut selectors = Vec::new();
        self.dmap.selectors(tables.dmap, &mut selectors);
        self.cmap.selectors(tables.cmap, &mut selectors);
        selectors.sort_unstable();
        selectors.dedup();
        let (mut dmap_read, mut cmap_read) = (BTreeMap::new(), BTreeMap::new());
        let mut out = Sequences::default();
        for selector in selectors {
            let dmap = answered(&self.dmap, tables.dmap, selector, &mut dmap_read, &mut work)?;
            let cmap = answered(&self.cmap, tables.cmap, selector, &mut cmap_read, &mut work)?;
            work = work.checked_sub(1 + dmap.len() + cmap.len())?;
            let taken = dmap.bases();
            let mut glyphs = dmap.glyphs.clone();
            glyphs.extend(cmap.glyphs.iter().filter(|&&(base, _)| !holds(&taken, base)));
            glyphs.sort_unstable();
            out.glyphs.extend(glyphs.into_iter().map(|(base, glyph)| (selector, base, glyph)));
            // DMAP's defaults, and cmap's less the bases DMAP gives a glyph.
            let mut defaults = dmap.defaults.clone();
            for &(first, last) in &cmap.defaults {
                let (mut from, at) = (first, dmap.glyphs.partition_point(|&(base, _)| base < first));
                for &(base, _) in dmap.glyphs[at..].iter().take_while(|&&(base, _)| base <= last) {
                    if from < base {
                        defaults.push((from, base - 1));
                    }
                    from = base + 1;
                }
                if from <= last {
                    defaults.push((from, last));
                }
            }
            defaults.sort_unstable();
            out.defaults.extend(merged(defaults).into_iter().map(|(first, last)| (selector, first, last)));
        }
        Some(out)
    }
}

// Glyph 0 is .notdef, so every lookup reads it as unmapped.
fn notdef_is_miss(gid: u32) -> Option<u32> {
    if gid == 0 { None } else { Some(gid) }
}

// A variation mapping's glyph: 16 bits in format 14, 24 in format 15.
fn mapped_glyph(cmap: &[u8], at: usize, wide: bool) -> Option<u32> {
    if wide { read_u24_be(cmap, at) } else { read_u16_be(cmap, at).map(u32::from) }
}

pub enum UvsLookup {
    Explicit(u32),
    UseDefault,
}

// A Windows symbol font: a (3,0) subtable and nothing keyed by Unicode.
pub(crate) fn cmap_is_symbol(cmap: &[u8]) -> bool {
    let (mut symbol, mut unicode) = (false, false);
    for i in 0..read_u16_be(cmap, 2).map_or(0, usize::from) {
        let (Some(platform), Some(encoding), Some(off)) =
            (read_u16_be(cmap, 4 + 8 * i), read_u16_be(cmap, 6 + 8 * i), read_u32_be(cmap, 8 + 8 * i))
        else {
            break;
        };
        if read_u16_be(cmap, off as usize).is_some() {
            symbol |= (platform, encoding) == (3, 0);
            unicode |= matches!((platform, encoding), (0 | 2, _) | (3, 1 | 10));
        }
    }
    symbol && !unicode
}

// A Unicode variation sequence: selector, base, and its glyph, None where it takes the base's own.
pub type Sequence = (u32, u32, Option<u32>);

// Pushes compact (sorted, first kept) past Unicode's size and again a quarter of it later, so repeats
// cost amortized time and the list never holds more than 1.25 times Unicode.
struct Entries {
    map: Vec<(u32, u32)>,
    compact_at: usize,
}

impl Put for Entries {
    fn put(&mut self, code: u32, gid: u32) {
        self.push(code, gid);
    }
}

// Codes taken one by one, by a reading that maps each.
struct Codes<F: FnMut(u32, u32)>(F);

impl<F: FnMut(u32, u32)> Put for Codes<F> {
    fn put(&mut self, code: u32, gid: u32) {
        (self.0)(code, gid);
    }
}

impl Entries {
    fn push(&mut self, code: u32, gid: u32) {
        self.map.push((code, gid));
        if self.map.len() > self.compact_at {
            self.compact();
            self.compact_at = (UNICODE_CODES as usize).max(self.map.len() + UNICODE_CODES as usize / 4);
        }
    }

    fn compact(&mut self) {
        self.map.sort_by_key(|&(code, _)| code);
        self.map.dedup_by_key(|&mut (code, _)| code);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/cmap.rs"]
mod tests;
