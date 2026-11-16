use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::super::decoder::{read_u16_be, read_u32_be, records_fit};

// EBLC, CBLC and bloc locate glyph images in EBDT, CBDT and bdat. Each strike has a BitmapSize record
// and an array of index subtables, each over a range of glyphs.

// Each strike's BitmapSize record; None when the stated count overruns the table.
pub fn bitmap_sizes(loc: &[u8]) -> Option<&[[u8; 48]]> {
    let count = read_u32_be(loc, 4)? as usize;
    loc.get(8..)?.as_chunks::<48>().0.get(..count)
}

// The ppem a BitmapSize record gives along x and y.
pub fn ppem(size: &[u8; 48]) -> (u8, u8) {
    (size[44], size[45])
}

// A strike's IndexSubTableArray: each record a first and last glyph and where its subtable starts.
pub struct IndexRecords<'a> {
    array: usize,
    records: &'a [[u8; 8]],
}

// A strike's index records; None when the array overruns the table.
pub fn index_records<'a>(loc: &'a [u8], size: &[u8; 48]) -> Option<IndexRecords<'a>> {
    let (array, count) = (read_u32_be(size, 0)? as usize, read_u32_be(size, 8)? as usize);
    Some(IndexRecords { array, records: loc.get(array..)?.as_chunks::<8>().0.get(..count)? })
}

fn range(r: &[u8; 8]) -> (u16, u16) {
    (u16::from_be_bytes([r[0], r[1]]), u16::from_be_bytes([r[2], r[3]]))
}

impl IndexRecords<'_> {
    pub fn count(&self) -> usize {
        self.records.len()
    }

    // Saturating, an offset past the address space lands past the table.
    fn record(&self, r: &[u8; 8]) -> (u16, u16, usize) {
        let (first, last) = range(r);
        (first, last, self.array.saturating_add(u32::from_be_bytes([r[4], r[5], r[6], r[7]]) as usize))
    }

    // Each record in order: its first glyph, last glyph and where its subtable starts.
    pub fn iter(&self) -> impl Iterator<Item = (u16, u16, usize)> + '_ {
        self.records.iter().map(|r| self.record(r))
    }

    // The record a strike looks a glyph up in: the first whose range covers it, even one holding no
    // image for the glyph.
    pub fn covering(&self, gid: u32) -> Option<(u16, u16, usize)> {
        let r = self.records.iter().find(|r| {
            let (first, last) = range(r);
            (u32::from(first)..=u32::from(last)).contains(&gid)
        })?;
        Some(self.record(r))
    }
}

// The glyph ranges earlier index records cover. Of a glyph two records cover, the first decides, as
// `IndexRecords::covering` finds it.
#[derive(Default)]
pub struct Claims(BTreeMap<u16, u16>);

impl Claims {
    // The parts of first..=last no earlier record covers, in order, which this record decides.
    pub fn claim(&mut self, first: u16, last: u16) -> Vec<(u16, u16)> {
        let mut near: Vec<(u16, u16)> = self.0.range(..=last).rev()
            .take_while(|&(_, &end)| u32::from(end) + 1 >= u32::from(first))
            .map(|(&start, &end)| (start, end))
            .collect();
        near.reverse();
        let (mut gaps, mut at) = (Vec::new(), u32::from(first));
        for &(start, end) in &near {
            if u32::from(start) > at {
                gaps.push((at as u16, start - 1));
            }
            at = at.max(u32::from(end) + 1);
        }
        if at <= u32::from(last) {
            gaps.push((at as u16, last));
        }
        let (mut lo, mut hi) = (first, last);
        for (start, end) in near {
            self.0.remove(&start);
            (lo, hi) = (lo.min(start), hi.max(end));
        }
        self.0.insert(lo, hi);
        gaps
    }
}

// One index subtable over first..=last, with the data table its images lie in.
pub struct IndexSubtable<'a> {
    loc: &'a [u8],
    data: &'a [u8],
    at: usize,
    first: u16,
    last: u16,
    pub index_format: u16,
    pub image_format: u16,
    image_data: usize,
    // Glyph entries the index lists: one per glyph of the range, or a sparse index's count.
    count: usize,
}

