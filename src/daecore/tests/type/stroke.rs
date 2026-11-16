use crate::daecore::daetype::outline::{Cap, Join, OutlinePen, Path, StrokeStyle};
use alloc::vec::Vec;

#[derive(Default)]
struct Recorder {
    contours: Vec<Vec<(f32, f32)>>,
}

impl OutlinePen for Recorder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(vec![(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(c) = self.contours.last_mut() {
            c.push((x, y));
        }
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.line_to(x, y);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.line_to(x, y);
    }
    fn close(&mut self) {}
}

impl Recorder {
    fn bounds(&self) -> (f32, f32, f32, f32) {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for c in &self.contours {
            for &(x, y) in c {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        (x0, y0, x1, y1)
    }
    fn signed_area(c: &[(f32, f32)]) -> f32 {
        let n = c.len();
        (0..n).map(|i| {
            let (x0, y0) = c[i];
            let (x1, y1) = c[(i + 1) % n];
            x0 * y1 - x1 * y0
        }).sum()
    }
}

fn line_path(a: (f32, f32), b: (f32, f32)) -> Path {
    let mut p = Path::default();
    p.move_to(a.0, a.1);
    p.line_to(b.0, b.1);
    p
}

fn square_path(s: f32) -> Path {
    let mut p = Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(s, 0.0);
    p.line_to(s, s);
    p.line_to(0.0, s);
    p.close();
    p
}

fn run(path: &Path, style: &StrokeStyle) -> Recorder {
    let mut r = Recorder::default();
    path.stroke(style, 0.05, &mut r).expect("strokes");
    r
}

#[test]
fn a_straight_line_strokes_to_a_rectangle_of_the_right_width() {
    let style = StrokeStyle { width: 2.0, join: Join::Bevel, cap: Cap::Butt };
    let r = run(&line_path((0.0, 0.0), (10.0, 0.0)), &style);
    assert_eq!(r.contours.len(), 1, "one open contour strokes to one closed loop");
    let (x0, y0, x1, y1) = r.bounds();
    assert!((y1 - y0 - 2.0).abs() < 1e-3, "height {} is not the stroke width", y1 - y0);
    assert!((x1 - x0 - 10.0).abs() < 1e-3, "a butt cap must not extend the length, got {}", x1 - x0);
}

#[test]
fn caps_extend_the_ends_by_half_the_width() {
    for (cap, name) in [(Cap::Square, "square"), (Cap::Round, "round")] {
        let style = StrokeStyle { width: 2.0, join: Join::Bevel, cap };
        let r = run(&line_path((0.0, 0.0), (10.0, 0.0)), &style);
        let (x0, _, x1, _) = r.bounds();
        assert!(
            (x1 - x0 - 12.0).abs() < 0.1,
            "{name} cap: length {} should be 10 plus half a width at each end",
            x1 - x0,
        );
    }
    let style = StrokeStyle { width: 2.0, join: Join::Bevel, cap: Cap::Butt };
    let r = run(&line_path((0.0, 0.0), (10.0, 0.0)), &style);
    let (x0, _, x1, _) = r.bounds();
    assert!((x1 - x0 - 10.0).abs() < 1e-3, "butt cap must not extend");
}

#[test]
fn a_closed_contour_strokes_to_two_opposed_loops() {
    let style = StrokeStyle { width: 2.0, join: Join::Miter { limit: 4.0 }, cap: Cap::Butt };
    let r = run(&square_path(10.0), &style);
    assert_eq!(r.contours.len(), 2, "a closed contour needs an outer and an inner loop");

    let a = Recorder::signed_area(&r.contours[0]);
    let b = Recorder::signed_area(&r.contours[1]);
    assert!(
        a.signum() != b.signum(),
        "the two loops wind the same way ({a} and {b}), so a non-zero fill would fill the middle",
    );
    assert!(a.abs() > b.abs() || b.abs() > a.abs(), "one loop must enclose the other");

    let (x0, y0, x1, y1) = r.bounds();
    assert!((x1 - x0 - 12.0).abs() < 0.1, "outer width {} should be 10 + 2r", x1 - x0);
    assert!((y1 - y0 - 12.0).abs() < 0.1, "outer height {} should be 10 + 2r", y1 - y0);
}

#[test]
fn the_miter_limit_replaces_a_spike_with_a_bevel() {
    let mut p = Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(10.0, 0.0);
    p.line_to(0.0, 0.5);

    let generous = run(&p, &StrokeStyle { width: 2.0, join: Join::Miter { limit: 100.0 }, cap: Cap::Butt });
    let limited = run(&p, &StrokeStyle { width: 2.0, join: Join::Miter { limit: 2.0 }, cap: Cap::Butt });
    let bevelled = run(&p, &StrokeStyle { width: 2.0, join: Join::Bevel, cap: Cap::Butt });

    let reach = |r: &Recorder| r.bounds().2;
    assert!(
        reach(&generous) > reach(&limited) + 1.0,
        "a generous limit must let the miter spike out further ({} vs {})",
        reach(&generous), reach(&limited),
    );
    assert!(
        (reach(&limited) - reach(&bevelled)).abs() < 1e-3,
        "past the limit the join must be exactly the bevel ({} vs {})",
        reach(&limited), reach(&bevelled),
    );
}

#[test]
fn a_round_join_curves_rather_than_spiking() {
    let mut p = Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(10.0, 0.0);
    p.line_to(10.0, 10.0);

    let round = run(&p, &StrokeStyle { width: 4.0, join: Join::Round, cap: Cap::Butt });
    let bevel = run(&p, &StrokeStyle { width: 4.0, join: Join::Bevel, cap: Cap::Butt });
    let n_round: usize = round.contours.iter().map(Vec::len).sum();
    let n_bevel: usize = bevel.contours.iter().map(Vec::len).sum();
    assert!(n_round > n_bevel, "a round join must add arc points ({n_round} vs {n_bevel})");

    let (_, _, x1, y1) = round.bounds();
    assert!(x1 <= 12.0 + 1e-3 && y1 <= 12.0 + 1e-3, "a round join must not reach past the miter point");
}

#[test]
fn a_tighter_tolerance_flattens_a_curve_more_finely() {
    let mut p = Path::default();
    p.move_to(0.0, 0.0);
    p.curve_to(0.0, 20.0, 20.0, 20.0, 20.0, 0.0);

    let count = |tol: f32| {
        let mut r = Recorder::default();
        p.stroke(&StrokeStyle { width: 1.0, join: Join::Bevel, cap: Cap::Butt }, tol, &mut r).expect("strokes");
        r.contours.iter().map(Vec::len).sum::<usize>()
    };
    assert!(count(0.01) > count(1.0), "tolerance must drive the segment count");
}

#[test]
fn degenerate_input_is_handled() {
    let style = StrokeStyle { width: 2.0, join: Join::Bevel, cap: Cap::Round };

    let mut p = Path::default();
    p.move_to(5.0, 5.0);
    assert!(!run(&p, &style).contours.is_empty(), "a round cap makes a lone point a dot");
    assert!(
        run(&p, &StrokeStyle { cap: Cap::Butt, ..style }).contours.is_empty(),
        "a butt cap gives a lone point no area",
    );

    assert!(run(&line_path((0.0, 0.0), (5.0, 0.0)), &StrokeStyle { width: 0.0, ..style }).contours.is_empty());
    assert!(run(&Path::default(), &style).contours.is_empty(), "an empty path strokes to nothing");
}

#[test]
fn the_inner_side_of_a_turn_has_no_loop() {
    let hairpin = |apex: f32| {
        let mut p = Path::default();
        p.move_to(0.0, 0.0);
        p.line_to(20.0, 0.0);
        p.line_to(0.0, apex);
        p
    };
    let cases: [(Path, f32, &str); 4] = [
        (hairpin(3.0), 2.0, "tightest hairpin, stroke narrower than the gap"),
        (hairpin(6.0), 2.0, "hairpin, stroke narrower than the gap"),
        (hairpin(10.0), 6.0, "hairpin, legs further apart than the stroke is wide"),
        (square_path(10.0), 2.0, "closed square"),
    ];
    for (path, width, label) in cases {
        let r = run(&path, &StrokeStyle { width, join: Join::Bevel, cap: Cap::Butt });
        for c in &r.contours {
            let n = c.len();
            let mut crossings = 0;
            for i in 0..n {
                for j in i + 2..n {
                    if i == 0 && j == n - 1 {
                        continue;
                    }
                    if segments_cross(c[i], c[(i + 1) % n], c[j], c[(j + 1) % n]) {
                        crossings += 1;
                    }
                }
            }
            assert_eq!(crossings, 0, "{label}: the stroke outline crosses itself {crossings} times");
        }
    }
}

fn segments_cross(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32), t: (f32, f32)| {
        (q.0 - p.0) * (t.1 - p.1) - (q.1 - p.1) * (t.0 - p.0)
    };
    let (d1, d2) = (side(a, b, c), side(a, b, d));
    let (d3, d4) = (side(c, d, a), side(c, d, b));
    d1 * d2 < -1e-6 && d3 * d4 < -1e-6
}

#[test]
fn simplifying_removes_the_global_overlap_a_plain_stroke_leaves() {
    let mut p = Path::default();
    p.move_to(0.0, 0.0);
    p.line_to(20.0, 0.0);
    p.line_to(0.0, 3.0);
    let style = StrokeStyle { width: 6.0, join: Join::Bevel, cap: Cap::Butt };

    let plain = run(&p, &style);
    let mut simple = Recorder::default();
    p.stroke_simplified(&style, 0.05, &mut simple).expect("strokes");

    let crossings = |r: &Recorder| {
        let mut n = 0;
        for c in &r.contours {
            let m = c.len();
            for i in 0..m {
                for j in i + 2..m {
                    if i == 0 && j == m - 1 {
                        continue;
                    }
                    if segments_cross(c[i], c[(i + 1) % m], c[j], c[(j + 1) % m]) {
                        n += 1;
                    }
                }
            }
        }
        n
    };

    assert!(crossings(&plain) > 0, "the fixture must actually exhibit the problem");
    assert_eq!(crossings(&simple), 0, "simplifying left {} crossings", crossings(&simple));
    assert!(!simple.contours.is_empty(), "simplifying produced no outline at all");

    let area = |r: &Recorder| r.contours.iter().map(|c| Recorder::signed_area(c).abs()).sum::<f32>() * 0.5;
    assert!(area(&simple) > 0.0, "the simplified outline encloses nothing");
    let (x0, y0, x1, y1) = simple.bounds();
    let (px0, py0, px1, py1) = plain.bounds();
    assert!(
        (x0 - px0).abs() < 0.5 && (y0 - py0).abs() < 0.5 && (x1 - px1).abs() < 0.5 && (y1 - py1).abs() < 0.5,
        "the outline moved: {:?} vs {:?}", (x0, y0, x1, y1), (px0, py0, px1, py1),
    );
}

#[test]
fn simplifying_a_non_overlapping_stroke_preserves_it() {
    let style = StrokeStyle { width: 2.0, join: Join::Bevel, cap: Cap::Butt };
    let mut simple = Recorder::default();
    let line = line_path((0.0, 0.0), (10.0, 0.0));
    line.stroke_simplified(&style, 0.05, &mut simple).expect("strokes");

    assert_eq!(simple.contours.len(), 1, "a plain line is one loop before and after");
    let (x0, y0, x1, y1) = simple.bounds();
    assert!((x1 - x0 - 10.0).abs() < 1e-2, "length changed: {}", x1 - x0);
    assert!((y1 - y0 - 2.0).abs() < 1e-2, "width changed: {}", y1 - y0);
}

// A sliver a few f32 steps wide still unites with what it crosses, its tip kept.
#[test]
fn a_sliver_unites_however_narrow() {
    let square = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    let sliver = vec![(50.0, 90.0), (50.0005, 90.0), (50.00025, 150.0)];
    let united = crate::daecore::daetype::outline::simplify::union(&[square, sliver]);
    assert_eq!(united.len(), 1, "{united:?}");
    let (top, at) = united[0].iter().fold((0.0f32, 0.0f32), |m, &(x, y)| if y > m.0 { (y, x) } else { m });
    assert_eq!(top, 150.0);
    assert!((at - 50.00025).abs() < 1e-4, "the tip moved to {at}");
}

// Edges crossing at under a millionth of a radian meet too close to call, so the union comes back as its input
// rather than as a guess.
#[test]
fn a_union_too_close_to_call_returns_its_input() {
    let square = vec![(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];
    let above = vec![(-500.0, 999.9995), (1500.0, 1000.0005), (1500.0, 1200.0), (-500.0, 1200.0)];
    let input = vec![square, above];
    assert_eq!(crate::daecore::daetype::outline::simplify::union(&input), input);
}
