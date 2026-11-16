use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::decoder::{bump_i16, clamp_i16, num_glyphs, read_u32_be};
use super::super::format::ivs::{round_fixed, DeltaSource, DeltaView};
use super::super::format::metrics::{bearing_at, shaped_tags, MetricsTable};
use super::super::vorg::Vorg;
use crate::daecore::daetype::{TableBytes, Tables};
use crate::daecore::daekit::base::sync::{GlyphSlots, Shared, SpareSlots};
use super::measured::MeasuredGlyphs;
use crate::daecore::Error;

// Every glyph below `count` its own long record, as shaping reads the table.
pub(crate) fn expand_metrics(table: &MetricsTable, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count * 4);
    for gid in 0..count {
        let (advance, bearing) = table.pair(gid);
        out.extend_from_slice(&advance.to_be_bytes());
        out.extend_from_slice(&bearing.to_be_bytes());
    }
    out
}

// HVAR's or VVAR's advance deltas; None where the table is missing or fails to read.
pub(crate) fn advance_deltas(table_map: &Tables, var_tag: &str) -> Option<DeltaSource> {
    let var = table_map.get(var_tag)?;
    let at = |field| read_u32_be(var, field).map(|v| v as usize);
    let map = at(8)?;
    DeltaSource::read(var, at(4)?, (map != 0).then_some(map)).ok()
}

// The same read in place, None exactly where `advance_deltas` is.
pub(crate) fn advance_view(table_map: &Tables, var_tag: &str) -> Option<DeltaView> {
    let var = table_map.get(var_tag)?;
    let at = |field| read_u32_be(var, field).map(|v| v as usize);
    let map = at(8)?;
    DeltaView::read(var, at(4)?, (map != 0).then_some(map))
}

// HVAR's view and VVAR's, read once for every location's advances: the store and index map don't vary, only the
// scalars a location works out.
pub(crate) fn advance_views(table_map: &Tables) -> [Option<Shared<DeltaView>>; 2] {
    ["HVAR", "VVAR"].map(|var| advance_view(table_map, var).map(Shared::new))
}

// How many glyphs have advances of their own once deltas reaching `extent` vary the table: those it has and those the
// deltas tell apart. Past them each advances by one delta alone, which no table can state for every id.
pub(crate) fn varied_count(table: &MetricsTable, extent: usize) -> usize {
    extent.min(usize::from(u16::MAX)).max(table.len())
}

// That count, and the advance of each glyph past it.
pub(crate) fn past_advance(table: &MetricsTable, deltas: &DeltaView, scalars: &[i64]) -> (usize, u32) {
    let count = varied_count(table, deltas.extent());
    (count, round_fixed(deltas.delta(count, (0, count), scalars)).max(0) as u32)
}

// A location's advances one way, each glyph's worked out when first read: as `vary_metrics` writes them from HVAR or
// VVAR, else, where gvar reads, from each glyph's phantom points as gvar moves them.
pub(crate) struct VariedAdvances {
    source: Source,
    past: u32,
    // Each of the first `varied_count` glyphs' advance plus one, 0 until read; threads filling one write the same.
    read: GlyphSlots,
}

enum Source {
    Deltas { table: TableBytes, stated: usize, glyphs: usize, deltas: Shared<DeltaView>, scalars: Vec<i64> },
    Phantoms { measured: Shared<MeasuredGlyphs>, vertical: bool },
}

impl VariedAdvances {
    // None where the font's advances hold at the location: its table has none, or it varies neither by HVAR or VVAR
    // nor by gvar. `view` is that way's of `advance_views`; the advances' table is taken from `spare`.
    pub(crate) fn new(
        table_map: &Tables, vertical: bool, location: &[f64],
        measured: Option<&Shared<MeasuredGlyphs>>, view: Option<&Shared<DeltaView>>, spare: &SpareSlots,
    ) -> Option<Self> {
        let tags = shaped_tags(table_map, vertical);
        let varies = location.iter().any(|&v| v != 0.0);
        let table = tags.table(table_map).filter(|t| varies && t.has_data())?;
        let glyphs = num_glyphs(table_map).map_or(0, |n| n as usize);
        let Some(deltas) = view.map(Shared::clone) else {
            let measured = Shared::clone(measured.filter(|m| m.varies())?);
            let read = spare.take(glyphs.min(measured.glyphs()));
            return Some(VariedAdvances { source: Source::Phantoms { measured, vertical }, past: 0, read });
        };
        let scalars = deltas.scalars(location);
        let (count, past) = past_advance(&table, &deltas, &scalars);
        let (table, stated) = (table_map.get(tags.mtx)?.clone(), tags.long(table_map));
        let read = spare.take(count);
        Some(VariedAdvances { source: Source::Deltas { table, stated, glyphs, deltas, scalars }, past, read })
    }

