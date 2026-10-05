use daegun::{Cap, Font, Join, OutlinePen, Path, StrokeStyle};

use crate::stroke::winding;

fn path(contours: &[(&[(f32, f32)], bool)]) -> Path {
    let mut p = Path::default();
    for &(points, closed) in contours {
        p.move_to(points[0].0, points[0].1);
        points[1..].iter().for_each(|q| p.line_to(q.0, q.1));
        if closed {
            p.close();
        }
    }
    p
}

fn style(width: f32, join: Join, cap: Cap) -> StrokeStyle {
    StrokeStyle { width, join, cap }
}

fn rings(draw: fn(&Path, &StrokeStyle, f32, &mut dyn OutlinePen), p: &Path, s: &StrokeStyle, tolerance: f32) -> Vec<Vec<(f32, f32)>> {
    let mut out = Path::default();
    draw(p, s, tolerance, &mut out);
    daegun::flatten(&out, 1.0).expect("within the cap")
}

fn covers(p: &Path, s: &StrokeStyle, at: &[(f32, f32)]) {
    for draw in [daegun::stroke, daegun::stroke_simplified] {
        let r = rings(draw, p, s, 0.05);
        for &(x, y) in at {
            assert_ne!(winding(&r, x, y), 0, "{s:?}: ({x}, {y}) is within half the width of the path but bare");
        }
    }
}

const JOINS: [Join; 3] = [Join::Round, Join::Miter { limit: 4.0 }, Join::Bevel];

// The normals of a U-turn have no cross product, as on a straight line, but point apart.
#[test]
fn a_path_that_turns_back_keeps_its_width() {
    for join in JOINS {
        let s = style(2.0, join, Cap::Butt);
        covers(&path(&[(&[(0.0, 0.0), (10.0, 0.0), (5.0, 0.0)], false)]), &s, &[(8.0, 0.5), (7.0, 0.9), (8.0, -0.5)]);
        let retrace: &[(f32, f32)] = &[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (10.0, 0.0)];
        covers(&path(&[(retrace, true)]), &s, &[(5.0, -0.8), (15.0, 0.8), (5.0, 0.8), (15.0, -0.8)]);
        covers(&path(&[(&[(0.0, 0.0), (10.0, 0.0)], true)]), &s, &[(5.0, 0.5), (5.0, -0.5)]);
    }
    let round = style(2.0, Join::Round, Cap::Butt);
    covers(&path(&[(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (10.0, 0.0)], true)]), &round, &[(20.5, 0.0), (-0.5, 0.0)]);
}

#[test]
fn an_open_path_ending_where_it_began_keeps_its_last_segment() {
    let open: &[(f32, f32)] = &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)];
    covers(&path(&[(open, false)]), &style(2.0, Join::Miter { limit: 4.0 }, Cap::Butt), &[(5.0, 5.0), (2.0, 2.0)]);
}

// A segment drawn without a move starts where the pen is, as `flatten` reads it.
#[test]
fn a_contour_without_a_move_starts_where_the_last_ended() {
    let s = style(4.0, Join::Miter { limit: 4.0 }, Cap::Butt);
    let mut p = Path::default();
    p.line_to(100.0, 0.0);
    p.line_to(100.0, 100.0);
    covers(&p, &s, &[(50.0, 0.0), (10.0, 1.0)]);

    let mut p = path(&[(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], true)]);
    p.line_to(0.0, 100.0);
    p.line_to(-50.0, 50.0);
    p.line_to(-50.0, 0.0);
    p.close();
    covers(&p, &s, &[(0.0, 30.0), (-25.0, 0.0)]);
}

// Wider than the square, the inner offsets cross, and the inner loop, turned inside out, would cancel
// the ring over the middle, though every point there is within reach of an edge.
#[test]
fn a_stroke_wider_than_its_contour_covers_the_middle() {
    let square = path(&[(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true)]);
    for join in JOINS {
        for width in [10.5, 12.0, 16.0] {
            covers(&square, &style(width, join, Cap::Butt), &[(5.0, 5.0), (4.0, 6.0)]);
        }
    }
}

