use alloc::vec::Vec;

use super::*;

// Premultiplied pixels: clear, translucent and opaque, in colors past each mode's branches.
fn pixels() -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for a in [0.0f32, 0.3, 1.0] {
        for c in [[0.0, 0.5, 1.0], [0.9, 0.1, 0.35], [0.2, 0.2, 0.2]] {
            out.push([c[0] * a, c[1] * a, c[2] * a, a]);
        }
    }
    out
}

// Each mode's row lands `cover` of the way from what was there to the pixel's own composite, source over's short
// way included; a paint is source over a row of its one color.
#[test]
fn a_row_composites_as_each_pixel_does() {
    let ps = pixels();
    for mode in (0..28).map(Blend::from_colr) {
        for s in &ps {
            for d in &ps {
                let r = mode.composite(straight(s), s[3], straight(d), d[3]);
                let full = [r.color[0], r.color[1], r.color[2], r.alpha];
                for k in [0.0f32, 0.25, 1.0] {
                    let mut row = [*d];
                    composite_row(mode, &[*s], &[k], &mut row);
                    for i in 0..4 {
                        let want = d[i] + k * (full[i] - d[i]);
                        assert!((row[0][i] - want).abs() <= 1e-6, "{mode:?} {s:?} onto {d:?} at {k}: {:?}", row[0]);
                    }
                    if mode == Blend::SrcOver {
                        let mut painted = [*d];
                        paint_row(*s, &[k], &mut painted);
                        assert!((0..4).all(|i| (painted[0][i] - row[0][i]).abs() <= 1e-6), "{s:?} onto {d:?} at {k}");
                    }
                }
            }
        }
    }
}
