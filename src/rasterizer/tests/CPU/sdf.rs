// Distance fields held to the truth: each texel's distance, in f64, to the edges of the outline flattened 256 chords
// a curve that bound what it fills, signed by the winding at its center.
use std::vec::Vec;

use super::coverage::{font, sample, FONTS};
use crate::daecore::daetype::outline::Verb;
use crate::rasterizer::Shape;
use crate::raster::{Bitmap, Field, Rasterizer};
use crate::{Font, GlyphId, Location};

type V = [f64; 2];

const CHORDS: usize = 256;

// A glyph's learned outline as chords in pixels, and whether its overlaps were left in.
fn outline(r: &Rasterizer, g: GlyphId, s: f64) -> (Vec<(V, V)>, bool) {
    let (path, shape) = &r.learned.last().unwrap().glyphs[&g.0];
    let place = |p: &crate::daecore::daetype::outline::Point| [f64::from(p.x) * s, f64::from(p.y) * s];
    let (mut points, mut chords) = (Vec::new(), Vec::new());
    let mut close = |points: &mut Vec<V>| {
        chords.extend(points.windows(2).map(|w| (w[0], w[1])).chain(points.last().map(|&l| (l, points[0]))));
        points.clear();
    };
    for (verb, ps) in path.segments() {
        let ps: Vec<V> = ps.iter().map(place).collect();
        match verb {
            Verb::Move => {
                close(&mut points);
                points.push(ps[0]);
            }
            Verb::Line => points.push(ps[0]),
            Verb::Quad | Verb::Cubic => {
                let c = [&points[points.len() - 1..], &ps[..]].concat();
                for i in 1..=CHORDS {
                    let t = i as f64 / CHORDS as f64;
                    let mut q = c.clone();
                    while q.len() > 1 {
                        q = q.windows(2).map(|w| [0, 1].map(|k| w[0][k] + (w[1][k] - w[0][k]) * t)).collect();
                    }
                    points.push(q[0]);
                }
            }
            Verb::Close => close(&mut points),
        }
    }
    close(&mut points);
    chords.retain(|(a, b)| a != b);
    (chords, *shape == Shape::Unsure)
}

fn winding(chords: &[(V, V)], x: f64, y: f64) -> i32 {
    let crossing =
        |(a, b): &&(V, V)| (a[1] <= y) != (b[1] <= y) && a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) > x;
    chords.iter().filter(crossing).map(|(a, b)| if b[1] > a[1] { 1 } else { -1 }).sum()
}

