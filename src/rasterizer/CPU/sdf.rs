// Distance fields: each texel the signed distance from its center to the outline, inside positive, in one channel
// (SDF), or in three whose median is it and keeps corners sharp (MSDF), after Chlumský's thesis. A level is
// 127.5 + 127.5·d / spread, rounded.
use alloc::vec::Vec;

use super::sweep::{Chain, Pt, Sweep, MEETINGS, MEETINGS_BASE};
use super::{corner, fits, Drawn, Rasterizer, Shape, TOO_LARGE, TOO_MUCH, UNFIT};
#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::daekit::base::error::{Error, Result};
use crate::daecore::daekit::draw::bitmap::{MAX_DRAW_BYTES, MAX_SIDE};
use crate::daecore::daekit::draw::matrix::transform_box;
use crate::daecore::daetype::outline::distance::{cross, dot, Chords, END, START, V};
use crate::daecore::daetype::outline::Path;

// Chords stray from their curves by at most this much of the spread: an eighth of a level.
const TOLERANCE: f64 = 1.0 / 1020.0;

const TANGLED: Error = Error::limit("the outline's chords cross each other more than 64 times a chord");

#[derive(Clone, Copy, PartialEq)]
enum End {
    Neither,
    Start,
    Finish,
    // An end where another contour cut the segment: no tangent carries on past it, and it's signed by the winding.
    Cut,
}

// A channel's nearest segment so far: the squared distance; where that's to an end, how nearly along the end's
// tangent the texel lies, so a corner's two segments, equally near, go to the one the texel is more beside; which
// side it's on, the segment, and the end.
#[derive(Clone, Copy)]
struct Near {
    d2: f64,
    along: f64,
    left: bool,
    segment: u32,
    end: End,
}

const NOWHERE: u32 = u32::MAX;

/// A distance field's buffers, kept between calls.
#[derive(Default)]
pub(crate) struct Fields {
    chords: Chords,
    nearest: Vec<f64>,
    channels: Vec<[Near; 3]>,
    values: Vec<[f64; 3]>,
    flagged: Vec<bool>,
    crossings: Vec<(u32, f64, i32)>,
    inside: Vec<bool>,
    // Where MSDF's channels may have made an artifact: the two texels and the plain distance's side, the point
    // between them and the winding there.
    places: Vec<(usize, usize, bool)>,
    points: Vec<V>,
    wound: Vec<i32>,
    swept: Swept,
}

// The chords swept for where they meet and the winding beside points: each its own line, y down.
#[derive(Default)]
struct Swept {
    sweep: Sweep,
    lines: Vec<Chain>,
    ends: Vec<Pt>,
    which: Vec<u32>,
    pairs: Vec<(u32, u32)>,
    queries: Vec<(f64, f64, u32)>,
    asked: Vec<(f64, f64)>,
    sorted: Vec<i32>,
}

impl Swept {
    // Each chord of `chords` as a line for the sweep, its ends top first and y down, levels kept to be passed.
    fn lines(&mut self, chords: &Chords) {
        let (pts, lines, ends, which) = (&chords.pts, &mut self.lines, &mut self.ends, &mut self.which);
        lines.clear();
        ends.clear();
        which.clear();
        for s in &chords.segments {
            for j in s.first..s.last {
                let (a, b) = (Pt { x: pts[j][0], y: -pts[j][1] }, Pt { x: pts[j + 1][0], y: -pts[j + 1][1] });
                let dir = if b.y > a.y { 1 } else if b.y < a.y { -1 } else { 0 };
                let start = ends.len() as u32;
                ends.extend(if dir < 0 { [b, a] } else { [a, b] });
                lines.push(Chain { start, end: start + 2, dir });
                which.push(j as u32);
            }
        }
        self.sweep.allowed = MEETINGS * lines.len() as u64 + MEETINGS_BASE;
    }

    // The windings at `points`, y up, by one sweep down the chords; false once they cross too often.
    fn windings(&mut self, points: &[V], out: &mut Vec<i32>) -> bool {
        self.queries.clear();
        self.queries.extend(points.iter().enumerate().map(|(i, p)| (-p[1], p[0], i as u32)));
        self.queries.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        self.asked.clear();
        self.asked.extend(self.queries.iter().map(|q| (q.0, q.1)));
        if !self.sweep.run(&self.ends, &self.lines, &self.asked, &mut self.sorted, |_, _| {}) {
            return false;
        }
        out.clear();
        out.resize(points.len(), 0);
        for (q, &w) in self.queries.iter().zip(&self.sorted) {
            out[q.2 as usize] = w;
        }
        true
    }
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}

fn median([a, b, c]: [f64; 3]) -> f64 {
    a.min(b).max(a.max(b).min(c))
}

