#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{
    clamp_i16, glyf_tags, pad4, read_i16_be, read_u16_be, read_u32_be, write_i16_be,
};
use super::super::format::ivs::{fixed_coords, round_fixed};
use super::super::format::round::ot_round_f32;
use super::drawn::{fixed_to_int, DeltaRoom, DeltaSource, Gvar};
use super::super::outline::{
    glyf_glyph_box_varied, points_box, Decoded, Deltas, Moves, PointBounds, Shape as Drawn, Vary,
};
use super::super::format::metrics::{shaped_tags, MetricsTable};
use super::coords::{GlyphCoords, extract_coords_into, write_composite_glyph, write_simple_glyph};
use super::super::format::glyf::{
    glyph_record, simple_point_count, Component, ComponentPath, Components, LocaRef, Stack, GLYPH_WORK,
    MAX_VISITS, MAX_POINTS, USE_MY_METRICS,
};
use super::super::subsetter::subset_budget;
use crate::daecore::daetype::{TableBytes, Tables};
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::Error;

type Point = (f32, f32);

// The instance's glyf and loca, and each glyph's advances across and down and its side bearings as its phantom
// points give them, a bearing None where glyf's doesn't change.
pub struct GvarResult {
    pub glyf_data: Vec<u8>,
    pub new_loca:  Vec<usize>,
    pub advances:  Vec<i32>,
    pub lsb_new:   Vec<Option<i32>>,
    pub vadvances: Vec<i32>,
    pub tsb_new:   Vec<Option<i32>>,
}

// gvar's header as a subset checks it: where glyphs' data starts and how its offsets are written. A font without gvar
// has an empty one.
#[derive(Default)]
pub(crate) struct GvarHeader {
    use_words: bool,
    wide: bool,
}

impl GvarHeader {
    // maxp counts the glyphs whatever gvar's header says, so the offsets must span them, and the shared tuples must lie
    // in the table. `wide` for GVAR, whose glyph count and each glyph's data offset take 3 bytes.
    pub(crate) fn read(gvar: &[u8], num_glyphs: usize, wide: bool) -> Result<GvarHeader, Error> {
        let u16_at = |at: usize| if gvar.is_empty() { Some(0) } else { read_u16_be(gvar, at) };
        let u32_at = |at: usize| if gvar.is_empty() { Some(0) } else { read_u32_be(gvar, at) };
        let shift = usize::from(wide);
        let axes               = u16_at(4).ok_or(Error::malformed("gvar: header truncated"))? as usize;
        let shared_tuple_count = u16_at(6).ok_or(Error::malformed("gvar: header truncated"))? as usize;
        let shared_tuples_off  = u32_at(8).ok_or(Error::malformed("gvar: header truncated"))? as usize;
        let flags              = u16_at(14 + shift).ok_or(Error::malformed("gvar: header truncated"))?;
        u32_at(16 + shift).ok_or(Error::malformed("gvar: header truncated"))?;
        if !gvar.is_empty() && read_u16_be(gvar, 0) != Some(1) {
            return Err(Error::unsupported("gvar: unknown major version"));
        }
        let shared_bytes = shared_tuple_count.checked_mul(axes).and_then(|n| n.checked_mul(2));
        let shared_end = shared_bytes.and_then(|n| shared_tuples_off.checked_add(n));
        if shared_tuple_count > 0 && shared_end.is_none_or(|end| end > gvar.len()) {
            return Err(Error::malformed("gvar: shared tuple array does not fit the table"));
        }
        let header = GvarHeader { use_words: flags & 1 != 0, wide };
        header.offset(gvar, num_glyphs).ok_or(Error::malformed("gvar: glyph variation offsets truncated"))?;
        Ok(header)
    }

    fn offset(&self, gvar: &[u8], gid: usize) -> Option<usize> {
        let at = 20 + usize::from(self.wide);
        match (gvar.is_empty(), self.use_words) {
            (true, _) => Some(0),
            (false, true) => read_u32_be(gvar, at + gid * 4).map(|v| v as usize),
            (false, false) => read_u16_be(gvar, at + gid * 2).map(|v| usize::from(v) * 2),
        }
    }
}

// gvar's deltas at a normalized `location` as drawing reads them, phantom points moved by gvar alone; None at the
// default and without gvar, Err where its header refuses every location. `coords` and `scalars` hold what it reads.
pub(crate) fn gvar_source<'a>(
    gvar: Option<&'a Gvar>, location: &[f64], coords: &'a mut Vec<i64>, scalars: &'a mut Vec<i64>,
) -> Option<DeltaSource<'a>> {
    let gvar = gvar?;
    *coords = fixed_coords(location);
    coords.resize(gvar.axes(), 0);
    if coords.iter().all(|&c| c == 0) {
        return None;
    }
    *scalars = gvar.tuple_scalars(coords);
    Some(DeltaSource { coords, gvar: Some(gvar), scalars: Some(scalars), hvar: false, vvar: false })
}