// The pieces of `chords` with what they fill on one side only: each cut wherever another crosses or touches it.
fn boundary(chords: &[(V, V)]) -> Vec<(V, V)> {
    let cross = |u: V, v: V| u[0] * v[1] - u[1] * v[0];
    let sub = |u: V, v: V| [u[0] - v[0], u[1] - v[1]];
    let mut out = Vec::new();
    for &(a, b) in chords {
        let ab = sub(b, a);
        let mut ts = vec![0.0, 1.0];
        for &(c, d) in chords {
            let (o1, o2) = (cross(ab, sub(c, a)), cross(ab, sub(d, a)));
            let cd = sub(d, c);
            let (o3, o4) = (cross(cd, sub(a, c)), cross(cd, sub(b, c)));
            if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
                ts.push(o3 / (o3 - o4));
            }
            for (p, o) in [(c, o1), (d, o2)] {
                let t = (sub(p, a)[0] * ab[0] + sub(p, a)[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]);
                if o == 0.0 && t > 0.0 && t < 1.0 {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        let at = |t: f64| [a[0] + ab[0] * t, a[1] + ab[1] * t];
        for w in ts.windows(2).filter(|w| w[1] > w[0]) {
            let (p, q) = (at(w[0]), at(w[1]));
            let m = at((w[0] + w[1]) * 0.5);
            let len = (ab[0] * ab[0] + ab[1] * ab[1]).sqrt();
            let n = [-ab[1] / len * 1e-7, ab[0] / len * 1e-7];
            let l = winding(chords, m[0] + n[0], m[1] + n[1]) != 0;
            let r = winding(chords, m[0] - n[0], m[1] - n[1]) != 0;
            if l != r {
                out.push((p, q));
            }
        }
    }
    out
}

// The signed distance from each texel's center of `image` to `edges`, at most `spread` either way, inside
// positive by the winding of `chords`: each row's edges within reach, by brute force.
fn truth(edges: &[(V, V)], chords: &[(V, V)], image: &Bitmap, spread: f64) -> Vec<f64> {
    let (w, h) = (image.width as usize, image.height as usize);
    let mut out = Vec::with_capacity(w * h);
    for row in 0..h {
        let y = f64::from(image.top) - row as f64 - 0.5;
        let reach = |(a, b): &&(V, V)| a[1].min(b[1]) - y <= spread && y - a[1].max(b[1]) <= spread;
        let near: Vec<_> = edges.iter().filter(reach).collect();
        for col in 0..w {
            let x = f64::from(image.left) + col as f64 + 0.5;
            let mut d2 = spread * spread;
            for (a, b) in &near {
                let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [x - a[0], y - a[1]]);
                let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1])).clamp(0.0, 1.0);
                let (dx, dy) = (ap[0] - t * ab[0], ap[1] - t * ab[1]);
                d2 = d2.min(dx * dx + dy * dy);
            }
            out.push(if winding(chords, x, y) != 0 { d2.sqrt() } else { -d2.sqrt() });
        }
    }
    out
}

fn level(d: f64, spread: f64) -> f64 {
    (127.5 + 127.5 * d / spread).clamp(0.0, 255.0)
}

// Glyphs of the first fonts whose overlaps are left in, curves in them touching too close to call.
fn left_in(name: &str) -> impl Iterator<Item = GlyphId> {
    let gids: &[u16] = match name {
        "inter/InterVariable.ttf" => &[741, 918, 2638],
        "source-serif/SourceSerif4Variable-Roman.otf" => &[204, 206, 1292],
        _ => &[],
    };
    gids.iter().map(|&g| GlyphId(g.into()))
}

fn field(r: &mut Rasterizer, font: &Font, g: GlyphId, px: f32, field: Field) -> Bitmap {
    r.distance_field(font, g, &Location::default(), px, field).unwrap().to_bitmap()
}

// Each texel within its rounding and the chords' eighth of a level of the truth.
#[test]
fn sdf_levels_the_distance_to_the_outline() {
    let mut r = Rasterizer::new();
    let (mut drawn, mut overlaps) = (0, 0);
    for name in FONTS.iter().take(4) {
        let Some(font) = font(name) else { continue };
        for g in sample(&font, 8).chain(left_in(name)) {
            for (px, spread) in [(16.0, 2.0), (48.0, 4.0), (128.0, 8.0)] {
                let image = field(&mut r, &font, g, px, Field::Sdf { spread });
                assert_eq!(image.channels, 1);
                let s = f64::from(px) / f64::from(font.units_per_em());
                let (chords, overlapping) = outline(&r, g, s);
                let edges = if overlapping { boundary(&chords) } else { chords.clone() };
                overlaps += usize::from(overlapping);
                let truth = truth(&edges, &chords, &image, f64::from(spread));
                for (i, (&v, &d)) in image.pixels.iter().zip(&truth).enumerate() {
                    let gap = (f64::from(v) - level(d, f64::from(spread))).abs();
                    assert!(gap <= 0.5 + 1.0 / 8.0, "{name} glyph {} at {px} px: texel {i} {gap} levels", g.0);
                }
                drawn += 1;
            }
        }
    }
    assert!(drawn > 60 && overlaps > 10, "only {drawn} fields drawn, {overlaps} with overlaps left in");
}

