use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use super::pen::OutlinePen;
use super::super::decoder::read_i16_be;
use super::super::instancer::{extract_coords_into, GlyphCoords};
#[cfg(test)]
use {super::super::decoder::glyf_tags, crate::daecore::daetype::Tables};
use crate::daecore::daekit::base::sync::Shared;
use super::super::format::glyf::{
    drawn_fits, glyph_bytes, simple_point_count, Component, ComponentPath, Components, LocaRef, Stack,
};
use super::super::format::glyf::{GLYPH_WORK, MAX_VISITS};
use crate::daecore::Error;

#[cfg(test)]
pub fn outline_glyf_glyph_with_loca(
    table_map: &Tables,
    loca:      &[usize],
    gid:       u32,
    pen:       &mut dyn OutlinePen,
) -> Result<(), Error> {
    let tags = glyf_tags(table_map);
    let glyf = table_map.get(tags.glyf).ok_or(Error::missing("glyf: missing glyf table"))?;
    outline_glyf_bytes(glyf, tags.extended, loca, gid, pen)
}

/// Draws glyph `gid` out of glyf bytes and loca's offsets, as [`parse_loca`](crate::raw::parse_loca) reads them,
/// composites resolved; `extended` where the bytes are GLYF's, whose composites can name a glyph in 24 bits.
///
/// # Errors
///
/// The decoder's, for a glyph that doesn't decode.
// The points are decoded into the coordinates the thread's last call grew.
pub fn outline_glyf_bytes(
    glyf:     &[u8],
    extended: bool,
    loca:     &[usize],
    gid:      u32,
    pen:      &mut dyn OutlinePen,
) -> Result<(), Error> {
    #[cfg(feature = "std")]
    let mut scratch = SPARE_COORDS.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
    #[cfg(not(feature = "std"))]
    let mut scratch = GlyphCoords::default();
    let drawn = outline_glyf_glyph_reusing_bytes(glyf, extended, loca, gid, &mut scratch, pen);
    // A thread past its locals' teardown drops them instead, as it does one huge glyph's.
    #[cfg(feature = "std")]
    if scratch.heap_bytes() <= SPARE_COORDS_BYTES {
        let _ = SPARE_COORDS.try_with(|spare| spare.set(Some(scratch)));
    }
    drawn
}

#[cfg(feature = "std")]
std::thread_local! {
    static SPARE_COORDS: core::cell::Cell<Option<GlyphCoords>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE_COORDS.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_COORDS_BYTES: usize = 64 << 10;

pub fn outline_glyf_glyph_reusing_bytes(
    glyf: &[u8],
    extended: bool,
    loca: &[usize],
    gid: u32,
    scratch: &mut GlyphCoords,
    pen: &mut dyn OutlinePen,
) -> Result<(), Error> {
    let budget = &mut Budget { extended, ..Budget::default() };
    draw_component(glyf, loca.into(), gid, budget, scratch, pen, false)
}

// As `outline_glyf_glyph_reusing_bytes`, each glyph's points moved as `vary` moves them at a location. The glyphs it
// read and the points it decoded, drawn or not, add to `work`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn outline_glyf_glyph_varied(
    glyf: &[u8],
    extended: bool,
    loca: LocaRef<'_>,
    gid: u32,
    scratch: &mut GlyphCoords,
    vary: &dyn Vary,
    work: &mut usize,
    pen: &mut dyn OutlinePen,
) -> Result<(), Error> {
    let budget = &mut Budget { vary, extended, ..Budget::default() };
    let drawn = draw_component(glyf, loca, gid, budget, scratch, pen, false);
    *work += (MAX_VISITS as u32 - budget.visits) as usize + (GLYPH_WORK - budget.points);
    drawn
}

// As `outline_glyf_glyph_varied` into the outline's box alone, which it fails or succeeds alike for: a glyph placed by
// shifts alone gives the box of its points, which its curves never leave, rather than its curves. Also the glyphs it
// visited and the points it charged, for a caller drawing many to bound them together.
pub(crate) fn glyf_glyph_box_varied(
    glyf: &[u8], extended: bool, loca: LocaRef<'_>, gid: u32, scratch: &mut GlyphCoords, vary: &dyn Vary,
    bounds: &mut super::path::PointBounds,
) -> (Result<(), Error>, [usize; 2]) {
    let budget = &mut Budget { vary, extended, ..Budget::default() };
    let drawn = draw_component(glyf, loca, gid, budget, scratch, bounds, true);
    (drawn, [MAX_VISITS - budget.visits as usize, GLYPH_WORK - budget.points])
}

// A glyph's points as a location's variation moves them, kept to be moved again before they're drawn. The glyphs it
// read, the points it decoded and the deltas it worked out, drawn or not, add to `work`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn glyph_points_varied(
    glyf: &[u8], extended: bool, loca: LocaRef<'_>, gid: u32, scratch: &mut GlyphCoords, vary: &dyn Vary,
    mut points: Points, work: &mut usize,
) -> Result<Points, Error> {
    points.clear();
    let budget = &mut Budget { vary, extended, ..Budget::default() };
    let drawn = collect(glyf, loca, Start::Glyph(gid), &mut ComponentPath::default(), budget, scratch, &mut points);
    *work += (MAX_VISITS as u32 - budget.visits) as usize + (GLYPH_WORK - budget.points);
    drawn.map(|()| points)
}