// A glyf font's glyphs varied at a location: gvar's deltas there as drawing reads them, none at the default, glyf
// and loca, and the metrics their phantom points start from.
pub(crate) struct GlyfAt<'a> {
    pub(crate) source: Option<DeltaSource<'a>>,
    pub(crate) glyf: &'a [u8],
    pub(crate) extended: bool,
    pub(crate) loca: LocaRef<'a>,
    pub(crate) horizontal: Option<(&'a [u8], usize)>,
    pub(crate) vertical: Option<(&'a [u8], usize)>,
    pub(crate) upem: i32,
}

impl<'a> GlyfAt<'a> {
    fn glyphs(&self) -> usize {
        self.loca.len().saturating_sub(1)
    }

    fn record(&self, gid: usize) -> Option<&'a [u8]> {
        u32::try_from(gid).ok().and_then(|g| glyph_record(self.glyf, self.loca, g))
    }

    fn glyph(&self, gid: usize) -> &'a [u8] {
        self.record(gid).unwrap_or_default()
    }

    // Each glyph's phantom points as stored: its origin and advance across, and its top and advance down. A glyph glyf
    // doesn't hold, or whose bytes don't read, is no glyph at all, with no metrics of its own.
    fn phantoms(&self, gid: usize, glyph: &[u8]) -> [Point; 4] {
        if self.record(gid).is_none() {
            return [(0.0, 0.0); 4];
        }
        let header = |at: usize| if glyph.len() >= 10 { read_i16_be(glyph, at).map_or(0, i32::from) } else { 0 };
        let pair = |m: Option<(&[u8], usize)>| m.map_or((0, 0), |(mtx, long)| MetricsTable::new(mtx, long).pair(gid));
        let ((advance, lsb), (vadvance, tsb)) = (pair(self.horizontal), pair(self.vertical));
        let (left, top) = (header(2) - i32::from(lsb), header(8) + i32::from(tsb));
        let right = left + i32::from(advance);
        [(left as f32, 0.0), (right as f32, 0.0), (0.0, top as f32), (0.0, (top - i32::from(vadvance)) as f32)]
    }

    // The glyphs varied within `left`, what the call varying them may spend.
    fn variation(&self, left: usize) -> GlyphVariation<'a> {
        let (source, extended) = (self.source, self.extended);
        GlyphVariation { source, extended, buffers: Buffers::default(), room: DeltaRoom::default(), left }
    }

    // A glyph and its components as walks reach them, which share what one glyph's walks may spend with the tuples
    // moving the glyphs.
    fn reached(&self) -> Reached<'_, 'a> {
        Reached { at: self, variation: self.variation(GLYPH_SPEND), own: BTreeMap::new(), points: Vec::new() }
    }

    // The glyph's box before rounding and its phantom points, its components varied as the walk reaches each, and
    // what its walks and tuples spent; past their budget it measures as failing, None past glyf's glyphs.
    fn reach(&self, gid: usize) -> Option<(Placed, usize)> {
        if gid >= self.glyphs() {
            return None;
        }
        let mut reached = self.reached();
        let (placed, spent) = walk_glyph(self, &mut reached, gid, &mut GlyphCoords::default());
        let failed = Placed { bounds: None, phantoms: None, drawn: false };
        Some(match reached.variation.spend(spent) {
            Some(()) => (placed, GLYPH_SPEND - reached.variation.left),
            None => (failed, GLYPH_SPEND),
        })
    }

    // The box of glyph `gid` drawn as drawing draws it, each glyph moved by the whole-unit deltas `kept` gives it;
    // whether it draws, and its spend, a glyph visited costing 16 as a phantom walk's visit does.
    fn draw<K: Points>(
        &self, gid: usize, kept: &mut K, scratch: &mut GlyphCoords, pen: &mut PointBounds,
    ) -> (bool, usize) {
        let Ok(gid) = u32::try_from(gid) else { return (false, 0) };
        let moved = Moved(core::cell::RefCell::new(kept));
        let (drawn, [visits, points]) =
            glyf_glyph_box_varied(self.glyf, self.extended, self.loca, gid, scratch, &moved, pen);
        (drawn.is_ok(), 16 * visits + points)
    }

    // One glyph measured at the location, None past glyf's glyphs.
    pub(crate) fn measure(&self, gid: usize) -> Option<Measured> {
        let (placed, _) = self.reach(gid)?;
        let s = placed.phantoms.map_or(0.0, |phantoms| phantoms[0].0);
        let edge = |v: f32| f64::from(v.round());
        let extents = placed.drawn.then(|| match placed.bounds {
            Some((x0, y0, x1, y1)) => {
                let (x, y) = (edge(x0 - s), edge(y1));
                [x as i32, y as i32, (edge(x1 - s) - x) as i32, (edge(y0) - y) as i32]
            }
            None => [0; 4],
        });
        let upem = self.upem as f32;
        let (advances, top) = match placed.phantoms {
            Some([left, right, top, bottom]) => {
                let advance = |v: f32| ot_round_f32(v).max(0) as u32;
                ([advance(right.0 - left.0), advance(top.1 - bottom.1)], ot_round_f32(top.1 * upem / upem))
            }
            None => ([self.upem as u32 / 2, self.upem as u32], self.upem),
        };
        Some(Measured { extents, advances, top })
    }

    // The glyph and every glyph it's built from, written as an instance holds them in a glyf and loca of their own,
    // every other glyph empty, for its points to be read as TrueType numbers them. None past glyf's glyphs.
    pub(crate) fn instanced(&self, gid: usize) -> Option<(Vec<u8>, Vec<usize>)> {
        if gid >= self.glyphs() {
            return None;
        }
        // Every glyph reached is walked whole, so the walks share what one glyph's may spend.
        let mut reached = self.reached();
        let (mut placed, mut next, mut scratch) = (BTreeMap::new(), alloc::vec![gid], GlyphCoords::default());
        while let Some(g) = next.pop() {
            let Some(own) = reached.own(g).filter(|_| !placed.contains_key(&g)) else { continue };
            let (walked, spent) = walk_glyph(self, &mut reached, g, &mut scratch);
            reached.variation.spend(spent)?;
            placed.insert(g, walked);
            if own.shape == Shape::Composite {
                let components = Components::new(self.glyph(g), self.extended).map_while(Result::ok);
                next.extend(components.map(|c| c.gid as usize));
            }
        }
        let shift = |g: usize| placed.get(&g).and_then(|p: &Placed| p.phantoms).map_or(0.0, |phantoms| phantoms[0].0);
        let (mut glyf, mut loca, mut buffers) = (Vec::new(), Vec::new(), Buffers::default());
        for (&g, &at) in &placed {
            loca.resize(g + 1, glyf.len());
            let Some(own) = reached.own(g) else { continue };
            // A glyph at a time, the flags are read again rather than kept.
            let glyph = self.glyph(g);
            let read = match own.shape {
                Shape::Simple(n) if extract_coords_into(glyph, n, &mut buffers.coords) => buffers.coords.flags.clone(),
                _ => Vec::new(),
            };
            let glyph = (glyph, self.extended);
            write_glyph(glyph, (&own, reached.points(g), &read), at, &shift, &mut buffers, &mut glyf);
            glyf.resize(pad4(glyf.len()), 0);
        }
        loca.push(glyf.len());
        Some((glyf, loca))
    }

    // The box of the glyph's drawn outline at the location, its origin on its left phantom point: None inside for one
    // with no points; None where it fails or past glyf's glyphs. What its walks spent adds to `work`.
    pub(crate) fn bounds(&self, gid: usize, work: &mut usize) -> Option<Option<Bounds>> {
        let (placed, spent) = self.reach(gid)?;
        *work = work.saturating_add(spent);
        let s = placed.phantoms.filter(|_| placed.drawn)?[0].0;
        Some(placed.bounds.map(|(x0, y0, x1, y1)| (x0 - s, y0, x1 - s, y1)))
    }
}

