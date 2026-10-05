use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use crate::daecore::daetype::decoder::{read_u16_be, read_i16_be, read_u32_be};
use super::super::anchor;
use super::super::gdef;
use super::super::{remap_gid, device_table};
use super::schema::{Schema, CountSource, DropPolicy, RebuildPolicy, OffsetWidth, PayloadShape, EmptyPolicy, EnvRef};
use super::value::{Covered, Matrix, Record, Value};

// Parse work for one table, 64 units a byte as HarfBuzz bounds its sanitizer: a unit per node or
// slot read, glyph listed, or eight bytes held or named. A coverage is read once and shared.
pub(crate) struct Work {
    units: usize,
    coverages: BTreeMap<usize, Coverage>,
    // A bit per byte of the table, set where a subtable or child table has been read.
    seen: Vec<u64>,
    // Each shared coverage's bytes once built, for every subtable that names it.
    pub(crate) built_coverages: BTreeMap<*const u16, Rc<[u8]>>,
}

impl Work {
    pub(crate) fn for_table(len: usize) -> Work {
        Work {
            units: len.saturating_mul(64).max(1 << 16),
            coverages: BTreeMap::new(),
            seen: alloc::vec![0; len.div_ceil(64)],
            built_coverages: BTreeMap::new(),
        }
    }

    fn spend(&mut self, units: usize) -> Result<(), String> {
        self.units = self.units.checked_sub(units).ok_or("generic: work budget exhausted")?;
        Ok(())
    }

    // Whether a table at `at` was read before, marking it read.
    pub(crate) fn seen(&mut self, at: usize) -> bool {
        let Some(word) = self.seen.get_mut(at / 64) else { return false };
        let bit = 1 << (at % 64);
        let before = *word & bit != 0;
        *word |= bit;
        before
    }
}

// A coverage's glyphs as the font lists them, and those that survive under their new IDs.
type Coverage = (Rc<[u16]>, Rc<[u16]>);

// A node's own units: one to visit it and what a Value holds; likewise a class matrix's record.
const NODE: usize = 1 + core::mem::size_of::<Value>() / 8;
const RECORD: usize = 1 + core::mem::size_of::<Record>() / 8;

// Device tables kept, dropped, or for an instance each VariationIndex folded into its value (added
// to the record if absent) and dropped, while Device tables of sizes stay.
#[derive(Clone, Copy)]
pub(crate) enum Devices<'a> {
    Keep,
    Strip,
    Vary(&'a dyn Fn(u16, u16) -> i32),
}

impl Devices<'_> {
    // A ValueFormat as written: the low byte, the reserved high bits left out.
    fn format(self, raw: u16) -> u16 {
        match self {
            Devices::Keep => raw & 0x00FF,
            Devices::Strip => raw & 0x000F,
            Devices::Vary(_) => (raw & 0x00FF) | ((raw >> 4) & 0x000F),
        }
    }
}

// A VariationIndex table's outer and inner index; a Device table of sizes is not one.
fn variation_index(device: &[u8]) -> Option<(u16, u16)> {
    if device.get(4..6)? != [0x80, 0x00] {
        return None;
    }
    Some((read_u16_be(device, 0)?, read_u16_be(device, 2)?))
}

