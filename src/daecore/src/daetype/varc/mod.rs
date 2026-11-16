// VARC, the variable composite glyphs: each covered glyph a record of components, each component another glyph drawn
// at its own axis values and transform. Read here for drawing, in 16.16 throughout.
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be, read_u24_be, read_u32_be};
use crate::daecore::daetype::format::glyf::GLYPH_WORK;
use crate::daecore::daetype::instancer::{div_fix, drawn_mul_fix as mul_fix, FIXED_ONE as ONE};

mod trig;
pub(crate) mod draw;
pub(crate) mod flatten;
pub(crate) mod measure;
pub(crate) mod subset;

pub(crate) const RESET_UNSPECIFIED_AXES: u32 = 1 << 0;
pub(crate) const HAVE_AXES: u32 = 1 << 1;
const AXIS_VALUES_HAVE_VARIATION: u32 = 1 << 2;
const TRANSFORM_HAS_VARIATION: u32 = 1 << 3;
const HAVE_CONDITION: u32 = 1 << 7;
const HAVE_SCALE_X: u32 = 1 << 8;
const HAVE_SCALE_Y: u32 = 1 << 9;
const GID_IS_24BIT: u32 = 1 << 12;
const RESERVED: u32 = 0xFFFF_8000;
const NO_VARIATION: u32 = 0xFFFF_FFFF;
// A draw's work, in units of about a value decoded: a component read and placed, a load (a leaf's call or a record's
// frame) and a condition worked out or looked up are priced as their sines, calls and lookups take.
const COMPONENT: usize = 32;
const LOAD: usize = 32;
const CONDITION: usize = 8;
// What a draw may work, past which it fails, as records naming others multiply their work without end: DECODES and
// PER_BYTE a byte of the table, room for a chain of any depth or a record of any length. Real glyphs take a few
// hundred. Their leaves' work is held apart, to what one glyph's draw may take, so a leaf draws as it does alone.
const DECODES: usize = 1 << 22;
const PER_BYTE: usize = 24;

// What the glyphs a whole instance or subset draws in VARC's place spend in all, past which it's refused: DECODES and
// 256 a byte of the font, of which real fonts spend a few.
pub(crate) fn drawn_budget(font_bytes: usize) -> usize {
    font_bytes.saturating_mul(256).saturating_add(DECODES)
}

// The transform's fields in record order: each one's flag, its scale from the record to 16.16 (26.6 for the
// translations and center), and from a 4-bit delta to the same.
const FIELDS: [(u32, i64, i64); 9] = [
    (1 << 4, 64, 4),
    (1 << 5, 64, 4),
    (1 << 6, 16, 1),
    (HAVE_SCALE_X, 64, 4),
    (HAVE_SCALE_Y, 64, 4),
    (1 << 13, 16, 1),
    (1 << 14, 16, 1),
    (1 << 10, 64, 4),
    (1 << 11, 64, 4),
];
const TX: usize = 0;
const TY: usize = 1;
const ROTATION: usize = 2;
const SX: usize = 3;
const SY: usize = 4;
const SKEW_X: usize = 5;
const SKEW_Y: usize = 6;
const CX: usize = 7;
const CY: usize = 8;

pub(crate) struct Varc<'a> {
    t: &'a [u8],
    coverage: usize,
    store: Option<usize>,
    conditions: Option<usize>,
    axis_indices: Option<usize>,
    records: usize,
    // The work done since the draw began, against its budget but for its leaves', held to GLYPH_WORK.
    pub(crate) work: Cell<usize>,
    pub(crate) leaves: Cell<usize>,
    budget: usize,
}

// Groups and negations already worked out, by where they start.
type Known = alloc::collections::BTreeMap<usize, bool>;

// One component as read, before its variations: its axis values in 16.16 and its transform's fields.
pub(crate) struct Component {
    pub(crate) flags: u32,
    pub(crate) gid: u32,
    condition: u32,
    axis_indices: u32,
    axis_values: Vec<i64>,
    axis_var: u32,
    transform_var: u32,
    fields: [i64; 9],
}

