use crate::daecore::daetype::subsetter::{remap_gid, BitSet, GlyphSet};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use crate::daecore::daetype::decoder::{add_i16, clamp_i16, read_u16_be, read_i16_be, read_u32_be, read_uint};
use crate::daecore::daetype::format::class_def;
use crate::daecore::daetype::format::ivs::variation_index;
use super::super::anchor;
use super::super::gdef;
use super::super::device_table;
use super::schema::{
    Schema, CountSource, DropPolicy, RebuildPolicy, OffsetWidth, PayloadShape, EmptyPolicy, EnvRef, Name, Width,
};
use super::value::{Covered, Matrix, Record, Value};
use crate::daecore::Error;

// Parse work for one table, 64 units a byte, the sanitizer's bound: a unit per node or slot
// read, glyph listed, or eight bytes held or named. A coverage is read once and shared.
pub(crate) struct Work {
    units: usize,
    coverages: BTreeMap<usize, Coverage>,
    // A bit per byte of the table, set where a subtable or child table has been read.
    seen: BitSet,
    // Each shared coverage's bytes once built, for every subtable that names it.
    pub(crate) built_coverages: BTreeMap<*const u32, Rc<[u8]>>,
    // Each lookup's number in the subset, where it numbers them afresh.
    pub(crate) lookups: Option<Vec<u16>>,
}

impl Work {
    pub(crate) fn for_table(len: usize) -> Work {
        Work {
            units: len.saturating_mul(64).max(1 << 16),
            coverages: BTreeMap::new(),
            seen: BitSet::new(len),
            built_coverages: BTreeMap::new(),
            lookups: None,
        }
    }

    pub(crate) fn spend(&mut self, units: usize) -> Result<(), Error> {
        let Some(left) = self.units.checked_sub(units) else {
            return Err(Error::limit("generic: work budget exhausted"));
        };
        self.units = left;
        Ok(())
    }

    // Whether a table at `at` was read before, marking it read. Past the table's end either answer
    // serves, as nothing there can be read.
    pub(crate) fn seen(&mut self, at: usize) -> bool {
        !self.seen.insert(at)
    }
}

// A coverage's glyphs as the font lists them, and those that survive under their new IDs.
type Coverage = (Rc<[u32]>, Rc<[u32]>);

// A node's own units: one to visit it and what a Value holds; likewise a class matrix's record.
const NODE: usize = 1 + core::mem::size_of::<Value>() / 8;
const RECORD: usize = 1 + core::mem::size_of::<Record>() / 8;

// Device tables kept, dropped, or for an instance each VariationIndex folded into its value (added
// to the record if absent) and dropped, while Device tables of sizes stay.
#[derive(Clone, Copy)]
pub(crate) enum Devices<'a> {
    Keep,
    // Device tables of sizes kept, VariationIndex ones dropped, as without a store they vary nothing.
    Sizes,
    Strip,
    Vary(&'a dyn Fn(u16, u16) -> i32),
}

impl Devices<'_> {
    // A ValueFormat as written: the low byte, the reserved high bits left out.
    fn format(self, raw: u16) -> u16 {
        match self {
            Devices::Keep | Devices::Sizes => raw & 0x00FF,
            Devices::Strip => raw & 0x000F,
            Devices::Vary(_) => (raw & 0x00FF) | ((raw >> 4) & 0x000F),
        }
    }
}

// The glyphs a parse keeps, under their new IDs: a subset's, or every glyph as it is where nothing is subset.
#[derive(Clone, Copy)]
pub(crate) enum Kept<'k> {
    Subset(&'k GlyphSet, &'k [u32]),
    All,
}

impl Kept<'_> {
    fn get(self, g: u32) -> Option<u32> {
        match self {
            Kept::Subset(active, gid_map) => remap_gid(active, gid_map, g),
            Kept::All => Some(g),
        }
    }
}

// The counts and value formats a subtable binds, by name.
pub(super) type Scalars = [Option<u32>; Name::COUNT];

pub(crate) struct Env<'b, 'w> {
    scalars: Scalars,
    coverages: [Option<Rc<[u32]>>; Name::COUNT],
    devices: Devices<'w>,
    kept: Kept<'w>,
    work: &'w mut Work,
    // Child tables by where they start and their schema. All a child reads from outside it (value
    // formats, class counts, bound coverages) is fixed for the subtable, so one parse serves each naming.
    children: BTreeMap<(usize, usize), Option<Rc<Value<'b>>>>,
    // By value format, the Device fields some record of it fills.
    filled: [u16; Name::COUNT],
}

impl<'b, 'w> Env<'b, 'w> {
    pub(crate) fn new(devices: Devices<'w>, kept: Kept<'w>, work: &'w mut Work) -> Self {
        let (scalars, coverages) = ([None; Name::COUNT], [const { None }; Name::COUNT]);
        Env { scalars, coverages, devices, kept, work, children: BTreeMap::new(), filled: [0; Name::COUNT] }
    }

