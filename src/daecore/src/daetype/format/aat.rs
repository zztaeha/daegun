#[cfg(doc)]
use crate::ErrorKind::{InvalidArgument, Malformed, Unsupported};
use alloc::vec::Vec;
use core::cmp::Ordering;
use crate::daecore::daetype::decoder::{midpoint_search, read_i16_be, read_u16_be, read_u32_be, window};
use super::index::SparseIndex;
use crate::daecore::Error;

/// The classes every AAT state table predefines.
pub mod class {
    /// The class a machine is handed once past the last glyph.
    pub const END_OF_TEXT: u16 = 0;
    pub(crate) const OUT_OF_BOUNDS: u16 = 1;
    /// The class of glyph 0xFFFF, the mark a deleted glyph leaves.
    pub const DELETED_GLYPH: u16 = 2;
}

/// The states every AAT state table predefines.
pub mod state {
    /// The state a machine starts in.
    pub const START_OF_TEXT: u16 = 0;
}

// A lookup value's width: 16 bits in most tables, 32 in Zapf's. Each width is its own copy of the
// reader, so the 16-bit one on the shaper's path holds its sizes as constants.
trait Value: Sized + TryFrom<u64> {
    const WIDTH: usize;
    fn read(data: &[u8], at: usize) -> Option<Self>;
}

impl Value for u16 {
    const WIDTH: usize = 2;
    fn read(data: &[u8], at: usize) -> Option<Self> {
        read_u16_be(data, at)
    }
}

impl Value for u32 {
    const WIDTH: usize = 4;
    fn read(data: &[u8], at: usize) -> Option<Self> {
        read_u32_be(data, at)
    }
}

// A format 10 unit of 1, 2, 4 or 8 bytes, the sizes Apple's manual allows, read big-endian; none
// for another size or a value too wide for the table's own.
fn read_unit<V: Value>(data: &[u8], at: usize, unit: usize) -> Option<V> {
    let bytes = data.get(at..at.checked_add(unit)?).filter(|_| matches!(unit, 1 | 2 | 4 | 8))?;
    V::try_from(bytes.iter().fold(0, |v, &b| v << 8 | u64::from(b))).ok()
}

/// A glyph and the value a lookup maps it to. Laid out as C reads it.
#[expect(clippy::exhaustive_structs, reason = "a pair the table stores, field for field")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GlyphValue {
    /// The glyph.
    pub glyph: u32,
    /// What the lookup maps it to.
    pub value: u16,
}

/// An AAT lookup table, glyph to 16-bit value, in any of its formats (0, 2, 4, 6, 8 and 10).
#[derive(Clone, Copy)]
pub struct Lookup<'a> {
    data: &'a [u8],
    format: u16,
    num_glyphs: u32,
    bin_srch: Option<(usize, usize)>,
}

impl core::fmt::Debug for Lookup<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lookup").field("format", &self.format).field("bytes", &self.data.len()).finish_non_exhaustive()
    }
}

impl<'a> Lookup<'a> {
    /// The lookup at the start of `data`, for a font of `num_glyphs` glyphs.
    ///
    /// # Errors
    ///
    /// [`Malformed`] for data too short to name a format, [`Unsupported`] for a format other than these.
    pub fn parse(data: &'a [u8], num_glyphs: u32) -> Result<Self, Error> {
        let format = read_u16_be(data, 0).ok_or(Error::malformed("AAT lookup: too short to name its format"))?;
        if !matches!(format, 0 | 2 | 4 | 6 | 8 | 10) {
            return Err(Error::unsupported("AAT lookup: a format other than 0, 2, 4, 6, 8 and 10"));
        }
        Ok(Lookup { data, format, num_glyphs, bin_srch: parse_bin_srch(data) })
    }

    /// A glyph's value; None for one the lookup doesn't list.
    pub fn value(&self, glyph: u32) -> Option<u16> {
        self.value_of(glyph)
    }

    // As `value`, for a lookup of 32-bit values.
    pub(crate) fn value_u32(&self, glyph: u32) -> Option<u32> {
        self.value_of(glyph)
    }

    // The value classes, substitutions and anchors are read through, `value` itself: format 10's units read as any
    // other format's values.
    pub(crate) fn typed_value(&self, glyph: u32) -> Option<u16> {
        self.value(glyph)
    }

