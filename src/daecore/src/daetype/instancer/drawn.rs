use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use crate::daecore::daetype::instancer::AxisTag;
use super::super::decoder::{glyf_tags, read_i16_be, read_u16_be, read_u24_be, read_u32_be};
use super::super::format::ivs::{fixed_coords, round_fixed, tuple_scalar, DeltaView, StoreView};
use super::super::format::metrics::drawn_metrics;
use crate::daecore::daetype::{TableBytes, Tables};
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::Error;

// A location as drawing reads it, apart from shaping and measuring: coordinates kept in 16.16 with no
// 2.14 step, and every product and quotient rounded in fixed point. Data past a table's end counts as missing.

pub(crate) const ONE: i64 = 0x10000;

// 16.16 products and quotients, in 128 bits where 64 don't hold them, so none wraps: halves rounded away from zero,
// results held to i64's range. A zero divisor gives 16.16's largest value with the dividend's sign; 0 / 0 gives 0.
pub(crate) fn mul_fix(a: i64, b: i64) -> i64 {
    if let Some(p) = a.checked_mul(b)
        && let Some(m) = p.unsigned_abs().checked_add(0x8000)
    {
        return (m >> 16) as i64 * p.signum();
    }
    let p = i128::from(a) * i128::from(b);
    let m = ((p.unsigned_abs() + 0x8000) >> 16) as i128;
    held(if p < 0 { -m } else { m })
}

pub(crate) fn div_fix(a: i64, b: i64) -> i64 {
    if b == 0 {
        return 0x7FFF_FFFF * a.signum();
    }
    let (n, d) = ((i128::from(a) << 16).unsigned_abs(), b.unsigned_abs() as u128);
    let q = ((n + d / 2) / d) as i128;
    held(if (a < 0) != (b < 0) { -q } else { q })
}

fn held(v: i128) -> i64 {
    v.clamp(i64::MIN.into(), i64::MAX.into()) as i64
}

// A 16.16 delta rounded half up to a whole unit, cut to 16 bits, and to 26.6 with its whole part cut alike, so a
// hinted point starts where the unhinted outline puts it.
pub(crate) fn fixed_to_int(x: i64) -> i16 {
    (x.wrapping_add(0x8000) >> 16) as i16
}

pub(crate) fn fixed_to_f26dot6(x: i64) -> i32 {
    let cut = i64::from(x.wrapping_add(0x8000) as i32) - 0x8000;
    ((cut + 0x200) >> 10) as i32
}

// fvar's axis count as drawing accepts it: version 1.0, 20-byte axes, one to 16,382 of them, instances of either size,
// all of it in the table.
fn fvar_axes(fvar: &[u8]) -> Option<usize> {
    let field = |at: usize| read_u16_be(fvar, at).map(usize::from);
    let (offset, count, instances, instance_size) = (field(4)?, field(8)?, field(12)?, field(14)?);
    let sized = instance_size == 4 + 4 * count || instance_size == 6 + 4 * count;
    if fvar.len() < 20 || read_u32_be(fvar, 0)? != 0x10000 || field(10)? != 20 || !(1..=0x3FFE).contains(&count)
        || !sized || instances > 0x7EFF || offset + 20 * count + instance_size * instances > fvar.len()
    {
        return None;
    }
    Some(count)
}

// The location's coordinates in 16.16, as the instancer normalizes them (`axis.rs`). None for a font without axes.
pub(crate) fn coordinates(table_map: &Tables, key: &[(AxisTag, f64)]) -> Option<Vec<i64>> {
    let location = super::Normalizer::read(table_map).ok()?.location(key);
    (!location.is_empty()).then(|| fixed_coords(&location))
}

// HVAR or VVAR, used where its store reads: its advances then come from it, and phantom points don't move.
pub(crate) struct Advances {
    deltas: Shared<DeltaView>,
    scalars: Vec<i64>,
}

impl Advances {
    // The table's store and index map, the same at every location.
    fn deltas(table: &TableBytes) -> Option<Shared<DeltaView>> {
        let t = table.as_slice();
        if read_u16_be(t, 0)? != 1 {
            return None;
        }
        let (store_at, map_at) = (read_u32_be(t, 4)? as usize, read_u32_be(t, 8)? as usize);
        DeltaView::read(table, store_at, (map_at != 0).then_some(map_at)).map(Shared::new)
    }