pub(crate) struct Env<'b, 'w> {
    scalars: Vec<(&'static str, u16)>,
    coverages: Vec<(&'static str, Rc<[u16]>)>,
    devices: Devices<'w>,
    work: &'w mut Work,
    // Child tables by where they start and their schema. All a child reads from outside it (value
    // formats, class counts, bound coverages) is fixed for the subtable, so one parse serves each naming.
    children: BTreeMap<(usize, usize), Option<Rc<Value<'b>>>>,
}

impl<'b, 'w> Env<'b, 'w> {
    pub(crate) fn new(devices: Devices<'w>, work: &'w mut Work) -> Self {
        Env { scalars: Vec::new(), coverages: Vec::new(), devices, work, children: BTreeMap::new() }
    }

    // A device offset's table as the output holds it, with the delta it adds when instancing.
    fn device(&mut self, buf: &'b [u8], at: Option<usize>) -> Result<(Option<&'b [u8]>, i32), String> {
        let (Some(at), false) = (at, matches!(self.devices, Devices::Strip)) else { return Ok((None, 0)) };
        let device = device_table(buf, at);
        self.work.spend(device.map_or(1, |d| 1 + d.len() / 8))?;
        Ok(match (self.devices, device) {
            (Devices::Vary(vary), Some(d)) => match variation_index(d) {
                Some((outer, inner)) => (None, vary(outer, inner)),
                None => (Some(d), 0),
            },
            (_, device) => (device, 0),
        })
    }

    fn scalar(&self, name: &str) -> Option<u16> {
        self.scalars.iter().find(|(k, _)| *k == name).map(|&(_, v)| v)
    }

    fn set_scalar(&mut self, name: &'static str, value: u16) {
        match self.scalars.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => self.scalars.push((name, value)),
        }
    }

    fn coverage(&self, name: &str) -> Option<Rc<[u16]>> {
        self.coverages.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())
    }

    fn set_coverage(&mut self, name: &'static str, gids: Rc<[u16]>) {
        match self.coverages.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = gids,
            None => self.coverages.push((name, gids)),
        }
    }

    fn coverage_at(&mut self, buf: &[u8], at: usize, active: &GlyphSet, gid_map: &[u16]) -> Result<Coverage, String> {
        if let Some((raw, kept)) = self.work.coverages.get(&at) {
            return Ok((raw.clone(), kept.clone()));
        }
        let gids = super::super::parse_coverage(buf, at).map_err(|e| format!("generic: {e}"))?;
        self.work.spend(gids.len())?;
        let kept: Rc<[u16]> = gids.iter().filter_map(|&g| remap_gid(active, gid_map, g)).collect();
        let raw: Rc<[u16]> = gids.into();
        self.work.coverages.insert(at, (raw.clone(), kept.clone()));
        Ok((raw, kept))
    }
}

// A child table, kept once named a second time so it is read at most twice per subtable. Leaves
// (anchors, coverages, class definitions, carets) are cheap and charged at every naming instead.
fn parse_child<'b>(
    buf: &'b [u8], at: usize, schema: &Schema, env: &mut Env<'b, '_>, active: &GlyphSet, gid_map: &[u16],
) -> Result<Option<Rc<Value<'b>>>, String> {
    if matches!(schema, Schema::Anchor | Schema::Coverage(..) | Schema::ClassDef(_) | Schema::CaretValue)
        || !env.work.seen(at)
    {
        let (value, _) = generic_parse(buf, at, at, schema, env, active, gid_map)?;
        return Ok(value.map(Rc::new));
    }
    let key = (at, schema as *const Schema as usize);
    if let Some(done) = env.children.get(&key) {
        env.work.spend(1)?;
        return Ok(done.clone());
    }
    let (value, _) = generic_parse(buf, at, at, schema, env, active, gid_map)?;
    let value = value.map(Rc::new);
    env.children.insert(key, value.clone());
    Ok(value)
}

fn offset_from(anchor: usize, rel: u32) -> Result<usize, String> {
    anchor.checked_add(rel as usize).ok_or_else(|| String::from("generic: offset overflows"))
}

fn read_offset(buf: &[u8], at: usize, width: OffsetWidth) -> Result<u32, String> {
    match width {
        OffsetWidth::W16 => read_u16_be(buf, at).map(u32::from),
        OffsetWidth::W32 => read_u32_be(buf, at),
    }
    .ok_or_else(|| String::from("generic: truncated (offset)"))
}

