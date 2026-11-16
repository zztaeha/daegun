use alloc::vec::Vec;
use super::super::decoder::{
    glyf_tags, num_glyphs, read_i16_be, read_u16_be, read_u32_be, records_fit, write_u16_be, write_u32_be,
};
use crate::daecore::daetype::{ByTag, TableBytes, Tables};

// hmtx or vmtx with its header's count of long records: past them a glyph takes the last advance and a bearing from
// the array after. Each array is absent where its count leaves the table, the long records taking every metric with
// them; a glyph past `glyphs` has none, and one past `bearings` a bearing of 0.
#[derive(Clone, Copy)]
pub(crate) struct MetricsTable<'a> {
    data: &'a [u8],
    long: usize,
    glyphs: usize,
    bearings: usize,
}

// Where hhea and vhea hold their count of long metrics, numberOfHMetrics and numOfLongVerMetrics.
pub(crate) const LONG_METRICS: usize = 34;

// One direction's header and metrics table, the beyond-64K pair counting its long records in 32 bits.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct MetricTags {
    pub(crate) hea: &'static str,
    pub(crate) mtx: &'static str,
    pub(crate) wide: bool,
}

impl MetricTags {
    pub(crate) const fn lower(vertical: bool) -> MetricTags {
        match vertical {
            false => MetricTags { hea: "hhea", mtx: "hmtx", wide: false },
            true => MetricTags { hea: "vhea", mtx: "vmtx", wide: false },
        }
    }

    pub(crate) const fn upper(vertical: bool) -> MetricTags {
        match vertical {
            false => MetricTags { hea: "HHEA", mtx: "HMTX", wide: true },
            true => MetricTags { hea: "VHEA", mtx: "VMTX", wide: true },
        }
    }

    // The count of long records as the header states it, unclamped, wherever it holds one.
    pub(crate) fn stated(self, hea: &[u8]) -> Option<usize> {
        match self.wide {
            false => read_u16_be(hea, LONG_METRICS).map(usize::from),
            true => read_u32_be(hea, LONG_METRICS).and_then(|n| usize::try_from(n).ok()),
        }
    }

    // The header at major 1, the one hhea and vhea define, and long enough for its count.
    pub(crate) fn header(self, map: &impl ByTag) -> Option<&[u8]> {
        let hea = map.table(self.hea)?;
        (read_u16_be(hea, 0) == Some(1) && self.stated(hea).is_some()).then_some(hea)
    }

    // The count shaping reads, 0 from a header that doesn't read whole.
    pub(crate) fn long(self, map: &impl ByTag) -> usize {
        self.header(map).and_then(|hea| self.stated(hea)).unwrap_or(0)
    }

    // `long` written into the header where it has room: 16 bits, capped, or 32 for the beyond-64K pair.
    pub(crate) fn write_long(self, hea: &mut [u8], long: usize) {
        match self.wide {
            false if hea.len() >= LONG_METRICS + 2 => { write_u16_be(hea, LONG_METRICS, long.min(0xFFFF) as u16); }
            true if hea.len() >= LONG_METRICS + 4 => {
                write_u32_be(hea, LONG_METRICS, long.min(u32::MAX as usize) as u32);
            }
            _ => {}
        }
    }

    // The metrics table by that count and the font's glyph count, as `MetricsTable::read` reads it. The table is
    // looked up first, so a font without one doesn't work out the count.
    pub(crate) fn table(self, map: &impl ByTag) -> Option<MetricsTable<'_>> {
        let mtx = map.table(self.mtx)?;
        MetricsTable::read(mtx, self.long(map), num_glyphs(map).map_or(0, |n| n as usize))
    }
}

// The pair shaping, measuring, subsetting and instancing read one direction from: HMTX (VMTX) where it gives a glyph
// a metric by HHEA's count, else hmtx.
pub(crate) fn shaped_tags(map: &impl ByTag, vertical: bool) -> MetricTags {
    let upper = MetricTags::upper(vertical);
    match upper.table(map).is_some_and(|t| t.has_data()) {
        true => upper,
        false => MetricTags::lower(vertical),
    }
}

// The header whose fields shaping reads: HHEA (VHEA) where it reads whole, else hhea where it does; else none.
pub(crate) fn shaped_header(map: &impl ByTag, vertical: bool) -> Option<&[u8]> {
    MetricTags::upper(vertical).header(map).or_else(|| MetricTags::lower(vertical).header(map))
}

// The header, metrics table and count drawing reads one way: beside GLYF, HHEA and HMTX alone; else hhea and hmtx
// (vhea and vmtx), or where that header is missing, HHEA and HMTX.
pub(crate) fn drawn_metrics(
    map: &Tables, vertical: bool,
) -> Option<(&TableBytes, Option<&TableBytes>, Option<usize>)> {
    let (lower, upper) = (MetricTags::lower(vertical), MetricTags::upper(vertical));
    let tags = match map.contains_key(lower.hea) && !glyf_tags(map).extended {
        true => lower,
        false => upper,
    };
    let hea = map.get(tags.hea)?;
    Some((hea, map.get(tags.mtx), tags.stated(hea)))
}

