#[cfg(doc)]
use crate::ErrorKind::{Malformed, Unsupported};
use alloc::vec::Vec;
use super::super::decoder::{read_f2dot14, read_i16_be, read_u16_be, read_u32_be, records_fit};
use super::round::ot_round;
use crate::daecore::daetype::TableBytes;
use crate::daecore::Error;

/// An ItemVariationStore: the regions of a variable font's design space, and the deltas each item takes in them.
#[non_exhaustive]
#[derive(Clone, Default, Debug)]
pub struct ItemVariationStore {
    /// Each region, one axis range a font axis.
    pub regions:    Vec<Vec<RegionAxis>>,
    /// Its subtables, the outer index of a delta-set index.
    pub ivd_data:   Vec<Ivd>,
    /// How many axes each region spans.
    pub axis_count: usize,
}

/// One axis of a region, in normalized coordinates: its influence rises from `start` to 1 at `peak` and falls back
/// to 0 at `end`. Laid out as C reads it.
#[expect(clippy::exhaustive_structs, reason = "the spec's record, field for field")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct RegionAxis {
    /// Where its influence starts.
    pub start: f64,
    /// Where its influence is whole.
    pub peak: f64,
    /// Where its influence ends.
    pub end: f64,
}

/// An ItemVariationData subtable: rows of deltas, one an item, a column a region it names.
#[derive(Clone, Default)]
pub struct Ivd {
    /// The regions its columns name, by index into the store's.
    pub region_indices: Vec<usize>,
    deltas: Vec<i32>,
    items: usize,
}

impl core::fmt::Debug for Ivd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = f.debug_struct("Ivd");
        s.field("region_indices", &self.region_indices).field("items", &self.items).finish_non_exhaustive()
    }
}

impl Ivd {
    /// Item `inner`'s deltas, one a region; None past its items. With no regions, each item is a row of none.
    pub fn row(&self, inner: usize) -> Option<&[i32]> {
        if inner >= self.items {
            return None;
        }
        let width = self.region_indices.len();
        let start = inner.checked_mul(width)?;
        self.deltas.get(start..start.checked_add(width)?)
    }

    /// How many items it holds.
    pub const fn rows(&self) -> usize {
        self.items
    }
}

impl ItemVariationStore {
    /// The ItemVariationStore at `base`.
    ///
    /// # Errors
    ///
    /// [`Malformed`] for one cut short or past the end, [`Unsupported`] for a format other than 1.
    #[doc(alias("parse_item_variation_store"))]
    pub fn parse(buf: &[u8], base: usize) -> Result<ItemVariationStore, Error> {
        // The base can come from C. Inside the buffer, the few bytes added to it below cannot overflow.
        if base >= buf.len() {
            return Err(Error::malformed("IVS: base past the end"));
        }
        if read_u16_be(buf, base) != Some(1) {
            return Err(Error::unsupported("IVS: unknown format"));
        }
        let layout = Layout::read(buf, base).ok_or(Error::malformed("IVS: header truncated"))?;
        let axis = |at: usize| {
            let field = |k: usize| read_f2dot14(buf, at + k).unwrap_or(0.0);
            RegionAxis { start: field(0), peak: field(2), end: field(4) }
        };
        let regions = (0..layout.region_count)
            .map(|i| (0..layout.axis_count).map(|j| axis(layout.region_at(i) + j * 6)).collect())
            .collect();
        let ivd_data = layout.subtables.iter().map(|s| s.decode(buf)).collect();
        Ok(ItemVariationStore { regions, ivd_data, axis_count: layout.axis_count })
    }
}

/// Where an item's deltas lie in an ItemVariationStore: its ItemVariationData subtable and its row there.
#[expect(clippy::exhaustive_structs, reason = "the spec's pair, field for field")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct DeltaSetIndex {
    /// The ItemVariationData subtable.
    pub outer: u32,
    /// The row within it.
    pub inner: u32,
}