pub(crate) fn generic_parse<'b>(
    buf: &'b [u8],
    pos: usize,
    anchor: usize,
    schema: &Schema,
    env: &mut Env<'b, '_>,
    active: &GlyphSet,
    gid_map: &[u16],
) -> Result<(Option<Value<'b>>, usize), String> {
    env.work.spend(NODE)?;

    match schema {
        Schema::U16 => {
            let v = read_u16_be(buf, pos).ok_or("generic: truncated (U16)")?;
            Ok((Some(Value::U16(v)), 2))
        }
        Schema::GlyphId => {
            let g = read_u16_be(buf, pos).ok_or("generic: truncated (GlyphId)")?;
            Ok((remap_gid(active, gid_map, g).map(Value::Glyph), 2))
        }
        Schema::Offset(width, child_schema) => {
            let rel = read_offset(buf, pos, *width)?;
            if rel == 0 {
                return Ok((Some(Value::Offset(*width, None)), width.bytes()));
            }
            let child = parse_child(buf, offset_from(anchor, rel)?, child_schema, env, active, gid_map)?;
            match child {
                Some(v) => Ok((Some(Value::Offset(*width, Some(v))), width.bytes())),
                None => Ok((None, width.bytes())),
            }
        }
        Schema::Array(elem_schema, count_source, drop_policy) => {
            // A count read as a field minus one that is 0 makes the enclosing rule or ligature
            // malformed: that one is left out, as daegun's shaper and HarfBuzz skip it.
            let Some(count) = resolve_count(count_source, env)? else { return Ok((None, 0)) };
            let mut elems: Vec<Option<Value>> = Vec::with_capacity(count.min(256));
            let mut cursor = pos;
            for i in 0..count {
                let (v, consumed) = generic_parse(buf, cursor, anchor, elem_schema, env, active, gid_map)?;
                if consumed == 0 && i + 1 < count {
                    return Err("generic: array element consumes no input, so its count is unbounded".into());
                }
                elems.push(v);
                cursor += consumed;
            }
            let total_consumed = cursor - pos;
            match drop_policy {
                DropPolicy::AllOrNothing => {
                    if elems.iter().any(Option::is_none) {
                        Ok((None, total_consumed))
                    } else {
                        write_back_count(count_source, env, count);
                        Ok((Some(Value::Array(elems.into_iter().map(Option::unwrap).collect())), total_consumed))
                    }
                }
                DropPolicy::FilterSurvivors => {
                    let survivors: Vec<Value> = elems.into_iter().flatten().collect();
                    write_back_count(count_source, env, survivors.len());
                    Ok((Some(Value::Array(survivors)), total_consumed))
                }
                DropPolicy::FilterSurvivorsOrFail => {
                    let survivors: Vec<Value> = elems.into_iter().flatten().collect();
                    if survivors.is_empty() { Ok((None, total_consumed)) }
                    else {
                        write_back_count(count_source, env, survivors.len());
                        Ok((Some(Value::Array(survivors)), total_consumed))
                    }
                }
            }
        }
        Schema::OffsetArray(child_schema, count_source, rebuild_policy, width) => {
            let Some(count) = resolve_count(count_source, env)? else { return Ok((None, 0)) };
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
                    slots.push(parse_child(buf, offset_from(anchor, rel)?, child_schema, env, active, gid_map)?);
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
            for field in fields {
                let (v, consumed) = generic_parse(buf, cursor, anchor, &field.schema, env, active, gid_map)
                    .map_err(|e| format!("{}: {e}", field.name))?;
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
                if let (Some(bind_name), Value::U16(_)) = (field.bind, &*value)
                    && let Some(actual) = env.scalar(bind_name) {
                        *value = Value::U16(actual);
                    }
            }
            Ok((Some(Value::Struct(out_fields)), total_consumed))
        }
        Schema::FormatSwitch(peek_offset, variants) => {
            let tag = read_u16_be(buf, pos + peek_offset).ok_or("generic: truncated (FormatSwitch tag)")?;
            let variant = variants.iter().find(|(t, _)| *t == tag)
                .ok_or_else(|| format!("generic: unknown format tag {tag}"))?;
            generic_parse(buf, pos, anchor, &variant.1, env, active, gid_map)
        }
        Schema::ValueRecordField(EnvRef(name)) => {
            let format = env.scalar(name).ok_or_else(|| format!("generic: unbound env ref '{name}'"))?;
            let (record, consumed) = parse_record(buf, pos, anchor, format, env)?;
            Ok((Some(Value::ValueRecord(record)), consumed))
        }
        Schema::Coverage(policy, raw_bind) => {
            let (gids, survivors) = env.coverage_at(buf, pos, active, gid_map)?;
            if let Some(EnvRef(name)) = raw_bind {
                env.set_coverage(name, gids);
            }
            match policy {
                EmptyPolicy::Fail if survivors.is_empty() => Ok((None, 0)),
                _ => Ok((Some(Value::Coverage(survivors)), 0)),
            }
        }
        Schema::ClassMatrix(EnvRef(vf1), EnvRef(vf2)) => {
            let [f1, f2] = [vf1, vf2].map(|name| env.scalar(name));
            let (Some(f1), Some(f2)) = (f1, f2) else {
                return Err("generic: unbound value format for a class matrix".into());
            };
            let cd1_rel = read_u16_be(buf, pos).ok_or("generic: truncated (ClassMatrix classDef1)")?;
            let cd2_rel = read_u16_be(buf, pos + 2).ok_or("generic: truncated (ClassMatrix classDef2)")?;
            let c1 = read_u16_be(buf, pos + 4).ok_or("generic: truncated (ClassMatrix class1Count)")? as usize;
            let c2 = read_u16_be(buf, pos + 6).ok_or("generic: truncated (ClassMatrix class2Count)")? as usize;
            let grid_at = pos + 8;

            let mut filtered = |rel: u16| -> Result<Vec<(u16, u16)>, String> {
                if rel == 0 { return Ok(Vec::new()); }
                let entries = super::super::parse_classdef(buf, offset_from(anchor, u32::from(rel))?)
                    .map_err(|e| format!("generic: {e}"))?;
                env.work.spend(entries.len())?;
                Ok(entries.into_iter()
                    .filter_map(|(g, c)| remap_gid(active, gid_map, g).map(|ng| (ng, c)))
                    .collect())
            };
            let cd1 = filtered(cd1_rel)?;
            let cd2 = filtered(cd2_rel)?;

            // A glyph in a class past the count gets nothing from the source, as the shaper refuses
            // the pair; it keeps a class past the new count rather than falling into class 0.
            let surviving = |entries: &[(u16, u16)], count: usize| -> (Vec<usize>, BTreeMap<u16, u16>) {
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
                let map = order.iter().enumerate().map(|(new, &old)| (old as u16, new as u16)).collect();
                (order, map)
            };
            let (rows, map1) = surviving(&cd1, c1);
            let (cols, map2) = surviving(&cd2, c2);

            let mut grid: Vec<Record> = Vec::new();
            if !rows.is_empty() && !cols.is_empty() {
                let stride = ((f1 & 0x00FF).count_ones() + (f2 & 0x00FF).count_ones()) as usize * 2;
                if stride == 0 {
                    return Err("generic: class matrix cell consumes no input, so its counts are unbounded".into());
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
                            .ok_or("generic: class matrix cell overflows")?;
                        env.work.spend(2 * RECORD)?;
                        let (first, read) = parse_record(buf, at, anchor, f1, env)?;
                        grid.push(first);
                        grid.push(parse_record(buf, at + read, anchor, f2, env)?.0);
                    }
                }
            }

            let renumber = |entries: Vec<(u16, u16)>, map: &BTreeMap<u16, u16>, past: usize| -> Vec<(u16, u16)> {
                entries.into_iter().map(|(g, c)| (g, map.get(&c).copied().unwrap_or(past as u16))).collect()
            };
            Ok((
                Some(Value::ClassMatrix(Box::new(Matrix {
                    class_def1: renumber(cd1, &map1, rows.len()),
                    class_def2: renumber(cd2, &map2, cols.len()),
                    class1_count: rows.len() as u16,
                    class2_count: cols.len() as u16,
                    grid,
                }))),
                8,
            ))
        }
        Schema::ClassDef(policy) => {
            let entries = super::super::parse_classdef(buf, pos).map_err(|e| format!("generic: {e}"))?;
            env.work.spend(entries.len())?;
            let survivors: Vec<(u16, u16)> = entries.into_iter()
                .filter_map(|(g, c)| remap_gid(active, gid_map, g).map(|ng| (ng, c)))
                .collect();
            match policy {
                EmptyPolicy::Fail if survivors.is_empty() => Ok((None, 0)),
                _ => Ok((Some(Value::ClassDef(survivors)), 0)),
            }
        }
        Schema::ValueFormatField(EnvRef(name)) => {
            let raw = read_u16_be(buf, pos).ok_or("generic: truncated (ValueFormatField)")?;
            env.set_scalar(name, raw);
            Ok((Some(Value::U16(env.devices.format(raw))), 2))
        }
        Schema::Anchor => {
            let (mut x, mut y) = anchor::parse_anchor(buf, pos).ok_or("generic: Anchor: truncated or unrecognized format")?;
            let point = anchor::parse_anchor_point(buf, pos);
            let mut devices: Option<Box<[Option<&[u8]>; 2]>> = None;
            if read_u16_be(buf, pos) == Some(3) {
                for (i, (slot, coord)) in [(6usize, &mut x), (8, &mut y)].into_iter().enumerate() {
                    let at = match read_u16_be(buf, pos + slot) {
                        Some(rel) if rel != 0 => Some(offset_from(pos, u32::from(rel))?),
                        _ => None,
                    };
                    let (device, delta) = env.device(buf, at)?;
                    if device.is_some() {
                        devices.get_or_insert_with(|| Box::new([None; 2]))[i] = device;
                    }
                    *coord = (i32::from(*coord) + delta).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                }
            }
            Ok((Some(Value::Anchor(x, y, point, devices)), 6))
        }
        Schema::CoveredArray(extra_fields, payload_schema, shape) => {
            let cov_rel = read_u16_be(buf, pos).ok_or("generic: truncated (CoveredArray Coverage offset)")?;
            let coverage: Rc<[u16]> = if cov_rel == 0 {
                Rc::from([])
            } else {
                env.coverage_at(buf, offset_from(anchor, u32::from(cov_rel))?, active, gid_map)?.0
            };
            let mut cursor = pos + 2;
            let mut extra_values: Vec<Value> = Vec::with_capacity(extra_fields.len());
            for field in extra_fields {
                let (v, consumed) = generic_parse(buf, cursor, anchor, &field.schema, env, active, gid_map)
                    .map_err(|e| format!("{}: {e}", field.name))?;
                cursor += consumed;
                let val = v.ok_or("generic: CoveredArray: an extra field (never GID-tagged in any real case) unexpectedly failed to survive")?;
                bind_field(env, field.bind, &val);
                extra_values.push(val);
            }
            let count = read_u16_be(buf, cursor).ok_or("generic: truncated (CoveredArray Count)")? as usize;
            cursor += 2;
            let mut entries: Vec<(u16, Rc<Value>)> = Vec::with_capacity(count.min(buf.len().saturating_sub(cursor)));
            for i in 0..count {
                let new_gid = coverage.get(i).and_then(|&g| remap_gid(active, gid_map, g));
                if new_gid.is_none() && let Some(width) = skip_offset(buf, cursor, *shape)? {
                    cursor += width;
                    continue;
                }
                let (payload_opt, consumed) =
                    parse_payload(buf, cursor, anchor, payload_schema, *shape, env, active, gid_map)?;
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
                let covered = Covered { fields: extra_values, shape: *shape, entries };
                Ok((Some(Value::CoveredArray(Box::new(covered))), total_consumed))
            }
        }
        Schema::ZippedWithBoundCoverage(EnvRef(cov_name), payload_schema, shape, drop_policy) => {
            let coverage = env.coverage(cov_name)
                .ok_or_else(|| format!("generic: unbound coverage ref '{cov_name}'"))?;
            let count = read_u16_be(buf, pos).ok_or("generic: truncated (ZippedWithBoundCoverage Count)")? as usize;
            let mut cursor = pos + 2;
            let mut payloads: Vec<Option<Rc<Value>>> = Vec::with_capacity(count.min(buf.len().saturating_sub(cursor)));
            for i in 0..count {
                let kept = coverage.get(i).is_some_and(|&g| remap_gid(active, gid_map, g).is_some());
                if !kept && let Some(width) = skip_offset(buf, cursor, *shape)? {
                    cursor += width;
                    payloads.push(None);
                    continue;
                }
                let (payload, consumed) =
                    parse_payload(buf, cursor, anchor, payload_schema, *shape, env, active, gid_map)?;
                cursor += consumed;
                payloads.push(payload);
            }
            let total_consumed = cursor - pos;
            // One entry for each glyph the rebuilt coverage keeps, in its order, so the two stay in
            // step: a glyph with no record keeps a NULL offset, since dropping it would shift the rest.
            let mut kept: Vec<(u16, usize)> = coverage.iter().enumerate()
                .filter_map(|(i, &g)| remap_gid(active, gid_map, g).map(|ng| (ng, i)))
                .collect();
            sort_by_glyph(&mut kept);
            let mut entries: Vec<Option<Rc<Value>>> = Vec::with_capacity(kept.len());
            for (_, i) in kept {
                let payload = payloads.get_mut(i).and_then(Option::take);
                if payload.is_none() && matches!(shape, PayloadShape::Inline) {
                    return Err("generic: a covered glyph has no inline record".into());
                }
                entries.push(payload);
            }
            match drop_policy {
                DropPolicy::FilterSurvivorsOrFail if entries.is_empty() => Ok((None, total_consumed)),
                DropPolicy::AllOrNothing if entries.iter().any(Option::is_none) => Ok((None, total_consumed)),
                _ => Ok((Some(Value::ZippedWithBoundCoverage(*shape, entries)), total_consumed)),
            }
        }
        Schema::CaretValue => {
            let format = read_u16_be(buf, pos).ok_or("generic: truncated (CaretValue)")?;
            let mut cv = gdef::parse_caret_value(buf, pos).ok_or("generic: CaretValue: truncated or unrecognized format")?;
            // Format 3 adjusts its coordinate by a Device or VariationIndex table, which a variable
            // font's carets need to vary.
            let at = match read_u16_be(buf, pos + 4) {
                Some(rel) if format == 3 && rel != 0 => Some(offset_from(pos, u32::from(rel))?),
                _ => None,
            };
            let (device, delta) = env.device(buf, at)?;
            if let gdef::CaretValue::Coordinate(c) = &mut cv {
                *c = (i32::from(*c) + delta).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            }
            Ok((Some(Value::CaretValue(cv, device)), if format == 3 { 6 } else { 4 }))
        }
        Schema::DeltaCoverageSubst => {
            let cov_rel = read_u16_be(buf, pos + 2).ok_or("generic: truncated (DeltaCoverageSubst Coverage offset)")?;
            let coverage: Rc<[u16]> = if cov_rel == 0 {
                Rc::from([])
            } else {
                env.coverage_at(buf, offset_from(anchor, u32::from(cov_rel))?, active, gid_map)?.0
            };
            let delta = read_i16_be(buf, pos + 4).ok_or("generic: truncated (DeltaCoverageSubst delta)")?;
            let mut entries: Vec<(u16, Rc<Value>)> = Vec::with_capacity(coverage.len());
            for &g in coverage.iter() {
                let sub = (g as i32 + delta as i32).rem_euclid(65536) as u16;
                if let (Some(new_g), Some(new_sub)) = (remap_gid(active, gid_map, g), remap_gid(active, gid_map, sub)) {
                    entries.push((new_g, Rc::new(Value::Glyph(new_sub))));
                }
            }
            sort_by_glyph(&mut entries);
            if entries.is_empty() {
                Ok((None, 6))
            } else {
                let out = Value::Struct(vec![
                    Value::U16(2),
                    Value::CoveredArray(Box::new(Covered { fields: Vec::new(), shape: PayloadShape::Inline, entries })),
                ]);
                Ok((Some(out), 6))
            }
        }
    }
}

