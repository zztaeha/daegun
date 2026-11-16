use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::daekit::draw::bitmap::{MAX_DRAW_BYTES, MAX_SIDE};
use crate::daecore::daekit::draw::subpixel::MAX_OVERSAMPLE;
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path, Verb};

use super::spans;
use super::sweep::{height_key, Chain, Pt, Sweep, ENTER, LEAVE, LINE, PASS};
use core::cell::Cell;

// A row's height, and a pixel's area, in the fixed point coverage is summed in. Each y is kept from its row's top,
// so the sums stay small whatever the image's size.
pub(super) const ONE: i32 = 1 << 16;

// A curve spans at least a sixteenth of its second difference (Markov's inequality, for a cubic), so one whose square
// passes this spans past MAX_SIDE pixels corner to corner at the most samples a pixel takes.
const WIDE: f32 = (32 * MAX_SIDE as u64 * MAX_OVERSAMPLE as u64).pow(2) as f32;

// How far a chord may stray from its curve, in samples.
pub(super) const TOLERANCE: f64 = 1.0 / 32.0;

/// What a fill may take, in the steps its work is priced in below: 2^27, some 2^31 instructions, and FILL_POINT a
/// point and FILL_PIXEL a pixel besides, so a path its size pays for fills and a hostile one is refused in seconds.
pub(crate) const FILL_BASE: u64 = 1 << 27;
pub(crate) const FILL_POINT: u64 = 1 << 9;
pub(crate) const FILL_PIXEL: u64 = 1 << 3;

/// The allowance of a fill of `points` points over `pixels` pixels.
pub(crate) fn allowance(points: usize, pixels: u64) -> u64 {
    let sized = FILL_POINT.saturating_mul(points as u64).saturating_add(FILL_PIXEL.saturating_mul(pixels));
    FILL_BASE.saturating_add(sized)
}

// What a fill's work costs, in steps of some 16 instructions, a pass over a pixel's sum, each priced as measured.
// A cell an edge crosses: summed, its passages marked, and traced where it's resolved.
pub(super) const CELL: u64 = 10;
// A point: placed, summed, chained, and its events dealt out and taken, each a step through LEVEL more.
const POINT: u64 = 48;
// A pixel: its sum cleared and read, and read again where it's resolved.
const PIXEL: u64 = 2;
// A passage listed and sorted, and a test `verdict` may make of two.
const PASSAGE: u64 = 32;
const TEST: u64 = 2;
// A swap where neighbors cross: popped, swapped and its new neighbors tested, each a step through LEVEL more.
const SWAP: u64 = 96;
// A winding changed beside a level stretch.
const STEP: u64 = 8;
// A step through each level of the order's tree and the crossings' heap, log2 of the lines deep.
const LEVEL: u64 = 4;


// The most pixel edges a chord is walked across one at a time; a longer one is cut row by row, as summing its middle
// pixels in one pass is cheaper.
const STEPS: usize = 3;

// A pixel's mark of the first passage to cross it in a call: the call's generation, then the passage's segments
// past its first, up to 7, and its first, or SHARED once a second has crossed it and both are listed.
const ID_BITS: u32 = 21;
const SHARED: u32 = (1 << ID_BITS) - 1;
const MARK_BITS: u32 = ID_BITS + 3;
const GENERATIONS: u32 = 1 << (32 - MARK_BITS);

// A pair's flag, over its second's last segment, that the walk tested it: one contour's, apart.
const APART: u32 = 1 << 31;

/// A path's contours as polygons in samples, y up, drawn onto as a pen: curves become chords within `TOLERANCE`,
/// their count from Wang's formula, found by comparisons and integer square roots so every build splits a curve alike.
/// Placed in f32 as `begin` takes it, its points held within the draw's bytes; or placed in f64 and cut to a box as
/// `begin_cut` takes it, so only that box's neighborhood reaches f32.
#[derive(Default)]
pub(crate) struct Edges {
    pts: Vec<[f32; 2]>,
    ends: Vec<u32>,
    placed: Vec<[f32; 2]>,
    first: usize,
    open: bool,
    matrix: [f32; 6],
    bounds: [f32; 4],
    unfit: bool,
    too_many: bool,
    far: bool,
    most: usize,
    cutting: bool,
    cut: Cut,
}

impl Edges {
    /// Empties it for a path drawn in units a 2×3 `matrix` `[a, b, c, d, dx, dy]` takes to samples, its points held
    /// within MAX_DRAW_BYTES.
    pub(crate) fn begin(&mut self, matrix: [f32; 6]) {
        self.pts.clear();
        self.ends.clear();
        self.first = 0;
        self.open = false;
        self.matrix = matrix;
        self.unfit = false;
        self.too_many = false;
        self.far = false;
        self.most = MAX_DRAW_BYTES / size_of::<[f32; 2]>();
        self.cutting = false;
    }

    /// Empties it for a path `matrix` takes to samples in f64, cut to `clip`, (x min, y min, x max, y max): what lies
    /// past a side moves onto it, so the winding inside is the path's own, and a curve's chords follow its length
    /// inside. However many, its points are what the box holds of the path.
    pub(crate) fn begin_cut(&mut self, matrix: [f64; 6], clip: [f64; 4]) {
        self.begin([0.0; 6]);
        self.most = usize::MAX;
        (self.cutting, self.cut.m, self.cut.clip, self.cut.keep) = (true, matrix, clip, false);
        self.cut.contour.clear();
    }

    /// Empties it for a path cut as `begin_cut` cuts it, its contours kept in f64 for `keep_into` to hand on rather
    /// than placed in f32.
    pub(crate) fn begin_keep(&mut self, matrix: [f64; 6], clip: [f64; 4]) {
        self.begin_cut(matrix, clip);
        self.cut.keep = true;
        self.cut.kept.clear();
        self.cut.kept_ends.clear();
    }

    /// Whether a point wasn't finite once placed.
    pub(crate) fn unfit(&self) -> bool {
        self.unfit
    }

    /// Whether the points passed what MAX_DRAW_BYTES holds.
    pub(crate) fn too_many(&self) -> bool {
        self.too_many
    }

    /// Whether a curve's second difference passed WIDE, so it reaches past what a lent image holds.
    pub(crate) fn far(&self) -> bool {
        self.far
    }

    // Whether a curve's squared second difference `dd` passes WIDE: it's left a chord, and the path too large to lend.
    fn wide(&mut self, dd: f32) -> bool {
        let wide = dd > WIDE;
        if wide {
            self.far = true;
        }
        wide
    }

    /// How many points it holds.
    pub(crate) fn points(&self) -> usize {
        self.pts.len()
    }

    /// The points' extent, (x min, y min, x max, y max), as `finish` found it; none without a contour.
    pub(crate) fn bounds(&self) -> Option<[f32; 4]> {
        (!self.ends.is_empty()).then_some(self.bounds)
    }

    fn place(&self, x: f32, y: f32) -> [f32; 2] {
        place(self.matrix, x, y)
    }

    // Whether n more points fit what it may hold; if not, the path has too many.
    fn room(&mut self, n: u32) -> bool {
        let fits = !self.too_many && self.pts.len().saturating_add(n as usize) <= self.most;
        self.too_many |= !fits;
        fits
    }

    fn push(&mut self, p: [f32; 2]) {
        if self.pts.last() != Some(&p) || self.pts.len() == self.first {
            self.pts.push(p);
        }
    }

    fn last(&self) -> [f32; 2] {
        self.pts.last().copied().unwrap_or_default()
    }

    // A contour under three points, or one closing on its first, encloses nothing more than without the repeat.
    fn end_contour(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        if self.pts.len() > self.first + 1 && self.pts.last() == self.pts.get(self.first) {
            self.pts.pop();
        }
        if self.pts.len() - self.first < 3 {
            self.pts.truncate(self.first);
        } else {
            self.ends.push(self.pts.len() as u32);
            self.first = self.pts.len();
        }
    }

    fn ensure_open(&mut self) {
        if !self.open {
            let p = self.last();
            self.open = true;
            self.first = self.pts.len();
            self.pts.push(p);
        }
    }
}

// The fewest chords keeping a curve within TOLERANCE, by Wang's formula: the smallest n with n⁴ >= v.
#[allow(clippy::neg_cmp_op_on_partial_ord, reason = "the negated form is what gives NaN one chord")]
fn chords(v: f32) -> u32 {
    if !(v > 1.0) {
        return 1;
    }
    // Most curves need few; counting up is exact and cheaper than two integer square roots.
    for n in 2..16u32 {
        let square = (n * n) as f32;
        if square * square >= v {
            return n;
        }
    }
    // A curve past WIDE is left a chord, so `v` stays under 2^52, where u64 holds it exactly.
    let squared = (v.ceil() as u64 - 1).isqrt() + 1;
    ((squared - 1).isqrt() + 1) as u32
}