// A glyph as measured: extents (bearings, width, height), all 0 for a glyph without points, None where its outline
// fails; advances both ways; vertical origin, the top phantom point at the em's own scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measured {
    pub(crate) extents: Option<[i32; 4]>,
    pub(crate) advances: [u32; 2],
    pub(crate) top: i32,
}

// The most a glyph's two walks spend, drawing's and the phantom points'.
const GLYPH_SPEND: usize = 16 * MAX_VISITS + 2 * GLYPH_WORK;

// The glyph's box, the box of its outline as drawing draws it, and the phantom points its metrics take; and what the
// two walks spent.
fn walk_glyph<K: Points>(at: &GlyfAt, kept: &mut K, gid: usize, scratch: &mut GlyphCoords) -> (Placed, usize) {
    let mut pen = PointBounds::default();
    let (drawn, drew) = at.draw(gid, kept, scratch, &mut pen);
    let mut walk = Walk::new(at, kept);
    let mut gathered = 0;
    let phantoms = walk.phantoms(gid, &mut gathered);
    let phantoms = phantoms.filter(|_| walk.spend(gathered).is_some());
    (Placed { bounds: pen.finish().filter(|_| drawn), phantoms, drawn }, drew + GLYPH_WORK - walk.work)
}

pub fn apply_gvar(
    table_map:     &Tables,
    glyf_data:     &[u8],
    glyph_offsets: &[usize],
    location:      &[f64],
) -> Result<GvarResult, Error> {
    // A font without gvar varies no glyph, though each is measured by its points at the location like any other.
    let gvar = Gvar::load(table_map).map_err(|()| Error::malformed("gvar: header malformed"))?;
    let (mut coords, mut scalars) = (Vec::new(), Vec::new());
    let metrics = |vertical: bool| {
        let tags = shaped_tags(table_map, vertical);
        table_map.get(tags.mtx).map(TableBytes::as_slice).zip(tags.header(table_map).map(|_| tags.long(table_map)))
    };
    let at = GlyfAt {
        source: gvar_source(gvar.as_ref(), location, &mut coords, &mut scalars),
        glyf: glyf_data,
        extended: glyf_tags(table_map).extended,
        loca: glyph_offsets.into(),
        horizontal: metrics(false),
        vertical: metrics(true),
        upem: table_map.get("head").and_then(|h| read_u16_be(h, 18)).map_or(1000, i32::from),
    };
    let glyphs = at.glyphs();
    // Glyphs sharing a deep composite or gvar data each walk it or work out its tuples again, so walks and tuples
    // share 256 a byte of glyf and a glyph, under 5% of it spent by real fonts, and one glyph draw's work, which a
    // glyph that draws may take whole; past it the outlines stay as stored.
    let over = || Error::limit("gvar: the glyphs' walks and tuples run past the instance's budget");
    let mut state = at.variation(glyf_data.len().saturating_add(glyphs).saturating_mul(256).saturating_add(GLYPH_WORK));

    // Every glyph's own points at the location before any is put together, since a composite takes its components',
    // and a simple glyph's flags as read, which its instance keeps. Glyphs can share their bytes, so the points kept
    // and the records copied are held to glyf's subset budget, a point to a byte, beside four phantom points a glyph.
    let mut kept = Kept { own: Vec::with_capacity(glyphs), points: Vec::new(), flags: Vec::new(), moves: [].into() };
    let (mut points, mut moves): (Vec<Point>, Vec<_>) = (Vec::new(), Vec::new());
    let (budget, mut copied) = (subset_budget(glyf_data.len()), 0usize);
    let room = budget.saturating_add(4 * glyphs);
    for gid in 0..glyphs {
        let glyph = at.glyph(gid);
        copied += glyph.len();
        if copied > budget {
            return Err(Error::limit("gvar: glyf ranges past the subset budget"));
        }
        let mut own = state.own(gid, at.record(gid).is_some(), glyph, at.phantoms(gid, glyph), &mut points, &mut moves);
        if state.left == 0 {
            return Err(over());
        }
        if kept.points.len() + points.len() > room {
            return Err(Error::limit("gvar: the glyphs' points run past the instance's budget"));
        }
        own.at = (kept.points.len(), points.len());
        kept.points.extend_from_slice(&points);
        moves.resize(kept.points.len(), (0, 0));
        let read = if matches!(own.shape, Shape::Simple(_)) && own.drawn { own.count } else { 0 };
        kept.flags.extend_from_slice(&state.buffers.coords.flags[..read]);
        kept.flags.resize(kept.points.len(), 0);
        kept.own.push(own);
    }
    kept.moves = moves.into();

    // Each glyph's box before rounding, as drawn, and the phantom points its metrics take, a USE_MY_METRICS
    // component's for a composite.
    let mut scratch = GlyphCoords::default();
    let mut placed = Vec::with_capacity(glyphs);
    for gid in 0..glyphs {
        let (walked, spent) = walk_glyph(&at, &mut kept, gid, &mut scratch);
        state.spend(spent).ok_or_else(over)?;
        placed.push(walked);
    }

    // Each glyph is written with its origin on its left phantom point, where it's drawn from at the location, so its
    // box and bearing are as drawn; a component's offset makes up for its own glyph's shift and the composite's.
    let shift = |gid: usize| placed.get(gid).and_then(|p| p.phantoms).map_or(0.0, |phantoms| phantoms[0].0);
    let mut new_glyf: Vec<u8> = Vec::with_capacity(glyf_data.len() + glyf_data.len() / 8);
    let mut new_loca: Vec<usize> = vec![0; glyphs + 1];
    let mut metrics = GlyphMetrics::new(glyphs, at.vertical.is_some(), at.upem);
    for gid in 0..glyphs {
        let s = shift(gid);
        let (own, points) = kept.get(gid).ok_or(Error::missing("gvar: a glyph's points are missing"))?;
        let read = &kept.flags[own.at.0..own.at.0 + own.count.min(own.at.1)];
        let glyph = (own, points, read);
        let source = (at.glyph(gid), at.extended);
        let bounds = write_glyph(source, glyph, placed[gid], &shift, &mut state.buffers, &mut new_glyf);
        metrics.set(gid, bounds, placed[gid].phantoms.map(|phantoms| phantoms.map(|(x, y)| (x - s, y))));
        new_loca[gid + 1] = new_glyf.len();
    }
    // Short offsets count 2-byte words, so where they reach the end each glyph is padded to 2 bytes; else unpadded.
    let odd = new_loca.windows(2).filter(|w| (w[1] - w[0]) % 2 == 1).count();
    if new_glyf.len() + odd < 0x1_FFFF {
        let (mut padded, mut start) = (Vec::with_capacity(new_glyf.len() + odd), 0);
        for end in &mut new_loca[1..] {
            padded.extend_from_slice(&new_glyf[start..*end]);
            padded.resize(padded.len() + padded.len() % 2, 0);
            (start, *end) = (*end, padded.len());
        }
        new_glyf = padded;
    }

    let GlyphMetrics { advances, lsb_new, vadvances, tsb_new, .. } = metrics;
    Ok(GvarResult { glyf_data: new_glyf, new_loca, advances, lsb_new, vadvances, tsb_new })
}