    // Each value format's fields less the Device fields none of its records fills, where any are.
    pub(super) fn unfilled(&self) -> Option<[u16; Name::COUNT]> {
        let mut written = [0xFFFF; Name::COUNT];
        for name in [Name::Vf, Name::Vf1, Name::Vf2] {
            let Some(raw) = self.scalar(name) else { continue };
            written[name as usize] = !(self.devices.format(raw as u16) & 0x00F0 & !self.filled[name as usize]);
        }
        written.iter().any(|&w| w != 0xFFFF).then_some(written)
    }

    // A device offset's table as the output holds it, with the delta it adds when instancing.
    fn device(&mut self, buf: &'b [u8], at: Option<usize>) -> Result<(Option<&'b [u8]>, i32), Error> {
        let (Some(at), false) = (at, matches!(self.devices, Devices::Strip)) else { return Ok((None, 0)) };
        let device = device_table(buf, at);
        self.work.spend(device.map_or(1, |d| 1 + d.len() / 8))?;
        Ok(match (self.devices, device) {
            (Devices::Vary(vary), Some(d)) => match variation_index(d, 0) {
                Some((outer, inner)) => (None, vary(outer, inner)),
                None => (Some(d), 0),
            },
            (Devices::Sizes, Some(d)) if variation_index(d, 0).is_some() => (None, 0),
            (_, device) => (device, 0),
        })
    }

    fn scalar(&self, name: Name) -> Option<u32> {
        self.scalars[name as usize]
    }

    fn set_scalar(&mut self, name: Name, value: u32) {
        self.scalars[name as usize] = Some(value);
    }

    fn coverage(&self, name: Name) -> Option<Rc<[u32]>> {
        self.coverages[name as usize].clone()
    }

    fn set_coverage(&mut self, name: Name, gids: Rc<[u32]>) {
        self.coverages[name as usize] = Some(gids);
    }

    fn coverage_at(&mut self, buf: &[u8], at: usize) -> Result<Coverage, Error> {
        if let Some((raw, kept)) = self.work.coverages.get(&at) {
            return Ok((raw.clone(), kept.clone()));
        }
        let gids = super::super::parse_coverage(buf, at).map_err(|e| e.in_context("generic"))?;
        self.work.spend(gids.len())?;
        let raw: Rc<[u32]> = gids.into();
        let kept = match self.kept {
            Kept::All => raw.clone(),
            // Format 2's or 4's ranges ascend, as it parsed, so its kept glyphs are found range by range.
            Kept::Subset(active, gid_map) if matches!(read_u16_be(buf, at), Some(2 | 4)) => {
                let w = if read_u16_be(buf, at) == Some(2) { 2 } else { 3 };
                let ranges = (0..read_uint(buf, at + 2, w).unwrap_or(0) as usize).filter_map(|i| {
                    let range = at + 2 + w + i * 3 * w;
                    Some((read_uint(buf, range, w)?, read_uint(buf, range + w, w)?))
                });
                ranges.flat_map(|(first, last)| active.within(first, last))
                    .filter_map(|g| gid_map.get(g as usize).copied())
                    .collect()
            }
            subset => raw.iter().filter_map(|&g| subset.get(g)).collect(),
        };
        self.work.coverages.insert(at, (raw.clone(), kept.clone()));
        Ok((raw, kept))
    }

    // A ClassDef's classes for the glyphs the subset keeps, by their new ids, charged for every glyph it classes.
    fn class_def_at(&mut self, buf: &[u8], at: usize) -> Result<Vec<(u32, u32)>, Error> {
        let runs = class_def::class_runs(buf.get(at..).unwrap_or_default())
            .ok_or(Error::malformed("generic: ClassDef: truncated"))?;
        self.work.spend(runs.iter().map(|&(first, last, _)| (last - first) as usize + 1).sum())?;
        Ok(match self.kept {
            Kept::All => runs.into_iter().flat_map(|(first, last, c)| (first..=last).map(move |g| (g, c))).collect(),
            Kept::Subset(active, gid_map) => runs.into_iter()
                .flat_map(|(first, last, c)| active.within(first, last).map(move |g| (g, c)))
                .filter_map(|(g, c)| Some((*gid_map.get(g as usize)?, c)))
                .collect(),
        })
    }
}