// A 16.16 matrix as [xx, xy, yx, yy], and a component's whole transform: the matrix and its offset in 26.6.
pub(crate) type Matrix = [i64; 4];

impl<'a> Varc<'a> {
    // The table, where it loads: 24 bytes at least, version 1.0, its coverage and records inside it; the store,
    // conditions and axis indices none where their offset is 0 or past the end.
    pub(crate) fn new(t: &'a [u8]) -> Option<Varc<'a>> {
        if t.len() < 24 || read_u32_be(t, 0)? != 0x10000 {
            return None;
        }
        let at = |i: usize| read_u32_be(t, 4 + 4 * i).map(|o| o as usize);
        let (coverage, records) = (at(0)?, at(4)?);
        if coverage >= t.len() || records >= t.len() {
            return None;
        }
        let optional = |i| at(i).filter(|&o| o > 0 && o < t.len());
        let (store, conditions, axis_indices) = (optional(1), optional(2), optional(3));
        let budget = t.len().saturating_mul(PER_BYTE).saturating_add(DECODES);
        let (work, leaves) = (Cell::new(0), Cell::new(0));
        Some(Varc { t, coverage, store, conditions, axis_indices, records, work, leaves, budget })
    }

    // Whether `n` more units of work keep the draw within its budget; once one doesn't, none does.
    fn spend(&self, n: usize) -> bool {
        self.work.set(self.work.get().saturating_add(n));
        !self.spent()
    }

    // A leaf's work, as `spend`.
    pub(crate) fn spend_leaf(&self, n: usize) -> bool {
        self.leaves.set(self.leaves.get().saturating_add(n));
        self.spend(n)
    }

    pub(crate) fn spent(&self) -> bool {
        self.work.get().saturating_sub(self.leaves.get()) > self.budget || self.leaves.get() > GLYPH_WORK
    }

    // One more than the units the budget leaves: a decode cut there and spent shows the draw past it.
    fn room(&self) -> usize {
        self.budget.saturating_add(1).saturating_sub(self.work.get().saturating_sub(self.leaves.get()))
    }

    // A new draw, nothing worked yet.
    pub(crate) fn start(&self) {
        self.work.set(0);
        self.leaves.set(0);
    }

    // The glyph's coverage index, a range's taken from its start without a check against the records.
    pub(crate) fn coverage(&self, gid: u32) -> Option<u32> {
        let (t, p) = (self.t, self.coverage);
        if p + 4 > t.len() {
            return None;
        }
        let format = read_u16_be(t, p)?;
        let size = if matches!(format, 3 | 4) { 3 } else { 2 };
        let read = |at: usize| if size == 3 { read_u24_be(t, at) } else { read_u16_be(t, at).map(u32::from) };
        let count = read(p + 2)? as usize;
        let first = p + 2 + size;
        let record = if matches!(format, 1 | 3) { size } else { 3 * size };
        if !matches!(format, 1..=4) || first + count * record > t.len() {
            return None;
        }
        let (mut lo, mut hi) = (0, count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let at = first + mid * record;
            let (start, end) = (read(at)?, if record == size { read(at)? } else { read(at + size)? });
            if gid < start {
                hi = mid;
            } else if gid > end {
                lo = mid + 1;
            } else {
                return Some(if record == size { mid as u32 } else { read(at + 2 * size)? + gid - start });
            }
        }
        None
    }

    // Entry `index` of the INDEX at `at`, as where its bytes start and how many: offsets 1-based, all inside the
    // table.
    fn entry(&self, at: usize, index: u32) -> Option<(usize, usize)> {
        let t = self.t;
        let count = read_u32_be(t, at)?;
        let size = usize::from(*t.get(at + 4)?);
        let p = at + 5;
        if !(1..=4).contains(&size) || index >= count || count as usize >= (t.len() - p) / size {
            return None;
        }
        let base = p + (count as usize + 1) * size;
        let read = |at: usize| t[at..at + size].iter().fold(0u64, |v, &b| v << 8 | u64::from(b));
        let (from, to) = (read(p + index as usize * size), read(p + (index as usize + 1) * size));
        if to < from || from < 1 || to - 1 > (t.len() - base) as u64 {
            return None;
        }
        Some((base + from as usize - 1, (to - from) as usize))
    }

    pub(crate) fn record(&self, gid: u32) -> Option<&'a [u8]> {
        let (at, len) = self.entry(self.records, self.coverage(gid)?)?;
        Some(&self.t[at..at + len])
    }