    // `pairs`, under the name its callers read it by.
    pub(crate) fn typed_entries(&self) -> Vec<(u16, u16)> {
        self.pairs()
    }

    // Format 10's value width, which a subset keeps along with the format.
    pub(crate) fn wide_unit(&self) -> Option<usize> {
        (self.format == 10).then(|| read_u16_be(self.data, 2).map(usize::from))?
    }

    // Every glyph the lookup answers for, as ranges: those a unit lists and its search finds, as `value` reads them,
    // so a glyph a misordered unit lists but the search misses is as absent as one no unit lists.
    pub(crate) fn collect(&self, out: &mut Vec<(u32, u32)>) {
        let found = |g: u16| self.value(g.into()).or_else(|| self.value_u32(g.into()).map(|_| 0)).is_some();
        self.each_named(|g| if found(g) { out.push((g.into(), g.into())) });
    }

    // As `collect`, into a set.
    pub(crate) fn glyph_set(&self) -> GlyphRanges {
        let mut out = Vec::new();
        self.collect(&mut out);
        GlyphRanges::new(out)
    }

    // As `collect`, for 16-bit values: the glyphs whose value `keep` takes.
    pub(crate) fn collect_where(&self, out: &mut Vec<(u32, u32)>, keep: &dyn Fn(u32) -> bool) {
        let kept = self.pairs().into_iter().filter(|&(_, v)| keep(v.into()));
        out.extend(kept.map(|(g, _)| (g.into(), g.into())));
    }

    // Each glyph its units or arrays name, in order, which values are read for: each up to the font's count for
    // format 0, each listed by a unit, short of the deletion sentinel, and the trimmed arrays' spans.
    fn each_named(&self, mut f: impl FnMut(u16)) {
        let span = match self.format {
            0 => Some((0, self.num_glyphs)),
            2 | 4 | 6 => {
                let set = GlyphRanges::new(self.listed_units().map(|(a, b, _)| (a, b)).collect());
                set.glyphs().filter(|&g| g != 0xFFFF).for_each(|g| f(g as u16));
                None
            }
            8 | 10 => self.trimmed_range(),
            _ => None,
        };
        if let Some((first, count)) = span {
            (first..first.saturating_add(count).min(0x1_0000)).for_each(|g| f(g as u16));
        }
    }