type Bounds = (f32, f32, f32, f32);

// Glyph `glyph` (GLYF's where `extended`) written onto `out` from its left phantom point, its box and bearing as drawn
// and a component's offset making up for both; returns the shifted box, None for a failing outline, written empty.
fn write_glyph(
    (glyph, extended): (&[u8], bool), (own, points, read): (&Own, &[Point], &[u8]), placed: Placed,
    shift: &dyn Fn(usize) -> f32, b: &mut Buffers, out: &mut Vec<u8>,
) -> Option<Bounds> {
    let (start, s) = (out.len(), placed.phantoms.map_or(0.0, |phantoms| phantoms[0].0));
    // Drawn, a glyph without points has a box of 0 at its own origin, shifted as a point would be, so its side
    // bearing holds.
    let bounds = placed.drawn.then(|| placed.bounds.unwrap_or_default());
    let bounds = bounds.map(|(x0, y0, x1, y1)| (x0 - s, y0, x1 - s, y1));
    match own.shape {
        _ if !placed.drawn => {}
        Shape::Empty => out.extend_from_slice(empty_glyph_kept(glyph)),
        // Off the default, every simple glyph is written again, packed afresh, without the padding or glyf's cubic
        // bit a stored one can carry.
        Shape::Simple(n_contours) => {
            b.shifted.clear();
            b.shifted.extend(points.iter().map(|&(x, y)| (x - s, y)));
            write_simple_glyph((glyph, extended), n_contours, &b.shifted, read, &mut b.coords, &mut b.flags, out);
        }
        Shape::Composite => {
            b.shifted.clear();
            for (i, c) in Components::new(glyph, extended).map_while(Result::ok).enumerate() {
                let offset = points.get(i).copied().unwrap_or_default();
                b.shifted.push(compensated(&c, offset, s, shift(c.gid as usize)));
            }
            if !own.varied && b.shifted.iter().zip(points).all(|(a, b)| a == b) {
                out.extend_from_slice(glyph);
            } else {
                write_composite_glyph(glyph, extended, &b.shifted, out);
            }
        }
    }
    if let Some(bounds) = bounds.filter(|_| own.shape != Shape::Empty) {
        write_box(out, start, bounds);
    }
    bounds
}

