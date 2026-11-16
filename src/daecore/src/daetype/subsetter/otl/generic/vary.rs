use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{add_i16, read_i16_be, read_u16_be, read_uint, write_i16_be, write_u16_be};
use crate::daecore::daetype::format::ivs::variation_index;
use super::parse::{read_offset, resolve_count, Scalars, Work};
use super::schema::{CountSource, EnvRef, Name, OffsetWidth, PayloadShape, Schema, StructField};

// Why a walk stopped for the whole table: a delta for a value its record doesn't hold, or the
// table's work spent.
#[derive(Debug)]
pub(crate) enum Stop {
    Widen,
    Budget,
}

// VariationIndex deltas applied in place by the schemas a rebuild parses: `out`, a copy of the source,
// gets each value plus its delta with the offset cleared, so a record reached twice comes out the same.
pub(crate) struct InPlace<'a, V> {
    src: &'a [u8],
    pub(crate) out: Vec<u8>,
    vary: &'a V,
    work: Work,
    // Why the walk returned None; nothing here means a malformed record, which ends only its table.
    stop: Option<Stop>,
}

impl<'a, V: Fn(u16, u16) -> i32> InPlace<'a, V> {
    pub(crate) fn new(src: &'a [u8], vary: &'a V) -> Self {
        InPlace { src, out: src.to_vec(), vary, work: Work::for_table(src.len()), stop: None }
    }

    pub(crate) fn subtable(&mut self, at: usize, schema: &Schema) -> Result<(), Stop> {
        self.table(at, schema, &mut [None; Name::COUNT]);
        self.stop.take().map_or(Ok(()), Err)
    }

    fn halt<T>(&mut self, stop: Stop) -> Option<T> {
        self.stop = Some(stop);
        None
    }

    // A table, walked once however many name it. One malformed leaves the rest of its parent.
    fn table(&mut self, at: usize, schema: &Schema, names: &mut Scalars) -> Option<()> {
        if !self.work.seen(at) {
            match schema {
                Schema::Anchor => self.anchor(at),
                _ => self.walk(at, at, schema, names),
            };
        }
        self.stop.is_none().then_some(())
    }

    // The bytes the schema at `pos` holds inline; offsets in it count from `base`.
    fn walk(&mut self, pos: usize, base: usize, schema: &Schema, names: &mut Scalars) -> Option<usize> {
        if self.work.spend(1).is_err() {
            return self.halt(Stop::Budget);
        }
        let src = self.src;
        Some(match schema {
            Schema::U16 | Schema::LookupIndex | Schema::GlyphId => 2,
            Schema::U24 | Schema::GlyphId24 => 3,
            Schema::Coverage(..) | Schema::ClassDef(_) => 0,
            Schema::Glyphs(w, count, _) => w.bytes() * count_of(count, names)?,
            Schema::ValueFormatField(EnvRef(name)) => {
                names[*name as usize] = Some(read_u16_be(src, pos)?.into());
                2
            }
            Schema::ValueRecordField(EnvRef(name)) => self.record(pos, base, names[*name as usize]? as u16)?,
            Schema::Offset(width, child) => {
                if !inert(child, names) {
                    self.child(pos, base, *width, child, names)?;
                }
                width.bytes()
            }
            Schema::OffsetArray(child, count, _, width) => {
                let count = count_of(count, names)?;
                self.each(pos, base, count, child, PayloadShape::Offsets(*width), names)?
            }
            Schema::Array(elem, count, _) => {
                let count = count_of(count, names)?;
                self.each(pos, base, count, elem, PayloadShape::Inline, names)?
            }
            Schema::Struct(fields) => self.fields(pos, base, fields, names)?,
            Schema::FormatSwitch(peek, variants) => {
                let tag = read_u16_be(src, pos + peek)?;
                let (_, variant) = variants.iter().find(|(t, _)| *t == tag)?;
                self.walk(pos, base, variant, names)?
            }
            Schema::CoveredArray(fields, payload, shape, w) => {
                let l = w.long().bytes();
                let at = pos + l + self.fields(pos + l, base, fields, names)?;
                let count = read_uint(src, at, w.bytes())? as usize;
                at + w.bytes() - pos + self.each(at + w.bytes(), base, count, payload, *shape, names)?
            }
            Schema::ZippedWithBoundCoverage(_, payload, shape, _, w) => {
                let count = read_uint(src, pos, w.bytes())? as usize;
                w.bytes() + self.each(pos + w.bytes(), base, count, payload, *shape, names)?
            }
            Schema::ClassMatrix(EnvRef(vf1), EnvRef(vf2), w) => {
                let (f1, f2) = (names[*vf1 as usize]? as u16, names[*vf2 as usize]? as u16);
                let counts = pos + 2 * w.long().bytes();
                // Records with no device offsets have nothing to vary. Each other one reads its offsets, so
                // the cells stop at the table's end whatever counts it states.
                if (f1 | f2) & 0x00F0 != 0 {
                    let mut at = counts + 4;
                    for _ in 0..usize::from(read_u16_be(src, counts)?) * usize::from(read_u16_be(src, counts + 2)?) {
                        at += self.record(at, base, f1)?;
                        at += self.record(at, base, f2)?;
                    }
                }
                counts + 4 - pos
            }
            Schema::Anchor => self.anchor(pos)?,
            Schema::CaretValue => match read_u16_be(src, pos)? {
                3 => {
                    self.device(Some(pos + 2), pos + 4, pos)?;
                    6
                }
                _ => 4,
            },
            Schema::DeltaCoverageSubst(w) => 2 + w.long().bytes() + w.bytes(),
            Schema::CoveredGlyphs(w) => {
                let at = pos + 2 + w.long().bytes();
                at - pos + w.bytes() * (1 + read_uint(src, at, w.bytes())? as usize)
            }
        })
    }

