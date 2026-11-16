use alloc::vec::Vec;

use super::*;
use crate::daecore::Orientation;

fn layouts() -> Vec<SubpixelLayout> {
    let blur: Vec<f32> = (0..36).map(|i| if i % 7 == 0 { 0.25 } else { 0.75 / 35.0 }).collect();
    let sharpen = [-0.25, 1.5, -0.25];
    let striped =
        |c: usize| (0..5).map(|k| if (c..c + 3).contains(&k) { sharpen[k - c] } else { 0.0 }).collect::<Vec<f32>>();
    let (r, g, b) = (striped(0), striped(1), striped(2));
    std::vec![
        SubpixelLayout::horizontal(StripeOrder::Rgb),
        SubpixelLayout::horizontal(StripeOrder::Bgr),
        SubpixelLayout::vertical(StripeOrder::Rgb),
        SubpixelLayout::unfiltered(StripeOrder::Bgr, Orientation::Horizontal),
        SubpixelLayout::unfiltered(StripeOrder::Rgb, Orientation::Vertical),
        SubpixelLayout::from_weights((4, 2), (6, 6), (-1, -2), [&blur, &blur, &blur]).unwrap(),
        SubpixelLayout::from_weights((3, 1), (3, 1), (0, 0), [&sharpen, &sharpen, &sharpen]).unwrap(),
        SubpixelLayout::from_weights((3, 1), (5, 1), (-1, 0), [&r, &g, &b]).unwrap(),
    ]
}

// Samples in runs: empty, full, or each its own level, the runs long enough to fill whole words of a row.
fn samples(n: usize) -> Vec<u8> {
    let (mut state, mut out) = (0x2545_f491_4f6c_dd1du64, Vec::with_capacity(n));
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    while out.len() < n {
        let (kind, run) = (next() % 3, 1 + (next() % 150) as usize);
        out.extend((0..run).map(|_| match kind {
            0 => 0,
            1 => 255,
            _ => (next() % 256) as u8,
        }));
    }
    out.truncate(n);
    out
}

// Each byte its window's weighted sum of the samples, in f64, clamped and rounded once.
#[test]
fn the_filter_rounds_each_window_s_sum_once() {
    for layout in layouts() {
        let ((mx, my), (ox, oy)) = (layout.margin(), layout.oversample());
        let (ox, oy) = (usize::from(ox), usize::from(oy));
        let (tx, (gx, gy)) = (usize::from(layout.taps().0), layout.origin());
        for (width, height) in [(1, 1), (70, 5), (300, 2)] {
            let stride = width * ox + 2 * mx;
            let tall = height * oy + 2 * my;
            let mut input = std::vec![0u8; stride * tall];
            let inner = samples(width * ox * height * oy);
            for (row, chunk) in inner.chunks_exact(width * ox).enumerate() {
                input[(row + my) * stride + mx..][..width * ox].copy_from_slice(chunk);
            }
            let mut out = std::vec![0u8; width * height * 3];
            layout.filter(&input, stride, (width, height), &mut out, &mut Vec::new());
            for (i, &v) in out.iter().enumerate() {
                let (c, x, y) = (i % 3, i / 3 % width, i / 3 / width);
                let sum: f64 = layout.weights(c).unwrap().iter().enumerate().map(|(k, &w)| {
                    let sx = (x * ox + mx) as isize + isize::from(gx) + (k % tx) as isize;
                    let sy = (y * oy + my) as isize + isize::from(gy) + (k / tx) as isize;
                    f64::from(w) * f64::from(input[sy as usize * stride + sx as usize])
                }).sum();
                let gap = (f64::from(v) - sum.clamp(0.0, 255.0)).abs();
                assert!(gap <= 0.5 + 1e-3, "{layout:?} {width}x{height}: pixel ({x}, {y}) channel {c} {gap} levels");
            }
        }
    }
}

// Stripes side by side give one kernel, shifted a sample a channel, red first or blue first; the rest give none:
// stripes down, a window two rows tall, one kernel on the same sample for every channel, and a negative weight.
#[test]
fn only_stripes_side_by_side_share_a_kernel() {
    let side_by_side = [true, true, false, true, false, false, false, false];
    for (layout, want) in layouts().into_iter().zip(side_by_side) {
        let Some((kernel, n, lo, reversed)) = layout.stripes() else {
            assert!(!want, "{layout:?} has stripes side by side");
            continue;
        };
        assert!(want, "{layout:?} has no stripes side by side");
        let tx = isize::from(layout.taps().0);
        for c in 0..3 {
            let shift = if reversed { 2 - c as isize } else { c as isize };
            let weights = layout.weights(c).unwrap();
            for k in 0..tx {
                let at = isize::from(layout.origin().0) + k - shift - lo;
                let want = if (0..n as isize).contains(&at) { kernel[at as usize] } else { 0.0 };
                assert_eq!(weights[k as usize], want, "{layout:?}: channel {c} tap {k}");
            }
        }
    }
}