    // The units collection walks, as many as `units` counts, each as its first and last glyph and where it lies,
    // less those that start at the deletion sentinel.
    fn listed_units(&self) -> impl Iterator<Item = (u32, u32, usize)> + '_ {
        let single = self.format == 6;
        let (unit, count) = match (self.format, self.bin_srch) {
            (2 | 4 | 6, Some((unit, n))) => (unit, self.units(unit, n, if single { 1 } else { 2 })),
            _ => (0, 0),
        };
        (0..count).filter_map(move |i| {
            let at = 12 + i * unit;
            let last = read_u16_be(self.data, at)?;
            let first = if single { last } else { read_u16_be(self.data, at + 2)? };
            (first != 0xFFFF).then_some((u32::from(first), u32::from(last), at))
        })
    }

    // A trimmed array's first glyph and count, none where it's empty or starts at the deletion sentinel.
    fn trimmed_range(&self) -> Option<(u32, u32)> {
        let at = if self.format == 8 { 2 } else { 4 };
        let (first, count) = (read_u16_be(self.data, at)?, read_u16_be(self.data, at + 2)?);
        (count > 0 && first != 0xFFFF).then_some((u32::from(first), u32::from(count)))
    }

    fn value_of<V: Value>(&self, glyph: u32) -> Option<V> {
        match self.format {
            // Format 0 is an array with no length of its own – it runs to the end of the font's
            // glyph range, so without this bound a glyph past that reads whatever bytes follow.
            0 if glyph < self.num_glyphs => V::read(self.data, 2 + V::WIDTH * glyph as usize),
            0 => None,
            2 => self.segment_single(glyph),
            4 => self.segment_array(glyph),
            6 => self.single_table(glyph),
            8 => self.trimmed_array(glyph),
            10 => self.trimmed_array_wide(glyph),
            _ => None,
        }
    }

    /// Each glyph listed, once and in order, with the value [`Lookup::value`] answers for it, so overlapping or
    /// repeated segments can't list more than the 65,535 glyphs there are.
    pub fn entries(&self) -> Vec<GlyphValue> {
        let mut out = Vec::new();
        let mut put = |glyph: u16, value| out.push(GlyphValue { glyph: glyph.into(), value });
        self.each_named(|g| if let Some(value) = self.value(g.into()) { put(g, value) });
        out
    }

    // As `entries`, as (glyph, value), which the subsetter writes lookups from.
    pub(crate) fn pairs(&self) -> Vec<(u16, u16)> {
        self.entries_by(|g| self.value(g.into()))
    }

    // As `pairs`, for a lookup whose values are 32 bits wide, its units sized for them.
    pub(crate) fn entries_u32(&self) -> Vec<(u16, u32)> {
        self.entries_by(|g| self.value_of(g.into()))
    }

    // `pairs` and `entries_u32`, under the names kerx's readers take them by.
    pub(crate) fn listed_entries(&self) -> Vec<(u16, u16)> {
        self.pairs()
    }

    pub(crate) fn listed_entries_u32(&self) -> Vec<(u16, u32)> {
        self.entries_u32()
    }

    // What expanding the lookup reads, found or not: its units and each glyph they or its array name. A few bytes can
    // name every glyph, so a reader expanding a lookup many subtables share charges this first.
    pub(crate) fn reads(&self) -> usize {
        let (units, named) = match self.format {
            0 => (0, self.num_glyphs as usize),
            8 | 10 => (0, self.trimmed_range().map_or(0, |(_, count)| count as usize)),
            _ => self.listed_units().fold((0, 0usize), |(n, named), (first, last, _)| {
                (n + 1, named.saturating_add((last + 1).saturating_sub(first) as usize))
            }),
        };
        units + named.min(0x1_0000)
    }

    fn entries_by<V>(&self, value: impl Fn(u16) -> Option<V>) -> Vec<(u16, V)> {
        let mut out = Vec::new();
        self.each_named(|g| if let Some(v) = value(g) { out.push((g, v)) });
        out
    }

    // The units searched and collected: `n` of `unit` bytes, less a last one whose first `words` words are all 0xFFFF.
    fn units(&self, unit: usize, n: usize, words: usize) -> usize {
        let last = 12 + unit * n.saturating_sub(1);
        let terminator = n > 0 && (0..words).all(|w| read_u16_be(self.data, last + 2 * w) == Some(0xFFFF));
        n - usize::from(terminator)
    }

    fn segment_single<V: Value>(&self, glyph: u32) -> Option<V> {
        V::read(self.data, self.segment(glyph, 4 + V::WIDTH)?.0 + 4)
    }

    fn segment_array<V: Value>(&self, glyph: u32) -> Option<V> {
        let (at, first) = self.segment(glyph, 6)?;
        V::read(self.data, usize::from(read_u16_be(self.data, at + 4)?) + V::WIDTH * (glyph - first) as usize)
    }

    // The segment found for `glyph`, the first midpoint whose glyphs hold it, and its first glyph; its units are at
    // least `min_unit` bytes.
    fn segment(&self, glyph: u32, min_unit: usize) -> Option<(usize, u32)> {
        let (unit, n) = self.bin_srch.filter(|&(unit, _)| unit >= min_unit)?;
        let bounds = |i: usize| {
            let at = 12 + i * unit;
            Some((u32::from(read_u16_be(self.data, at + 2)?), u32::from(read_u16_be(self.data, at)?)))
        };
        let i = midpoint_search(self.units(unit, n, 2), |i| {
            let (first, last) = bounds(i)?;
            Some(match () {
                _ if glyph < first => Ordering::Less,
                _ if glyph <= last => Ordering::Equal,
                _ => Ordering::Greater,
            })
        })?;
        Some((12 + i * unit, bounds(i)?.0))
    }

    fn single_table<V: Value>(&self, glyph: u32) -> Option<V> {
        let (unit, n) = self.bin_srch.filter(|&(unit, _)| unit >= 2 + V::WIDTH)?;
        let unit_glyph = |i: usize| read_u16_be(self.data, 12 + i * unit).map(u32::from);
        let i = midpoint_search(self.units(unit, n, 1), |i| Some(glyph.cmp(&unit_glyph(i)?)))?;
        V::read(self.data, 12 + i * unit + 2)
    }

    fn trimmed_array<V: Value>(&self, glyph: u32) -> Option<V> {
        let h = window::<4>(self.data, 2)?;
        let first = u32::from(u16::from_be_bytes([h[0], h[1]]));
        let count = u32::from(u16::from_be_bytes([h[2], h[3]]));
        let i = glyph.checked_sub(first)?;
        (i < count).then(|| V::read(self.data, 6 + V::WIDTH * i as usize))?
    }

    fn trimmed_array_wide<V: Value>(&self, glyph: u32) -> Option<V> {
        let h = window::<6>(self.data, 2)?;
        let unit = usize::from(u16::from_be_bytes([h[0], h[1]]));
        let first = u32::from(u16::from_be_bytes([h[2], h[3]]));
        let count = u32::from(u16::from_be_bytes([h[4], h[5]]));
        let i = glyph.checked_sub(first)?;
        if i >= count {
            return None;
        }
        read_unit(self.data, 8 + i as usize * unit, unit)
    }
}