// A child table, kept once named a second time so it is read at most twice per subtable. Leaves
// (anchors, coverages, class definitions, carets) are cheap and charged at every naming instead.
fn parse_child<'b>(
    buf: &'b [u8],
    at: usize,
    schema: &Schema,
    env: &mut Env<'b, '_>,
) -> Result<Option<Rc<Value<'b>>>, Error> {
    if matches!(schema, Schema::Coverage(..) | Schema::ClassDef(_) | Schema::CaretValue)
        || !env.work.seen(at)
    {
        let (value, _) = generic_parse(buf, at, at, schema, env)?;
        return Ok(value.map(Rc::new));
    }
    let key = (at, schema as *const Schema as usize);
    if let Some(done) = env.children.get(&key) {
        env.work.spend(1)?;
        return Ok(done.clone());
    }
    let (value, _) = generic_parse(buf, at, at, schema, env)?;
    let value = value.map(Rc::new);
    env.children.insert(key, value.clone());
    Ok(value)
}

fn offset_from(anchor: usize, rel: u32) -> Result<usize, Error> {
    anchor.checked_add(rel as usize).ok_or(Error::malformed("generic: offset overflows"))
}

pub(super) fn read_offset(buf: &[u8], at: usize, width: OffsetWidth) -> Result<u32, Error> {
    match width {
        OffsetWidth::W16 => read_u16_be(buf, at).map(u32::from),
        OffsetWidth::W24 => read_uint(buf, at, 3),
        OffsetWidth::W32 => read_u32_be(buf, at),
    }
    .ok_or(Error::malformed("generic: truncated (offset)"))
}