    fn at(deltas: Shared<DeltaView>, coords: &[i64]) -> Advances {
        let scalars = deltas.scalars_at(coords);
        Advances { deltas, scalars }
    }

    // The advance varied, wrapped to 16 bits as hmtx holds it.
    pub(crate) fn adjust(&self, gid: usize, advance: u16) -> u16 {
        let delta = round_fixed(self.deltas.delta(gid, (0, gid), &self.scalars));
        i32::from(advance).wrapping_add(delta) as u16
    }

    // Its scalars: the store and map are the font's.
    pub(crate) fn heap_bytes(&self) -> usize {
        crate::daecore::daekit::base::sync::heap_of(&self.scalars)
    }
}

// HVAR's and VVAR's stores and index maps, loaded once a font: each of Inter's locations took 3.7 µs more to load and
// check them again, and to drop them.
pub(crate) struct DrawnAdvances {
    hvar: Option<Shared<DeltaView>>,
    vvar: Option<Shared<DeltaView>>,
}

impl DrawnAdvances {
    // HVAR's, and VVAR's where the font has vertical metrics.
    pub(crate) fn load(table_map: &Tables) -> DrawnAdvances {
        let vertical = drawn_metrics(table_map, true).is_some_and(|(_, vmtx, long)| vmtx.is_some() && long.is_some());
        let load = |tag: &str| table_map.get(tag).and_then(Advances::deltas);
        DrawnAdvances { hvar: load("HVAR"), vvar: vertical.then(|| load("VVAR")).flatten() }
    }
}

// A CFF2 variation store as drawing loads it, within its length (CFF2, "VariationStore"), none at a length of 0: its
// regions and each data set's region indices, read as every store is; the deltas live in the charstrings.
pub(crate) struct Cff2Store {
    store: StoreView,
}

impl Cff2Store {
    pub(crate) fn load(cff2: &[u8], at: usize) -> Option<Cff2Store> {
        let size = usize::from(read_u16_be(cff2, at)?);
        let bytes = cff2.get(at.checked_add(2)?..)?.get(..size).filter(|store| !store.is_empty())?;
        Some(Cff2Store { store: StoreView::read(TableBytes::from(bytes.to_vec()), 0)? })
    }

    pub(crate) fn axes(&self) -> usize {
        self.store.axis_count()
    }

    // How many weights a vsindex blends by, the default master's among them; None for no set.
    pub(crate) fn weight_count(&self, vsindex: u32) -> Option<usize> {
        self.store.data_regions(vsindex as usize).map(|regions| regions.len() + 1)
    }

    // What weighing a vsindex takes: each region's scalar reads every axis, where `coords` are weighed at all.
    pub(crate) fn work(&self, vsindex: u32, coords: Option<&[i64]>) -> usize {
        let per = if coords.is_some_and(|c| c.len() == self.axes()) { self.axes().max(1) } else { 1 };
        self.store.data_regions(vsindex as usize).map_or(0, |regions| regions.len().saturating_mul(per))
    }

    // A vsindex's blend weights: the default master's 1, then each region's scalar at `coords`, 0 for a region the
    // store doesn't hold and all where its region list is absent or spans other axes than the font's. Err for no set.
    pub(crate) fn weights(&self, vsindex: u32, coords: Option<&[i64]>) -> Result<Vec<i32>, Error> {
        let coords = coords.filter(|c| c.len() == self.axes());
        let regions = self.store.data_regions(vsindex as usize);
        let regions = regions.ok_or(Error::malformed("CFF2: vsindex past the variation store"))?;
        let weight = |r: usize| coords.map_or(0, |coords| self.store.region_scalar(coords, r) as i32);
        Ok(core::iter::once(ONE as i32).chain(regions.map(weight)).collect())
    }
}

// Glyph `i`'s offset into gvar's data, from the array at `at`.
fn offset(t: &[u8], at: usize, long: bool, i: usize) -> u64 {
    match long {
        true => u64::from(read_u32_be(t, at + 4 * i).unwrap_or(0)),
        false => 2 * u64::from(read_u16_be(t, at + 2 * i).unwrap_or(0)),
    }
}

// gvar as drawing loads it, once a font: where its glyph data offsets lie, and the shared tuples, of which the first
// 4,096 can be named.
pub(crate) struct Gvar {
    data: TableBytes,
    extended: bool,
    axes: usize,
    offsets: (usize, bool),
    data_at: usize,
    glyphs: usize,
    shared: Vec<i64>,
    tuple_count: usize,
}

