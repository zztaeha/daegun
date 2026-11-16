use alloc::vec::Vec;

use super::{cross, dot, Chords, Segment, END, START, V, WHITE};
use crate::daecore::daetype::outline::{OutlinePen, Path};

const ID: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn built(path: &Path, tolerance: f64) -> Chords {
    let mut c = Chords::default();
    assert!(c.build(path, &ID, tolerance, 1 << 20));
    c
}

// A square from (x0, y0) to (x1, y1), counterclockwise, or clockwise if `back`.
fn square(p: &mut Path, [x0, y0, x1, y1]: [f32; 4], back: bool) {
    let mut corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    if back {
        corners.reverse();
    }
    p.move_to(corners[0].0, corners[0].1);
    corners[1..].iter().for_each(|&(x, y)| p.line_to(x, y));
    p.close();
}

fn chord_distance(q: V, a: V, b: V) -> f64 {
    let (ab, aq) = ([b[0] - a[0], b[1] - a[1]], [q[0] - a[0], q[1] - a[1]]);
    let t = (dot(aq, ab) / dot(ab, ab)).clamp(0.0, 1.0);
    let d = [aq[0] - ab[0] * t, aq[1] - ab[1] * t];
    dot(d, d).sqrt()
}

// Each segment's chords start and end on its curve's ends, and no point of the curve strays further from them.
#[test]
fn chords_keep_within_the_tolerance() {
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.quad_to(30.0, 40.0, 60.0, 0.0);
    p.curve_to(50.0, -30.0, 10.0, 25.0, 0.0, 0.0);
    p.close();
    let quad = |t: f64| {
        let u = 1.0 - t;
        [0, 1].map(|k| {
            let (a, c, b) = ([0.0, 0.0][k], [30.0, 40.0][k], [60.0, 0.0][k]);
            u * u * a + 2.0 * u * t * c + t * t * b
        })
    };
    let cubic = |t: f64| {
        let u = 1.0 - t;
        [0, 1].map(|k| {
            let (a, c, d, b) = ([60.0, 0.0][k], [50.0, -30.0][k], [10.0, 25.0][k], [0.0, 0.0][k]);
            u * u * u * a + 3.0 * u * u * t * c + 3.0 * u * t * t * d + t * t * t * b
        })
    };
    for tolerance in [0.5, 0.01, 0.0005] {
        let c = built(&p, tolerance);
        assert_eq!(c.segments.len(), 2);
        let curves: [&dyn Fn(f64) -> V; 2] = [&quad, &cubic];
        for (s, curve) in c.segments.iter().zip(curves) {
            let pts = &c.pts[s.first..=s.last];
            assert_eq!((pts[0], pts[pts.len() - 1]), (curve(0.0), curve(1.0)));
            for i in 0..=2000 {
                let q = curve(f64::from(i) / 2000.0);
                let d = pts.windows(2).map(|w| chord_distance(q, w[0], w[1])).fold(f64::MAX, f64::min);
                assert!(d <= tolerance * (1.0 + 1e-9), "{d} past {tolerance} at {q:?}");
            }
        }
    }
}

fn colors(c: &Chords) -> Vec<u8> {
    c.segments.iter().map(|s| s.color).collect()
}

// Two segments meeting at a corner share exactly one channel, so the median of three keeps the corner.
fn meet(a: u8, b: u8) -> bool {
    a.count_ones() == 2 && b.count_ones() == 2 && (a & b).count_ones() == 1
}

#[test]
fn corners_split_the_channels() {
    // A square: four corners, each run's color unlike both its neighbors', the last's too.
    let mut p = Path::new();
    square(&mut p, [0.0, 0.0, 10.0, 10.0], false);
    let c = colors(&built(&p, 0.01));
    assert_eq!(c.len(), 4);
    assert!((0..4).all(|i| meet(c[i], c[(i + 1) % 4])), "{c:?}");
    // A smooth loop: no corner, so every channel measures to all of it.
    let mut p = Path::new();
    p.move_to(10.0, 0.0);
    let quarters = [(10.0, 10.0, 0.0, 10.0), (-10.0, 10.0, -10.0, 0.0), (-10.0, -10.0, 0.0, -10.0)];
    for (cx, cy, x, y) in quarters.into_iter().chain([(10.0, -10.0, 10.0, 0.0)]) {
        p.quad_to(cx, cy, x, y);
    }
    p.close();
    assert_eq!(colors(&built(&p, 0.01)), [WHITE; 4]);
    // A teardrop: one corner, the curve cut into three, a color leaving the corner and another arriving, white
    // between.
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.curve_to(40.0, 30.0, 40.0, -30.0, 0.0, 0.0);
    p.close();
    let c = colors(&built(&p, 0.01));
    assert!(c.len() >= 3 && meet(c[0], c[c.len() - 1]) && c.contains(&WHITE), "{c:?}");
}