pub(crate) fn generic_parse<'b>(
    buf: &'b [u8], pos: usize, anchor: usize, schema: &Schema, env: &mut Env<'b, '_>,
) -> Result<(Option<Value<'b>>, usize), Error> {
    env.work.spend(NODE)?;

    match schema {
        Schema::U16 => {
            let v = read_u16_be(buf, pos).ok_or(Error::malformed("generic: truncated (U16)"))?;
            Ok((Some(Value::U16(v)), 2))
        }
        Schema::LookupIndex => {
            let v = read_u16_be(buf, pos).ok_or(Error::malformed("generic: truncated (LookupIndex)"))?;
            Ok((Some(Value::LookupIndex(v)), 2))
        }
        Schema::U24 => {
            let v = read_uint(buf, pos, 3).ok_or(Error::malformed("generic: truncated (U24)"))?;
            Ok((Some(Value::U24(v)), 3))
        }
        Schema::GlyphId | Schema::GlyphId24 => {
            let w = if matches!(schema, Schema::GlyphId) { Width::W16 } else { Width::W24 };
            let g = read_uint(buf, pos, w.bytes()).ok_or(Error::malformed("generic: truncated (GlyphId)"))?;
            Ok((env.kept.get(g).map(|g| Value::Glyph(w, g)), w.bytes()))
        }
        Schema::Offset(width, child_schema) => {
            let rel = read_offset(buf, pos, *width)?;
            if rel == 0 {
                return Ok((Some(Value::Offset(*width, None)), width.bytes()));
            }
            let at = offset_from(anchor, rel)?;
            if let Schema::Anchor = child_schema {
                return Ok((Some(anchor_at(buf, at, *width, env)?), width.bytes()));
            }
            let child = parse_child(buf, at, child_schema, env)?;
            match child {
                Some(v) => Ok((Some(Value::Offset(*width, Some(v))), width.bytes())),
                None => Ok((None, width.bytes())),
            }
        }
        Schema::Array(elem_schema, count_source, drop_policy) => {
            let Some(count) = resolve_count(count_source, &env.scalars)? else { return Ok((None, 0)) };
            // An array of one is held as its element, as every reader takes an array's elements in turn: a base's
            // anchors in a subtable of one mark class allocate nothing.
            if count == 1 {
                let (v, consumed) = generic_parse(buf, pos, anchor, elem_schema, env)?;
                let left = usize::from(v.is_some());
                if !survives(*drop_policy, v.is_none(), left) { return Ok((None, consumed)); }
                write_back_count(count_source, env, left);
                return Ok((Some(v.unwrap_or(Value::Array(Vec::new()))), consumed));
            }
            let mut survivors: Vec<Value> = Vec::with_capacity(count.min(256));
            let (mut cursor, mut dropped) = (pos, false);
            for i in 0..count {
                let (v, consumed) = generic_parse(buf, cursor, anchor, elem_schema, env)?;
                if consumed == 0 && i + 1 < count {
                    return Err(Error::malformed("generic: array element consumes no input, so its count is unbounded"));
                }
                match v {
                    Some(v) => survivors.push(v),
                    None => dropped = true,
                }
                cursor += consumed;
            }
            let total_consumed = cursor - pos;
            if !survives(*drop_policy, dropped, survivors.len()) { return Ok((None, total_consumed)); }
            write_back_count(count_source, env, survivors.len());
            Ok((Some(Value::Array(survivors)), total_consumed))
        }
        Schema::Glyphs(w, count_source, drop_policy) => {
            let Some(count) = resolve_count(count_source, &env.scalars)? else { return Ok((None, 0)) };
            let size = w.bytes();
            let mut glyphs = Vec::with_capacity(count.min(buf.len().saturating_sub(pos) / size));
            let mut dropped = false;
            for i in 0..count {
                env.work.spend(NODE)?;
                let g = read_uint(buf, pos + size * i, size).ok_or(Error::malformed("generic: truncated (GlyphId)"))?;
                match env.kept.get(g) {
                    Some(new_g) => glyphs.push(new_g),
                    None => dropped = true,
                }
            }
            if !survives(*drop_policy, dropped, glyphs.len()) { return Ok((None, size * count)); }
            write_back_count(count_source, env, glyphs.len());
            Ok((Some(Value::Glyphs(*w, glyphs)), size * count))
        }
        Schema::OffsetArray(child_schema, count_source, rebuild_policy, width) => {
            let Some(count) = resolve_count(count_source, &env.scalars)? else { return Ok((None, 0)) };
            let elem_width = width.bytes();
            let fits = buf.len().saturating_sub(pos) / elem_width;
            let mut slots: Vec<Option<Rc<Value>>> = Vec::with_capacity(count.min(fits));
            let mut cursor = pos;
            for _ in 0..count {
                env.work.spend(1)?;
                let rel = read_offset(buf, cursor, *width)?;
                if rel == 0 {
                    slots.push(None);
                } else {
                    slots.push(parse_child(buf, offset_from(anchor, rel)?, child_schema, env)?);
                }
                cursor += elem_width;
            }
            let total_consumed = cursor - pos;
            let final_slots = match rebuild_policy {
                RebuildPolicy::CompactSurvivors => { let mut s = slots; s.retain(Option::is_some); s }
                RebuildPolicy::PreserveSlotPositions => {
                    let mut s = slots;
                    while matches!(s.last(), Some(None)) { s.pop(); }
                    s
                }
            };
            if final_slots.is_empty() {
                Ok((None, total_consumed))
            } else {
                write_back_count(count_source, env, final_slots.len());
                Ok((Some(Value::OffsetArray(*width, final_slots)), total_consumed))
            }
        }
        Schema::Struct(fields) => {
            let mut cursor = pos;
            let mut out_fields: Vec<Value> = Vec::with_capacity(fields.len());
            let mut any_failed = false;
            for field in fields.iter() {
                let (v, consumed) = generic_parse(buf, cursor, anchor, &field.schema, env)
                    .map_err(|e| e.in_context(field.name))?;
                cursor += consumed;
                match v {
                    Some(val) => {
                        bind_field(env, field.bind, &val);
                        out_fields.push(val);
                    }
                    None => any_failed = true,
                }
            }
            let total_consumed = cursor - pos;
            if any_failed { return Ok((None, total_consumed)); }
            for (field, value) in fields.iter().zip(out_fields.iter_mut()) {
                if let Some(actual) = field.bind.and_then(|name| env.scalar(name)) {
                    match value {
                        Value::U16(v) => *v = actual as u16,
                        Value::U24(v) => *v = actual,
                        _ => {}
                    }
                }
            }
            Ok((Some(Value::Struct(out_fields)), total_consumed))
        }
        Schema::FormatSwitch(peek_offset, variants) => {
            let tag =
                read_u16_be(buf, pos + peek_offset).ok_or(Error::malformed("generic: truncated (FormatSwitch tag)"))?;
            let variant = variants.iter().find(|(t, _)| *t == tag).ok_or_else(|| {
                Error::owned(crate::daecore::ErrorKind::Unsupported, format!("generic: unknown format tag {tag}"))
            })?;
            generic_parse(buf, pos, anchor, &variant.1, env)
        }
        Schema::ValueRecordField(EnvRef(name)) => {
            let format = env.scalar(*name).ok_or_else(|| {
                Error::owned(crate::daecore::ErrorKind::Malformed, format!("generic: unbound env ref '{name:?}'"))
            })?;
            let (record, consumed) = parse_record(buf, pos, anchor, format as u16, *name, env)?;
            Ok((Some(Value::ValueRecord(record)), consumed))
        }
        Schema::Coverage(policy, raw_bind) => {
            let (gids, survivors) = env.coverage_at(buf, pos)?;
            if let Some(EnvRef(name)) = raw_bind {
                env.set_coverage(*name, gids);
            }
            match policy {
                EmptyPolicy::Fail if survivors.is_empty() => Ok((None, 0)),
                _ => Ok((Some(Value::Coverage(survivors)), 0)),
            }
        }
        Schema::ClassMatrix(EnvRef(vf1), EnvRef(vf2), w) => {
            let [f1, f2] = [*vf1, *vf2].map(|name| env.scalar(name).map(|f| f as u16));
            let (Some(f1), Some(f2)) = (f1, f2) else {
                return Err(Error::malformed("generic: unbound value format for a class matrix"));
            };
            let l = w.long().bytes();
            let cd1_rel = read_offset(buf, pos, w.long())?;
            let cd2_rel = read_offset(buf, pos + l, w.long())?;
            let c1 = read_u16_be(buf, pos + 2 * l)
                .ok_or(Error::malformed("generic: truncated (ClassMatrix class1Count)"))? as usize;
            let c2 = read_u16_be(buf, pos + 2 * l + 2)
                .ok_or(Error::malformed("generic: truncated (ClassMatrix class2Count)"))? as usize;
            let grid_at = pos + 2 * l + 4;

            let mut filtered = |rel: u32| -> Result<Vec<(u32, u32)>, Error> {
                if rel == 0 { return Ok(Vec::new()); }
                env.class_def_at(buf, offset_from(anchor, rel)?)
            };
            let cd1 = filtered(cd1_rel)?;
            let cd2 = filtered(cd2_rel)?;

            // A glyph in a class past the count gets nothing from the source, as the shaper refuses
            // the pair; it keeps a class past the new count rather than falling into class 0.
            let surviving = |entries: &[(u32, u32)], count: usize| -> (Vec<usize>, BTreeMap<u32, u16>) {
                let mut classes: Vec<usize> = entries
                    .iter()
                    .map(|&(_, c)| c as usize)
                    .filter(|&c| c != 0 && c < count)
                    .collect();
                classes.sort_unstable();
                classes.dedup();
                let mut order: Vec<usize> = Vec::with_capacity(classes.len() + 1);
                if count > 0 { order.push(0); }
                order.extend(classes);
                let map = order.iter().enumerate().map(|(new, &old)| (old as u32, new as u16)).collect();
                (order, map)
            };
            let (rows, map1) = surviving(&cd1, c1);
            let (cols, map2) = surviving(&cd2, c2);

            let mut grid: Vec<Record> = Vec::new();
            if !rows.is_empty() && !cols.is_empty() {
                let stride = ((f1 & 0x00FF).count_ones() + (f2 & 0x00FF).count_ones()) as usize * 2;
                if stride == 0 {
                    return Err(Error::malformed(
                        "generic: class matrix cell consumes no input, so its counts are unbounded",
                    ));
                }
                // Each cell is its own run of `stride` bytes, so no more can be read than the table holds,
                // however many classes the font claims.
                let room = buf.len().saturating_sub(grid_at) / stride;
                grid.reserve(rows.len().saturating_mul(cols.len()).min(room).saturating_mul(2));
                for &r in &rows {
                    for &c in &cols {
                        let at = r.checked_mul(c2)
                            .and_then(|i| i.checked_add(c))
                            .and_then(|i| i.checked_mul(stride))
                            .and_then(|i| i.checked_add(grid_at))
                            .ok_or(Error::malformed("generic: class matrix cell overflows"))?;
                        env.work.spend(2 * RECORD)?;
                        let (first, read) = parse_record(buf, at, anchor, f1, *vf1, env)?;
                        grid.push(first);
                        grid.push(parse_record(buf, at + read, anchor, f2, *vf2, env)?.0);
                    }
                }
            }

            let renumber = |entries: Vec<(u32, u32)>, map: &BTreeMap<u32, u16>, past: usize| -> Vec<(u32, u16)> {
                entries.into_iter().map(|(g, c)| (g, map.get(&c).copied().unwrap_or(past as u16))).collect()
            };
            Ok((
                Some(Value::ClassMatrix(Box::new(Matrix {
                    width: *w,
                    class_def1: renumber(cd1, &map1, rows.len()),
                    class_def2: renumber(cd2, &map2, cols.len()),
                    class1_count: rows.len() as u16,
                    class2_count: cols.len() as u16,
                    grid,
                }))),
                2 * l + 4,
            ))
        }
        Schema::ClassDef(policy) => {
            let survivors = env.class_def_at(buf, pos)?;
            match policy {
                EmptyPolicy::Fail if survivors.is_empty() => Ok((None, 0)),
                _ => Ok((Some(Value::ClassDef(survivors)), 0)),
            }
        }
        Schema::ValueFormatField(EnvRef(name)) => {
            let raw = read_u16_be(buf, pos).ok_or(Error::malformed("generic: truncated (ValueFormatField)"))?;
            env.set_scalar(*name, u32::from(raw));
            Ok((Some(Value::ValueFormat(*name, env.devices.format(raw))), 2))
        }
        Schema::Anchor => Err(Error::malformed("generic: an anchor is read through the offset naming it")),
        Schema::CoveredArray(extra_fields, payload_schema, shape, w) => {
            let cov_rel = read_offset(buf, pos, w.long())?;
            let coverage: Rc<[u32]> = if cov_rel == 0 {
                Rc::from([])
            } else {
                env.coverage_at(buf, offset_from(anchor, cov_rel)?)?.0
            };
            let mut cursor = pos + w.long().bytes();
            let mut extra_values: Vec<Value> = Vec::with_capacity(extra_fields.len());
            for field in extra_fields.iter() {
                let (v, consumed) = generic_parse(buf, cursor, anchor, &field.schema, env)
                    .map_err(|e| e.in_context(field.name))?;
                cursor += consumed;
                let val = v.ok_or(Error::malformed(
                    "generic: CoveredArray: an extra field (never GID-tagged in any real case) \
                     unexpectedly failed to survive",
                ))?;
                bind_field(env, field.bind, &val);
                extra_values.push(val);
            }
            let count = read_uint(buf, cursor, w.bytes())
                .ok_or(Error::malformed("generic: truncated (CoveredArray Count)"))? as usize;
            cursor += w.bytes();
            let mut entries: Vec<(u32, Value)> = Vec::with_capacity(count.min(coverage.len()));
            for i in 0..count {
                let new_gid = coverage.get(i).and_then(|&g| env.kept.get(g));
                if new_gid.is_none() && let Some(width) = skip_offset(buf, cursor, *shape, env)? {
                    cursor += width;
                    continue;
                }
                let (payload_opt, consumed) =
                    parse_payload(buf, cursor, anchor, payload_schema, *shape, env)?;
                cursor += consumed;
                if let (Some(pv), Some(new_gid)) = (payload_opt, new_gid) {
                    entries.push((new_gid, pv));
                }
            }
            let total_consumed = cursor - pos;
            sort_by_glyph(&mut entries);
            if entries.is_empty() {
                Ok((None, total_consumed))
            } else {
                let covered = Covered { fields: extra_values, shape: *shape, entries, width: *w };
                Ok((Some(Value::CoveredArray(Box::new(covered))), total_consumed))
            }
        }
        Schema::ZippedWithBoundCoverage(EnvRef(cov_name), payload_schema, shape, drop_policy, w) => {
            let coverage = env.coverage(*cov_name).ok_or_else(|| {
                Error::owned(
                    crate::daecore::ErrorKind::Malformed,
                    format!("generic: unbound coverage ref '{cov_name:?}'"),
                )
            })?;
            // Read once a table, the coverage is walked again by every subtable binding it.
            env.work.spend(coverage.len())?;
            let count = read_uint(buf, pos, w.bytes())
                .ok_or(Error::malformed("generic: truncated (ZippedWithBoundCoverage Count)"))?;
            let count = count as usize;
            let mut cursor = pos + w.bytes();
            let mut payloads: Vec<Option<Value>> = Vec::with_capacity(count.min(coverage.len()));
            for i in 0..count {
                let kept = coverage.get(i).is_some_and(|&g| env.kept.get(g).is_some());
                if !kept && let Some(width) = skip_offset(buf, cursor, *shape, env)? {
                    cursor += width;
                    payloads.push(None);
                    continue;
                }
                let (payload, consumed) =
                    parse_payload(buf, cursor, anchor, payload_schema, *shape, env)?;
                cursor += consumed;
                payloads.push(payload);
            }
            let total_consumed = cursor - pos;
            // One entry for each glyph the rebuilt coverage keeps, in its order, so the two stay in
            // step: a glyph with no record keeps a NULL offset, since dropping it would shift the rest.
            let mut kept: Vec<(u32, usize)> = coverage.iter().enumerate()
                .filter_map(|(i, &g)| env.kept.get(g).map(|ng| (ng, i)))
                .collect();
            sort_by_glyph(&mut kept);
            let mut entries: Vec<Option<Value>> = Vec::with_capacity(kept.len());
            for (_, i) in kept {
                let payload = payloads.get_mut(i).and_then(Option::take);
                if payload.is_none() && matches!(shape, PayloadShape::Inline) {
                    return Err(Error::missing("generic: a covered glyph has no inline record"));
                }
                entries.push(payload);
            }
            match drop_policy {
                DropPolicy::FilterSurvivorsOrFail if entries.is_empty() => Ok((None, total_consumed)),
                DropPolicy::AllOrNothing if entries.iter().any(Option::is_none) => Ok((None, total_consumed)),
                _ => Ok((Some(Value::ZippedWithBoundCoverage(*shape, *w, entries)), total_consumed)),
            }
        }
        Schema::CaretValue => {
            let format = read_u16_be(buf, pos).ok_or(Error::malformed("generic: truncated (CaretValue)"))?;
            let mut cv = gdef::parse_caret_value(buf, pos)
                .ok_or(Error::malformed("generic: CaretValue: truncated or unrecognized format"))?;
            // Format 3 adjusts its coordinate by a Device or VariationIndex table, which a variable
            // font's carets need to vary.
            let at = match read_u16_be(buf, pos + 4) {
                Some(rel) if format == 3 && rel != 0 => Some(offset_from(pos, u32::from(rel))?),
                _ => None,
            };
            let (device, delta) = env.device(buf, at)?;
            if let gdef::CaretValue::Coordinate(c) = &mut cv {
                *c = add_i16(*c, delta);
            }
            Ok((Some(Value::CaretValue(cv, device)), if format == 3 { 6 } else { 4 }))
        }
        Schema::DeltaCoverageSubst(w) => {
            let (l, size) = (w.long().bytes(), w.bytes());
            let coverage = covered_glyphs(buf, pos + 2, anchor, w.long(), env)?;
            let delta = read_uint(buf, pos + 2 + l, size)
                .ok_or(Error::malformed("generic: truncated (DeltaCoverageSubst delta)"))?;
            let mask = if *w == Width::W16 { 0xFFFF } else { 0xFF_FFFF };
            env.work.spend(coverage.len())?;
            let subs = coverage.iter().map(|&g| g.wrapping_add(delta) & mask);
            Ok((glyph_map(&coverage, subs, env.kept), 2 + l + size))
        }
        Schema::CoveredGlyphs(w) => {
            let (l, size) = (w.long().bytes(), w.bytes());
            let coverage = covered_glyphs(buf, pos + 2, anchor, w.long(), env)?;
            let at = pos + 2 + l;
            let count =
                read_uint(buf, at, size).ok_or(Error::malformed("generic: truncated (CoveredGlyphs Count)"))? as usize;
            let mut subs = Vec::with_capacity(count.min(coverage.len()));
            for i in 0..count {
                env.work.spend(NODE)?;
                subs.push(
                    read_uint(buf, at + size * (1 + i), size)
                        .ok_or(Error::malformed("generic: truncated (GlyphId)"))?,
                );
            }
            Ok((glyph_map(&coverage, subs.into_iter(), env.kept), 2 + l + size * (1 + count)))
        }
    }
}

