// COLR v1 written back out for the subsetter and instancer: what the kept glyphs reach, each table once, packed after
// what it points to and written in reverse, so every offset, unsigned in the format, points forward.
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ops::Range;

use super::super::decoder::{
    clamp_i16, f2dot14, fixed, read_i16_be, read_i32_be, read_offset24, read_u16_be, read_u32_be, read_uint,
    records_fit, write_offset24, write_u16_be, write_u32_be, Fingerprint, FingerprintIndex, OFFSET24_MAX,
};
use super::super::format::ivs::DeltaSource;
use super::super::format::round::ot_round;
use super::format::{clip_box_at, color_line_stops, paint_layout};
use super::varfield::{resolve_delta, resolve_units};
use super::Center;
use crate::daecore::daekit::draw::matrix::Matrix;
use crate::daecore::daetype::subsetter::colr::ColrKept;

// Where an instance is made: what a varIndexBase's deltas come to there.
pub(crate) struct Location<'a> {
    pub(crate) source: &'a DeltaSource,
    pub(crate) scalars: &'a [i64],
}

impl Location<'_> {
    fn delta(&self, var_index_base: u32, k: u32) -> f64 {
        resolve_delta(super::Deltas::Measured(Some(self.source), self.scalars), var_index_base, k)
    }

    fn units(&self, var_index_base: u32, k: u32) -> f64 {
        resolve_units(super::Deltas::Measured(Some(self.source), self.scalars), var_index_base, k)
    }
}

#[derive(Clone, Copy)]
enum Width {
    Offset24,
    Offset32,
}

impl Width {
    fn bytes(self) -> usize {
        match self { Width::Offset24 => 3, Width::Offset32 => 4 }
    }
}

type Ref = (usize, Width, usize);

// Where a table's bytes and offsets sit in the buffers every table shares, and when a subset of it fails.
struct Table {
    bytes: Range<usize>,
    refs: Range<usize>,
    kind: Kind,
}

// A table fails where any table it points to fails, or, a PaintComposite or the LayerList, only where all do, an
// offset to a failed one written 0. A null offset is copied and fails nothing: absent, as in the font.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    All,
    Any,
    Null,
}

// A paint's bytes and offsets as it is built, one buffer reused for every paint.
#[derive(Default)]
struct Node {
    bytes: Vec<u8>,
    refs: Vec<Ref>,
}

// A paint that cannot be read is written as an empty PaintColrLayers, which draws nothing and is
// bounded, as the spec has a reader treat it.
const NOTHING: [u8; 6] = [1, 0, 0, 0, 0, 0];

pub(crate) struct Built {
    tables: Vec<Table>,
    data: Vec<u8>,
    refs: Vec<Ref>,
    lists: [Option<usize>; 3],
}

// Keys found again by fingerprint, each with its table, for the walk's many lookups. A key past the index's 64
// probes is missed and its table built twice, each copy alike, which the write pass shares.
#[derive(Default)]
struct Seen {
    index: FingerprintIndex,
    keys: Vec<(usize, usize)>,
}

impl Seen {
    fn print(key: usize) -> u64 {
        let mut print = Fingerprint::default();
        print.mix(&(key as u64).to_le_bytes());
        print.finish()
    }

    fn get(&self, key: usize) -> Option<usize> {
        self.index.find(Self::print(key), |n| self.keys[n].0 == key).map(|n| self.keys[n].1)
    }

    fn insert(&mut self, key: usize, t: usize) {
        self.index.push(Self::print(key));
        self.keys.push((key, t));
    }

    // Each key once, ascending, with the first table made for it.
    fn sorted(&self) -> impl Iterator<Item = (usize, usize)> {
        let mut keys = self.keys.clone();
        keys.sort_unstable();
        keys.dedup_by_key(|k| k.0);
        keys.into_iter()
    }
}

