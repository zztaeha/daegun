use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use super::super::decoder::{
    read_offset24, read_u16_be, read_u32_be, read_uint, search_records, write_u16_be, write_u32_be,
};
use super::super::colr_v0::{base_glyph_layers, colr_v0_base_glyphs, colr_v0_header, layer_run, CpalV1};
use super::super::colr_v1::{self, paint_layout};
use super::super::colr_v1::write::{assemble, layer_range, rebuild};
use super::remap_gid16;
use super::subset_budget;

// What COLR's stage keeps: the glyphs then in the set and in the font, whose color drawing can show; each palette
// index the kept paints use as its rank (0xFFFF to itself), so CPAL keeps only those colors; the LayerList indices.
pub(crate) struct ColrKept {
    pub(crate) glyphs: GlyphSet,
    pub(crate) palettes: BTreeMap<u16, u16>,
    pub(crate) layers: Vec<(u64, u64)>,
}

// Nothing kept, as for a font without COLR: its glyph set takes no room.
impl Default for ColrKept {
    fn default() -> ColrKept {
        ColrKept { glyphs: GlyphSet::empty(), palettes: BTreeMap::new(), layers: Vec::new() }
    }
}

// The glyphs COLR adds to `glyphs`: version 0's layers of each, then every paint the base glyphs reach. And what the
// subset keeps by them: those paints' palette indices and each held record's layers.
pub(crate) fn colr_closure(colr: &[u8], glyphs: &GlyphSet, num_glyphs: u32) -> (GlyphSet, ColrKept) {
    let (mut found, mut palettes, mut layers) = (GlyphSet::new(), BTreeSet::new(), Vec::new());
    found.extend(colr_v0_layers(colr, glyphs).map(|&[g0, g1, ..]| u32::from(u16::from_be_bytes([g0, g1]))));
    let mut based = found.clone();
    based.union(glyphs);
    colr_v1_closure(colr, &based, &mut found, &mut palettes, &mut layers);
    based.union(&found);
    // Each kept glyph's version 0 layers, as drawing reads them, give their palette indices.
    palettes.extend(colr_v0_layers(colr, &based).map(|&[_, _, p0, p1]| u16::from_be_bytes([p0, p1])));
    let mut rank = 0;
    let palettes = palettes.into_iter().map(|i| (i, if i == 0xFFFF { i } else { rank += 1; rank - 1 })).collect();
    let glyphs = based.iter().filter(|&g| g < num_glyphs).collect();
    (found, ColrKept { glyphs, palettes, layers })
}

// The layers of each record the search finds for a glyph, as drawing reads them: a run past the layers has none.
// Base glyphs may share layers, so their ranges are merged and each layer is read once.
fn colr_v0_layers<'c>(colr: &'c [u8], glyphs: &GlyphSet) -> impl Iterator<Item = &'c [u8; 4]> {
    let (n_base, base_off, layers_off, _) = colr_v0_header(colr).unwrap_or_default();
    let layers = colr.get(layers_off..).map(|l| l.as_chunks::<4>().0).unwrap_or_default();
    let gid_at = |i: usize| read_u16_be(colr, base_off.checked_add(i * 6)?).map(u32::from);
    let mut ranges: Vec<(usize, usize)> = colr_v0_base_glyphs(colr)
        .into_iter()
        .enumerate()
        .filter(|&(i, (gid, ..))| {
            glyphs.contains(&gid) && search_records(n_base, gid.into(), gid_at).and_then(Result::ok) == Some(i)
        })
        .filter(|&(_, (_, first, n))| layer_run(colr, first, n).is_some())
        .map(|(_, (_, first, n))| (first, first + n))
        .collect();
    ranges.sort_unstable();
    let mut next = 0;
    ranges.into_iter().flat_map(move |(start, end)| {
        let from = start.max(next);
        next = next.max(end);
        &layers[from..end.max(from)]
    })
}

