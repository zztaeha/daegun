use super::*;
use crate::daecore::daetype::format::glyf::GLYPH_WORK;
use crate::daecore::daetype::instancer::{drawn_coordinates, AxisTag, DeltaSource};
use crate::daecore::daetype::outline::{glyph_points_varied, OutlinePen, Path, Points, Stored, Vary};
use crate::daecore::daetype::varc::draw::{draw, moved, Leaves, Moving};
use crate::daecore::daetype::varc::{drawn_budget, Matrix, Varc};

// The points a thread's last leaf was collected in, lent to its next, as one glyph draws many; one past
// `SPARE_POINTS_BYTES` is let go, as the path a thread keeps is.
#[cfg(feature = "std")]
std::thread_local! {
    static SPARE_POINTS: core::cell::Cell<Option<Points>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE_POINTS.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_POINTS_BYTES: usize = 64 << 10;

#[cfg(feature = "std")]
fn take_points() -> Points {
    SPARE_POINTS.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default()
}

#[cfg(feature = "std")]
fn give_points(points: Points) {
    if points.heap_bytes() <= SPARE_POINTS_BYTES {
        let _ = SPARE_POINTS.try_with(|spare| spare.set(Some(points)));
    }
}

#[cfg(not(feature = "std"))]
fn take_points() -> Points {
    Points::default()
}

#[cfg(not(feature = "std"))]
fn give_points(_: Points) {}

impl FontCache {
    fn varc(&self) -> Option<Varc<'_>> {
        if !self.font.varc {
            return None;
        }
        Varc::new(self.table("VARC")?.as_slice())
    }

    pub(crate) fn varc_loads(&self) -> bool {
        self.varc().is_some()
    }

    // Glyph `gid` drawn from its VARC record at this cache's location, unhinted; None where the font has no VARC or
    // the glyph no working record, which then draws its own outline. Inlined, so a font without VARC pays no call.
    // The decodes the draw took, drawn or not, add to `work`.
    #[inline]
    pub(crate) fn draw_varc(&self, gid: u32, work: &mut usize, pen: &mut dyn OutlinePen) -> Option<()> {
        if !self.font.varc {
            return None;
        }
        self.varc_path(gid, work, &mut 0)?.replay(None, pen);
        Some(())
    }

    // Each of `gids` VARC draws, drawn at `key`, for an instance or subset that leaves VARC out. Each glyph draws as
    // drawing does; all share one `drawn_budget`, which their leaves may pass by one glyph draw's work, past which the
    // instance or subset is refused.
    pub(crate) fn varc_drawn(&self, key: &AxisKey, gids: &[u32]) -> Result<BTreeMap<u32, Path>, Error> {
        let own = drawn_budget(self.font.tables.iter().map(|(_, t)| t.len()).sum());
        let mut left = [own.saturating_add(GLYPH_WORK), own];
        self.at_location(key, |at| {
            let mut drawn = BTreeMap::new();
            for &gid in gids {
                let (mut spent, mut leaves) = (0, 0);
                if let Some(path) = at.varc_path(gid, &mut spent, &mut leaves) {
                    drawn.insert(gid, path);
                }
                let past = || Error::limit("VARC: the glyphs drawn in its place run past their budget");
                left[0] = left[0].checked_sub(spent).ok_or_else(past)?;
                left[1] = left[1].checked_sub(spent - leaves).ok_or_else(past)?;
            }
            Ok(drawn)
        })
    }

    // Glyph `gid` as VARC draws it, and the decodes the draw `spent`, drawn or not, `leaves` of them its leaves'.
    fn varc_path(&self, gid: u32, spent: &mut usize, leaves: &mut usize) -> Option<Path> {
        let varc = self.varc()?;
        let mut path = Path::default();
        // A glyph's components hold a few hundred points, which the path then doesn't grow to one at a time.
        path.reserve(64, 256);
        let drawn = draw(&varc, self, self.varc_coords(), gid, &mut path);
        *spent += varc.work.get();
        *leaves += varc.leaves.get();
        drawn.map(|()| path)
    }

    // Whether VARC covers the glyph, and whether it draws it, which then isn't hinted, as VARC holds no hints for the
    // glyph it composes; the first inlined as `draw_varc` is.
    #[inline]
    pub(crate) fn varc_covers(&self, gid: u32) -> bool {
        self.font.varc && self.varc().is_some_and(|v| v.coverage(gid).is_some())
    }

    pub(crate) fn varc_draws(&self, gid: u32) -> bool {
        self.varc_covers(gid) && self.varc_path(gid, &mut 0, &mut 0).is_some()
    }

    // Glyph `gid`'s extents through VARC at this cache's location: the box of the outline VARC draws there, its edges
    // rounded half away from 0. None where VARC leaves the glyph to its own outline.
    pub(crate) fn varc_extents(&self, gid: u32) -> Option<GlyphExtents> {
        use crate::daecore::daetype::{format::ivs::fixed_coords, outline::PointBounds};
        let varc = self.varc()?;
        let location = self.location.as_deref().map(|l| &l.coords).filter(|l| !l.is_empty()).map(|l| fixed_coords(l));
        let mut path = Path::default();
        draw(&varc, self, location.as_deref().or_else(|| self.varc_coords()), gid, &mut path)?;
        let mut bounds = PointBounds::default();
        path.replay(None, &mut bounds);
        let edges = bounds.finish().map(|(x0, y0, x1, y1)| [x0, y0, x1, y1].map(f64::from));
        Some(edges.map_or_else(GlyphExtents::default, |b| GlyphExtents::from_bounds(b, |v| v.round())))
    }

    // Where VARC components start from: this location's normalized coordinates, fvar's at the default; without fvar
    // gvar's axes, or its CFF2 store's, all at 0; none with neither. Worked out once, from the font's own tables.
    fn varc_coords(&self) -> Option<&[i64]> {
        let default = || once_slot(&self.font.varc_default, || self.varc_default(), |_| {}).as_deref();
        self.cff2_coords().or(self.varc_at.as_deref()).or_else(default)
    }

    fn varc_default(&self) -> Option<Vec<i64>> {
        if let Some(default) = drawn_coordinates(&self.font.tables, &[]) {
            return Some(default);
        }
        let axes = match self.cff2 {
            true => self.cff2_outlines()?.store_axes(),
            false => self.drawn_gvar().ok()??.axes(),
        };
        (axes > 0).then(|| alloc::vec![0; axes])
    }

    fn glyf_leaf(
        &self, gid: u32, coords: Option<&[i64]>, m: &Matrix, by: (i64, i64), out: &mut Path, work: &mut usize,
    ) -> Result<(), ()> {
        let (glyf, loca) = (self.glyf.as_ref().ok_or(())?, self.loca_offsets().ok_or(())?);
        let loca = loca.into();
        let source = coords.and_then(|c| DeltaSource::passing(c, self.drawn_gvar(), self.drawn_advances()));
        let extended = self.glyf_extended;
        let (last, before) = Default::default();
        let passing = source.map(|source| super::drawn::Passing { source, extended, last, before });
        let vary: &dyn Vary = passing.as_ref().map_or(&Stored, |p| p);
        // The scratch drawing lends every glyf glyph, as a glyph's leaves each want one.
        let (mut scratch, mut left) = (self.take_scratch(), GLYPH_WORK);
        let drawn = self.drawn_phantom::<false>(glyf, loca, gid, vary, &mut scratch, 0, &mut left).and_then(|origin| {
            Ok((origin, glyph_points_varied(glyf, extended, loca, gid, &mut scratch, vary, take_points(), work)?))
        });
        self.give_scratch(scratch);
        *work = work.saturating_add(GLYPH_WORK - left);
        let (origin, mut points) = drawn.map_err(drop)?;
        for (x, y) in points.x.iter_mut().zip(&mut points.y) {
            (*x, *y) = moved(m, by, *x - f64::from(origin), *y);
        }
        points.draw(out);
        give_points(points);
        Ok(())
    }
}