struct Builder<'a> {
    colr: &'a [u8],
    layer_list: Option<(usize, usize)>,
    remap: &'a dyn Fn(u32) -> u32,
    kept: Option<&'a ColrKept>,
    at: Option<&'a Location<'a>>,
    tables: Vec<Table>,
    data: Vec<u8>,
    refs: Vec<Ref>,
    paints: Seen,
    parts: BTreeMap<(usize, u8, u64, u64), usize>,
    // Paints to build, with their depth below their root, taken first in first out so a paint is first met at its
    // shallowest; `depth` is the one being built's.
    queue: Vec<(usize, usize, usize)>,
    depth: usize,
    colr_layers: Vec<(usize, u64, u8)>,
    layer_paints: Seen,
    // The LayerList indices each PaintColrLayers names, kept or past the list's end, where `kept` gives none.
    layer_ranges: Vec<(u64, u64)>,
    // Set where a paint names a palette index the closure didn't reach, which has no index in the subset to write.
    refused: bool,
    null: Option<usize>,
    bytes_left: usize,
    // A gradient's stops and its color line's bytes, reused from one gradient to the next.
    stops: Vec<(f64, u16, i16)>,
    line: Vec<u8>,
}

// The v1 tables for `keep`'s glyphs in BaseGlyphList order, ids through `remap`, palette indices and layers per the
// closure, static at `at` if given; None past `budget`, on offset overflow, or where a palette index is refused.
pub(crate) fn rebuild(
    colr: &[u8],
    keep: &dyn Fn(u16) -> Option<u16>,
    remap: &dyn Fn(u32) -> u32,
    kept: Option<&ColrKept>,
    at: Option<&Location>,
    budget: usize,
) -> Option<Built> {
    let header = super::header(colr)?;
    let list = header.base_glyph_list?;
    // A count leaving the table holds the layers within it, as drawing reads them.
    let layer_list = header.layer_list
        .and_then(|off| Some((off, read_u32_be(colr, off)? as usize)))
        .map(|(off, n)| (off, n.min((colr.len() - off - 4) / 4)));

    // Each kept glyph's root as drawing finds it, so a record the search misses, or whose paint leaves the table, is
    // left out; in glyph order, as the records the search finds run.
    let records: Vec<(u16, usize)> = (0..=u16::MAX)
        .filter_map(|gid| Some((keep(gid)?, super::lookup_base_glyph_paint_offset(colr, list, gid.into())?)))
        .collect();
    if records.is_empty() { return None; }

    let mut b = Builder {
        colr, layer_list, remap, kept, at,
        tables: Vec::new(),
        data: Vec::new(),
        refs: Vec::new(),
        paints: Seen::default(),
        parts: BTreeMap::new(),
        queue: Vec::new(),
        depth: 0,
        colr_layers: Vec::new(),
        layer_paints: Seen::default(),
        layer_ranges: Vec::new(),
        refused: false,
        null: None,
        bytes_left: budget,
        stops: Vec::new(),
        line: Vec::new(),
    };
    // The table's end is the null paint drawing reads for a record's offset of 0, written as one.
    let roots: Vec<usize> =
        records.iter().map(|&(_, src)| if src == colr.len() { b.null() } else { b.paint(src, 0) }).collect();
    let mut node = Node::default();
    let mut next = 0;
    while let Some(&(table, src, depth)) = b.queue.get(next) {
        next += 1;
        b.depth = depth;
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
        for (&(gid, _), &root) in records.iter().zip(&roots) {
            refs.push((bytes.len() + 2, Width::Offset32, root));
            bytes.extend(gid.to_be_bytes());
            bytes.extend([0; 4]);
        }
        b.add(&bytes, &refs)
    };
    let layer_list = b.layer_list_table();
    let clip_list = b.clip_list_table(keep);
    (b.bytes_left > 0 && !b.refused).then_some(())?;
    Some(Built { tables: b.tables, data: b.data, refs: b.refs, lists: [Some(base_glyph_list), layer_list, clip_list] })
}