impl Gvar {
    // Ok(None) without gvar, where glyphs draw as stored; Err where a malformed header refuses every location. A
    // font without fvar takes gvar's own axis count, as VARC components can still set its coordinates.
    pub(crate) fn load(table_map: &Tables) -> Result<Option<Gvar>, ()> {
        // Beside GLYF, GVAR alone; else gvar, else GVAR.
        let gvar = table_map.get("gvar").filter(|_| !glyf_tags(table_map).extended);
        let (data, extended) = match (gvar, table_map.get("GVAR")) {
            (Some(g), _) => (g.clone(), false),
            (None, Some(g)) => (g.clone(), true),
            (None, None) => return Ok(None),
        };
        let t = data.as_slice();
        let fvar = table_map.get("fvar").and_then(|f| fvar_axes(f));
        let Some(axes) = fvar.or_else(|| read_u16_be(t, 4).map(usize::from)) else { return Err(()) };
        let header = if extended { 21 } else { 20 };
        if t.len() < header || read_u32_be(t, 0) != Some(0x10000) || read_u16_be(t, 4) != Some(axes as u16) {
            return Err(());
        }
        let field = |at: usize| read_u16_be(t, at).map(usize::from).ok_or(());
        let (coords_count, coords_at) = (field(6)?, read_u32_be(t, 8).ok_or(())? as usize);
        let glyphs = if extended { read_u24_be(t, 12).ok_or(())? as usize } else { field(12)? };
        let flags = field(header - 6)?;
        let data_at = read_u32_be(t, header - 4).ok_or(())? as usize;
        let long = flags & 1 != 0;
        let unit = if long { 4 } else { 2 };
        if coords_count * axes > t.len() / 2 || (glyphs + 1) * unit > t.len() - header {
            return Err(());
        }
        let shared = match coords_count {
            0 => &[][..],
            _ => t.get(coords_at..).and_then(|shared| shared.get(..2 * coords_count * axes)).ok_or(())?,
        };
        let shared = shared.as_chunks().0.iter().map(|&c| i64::from(i16::from_be_bytes(c)) * 4).collect();
        let offsets = (header, long);
        // Without axes a shared tuple holds no coordinate, so none is counted, as there'd be no scalar for it.
        let tuple_count = if axes == 0 { 0 } else { coords_count.min(0x1000) };
        Ok(Some(Gvar { data, extended, axes, offsets, data_at, glyphs, shared, tuple_count }))
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        crate::daecore::daekit::base::sync::heap_of(&self.shared)
    }

    // Glyph `gid`'s data, its start and end in the table; None past the glyphs gvar counts, and where the next offset
    // runs back or past the table, as that glyph's data is then malformed.
    fn range(&self, gid: usize) -> Option<(usize, usize)> {
        let (t, (at, long)) = (self.data.as_slice(), self.offsets);
        let place = |i| self.data_at as u64 + offset(t, at, long, i);
        let (start, end) = (place(gid), place(gid + 1));
        (gid < self.glyphs && start <= end && end <= t.len() as u64).then_some((start as usize, end as usize))
    }

    pub(crate) fn axes(&self) -> usize {
        self.axes
    }

    pub(crate) fn tuple_scalars(&self, coords: &[i64]) -> Vec<i64> {
        let tuples = self.shared.chunks_exact(self.axes.max(1)).take(self.tuple_count);
        tuples.map(|t| tuple_scalar(coords, t, None)).collect()
    }
}

const SHARE_POINT_NUMBERS: usize = 0x8000;
const EMBEDDED_PEAK: usize = 0x8000;
const INTERMEDIATE: usize = 0x4000;
const PRIVATE_POINTS: usize = 0x2000;

// What a glyph's deltas are worked out from: coordinates, gvar, shared tuples' scalars where a location keeps them (a
// VARC leaf works out only those its glyph names), and whether HVAR and VVAR, not gvar, move the phantom points.
#[derive(Clone, Copy)]
pub(crate) struct DeltaSource<'a> {
    pub(crate) coords: &'a [i64],
    pub(crate) gvar: Option<&'a Gvar>,
    pub(crate) scalars: Option<&'a [i64]>,
    pub(crate) hvar: bool,
    pub(crate) vvar: bool,
}

