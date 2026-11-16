use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use crate::daecore::daetype::decoder::{write_u16_be, write_i16_be, Fingerprint, FingerprintIndex};
use super::super::anchor::build_anchor_with_devices;
use super::super::gdef::{build_caret_value, CaretValue};
use super::super::{build_coverage, build_classdef};
use super::schema::{OffsetWidth, PayloadShape, Width};
use super::parse::Work;
use super::value::{Record, Value};

// Inline fields, then children as named (coverage first, identical ones once), then the subtable's
// own Device tables, or those first on overflow. An offset that still won't fit gives None, not NULL.
pub(crate) fn generic_build<'v>(value: &'v Value<'v>, work: &mut Work) -> Option<Vec<u8>> {
    let mut cx = Cx { coverages: &mut work.built_coverages, lookups: work.lookups.as_deref(), built: BTreeMap::new() };
    [false, true]
        .into_iter()
        .find_map(|devices_first| build(value, Some(devices_first), &mut cx))
        .map(|(bytes, _)| bytes)
}

// Each shared coverage's bytes by the glyph list it is built from, kept for the whole table.
pub(super) type Coverages = BTreeMap<*const u32, Rc<[u8]>>;

// What has been built: the table's coverages, and a child table's bytes by the shared value it is, so a part many
// name is built once.
struct Cx<'v, 'w> {
    coverages: &'w mut Coverages,
    lookups: Option<&'w [u16]>,
    built: BTreeMap<*const Value<'v>, Option<Rc<Built<'v>>>>,
}

// A table's bytes and the Device tables it hands up, with a fingerprint of both.
#[derive(PartialEq, Eq)]
struct Built<'v> {
    hash: u64,
    bytes: Vec<u8>,
    devices: Vec<DeviceSlot<'v>>,
}

impl<'v> Built<'v> {
    fn new(bytes: Vec<u8>, devices: Vec<DeviceSlot<'v>>) -> Self {
        let mut print = Fingerprint::default();
        print.mix(&bytes);
        for &(slot, device) in &devices {
            print.mix(&slot.to_le_bytes());
            print.mix(device);
        }
        Built { hash: print.finish(), bytes, devices }
    }
}

// Placed tables found by fingerprint, each with where it was placed.
#[derive(Default)]
struct Placed<'v> {
    index: FingerprintIndex,
    tables: Vec<(Rc<Built<'v>>, usize)>,
}

impl<'v> Placed<'v> {
    fn get(&self, built: &Built<'v>) -> Option<usize> {
        self.index.find(built.hash, |n| *self.tables[n].0 == *built).map(|n| self.tables[n].1)
    }

    fn insert(&mut self, built: Rc<Built<'v>>, at: usize) {
        self.index.push(built.hash);
        self.tables.push((built, at));
    }
}

fn coverage_bytes(gids: &Rc<[u32]>, coverages: &mut Coverages) -> Rc<[u8]> {
    coverages.entry(gids.as_ptr()).or_insert_with(|| build_coverage(gids).into()).clone()
}

// A Device table's slot and bytes, the offset counted from the table that holds the record.
type DeviceSlot<'v> = (usize, &'v [u8]);

// A child's Device tables are placed by its parent after the children, written out before the next
// child would push them past 16-bit reach of the first naming them; later children get a copy.
struct Table<'v, 'c, 'w> {
    cx: &'c mut Cx<'v, 'w>,
    header: Vec<u8>,
    tail: Vec<u8>,
    own: Vec<DeviceSlot<'v>>,
    own_placed: BTreeMap<&'v [u8], usize>,
    pending: BTreeMap<&'v [u8], Vec<(usize, usize)>>,
    pending_len: usize,
    first_base: usize,
    shared: Placed<'v>,
    overflow: bool,
}