impl Builder<'_> {
    fn add(&mut self, bytes: &[u8], refs: &[Ref]) -> usize {
        self.bytes_left = self.bytes_left.saturating_sub(bytes.len());
        let (bytes, refs) = self.append(bytes, refs);
        self.tables.push(Table { bytes, refs, kind: Kind::All });
        self.tables.len() - 1
    }

    // The one table every null offset names.
    fn null(&mut self) -> usize {
        if let Some(t) = self.null { return t; }
        let t = self.add(&[], &[]);
        self.tables[t].kind = Kind::Null;
        self.null = Some(t);
        t
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

    // The table for the paint at `src`, built later from the queue so a deep graph needs no stack; absent from
    // MAX_PAINT_DEPTH levels down, as drawing reads none that deep. A shared paint is written at its shallowest.
    fn paint(&mut self, src: usize, depth: usize) -> usize {
        if depth >= super::MAX_PAINT_DEPTH { return self.null(); }
        let t = self.tables.len();
        if let Some(known) = self.paints.get(src) { return known; }
        self.paints.insert(src, t);
        self.add(&[], &[]);
        self.queue.push((t, src, depth));
        t
    }

    fn child(&mut self, src: usize, at: usize) -> Option<usize> {
        let rel = read_offset24(self.colr, src + at)?;
        Some(match src.checked_add(rel) {
            _ if rel == 0 => self.null(),
            Some(t) => self.paint(t, self.depth + 1),
            None => self.add(&NOTHING, &[]),
        })
    }

    // A palette index as the subset writes it, refused where its closure didn't reach it.
    fn palette(&mut self, index: u16) -> Option<u16> {
        let Some(kept) = self.kept else { return Some(index) };
        let palette = kept.palettes.get(&index).copied();
        self.refused |= palette.is_none();
        palette
    }

    // A color line copied once per source offset and width, its stops' palette indices remapped: one offset read as
    // a ColorLine and a VarColorLine is two lines.
    fn color_line(&mut self, src: usize, n: usize, stride: usize) -> Option<usize> {
        let key = (src, 2, n as u64, stride as u64);
        if let Some(&t) = self.parts.get(&key) { return Some(t); }
        let mut line = core::mem::take(&mut self.line);
        line.clear();
        line.extend_from_slice(self.colr.get(src..src + 3 + n * stride)?);
        for stop in line[3..].chunks_exact_mut(stride) {
            let palette = self.palette(u16::from_be_bytes([stop[2], stop[3]]))?;
            stop[2..4].copy_from_slice(&palette.to_be_bytes());
        }
        let t = self.add(&line, &[]);
        self.line = line;
        self.parts.insert(key, t);
        Some(t)
    }

    // A transform or clip box copied as it is, once per source offset and length.
    fn verbatim(&mut self, src: usize, len: usize) -> Option<usize> {
        let key = (src, 0, len as u64, 0);
        if let Some(&t) = self.parts.get(&key) { return Some(t); }
        let colr = self.colr;
        let bytes = colr.get(src..src.checked_add(len)?)?;
        let t = self.add(bytes, &[]);
        self.parts.insert(key, t);
        Some(t)
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
        if format == 32 {
            self.tables[table].kind = Kind::Any;
        }
        for &pos in layout.children {
            node.refs.push((pos, Width::Offset24, self.child(src, pos)?));
        }
        if matches!(format, 12 | 13) {
            let affine = match read_offset24(colr, src + 4)? {
                0 => self.null(),
                rel => self.verbatim(src.checked_add(rel)?, if format == 13 { 28 } else { 24 })?,
            };
            node.refs.push((4, Width::Offset24, affine));
        }
        match format {
            2 | 3 => {
                let palette = self.palette(read_u16_be(colr, src + 1)?)?;
                write_u16_be(&mut node.bytes, 1, palette);
            }
            4..=9 => {
                let line = match read_offset24(colr, src + 1)? {
                    0 => self.null(),
                    rel => {
                        let line = src.checked_add(rel)?;
                        let (n, stride) = color_line_stops(colr, line, format % 2 == 1)?;
                        self.color_line(line, n, stride)?
                    }
                };
                node.refs.push((1, Width::Offset24, line));
            }
            _ => {}
        }
        if let Some((pos, w)) = layout.glyph {
            let gid = (self.remap)(read_uint(colr, src + pos, w)?);
            if gid >> (8 * w) != 0 { return None; }
            node.bytes.get_mut(pos..pos + w)?.copy_from_slice(&gid.to_be_bytes()[4 - w..]);
        }
        Some(())
    }

    // A PaintColrLayers names a range of the LayerList, its first index set once the list is compacted. Its count is
    // kept, a range past the list's end too, which a reader draws as nothing.
    fn colr_layers_paint(&mut self, src: usize, table: usize) -> Option<()> {
        let (list, count) = self.layer_list?;
        let n = *self.colr.get(src + 1)?;
        let first = read_u32_be(self.colr, src + 2)?;
        let end = (first as usize).saturating_add(n.into()).min(count);
        for idx in first as usize..end {
            if self.layer_paints.get(idx).is_none() {
                let t = match read_u32_be(self.colr, list + 4 + idx * 4)? {
                    0 => self.null(),
                    rel => self.paint(list.checked_add(rel as usize)?, self.depth + 1),
                };
                self.layer_paints.insert(idx, t);
            }
        }
        self.colr_layers.push((table, first.into(), n));
        self.layer_ranges.extend(layer_range(first, n));
        Some(())
    }

    // The LayerList of the layers the closure keeps, none where it keeps none. A PaintColrLayers' first index is its
    // rank among every index any of them names: 0 for no layers, and for one unnamed u32::MAX, past any list.
    fn layer_list_table(&mut self) -> Option<usize> {
        self.layer_list?;
        let ranges = merged(self.kept.map_or(&self.layer_ranges, |k| &k.layers));
        // The indices below each range, so an index ranks by one search.
        let below: Vec<u64> =
            ranges.iter().scan(0, |n, &(start, end)| Some(core::mem::replace(n, *n + end - start))).collect();
        let rank = |idx: u64| {
            let i = ranges.partition_point(|&(_, end)| end <= idx);
            ranges.get(i).filter(|&&(start, _)| start <= idx).map(|&(start, _)| below[i] + idx - start)
        };
        for &(table, first, n) in &self.colr_layers {
            let first = if n == 0 { 0 } else { rank(first).map_or(u32::MAX, |r| r as u32) };
            write_u32_be(&mut self.data, self.tables[table].bytes.start + 2, first);
        }
        let layers: Vec<usize> =
            self.layer_paints.sorted().filter(|&(idx, _)| rank(idx as u64).is_some()).map(|(_, t)| t).collect();
        if layers.is_empty() { return None; }
        let mut bytes = (layers.len() as u32).to_be_bytes().to_vec();
        let mut refs = Vec::with_capacity(layers.len());
        for t in layers {
            refs.push((bytes.len(), Width::Offset32, t));
            bytes.extend([0; 4]);
        }
        let list = self.add(&bytes, &refs);
        self.tables[list].kind = Kind::Any;
        Some(list)
    }

    // Clip records for every glyph `keep` names under its new ID, each with the box its font reads for it, so the
    // subset clips alike; a run of glyphs sharing a box is one record.
    fn clip_list_table(&mut self, keep: &dyn Fn(u16) -> Option<u16>) -> Option<usize> {
        let colr = self.colr;
        let list = super::header(colr)?.clip_list?;
        let mut clips: Vec<(u16, u16, usize)> = Vec::new();
        for (gid, src) in read_clips(colr, keep) {
            let Some(t) = self.clip_box(src) else { continue };
            match clips.last_mut() {
                Some(last) if last.2 == t && last.1.checked_add(1) == Some(gid) => last.1 = gid,
                _ => clips.push((gid, gid, t)),
            }
        }
        if clips.is_empty() { return None; }
        let mut bytes = alloc::vec![colr[list]];
        bytes.extend((clips.len() as u32).to_be_bytes());
        let mut refs = Vec::with_capacity(clips.len());
        for (start, end, t) in clips {
            bytes.extend(start.to_be_bytes());
            bytes.extend(end.to_be_bytes());
            refs.push((bytes.len(), Width::Offset24, t));
            bytes.extend([0; 3]);
        }
        Some(self.add(&bytes, &refs))
    }

    fn clip_box(&mut self, src: usize) -> Option<usize> {
        let (edges, var) = clip_box_at(self.colr, src)?;
        match (var, self.at) {
            (None, _) => self.verbatim(src, 9),
            (Some(_), None) => self.verbatim(src, 13),
            (Some(vib), Some(at)) => {
                if let Some(&t) = self.parts.get(&(src, 1, 0, 0)) { return Some(t); }
                let mut bytes = [1u8; 9];
                for (k, edge) in super::varied_box(edges, |k| at.units(vib, k)).into_iter().enumerate() {
                    bytes[1 + k * 2..3 + k * 2].copy_from_slice(&clamp_i16(edge).to_be_bytes());
                }
                let t = self.add(&bytes, &[]);
                self.parts.insert((src, 1, 0, 0), t);
                Some(t)
            }
        }
    }

    // A Var paint as its static format at the location; a value its field cannot hold is written
    // another way that draws the same (a PaintTransform, or a gradient's geometry moved).
    fn instance_paint(&mut self, src: usize, format: u8, at: &Location, node: &mut Node) -> Option<()> {
        let colr = self.colr;
        let i16_at = |pos: usize, vib: u32, k: u32| Some(f64::from(read_i16_be(colr, src + pos)?) + at.delta(vib, k));
        let vib_at = |pos: usize| read_u32_be(colr, src + pos);
        match format {
            3 => {
                let vib = vib_at(5)?;
                let alpha = ot_round(i16_at(3, vib, 0)?).clamp(0, 16384) as i16;
                let palette = self.palette(read_u16_be(colr, src + 1)?)?;
                node.bytes.push(2);
                node.bytes.extend(palette.to_be_bytes());
                node.bytes.extend(alpha.to_be_bytes());
                Some(())
            }
            5 | 7 | 9 => self.instance_gradient(src, format, at, node),
            13 => {
                // A null offset names no matrix, so it stays null, as a copy keeps it, rather than read from the paint.
                let affine = match read_offset24(colr, src + 4)? {
                    0 => return self.transform(src, None, node),
                    rel => src.checked_add(rel)?,
                };
                let vib = read_u32_be(colr, affine + 24)?;
                let mut m = [0.0; 6];
                for (k, v) in m.iter_mut().enumerate() {
                    *v = fixed(f64::from(read_i32_be(colr, affine + k * 4)?) + at.delta(vib, k as u32));
                }
                self.transform(src, Some(m), node)
            }
            15 => {
                let vib = vib_at(8)?;
                let (dx, dy) = (i16_at(4, vib, 0)?, i16_at(6, vib, 1)?);
                // Float casts saturate, here and below, so a value past its field is pinned to its range.
                self.static_node(src, 14, &[dx.round() as i16, dy.round() as i16], node)
            }
            17 | 19 | 21 | 23 => {
                let uniform = format >= 21;
                let centered = matches!(format, 19 | 23);
                let after = if uniform { 6 } else { 8 };
                let vib = vib_at(if centered { after + 4 } else { after })?;
                let sx = ot_round(i16_at(4, vib, 0)?);
                let sy = if uniform { sx } else { ot_round(i16_at(6, vib, 1)?) };
                let k = if uniform { 1 } else { 2 };
                let center = if centered {
                    Some(Center { x: i16_at(after, vib, k)?, y: i16_at(after + 2, vib, k + 1)? })
                } else {
                    None
                };
                let fields = || {
                    let mut f = alloc::vec![i16::try_from(sx).ok()?];
                    if !uniform { f.push(i16::try_from(sy).ok()?); }
                    if let Some(c) = center { f.extend([fits_i16(c.x)?, fits_i16(c.y)?]); }
                    Some(f)
                };
                match fields() {
                    Some(f) => self.static_node(src, format - 1, &f, node),
                    None => {
                        let center = center.map(|c| Center { x: c.x.round(), y: c.y.round() });
                        self.transform(src, Some(super::scale_matrix(f2dot14(sx), f2dot14(sy), center)), node)
                    }
                }
            }
            25 | 27 => {
                let centered = format == 27;
                let vib = vib_at(if centered { 10 } else { 6 })?;
                let angle = wrap_angle(ot_round(i16_at(4, vib, 0)?));
                let center = if centered {
                    Some(Center { x: i16_at(6, vib, 1)?.round(), y: i16_at(8, vib, 2)?.round() })
                } else {
                    None
                };
                match center.map(|c| fits_i16(c.x).zip(fits_i16(c.y))) {
                    None => self.static_node(src, 24, &[angle], node),
                    Some(Some((cx, cy))) => self.static_node(src, 26, &[angle, cx, cy], node),
                    Some(None) => self.transform(src, Some(super::rotate_matrix(f2dot14(angle) * 180.0, center)), node),
                }
            }
            29 | 31 => {
                let centered = format == 31;
                let vib = vib_at(if centered { 12 } else { 8 })?;
                let x = wrap_angle(ot_round(i16_at(4, vib, 0)?));
                let y = wrap_angle(ot_round(i16_at(6, vib, 1)?));
                let center = if centered {
                    Some(Center { x: i16_at(8, vib, 2)?.round(), y: i16_at(10, vib, 3)?.round() })
                } else {
                    None
                };
                match center.map(|c| fits_i16(c.x).zip(fits_i16(c.y))) {
                    None => self.static_node(src, 28, &[x, y], node),
                    Some(Some((cx, cy))) => self.static_node(src, 30, &[x, y, cx, cy], node),
                    Some(None) => {
                        let (xd, yd) = (f2dot14(x) * 180.0, f2dot14(y) * 180.0);
                        self.transform(src, Some(super::skew_matrix(xd, yd, center)), node)
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

    fn transform(&mut self, src: usize, m: Option<Matrix>, node: &mut Node) -> Option<()> {
        let child = self.child(src, 1)?;
        let affine = match m {
            Some(m) => {
                let mut affine = [0u8; 24];
                for (field, &v) in affine.as_chunks_mut::<4>().0.iter_mut().zip(m.iter()) {
                    *field = ((v * 65536.0).round() as i32).to_be_bytes();
                }
                self.add(&affine, &[])
            }
            None => self.null(),
        };
        node.bytes.extend([12, 0, 0, 0, 0, 0, 0]);
        node.refs.extend([(4, Width::Offset24, affine), (1, Width::Offset24, child)]);
        Some(())
    }

    fn instance_gradient(&mut self, src: usize, format: u8, at: &Location, node: &mut Node) -> Option<()> {
        let colr = self.colr;
        // A null color line names no stops, so it stays null, as a copy keeps it, rather than read from the paint.
        let rel = read_offset24(colr, src + 1)?;
        let line = src.checked_add(rel)?;
        let (n, extend) = match rel {
            0 => (0, 0),
            _ => (color_line_stops(colr, line, true)?.0, *colr.get(line)?),
        };
        let mut stops = core::mem::take(&mut self.stops);
        stops.clear();
        for i in 0..n {
            let s = line + 3 + i * 10;
            let vib = read_u32_be(colr, s + 6)?;
            let offset = f2dot14(f64::from(read_i16_be(colr, s)?) + at.delta(vib, 0));
            let alpha = ot_round(f64::from(read_i16_be(colr, s + 4)?) + at.delta(vib, 1)).clamp(0, 16384) as i16;
            stops.push((offset, self.palette(read_u16_be(colr, s + 2)?)?, alpha));
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
            g[2] = f2dot14(g[2]);
            g[3] = f2dot14(g[3]);
        }

        if format == 9 {
            visible_sweep_stops(&g, &mut stops, extend);
        }
        let (a, len) = refit(format, &g, &stops);
        let (geometry, n_fields) = gradient_fields(format, &g, a, len);
        node.bytes.extend([format - 1, 0, 0, 0]);
        for (k, &v) in geometry[..n_fields].iter().enumerate() {
            if format == 7 && (k == 2 || k == 5) {
                node.bytes.extend((v.round() as u16).to_be_bytes());
            } else {
                node.bytes.extend((v.round() as i16).to_be_bytes());
            }
        }
        // A zero len is a radial gradient whose radii are negative throughout: nothing is drawn.
        let mut out = core::mem::take(&mut self.line);
        out.clear();
        out.push(extend);
        let kept: &[_] = if len == 0.0 { &[] } else { &stops };
        out.extend((kept.len() as u16).to_be_bytes());
        for &(offset, palette, alpha) in kept {
            out.extend((((offset - a) / len * 16384.0).round() as i16).to_be_bytes());
            out.extend(palette.to_be_bytes());
            out.extend(alpha.to_be_bytes());
        }
        let line_table = if rel == 0 { self.null() } else { self.add(&out, &[]) };
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

// The color line's stretch, `a` over `len`, 0 to 1 (extends work over the stops, so moved geometry draws alike): the
// first fit of as is, stop to stop, a radial past negative radii, a sweep's widest span. 0 `len`: no radius.
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
        let fits_f2dot14 = |v: f64| (-2.0..=F2DOT14_MAX).contains(&v);
        let (fields, n_fields) = gradient_fields(format, g, a, len);
        stops.iter().all(|s| fits_f2dot14((s.0 - a) / len))
            && fields[..n_fields].iter().enumerate().all(|(k, &v)| match (format, k) {
                (7, 2 | 5) => (-0.5..=65535.5).contains(&v),
                (9, 2 | 3) => fits_f2dot14(f2dot14(v)),
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
    let (lo, hi) =
        stops.iter().map(|s| theta(s.0)).fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), t| (l.min(t), h.max(t)));
    if matches!(extend, 1 | 2) && !(lo <= 0.0 && hi >= 360.0) {
        return;
    }
    let below = stops.iter().map(|s| theta(s.0)).filter(|&t| t < 0.0).fold(f64::NEG_INFINITY, f64::max);
    let above = stops.iter().map(|s| theta(s.0)).filter(|&t| t > 360.0).fold(f64::INFINITY, f64::min);
    stops.retain(|s| (below..=above).contains(&theta(s.0)));
}

// The indices a PaintColrLayers names: none for no layers, or where the last would reach u32::MAX, past any index a
// LayerList's 32-bit count holds.
pub(crate) fn layer_range(first: u32, n: u8) -> Option<(u64, u64)> {
    let end = u64::from(first) + u64::from(n);
    (n > 0 && end <= u64::from(u32::MAX)).then_some((first.into(), end))
}

fn merged(ranges: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(sorted.len());
    for (start, end) in sorted {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

// Each kept glyph a ClipList record names, under its new id, in order, with the ClipBox `clip_box_offset` reads for
// it; none for a list of an unknown format or whose records leave the table.
fn read_clips(colr: &[u8], keep: &dyn Fn(u16) -> Option<u16>) -> BTreeMap<u16, usize> {
    let mut boxes = BTreeMap::new();
    let Some(list) = super::header(colr).and_then(|h| h.clip_list).filter(|&l| colr.get(l) == Some(&1)) else {
        return boxes;
    };
    let count = read_u32_be(colr, list + 1).map_or(0, |n| n as usize);
    if !records_fit(list + 5, count, 7, colr.len()) { return boxes; }
    let range = |i: usize| Some((read_u16_be(colr, list + 5 + i * 7)?, read_u16_be(colr, list + 7 + i * 7)?));
    let ranges: Vec<(u64, u64)> =
        (0..count).filter_map(range).filter(|&(f, l)| f <= l).map(|(f, l)| (f.into(), u64::from(l) + 1)).collect();
    for gid in merged(&ranges).into_iter().flat_map(|(start, end)| start..end).map(|g| g as u16) {
        if let Some((new, src)) = keep(gid).zip(super::clip_box_offset(colr, gid.into())) {
            boxes.insert(new, src);
        }
    }
    boxes
}

fn wrap_angle(v: i32) -> i16 {
    ((i64::from(v) + 0x8000).rem_euclid(0x1_0000) - 0x8000) as i16
}

fn fits_i16(v: f64) -> Option<i16> {
    let v = v.round();
    (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&v).then_some(v as i16)
}

impl Built {
    // Appends the tables to `out`, each packed after its targets and written before them, alike ones shared; gives
    // where the three lists start (0 if unwritten), None for a cycle or a failing BaseGlyphList.
    pub(crate) fn write(self, out: &mut Vec<u8>) -> Option<[u32; 3]> {
        const UNPACKED: usize = usize::MAX;
        const FAILED: usize = usize::MAX - 1;
        let n = self.tables.len();
        let (mut packed, mut same, mut entered) = (Vec::new(), alloc::vec![UNPACKED; n], alloc::vec![false; n]);
        let mut stack: Vec<(usize, usize)> = Vec::new();
        // A table as it's compared for sharing, in one run: its length, its bytes with offsets 0, then each offset's
        // place, width and table. Packed ones are found by fingerprint, as `packed` numbers them.
        let (mut known, mut form, mut other) = (FingerprintIndex::with_capacity(n), Vec::new(), Vec::new());
        let compared = |t: usize, same: &[usize], out: &mut Vec<u8>| -> Option<()> {
            let (bytes, refs) = (&self.data[self.tables[t].bytes.clone()], &self.refs[self.tables[t].refs.clone()]);
            out.clear();
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
            for &(pos, w, _) in refs {
                out.get_mut(4 + pos..4 + pos + w.bytes())?.fill(0);
            }
            for &(pos, w, to) in refs {
                out.extend_from_slice(&(pos as u32).to_le_bytes());
                out.push(w.bytes() as u8);
                out.extend_from_slice(&(same[to] as u64).to_le_bytes());
            }
            Some(())
        };
        for &root in self.lists.iter().flatten() {
            if same[root] != UNPACKED { continue; }
            entered[root] = true;
            stack.push((root, 0));
            while let Some((t, next)) = stack.last_mut() {
                let (t, refs) = (*t, self.tables[*t].refs.clone());
                if *next < refs.len() {
                    let to = self.refs[refs.start + *next].2;
                    *next += 1;
                    if same[to] == UNPACKED {
                        if entered[to] { return None; }
                        entered[to] = true;
                        stack.push((to, 0));
                    }
                    continue;
                }
                stack.pop();
                let refs = &self.refs[refs];
                let failed = |&(.., to): &Ref| same[to] == FAILED && self.tables[to].kind != Kind::Null;
                if match self.tables[t].kind {
                    Kind::Null => true,
                    Kind::All => refs.iter().any(failed),
                    Kind::Any => refs.iter().all(failed),
                } {
                    same[t] = FAILED;
                    continue;
                }
                compared(t, &same, &mut form)?;
                let mut print = Fingerprint::default();
                print.mix(&form);
                let hash = print.finish();
                let found = known.find(hash, |k| compared(packed[k], &same, &mut other).is_some() && other == form);
                same[t] = match found {
                    Some(k) => packed[k],
                    None => {
                        known.push(hash);
                        packed.push(t);
                        t
                    }
                };
            }
        }
        if self.lists[0].is_some_and(|base| same[base] == FAILED) {
            return None;
        }
        let mut at = alloc::vec![0usize; n];
        let mut end = out.len();
        for &t in packed.iter().rev() {
            at[t] = end;
            end = end.checked_add(self.tables[t].bytes.len())?;
        }
        if end > u32::MAX as usize { return None; }
        let at = |t: usize| (same[t] != FAILED).then(|| at[same[t]]);
        for &t in packed.iter().rev() {
            let table = &self.tables[t];
            let base = out.len();
            out.extend_from_slice(&self.data[table.bytes.clone()]);
            for &(pos, w, to) in &self.refs[table.refs.clone()] {
                let dist = match at(to) {
                    Some(to) => to.checked_sub(at(t)?)?,
                    None => 0,
                };
                match w {
                    Width::Offset24 if dist <= OFFSET24_MAX => { write_offset24(out, base + pos, dist); }
                    Width::Offset32 => { write_u32_be(out, base + pos, dist as u32); }
                    Width::Offset24 => return None,
                }
            }
        }
        Some(self.lists.map(|l| l.and_then(at).unwrap_or(0) as u32))
    }
}

// A COLR from its parts in this order: header, version 1's tables, version 0's layer and base records,
// each copied where the header points; no layers take no offset, and layers the same as the base records share them.
pub(crate) fn assemble(version: u16, v0: Option<(&[u8], u16, &[u8], u16)>, v1: Option<Built>) -> Option<Vec<u8>> {
    let mut out = alloc::vec![0u8; if version == 0 { 14 } else { 34 }];
    write_u16_be(&mut out, 0, version);
    if let Some(v1) = v1 {
        let [base, layers, clips] = v1.write(&mut out)?;
        write_u32_be(&mut out, 14, base);
        write_u32_be(&mut out, 18, layers);
        write_u32_be(&mut out, 22, clips);
    }
    if let Some((base, n, layers, m)) = v0 {
        write_u16_be(&mut out, 2, n);
        write_u16_be(&mut out, 12, m);
        let layer_records = u32::try_from(out.len()).ok()?;
        if !layers.is_empty() {
            write_u32_be(&mut out, 8, layer_records);
            out.extend_from_slice(layers);
        }
        let records = if base == layers { layer_records } else { u32::try_from(out.len()).ok()? };
        write_u32_be(&mut out, 4, records);
        if base != layers {
            out.extend_from_slice(base);
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/colr_v1/write.rs"]
mod tests;