// Past r / tolerance = 8e8 a chord count can overflow to one; a finer tolerance never gives fewer.
#[test]
fn a_finer_tolerance_never_rounds_a_cap_less() {
    let line = path(&[(&[(0.0, 0.0), (1e6, 0.0)], false)]);
    let s = style(2e5, Join::Round, Cap::Round);
    let count = |tolerance: f32| {
        let mut out = Path::default();
        daegun::stroke(&line, &s, tolerance, &mut out);
        out.parts().1.len()
    };
    assert!(count(1e-4) >= count(2e-4), "{} points at 1e-4, {} at 2e-4", count(1e-4), count(2e-4));
    let r = rings(daegun::stroke, &line, &s, 1e-4);
    let inside = 1e5 * 0.95 * std::f32::consts::FRAC_1_SQRT_2;
    assert_ne!(winding(&r, 1e6 + inside, inside), 0, "the cap's arc was cut to a chord");
}

// A lone point's dot winds as the rest of the stroke does, so one on a line adds to it.
#[test]
fn a_dot_on_a_line_does_not_cancel_it() {
    for cap in [Cap::Round, Cap::Square] {
        let p = path(&[(&[(0.0, 0.0), (10.0, 0.0)], false), (&[(5.0, 0.0)], false)]);
        covers(&p, &style(2.0, Join::Bevel, cap), &[(5.0, 0.2), (5.5, 0.5)]);
        let ring = path(&[(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true), (&[(5.0, 0.0)], false)]);
        covers(&ring, &style(2.0, Join::Bevel, cap), &[(5.0, 0.2)]);
    }
}

// A crossing 0.05 from a corner of the big square sits within EPS of the edge's length but not of its
// distance; split on one edge only, it would break the chain and drop part of the small one.
#[test]
fn a_crossing_near_a_corner_splits_both_edges() {
    let big = vec![(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];
    for left in [0.05, 0.09] {
        let small = vec![(left, -10.0), (20.0, -10.0), (20.0, 10.0), (left, 10.0)];
        let resolved = daegun::resolve_overlaps(&[big.clone(), small]).expect("two squares that overlap resolve");
        for (x, y) in [(5.0, -3.0), (1.0, -9.0), (19.0, -1.0), (500.0, 500.0)] {
            assert_ne!(winding(&resolved, x, y), 0, "left {left}: ({x}, {y}) was dropped");
        }
        assert_eq!(winding(&resolved, 30.0, -5.0), 0);
    }
}

// Inter's glyphs at a width that crosses most of them: the simplified stroke covers what the stroke
// covers, at font units and in ems, and the last three, which cannot close, come back as the stroke.
#[test]
fn a_simplified_stroke_covers_what_the_stroke_covers() {
    let f = Font::from_bytes(&std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).unwrap()).unwrap();
    for (gid, width, resolves) in [(35u16, 80.0f32, true), (121, 200.0, true), (36, 80.0, true), (4, 80.0, false), (111, 80.0, false), (119, 80.0, false)] {
        for scale in [1.0f32, 1.0 / 2048.0] {
            let mut glyph = Path::default();
            let k = f64::from(scale);
            f.outline_glyph(gid, &mut daegun::TransformPen::new(&mut glyph, [k, 0.0, 0.0, k, 0.0, 0.0])).expect("outline");
            let s = style(width * scale, Join::Round, Cap::Round);
            let (plain, simple) = (rings(daegun::stroke, &glyph, &s, 0.25 * scale), rings(daegun::stroke_simplified, &glyph, &s, 0.25 * scale));
            assert!(!resolves || plain != simple, "gid {gid} at {scale}: the union fell back to the plain stroke");
            let (x0, y0, x1, y1) = glyph.bounds().expect("ink");
            let pad = f64::from(width * scale);
            let (mut ink, mut differ) = (0, 0);
            for i in 0..60 {
                for j in 0..60 {
                    let x = (x0 - pad + (x1 - x0 + 2.0 * pad) * (f64::from(i) + 0.5) / 60.0) as f32;
                    let y = (y0 - pad + (y1 - y0 + 2.0 * pad) * (f64::from(j) + 0.5) / 60.0) as f32;
                    let covered = winding(&plain, x, y) != 0;
                    ink += i32::from(covered);
                    differ += i32::from(covered != (winding(&simple, x, y) != 0));
                }
            }
            assert!(differ * 500 < ink, "gid {gid} at {scale}: {differ} of {ink} samples differ once simplified");
        }
    }
}