// One location as drawing varies glyphs there.
pub(crate) struct DrawnLocation {
    pub(crate) coords: Vec<i64>,
    gvar: Option<Shared<Gvar>>,
    scalars: Vec<i64>,
    pub(crate) hvar: Option<Advances>,
    pub(crate) vvar: Option<Advances>,
}

impl DrawnLocation {
    // None at the default and where gvar refuses every location: glyphs then draw as stored. Its advances take the
    // font's `kept` stores.
    pub(crate) fn new(
        table_map: &Tables,
        key: &[(AxisTag, f64)],
        gvar: Result<Option<Shared<Gvar>>, ()>,
        kept: &DrawnAdvances,
    ) -> Option<DrawnLocation> {
        let coords = coordinates(table_map, key)?;
        if coords.iter().all(|&v| v == 0) {
            return None;
        }
        let gvar = gvar.ok()?;
        let scalars = gvar.as_ref().map_or_else(Vec::new, |g| g.tuple_scalars(&coords));
        let [hvar, vvar] = [&kept.hvar, &kept.vvar].map(|d| d.clone().map(|d| Advances::at(d, &coords)));
        Some(DrawnLocation { coords, gvar, scalars, hvar, vvar })
    }

    pub(crate) fn source(&self) -> DeltaSource<'_> {
        DeltaSource {
            coords: &self.coords,
            gvar: self.gvar.as_deref(),
            scalars: Some(&self.scalars),
            hvar: self.hvar.is_some(),
            vvar: self.vvar.is_some(),
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let advances = [&self.hvar, &self.vvar].into_iter().flatten().map(Advances::heap_bytes).sum::<usize>();
        heap_of(&self.coords) + heap_of(&self.scalars) + advances
    }
}