    // The component at `p` in record `r`, `p` moved past it; None where the record ends inside it or the draw's
    // budget is spent.
    pub(crate) fn component(&self, r: &[u8], p: &mut usize) -> Option<Component> {
        if !self.spend(COMPONENT) {
            return None;
        }
        let flags = uint32var(r, p)?;
        let gid = match flags & GID_IS_24BIT != 0 {
            true => read_u24_be(r, *p)?,
            false => u32::from(read_u16_be(r, *p)?),
        };
        *p += if flags & GID_IS_24BIT != 0 { 3 } else { 2 };
        let condition = if flags & HAVE_CONDITION != 0 { uint32var(r, p)? } else { 0 };
        let (mut axis_indices, mut axis_values) = (0, Vec::new());
        if flags & HAVE_AXES != 0 {
            axis_indices = uint32var(r, p)?;
            let (at, len) = self.entry(self.axis_indices?, axis_indices)?;
            // The values are as many as the axis indices list, its runs counted up to one cut short.
            let mut count = 0;
            each_run(&self.t[at..at + len], self.room(), false, |_| count += 1);
            if !self.spend(count) {
                return None;
            }
            axis_values = runs(r, p, count, true).ok().filter(|v| v.len() == count)?;
            axis_values.iter_mut().for_each(|v| *v = i64::from((*v as i32).wrapping_mul(4)));
        }
        let axis_var = if flags & AXIS_VALUES_HAVE_VARIATION != 0 { uint32var(r, p)? } else { 0 };
        let transform_var = if flags & TRANSFORM_HAS_VARIATION != 0 { uint32var(r, p)? } else { 0 };
        let mut fields = [0, 0, 0, ONE, ONE, 0, 0, 0, 0];
        for (field, &(flag, scale, _)) in fields.iter_mut().zip(&FIELDS) {
            if flags & flag != 0 {
                *field = i64::from(read_i16_be(r, *p)?) * scale;
                *p += 2;
            }
        }
        if flags & (HAVE_SCALE_X | HAVE_SCALE_Y) == HAVE_SCALE_X {
            fields[SY] = fields[SX];
        }
        // A reserved flag stands for one more number each, skipped.
        for _ in 0..(flags & RESERVED).count_ones() {
            uint32var(r, p)?;
        }
        Some(Component { flags, gid, condition, axis_indices, axis_values, axis_var, transform_var, fields })
    }