fn parse_bin_srch(data: &[u8]) -> Option<(usize, usize)> {
    let h = window::<4>(data, 2)?;
    let unit = usize::from(u16::from_be_bytes([h[0], h[1]]));
    let declared = usize::from(u16::from_be_bytes([h[2], h[3]]));
    if unit == 0 {
        return None;
    }
    let fits = data.len().saturating_sub(12) / unit;
    Some((unit, declared.min(fits)))
}

// A glyph set collected from a lookup's units, its ranges sorted and merged, so it holds no more than the units it was
// collected from: as bits over its span where they take no more room, so a start or pair check is one read.
#[derive(Default)]
#[cfg_attr(test, derive(PartialEq, Debug))]
pub(crate) struct GlyphRanges(Held);

#[cfg_attr(test, derive(PartialEq, Debug))]
enum Held {
    Ranges(Vec<(u32, u32)>),
    Bits { first: u32, words: Vec<u64> },
}

impl Default for Held {
    fn default() -> Self {
        Held::Ranges(Vec::new())
    }
}

impl GlyphRanges {
    // From ranges in any order, each from its first glyph to its last; one ending before it starts holds none.
    pub(crate) fn new(mut ranges: Vec<(u32, u32)>) -> Self {
        ranges.retain(|&(first, last)| first <= last);
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (first, last) in ranges {
            match merged.last_mut() {
                Some(prev) if first <= prev.1.saturating_add(1) => prev.1 = prev.1.max(last),
                _ => merged.push((first, last)),
            }
        }
        if let (Some(&(first, _)), Some(&(_, last))) = (merged.first(), merged.last())
            && (last - first) as usize / 64 < merged.len()
        {
            let mut words = alloc::vec![0u64; (last - first) as usize / 64 + 1];
            for i in merged.iter().flat_map(|&(a, b)| (a - first) as usize..=(b - first) as usize) {
                words[i / 64] |= 1 << (i % 64);
            }
            return GlyphRanges(Held::Bits { first, words });
        }
        merged.shrink_to_fit();
        GlyphRanges(Held::Ranges(merged))
    }

    // As `new` from the runs `bits` holds.
    pub(crate) fn of_bits(bits: &super::index::GlyphBits) -> Self {
        let (runs, first, last) = bits.runs().fold((0, u32::MAX, 0), |(n, first, _), (a, b)| (n + 1, first.min(a), b));
        if runs == 0 {
            return GlyphRanges::default();
        }
        if (last - first) as usize / 64 < runs {
            let mut words = alloc::vec![0u64; (last - first) as usize / 64 + 1];
            for i in bits.runs().flat_map(|(a, b)| (a - first) as usize..=(b - first) as usize) {
                words[i / 64] |= 1 << (i % 64);
            }
            return GlyphRanges(Held::Bits { first, words });
        }
        let mut ranges = Vec::with_capacity(runs);
        ranges.extend(bits.runs());
        GlyphRanges(Held::Ranges(ranges))
    }