impl<'a> DeltaSource<'a> {
    // A VARC leaf at normalized coordinates, as `DrawnLocation::new` reads a location, by the font's `kept` stores:
    // None where the leaf draws as stored.
    pub(crate) fn passing(
        coords: &'a [i64], gvar: Result<Option<&'a Shared<Gvar>>, ()>, kept: &DrawnAdvances,
    ) -> Option<DeltaSource<'a>> {
        if coords.iter().all(|&v| v == 0) {
            return None;
        }
        let gvar = gvar.ok()?.map(|gvar| &**gvar);
        Some(DeltaSource { coords, gvar, scalars: None, hvar: kept.hvar.is_some(), vvar: kept.vvar.is_some() })
    }

    // A glyph's `n` deltas in 16.16, phantoms last, worked out in `room` from its contours' points `org`, the rest at
    // 0, and their `ends`. None where it doesn't vary, as where its data fails its checks: the data is absent there.
    // The work, a unit and an axis a tuple and for each applied its bytes and the points it moves, twice where it
    // infers them, is taken from `left`: Err past it, which then reads 0.
    pub(crate) fn deltas<'r>(
        &self, gid: usize, n: usize, org: (&[i32], &[i32]), ends: &[usize], room: &'r mut DeltaRoom, left: &mut usize,
    ) -> Result<Option<&'r [(i64, i64)]>, Error> {
        let mut over = false;
        let mut charge = |n: usize| {
            let spent = left.checked_sub(n);
            (*left, over) = (spent.unwrap_or(0), spent.is_none());
            spent.map(drop)
        };
        let deltas = self.worked_out(gid, n, org, ends, room, &mut charge);
        match over {
            true => Err(Error::limit("gvar: a glyph's tuples run past the work budget")),
            false => Ok(deltas),
        }
    }

    fn worked_out<'r>(
        &self, gid: usize, n: usize, org: (&[i32], &[i32]), ends: &[usize], room: &'r mut DeltaRoom,
        charge: &mut impl FnMut(usize) -> Option<()>,
    ) -> Option<&'r [(i64, i64)]> {
        let gvar = self.gvar?;
        let (start, end) = gvar.range(gid)?;
        if start == end {
            return None;
        }
        let t = gvar.data.as_slice();
        let size = end - start;
        let head = if gvar.extended { 5 } else { 4 };
        if size < head {
            return None;
        }
        let count = usize::from(read_u16_be(t, start)?);
        let data_offset = match gvar.extended {
            true => read_u24_be(t, start + 2)? as usize,
            false => usize::from(read_u16_be(t, start + 2)?),
        };
        if data_offset > size || (count & 0x0FFF) * 4 > size {
            return None;
        }
        let DeltaRoom { deltas, tuple, xs, ys, shared: shared_list, private: own_list, inference } = room;
        let reader = Reader { table: t, limit: end };
        let mut data_at = start + data_offset;
        let shared_points = if count & SHARE_POINT_NUMBERS != 0 {
            let (points, after) = reader.points(data_at.min(end), shared_list);
            charge(after.saturating_sub(data_at))?;
            data_at = after;
            points
        } else {
            // With no shared numbers, a tuple without its own names no subset: its deltas run over every point.
            Some(None)
        };
        let count = count & 0x0FFF;
        let mut p = start + head;
        if 4 * count > end - p {
            return None;
        }
        deltas.clear();
        deltas.resize(n, (0, 0));
        let axes = gvar.axes;
        tuple.clear();
        tuple.resize(3 * axes, 0);
        let (peak, lo) = tuple.split_at_mut(axes);
        let (lo, hi) = lo.split_at_mut(axes);
        let mut laid_out = false;
        let coords16 = |at: usize, out: &mut [i64]| {
            for (j, v) in out.iter_mut().enumerate() {
                *v = i64::from(read_i16_be(t, at + 2 * j).unwrap_or(0)) * 4;
            }
        };
        for i in 0..count {
            charge(1 + axes)?;
            let tuple_size = usize::from(read_u16_be(t, p)?);
            let index = usize::from(read_u16_be(t, p + 2)?);
            p += 4;
            let shared = index & 0x0FFF;
            let apply = if index & !PRIVATE_POINTS < gvar.tuple_count {
                match self.scalars {
                    Some(scalars) => scalars[shared],
                    None => tuple_scalar(self.coords, &gvar.shared[shared * axes..(shared + 1) * axes], None),
                }
            } else {
                let tuple: &[i64] = if index & EMBEDDED_PEAK != 0 {
                    if 2 * axes + 4 * (count - i - 1) > end - p {
                        return None;
                    }
                    coords16(p, peak);
                    p += 2 * axes;
                    peak
                } else if shared < gvar.tuple_count {
                    &gvar.shared[shared * axes..(shared + 1) * axes]
                } else {
                    return None;
                };
                let region = if index & INTERMEDIATE != 0 {
                    if 4 * axes + 4 * (count - i - 1) > end - p {
                        return None;
                    }
                    coords16(p, lo);
                    coords16(p + 2 * axes, hi);
                    p += 4 * axes;
                    Some((&lo[..], &hi[..]))
                } else {
                    None
                };
                tuple_scalar(self.coords, tuple, region)
            };
            if apply != 0 {
                // A tuple's points and deltas lie within its variationDataSize; one running past it is skipped.
                let reader = Reader { table: t, limit: data_at.saturating_add(tuple_size).min(end) };
                let mut at = data_at.min(end);
                charge((reader.limit - at).saturating_add(n))?;
                let points = match index & PRIVATE_POINTS != 0 {
                    true => {
                        let private;
                        (private, at) = reader.points(at, own_list);
                        private
                    }
                    false => shared_points,
                };
                let listed = points.flatten().map_or(0, <[u16]>::len);
                let wanted = if listed == 0 { n } else { listed };
                let read = reader.deltas(at, wanted, xs).and_then(|at| reader.deltas(at, wanted, ys));
                match points.filter(|_| read.is_some()) {
                    Some(None) => {
                        // Whole units, so `mul_fix`'s product is exact: its low 16 bits are 0, and no rounding is due.
                        for ((d, &x), &y) in deltas.iter_mut().zip(&*xs).zip(&*ys) {
                            d.0 += (x >> 16) * apply;
                            d.1 += (y >> 16) * apply;
                        }
                    }
                    Some(Some(points)) => {
                        // Inferring the points the tuple doesn't list costs about as much again as adding them in.
                        charge(n)?;
                        if !laid_out {
                            inference.lay_out(org, n);
                            laid_out = true;
                        }
                        inference.apply(deltas, points, (xs, ys), apply, ends);
                    }
                    None => {}
                }
            }
            data_at += tuple_size;
        }
        if self.hvar {
            deltas[n - 4] = (0, 0);
            deltas[n - 3] = (0, 0);
        }
        if self.vvar {
            deltas[n - 2] = (0, 0);
            deltas[n - 1] = (0, 0);
        }
        Some(deltas.as_slice())
    }
}

