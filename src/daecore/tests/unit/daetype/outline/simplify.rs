use alloc::vec::Vec;

use super::{remove_overlaps, Overlaps};
use crate::daecore::daetype::outline::{OutlinePen, Path};

// Curves cut into 256 chords, fine enough that a grid point between them sits on the side the curve puts it.
#[derive(Default)]
struct Chords {
    contours: Vec<Vec<(f64, f64)>>,
}

impl Chords {
    fn last(&self) -> (f64, f64) {
        *self.contours.last().and_then(|c| c.last()).unwrap_or(&(0.0, 0.0))
    }

    fn curve(&mut self, f: impl Fn(f64) -> (f64, f64)) {
        let contour = self.contours.last_mut().expect("a contour is open");
        contour.extend((1..=256).map(|i| f(f64::from(i) / 256.0)));
    }
}

impl OutlinePen for Chords {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(alloc::vec![(f64::from(x), f64::from(y))]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let contour = self.contours.last_mut().expect("a contour is open");
        contour.push((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (a, c, b) = (self.last(), (f64::from(cx), f64::from(cy)), (f64::from(x), f64::from(y)));
        self.curve(|t| {
            let u = 1.0 - t;
            (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1)
        });
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let (p, c, d) = (self.last(), (f64::from(ax), f64::from(ay)), (f64::from(bx), f64::from(by)));
        let e = (f64::from(x), f64::from(y));
        self.curve(|t| {
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (w0 * p.0 + w1 * c.0 + w2 * d.0 + w3 * e.0, w0 * p.1 + w1 * c.1 + w2 * d.1 + w3 * e.1)
        });
    }
    fn close(&mut self) {}
}

fn winding(path: &Path, x: f64, y: f64) -> i32 {
    let mut chords = Chords::default();
    path.replay(None, &mut chords);
    let mut w = 0;
    for c in &chords.contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.1 <= y) != (b.1 <= y) && a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) > x {
                w += if b.1 > a.1 { 1 } else { -1 };
            }
        }
    }
    w
}

// Every point of a grid filled after exactly where it was before, and wound once, left of its contours.
fn removed(before: &Path) -> Path {
    let Overlaps::Removed(after) = remove_overlaps(before) else { panic!("the overlaps were not removed") };
    let b = before.bounds().expect("the path has points");
    for i in 0..=96 {
        for j in 0..=96 {
            let x = b.x_min - 1.0 + (b.x_max - b.x_min + 2.0) * (f64::from(i) + 0.37) / 97.0;
            let y = b.y_min - 1.0 + (b.y_max - b.y_min + 2.0) * (f64::from(j) + 0.61) / 97.0;
            let (was, is) = (winding(before, x, y), winding(&after, x, y));
            assert_eq!(was != 0, is != 0, "({x}, {y}) was wound {was}, is {is}");
            assert!(is == 0 || is == 1, "({x}, {y}) is wound {is}");
        }
    }
    after
}

fn polygon(path: &mut Path, points: &[(f32, f32)]) {
    path.move_to(points[0].0, points[0].1);
    for &(x, y) in &points[1..] {
        path.line_to(x, y);
    }
    path.close();
}

fn contours(path: &Path) -> usize {
    path.segments().filter(|(verb, _)| *verb == crate::daecore::daetype::outline::Verb::Move).count()
}

#[test]
fn two_squares_overlapping_become_one() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (0.0, 60.0)]);
    polygon(&mut p, &[(30.0, 30.0), (90.0, 30.0), (90.0, 90.0), (30.0, 90.0)]);
    assert_eq!(contours(&removed(&p)), 1);
}

#[test]
fn clockwise_squares_overlapping_become_one() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (0.0, 60.0), (60.0, 60.0), (60.0, 0.0)]);
    polygon(&mut p, &[(30.0, 30.0), (30.0, 90.0), (90.0, 90.0), (90.0, 30.0)]);
    assert_eq!(contours(&removed(&p)), 1);
}

#[test]
fn a_square_inside_one_wound_alike_goes() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    polygon(&mut p, &[(20.0, 20.0), (80.0, 20.0), (80.0, 80.0), (20.0, 80.0)]);
    assert_eq!(contours(&removed(&p)), 1);
}

#[test]
fn squares_wound_against_each_other_turn_alike() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (0.0, 60.0)]);
    polygon(&mut p, &[(30.0, 30.0), (30.0, 90.0), (90.0, 90.0), (90.0, 30.0)]);
    removed(&p);
}

#[test]
fn a_star_crossing_itself_keeps_its_rim() {
    let mut p = Path::new();
    polygon(&mut p, &[(50.0, 1.0), (62.0, 97.0), (3.0, 35.0), (98.0, 34.0), (36.0, 99.0)]);
    removed(&p);
}

#[test]
fn curves_crossing_split_where_they_meet() {
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.quad_to(50.0, 120.0, 100.0, 0.0);
    p.close();
    p.move_to(0.0, 50.0);
    p.curve_to(30.0, -40.0, 70.0, -40.0, 100.0, 50.0);
    p.close();
    removed(&p);
}

#[test]
fn a_stem_and_an_arch_join_as_a_font_draws_them() {
    // Exo 2's n, its arch tucked into the stem.
    let mut p = Path::new();
    p.move_to(374.0, 499.0);
    p.quad_to(506.0, 499.0, 506.0, 365.0);
    p.line_to(506.0, 0.0);
    p.line_to(421.0, 0.0);
    p.line_to(421.0, 341.0);
    p.quad_to(421.0, 390.0, 405.0, 408.5);
    p.quad_to(389.0, 427.0, 351.0, 427.0);
    p.quad_to(306.0, 427.0, 261.0, 405.0);
    p.quad_to(216.0, 383.0, 154.0, 345.0);
    p.line_to(150.0, 406.0);
    p.quad_to(207.0, 450.0, 263.0, 474.5);
    p.quad_to(319.0, 499.0, 374.0, 499.0);
    p.close();
    polygon(&mut p, &[(150.0, 487.0), (157.0, 394.0), (163.0, 379.0), (163.0, 0.0), (78.0, 0.0), (78.0, 487.0)]);
    assert_eq!(contours(&removed(&p)), 1);
}

