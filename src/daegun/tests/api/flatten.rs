use daegun::{flatten, max_area_for, resolve_overlaps, Font, OutlinePen, Path, MAX_FLATTEN_POINTS, MAX_RESOLVE_EDGES};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

fn outline(f: &Font, gid: u16) -> Option<Path> {
    let mut p = Path::default();
    f.outline_glyph_instanced(gid, &[], &mut p)?;
    (!p.is_empty()).then_some(p)
}

type Contours = Vec<Vec<(f32, f32)>>;

fn winding(contours: &Contours, x: f32, y: f32) -> i32 {
    let mut w = 0;
    for c in contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.1 <= y) != (b.1 <= y) {
                let cross = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
                if cross > x {
                    w += if b.1 > a.1 { 1 } else { -1 };
                }
            }
        }
    }
    w
}

// Signed areas summed, so an overlap counts twice and a hole wound the other way subtracts.
fn area(contours: &Contours) -> f32 {
    let s: f32 = contours
        .iter()
        .flat_map(|c| (0..c.len()).map(move |i| {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            a.0 * b.1 - b.0 * a.1
        }))
        .sum();
    (s / 2.0).abs()
}

fn contours_of(f: &Font, ch: char) -> Contours {
    let gid = f.glyph_id(ch as u32).unwrap_or_else(|| panic!("the font maps {ch}"));
    flatten(&outline(f, gid).expect("an outline"), max_area_for(64.0, f32::from(f.upm()))).expect("within the cap")
}