// The room a glyph's deltas are worked out in, kept so that the next glyph's allocate nothing: the deltas, a tuple's
// peak and region, its deltas by axis, the shared and a tuple's own point numbers, and inference's points.
#[derive(Default)]
pub(crate) struct DeltaRoom {
    deltas: Vec<(i64, i64)>,
    tuple: Vec<i64>,
    xs: Vec<i64>,
    ys: Vec<i64>,
    shared: Vec<u16>,
    private: Vec<u16>,
    inference: Inference,
}

// A room past this is dropped rather than kept, as a list of points can run far past its glyph's.
#[cfg(feature = "std")]
const ROOM_KEPT_BYTES: usize = 64 * 1024;

#[cfg(feature = "std")]
impl DeltaRoom {
    // Whether its thread keeps it for the next glyph.
    pub(crate) fn kept(&self) -> bool {
        use crate::daecore::daekit::base::sync::heap_of;
        let DeltaRoom { deltas, tuple, xs, ys, shared, private, inference: Inference { org, raw, listed } } = self;
        let points = heap_of(deltas) + heap_of(org) + heap_of(raw) + heap_of(listed);
        points + heap_of(tuple) + heap_of(xs) + heap_of(ys) + heap_of(shared) + heap_of(private) <= ROOM_KEPT_BYTES
    }
}

impl DrawnLocation {
    // GETVARIATION's coordinates: 16.16 cut to 2.14 by a shift.
    pub(crate) fn f2dot14(&self) -> Vec<i32> {
        self.coords.iter().map(|&v| (v >> 2) as i32).collect()
    }

    // cvt's values at the location, each moved by its delta rounded once: None where cvt has none or cvar doesn't
    // vary it, as where cvar is malformed.
    pub(crate) fn cvt(&self, table_map: &Tables) -> Option<Vec<i32>> {
        let cvt = table_map.get("cvt ")?.as_chunks::<2>().0.iter().map(|c| i32::from(i16::from_be_bytes(*c)));
        let cvt: Vec<i32> = cvt.collect();
        let deltas = cvt_deltas(table_map.get("cvar")?, cvt.len(), &self.coords)?;
        Some(cvt.iter().zip(deltas).map(|(&v, d)| v.saturating_add(round_fixed(d))).collect())
    }
}

// Each of `n` CVT entries' 16.16 delta at `coords`, None where cvar's header doesn't read. A tuple must embed its
// peak, as cvar has no shared tuples, so one that doesn't is skipped, as is one whose data doesn't read.
pub(crate) fn cvt_deltas(t: &[u8], n: usize, coords: &[i64]) -> Option<Vec<i64>> {
    if read_u32_be(t, 0)? != 0x10000 {
        return None;
    }
    let count = usize::from(read_u16_be(t, 4)?);
    let mut data_at = usize::from(read_u16_be(t, 6)?);
    if data_at + (count & 0x0FFF) * 4 > t.len() {
        return None;
    }
    let reader = Reader { table: t, limit: t.len() };
    let (mut shared_list, mut own_list) = (Vec::new(), Vec::new());
    let shared_points = if count & SHARE_POINT_NUMBERS != 0 {
        let (points, after) = reader.points(data_at, &mut shared_list);
        data_at = after;
        points
    } else {
        Some(None)
    };
    let axes = coords.len();
    let coords16 = |at: usize| -> Option<Vec<i64>> {
        (0..axes).map(|j| read_i16_be(t, at + 2 * j).map(|v| i64::from(v) * 4)).collect()
    };
    let (mut deltas, mut values) = (vec![0i64; n], Vec::new());
    let mut p = 8;
    for _ in 0..count & 0x0FFF {
        let (size, index) = (usize::from(read_u16_be(t, p)?), usize::from(read_u16_be(t, p + 2)?));
        p += 4;
        let peak = match index & EMBEDDED_PEAK != 0 {
            true => {
                p += 2 * axes;
                Some(coords16(p - 2 * axes)?)
            }
            false => None,
        };
        let region: Option<(Vec<i64>, Vec<i64>)> = match index & INTERMEDIATE != 0 {
            true => {
                p += 4 * axes;
                Some((coords16(p - 4 * axes)?, coords16(p - 2 * axes)?))
            }
            false => None,
        };
        let region = region.as_ref().map(|(lo, hi)| (&lo[..], &hi[..]));
        let apply = peak.map_or(0, |peak| tuple_scalar(coords, &peak, region));
        if apply != 0 {
            // As in gvar, a tuple's points and deltas lie within its variationDataSize.
            let reader = Reader { table: t, limit: data_at.saturating_add(size).min(t.len()) };
            let mut at = data_at.min(t.len());
            let points = match index & PRIVATE_POINTS != 0 {
                true => {
                    let private;
                    (private, at) = reader.points(at, &mut own_list);
                    private
                }
                false => shared_points,
            };
            let listed = points.flatten().map_or(0, <[u16]>::len);
            let count = if listed == 0 { n } else { listed };
            if let (Some(points), Some(_)) = (points, reader.deltas(at, count, &mut values)) {
                match points {
                    None => deltas.iter_mut().zip(&values).for_each(|(d, &v)| *d += mul_fix(v, apply)),
                    Some(points) => {
                        for (&k, &v) in points.iter().zip(&values) {
                            if let Some(d) = deltas.get_mut(usize::from(k)) {
                                *d += mul_fix(v, apply);
                            }
                        }
                    }
                }
            }
        }
        data_at += size;
    }
    Some(deltas)
}