    // What a glyph past the table advances by, the delta alone.
    pub(crate) fn past(&self) -> u32 {
        self.past
    }

    #[inline]
    pub(crate) fn of(&self, gid: u32) -> u32 {
        match self.read.get(gid) {
            0 => self.fill(gid),
            held => held - 1,
        }
    }

    // Out of line, so a glyph read again carries none of the first read's work.
    #[cold]
    #[inline(never)]
    fn fill(&self, gid: u32) -> u32 {
        if gid as usize >= self.read.len() {
            return self.past;
        }
        let advance = self.vary(gid);
        self.read.set(gid, advance + 1);
        advance
    }

    // The advances' table, to give back to the font's spares.
    pub(crate) fn take_read(&mut self) -> GlyphSlots {
        core::mem::take(&mut self.read)
    }

    // A glyph's advance at the location: one past glyf's glyphs advances by nothing.
    fn vary(&self, gid: u32) -> u32 {
        match &self.source {
            Source::Deltas { table, stated, glyphs, deltas, scalars } => {
                let gid = gid as usize;
                let stored = MetricsTable::read(table, *stated, *glyphs).map_or(0, |t| t.advance(gid));
                let delta = round_fixed(deltas.delta(gid, (0, gid), scalars));
                i32::from(stored).saturating_add(delta).max(0) as u32
            }
            Source::Phantoms { measured, vertical } => {
                measured.of(gid).map_or(0, |m| m.advances[usize::from(*vertical)])
            }
        }
    }

    // A glyph measured for its phantom points is charged to the location's measured glyphs.
    pub(crate) fn heap_bytes(&self) -> usize {
        let source = match &self.source {
            Source::Deltas { scalars, .. } => crate::daecore::daekit::base::sync::heap_of(scalars),
            Source::Phantoms { .. } => 0,
        };
        source + self.read.heap_bytes()
    }
}

// VORG at a location as `vary_vorg` writes it, each origin moved by VVAR's VOrg delta and rounded once. Glyphs from
// `end` on move alike and take the origin at `end`.
struct VorgAt {
    vorg: TableBytes,
    deltas: DeltaView,
    scalars: Vec<i64>,
    end: usize,
}

impl VorgAt {
    fn new(table_map: &Tables, location: &[f64]) -> Option<VorgAt> {
        let (vorg, vvar) = (table_map.get("VORG")?, table_map.get("VVAR")?);
        let map_off = read_u32_be(vvar, 20)? as usize;
        if map_off == 0 {
            return None;
        }
        let deltas = DeltaView::read(vvar, read_u32_be(vvar, 4)? as usize, Some(map_off))?;
        let table = Vorg::read(vorg)?;
        let listed = table.records().map(|(gid, _)| gid as usize + 1).max().unwrap_or(0);
        let end = deltas.extent().max(listed).min(1 << (8 * table.width));
        Some(VorgAt { vorg: vorg.clone(), scalars: deltas.scalars(location), deltas, end })
    }

    // Read again rather than kept, as each location's cache holds one of these.
    fn width(&self) -> usize {
        Vorg::read(&self.vorg).map_or(2, |t| t.width)
    }

    // The last glyph the table's width names.
    fn last(&self) -> usize {
        (1 << (8 * self.width())) - 1
    }

    // For a glyph below `end`, or at it up to `last`.
    fn origin(&self, gid: usize) -> i16 {
        self.moved(gid, self.deltas.delta(gid, (0, gid), &self.scalars))
    }

    // Its stored origin moved by its `delta`.
    fn moved(&self, gid: usize, delta: i64) -> i16 {
        let stored = Vorg::read(&self.vorg).map_or(0, |t| t.origin(gid as u32));
        clamp_i16(i32::from(stored).saturating_add(round_fixed(delta)))
    }

    // The origin every glyph past the map that VORG doesn't list takes: its default, moved by the map's last entry.
    fn past(&self) -> i16 {
        let stored = Vorg::read(&self.vorg).map_or(0, |t| t.default);
        let delta = self.deltas.delta(self.end, (0, self.end), &self.scalars);
        clamp_i16(i32::from(stored).saturating_add(round_fixed(delta)))
    }
}

