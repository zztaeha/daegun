use super::*;
use crate::daecore::daetype::outline::OutlinePen;

fn path(draw: impl FnOnce(&mut Path)) -> Path {
    let mut path = Path::default();
    draw(&mut path);
    path
}

// 10.9 rounds to 11, 100.5 to 101 and -0.5 to -1, halves away from 0.
#[test]
fn points_round_to_the_nearest_unit_and_a_curve_closing_on_its_start_drops_that_point() {
    let curved = path(|p| {
        p.move_to(10.9, -10.9);
        p.line_to(100.5, -0.5);
        p.quad_to(50.7, 80.2, 10.9, -10.9);
        p.close();
    });
    assert_eq!(contours(&curved), [vec![((11, -11), ON), ((101, -1), ON), ((51, 80), 0)]]);
    // A line back to the start is a point of its own, which the closing line then repeats.
    let lined = path(|p| {
        p.move_to(0.0, 0.0);
        p.line_to(10.0, 0.0);
        p.line_to(0.0, 0.0);
        p.close();
    });
    assert_eq!(contours(&lined), [vec![((0, 0), ON), ((10, 0), ON), ((0, 0), ON)]]);
}

#[test]
fn a_glyf_glyph_holds_the_points_and_their_box() {
    let triangle = path(|p| {
        p.move_to(0.0, 0.0);
        p.line_to(100.0, 0.0);
        p.line_to(50.0, 80.0);
        p.close();
    });
    let header = [0, 1, 0, 0, 0, 0, 0, 100, 0, 80, 0, 2, 0, 0];
    assert_eq!(glyf(&triangle, false), Some([&header[..], &[0x31, 0x33, 0x27, 100, 50, 80]].concat()));
    assert_eq!(glyf(&Path::default(), false), Some(Vec::new()));
}

#[test]
fn a_point_past_16_bits_or_a_cubic_in_glyf_is_not_held() {
    let wide = path(|p| {
        p.move_to(40_000.0, 0.0);
        p.line_to(0.0, 0.0);
        p.close();
    });
    assert_eq!((glyf(&wide, true), charstring(&wide, false)), (None, None));
    let cubic = path(|p| {
        p.move_to(0.0, 0.0);
        p.curve_to(10.0, 10.0, 20.0, 10.0, 30.0, 0.0);
        p.close();
    });
    assert!(glyf(&cubic, false).is_none() && glyf(&cubic, true).is_some());
    // Each point fits, the step between them doesn't.
    let far = path(|p| {
        p.move_to(-30_000.0, 0.0);
        p.line_to(30_000.0, 0.0);
    });
    assert_eq!(charstring(&far, false), None);
}

#[test]
fn a_charstring_steps_from_point_to_point_and_raises_a_quadratic() {
    let line = path(|p| {
        p.move_to(10.0, 20.0);
        p.line_to(30.0, 20.0);
        p.close();
    });
    assert_eq!(charstring(&line, true), Some(vec![149, 159, 21, 159, 139, 5, 14]));
    let quad = path(|p| {
        p.move_to(0.0, 0.0);
        p.quad_to(30.0, 30.0, 60.0, 0.0);
    });
    assert_eq!(charstring(&quad, false), Some(vec![139, 139, 21, 159, 159, 159, 139, 159, 119, 8]));
}

#[test]
fn a_box_measures_drawn_points_and_an_axis_it_does_not_span_at_0() {
    let flat = path(|p| {
        p.move_to(500.0, 500.0);
        p.close();
        p.move_to(0.0, 7.0);
        p.line_to(100.0, 7.0);
        p.close();
    });
    assert_eq!(measured(&flat), Some([0, 0, 100, 0]));
    assert_eq!(measured(&path(|p| p.move_to(5.0, 5.0))), None);
}
