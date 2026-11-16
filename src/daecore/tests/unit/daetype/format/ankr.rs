use super::*;
use alloc::vec;

#[test]
fn a_point_count_is_the_points_that_exist() {
    let mut t = vec![0, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 20];
    t.extend([0, 8, 0, 0, 0, 1, 0, 0]);
    t.extend([0xFF, 0xFF, 0xFF, 0xFF, 0, 3, 0, 4, 0, 5, 0, 6]);
    let ankr = Ankr::parse(&t, 1).expect("parses");
    assert_eq!(ankr.point_count(0), 2);
    assert_eq!(ankr.anchor_point(0, 1), Some(AnchorPoint { x: 5, y: 6 }));
    assert_eq!(ankr.anchor_point(0, 2), None);
}