// A point in units to samples. Unchecked here: `finish` checks every point at once, and a wild one only caps a
// curve's chords meanwhile.
#[inline(always)]
fn place(m: [f32; 6], x: f32, y: f32) -> [f32; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

fn second_difference(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    let dx = a[0] - 2.0 * b[0] + c[0];
    let dy = a[1] - 2.0 * b[1] + c[1];
    dx * dx + dy * dy
}

impl Edges {
    /// A whole path, as the pen would draw it: every point placed in one pass first, then the verbs flattened.
    #[inline(always)]
    pub(crate) fn path(&mut self, path: &Path) {
        if self.cutting {
            return path.view().replay(None, self);
        }
        let (verbs, points) = path.parts();
        // The matrix copied out, so the loop needn't reload it after every store.
        let m = self.matrix;
        let mut placed = core::mem::take(&mut self.placed);
        placed.clear();
        placed.extend(points.iter().map(|p| place(m, p.x, p.y)));
        let mut at = placed.as_slice();
        // A path holds what its verbs take, so no verb runs out of points.
        for verb in verbs {
            match verb {
                Verb::Move => {
                    let Some(([p], rest)) = at.split_first_chunk() else { break };
                    self.moved(*p);
                    at = rest;
                }
                Verb::Line => {
                    let Some(([p], rest)) = at.split_first_chunk() else { break };
                    self.line(*p);
                    at = rest;
                }
                Verb::Quad => {
                    let Some(([c, p], rest)) = at.split_first_chunk() else { break };
                    self.quad(*c, *p);
                    at = rest;
                }
                Verb::Cubic => {
                    let Some(([c1, c2, p], rest)) = at.split_first_chunk() else { break };
                    self.cubic(*c1, *c2, *p);
                    at = rest;
                }
                Verb::Close => self.end_contour(),
            }
        }
        self.placed = placed;
    }

    #[inline(always)]
    fn moved(&mut self, p: [f32; 2]) {
        self.end_contour();
        self.open = true;
        self.first = self.pts.len();
        self.pts.push(p);
    }

    #[inline(always)]
    fn line(&mut self, p: [f32; 2]) {
        self.ensure_open();
        self.push(p);
    }

    #[inline(always)]
    fn quad(&mut self, c: [f32; 2], b: [f32; 2]) {
        self.ensure_open();
        let a = self.last();
        let dd = second_difference(a, c, b);
        if self.wide(dd) {
            return self.push(b);
        }
        // Wang's k for a quadratic is 1/4, squared here as the bound is taken to the fourth.
        let n = chords(dd * const { (1.0 / 16.0 / (TOLERANCE * TOLERANCE)) as f32 });
        if self.room(n) {
            let h = 1.0 / n as f32;
            let k1 = [2.0 * (c[0] - a[0]), 2.0 * (c[1] - a[1])];
            let k2 = [a[0] - 2.0 * c[0] + b[0], a[1] - 2.0 * c[1] + b[1]];
            self.pts.extend((1..n).map(|i| {
                let t = i as f32 * h;
                [(k2[0] * t + k1[0]) * t + a[0], (k2[1] * t + k1[1]) * t + a[1]]
            }));
            self.pts.push(b);
        }
    }

    #[inline(always)]
    fn cubic(&mut self, c1: [f32; 2], c2: [f32; 2], b: [f32; 2]) {
        self.ensure_open();
        let a = self.last();
        // Wang's k for a cubic is 3/4.
        let dd = second_difference(a, c1, c2).max(second_difference(c1, c2, b));
        if self.wide(dd) {
            return self.push(b);
        }
        let n = chords(dd * const { (9.0 / 16.0 / (TOLERANCE * TOLERANCE)) as f32 });
        if self.room(n) {
            let h = 1.0 / n as f32;
            let k1 = [3.0 * (c1[0] - a[0]), 3.0 * (c1[1] - a[1])];
            let k2 = [3.0 * (a[0] - 2.0 * c1[0] + c2[0]), 3.0 * (a[1] - 2.0 * c1[1] + c2[1])];
            let k3 = [b[0] - a[0] + 3.0 * (c1[0] - c2[0]), b[1] - a[1] + 3.0 * (c1[1] - c2[1])];
            self.pts.extend((1..n).map(|i| {
                let t = i as f32 * h;
                [((k3[0] * t + k2[0]) * t + k1[0]) * t + a[0], ((k3[1] * t + k2[1]) * t + k1[1]) * t + a[1]]
            }));
            self.pts.push(b);
        }
    }
}

impl OutlinePen for Edges {
    #[inline(always)]
    fn move_to(&mut self, x: f32, y: f32) {
        if self.cutting {
            return self.cut_move(x, y);
        }
        self.moved(self.place(x, y));
    }

    #[inline(always)]
    fn line_to(&mut self, x: f32, y: f32) {
        if self.cutting {
            return self.cut_curve(&[[x, y]]);
        }
        self.line(self.place(x, y));
    }

    #[inline(always)]
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if self.cutting {
            return self.cut_curve(&[[cx, cy], [x, y]]);
        }
        self.quad(self.place(cx, cy), self.place(x, y));
    }

    #[inline(always)]
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        if self.cutting {
            return self.cut_curve(&[[c1x, c1y], [c2x, c2y], [x, y]]);
        }
        self.cubic(self.place(c1x, c1y), self.place(c2x, c2y), self.place(x, y));
    }

    fn close(&mut self) {
        if self.cutting {
            return self.cut_end();
        }
        self.end_contour();
    }
}

impl Edges {
    /// Closes the contour being drawn, as `close` would, and measures the points: unfit if any isn't finite. The
    /// extent skips a NaN, so it's caught apart.
    pub(crate) fn finish(&mut self) {
        if self.cutting {
            self.cut_end();
        }
        self.end_contour();
        let pts = &self.pts[..self.ends.last().map_or(0, |&e| e as usize)];
        let Some(&[x, y]) = pts.first() else { return };
        let (mut b, mut nan) = ([x, y, x, y], false);
        for p in pts {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
            nan |= p[0].is_nan() | p[1].is_nan();
        }
        self.unfit |= nan || b.iter().any(|v| v.is_infinite());
        self.bounds = b;
    }

    /// The cells its edges pass at most, a sample's run across and down each.
    pub(crate) fn span(&self) -> u64 {
        let mut span = 0.0;
        for contour in self.contours() {
            let mut q = contour[contour.len() - 1];
            for &p in contour {
                span += f64::from((p[0] - q[0]).abs() + (p[1] - q[1]).abs());
                q = p;
            }
        }
        span as u64
    }

    // Each contour's points, as flattened.
    pub(super) fn contours(&self) -> impl Iterator<Item = &[[f32; 2]]> {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let contour = &self.pts[start..end as usize];
            start = end as usize;
            contour
        })
    }
}

// What `begin_cut` keeps: the matrix and box, the contour being cut in f64, and the curve pieces still to read; and
// for `begin_keep`, the contours cut, kept in f64.
#[derive(Default)]
struct Cut {
    m: [f64; 6],
    clip: [f64; 4],
    contour: Vec<[f64; 2]>,
    spare: Vec<[f64; 2]>,
    pieces: Vec<([[f64; 2]; 4], f64, f64)>,
    keep: bool,
    kept: Vec<[f64; 2]>,
    kept_ends: Vec<u32>,
}

// The fewest chords keeping a curve within TOLERANCE, by Wang's formula, as `chords` counts them in f32 and saturating
// alike, so each step's place along the curve is exact.
fn chords_f64(c: &[[f64; 2]; 4], n: usize) -> f64 {
    let dd = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let (x, y) = (a[0] - 2.0 * b[0] + c[0], a[1] - 2.0 * b[1] + c[1]);
        x * x + y * y
    };
    let v = match n {
        3 => dd(c[0], c[1], c[2]) / 16.0,
        _ => dd(c[0], c[1], c[2]).max(dd(c[1], c[2], c[3])) * 9.0 / 16.0,
    };
    (v / (TOLERANCE * TOLERANCE)).sqrt().sqrt().ceil().max(1.0).min(f64::from(u32::MAX))
}

// A piece of fewer steps is read whole; one of more is halved, unless wholly past a side.
const FEW: f64 = 64.0;

impl Edges {
    fn place_f64(&mut self, [x, y]: [f32; 2]) -> [f64; 2] {
        let (m, (x, y)) = (self.cut.m, (f64::from(x), f64::from(y)));
        let p = [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
        self.unfit |= !(p[0].is_finite() && p[1].is_finite());
        p
    }

    fn cut_move(&mut self, x: f32, y: f32) {
        self.cut_end();
        let p = self.place_f64([x, y]);
        self.cut.contour.push(p);
    }

    // A line, quadratic or cubic, its controls and end `rest`, flattened at the steps `quad` and `cubic` take, read
    // from the whole curve so every box finds the same points: a piece wholly past a side keeps its first and last.
    fn cut_curve(&mut self, rest: &[[f32; 2]]) {
        if self.cut.contour.is_empty() {
            let at = self.cut.spare.last().copied().unwrap_or_default();
            self.cut.contour.push(at);
        }
        let n = rest.len() + 1;
        let mut c = [self.cut.contour[self.cut.contour.len() - 1]; 4];
        for (k, &p) in rest.iter().enumerate() {
            c[k + 1] = self.place_f64(p);
        }
        if self.unfit {
            return;
        }
        if n == 2 {
            return self.cut.contour.push(c[1]);
        }
        let steps = chords_f64(&c, n);
        // The curve as a polynomial in t, each step read from it alone, so every box finds the same point.
        let k = match n {
            3 => [0, 1].map(|j| [c[0][j], 2.0 * (c[1][j] - c[0][j]), c[0][j] - 2.0 * c[1][j] + c[2][j], 0.0]),
            _ => [0, 1].map(|j| {
                let (a, b) = (3.0 * (c[1][j] - c[0][j]), 3.0 * (c[0][j] - 2.0 * c[1][j] + c[2][j]));
                [c[0][j], a, b, c[3][j] - c[0][j] + 3.0 * (c[1][j] - c[2][j])]
            }),
        };
        let step = |i: f64| {
            let t = i / steps;
            if i == steps { c[n - 1] } else { [0, 1].map(|j| ((k[j][3] * t + k[j][2]) * t + k[j][1]) * t + k[j][0]) }
        };
        let [x0, y0, x1, y1] = self.cut.clip;
        let mut piece = Some((c, 0.0, steps));
        // Each piece's controls and the steps it spans, those after `a` up to `b` its own.
        while let Some((c, a, b)) = piece.take().or_else(|| self.cut.pieces.pop()) {
            let (lo, hi) = c[..n].iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| {
                ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])])
            });
            let past = hi[0] <= x0 || lo[0] >= x1 || hi[1] <= y0 || lo[1] >= y1;
            let (first, last) = (a.floor() + 1.0, b.floor());
            if past || last - first < FEW {
                let mut i = first;
                while i <= last {
                    self.cut.contour.push(step(i));
                    i = if past && i < last { last } else { i + 1.0 };
                }
                continue;
            }
            let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let halves = match n {
                3 => {
                    let (p, q) = (mid(c[0], c[1]), mid(c[1], c[2]));
                    let m = mid(p, q);
                    [[c[0], p, m, m], [m, q, c[2], c[2]]]
                }
                _ => {
                    let (p, q, r) = (mid(c[0], c[1]), mid(c[1], c[2]), mid(c[2], c[3]));
                    let (s, t) = (mid(p, q), mid(q, r));
                    let m = mid(s, t);
                    [[c[0], p, s, m], [m, t, r, c[3]]]
                }
            };
            let m = (a + b) * 0.5;
            self.cut.pieces.push((halves[1], m, b));
            piece = Some((halves[0], a, m));
        }
    }

    // The contour cut to the box side by side, then placed in f32, as `end_contour` keeps it. The last point stays,
    // for a contour drawn on without a move.
    fn cut_end(&mut self) {
        let Some(&last) = self.cut.contour.last() else { return };
        let mut contour = core::mem::take(&mut self.cut.contour);
        let mut spare = core::mem::take(&mut self.cut.spare);
        let [x0, y0, x1, y1] = self.cut.clip;
        let inside = |p: &[f64; 2]| x0 <= p[0] && p[0] <= x1 && y0 <= p[1] && p[1] <= y1;
        if !contour.iter().all(inside) {
            for (axis, v, low) in [(0, x0, true), (0, x1, false), (1, y0, true), (1, y1, false)] {
                side(&contour, &mut spare, axis, v, low);
                core::mem::swap(&mut contour, &mut spare);
            }
        }
        // One lying along a side, or any one line across or down, encloses nothing.
        let along = |k: usize| contour.iter().all(|p| p[k] == contour[0][k]);
        if self.cut.keep && !along(0) && !along(1) {
            self.cut.kept.extend_from_slice(&contour);
            self.cut.kept_ends.push(self.cut.kept.len() as u32);
        } else if !along(0) && !along(1) && self.room(contour.len() as u32) {
            self.end_contour();
            (self.open, self.first) = (true, self.pts.len());
            for p in &contour {
                self.push(p.map(|v| v as f32));
            }
            self.end_contour();
        }
        contour.clear();
        spare.clear();
        spare.push(last);
        (self.cut.contour, self.cut.spare) = (contour, spare);
    }
}