// Every paint the base glyphs reach, each read again only with more depth left, none past `MAX_PAINT_DEPTH`, the
// depth drawing reads to. A glyph a paint names keeps its own record in the subset, so its paint is walked too.
fn colr_v1_closure(
    colr: &[u8], glyphs: &GlyphSet, found: &mut GlyphSet, palettes: &mut BTreeSet<u16>, layers: &mut Vec<(u64, u64)>,
) {
    let Some(header) = colr_v1::header(colr) else { return };
    let Some(list) = header.base_glyph_list else { return };
    let left_at = BTreeMap::new();
    let fresh = Vec::new();
    let mut walk = PaintWalk { colr, list, layer_list: header.layer_list, left_at, found, fresh, palettes, layers };
    // Each glyph's root as drawing finds it, so a record the search misses keeps nothing.
    for gid in glyphs.iter() {
        if let Some(root) = colr_v1::lookup_base_glyph_paint_offset(colr, list, gid) {
            walk.paint(root, colr_v1::MAX_PAINT_DEPTH);
        }
    }
    while let Some(gid) = walk.fresh.pop() {
        if !glyphs.contains(&gid) && let Some(root) = colr_v1::lookup_base_glyph_paint_offset(colr, list, gid) {
            walk.paint(root, colr_v1::MAX_PAINT_DEPTH);
        }
    }
}

struct PaintWalk<'a, 'b> {
    colr: &'a [u8],
    list: usize,
    layer_list: Option<usize>,
    // The most depth left each paint was walked with: one shared at several depths is walked again from a shallower
    // place, so it keeps all that place reaches.
    left_at: BTreeMap<usize, usize>,
    found: &'b mut GlyphSet,
    // Each glyph `found` gained, whose own paint is walked next.
    fresh: Vec<u32>,
    palettes: &'b mut BTreeSet<u16>,
    layers: &'b mut Vec<(u64, u64)>,
}

impl PaintWalk<'_, '_> {
    // A paint's glyphs, palette indices and layers, then its children in order. A PaintColrGlyph names its glyph
    // only where it has a base record, whose paint is walked next from its own root, as a subset writes it there.
    fn paint(&mut self, at: usize, left: usize) {
        if left == 0 || self.left_at.get(&at).is_some_and(|&l| l >= left) { return; }
        self.left_at.insert(at, left);
        let colr = self.colr;
        let Some(layout) = colr.get(at).and_then(|&f| paint_layout(f)) else { return };
        let child = |rel: Option<usize>| rel.filter(|&rel| rel != 0).and_then(|rel| at.checked_add(rel));
        match colr[at] {
            1 => {
                let (Some(&n), Some(first)) = (colr.get(at + 1), read_u32_be(colr, at + 2)) else { return };
                self.layers.extend(layer_range(first, n));
                let Some(layers) = self.layer_list else { return };
                let total = read_u32_be(colr, layers).unwrap_or(0) as usize;
                // A layer past the table's end reads nothing, and past a quarter of a 32-bit usize its place overflows.
                let end = (first as usize).saturating_add(usize::from(n)).min(total).min(colr.len() / 4);
                for i in first as usize..end {
                    let rel = read_u32_be(colr, layers + 4 + i * 4).filter(|&rel| rel != 0);
                    if let Some(paint) = rel.and_then(|rel| layers.checked_add(rel as usize)) {
                        self.paint(paint, left - 1);
                    }
                }
            }
            11 => {
                let Some(gid) = read_u16_be(colr, at + 1).map(u32::from) else { return };
                if colr_v1::lookup_base_glyph_paint_offset(colr, self.list, gid).is_some() && self.found.insert(gid) {
                    self.fresh.push(gid);
                }
            }
            _ => {
                if let Some(gid) = layout.glyph.and_then(|(pos, w)| read_uint(colr, at + pos, w))
                    && self.found.insert(gid)
                {
                    self.fresh.push(gid);
                }
                match colr[at] {
                    2 | 3 => self.palettes.extend(read_u16_be(colr, at + 1)),
                    format @ 4..=9 => {
                        let line = read_offset24(colr, at + 1).and_then(|rel| at.checked_add(rel));
                        let stride = if format % 2 == 1 { 10 } else { 6 };
                        if let Some(line) = line {
                            let stops = read_u16_be(colr, line + 1).map_or(0, usize::from);
                            let palette = |i: usize| read_u16_be(colr, line + 3 + i * stride + 2);
                            self.palettes.extend((0..stops).filter_map(palette));
                        }
                    }
                    _ => {}
                }
                for &pos in layout.children {
                    if let Some(paint) = child(read_offset24(colr, at + pos)) {
                        self.paint(paint, left - 1);
                    }
                }
            }
        }
    }
}