// What a tuple listing some points works with, laid out at the glyph's first such tuple: the points in font units,
// and the tuple's unscaled deltas in 16.16 with whether it listed each point.
#[derive(Default)]
struct Inference {
    org: Vec<(i32, i32)>,
    raw: Vec<(i64, i64)>,
    listed: Vec<bool>,
}

impl Inference {
    fn lay_out(&mut self, (x, y): (&[i32], &[i32]), n: usize) {
        let points = x.iter().copied().zip(y.iter().copied());
        self.org.clear();
        self.org.extend(points.chain(core::iter::repeat((0, 0))).take(n));
        self.raw.resize(n, (0, 0));
        self.listed.resize(n, false);
    }

    // One tuple's deltas onto `deltas`, scaled by `apply` once inferred, as gvar infers on the unscaled deltas. A
    // point listed twice takes both deltas.
    fn apply(
        &mut self, deltas: &mut [(i64, i64)], points: &[u16], (xs, ys): (&[i64], &[i64]), apply: i64, ends: &[usize],
    ) {
        let Inference { org, raw, listed } = self;
        raw.fill((0, 0));
        listed.fill(false);
        for ((&p, &x), &y) in points.iter().zip(xs).zip(ys) {
            let p = usize::from(p);
            if let Some(r) = raw.get_mut(p) {
                *r = (r.0.saturating_add(x), r.1.saturating_add(y));
                listed[p] = true;
            }
        }
        let mut start = 0;
        for &end in ends {
            let stop = end.saturating_add(1).min(raw.len());
            if start < stop {
                infer_contour(start..stop, org, raw, listed);
            }
            start = stop.max(start);
        }
        for (d, r) in deltas.iter_mut().zip(&*raw) {
            *d = (d.0.saturating_add(mul_fix(r.0, apply)), d.1.saturating_add(mul_fix(r.1, apply)));
        }
    }
}

// A contour's unlisted points, each from the listed points before and after it, the contour read as a ring. With
// one point listed, that point is both, so every point takes its delta.
fn infer_contour(span: Range<usize>, org: &[(i32, i32)], raw: &mut [(i64, i64)], listed: &[bool]) {
    let Some(first) = span.clone().find(|&p| listed[p]) else { return };
    let next = |p: usize| if p + 1 == span.end { span.start } else { p + 1 };
    let mut before = first;
    loop {
        let mut after = next(before);
        while !listed[after] {
            after = next(after);
        }
        if after == before {
            let only = raw[before];
            raw[span].fill(only);
            return;
        }
        let ((xb, yb), (xa, ya), (db, da)) = (org[before], org[after], (raw[before], raw[after]));
        let mut p = next(before);
        while p != after {
            let (x, y) = org[p];
            raw[p] = (infer(x, (xb, db.0), (xa, da.0)), infer(y, (yb, db.1), (ya, da.1)));
            p = next(p);
        }
        if after == first {
            return;
        }
        before = after;
    }
}