// A glyph's points as TrueType numbers them, a composite's components placed in order: what a contour
// point index names.
pub(crate) fn glyph_points(
    glyf: &[u8], extended: bool, loca: LocaRef<'_>, gid: u32,
) -> Result<(Vec<f64>, Vec<f64>), Error> {
    let mut points = Points::default();
    let budget = &mut Budget { extended, ..Budget::default() };
    let (start, path) = (Start::Glyph(gid), &mut ComponentPath::default());
    collect(glyf, loca, start, path, budget, &mut GlyphCoords::default(), &mut points)?;
    Ok((points.x, points.y))
}

// A glyph as a location's variation reads it: its decoded points and contour ends, a composite's record, or nothing
// to draw.
pub(crate) enum Shape<'a> {
    Empty,
    Simple { x: &'a [i32], y: &'a [i32], ends: &'a [usize] },
    Composite(&'a [u8]),
}

impl<'a> Shape<'a> {
    // As `extract_coords_into` leaves them, which every caller has just run: its vectors hold the points alone.
    pub(crate) fn simple(c: &'a GlyphCoords) -> Shape<'a> {
        debug_assert_eq!((c.x_coords.len(), c.y_coords.len()), (c.num_points, c.num_points));
        Shape::Simple { x: &c.x_coords, y: &c.y_coords, ends: &c.end_pts }
    }
}

// A glyph's deltas in whole units, and finer in 26.6.
pub(crate) type Deltas = Option<Moves>;
pub(crate) type Fine = Option<Shared<[(i32, i32)]>>;

// A glyph's whole-unit deltas: its own, or its run of those held for every glyph of an instance, which then takes one
// allocation rather than one a glyph.
#[derive(Clone)]
pub(crate) struct Moves {
    all: Shared<[(i16, i16)]>,
    at: core::ops::Range<usize>,
}

impl Moves {
    pub(crate) fn within(all: &Shared<[(i16, i16)]>, at: core::ops::Range<usize>) -> Moves {
        Moves { all: all.clone(), at }
    }
}

impl From<Shared<[(i16, i16)]>> for Moves {
    fn from(all: Shared<[(i16, i16)]>) -> Moves {
        Moves { at: 0..all.len(), all }
    }
}

impl core::ops::Deref for Moves {
    type Target = [(i16, i16)];

    fn deref(&self) -> &[(i16, i16)] {
        self.all.get(self.at.clone()).unwrap_or_default()
    }
}

// How a location moves a glyph's points: each by a whole-unit delta, phantom points last, None where none moves. Err
// where the glyph doesn't draw there, as where working the deltas out runs past the work `left` to the draw.
pub(crate) trait Vary {
    fn deltas(&self, gid: u32, shape: Shape<'_>, left: &mut usize) -> Result<Deltas, Error>;

    // The deltas where they're known without the glyph's shape, what working them out took spent from `left` again
    // where it's kept beyond one call, so a draw spends alike whatever was drawn before it.
    fn known(&self, gid: u32, left: &mut usize) -> Option<Result<Deltas, Error>>;

    // Whether the glyphs sit off the default, where hinting scales every simple glyph's points from 26.6.
    fn located(&self) -> bool {
        false
    }

    // The deltas unrounded, in 26.6, which hinting scales from.
    fn deltas_f26dot6(&self, _: u32, _: Shape<'_>, _: &mut usize) -> Result<Fine, Error> {
        Ok(None)
    }

    // Those in 26.6 where they're known without the glyph's shape, spent as `known` spends.
    fn known_f26dot6(&self, _: u32, _: &mut usize) -> Option<Result<Fine, Error>> {
        None
    }

    // An advance across or down as HVAR or VVAR varies it, which places hinting's phantom points; None where neither
    // does, and gvar moves those points instead.
    fn advance(&self, _: u32, _: bool, _: u16) -> Option<u16> {
        None
    }

    // A simple glyph decoded and moved, where that's known without decoding it again.
    fn decoded(&self, _: u32) -> Option<Decoded> {
        None
    }
}

// A draw of glyph `drawn` at a location whose deltas are kept beyond it, each use spending again what working them out
// took: the draw spends each glyph's once, whole and in 26.6, however many composites name it.
pub(crate) struct Paying<'a, V: ?Sized> {
    vary: &'a V,
    drawn: u32,
    paid: [Paid; 2],
}

impl<'a, V: Vary + ?Sized> Paying<'a, V> {
    pub(crate) fn new(vary: &'a V, drawn: u32) -> Self {
        Paying { vary, drawn, paid: Default::default() }
    }

    // Paid for where working them out `spent` any, unless it's the glyph drawn, which is never inside itself.
    fn paying(&self, fine: bool, gid: u32, spent: bool) {
        if spent && gid != self.drawn {
            self.paid[usize::from(fine)].add(gid);
        }
    }
}

impl<V: Vary + ?Sized> Vary for Paying<'_, V> {
    fn deltas(&self, gid: u32, shape: Shape<'_>, left: &mut usize) -> Result<Deltas, Error> {
        if self.paid[0].holds(gid)
            && let Some(known) = self.vary.known(gid, &mut { usize::MAX })
        {
            return known;
        }
        let before = *left;
        let deltas = self.vary.deltas(gid, shape, left);
        self.paying(false, gid, *left != before);
        deltas
    }

    fn known(&self, gid: u32, left: &mut usize) -> Option<Result<Deltas, Error>> {
        if self.paid[0].holds(gid) {
            return self.vary.known(gid, &mut { usize::MAX });
        }
        let before = *left;
        let known = self.vary.known(gid, left);
        self.paying(false, gid, *left != before);
        known
    }

    fn located(&self) -> bool {
        self.vary.located()
    }

    fn deltas_f26dot6(&self, gid: u32, shape: Shape<'_>, left: &mut usize) -> Result<Fine, Error> {
        if self.paid[1].holds(gid)
            && let Some(known) = self.vary.known_f26dot6(gid, &mut { usize::MAX })
        {
            return known;
        }
        let before = *left;
        let fine = self.vary.deltas_f26dot6(gid, shape, left);
        self.paying(true, gid, *left != before);
        fine
    }

    fn advance(&self, gid: u32, vertical: bool, advance: u16) -> Option<u16> {
        self.vary.advance(gid, vertical, advance)
    }

    fn decoded(&self, gid: u32) -> Option<Decoded> {
        self.vary.decoded(gid)
    }
}

// Glyphs a draw has paid for: the first few held in place, so a real composite allocates nothing for them.
#[derive(Default)]
struct Paid {
    near: [Cell<u32>; 4],
    len: Cell<usize>,
    far: RefCell<Option<BTreeSet<u32>>>,
}

impl Paid {
    fn holds(&self, gid: u32) -> bool {
        let len = self.len.get();
        let near = || self.near[..len.min(4)].iter().any(|held| held.get() == gid);
        len != 0 && (near() || len > 4 && self.far.borrow().as_ref().is_some_and(|far| far.contains(&gid)))
    }

    fn add(&self, gid: u32) {
        let len = self.len.get();
        match self.near.get(len) {
            Some(held) => held.set(gid),
            None => drop(self.far.borrow_mut().get_or_insert_default().insert(gid)),
        }
        self.len.set(len + 1);
    }
}

// A simple glyph's point count and the box `points_box` takes of its points as moved.
#[derive(Clone, Copy)]
pub(crate) struct Decoded {
    pub(crate) points: usize,
    pub(crate) bounds: Option<[i32; 4]>,
}

// The glyphs as stored, at the default or in a font that doesn't vary.
pub(crate) struct Stored;

impl Vary for Stored {
    fn deltas(&self, _: u32, _: Shape<'_>, _: &mut usize) -> Result<Deltas, Error> {
        Ok(None)
    }

    fn known(&self, _: u32, _: &mut usize) -> Option<Result<Deltas, Error>> {
        Some(Ok(None))
    }
}

// What drawing one glyph may do: the glyphs it reads, and the points it decodes and the deltas moving them, from GLYF
// or glyf, as its location moves them; the points and contours its outline has gathered.
struct Budget<'a> {
    visits: u32,
    points: usize,
    extended: bool,
    vary: &'a dyn Vary,
    gathered: (usize, usize),
}