// What a glyph of no contours keeps: its header and instructions, which can move its phantom points, unless they're
// said to run past it.
fn empty_glyph_kept(glyph: &[u8]) -> &[u8] {
    let instructions = read_u16_be(glyph, 10).map_or(0, usize::from);
    glyph.get(..12 + instructions).filter(|_| instructions > 0).unwrap_or_default()
}

// What the walks give a glyph: its box, the phantom points its metrics take, and whether its outline is drawn.
#[derive(Clone, Copy)]
struct Placed {
    bounds: Option<Bounds>,
    phantoms: Option<[Point; 4]>,
    drawn: bool,
}

// A glyph's box in its header at `at`, each edge rounded once from the float box, half away from 0.
fn write_box(glyf: &mut [u8], at: usize, (x_min, y_min, x_max, y_max): Bounds) {
    for (k, v) in [x_min, y_min, x_max, y_max].into_iter().enumerate() {
        write_i16_be(glyf, at + 2 + 2 * k, clamp_i16(v.round() as i32));
    }
}

// Each glyph's metrics from its phantom points and box: the advances between the phantom points, and the bearings
// from the left one and the top one. Without phantom points it advances half an em across and an em down.
struct GlyphMetrics {
    advances: Vec<i32>,
    lsb_new: Vec<Option<i32>>,
    vadvances: Vec<i32>,
    tsb_new: Vec<Option<i32>>,
    vertical: bool,
    upem: i32,
}