// A closed contour into `dst`, a point added where it crosses `v` along `axis`, and every point past `v`, below it
// where `low`, moved onto it.
fn side(src: &[[f64; 2]], dst: &mut Vec<[f64; 2]>, axis: usize, v: f64, low: bool) {
    dst.clear();
    let mut p = src.last().copied().unwrap_or_default();
    for &q in src {
        let (a, b) = (p[axis] - v, q[axis] - v);
        if a * b < 0.0 {
            let mut x = [v; 2];
            x[1 - axis] = p[1 - axis] + a / (a - b) * (q[1 - axis] - p[1 - axis]);
            dst.push(x);
        }
        let mut c = q;
        c[axis] = if low { c[axis].max(v) } else { c[axis].min(v) };
        dst.push(c);
        p = q;
    }
}

/// A path kept by `begin_keep`, read a band of `rows` rows at a time, y up from the box's top at 0: each chord dealt
/// out to the first band it reaches and carried on while it reaches the next, so a band reads only its own.
#[derive(Default)]
pub(crate) struct Strips {
    pts: Vec<[f64; 2]>,
    ends: Vec<u32>,
    rows: f64,
    // The chords, each by its first point, by the band they first reach, and where each band's chords begin.
    dealt: Vec<u32>,
    starts: Vec<u32>,
    // Those reaching the band being read, in order, and those found past it.
    active: Vec<u32>,
    spare: Vec<u32>,
}

impl Strips {
    /// How many points it holds.
    pub(crate) fn points(&self) -> usize {
        self.pts.len()
    }

    // Chord g's ends, in its contour's order, and the bands it reaches.
    fn chord(&self, g: u32, start: u32, end: u32) -> ([f64; 2], [f64; 2], Option<(usize, usize)>) {
        let (p, q) = (self.pts[g as usize], self.pts[if g + 1 == end { start } else { g + 1 } as usize]);
        let (top, bottom) = (-p[1].max(q[1]) / self.rows, -p[1].min(q[1]) / self.rows);
        let (first, last) = (top.floor(), bottom.ceil() - 1.0);
        (p, q, (top < bottom && first <= last).then_some((first as usize, last as usize)))
    }

    // Each contour's start and end, for the chords in order.
    fn contours(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let mut start = 0;
        self.ends.iter().map(move |&end| (core::mem::replace(&mut start, end), end))
    }

    // Each chord reaching a band, and the first it reaches.
    fn firsts(&self) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.contours().flat_map(move |(s, e)| (s..e).filter_map(move |g| Some((g, self.chord(g, s, e).2?.0))))
    }
}

impl Edges {
    /// The contours `begin_keep` kept, handed to `strips` to read `bands` bands of `rows` rows from.
    pub(crate) fn keep_into(&mut self, strips: &mut Strips, rows: usize, bands: usize) {
        core::mem::swap(&mut strips.pts, &mut self.cut.kept);
        core::mem::swap(&mut strips.ends, &mut self.cut.kept_ends);
        strips.rows = rows as f64;
        let (mut starts, mut dealt) = (core::mem::take(&mut strips.starts), core::mem::take(&mut strips.dealt));
        starts.clear();
        starts.resize(bands + 2, 0);
        for (_, first) in strips.firsts() {
            starts[first.min(bands) + 1] += 1;
        }
        for b in 1..starts.len() {
            starts[b] += starts[b - 1];
        }
        dealt.clear();
        dealt.resize(starts[bands + 1] as usize, 0);
        for (g, first) in strips.firsts() {
            let at = &mut starts[first.min(bands)];
            dealt[*at as usize] = g;
            *at += 1;
        }
        // Dealing left each band's start at its end, the next one's start.
        starts.copy_within(..=bands, 1);
        starts[0] = 0;
        (strips.starts, strips.dealt) = (starts, dealt);
        strips.active.clear();
    }

    /// Empties it for band b of `strips`, `down` rows from its top: the chords reaching it cut to it, each contour's
    /// joined where it leaves and comes back along the same side, placed in f32 from the band's top.
    pub(crate) fn band(&mut self, strips: &mut Strips, b: usize, down: usize) {
        self.begin([0.0; 6]);
        self.most = usize::MAX;
        let (top, bottom) = (-(b as f64) * strips.rows, -(b as f64) * strips.rows - down as f64);
        let (mut active, mut spare) = (core::mem::take(&mut strips.active), core::mem::take(&mut strips.spare));
        // Those still reaching this band, merged in order with those reaching it first.
        spare.clear();
        let mut contours = strips.contours().peekable();
        for &g in &active {
            while contours.next_if(|&(_, e)| e <= g).is_some() {}
            let (s, e) = contours.peek().copied().unwrap_or_default();
            if strips.chord(g, s, e).2.is_some_and(|(_, last)| last >= b) {
                spare.push(g);
            }
        }
        drop(contours);
        let new = &strips.dealt[strips.starts[b] as usize..strips.starts[b + 1] as usize];
        active.clear();
        let (mut old, mut new) = (spare.iter().copied().peekable(), new.iter().copied().peekable());
        while let Some(g) = match (old.peek(), new.peek()) {
            (Some(&a), Some(&b)) if a < b => old.next(),
            (_, Some(_)) => new.next(),
            _ => old.next(),
        } {
            active.push(g);
        }
        // Where chord p to q crosses height y, worked out from p, so a band and the next find the same point.
        let cross = |p: [f64; 2], q: [f64; 2], y: f64| [p[0] + (y - p[1]) / (q[1] - p[1]) * (q[0] - p[0]), y];
        let cut = |c: [f64; 2], p, q| {
            if c[1] > top { cross(p, q, top) } else if c[1] < bottom { cross(p, q, bottom) } else { c }
        };
        let mut contours = strips.contours().peekable();
        let mut open = None;
        for &g in &active {
            while contours.next_if(|&(_, e)| e <= g).is_some() {}
            let (s, e) = contours.peek().copied().unwrap_or_default();
            let (p, q, _) = strips.chord(g, s, e);
            if open != Some(s) {
                self.end_contour();
                (self.open, self.first, open) = (true, self.pts.len(), Some(s));
            }
            for c in [cut(p, p, q), cut(q, p, q)] {
                self.push([c[0] as f32, (c[1] - top) as f32]);
            }
        }
        drop(contours);
        self.end_contour();
        self.finish();
        (strips.active, strips.spare) = (active, spare);
    }
}

/// Where coverage goes: an image `width` by `height` samples whose top left sample is `left` across and `top` up
/// from the path's origin.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: usize,
    pub(crate) height: usize,
}


/// The coverage of polygons, exact: each piece of an edge in a pixel adds its area and height to a grid of sums, read
/// back left to right. Where a pixel's winding stays within {0, 1} or {0, -1}, as `verdict` tells from the passages
/// the walk found sharing pixels, the sums are the coverage itself; elsewhere one sweep down the chains orders them
/// across, swaps them where they cross, and sums only the edges where the filled state changes, in the rows needed.
#[derive(Default)]
pub(crate) struct Coverage {
    pts: Vec<Pt>,
    ring: Vec<Pt>,
    chains: Vec<Chain>,
    slopes: Vec<f64>,
    sweep: Sweep,
    traced: Traced,
    grid: Vec<f32>,
    touched: Vec<u64>,
    // A row's sums filtered across.
    filtered: Vec<f32>,
    owner: Vec<u32>,
    generation: u32,
    dirty: usize,
    pub(super) tags: Tags,
    scratch: Scratch,
}

// What a fill keeps between calls: the rows `verdict` leaves to resolve, marked a row each first, each point's
// contour, whether each contour meets itself, `wound_once`'s boxes and turns, and the rows `exact` sweeps.
#[derive(Default)]
struct Scratch {
    rows: Vec<u32>,
    marked: Vec<bool>,
    contour: Vec<u32>,
    twisted: Vec<bool>,
    turns: Vec<([f32; 4], i32)>,
    swept: Vec<bool>,
}

// What the walk found: each pixel's first two passages, as the pixel, the first's mark less its generation, and the
// second's first and last; any later, as the pixel, first and last; and passages that might meet themselves. All are
// held within `room` bytes, a passage's at most for each of the frame's pixels and the path's points, `over` past it.
#[derive(Default)]
pub(super) struct Tags {
    pairs: Vec<[u32; 4]>,
    crowd: Vec<[u32; 3]>,
    folds: Vec<(Run, u32)>,
    room: usize,
    pub(super) over: bool,
}