// The median of three.
fn median(v: [f64; 3]) -> f64 {
    v[0].min(v[1]).max(v[0].max(v[1]).min(v[2]))
}

// The field read back between texel centers, bilinear, at `q` (pixels, y up): its channels' median, or its one.
fn read(image: &Bitmap, q: V) -> f64 {
    let (w, h, ch) = (image.width as usize, image.height as usize, usize::from(image.channels));
    let fx = (q[0] - f64::from(image.left) - 0.5).clamp(0.0, (w - 1) as f64);
    let fy = (f64::from(image.top) - q[1] - 0.5).clamp(0.0, (h - 1) as f64);
    let (x0, y0) = ((fx as usize).min(w.saturating_sub(2)), (fy as usize).min(h.saturating_sub(2)));
    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
    let at = |x: usize, y: usize, c: usize| f64::from(image.pixels[(y.min(h - 1) * w + x.min(w - 1)) * ch + c]);
    let v = |c: usize| {
        let top = at(x0, y0, c) * (1.0 - tx) + at(x0 + 1, y0, c) * tx;
        let bottom = at(x0, y0 + 1, c) * (1.0 - tx) + at(x0 + 1, y0 + 1, c) * tx;
        top * (1.0 - ty) + bottom * ty
    };
    if ch == 3 { median([v(0), v(1), v(2)]) } else { v(0) }
}

fn distance_to(edges: &[(V, V)], q: V) -> f64 {
    let to = |(a, b): &(V, V)| {
        let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [q[0] - a[0], q[1] - a[1]]);
        let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1])).clamp(0.0, 1.0);
        ((ap[0] - t * ab[0]).powi(2) + (ap[1] - t * ab[1]).powi(2)).sqrt()
    };
    edges.iter().map(to).fold(f64::MAX, f64::min)
}

// Each point of a row at height `y` from `xs`, ascending, whether `chords` wind it.
fn wound(chords: &[(V, V)], y: f64, xs: impl Iterator<Item = f64>) -> Vec<bool> {
    let mut crossings: Vec<(f64, i32)> = chords
        .iter()
        .filter(|(a, b)| (a[1] <= y) != (b[1] <= y))
        .map(|(a, b)| (a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]), if b[1] > a[1] { 1 } else { -1 }))
        .collect();
    crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
    let (mut at, mut winding) = (0, 0);
    xs.map(|x| {
        while crossings.get(at).is_some_and(|c| c.0 <= x) {
            winding -= crossings[at].1;
            at += 1;
        }
        winding != 0
    })
    .collect()
}

