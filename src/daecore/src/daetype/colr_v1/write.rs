// COLR v1 written back out for the subsetter and instancer: what the kept glyphs reach, each table
// once, in source order so every offset, unsigned in the format, still points forward.
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::collections::btree_map::Entry;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ops::Range;

use super::super::decoder::{read_i16_be, read_offset24, read_u16_be, read_u32_be, records_fit, write_offset24, write_u16_be, write_u32_be};
use super::super::format::ivs::ItemVariationStore;
use super::super::format::round::ot_round;
use super::format::paint_layout;
use super::varfield::resolve_delta_raw;
use crate::daecore::daemachine::daemath::matrix::Matrix;

const MAX_BASE_GLYPHS: usize = 65536;

// Where an instance is made: what a varIndexBase's deltas come to there.
pub(crate) struct Location<'a> {
    pub(crate) store: &'a ItemVariationStore,
    pub(crate) map: Option<&'a [(u32, u32)]>,
    pub(crate) scalars: &'a [f64],
}

impl Location<'_> {
    fn delta(&self, var_index_base: u32, k: u32) -> f64 {
        resolve_delta_raw(Some(self.store), self.map, self.scalars, var_index_base, k)
    }
}

#[derive(Clone, Copy)]
enum Width {
    Offset24,
    Offset32,
}

type Ref = (usize, Width, usize);

struct Table {
    // The order written in: the three lists first, then by source offset, a paint before a color
    // line, transform or clip box at the same offset, then in the order made.
    key: (usize, u8, usize),
    // Where its bytes and offsets sit in the buffers every table shares.
    bytes: Range<usize>,
    refs: Range<usize>,
}

// A paint's bytes and offsets as it is built, one buffer reused for every paint.
#[derive(Default)]
struct Node {
    bytes: Vec<u8>,
    refs: Vec<Ref>,
}

const LIST: u8 = 0;
const PAINT: u8 = 1;
const PART: u8 = 2;

// A paint that cannot be read is written as an empty PaintColrLayers, which draws nothing and is
// bounded, as the spec has a reader treat it.
const NOTHING: [u8; 6] = [1, 0, 0, 0, 0, 0];

pub(crate) struct Built {
    tables: Vec<Table>,
    data: Vec<u8>,
    refs: Vec<Ref>,
    lists: [Option<usize>; 3],
}

struct Builder<'a> {
    colr: &'a [u8],
    layer_list: Option<(usize, usize)>,
    remap: &'a dyn Fn(u16) -> u16,
    at: Option<&'a Location<'a>>,
    tables: Vec<Table>,
    data: Vec<u8>,
    refs: Vec<Ref>,
    paints: BTreeMap<usize, usize>,
    parts: BTreeMap<(usize, u8, u64, u64), usize>,
    lines: BTreeMap<Vec<u8>, usize>,
    queue: Vec<(usize, usize)>,
    colr_layers: Vec<(usize, usize, usize)>,
    layer_paints: BTreeMap<usize, usize>,
    bytes_left: usize,
    // A gradient's stops and its color line's bytes, reused from one gradient to the next.
    stops: Vec<(f64, u16, i16)>,
    line: Vec<u8>,
}