// The glyphs a coverage offset at `at` names, as the font lists them; a NULL offset names none.
fn covered_glyphs(
    buf: &[u8], at: usize, anchor: usize, width: OffsetWidth, env: &mut Env<'_, '_>,
) -> Result<Rc<[u32]>, Error> {
    match read_offset(buf, at, width)? {
        0 => Ok(Rc::from([])),
        rel => Ok(env.coverage_at(buf, offset_from(anchor, rel)?)?.0),
    }
}

// Each covered glyph paired with its substitute, both kept, under their new IDs; None if no pair is.
fn glyph_map<'b>(coverage: &[u32], subs: impl Iterator<Item = u32>, kept: Kept) -> Option<Value<'b>> {
    let remap = |g| kept.get(g);
    let mut pairs: Vec<(u32, u32)> =
        coverage.iter().zip(subs).filter_map(|(&g, sub)| Some((remap(g)?, remap(sub)?))).collect();
    sort_by_glyph(&mut pairs);
    (!pairs.is_empty()).then_some(Value::GlyphMap(pairs))
}

// A ValueRecord in `format`, any VariationIndex being applied added to its value. The high byte of
// a ValueFormat is reserved: a field is held only for each low bit.
fn parse_record<'b>(
    buf: &'b [u8], pos: usize, anchor: usize, format: u16, name: Name, env: &mut Env<'b, '_>,
) -> Result<(Record<'b>, usize), Error> {
    let format = format & 0x00FF;
    let out_format = env.devices.format(format);
    let mut fields = [0i32; 4];
    let mut cursor = pos;
    for (bit, field) in fields.iter_mut().enumerate() {
        if format & (1 << bit) != 0 {
            *field =
                i32::from(read_i16_be(buf, cursor).ok_or(Error::malformed("generic: truncated (ValueRecordField)"))?);
            cursor += 2;
        }
    }
    let mut devices: Option<Box<[Option<&[u8]>; 4]>> = None;
    let mut n = 0;
    for (bit, field) in fields.iter_mut().enumerate() {
        if format & (0x10 << bit) == 0 {
            continue;
        }
        let rel = read_u16_be(buf, cursor).ok_or(Error::malformed("generic: truncated (ValueRecord device offset)"))?;
        cursor += 2;
        let at = if rel == 0 { None } else { Some(offset_from(anchor, u32::from(rel))?) };
        let (device, delta) = env.device(buf, at)?;
        *field = field.saturating_add(delta);
        if out_format & (0x10 << bit) == 0 {
            continue;
        }
        if device.is_some() {
            env.filled[name as usize] |= 0x10 << bit;
            devices.get_or_insert_with(|| Box::new([None; 4]))[n] = device;
        }
        n += 1;
    }
    let mut values = [0i16; 4];
    let present = (0..4).filter(|bit| out_format & (1 << bit) != 0);
    for (slot, bit) in values.iter_mut().zip(present) {
        *slot = clamp_i16(fields[bit]);
    }
    Ok((Record { format: out_format, values, name, devices }, cursor - pos))
}