    fn anchor(&mut self, pos: usize) -> Option<usize> {
        if read_u16_be(self.src, pos)? == 3 {
            self.device(Some(pos + 2), pos + 6, pos)?;
            self.device(Some(pos + 4), pos + 8, pos)?;
        }
        Some(6)
    }

    fn fields(&mut self, pos: usize, base: usize, fields: &[StructField], names: &mut Scalars) -> Option<usize> {
        let mut at = pos;
        for field in fields {
            let len = self.walk(at, base, &field.schema, names)?;
            if let Some(name) = field.bind {
                let width = if matches!(field.schema, Schema::U24) { 3 } else { 2 };
                names[name as usize] = Some(read_uint(self.src, at, width)?);
            }
            at += len;
        }
        Some(at - pos)
    }

    // `count` records in a row, inline or behind offsets. An inline array of offsets, or of arrays of
    // them, is one run of offsets: a table an offset names binds nothing its parent reads.
    fn each(
        &mut self, pos: usize, base: usize, count: usize, schema: &Schema, shape: PayloadShape, names: &mut Scalars,
    ) -> Option<usize> {
        let (mut count, mut schema, mut shape) = (count, schema, shape);
        loop {
            match (schema, shape) {
                (Schema::Offset(width, child), PayloadShape::Inline) => {
                    (schema, shape) = (child, PayloadShape::Offsets(*width));
                }
                (Schema::Array(elem, source, _), PayloadShape::Inline) if matches!(elem, Schema::Offset(..)) => {
                    (count, schema) = (count.checked_mul(count_of(source, names)?)?, elem);
                }
                _ => break,
            }
        }
        match shape {
            PayloadShape::Offsets(width) if inert(schema, names) => return count.checked_mul(width.bytes()),
            PayloadShape::Inline => {
                if let Some(plan) = Plan::of(schema, names) {
                    if plan.len > 0 {
                        for at in (0..count).map(|i| pos + i * plan.width) {
                            for &(offset, part) in &plan.parts[..plan.len] {
                                match part {
                                    Part::Values(format) => { self.record(at + offset, base, format)?; }
                                    Part::Table(width, child) => self.child(at + offset, base, width, child, names)?,
                                }
                            }
                        }
                    }
                    return Some(count * plan.width);
                }
            }
            PayloadShape::Offsets(_) => {}
        }
        // A record of no bytes ends the row, as each after it would walk the same ones.
        let mut at = pos;
        for _ in 0..count {
            at += match shape {
                PayloadShape::Inline => match self.walk(at, base, schema, names)? {
                    0 => break,
                    len => len,
                },
                PayloadShape::Offsets(width) => {
                    self.child(at, base, width, schema, names)?;
                    width.bytes()
                }
            };
        }
        Some(at - pos)
    }

    fn child(
        &mut self, pos: usize, base: usize, width: OffsetWidth, schema: &Schema, names: &mut Scalars,
    ) -> Option<()> {
        match read_offset(self.src, pos, width).ok()? {
            0 => Some(()),
            rel => self.table(base.checked_add(rel as usize)?, schema, names),
        }
    }