// The v1 tables for the glyphs `keep` names, glyph IDs in paints through `remap`, static at `at`
// when given. None past `budget` bytes or when an offset would not fit its field.
pub(crate) fn rebuild(
    colr: &[u8],
    keep: &dyn Fn(u16) -> Option<u16>,
    remap: &dyn Fn(u16) -> u16,
    at: Option<&Location>,
    budget: usize,
) -> Option<Built> {
    if colr.len() < 34 || read_u16_be(colr, 0)? != 1 { return None; }
    let list = read_u32_be(colr, 14).filter(|&v| v != 0)? as usize;
    let layer_list = read_u32_be(colr, 18)
        .filter(|&v| v != 0)
        .and_then(|off| Some((off as usize, read_u32_be(colr, off as usize)? as usize)))
        .filter(|&(off, n)| records_fit(off + 4, n, 4, colr.len()));
    let count = read_u32_be(colr, list)? as usize;
    if !records_fit(list.checked_add(4)?, count.min(MAX_BASE_GLYPHS), 6, colr.len()) { return None; }

    let mut records: Vec<(u16, u16, usize)> = (0..count.min(MAX_BASE_GLYPHS))
        .filter_map(|i| {
            let rec = list + 4 + i * 6;
            let gid = read_u16_be(colr, rec)?;
            Some((keep(gid)?, gid, list.checked_add(read_u32_be(colr, rec + 2)? as usize)?))
        })
        .collect();
    records.sort_by_key(|r| r.0);
    records.dedup_by_key(|r| r.0);
    if records.is_empty() { return None; }

    let mut b = Builder {
        colr, layer_list, remap, at,
        tables: Vec::new(),
        data: Vec::new(),
        refs: Vec::new(),
        paints: BTreeMap::new(),
        parts: BTreeMap::new(),
        lines: BTreeMap::new(),
        queue: Vec::new(),
        colr_layers: Vec::new(),
        layer_paints: BTreeMap::new(),
        bytes_left: budget,
        stops: Vec::new(),
        line: Vec::new(),
    };
    let roots: Vec<usize> = records.iter().map(|&(_, _, src)| b.paint(src)).collect();
    let mut node = Node::default();
    while let Some((table, src)) = b.queue.pop() {
        node.bytes.clear();
        node.refs.clear();
        if b.build_paint(src, table, &mut node).is_none() {
            node.bytes.clear();
            node.refs.clear();
            node.bytes.extend_from_slice(&NOTHING);
        }
        b.bytes_left = b.bytes_left.checked_sub(node.bytes.len())?;
        b.fill(table, &node.bytes, &node.refs);
    }

    let base_glyph_list = {
        let mut bytes = (records.len() as u32).to_be_bytes().to_vec();
        let mut refs = Vec::with_capacity(records.len());
        for (&(gid, _, _), &root) in records.iter().zip(&roots) {
            refs.push((bytes.len() + 2, Width::Offset32, root));
            bytes.extend(gid.to_be_bytes());
            bytes.extend([0; 4]);
        }
        b.add((0, LIST, 0), &bytes, &refs)
    };
    let layer_list = b.layer_list_table();
    let clip_list = b.clip_list_table(&records);
    (b.bytes_left > 0).then_some(())?;
    Some(Built { tables: b.tables, data: b.data, refs: b.refs, lists: [Some(base_glyph_list), layer_list, clip_list] })
}