// A ValueRecord in `format`, any VariationIndex being applied added to its value. The high byte of
// a ValueFormat is reserved: a field is held only for each low bit, as HarfBuzz and fontTools read it.
fn parse_record<'b>(
    buf: &'b [u8], pos: usize, anchor: usize, format: u16, env: &mut Env<'b, '_>,
) -> Result<(Record<'b>, usize), String> {
    let format = format & 0x00FF;
    let mut fields = [0i32; 4];
    let mut cursor = pos;
    for (bit, field) in fields.iter_mut().enumerate() {
        if format & (1 << bit) != 0 {
            *field = i32::from(read_i16_be(buf, cursor).ok_or("generic: truncated (ValueRecordField)")?);
            cursor += 2;
        }
    }
    let mut devices: Option<Box<[Option<&[u8]>; 4]>> = None;
    let mut n = 0;
    for (bit, field) in fields.iter_mut().enumerate() {
        if format & (0x10 << bit) == 0 {
            continue;
        }
        let rel = read_u16_be(buf, cursor).ok_or("generic: truncated (ValueRecord device offset)")?;
        cursor += 2;
        let at = if rel == 0 { None } else { Some(offset_from(anchor, u32::from(rel))?) };
        let (device, delta) = env.device(buf, at)?;
        *field += delta;
        if device.is_some() {
            devices.get_or_insert_with(|| Box::new([None; 4]))[n] = device;
        }
        n += 1;
    }
    let out_format = env.devices.format(format);
    let mut values = [0i16; 4];
    let present = (0..4).filter(|bit| out_format & (1 << bit) != 0);
    for (slot, bit) in values.iter_mut().zip(present) {
        *slot = fields[bit].clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    }
    Ok((Record { format: out_format, values, devices }, cursor - pos))
}