    pub(crate) fn glyphs(&self) -> impl Iterator<Item = u32> + '_ {
        let (ranges, bits) = match &self.0 {
            Held::Ranges(ranges) => (&ranges[..], None),
            Held::Bits { first, words } => (&[][..], Some((*first, &words[..]))),
        };
        let set = bits.into_iter().flat_map(|(first, words)| {
            let held = |(w, &bits): (usize, &u64)| (0..64).filter(move |b| bits >> b & 1 != 0).map(move |b| w * 64 + b);
            words.iter().enumerate().flat_map(held).map(move |i| first + i as u32)
        });
        ranges.iter().flat_map(|&(first, last)| first..=last).chain(set)
    }

    pub(crate) fn contains(&self, glyph: u32) -> bool {
        match &self.0 {
            Held::Ranges(ranges) => {
                let after = ranges.partition_point(|&(first, _)| first <= glyph);
                after.checked_sub(1).is_some_and(|i| ranges[i].1 >= glyph)
            }
            Held::Bits { first, words } => {
                let i = glyph.wrapping_sub(*first) as usize;
                words.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 != 0)
            }
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        match &self.0 {
            Held::Ranges(ranges) => ranges.capacity() * size_of::<(u32, u32)>(),
            Held::Bits { words, .. } => words.capacity() * size_of::<u64>(),
        }
    }
}

// An extended state table's header: its class count and where its class lookup, state array and
// entry table start.
#[derive(Clone, Copy)]
pub(crate) struct StxHeader {
    pub n_classes: usize,
    pub class: usize,
    pub state: usize,
    pub entry: usize,
}

impl StxHeader {
    pub fn read(data: &[u8]) -> Option<Self> {
        let h = window::<16>(data, 0)?;
        let word = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]) as usize;
        let header = StxHeader { n_classes: word(0), class: word(4), state: word(8), entry: word(12) };
        (1..=0xFFFF).contains(&header.n_classes).then_some(header)
    }

    // The class lookup, read to the table's end as the shaper and subsetter both take it.
    pub fn classes<'a>(&self, data: &'a [u8], num_glyphs: u32) -> Result<Lookup<'a>, Error> {
        let at = data.get(self.class..).ok_or(Error::malformed("AAT state table: its class lookup lies past its end"))?;
        Lookup::parse(at, num_glyphs)
    }

    pub fn starts(&self) -> [usize; 3] {
        [self.class, self.state, self.entry]
    }
}

// A morx chain: where it starts, its bytes, its default flags, the length of its header and features, and how many
// subtables it names.
#[derive(Clone, Copy)]
pub(crate) struct MorxChain<'a> {
    pub(crate) at: usize,
    pub(crate) bytes: &'a [u8],
    pub(crate) default_flags: u32,
    pub(crate) prefix: usize,
    pub(crate) subtable_count: u32,
}

impl<'a> MorxChain<'a> {
    // Each feature as its type, setting, enable flags and disable flags.
    pub(crate) fn features(&self) -> impl Iterator<Item = (u16, u16, u32, u32)> + 'a {
        let u32_at = |f: &[u8], i: usize| u32::from_be_bytes([f[i], f[i + 1], f[i + 2], f[i + 3]]);
        self.bytes[16..self.prefix].as_chunks::<12>().0.iter().map(move |f| {
            (u16::from_be_bytes([f[0], f[1]]), u16::from_be_bytes([f[2], f[3]]), u32_at(f, 4), u32_at(f, 8))
        })
    }

    // Its subtables as shaping reads them in `morx`, one after another by length and on past the chain's end. One
    // that doesn't fit in the table ends the walk.
    pub(crate) fn subtables(self, morx: &'a [u8]) -> impl Iterator<Item = MorxSubtable<'a>> {
        let mut at = self.at + self.prefix;
        (0..self.subtable_count).map_while(move |_| {
            let length = read_u32_be(morx, at)? as usize;
            if length < 12 || morx.len() - at < length {
                return None;
            }
            let sub = MorxSubtable {
                coverage: read_u32_be(morx, at + 4)?,
                flags: read_u32_be(morx, at + 8)?,
                body: &morx[at + 12..],
                own: length - 12,
            };
            at += length;
            Some(sub)
        })
    }
}