// COLR for the kept base glyphs under new IDs, indices remapped; version 0, the smaller, where the source is or no kept
// glyph has a paint, its 14-byte header alone for none kept. None where version 0 keeps no record or loses a layer.
pub(crate) fn subset_colr(colr: &[u8], kept: &ColrKept, active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    let version = read_u16_be(colr, 0)?;
    let v0 = v0_written(colr, kept, active, gid_map)?;
    if version == 0 && v0.is_none() {
        return None;
    }
    let v0 = v0.as_ref().map(|(base, n, layers, m)| (base.as_slice(), *n, layers.as_slice(), *m));
    let list = colr_v1::header(colr).and_then(|h| h.base_glyph_list);
    let painted = list.is_some_and(|list| {
        kept.glyphs.iter().any(|g| colr_v1::lookup_base_glyph_paint_offset(colr, list, g).is_some())
    });
    if version == 0 || !painted {
        return assemble(0, v0, None);
    }
    let keep = |g: u16| kept.glyphs.contains(&g).then(|| remap_gid16(active, gid_map, g)).flatten();
    let remap = |g: u32| gid_map.get(g as usize).copied().unwrap_or(g);
    let v1 = rebuild(colr, &keep, &remap, Some(kept), None, subset_budget(colr.len()))?;
    assemble(version, v0, Some(v1))
}

type V0Records = (Vec<u8>, u16, Vec<u8>, u16);

// Kept base glyphs' version 0 records, each with its own copy of its layers (simplest to write); a record drawing reads
// no layers of, or whose layer is gone, is left out. Some(None) for none; None past 16-bit layers or an unplaced index.
fn v0_written(colr: &[u8], kept: &ColrKept, active: &GlyphSet, gid_map: &[u32]) -> Option<Option<V0Records>> {
    if colr_v0_header(colr).is_none() {
        return Some(None);
    }
    let (mut base, mut layers, mut total, mut count) = (Vec::new(), Vec::new(), 0u16, 0u16);
    for gid in kept.glyphs.iter() {
        let (Some(new), Some(run)) = (remap_gid16(active, gid_map, gid as u16), base_glyph_layers(colr, gid)) else {
            continue;
        };
        let glyphs: Option<Vec<u16>> =
            run.iter().map(|&[g0, g1, ..]| remap_gid16(active, gid_map, u16::from_be_bytes([g0, g1]))).collect();
        let Some(glyphs) = glyphs else { continue };
        for (glyph, &[.., p0, p1]) in glyphs.into_iter().zip(run) {
            let palette = *kept.palettes.get(&u16::from_be_bytes([p0, p1]))?;
            layers.extend([glyph, palette].map(u16::to_be_bytes).concat());
        }
        let n = run.len() as u16;
        base.extend([new, total, n].map(u16::to_be_bytes).concat());
        (total, count) = (total.checked_add(n)?, count.checked_add(1)?);
    }
    Some((count > 0).then_some((base, count, layers, total)))
}