    // Item `var`'s `n` deltas at `coords`, each kept to `shift` fractional bits, or with a condition's `default`, 1
    // where default and deltas sum above 0. Without coordinates the first region applies whole, the rest not at all.
    fn deltas(&self, var: u32, n: usize, shift: u32, coords: Option<&[i64]>, default: Option<i16>) -> Option<Vec<i64>> {
        let Some(store) = self.store else { return Some(vec![default.map_or(0, |d| i64::from(d > 0)); n]) };
        let t = self.t;
        let (outer, inner) = ((var >> 16) as usize, var & 0xFFFF);
        if store + 8 > t.len() || read_u16_be(t, store)? != 1 || outer >= usize::from(read_u16_be(t, store + 6)?) {
            return None;
        }
        let set = read_u32_be(t, store + 8 + 4 * outer)? as usize;
        let data = store.checked_add(set).filter(|&d| d <= t.len() && t.len() - d >= 7 && t[d] == 1)?;
        let region_count = usize::from(read_u16_be(t, data + 1)?);
        let indices = data + 3;
        if region_count > (t.len() - indices) / 2 || t.len() - (indices + 2 * region_count) < 4 {
            return None;
        }
        let delta_sets = read_u32_be(t, indices + 2 * region_count)? as usize;
        if delta_sets == 0 || delta_sets > t.len() - data || t.len() - data - delta_sets < 4 {
            return None;
        }
        let (at, len) = self.entry(data + delta_sets, inner)?;
        let regions = read_u32_be(t, store + 2)? as usize;
        let list = store.checked_add(regions).filter(|&l| regions > 0 && l <= t.len() && t.len() - l >= 4)?;
        let listed = read_u32_be(t, list)? as usize;
        if listed > (t.len() - list - 4) / 4 {
            return None;
        }
        let scalar = |r: usize| {
            let Some(coords) = coords else { return if r == 0 { ONE } else { 0 } };
            let index = usize::from(read_u16_be(t, indices + 2 * r).unwrap_or(0));
            let offset = (index < listed).then(|| read_u32_be(t, list + 4 + 4 * index)).flatten();
            offset.filter(|&o| (o as usize) < t.len() - list).map_or(0, |o| self.scalar(list + o as usize, coords))
        };
        // The tuple is laid out region by region, values missing at its end 0; read as it goes, as most are. A region
        // is worked out at its first value that isn't 0, as most of a tuple's regions have none.
        let mut acc = vec![default.map_or(0, |d| i64::from(d) << 16); n];
        let total = region_count.saturating_mul(n);
        let (mut k, mut region) = (0, (usize::MAX, 0));
        let read = each_run(&t[at..at + len], total.min(self.room()), true, |v| {
            if v != 0 {
                if region.0 != k / n {
                    region = (k / n, scalar(k / n));
                }
                acc[k % n] = acc[k % n].wrapping_add(v.wrapping_mul(region.1));
            }
            k += 1;
        });
        // The values walked are paid for, a set cut short too.
        if !self.spend(k) || read.is_none() {
            return None;
        }
        let right = 16 - shift;
        let round = if right > 0 { 1i64 << (right - 1) } else { 0 };
        let value = |a: i64| if default.is_some() { i64::from(a > 0) } else { a.wrapping_add(round) >> right };
        Some(acc.into_iter().map(value).collect())
    }

    // A sparse region's scalar at `coords` by OpenType's per-axis rule (otvaroverview): an axis peaking at 0 or
    // ill-formed is left out; one past the coordinates sits at its default, 0.
    fn scalar(&self, region: usize, coords: &[i64]) -> i64 {
        let t = self.t;
        let room = t.len() - region;
        if coords.is_empty() {
            return ONE;
        }
        let Some(count) = read_u16_be(t, region).filter(|_| room >= 2).map(usize::from) else { return 0 };
        if count > (room - 2) / 4 || !self.spend(count) {
            return 0;
        }
        let mut scalar = ONE;
        for i in 0..count {
            let offset = read_u32_be(t, region + 2 + 4 * i).map_or(usize::MAX, |o| o as usize);
            if offset > room || room - offset < 8 {
                return 0;
            }
            let axis = region + offset;
            let coord = coords.get(usize::from(read_u16_be(t, axis).unwrap_or(0))).copied().unwrap_or(0);
            let [min, peak, max] = [2, 4, 6].map(|o| i64::from(read_i16_be(t, axis + o).unwrap_or(0)) * 4);
            if peak == 0 || min > peak || peak > max || (min < 0 && max > 0) || coord == peak {
                continue;
            }
            if coord == 0 || coord < min || coord > max {
                return 0;
            }
            let factor = if coord < peak { div_fix(coord - min, peak - min) } else { div_fix(max - coord, max - peak) };
            scalar = mul_fix(scalar, factor);
            if scalar == 0 {
                return 0;
            }
        }
        scalar
    }