// morx's chains in order. One that doesn't fit in the table, or whose features run past its end, ends the walk.
pub(crate) fn morx_chains(morx: &[u8]) -> impl Iterator<Item = MorxChain<'_>> {
    let mut at = 8usize;
    (0..read_u32_be(morx, 4).unwrap_or(0)).map_while(move |_| {
        let length = read_u32_be(morx, at.checked_add(4)?)? as usize;
        let prefix = (read_u32_be(morx, at + 8)? as usize).checked_mul(12)?.checked_add(16)?;
        let chain = MorxChain {
            at,
            bytes: morx.get(at..at.checked_add(length)?)?,
            default_flags: read_u32_be(morx, at)?,
            prefix,
            subtable_count: read_u32_be(morx, at + 12)?,
        };
        at += length;
        (prefix <= length).then_some(chain)
    })
}

// A morx subtable: its coverage and flags, its body, which runs on to the table's end, and the body's own length.
pub(crate) struct MorxSubtable<'a> {
    pub(crate) coverage: u32,
    pub(crate) flags: u32,
    pub(crate) body: &'a [u8],
    pub(crate) own: usize,
}

// Where a part starting at `from` ends: at the next of the `starts` a header names, or at `len`.
pub(crate) fn end_of(starts: &[usize], from: usize, len: usize) -> usize {
    starts.iter().copied().chain([len]).filter(|&o| o > from).min().unwrap_or(len)
}

// A kerx value with tuples is a byte offset from `base` to its tuple, whose first value kerns, or 0 where the tuple
// doesn't fit `data`. A negative value is an offset past any table.
pub(crate) fn tuple_kern(data: &[u8], base: usize, value: i32, tuples: u32) -> i32 {
    if tuples == 0 {
        return value;
    }
    let len = usize::try_from(tuples).ok().and_then(|n| n.checked_mul(2));
    let at = base.checked_add(value as u32 as usize);
    let tuple = at.zip(len).and_then(|(at, len)| data.get(at..at.checked_add(len)?));
    tuple.and_then(|t| read_i16_be(t, 0)).map_or(0, i32::from)
}

/// One entry of an extended AAT state table. Laid out as C reads it.
#[expect(clippy::exhaustive_structs, reason = "the spec's record, field for field")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct Entry {
    /// The state to go to.
    pub new_state: u16,
    /// The entry's flags, which each table defines.
    pub flags: u16,
    /// The first extra word, 0 for a table with none.
    pub word1: u16,
    /// The second extra word, 0 for a table with fewer than two.
    pub word2: u16,
}

// The flag every machine sets on an entry that starts an action: Mark, SetMark, SetComponent or Push.
const INITIATES_ACTION: u16 = 0x8000;

// The start-state filter: the classes that leave the start state, act there or start an action, as bits over the
// table's classes. None past 512 classes, which a filter this size can't hold, where every glyph may start.
pub(crate) fn start_classes(table: &impl Machine, actionable: fn(&Entry) -> bool) -> Option<StartClasses> {
    if table.n_classes() > 512 {
        return None;
    }
    let mut page = [0u64; 8];
    for k in 0..table.n_classes() as u16 {
        let idle = table.entry_at(i32::from(state::START_OF_TEXT), k).is_some_and(|e| {
            table.next_state(&e) == i32::from(state::START_OF_TEXT)
                && e.flags & INITIATES_ACTION == 0
                && !actionable(&e)
        });
        if !idle {
            page[usize::from(k / 64)] |= 1 << (k % 64);
        }
    }
    Some(StartClasses { page, n_classes: table.n_classes() as u32 })
}

#[derive(Clone, Copy)]
pub(crate) struct StartClasses {
    page: [u64; 8],
    n_classes: u32,
}

impl StartClasses {
    // A class past the table's count reads as out of bounds, as the driver reads it.
    pub(crate) fn takes(&self, class: u32) -> bool {
        let class = if class < self.n_classes { class } else { class::OUT_OF_BOUNDS.into() };
        self.page[(class / 64) as usize] >> (class % 64) & 1 != 0
    }
}

// The glyphs that can start a machine: those the class table lists in a class the start filter takes, and the
// deletion sentinel when it takes its class. Where the start state acts at the end of text or on a glyph out of
// bounds, which any text holds, every glyph can.
pub(crate) fn initial_glyphs(table: &impl Machine, actionable: fn(&Entry) -> bool) -> GlyphRanges {
    let mut out = Vec::new();
    match start_classes(table, actionable) {
        Some(classes) if ![class::END_OF_TEXT, class::OUT_OF_BOUNDS].iter().any(|&k| classes.takes(k.into())) => {
            if classes.takes(class::DELETED_GLYPH.into()) {
                out.push((0xFFFF, 0xFFFF));
            }
            table.collect_classes_where(&mut out, &|k| classes.takes(k));
        }
        _ => out.push((0, 0xFFFF)),
    }
    GlyphRanges::new(out)
}