// Every texel's median on the truth's side, past rounding. Read between texels 4 by 4 a texel, fewer points land
// on the wrong side than SDF puts there, and none SDF reads right lies farther than half a texel from the edges,
// nearer than which a grid of texels can't place an edge.
#[test]
fn msdf_keeps_its_medians_inside_and_reads_closer_than_sdf() {
    let mut r = Rasterizer::new();
    let (mut drawn, mut overlaps) = (0, 0);
    for name in FONTS.iter().take(4) {
        let Some(font) = font(name) else { continue };
        let (mut sdf, mut msdf) = (0, 0);
        let named = [font.glyph_id('E'), font.glyph_id('k')].into_iter().flatten();
        for g in sample(&font, 16).chain(named).chain(left_in(name)) {
            for (px, spread) in [(24.0, 2.0), (64.0, 4.0)] {
                let one = field(&mut r, &font, g, px, Field::Sdf { spread });
                let three = field(&mut r, &font, g, px, Field::Msdf { spread });
                assert_eq!((three.channels, three.left, three.top, three.width), (3, one.left, one.top, one.width));
                let s = f64::from(px) / f64::from(font.units_per_em());
                let (chords, overlapping) = outline(&r, g, s);
                let edges = if overlapping { boundary(&chords) } else { chords.clone() };
                let spread = f64::from(spread);
                for (i, &d) in truth(&edges, &chords, &three, spread).iter().enumerate() {
                    let m = median([0, 1, 2].map(|c| f64::from(three.pixels[i * 3 + c])));
                    let want = level(d, spread);
                    let near = (want - 127.5).abs() <= 0.5 + 1.0 / 8.0;
                    assert!(near || (m - 127.5) * (want - 127.5) > 0.0, "{name} glyph {} at {px} px: texel {i}", g.0);
                }
                let (w, h) = (one.width as usize, one.height as usize);
                if w < 2 || h < 2 {
                    continue;
                }
                let (left, top) = (f64::from(one.left) + 0.5, f64::from(one.top) - 0.5);
                for y in 0..(h - 1) * 4 {
                    let y = top - (y as f64 + 0.5) / 4.0;
                    let xs = (0..(w - 1) * 4).map(|x| left + (x as f64 + 0.5) / 4.0);
                    for (x, ink) in xs.clone().zip(wound(&chords, y, xs)) {
                        let (a, b) = ((read(&one, [x, y]) > 127.5) != ink, (read(&three, [x, y]) > 127.5) != ink);
                        (sdf, msdf) = (sdf + usize::from(a), msdf + usize::from(b));
                        if b && !a {
                            let far = distance_to(&edges, [x, y]);
                            assert!(far <= 0.5, "{name} glyph {} at {px} px: ({x}, {y}) {far} px in", g.0);
                        }
                    }
                }
                (drawn, overlaps) = (drawn + 1, overlaps + usize::from(overlapping));
            }
        }
        assert!(msdf < sdf, "{name}: {msdf} points read wrong, {sdf} by SDF");
    }
    assert!(drawn > 100 && overlaps > 10, "only {drawn} fields drawn, {overlaps} with overlaps left in");
}

// Past both sides' ends outside a rectangle's corner, the median is the distance to the sides' lines carried on, the
// larger offset, where the plain distance runs to the corner itself: Inter's `I`, a rectangle drawn clockwise.
#[test]
fn msdf_carries_a_corner_s_sides_on_past_it() {
    let Some(font) = font(FONTS[0]) else { return };
    let g = font.glyph_id('I').unwrap();
    let (px, spread) = (64.0, 4.0);
    let mut r = Rasterizer::new();
    let image = field(&mut r, &font, g, px, Field::Msdf { spread });
    let (path, _) = &r.learned.last().unwrap().glyphs[&g.0];
    let corners: Vec<(f32, f32)> = path.parts().1.iter().map(|p| (p.x, p.y)).collect();
    assert_eq!(corners, [(370.0, 1490.0), (370.0, 0.0), (180.0, 0.0), (180.0, 1490.0)]);
    let s = f64::from(px) / f64::from(font.units_per_em());
    let (x0, x1, y0, y1) = (180.0 * s, 370.0 * s, 0.0, 1490.0 * s);
    let (w, mut seen) = (image.width as usize, 0);
    for (i, p) in image.pixels.as_chunks::<3>().0.iter().enumerate() {
        let (x, y) = (f64::from(image.left) + (i % w) as f64 + 0.5, f64::from(image.top) - (i / w) as f64 - 0.5);
        let (a, b) = ((x0 - x).max(x - x1), (y0 - y).max(y - y1));
        if a > 0.25 && b > 0.25 && a.hypot(b) < f64::from(spread) {
            let m = median([p[0], p[1], p[2]].map(f64::from));
            let want = level(-a.max(b), f64::from(spread));
            assert!((m - want).abs() <= 0.5 + 1.0 / 8.0, "({x}, {y}): {m}, not {want}");
            seen += 1;
        }
    }
    assert!(seen >= 16, "only {seen} texels past the corners");
}