/// The DeltaSetIndexMap at `base`, an entry an item.
///
/// # Errors
///
/// [`Malformed`] for one cut short, [`Unsupported`] for a format past 1.
pub fn parse_delta_set_index_map(buf: &[u8], base: usize) -> Result<Vec<DeltaSetIndex>, Error> {
    let fmt       = *buf.get(base).ok_or(Error::malformed("IVS: delta set index map truncated"))?;
    if fmt > 1 {
        return Err(Error::unsupported("IVS: unknown delta set index map format"));
    }
    let entry_fmt = *buf.get(base + 1).ok_or(Error::malformed("IVS: delta set index map truncated"))?;
    let map_count: usize = if fmt == 0 {
        read_u16_be(buf, base + 2).ok_or(Error::malformed("IVS: delta set index map truncated"))? as usize
    } else {
        read_u32_be(buf, base + 2).ok_or(Error::malformed("IVS: delta set index map truncated"))? as usize
    };
    let inner_bits = (entry_fmt & 0x0F) as usize + 1;
    let entry_size = ((entry_fmt >> 4) & 0x3) as usize + 1;
    let inner_mask = (1usize << inner_bits) - 1;
    let data_off   = if fmt == 0 { 4 } else { 6 };

    let need = map_count.checked_mul(entry_size)
        .and_then(|n| n.checked_add(base + data_off))
        .ok_or(Error::malformed("IVS: delta set index map too large"))?;
    if need > buf.len() { return Err(Error::malformed("IVS: delta set index map truncated")); }

    let entries = buf.get(base + data_off..need).ok_or(Error::malformed("IVS: delta set index map truncated"))?;
    let map: Vec<DeltaSetIndex> = entries
        .chunks_exact(entry_size)
        .map(|e| {
            let val = e.iter().fold(0usize, |acc, &b| (acc << 8) | b as usize);
            DeltaSetIndex { outer: (val >> inner_bits) as u32, inner: (val & inner_mask) as u32 }
        })
        .collect();

    Ok(map)
}

/// Item `idx`'s delta-set index in a map: the last entry past its end, and outer 0, inner `idx` for an empty map,
/// which is no map.
pub fn delta_set_index_map_lookup(map: &[DeltaSetIndex], idx: usize) -> DeltaSetIndex {
    map.get(idx).or(map.last()).copied().unwrap_or(DeltaSetIndex { outer: 0, inner: saturated(idx) })
}