impl GlyphMetrics {
    fn new(glyphs: usize, vertical: bool, upem: i32) -> Self {
        GlyphMetrics {
            advances: vec![0; glyphs],
            lsb_new: vec![None; glyphs],
            vadvances: vec![0; glyphs],
            tsb_new: vec![None; glyphs],
            vertical,
            upem,
        }
    }

    fn set(&mut self, gid: usize, bounds: Option<Bounds>, phantoms: Option<[Point; 4]>) {
        // A glyph drawn empty has a box of 0, so its top bearing places its origin.
        let y_max = bounds.map_or(0, |(_, _, _, y_max)| i32::from(clamp_i16(y_max.round() as i32)));
        let Some([left, right, top, bottom]) = phantoms else {
            (self.advances[gid], self.vadvances[gid]) = (self.upem / 2, self.upem);
            self.tsb_new[gid] = self.vertical.then_some(self.upem);
            return;
        };
        self.advances[gid] = ot_round_f32(right.0 - left.0).max(0);
        self.vadvances[gid] = ot_round_f32(top.1 - bottom.1).max(0);
        self.lsb_new[gid] = bounds.map(|(x_min, ..)| (x_min - left.0).round() as i32);
        self.tsb_new[gid] = self.vertical.then(|| ot_round_f32(top.1) - y_max);
    }
}

// A glyph as gvar varies it: a simple glyph of its contours, a composite of its components' offsets, or one with no
// outline, of its phantom points alone.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Simple(usize),
    Composite,
    Empty,
}

// What a glyph's own data gives at the location: its points' place in the arena, its point or component count,
// whether its outline can be drawn and its data read, if moved, and a simple glyph's points as decoded and moved.
#[derive(Clone, Copy)]
struct Own {
    at: (usize, usize),
    shape: Shape,
    count: usize,
    drawn: bool,
    readable: bool,
    varied: bool,
    decoded: Option<Decoded>,
}

// The points each glyph is left with at the location, as a walk reaches it: what its own data gives, None for a glyph
// glyf doesn't hold, then its outline or offsets and its phantom points.
trait Points {
    fn own(&mut self, gid: usize) -> Option<Own>;
    fn points(&self, gid: usize) -> &[Point];
    // Its whole-unit deltas, as drawing moves its points.
    fn deltas(&mut self, gid: usize) -> Deltas;
    // A simple glyph's points as its own data decoded and moved them.
    fn decoded(&mut self, gid: usize) -> Option<Decoded> {
        self.own(gid)?.decoded
    }
}

// Drawing's view of a walk's glyphs: each moved by the deltas its own data gave, worked out once within the walks'
// own budget.
struct Moved<'k, K>(core::cell::RefCell<&'k mut K>);

impl<K: Points> Vary for Moved<'_, K> {
    fn deltas(&self, gid: u32, _: Drawn<'_>, _: &mut usize) -> Result<Deltas, Error> {
        Ok(self.0.borrow_mut().deltas(gid as usize))
    }

    fn known(&self, gid: u32, _: &mut usize) -> Option<Result<Deltas, Error>> {
        Some(Ok(self.0.borrow_mut().deltas(gid as usize)))
    }

    fn decoded(&self, gid: u32) -> Option<Decoded> {
        self.0.borrow_mut().decoded(gid as usize)
    }
}

// Each glyph's own points at the location, outline or offsets then phantom points, all in one arena.
struct Kept {
    own: Vec<Own>,
    points: Vec<Point>,
    // Beside each point, its flags where its glyph is simple and read, and its whole-unit delta.
    flags: Vec<u8>,
    moves: Shared<[(i16, i16)]>,
}

impl Kept {
    fn get(&self, gid: usize) -> Option<(&Own, &[Point])> {
        let own = self.own.get(gid)?;
        Some((own, self.points.get(own.at.0..own.at.0 + own.at.1)?))
    }
}

impl Points for Kept {
    fn own(&mut self, gid: usize) -> Option<Own> {
        self.own.get(gid).copied()
    }

    fn points(&self, gid: usize) -> &[Point] {
        self.get(gid).map_or(&[], |(_, points)| points)
    }

    fn deltas(&mut self, gid: usize) -> Deltas {
        let own = self.own.get(gid).filter(|own| own.varied)?;
        Some(Moves::within(&self.moves, own.at.0..own.at.0 + own.at.1))
    }
}

// One glyph's points and its components', each varied when a walk first reaches it.
struct Reached<'g, 'a> {
    at: &'g GlyfAt<'a>,
    variation: GlyphVariation<'a>,
    own: BTreeMap<usize, (Own, Deltas)>,
    points: Vec<Point>,
}

