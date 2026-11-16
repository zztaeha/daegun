// Glyph positioning (OpenType GPOS): single and pair adjustments here; attachment by anchors in `marks` and
// `cursive`; the chains attachments leave, resolved once every lookup has run, in `attach`.
mod attach;
mod cursive;
mod marks;

pub(crate) use attach::resolve_attachments;
use marks::Onto;

use super::apply::{chained, sequence, Engine};
use super::{long, nth_table_width, value_record_len, LookupKinds, Lookup, MEDIUM};
use crate::daecore::daeshaper::face::Deltas;
use crate::daecore::daeshaper::run::Run;
use crate::daecore::daetype::decoder::{midpoint_search, read_i16_be, read_u16_be, read_uint, records_fit};

// GPOS lookup types.
pub(crate) const SINGLE: u16 = 1;
pub(crate) const PAIR: u16 = 2;
pub(crate) const CURSIVE: u16 = 3;
pub(crate) const MARK_BASE: u16 = 4;
pub(crate) const MARK_LIG: u16 = 5;
pub(crate) const MARK_MARK: u16 = 6;
pub(crate) const CONTEXT: u16 = 7;
pub(crate) const CHAIN_CONTEXT: u16 = 8;
pub(crate) const EXTENSION: u16 = 9;

pub(crate) const KINDS: LookupKinds = LookupKinds {
    last: EXTENSION,
    extension: EXTENSION,
    context: CONTEXT,
    chain_context: CHAIN_CONTEXT,
    pair_pos: Some(PAIR),
    ligature: None,
    reverse: None,
    marks: &[MARK_BASE, MARK_LIG, MARK_MARK],
};

// ValueFormat: a value record holds the fields its bits name, x and y placement, then x and y advance, and after
// them their device or variation index offsets in the same order.
const VALUES: u16 = 0x000F;
const X_ADVANCE: u16 = 0x0004;
const Y_ADVANCE: u16 = 0x0008;
const DEVICES: u16 = 0x00F0;

// The glyph a pair reads second, looked for once however many pair subtables the lookup tries: not yet looked for,
// found at a place, or missing with the read ending at a place.
#[derive(Clone, Copy)]
enum Second {
    Unread,
    At(usize, u32),
    Missing(usize),
}

// `lookup` at the current glyph.
pub(crate) fn at_glyph<'a>(e: &mut Engine<'a>, lookup: &Lookup<'a>) -> bool {
    let mut second = Second::Unread;
    e.each_subtable(lookup, &KINDS, |e, kind, data| match kind & MEDIUM {
        0 => by_kind::<2>(e, kind, data, &mut second),
        _ => wide(e, kind & !MEDIUM, data, &mut second),
    })
}

// Each type's reader takes the width of its format's glyphs, `W`: 2, or 3 for the 24-bit forms `MEDIUM` marks.
fn by_kind<const W: usize>(e: &mut Engine, kind: u16, data: &[u8], second: &mut Second) -> bool {
    match kind {
        SINGLE => single::<W>(e, data),
        PAIR => pair::<W>(e, data, second),
        CURSIVE => cursive::attach::<W>(e, data),
        MARK_BASE => marks::attach::<W>(e, data, Onto::Base),
        MARK_LIG => marks::attach::<W>(e, data, Onto::Ligature),
        MARK_MARK => marks::attach::<W>(e, data, Onto::Mark),
        CONTEXT => sequence::<W>(e, data),
        CHAIN_CONTEXT => chained::<W>(e, data),
        _ => false,
    }
}

// The 24-bit forms out of line, so the 16-bit readers keep their code tight.
#[inline(never)]
fn wide(e: &mut Engine, kind: u16, data: &[u8], second: &mut Second) -> bool {
    by_kind::<3>(e, kind, data, second)
}

// The format a type with one 16-bit format has, or its 24-bit form.
pub(super) const fn sole_format(width: usize) -> u16 {
    if width == 3 { 2 } else { 1 }
}

// How far SinglePos's and PairPos's 24-bit formats, 3 and 4, lie past their 16-bit ones.
const fn wide_shift(width: usize) -> u16 {
    if width == 3 { 2 } else { 0 }
}

// A value record's x and y placement and advance, each 0 where its format leaves it out.
#[inline(always)]
fn read_values(data: &[u8], at: usize, format: u16) -> [i32; 4] {
    let mut values = [0; 4];
    let mut field = at;
    for (bit, value) in values.iter_mut().enumerate() {
        if format & 1 << bit != 0 {
            *value = read_i16_be(data, field).map_or(0, i32::from);
            field += 2;
        }
    }
    values
}