// An index past u32 names no row, as u32::MAX doesn't.
fn saturated(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

// A table's ItemVariationStore and the DeltaSetIndexMap it names, if any: what its variation indices
// resolve to. An empty map is no map.
pub struct DeltaSource {
    pub store: ItemVariationStore,
    map: Option<Vec<DeltaSetIndex>>,
}

impl DeltaSource {
    pub fn read(table: &[u8], store_at: usize, map_at: Option<usize>) -> Result<Self, Error> {
        let store = ItemVariationStore::parse(table, store_at)?;
        let map = map_at.map(|at| parse_delta_set_index_map(table, at)).transpose()?.filter(|m| !m.is_empty());
        Ok(DeltaSource { store, map })
    }

    // Each region's scalar at `location`, which `delta` takes.
    pub fn scalars(&self, location: &[f64]) -> Vec<i64> {
        store_scalars(&self.store, location)
    }

    // The delta `index` names in 16.16: through the map, where past the end the last entry answers, or through
    // the table's own `implicit` (outer, inner) where it has none.
    pub fn delta(&self, index: usize, implicit: (usize, usize), scalars: &[i64]) -> i64 {
        let (outer, inner) = match self.map.as_deref() {
            Some(map) => delta_set_index_map_lookup(map, index).into(),
            None => implicit,
        };
        store_delta(&self.store, outer, inner, scalars)
    }

    // `delta` for glyphs, each implicitly (0, glyph), a long row worked out once: a map can name one for every glyph,
    // which a pass over them would otherwise read whole for each.
    pub(crate) fn glyph_deltas<'s>(&'s self, scalars: &'s [i64]) -> impl FnMut(usize) -> i64 + 's {
        let mut worked = alloc::collections::BTreeMap::new();
        move |gid| {
            let (outer, inner) = match self.map.as_deref() {
                Some(map) => delta_set_index_map_lookup(map, gid).into(),
                None => (0, gid),
            };
            let delta = || store_delta(&self.store, outer, inner, scalars);
            match self.store.ivd_data.get(outer).is_some_and(|set| set.region_indices.len() > SHORT_ROW) {
                true => *worked.entry((outer, inner)).or_insert_with(delta),
                false => delta(),
            }
        }
    }

    // How many leading indices can each have a delta of their own, through the map or the implicit (0, index);
    // all indices past them share one.
    pub fn extent(&self) -> usize {
        self.map.as_ref().map_or_else(|| self.store.ivd_data.first().map_or(0, Ivd::rows), Vec::len)
    }
}

// A store's structure, which every reader takes: nothing past the table, a failing region list, data set or row absent
// (deltas 0), not the store. None without a store: past the table, another format, or its header cut short.
struct Layout {
    regions_at: usize,
    axis_count: usize,
    region_count: usize,
    subtables: Vec<SubtableView>,
}

impl Layout {
    fn read(buf: &[u8], base: usize) -> Option<Layout> {
        if base >= buf.len() || read_u16_be(buf, base) != Some(1) {
            return None;
        }
        let (list_at, ivd_count) = (read_u32_be(buf, base + 2)? as usize, usize::from(read_u16_be(buf, base + 6)?));
        let list = base.checked_add(list_at).and_then(|at| {
            let (axes, count) = (usize::from(read_u16_be(buf, at)?), usize::from(read_u16_be(buf, at + 2)?));
            records_fit(at + 4, count, axes * 6, buf.len()).then_some((at + 4, axes, count))
        });
        let (regions_at, axis_count, region_count) = list.unwrap_or_default();
        // Offsets may repeat, so a set past these budgets is absent: four values per byte from the store's start,
        // which is all its offsets reach, and rows budgeted apart, as an empty row costs none.
        let mut rows_left: usize = 1_000_000;
        let mut values_left = buf.len().saturating_sub(base).saturating_mul(4).max(1 << 16);
        let mut subtables = Vec::with_capacity(ivd_count.min(256));
        for i in 0..ivd_count {
            let at = read_u32_be(buf, base + 8 + i * 4).filter(|&off| off != 0);
            let set = at.and_then(|off| SubtableView::read(buf, base.checked_add(off as usize)?)).filter(|set| {
                let values = set.items.checked_mul(set.regions).and_then(|v| v.checked_add(set.regions));
                let left = rows_left.checked_sub(set.items).zip(values.and_then(|v| values_left.checked_sub(v)));
                left.map(|(rows, values)| (rows_left, values_left) = (rows, values)).is_some()
            });
            subtables.push(set.unwrap_or_default());
        }
        Some(Layout { regions_at, axis_count, region_count, subtables })
    }

    fn region_at(&self, i: usize) -> usize {
        self.regions_at + i * self.axis_count * 6
    }
}

// An ItemVariationData's region indices, then its rows: `word_count` deltas `wide` bytes each, the rest half that.
#[derive(Default)]
struct SubtableView {
    indices_at: usize,
    regions: usize,
    word_count: usize,
    wide: usize,
    rows_at: usize,
    row_bytes: usize,
    items: usize,
}

impl SubtableView {
    // None where its header or region indices run past the table, or it says more word deltas than regions.
    fn read(buf: &[u8], ivd: usize) -> Option<SubtableView> {
        let field = |at: usize| read_u16_be(buf, ivd.checked_add(at)?).map(usize::from);
        let (items, word_count, regions) = (field(0)?, field(2)?, field(4)?);
        let wide = if word_count & 0x8000 != 0 { 4 } else { 2 };
        let word_count = word_count & 0x7FFF;
        if !records_fit(ivd + 6, regions, 2, buf.len()) || word_count > regions {
            return None;
        }
        let row_bytes = word_count * wide + (regions - word_count) * wide / 2;
        let rows_at = ivd + 6 + regions * 2;
        Some(SubtableView { indices_at: ivd + 6, regions, word_count, wide, rows_at, row_bytes, items })
    }

    // Row `inner`'s bytes, None past its items or the table.
    fn row<'b>(&self, buf: &'b [u8], inner: usize) -> Option<&'b [u8]> {
        let at = inner.checked_mul(self.row_bytes)?.checked_add(self.rows_at)?;
        buf.get(at..at.checked_add(self.row_bytes)?).filter(|_| inner < self.items)
    }

    // Region index `j`'s delta in a row.
    fn value(&self, row: &[u8], j: usize) -> i32 {
        let (at, size) = match j < self.word_count {
            true => (j * self.wide, self.wide),
            false => (self.word_count * self.wide + (j - self.word_count) * self.wide / 2, self.wide / 2),
        };
        match (size, row.get(at..at + size)) {
            (4, Some(&[a, b, c, d])) => i32::from_be_bytes([a, b, c, d]),
            (2, Some(&[a, b])) => i32::from(i16::from_be_bytes([a, b])),
            (1, Some(&[a])) => i32::from(a as i8),
            _ => 0,
        }
    }

    fn region(&self, buf: &[u8], j: usize) -> usize {
        read_u16_be(buf, self.indices_at + 2 * j).map_or(usize::MAX, usize::from)
    }

    // Its rows decoded, as `value` reads each, a row past the table all 0.
    fn decode(&self, buf: &[u8]) -> Ivd {
        let region_indices = (0..self.regions).map(|j| self.region(buf, j)).collect();
        let mut deltas = Vec::with_capacity(self.items.min(256).saturating_mul(self.regions));
        let rows = buf.get(self.rows_at..).unwrap_or_default();
        let fit = rows.len().checked_div(self.row_bytes).unwrap_or(0).min(self.items);
        for row in rows[..fit * self.row_bytes].chunks_exact(self.row_bytes.max(1)) {
            let (wide, narrow) = row.split_at(self.word_count * self.wide);
            if self.wide == 4 {
                deltas.extend(wide.as_chunks::<4>().0.iter().map(|&c| i32::from_be_bytes(c)));
                deltas.extend(narrow.as_chunks::<2>().0.iter().map(|&c| i32::from(i16::from_be_bytes(c))));
            } else {
                deltas.extend(wide.as_chunks::<2>().0.iter().map(|&c| i32::from(i16::from_be_bytes(c))));
                deltas.extend(narrow.iter().map(|&b| i32::from(b as i8)));
            }
        }
        deltas.resize(self.items * self.regions, 0);
        Ivd { region_indices, deltas, items: self.items }
    }
}

