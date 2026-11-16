use super::*;

// x runs 0, 50, 150, 100: one control between the ends and one past them, so the curve bulges
// to about 113.7 before it comes back to 100. y stays between its ends and adds nothing.
#[test]
fn a_curve_with_one_control_past_its_ends_is_bounded_by_its_bulge() {
    let mut path = Path::default();
    path.move_to(0.0, 0.0);
    path.curve_to(50.0, 10.0, 150.0, 20.0, 100.0, 30.0);
    let b = path.bounds().expect("a box");
    assert_eq!((b.x_min, b.y_min, b.y_max), (0.0, 0.0, 30.0));
    assert!((113.0..114.0).contains(&b.x_max), "the bulge reached {}", b.x_max);
}