// CPAL with every palette, each the colors `kept` keeps in index order, palettes left alike sharing records, version
// 1's types and labels carried. None where no color is kept or the records pass its count.
pub(crate) fn subset_cpal(cpal: &[u8], kept: &ColrKept) -> Option<Vec<u8>> {
    let [version, n_colors, n_palettes, n_records] = [0, 2, 4, 6].map(|at| read_u16_be(cpal, at));
    let (version, n_colors, n_palettes, n_records) = (version?, n_colors?, usize::from(n_palettes?), n_records?);
    let records = read_u32_be(cpal, 8)? as usize;
    // An index past the palette's entries has no color, and past the subset's entries it has none still.
    let colors: Vec<usize> = kept.palettes.keys().map(|&i| usize::from(i)).filter(|&i| i < n_colors.into()).collect();
    if n_palettes == 0 || colors.is_empty() {
        return None;
    }
    let record = |i: usize| (i < usize::from(n_records)).then(|| cpal.get(records.checked_add(4 * i)?..)?.get(..4));
    // A palette whose entries pass the color records reads as palette 0, as drawing selects it, and is written so.
    let first_of = |p: usize| {
        let first = usize::from(read_u16_be(cpal, 12 + 2 * p)?);
        let held = first + usize::from(n_colors) <= usize::from(n_records);
        if held || p == 0 { Some(first) } else { read_u16_be(cpal, 12).map(usize::from) }
    };
    let (mut blocks, mut firsts) = (BTreeMap::<Vec<u8>, usize>::new(), BTreeMap::new());
    let mut written = Vec::new();
    let mut out = [version, colors.len() as u16, n_palettes as u16, 0].map(u16::to_be_bytes).concat();
    out.extend([0; 4]);
    for p in 0..n_palettes {
        let first = first_of(p)?;
        let block = match firsts.get(&first) {
            Some(&block) => block,
            None => {
                let kept_colors = colors.iter().map(|&c| record(first + c).flatten()).collect::<Option<Vec<_>>>()?;
                let kept_colors = kept_colors.concat();
                let next = blocks.len();
                let block = *blocks.entry(kept_colors).or_insert_with_key(|kept_colors| {
                    written.extend_from_slice(kept_colors);
                    next
                });
                if blocks.len() * colors.len() > 0xFFFF {
                    return None;
                }
                firsts.insert(first, block);
                block
            }
        };
        out.extend(((block * colors.len()) as u16).to_be_bytes());
    }
    write_u16_be(&mut out, 6, (blocks.len() * colors.len()) as u16);
    let v1 = (version == 1).then(|| CpalV1::read(cpal)).flatten();
    let tail = out.len();
    // The color records and version 1's arrays, as (field, bytes), written in turn after the header; one equal to an
    // earlier one shares its bytes, the smaller table.
    let mut packed = alloc::vec![(8, written)];
    if let Some(v1) = &v1 {
        out.extend([0; 12]);
        let array = |at: usize, len: usize| (at != 0).then(|| cpal.get(at..at + len).map(<[u8]>::to_vec)).flatten();
        packed.extend(array(v1.types, 4 * n_palettes).map(|t| (tail, t)));
        packed.extend(array(v1.labels, 2 * n_palettes).map(|t| (tail + 4, t)));
        if v1.entry_labels != 0 {
            let label = |i: usize| cpal.get(v1.entry_labels + 2 * i..v1.entry_labels + 2 * i + 2);
            let labels = colors.iter().filter(|&&i| i < usize::from(n_colors)).map(|&i| label(i));
            let labels = labels.collect::<Option<Vec<_>>>()?.concat();
            packed.extend((!labels.is_empty()).then_some((tail + 8, labels)));
        }
    }
    let mut placed: Vec<(&[u8], usize)> = Vec::with_capacity(packed.len());
    for (field, bytes) in &packed {
        let at = match placed.iter().find(|(placed, _)| placed == bytes) {
            Some(&(_, at)) => at,
            None => {
                placed.push((bytes, out.len()));
                out.extend_from_slice(bytes);
                out.len() - bytes.len()
            }
        };
        write_u32_be(&mut out, *field, at as u32);
    }
    Some(out)
}

// The name IDs a subset keeps for CPAL, whether or not it writes the table: a version 1 table's palette labels, all
// of them, and the labels of its colors `kept` keeps.
pub(crate) fn cpal_names(cpal: &[u8], kept: &ColrKept) -> BTreeSet<u16> {
    let Some(v1) = CpalV1::read(cpal).filter(|_| read_u16_be(cpal, 0) == Some(1)) else { return BTreeSet::new() };
    let count = |at: usize| read_u16_be(cpal, at).map_or(0, usize::from);
    let label = |array: usize, i: usize| (array != 0).then(|| read_u16_be(cpal, array + 2 * i)).flatten();
    let palettes = (0..count(4)).filter_map(|i| label(v1.labels, i));
    let kept_colors = (0..count(2)).filter(|&i| kept.palettes.contains_key(&(i as u16)));
    let colors = kept_colors.filter_map(|i| label(v1.entry_labels, i));
    palettes.chain(colors).collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/colr.rs"]
mod tests;
