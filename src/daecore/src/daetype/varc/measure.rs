// VARC's records read strictly, as a subset rewrites them once the table is sanitized.
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be, read_u24_be, read_u32_be};
use crate::daecore::daetype::format::coverage::coverage_index;
use super::uint32var;

pub(super) const NO_VARIATION: u32 = u32::MAX;
pub(super) const HAVE_AXES: u32 = 1 << 1;
pub(super) const AXIS_VALUES_HAVE_VARIATION: u32 = 1 << 2;
pub(super) const TRANSFORM_HAS_VARIATION: u32 = 1 << 3;
pub(super) const HAVE_CONDITION: u32 = 1 << 7;
const HAVE_SCALE_Y: u32 = 1 << 9;
pub(super) const GID_IS_24BIT: u32 = 1 << 12;
const RESERVED: u32 = 0xFFFF_8000;
// The transform's fields in record order: each one's flag and the bits its value keeps after the point.
const FIELDS: [(u32, u32); 9] = [
    (1 << 4, 0),
    (1 << 5, 0),
    (1 << 6, 12),
    (1 << 8, 10),
    (HAVE_SCALE_Y, 10),
    (1 << 13, 12),
    (1 << 14, 12),
    (1 << 10, 0),
    (1 << 11, 0),
];

// The sanitized table's parts, each None where its offset is null.
pub(crate) struct Table<'a> {
    pub(super) t: &'a [u8],
    pub(super) coverage: Option<usize>,
    pub(super) store: Option<usize>,
    pub(super) conditions: Option<usize>,
    pub(super) axis_indices: Option<usize>,
    pub(super) records: Option<usize>,
}

impl<'a> Table<'a> {
    pub(crate) fn new(t: &'a [u8]) -> Table<'a> {
        let at = |i: usize| read_u32_be(t, 4 + 4 * i).filter(|&o| o != 0).map(|o| (o as usize).min(t.len()));
        Table { t, coverage: at(0), store: at(1), conditions: at(2), axis_indices: at(3), records: at(4) }
    }

    // Where an offset lands: nothing reads past the table, so one that far stops at its end, and no sum after it can
    // overflow a 32-bit usize.
    pub(super) fn at(&self, base: usize, off: u32) -> usize {
        base.saturating_add(off as usize).min(self.t.len())
    }

    pub(super) fn u16(&self, at: usize) -> u16 {
        read_u16_be(self.t, at).unwrap_or(0)
    }

    pub(super) fn u32(&self, at: usize) -> u32 {
        read_u32_be(self.t, at).unwrap_or(0)
    }

    pub(super) fn coverage(&self, gid: u32) -> Option<u32> {
        coverage_index(self.t.get(self.coverage?..)?, gid)
    }

    // INDEX entry `i`, empty where the INDEX is null or the entry's offsets run backward or past the last.
    pub(super) fn entry(&self, at: Option<usize>, i: u32) -> &'a [u8] {
        let Some(at) = at.filter(|&at| i < self.u32(at)) else { return &[] };
        let (count, size) = (self.u32(at) as usize, usize::from(self.t.get(at + 4).copied().unwrap_or(0)));
        let byte = |at: usize| usize::from(self.t.get(at).copied().unwrap_or(0));
        let offset = |k: usize| {
            let entry = (at + 5).saturating_add(k.saturating_mul(size));
            (0..size).fold(0, |v, b| v << 8 | byte(entry.saturating_add(b)))
        };
        let (from, to) = (offset(i as usize), offset(i as usize + 1));
        if to < from || to > offset(count) {
            return &[];
        }
        let base = (at + 4).saturating_add(count.saturating_add(1).saturating_mul(size));
        self.t.get(base.saturating_add(from)..base.saturating_add(to)).unwrap_or_default()
    }

    // Where condition `index` starts: None for a null one, or one past the list, either of which holds.
    pub(super) fn condition(&self, index: u32) -> Option<usize> {
        let list = self.conditions?;
        if index >= self.u32(list) {
            return None;
        }
        match self.u32((list + 4).saturating_add((index as usize).saturating_mul(4))) {
            0 => None,
            off => Some(self.at(list, off)),
        }
    }

}

// TupleValues read run by run: a run cut short ends them.
struct Fetcher<'a> {
    d: &'a [u8],
    p: usize,
    run: usize,
    width: usize,
}