// The ascender and descender drawing falls back on without vertical metrics: OS/2's typo pair, else hhea's.
pub(crate) fn drawn_extents(map: &Tables) -> (i16, i16) {
    let os2 = map.get("OS/2").map_or(&[][..], |t| &t[..]);
    match (read_i16_be(os2, 68), read_i16_be(os2, 70)) {
        (Some(ascender), Some(descender)) => (ascender, descender),
        _ => {
            let hhea = drawn_metrics(map, false).map_or(&[][..], |(hea, ..)| &hea[..]);
            (read_i16_be(hhea, 4).unwrap_or(0), read_i16_be(hhea, 6).unwrap_or(0))
        }
    }
}

impl<'a> MetricsTable<'a> {
    // A table for no glyph count: every glyph past the long records takes the last advance, and the bearings run on
    // to the table's end.
    pub(crate) fn new(data: &'a [u8], long: usize) -> MetricsTable<'a> {
        let long = if long_fits(data, long) { long } else { 0 };
        let bearings = if long == 0 { 0 } else { long + (data.len() - 4 * long) / 2 };
        MetricsTable { data, long, glyphs: if long == 0 { 0 } else { usize::MAX }, bearings }
    }

    // A table for `count` glyphs, by its header's `stated` count of long records; None where no glyph has a metric.
    pub(crate) fn read(data: &'a [u8], stated: usize, count: usize) -> Option<MetricsTable<'a>> {
        let long = Some(stated).filter(|&n| n > 0 && long_fits(data, n))?;
        let glyphs = count.max(long);
        let bearings = if records_fit(4 * long, glyphs - long, 2, data.len()) { glyphs } else { long };
        Some(MetricsTable { data, long, glyphs, bearings })
    }

    // Whether any glyph has a metric here.
    pub(crate) fn has_data(&self) -> bool {
        self.glyphs > 0
    }

    // How many glyphs have a metric here.
    pub(crate) fn len(&self) -> usize {
        self.glyphs
    }

    pub(crate) fn long(&self) -> usize {
        self.long
    }

    pub(crate) fn advance(&self, gid: usize) -> u16 {
        let Some(last) = self.long.checked_sub(1).filter(|_| gid < self.glyphs) else { return 0 };
        read_u16_be(self.data, 4 * gid.min(last)).unwrap_or(0)
    }

    pub(crate) fn bearing(&self, gid: usize) -> i16 {
        let at = bearing_at(self.long, gid).filter(|_| gid < self.bearings);
        at.and_then(|at| read_i16_be(self.data, at)).unwrap_or(0)
    }

    pub(crate) fn pair(&self, gid: usize) -> (u16, i16) {
        (self.advance(gid), self.bearing(gid))
    }
}

fn long_fits(data: &[u8], long: usize) -> bool {
    records_fit(0, long, 4, data.len())
}

// Where a glyph's bearing is stored, for a table with `long` long records; None where there are none.
pub(crate) fn bearing_at(long: usize, gid: usize) -> Option<usize> {
    (long > 0).then(|| if gid < long { 4 * gid + 2 } else { 4 * long + 2 * (gid - long) })
}

// hmtx or vmtx with its `long` records cut back to the last advance that changes, those past it written as bearings
// alone, the smallest the table can be; with the count of long records, unchanged where none can go.
pub(crate) fn compacted(mtx: Vec<u8>, long: usize) -> (Vec<u8>, usize) {
    if mtx.len() < 4 * long {
        return (mtx, long);
    }
    let advance = |gid: usize| &mtx[4 * gid..4 * gid + 2];
    let mut needed = long;
    while needed > 1 && advance(needed - 1) == advance(needed - 2) {
        needed -= 1;
    }
    if needed == long {
        return (mtx, long);
    }
    let mut out = mtx[..4 * needed].to_vec();
    out.extend((needed..long).flat_map(|gid| [mtx[4 * gid + 2], mtx[4 * gid + 3]]));
    out.extend_from_slice(&mtx[4 * long..]);
    (out, needed)
}

// The same metrics for a 16-bit count of long records: past 65,535 a record keeps its bearing alone, the count cut
// back again as `compacted` cuts it.
pub(crate) fn narrowed(mtx: Vec<u8>, long: usize) -> (Vec<u8>, usize) {
    const MOST: usize = 0xFFFF;
    if long <= MOST || mtx.len() < 4 * long {
        return (mtx, long.min(MOST));
    }
    let mut out = mtx[..4 * MOST].to_vec();
    out.extend((MOST..long).flat_map(|gid| [mtx[4 * gid + 2], mtx[4 * gid + 3]]));
    out.extend_from_slice(&mtx[4 * long..]);
    compacted(out, MOST)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/metrics.rs"]
mod tests;