impl Tags {
    // Whether another passage fits `room` with those held; once not, none is listed.
    fn admit(&mut self) -> bool {
        let held = size_of_val(&*self.pairs) + size_of_val(&*self.crowd) + size_of_val(&*self.folds);
        self.over |= held + size_of::<(Run, u32)>() > self.room;
        !self.over
    }
}


// Segment j's x at y, its slope taken ahead, its ends exact.
#[inline]
fn x_on(pts: &[Pt], slopes: &[f64], j: usize, y: f64) -> f64 {
    let (p, q) = (pts[j], pts[j + 1]);
    if y == p.y {
        p.x
    } else if y == q.y {
        q.x
    } else {
        p.x + (y - p.y) * slopes[j]
    }
}

// A y in its row as fixed point, the row's top 0 and its bottom ONE.
#[inline]
fn row_q(y: f64, r0: f64) -> i32 {
    ((y - r0) * f64::from(ONE)).round() as i32
}

#[inline]
fn cell(x: f64, last: usize) -> usize {
    (x.floor().max(0.0) as usize).min(last)
}

// One piece of an edge within a cell: its height adds to every pixel right of the cell, and to the cell's own pixel
// all but the share left of the edge, which goes to the next sum so it ends at the cell.
#[inline]
fn add_piece(acc: &mut [i32], c: usize, x0: f64, x1: f64, q0: i32, q1: i32, dir: i32) {
    let cover = (q1 - q0) * dir;
    let area = (f64::from(cover) * (x0 + x1 - 2.0 * c as f64) * 0.5).round() as i32;
    acc[c] = acc[c].wrapping_add(cover - area);
    acc[c + 1] = acc[c + 1].wrapping_add(area);
}

// An edge from (xa, row's qa) to (xb, qb), within one row, split at each pixel boundary it crosses.
fn line(acc: &mut [i32], width: usize, xa: f64, xb: f64, qa: i32, qb: i32, dir: i32) {
    if qa == qb {
        return;
    }
    let last = width - 1;
    let (ca, cb) = (cell(xa, last), cell(xb, last));
    if ca == cb {
        add_piece(acc, ca, xa, xb, qa, qb, dir);
        return;
    }
    let dq = f64::from(qb - qa) / (xb - xa);
    let (lo, hi) = if qa < qb { (qa, qb) } else { (qb, qa) };
    let (mut x0, mut q0, mut c) = (xa, qa, ca);
    if cb > ca {
        while c < cb {
            let edge = (c + 1) as f64;
            let q = (f64::from(qa) + (edge - xa) * dq).round().clamp(f64::from(lo), f64::from(hi)) as i32;
            add_piece(acc, c, x0, edge, q0, q, dir);
            (x0, q0, c) = (edge, q, c + 1);
        }
    } else {
        while c > cb {
            let edge = c as f64;
            let q = (f64::from(qa) + (edge - xa) * dq).round().clamp(f64::from(lo), f64::from(hi)) as i32;
            add_piece(acc, c, x0, edge, q0, q, dir);
            (x0, q0, c) = (edge, q, c - 1);
        }
    }
    add_piece(acc, cb, x0, xb, q0, qb, dir);
}

// How far summing was exact.
enum Verdict {
    // Every pixel, none meeting another or itself: under the nonzero rule where no two contours share a pixel, as Rows
    // has it, else where `wound_once` or `nested` finds them wound 0 or one sign's 1; else as Rows or Whole by rule.
    Nested,
    // All but the rows `verdict` lists. Under the nonzero rule a pixel crossed by one contour alone that doesn't meet
    // itself, or by one passage that doesn't, holds two windings a step apart, which summed and clamped is exact.
    Rows,
    // Not to be told, as the rule is even-odd, the passages passed their room or the budget ran out.
    Whole,
}

// Segments meeting share a pixel, so only the passages the walk found sharing one and didn't test are tested, and
// those that might meet themselves, every test paid from a budget a few times the path's points and passages.
// Its work, in `fill`'s units, comes with it: a passage's listing, sorting and reading some 40, a test 4.
fn verdict(
    edges: &Edges, frame: Frame, rule: FillRule, tags: &mut Tags, scratch: &mut Scratch, allowance: &mut u64,
) -> Option<Verdict> {
    if tags.over {
        return Some(Verdict::Whole);
    }
    let passages = tags.pairs.len() + tags.crowd.len() + tags.folds.len();
    if passages == 0 {
        scratch.rows.clear();
        return Some(Verdict::Nested);
    }
    let given = 16 * edges.pts.len() + 4 * passages + 256;
    if !spend(allowance, PASSAGE * passages as u64 + TEST * given as u64) {
        return None;
    }
    let budget = Cell::new(given);
    let verdict = judge(edges, frame, rule, tags, scratch, &budget);
    *allowance += TEST * budget.get() as u64;
    Some(verdict)
}

// `verdict`'s tests, each paid from `budget`.
fn judge(
    edges: &Edges, frame: Frame, rule: FillRule, tags: &mut Tags, scratch: &mut Scratch, budget: &Cell<usize>,
) -> Verdict {
    let pts = &edges.pts;
    let pitch = frame.width + 2;
    let pay = || budget.get().checked_sub(1).map(|left| budget.set(left)).is_some();
    // Whether any of two passages' segments meet, or of one's own; none once the budget's spent.
    let meets = |a: Run, b: Run, own: bool| {
        if !own {
            let (p, q) = (a.bounds(pts), b.bounds(pts));
            if p[2] < q[0] || q[2] < p[0] || p[3] < q[1] || q[3] < p[1] {
                return Some(false);
            }
        }
        for s in a.first..=a.last {
            for t in if own { s + 2 } else { b.first }..=b.last {
                if !pay() {
                    return None;
                }
                if touch(pts, a.ends(s), b.ends(t)) {
                    return Some(true);
                }
            }
        }
        Some(false)
    };
    let Scratch { rows, marked, contour, twisted, .. } = scratch;
    rows.clear();
    marked.clear();
    marked.resize(frame.height, false);
    twisted.clear();
    twisted.resize(edges.ends.len(), false);
    contour.clear();
    let mut start = 0;
    for (k, &end) in edges.ends.iter().enumerate() {
        contour.extend((start..end).map(|_| k as u32));
        start = end;
    }
    // Segments `first` to `last` of the contour holding them.
    let run = |first: u32, last: u32| {
        let k = contour[first as usize] as usize;
        Run { start: if k == 0 { 0 } else { edges.ends[k - 1] }, end: edges.ends[k], first, last }
    };
    // Contours not yet found twisted: once none is left, no test can tell more.
    let untwisted = Cell::new(edges.ends.len());
    let twist = |twisted: &mut [bool], k: usize| {
        untwisted.set(untwisted.get() - usize::from(!twisted[k]));
        twisted[k] = true;
    };
    // A passage meeting itself: its contour twists, and its row is resolved.
    for &(run, i) in &tags.folds {
        match meets(run, run, true) {
            None => return Verdict::Whole,
            Some(true) => {
                twist(twisted, contour[run.first as usize] as usize);
                marked[i as usize / pitch] = true;
            }
            Some(false) => {}
        }
    }
    let (left, top) = (frame.left as f32, frame.top as f32);
    // A pixel's first passage from its mark; one of 8 segments or more is found again, as the walk stayed while each
    // segment ended within the pixel.
    let passage = |i: u32, mark: u32| {
        let (first, more) = (mark & SHARED, mark >> ID_BITS);
        let mut run = run(first, first + more);
        if more < 7 {
            return run;
        }
        let within = |g: u32| {
            let [x, y] = pts[g as usize];
            (top - y) as usize * pitch + (x - left) as usize == i as usize
        };
        while run.last + 1 < run.end && within(run.last) {
            run.last += 1;
        }
        run
    };
    let mut crossed = false;
    // Two passages through pixel i: one contour's meeting twist it, and two contours' leave the row to resolve.
    let mut test = |i: u32, a: Run, b: Run| {
        // A twisted contour has every pixel its passages share resolved, so a passage of one is tested no further.
        let (ka, kb) = (contour[a.first as usize] as usize, contour[b.first as usize] as usize);
        if twisted[ka] || twisted[kb] {
            return true;
        }
        let met = if ka != kb && crossed { Some(false) } else { meets(a, b, false) };
        let Some(met) = met else { return false };
        if ka == kb {
            if met {
                twist(twisted, ka);
            }
        } else {
            marked[i as usize / pitch] = true;
            crossed |= met;
        }
        true
    };
    for &[i, a, b, last] in &tags.pairs {
        if untwisted.get() == 0 {
            break;
        }
        if last & APART == 0 && !test(i, passage(i, a), run(b, last)) {
            return Verdict::Whole;
        }
    }
    // A pixel's third passage or later, against those before it in that pixel, found by sorting both by pixel.
    if !tags.crowd.is_empty() {
        tags.pairs.sort_unstable_by_key(|p| p[0]);
        tags.crowd.sort_by_key(|c| c[0]);
    }
    let mut at = 0;
    for (k, &[i, first, last]) in tags.crowd.iter().enumerate() {
        if untwisted.get() == 0 {
            break;
        }
        let c = run(first, last);
        while tags.pairs.get(at).is_some_and(|p| p[0] < i) {
            at += 1;
        }
        for &[_, a, b, last] in tags.pairs[at..].iter().take_while(|p| p[0] == i) {
            if !pay() || !(test(i, c, passage(i, a)) && test(i, c, run(b, last & !APART))) {
                return Verdict::Whole;
            }
        }
        for &[_, first, last] in tags.crowd[..k].iter().rev().take_while(|e| e[0] == i) {
            if !pay() || !test(i, c, run(first, last)) {
                return Verdict::Whole;
            }
        }
    }
    let nested = !crossed && untwisted.get() == edges.ends.len();
    if !nested && rule != FillRule::NonZero {
        return Verdict::Whole;
    }
    // Then every pixel a twisted contour's passages share.
    if untwisted.get() < edges.ends.len() {
        let twists = |g: u32| twisted[contour[g as usize] as usize];
        for &[i, a, b, _] in &tags.pairs {
            if twists(a & SHARED) || twists(b) {
                marked[i as usize / pitch] = true;
            }
        }
        for &[i, first, _] in &tags.crowd {
            if twists(first) {
                marked[i as usize / pitch] = true;
            }
        }
    }
    rows.extend((0..frame.height as u32).filter(|&r| marked[r as usize]));
    if nested { Verdict::Nested } else { Verdict::Rows }
}