// An ItemVariationStore read in place, where `ItemVariationStore::parse` decodes the same layout whole, and summed as
// `store_delta`.
pub(crate) struct StoreView {
    table: TableBytes,
    layout: Layout,
}

impl StoreView {
    pub(crate) fn read(table: TableBytes, base: usize) -> Option<StoreView> {
        let layout = Layout::read(table.as_slice(), base)?;
        Some(StoreView { table, layout })
    }

    pub(crate) fn axis_count(&self) -> usize {
        self.layout.axis_count
    }

    // Region `i`'s scalar at 16.16 `coords`; 0 past the regions.
    pub(crate) fn region_scalar(&self, coords: &[i64], i: usize) -> i64 {
        if i >= self.layout.region_count {
            return 0;
        }
        let (buf, region) = (self.table.as_slice(), self.layout.region_at(i));
        let field = |at: usize| read_i16_be(buf, at).map_or(0, |v| i64::from(v) * 4);
        let axes = (0..self.layout.axis_count).map(|j| region + j * 6).map(|a| (field(a), field(a + 2), field(a + 4)));
        region_scalar(coords, axes)
    }

    // Data set `outer`'s region indices, None where it's absent or past the store.
    pub(crate) fn data_regions(&self, outer: usize) -> Option<impl ExactSizeIterator<Item = usize> + '_> {
        let set = self.layout.subtables.get(outer).filter(|s| s.indices_at != 0)?;
        Some((0..set.regions).map(|j| set.region(self.table.as_slice(), j)))
    }

    // Each region's scalar at 16.16 `coords`, which `delta` takes, as `store_scalars`.
    pub(crate) fn scalars(&self, coords: &[i64]) -> Vec<i64> {
        (0..self.layout.region_count).map(|i| self.region_scalar(coords, i)).collect()
    }

    // An item's delta from `buf`, the store's own table as its reader holds it. A region at 0 adds nothing, so its
    // delta is never read.
    fn delta(&self, buf: &[u8], outer: usize, inner: usize, scalars: &[i64]) -> i64 {
        #[cfg(test)]
        let _ = ROWS_SUMMED.try_with(|n| n.set(n.get() + 1));
        let Some(set) = self.layout.subtables.get(outer) else { return 0 };
        let Some(row) = set.row(buf, inner) else { return 0 };
        let indices = buf.get(set.indices_at..set.indices_at + 2 * set.regions).unwrap_or_default();
        let scalar = |r: &[u8; 2]| scalars.get(usize::from(u16::from_be_bytes(*r))).copied().unwrap_or(0);
        indices.as_chunks::<2>().0.iter().enumerate().fold(0i64, |sum, (j, r)| match scalar(r) {
            0 => sum,
            s => sum.saturating_add(i64::from(set.value(row, j)) * s),
        })
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        crate::daecore::daekit::base::sync::heap_of(&self.layout.subtables)
    }
}