impl Builder<'_> {
    fn add(&mut self, key: (usize, u8, usize), bytes: &[u8], refs: &[Ref]) -> usize {
        self.bytes_left = self.bytes_left.saturating_sub(bytes.len());
        let (bytes, refs) = self.append(bytes, refs);
        self.tables.push(Table { key, bytes, refs });
        self.tables.len() - 1
    }

    // A paint made as a placeholder, given its bytes once built.
    fn fill(&mut self, table: usize, bytes: &[u8], refs: &[Ref]) {
        let (bytes, refs) = self.append(bytes, refs);
        self.tables[table].bytes = bytes;
        self.tables[table].refs = refs;
    }

    fn append(&mut self, bytes: &[u8], refs: &[Ref]) -> (Range<usize>, Range<usize>) {
        let (at, ref_at) = (self.data.len(), self.refs.len());
        self.data.extend_from_slice(bytes);
        self.refs.extend_from_slice(refs);
        (at..self.data.len(), ref_at..self.refs.len())
    }

    fn next_key(&self, src: usize, kind: u8) -> (usize, u8, usize) {
        (src, kind, self.tables.len())
    }

    // The table for the paint at `src`, built later from the queue so a deep graph needs no stack.
    fn paint(&mut self, src: usize) -> usize {
        let t = self.tables.len();
        match self.paints.entry(src) {
            Entry::Occupied(known) => return *known.get(),
            Entry::Vacant(slot) => { slot.insert(t); }
        }
        self.add(self.next_key(src, PAINT), &[], &[]);
        self.queue.push((t, src));
        t
    }

    fn child(&mut self, src: usize, at: usize) -> Option<usize> {
        let target = src.checked_add(read_offset24(self.colr, src + at)?);
        Some(match target {
            Some(t) => self.paint(t),
            None => self.add(self.next_key(src, PART), &NOTHING, &[]),
        })
    }

    // A color line, transform or clip box copied as it is, once per source offset.
    fn verbatim(&mut self, src: usize, len: usize) -> Option<usize> {
        if let Some(&t) = self.parts.get(&(src, 0, 0, 0)) { return Some(t); }
        let colr = self.colr;
        let bytes = colr.get(src..src.checked_add(len)?)?;
        let t = self.add(self.next_key(src, PART), bytes, &[]);
        self.parts.insert((src, 0, 0, 0), t);
        Some(t)
    }

    fn color_line_len(&self, src: usize, is_var: bool) -> Option<usize> {
        let n = usize::from(read_u16_be(self.colr, src.checked_add(1)?)?);
        let stride = if is_var { 10 } else { 6 };
        records_fit(src + 3, n, stride, self.colr.len()).then_some(3 + n * stride)
    }

    fn build_paint(&mut self, src: usize, table: usize, node: &mut Node) -> Option<()> {
        let colr = self.colr;
        let format = *colr.get(src)?;
        let layout = paint_layout(format)?;
        let inline = colr.get(src..src.checked_add(layout.inline_len)?)?;
        if format == 1 {
            self.colr_layers_paint(src, table)?;
            node.bytes.extend_from_slice(inline);
            return Some(());
        }
        if let Some(at) = self.at
            && matches!(format, 3 | 5 | 7 | 9 | 13 | 15 | 17 | 19 | 21 | 23 | 25 | 27 | 29 | 31)
        {
            return self.instance_paint(src, format, at, node);
        }
        node.bytes.extend_from_slice(inline);
        for &pos in layout.children {
            node.refs.push((pos, Width::Offset24, self.child(src, pos)?));
        }
        match format {
            4..=9 => {
                let line = src.checked_add(read_offset24(colr, src + 1)?)?;
                let len = self.color_line_len(line, format % 2 == 1)?;
                node.refs.push((1, Width::Offset24, self.verbatim(line, len)?));
            }
            12 | 13 => {
                let affine = src.checked_add(read_offset24(colr, src + 4)?)?;
                node.refs.push((4, Width::Offset24, self.verbatim(affine, if format == 13 { 28 } else { 24 })?));
            }
            10 => write_u16_be(&mut node.bytes, 4, (self.remap)(read_u16_be(colr, src + 4)?)),
            11 => write_u16_be(&mut node.bytes, 1, (self.remap)(read_u16_be(colr, src + 1)?)),
            _ => {}
        }
        Some(())
    }

    // A PaintColrLayers names a range of the LayerList; its first index is set once the list is
    // compacted, and the layers past the list's end, which a reader draws as nothing, are dropped.
    fn colr_layers_paint(&mut self, src: usize, table: usize) -> Option<()> {
        let (list, count) = self.layer_list?;
        let n = usize::from(*self.colr.get(src + 1)?);
        let first = read_u32_be(self.colr, src + 2)? as usize;
        let end = first.saturating_add(n).min(count);
        for idx in first..end {
            if !self.layer_paints.contains_key(&idx) {
                let target = list.checked_add(read_u32_be(self.colr, list + 4 + idx * 4)? as usize)?;
                let t = self.paint(target);
                self.layer_paints.insert(idx, t);
            }
        }
        self.colr_layers.push((table, first, end.saturating_sub(first)));
        Some(())
    }

    fn layer_list_table(&mut self) -> Option<usize> {
        if self.colr_layers.is_empty() { return None; }
        let rank: BTreeMap<usize, usize> = self.layer_paints.keys().enumerate().map(|(r, &idx)| (idx, r)).collect();
        for &(table, first, n) in &self.colr_layers {
            let at = self.tables[table].bytes.start;
            if let Some(count) = self.data.get_mut(at + 1) { *count = n as u8; }
            write_u32_be(&mut self.data, at + 2, if n == 0 { 0 } else { rank[&first] as u32 });
        }
        let mut bytes = (self.layer_paints.len() as u32).to_be_bytes().to_vec();
        let mut refs = Vec::with_capacity(self.layer_paints.len());
        for &t in self.layer_paints.values() {
            refs.push((bytes.len(), Width::Offset32, t));
            bytes.extend([0; 4]);
        }
        Some(self.add((0, LIST, 1), &bytes, &refs))
    }

    // Clip records for the kept glyphs under their new IDs, a run of glyphs sharing a box as one.
    fn clip_list_table(&mut self, records: &[(u16, u16, usize)]) -> Option<usize> {
        let mut clips: Vec<(u16, u16, usize)> = Vec::new();
        for &(gid, old, _) in records {
            let Some(src) = super::clip_box_offset(self.colr, old) else { continue };
            let Some(t) = self.clip_box(src) else { continue };
            match clips.last_mut() {
                Some(last) if last.2 == t && last.1.checked_add(1) == Some(gid) => last.1 = gid,
                _ => clips.push((gid, gid, t)),
            }
        }
        if clips.is_empty() { return None; }
        let mut bytes = alloc::vec![1u8];
        bytes.extend((clips.len() as u32).to_be_bytes());
        let mut refs = Vec::with_capacity(clips.len());
        for (start, end, t) in clips {
            bytes.extend(start.to_be_bytes());
            bytes.extend(end.to_be_bytes());
            refs.push((bytes.len(), Width::Offset24, t));
            bytes.extend([0; 3]);
        }
        Some(self.add((0, LIST, 2), &bytes, &refs))
    }

    fn clip_box(&mut self, src: usize) -> Option<usize> {
        match (*self.colr.get(src)?, self.at) {
            (1, _) => self.verbatim(src, 9),
            (2, None) => self.verbatim(src, 13),
            (2, Some(at)) => {
                if let Some(&t) = self.parts.get(&(src, 1, 0, 0)) { return Some(t); }
                let vib = read_u32_be(self.colr, src + 9)?;
                let mut bytes = [1u8; 9];
                for k in 0..4 {
                    let v = f64::from(read_i16_be(self.colr, src + 1 + k * 2)?) + at.delta(vib, k as u32);
                    bytes[1 + k * 2..3 + k * 2].copy_from_slice(&clamp_i16(f64::from(super::outward(v, k >= 2))).to_be_bytes());
                }
                let t = self.add(self.next_key(src, PART), &bytes, &[]);
                self.parts.insert((src, 1, 0, 0), t);
                Some(t)
            }
            _ => None,
        }
    }

    // A Var paint as its static format at the location; a value its field cannot hold is written
    // another way that draws the same (a PaintTransform, or a gradient's geometry moved).
    fn instance_paint(&mut self, src: usize, format: u8, at: &Location, node: &mut Node) -> Option<()> {
        let colr = self.colr;
        let i16_at = |pos: usize, vib: u32, k: u32| Some(f64::from(read_i16_be(colr, src + pos)?) + at.delta(vib, k));
        let vib_at = |pos: usize| read_u32_be(colr, src + pos);
        let f2dot14 = |v: f64| v / 16384.0;
        match format {
            3 => {
                let vib = vib_at(5)?;
                let alpha = ot_round(i16_at(3, vib, 0)?).clamp(0, 16384) as i16;
                let palette = read_u16_be(colr, src + 1)?;
                node.bytes.push(2);
                node.bytes.extend(palette.to_be_bytes());
                node.bytes.extend(alpha.to_be_bytes());
                Some(())
            }
            5 | 7 | 9 => self.instance_gradient(src, format, at, node),
            13 => {
                let affine = src.checked_add(read_offset24(colr, src + 4)?)?;
                let vib = read_u32_be(colr, affine + 24)?;
                let mut m = [0.0; 6];
                for (k, v) in m.iter_mut().enumerate() {
                    *v = (f64::from(read_u32_be(colr, affine + k * 4)? as i32) + at.delta(vib, k as u32)) / 65536.0;
                }
                self.transform(src, m, node)
            }
            15 => {
                let vib = vib_at(8)?;
                let (dx, dy) = (i16_at(4, vib, 0)?, i16_at(6, vib, 1)?);
                self.static_node(src, 14, &[clamp_i16(dx.round()), clamp_i16(dy.round())], node)
            }
            17 | 19 | 21 | 23 => {
                let uniform = format >= 21;
                let centered = matches!(format, 19 | 23);
                let after = if uniform { 6 } else { 8 };
                let vib = vib_at(if centered { after + 4 } else { after })?;
                let sx = ot_round(i16_at(4, vib, 0)?);
                let sy = if uniform { sx } else { ot_round(i16_at(6, vib, 1)?) };
                let k = if uniform { 1 } else { 2 };
                let center = if centered { Some((i16_at(after, vib, k)?, i16_at(after + 2, vib, k + 1)?)) } else { None };
                let fields = || {
                    let mut f = alloc::vec![i16::try_from(sx).ok()?];
                    if !uniform { f.push(i16::try_from(sy).ok()?); }
                    if let Some((cx, cy)) = center { f.extend([fits_i16(cx)?, fits_i16(cy)?]); }
                    Some(f)
                };
                match fields() {
                    Some(f) => self.static_node(src, format - 1, &f, node),
                    None => {
                        let center = center.map(|(cx, cy)| (cx.round(), cy.round()));
                        self.transform(src, super::scale_matrix(f2dot14(f64::from(sx)), f2dot14(f64::from(sy)), center), node)
                    }
                }
            }
            25 | 27 => {
                let centered = format == 27;
                let vib = vib_at(if centered { 10 } else { 6 })?;
                let angle = wrap_angle(ot_round(i16_at(4, vib, 0)?));
                let center = if centered { Some((i16_at(6, vib, 1)?.round(), i16_at(8, vib, 2)?.round())) } else { None };
                match center.map(|(cx, cy)| fits_i16(cx).zip(fits_i16(cy))) {
                    None => self.static_node(src, 24, &[angle], node),
                    Some(Some((cx, cy))) => self.static_node(src, 26, &[angle, cx, cy], node),
                    Some(None) => self.transform(src, super::rotate_matrix(f2dot14(f64::from(angle)) * 180.0, center), node),
                }
            }
            29 | 31 => {
                let centered = format == 31;
                let vib = vib_at(if centered { 12 } else { 8 })?;
                let x = wrap_angle(ot_round(i16_at(4, vib, 0)?));
                let y = wrap_angle(ot_round(i16_at(6, vib, 1)?));
                let center = if centered { Some((i16_at(8, vib, 2)?.round(), i16_at(10, vib, 3)?.round())) } else { None };
                match center.map(|(cx, cy)| fits_i16(cx).zip(fits_i16(cy))) {
                    None => self.static_node(src, 28, &[x, y], node),
                    Some(Some((cx, cy))) => self.static_node(src, 30, &[x, y, cx, cy], node),
                    Some(None) => {
                        let (xd, yd) = (f2dot14(f64::from(x)) * 180.0, f2dot14(f64::from(y)) * 180.0);
                        self.transform(src, super::skew_matrix(xd, yd, center), node)
                    }
                }
            }
            _ => None,
        }
    }

    // A static transforming paint: its format, its child and these fields.
    fn static_node(&mut self, src: usize, format: u8, fields: &[i16], node: &mut Node) -> Option<()> {
        let child = self.child(src, 1)?;
        node.bytes.extend([format, 0, 0, 0]);
        fields.iter().for_each(|v| node.bytes.extend(v.to_be_bytes()));
        node.refs.push((1, Width::Offset24, child));
        Some(())
    }

    fn transform(&mut self, src: usize, m: Matrix, node: &mut Node) -> Option<()> {
        let child = self.child(src, 1)?;
        let mut affine = [0u8; 24];
        for (field, &v) in affine.as_chunks_mut::<4>().0.iter_mut().zip(m.iter()) {
            *field = clamp_i32((v * 65536.0).round()).to_be_bytes();
        }
        let affine = self.add(self.next_key(src, PART), &affine, &[]);
        node.bytes.extend([12, 0, 0, 0, 0, 0, 0]);
        node.refs.extend([(1, Width::Offset24, child), (4, Width::Offset24, affine)]);
        Some(())
    }

    fn instance_gradient(&mut self, src: usize, format: u8, at: &Location, node: &mut Node) -> Option<()> {
        let colr = self.colr;
        let line = src.checked_add(read_offset24(colr, src + 1)?)?;
        self.color_line_len(line, true)?;
        let extend = *colr.get(line)?;
        let n = usize::from(read_u16_be(colr, line + 1)?);
        let mut stops = core::mem::take(&mut self.stops);
        stops.clear();
        for i in 0..n {
            let s = line + 3 + i * 10;
            let vib = read_u32_be(colr, s + 6)?;
            let offset = (f64::from(read_i16_be(colr, s)?) + at.delta(vib, 0)) / 16384.0;
            let alpha = ot_round(f64::from(read_i16_be(colr, s + 4)?) + at.delta(vib, 1)).clamp(0, 16384) as i16;
            stops.push((offset, read_u16_be(colr, s + 2)?, alpha));
        }
        let vib = read_u32_be(colr, src + if format == 9 { 12 } else { 16 })?;
        let fields = if format == 9 { 4 } else { 6 };
        let mut g = [0.0f64; 6];
        for (k, v) in g.iter_mut().enumerate().take(fields) {
            let raw = if format == 7 && (k == 2 || k == 5) {
                f64::from(read_u16_be(colr, src + 4 + k * 2)?)
            } else {
                f64::from(read_i16_be(colr, src + 4 + k * 2)?)
            };
            *v = raw + at.delta(vib, k as u32);
        }
        if format == 9 {
            g[2] /= 16384.0;
            g[3] /= 16384.0;
        }

        if format == 9 {
            visible_sweep_stops(&g, &mut stops, extend);
        }
        let (a, len) = refit(format, &g, &stops);
        let (geometry, n_fields) = gradient_fields(format, &g, a, len);
        node.bytes.extend([format - 1, 0, 0, 0]);
        for (k, &v) in geometry[..n_fields].iter().enumerate() {
            if format == 7 && (k == 2 || k == 5) {
                node.bytes.extend((v.round().clamp(0.0, f64::from(u16::MAX)) as u16).to_be_bytes());
            } else {
                node.bytes.extend(clamp_i16(v.round()).to_be_bytes());
            }
        }
        // A zero len is a radial gradient whose radii are negative throughout: nothing is drawn.
        let mut out = core::mem::take(&mut self.line);
        out.clear();
        out.push(extend);
        let kept: &[_] = if len == 0.0 { &[] } else { &stops };
        out.extend((kept.len() as u16).to_be_bytes());
        for &(offset, palette, alpha) in kept {
            out.extend(clamp_i16(((offset - a) / len * 16384.0).round()).to_be_bytes());
            out.extend(palette.to_be_bytes());
            out.extend(alpha.to_be_bytes());
        }
        // Lines are shared by content, which a line moved for one gradient's geometry rarely is.
        let line_table = match self.lines.get(out.as_slice()) {
            Some(&t) => t,
            None => {
                let t = self.add(self.next_key(line, PART), &out, &[]);
                self.lines.insert(out.clone(), t);
                t
            }
        };
        (self.stops, self.line) = (stops, out);
        node.refs.push((1, Width::Offset24, line_table));
        Some(())
    }
}

