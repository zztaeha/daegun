use alloc::collections::btree_map::Entry;
use alloc::collections::{BTreeMap, BinaryHeap};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cmp::Reverse;
use crate::daecore::daetype::decoder::{write_i16_be, write_u16_be, Fingerprint, FingerprintIndex};
use super::super::{build_classdef, build_coverage};
use super::build::{inline_width, leaf, numbered, single_delta, single_format, substitute_width, Coverages};
use super::schema::PayloadShape;
use super::value::{Record, Value};

// A layout table's objects, each its own bytes and the offsets it holds to others. One with the bytes and offsets of
// one held already is that one, so a part many subtables or lookups name is written once across the table.
#[derive(Default)]
pub(crate) struct Graph {
    objects: Vec<Object>,
    index: FingerprintIndex,
    // The bytes of every object held, each once, and of each copy a pack made, which stays.
    held: usize,
    // What the objects held and the copies every pack makes of them may come to, where bounded.
    budget: Option<usize>,
}

#[derive(PartialEq, Eq)]
struct Object {
    bytes: Vec<u8>,
    links: Vec<Link>,
}

// An offset `width` bytes wide at `at` in its object, counted from the object's start, to object `to`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) at: usize,
    pub(crate) width: usize,
    pub(crate) to: usize,
}

impl Graph {
    pub(crate) fn within(budget: usize) -> Graph {
        Graph { budget: Some(budget), ..Graph::default() }
    }

    // The object with these bytes and offsets, added if none is held. Each names only objects added before it.
    pub(crate) fn add(&mut self, bytes: Vec<u8>, links: Vec<Link>) -> usize {
        let mut print = Fingerprint::default();
        print.mix(&bytes);
        for v in links.iter().flat_map(|link| [link.at, link.width, link.to]) {
            print.mix(&(v as u64).to_le_bytes());
        }
        let hash = print.finish();
        let object = Object { bytes, links };
        if let Some(n) = self.index.find(hash, |n| self.objects[n] == object) {
            return n;
        }
        self.held += object.bytes.len();
        self.index.push(hash);
        self.objects.push(object);
        self.objects.len() - 1
    }

    pub(crate) fn held(&self) -> usize {
        self.held
    }

    // What `root` reaches, parents first once all naming them are placed, 32-bit-only objects last, so 16-bit offsets
    // point near. A shared object some can't reach is copied for them, once or once each; None where neither fits, or
    // where the copies, with any an earlier pack left, come to more than the budget leaves.
    pub(crate) fn pack(&mut self, root: usize) -> Option<Vec<u8>> {
        for _ in 0..PACK_ROUNDS {
            let (short, naming) = match self.lay_out(root) {
                Ok(out) => return Some(out),
                Err(short) => short?,
            };
            let mut by_target: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
            for (from, k) in short {
                by_target.entry(self.objects[from].links[k].to).or_default().push((from, k));
            }
            let mut copies = BTreeMap::new();
            for (to, links) in by_target {
                let every = links.len() == naming[to];
                for (from, k) in links {
                    let copy = match copies.entry((if every { from } else { usize::MAX }, to)) {
                        Entry::Occupied(copy) => *copy.get(),
                        Entry::Vacant(slot) => {
                            let held = self.held.checked_add(self.objects[to].bytes.len());
                            self.held = held.filter(|&held| self.budget.is_none_or(|budget| held <= budget))?;
                            let (bytes, links) = (self.objects[to].bytes.clone(), self.objects[to].links.clone());
                            self.objects.push(Object { bytes, links });
                            *slot.insert(self.objects.len() - 1)
                        }
                    };
                    self.objects[from].links[k].to = copy;
                }
            }
        }
        None
    }