impl<'a> IndexSubtable<'a> {
    // None for an unknown index format, a backward range, or a header, fixed metrics or sparse list
    // the table cuts short. Offsets of formats 1 and 3 are read glyph by glyph.
    pub fn new(loc: &'a [u8], data: &'a [u8], at: usize, first: u16, last: u16) -> Option<IndexSubtable<'a>> {
        let (index_format, image_format) = (read_u16_be(loc, at)?, read_u16_be(loc, at + 2)?);
        let image_data = read_u32_be(loc, at + 4)? as usize;
        let span = usize::from(last.checked_sub(first)?) + 1;
        let count = match index_format {
            1 | 3 => span,
            2 => loc.get(at + 12..at + 20).map(|_| span)?,
            4 => {
                let n = read_u32_be(loc, at + 8)? as usize;
                records_fit(at + 12, n.checked_add(1)?, 4, loc.len()).then_some(n)?
            }
            5 => {
                let n = read_u32_be(loc, at + 20)? as usize;
                records_fit(at + 24, n, 2, loc.len()).then_some(n)?
            }
            _ => return None,
        };
        Some(IndexSubtable { loc, data, at, first, last, index_format, image_format, image_data, count })
    }

    pub fn entry_count(&self) -> usize {
        self.count
    }

    // Formats 2 and 5 give every glyph one image size and the big metrics it is drawn with.
    pub fn fixed(&self) -> Option<(u32, &'a [u8; 8])> {
        if !matches!(self.index_format, 2 | 5) {
            return None;
        }
        Some((read_u32_be(self.loc, self.at + 8)?, self.loc.get(self.at + 12..self.at + 20)?.try_into().ok()?))
    }

    // The `k`th entry's glyph, and its image's start and end past the subtable's image data. With `k`
    // below the entry count, a dense index's glyph is within its range.
    fn entry(&self, k: usize) -> Option<(u16, usize, usize)> {
        let (loc, at) = (self.loc, self.at);
        let dense = || self.first + k as u16;
        let fixed = || {
            let size = read_u32_be(loc, at + 8)? as usize;
            let start = k.checked_mul(size)?;
            Some((start, start.checked_add(size)?))
        };
        let (gid, (start, end)) = match self.index_format {
            1 => (dense(), (read_u32_be(loc, at + 8 + 4 * k)? as usize, read_u32_be(loc, at + 12 + 4 * k)? as usize)),
            3 => (dense(), (read_u16_be(loc, at + 8 + 2 * k)?.into(), read_u16_be(loc, at + 10 + 2 * k)?.into())),
            2 => (dense(), fixed()?),
            4 => {
                let offset = |j: usize| read_u16_be(loc, at + 14 + 4 * j).map(usize::from);
                (read_u16_be(loc, at + 12 + 4 * k)?, (offset(k)?, offset(k + 1)?))
            }
            _ => (read_u16_be(loc, at + 24 + 2 * k)?, fixed()?),
        };
        Some((gid, start, end))
    }

    // An image's bytes in the data table; an empty or backward range is no image.
    fn image(&self, start: usize, end: usize) -> Option<&'a [u8]> {
        if end <= start {
            return None;
        }
        self.data.get(self.image_data.checked_add(start)?..self.image_data.checked_add(end)?)
    }

    // A glyph's image, if this subtable holds one. A sparse index is searched at a unit of `work` an
    // entry, and its first entry for the glyph is the one.
    pub fn locate(&self, gid: u32, work: &mut usize) -> Option<&'a [u8]> {
        let k = match self.index_format {
            4 | 5 => {
                *work = work.checked_sub(self.count)?;
                (0..self.count).find(|&k| self.entry(k).is_some_and(|e| u32::from(e.0) == gid))?
            }
            _ => gid.checked_sub(u32::from(self.first))? as usize,
        };
        if k >= self.count {
            return None;
        }
        let (_, start, end) = self.entry(k)?;
        self.image(start, end)
    }

    // Each glyph of the range this subtable holds an image for, in index order, with the image
    // `locate` finds for it. An entry the table cuts short ends the list.
    pub fn entries(&self) -> impl Iterator<Item = (u16, &'a [u8])> + '_ {
        let mut seen = BTreeSet::new();
        let sparse = matches!(self.index_format, 4 | 5);
        (0..self.count)
            .map_while(|k| self.entry(k))
            .filter(move |&(gid, ..)| (self.first..=self.last).contains(&gid) && (!sparse || seen.insert(gid)))
            .filter_map(|(gid, start, end)| Some((gid, self.image(start, end)?)))
    }
}

// A composite image's components (EBDT formats 8 and 9): each one's place in the image, glyph, and x
// and y offset. Another format has none. None when the list runs past the image.
pub fn components(image_format: u16, image: &[u8]) -> Option<impl ExactSizeIterator<Item = (usize, u16, i8, i8)> + '_> {
    let (at, records): (usize, &[[u8; 4]]) = match image_format {
        8 | 9 => {
            let at = if image_format == 8 { 6 } else { 8 };
            let count = usize::from(read_u16_be(image, at)?);
            (at, image.get(at + 2..)?.as_chunks::<4>().0.get(..count)?)
        }
        _ => (0, &[]),
    };
    let component = move |(k, &[g0, g1, dx, dy]): (usize, &[u8; 4])| {
        (at + 2 + 4 * k, u16::from_be_bytes([g0, g1]), dx as i8, dy as i8)
    };
    Some(records.iter().enumerate().map(component))
}