// The anchor table at `at`, with the width of the offset naming it.
fn anchor_at<'b>(buf: &'b [u8], at: usize, width: OffsetWidth, env: &mut Env<'b, '_>) -> Result<Value<'b>, Error> {
    env.work.spend(NODE)?;
    let (mut x, mut y) =
        anchor::parse_anchor(buf, at).ok_or(Error::malformed("generic: Anchor: truncated or unrecognized format"))?;
    let point = anchor::parse_anchor_point(buf, at);
    let mut devices: Option<Box<[Option<&[u8]>; 2]>> = None;
    if read_u16_be(buf, at) == Some(3) {
        for (i, (slot, coord)) in [(6usize, &mut x), (8, &mut y)].into_iter().enumerate() {
            let device_at = match read_u16_be(buf, at + slot) {
                Some(rel) if rel != 0 => Some(offset_from(at, u32::from(rel))?),
                _ => None,
            };
            let (device, delta) = env.device(buf, device_at)?;
            if device.is_some() {
                devices.get_or_insert_with(|| Box::new([None; 2]))[i] = device;
            }
            *coord = add_i16(*coord, delta);
        }
    }
    Ok(Value::Anchor(width, x, y, point, devices))
}

// One record of a covered array, inline or behind an offset; a NULL offset is no record.
fn parse_payload<'b>(
    buf: &'b [u8], cursor: usize, anchor: usize, schema: &Schema, shape: PayloadShape, env: &mut Env<'b, '_>,
) -> Result<(Option<Value<'b>>, usize), Error> {
    match shape {
        PayloadShape::Inline => generic_parse(buf, cursor, anchor, schema, env),
        PayloadShape::Offsets(width) => {
            env.work.spend(1)?;
            let rel = read_offset(buf, cursor, width)?;
            if rel == 0 {
                return Ok((None, width.bytes()));
            }
            let child = parse_child(buf, offset_from(anchor, rel)?, schema, env)?;
            Ok((child.map(|c| Value::Offset(width, Some(c))), width.bytes()))
        }
    }
}