// A gradient's fields with the stretch of its color line from `a` over `len` made 0 to 1: the
// points, radii or angles there. Sweep angles stay in their stored unit, (degrees / 180) - 1.
fn gradient_fields(format: u8, g: &[f64; 6], a: f64, len: f64) -> ([f64; 6], usize) {
    let p = |i: usize, j: usize, t: f64| g[i] + t * (g[j] - g[i]);
    match format {
        5 => {
            let (x0, y0) = (p(0, 2, a), p(1, 3, a));
            ([x0, y0, p(0, 2, a + len), p(1, 3, a + len), g[4] + x0 - g[0], g[5] + y0 - g[1]], 6)
        }
        7 => ([p(0, 3, a), p(1, 4, a), p(2, 5, a), p(0, 3, a + len), p(1, 4, a + len), p(2, 5, a + len)], 6),
        _ => ([g[0], g[1], p(2, 3, a) * 16384.0, p(2, 3, a + len) * 16384.0, 0.0, 0.0], 4),
    }
}

// The stretch of the color line, `a` over `len`, written as 0 to 1; the extend modes are defined over
// the stops' own interval, so moving geometry with stops draws the same. Taken, the first that fits:
// the line as is, first stop to last, a radial moved past its negative radii (direction kept, as
// later circles draw on top), a sweep over the widest stored span either way. Zero `len`: no radius.
fn refit(format: u8, g: &[f64; 6], stops: &[(f64, u16, i16)]) -> (f64, f64) {
    let (lo, hi) = stops.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), s| (l.min(s.0), h.max(s.0)));
    // Two at most to start, each radial one with its moved form, or two sweep spans: four in all.
    let mut candidates = [(0.0, 1.0); 4];
    let mut n = 1;
    if !stops.is_empty() {
        candidates[n] = (lo, if hi > lo { hi - lo } else { 1.0 });
        n += 1;
    }
    if format == 7 {
        let r = |t: f64| g[2] + t * (g[5] - g[2]);
        let given = n;
        for i in 0..given {
            let (a, len) = candidates[i];
            let (ra, rb) = (r(a), r(a + len));
            if ra >= 0.0 && rb >= 0.0 { continue; }
            let dr = rb - ra;
            candidates[n] = if dr == 0.0 {
                (a, 0.0)
            } else {
                let zero = -ra / dr;
                let span = 2.0 * 1f64.max(zero.abs()).max((1.0 - zero).abs());
                let start = if dr > 0.0 { zero } else { zero - span };
                (a + start * len, len * span)
            };
            n += 1;
        }
    }
    // A static sweep's angles run from -180 to just under 540 degrees: from one end to the other,
    // or back, holds stops furthest out.
    if format == 9 && g[3] != g[2] {
        let span = g[3] - g[2];
        candidates[n] = ((-2.0 - g[2]) / span, (F2DOT14_MAX + 2.0) / span);
        candidates[n + 1] = ((F2DOT14_MAX - g[2]) / span, (-2.0 - F2DOT14_MAX) / span);
        n += 2;
    }
    let fits = |&(a, len): &(f64, f64)| {
        if len == 0.0 { return true; }
        let f2dot14 = |v: f64| (-2.0..=F2DOT14_MAX).contains(&v);
        let (fields, n_fields) = gradient_fields(format, g, a, len);
        stops.iter().all(|s| f2dot14((s.0 - a) / len))
            && fields[..n_fields].iter().enumerate().all(|(k, &v)| match (format, k) {
                (7, 2 | 5) => (-0.5..=65535.5).contains(&v),
                (9, 2 | 3) => f2dot14(v / 16384.0),
                _ => (-32768.5..=32767.5).contains(&v),
            })
    };
    candidates[..n].iter().copied().find(fits).unwrap_or(candidates[n - 1])
}