// `nested`'s answer read pair by pair, each contour's winding just outside and inside it at its first point; none
// past four passes over the points, or for a contour enclosing nothing, which `nested` then tells in one sweep.
fn wound_once(edges: &Edges, turns: &mut Vec<([f32; 4], i32)>) -> Option<bool> {
    let mut left = 4 * edges.pts.len();
    // Each contour reads every other's box at least, so past that it's known before reading any.
    if edges.ends.len().saturating_mul(edges.ends.len()) > left {
        return None;
    }
    turns.clear();
    for contour in edges.contours() {
        let (mut b, mut area) = ([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], 0.0f64);
        let mut p = contour[contour.len() - 1];
        for &q in contour {
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
            area += f64::from(p[0]) * f64::from(q[1]) - f64::from(q[0]) * f64::from(p[1]);
            p = q;
        }
        turns.push((b, if area > 0.0 { 1 } else if area < 0.0 { -1 } else { return None }));
    }
    let mut sign = 0;
    let mut fits = |w: i32| {
        let fit = w == 0 || (w.abs() == 1 && (sign == 0 || sign == w));
        if w != 0 {
            sign = w;
        }
        fit
    };
    for (k, (contour, &(_, turn))) in edges.contours().zip(turns.iter()).enumerate() {
        let [x, y] = contour[0];
        left = left.checked_sub(turns.len())?;
        let mut outside = 0;
        for (l, (other, &(b, _))) in edges.contours().zip(turns.iter()).enumerate() {
            if l != k && y >= b[1] && y <= b[3] && x < b[2] {
                left = left.checked_sub(other.len())?;
                outside += winding(other, x, y);
            }
        }
        if !fits(outside) || !fits(outside + turn) {
            return Some(false);
        }
    }
    Some(true)
}

// A contour's winding about (x, y): its crossings of the ray going right, each a segment's way up or down, every
// segment taken over its rows from its lower end up to but not including its upper.
fn winding(contour: &[[f32; 2]], x: f32, y: f32) -> i32 {
    let (x, y) = (f64::from(x), f64::from(y));
    let mut p = contour[contour.len() - 1].map(f64::from);
    let mut w = 0;
    for q in contour {
        let q = q.map(f64::from);
        if (p[1] <= y) != (q[1] <= y) && p[0] + (y - p[1]) * (q[0] - p[0]) / (q[1] - p[1]) > x {
            w += if q[1] > p[1] { 1 } else { -1 };
        }
        p = q;
    }
    w
}

// A passage's segments, `first` to `last`, in the contour whose points run from `start` up to `end`.
#[derive(Clone, Copy)]
struct Run {
    start: u32,
    end: u32,
    first: u32,
    last: u32,
}

impl Run {
    // The points segment g runs between: the one before, or the contour's last for its first, then point g.
    fn ends(&self, g: u32) -> [u32; 2] {
        [if g == self.start { self.end - 1 } else { g - 1 }, g]
    }

    // Its first segment's start.
    fn from(&self, pts: &[[f32; 2]]) -> [f32; 2] {
        pts[self.ends(self.first)[0] as usize]
    }

    fn bounds(&self, pts: &[[f32; 2]]) -> [f32; 4] {
        let [x, y] = self.from(pts);
        let mut b = [x, y, x, y];
        for p in &pts[self.first as usize..=self.last as usize] {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        b
    }

    // Whether its points run one way in x or in y, so no two of its segments can meet.
    fn monotone(&self, pts: &[[f32; 2]]) -> bool {
        let mut p = self.from(pts);
        let mut ways = [true; 4];
        for &q in &pts[self.first as usize..=self.last as usize] {
            ways = [ways[0] && q[0] > p[0], ways[1] && q[0] < p[0], ways[2] && q[1] > p[1], ways[3] && q[1] < p[1]];
            p = q;
        }
        ways.contains(&true)
    }
}

// Whether two segments, as their points, touch or cross, but one and the same or neighbors: folding back along each
// other, those enclose nothing.
fn touch(pts: &[[f32; 2]], [a, b]: [u32; 2], [c, d]: [u32; 2]) -> bool {
    if (a, b) == (c, d) || b == c || d == a {
        return false;
    }
    let [a, b, c, d] = [a, b, c, d].map(|i| pts[i as usize]);
    // Boxes apart can't meet, and collinear, it's all that tells.
    let apart = |k: usize| a[k].max(b[k]) < c[k].min(d[k]) || c[k].max(d[k]) < a[k].min(b[k]);
    if apart(0) || apart(1) {
        return false;
    }
    let [a, b, c, d] = [a, b, c, d].map(|p| p.map(f64::from));
    let turn = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let side = |u: f64, v: f64| (u > 0.0 && v > 0.0) || (u < 0.0 && v < 0.0);
    !side(turn(a, b, c), turn(a, b, d)) && !side(turn(c, d, a), turn(c, d, b))
}

fn filled(rule: FillRule, winding: i32) -> bool {
    match rule {
        FillRule::EvenOdd => winding & 1 != 0,
        _ => winding != 0,
    }
}

impl Coverage {
    // `fill` where contours may overlap (`exact`, the bytes its passages may hold and its work's allowance), else
    // summed straight. False where the fill's work would pass its allowance, leaving `out` unfinished.
    #[must_use]
    pub(crate) fn draw<T: Level>(
        &mut self, edges: &Edges, rule: FillRule, frame: Frame, exact: Option<(usize, &mut u64)>, out: &mut [T],
        stride: usize,
    ) -> bool {
        if let Some((room, allowance)) = exact {
            self.fill(edges, rule, frame, (room, allowance), out, stride)
        } else {
            self.summed::<_, false>(edges, frame, out, stride);
            true
        }
    }

    /// Writes the coverage of `edges`' polygons under `rule` into `out`, a row of `frame.width` bytes every `stride`:
    /// summed, then resolved where `verdict` finds the sums may be wrong, its passages held within `room` bytes. Its
    /// work is paid from `allowance`, foreseen before it's done but for swaps, paid as they're made; false past it.
    pub(crate) fn fill<T: Level>(
        &mut self, edges: &Edges, rule: FillRule, frame: Frame, (room, allowance): (usize, &mut u64), out: &mut [T],
        stride: usize,
    ) -> bool {
        let (width, height) = (frame.width, frame.height);
        if width == 0 || height == 0 {
            return true;
        }
        let (points, pixels) = (edges.pts.len(), (width as u64).saturating_mul(height as u64));
        let each = (POINT + LEVEL * levels(points)) * points as u64;
        if !spend(allowance, CELL.saturating_mul(edges.span()).saturating_add(each).saturating_add(PIXEL * pixels)) {
            return false;
        }
        let passages = (pixels as usize).saturating_add(points);
        self.tags.room = room.min(passages.saturating_mul(size_of::<(Run, u32)>()));
        self.summed::<_, true>(edges, frame, out, stride);
        let Some(verdict) = verdict(edges, frame, rule, &mut self.tags, &mut self.scratch, allowance) else {
            return false;
        };
        let verdict = match verdict {
            Verdict::Nested if edges.ends.len() == 1 => return true,
            Verdict::Nested if rule == FillRule::NonZero && self.scratch.rows.is_empty() => return true,
            Verdict::Nested if wound_once(edges, &mut self.scratch.turns) == Some(true) => return true,
            verdict => verdict,
        };
        self.chains_of(edges, frame);
        let every = match verdict {
            Verdict::Nested if self.nested() => return true,
            Verdict::Nested => rule != FillRule::NonZero,
            Verdict::Rows => false,
            Verdict::Whole => true,
        };
        if every {
            self.scratch.rows.clear();
            self.scratch.rows.extend(0..height as u32);
        }
        self.scratch.rows.is_empty() || self.exact(rule, (width, height), allowance, out, stride)
    }

    // Each contour's runs going one way in y, in image space, y down, stored top first; a flat stretch joins the run
    // it lies in. Read from its topmost point, no run wraps round.
    fn chains_of(&mut self, edges: &Edges, frame: Frame) {
        self.pts.clear();
        self.chains.clear();
        let (left, top) = (f64::from(frame.left), f64::from(frame.top));
        let image = |p: &[f32; 2]| Pt { x: f64::from(p[0]) - left, y: top - f64::from(p[1]) };
        let mut start = 0usize;
        for &end in &edges.ends {
            let contour = &edges.pts[start..end as usize];
            start = end as usize;
            let top = |t: (usize, f32), (i, p): (usize, &[f32; 2])| if p[1] > t.1 { (i, p[1]) } else { t };
            let first = contour.iter().enumerate().fold((0, f32::MIN), top).0;
            self.ring.clear();
            self.ring.extend(contour[first..].iter().chain(&contour[..=first]).map(image));
            let (mut from, mut dir) = (0, 0);
            for i in 1..self.ring.len() {
                let dy = self.ring[i].y - self.ring[i - 1].y;
                let d = if dy > 0.0 { 1 } else if dy < 0.0 { -1 } else { 0 };
                if d != 0 && dir != 0 && d != dir {
                    self.push_chain(from, i - 1, dir);
                    from = i - 1;
                }
                if d != 0 {
                    dir = d;
                }
            }
            self.push_chain(from, self.ring.len() - 1, dir);
        }
    }

    fn push_chain(&mut self, a: usize, b: usize, dir: i32) {
        if dir == 0 {
            return;
        }
        let start = self.pts.len() as u32;
        if dir > 0 {
            self.pts.extend_from_slice(&self.ring[a..=b]);
        } else {
            self.pts.extend(self.ring[a..=b].iter().rev());
        }
        self.chains.push(Chain { start, end: self.pts.len() as u32, dir });
    }

