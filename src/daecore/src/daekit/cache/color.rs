use super::*;
use crate::daecore::daetype::colr_v1::{Deltas, Lists, PaintVisitor, Walk};
use crate::daecore::daetype::format::ivs::DeltaView;

type DrawnScalars = Option<Shared<Vec<i64>>>;

// Whether drawing takes the color tables, settled on first ask, as the sanitizer's check walks every paint; with the
// tables, where v1 walks start and its deltas.
pub(crate) struct ColorTables {
    colr: Option<TableBytes>,
    cpal: Option<TableBytes>,
    lists: Option<Lists>,
    drawn: bool,
    // The points a color glyph's scene may hold and decode, as `scene_points` gives them.
    points: usize,
    palettes: Once<bool>,
    // COLR's store as drawing reads it, loaded at the first location drawn, and the last location's scalars there.
    drawn_store: Once<Option<DeltaView>>,
    drawn_scalars: Mutable<Option<(AxisKey, DrawnScalars)>>,
}

// What any scene may hold and decode in points.
const SCENE_POINTS: usize = 1 << 20;

// Each glyph a scene draws is held once, but composites repeat a glyph without end, so a scene is refused past 16
// points a byte of the outline tables, and a million.
fn scene_points(stored: &Tables) -> usize {
    let bytes: usize = ["glyf", "CFF ", "CFF2", "VARC"].iter().filter_map(|t| stored.get(t)).map(|t| t.len()).sum();
    SCENE_POINTS.max(bytes.saturating_mul(16))
}

// Where a color glyph is read: at a set of axis values, or at a normalized location.
#[derive(Clone, Copy)]
pub(crate) enum ColrAt<'a> {
    Drawn(&'a AxisKey),
    Normalized(&'a [f64]),
}

// The store and the scalars a walk's deltas take where it lies, held while it walks.
struct Scalars<'a>(Option<(&'a DeltaView, Shared<Vec<i64>>)>);

impl<'a> Scalars<'a> {
    fn deltas(&'a self) -> Deltas<'a> {
        Deltas::Drawn(self.0.as_ref().map(|(store, scalars)| (*store, scalars.as_slice())))
    }
}

impl FontCache {
    fn color_tables(&self) -> Option<&ColorTables> {
        use crate::daecore::daetype::color_load::colr_drawn;
        let build = || {
            let (colr, cpal) = (self.bytes("COLR"), self.bytes("CPAL"));
            let drawn = |c: &TableBytes| colr_drawn(c, cpal);
            (colr.is_some() || cpal.is_some()).then(|| {
                Shared::new(ColorTables {
                    colr: self.table("COLR").cloned(),
                    cpal: self.table("CPAL").cloned(),
                    lists: colr.and_then(Lists::of),
                    drawn: self.table("COLR").is_some_and(drawn),
                    points: scene_points(&self.font.tables),
                    palettes: Once::new(),
                    drawn_store: Once::new(),
                    drawn_scalars: mutable(None),
                })
            })
        };
        once_slot(&self.color_tables, build, |_| {}).as_deref()
    }

    fn bytes(&self, tag: &str) -> Option<&[u8]> {
        self.table(tag).map(|t| t.as_slice())
    }

    // Whether drawing reads the font's COLR, which extents then read too.
    pub(crate) fn colr_drawn(&self) -> bool {
        self.color_tables().is_some_and(|t| t.drawn)
    }

    fn colr_read(&self) -> Option<&ColorTables> {
        self.color_tables().filter(|_| self.colr_drawn())
    }

    // Whether the sanitizer passes the font's CPAL, which palette info reports.
    pub(crate) fn palettes_read(&self) -> bool {
        use crate::daecore::daetype::format::sanitize::cpal_passes;
        self.color_tables().is_some_and(|t| *t.palettes.get_or_init(|| self.bytes("CPAL").is_some_and(cpal_passes)))
    }

    // Glyphs a font is drawn with: glyf's by drawn loca, CFF's and CFF2's by CharStrings, else maxp's. A CFF2 font
    // with VARC keeps maxp's where CharStrings hold no more, as VARC draws past them.
    pub(crate) fn drawn_glyph_count(&self) -> u32 {
        if let Some(loca) = self.loca_offsets() {
            return loca.len().saturating_sub(1) as u32;
        }
        let charstrings = match self.cff.is_some() {
            true => self.cff_outlines().map(|o| o.num_glyphs()),
            false => self.cff2_parsed().map(|o| o.num_glyphs()),
        };
        let past = |n: &usize| self.cff.is_some() || *n > self.num_glyphs as usize || !self.varc_loads();
        charstrings.filter(past).map_or(self.num_glyphs, |n| n as u32)
    }

    // The store and its scalars at a normalized location, none at the default.
    fn colr_located_scalars<'a>(
        &self, tables: &'a ColorTables, location: &[f64],
    ) -> Option<(&'a DeltaView, Shared<Vec<i64>>)> {
        use crate::daecore::daetype::format::ivs::fixed_coords;
        if location.iter().all(|&c| c == 0.0) {
            return None;
        }
        let store = self.colr_drawn_store(tables)?;
        let scalars = || Shared::new(store.scalars_at(&fixed_coords(location)));
        Some((store, memo_last(&self.colr_v1_scalars, location, scalars)))
    }