// Where, read from texel `a` to texel `b`, two channels cross: the only places the median turns.
fn turns(a: [f64; 3], b: [f64; 3]) -> impl Iterator<Item = f64> {
    [(0, 1), (0, 2), (1, 2)].into_iter().filter_map(move |(k, l)| {
        let (da, db) = (a[k] - a[l], b[k] - b[l]);
        (da * db < 0.0).then(|| da / (da - db))
    })
}

impl Rasterizer {
    /// Learned glyph `glyph` of `self.learned[at]`, placed by `m` in pixels y up, as a distance field reaching
    /// `spread` pixels each way: three channels if `multi`.
    pub(crate) fn distance(&mut self, at: usize, glyph: u32, m: &[f64; 6], spread: f64, multi: bool) -> Result<Drawn> {
        let (path, shape) = &self.learned[at].glyphs[&glyph];
        field(&mut self.fields, &mut self.pixels, path, *shape == Shape::Unsure, m, spread, multi)
    }
}

// `path` as a distance field into `pixels`. One whose overlaps weren't removed (`overlapping`) is measured to the
// parts of its edges that bound what it fills.
fn field(
    fields: &mut Fields, pixels: &mut Vec<u8>, path: &Path, overlapping: bool, m: &[f64; 6], spread: f64,
    multi: bool,
) -> Result<Drawn> {
    let mut chords = core::mem::take(&mut fields.chords);
    let drawn = measure(fields, &mut chords, pixels, path, overlapping, m, spread, multi);
    fields.chords = chords;
    drawn
}