    // Whether every region the contours bound, none meeting another or itself, is wound 0 or one sign's 1: each is
    // born between two chains starting at a point, read from the winding beside them as the sweep passes down.
    fn nested(&mut self) -> bool {
        let (pts, chains, s) = (&self.pts, &self.chains, &mut self.sweep);
        s.reset(chains);
        for (i, c) in chains.iter().enumerate() {
            s.events.push((pts[c.start as usize].y, ENTER | i as u32));
            s.events.push((pts[c.end as usize - 1].y, LEAVE | i as u32));
        }
        s.events.sort_unstable_by_key(|&(y, e)| (height_key(y), e));
        let mut sign = 0;
        let mut fits = |w: i32| {
            let fit = w == 0 || (w.abs() == 1 && (sign == 0 || sign == w));
            if w != 0 {
                sign = w;
            }
            fit
        };
        let mut k = 0;
        while let Some(&(y, _)) = s.events.get(k) {
            while let Some(&(at, e)) = s.events.get(k).filter(|e| e.0 == y) {
                k += 1;
                let i = e & LINE;
                if e & ENTER != 0 { s.enter(pts, chains, i, at) } else { s.leave(i) }
            }
            if !s.wind(chains, |_, _, _, w| fits(w)) {
                return false;
            }
            s.touched.clear();
        }
        true
    }

    // The rows in `scratch` from one sweep down every chain, each traced within a row where `rule` fills one side of
    // it and not the other. False once its swaps would pass `allowance`.
    fn exact<T: Level>(
        &mut self, rule: FillRule, (width, height): (usize, usize), allowance: &mut u64, out: &mut [T], stride: usize,
    ) -> bool {
        // Each segment's x per y, taken once; one from a chain's last point to the next's first is never read.
        self.slopes.clear();
        self.slopes.extend(self.pts.windows(2).map(|w| (w[1].x - w[0].x) / (w[1].y - w[0].y)));
        let (pts, lines, rows, s, t) = (&self.pts, &self.chains, &self.scratch.rows, &mut self.sweep, &mut self.traced);
        let swept = &mut self.scratch.swept;
        let path = (pts.as_slice(), self.slopes.as_slice());
        s.reset(lines);
        (s.swaps, s.steps) = (0, 0);
        let swap = SWAP + LEVEL * levels(lines.len());
        // Rows swept: the listed ones, and gaps between whose events, spread evenly, number half the lines or fewer.
        // Past a longer gap the order's built again at the next row, for about what taking them would cost.
        let each = (pts.len() + lines.len()) as f64 / height as f64;
        swept.clear();
        swept.resize(height, false);
        let (mut from, mut gaps) = (0, false);
        for &r in rows {
            let r = r as usize;
            let gap = (r - from) as f64 * each > (lines.len() / 2) as f64;
            swept[if gap { r } else { from }..=r].fill(true);
            (from, gaps) = (r + 1, gaps | gap);
        }
        // An event at the top of a run past a gap is taken by building the order there.
        let within = |y: f64| {
            let r = y as usize;
            !gaps || r < height && swept[r] && (y > r as f64 || r == 0 || swept[r - 1])
        };
        for (i, l) in lines.iter().enumerate() {
            let i = i as u32;
            let (top, bottom) = (pts[l.start as usize].y, pts[l.end as usize - 1].y);
            if within(top) {
                s.events.push((top, ENTER | i));
            }
            let mut last = top;
            for p in &pts[l.start as usize + 1..l.end as usize - 1] {
                if p.y > last && p.y < bottom {
                    if within(p.y) {
                        s.events.push((p.y, PASS | i));
                    }
                    last = p.y;
                }
            }
            if within(bottom) {
                s.events.push((bottom, LEAVE | i));
            }
        }
        s.sort(height);
        t.acc.clear();
        t.acc.resize(width + 1, 0);
        t.open.clear();
        t.listed.clear();
        t.listed.resize(lines.len(), false);
        t.row = None;
        let way = |wl: i32, wr: i32| match (filled(rule, wl), filled(rule, wr)) {
            (false, true) => 1,
            (true, false) => -1,
            _ => 0,
        };
        let (mut y, mut k, mut next) = (f64::NEG_INFINITY, 0, 0);
        loop {
            s.wind(lines, |s, i, wl, wr| {
                t.turn(s, path, width, i, way(wl, wr), y);
                true
            });
            s.tests(pts, lines, y);
            if swap * s.swaps + STEP * s.steps > *allowance {
                return false;
            }
            let event = s.events.get(k).map(|e| e.0);
            let ahead = s.ahead(event);
            // The row whose bottom the sweep reaches is read, and the next begun once it reaches its top.
            loop {
                if let Some(r0) = t.row {
                    if ahead.is_some_and(|a| a < r0 + 1.0) {
                        break;
                    }
                    t.end(s, path, &mut out[r0 as usize * stride..][..width]);
                }
                // A row starting a run is begun once the order's built there.
                let begins = |r: u32| y == f64::from(r) || r == 0 || swept[r as usize - 1];
                let Some(&r) = rows.get(next).filter(|&&r| ahead.is_none_or(|a| a >= f64::from(r)) && begins(r)) else {
                    break;
                };
                t.begin(s, f64::from(r));
                next += 1;
            }
            // Past a gap the order is built again at the next listed row's top.
            let past = |r: u32| t.row.is_none() && r > 0 && !swept[r as usize - 1] && y < f64::from(r);
            if let Some(&r) = rows.get(next).filter(|&&r| past(r)) {
                y = f64::from(r);
                s.restart(pts, lines, y);
                continue;
            }
            let Some(at) = ahead.filter(|_| t.row.is_some() || next < rows.len()) else { break };
            y = at;
            if event == Some(y) {
                let from = k;
                while s.events.get(k).is_some_and(|e| e.0 == y) {
                    let i = s.events[k].1 & LINE;
                    match s.events[k].1 & !LINE {
                        PASS => {
                            s.pass(pts, lines, i, y);
                            s.touched.push(i);
                        }
                        LEAVE => {
                            t.turn(s, path, width, i, 0, y);
                            s.leave(i);
                        }
                        _ => s.enter(pts, lines, i, y),
                    }
                    k += 1;
                }
                for e in from..k {
                    let (i, kind) = (s.events[e].1 & LINE, s.events[e].1 & !LINE);
                    if kind == PASS && s.alive[i as usize] {
                        s.settle(pts, lines, i, y);
                    }
                }
                continue;
            }
            s.cross();
        }
        *allowance -= swap * s.swaps + STEP * s.steps;
        true
    }
}

// How deep the order and the crossings' heap run for n lines: log2 n.
fn levels(n: usize) -> u64 {
    u64::from(usize::BITS - n.leading_zeros())
}

// `work` out of `allowance`, if it holds it.
fn spend(allowance: &mut u64, work: u64) -> bool {
    allowance.checked_sub(work).map(|left| *allowance = left).is_some()
}

// What a sweep traces into the row it resolves: the row's sums and top, and the lines whose stretch is open, each
// listed once.
#[derive(Default)]
struct Traced {
    acc: Vec<i32>,
    row: Option<f64>,
    open: Vec<u32>,
    listed: Vec<bool>,
}

impl Traced {
    // Line i's stretch, traced where it lies in the row, ended at y and opened again as `way` says.
    fn turn(&mut self, s: &mut Sweep, path: (&[Pt], &[f64]), width: usize, i: u32, way: i32, y: f64) {
        let (from, seg, was) = s.open[i as usize];
        if was == way {
            return;
        }
        if let Some(r0) = self.row.filter(|_| was != 0) {
            trace(&mut self.acc, width, path, r0, seg, from, y, was);
        }
        s.open[i as usize] = (y, s.at[i as usize], way);
        if way != 0 && !self.listed[i as usize] {
            self.listed[i as usize] = true;
            self.open.push(i);
        }
    }

    // The row from r0 begun, each open stretch taken from there.
    fn begin(&mut self, s: &mut Sweep, r0: f64) {
        let listed = &mut self.listed;
        self.open.retain(|&i| {
            listed[i as usize] = s.open[i as usize].2 != 0;
            listed[i as usize]
        });
        for &i in &self.open {
            (s.open[i as usize].0, s.open[i as usize].1) = (r0, s.at[i as usize]);
        }
        self.row = Some(r0);
    }