const F2DOT14_MAX: f64 = 32767.0 / 16384.0;

// A sweep shows only 0 to 360 degrees: past the nearest stops (all at their offset) each side, none
// colors it. Padded, or repeating over 360 degrees or more, the others are dropped, exactly.
fn visible_sweep_stops(g: &[f64; 6], stops: &mut Vec<(f64, u16, i16)>, extend: u8) {
    if g[2] == g[3] { return; }
    let theta = |s: f64| (g[2] + s * (g[3] - g[2]) + 1.0) * 180.0;
    let (lo, hi) = stops.iter().map(|s| theta(s.0)).fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), t| (l.min(t), h.max(t)));
    if matches!(extend, 1 | 2) && !(lo <= 0.0 && hi >= 360.0) {
        return;
    }
    let below = stops.iter().map(|s| theta(s.0)).filter(|&t| t < 0.0).fold(f64::NEG_INFINITY, f64::max);
    let above = stops.iter().map(|s| theta(s.0)).filter(|&t| t > 360.0).fold(f64::INFINITY, f64::min);
    stops.retain(|s| (below..=above).contains(&theta(s.0)));
}

fn wrap_angle(v: i32) -> i16 {
    ((i64::from(v) + 0x8000).rem_euclid(0x1_0000) - 0x8000) as i16
}