// A DeltaSetIndexMap read in place, checked as `parse_delta_set_index_map` checks it.
struct MapView {
    data_at: usize,
    count: usize,
    entry_size: usize,
    inner_bits: usize,
}

impl MapView {
    // None where the parse fails, Some(None) for an empty map, which is no map.
    fn read(buf: &[u8], base: usize) -> Option<Option<MapView>> {
        let format = *buf.get(base)?;
        if format > 1 {
            return None;
        }
        let entry_format = *buf.get(base + 1)?;
        let (count, data_at) = match format {
            0 => (usize::from(read_u16_be(buf, base + 2)?), base + 4),
            _ => (read_u32_be(buf, base + 2)? as usize, base + 6),
        };
        let entry_size = usize::from((entry_format >> 4) & 0x3) + 1;
        if !records_fit(data_at, count, entry_size, buf.len()) {
            return None;
        }
        let inner_bits = usize::from(entry_format & 0x0F) + 1;
        Some((count > 0).then_some(MapView { data_at, count, entry_size, inner_bits }))
    }

    // As `delta_set_index_map_lookup`: past the end, the last entry answers.
    fn lookup(&self, buf: &[u8], index: usize) -> (usize, usize) {
        // An entry is the low bytes of the four ending it, the map's header lying before the first.
        let end = self.data_at + (index.min(self.count - 1) + 1) * self.entry_size;
        let value = read_u32_be(buf, end - 4).map_or(0, |v| (v & (u32::MAX >> (32 - 8 * self.entry_size))) as usize);
        (value >> self.inner_bits, value & ((1 << self.inner_bits) - 1))
    }
}

// `DeltaSource` read in place: a table's store and the map it names, if any.
pub(crate) struct DeltaView {
    store: StoreView,
    map: Option<MapView>,
}

impl DeltaView {
    pub(crate) fn read(table: &TableBytes, store_at: usize, map_at: Option<usize>) -> Option<DeltaView> {
        let map = match map_at {
            Some(at) => MapView::read(table, at)?,
            None => None,
        };
        Some(DeltaView { store: StoreView::read(table.clone(), store_at)?, map })
    }

    pub(crate) fn scalars(&self, location: &[f64]) -> Vec<i64> {
        self.store.scalars(&fixed_coords(location))
    }

    // Its scalars at 16.16 `coords`.
    pub(crate) fn scalars_at(&self, coords: &[i64]) -> Vec<i64> {
        self.store.scalars(coords)
    }

    pub(crate) fn delta(&self, index: usize, implicit: (usize, usize), scalars: &[i64]) -> i64 {
        let buf = self.store.table.as_slice();
        let (outer, inner) = self.map.as_ref().map_or(implicit, |map| map.lookup(buf, index));
        self.store.delta(buf, outer, inner, scalars)
    }