impl Default for Budget<'_> {
    fn default() -> Self {
        Budget { visits: MAX_VISITS as u32, points: GLYPH_WORK, extended: false, vary: &Stored, gathered: (0, 0) }
    }
}

impl Budget<'_> {
    // A glyph's bytes, charged as one visit: the glyph asked for, or a component of the composites on `path`, refused
    // where it's one of them.
    fn visit<'a>(
        &mut self, glyf: &'a [u8], loca: LocaRef<'_>, gid: u32, path: Option<&ComponentPath>,
    ) -> Result<&'a [u8], Error> {
        if path.is_some_and(|path| path.holds(gid)) {
            return Err(Error::malformed("glyf: composite glyph inside itself"));
        }
        self.visits = self.visits.checked_sub(1).ok_or(Error::limit("glyf: composite glyph work budget exhausted"))?;
        // Past glyf's glyphs the glyph asked for has no outline; a component there is the font's error.
        glyph_bytes(glyf, loca, gid).ok_or(match path {
            None => Error::new(crate::daecore::ErrorKind::NoOutline, "glyf: glyph index out of range"),
            Some(_) => Error::malformed("glyf: glyph index out of range"),
        })
    }

    // A simple glyph's points charged, decoded as they are whether they fit or not, then gathered into the outline
    // with its contours.
    fn gather(&mut self, points: usize, contours: usize) -> Result<(), Error> {
        self.spend(points)?;
        if !drawn_fits(self.gathered, points, contours) {
            return Err(Error::malformed("glyf: the glyph holds more points or contours than an outline can"));
        }
        self.gathered = (self.gathered.0 + points, self.gathered.1 + contours);
        Ok(())
    }

    fn spend(&mut self, points: usize) -> Result<(), Error> {
        self.points =
            self.points.checked_sub(points).ok_or(Error::limit("glyf: composite glyph point budget exhausted"))?;
        Ok(())
    }

    // A glyph's deltas, what working them out takes spent from the points' budget.
    fn deltas(&mut self, gid: u32, shape: Shape<'_>) -> Result<Deltas, Error> {
        self.vary.deltas(gid, shape, &mut self.points)
    }
}