    // Where condition `index` starts; None for one the list doesn't hold, which leaves its component shown.
    fn condition(&self, index: u32) -> Option<usize> {
        let (t, list) = (self.t, self.conditions?);
        if list + 4 > t.len() || index >= read_u32_be(t, list)? || index as usize > (t.len() - list - 4) / 4 {
            return None;
        }
        let offset = read_u32_be(t, list + 4 + 4 * index as usize)? as usize;
        (offset < t.len() - list).then(|| list + offset)
    }

    // Whether the condition at `at` holds at `coords`. A null offset holds; one unknown or cut short doesn't, nor any
    // once the draw's budget is spent. Groups can name one child many times over, so each group or negation is worked
    // out once, a range or value each time at its price; a child lies past its parent, so the walk ends however deep.
    fn holds(&self, at: usize, coords: Option<&[i64]>) -> bool {
        let t = self.t;
        let mut known = Known::new();
        // The groups and negations being worked out, each with its next child, and what the last one finished held.
        let mut open = Vec::new();
        let mut held = self.condition_at(at, coords, &mut known, &mut open);
        while let Some(&mut (at, ref mut next)) = open.last_mut() {
            let format = read_u16_be(t, at).unwrap_or(0);
            let count = if format == 5 { 1 } else { usize::from(t[at + 2]) };
            // A child decides an And where it fails, an Or where it holds, and a negation either way.
            let decided = match (format, held) {
                (3, Some(false)) => Some(false),
                (4, Some(true)) => Some(true),
                (5, Some(held)) => Some(!held),
                _ => (*next == count).then_some(format == 3),
            };
            if let Some(decided) = decided {
                open.pop();
                // No group names the condition at the top, so it isn't remembered.
                if !open.is_empty() {
                    known.insert(at, decided);
                }
                held = Some(decided);
                continue;
            }
            let field = if format == 5 { 2 } else { 3 + 3 * *next };
            *next += 1;
            // A null child holds, so a negation of one doesn't.
            held = match read_u24_be(t, at + field).map_or(0, |o| o as usize) {
                0 => Some(true),
                o => self.condition_at(at + o, coords, &mut known, &mut open),
            };
        }
        held == Some(true)
    }

    // Whether the condition at `at` holds, where that's known without its children; else None, it opened onto `open`.
    fn condition_at(
        &self, at: usize, coords: Option<&[i64]>, known: &mut Known, open: &mut Vec<(usize, usize)>,
    ) -> Option<bool> {
        let t = self.t;
        if Some(at) == self.conditions {
            return Some(true);
        }
        if at + 2 > t.len() {
            return Some(false);
        }
        // One looked up costs as one worked out, as each of a group's 255 children can name it.
        if let Some(&held) = known.get(&at) {
            return Some(self.spend(CONDITION) && held);
        }
        let held = self.spend(CONDITION) && match read_u16_be(t, at).unwrap_or(0) {
            1 if at + 8 <= t.len() => {
                let axis = usize::from(read_u16_be(t, at + 2).unwrap_or(0));
                let coord = coords.and_then(|c| c.get(axis)).copied().unwrap_or(0);
                let [min, max] = [4, 6].map(|o| i64::from(read_i16_be(t, at + o).unwrap_or(0)) * 4);
                (min..=max).contains(&coord)
            }
            2 if at + 8 <= t.len() => {
                let default = read_i16_be(t, at + 2).unwrap_or(0);
                let var = read_u32_be(t, at + 4).unwrap_or(0);
                let varied = (var != NO_VARIATION).then(|| self.deltas(var, 1, 0, coords, Some(default))).flatten();
                varied.map_or(default > 0, |d| d[0] != 0)
            }
            3 | 4 if at + 3 <= t.len() => {
                let count = usize::from(t[at + 2]);
                if at + 3 + 3 * count <= t.len() && self.spend(count) {
                    open.push((at, 0));
                    return None;
                }
                false
            }
            5 if at + 5 <= t.len() => {
                open.push((at, 0));
                return None;
            }
            _ => false,
        };
        Some(held)
    }