#[test]
fn the_inside_is_where_the_area_says() {
    for back in [false, true] {
        let mut p = Path::new();
        square(&mut p, [0.0, 0.0, 10.0, 10.0], back);
        let mut c = built(&p, 0.01);
        c.orient();
        assert!(c.segments.iter().all(|s| s.left == !back));
    }
}

// The winding about `q` read along a ray from every chord, as a sweep reads it.
fn winding(c: &Chords, q: V) -> i32 {
    let pts = &c.pts;
    c.segments.iter().flat_map(|s| s.first..s.last).map(|j| (pts[j], pts[j + 1])).filter(|(a, b)| {
        (a[1] <= q[1]) != (b[1] <= q[1]) && a[0] + (q[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) > q[0]
    }).map(|(a, b)| if b[1] > a[1] { 1 } else { -1 }).sum()
}

// Every pair of chords for `bound` to test, and the windings it asks for read from every chord.
fn bounded(c: &mut Chords) {
    let n = c.pts.len() as u32;
    let pairs: Vec<(u32, u32)> = (0..n).flat_map(|j| (j + 1..n).map(move |k| (j, k))).collect();
    let pairs: Vec<(u32, u32)> = pairs.into_iter().filter(|&(j, k)| {
        let is = |j: u32| c.segments.iter().any(|s| (s.first..s.last).contains(&(j as usize)));
        is(j) && is(k)
    }).collect();
    let copy = Chords { pts: c.pts.clone(), segments: c.segments.clone(), ..Chords::default() };
    c.bound(&pairs, &mut |points, out| out.extend(points.iter().map(|&q| winding(&copy, q))));
}

fn length(c: &Chords, runs: &[Segment]) -> f64 {
    let pts = &c.kept;
    let chord = |w: &[V]| dot([w[1][0] - w[0][0], w[1][1] - w[0][1]], [w[1][0] - w[0][0], w[1][1] - w[0][1]]).sqrt();
    runs.iter().flat_map(|s| pts[s.first..=s.last].windows(2)).map(chord).sum()
}

// Two squares overlapping keep only their union's edge, 60 long; two sharing a side keep neither copy of it.
// Each piece kept has its inside on the side it says, and an end of its own only where its curve's end is.
#[test]
fn only_what_bounds_the_inside_is_kept() {
    for (b, perimeter) in [([5.0, 5.0, 15.0, 15.0], 60.0), ([10.0, 0.0, 20.0, 10.0], 60.0)] {
        let mut p = Path::new();
        square(&mut p, [0.0, 0.0, 10.0, 10.0], false);
        square(&mut p, b, false);
        let mut c = built(&p, 0.01);
        bounded(&mut c);
        let kept = (length(&c, &c.runs) - perimeter).abs();
        assert!(kept < 1e-9, "{b:?}: {} kept", length(&c, &c.runs));
        for s in &c.runs {
            for w in c.kept[s.first..=s.last].windows(2) {
                let m = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
                let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
                let n = [-d[1] * 1e-3, d[0] * 1e-3];
                let winding = |q: V| winding(&c, q);
                let (l, r) = (winding([m[0] + n[0], m[1] + n[1]]), winding([m[0] - n[0], m[1] - n[1]]));
                assert_eq!((l != 0, r != 0), (s.left, !s.left), "{b:?}: a piece at {m:?}");
            }
            let at = |k: usize| c.kept[k];
            let corner = |q: V| [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]].contains(&q)
                || [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|v| v.map(f64::from)).contains(&q);
            assert_eq!(s.ends & START != 0, corner(at(s.first)), "{b:?}: a run's start at {:?}", at(s.first));
            assert_eq!(s.ends & END != 0, corner(at(s.last)), "{b:?}: a run's end at {:?}", at(s.last));
            assert!(cross(s.start, s.end).abs() < 1e-12 && dot(s.start, s.end) > 0.0, "a side runs straight");
        }
    }
}