// A glyph's contour count, None for one that draws nothing: an empty record or one of 0 contours. A record under its
// 10-byte header is refused.
fn contours(glyph: &[u8]) -> Result<Option<i16>, Error> {
    if glyph.is_empty() {
        return Ok(None);
    }
    let n_contours =
        glyph.get(..10).and_then(|h| read_i16_be(h, 0)).ok_or(Error::malformed("glyf: glyph header truncated"))?;
    Ok(Some(n_contours).filter(|&n| n != 0))
}

// One composite being drawn: the components left, its deltas, and whether it's placed by shifts alone where `boxed`.
struct Drawing<'g> {
    components: Components<'g>,
    moved: Deltas,
    index: usize,
    boxed: bool,
}

// `boxed` where the pen takes only a box and every transform on the way to it is a shift. A glyph that isn't a
// composite is drawn as it's read, so only a composite sets up the walk.
fn draw_component(
    glyf: &[u8], loca: LocaRef<'_>, gid: u32, budget: &mut Budget, scratch: &mut GlyphCoords,
    pen: &mut dyn OutlinePen, boxed: bool,
) -> Result<(), Error> {
    let glyph = budget.visit(glyf, loca, gid, None)?;
    match contours(glyph)? {
        Some(..=-1) => draw_composite(glyf, loca, (gid, glyph), budget, scratch, pen, boxed),
        n_contours => draw_leaf((gid, glyph, n_contours), 0, budget, scratch, pen, boxed),
    }
}

// A glyph with no components, `placed` transforms deep, its contour count as `contours` reads it. Inlined, as a call
// cost a simple glyph's draw 1%.
#[inline(always)]
fn draw_leaf(
    (gid, glyph, n_contours): (u32, &[u8], Option<i16>), placed: usize, budget: &mut Budget,
    scratch: &mut GlyphCoords, pen: &mut dyn OutlinePen, boxed: bool,
) -> Result<(), Error> {
    let Some(n_contours) = n_contours else { return budget.deltas(gid, Shape::Empty).map(drop) };
    let n = n_contours as usize;
    if boxed && let Some(known) = budget.vary.decoded(gid) {
        budget.spend(n)?;
        budget.gather(known.points, n)?;
        budget.spend(known.points * placed.saturating_sub(1))?;
        draw_box(known.bounds, pen);
    } else {
        decode_simple(glyph, gid, n, budget, scratch)?;
        move_points(gid, scratch, budget)?;
        budget.spend(scratch.num_points * placed.saturating_sub(1))?;
        match boxed {
            true => draw_box(points_box(scratch, None), pen),
            false => draw_points(scratch, budget.extended, pen),
        }
    }
    Ok(())
}

// A composite's components drawn in order, walked on the heap, each component's transform kept while it's drawn.
#[allow(clippy::too_many_arguments)]
fn draw_composite(
    glyf: &[u8], loca: LocaRef<'_>, (gid, glyph): (u32, &[u8]), budget: &mut Budget, scratch: &mut GlyphCoords,
    pen: &mut dyn OutlinePen, boxed: bool,
) -> Result<(), Error> {
    let mut path = ComponentPath::default();
    // Two levels in place, as nearly all real fonts nest one or two deep: setting up eight cost instancing Inter 0.6%.
    let (mut inside, mut placed) = (Stack::<Drawing, 2>::default(), Stack::<[f64; 6], 2>::default());
    let (mut gid, mut glyph, mut boxed) = (gid, glyph, boxed);
    loop {
        let mut via;
        let pen: &mut dyn OutlinePen = match placed.is_empty() {
            true => &mut *pen,
            false => {
                via = Placed { pen: &mut *pen, placed: &placed };
                &mut via
            }
        };
        match contours(glyph)? {
            Some(..=-1) => {
                let moved = budget.deltas(gid, Shape::Composite(glyph))?;
                if !needs_points(glyph, budget.extended) {
                    path.enter(gid);
                    let components = Components::new(glyph, budget.extended);
                    inside.push(Drawing { components, moved, index: 0, boxed });
                } else {
                    let mut points = Points::default();
                    collect(glyf, loca, Start::Composite(gid, glyph, moved), &mut path, budget, scratch, &mut points)?;
                    budget.spend(points.x.len() * placed.len().saturating_sub(1))?;
                    points.draw(pen);
                }
            }
            n_contours => draw_leaf((gid, glyph, n_contours), placed.len(), budget, scratch, pen, boxed)?,
        }
        // The next component, out of each composite whose components are all drawn.
        loop {
            let Some(drawing) = inside.last_mut() else { return Ok(()) };
            if let Some(c) = drawing.components.next() {
                let c = c.map_err(|_| Error::malformed("glyf: composite stream truncated"))?;
                let c = moved(c, drawing.moved.as_deref(), drawing.index);
                drawing.index += 1;
                let (a, b, cc, d) = c.transform();
                let (dx, dy) = c.offset();
                placed.truncate(path.depth() - 1);
                placed.push([a, b, cc, d, dx, dy]);
                (gid, boxed) = (c.gid, drawing.boxed && (a, b, cc, d) == (1.0, 0.0, 0.0, 1.0));
                break;
            }
            inside.pop();
            path.leave();
            placed.truncate(path.depth());
        }
        glyph = budget.visit(glyf, loca, gid, Some(&path))?;
    }
}