#[allow(clippy::too_many_arguments, reason = "field's arguments, and the chords it measures to")]
fn measure(
    fields: &mut Fields, chords: &mut Chords, pixels: &mut Vec<u8>, path: &Path, overlapping: bool, m: &[f64; 6],
    spread: f64, multi: bool,
) -> Result<Drawn> {
    let Fields { nearest, channels, values, flagged, crossings, inside, places, points, wound, swept, .. } = fields;
    // A curve reaches past a sixth of its controls' box, so a box past six MAX_SIDE holds a field past MAX_SIDE.
    if let Some(b) = path.view().bounds() {
        let b = transform_box(m, [b.x_min, b.y_min, b.x_max, b.y_max]);
        if !b.iter().all(|v| v.is_finite()) {
            return Err(UNFIT);
        }
        if b[2] - b[0] > 6.0 * f64::from(MAX_SIDE) || b[3] - b[1] > 6.0 * f64::from(MAX_SIDE) {
            return Err(TOO_LARGE);
        }
    }
    // Each chord's ends are held within the bytes a draw holds.
    let built = chords.build(path, m, spread * TOLERANCE, MAX_DRAW_BYTES / size_of::<V>());
    if !chords.pts.iter().flatten().all(|v| v.is_finite()) {
        return Err(UNFIT);
    }
    if !built {
        return Err(TOO_MUCH);
    }
    let all = &chords.pts;
    let Some(bounds) = all.iter().fold(None, |b: Option<[f64; 4]>, p| {
        Some(b.map_or([p[0], p[1], p[0], p[1]], |b| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]))
    }) else {
        return Ok(Drawn { left: 0, top: 0, width: 0, height: 0, channels: if multi { 3 } else { 1 } });
    };
    let (left, bottom) = ((bounds[0] - spread).floor(), (bounds[1] - spread).floor());
    let (right, top) = ((bounds[2] + spread).ceil(), (bounds[3] + spread).ceil());
    if right - left > f64::from(MAX_SIDE) || top - bottom > f64::from(MAX_SIDE) {
        return Err(TOO_LARGE);
    }
    corner(left as i64)?;
    corner(top as i64)?;
    let (width, height) = ((right - left) as usize, (top - bottom) as usize);
    let n = width * height;
    // Each texel's nearest distance, side, flag and bytes, and for MSDF its three channels' nearest and values.
    let multi_bytes = if multi { size_of::<[Near; 3]>() + size_of::<[f64; 3]>() + 2 } else { 0 };
    let texels = n as u64 * (size_of::<f64>() + 3 + multi_bytes) as u64;
    fits(texels)?;
    let cap = spread * spread;
    nearest.clear();
    nearest.resize(n, cap);
    if multi {
        let far = Near { d2: cap, along: 0.0, left: false, segment: NOWHERE, end: End::Neither };
        channels.clear();
        channels.resize(n, [far; 3]);
    }
    let center = |r: usize, c: usize| [left + c as f64 + 0.5, top - r as f64 - 0.5];
    // Inside where the winding at a texel's center isn't 0: each row's crossings, left to right, held within
    // MAX_DRAW_BYTES with the texels, as are the chords filed by height after.
    let rows = |a: V, b: V| {
        let (lo, hi, way) = if a[1] < b[1] { (a, b, 1) } else { (b, a, -1) };
        // The rows whose centers lie in [lo, hi), each tested again exactly.
        let r0 = ((top - 0.5 - hi[1]).floor()).max(0.0) as usize;
        let r1 = ((top - 0.5 - lo[1]).floor() + 1.0).clamp(0.0, height as f64) as usize;
        (lo, hi, way, r0..r1)
    };
    let chord_ends = || chords.segments.iter().flat_map(|s| all[s.first..=s.last].windows(2));
    let crossed: u64 = chord_ends().map(|w| rows(w[0], w[1]).3.len() as u64).sum();
    let held = texels + crossed * size_of::<(u32, f64, i32)>() as u64;
    fits(held)?;
    crossings.clear();
    for w in chord_ends() {
        let (lo, hi, way, crossed) = rows(w[0], w[1]);
        for r in crossed {
            let y = top - r as f64 - 0.5;
            if y >= lo[1] && y < hi[1] {
                let x = lo[0] + (y - lo[1]) * (hi[0] - lo[0]) / (hi[1] - lo[1]);
                crossings.push((r as u32, x, way));
            }
        }
    }
    crossings.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    inside.clear();
    inside.resize(n, false);
    let mut at = 0;
    for r in 0..height {
        let mut winding = 0;
        for c in 0..width {
            let x = center(r, c)[0];
            while let Some(&(row, cx, way)) = crossings.get(at)
                && row as usize == r
                && cx < x
            {
                winding += way;
                at += 1;
            }
            inside[r * width + c] = winding != 0;
        }
        while crossings.get(at).is_some_and(|c| c.0 as usize == r) {
            at += 1;
        }
    }
    // Measured only to what bounds the outline's inside: where overlaps were left in, not every edge does. The
    // chords that meet are found, and the winding beside each piece they cut each other into read, by sweeps.
    swept.lines(chords);
    if overlapping {
        let (mut pairs, which) = (core::mem::take(&mut swept.pairs), core::mem::take(&mut swept.which));
        pairs.clear();
        let met = swept.sweep.run(&swept.ends, &swept.lines, &[], &mut Vec::new(), |a, b| {
            let (j, k) = (which[a as usize / 2], which[b as usize / 2]);
            pairs.push((j.min(k), j.max(k)));
        });
        pairs.sort_unstable();
        pairs.dedup();
        swept.which = which;
        let mut tangled = !met;
        if met {
            chords.bound(&pairs, &mut |points, out| tangled |= !swept.windings(points, out));
        }
        swept.pairs = pairs;
        if tangled {
            return Err(TANGLED);
        }
    } else {
        chords.orient();
    }
    let (pts, segments) = if overlapping { (&chords.kept, &chords.runs) } else { (&chords.pts, &chords.segments) };
    // Each chord reaches the texels within `spread` of it: on each row of its box, `spread` wider each way, those
    // within `spread` across of where it runs between the heights `spread` above and below the row.
    let reach = |a: V, b: V, r: usize| {
        let y = top - r as f64 - 0.5;
        let (lo, hi) = (a[1].min(b[1]).max(y - spread), a[1].max(b[1]).min(y + spread));
        let x = |y: f64| if a[1] == b[1] { a[0] } else { a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) };
        let (x0, x1) = if a[1] == b[1] { (a[0], b[0]) } else { (x(lo), x(hi)) };
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let c0 = (x0 - spread - left - 0.5).ceil().max(0.0) as usize;
        let c1 = ((x1 + spread - left - 0.5).floor() + 1.0).clamp(0.0, width as f64) as usize;
        c0..c1.max(c0)
    };
    for (k, s) in segments.iter().enumerate() {
        for j in s.first..s.last {
            let (a, b) = (pts[j], pts[j + 1]);
            let (ab, len2) = (sub(b, a), dot(sub(b, a), sub(b, a)));
            let r0 = (top - 0.5 - a[1].max(b[1]) - spread).ceil().max(0.0) as usize;
            let r1 = ((top - 0.5 - a[1].min(b[1]) + spread).floor() + 1.0).clamp(0.0, height as f64) as usize;
            for r in r0..r1 {
                for c in reach(a, b, r) {
                    let p = center(r, c);
                    let ap = sub(p, a);
                    let t = dot(ap, ab);
                    let (d2, end, from) = if t <= 0.0 || len2 == 0.0 {
                        let end = match (j == s.first, s.ends & START != 0) {
                            (false, _) => End::Neither,
                            (true, own) => if own { End::Start } else { End::Cut },
                        };
                        (dot(ap, ap), end, ap)
                    } else if t >= len2 {
                        let bp = sub(p, b);
                        let end = match (j + 1 == s.last, s.ends & END != 0) {
                            (false, _) => End::Neither,
                            (true, own) => if own { End::Finish } else { End::Cut },
                        };
                        (dot(bp, bp), end, bp)
                    } else {
                        let q = cross(ab, ap);
                        (q * q / len2, End::Neither, ap)
                    };
                    let i = r * width + c;
                    if d2 < nearest[i] {
                        nearest[i] = d2;
                    }
                    if !multi {
                        continue;
                    }
                    let along = match end {
                        End::Start if d2 > 0.0 => dot(s.start, from).abs() / d2.sqrt(),
                        End::Finish if d2 > 0.0 => dot(s.end, from).abs() / d2.sqrt(),
                        _ => 0.0,
                    };
                    let near = Near { d2, along, left: (cross(ab, ap) >= 0.0) == s.left, segment: k as u32, end };
                    for (ch, best) in channels[i].iter_mut().enumerate() {
                        if s.color >> ch & 1 != 0 && (d2 < best.d2 || d2 == best.d2 && along < best.along) {
                            *best = near;
                        }
                    }
                }
            }
        }
    }
    let level = |d: f64| ((127.5 + 127.5 * d / spread).clamp(0.0, 255.0) + 0.5) as u8;
    let signed = |i: usize| if inside[i] { nearest[i].sqrt() } else { -nearest[i].sqrt() };
    let channels_out = if multi { 3 } else { 1 };
    if pixels.len() < n * channels_out {
        pixels.resize(n * channels_out, 0);
    }
    if !multi {
        for (i, o) in pixels[..n].iter_mut().enumerate() {
            *o = level(signed(i));
        }
        return Ok(Drawn { left: left as i32, top: top as i32, width, height, channels: 1 });
    }
    values.clear();
    values.extend((0..n).map(|i| {
        let p = center(i / width, i % width);
        channels[i].map(|near| {
            if near.segment == NOWHERE {
                return signed(i).signum() * spread;
            }
            let s = &segments[near.segment as usize];
            let inward = if s.left { 1.0 } else { -1.0 };
            // Past a segment's end, the distance to the line its end runs along, which carries a corner on.
            let d = match near.end {
                End::Start if dot(sub(p, pts[s.first]), s.start) < 0.0 => cross(s.start, sub(p, pts[s.first])) * inward,
                End::Finish if dot(sub(p, pts[s.last]), s.end) > 0.0 => cross(s.end, sub(p, pts[s.last])) * inward,
                End::Cut => signed(i),
                _ if near.left => near.d2.sqrt(),
                _ => -near.d2.sqrt(),
            };
            d.clamp(-spread, spread)
        })
    }));
    // Read between two texels, across, down or diagonally, where the median turns to the other side from the plain
    // distance read between them and the winding there sides with the plain distance, the channels made an
    // artifact: both texels take the plain distance, as does any whose own median lies on the wrong side.
    flagged.clear();
    flagged.resize(n, false);
    // Each place to test, the two texels and the plain distance's side there, then the windings there in one sweep.
    places.clear();
    points.clear();
    for i in 0..n {
        let (r, c) = (i / width, i % width);
        for (down, across) in [(0, 1), (1, 0), (1, 1), (1, -1)] {
            let (r2, c2) = (r + down, c as isize + across);
            if r2 >= height || c2 < 0 || c2 as usize >= width {
                continue;
            }
            let j = r2 * width + c2 as usize;
            let (a, b, sa, sb) = (values[i], values[j], signed(i), signed(j));
            for t in turns(a, b) {
                let (m, plain) = (median([0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)), sa + (sb - sa) * t);
                if m * plain < 0.0 {
                    let (p, q) = (center(r, c), center(r2, c2 as usize));
                    points.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
                    places.push((i, j, plain > 0.0));
                }
            }
        }
    }
    if !swept.windings(points, wound) {
        return Err(TANGLED);
    }
    for (&(i, j, within), &w) in places.iter().zip(wound.iter()) {
        if (w != 0) == within {
            flagged[i] = true;
            flagged[j] = true;
        }
    }
    for (i, v) in values.iter_mut().enumerate() {
        let truth = signed(i).clamp(-spread, spread);
        if flagged[i] || median(*v) * truth < 0.0 {
            *v = [truth; 3];
        }
    }
    for (o, v) in pixels[..n * 3].as_chunks_mut::<3>().0.iter_mut().zip(values.iter()) {
        *o = v.map(level);
    }
    Ok(Drawn { left: left as i32, top: top as i32, width, height, channels: 3 })
}