// Inter's `4` is a diagonal and a stem sharing a flat top, and `A`'s crossbar runs into its diagonals:
// a coverage-summing rasterizer counts those overlaps twice.
#[test]
fn overlapping_glyphs_resolve_to_one_boundary() {
    let f = font("inter/InterVariable.ttf");
    for ch in ['4', 'A'] {
        let before = contours_of(&f, ch);
        let after = resolve_overlaps(&before).unwrap_or_else(|| panic!("{ch}: the overlap was left alone"));
        assert!(area(&after) < area(&before), "{ch}: the union is no smaller than the overlapping parts");

        let all: Vec<&(f32, f32)> = before.iter().flatten().collect();
        let (x0, x1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
        let (y0, y1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
        let (mut inside, mut twice) = (0usize, 0usize);
        for i in 0..64 {
            for j in 0..64 {
                let x = x0 + (x1 - x0) * (i as f32 + 0.37) / 64.0;
                let y = y0 + (y1 - y0) * (j as f32 + 0.61) / 64.0;
                let (b, a) = (winding(&before, x, y), winding(&after, x, y));
                assert_eq!(b != 0, a != 0, "{ch}: ({x}, {y}) changed sides");
                if b != 0 {
                    inside += 1;
                    twice += usize::from(b.abs() > 1);
                    assert_eq!(a.abs(), 1, "{ch}: ({x}, {y}) is still covered {a} times");
                }
            }
        }
        assert!(inside > 300 && twice > 0, "{ch}: the grid found {inside} inside and {twice} overlapping");
    }
}

// Single contours, and nested ones wound opposite so the counter is a hole: both are already a clean
// arrangement, and resolving them would only flatten detail out.
#[test]
fn glyphs_without_overlap_are_left_alone() {
    let f = font("inter/InterVariable.ttf");
    for ch in ['l', 'I', 'H', 'o', '8'] {
        assert!(resolve_overlaps(&contours_of(&f, ch)).is_none(), "{ch} was resolved");
    }
}

#[test]
fn resolving_declines_past_the_edge_limit() {
    let square = |x: f32, n: usize| -> Vec<(f32, f32)> {
        (0..n).map(|i| (x + i as f32 / n as f32 * 10.0, 0.0)).chain([(x + 10.0, 10.0), (x, 10.0)]).collect()
    };
    let small = vec![square(0.0, 2), square(5.0, 2)];
    assert!(resolve_overlaps(&small).is_some(), "two overlapping squares were not resolved");
    let half = MAX_RESOLVE_EDGES / 2;
    let at_limit = vec![square(0.0, half - 2), square(5.0, half - 2)];
    assert_eq!(at_limit.iter().map(Vec::len).sum::<usize>(), MAX_RESOLVE_EDGES);
    assert!(resolve_overlaps(&at_limit).is_some(), "an arrangement at the limit was declined");
    let big = vec![square(0.0, half), square(5.0, half)];
    assert!(big.iter().map(Vec::len).sum::<usize>() > MAX_RESOLVE_EDGES);
    assert!(resolve_overlaps(&big).is_none(), "an arrangement past the limit was attempted");
}

// Two 256-point zigzags, one across the other, cross about 66,000 times, which splits their 512 edges
// past the 131,072 pieces a resolve may hold, so the arrangement is declined rather than worked through.
#[test]
fn an_arrangement_that_splits_too_far_is_declined_quickly() {
    let zigzag = |across: bool| -> Vec<(f32, f32)> {
        (0..MAX_RESOLVE_EDGES / 2)
            .map(|i| {
                let (t, side) = (i as f32 * 1000.0 / 256.0, if i % 2 == 0 { 0.0 } else { 1000.0 });
                if across { (side, t) } else { (t, side) }
            })
            .collect()
    };
    let contours = vec![zigzag(false), zigzag(true)];
    let started = std::time::Instant::now();
    assert!(resolve_overlaps(&contours).is_none(), "66,000 crossings were resolved");
    assert!(started.elapsed().as_secs_f64() < 2.0, "declining took {:?}", started.elapsed());

    let mut path = Path::default();
    for c in &contours {
        path.move_to(c[0].0, c[0].1);
        for p in &c[1..] {
            path.line_to(p.0, p.1);
        }
        path.close();
    }
    let style = daegun::StrokeStyle::default();
    let (mut plain, mut simplified) = (Path::default(), Path::default());
    daegun::stroke(&path, &style, 0.25, &mut plain);
    let started = std::time::Instant::now();
    daegun::stroke_simplified(&path, &style, 0.25, &mut simplified);
    assert!(started.elapsed().as_secs_f64() < 2.0, "the simplified stroke took {:?}", started.elapsed());
    assert_eq!(flatten(&simplified, 1.0).expect("within the cap"), flatten(&plain, 1.0).expect("within the cap"), "a declined stroke is not the plain stroke");

    // Too many edges to test every pair, even with nothing crossing.
    let mut squares = Path::default();
    for i in 0..1500 {
        let (x, y) = ((i % 50) as f32 * 30.0, (i / 50) as f32 * 30.0);
        squares.move_to(x, y);
        squares.line_to(x + 10.0, y);
        squares.line_to(x + 10.0, y + 10.0);
        squares.line_to(x, y + 10.0);
        squares.close();
    }
    let (mut plain, mut simplified) = (Path::default(), Path::default());
    daegun::stroke(&squares, &style, 0.25, &mut plain);
    let edges: usize = flatten(&plain, 1e-6).expect("within the cap").iter().map(Vec::len).sum();
    assert!(edges > 16_384, "the squares stroke to only {edges} edges");
    let started = std::time::Instant::now();
    daegun::stroke_simplified(&squares, &style, 0.25, &mut simplified);
    assert!(started.elapsed().as_secs_f64() < 2.0, "{edges} edges took {:?}", started.elapsed());
    assert_eq!(flatten(&simplified, 1.0).expect("within the cap"), flatten(&plain, 1.0).expect("within the cap"), "a declined stroke is not the plain stroke");
}

fn has_overlap(contours: &Contours) -> bool {
    let all: Vec<&(f32, f32)> = contours.iter().flatten().collect();
    let (x0, x1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (y0, y1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    (0..32).any(|i| (0..32).any(|j| {
        let x = x0 + (x1 - x0) * (i as f32 + 0.37) / 32.0;
        let y = y0 + (y1 - y0) * (j as f32 + 0.61) / 32.0;
        winding(contours, x, y).abs() > 1
    }))
}

// Across four faces and three scripts. A union the resolver cannot verify is withheld, and the caller
// keeps the overlapping contours: four of Inter's, none of the others'.
#[test]
fn few_overlapping_glyphs_decline_to_resolve() {
    let faces = [
        ("Inter", "inter/InterVariable.ttf", 4),
        ("EB Garamond", "eb-garamond/EBGaramond.ttf", 0),
        ("Source Serif", "source-serif/SourceSerif4Variable-Roman.otf", 0),
        ("Devanagari", "noto-devanagari/NotoSansDevanagari.ttf", 0),
    ];
    for (name, rel, budget) in faces {
        let f = font(rel);
        let upm = f32::from(f.upm());
        let (mut overlapping, mut declined) = (0usize, 0usize);
        for gid in 1..f.num_glyphs().min(400) {
            let Some(path) = outline(&f, gid) else { continue };
            let contours = flatten(&path, max_area_for(34.0, upm)).expect("within the cap");
            if !has_overlap(&contours) {
                continue;
            }
            overlapping += 1;
            declined += usize::from(resolve_overlaps(&contours).is_none());
        }
        assert!(overlapping > 30, "{name}: only {overlapping} overlapping glyphs, too few to mean anything");
        assert!(declined <= budget, "{name}: {declined} of {overlapping} overlapping glyphs were not resolved");
    }
}

// Flatness is judged at a curve's midpoint, which an S-curve symmetric about its middle puts on its
// chord, and a loop on top of both its ends.
#[test]
fn a_curve_whose_midpoint_is_on_its_chord_still_bends() {
    let mut s = Path::default();
    s.move_to(0.0, 0.0);
    s.curve_to(0.0, 100.0, 100.0, -100.0, 100.0, 0.0);
    s.close();
    let reach = flatten(&s, 1.0).expect("within the cap").iter().flatten().fold(0.0f32, |m, p| m.max(p.1.abs()));
    assert!(reach > 28.0, "an S-curve reaching 28.9 from its chord flattened to {reach}");

    let mut looped = Path::default();
    looped.move_to(0.0, 0.0);
    looped.curve_to(100.0, 100.0, -100.0, 100.0, 0.0, 0.0);
    looped.close();
    assert!(area(&flatten(&looped, 1.0).expect("within the cap")) > 1000.0, "a closed loop flattened to nothing");
}

// The S and the loop above, each run on to twice its length, so that it is the first half of the
// curve. The whole curve's midpoint and quarters miss it; only a check at every level sees it.
#[test]
fn an_s_or_a_loop_inside_a_longer_curve_still_bends() {
    let flat = |c: [f32; 6]| {
        let mut p = Path::default();
        p.move_to(0.0, 0.0);
        p.curve_to(c[0], c[1], c[2], c[3], c[4], c[5]);
        p.close();
        flatten(&p, 1.0).expect("within the cap").concat()
    };
    let dip = flat([0.0, 200.0, 400.0, -800.0, -400.0, 1800.0]).iter().fold(0.0f32, |m, p| m.min(p.1));
    assert!(dip < -28.0, "an S dipping 28.9 below its chord flattened to {dip}");
    let rise = flat([200.0, 200.0, -800.0, 0.0, 1800.0, -600.0]).iter().fold(0.0f32, |m, p| m.max(p.1));
    assert!(rise > 70.0, "a loop rising 75 flattened to {rise}");
}

// A contour drawn after `close` without a `move_to` starts where the last one did, as `QuadraticPen`
// reads it. Left open, it still closes back to that start, which a closing `close` would hide.
#[test]
fn a_contour_continued_after_close_keeps_its_start() {
    let mut p = Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(100.0, 0.0);
    p.line_to(100.0, 100.0);
    p.close();
    p.line_to(0.0, 100.0);
    p.line_to(-50.0, 50.0);
    p.line_to(-50.0, 0.0);
    let contours = flatten(&p, 1.0).expect("within the cap");
    assert_eq!(contours.len(), 2, "{contours:?}");
    assert_eq!(contours[1], vec![(0.0, 0.0), (0.0, 100.0), (-50.0, 50.0), (-50.0, 0.0)]);
}

// At the finest tolerance one curve becomes 4,097 points, so curves enough cross the limit.
#[test]
fn flattening_declines_past_the_point_limit() {
    let wave = |n: usize| {
        let mut p = Path::default();
        p.move_to(0.0, 0.0);
        for i in 0..n {
            let x = i as f32 * 10.0;
            p.curve_to(x + 3.0, 50.0, x + 6.0, -50.0, x + 10.0, 0.0);
        }
        p.close();
        p
    };
    let one: usize = flatten(&wave(1), 0.0).expect("one curve").iter().map(Vec::len).sum();
    assert!(one > 4_000, "a curve at the finest tolerance gave only {one} points");
    let under = MAX_FLATTEN_POINTS / one;
    assert!(flatten(&wave(under), 0.0).is_some(), "{under} curves, under the limit, were refused");
    assert!(flatten(&wave(under + 2), 0.0).is_none(), "{} curves, past the limit, were flattened", under + 2);

    // The limit is on the polygons returned: each square below is read as five points, its closing one
    // included, and kept as four, so 209,716 of them hold 80% of it.
    let mut squares = Path::default();
    for i in 0..209_716 {
        let x = (i % 1000) as f32 * 3.0;
        let y = (i / 1000) as f32 * 3.0;
        squares.move_to(x, y);
        squares.line_to(x + 1.0, y);
        squares.line_to(x + 1.0, y + 1.0);
        squares.line_to(x, y + 1.0);
        squares.close();
    }
    let kept = flatten(&squares, 1.0).map(|c| c.iter().map(Vec::len).sum::<usize>());
    assert_eq!(kept, Some(838_864), "squares holding 80% of the limit were refused");
}

// A tiny px asks for the coarsest flattening, which an area that overflowed to infinity would read
// as the finest. And a point that is not finite has no place in a polygon.
#[test]
fn flattening_takes_the_extremes_as_asked() {
    let area = max_area_for(1e-38, 1000.0);
    assert!(area.is_finite(), "max_area_for a tiny px overflowed to {area}");
    for bad in [-0.0, -16.0, f32::NEG_INFINITY] {
        assert_eq!(max_area_for(bad, 1000.0), f32::MAX, "a px of {bad} did not ask for the coarsest");
    }
    let mut wave = Path::default();
    wave.move_to(0.0, 0.0);
    wave.curve_to(3.0, 50.0, 6.0, -50.0, 10.0, 0.0);
    wave.close();
    let coarse: usize = flatten(&wave, area).expect("one curve").iter().map(Vec::len).sum();
    assert!(coarse <= 3, "the coarsest flattening of one curve gave {coarse} points");

    for bad in [f32::NAN, f32::INFINITY] {
        let mut p = Path::default();
        p.move_to(0.0, 0.0);
        p.line_to(bad, 10.0);
        p.line_to(10.0, 10.0);
        p.close();
        assert!(flatten(&p, 1.0).is_none(), "a {bad} coordinate was flattened");
    }

    // A control point counts as much as an end point.
    for bad in [f32::NAN, f32::INFINITY, 1e38] {
        let mut curve = Path::default();
        curve.move_to(0.0, 0.0);
        curve.curve_to(bad, 5.0, 10.0, 10.0, 10.0, 0.0);
        curve.close();
        assert!(flatten(&curve, 1.0).is_none(), "a cubic's {bad} control was flattened");
        let mut quad = Path::default();
        quad.move_to(0.0, 0.0);
        quad.quad_to(5.0, bad, 10.0, 0.0);
        quad.close();
        assert!(flatten(&quad, 1.0).is_none(), "a quadratic's {bad} control was flattened");
    }

    // Finite, but a cubic this close to f32::MAX rounds past it at some of its points.
    let mut p = Path::default();
    p.move_to(f32::MAX, 0.0);
    p.curve_to(f32::MAX, 1e30, f32::MAX, -1e30, f32::MAX.next_down(), 0.0);
    p.close();
    assert!(flatten(&p, 1.0).is_none(), "a curve at f32::MAX was flattened");
}

// Scaled by a power of two, which is exact, curves must split exactly as before. At 2^60 their cross
// products pass what an f32 holds, and at 2^-48 the products bounding a cubic's quarter checks vanish.
#[test]
fn a_curve_splits_the_same_huge_or_tiny() {
    let points = |s: f32| {
        let mut p = Path::default();
        p.move_to(0.0, 0.0);
        p.quad_to(100.0 * s, 0.0, 100.0 * s, 100.0 * s);
        p.curve_to(0.0, 200.0 * s, 300.0 * s, 0.0, 0.0, 0.0);
        p.close();
        p.move_to(0.0, 0.0);
        p.curve_to(0.0, 100.0 * s, 100.0 * s, -100.0 * s, 100.0 * s, 0.0);
        p.close();
        flatten(&p, s * s).expect("finite").iter().map(Vec::len).sum::<usize>()
    };
    let one = points(1.0);
    assert!(one > 32, "the curves gave only {one} points");
    assert_eq!(points(2f32.powi(60)), one, "scaled by 2^60, the curves split differently");
    assert_eq!(points(2f32.powi(-48)), one, "scaled by 2^-48, the curves split differently");
}

// Two squares that overlap in a corner, where no edge's midpoint lies: the overlap is still found.
#[test]
fn an_overlap_away_from_every_edge_midpoint_is_resolved() {
    let square = |x: f32, y: f32| vec![(x, y), (x + 100.0, y), (x + 100.0, y + 100.0), (x, y + 100.0)];
    let resolved = daegun::resolve_overlaps(&[square(0.0, 0.0), square(60.0, 60.0)]).expect("the corner overlap was not found");
    assert_eq!(resolved.len(), 1, "two overlapping squares resolve to one boundary");
    assert_eq!(resolved[0].len(), 8, "the union of two squares overlapping in a corner has eight corners");
}
