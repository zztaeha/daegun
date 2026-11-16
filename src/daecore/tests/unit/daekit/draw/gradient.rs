use alloc::vec::Vec;

use super::*;
use crate::daecore::daekit::draw::srgb::encoded;

const ID: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn kinds() -> Vec<GradientKind> {
    std::vec![
        GradientKind::Linear { x0: 1.0, y0: 2.0, x1: 9.0, y1: 5.0 },
        GradientKind::Radial { x0: 4.0, y0: 4.0, r0: 1.0, x1: 6.0, y1: 5.0, r1: 5.0 },
        GradientKind::Radial { x0: 2.0, y0: 2.0, r0: 3.0, x1: 8.0, y1: 6.0, r1: 1.0 },
        GradientKind::Sweep { cx: 5.0, cy: 5.0, start_angle: 30.0, end_angle: 250.0 },
        GradientKind::Sweep { cx: 5.0, cy: 5.0, start_angle: 90.0, end_angle: 90.0 },
    ]
}

// In linear light a row holds what `at` rounds: each straight channel within half a level once encoded, and the
// hundredth `encoded` strays from sRGB's curve; the alpha within half of its. Otherwise it's `at`'s color itself.
#[test]
fn a_row_samples_as_at_does_but_unrounded() {
    let red = Rgba { r: 255, g: 0, b: 0, a: 255 };
    let stops = [
        Stop { offset: 0.0, color: red },
        Stop { offset: 0.4, color: Rgba { r: 20, g: 200, b: 90, a: 60 } },
        Stop { offset: 1.0, color: Rgba { r: 0, g: 0, b: 255, a: 200 } },
    ];
    let flat = [Stop { offset: 0.0, color: red }, Stop { offset: 1.0, color: red }];
    let device = [1.2, 0.3, -0.2, 1.1, 0.5, 0.7];
    let (mut row, mut unrounded, mut none) = ([[0.0f32; 4]; 16], 0, 0);
    for kind in kinds() {
        for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect] {
            let (light, srgb) = (Interpolation::LinearLight, Interpolation::Srgb);
            let ways = [(&stops[..], light), (&stops, srgb), (&flat, light)];
            for (stops, how) in ways {
                let ramp = Ramp::with_interpolation(&Gradient::new(kind, stops, extend, ID), &device, how);
                let varying = how == Interpolation::LinearLight && matches!(ramp.0, Shape::Varying { .. });
                for y in 0..16 {
                    ramp.fill_row(3, y, &mut row);
                    for (i, p) in row.iter().enumerate() {
                        let Some(c) = ramp.at((3 + i) as f64, y as f64) else {
                            assert_eq!(*p, [0.0; 4], "{kind:?} {extend:?}: paints where `at` doesn't");
                            none += 1;
                            continue;
                        };
                        if !varying {
                            assert_eq!(*p, premultiplied(c), "{kind:?} {extend:?} {how:?}");
                            continue;
                        }
                        assert!((p[3] * 255.0 - f32::from(c.a)).abs() <= 0.501, "{kind:?} {extend:?}: {p:?} {c:?}");
                        if c.a > 0 {
                            for (&v, want) in p[..3].iter().zip([c.r, c.g, c.b]) {
                                let level = encoded((v / p[3]).min(1.0));
                                let gap = (level - f32::from(want)).abs();
                                assert!(gap <= 0.51, "{kind:?} {extend:?}: {p:?} {c:?} {level}");
                            }
                            unrounded += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(unrounded > 1000 && none > 50, "{unrounded} pixels unrounded, {none} painted nothing");
}