// A pen drawn onto through the transforms placing a component, innermost first, each rounded to f32 as nested
// transform pens round them.
struct Placed<'a> {
    pen: &'a mut dyn OutlinePen,
    placed: &'a Stack<[f64; 6], 2>,
}

impl Placed<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        self.placed.iter().rev().fold((x, y), |(x, y), &t| super::pen::transformed(t, x, y))
    }
}

impl OutlinePen for Placed<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.pen.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.pen.line_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let ((cx, cy), (x, y)) = (self.at(cx, cy), self.at(x, y));
        self.pen.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let ((c1x, c1y), (c2x, c2y), (x, y)) = (self.at(c1x, c1y), self.at(c2x, c2y), self.at(x, y));
        self.pen.curve_to(c1x, c1y, c2x, c2y, x, y);
    }
    fn close(&mut self) {
        self.pen.close();
    }
}

fn move_points(gid: u32, coords: &mut GlyphCoords, budget: &mut Budget) -> Result<(), Error> {
    let deltas = match budget.vary.known(gid, &mut budget.points) {
        Some(known) => known?,
        None => budget.deltas(gid, Shape::simple(coords))?,
    };
    let Some(deltas) = deltas else { return Ok(()) };
    for ((x, y), &(dx, dy)) in coords.x_coords.iter_mut().zip(&mut coords.y_coords).zip(deltas.iter()) {
        *x += i32::from(dx);
        *y += i32::from(dy);
    }
    Ok(())
}

// A component with its offset moved by the composite's deltas, cut to 16 bits as glyf holds it; one placed by its
// points keeps them.
pub(crate) fn moved(c: Component, deltas: Option<&[(i16, i16)]>, i: usize) -> Component {
    match deltas.and_then(|d| d.get(i)).filter(|_| c.has_offset()) {
        Some(&(dx, dy)) => {
            let shift = |v: i32, d: i16| i32::from(v.wrapping_add(i32::from(d)) as i16);
            Component { args: (shift(c.args.0, dx), shift(c.args.1, dy)), ..c }
        }
        None => c,
    }
}

// The contours are charged before they are read, since reading a glyph's end points costs as much
// whether or not its points then decode; a decode that fails has read up to every point all the same.
fn decode_simple(
    glyph: &[u8], gid: u32, n_contours: usize, budget: &mut Budget, coords: &mut GlyphCoords,
) -> Result<(), Error> {
    budget.spend(n_contours)?;
    if coords.decoded.take() != Some(gid) && !extract_coords_into(glyph, n_contours, coords) {
        budget.spend(simple_point_count(glyph, n_contours).unwrap_or(0))?;
        return Err(Error::malformed("glyf: simple glyph truncated, or its end points do not increase"));
    }
    budget.gather(coords.num_points, n_contours)
}

// GLYF's flag for a cubic control, which glyf leaves unread.
const FLAG_CUBIC: u8 = 0x80;

fn draw_points(coords: &GlyphCoords, extended: bool, pen: &mut dyn OutlinePen) {
    if extended && coords.flags.iter().any(|f| f & (FLAG_CUBIC | 1) == FLAG_CUBIC) {
        return draw_cubic_points(coords, pen);
    }
    let mut c_start = 0usize;
    for &c_end in &coords.end_pts {
        if c_end >= coords.num_points { break; }
        draw_contour_over(&GlyfContour { coords, start: c_start, end: c_end }, pen);
        c_start = c_end + 1;
    }
}

// The box of the points `draw_points` draws, each moved by `moves` where given as `move_points` moves it: each point is
// drawn or is a control, and a point the curves imply lies between two of them.
pub(crate) fn points_box(coords: &GlyphCoords, moves: Option<&[(i16, i16)]>) -> Option<[i32; 4]> {
    let drawn = coords.end_pts.iter().take_while(|&&end| end < coords.num_points).last().map_or(0, |&end| end + 1);
    let moved = |i: usize| {
        let (dx, dy) = moves.and_then(|m| m.get(i)).map_or((0, 0), |&(dx, dy)| (i32::from(dx), i32::from(dy)));
        (coords.x_coords[i].saturating_add(dx), coords.y_coords[i].saturating_add(dy))
    };
    (0..drawn).map(moved).fold(None, |held, (x, y)| match held {
        None => Some([x, y, x, y]),
        Some([x0, y0, x1, y1]) => Some([x0.min(x), y0.min(y), x1.max(x), y1.max(y)]),
    })
}