// A one-unit square at x 32,766 in a font of 16 units to the em, drawn at 6,400 px, lies 13 million pixels right:
// measured in f64, its field is the square at 0's, moved.
#[test]
fn a_field_past_ten_million_pixels_is_the_near_one_moved() {
    use crate::{ErrorKind, Tag};
    let words = |w: &[i16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let square = |x: i16| [words(&[1, x, 0, x + 1, 1, 3, 0]), vec![1; 4], words(&[x, 1, 0, -1, 0, 0, 1, 0])].concat();
    let (near, far) = (square(0), square(32_766));
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&16u16.to_be_bytes());
    head[51] = 1;
    let ends = [0, 0, near.len() as u32, (near.len() + far.len()) as u32];
    let tables: alloc::collections::BTreeMap<Tag, Vec<u8>> = [
        (*b"head", head),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 3]),
        (*b"loca", ends.map(u32::to_be_bytes).concat()),
        (*b"glyf", [near, far].concat()),
    ]
    .into_iter()
    .map(|(tag, t)| (Tag::new(&tag), t))
    .collect();
    let font = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the font parses");
    let mut r = Rasterizer::new();
    let at = Location::default();
    let sdf = Field::Sdf { spread: 4.0 };
    let mut field = |g| r.distance_field(&font, GlyphId(g), &at, 6_400.0, sdf).unwrap().to_bitmap();
    let (near, far) = (field(1), field(2));
    let moved = (far.left - near.left, far.top, far.width, far.height);
    assert_eq!(moved, (32_766 * 400, near.top, near.width, near.height));
    assert_eq!(far.pixels, near.pixels);
    let _ = ErrorKind::InvalidArgument;
}

// A sun of 300 quadratic spikes round a circle of radius 1,000 units, at 20,000 pixels a unit, would take millions of
// chords; like its polygon its field is past MAX_SIDE, and both are refused as too large before either is flattened.
#[test]
fn a_curvy_field_past_max_side_is_too_large_as_a_straight_one_is() {
    use crate::daecore::daetype::outline::{OutlinePen, Path};
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    r.distance_field(&font, GlyphId(1), &Location::default(), 16.0, Field::Sdf { spread: 4.0 }).map(drop).unwrap();
    let sun = |curved: bool| {
        let mut p = Path::new();
        let at = |k: f64, radius: f64| {
            let (s, c) = (k * core::f64::consts::TAU / 600.0).sin_cos();
            ((radius * c) as f32, (radius * s) as f32)
        };
        p.move_to(at(0.0, 1_000.0).0, at(0.0, 1_000.0).1);
        for i in 0..300 {
            let ((cx, cy), (x, y)) = (at(f64::from(2 * i + 1), 1_300.0), at(f64::from(2 * i + 2), 1_000.0));
            if curved {
                p.quad_to(cx, cy, x, y);
            } else {
                p.line_to(cx, cy);
                p.line_to(x, y);
            }
        }
        p.close();
        p
    };
    for curved in [false, true] {
        r.learned.last_mut().unwrap().glyphs.get_mut(&1).unwrap().0 = sun(curved);
        let m = [20_000.0, 0.0, 0.0, 20_000.0, 0.0, 0.0];
        let kind = r.distance(r.learned.len() - 1, 1, &m, 4.0, false).map(drop).map_err(|e| e.kind());
        assert_eq!(kind, Err(crate::ErrorKind::LimitExceeded), "curved: {curved}");
    }
}