// What the driver needs of a state table, in either form: morx's and kerx's extended one, and the 16-bit one of
// Apple's `kern`.
pub(crate) trait Machine {
    fn class(&self, glyph: u32) -> u16;
    fn n_classes(&self) -> usize;
    // The class table's glyphs as `Lookup::collect_where` gathers them, by class.
    fn collect_classes_where(&self, out: &mut Vec<(u32, u32)>, keep: &dyn Fn(u32) -> bool);
    fn entry_at(&self, state: i32, klass: u16) -> Option<Entry>;
    fn next_state(&self, e: &Entry) -> i32;
}

const PAST_STATES: Error = Error::malformed("AAT state table: its state array lies past its end");
const PAST_ENTRIES: Error = Error::malformed("AAT state table: its entry table lies past its end");

/// An extended AAT state table (STXHeader), as morx and kerx hold them.
#[derive(Clone, Copy)]
pub struct StateTable<'a> {
    class_lookup: Lookup<'a>,
    // The class table's index, where a face built one, answering for its search.
    classes: Option<&'a SparseIndex>,
    state_array: &'a [u8],
    entry_table: &'a [u8],
    n_classes: usize,
    extra_words: usize,
    stride: usize,
}

impl core::fmt::Debug for StateTable<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StateTable").field("classes", &self.n_classes).finish_non_exhaustive()
    }
}

impl<'a> StateTable<'a> {
    /// The table at the start of `data`, its entries `extra_words` words past their state and flags, for a font
    /// of `num_glyphs` glyphs.
    ///
    /// # Errors
    ///
    /// [`Malformed`] for a header cut short, naming no classes or more than 65,535, or pointing past the data, and
    /// the class lookup's own; [`InvalidArgument`] for `extra_words` an entry can't hold.
    pub fn parse(data: &'a [u8], extra_words: usize, num_glyphs: u32) -> Result<Self, Error> {
        let header = StxHeader::read(data).ok_or(if data.len() < 16 {
            Error::malformed("AAT state table: its header is cut short")
        } else {
            Error::malformed("AAT state table: it names no classes, or more than 65,535")
        })?;
        // An entry is its new state and flags, then two bytes per extra word, which a caller names.
        let stride = extra_words.checked_mul(2).and_then(|n| n.checked_add(4));
        let stride = stride.ok_or(Error::invalid("AAT state table: more extra words than an entry can hold"))?;
        Ok(StateTable {
            class_lookup: header.classes(data, num_glyphs)?,
            classes: None,
            state_array: data.get(header.state..).ok_or(PAST_STATES)?,
            entry_table: data.get(header.entry..).ok_or(PAST_ENTRIES)?,
            n_classes: header.n_classes,
            extra_words,
            stride,
        })
    }

    pub(crate) fn with_classes(self, classes: Option<&'a SparseIndex>) -> Self {
        StateTable { classes, ..self }
    }

    // A searched class table's glyphs and classes, for an index to answer in its place: absent glyphs read out of
    // bounds either way. None for the array formats, which a glyph reads directly.
    pub(crate) fn searched_classes(&self) -> Option<Vec<(u16, u16)>> {
        let lookup = &self.class_lookup;
        matches!(lookup.format, 2 | 4 | 6).then(|| lookup.typed_entries())
    }

    /// A glyph's class: [`class::DELETED_GLYPH`] for 0xFFFF, and out of bounds (1) for one the class table
    /// doesn't list.
    pub fn class(&self, glyph: u32) -> u16 {
        if glyph == 0xFFFF {
            return class::DELETED_GLYPH;
        }
        match self.classes {
            Some(index) => index.lookup(glyph),
            None => self.class_lookup.typed_value(glyph).unwrap_or(class::OUT_OF_BOUNDS),
        }
    }

