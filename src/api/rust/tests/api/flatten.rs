use daegun::{ErrorKind, Font, GlyphId, Location};
use daegun::outline::{max_area_for, resolve_overlaps, OutlinePen, Path, Point, MAX_FLATTEN_POINTS};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

fn outline(f: &Font, gid: GlyphId) -> Option<Path> {
    let mut p = Path::default();
    f.outline(gid, &Location::default(), &mut p).ok()?;
    (!p.is_empty()).then_some(p)
}

type Contours = Vec<Vec<Point>>;

fn winding(contours: &Contours, x: f32, y: f32) -> i32 {
    let mut w = 0;
    for c in contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.y <= y) != (b.y <= y) {
                let cross = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                if cross > x {
                    w += if b.y > a.y { 1 } else { -1 };
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
            a.x * b.y - b.x * a.y
        }))
        .sum();
    (s / 2.0).abs()
}

// Why flatten refused a path, if it did.
fn refused(p: &Path) -> Option<ErrorKind> {
    p.flatten(1.0).err().map(|e| e.kind())
}

fn contours_of(f: &Font, ch: char) -> Contours {
    let gid = f.glyph_id(ch).unwrap_or_else(|| panic!("the font maps {ch}"));
    (outline(f, gid).expect("an outline")).flatten(max_area_for(64.0, f32::from(f.units_per_em())))
        .expect("within the cap")
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

        let all: Vec<&Point> = before.iter().flatten().collect();
        let (x0, x1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
        let (y0, y1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
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

// Every point of a grid over the contours on the same side after as before, and wound once where filled.
fn resolves_alike(before: &Contours, after: &Contours, grid: usize) {
    let all: Vec<&Point> = before.iter().flatten().collect();
    let (x0, x1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    let (y0, y1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    for i in 0..grid {
        for j in 0..grid {
            let x = x0 + (x1 - x0) * (i as f32 + 0.37) / grid as f32;
            let y = y0 + (y1 - y0) * (j as f32 + 0.61) / grid as f32;
            let (b, a) = (winding(before, x, y), winding(after, x, y));
            assert_eq!(b != 0, a != 0, "({x}, {y}) changed sides");
            assert!(a == 0 || a.abs() == 1, "({x}, {y}) is covered {a} times");
        }
    }
}

// Two squares of 5,000 points each resolve to one.
#[test]
fn resolving_takes_any_number_of_points() {
    let square = |x: f32, n: usize| -> Vec<Point> {
        let corners = (0..n).map(|i| (x + i as f32 / n as f32 * 10.0, 0.0)).chain([(x + 10.0, 10.0), (x, 10.0)]);
        corners.map(Point::from).collect()
    };
    let big = vec![square(0.0, 5_000), square(5.0, 5_000)];
    let resolved = resolve_overlaps(&big).expect("two overlapping squares of 5,000 points were declined");
    assert_eq!(resolved.len(), 1);
    resolves_alike(&big, &resolved, 32);
}

// A comb of 2,000 teeth under a bar across them all: 4,000 flat edges at two heights, each tested only with what
// touches it, and the bar's own two with the 4,000 edges they cross or end on.
#[test]
fn a_comb_of_many_flat_edges_resolves() {
    let mut comb = vec![Point::from((-1.0, -10.0))];
    for i in 0..2_000 {
        let x = 2.0 * i as f32;
        comb.extend([(x, 0.0), (x, 10.0), (x + 1.0, 10.0), (x + 1.0, 0.0)].map(Point::from));
    }
    comb.push(Point::from((4_000.0, -10.0)));
    comb.reverse();
    let bar = [(-2.0, 5.0), (4_001.0, 5.0), (4_001.0, 15.0), (-2.0, 15.0)].map(Point::from).to_vec();
    let contours = vec![comb, bar];
    let resolved = resolve_overlaps(&contours).expect("the comb under its bar was declined");
    for i in [0, 999, 1_999] {
        let x = 2.0 * i as f32;
        let at = |dx: f32, y: f32| winding(&resolved, x + dx, y).abs();
        let around = [at(0.5, 2.0), at(1.5, 2.0), at(0.5, 7.0), at(1.5, 7.0), at(1.5, 12.0)];
        assert_eq!(around, [1, 0, 1, 1, 1], "tooth {i}");
    }
    resolves_alike(&contours, &resolved, 16);
}

// Two 256-point zigzags, one across the other, cross about 66,000 times, 128 times an edge: far more often than a
// glyph's curves meet, so they're declined once the work their edges allow is spent, and so is their stroke's union.
#[test]
fn an_arrangement_crossing_more_than_its_edges_allow_is_declined_quickly() {
    let zigzag = |across: bool| -> Vec<Point> {
        (0..256)
            .map(|i| {
                let (t, side) = (i as f32 * 1000.0 / 256.0, if i % 2 == 0 { 0.0 } else { 1000.0 });
                if across { Point { x: side, y: t } } else { Point { x: t, y: side } }
            })
            .collect()
    };
    let contours = vec![zigzag(false), zigzag(true)];
    let started = std::time::Instant::now();
    assert!(resolve_overlaps(&contours).is_none(), "66,000 crossings were resolved");
    assert!(started.elapsed().as_secs_f64() < 2.0, "declining took {:?}", started.elapsed());

    let mut path = Path::default();
    for c in &contours {
        path.move_to(c[0].x, c[0].y);
        for p in &c[1..] {
            path.line_to(p.x, p.y);
        }
        path.close();
    }
    let style = daegun::outline::StrokeStyle::default();
    let (mut plain, mut simplified) = (Path::default(), Path::default());
    path.stroke(&style, 0.25, &mut plain).expect("strokes");
    let started = std::time::Instant::now();
    path.stroke_simplified(&style, 0.25, &mut simplified).expect("strokes");
    assert!(started.elapsed().as_secs_f64() < 2.0, "the simplified stroke took {:?}", started.elapsed());
    assert_eq!(simplified.flatten(1.0).expect("flattens"), plain.flatten(1.0).expect("flattens"));
}

// One edge drawn back and forth 4,000 times under a triangle: every two of its edges lie along one another, so it's
// the pairs met, not the meetings, that spend the outline's work, as quickly.
#[test]
fn an_edge_drawn_over_itself_more_than_its_edges_allow_is_declined_quickly() {
    let edge: Vec<Point> = (0..4000).map(|i| Point { x: (i % 2) as f32 * 1000.0, y: 0.0 }).collect();
    let triangle = vec![Point { x: 500.0, y: -500.0 }, Point { x: 2000.0, y: -500.0 }, Point { x: 500.0, y: 500.0 }];
    let started = std::time::Instant::now();
    assert!(resolve_overlaps(&[edge, triangle]).is_none(), "the edge drawn over itself was resolved");
    assert!(started.elapsed().as_secs_f64() < 2.0, "declining took {:?}", started.elapsed());
}

// The stroke of 1,500 squares, each overlapping its neighbors, over 16,384 edges, resolves to its union.
#[test]
fn the_stroke_of_1_500_overlapping_squares_resolves() {
    let mut squares = Path::default();
    for i in 0..1500 {
        let (x, y) = ((i % 50) as f32 * 8.0, (i / 50) as f32 * 8.0);
        squares.move_to(x, y);
        squares.line_to(x + 10.0, y);
        squares.line_to(x + 10.0, y + 10.0);
        squares.line_to(x, y + 10.0);
        squares.close();
    }
    let style = daegun::outline::StrokeStyle::default();
    let (mut plain, mut simplified) = (Path::default(), Path::default());
    squares.stroke(&style, 0.25, &mut plain).expect("strokes");
    squares.stroke_simplified(&style, 0.25, &mut simplified).expect("strokes");
    let (plain, simplified) = (plain.flatten(1e-3).expect("flattens"), simplified.flatten(1e-3).expect("flattens"));
    assert_ne!(simplified, plain, "the stroke's union was declined");
    resolves_alike(&plain, &simplified, 16);
}

fn has_overlap(contours: &Contours) -> bool {
    let all: Vec<&Point> = contours.iter().flatten().collect();
    let (x0, x1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    let (y0, y1) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
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
        let upm = f32::from(f.units_per_em());
        let (mut overlapping, mut declined) = (0usize, 0usize);
        for gid in 1..f.glyph_count().min(400) {
            let Some(path) = outline(&f, GlyphId(gid)) else { continue };
            let contours = path.flatten(max_area_for(34.0, upm)).expect("within the cap");
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
    let reach = s.flatten(1.0).expect("within the cap").iter().flatten().fold(0.0f32, |m, p| m.max(p.y.abs()));
    assert!(reach > 28.0, "an S-curve reaching 28.9 from its chord flattened to {reach}");

    let mut looped = Path::default();
    looped.move_to(0.0, 0.0);
    looped.curve_to(100.0, 100.0, -100.0, 100.0, 0.0, 0.0);
    looped.close();
    assert!(area(&looped.flatten(1.0).expect("within the cap")) > 1000.0, "a closed loop flattened to nothing");
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
        p.flatten(1.0).expect("within the cap").concat()
    };
    let dip = flat([0.0, 200.0, 400.0, -800.0, -400.0, 1800.0]).iter().fold(0.0f32, |m, p| m.min(p.y));
    assert!(dip < -28.0, "an S dipping 28.9 below its chord flattened to {dip}");
    let rise = flat([200.0, 200.0, -800.0, 0.0, 1800.0, -600.0]).iter().fold(0.0f32, |m, p| m.max(p.y));
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
    let contours = p.flatten(1.0).expect("within the cap");
    assert_eq!(contours.len(), 2, "{contours:?}");
    assert_eq!(contours[1], [(0.0, 0.0), (0.0, 100.0), (-50.0, 50.0), (-50.0, 0.0)].map(Point::from));
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
    let one: usize = (wave(1)).flatten(0.0).expect("one curve").iter().map(Vec::len).sum();
    assert!(one > 4_000, "a curve at the finest tolerance gave only {one} points");
    let under = MAX_FLATTEN_POINTS / one;
    assert!((wave(under)).flatten(0.0).is_ok(), "{under} curves, under the limit, were refused");
    let past = (wave(under + 2)).flatten(0.0).map_err(|e| e.kind()).err();
    assert_eq!(past, Some(ErrorKind::LimitExceeded), "{} curves, past the limit, were flattened", under + 2);

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
    let kept = squares.flatten(1.0).map(|c| c.iter().map(Vec::len).sum::<usize>());
    assert_eq!(kept.ok(), Some(838_864), "squares holding 80% of the limit were refused");
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
    let coarse: usize = wave.flatten(area).expect("one curve").iter().map(Vec::len).sum();
    assert!(coarse <= 3, "the coarsest flattening of one curve gave {coarse} points");

    for bad in [f32::NAN, f32::INFINITY] {
        let mut p = Path::default();
        p.move_to(0.0, 0.0);
        p.line_to(bad, 10.0);
        p.line_to(10.0, 10.0);
        p.close();
        assert_eq!(refused(&p), Some(ErrorKind::InvalidArgument), "a {bad} coordinate");
    }

    // A control point counts as much as an end point.
    for bad in [f32::NAN, f32::INFINITY, 1e38] {
        let mut curve = Path::default();
        curve.move_to(0.0, 0.0);
        curve.curve_to(bad, 5.0, 10.0, 10.0, 10.0, 0.0);
        curve.close();
        assert_eq!(refused(&curve), Some(ErrorKind::InvalidArgument), "a cubic's {bad} control");
        let mut quad = Path::default();
        quad.move_to(0.0, 0.0);
        quad.quad_to(5.0, bad, 10.0, 0.0);
        quad.close();
        assert_eq!(refused(&quad), Some(ErrorKind::InvalidArgument), "a quadratic's {bad} control");
    }

    // Finite, but a cubic this close to f32::MAX rounds past it at some of its points.
    let mut p = Path::default();
    p.move_to(f32::MAX, 0.0);
    p.curve_to(f32::MAX, 1e30, f32::MAX, -1e30, f32::MAX.next_down(), 0.0);
    p.close();
    assert_eq!(refused(&p), Some(ErrorKind::InvalidArgument), "a curve at f32::MAX");
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
        p.flatten(s * s).expect("finite").iter().map(Vec::len).sum::<usize>()
    };
    let one = points(1.0);
    assert!(one > 32, "the curves gave only {one} points");
    assert_eq!(points(2f32.powi(60)), one, "scaled by 2^60, the curves split differently");
    assert_eq!(points(2f32.powi(-48)), one, "scaled by 2^-48, the curves split differently");
}

// Two squares that overlap in a corner, where no edge's midpoint lies: the overlap is still found.
#[test]
fn an_overlap_away_from_every_edge_midpoint_is_resolved() {
    let square = |x: f32, y: f32| {
        [(x, y), (x + 100.0, y), (x + 100.0, y + 100.0), (x, y + 100.0)].map(Point::from).to_vec()
    };
    let resolved = daegun::outline::resolve_overlaps(&[square(0.0, 0.0), square(60.0, 60.0)])
        .expect("the corner overlap was not found");
    assert_eq!(resolved.len(), 1, "two overlapping squares resolve to one boundary");
    assert_eq!(resolved[0].len(), 8, "the union of two squares overlapping in a corner has eight corners");
}

// Lists kept between calls flatten each glyph as a fresh call does, whatever they held, as glyphs gain and lose
// contours; an error leaves them empty.
#[test]
fn flattening_into_kept_lists_matches_a_fresh_call() {
    let f = font("inter/InterVariable.ttf");
    let max_area = max_area_for(16.0, f32::from(f.units_per_em()));
    let mut kept: Contours = vec![vec![Point { x: 1.0, y: 2.0 }; 5]; 2];
    for ch in ['B', '.', '%', 'i', 'g', 'o', 'B'] {
        let p = outline(&f, f.glyph_id(ch).expect("the font maps it")).expect("an outline");
        p.flatten_into(max_area, &mut kept).expect("within the cap");
        assert_eq!(kept, p.flatten(max_area).expect("within the cap"), "{ch}");
    }
    let mut bad = Path::default();
    bad.move_to(0.0, 0.0);
    bad.line_to(f32::NAN, 1.0);
    bad.line_to(2.0, 2.0);
    bad.close();
    assert_eq!(bad.flatten_into(1.0, &mut kept).err().map(|e| e.kind()), Some(ErrorKind::InvalidArgument));
    assert!(kept.is_empty(), "an error left {} contours", kept.len());
}