    // The row ended, each open stretch traced to its bottom and the sums read into `out`.
    fn end<T: Level>(&mut self, s: &Sweep, path: (&[Pt], &[f64]), out: &mut [T]) {
        let Some(r0) = self.row.take() else { return };
        let width = out.len();
        for &i in &self.open {
            let (from, seg, way) = s.open[i as usize];
            if way != 0 {
                trace(&mut self.acc, width, path, r0, seg, from, r0 + 1.0, way);
            }
        }
        spans::coverage_row(&mut self.acc, out);
    }
}

// A line's stretch from segment `seg` at y0 down to y1 summed, point to point, `dir` being 1 where filled area lies
// right of it.
#[allow(clippy::too_many_arguments, reason = "a stretch of a line, and the row it's summed into")]
fn trace(acc: &mut [i32], width: usize, (pts, slopes): (&[Pt], &[f64]), r0: f64, seg: u32, y0: f64, y1: f64, dir: i32) {
    let (mut j, mut y) = (seg as usize, y0);
    loop {
        let y2 = pts[j + 1].y.min(y1);
        line(acc, width, x_on(pts, slopes, j, y), x_on(pts, slopes, j, y2), row_q(y, r0), row_q(y2, r0), dir);
        if y2 >= y1 {
            return;
        }
        (j, y) = (j + 1, y2);
    }
}

// A sum as a byte: its magnitude, at most a whole pixel, scaled to 255 and rounded.
#[inline]
fn level(sum: f32) -> u8 {
    (sum.abs().min(1.0) * 255.0 + 0.5) as u8
}

// What coverage is written as: bytes for an image, or unrounded for a scene composited in linear light, where a
// byte's half a level would carry through to several near black.
pub(crate) trait Level: Copy {
    fn of(sum: f32) -> Self;
    fn fixed(sum: i32) -> Self;
}

impl Level for u8 {
    #[inline(always)]
    fn of(sum: f32) -> u8 {
        level(sum)
    }
    #[inline(always)]
    fn fixed(sum: i32) -> u8 {
        spans::level(sum)
    }
}

impl Level for f32 {
    #[inline(always)]
    fn of(sum: f32) -> f32 {
        sum.abs().min(1.0)
    }
    #[inline(always)]
    fn fixed(sum: i32) -> f32 {
        sum.unsigned_abs().min(ONE as u32) as f32 / ONE as f32
    }
}

// A row of sums read into coverage after `sum`, each sum taken. Four at a time, their partial sums apart from the
// running one, so it waits on one add per four.
#[inline(always)]
fn read<T: Level>(row: &mut [T], sums: &mut [f32], mut sum: f32) {
    let width = row.len();
    let (four, rest) = row.split_at_mut(width & !3);
    for (o, s) in four.as_chunks_mut::<4>().0.iter_mut().zip(sums.as_chunks_mut::<4>().0) {
        let v = core::mem::take(s);
        let (ab, cd) = (v[0] + v[1], v[2] + v[3]);
        o[0] = T::of(sum + v[0]);
        o[1] = T::of(sum + ab);
        o[2] = T::of(sum + (ab + v[2]));
        sum += ab + cd;
        o[3] = T::of(sum);
    }
    for (o, s) in rest.iter_mut().zip(&mut sums[width & !3..]) {
        sum += core::mem::take(s);
        *o = T::of(sum);
    }
}

impl Coverage {
    /// Coverage summed edge by edge straight into a grid kept between calls, each edge's area and height added to the
    /// pixels it crosses: exact for polygons whose contours neither cross nor nest the same way round. A bit marks
    /// each sum touched, so a row is read in runs, a long one inside the glyph filled whole, and the grid is left
    /// clear as it's read. With `TAG`, the walk marks each pixel's passages for `verdict`.
    pub(crate) fn summed<T: Level, const TAG: bool>(
        &mut self, edges: &Edges, frame: Frame, out: &mut [T], stride: usize,
    ) {
        let (width, height) = (frame.width, frame.height);
        if width == 0 || height == 0 {
            return;
        }
        // Two sums past the last pixel: a point on the image's right edge has its cell there, and that cell's share
        // the next, so no cell is ever clamped.
        let pitch = width + 2;
        let words = pitch.div_ceil(64);
        if self.grid.len() < pitch * height {
            self.grid.resize(pitch * height, 0.0);
        }
        if self.touched.len() < words * height {
            self.touched.resize(words * height, 0);
        }
        if TAG {
            self.generation += 1;
            if self.generation == GENERATIONS {
                self.owner[..self.dirty].fill(0);
                (self.generation, self.dirty) = (1, 0);
            }
            if self.owner.len() < pitch * height {
                self.owner.resize(pitch * height, 0);
            }
            self.dirty = self.dirty.max(pitch * height);
            self.tags.pairs.clear();
            self.tags.crowd.clear();
            self.tags.folds.clear();
            self.tags.over = false;
        }
        let (left, top) = (frame.left as f32, frame.top as f32);
        // Up to 254 pixels wide a row is read whole, cheaper than finding its runs, so nothing marks what it touches.
        if words <= 4 {
            self.grid::<false, TAG>(&edges.pts, pitch, height).walk(edges, left, top);
            for r in 0..height {
                let sums = &mut self.grid[r * pitch..(r + 1) * pitch];
                read(&mut out[r * stride..r * stride + width], sums, 0.0);
                sums[width..].fill(0.0);
            }
            return;
        }
        self.grid::<true, TAG>(&edges.pts, pitch, height).walk(edges, left, top);
        for r in 0..height {
            let row = &mut out[r * stride..r * stride + width];
            let sums = &mut self.grid[r * pitch..(r + 1) * pitch];
            let (mut sum, mut x) = (0.0f32, 0usize);
            for (w, word) in self.touched[r * words..(r + 1) * words].iter_mut().enumerate() {
                let mut bits = core::mem::take(word);
                while bits != 0 {
                    let c = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    row[x..c.min(width)].fill(T::of(sum));
                    sum += core::mem::take(&mut sums[c]);
                    if c < width {
                        row[c] = T::of(sum);
                        x = c + 1;
                    } else {
                        x = width;
                    }
                }
            }
            row[x..].fill(T::of(sum));
        }
    }

    /// `summed`'s coverage filtered across by `kernel`, its first tap `lo` samples from the sample it's for: each byte
    /// one stripe's channel. Filtering the sums before they're read is filtering coverage after only while both keep
    /// one sign, as a learned glyph's coverage does and a kernel `stripes` gives, none of its weights negative.
    pub(crate) fn striped(
        &mut self, edges: &Edges, frame: Frame, (kernel, lo): (&[f32], isize), out: &mut [u8], stride: usize,
    ) {
        let (width, height) = (frame.width, frame.height);
        if width == 0 || height == 0 {
            return;
        }
        let pitch = width + 2;
        let words = pitch.div_ceil(64);
        if self.grid.len() < pitch * height {
            self.grid.resize(pitch * height, 0.0);
        }
        if self.touched.len() < words * height {
            self.touched.resize(words * height, 0);
        }
        if self.filtered.len() < pitch {
            self.filtered.resize(pitch, 0.0);
        }
        let (left, top) = (frame.left as f32, frame.top as f32);
        // Up to 126 samples wide every row is read whole; past that a row is, where at least one sum in 16 is
        // touched, and else read from its touched sums, measured both ways at 10 to 200 px.
        let narrow = words <= 2;
        if narrow {
            self.grid::<false, false>(&edges.pts, pitch, height).walk(edges, left, top);
        } else {
            self.grid::<true, false>(&edges.pts, pitch, height).walk(edges, left, top);
        }
        // The filtered sum at i, each tap's share added in order: the sums from i + lo on, as many as the taps.
        let at = |sums: &[f32], i: isize| {
            let mut v = 0.0f32;
            for (k, &w) in kernel.iter().enumerate() {
                if let Some(&s) = usize::try_from(i + lo + k as isize).ok().and_then(|j| sums.get(j)) {
                    v += w * s;
                }
            }
            v
        };
        for r in 0..height {
            let row = &mut out[r * stride..r * stride + width];
            let sums = &mut self.grid[r * pitch..(r + 1) * pitch];
            let filtered = &mut self.filtered[..width];
            // Filtered sums before the row's first sample start its running sum.
            let mut sum = 0.0f32;
            for i in -(lo + kernel.len() as isize - 1)..0 {
                sum += at(sums, i);
            }
            let bits = &mut self.touched[r * words..(r + 1) * words];
            if narrow || bits.iter().map(|b| b.count_ones() as usize).sum::<usize>() * 16 >= width {
                if !narrow {
                    bits.iter_mut().for_each(|b| *b = 0);
                }
                filtered.fill(0.0);
                for (k, &w) in kernel.iter().enumerate() {
                    let shift = lo + k as isize;
                    let from = (-shift).clamp(0, width as isize) as usize;
                    let to = (pitch as isize - shift).clamp(from as isize, width as isize) as usize;
                    for (f, &s) in filtered[from..to].iter_mut().zip(&sums[(from as isize + shift) as usize..]) {
                        *f += w * s;
                    }
                }
                sums.fill(0.0);
                read(row, filtered, sum);
                continue;
            }
            // Each touched sum, left to right, adds its taps' shares to the filtered sums it reaches; those it can't
            // reach any more are read out, and the stretch between two sums' reach takes one value.
            let taps = kernel.len() as isize;
            let (mut next, mut reached) = (0usize, 0usize);
            for (w, word) in self.touched[r * words..(r + 1) * words].iter_mut().enumerate() {
                let mut bits = core::mem::take(word);
                while bits != 0 {
                    let c = (w * 64 + bits.trailing_zeros() as usize) as isize;
                    bits &= bits - 1;
                    let v = core::mem::take(&mut sums[c as usize]);
                    let first = c - lo - (taps - 1);
                    let settled = first.clamp(0, width as isize) as usize;
                    while next < settled.min(reached) {
                        sum += core::mem::take(&mut filtered[next]);
                        row[next] = level(sum);
                        next += 1;
                    }
                    if next < settled {
                        row[next..settled].fill(level(sum));
                        next = settled;
                    }
                    for (k, &w) in kernel.iter().enumerate() {
                        let i = c - lo - k as isize;
                        if i < 0 {
                            sum += w * v;
                        } else if let Some(f) = filtered.get_mut(i as usize) {
                            *f += w * v;
                        }
                    }
                    reached = reached.max(((c - lo + 1).max(0) as usize).min(width));
                }
            }
            while next < reached {
                sum += core::mem::take(&mut filtered[next]);
                row[next] = level(sum);
                next += 1;
            }
            row[next..].fill(level(sum));
        }
    }

    fn grid<'a, const MARK: bool, const TAG: bool>(
        &'a mut self,
        pts: &'a [[f32; 2]],
        pitch: usize,
        height: usize,
    ) -> Grid<'a, MARK, TAG> {
        Grid {
            sums: &mut self.grid,
            touched: &mut self.touched,
            pitch,
            words: pitch.div_ceil(64),
            owner: &mut self.owner,
            tags: &mut self.tags,
            pts,
            generation: self.generation,
            height,
            contour: [0; 2],
        }
    }
}

struct Grid<'a, const MARK: bool, const TAG: bool> {
    sums: &'a mut [f32],
    touched: &'a mut [u64],
    pitch: usize,
    words: usize,
    // Each pixel's mark, as at SHARED, what the marks found, the points to test, and the contour walked.
    owner: &'a mut [u32],
    tags: &'a mut Tags,
    pts: &'a [[f32; 2]],
    generation: u32,
    height: usize,
    contour: [u32; 2],
}

