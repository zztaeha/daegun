use daegun::{Font, OutlinePen, Quad, QuadError, QuadraticPen, MAX_CURVES_PER_GLYPH};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

fn signed_area(curves: &[Quad]) -> f64 {
    let cross = |a: [f32; 2], b: [f32; 2]| f64::from(a[0]) * f64::from(b[1]) - f64::from(b[0]) * f64::from(a[1]);
    curves.iter().map(|c| 2.0 * (cross(c[0], c[1]) + cross(c[1], c[2])) + cross(c[0], c[2])).sum()
}

// A TrueType curve keeps its control point, read off the decoded path rather than through the same pen.
// Lines come back as quads with their control at the midpoint, so both sides leave those out.
#[test]
fn glyph_quads_keep_every_truetype_control_point() {
    let bent = |q: &Quad| (0..2).any(|i| (q[1][i] - (q[0][i] + q[2][i]) / 2.0).abs() > 1e-6);
    for rel in ["inter/InterVariable.ttf", "eb-garamond/EBGaramond.ttf"] {
        let f = font(rel);
        let upm = f32::from(f.upm());
        let mut curves = 0usize;
        for gid in 0..f.num_glyphs().min(300) {
            let mut path = daegun::Path::default();
            if f.outline_glyph_instanced(gid, &[], &mut path).is_none() {
                continue;
            }
            let (verbs, points) = path.parts();
            let (mut want, mut at, mut cur, mut start) = (Vec::new(), 0, (0.0, 0.0), (0.0, 0.0));
            for v in verbs {
                match v {
                    daegun::Verb::Move => (cur, start, at) = (points[at], points[at], at + 1),
                    daegun::Verb::Line => (cur, at) = (points[at], at + 1),
                    daegun::Verb::Quad => {
                        let (c, e) = (points[at], points[at + 1]);
                        want.push([[cur.0 / upm, cur.1 / upm], [c.0 / upm, c.1 / upm], [e.0 / upm, e.1 / upm]]);
                        (cur, at) = (e, at + 2);
                    }
                    daegun::Verb::Cubic => unreachable!("{rel} is TrueType"),
                    daegun::Verb::Close => cur = start,
                }
            }
            want.retain(bent);
            let Ok(quads) = f.glyph_quads(gid, &[]) else { continue };
            let got: Vec<Quad> = quads.into_iter().filter(bent).collect();
            assert_eq!(got.len(), want.len(), "{rel} gid {gid}: {} curves of {}", got.len(), want.len());
            for (g, w) in got.iter().zip(&want) {
                let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
                assert!(close(g[1], w[1]), "{rel} gid {gid}: control {:?} where the outline has {:?}", g[1], w[1]);
            }
            curves += want.len();
        }
        assert!(curves > 1000, "{rel}: only {curves} curves compared");
    }
}

// The outline at the caller's axes through `QuadraticPen`, built again from public parts. The winding is
// checked directly too, because both routes share `normalize_winding`.
#[test]
fn glyph_quads_are_the_outline_wound_clockwise() {
    let heavy: &[(&str, f64)] = &[("wght", 900.0)];
    for (rel, axes) in [
        ("eb-garamond/EBGaramond.ttf", &[][..]),
        ("stix-two-math/STIX2Math.otf", &[]),
        ("source-han-sans/SourceHanSansJP-VF.otf", &[]),
        ("inter/InterVariable.ttf", heavy),
    ] {
        let f = font(rel);
        let mut compared = 0usize;
        for gid in 0..f.num_glyphs().min(300) {
            let mut pen = QuadraticPen::new(f32::from(f.upm()));
            let want = match f.outline_glyph_instanced(gid, axes, &mut pen) {
                None => Err(QuadError::NoOutline),
                Some(()) => pen.finish().map(|mut q| {
                    daegun::normalize_winding(&mut q);
                    q
                }),
            };
            let got = f.glyph_quads(gid, axes);
            assert_eq!(got, want, "{rel} gid {gid}");
            if let Ok(q) = got {
                assert!(signed_area(&q) <= 0.0, "{rel} gid {gid} is wound counterclockwise");
                compared += 1;
            }
        }
        assert!(compared > 200, "{rel}: only {compared} glyphs compared");
    }

    let f = font("inter/InterVariable.ttf");
    let moved = (1..300u16)
        .filter(|&g| f.glyph_quads(g, &[("wght", 100.0)]).ok() != f.glyph_quads(g, heavy).ok())
        .count();
    assert!(moved > 100, "only {moved} glyphs changed between wght 100 and 900");
}

#[derive(Default)]
struct Cubics {
    at: (f32, f32),
    found: Vec<[(f32, f32); 4]>,
}

impl OutlinePen for Cubics {
    fn move_to(&mut self, x: f32, y: f32) {
        self.at = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.at = (x, y);
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.at = (x, y);
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.found.push([self.at, (ax, ay), (bx, by), (x, y)]);
        self.at = (x, y);
    }
    fn close(&mut self) {}
}

fn cubic_at(c: &[(f64, f64); 4], t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, d, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * c[0].0 + b * c[1].0 + d * c[2].0 + e * c[3].0,
        a * c[0].1 + b * c[1].1 + d * c[2].1 + e * c[3].1,
    )
}

fn quad_at(q: &Quad, t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let p = |i: usize| (f64::from(q[i][0]), f64::from(q[i][1]));
    let (a, b, c) = (p(0), p(1), p(2));
    (
        u * u * a.0 + 2.0 * u * t * b.0 + t * t * c.0,
        u * u * a.1 + 2.0 * u * t * b.1 + t * t * c.1,
    )
}

fn to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