// Within half the width of the outline and well clear of that edge, covered; well beyond it, bare.
fn matches_reach(glyph: &Path, s: &StrokeStyle, tolerance: f32) -> (usize, usize) {
    let outline = daegun::flatten(glyph, tolerance * 0.04).expect("within the cap");
    let r = f64::from(s.width) / 2.0;
    let near = |x: f64, y: f64| {
        outline.iter().flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()]))).fold(f64::MAX, |m, (a, b)| {
            let (ax, ay, bx, by) = (f64::from(a.0), f64::from(a.1), f64::from(b.0), f64::from(b.1));
            let (dx, dy) = (bx - ax, by - ay);
            let t = if dx * dx + dy * dy > 0.0 { (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0) } else { 0.0 };
            m.min(((x - ax - dx * t).powi(2) + (y - ay - dy * t).powi(2)).sqrt())
        })
    };
    let rings = rings(daegun::stroke, glyph, s, tolerance);
    let (x0, y0, x1, y1) = glyph.bounds().expect("ink");
    let (mut checked, mut wrong) = (0, 0);
    for i in 0..60 {
        for j in 0..60 {
            let x = x0 - r + (x1 - x0 + 2.0 * r) * (f64::from(i) + 0.5) / 60.0;
            let y = y0 - r + (y1 - y0 + 2.0 * r) * (f64::from(j) + 0.5) / 60.0;
            let d = near(x, y);
            if (d - r).abs() < 0.02 * r + f64::from(tolerance) {
                continue;
            }
            checked += 1;
            wrong += usize::from((winding(&rings, x as f32, y as f32) != 0) != (d < r));
        }
    }
    (checked, wrong)
}

// Where the offsets of a tight curve miss each other, a chord between them would let the inside of
// the turn show through, so the inner side runs through the corner.
#[test]
fn a_stroke_covers_exactly_its_reach() {
    let stix = Font::from_bytes(&std::fs::read(format!("{}/stix-two-math/STIX2Math.otf", crate::FONTS)).unwrap()).unwrap();
    let garamond = Font::from_bytes(&std::fs::read(format!("{}/eb-garamond/EBGaramond.ttf", crate::FONTS)).unwrap()).unwrap();
    for (font, gid, width) in [(&stix, 111u16, 200.0f32), (&stix, 20, 20.0), (&garamond, 53, 200.0)] {
        let mut glyph = Path::default();
        font.outline_glyph(gid, &mut glyph).expect("outline");
        let (checked, wrong) = matches_reach(&glyph, &style(width, Join::Round, Cap::Round), 0.25);
        assert_eq!(wrong, 0, "gid {gid} at width {width}: {wrong} of {checked} samples disagree with the distance to the outline");
    }
}

// A sliver narrower than the side probes leaves a gap in the boundary, so the pieces cannot be
// chained shut. The union comes back as its input rather than as the part that chained.
#[test]
fn a_union_that_cannot_close_returns_its_input() {
    let square = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    let sliver = vec![(50.0, 90.0), (50.0005, 90.0), (50.00025, 150.0)];
    let input = vec![square, sliver];
    assert_eq!(daegun::daecore::daetype::outline::simplify::union(&input), input);
}
