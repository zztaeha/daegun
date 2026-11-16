use super::*;

// From the top at 706, the walk goes on past a point 6 units down but 300 across, a shallow
// angle, and the run ends on that control point: round. Stopping at the drop would make it flat.
#[test]
fn a_shallow_drop_stays_in_the_level_run() {
    let pts = AutoPoints {
        x: alloc::vec![0.0, 0.0, 100.0, 400.0, 400.0],
        y: alloc::vec![0.0, 700.0, 706.0, 700.0, 0.0],
        flags: alloc::vec![ON_CURVE, ON_CURVE, ON_CURVE, 0, ON_CURVE],
        contour_ends: alloc::vec![4],
    };
    assert_eq!(extremum(&pts, true, 71.0), Some((706.0, true)));
}
