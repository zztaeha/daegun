use daegun::{Cap, Font, Join, StrokeStyle};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

pub(crate) fn winding(contours: &[Vec<(f32, f32)>], x: f32, y: f32) -> i32 {
    let mut w = 0;
    for c in contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.1 <= y) != (b.1 <= y) && a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) > x {
                w += if b.1 > a.1 { 1 } else { -1 };
            }
        }
    }
    w
}

#[test]
fn a_stroke_outlines_the_glyph_instead_of_filling_it() {
    use daegun::OutlineOptions;
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('O' as u32).expect("Inter maps O");
    let draw = |o: &OutlineOptions| {
        let mut p = daegun::Path::default();
        f.prepared_outline(gid, 64.0, &[], o, &mut p).expect("O has an outline");
        p
    };
    let filled = draw(&OutlineOptions::default());
    let thick = draw(&OutlineOptions::default().with_stroke(StrokeStyle { width: 200.0, join: Join::Round, cap: Cap::Butt }));
    let (fx0, fy0, fx1, fy1) = filled.bounds().expect("ink");
    let (sx0, _, sx1, _) = thick.bounds().expect("ink");
    assert!(sx1 - sx0 > fx1 - fx0, "a stroke must reach outside the fill");

    let thin = draw(&OutlineOptions::default().with_stroke(StrokeStyle { width: 60.0, join: Join::Round, cap: Cap::Butt }));
    let rings = daegun::flatten(&thin, 0.01).expect("within the cap");
    let (cx, cy) = (((fx0 + fx1) / 2.0) as f32, ((fy0 + fy1) / 2.0) as f32);
    assert_eq!(winding(&rings, cx, cy), 0, "the center of a thin outlined O is covered");
    let on_ring = (fx0 as f32 + 0.5, cy);
    assert_ne!(winding(&rings, on_ring.0, on_ring.1), 0, "the ring itself is not covered");
}

fn zigzag(turns: usize) -> daegun::Path {
    use daegun::OutlinePen;
    let mut p = daegun::Path::default();
    p.move_to(0.0, 0.0);
    for i in 1..=turns {
        p.line_to(i as f32 * 0.5, if i % 2 == 1 { 1000.0 } else { 0.0 });
    }
    p
}

// Every turn of a wide, round-joined zigzag takes 256 points, so 20,000 of them would take five million.
// Past MAX_FLATTEN_POINTS nothing is drawn, not the part that fit.
#[test]
fn a_stroke_past_the_point_cap_draws_nothing() {
    let style = StrokeStyle { width: 200.0, join: Join::Round, cap: Cap::Butt };
    let points = |p: &daegun::Path| p.parts().1.len();
    let mut small = daegun::Path::default();
    daegun::stroke(&zigzag(100), &style, 0.001, &mut small);
    assert!(points(&small) > 100 * 250, "the turns took {} points, too few to reach the cap", points(&small));
    for draw in [daegun::stroke, daegun::stroke_simplified] {
        let mut huge = daegun::Path::default();
        draw(&zigzag(20_000), &style, 0.001, &mut huge);
        assert!(huge.is_empty(), "a stroke past the cap drew {} points", points(&huge));
    }
}

// `union` resolves with absolute tolerances, which suit font units: the same stroke, scaled past them,
// must still cover what the plain stroke covers. Inter's H at 16 and 32 times, and two squares.
#[test]
fn a_simplified_stroke_covers_the_stroke_at_any_scale() {
    use daegun::OutlinePen;
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("Inter maps H");
    let square = |side: f32| {
        let mut p = daegun::Path::default();
        p.move_to(0.0, 0.0);
        p.line_to(side, 0.0);
        p.line_to(side, side);
        p.line_to(0.0, side);
        p.close();
        p
    };
    let mut cases = Vec::new();
    for k in [16.0, 32.0] {
        let mut h = daegun::Path::default();
        f.outline_glyph(gid, &mut daegun::TransformPen::new(&mut h, [k, 0.0, 0.0, k, 0.0, 0.0])).expect("H");
        cases.push((h, StrokeStyle { width: 40.0 * k as f32, join: Join::Round, cap: Cap::Butt }));
    }
    for side in [50_000.0, 1_000_000.0, 1e20, 1e30, 1e35, 1e37] {
        cases.push((square(side), StrokeStyle { width: side / 10.0, join: Join::Miter { limit: 4.0 }, cap: Cap::Butt }));
    }
    for (path, style) in cases {
        let (mut plain, mut simple) = (daegun::Path::default(), daegun::Path::default());
        daegun::stroke(&path, &style, 0.0, &mut plain);
        daegun::stroke_simplified(&path, &style, 0.0, &mut simple);
        let (a, b) = (daegun::flatten(&plain, 1.0).expect("fits"), daegun::flatten(&simple, 1.0).expect("fits"));
        let (x0, y0, x1, y1) = plain.bounds().expect("ink");
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

// A miter's tip reaches r / cos(half the turn), which overflows f32 long before the width does.
#[test]
fn a_stroke_that_overflows_draws_nothing() {
    use daegun::OutlinePen;
    let mut p = daegun::Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(1000.0, 0.0);
    p.line_to(0.0, 1.0);
    for width in [1e33, f32::NAN] {
        let style = StrokeStyle { width, join: Join::Miter { limit: 1e30 }, cap: Cap::Butt };
        for draw in [daegun::stroke, daegun::stroke_simplified] {
            let mut out = daegun::Path::default();
            draw(&p, &style, 0.0, &mut out);
            assert!(out.parts().1.iter().all(|q| q.0.is_finite() && q.1.is_finite()), "width {width} drew a point past f32");
        }
    }
}

// A segment's length squared overflows f32 past about 1.8e19 units, which would leave every normal
// (0, 0) and the stroke tracing its own path with no width.
#[test]
fn a_huge_stroke_keeps_its_width() {
    use daegun::OutlinePen;
    let side = 1e20f32;
    let mut p = daegun::Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(side, 0.0);
    p.line_to(side, side);
    p.line_to(0.0, side);
    p.close();
    let style = StrokeStyle { width: side / 10.0, join: Join::Miter { limit: 4.0 }, cap: Cap::Butt };
    let mut out = daegun::Path::default();
    daegun::stroke(&p, &style, 0.0, &mut out);
    let rings = daegun::flatten(&out, side).expect("within the cap");
    assert_ne!(winding(&rings, -side / 40.0, side / 2.0), 0, "the outside half of the stroke has no ink");
    assert_ne!(winding(&rings, side / 40.0, side / 2.0), 0, "the inside half of the stroke has no ink");
}