    // Axis indices list entry `index`'s first `n` indices, None where it holds fewer.
    fn axis_indices(&self, index: u32, n: usize) -> Option<Vec<i64>> {
        let (at, len) = self.entry(self.axis_indices?, index)?;
        if !self.spend(n) {
            return None;
        }
        runs(&self.t[at..at + len], &mut 0, n, false).ok().filter(|v| v.len() == n)
    }

    // The component's transform where `coords` vary it: whether it shows there, its axis values with their deltas,
    // its matrix and its offset in 26.6.
    pub(crate) fn place(&self, c: &mut Component, coords: Option<&[i64]>) -> Option<(Matrix, (i64, i64))> {
        if c.flags & HAVE_CONDITION != 0
            && let Some(at) = self.condition(c.condition)
            && !self.holds(at, coords)
        {
            return None;
        }
        // A variation that doesn't read leaves its values as they were.
        if self.store.is_some() && c.flags & AXIS_VALUES_HAVE_VARIATION != 0 && !c.axis_values.is_empty()
            && let Some(deltas) = self.deltas(c.axis_var, c.axis_values.len(), 2, coords, None)
        {
            c.axis_values.iter_mut().zip(deltas).for_each(|(v, d)| *v += d);
        }
        let present = (0..9).filter(|&i| c.flags & FIELDS[i].0 != 0);
        if self.store.is_some() && c.flags & TRANSFORM_HAS_VARIATION != 0 && present.clone().next().is_some()
            && let Some(deltas) = self.deltas(c.transform_var, present.clone().count(), 4, coords, None)
        {
            for (i, d) in present.zip(deltas) {
                c.fields[i] += d * FIELDS[i].2;
            }
            if c.flags & (HAVE_SCALE_X | HAVE_SCALE_Y) == HAVE_SCALE_X {
                c.fields[SY] = c.fields[SX];
            }
        }
        Some(transform(&c.fields))
    }

    // The coordinates component `c` draws at, from its parent's and the font's: its axis values set over them, each
    // cut to a 2.14 step, an axis past them ignored. False where it sets none or its indices don't read.
    pub(crate) fn coords(&self, c: &Component, parent: &[i64], font: &[i64], coords: &mut Vec<i64>) -> bool {
        if (c.axis_values.is_empty() && c.flags & RESET_UNSPECIFIED_AXES == 0) || parent.is_empty() {
            return false;
        }
        if !self.spend(parent.len()) {
            return false;
        }
        coords.clear();
        match c.flags & RESET_UNSPECIFIED_AXES != 0 {
            true => coords.extend_from_slice(&font[..font.len().min(parent.len())]),
            false => coords.extend_from_slice(parent),
        }
        coords.resize(parent.len(), 0);
        if c.flags & HAVE_AXES != 0 && self.axis_indices.is_some() {
            let Some(indices) = self.axis_indices(c.axis_indices, c.axis_values.len()) else { return false };
            for (&axis, &v) in indices.iter().zip(&c.axis_values) {
                if let Some(slot) = usize::try_from(axis).ok().and_then(|a| coords.get_mut(a)) {
                    *slot = if v >= 0 { (v + 2) & !3 } else { -((-v + 2) & !3) };
                }
            }
        }
        true
    }
}

// The matrix and offset a component's fields make, in the VARC spec's order: rotated, scaled and skewed about its
// center, then translated. Worked from the exact sines in Q60, each value rounded to 16.16 or 26.6 once.
fn transform(f: &[i64; 9]) -> (Matrix, (i64, i64)) {
    use trig::round_shift;
    let (sin, cos) = trig::sin_cos(mul_fix(f[ROTATION], trig::PI));
    let [tan_x, tan_y] = [f[SKEW_X], f[SKEW_Y]].map(|s| trig::tan(mul_fix(s, trig::PI)));
    let [sx, sy] = [f[SX], f[SY]].map(|v| i128::from(v.clamp(-0x7FFF_FFFF, 0x7FFF_FFFF)));
    let skewed = |t: i128, tan: i128| round_shift(t * tan, 44);
    // Rotation · scale · [1, −tan x; tan y, 1], each term Q60 times a 16.16 scale.
    let m = [
        cos * sx - skewed(sin, tan_y) * sy,
        -skewed(cos, tan_x) * sx - sin * sy,
        sin * sx + skewed(cos, tan_y) * sy,
        cos * sy - skewed(sin, tan_x) * sx,
    ]
    .map(|v| round_shift(v, trig::Q).clamp(-0x7FFF_FFFF, 0x7FFF_FFFF) as i64);
    let (cx, cy) = (f[CX], f[CY]);
    let moved = |a: i64, b: i64| {
        round_shift(i128::from(a) * i128::from(cx) + i128::from(b) * i128::from(cy), 16) as i64
    };
    let offset = |t: i64, c: i64, moved: i64| t.saturating_add(c).saturating_sub(moved);
    (m, (offset(f[TX], cx, moved(m[0], m[1])), offset(f[TY], cy, moved(m[2], m[3]))))
}