impl<const MARK: bool, const TAG: bool> Grid<'_, MARK, TAG> {
    // A pixel's pieces add up in a run, written once the walk leaves it: a piece's share is its height times its
    // middle less the pixel's left, so a run keeps its heights, h, and their products with its ends' sums, hx.
    fn walk(&mut self, edges: &Edges, left: f32, top: f32) {
        let image = |p: [f32; 2]| [p[0] - left, top - p[1]];
        let mut start = 0u32;
        for &end in &edges.ends {
            let contour = &edges.pts[start as usize..end as usize];
            let mut p = image(contour[contour.len() - 1]);
            let mut at = (p[0] as usize, p[1] as usize);
            let (mut h, mut hx) = (0.0f32, 0.0f32);
            // The segment the walk came into `at` on, segment g being the one ending at point g.
            let mut first = start;
            self.contour = [start, end];
            for (g, &q) in (start..).zip(contour) {
                let q = image(q);
                let next = (q[0] as usize, q[1] as usize);
                if next == at {
                    let dy = q[1] - p[1];
                    h += dy;
                    hx += dy * (p[0] + q[0]);
                } else if next.0.abs_diff(at.0) + next.1.abs_diff(at.1) <= STEPS {
                    // Into each next pixel through the nearer of its column and row edges, found from the chord's
                    // start so rounding never builds up; each pixel left behind takes its run. A row edge the chord
                    // ends on is crossed last, so one ending on the bottom edge never steps into the row past it.
                    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                    let mut a = p;
                    while at != next {
                        let x = (at.0 + usize::from(dx > 0.0)) as f32;
                        let y = (at.1 + usize::from(dy > 0.0)) as f32;
                        let across = at.0 != next.0
                            && (at.1 == next.1
                                || y == q[1]
                                || (x - p[0]).abs() * dy.abs() < (y - p[1]).abs() * dx.abs());
                        let m = if across {
                            [x, p[1] + (x - p[0]) * dy / dx]
                        } else {
                            [p[0] + (y - p[1]) * dx / dy, y]
                        };
                        let d = m[1] - a[1];
                        self.run(at, h + d, hx + d * (a[0] + m[0]));
                        self.pass(at, first, g);
                        (h, hx, first) = (0.0, 0.0, g);
                        if across {
                            at.0 = if dx > 0.0 { at.0 + 1 } else { at.0 - 1 };
                        } else {
                            at.1 = if dy > 0.0 { at.1 + 1 } else { at.1 - 1 };
                        }
                        a = m;
                    }
                    h = q[1] - a[1];
                    hx = h * (a[0] + q[0]);
                } else {
                    self.run(at, h, hx);
                    self.pass(at, first, g);
                    (h, hx, first) = (0.0, 0.0, g);
                    self.edge(p, q, g);
                    at = next;
                }
                p = q;
            }
            self.run(at, h, hx);
            self.pass(at, first, end - 1);
            start = end;
        }
    }

    // A passage through pixel (c, r), segments `first` to `last`: the pixel's first marks it, and any later, or one of
    // three segments or more, goes to `contest`.
    #[inline(always)]
    fn pass(&mut self, (c, r): (usize, usize), first: u32, last: u32) {
        // Past their room the passages tell nothing: every row is resolved.
        if !TAG || r >= self.height || self.tags.over {
            return;
        }
        let i = r * self.pitch + c;
        if self.owner[i] >> MARK_BITS == self.generation || last - first >= 2 {
            self.contest(i, first, last);
        } else {
            self.owner[i] = self.generation << MARK_BITS | (last - first) << ID_BITS | first;
        }
    }

    // One that might meet itself is listed; a second through a pixel is listed with the first, and a third or later
    // alone.
    #[cold]
    #[inline(never)]
    fn contest(&mut self, i: usize, first: u32, last: u32) {
        let [start, end] = self.contour;
        let run = Run { start, end, first, last };
        if last - first >= 2 && self.tags.admit() && !run.monotone(self.pts) {
            self.tags.folds.push((run, i as u32));
        }
        let mark = self.owner[i];
        if mark >> MARK_BITS != self.generation {
            self.owner[i] = self.generation << MARK_BITS | (last - first).min(7) << ID_BITS | first;
            return;
        }
        if mark & SHARED == SHARED {
            if self.tags.admit() {
                self.tags.crowd.push([i as u32, first, last]);
            }
            return;
        }
        // One contour's two passages of few segments are tested here; apart, they're listed only to be told
        // against a third.
        let (f, more) = (mark & SHARED, mark >> ID_BITS & 7);
        if self.tags.admit() {
            let apart = f >= start && more < 7 && last - first < 8 && {
                let owner = Run { start, end, first: f, last: f + more };
                !(f..=f + more).any(|s| (first..=last).any(|t| touch(self.pts, owner.ends(s), run.ends(t))))
            };
            let pair = [i as u32, mark & ((1 << MARK_BITS) - 1), first, last | if apart { APART } else { 0 }];
            self.tags.pairs.push(pair);
        }
        self.owner[i] = self.generation << MARK_BITS | SHARED;
    }

    // The pixels of row r a piece of segment g crosses, between x0 and x1, but those in `skip`: its ends', which the
    // walk's runs pass.
    fn tag(&mut self, r: usize, x0: f32, x1: f32, g: u32, skip: [usize; 2]) {
        let (cl, cr) = if x0 < x1 { (x0 as usize, x1 as usize) } else { (x1 as usize, x0 as usize) };
        for c in cl..=cr {
            if c != skip[0] && c != skip[1] {
                self.pass((c, r), g, g);
            }
        }
    }

    // A run's sums into pixel (c, r). One along the image's bottom edge is flat and adds nothing, so it's skipped
    // before its row, one past the last, is reached.
    #[inline]
    fn run(&mut self, (c, r): (usize, usize), h: f32, hx: f32) {
        if h == 0.0 && hx == 0.0 {
            return;
        }
        let share = 0.5 * hx - h * c as f32;
        let i = r * self.pitch + c;
        let pair = &mut self.sums[i..i + 2];
        pair[0] += h - share;
        pair[1] += share;
        if MARK {
            self.mark(r, c, c + 1);
        }
    }

    // Marks sums a..=b of row r touched.
    #[inline]
    fn mark(&mut self, r: usize, a: usize, b: usize) {
        let row = &mut self.touched[r * self.words..(r + 1) * self.words];
        let (wa, wb) = (a / 64, b / 64);
        if wa == wb {
            row[wa] |= (u64::MAX >> (63 - (b - a))) << (a % 64);
        } else {
            row[wa] |= u64::MAX << (a % 64);
            row[wa + 1..wb].fill(u64::MAX);
            row[wb] |= u64::MAX >> (63 - b % 64);
        }
    }

    // An edge from p to q in image space, y down: its first row's stretch, the rows it crosses whole, its last's.
    fn edge(&mut self, p: [f32; 2], q: [f32; 2], g: u32) {
        let ends = [p[0] as usize, q[0] as usize];
        if p[1] == q[1] {
            if TAG {
                self.tag(p[1] as usize, p[0], q[0], g, ends);
            }
            return;
        }
        let (dir, a, b) = if p[1] < q[1] { (1.0, p, q) } else { (-1.0, q, p) };
        let (r0, r1) = (a[1] as usize, b[1] as usize);
        if r0 == r1 {
            self.row_piece(r0, a[0], b[0], (b[1] - a[1]) * dir);
            if TAG {
                self.tag(r0, a[0], b[0], g, ends);
            }
            return;
        }
        let dxdy = (b[0] - a[0]) / (b[1] - a[1]);
        let at = |r: usize| a[0] + (r as f32 - a[1]) * dxdy;
        let mut x = at(r0 + 1);
        self.row_piece(r0, a[0], x, ((r0 + 1) as f32 - a[1]) * dir);
        if TAG {
            self.tag(r0, a[0], x, g, [a[0] as usize, usize::MAX]);
        }
        if dxdy == 0.0 {
            // A vertical edge stays in one pixel of each row, which takes the same two shares every row.
            let c = x as usize;
            let share = dir * (x - c as f32);
            for r in r0 + 1..r1 {
                let i = r * self.pitch + c;
                let pair = &mut self.sums[i..i + 2];
                pair[0] += dir - share;
                pair[1] += share;
                if MARK {
                    self.mark(r, c, c + 1);
                }
                self.pass((c, r), g, g);
            }
        } else {
            for r in r0 + 1..r1 {
                let x1 = at(r + 1);
                self.row_piece(r, x, x1, dir);
                if TAG {
                    self.tag(r, x, x1, g, [usize::MAX; 2]);
                }
                x = x1;
            }
        }
        // An end on a row's top edge leaves nothing in that row, which may be the one past the image.
        if b[1] > r1 as f32 {
            self.row_piece(r1, x, b[0], (b[1] - r1 as f32) * dir);
            if TAG {
                self.tag(r1, x, b[0], g, [b[0] as usize, usize::MAX]);
            }
        }
    }

    // An edge's piece within row r, across x0..x1, `h` high signed by its direction. Each pixel it crosses keeps the
    // share of its height right of the edge, and the sum after takes the rest, so pixels past it take the whole.
    #[inline(always)]
    fn row_piece(&mut self, r: usize, x0: f32, x1: f32, h: f32) {
        let (xl, xr) = if x0 < x1 { (x0, x1) } else { (x1, x0) };
        let (cl, cr) = (xl as usize, xr as usize);
        // The sums the piece touches, one check for all of them.
        let row = r * self.pitch;
        let sums = &mut self.sums[row + cl..row + cr + 2];
        let n = sums.len() - 1;
        if n == 1 {
            let share = h * (0.5 * (xl + xr) - cl as f32);
            sums[0] += h - share;
            sums[1] += share;
        } else {
            // Height per unit across: the first and last pixels take their stretches', each between a whole unit's.
            let s = h / (xr - xl);
            let (fl, fr) = ((cl + 1) as f32 - xl, xr - cr as f32);
            let (hl, hr) = (s * fl, s * fr);
            sums[0] += 0.5 * hl * fl;
            sums[1] += hl - 0.5 * hl * fl + 0.5 * s;
            for sum in &mut sums[2..n] {
                *sum += s;
            }
            sums[n - 1] += hr - 0.5 * hr * fr - 0.5 * s;
            sums[n] += 0.5 * hr * fr;
        }
        if MARK {
            self.mark(r, cl, cr + 1);
        }
    }
}