impl Points for Reached<'_, '_> {
    fn own(&mut self, gid: usize) -> Option<Own> {
        if gid >= self.at.glyphs() {
            return None;
        }
        if let Some(&(own, _)) = self.own.get(&gid) {
            return Some(own);
        }
        let glyph = self.at.glyph(gid);
        let (mut points, mut moves) = (Vec::new(), Vec::new());
        let known = self.at.record(gid).is_some();
        let phantoms = self.at.phantoms(gid, glyph);
        let mut own = self.variation.own(gid, known, glyph, phantoms, &mut points, &mut moves);
        own.at = (self.points.len(), points.len());
        self.points.extend_from_slice(&points);
        self.own.insert(gid, (own, own.varied.then(|| Shared::<[_]>::from(moves).into())));
        Some(own)
    }

    fn points(&self, gid: usize) -> &[Point] {
        let at = self.own.get(&gid).map_or((0, 0), |(own, _)| own.at);
        self.points.get(at.0..at.0 + at.1).unwrap_or_default()
    }

    fn deltas(&mut self, gid: usize) -> Deltas {
        self.own(gid)?;
        self.own.get(&gid).and_then(|(_, deltas)| deltas.clone())
    }
}

// The phantom points a glyph's metrics take from the points each glyph was left with at the location, within the
// work, points and visits a glyph may take.
struct Walk<'w, 'a, K> {
    at: &'w GlyfAt<'a>,
    kept: &'w mut K,
    work: usize,
    edges: usize,
}

impl<'w, 'a, K: Points> Walk<'w, 'a, K> {
    fn new(at: &'w GlyfAt<'a>, kept: &'w mut K) -> Self {
        Walk { at, kept, work: GLYPH_WORK, edges: 0 }
    }

    fn spend(&mut self, work: usize) -> Option<()> {
        self.work = self.work.checked_sub(work)?;
        Some(())
    }

    // A glyph's visit: one more glyph, and its own points.
    fn visit(&mut self, work: usize) -> Option<()> {
        if self.edges >= MAX_VISITS {
            return None;
        }
        self.edges += 1;
        self.spend(16 + work)
    }

    // The phantom points of a glyph the walk has reached.
    fn phantoms_of(&self, gid: usize) -> Option<[Point; 4]> {
        let points = self.kept.points(gid);
        points.get(points.len().checked_sub(4)?..)?.try_into().ok()
    }

    // Glyph `gid`'s phantom points alone, through USE_MY_METRICS components, every component read spending work.
    // `gathered` counts points drawing holds: a simple glyph's unread, a component's dropped once placed or skipped.
    // Composites are walked on the heap: each one's components left, its phantoms so far, and what was gathered
    // before the component being read, two in place, as setting up eight for every glyph cost instancing Inter 0.7%.
    fn phantoms(&mut self, gid: usize, gathered: &mut usize) -> Option<[Point; 4]> {
        let (mut inside, mut path) = (Stack::<(Components, [Point; 4], usize), 2>::default(), ComponentPath::default());
        let mut gid = gid;
        loop {
            let glyph = self.at.glyph(gid);
            let mut read = None;
            match self.kept.own(gid) {
                None => {
                    self.visit(0)?;
                    *gathered += 4;
                    read = Some(self.at.phantoms(gid, glyph));
                }
                Some(own) => {
                    let simple = if let Shape::Simple(_) = own.shape { own.count } else { 0 };
                    if !own.readable {
                        return None;
                    }
                    self.visit(simple)?;
                    *gathered += simple;
                    if *gathered + 4 > MAX_POINTS {
                        return None;
                    }
                    let phantoms = self.phantoms_of(gid)?;
                    if own.shape == Shape::Composite {
                        path.enter(gid as u32);
                        inside.push((Components::new(glyph, self.at.extended), phantoms, 0));
                    } else {
                        *gathered += 4;
                        read = Some(phantoms);
                    }
                }
            }
            // The next USE_MY_METRICS component to read, each glyph read whole handing its phantoms to its composite.
            gid = loop {
                if let Some(phantoms) = read.take() {
                    let Some((_, held, before)) = inside.last_mut() else { return Some(phantoms) };
                    *held = phantoms;
                    *gathered -= 4;
                    let more = *gathered - *before;
                    self.spend(more)?;
                    if *gathered + 4 > MAX_POINTS {
                        return None;
                    }
                }
                let (components, _, before) = inside.last_mut()?;
                let mut down = None;
                while let Some(Ok(c)) = components.next() {
                    self.spend(1)?;
                    if path.holds(c.gid) {
                        continue;
                    }
                    if c.flags & USE_MY_METRICS != 0 {
                        (*before, down) = (*gathered, Some(c.gid as usize));
                        break;
                    }
                    if *gathered + 4 > MAX_POINTS {
                        return None;
                    }
                }
                if let Some(down) = down {
                    break down;
                }
                let (_, phantoms, _) = inside.pop()?;
                path.leave();
                *gathered += 4;
                read = Some(phantoms);
            };
        }
    }
}