    // The layout, or each offset that can't reach a shared object, by its table and link, with how many offsets name
    // each object; none when one can't reach an object only it names.
    #[allow(clippy::type_complexity)]
    fn lay_out(&self, root: usize) -> Result<Vec<u8>, Option<(Vec<(usize, usize)>, Vec<usize>)>> {
        let n = self.objects.len();
        let (mut met, mut naming) = (alloc::vec![usize::MAX; n], alloc::vec![0usize; n]);
        let mut near = alloc::vec![false; n];
        // Each object's children met smallest first, so a table's small Device tables come before its large class
        // definitions and stay in reach of it.
        let links = self.objects.iter().map(|object| object.links.len()).sum();
        let (mut order, mut starts) = (Vec::with_capacity(links), Vec::with_capacity(n + 1));
        for object in &self.objects {
            let start = order.len();
            starts.push(start);
            order.extend(0..object.links.len());
            order[start..].sort_by_key(|&k| self.objects[object.links[k].to].bytes.len());
        }
        starts.push(order.len());
        let mut walk = alloc::vec![(root, 0)];
        met[root] = 0;
        let (mut seen, mut size) = (1, self.objects[root].bytes.len());
        while let Some((at, next)) = walk.pop() {
            let Some(&k) = order[starts[at]..starts[at + 1]].get(next) else { continue };
            let link = &self.objects[at].links[k];
            walk.push((at, next + 1));
            naming[link.to] += 1;
            near[link.to] |= link.width < 4;
            if met[link.to] == usize::MAX {
                met[link.to] = seen;
                seen += 1;
                size += self.objects[link.to].bytes.len();
                walk.push((link.to, 0));
            }
        }
        let counted = naming.clone();
        let (mut placed, mut out) = (alloc::vec![usize::MAX; n], Vec::with_capacity(size));
        let mut ready = BinaryHeap::from([Reverse((false, 0, root))]);
        while let Some(Reverse((_, _, at))) = ready.pop() {
            placed[at] = out.len();
            out.extend_from_slice(&self.objects[at].bytes);
            for link in &self.objects[at].links {
                naming[link.to] -= 1;
                if naming[link.to] == 0 {
                    ready.push(Reverse((!near[link.to], met[link.to], link.to)));
                }
            }
        }
        let mut short = Vec::new();
        for (from, object) in self.objects.iter().enumerate().filter(|&(at, _)| placed[at] != usize::MAX) {
            for (k, link) in object.links.iter().enumerate() {
                let distance = placed[link.to] - placed[from];
                if link.width < 4 && distance >> (8 * link.width) != 0 || distance > u32::MAX as usize {
                    if counted[link.to] < 2 {
                        return Err(None);
                    }
                    short.push((from, k));
                    continue;
                }
                let at = placed[from] + link.at;
                out[at..at + link.width].copy_from_slice(&(distance as u32).to_be_bytes()[4 - link.width..]);
            }
        }
        if short.is_empty() { Ok(out) } else { Err(Some((short, counted))) }
    }
}

// Rounds of copying shared objects out of reach before a graph gives way to the writer that copies per subtable.
const PACK_ROUNDS: usize = 16;

// A subtable's value as objects of `graph`: each table its inline fields, each child, coverage, class definition and
// Device table an object of its own. None where a value won't fit its field.
pub(crate) fn generic_emit(
    value: &Value<'_>, graph: &mut Graph, coverages: &mut Coverages, lookups: Option<&[u16]>,
) -> Option<usize> {
    Emit { graph, coverages, lookups, built: BTreeMap::new() }.table(value)
}

struct Emit<'v, 'g> {
    graph: &'g mut Graph,
    coverages: &'g mut Coverages,
    lookups: Option<&'g [u16]>,
    built: BTreeMap<*const Value<'v>, Option<usize>>,
}

// A table being written: its inline fields and the offsets among them.
struct Node {
    header: Vec<u8>,
    links: Vec<Link>,
    fits: bool,
}

impl Node {
    fn link(&mut self, at: usize, width: usize, to: Option<usize>) {
        match to {
            Some(to) => self.links.push(Link { at, width, to }),
            None => self.fits = false,
        }
    }

    // `v` in `w` bytes at `pos`; one too wide for them fails the table.
    fn uint(&mut self, pos: usize, w: usize, v: u32) -> usize {
        if w < 4 && v >> (8 * w) != 0 {
            self.fits = false;
        }
        self.header[pos..pos + w].copy_from_slice(&v.to_be_bytes()[4 - w..]);
        w
    }
}

