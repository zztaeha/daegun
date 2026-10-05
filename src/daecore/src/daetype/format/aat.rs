use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be, window};

pub mod class {
    pub const END_OF_TEXT: u16 = 0;
    pub(crate) const OUT_OF_BOUNDS: u16 = 1;
    pub const DELETED_GLYPH: u16 = 2;
}

pub mod state {
    pub const START_OF_TEXT: u16 = 0;
}

#[derive(Clone, Copy)]
pub struct Lookup<'a> {
    data: &'a [u8],
    format: u16,
    num_glyphs: u16,
    bin_srch: Option<(usize, usize)>,
}

impl<'a> Lookup<'a> {
    pub fn parse(data: &'a [u8], num_glyphs: u16) -> Option<Self> {
        let format = read_u16_be(data, 0)?;
        if !matches!(format, 0 | 2 | 4 | 6 | 8 | 10) {
            return None;
        }
        let bin_srch = parse_bin_srch(data);
        Some(Lookup { data, format, num_glyphs, bin_srch })
    }

    pub fn value(&self, glyph: u16) -> Option<u16> {
        match self.format {
            // Format 0 is an array with no length of its own – it runs to the end of the font's
            // glyph range, so without this bound a glyph past that reads whatever bytes follow.
            0 if glyph < self.num_glyphs => read_u16_be(self.data, 2 + 2 * usize::from(glyph)),
            0 => None,
            2 => self.segment_single(glyph),
            4 => self.segment_array(glyph),
            6 => self.single_table(glyph),
            8 => self.trimmed_array(glyph),
            10 => self.trimmed_array_wide(glyph),
            _ => None,
        }
    }

    // Each glyph once, in order, with the value `value` answers for it, so overlapping or repeated
    // segments cannot list more than the 65,535 glyphs there are.
    pub fn entries(&self) -> Vec<(u16, u16)> {
        let mut out = Vec::new();
        let push = |g: u16, out: &mut Vec<(u16, u16)>| {
            if let Some(v) = self.value(g) { out.push((g, v)); }
        };
        match self.format {
            0 => for g in 0..self.num_glyphs { push(g, &mut out); },
            2 | 4 | 6 => {
                let Some((unit, n)) = self.bin_srch() else { return out };
                let mut next = 0u16;
                for i in 0..n {
                    let rec = 12 + i * unit;
                    let (first, last) = if self.format == 6 {
                        let Some(g) = read_u16_be(self.data, rec) else { break };
                        (g, g)
                    } else {
                        let (Some(last), Some(first)) =
                            (read_u16_be(self.data, rec), read_u16_be(self.data, rec + 2))
                        else { break };
                        (first, last)
                    };
                    let last = last.min(0xFFFE);
                    if first > last || last < next { continue; }
                    for g in first.max(next)..=last { push(g, &mut out); }
                    next = last + 1;
                }
            }
            8 | 10 => {
                let at = if self.format == 8 { 2 } else { 4 };
                let (Some(first), Some(count)) =
                    (read_u16_be(self.data, at), read_u16_be(self.data, at + 2)) else { return out };
                for i in 0..count {
                    let Some(g) = first.checked_add(i) else { break };
                    push(g, &mut out);
                }
            }
            _ => {}
        }
        out
    }

    fn bin_srch(&self) -> Option<(usize, usize)> {
        self.bin_srch
    }

    fn lower_bound(&self, glyph: u16, unit: usize, n: usize) -> Option<usize> {
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if read_u16_be(self.data, 12 + mid * unit)? < glyph {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo < n).then(|| 12 + lo * unit)
    }

    fn segment_single(&self, glyph: u16) -> Option<u16> {
        let (unit, n) = self.bin_srch()?;
        if unit < 6 {
            return None;
        }
        let at = self.lower_bound(glyph, unit, n)?;
        let seg = window::<4>(self.data, at)?;
        let last = u16::from_be_bytes([seg[0], seg[1]]);
        let first = u16::from_be_bytes([seg[2], seg[3]]);
        if first == 0xFFFF && last == 0xFFFF {
            return None;
        }
        (first <= glyph).then(|| read_u16_be(self.data, at + 4))?
    }

