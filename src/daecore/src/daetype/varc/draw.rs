// A VARC glyph drawn: each component's transform composed onto its parent's in 16.16, each leaf loaded at its
// component's coordinates and moved point by point, unhinted.
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::{multiply, Matrix, Varc, LOAD};
use crate::daecore::daetype::instancer::drawn_mul_fix as mul_fix;
use crate::daecore::daetype::outline::{OutlinePen, Path};

// A component's matrix and its offset in 26.6, its parents' composed onto it.
type Placed = (Matrix, (i64, i64));

// How many of the records being drawn are scanned for a glyph inside itself, those past them looked up in a set: a
// glyph's few levels never fill it, and a deep one checks in log time.
const SCANNED: usize = 16;

// The outlines a VARC glyph's components draw with.
pub(crate) trait Leaves {
    // Glyph `gid`'s own outline at `coords`, each point moved by `m` and then `by` whole units, appended to `out`; Err
    // where it doesn't load. What loading it took, loaded or not, adds to `work`.
    fn leaf(&self, gid: u32, coords: Option<&[i64]>, m: &Matrix, by: (i64, i64), out: &mut Path, work: &mut usize)
    -> Result<(), ()>;

    // Whether a leaf that fails to load fails at any coordinates, so that a draw loads it once.
    fn fails_alike(&self) -> bool {
        false
    }
}

// A record being drawn: its glyph, the components left to read, where its parent placed it, and whether the component
// naming it set the coordinates it draws at.
struct Frame<'a> {
    gid: u32,
    record: &'a [u8],
    p: usize,
    placed: Option<Placed>,
    set: bool,
}

struct Walk<'a, L> {
    varc: &'a Varc<'a>,
    leaves: &'a L,
    coords: Option<&'a [i64]>,
    font: &'a [i64],
    // The records around the one being drawn, outermost first, which `run` holds apart so a glyph one level deep
    // allocates none, and the glyphs of those past the first SCANNED: none of these may be named again inside them.
    frames: Vec<Frame<'a>>,
    deep: BTreeSet<u32>,
    // The coordinates the components naming the records being drawn set, the last those the innermost draws at.
    set: Vec<Vec<i64>>,
    // Coordinates of components done with, each kept for the next, as most components set their own; where none
    // does, none is kept.
    spare: Vec<Vec<i64>>,
    // Leaves that failed to load, where they fail alike.
    failed: BTreeSet<u32>,
}

// Covered glyph `gid` drawn at the font's coordinates `font` into `out`. None where it isn't covered, its record
// fails or the draw runs past its budget, where the glyph draws its own outline instead.
pub(crate) fn draw(varc: &Varc, leaves: &impl Leaves, font: Option<&[i64]>, gid: u32, out: &mut Path) -> Option<()> {
    varc.start();
    let record = varc.record(gid)?;
    let mut walk = Walk {
        varc, leaves, coords: font, font: font.unwrap_or(&[]), frames: Vec::new(), deep: BTreeSet::new(),
        set: Vec::new(), spare: Vec::new(), failed: BTreeSet::new(),
    };
    let top = Frame { gid, record, p: 0, placed: None, set: false };
    kept(out, |out| walk.run(top, out).ok_or(())).ok()
}

impl<'a, L: Leaves> Walk<'a, L> {
    // Whether glyph `gid` is being drawn, by record `frame` or one around it.
    fn drawing(&self, frame: &Frame, gid: u32) -> bool {
        frame.gid == gid || self.frames.iter().take(SCANNED).any(|f| f.gid == gid) || self.deep.contains(&gid)
    }