// The same, each origin worked out when first read.
pub(crate) struct VariedOrigins {
    at: VorgAt,
    // The origin's bits plus one for each glyph to `end`, 0 until read; threads filling one write the same.
    read: GlyphSlots,
}

impl VariedOrigins {
    // None where `finish` leaves VORG as it is; the origins' table is taken from `spare`. It holds the glyphs the map
    // or the font counts, not every glyph up to a record far past both.
    pub(crate) fn new(table_map: &Tables, location: &[f64], spare: &SpareSlots) -> Option<VariedOrigins> {
        let at = location.iter().any(|&v| v != 0.0).then(|| VorgAt::new(table_map, location)).flatten()?;
        let counted = at.deltas.extent().max(num_glyphs(table_map).map_or(0, |n| n as usize));
        let read = spare.take(at.end.min(at.last()).min(counted) + 1);
        Some(VariedOrigins { at, read })
    }

    pub(crate) fn origin(&self, gid: u32) -> i16 {
        let held = self.read.len() as u32 - 1;
        // A glyph past the slots but before `end` is worked out each time it's read.
        if gid > held && (held as usize) < self.at.end.min(self.at.last()) {
            return self.at.origin(gid as usize);
        }
        let gid = gid.min(held);
        match self.read.get(gid) {
            0 => {
                let origin = self.at.origin(gid as usize);
                self.read.set(gid, u32::from(origin as u16) + 1);
                origin
            }
            held => (held - 1) as u16 as i16,
        }
    }

    // The origins' table, to give back to the font's spares.
    pub(crate) fn take_read(&mut self) -> GlyphSlots {
        core::mem::take(&mut self.read)
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        self.at.deltas.heap_bytes() + heap_of(&self.at.scalars) + self.read.heap_bytes()
    }
}

// HVAR's lsb and VVAR's tsb mappings, read only where a font has no phantom points to give them
// (CFF2); a NULL mapping means the font varies no bearings.
pub(crate) fn apply_bearing_var(
    table_map: &Tables,
    var_tag: &str,
    mtx_data: &mut [u8],
    count: usize,
    long_metrics: usize,
    location: &[f64],
) -> Result<(), Error> {
    let Some(var) = table_map.get(var_tag) else { return Ok(()) };
    let map_off = read_u32_be(var, 12)
        .ok_or_else(|| Error::owned(crate::daecore::ErrorKind::Malformed, format!("{}: header truncated", var_tag)))?
        as usize;
    if map_off == 0 {
        return Ok(());
    }
    let source = DeltaSource::read(var, read_u32_be(var, 4).unwrap_or(0) as usize, Some(map_off))?;
    let scalars = source.scalars(location);
    let mut delta = source.glyph_deltas(&scalars);
    for gid in 0..count {
        if let Some(at) = bearing_at(long_metrics, gid) {
            bump_i16(mtx_data, at, round_fixed(delta(gid)));
        }
    }
    Ok(())
}

// VORG at the location, each origin moved by VVAR's VOrg delta and rounded once. Glyphs past the map and records move
// alike, so theirs becomes the default; a record is written for each glyph that differs, in the source's format. Past
// the map only a listed glyph can differ, so only those are read there.
pub(crate) fn vary_vorg(table_map: &Tables, location: &[f64]) -> Option<Vec<u8>> {
    let at = VorgAt::new(table_map, location)?;
    let (w, default, mapped) = (at.width(), at.past(), at.deltas.extent().min(at.end));
    let listed: BTreeSet<usize> = Vorg::read(&at.vorg)?.records().map(|(gid, _)| gid as usize).collect();
    let mut delta = at.deltas.glyph_deltas(&at.scalars);
    let records: Vec<(usize, i16)> = (0..mapped)
        .chain(listed.into_iter().filter(|&gid| gid >= mapped))
        .map(|gid| (gid, at.moved(gid, delta(gid))))
        .filter(|&(_, y)| y != default)
        .collect();
    if records.len() > at.last() {
        return None;
    }
    let mut out = Vec::with_capacity(6 + w + (w + 2) * records.len());
    out.extend_from_slice(&at.vorg[..4]);
    out.extend_from_slice(&default.to_be_bytes());
    out.extend_from_slice(&(records.len() as u32).to_be_bytes()[4 - w..]);
    for (gid, y) in records {
        out.extend_from_slice(&(gid as u32).to_be_bytes()[4 - w..]);
        out.extend_from_slice(&y.to_be_bytes());
    }
    Some(out)
}
