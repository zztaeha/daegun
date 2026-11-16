use daegun::{Bounds, ErrorKind, Font, Location};
use daegun::outline::{Join, Path, Point, StrokeStyle};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

pub(crate) fn winding(contours: &[Vec<Point>], x: f32, y: f32) -> i32 {
    let mut w = 0;
    for c in contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.y <= y) != (b.y <= y) && a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x) > x {
                w += if b.y > a.y { 1 } else { -1 };
            }
        }
    }
    w
}

#[test]
fn a_stroke_outlines_the_glyph_instead_of_filling_it() {
    use daegun::outline::OutlineOptions;
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('O').expect("Inter maps O");
    let draw = |o: &OutlineOptions| {
        let mut p = daegun::outline::Path::default();
        f.prepared_outline(gid, &Location::default(), 64.0, o, &mut p).expect("O has an outline");
        p
    };
    let filled = draw(&OutlineOptions::default());
    let thick =
        draw(&OutlineOptions::default().with_stroke(StrokeStyle::new(200.0).with_join(Join::Round)));
    let Bounds { x_min: fx0, y_min: fy0, x_max: fx1, y_max: fy1 } = filled.bounds().expect("ink");
    let Bounds { x_min: sx0, y_min: _, x_max: sx1, y_max: _ } = thick.bounds().expect("ink");
    assert!(sx1 - sx0 > fx1 - fx0, "a stroke must reach outside the fill");

    let thin =
        draw(&OutlineOptions::default().with_stroke(StrokeStyle::new(60.0).with_join(Join::Round)));
    let rings = thin.flatten(0.01).expect("within the cap");
    let (cx, cy) = (((fx0 + fx1) / 2.0) as f32, ((fy0 + fy1) / 2.0) as f32);
    assert_eq!(winding(&rings, cx, cy), 0, "the center of a thin outlined O is covered");
    let on_ring = (fx0 as f32 + 0.5, cy);
    assert_ne!(winding(&rings, on_ring.0, on_ring.1), 0, "the ring itself is not covered");
}

fn zigzag(turns: usize) -> daegun::outline::Path {
    use daegun::outline::OutlinePen;
    let mut p = daegun::outline::Path::default();
    p.move_to(0.0, 0.0);
    for i in 1..=turns {
        p.line_to(i as f32 * 0.5, if i % 2 == 1 { 1000.0 } else { 0.0 });
    }
    p
}

// Every turn of a wide, round-joined zigzag takes 256 points, so 20,000 of them take five million: the stroke draws
// whole, as it streams, while the simplified one, held whole to be resolved, is refused with nothing drawn.
#[test]
fn a_stroke_past_a_million_points_draws_and_a_simplified_one_is_refused() {
    let style = StrokeStyle::new(200.0).with_join(Join::Round);
    let points = |p: &daegun::outline::Path| p.parts().1.len();
    let mut whole = daegun::outline::Path::default();
    zigzag(20_000).stroke(&style, 0.001, &mut whole).expect("a stroke of any size draws");
    assert!(points(&whole) > daegun::outline::MAX_FLATTEN_POINTS, "the turns took {} points", points(&whole));
    let mut simplified = daegun::outline::Path::default();
    let kind = zigzag(20_000).stroke_simplified(&style, 0.001, &mut simplified).map_err(|e| e.kind());
    assert_eq!(kind, Err(ErrorKind::LimitExceeded));
    assert!(simplified.is_empty(), "a refused stroke drew {} points", points(&simplified));
}

// A curve that folds back on itself, out from (2, 0) and back to it, turns nearly back where it folds, which can round
// the cosine below -1, which read unclamped is a corner cut at no distance, putting a point hundreds of widths away.
// Every point stays within the stroke's reach of the curve, a miter's at most.
#[test]
fn a_stroke_turning_nearly_back_on_itself_stays_within_its_width() {
    use daegun::outline::OutlinePen;
    for (cx, cy) in [(0.0, 1.0), (1.0, 2.0)] {
        for join in [Join::Bevel, Join::Round, Join::Miter { limit: 4.0 }] {
            let mut p = daegun::outline::Path::default();
            p.move_to(2.0, 0.0);
            p.quad_to(cx, cy, 2.0, 0.0);
            let mut out = daegun::outline::Path::default();
            p.stroke(&StrokeStyle::new(0.5).with_join(join), 0.001, &mut out).expect("strokes");
            let within = |q: &Point| q.x >= -1.01 && q.x <= 3.01 && q.y >= -1.01 && q.y <= 3.01;
            assert!(out.parts().1.iter().all(within), "the fold round ({cx}, {cy}), {join:?}, drew past its reach");
        }
    }
}