    // Each record's components in order, each placed where its parent lies, a covered one's record drawn in its place
    // unless it's already being drawn: one cut short ends its record, one that doesn't load is left out.
    fn run(&mut self, mut frame: Frame<'a>, out: &mut Path) -> Option<()> {
        loop {
            let c = (frame.p < frame.record.len()).then(|| self.varc.component(frame.record, &mut frame.p)).flatten();
            if self.varc.spent() {
                return None;
            }
            let Some(mut c) = c else {
                if self.frames.len() >= SCANNED {
                    self.deep.remove(&frame.gid);
                }
                if frame.set {
                    self.spare.extend(self.set.pop());
                }
                let Some(parent) = self.frames.pop() else { return Some(()) };
                frame = parent;
                continue;
            };
            let coords = self.set.last().map(Vec::as_slice).or(self.coords);
            let Some((m, offset)) = self.varc.place(&mut c, coords) else { continue };
            let mut own = self.spare.pop().unwrap_or_default();
            let set = coords.is_some_and(|parent| self.varc.coords(&c, parent, self.font, &mut own));
            let placed = match frame.placed {
                Some((pm, (px, py))) => {
                    let (x, y) = offset;
                    let dx = mul_fix(x, pm[0]).saturating_add(mul_fix(y, pm[1])).saturating_add(px);
                    let dy = mul_fix(x, pm[2]).saturating_add(mul_fix(y, pm[3])).saturating_add(py);
                    (multiply(&pm, &m), (dx, dy))
                }
                None => (m, offset),
            };
            if !self.varc.spend(LOAD) {
                return None;
            }
            let record = self.varc.record(c.gid).filter(|_| !self.drawing(&frame, c.gid));
            if let Some(record) = record {
                if set {
                    self.set.push(own);
                } else if own.capacity() > 0 {
                    self.spare.push(own);
                }
                let child = Frame { gid: c.gid, record, p: 0, placed: Some(placed), set };
                self.frames.push(core::mem::replace(&mut frame, child));
                if self.frames.len() >= SCANNED {
                    self.deep.insert(c.gid);
                }
                continue;
            }
            // A glyph inside itself, or one whose record fails, is its own outline.
            if !self.failed.contains(&c.gid) {
                let (m, (dx, dy)) = placed;
                let by = (dx.saturating_add(32) >> 6, dy.saturating_add(32) >> 6);
                let at = if set { Some(own.as_slice()) } else { coords };
                let mut work = 0;
                let leaf = kept(out, |out| self.leaves.leaf(c.gid, at, &m, by, out, &mut work));
                self.varc.spend_leaf(work);
                if leaf.is_err() && self.leaves.fails_alike() {
                    self.failed.insert(c.gid);
                }
            }
            if own.capacity() > 0 {
                self.spare.push(own);
            }
        }
    }
}

// `f`'s drawing into `out`, none of it kept where it fails.
fn kept<T, E>(out: &mut Path, f: impl FnOnce(&mut Path) -> Result<T, E>) -> Result<T, E> {
    let (verbs, points) = (out.parts().0.len(), out.parts().1.len());
    let drawn = f(out);
    if drawn.is_err() {
        out.truncate(verbs, points);
    }
    drawn
}

// A point moved as a loaded component's is: each product rounded as 16.16, then the offset added. A point off the
// unit grid, as a glyf composite's scaled one can be here, takes the plain product.
pub(crate) fn moved(m: &Matrix, by: (i64, i64), x: f64, y: f64) -> (f64, f64) {
    let product = |v: f64, k: i64| match (v as i64) as f64 == v {
        true => mul_fix(v as i64, k) as f64,
        false => v * k as f64 / 65536.0,
    };
    (product(x, m[0]) + product(y, m[1]) + by.0 as f64, product(x, m[2]) + product(y, m[3]) + by.1 as f64)
}

// A pen that moves each point as `moved` does.
pub(crate) struct Moving<'a> {
    pub(crate) pen: &'a mut dyn OutlinePen,
    pub(crate) m: Matrix,
    pub(crate) by: (i64, i64),
}

impl Moving<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = moved(&self.m, self.by, f64::from(x), f64::from(y));
        (x as f32, y as f32)
    }
}

impl OutlinePen for Moving<'_> {
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

#[cfg(test)]
#[path = "../../../tests/unit/daetype/varc/draw.rs"]
mod tests;
