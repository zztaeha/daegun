use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use crate::daecore::daetype::decoder::{write_u16_be, write_i16_be, write_u32_be};
use super::super::anchor::build_anchor_with_devices;
use super::super::gdef::{build_caret_value, CaretValue};
use super::super::{build_coverage, build_classdef};
use super::schema::{OffsetWidth, PayloadShape};
use super::parse::Work;
use super::value::{Record, Value};

// Inline fields, then children as named (coverage first, identical ones once), then the subtable's
// own Device tables, or those first on overflow. An offset that still won't fit gives None, not NULL.
pub(crate) fn generic_build<'v>(value: &'v Value<'v>, work: &mut Work) -> Option<Vec<u8>> {
    let mut cx = Cx { coverages: &mut work.built_coverages, built: BTreeMap::new() };
    [false, true]
        .into_iter()
        .find_map(|devices_first| build(value, Some(devices_first), &mut cx))
        .map(|(bytes, _)| bytes)
}

// What has been built: a coverage's bytes by the glyph list it is built from, kept for the whole
// table, and a child table's by the shared value it is, so a part many name is built once.
struct Cx<'v, 'w> {
    coverages: &'w mut BTreeMap<*const u16, Rc<[u8]>>,
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
        let mut hash = 0u64;
        let mut mix = |part: &[u8]| {
            let (words, rest) = part.as_chunks::<8>();
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            for &word in words.iter().chain([&last]) {
                hash = (hash.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(0x517c_c1b7_2722_0a95);
            }
        };
        mix(&bytes);
        for &(slot, device) in &devices {
            mix(&slot.to_le_bytes());
            mix(device);
        }
        hash = (hash ^ (hash >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
        hash = (hash ^ (hash >> 33)).wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        Built { hash: hash ^ (hash >> 33), bytes, devices }
    }
}

// Placed tables found by fingerprint: open addressing over indexes into `tables`, at most half full.
// A probe stops after 64 slots, so tables crafted to collide cost a missed share, not quadratic time.
#[derive(Default)]
struct Placed<'v> {
    slots: Vec<u32>,
    tables: Vec<(u64, Rc<Built<'v>>, usize)>,
}

const PROBES: usize = 64;

impl<'v> Placed<'v> {
    fn get(&self, built: &Built<'v>) -> Option<usize> {
        let mask = self.slots.len().checked_sub(1)?;
        (0..PROBES)
            .map_while(|i| self.tables.get(self.slots[(built.hash as usize).wrapping_add(i) & mask] as usize))
            .find(|(hash, placed, _)| *hash == built.hash && **placed == *built)
            .map(|&(_, _, at)| at)
    }

    fn insert(&mut self, built: Rc<Built<'v>>, at: usize) {
        if (self.tables.len() + 1) * 2 > self.slots.len() {
            self.slots = alloc::vec![u32::MAX; (self.slots.len() * 2).max(16)];
            for (n, &(hash, ..)) in self.tables.iter().enumerate() {
                Self::put(&mut self.slots, hash, n);
            }
        }
        Self::put(&mut self.slots, built.hash, self.tables.len());
        self.tables.push((built.hash, built, at));
    }

    fn put(slots: &mut [u32], hash: u64, n: usize) {
        let mask = slots.len() - 1;
        if let Some(i) = (0..PROBES).map(|i| (hash as usize).wrapping_add(i) & mask).find(|&i| slots[i] == u32::MAX) {
            slots[i] = n as u32;
        }
    }
}

fn coverage_bytes(gids: &Rc<[u16]>, cx: &mut Cx<'_, '_>) -> Rc<[u8]> {
    cx.coverages.entry(gids.as_ptr()).or_insert_with(|| build_coverage(gids).into()).clone()
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
    if let Some(bytes) = leaf(value, cx) {
        return Some((bytes, Vec::new()));
    }
    let mut t = Table {
        header: alloc::vec![0u8; inline_width(value, cx)],
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
fn leaf(value: &Value<'_>, cx: &mut Cx<'_, '_>) -> Option<Vec<u8>> {
    Some(match value {
        Value::Coverage(gids) => coverage_bytes(gids, cx).to_vec(),
        Value::ClassDef(entries) => build_classdef(entries),
        Value::Anchor(x, y, point, devices) => {
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
        Value::ZippedWithBoundCoverage(PayloadShape::Inline, entries) => {
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
    fn child(&mut self, value: &'v Rc<Value<'v>>) -> usize {
        // Only a child more than one table names is kept once built.
        let built = if Rc::strong_count(value) > 1 {
            let key = Rc::as_ptr(value);
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
            Some(in_tail) => write_u16_be(&mut self.tail, in_tail, v),
            None => write_u16_be(&mut self.header, slot, v),
        }
    }

    fn blob(&mut self, bytes: Vec<u8>) -> usize {
        self.place(Rc::new(Built::new(bytes, Vec::new())))
    }

    fn offset(&mut self, pos: usize, width: OffsetWidth, target: usize) {
        match width {
            OffsetWidth::W16 => match u16::try_from(target) {
                Ok(v) => write_u16_be(&mut self.header, pos, v),
                Err(_) => self.overflow = true,
            },
            OffsetWidth::W32 => match u32::try_from(target) {
                Ok(v) => write_u32_be(&mut self.header, pos, v),
                Err(_) => self.overflow = true,
            },
        }
    }

    fn write(&mut self, value: &'v Value<'v>, pos: usize) -> usize {
        match value {
            Value::U16(v) | Value::Glyph(v) => { write_u16_be(&mut self.header, pos, *v); 2 }
            Value::Offset(width, child) => {
                if let Some(v) = child {
                    let target = self.child(v);
                    self.offset(pos, *width, target);
                }
                width.bytes()
            }
            Value::Array(elems) => {
                let mut p = pos;
                for e in elems {
                    p += self.write(e, p);
                }
                p - pos
            }
            Value::OffsetArray(width, slots) => {
                let mut p = pos;
                for slot in slots {
                    if let Some(v) = slot {
                        let target = self.child(v);
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
            Value::Coverage(_) | Value::ClassDef(_) | Value::Anchor(..) | Value::CaretValue(..) => {
                let bytes = leaf(value, self.cx).unwrap_or_default();
                self.inline(pos, &bytes)
            }
            Value::ClassMatrix(m) => {
                for (slot, entries) in [(pos, &m.class_def1), (pos + 2, &m.class_def2)] {
                    let target = self.blob(build_classdef(entries));
                    self.offset(slot, OffsetWidth::W16, target);
                }
                write_u16_be(&mut self.header, pos + 4, m.class1_count);
                write_u16_be(&mut self.header, pos + 6, m.class2_count);
                let mut p = pos + 8;
                for r in &m.grid {
                    p += self.record(r, p);
                }
                p - pos
            }
            Value::CoveredArray(c) => {
                let gids: Vec<u16> = c.entries.iter().map(|(g, _)| *g).collect();
                let target = self.blob(build_coverage(&gids));
                self.offset(pos, OffsetWidth::W16, target);
                let mut p = pos + 2;
                for v in &c.fields {
                    p += self.write(v, p);
                }
                write_u16_be(&mut self.header, p, c.entries.len() as u16);
                p += 2;
                for (_, v) in &c.entries {
                    p += self.payload(c.shape, Some(v), p);
                }
                p - pos
            }
            Value::ZippedWithBoundCoverage(shape, entries) => {
                write_u16_be(&mut self.header, pos, entries.len() as u16);
                let mut p = pos + 2;
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
                    Some(Ok(at)) => write_u16_be(&mut self.header, p, at),
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
    fn payload(&mut self, shape: PayloadShape, value: Option<&'v Rc<Value<'v>>>, pos: usize) -> usize {
        match (shape, value) {
            (PayloadShape::Inline, Some(v)) => self.write(v, pos),
            (PayloadShape::Inline, None) => 0,
            (PayloadShape::Offsets(width), v) => {
                if let Some(v) = v {
                    let target = self.child(v);
                    self.offset(pos, width, target);
                }
                width.bytes()
            }
        }
    }
}

fn inline_width(value: &Value, cx: &mut Cx<'_, '_>) -> usize {
    match value {
        Value::U16(_) | Value::Glyph(_) => 2,
        Value::Offset(width, _) => width.bytes(),
        Value::Array(elems) => elems.iter().map(|e| inline_width(e, cx)).sum(),
        Value::OffsetArray(width, slots) => slots.len() * width.bytes(),
        Value::Struct(fields) => fields.iter().map(|v| inline_width(v, cx)).sum(),
        Value::ValueRecord(r) => r.width(),
        Value::Coverage(_) | Value::ClassDef(_) | Value::Anchor(..) | Value::CaretValue(..) => {
            leaf(value, cx).map_or(0, |b| b.len())
        }
        Value::CoveredArray(c) => {
            let fields: usize = c.fields.iter().map(|v| inline_width(v, cx)).sum();
            4 + fields + payload_width(c.shape, c.entries.iter().map(|(_, v)| Some(&**v)), cx)
        }
        Value::ZippedWithBoundCoverage(shape, entries) => {
            2 + payload_width(*shape, entries.iter().map(|e| e.as_deref()), cx)
        }
        Value::ClassMatrix(m) => 8 + m.grid.iter().map(Record::width).sum::<usize>(),
    }
}

fn payload_width<'a>(
    shape: PayloadShape, entries: impl Iterator<Item = Option<&'a Value<'a>>>, cx: &mut Cx<'_, '_>,
) -> usize {
    match shape {
        PayloadShape::Inline => entries.flatten().map(|e| inline_width(e, cx)).sum(),
        PayloadShape::Offsets(w) => entries.count() * w.bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tables crafted to one fingerprint are told apart by their bytes, past the probe limit too.
    #[test]
    fn tables_of_one_fingerprint_are_never_confused() {
        let table = |n: u16| Built { hash: 0, bytes: n.to_be_bytes().to_vec(), devices: Vec::new() };
        let mut placed = Placed::default();
        for n in 0..100 {
            if placed.get(&table(n)).is_none() {
                placed.insert(Rc::new(table(n)), usize::from(n));
            }
        }
        for n in 0..100 {
            let got = placed.get(&table(n));
            assert!(got.is_none() || got == Some(usize::from(n)), "table {n} found at {got:?}");
        }
        assert_eq!(placed.get(&table(0)), Some(0));
    }
}