// The table's bytes, and at the top (`top` saying whether its own Device tables go first) every
// Device placed; below it, its own records' Device tables handed up for the parent to place.
fn build<'v>(value: &'v Value<'v>, top: Option<bool>, cx: &mut Cx<'v, '_>) -> Option<(Vec<u8>, Vec<DeviceSlot<'v>>)> {
    if let Some(bytes) = leaf(value, cx.coverages) {
        return Some((bytes, Vec::new()));
    }
    let mut t = Table {
        header: alloc::vec![0u8; inline_width(value, cx.coverages)],
        cx,
        tail: Vec::new(),
        own: Vec::new(),
        own_placed: BTreeMap::new(),
        pending: BTreeMap::new(),
        pending_len: 0,
        first_base: usize::MAX,
        shared: Placed::default(),
        overflow: false,
    };
    if top == Some(true) {
        let mut devices = Vec::new();
        inline_devices(value, &mut devices);
        for bytes in devices {
            if !t.own_placed.contains_key(bytes) {
                t.own_placed.insert(bytes, t.end());
                t.tail.extend_from_slice(bytes);
            }
        }
    }
    t.write(value, 0);
    let own = core::mem::take(&mut t.own);
    if top.is_some() {
        for &(slot, bytes) in &own {
            t.wait(bytes, slot, 0);
        }
    }
    t.flush();
    if t.overflow {
        return None;
    }
    let mut bytes = t.header;
    bytes.extend_from_slice(&t.tail);
    Some((bytes, if top.is_some() { Vec::new() } else { own }))
}

// A table of only its own bytes, built directly: an anchor, coverage, class definition or caret.
pub(super) fn leaf(value: &Value<'_>, coverages: &mut Coverages) -> Option<Vec<u8>> {
    Some(match value {
        Value::Coverage(gids) => coverage_bytes(gids, coverages).to_vec(),
        Value::ClassDef(entries) => build_classdef(entries),
        Value::Anchor(_, x, y, point, devices) => {
            let [dx, dy] = devices.as_deref().copied().unwrap_or_default();
            build_anchor_with_devices(*x, *y, *point, dx, dy)
        }
        Value::CaretValue(CaretValue::Coordinate(c), Some(device)) => {
            [&[3u16, *c as u16, 6].map(u16::to_be_bytes).concat()[..], device].concat()
        }
        Value::CaretValue(cv, _) => build_caret_value(cv),
        _ => return None,
    })
}

// The Device tables a table's own records name, those in its inline fields and not its children.
fn inline_devices<'v>(value: &'v Value<'v>, out: &mut Vec<&'v [u8]>) {
    match value {
        Value::ValueRecord(r) => record_devices(r, out),
        Value::Array(elems) | Value::Struct(elems) => elems.iter().for_each(|e| inline_devices(e, out)),
        Value::ClassMatrix(m) => m.grid.iter().for_each(|r| record_devices(r, out)),
        Value::CoveredArray(c) => {
            c.fields.iter().for_each(|v| inline_devices(v, out));
            if let PayloadShape::Inline = c.shape {
                c.entries.iter().for_each(|(_, v)| inline_devices(v, out));
            }
        }
        Value::ZippedWithBoundCoverage(PayloadShape::Inline, _, entries) => {
            entries.iter().flatten().for_each(|v| inline_devices(v, out));
        }
        _ => {}
    }
}

fn record_devices<'v>(r: &'v Record<'v>, out: &mut Vec<&'v [u8]>) {
    out.extend(r.devices.iter().flat_map(|d| d.iter().flatten()));
}

impl<'v> Table<'v, '_, '_> {
    fn end(&self) -> usize {
        self.header.len() + self.tail.len()
    }

    // Where a child table goes; one already placed with the same bytes and Device tables is reused.
    fn child(&mut self, value: &'v Value<'v>, shared: bool) -> usize {
        // Only a child more than one table names is kept once built.
        let built = if shared {
            let key: *const Value<'v> = value;
            match self.cx.built.get(&key) {
                Some(done) => done.clone(),
                None => {
                    let done = self.build(value);
                    self.cx.built.insert(key, done.clone());
                    done
                }
            }
        } else {
            self.build(value)
        };
        let Some(built) = built else {
            self.overflow = true;
            return 0;
        };
        self.place(built)
    }