    fn segment_array(&self, glyph: u16) -> Option<u16> {
        let (unit, n) = self.bin_srch()?;
        if unit < 6 {
            return None;
        }
        let at = self.lower_bound(glyph, unit, n)?;
        let seg = window::<4>(self.data, at)?;
        let last = u16::from_be_bytes([seg[0], seg[1]]);
        let first = u16::from_be_bytes([seg[2], seg[3]]);
        if first == 0xFFFF && last == 0xFFFF {
            return None;
        }
        if first > glyph {
            return None;
        }
        let values = usize::from(read_u16_be(self.data, at + 4)?);
        read_u16_be(self.data, values + 2 * usize::from(glyph - first))
    }

    fn single_table(&self, glyph: u16) -> Option<u16> {
        let (unit, n) = self.bin_srch()?;
        if unit < 4 {
            return None;
        }
        let at = self.lower_bound(glyph, unit, n)?;
        let g = read_u16_be(self.data, at)?;
        if g != glyph || g == 0xFFFF {
            return None;
        }
        read_u16_be(self.data, at + 2)
    }

    fn trimmed_array(&self, glyph: u16) -> Option<u16> {
        let h = window::<4>(self.data, 2)?;
        let first = u16::from_be_bytes([h[0], h[1]]);
        let count = u16::from_be_bytes([h[2], h[3]]);
        let i = glyph.checked_sub(first)?;
        (i < count).then(|| read_u16_be(self.data, 6 + 2 * usize::from(i)))?
    }