impl Leaves for FontCache {
    // A leaf loads as any glyph the font counts, through the outlines drawing reads; its work is its charstring steps,
    // or the glyphs and points glyf reads and the gvar tuples moving them.
    fn leaf(
        &self, gid: u32, coords: Option<&[i64]>, m: &Matrix, by: (i64, i64), out: &mut Path, work: &mut usize,
    ) -> Result<(), ()> {
        use crate::daecore::daetype::outline::{outline_cff2_glyph_counted, outline_cff_glyph_counted};
        if gid >= self.drawn_glyph_count() {
            return Err(());
        }
        let mut moving = Moving { pen: out, m: *m, by };
        if let Some(cff) = self.cff() {
            outline_cff_glyph_counted(self.cff_outlines().ok_or(())?, cff, gid, work, &mut moving).map_err(drop)?;
        } else if self.cff2 {
            let (outlines, cff2) = (self.cff2_outlines().ok_or(())?, self.table("CFF2").ok_or(())?);
            // Coordinates all at 0 blend as none do.
            let coords = coords.filter(|c| c.iter().any(|&v| v != 0));
            outline_cff2_glyph_counted(outlines, cff2, gid, coords, None, work, &mut moving).map_err(drop)?;
        } else {
            return self.glyf_leaf(gid, coords, m, by, out, work);
        }
        Ok(())
    }

    // A CFF2 charstring's blends can pick the subroutines it calls, so only glyf's and CFF's leaves fail alike
    // everywhere: a glyf leaf's deltas fail only past its budget, which spends the draw.
    fn fails_alike(&self) -> bool {
        !self.cff2
    }
}

// As `varc_drawn`, for a font the instancer or subsetter reads from its tables alone.
pub(crate) fn varc_drawn_from(
    map: &Tables, at: &[(AxisTag, f64)], gids: &[u32],
) -> Result<BTreeMap<u32, Path>, Error> {
    FontCache::new(map.clone()).map_or_else(|_| Ok(BTreeMap::new()), |font| font.varc_drawn(&AxisKey::of(at), gids))
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/varc.rs"]
mod tests;