fn fits_i16(v: f64) -> Option<i16> {
    let v = v.round();
    (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&v).then_some(v as i16)
}

fn clamp_i16(v: f64) -> i16 {
    v.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

fn clamp_i32(v: f64) -> i32 {
    v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

impl Built {
    // Appends the tables to `out`, a COLR table being assembled, and gives where the
    // BaseGlyphList, LayerList and ClipList start in it (0 for one not written).
    pub(crate) fn write(self, out: &mut Vec<u8>) -> Option<[u32; 3]> {
        // Keys sorted beside their tables rather than looked up through them. Each ends in the order
        // its table was made, so no two are equal and an unstable sort keeps that order.
        let mut keyed: Vec<((usize, u8, usize), usize)> = self.tables.iter().enumerate().map(|(t, table)| (table.key, t)).collect();
        keyed.sort_unstable();
        let order: Vec<usize> = keyed.into_iter().map(|(_, t)| t).collect();
        let mut at = alloc::vec![0usize; self.tables.len()];
        let mut end = out.len();
        for &t in &order {
            at[t] = end;
            end = end.checked_add(self.tables[t].bytes.len())?;
        }
        if end > u32::MAX as usize { return None; }
        for &t in &order {
            let table = &self.tables[t];
            let base = out.len();
            out.extend_from_slice(&self.data[table.bytes.clone()]);
            for &(pos, width, to) in &self.refs[table.refs.clone()] {
                let dist = at[to].checked_sub(at[t])?;
                match width {
                    Width::Offset24 if dist <= 0xFF_FFFF => write_offset24(out, base + pos, dist),
                    Width::Offset32 => write_u32_be(out, base + pos, dist as u32),
                    Width::Offset24 => return None,
                }
            }
        }
        Some(self.lists.map(|l| l.map_or(0, |t| at[t] as u32)))
    }
}

// A COLR table from its parts: the version 0 records, then for version 1 its tables, each copied
// where the header points. Nothing varies in a table written this way.
pub(crate) fn assemble(version: u16, v0: Option<(&[u8], u16, &[u8], u16)>, v1: Option<Built>) -> Option<Vec<u8>> {
    let mut out = alloc::vec![0u8; if version == 0 { 14 } else { 34 }];
    write_u16_be(&mut out, 0, version);
    if let Some((base, n, layers, m)) = v0 {
        let records = out.len() as u32;
        write_u16_be(&mut out, 2, n);
        write_u32_be(&mut out, 4, records);
        out.extend_from_slice(base);
        let layer_records = u32::try_from(out.len()).ok()?;
        write_u32_be(&mut out, 8, layer_records);
        out.extend_from_slice(layers);
        write_u16_be(&mut out, 12, m);
    }
    if let Some(v1) = v1 {
        let [base, layers, clips] = v1.write(&mut out)?;
        write_u32_be(&mut out, 14, base);
        write_u32_be(&mut out, 18, layers);
        write_u32_be(&mut out, 22, clips);
    }
    Some(out)
}