    fn trimmed_array_wide(&self, glyph: u16) -> Option<u16> {
        let h = window::<6>(self.data, 2)?;
        let unit = u16::from_be_bytes([h[0], h[1]]);
        let first = u16::from_be_bytes([h[2], h[3]]);
        let count = u16::from_be_bytes([h[4], h[5]]);
        let i = glyph.checked_sub(first)?;
        if i >= count {
            return None;
        }
        let at = 8 + usize::from(i) * usize::from(unit);
        match unit {
            1 => self.data.get(at).map(|b| u16::from(*b)),
            2 => read_u16_be(self.data, at),
            4 => read_u32_be(self.data, at).map(|v| v as u16),
            _ => None,
        }
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

#[derive(Clone, Copy, Default)]
pub struct Entry {
    pub new_state: u16,
    pub flags: u16,
    pub word1: u16,
    pub word2: u16,
}

pub struct StateTable<'a> {
    class_lookup: Lookup<'a>,
    state_array: &'a [u8],
    entry_table: &'a [u8],
    n_classes: usize,
    extra_words: usize,
    stride: usize,
}

impl<'a> StateTable<'a> {
    pub fn parse(data: &'a [u8], extra_words: usize, num_glyphs: u16) -> Option<Self> {
        let h = window::<16>(data, 0)?;
        let word = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]) as usize;
        let (n_classes, class_off, state_off, entry_off) = (word(0), word(4), word(8), word(12));
        if n_classes == 0 || n_classes > 0xFFFF {
            return None;
        }
        // An entry is its new state and flags, then two bytes per extra word, which a caller names.
        let stride = extra_words.checked_mul(2)?.checked_add(4)?;
        Some(StateTable {
            class_lookup: Lookup::parse(data.get(class_off..)?, num_glyphs)?,
            state_array: data.get(state_off..)?,
            entry_table: data.get(entry_off..)?,
            n_classes,
            extra_words,
            stride,
        })
    }

    pub fn class(&self, glyph: u16) -> u16 {
        if glyph == 0xFFFF {
            return class::DELETED_GLYPH;
        }
        self.class_lookup.value(glyph).unwrap_or(class::OUT_OF_BOUNDS)
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn words(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    // A binary-searched lookup of the given format with `units` of `size` words each.
    fn searched(format: u16, units: &[&[u16]], tail: &[u16]) -> Vec<u8> {
        let size = units.first().map_or(0, |u| u.len() as u16 * 2);
        let mut t = words(&[format, size, units.len() as u16, 0, 0, 0]);
        units.iter().for_each(|u| t.extend(words(u)));
        t.extend(words(tail));
        t
    }

    #[test]
    fn overlapping_segments_list_each_glyph_once() {
        let segments = vec![&[0xFFFE, 0, 1][..]; 50];
        let bytes = searched(2, &segments, &[]);
        let lookup = Lookup::parse(&bytes, 0xFFFF).unwrap();
        let entries = lookup.entries();
        assert_eq!(entries.len(), 0xFFFF, "every glyph below 0xFFFF once");
        assert!(entries.iter().all(|&(g, v)| lookup.value(g) == Some(v)));
    }

    #[test]
    fn a_segment_ending_at_ffff_lists_the_glyphs_value_answers_for() {
        let bytes = searched(2, &[&[0xFFFF, 0xFFF0, 7], &[0xFFFF, 0xFFFF, 0]], &[]);
        let lookup = Lookup::parse(&bytes, 0xFFFF).unwrap();
        assert_eq!(lookup.value(0xFFF5), Some(7));
        assert_eq!(lookup.entries(), (0xFFF0..=0xFFFE).map(|g| (g, 7)).collect::<Vec<_>>());
    }

    #[test]
    fn every_lookup_format_reads_its_values() {
        let simple = words(&[0, 10, 11, 12]);
        let simple = Lookup::parse(&simple, 3).unwrap();
        assert_eq!((0..4).map(|g| simple.value(g)).collect::<Vec<_>>(), [Some(10), Some(11), Some(12), None]);

        let single = searched(2, &[&[5, 3, 0x11], &[9, 8, 0x22], &[0xFFFF, 0xFFFF, 0]], &[]);
        let single = Lookup::parse(&single, 20).unwrap();
        assert_eq!([2, 3, 5, 7, 8, 9, 10].map(|g| single.value(g)), [None, Some(0x11), Some(0x11), None, Some(0x22), Some(0x22), None]);
        assert_eq!(single.entries(), [(3, 0x11), (4, 0x11), (5, 0x11), (8, 0x22), (9, 0x22)]);

        let array = searched(4, &[&[5, 3, 30], &[8, 8, 36], &[0xFFFF, 0xFFFF, 0]], &[0x31, 0x32, 0x33, 0x41]);
        let array = Lookup::parse(&array, 20).unwrap();
        assert_eq!(array.entries(), [(3, 0x31), (4, 0x32), (5, 0x33), (8, 0x41)]);

        let table = searched(6, &[&[2, 0x52], &[7, 0x57], &[0xFFFF, 0]], &[]);
        assert_eq!(Lookup::parse(&table, 20).unwrap().entries(), [(2, 0x52), (7, 0x57)]);

        let trimmed = words(&[8, 4, 2, 0x84, 0x85]);
        let trimmed = Lookup::parse(&trimmed, 20).unwrap();
        assert_eq!(trimmed.entries(), [(4, 0x84), (5, 0x85)]);

        let mut wide = words(&[10, 1, 6, 3]);
        wide.extend([0xA6, 0xA7, 0xA8]);
        assert_eq!(Lookup::parse(&wide, 20).unwrap().entries(), [(6, 0xA6), (7, 0xA7), (8, 0xA8)]);
        let mut long = words(&[10, 4, 6, 1]);
        long.extend([0, 1, 0xB0, 0x0B]);
        assert_eq!(Lookup::parse(&long, 20).unwrap().value(6), Some(0xB00B));
    }

    #[test]
    fn a_state_entry_reads_both_its_extra_words() {
        let mut t = vec![0, 0, 0, 5, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0, 44];
        t.extend(words(&[8, 4, 1, 4]));
        t.extend(words(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0]));
        t.extend(words(&[0, 0, 0, 0, 1, 0x4000, 0x0111, 0x0222]));
        let table = StateTable::parse(&t, 2, 10).unwrap();
        assert_eq!(table.class(4), 4);
        let e = table.entry(0, 4).unwrap();
        assert_eq!((e.new_state, e.flags, e.word1, e.word2), (1, 0x4000, 0x0111, 0x0222));
    }
}