// The fields of a record that move the glyph: those not 0.
fn nonzero(values: [i32; 4]) -> u16 {
    (0..4).fold(0, |bits, k| bits | u16::from(values[k] != 0) << k)
}

// The value record at `at` in `base`, the table its device offsets count from (a SinglePos or PairPos format 2
// subtable, or a PairSet), applied to glyph `i`. Whether it moved the glyph.
#[inline(always)]
fn adjust(run: &mut Run, deltas: Option<&Deltas>, i: usize, (base, at): (&[u8], usize), format: u16) -> bool {
    let (values, moved) = match deltas.filter(|_| format & DEVICES != 0) {
        Some(deltas) => with_devices(base, at, format, deltas),
        None => {
            let values = read_values(base, at, format);
            (values, nonzero(values))
        }
    };
    place(run, i, values, moved)
}

// At a location, each field plus its device's delta; a field with a device moves the glyph even where both are 0.
// Out of line, as only a location reads devices.
#[cold]
#[inline(never)]
fn with_devices(base: &[u8], at: usize, format: u16, deltas: &Deltas) -> ([i32; 4], u16) {
    let mut values = read_values(base, at, format);
    let mut moved = nonzero(values);
    let mut field = at + 2 * (format & VALUES).count_ones() as usize;
    for (bit, value) in values.iter_mut().enumerate() {
        if format & DEVICES & 0x10 << bit == 0 {
            continue;
        }
        let offset = read_u16_be(base, field).unwrap_or(0);
        field += 2;
        if offset != 0 {
            moved |= 1 << bit;
            *value = value.saturating_add(device_delta(base, offset, deltas));
        }
    }
    (values, moved)
}

// The fields `moved` names added to glyph `i`: placements as they are, and the advance along the run's axis, which
// runs down in vertical text. Whether any of them moves the glyph in this direction.
#[inline(always)]
fn place(run: &mut Run, i: usize, values: [i32; 4], moved: u16) -> bool {
    let moved = moved & if run.direction.is_horizontal() { !Y_ADVANCE } else { !X_ADVANCE };
    let p = &mut run.places[i];
    let fields = [&mut p.offset_x, &mut p.offset_y, &mut p.advance_x, &mut p.advance_y];
    for (bit, (field, value)) in fields.into_iter().zip(values).enumerate() {
        if moved & 1 << bit != 0 {
            *field = if bit == 3 { field.saturating_sub(value) } else { field.saturating_add(value) };
        }
    }
    moved & VALUES != 0
}

// The delta at the face's location of the VariationIndex table at `offset` from `base`. A Device table, for a pixel
// size shaping doesn't have, and one that doesn't read give none.
fn device_delta(base: &[u8], offset: u16, deltas: &Deltas) -> i32 {
    variation_index(base, offset).map_or(0, |(outer, inner)| deltas.of(outer, inner))
}

pub(super) fn variation_index(base: &[u8], offset: u16) -> Option<(u16, u16)> {
    let at = usize::from(offset);
    let word = |k: usize| read_u16_be(base, at + k);
    (word(4)? == 0x8000).then(|| Some((word(0)?, word(2)?))).flatten()
}

// SinglePos formats 1 and 2, or 3 and 4: one record for every covered glyph, or one each after their count.
fn single<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    let l = long(W);
    let Some(index) = e.coverage(data, 2, l).index_of(e.run.current().id) else { return false };
    let [Some(format), Some(values)] = [0, 2 + l].map(|k| read_u16_be(data, k)) else { return false };
    let size = value_record_len(values);
    let records = 4 + l + W;
    let fits = |count: &u32| index < *count && records_fit(records, *count as usize, size, data.len());
    let record = match format.wrapping_sub(wide_shift(W)) {
        1 => Some(4 + l),
        2 => read_uint(data, 4 + l, W).filter(fits).map(|_| records + index as usize * size),
        _ => None,
    };
    let Some(record) = record else { return false };
    let i = e.run.cursor;
    adjust(e.run, e.deltas, i, (data, record), values);
    e.run.cursor += 1;
    true
}