    fn build(&mut self, value: &'v Value<'v>) -> Option<Rc<Built<'v>>> {
        build(value, None, self.cx).map(|(bytes, devices)| Rc::new(Built::new(bytes, devices)))
    }

    fn place(&mut self, built: Rc<Built<'v>>) -> usize {
        if let Some(at) = self.shared.get(&built) {
            return at;
        }
        let arriving: usize =
            built.devices.iter().filter(|(_, d)| !self.pending.contains_key(d)).map(|(_, d)| d.len()).sum();
        let reach = self.end() + built.bytes.len() + self.pending_len + arriving;
        if !self.pending.is_empty() && reach - self.first_base > 0xFFFF {
            self.flush();
        }
        let at = self.end();
        self.tail.extend_from_slice(&built.bytes);
        for &(slot, device) in &built.devices {
            self.wait(device, at + slot, at);
        }
        self.shared.insert(built, at);
        at
    }

    fn wait(&mut self, device: &'v [u8], slot: usize, base: usize) {
        let referrers = self.pending.entry(device).or_default();
        if referrers.is_empty() {
            self.pending_len += device.len();
        }
        referrers.push((slot, base));
        self.first_base = self.first_base.min(base);
    }

    // Writes out every waiting Device table, once, and points its referrers at it.
    fn flush(&mut self) {
        for (device, referrers) in core::mem::take(&mut self.pending) {
            let at = self.end();
            self.tail.extend_from_slice(device);
            for (slot, base) in referrers {
                match u16::try_from(at - base) {
                    Ok(v) => self.patch(slot, v),
                    Err(_) => self.overflow = true,
                }
            }
        }
        self.pending_len = 0;
        self.first_base = usize::MAX;
    }

    fn patch(&mut self, slot: usize, v: u16) {
        match slot.checked_sub(self.header.len()) {
            Some(in_tail) => { write_u16_be(&mut self.tail, in_tail, v); }
            None => { write_u16_be(&mut self.header, slot, v); }
        }
    }

    fn blob(&mut self, bytes: Vec<u8>) -> usize {
        self.place(Rc::new(Built::new(bytes, Vec::new())))
    }

    fn offset(&mut self, pos: usize, width: OffsetWidth, target: usize) {
        match u32::try_from(target) {
            Ok(v) => {
                self.uint(pos, width.bytes(), v);
            }
            Err(_) => self.overflow = true,
        }
    }

    // `v` in `w` bytes at `pos`; one too wide for them fails the table.
    fn uint(&mut self, pos: usize, w: usize, v: u32) -> usize {
        if w < 4 && v >> (8 * w) != 0 {
            self.overflow = true;
        }
        self.header[pos..pos + w].copy_from_slice(&v.to_be_bytes()[4 - w..]);
        w
    }

    fn write(&mut self, value: &'v Value<'v>, pos: usize) -> usize {
        match value {
            Value::U16(v) | Value::ValueFormat(_, v) => { write_u16_be(&mut self.header, pos, *v); 2 }
            Value::LookupIndex(v) => { write_u16_be(&mut self.header, pos, numbered(self.cx.lookups, *v)); 2 }
            Value::U24(v) => self.uint(pos, 3, *v),
            Value::Glyph(w, g) => self.uint(pos, w.bytes(), *g),
            Value::Offset(width, child) => {
                if let Some(v) = child {
                    let target = self.child(v, Rc::strong_count(v) > 1);
                    self.offset(pos, *width, target);
                }
                width.bytes()
            }
            Value::Anchor(width, ..) => {
                let target = self.child(value, false);
                self.offset(pos, *width, target);
                width.bytes()
            }
            Value::Array(elems) => {
                let mut p = pos;
                for e in elems {
                    p += self.write(e, p);
                }
                p - pos
            }
            Value::Glyphs(w, glyphs) => {
                glyphs.iter().enumerate().map(|(i, &g)| self.uint(pos + w.bytes() * i, w.bytes(), g)).sum()
            }
            Value::GlyphMap(pairs) => {
                let w = substitute_width(pairs);
                let (l, size, delta) = (w.long().bytes(), w.bytes(), single_delta(pairs, w));
                write_u16_be(&mut self.header, pos, single_format(w, delta));
                let gids: Vec<u32> = pairs.iter().map(|&(g, _)| g).collect();
                let target = self.blob(build_coverage(&gids));
                self.offset(pos + 2, w.long(), target);
                if let Some(delta) = delta {
                    return 2 + l + self.uint(pos + 2 + l, size, delta);
                }
                self.uint(pos + 2 + l, size, pairs.len() as u32);
                for (i, &(_, sub)) in pairs.iter().enumerate() {
                    self.uint(pos + 2 + l + size * (1 + i), size, sub);
                }
                2 + l + size * (1 + pairs.len())
            }
            Value::OffsetArray(width, slots) => {
                let mut p = pos;
                for slot in slots {
                    if let Some(v) = slot {
                        let target = self.child(v, Rc::strong_count(v) > 1);
                        self.offset(p, *width, target);
                    }
                    p += width.bytes();
                }
                p - pos
            }
            Value::Struct(fields) => {
                let mut p = pos;
                for v in fields {
                    p += self.write(v, p);
                }
                p - pos
            }
            Value::ValueRecord(r) => self.record(r, pos),
            Value::Coverage(_) | Value::ClassDef(_) | Value::CaretValue(..) => {
                let bytes = leaf(value, self.cx.coverages).unwrap_or_default();
                self.inline(pos, &bytes)
            }
            Value::ClassMatrix(m) => {
                let l = m.width.long().bytes();
                for (slot, entries) in [(pos, &m.class_def1), (pos + l, &m.class_def2)] {
                    let target = self.blob(build_classdef(entries));
                    self.offset(slot, m.width.long(), target);
                }
                write_u16_be(&mut self.header, pos + 2 * l, m.class1_count);
                write_u16_be(&mut self.header, pos + 2 * l + 2, m.class2_count);
                let mut p = pos + 2 * l + 4;
                for r in &m.grid {
                    p += self.record(r, p);
                }
                p - pos
            }
            Value::CoveredArray(c) => {
                let gids: Vec<u32> = c.entries.iter().map(|(g, _)| *g).collect();
                let target = self.blob(build_coverage(&gids));
                self.offset(pos, c.width.long(), target);
                let mut p = pos + c.width.long().bytes();
                for v in &c.fields {
                    p += self.write(v, p);
                }
                p += self.uint(p, c.width.bytes(), c.entries.len() as u32);
                for (_, v) in &c.entries {
                    p += self.payload(c.shape, Some(v), p);
                }
                p - pos
            }
            Value::ZippedWithBoundCoverage(shape, w, entries) => {
                let mut p = pos + self.uint(pos, w.bytes(), entries.len() as u32);
                for v in entries {
                    p += self.payload(*shape, v.as_ref(), p);
                }
                p - pos
            }
        }
    }

    // A ValueRecord's values, then its Device offsets: one placed already is pointed at, the rest wait
    // for the parent.
    fn record(&mut self, r: &'v Record<'v>, pos: usize) -> usize {
        let mut p = pos;
        for v in r.values.iter().take((r.format & 0x000F).count_ones() as usize) {
            write_i16_be(&mut self.header, p, *v);
            p += 2;
        }
        for i in 0..(r.format & 0x00F0).count_ones() as usize {
            if let Some(bytes) = r.devices.as_ref().and_then(|d| d[i]) {
                match self.own_placed.get(bytes).map(|&at| u16::try_from(at)) {
                    Some(Ok(at)) => { write_u16_be(&mut self.header, p, at); }
                    Some(Err(_)) => self.overflow = true,
                    None => self.own.push((p, bytes)),
                }
            }
            p += 2;
        }
        p - pos
    }

    fn inline(&mut self, pos: usize, bytes: &[u8]) -> usize {
        self.header[pos..pos + bytes.len()].copy_from_slice(bytes);
        bytes.len()
    }

    // A record of a covered array, inline or behind an offset (NULL for none).
    fn payload(&mut self, shape: PayloadShape, value: Option<&'v Value<'v>>, pos: usize) -> usize {
        match (shape, value) {
            (_, Some(v)) => self.write(v, pos),
            (PayloadShape::Inline, None) => 0,
            (PayloadShape::Offsets(width), None) => width.bytes(),
        }
    }
}