fn to_polyline(p: (f64, f64), line: &[(f64, f64)]) -> f64 {
    line.windows(2).map(|w| to_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

// Both ways, so a chain that strays from the cubic is caught as well as one that falls short of it.
#[test]
fn quads_stay_within_tolerance_of_the_outline() {
    const BOUND: f64 = 2.0 / 4096.0;
    let f = font("stix-two-math/STIX2Math.otf");
    let upm = f64::from(f.upm());
    let (mut cubics, mut worst) = (0usize, 0.0f64);

    for gid in 0..f.num_glyphs().min(300) {
        let mut found = Cubics::default();
        if f.outline_glyph_instanced(gid, &[], &mut found).is_none() {
            continue;
        }
        for c in found.found {
            let mut pen = QuadraticPen::new(f.upm() as f32);
            pen.move_to(c[0].0, c[0].1);
            pen.curve_to(c[1].0, c[1].1, c[2].0, c[2].1, c[3].0, c[3].1);
            let mut chain = pen.finish().expect("one cubic always converts");
            if c[0] != c[3] {
                chain.pop();
            }

            let em = c.map(|(x, y)| (f64::from(x) / upm, f64::from(y) / upm));
            let exact: Vec<(f64, f64)> = (0..=256).map(|i| cubic_at(&em, f64::from(i) / 256.0)).collect();
            let approx: Vec<(f64, f64)> = chain
                .iter()
                .flat_map(|q| (0..=32).map(move |i| quad_at(q, f64::from(i) / 32.0)))
                .collect();
            for &p in &exact {
                worst = worst.max(to_polyline(p, &approx));
            }
            for &p in &approx {
                worst = worst.max(to_polyline(p, &exact));
            }
            cubics += 1;
        }
    }
    eprintln!("quads_stay_within_tolerance_of_the_outline: {cubics} cubics, worst {worst:.3e} em");
    assert!(cubics > 1000, "only {cubics} cubics checked");
    assert!(worst <= BOUND, "a quad chain strayed {worst:.3e} em from its cubic, past {BOUND:.3e}");
}

#[test]
fn quads_refuse_what_they_cannot_represent() {
    let mut pen = QuadraticPen::new(1000.0);
    pen.move_to(0.0, 0.0);
    pen.line_to(f32::NAN, 10.0);
    assert_eq!(pen.finish(), Err(QuadError::NonFinite), "a NaN coordinate was accepted");
    assert_eq!(QuadraticPen::new(0.0).finish(), Err(QuadError::BadUnitsPerEm), "zero units per em was accepted");
    let mut tiny = QuadraticPen::new(1e-39);
    tiny.move_to(0.0, 0.0);
    tiny.line_to(1.0, 0.0);
    assert_eq!(tiny.finish(), Err(QuadError::BadUnitsPerEm), "units per em whose reciprocal is infinite was accepted");
    assert_eq!(QuadraticPen::new(1000.0).finish(), Err(QuadError::NoOutline), "nothing drawn, yet curves");

    let lines = |n: usize| {
        let mut pen = QuadraticPen::new(1000.0);
        pen.move_to(0.0, 0.0);
        for i in 0..n {
            pen.line_to(i as f32 + 1.0, 0.0);
        }
        pen.finish().map(|q| q.len())
    };
    assert_eq!(lines(MAX_CURVES_PER_GLYPH - 1), Ok(MAX_CURVES_PER_GLYPH), "the limit itself was refused");
    assert_eq!(lines(MAX_CURVES_PER_GLYPH), Err(QuadError::TooComplex), "one past the limit was accepted");

    let f = font("inter/InterVariable.ttf");
    let space = f.glyph_id(' ' as u32).expect("Inter maps a space");
    assert_eq!(f.glyph_quads(space, &[]), Err(QuadError::NoOutline), "the space glyph has curves");
    assert_eq!(f.glyph_quads(f.num_glyphs(), &[]), Err(QuadError::NoOutline), "a gid past the end has curves");
}

// Finite input can still overflow on the way: a midpoint of two huge coordinates, a cubic's controls
// combined into its quads, or a winding area whose products leave a counterclockwise outline unturned.
#[test]
fn quads_stay_finite_and_wound_however_large_the_input() {
    let mut pen = QuadraticPen::new(1.0);
    pen.move_to(3e38, 0.0);
    pen.line_to(3e38, 3e38);
    assert_eq!(pen.finish(), Err(QuadError::NonFinite), "a midpoint past f32 was accepted");

    let mut pen = QuadraticPen::new(1.0);
    pen.move_to(0.0, 0.0);
    pen.curve_to(3e38, 1.0, 3e38, -1.0, 0.0, 0.0);
    assert_eq!(pen.finish(), Err(QuadError::NonFinite), "a cubic's control past f32 / 16 was accepted");

    // 6e37 is under f32::MAX / 4, yet eight of it, which a cubic's controls can sum to, are not.
    let mut pen = QuadraticPen::new(1.0);
    pen.move_to(-6e37, 0.0);
    pen.curve_to(6e37, 0.0, 6e37, 0.0, -6e37, 0.0);
    assert_eq!(pen.finish(), Err(QuadError::NonFinite), "a cubic summing past f32::MAX was accepted");

    let mut pen = QuadraticPen::new(1.0);
    pen.move_to(0.0, 0.0);
    pen.line_to(1e20, 0.0);
    pen.line_to(1e20, 1e20);
    pen.line_to(0.0, 1e20);
    let mut square = pen.finish().expect("a finite square");
    daegun::normalize_winding(&mut square);
    let area: f64 = square
        .iter()
        .map(|q| f64::from(q[0][0]) * f64::from(q[2][1]) - f64::from(q[2][0]) * f64::from(q[0][1]))
        .sum();
    assert!(area <= 0.0, "a counterclockwise square of side 1e20 was left counterclockwise");
}
