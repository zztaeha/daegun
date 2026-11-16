use super::*;
use super::blues::BlueZone;

// A variable instance's fractional coordinates scale as they are: 0.9 and -0.9 units of 64 at
// 64 px are 58/64 px either side of zero, where truncating first would make both 0.
#[test]
fn a_fractional_coordinate_is_scaled_as_it_is() {
    let zone = BlueZone::new;
    let blues = BlueZones { zones: alloc::vec![zone(50.0, 51.0, true), zone(0.0, -1.0, false)] };
    let mut hinter = AutoHinter::from_zones(blues, 64).expect("two zones");
    let pts = AutoPoints {
        x: alloc::vec![0.9, -0.9],
        y: alloc::vec![0.9, -0.9],
        flags: alloc::vec![ON_CURVE; 2],
        contour_ends: alloc::vec![0, 1],
    };
    let out = hinter.hint(&pts, super::super::f26dot6::Size::from_px(64.0).expect("a size"));
    assert_eq!((&out.x[..], &out.y[..]), (&[58, -58][..], &[58, -58][..]));
}