// PairPos formats 1 and 2, or 3 and 4: the current glyph and the next one the lookup reads. A pair the subtable has no
// record for still marks them unsafe to concatenate.
fn pair<const W: usize>(e: &mut Engine, data: &[u8], second: &mut Second) -> bool {
    let l = long(W);
    if data.len() < 6 + l {
        return false;
    }
    let first = e.run.current().id;
    let Some(index) = e.coverage(data, 2, l).index_of(first) else { return false };
    let start = e.run.cursor;
    let (at, glyph) = match second_glyph(e, second) {
        Ok(found) => found,
        Err(end) => {
            e.unsafe_to_concat(start..end);
            return false;
        }
    };
    let formats = (read_u16_be(data, 2 + l).unwrap_or(0), read_u16_be(data, 4 + l).unwrap_or(0));
    let Some(found) = pair_values::<W>(e, data, (first, index), glyph, formats) else { return false };
    let Some((base, record)) = found else {
        e.unsafe_to_concat(start..at + 1);
        return false;
    };
    let records = [(start, record, formats.0), (at, record + value_record_len(formats.0), formats.1)];
    let moved = records.map(|(i, record, format)| adjust(e.run, e.deltas, i, (base, record), format));
    if moved.contains(&true) {
        e.run.unsafe_to_break(start..at + 1);
    } else {
        e.unsafe_to_concat(start..at + 1);
    }
    // A pair that moves its second glyph has positioned it, so the lookup goes on past it.
    let past = value_record_len(formats.1) != 0;
    if past {
        e.run.unsafe_to_break(start..at + 2);
    }
    e.run.cursor = at + usize::from(past);
    true
}

// The table a pair's records sit in, which their device offsets count from, and where they start. None for a subtable
// that doesn't read, which marks nothing; Some(None) for a pair it has no record for.
fn pair_values<'d, const W: usize>(
    e: &Engine,
    data: &'d [u8],
    (first, index): (u32, u32),
    second: u32,
    formats: (u16, u16),
) -> Option<Option<(&'d [u8], usize)>> {
    let l = long(W);
    match read_u16_be(data, 0)?.wrapping_sub(wide_shift(W)) {
        1 => {
            // An index of the seconds any pair set names passes most glyphs by without a search.
            let named = e.subtable_index().and_then(|i| i.second.as_ref()).is_none_or(|i| i.lookup(second) != 0);
            let set = || nth_table_width(data, 6 + l, W, W, index);
            Some(named.then(set).and_then(|set| pair_record::<W>(set, second, formats).map(|at| (set, at))))
        }
        2 => {
            let (rows, columns) = (read_u16_be(data, 6 + 3 * l)?, read_u16_be(data, 8 + 3 * l)?);
            let row = e.pair_classes(data, 6 + l, l).class_of(first);
            let column = e.pair_classes(data, 6 + 2 * l, l).class_of(second);
            Some(class_record(data, (row, column), (rows, columns), 10 + 3 * l, formats).map(|at| (data, at)))
        }
        _ => None,
    }
}

// Inline, as the lookup's later pair subtables only read what the first found; the walk itself apart.
#[inline(always)]
fn second_glyph(e: &mut Engine, second: &mut Second) -> Result<(usize, u32), usize> {
    if let Second::Unread = second {
        *second = read_second(e);
    }
    match *second {
        Second::At(at, glyph) => Ok((at, glyph)),
        Second::Missing(end) => Err(end),
        Second::Unread => Err(e.run.cursor),
    }
}

#[inline(never)]
fn read_second(e: &mut Engine) -> Second {
    match e.next_present() {
        Ok(at) => Second::At(at, e.run.glyphs[at].id),
        Err(end) => Second::Missing(end),
    }
}

// Where in a PairSet the records for second glyph `glyph` start, found by search: each PairValueRecord holds the
// second glyph, then both value records.
pub(super) fn pair_record<const W: usize>(set: &[u8], glyph: u32, (first, second): (u16, u16)) -> Option<usize> {
    let count = read_uint(set, 0, W)? as usize;
    let size = W + value_record_len(first) + value_record_len(second);
    let record = |i: usize| W + i * size;
    let found = midpoint_search(count, |i| Some(glyph.cmp(&read_uint(set, record(i), W)?)))?;
    Some(record(found) + W)
}

// Where PairPos format 2's records for a pair of classes start, `records` the first class pair's; none outside the
// counts or the table.
fn class_record(
    data: &[u8],
    (row, column): (u32, u32),
    (rows, columns): (u16, u16),
    records: usize,
    (first, second): (u16, u16),
) -> Option<usize> {
    if row >= u32::from(rows) || column >= u32::from(columns) {
        return None;
    }
    let size = value_record_len(first) + value_record_len(second);
    let cell = (row as usize).checked_mul(usize::from(columns))?.checked_add(column as usize)?;
    let at = cell.checked_mul(size)?.checked_add(records)?;
    at.checked_add(size).is_some_and(|end| end <= data.len()).then_some(at)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/ot/gpos/mod.rs"]
mod tests;