// One record of a covered array, inline or behind an offset; a NULL offset is no record.
#[allow(clippy::too_many_arguments)]
fn parse_payload<'b>(
    buf: &'b [u8], cursor: usize, anchor: usize, schema: &Schema, shape: PayloadShape, env: &mut Env<'b, '_>,
    active: &GlyphSet, gid_map: &[u16],
) -> Result<(Option<Rc<Value<'b>>>, usize), String> {
    match shape {
        PayloadShape::Inline => {
            let (v, consumed) = generic_parse(buf, cursor, anchor, schema, env, active, gid_map)?;
            Ok((v.map(Rc::new), consumed))
        }
        PayloadShape::Offsets(width) => {
            env.work.spend(1)?;
            let rel = read_offset(buf, cursor, width)?;
            if rel == 0 {
                return Ok((None, width.bytes()));
            }
            Ok((parse_child(buf, offset_from(anchor, rel)?, schema, env, active, gid_map)?, width.bytes()))
        }
    }
}

// The width of an offset to a child the subset drops, read but not followed; None for an inline
// payload, whose width is known only by parsing it.
fn skip_offset(buf: &[u8], cursor: usize, shape: PayloadShape) -> Result<Option<usize>, String> {
    match shape {
        PayloadShape::Offsets(width) => read_offset(buf, cursor, width).map(|_| Some(width.bytes())),
        PayloadShape::Inline => Ok(None),
    }
}