// The width of an offset to a child the subset drops, read but not followed; None for an inline
// payload, whose width is known only by parsing it.
fn skip_offset(buf: &[u8], cursor: usize, shape: PayloadShape, env: &mut Env) -> Result<Option<usize>, Error> {
    match shape {
        PayloadShape::Offsets(width) => {
            env.work.spend(1)?;
            read_offset(buf, cursor, width).map(|_| Some(width.bytes()))
        }
        PayloadShape::Inline => Ok(None),
    }
}

// Records keyed by glyph in the order the rebuilt coverage lists them: sorted, a glyph listed twice
// keeping its first record, as a coverage out of order or with repeats would pair others' data.
fn sort_by_glyph<T>(entries: &mut Vec<(u32, T)>) {
    entries.sort_by_key(|e| e.0);
    entries.dedup_by_key(|e| e.0);
}

fn bind_field(env: &mut Env, bind: Option<Name>, val: &Value) {
    let Some(bind_name) = bind else { return };
    match val {
        Value::U16(raw) => env.set_scalar(bind_name, u32::from(*raw)),
        Value::U24(raw) => env.set_scalar(bind_name, *raw),
        _ => {}
    }
}

// Whether an array is kept: all or nothing loses it to any element dropped, and filter-or-fail to
// having none left.
fn survives(policy: DropPolicy, dropped: bool, left: usize) -> bool {
    match policy {
        DropPolicy::AllOrNothing => !dropped,
        DropPolicy::FilterSurvivors => true,
        DropPolicy::FilterSurvivorsOrFail => left > 0,
    }
}

// None for a count read as a field minus one that is 0: the rule or ligature it counts is malformed
// and left out, as the shaper skips it.
pub(super) fn resolve_count(source: &CountSource, scalars: &Scalars) -> Result<Option<usize>, Error> {
    let (CountSource::Field(EnvRef(name)) | CountSource::FieldMinusOne(EnvRef(name))) = source;
    let v = scalars[*name as usize].ok_or_else(|| {
        Error::owned(crate::daecore::ErrorKind::Malformed, format!("generic: unbound env ref '{name:?}' for count"))
    })? as usize;
    Ok(match source {
        CountSource::Field(_) => Some(v),
        CountSource::FieldMinusOne(_) => v.checked_sub(1),
    })
}

fn write_back_count(source: &CountSource, env: &mut Env, actual_len: usize) {
    match source {
        CountSource::Field(EnvRef(name)) => { env.set_scalar(*name, actual_len as u32); }
        CountSource::FieldMinusOne(EnvRef(name)) => { env.set_scalar(*name, (actual_len + 1) as u32); }
    }
}