#[test]
fn contours_apart_or_holed_are_left_alone() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]);
    polygon(&mut p, &[(60.0, 0.0), (100.0, 0.0), (100.0, 40.0), (60.0, 40.0)]);
    polygon(&mut p, &[(70.0, 10.0), (70.0, 30.0), (90.0, 30.0), (90.0, 10.0)]);
    assert!(matches!(remove_overlaps(&p), Overlaps::None));
    assert!(matches!(remove_overlaps(&Path::new()), Overlaps::None));
}

// Curves touching at their turns without crossing stay two contours, turned alike.
#[test]
fn curves_touching_where_they_turn_stay_apart() {
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.quad_to(50.0, 100.0, 100.0, 0.0);
    p.close();
    p.move_to(0.0, 100.0);
    p.quad_to(50.0, 0.0, 100.0, 100.0);
    p.close();
    assert_eq!(contours(&removed(&p)), 2);
}

// A line along an arch, touching it at (25, 37.5) without crossing, can't be told from one crossing it twice.
#[test]
fn a_curve_touching_a_line_at_a_tangent_is_too_close_to_call() {
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.quad_to(50.0, 100.0, 100.0, 0.0);
    p.close();
    polygon(&mut p, &[(-20.0, -7.5), (60.0, 72.5), (-20.0, 72.5)]);
    assert!(matches!(remove_overlaps(&p), Overlaps::Unsure));
}

// Contours sharing a stretch of an edge, back to back, and a third over both: the shared stretch bounds nothing.
#[test]
fn a_shared_stretch_bounds_nothing() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)]);
    polygon(&mut p, &[(50.0, 10.0), (90.0, 10.0), (90.0, 40.0), (50.0, 40.0)]);
    polygon(&mut p, &[(20.0, 20.0), (60.0, 20.0), (60.0, 30.0), (20.0, 30.0)]);
    assert_eq!(contours(&removed(&p)), 1);
}

// Two curves leaving a cusp in one direction, the right one met first, wound once both lie on the line: Mishafi's
// glyph 638, with a box over the cusp to remove.
#[test]
fn curves_leaving_a_cusp_alike_are_wound_as_they_lie() {
    let mut p = Path::new();
    p.move_to(472.0, 738.0);
    p.quad_to(522.0, 706.0, 522.0, 682.0);
    p.quad_to(522.0, 674.0, 517.0, 674.0);
    p.quad_to(520.0, 674.0, 490.0, 685.0);
    p.line_to(420.0, 696.0);
    p.close();
    polygon(&mut p, &[(500.0, 660.0), (500.0, 700.0), (540.0, 700.0), (540.0, 660.0)]);
    removed(&p);
}

// A cubic whose x stops turning just as it ends, at a cusp a line leaves along: no turn splits off a sliver there.
#[test]
fn a_curve_only_touching_a_turn_at_its_end_is_one_stretch() {
    let mut p = Path::new();
    p.move_to(391.0, 511.0);
    p.curve_to(390.0, 510.0, 390.0, 510.0, 390.0, 509.0);
    polygon_on(&mut p, &[(390.0, 746.0), (104.0, 746.0), (104.0, 300.0), (500.0, 300.0)]);
    assert!(matches!(remove_overlaps(&p), Overlaps::None));
}

// Curves folded back along one line are that line out and back, cut where the flats beside them end.
#[test]
fn curves_folded_flat_are_their_lines() {
    let mut p = Path::new();
    p.move_to(100.0, 500.0);
    p.line_to(200.0, 500.0);
    p.line_to(200.0, 570.0);
    p.line_to(154.0, 570.0);
    p.quad_to(150.0, 570.0, 150.5, 570.0);
    p.quad_to(151.0, 570.0, 148.5, 570.0);
    polygon_on(&mut p, &[(100.0, 570.0)]);
    let after = removed(&p);
    let b = after.bounds().expect("points");
    assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (100.0, 500.0, 200.0, 570.0));
}

// A spike out along a curve and back along the same one bounds nothing, however long the two run together.
#[test]
fn a_curve_run_back_over_itself_bounds_nothing() {
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.line_to(100.0, 0.0);
    p.line_to(100.0, 100.0);
    p.quad_to(130.0, 120.0, 150.0, 160.0);
    p.quad_to(130.0, 120.0, 100.0, 100.0);
    p.line_to(0.0, 100.0);
    p.close();
    let after = removed(&p);
    let b = after.bounds().expect("points");
    assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (0.0, 0.0, 100.0, 100.0));
}

// A curve leaving a line's middle along it, then bending away, lies on the side it bends to.
#[test]
fn a_curve_leaving_a_line_along_it_lies_where_it_bends() {
    let mut p = Path::new();
    p.move_to(520.0, -1.0);
    p.quad_to(523.0, 0.0, 521.5, -1.0);
    p.quad_to(520.0, -2.0, 522.0, -1.0);
    polygon_on(&mut p, &[(522.0, 100.0), (500.0, 100.0)]);
    assert!(!matches!(remove_overlaps(&p), Overlaps::Unsure));
}

// Lines on from the current point, then closed.
fn polygon_on(path: &mut Path, points: &[(f32, f32)]) {
    for &(x, y) in points {
        path.line_to(x, y);
    }
    path.close();
}