// A box drawn as its two corners.
fn draw_box(bounds: Option<[i32; 4]>, pen: &mut dyn OutlinePen) {
    if let Some([x0, y0, x1, y1]) = bounds {
        pen.move_to(x0 as f32, y0 as f32);
        pen.line_to(x1 as f32, y1 as f32);
    }
}

// A glyph holding cubic controls, kept out of line so the loop above compiles as it does alone.
#[cold]
#[inline(never)]
fn draw_cubic_points(coords: &GlyphCoords, pen: &mut dyn OutlinePen) {
    let mut c_start = 0usize;
    for &c_end in &coords.end_pts {
        if c_end >= coords.num_points { break; }
        draw_cubic_contour_over(&GlyfContour { coords, start: c_start, end: c_end }, pen);
        c_start = c_end + 1;
    }
}

pub trait ContourPoints {
    fn len(&self) -> usize;
    fn get(&self, i: usize) -> (f32, f32, bool);
    // Whether a point off the curve is a cubic control.
    fn cubic(&self, _: usize) -> bool { false }
}

struct GlyfContour<'a> {
    coords: &'a GlyphCoords,
    start: usize,
    end: usize,
}

impl ContourPoints for GlyfContour<'_> {
    fn len(&self) -> usize { self.end - self.start + 1 }
    fn get(&self, i: usize) -> (f32, f32, bool) {
        let k = self.start + i;
        (
            self.coords.x_coords[k] as f32,
            self.coords.y_coords[k] as f32,
            self.coords.flags[k] & 0x01 != 0,
        )
    }
    fn cubic(&self, i: usize) -> bool { self.coords.flags[self.start + i] & FLAG_CUBIC != 0 }
}

pub(crate) fn draw_contour_over<P: ContourPoints + ?Sized>(pts: &P, pen: &mut (impl OutlinePen + ?Sized)) {
    let n = pts.len();
    if n == 0 { return; }

    // TrueType starts a contour at point 0 if it is on-curve, otherwise at the last point if that
    // one is, and only failing both at the midpoint the `None` arm implies.
    let start_idx = if pts.get(0).2 {
        Some(0)
    } else if pts.get(n - 1).2 {
        Some(n - 1)
    } else {
        None
    };
    let (start_x, start_y, walk_from) = match start_idx {
        Some(i) => { let p = pts.get(i); (p.0, p.1, (i + 1) % n) }
        None => {
            let (lx, ly, _) = pts.get(n - 1);
            let (fx, fy, _) = pts.get(0);
            ((lx + fx) * 0.5, (ly + fy) * 0.5, 0)
        }
    };
    pen.move_to(start_x, start_y);

    let steps = if start_idx.is_some() { n - 1 } else { n };
    let mut pending_ctrl: Option<(f32, f32)> = None;
    let mut idx = walk_from;
    for _ in 0..steps {
        let (x, y, on) = pts.get(idx);
        idx += 1;
        if idx == n { idx = 0; }
        if on {
            match pending_ctrl.take() {
                Some((cx, cy)) => pen.quad_to(cx, cy, x, y),
                None => pen.line_to(x, y),
            }
        } else if let Some((cx, cy)) = pending_ctrl.replace((x, y)) {
            let (mx, my) = ((cx + x) * 0.5, (cy + y) * 0.5);
            pen.quad_to(cx, cy, mx, my);
        }
    }
    if let Some((cx, cy)) = pending_ctrl {
        pen.quad_to(cx, cy, start_x, start_y);
    }
    pen.close();
}

type At = (f32, f32);