// `union` resolves with absolute tolerances, which suit font units: the same stroke, scaled past them,
// must still cover what the plain stroke covers. Inter's H at 16 and 32 times, and two squares.
#[test]
fn a_simplified_stroke_covers_the_stroke_at_any_scale() {
    use daegun::outline::OutlinePen;
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H').expect("Inter maps H");
    let square = |side: f32| {
        let mut p = daegun::outline::Path::default();
        p.move_to(0.0, 0.0);
        p.line_to(side, 0.0);
        p.line_to(side, side);
        p.line_to(0.0, side);
        p.close();
        p
    };
    let mut cases = Vec::new();
    for k in [16.0, 32.0] {
        let mut h = daegun::outline::Path::default();
        f.outline(
            gid,
            &Location::default(),
            &mut daegun::outline::TransformPen::new(&mut h, [k, 0.0, 0.0, k, 0.0, 0.0]),
        )
        .expect("H");
        cases.push((h, StrokeStyle::new(40.0 * k as f32).with_join(Join::Round)));
    }
    for side in [50_000.0, 1_000_000.0, 1e20, 1e30, 1e35, 1e37] {
        cases
            .push((square(side), StrokeStyle::new(side / 10.0)));
    }
    for (path, style) in cases {
        let (mut plain, mut simple) = (daegun::outline::Path::default(), daegun::outline::Path::default());
        path.stroke(&style, 0.0, &mut plain).expect("strokes");
        path.stroke_simplified(&style, 0.0, &mut simple).expect("strokes");
        let (a, b) = (
            plain.flatten(1.0).expect("fits"),
            simple.flatten(1.0).expect("fits"),
        );
        let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } = plain.bounds().expect("ink");
        let (mut ink, mut differ) = (0, 0);
        for i in 0..200 {
            for j in 0..200 {
                let x = (x0 + (x1 - x0) * (f64::from(i) + 0.5) / 200.0) as f32;
                let y = (y0 + (y1 - y0) * (f64::from(j) + 0.5) / 200.0) as f32;
                let covered = winding(&a, x, y) != 0;
                ink += i32::from(covered);
                differ += i32::from(covered != (winding(&b, x, y) != 0));
            }
        }
        assert!(differ * 200 < ink, "a stroke {} wide: {differ} of {ink} samples differ once simplified", style.width);
    }
}

// A miter's tip reaches r / cos(half the turn), which overflows f32 long before the width does; a width not
// finite is refused before it is tried.
#[test]
fn a_stroke_that_overflows_draws_nothing() {
    use daegun::outline::OutlinePen;
    let mut p = daegun::outline::Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(1000.0, 0.0);
    p.line_to(0.0, 1.0);
    for (width, why) in [(1e33, ErrorKind::Overflow), (f32::NAN, ErrorKind::InvalidArgument)] {
        let style = StrokeStyle::new(width).with_join(Join::Miter { limit: 1e30 });
        for draw in [Path::stroke, Path::stroke_simplified] {
            let mut out = daegun::outline::Path::default();
            assert_eq!(draw(&p, &style, 0.0, &mut out).map_err(|e| e.kind()), Err(why), "width {width}");
            assert!(out.is_empty(), "width {width} drew");
        }
    }
}

// A NaN point inside a triangle and one starting it: each is refused as an infinite one is. Read as a repeat of
// the point before, a NaN would go unseen, the first taking its whole contour with it.
#[test]
fn a_path_with_a_point_not_finite_strokes_nothing() {
    use daegun::outline::OutlinePen;
    for bad in [(f32::NAN, f32::NAN), (f32::NAN, 0.0), (f32::INFINITY, 5.0)] {
        for first in [false, true] {
            let mut p = Path::default();
            let points = if first { [bad, (0.0, 0.0), (10.0, 10.0)] } else { [(0.0, 0.0), bad, (10.0, 10.0)] };
            p.move_to(points[0].0, points[0].1);
            points[1..].iter().for_each(|&(x, y)| p.line_to(x, y));
            p.close();
            for draw in [Path::stroke, Path::stroke_simplified] {
                let mut out = Path::default();
                let drawn = draw(&p, &StrokeStyle::new(2.0), 0.0, &mut out).map_err(|e| e.kind());
                assert_eq!(drawn, Err(ErrorKind::Overflow), "{bad:?}, first {first}");
                assert!(out.is_empty(), "{bad:?}, first {first} drew");
            }
        }
    }
}

// A segment's length squared overflows f32 past about 1.8e19 units, which would leave every normal
// (0, 0) and the stroke tracing its own path with no width.
#[test]
fn a_huge_stroke_keeps_its_width() {
    use daegun::outline::OutlinePen;
    let side = 1e20f32;
    let mut p = daegun::outline::Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(side, 0.0);
    p.line_to(side, side);
    p.line_to(0.0, side);
    p.close();
    let style = StrokeStyle::new(side / 10.0);
    let mut out = daegun::outline::Path::default();
    p.stroke(&style, 0.0, &mut out).expect("strokes");
    let rings = out.flatten(side).expect("within the cap");
    assert_ne!(winding(&rings, -side / 40.0, side / 2.0), 0, "the outside half of the stroke has no ink");
    assert_ne!(winding(&rings, side / 40.0, side / 2.0), 0, "the inside half of the stroke has no ink");
}

// A 60-degree corner miters at the default limit of 4, which a limit not above 0 or NaN stands for, and bevels
// at 1.
#[test]
fn a_miter_limit_not_above_zero_is_the_default() {
    use daegun::outline::OutlinePen;
    let mut corner = daegun::outline::Path::default();
    corner.move_to(0.0, 0.0);
    corner.line_to(100.0, 0.0);
    corner.line_to(50.0, 86.6);
    let reach = |limit: f32| {
        let style = StrokeStyle::new(10.0).with_join(Join::Miter { limit });
        let mut out = daegun::outline::Path::default();
        corner.stroke(&style, 0.25, &mut out).expect("strokes");
        out.bounds().expect("draws").x_max
    };
    let default = reach(4.0);
    for limit in [0.0, -1.0, f32::NAN] {
        assert_eq!(reach(limit), default, "a limit of {limit} did not miter as 4 does");
    }
    assert!(default > 108.0 && reach(1.0) < 105.0, "the miter reached {default} and the bevel {}", reach(1.0));
}