// A nested lookup's number in the subset, where the subset numbers them, past the lookups for one it left out.
pub(super) fn numbered(numbers: Option<&[u16]>, lookup: u16) -> u16 {
    numbers.map_or(lookup, |numbers| numbers.get(usize::from(lookup)).copied().unwrap_or(u16::MAX))
}

// A SingleSubst's width: 24 bits where a substitute needs them, written as format 3 or 4.
pub(super) fn substitute_width(pairs: &[(u32, u32)]) -> Width {
    if pairs.iter().any(|&(_, sub)| sub > 0xFFFF) { Width::W24 } else { Width::W16 }
}

// The one delta each substitute is from its glyph, modulo the width, where there is one, so the delta format holds it.
pub(super) fn single_delta(pairs: &[(u32, u32)], w: Width) -> Option<u32> {
    let mask = if w == Width::W16 { 0xFFFF } else { 0xFF_FFFF };
    let delta = |&(g, sub): &(u32, u32)| sub.wrapping_sub(g) & mask;
    let first = delta(pairs.first()?);
    pairs.iter().all(|pair| delta(pair) == first).then_some(first)
}

// Format 1 for a delta, 2 for a list of substitutes; 3 and 4 at 24 bits.
pub(super) fn single_format(w: Width, delta: Option<u32>) -> u16 {
    (if delta.is_some() { 1 } else { 2 }) + if w == Width::W16 { 0 } else { 2 }
}