    /// The entry for a class in a state; None past the classes or where it doesn't read.
    pub fn entry(&self, state: u16, klass: u16) -> Option<Entry> {
        let klass = usize::from(klass);
        if klass >= self.n_classes {
            return None;
        }
        let cell = usize::from(state).checked_mul(self.n_classes)?.checked_add(klass)?.checked_mul(2)?;
        let index = read_u16_be(self.state_array, cell)?;
        let at = usize::from(index).checked_mul(self.stride)?;
        let e = window::<4>(self.entry_table, at)?;
        Some(Entry {
            new_state: u16::from_be_bytes([e[0], e[1]]),
            flags: u16::from_be_bytes([e[2], e[3]]),
            word1: if self.extra_words >= 1 { read_u16_be(self.entry_table, at + 4)? } else { 0 },
            word2: if self.extra_words >= 2 { read_u16_be(self.entry_table, at + 6)? } else { 0 },
        })
    }
}

impl Machine for StateTable<'_> {
    fn class(&self, glyph: u32) -> u16 {
        StateTable::class(self, glyph)
    }

    fn n_classes(&self) -> usize {
        self.n_classes
    }

    fn collect_classes_where(&self, out: &mut Vec<(u32, u32)>, keep: &dyn Fn(u32) -> bool) {
        self.class_lookup.collect_where(out, keep);
    }

    fn entry_at(&self, state: i32, klass: u16) -> Option<Entry> {
        self.entry(u16::try_from(state).ok()?, klass)
    }

    fn next_state(&self, e: &Entry) -> i32 {
        i32::from(e.new_state)
    }
}

// The 16-bit form, Apple `kern` format 1's: byte classes and cells, new states as byte offsets to rows. A new state
// before the state array is negative, and its row is absent.
pub(crate) struct ObsoleteStateTable<'a> {
    data: &'a [u8],
    first: u16,
    classes: &'a [u8],
    states: usize,
    entries: usize,
    n_classes: u16,
    state_offset: i32,
}

impl<'a> ObsoleteStateTable<'a> {
    // The table whose header lies at `at`, its offsets counted from there.
    pub(crate) fn parse(data: &'a [u8], at: usize) -> Option<Self> {
        let h = window::<8>(data, at)?;
        let word = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let class = at + usize::from(word(2));
        let c = window::<4>(data, class)?;
        let count = usize::from(u16::from_be_bytes([c[2], c[3]]));
        (word(0) != 0).then_some(ObsoleteStateTable {
            data,
            first: u16::from_be_bytes([c[0], c[1]]),
            classes: &data[class + 4..(class + 4 + count).min(data.len())],
            states: at + usize::from(word(4)),
            entries: at + usize::from(word(6)),
            n_classes: word(0),
            state_offset: i32::from(word(4)),
        })
    }
}

impl Machine for ObsoleteStateTable<'_> {
    fn class(&self, glyph: u32) -> u16 {
        if glyph == 0xFFFF {
            return class::DELETED_GLYPH;
        }
        let class = glyph.checked_sub(u32::from(self.first)).and_then(|i| self.classes.get(i as usize));
        class.map_or(class::OUT_OF_BOUNDS, |&k| k.into())
    }

    fn n_classes(&self) -> usize {
        usize::from(self.n_classes)
    }

    fn collect_classes_where(&self, out: &mut Vec<(u32, u32)>, keep: &dyn Fn(u32) -> bool) {
        let glyphs = self.classes.iter().zip(u32::from(self.first)..).filter(|&(&k, _)| keep(k.into()));
        out.extend(glyphs.map(|(_, g)| (g, g)));
    }

    // A state's row is no more than 65,535 bytes in, as `next_state` divides a 16-bit offset, and the rest are bytes
    // and offsets within the table, so no sum here can overflow.
    fn entry_at(&self, state: i32, klass: u16) -> Option<Entry> {
        if klass >= self.n_classes {
            return None;
        }
        let row = usize::try_from(state).ok()? * usize::from(self.n_classes);
        let index = *self.data.get(self.states + row + usize::from(klass))?;
        let e = window::<4>(self.data, self.entries + usize::from(index) * 4)?;
        Some(Entry {
            new_state: u16::from_be_bytes([e[0], e[1]]),
            flags: u16::from_be_bytes([e[2], e[3]]),
            word1: 0,
            word2: 0,
        })
    }

    fn next_state(&self, e: &Entry) -> i32 {
        (i32::from(e.new_state) - self.state_offset).div_euclid(i32::from(self.n_classes))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/aat.rs"]
mod tests;
