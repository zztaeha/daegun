use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::daecore::daetype::format::glyf::GlyfLoca;
use super::super::decoder::{glyf_tags, read_u16_be};
use super::super::format::metrics::shaped_tags;
use super::drawn::Gvar;
use super::gvar::{gvar_source, GlyfAt, Measured};
use crate::daecore::daetype::{TableBytes, Tables};
use crate::daecore::daekit::base::sync::{heap_of, once_slot, Counter, Once, Shared};

// A glyf font's own tables for measuring its glyphs, which every location shares: gvar as drawing reads it; glyf and
// loca as shaping reads them; hmtx and vmtx with their counts of long records; and the em.
pub(crate) struct GlyfTables {
    gvar: Option<Gvar>,
    glyf: TableBytes,
    extended: bool,
    loca: GlyfLoca,
    metrics: [Option<(TableBytes, usize)>; 2],
    upem: i32,
}

impl GlyfTables {
    // None without glyf. A gvar whose header doesn't read varies nothing, as one the font lacks.
    pub(crate) fn load(table_map: &Tables, loca: GlyfLoca) -> Option<GlyfTables> {
        let tags = glyf_tags(table_map);
        let glyf = table_map.get(tags.glyf)?.clone();
        let gvar = Gvar::load(table_map).ok().flatten();
        let metrics = [false, true].map(|vertical| {
            let tags = shaped_tags(table_map, vertical);
            tags.header(table_map)?;
            Some((table_map.get(tags.mtx)?.clone(), tags.long(table_map)))
        });
        let upem = table_map.get("head").and_then(|h| read_u16_be(h, 18)).map_or(1000, i32::from);
        Some(GlyfTables { gvar, glyf, extended: tags.extended, loca, metrics, upem })
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.gvar.as_ref().map_or(0, Gvar::heap_bytes)
    }

    fn at<'a>(&'a self, coords: &'a [i64], scalars: &'a [i64]) -> GlyfAt<'a> {
        fn metrics(m: &Option<(TableBytes, usize)>) -> Option<(&[u8], usize)> {
            m.as_ref().map(|(mtx, long)| (mtx.as_slice(), *long))
        }
        let source = self.gvar.as_ref().filter(|_| coords.iter().any(|&c| c != 0)).map(|gvar| {
            super::drawn::DeltaSource { coords, gvar: Some(gvar), scalars: Some(scalars), hvar: false, vvar: false }
        });
        GlyfAt {
            source,
            glyf: &self.glyf,
            extended: self.extended,
            loca: (&self.loca).into(),
            horizontal: metrics(&self.metrics[0]),
            vertical: metrics(&self.metrics[1]),
            upem: self.upem,
        }
    }
}

// A location's glyf glyphs as measured there, each measured when first read and kept, in pages
// of cells made as a glyph in each is first read: a read takes no lock, as every vertical origin reads one.
pub(crate) struct MeasuredGlyphs {
    tables: Shared<GlyfTables>,
    coords: Vec<i64>,
    scalars: Vec<i64>,
    pages: Vec<Once<Box<Page>>>,
    bytes: Counter,
}

const PAGE: usize = 32;
type Page = [Once<Option<Measured>>; PAGE];

impl MeasuredGlyphs {
    pub(crate) fn new(tables: Shared<GlyfTables>, location: &[f64]) -> MeasuredGlyphs {
        let (mut coords, mut scalars) = (Vec::new(), Vec::new());
        gvar_source(tables.gvar.as_ref(), location, &mut coords, &mut scalars);
        let pages: Vec<_> = (0..tables.loca.len().saturating_sub(1).div_ceil(PAGE)).map(|_| Once::new()).collect();
        let bytes = Counter::new(heap_of(&coords) + heap_of(&scalars) + heap_of(&pages));
        MeasuredGlyphs { tables, coords, scalars, pages, bytes }
    }

    fn at(&self) -> GlyfAt<'_> {
        self.tables.at(&self.coords, &self.scalars)
    }

    // None past glyf's glyphs.
    pub(crate) fn of(&self, gid: u32) -> Option<Measured> {
        let gid = gid as usize;
        let page = self.pages.get(gid / PAGE)?;
        let page = once_slot(page, || Box::new([const { Once::new() }; PAGE]), |_| self.bytes.add(size_of::<Page>()));
        *once_slot(&page[gid % PAGE], || self.at().measure(gid), |_| {})
    }

    // The glyph's points at the location as TrueType numbers them, read from it and its components as an instance
    // writes them; None past glyf's glyphs.
    pub(crate) fn points(&self, gid: u32) -> Option<(Vec<f64>, Vec<f64>)> {
        let (glyf, loca) = self.at().instanced(gid as usize)?;
        crate::daecore::daetype::outline::glyph_points(&glyf, self.tables.extended, (&loca).into(), gid).ok()
    }

    // The box of the glyph's drawn outline at the location, None inside for one with no points; None where its outline
    // fails or past glyf's glyphs. What its walks spent adds to `work`.
    pub(crate) fn bounds(&self, gid: u32, work: &mut usize) -> Option<Option<[f64; 4]>> {
        let bounds = self.at().bounds(gid as usize, work)?;
        Some(bounds.map(|(x0, y0, x1, y1)| [x0, y0, x1, y1].map(f64::from)))
    }

    // How many glyphs glyf holds; past them a glyph has no metrics of its own.
    pub(crate) fn glyphs(&self) -> usize {
        self.tables.loca.len().saturating_sub(1)
    }

    // Whether gvar reads, so that advances HVAR or VVAR don't give come from the phantom points.
    pub(crate) fn varies(&self) -> bool {
        self.tables.gvar.is_some()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes.get()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/measured.rs"]
mod tests;