    // As `DeltaSource::glyph_deltas`.
    pub(crate) fn glyph_deltas<'s>(&'s self, scalars: &'s [i64]) -> impl FnMut(usize) -> i64 + 's {
        let (buf, mut worked) = (self.store.table.as_slice(), alloc::collections::BTreeMap::new());
        move |gid| {
            let (outer, inner) = self.map.as_ref().map_or((0, gid), |map| map.lookup(buf, gid));
            let delta = || self.store.delta(buf, outer, inner, scalars);
            match self.store.layout.subtables.get(outer).is_some_and(|set| set.regions > SHORT_ROW) {
                true => *worked.entry((outer, inner)).or_insert_with(delta),
                false => delta(),
            }
        }
    }

    pub(crate) fn extent(&self) -> usize {
        self.map.as_ref().map_or_else(|| self.store.layout.subtables.first().map_or(0, |s| s.items), |m| m.count)
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.store.heap_bytes()
    }
}

impl ItemVariationStore {
    /// Each region's scalar at normalized coordinates `location`, an axis past them at 0, worked out in 16.16 as
    /// every variation in daegun is.
    #[doc(alias("precompute_region_scalars"))]
    pub fn region_scalars(&self, location: &[f64]) -> Vec<f64> {
        store_scalars(self, location).into_iter().map(units).collect()
    }

    /// An item's delta: its row's deltas weighted by [`region_scalars`](Self::region_scalars)'s scalars and summed;
    /// 0 for an index past the store's.
    #[doc(alias("compute_ivs_delta_f64"))]
    pub fn delta(&self, index: DeltaSetIndex, region_scalars: &[f64]) -> f64 {
        let (outer, inner) = index.into();
        units(weighted_delta(self, outer, inner, |r| region_scalars.get(r).map_or(0, |&s| fixed_coord(s))))
    }
}

impl From<DeltaSetIndex> for (usize, usize) {
    fn from(index: DeltaSetIndex) -> (usize, usize) {
        (index.outer as usize, index.inner as usize)
    }
}

// The most regions a row has that's summed again for each glyph naming it, which costs no more than looking up its
// sum; real fonts' rows hold a few dozen at most.
const SHORT_ROW: usize = 64;

// 16.16's one: every scalar and delta is worked in 16.16, which meets the variations overview's 16 fractional bits.
pub(crate) const ONE: i64 = 0x10000;

// A normalized coordinate in 16.16; a location's are 2.14 values, which convert exactly.
pub(crate) fn fixed_coord(v: f64) -> i64 {
    i64::from(ot_round(v * 65_536.0))
}

pub(crate) fn fixed_coords(location: &[f64]) -> Vec<i64> {
    location.iter().map(|&v| fixed_coord(v)).collect()
}

// A 16.16 value as a number of units, exact in a double.
pub(crate) fn units(v: i64) -> f64 {
    v as f64 / 65_536.0
}