// Chords at random, all below 0, with level ones some end on, start on and run along: the sweep pairs every two that
// cross or touch and reads every winding as a ray going left counts it.
#[test]
fn the_sweep_meets_every_pair_and_reads_every_winding() {
    use crate::rasterizer::sweep::{Chain, Pt, Sweep};
    let mut seed = 7u64;
    let mut next = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / f64::from(1u32 << 31)
    };
    let (mut ends, mut lines) = (Vec::new(), Vec::new());
    let mut line = |a: Pt, b: Pt| {
        let (a, b, dir) = if a.y < b.y { (a, b, 1) } else if a.y > b.y { (b, a, -1) } else { (a, b, 0) };
        let start = ends.len() as u32;
        ends.extend([a, b]);
        lines.push(Chain { start, end: start + 2, dir });
    };
    for _ in 0..60 {
        let (a, b) = (Pt { x: next() * 100.0, y: -next() * 100.0 }, Pt { x: next() * 100.0, y: -next() * 100.0 });
        line(a, b);
    }
    let level = -50.0;
    line(Pt { x: 10.0, y: level }, Pt { x: 60.0, y: level });
    line(Pt { x: 20.0, y: level }, Pt { x: 30.0, y: level });
    line(Pt { x: 40.0, y: level - 9.0 }, Pt { x: 40.0, y: level });
    line(Pt { x: 50.0, y: level }, Pt { x: 52.0, y: level + 9.0 });
    let mut queries: Vec<(f64, f64)> = (0..400).map(|_| (-next() * 100.0, next() * 100.0)).collect();
    queries.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let (mut sweep, mut windings, mut pairs) = (Sweep::default(), Vec::new(), Vec::new());
    sweep.allowed = u64::MAX;
    assert!(sweep.run(&ends, &lines, &queries, &mut windings, |a, b| pairs.push((a.min(b) / 2, a.max(b) / 2))));
    pairs.sort_unstable();
    pairs.dedup();
    let cross = |u: Pt, v: Pt| u.x * v.y - u.y * v.x;
    let sub = |u: Pt, v: Pt| Pt { x: u.x - v.x, y: u.y - v.y };
    for j in 0..lines.len() {
        for k in j + 1..lines.len() {
            let ((a, b), (c, d)) = ((ends[2 * j], ends[2 * j + 1]), (ends[2 * k], ends[2 * k + 1]));
            let (ab, cd) = (sub(b, a), sub(d, c));
            let (o1, o2) = (cross(ab, sub(c, a)), cross(ab, sub(d, a)));
            let (o3, o4) = (cross(cd, sub(a, c)), cross(cd, sub(b, c)));
            let within = |p: Pt, a: Pt, b: Pt| {
                (a.x.min(b.x)..=a.x.max(b.x)).contains(&p.x) && (a.y.min(b.y)..=a.y.max(b.y)).contains(&p.y)
            };
            let meet = o1 * o2 < 0.0 && o3 * o4 < 0.0
                || o1 == 0.0 && within(c, a, b)
                || o2 == 0.0 && within(d, a, b)
                || o3 == 0.0 && within(a, c, d)
                || o4 == 0.0 && within(b, c, d);
            assert!(!meet || pairs.binary_search(&(j as u32, k as u32)).is_ok(), "{j} and {k} meet unpaired");
        }
    }
    for (&(y, x), &w) in queries.iter().zip(&windings) {
        let left = lines.iter().filter(|l| {
            let (a, b) = (ends[l.start as usize], ends[l.start as usize + 1]);
            a.y < y && y <= b.y && a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y) <= x
        });
        assert_eq!(w, left.map(|l| l.dir).sum::<i32>(), "at ({x}, {y})");
    }
}


// A field 10^15 pixels along lies past what an image's i32 corner can say: refused, not saturated.
#[test]
fn a_field_past_what_an_image_s_corner_holds_is_refused() {
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let g = font.glyph_id('H').expect("Inter maps H");
    let mut r = Rasterizer::new();
    r.distance_field(&font, g, &Location::default(), 16.0, Field::Sdf { spread: 4.0 }).map(drop).unwrap();
    let along = [0.01, 0.0, 0.0, 0.01, 1.0e15, 0.0];
    let kind = r.distance(r.learned.len() - 1, g.0, &along, 4.0, false).map(drop).map_err(|e| e.kind());
    assert_eq!(kind, Err(crate::ErrorKind::LimitExceeded));
}