// Records keyed by glyph in the order the rebuilt coverage lists them: sorted, a glyph listed twice
// keeping its first record, as a coverage out of order or with repeats would pair others' data.
fn sort_by_glyph<T>(entries: &mut Vec<(u16, T)>) {
    entries.sort_by_key(|e| e.0);
    entries.dedup_by_key(|e| e.0);
}

fn bind_field(env: &mut Env, bind: Option<&'static str>, val: &Value) {
    let Some(bind_name) = bind else { return };
    if let Value::U16(raw) = val {
        env.set_scalar(bind_name, *raw);
    }
}

// None for a count read as a field minus one that is 0.
fn resolve_count(source: &CountSource, env: &Env) -> Result<Option<usize>, String> {
    let (CountSource::Field(EnvRef(name)) | CountSource::FieldMinusOne(EnvRef(name))) = source;
    let v = env.scalar(name).ok_or_else(|| format!("generic: unbound env ref '{name}' for count"))? as usize;
    Ok(match source {
        CountSource::Field(_) => Some(v),
        CountSource::FieldMinusOne(_) => v.checked_sub(1),
    })
}

fn write_back_count(source: &CountSource, env: &mut Env, actual_len: usize) {
    match source {
        CountSource::Field(EnvRef(name)) => { env.set_scalar(name, actual_len as u16); }
        CountSource::FieldMinusOne(EnvRef(name)) => { env.set_scalar(name, (actual_len + 1) as u16); }
    }
}