// A 16.16 value rounded to a whole unit, a half up, as a delta is stored.
pub(crate) fn round_fixed(v: i64) -> i32 {
    (v.saturating_add(0x8000) >> 16).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub(crate) fn store_scalars(store: &ItemVariationStore, location: &[f64]) -> Vec<i64> {
    let coords = fixed_coords(location);
    let fixed = |a: &RegionAxis| (fixed_coord(a.start), fixed_coord(a.peak), fixed_coord(a.end));
    store.regions.iter().map(|region| region_scalar(&coords, region.iter().map(fixed))).collect()
}

// An item's delta in 16.16: its row's deltas times their regions' scalars, summed exactly.
pub(crate) fn store_delta(store: &ItemVariationStore, outer: usize, inner: usize, scalars: &[i64]) -> i64 {
    weighted_delta(store, outer, inner, |r| scalars.get(r).copied().unwrap_or(0))
}

// The rows summed on this thread, which the tests count.
#[cfg(test)]
std::thread_local! {
    pub(crate) static ROWS_SUMMED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

// The same with region `r`'s scalar as `scalar` gives it.
fn weighted_delta(store: &ItemVariationStore, outer: usize, inner: usize, scalar: impl Fn(usize) -> i64) -> i64 {
    #[cfg(test)]
    let _ = ROWS_SUMMED.try_with(|n| n.set(n.get() + 1));
    let Some(ivd) = store.ivd_data.get(outer) else { return 0 };
    let Some(row) = ivd.row(inner) else { return 0 };
    ivd.region_indices.iter().zip(row).fold(0, |sum, (&r, &value)| sum.saturating_add(i64::from(value) * scalar(r)))
}

// A VariationIndex table's outer and inner index; a Device table of sizes is not one.
pub(crate) fn variation_index(table: &[u8], at: usize) -> Option<(u16, u16)> {
    if read_u16_be(table, at.checked_add(4)?)? != 0x8000 {
        return None;
    }
    Some((read_u16_be(table, at)?, read_u16_be(table, at + 2)?))
}

// What a Device or VariationIndex table adds at a location in 16.16, from the ItemVariationStore of the table
// that holds it; each (outer, inner) is worked out once, as many tables may name one.
pub(crate) struct DeviceDeltas {
    store: crate::daecore::daekit::base::sync::Shared<ItemVariationStore>,
    scalars: Vec<i64>,
    worked_out: alloc::collections::BTreeMap<(u16, u16), i64>,
}

impl DeviceDeltas {
    pub(crate) fn new(table: &[u8], store_at: usize, location: &[f64]) -> Option<DeviceDeltas> {
        if store_at == 0 || location.iter().all(|&c| c == 0.0) {
            return None;
        }
        DeviceDeltas::from_store(
            crate::daecore::daekit::base::sync::Shared::new(ItemVariationStore::parse(table, store_at).ok()?),
            location,
        )
    }

    // Over a store parsed already, such as the one a font keeps for the shaper.
    pub(crate) fn from_store(
        store: crate::daecore::daekit::base::sync::Shared<ItemVariationStore>,
        location: &[f64],
    ) -> Option<DeviceDeltas> {
        if location.iter().all(|&c| c == 0.0) {
            return None;
        }
        let scalars = store_scalars(&store, location);
        Some(DeviceDeltas { store, scalars, worked_out: alloc::collections::BTreeMap::new() })
    }

    // A VariationIndex's delta in 16.16; a Device table of sizes adds nothing, applying only at a ppem.
    pub(crate) fn delta(&mut self, table: &[u8], device_at: usize) -> i64 {
        let Some((outer, inner)) = variation_index(table, device_at) else { return 0 };
        let (store, scalars) = (&self.store, &self.scalars);
        *self
            .worked_out
            .entry((outer, inner))
            .or_insert_with(|| store_delta(store, usize::from(outer), usize::from(inner), scalars))
    }
}

// A region's scalar at 16.16 `coords` by the variations overview's per-axis rule, each axis's share multiplied in
// and rounded in 16.16: an ill-formed axis or one peaking at 0 is left out, one past the coordinates sits at 0.
pub(crate) fn region_scalar(coords: &[i64], axes: impl Iterator<Item = (i64, i64, i64)>) -> i64 {
    let mut scalar = ONE;
    for (j, (start, peak, end)) in axes.enumerate() {
        let c = coords.get(j).copied().unwrap_or(0);
        if start > peak || peak > end || (start < 0 && end > 0) || peak == 0 || c == peak {
            continue;
        }
        if c <= start || c >= end {
            return 0;
        }
        let (part, whole) = if c < peak { (c - start, peak - start) } else { (end - c, end - peak) };
        scalar = (scalar * part + whole / 2) / whole;
    }
    scalar
}

// A gvar or cvar tuple's scalar: a peak alone spans from 0 to it, an intermediate region from its start to its end.
pub(crate) fn tuple_scalar(coords: &[i64], peak: &[i64], region: Option<(&[i64], &[i64])>) -> i64 {
    match region {
        Some((start, end)) => region_scalar(coords, start.iter().zip(peak).zip(end).map(|((&s, &p), &e)| (s, p, e))),
        None => region_scalar(coords, peak.iter().map(|&p| (p.min(0), p, p.max(0)))),
    }
}

// The scalars of one ItemVariationData's regions in its order, as a CFF2 blend takes them.
pub(crate) fn data_scalars(store: &ItemVariationStore, outer: usize, location: &[f64]) -> Option<Vec<i64>> {
    let ivd = store.ivd_data.get(outer)?;
    let scalars = store_scalars(store, location);
    Some(ivd.region_indices.iter().map(|&r| scalars.get(r).copied().unwrap_or(0)).collect())
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/ivs.rs"]
mod tests;