    // A ValueRecord in `format`: its values, then a device offset for each device bit.
    fn record(&mut self, pos: usize, base: usize, format: u16) -> Option<usize> {
        let format = format & 0x00FF;
        let mut slot = pos + 2 * (format & 0x000F).count_ones() as usize;
        for bit in 0..4 {
            if format & (0x10 << bit) != 0 {
                let value =
                    (format & (1 << bit) != 0).then(|| pos + 2 * (format & ((1 << bit) - 1)).count_ones() as usize);
                self.device(value, slot, base)?;
                slot += 2;
            }
        }
        Some(slot - pos)
    }

    // The table the device offset in `slot` names, if a VariationIndex: its delta added to the value at
    // `value` and the offset cleared. A nonzero delta with no value to hold it stops the walk.
    fn device(&mut self, value: Option<usize>, slot: usize, base: usize) -> Option<()> {
        let rel = usize::from(read_u16_be(self.src, slot)?);
        let Some((outer, inner)) = (rel != 0).then(|| variation_index(self.src, base + rel)).flatten() else {
            return Some(());
        };
        let delta = (self.vary)(outer, inner);
        match value {
            Some(at) => {
                let v = read_i16_be(self.src, at)?;
                write_i16_be(&mut self.out, at, add_i16(v, delta));
            }
            None if delta != 0 => return self.halt(Stop::Widen),
            None => {}
        }
        write_u16_be(&mut self.out, slot, 0);
        Some(())
    }
}

// Whether a table of this schema holds no Device offset, by the value formats bound so far.
fn inert(schema: &Schema, names: &Scalars) -> bool {
    let plain = |name: &Name| names[*name as usize].is_some_and(|format| format & 0x00F0 == 0);
    match schema {
        Schema::ValueRecordField(EnvRef(name)) => plain(name),
        Schema::ClassMatrix(EnvRef(vf1), EnvRef(vf2), _) => plain(vf1) && plain(vf2),
        Schema::Anchor | Schema::CaretValue => false,
        Schema::Offset(_, child)
        | Schema::Array(child, ..)
        | Schema::OffsetArray(child, ..)
        | Schema::ZippedWithBoundCoverage(_, child, ..) => inert(child, names),
        Schema::Struct(fields) => fields.iter().all(|f| inert(&f.schema, names)),
        Schema::CoveredArray(fields, child, ..) => {
            fields.iter().all(|f| inert(&f.schema, names)) && inert(child, names)
        }
        Schema::FormatSwitch(_, variants) => variants.iter().all(|(_, s)| inert(s, names)),
        Schema::U16
        | Schema::U24
        | Schema::GlyphId
        | Schema::GlyphId24
        | Schema::Glyphs(..)
        | Schema::Coverage(..)
        | Schema::ClassDef(_)
        | Schema::ValueFormatField(_)
        | Schema::DeltaCoverageSubst(_)
        | Schema::CoveredGlyphs(_)
        | Schema::LookupIndex => true,
    }
}

// A part of a record that can hold deltas: a ValueRecord in a format with device offsets, or an
// offset to a table that may.
#[derive(Clone, Copy)]
enum Part {
    Values(u16),
    Table(OffsetWidth, &'static Schema),
}

// Records laid out alike that bind nothing, worked out once for a run of them: each one's width and
// the parts in it worth reading, by where they start in the record.
struct Plan {
    width: usize,
    parts: [(usize, Part); 4],
    len: usize,
}

impl Plan {
    fn of(schema: &Schema, names: &Scalars) -> Option<Self> {
        let mut plan = Plan { width: 0, parts: [(0, Part::Values(0)); 4], len: 0 };
        plan.add(schema, names)?;
        Some(plan)
    }

    fn add(&mut self, schema: &Schema, names: &Scalars) -> Option<()> {
        let at = self.width;
        let part = match schema {
            Schema::U16 | Schema::LookupIndex | Schema::GlyphId => {
                self.width += 2;
                None
            }
            Schema::U24 | Schema::GlyphId24 => {
                self.width += 3;
                None
            }
            Schema::ValueRecordField(EnvRef(name)) => {
                let format = names[*name as usize]? as u16;
                self.width += 2 * (format & 0x00FF).count_ones() as usize;
                (format & 0x00F0 != 0).then_some(Part::Values(format))
            }
            Schema::Offset(width, child) => {
                self.width += width.bytes();
                (!inert(child, names)).then_some(Part::Table(*width, child))
            }
            Schema::Struct(fields) => {
                return fields.iter().try_for_each(|f| if f.bind.is_some() { None } else { self.add(&f.schema, names) });
            }
            _ => return None,
        };
        if let Some(part) = part {
            *self.parts.get_mut(self.len)? = (at, part);
            self.len += 1;
        }
        Some(())
    }
}

fn count_of(source: &CountSource, names: &Scalars) -> Option<usize> {
    resolve_count(source, names).ok().flatten()
}