fn mid(a: At, b: At) -> At {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

// Whether a contour keeps GLYF's cubic rule (the beyond-64k draft): between points on the curve, wrapping around, the
// controls are all cubic or all quadratic, and each unbroken run of cubic ones is even. Counted from past a point
// that isn't a cubic control, so a run through the contour's first point counts whole.
fn cubic_form_holds<P: ContourPoints + ?Sized>(pts: &P) -> bool {
    let n = pts.len();
    let kind = |i: usize| {
        let on = pts.get(i).2;
        (on, !on && pts.cubic(i))
    };
    let from = (0..n).find(|&i| !kind(i).1).map_or(0, |i| i + 1);
    let mut run = 0usize;
    for i in (from..from + n).map(|k| k % n) {
        let ((on, cubic), (next_on, next_cubic)) = (kind(i), kind((i + 1) % n));
        if cubic {
            run += 1;
        } else if !run.is_multiple_of(2) {
            return false;
        } else {
            run = 0;
        }
        if !on && !next_on && cubic != next_cubic {
            return false;
        }
    }
    run.is_multiple_of(2)
}

// A contour by GLYF's cubic rule: two cubic controls in a row make a cubic, a point on the curve implied between
// pairs; quadratic ones draw as in glyf. One breaking the rule, which the draft leaves undefined, draws as quadratic.
pub(crate) fn draw_cubic_contour_over<P: ContourPoints + ?Sized>(pts: &P, pen: &mut (impl OutlinePen + ?Sized)) {
    let n = pts.len();
    if n == 0 || !cubic_form_holds(pts) {
        return draw_contour_over(pts, pen);
    }
    let at = |i: usize| {
        let (x, y, _) = pts.get(i);
        (x, y)
    };
    // From point 0 or the last where on the curve, as glyf starts, else the first on it; with none, all are cubic,
    // and the first control starts a pair.
    let on = |i: usize| pts.get(i).2;
    let first = if on(0) { Some(0) } else if on(n - 1) { Some(n - 1) } else { (0..n).find(|&i| on(i)) };
    let (start, from, steps) = match first {
        Some(s) => (at(s), s + 1, n - 1),
        None => (mid(at(n - 1), at(0)), 0, n),
    };
    pen.move_to(start.0, start.1);
    let mut pending = Pending::default();
    for i in (from..from + steps).map(|k| k % n) {
        let (x, y, on) = pts.get(i);
        let p = (x, y);
        if on {
            pending.to(p, pen);
        } else if pts.cubic(i) {
            match (pending.first, pending.second) {
                (Some(_), None) => pending.second = Some(p),
                (Some(_), Some(b)) => {
                    pending.to(mid(b, p), pen);
                    pending.first = Some(p);
                }
                _ => pending.first = Some(p),
            }
        } else if let Some(q) = pending.quad.replace(p) {
            let m = mid(q, p);
            pen.quad_to(q.0, q.1, m.0, m.1);
        }
    }
    if pending.quad.is_some() || pending.first.is_some() {
        pending.to(start, pen);
    }
    pen.close();
}

// The controls since the last point on the curve: a quadratic one, or a cubic pair.
#[derive(Default)]
struct Pending {
    quad: Option<At>,
    first: Option<At>,
    second: Option<At>,
}

impl Pending {
    // The segment from the last point on the curve to `p`, through what's pending.
    fn to(&mut self, p: At, pen: &mut (impl OutlinePen + ?Sized)) {
        match (self.quad.take(), self.first.take(), self.second.take()) {
            (Some(q), ..) => pen.quad_to(q.0, q.1, p.0, p.1),
            (None, Some(a), Some(b)) => pen.curve_to(a.0, a.1, b.0, b.1, p.0, p.1),
            _ => pen.line_to(p.0, p.1),
        }
    }
}

fn needs_points(glyph: &[u8], extended: bool) -> bool {
    Components::new(glyph, extended).map_while(Result::ok).any(|c| !c.has_offset())
}

// A composite's points, kept rather than streamed, for a component placed by matching one of its
// points to one of the points before it.
#[derive(Default)]
pub(crate) struct Points {
    pub(crate) x: Vec<f64>,
    pub(crate) y: Vec<f64>,
    on: Vec<bool>,
    cubic: Vec<bool>,
    pub(crate) ends: Vec<usize>,
}

impl Points {
    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
        self.on.clear();
        self.cubic.clear();
        self.ends.clear();
    }

    #[cfg(feature = "std")]
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.x) + heap_of(&self.y) + heap_of(&self.on) + heap_of(&self.cubic) + heap_of(&self.ends)
    }

    // `child` after these points, shifted by (dx, dy), its contours numbered on from them.
    fn append(&mut self, child: Points, (dx, dy): (f64, f64)) {
        let base = self.x.len();
        self.x.extend(child.x.iter().map(|x| x + dx));
        self.y.extend(child.y.iter().map(|y| y + dy));
        self.on.extend(child.on);
        self.cubic.extend(child.cubic);
        self.ends.extend(child.ends.iter().map(|e| base + e));
    }

    pub(crate) fn draw(&self, pen: &mut dyn OutlinePen) {
        let mut start = 0;
        for &end in &self.ends {
            let contour = PointsContour { points: self, start, end };
            if self.cubic[start..=end].contains(&true) {
                draw_cubic_contour_over(&contour, pen);
            } else {
                draw_contour_over(&contour, pen);
            }
            start = end + 1;
        }
    }
}

struct PointsContour<'a> {
    points: &'a Points,
    start: usize,
    end: usize,
}

impl ContourPoints for PointsContour<'_> {
    fn len(&self) -> usize { self.end - self.start + 1 }
    fn get(&self, i: usize) -> (f32, f32, bool) {
        let k = self.start + i;
        (self.points.x[k] as f32, self.points.y[k] as f32, self.points.on[k])
    }
    fn cubic(&self, i: usize) -> bool { self.points.cubic[self.start + i] }
}