pub(super) fn inline_width(value: &Value, cx: &mut Coverages) -> usize {
    match value {
        Value::U16(_) | Value::ValueFormat(..) | Value::LookupIndex(_) => 2,
        Value::U24(_) => 3,
        Value::Glyph(w, _) => w.bytes(),
        Value::Offset(width, _) | Value::Anchor(width, ..) => width.bytes(),
        Value::Array(elems) => elems.iter().map(|e| inline_width(e, cx)).sum(),
        Value::Glyphs(w, glyphs) => w.bytes() * glyphs.len(),
        Value::GlyphMap(pairs) => {
            let w = substitute_width(pairs);
            let n = if single_delta(pairs, w).is_some() { 0 } else { pairs.len() };
            2 + w.long().bytes() + w.bytes() * (1 + n)
        }
        Value::OffsetArray(width, slots) => slots.len() * width.bytes(),
        Value::Struct(fields) => fields.iter().map(|v| inline_width(v, cx)).sum(),
        Value::ValueRecord(r) => r.width(),
        Value::Coverage(_) | Value::ClassDef(_) | Value::CaretValue(..) => {
            leaf(value, cx).map_or(0, |b| b.len())
        }
        Value::CoveredArray(c) => {
            let fields: usize = c.fields.iter().map(|v| inline_width(v, cx)).sum();
            let head = c.width.long().bytes() + c.width.bytes();
            head + fields + payload_width(c.shape, c.entries.iter().map(|(_, v)| Some(v)), cx)
        }
        Value::ZippedWithBoundCoverage(shape, w, entries) => {
            w.bytes() + payload_width(*shape, entries.iter().map(Option::as_ref), cx)
        }
        Value::ClassMatrix(m) => 2 * m.width.long().bytes() + 4 + m.grid.iter().map(Record::width).sum::<usize>(),
    }
}

fn payload_width<'a>(
    shape: PayloadShape, entries: impl Iterator<Item = Option<&'a Value<'a>>>, cx: &mut Coverages,
) -> usize {
    match shape {
        PayloadShape::Inline => entries.flatten().map(|e| inline_width(e, cx)).sum(),
        PayloadShape::Offsets(w) => entries.count() * w.bytes(),
    }
}

#[cfg(test)]
#[path = "../../../../../tests/unit/daetype/subsetter/otl/generic/build.rs"]
mod tests;
