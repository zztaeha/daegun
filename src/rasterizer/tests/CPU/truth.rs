// The truth the rasterizer is held to: each pixel row cut into bands wherever an edge ends, crosses a pixel's side or
// crosses another edge, then each band crossed by lines, 64 a row's height and at least 2, whose filled stretches
// are measured exactly in x. Within a band each pixel's filled width is linear in y, which the midpoint rule
// integrates exactly, so what's left is rounding.
use crate::daecore::daetype::outline::{FillRule, OutlinePen};
use std::vec::Vec;

const LINES: usize = 64;

/// A path's contours in pixels, y up, each curve cut into 512 chords, near enough to the curve for any size tested.
#[derive(Default)]
pub(super) struct Fine {
    pub(super) contours: Vec<Vec<[f64; 2]>>,
}

const CHORDS: usize = 512;

impl Fine {
    fn last(&self) -> [f64; 2] {
        self.contours.last().and_then(|c| c.last()).copied().unwrap_or_default()
    }
}

impl OutlinePen for Fine {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(vec![[f64::from(x), f64::from(y)]]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.contours.last_mut().unwrap().push([f64::from(x), f64::from(y)]);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (a, c, b) = (self.last(), [f64::from(cx), f64::from(cy)], [f64::from(x), f64::from(y)]);
        let contour = self.contours.last_mut().unwrap();
        for i in 1..=CHORDS {
            let t = i as f64 / CHORDS as f64;
            let u = 1.0 - t;
            contour.push([0, 1].map(|k| u * u * a[k] + 2.0 * u * t * c[k] + t * t * b[k]));
        }
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let a = self.last();
        let (c1, c2, b) = ([c1x, c1y].map(f64::from), [c2x, c2y].map(f64::from), [x, y].map(f64::from));
        let contour = self.contours.last_mut().unwrap();
        for i in 1..=CHORDS {
            let t = i as f64 / CHORDS as f64;
            let u = 1.0 - t;
            contour.push([0, 1].map(|k| {
                u * u * u * a[k] + 3.0 * u * u * t * c1[k] + 3.0 * u * t * t * c2[k] + t * t * t * b[k]
            }));
        }
    }
    fn close(&mut self) {}
}

/// Coverage of `contours` (pixels, y up) under `rule` on the grid of an image whose top left corner is `left`
/// across and `top` up, `width` by `height`, row by row.
pub(super) fn coverage(
    contours: &[Vec<[f64; 2]>],
    rule: FillRule,
    left: i32,
    top: i32,
    width: usize,
    height: usize,
) -> Vec<f64> {
    let mut rows: Vec<Vec<([f64; 2], [f64; 2])>> = vec![Vec::new(); height];
    for contour in contours {
        for i in 0..contour.len() {
            let (p, q) = (contour[i], contour[(i + 1) % contour.len()]);
            let a = [p[0] - f64::from(left), f64::from(top) - p[1]];
            let b = [q[0] - f64::from(left), f64::from(top) - q[1]];
            if a[1] == b[1] {
                continue;
            }
            let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
            let first = (lo.floor().max(0.0) as usize).min(height);
            let last = (hi.ceil().max(0.0) as usize).min(height);
            for row in &mut rows[first..last] {
                row.push((a, b));
            }
        }
    }
    let mut out = vec![0.0; width * height];
    let (mut crossings, mut cuts) = (Vec::new(), Vec::new());
    for (r, edges) in rows.iter().enumerate() {
        let (r0, r1) = (r as f64, r as f64 + 1.0);
        cuts.clear();
        cuts.extend(edges.iter().flat_map(|&(a, b)| [a[1], b[1]]));
        for &(a, b) in edges {
            let (lo, hi) = (a[0].min(b[0]).ceil() as i64, a[0].max(b[0]).floor() as i64);
            cuts.extend((lo..=hi).map(|x| a[1] + (x as f64 - a[0]) * (b[1] - a[1]) / (b[0] - a[0])));
        }
        let mut by_x: Vec<_> = edges.iter().map(|&(a, b)| (a[0].min(b[0]), a[0].max(b[0]), a, b)).collect();
        by_x.sort_by(|p, q| p.0.total_cmp(&q.0));
        for (i, &(_, reach, a, b)) in by_x.iter().enumerate() {
            for &(x0, _, c, d) in &by_x[i + 1..] {
                if x0 > reach {
                    break;
                }
                cuts.extend(crossing(a, b, c, d));
            }
        }
        cuts.retain(|&y| y > r0 && y < r1);
        cuts.extend([r0, r1]);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        for band in cuts.windows(2) {
            let (y0, h) = (band[0], band[1] - band[0]);
            let lines = ((h * LINES as f64).ceil() as usize).max(2);
            for k in 0..lines {
                let y = y0 + h * (k as f64 + 0.5) / lines as f64;
                crossings.clear();
                for &(a, b) in edges {
                    let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
                    if y >= lo && y < hi {
                        let x = a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                        crossings.push((x, if b[1] > a[1] { 1 } else { -1 }));
                    }
                }
                crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut winding = 0;
                for w in 0..crossings.len() {
                    winding += crossings[w].1;
                    let inside = match rule {
                        FillRule::EvenOdd => winding & 1 != 0,
                        _ => winding != 0,
                    };
                    if inside && w + 1 < crossings.len() {
                        add(&mut out[r * width..(r + 1) * width], crossings[w].0, crossings[w + 1].0, h / lines as f64);
                    }
                }
            }
        }
    }
    out
}

// Where segments ab and cd cross, strictly within both.
fn crossing(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<f64> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    if den == 0.0 {
        return None;
    }
    let (e0, e1) = (c[0] - a[0], c[1] - a[1]);
    let (t, u) = ((e0 * s[1] - e1 * s[0]) / den, (e0 * r[1] - e1 * r[0]) / den);
    (t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0).then(|| a[1] + t * r[1])
}

fn add(row: &mut [f64], x0: f64, x1: f64, weight: f64) {
    let (x0, x1) = (x0.max(0.0), x1.min(row.len() as f64));
    if x1 <= x0 {
        return;
    }
    let (first, last) = (x0.floor() as usize, (x1.ceil() as usize).min(row.len()));
    for (c, v) in row.iter_mut().enumerate().take(last).skip(first) {
        let (a, b) = (x0.max(c as f64), x1.min(c as f64 + 1.0));
        if b > a {
            *v += (b - a) * weight;
        }
    }
}