impl Fetcher<'_> {
    fn ensure(&mut self) -> bool {
        if self.run > 0 {
            return true;
        }
        let Some(&control) = self.d.get(self.p) else { return false };
        self.p += 1;
        (self.run, self.width) = (usize::from(control & 0x3F) + 1, width(control));
        if self.p + self.run * self.width > self.d.len() {
            self.run = 0;
            return false;
        }
        true
    }

}

fn width(control: u8) -> usize {
    match control & 0xC0 {
        0x80 => 0,
        0x40 => 2,
        0xC0 => 4,
        _ => 1,
    }
}

// How many indices an axis indices entry holds, read run by run as drawing reads it: a run cut short ends it.
fn index_count(d: &[u8]) -> usize {
    let mut f = Fetcher { d, p: 0, run: 0, width: 0 };
    let mut count = 0;
    while f.ensure() {
        count += f.run;
        (f.p, f.run) = (f.p + f.width * f.run, 0);
    }
    count
}

// A component as read: its axis list and that list's indices, its axis values, its transform's fields as stored,
// where its glyph id and four indices lie (start and length, by `GID` to `TRANSFORM_VAR`), and its length.
pub(super) struct Component {
    pub(super) flags: u32,
    pub(super) gid: u32,
    pub(super) condition: u32,
    pub(super) axis_list: u32,
    pub(super) axis_var: u32,
    pub(super) transform_var: u32,
    pub(super) spans: [(usize, usize); 5],
    pub(super) size: usize,
}

pub(super) const GID: usize = 0;
pub(super) const CONDITION: usize = 1;
pub(super) const AXIS_LIST: usize = 2;
pub(super) const AXIS_VAR: usize = 3;
pub(super) const TRANSFORM_VAR: usize = 4;

// The component at the front of `r`, read strictly; None where it doesn't read.
pub(super) fn decompile(table: &Table, r: &[u8]) -> Option<Component> {
    let (mut p, mut spans) = (0, [(0, 0); 5]);
    let mut var = |p: &mut usize, span: usize| {
        let at = *p;
        let v = uint32var(r, p)?;
        spans[span] = (at, *p - at);
        Some(v)
    };
    let flags = uint32var(r, &mut p)?;
    let (gid_at, wide) = (p, flags & GID_IS_24BIT != 0);
    let gid = if wide { read_u24_be(r, p)? } else { u32::from(read_u16_be(r, p)?) };
    let width = if wide { 3 } else { 2 };
    p += width;
    let condition = if flags & HAVE_CONDITION != 0 { var(&mut p, CONDITION)? } else { 0 };
    let mut axis_list = 0;
    if flags & HAVE_AXES != 0 {
        axis_list = var(&mut p, AXIS_LIST)?;
        strict_values(r, &mut p, index_count(table.entry(table.axis_indices, axis_list)))?;
    }
    let axis_var = if flags & AXIS_VALUES_HAVE_VARIATION != 0 { var(&mut p, AXIS_VAR)? } else { NO_VARIATION };
    let transform_var =
        if flags & TRANSFORM_HAS_VARIATION != 0 { var(&mut p, TRANSFORM_VAR)? } else { NO_VARIATION };
    for &(flag, _) in &FIELDS {
        if flags & flag != 0 {
            read_i16_be(r, p)?;
            p += 2;
        }
    }
    for _ in 0..(flags & RESERVED).count_ones() {
        uint32var(r, &mut p)?;
    }
    spans[GID] = (gid_at, width);
    let size = p;
    Some(Component { flags, gid, condition, axis_list, axis_var, transform_var, spans, size })
}

// `p` moved past exactly `count` TupleValues in `r`, checked run by run without holding them: a run past them or
// past the data refuses the record.
fn strict_values(r: &[u8], p: &mut usize, count: usize) -> Option<()> {
    let mut i = 0;
    while i < count {
        let control = *r.get(*p)?;
        *p += 1;
        let stop = i + usize::from(control & 0x3F) + 1;
        let bytes = (stop - i) * width(control);
        if stop > count || *p + bytes > r.len() {
            return None;
        }
        (*p, i) = (*p + bytes, stop);
    }
    Some(())
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/varc/measure.rs"]
mod tests;