    // The store as drawing reads it and its scalars at `key`. None at the default, which draws as a face no location
    // is set on, and where gvar refuses every location, which leaves the coordinates unset.
    fn colr_drawn_scalars<'a>(
        &self, tables: &'a ColorTables, key: &AxisKey,
    ) -> Option<(&'a DeltaView, Shared<Vec<i64>>)> {
        if key.is_empty() || (!self.cff2 && self.drawn_gvar().is_err()) {
            return None;
        }
        let scalars = memo_last(&tables.drawn_scalars, key, || {
            let coords = crate::daecore::daetype::instancer::drawn_coordinates(self.tables(), key)?;
            Some(Shared::new(self.colr_drawn_store(tables)?.scalars_at(&coords)))
        })?;
        Some((tables.drawn_store.get()?.as_ref()?, scalars))
    }

    fn colr_drawn_store<'a>(&self, tables: &'a ColorTables) -> Option<&'a DeltaView> {
        let load = || {
            let colr = self.table("COLR")?;
            let header = crate::daecore::daetype::colr_v1::header(colr)?;
            let store = DeltaView::read(colr, header.var_store?, header.var_index_map)?;
            self.held.built.add(store.heap_bytes());
            Some(store)
        };
        tables.drawn_store.get_or_init(load).as_ref()
    }

    fn colr_scalars<'a>(&self, tables: &'a ColorTables, at: ColrAt<'_>) -> Scalars<'a> {
        Scalars(match at {
            ColrAt::Normalized(location) => self.colr_located_scalars(tables, location),
            ColrAt::Drawn(key) => self.colr_drawn_scalars(tables, key),
        })
    }

    pub(crate) fn colr_v1_paint(
        &self, gid: u32, key: &AxisKey, palette_index: u16,
    ) -> Option<crate::daecore::daetype::colr_v1::PaintGraph> {
        self.colr_v1_walk(gid, ColrAt::Drawn(key), palette_index, |walk, root| walk.paint(root))
    }

    pub(crate) fn colr_v1_visit(
        &self, gid: u32, key: &AxisKey, palette_index: u16, visitor: &mut (impl PaintVisitor + ?Sized),
    ) -> bool {
        let visit = |walk: &mut Walk, root| walk.visit(Some(root), visitor);
        self.colr_v1_walk(gid, ColrAt::Drawn(key), palette_index, visit).is_some()
    }

    // Color glyph `gid`'s version 1 paint graph walked where it lies, `f` handed the walk and its root's offset: as
    // drawn, or for extents, counting every fill alike. None for a glyph with no version 1 paint drawing takes.
    pub(crate) fn colr_v1_walk<R>(
        &self,
        gid: u32,
        at: ColrAt<'_>,
        palette_index: u16,
        f: impl FnOnce(&mut Walk, usize) -> R,
    ) -> Option<R> {
        let drawn = matches!(at, ColrAt::Drawn(_));
        let tables = self.colr_read()?;
        let (colr, lists) = (tables.colr.as_deref()?, tables.lists?);
        let scalars = self.colr_scalars(tables, at);
        let deltas = scalars.deltas();
        let cpal = tables.cpal.as_deref();
        Walk::over(colr, cpal, lists, gid, palette_index, deltas, drawn, f)
    }

    // The points a color glyph's scene may hold and decode, worked out once a font.
    pub(crate) fn scene_points(&self) -> usize {
        self.color_tables().map_or(SCENE_POINTS, |tables| tables.points)
    }

    // A color glyph's extents: its clip box, else the box its version 1 paint covers, else the box of
    // its version 0 layers, all 0 for one that paints nothing. Read at this cache's location. None past
    // `scene_points` points drawn and decoded in all, where drawing refuses its scene.
    pub(crate) fn colr_extents(&self, gid: u32) -> Option<GlyphExtents> {
        use crate::daecore::daetype::paint::colr::{layers_box, painted_box};
        let colr = self.table("COLR").filter(|_| self.colr_drawn())?;
        let most = self.scene_points();
        let location = self.location.as_deref().map_or(&[][..], |l| &l.coords[..]);
        if let Some([x_min, y_min, x_max, y_max]) = self.colr_clip_box(gid, ColrAt::Normalized(location)) {
            return Some(GlyphExtents {
                x_bearing: x_min,
                y_bearing: y_max,
                width: x_max.saturating_sub(x_min),
                height: y_min.saturating_sub(y_max),
            });
        }
        // A glyph that draws nothing covers nothing. Each glyph is measured once, however many layers name it.
        let (mut boxes, mut work) = (alloc::collections::BTreeMap::new(), 0);
        let mut outline_box = |g: u32| {
            *boxes.entry(g).or_insert_with(|| {
                let within = work <= most;
                within.then(|| self.drawn_bounds(g, &mut work).flatten()).flatten()
            })
        };
        let clip_box = |g| self.colr_clip_box(g, ColrAt::Normalized(location));
        // A glyph with a version 1 record is measured by it, whatever its paint reads, never by its version 0 layers.
        let painted = self.colr_v1_walk(gid, ColrAt::Normalized(location), 0, |walk, root| {
            painted_box(walk, Some(root), &clip_box, &mut outline_box)
        });
        let painted = match painted {
            Some(painted) => painted,
            None => {
                let layers = crate::daecore::daetype::colr_v0::base_glyph_layers(colr, gid)?;
                layers_box(layers.iter().map(|&[g0, g1, ..]| u16::from_be_bytes([g0, g1]).into()), &mut outline_box)
            }
        };
        if work > most {
            return None;
        }
        // Edges round half away from 0, as an outline's do, so a mirrored box mirrors.
        Some(painted.map_or_else(GlyphExtents::default, |b| GlyphExtents::from_bounds(b, |v| v.round())))
    }

    // A color glyph's clip box, as drawn or for extents.
    pub(crate) fn colr_clip_box(&self, gid: u32, at: ColrAt<'_>) -> Option<[i32; 4]> {
        let tables = self.colr_read()?;
        let colr = tables.colr.as_deref()?;
        let scalars = self.colr_scalars(tables, at);
        crate::daecore::daetype::colr_v1::clip_box(colr, gid, scalars.deltas())
    }
}