// a · b, each product rounded as 16.16.
pub(crate) fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    [
        mul_fix(a[0], b[0]).wrapping_add(mul_fix(a[1], b[2])),
        mul_fix(a[0], b[1]).wrapping_add(mul_fix(a[1], b[3])),
        mul_fix(a[2], b[0]).wrapping_add(mul_fix(a[3], b[2])),
        mul_fix(a[2], b[1]).wrapping_add(mul_fix(a[3], b[3])),
    ]
}

// A variable-length number: its first byte's leading ones count the bytes after it, four at most, so any first byte
// from 0xF0 takes four, which alone hold the value.
fn uint32var(r: &[u8], p: &mut usize) -> Option<u32> {
    let lead = *r.get(*p)?;
    let n = match lead {
        0..0x80 => 0,
        0x80..0xC0 => 1,
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        _ => 4,
    };
    let rest = r.get(*p + 1..*p + 1 + n)?;
    *p += 1 + n;
    Some(rest.iter().fold(if n == 4 { 0 } else { u32::from(lead) & (0x7F >> n) }, |v, &b| v << 8 | u32::from(b)))
}

// Up to `n` TupleValues from `d` at `p`, each run cut to what's left of them, signed or not: Ok where they or the
// data run out, Err with how many were read where a run is cut short.
fn runs(d: &[u8], p: &mut usize, n: usize, signed: bool) -> Result<Vec<i64>, usize> {
    let mut out = Vec::with_capacity(n.min(d.len()));
    let read = each_run(&d[(*p).min(d.len())..], n, signed, |v| out.push(v));
    match read {
        Some(used) => {
            *p += used;
            Ok(out)
        }
        None => Err(out.len()),
    }
}

// Each of up to `n` TupleValues in `d` handed to `f`, a run cut to what's left of them; the bytes read, or None
// where a run is cut short.
fn each_run(d: &[u8], n: usize, signed: bool, mut f: impl FnMut(i64)) -> Option<usize> {
    let (mut p, mut i) = (0, 0);
    while i < n && p < d.len() {
        let control = d[p];
        p += 1;
        let run = (usize::from(control & 0x3F) + 1).min(n - i);
        let width = match control >> 6 {
            3 => 4,
            2 => 0,
            1 => 2,
            _ => 1,
        };
        let bytes = d.get(p..p + width * run)?;
        if width == 0 {
            (0..run).for_each(|_| f(0));
        }
        for v in bytes.chunks_exact(width.max(1)).take(if width == 0 { 0 } else { run }) {
            f(match (width, signed) {
                (4, true) => i64::from(i32::from_be_bytes([v[0], v[1], v[2], v[3]])),
                (4, false) => i64::from(u32::from_be_bytes([v[0], v[1], v[2], v[3]])),
                (2, true) => i64::from(i16::from_be_bytes([v[0], v[1]])),
                (2, false) => i64::from(u16::from_be_bytes([v[0], v[1]])),
                (_, true) => i64::from(v[0] as i8),
                _ => i64::from(v[0]),
            });
        }
        p += width * run;
        i += run;
    }
    Some(p)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/varc/mod.rs"]
mod tests;