// A component's offset as written: moved for the composite's shift `s` and for its own glyph's, `component`, which
// the component's outline was written with, through its matrix as its flags place the offset.
fn compensated(c: &Component, offset: Point, s: f32, component: f32) -> Point {
    let m = c.matrix.map(|v| f32::from(v) / 16384.0);
    if !c.offset_through_matrix() {
        return (offset.0 + m[0] * component - s, offset.1 + m[1] * component);
    }
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 {
        return (offset.0 + component, offset.1);
    }
    (offset.0 + component - m[3] * s / det, offset.1 + m[1] * s / det)
}

#[derive(Default)]
struct Buffers {
    shifted: Vec<Point>,
    coords: GlyphCoords,
    flags: Vec<u8>,
}

// The tuples moving the glyphs and the walks placing them share `left`, spent once it reaches 0.
struct GlyphVariation<'a> {
    source: Option<DeltaSource<'a>>,
    extended: bool,
    buffers: Buffers,
    room: DeltaRoom,
    left: usize,
}

impl GlyphVariation<'_> {
    fn spend(&mut self, n: usize) -> Option<()> {
        self.left = self.left.checked_sub(n).filter(|&left| left > 0)?;
        Some(())
    }

    // Glyph `gid`'s points at the location onto `points`, offsets then phantoms, each moved by its 16.16 delta rounded
    // once, onto `moves` that delta in 16 bits; unread points are 0, and failed data or no glyf record stays as stored.
    fn own(
        &mut self, gid: usize, known: bool, glyph: &[u8], phantoms: [Point; 4], points: &mut Vec<Point>,
        moves: &mut Vec<(i16, i16)>,
    ) -> Own {
        let start = moves.len();
        let mut own = self.own_points(gid, known, glyph, phantoms, points, moves);
        if matches!(own.shape, Shape::Simple(_)) && own.drawn {
            let coords = &self.buffers.coords;
            let moved = own.varied.then(|| &moves[start..]);
            own.decoded = Some(Decoded { points: coords.num_points, bounds: points_box(coords, moved) });
        }
        own
    }

    fn own_points(
        &mut self, gid: usize, known: bool, glyph: &[u8], phantoms: [Point; 4], points: &mut Vec<Point>,
        moves: &mut Vec<(i16, i16)>,
    ) -> Own {
        points.clear();
        let n_contours = if glyph.len() < 10 { 0 } else { read_i16_be(glyph, 0).unwrap_or(0) };
        let mut own = Own {
            at: (0, 0), shape: Shape::Empty, count: 0, drawn: true, readable: true, varied: false, decoded: None,
        };
        let cc = &mut self.buffers.coords;
        let mut contours = false;
        match n_contours {
            1.. => {
                let n = n_contours as usize;
                own.shape = Shape::Simple(n);
                let Some(count) = simple_point_count(glyph, n) else {
                    (own.drawn, own.readable) = (false, false);
                    points.extend(phantoms);
                    return own;
                };
                own.count = count;
                own.drawn = extract_coords_into(glyph, n, cc);
                if own.drawn {
                    points.extend(cc.x_coords.iter().zip(&cc.y_coords).map(|(&x, &y)| (x as f32, y as f32)));
                    contours = true;
                } else {
                    points.resize(count, (0.0, 0.0));
                }
            }
            ..=-1 => {
                let offset = |c: Component| {
                    if c.has_offset() { (c.args.0 as f32, c.args.1 as f32) } else { (0.0, 0.0) }
                };
                points.extend(Components::new(glyph, self.extended).map_while(Result::ok).map(offset));
                (own.shape, own.count) = (Shape::Composite, points.len());
            }
            0 => {}
        }
        points.extend(phantoms);
        let Some(source) = self.source.filter(|_| known) else { return own };
        let (x, y, ends) = match contours {
            true => (&cc.x_coords[..], &cc.y_coords[..], &cc.end_pts[..]),
            false => (&[][..], &[][..], &[][..]),
        };
        // Past the budget the glyph stays as stored, and whoever varies it fails once `left` is spent.
        let deltas = source.deltas(gid, points.len(), (x, y), ends, &mut self.room, &mut self.left);
        let Ok(Some(deltas)) = deltas else { return own };
        own.varied = true;
        for (p, &(dx, dy)) in points.iter_mut().zip(deltas) {
            *p = (p.0 + round_fixed(dx) as f32, p.1 + round_fixed(dy) as f32);
        }
        moves.extend(deltas.iter().map(|&(x, y)| (fixed_to_int(x), fixed_to_int(y))));
        own
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/gvar.rs"]
mod tests;