impl<'v> Emit<'v, '_> {
    fn table(&mut self, value: &'v Value<'v>) -> Option<usize> {
        // A format 3 anchor's Device tables are objects of their own, so anchors naming one share it.
        if let Value::Anchor(_, x, y, _, Some(devices)) = value
            && devices.iter().any(Option::is_some)
        {
            let mut header = alloc::vec![0u8; 10];
            write_u16_be(&mut header, 0, 3);
            write_i16_be(&mut header, 2, *x);
            write_i16_be(&mut header, 4, *y);
            let mut links = Vec::new();
            for (at, device) in [(6, devices[0]), (8, devices[1])] {
                if let Some(bytes) = device {
                    links.push(Link { at, width: 2, to: self.graph.add(bytes.to_vec(), Vec::new()) });
                }
            }
            return Some(self.graph.add(header, links));
        }
        if let Some(bytes) = leaf(value, self.coverages) {
            return Some(self.graph.add(bytes, Vec::new()));
        }
        let header = alloc::vec![0u8; inline_width(value, self.coverages)];
        let mut node = Node { header, links: Vec::new(), fits: true };
        self.write(&mut node, value, 0);
        node.fits.then(|| self.graph.add(node.header, node.links))
    }

    // A child more than one table names is emitted once.
    fn child(&mut self, value: &'v Rc<Value<'v>>) -> Option<usize> {
        if Rc::strong_count(value) == 1 {
            return self.table(value);
        }
        if let Some(&done) = self.built.get(&Rc::as_ptr(value)) {
            return done;
        }
        let done = self.table(value);
        self.built.insert(Rc::as_ptr(value), done);
        done
    }

    fn blob(&mut self, bytes: Vec<u8>) -> Option<usize> {
        Some(self.graph.add(bytes, Vec::new()))
    }

    fn write(&mut self, node: &mut Node, value: &'v Value<'v>, pos: usize) -> usize {
        match value {
            Value::U16(v) | Value::ValueFormat(_, v) => {
                write_u16_be(&mut node.header, pos, *v);
                2
            }
            Value::LookupIndex(v) => {
                write_u16_be(&mut node.header, pos, numbered(self.lookups, *v));
                2
            }
            Value::U24(v) => node.uint(pos, 3, *v),
            Value::Glyph(w, g) => node.uint(pos, w.bytes(), *g),
            Value::Offset(width, child) => {
                if let Some(v) = child {
                    let to = self.child(v);
                    node.link(pos, width.bytes(), to);
                }
                width.bytes()
            }
            Value::Anchor(width, ..) => {
                let to = self.table(value);
                node.link(pos, width.bytes(), to);
                width.bytes()
            }
            Value::Array(elems) | Value::Struct(elems) => elems.iter().fold(0, |p, e| p + self.write(node, e, pos + p)),
            Value::Glyphs(w, glyphs) => {
                glyphs.iter().enumerate().map(|(i, &g)| node.uint(pos + w.bytes() * i, w.bytes(), g)).sum()
            }
            Value::GlyphMap(pairs) => {
                let w = substitute_width(pairs);
                let (l, size, delta) = (w.long().bytes(), w.bytes(), single_delta(pairs, w));
                write_u16_be(&mut node.header, pos, single_format(w, delta));
                let gids: Vec<u32> = pairs.iter().map(|&(g, _)| g).collect();
                let to = self.blob(build_coverage(&gids));
                node.link(pos + 2, l, to);
                if let Some(delta) = delta {
                    return 2 + l + node.uint(pos + 2 + l, size, delta);
                }
                node.uint(pos + 2 + l, size, pairs.len() as u32);
                for (i, &(_, sub)) in pairs.iter().enumerate() {
                    node.uint(pos + 2 + l + size * (1 + i), size, sub);
                }
                2 + l + size * (1 + pairs.len())
            }
            Value::OffsetArray(width, slots) => {
                for (i, slot) in slots.iter().enumerate() {
                    if let Some(v) = slot {
                        let to = self.child(v);
                        node.link(pos + i * width.bytes(), width.bytes(), to);
                    }
                }
                slots.len() * width.bytes()
            }
            Value::ValueRecord(r) => self.record(node, r, pos),
            Value::Coverage(_) | Value::ClassDef(_) | Value::CaretValue(..) => {
                let bytes = leaf(value, self.coverages).unwrap_or_default();
                node.header[pos..pos + bytes.len()].copy_from_slice(&bytes);
                bytes.len()
            }
            Value::ClassMatrix(m) => {
                let l = m.width.long().bytes();
                for (slot, entries) in [(pos, &m.class_def1), (pos + l, &m.class_def2)] {
                    let to = self.blob(build_classdef(entries));
                    node.link(slot, l, to);
                }
                write_u16_be(&mut node.header, pos + 2 * l, m.class1_count);
                write_u16_be(&mut node.header, pos + 2 * l + 2, m.class2_count);
                m.grid.iter().fold(2 * l + 4, |p, r| p + self.record(node, r, pos + p))
            }
            Value::CoveredArray(c) => {
                let gids: Vec<u32> = c.entries.iter().map(|(g, _)| *g).collect();
                let to = self.blob(build_coverage(&gids));
                node.link(pos, c.width.long().bytes(), to);
                let mut p = pos + c.width.long().bytes();
                for v in &c.fields {
                    p += self.write(node, v, p);
                }
                p += node.uint(p, c.width.bytes(), c.entries.len() as u32);
                for (_, v) in &c.entries {
                    p += self.payload(node, c.shape, Some(v), p);
                }
                p - pos
            }
            Value::ZippedWithBoundCoverage(shape, w, entries) => {
                let mut p = pos + node.uint(pos, w.bytes(), entries.len() as u32);
                for v in entries {
                    p += self.payload(node, *shape, v.as_ref(), p);
                }
                p - pos
            }
        }
    }

    // A ValueRecord's values, then its Device offsets, each Device table an object of its own.
    fn record(&mut self, node: &mut Node, r: &'v Record<'v>, pos: usize) -> usize {
        let mut p = pos;
        for v in r.values.iter().take((r.format & 0x000F).count_ones() as usize) {
            write_i16_be(&mut node.header, p, *v);
            p += 2;
        }
        for i in 0..(r.format & 0x00F0).count_ones() as usize {
            if let Some(bytes) = r.devices.as_ref().and_then(|d| d[i]) {
                let to = self.blob(bytes.to_vec());
                node.link(p, 2, to);
            }
            p += 2;
        }
        p - pos
    }

    // A record of a covered array, inline or behind an offset (NULL for none).
    fn payload(&mut self, node: &mut Node, shape: PayloadShape, value: Option<&'v Value<'v>>, pos: usize) -> usize {
        match (shape, value) {
            (_, Some(v)) => self.write(node, v, pos),
            (PayloadShape::Inline, None) => 0,
            (PayloadShape::Offsets(width), None) => width.bytes(),
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/unit/daetype/subsetter/otl/generic/graph.rs"]
mod tests;