// One axis of a point at `t` between listed points at `a` and `b` with deltas `da` and `db`, by gvar's rule: where
// they meet, their delta if they agree and none if not; beyond both, the nearer one's; between, the line through
// both, rounded once to 16.16, half away from zero.
fn infer(t: i32, (a, da): (i32, i64), (b, db): (i32, i64)) -> i64 {
    if a == b {
        return if da == db { da } else { 0 };
    }
    let ((lo, low), (hi, high)) = if a < b { ((a, da), (b, db)) } else { ((b, db), (a, da)) };
    if t <= lo {
        return low;
    }
    if t >= hi {
        return high;
    }
    let (diff, along, run) = (i128::from(high) - i128::from(low), (t - lo) as u64, (hi - lo) as u64);
    // In 64 bits where the product fits, as it nearly always does: 128-bit division takes many times as long.
    let narrow = u64::try_from(diff.unsigned_abs()).ok().and_then(|d| d.checked_mul(along)?.checked_add(run / 2));
    let step = match narrow {
        Some(rise) => u128::from(rise / run),
        None => ((diff * i128::from(along)).unsigned_abs() + u128::from(run / 2)) / u128::from(run),
    };
    low + if diff < 0 { -(step as i64) } else { step as i64 }
}

// Packed point numbers and deltas inside one glyph's data, which ends at `limit`. A run is checked against it; a
// point count isn't, so it may be read on into the table.
struct Reader<'a> {
    table: &'a [u8],
    limit: usize,
}

impl Reader<'_> {
    // The point numbers, read into `out`: Some(None) for every point, None where they don't read, as where a run
    // passes the count; and where the reading stopped.
    fn points<'b>(&self, mut at: usize, out: &'b mut Vec<u16>) -> (Option<Option<&'b [u16]>>, usize) {
        let byte = |at: &mut usize| {
            let b = self.table.get(*at).copied().filter(|_| *at < self.limit);
            *at += 1;
            b
        };
        let Some(first) = byte(&mut at) else { return (None, at) };
        let mut n = usize::from(first);
        if n == 0 {
            return (Some(None), at);
        }
        if n & 0x80 != 0 {
            let Some(low) = byte(&mut at) else { return (None, at) };
            n = ((n & 0x7F) << 8) | usize::from(low);
        }
        let start = at;
        out.clear();
        out.reserve(n);
        let mut p = at;
        let mut last = 0u16;
        while out.len() < n {
            if p >= self.limit {
                return (None, start);
            }
            let run = self.table[p];
            p += 1;
            let count = usize::from(run & 0x7F) + 1;
            let width = if run & 0x80 != 0 { 2 } else { 1 };
            if count > n - out.len() || width * count > self.limit - p {
                return (None, start);
            }
            for _ in 0..count {
                let step = match width {
                    2 => u16::from_be_bytes([self.table[p], self.table[p + 1]]),
                    _ => u16::from(self.table[p]),
                };
                p += width;
                last = last.wrapping_add(step);
                out.push(last);
            }
        }
        (Some(Some(out)), p)
    }

    // `count` deltas in 16.16 into `out`, and where they end, None where they don't read, as where a run passes the
    // count. Both flags set mark a run of 32-bit deltas, daegun's own extension for values past 16 bits.
    fn deltas(&self, at: usize, count: usize, out: &mut Vec<i64>) -> Option<usize> {
        out.clear();
        out.reserve(count);
        let mut p = at;
        while out.len() < count {
            if p >= self.limit {
                return None;
            }
            let run = self.table[p];
            p += 1;
            let n = usize::from(run & 0x3F) + 1;
            let width = [1, 2, 0, 4][usize::from(run >> 6)];
            if n > count - out.len() || width * n > self.limit - p {
                return None;
            }
            let bytes = &self.table[p..p + width * n];
            match width {
                0 => out.extend(core::iter::repeat_n(0, n)),
                1 => out.extend(bytes.iter().map(|&b| i64::from(b as i8) << 16)),
                2 => out.extend(bytes.as_chunks::<2>().0.iter().map(|&w| i64::from(i16::from_be_bytes(w)) << 16)),
                _ => out.extend(bytes.as_chunks::<4>().0.iter().map(|&w| i64::from(i32::from_be_bytes(w)) << 16)),
            }
            p += width * n;
        }
        Some(p)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/drawn.rs"]
pub(super) mod tests;