// Where collecting starts: a glyph to read, or a composite already read with its deltas.
enum Start<'g> {
    Glyph(u32),
    Composite(u32, &'g [u8], Deltas),
}

// A composite being collected: the components left, its deltas, its points so far, and the component being read.
struct Collecting<'g> {
    components: Components<'g>,
    moved: Deltas,
    index: usize,
    points: Points,
    reading: Option<Component>,
}

// A glyph's points onto `out`, composites walked on the heap. As the spec places a component by points, the child's
// point (its own numbering, after its matrix) lands on the parent's point (numbered across the components before it).
#[allow(clippy::too_many_arguments)]
fn collect(
    glyf: &[u8], loca: LocaRef<'_>, start: Start<'_>, path: &mut ComponentPath, budget: &mut Budget,
    scratch: &mut GlyphCoords, out: &mut Points,
) -> Result<(), Error> {
    let mut inside = Stack::<Collecting, 8>::default();
    let mut next = None;
    match start {
        Start::Glyph(gid) => next = Some(gid),
        Start::Composite(gid, glyph, moved) => collecting(&mut inside, path, gid, glyph, moved, budget.extended, out),
    }
    loop {
        let mut read = None;
        if let Some(gid) = next.take() {
            let glyph = budget.visit(glyf, loca, gid, Some(&*path).filter(|path| path.depth() > 0))?;
            match contours(glyph)? {
                None => {
                    budget.deltas(gid, Shape::Empty)?;
                    read = Some(outermost(&inside, out));
                }
                Some(n_contours @ 1..) => {
                    decode_simple(glyph, gid, n_contours as usize, budget, scratch)?;
                    move_points(gid, scratch, budget)?;
                    let mut points = outermost(&inside, out);
                    let base = points.x.len();
                    for k in 0..scratch.num_points {
                        points.x.push(f64::from(scratch.x_coords[k]));
                        points.y.push(f64::from(scratch.y_coords[k]));
                        points.on.push(scratch.flags[k] & 0x01 != 0);
                        points.cubic.push(budget.extended && scratch.flags[k] & (FLAG_CUBIC | 1) == FLAG_CUBIC);
                    }
                    points.ends.extend(scratch.end_pts.iter().map(|&e| base + e));
                    read = Some(points);
                }
                Some(_) => {
                    let moved = budget.deltas(gid, Shape::Composite(glyph))?;
                    collecting(&mut inside, path, gid, glyph, moved, budget.extended, out);
                }
            }
        }
        // A glyph read whole goes to the composite reading it, past the outermost each level placing it once more.
        loop {
            let deep = inside.len() > 1;
            let Some(parent) = inside.last_mut() else {
                *out = read.take().unwrap_or_default();
                return Ok(());
            };
            if let Some(child) = read.take()
                && let Some(c) = parent.reading.take()
            {
                if deep {
                    budget.spend(child.x.len())?;
                }
                parent.place(c, child)?;
            }
            let Some(c) = parent.components.next() else {
                read = inside.pop().map(|done| done.points);
                path.leave();
                continue;
            };
            let c = c.map_err(|_| Error::malformed("glyf: composite stream truncated"))?;
            let c = moved(c, parent.moved.as_deref(), parent.index);
            (parent.index, parent.reading, next) = (parent.index + 1, Some(c), Some(c.gid));
            break;
        }
    }
}

// The outermost glyph is read into `out`'s own buffers, kept from glyph to glyph; a component into new ones.
fn outermost(inside: &Stack<Collecting<'_>, 8>, out: &mut Points) -> Points {
    match inside.is_empty() {
        true => core::mem::take(out),
        false => Points::default(),
    }
}

fn collecting<'g>(
    inside: &mut Stack<Collecting<'g>, 8>, path: &mut ComponentPath, gid: u32, glyph: &'g [u8], moved: Deltas,
    extended: bool, out: &mut Points,
) {
    path.enter(gid);
    let (components, points) = (Components::new(glyph, extended), outermost(inside, out));
    inside.push(Collecting { components, moved, index: 0, points, reading: None });
}

impl Collecting<'_> {
    // The points of the component being read, through its matrix, then shifted by its offset or onto the point it
    // matches.
    fn place(&mut self, c: Component, mut child: Points) -> Result<(), Error> {
        let (a, b, cc, d) = c.transform();
        for k in 0..child.x.len() {
            let (x, y) = (child.x[k], child.y[k]);
            child.x[k] = x * a + y * cc;
            child.y[k] = x * b + y * d;
        }
        let out = &mut self.points;
        let (dx, dy) = if c.has_offset() {
            c.offset()
        } else {
            let (parent, own) = (c.args.0 as usize, c.args.1 as usize);
            let (Some(&px), Some(&py), Some(&cx), Some(&cy)) =
                (out.x.get(parent), out.y.get(parent), child.x.get(own), child.y.get(own))
            else {
                return Err(Error::malformed("glyf: a matched component point is past its glyph's points"));
            };
            (px - cx, py - cy)
        };
        out.append(child, (dx, dy));
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/outline/glyf_pen.rs"]
mod tests;

