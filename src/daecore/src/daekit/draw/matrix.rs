/// A 2×3 transform `[a, b, c, d, e, f]`: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`. The same convention as
/// [`TransformPen`](crate::outline::TransformPen), [`Path::replay`](crate::outline::Path::replay) and
/// [`OutlineOptions::transform`](crate::outline::OutlineOptions::transform), so a matrix never needs converting
/// between them.
pub type Matrix = [f64; 6];

/// The transform that moves nothing.
pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The transform that undoes `t`; None for one that collapses the plane or isn't finite.
pub fn invert(t: &Matrix) -> Option<Matrix> {
    let [a, b, c, d, e, f] = *t;
    let det = a * d - b * c;
    if !det.is_finite() || det == 0.0 || !(e.is_finite() && f.is_finite()) {
        return None;
    }
    let inv = [
        d / det,
        -b / det,
        -c / det,
        a / det,
        (c * f - d * e) / det,
        (b * e - a * f) / det,
    ];
    inv.iter().all(|v| v.is_finite()).then_some(inv)
}

/// The transform applying `first`, then `second`.
pub const fn concat(first: &Matrix, second: &Matrix) -> Matrix {
    [
        first[0] * second[0] + first[1] * second[2],
        first[0] * second[1] + first[1] * second[3],
        first[2] * second[0] + first[3] * second[2],
        first[2] * second[1] + first[3] * second[3],
        first[4] * second[0] + first[5] * second[2] + second[4],
        first[4] * second[1] + first[5] * second[3] + second[5],
    ]
}

// The box around a box's four corners once transformed, (x_min, y_min, x_max, y_max) both ways.
pub fn transform_box(t: &Matrix, [x0, y0, x1, y1]: [f64; 4]) -> [f64; 4] {
    let corners =
        [(x0, y0), (x0, y1), (x1, y0), (x1, y1)].map(|(x, y)| (t[0] * x + t[2] * y + t[4], t[1] * x + t[3] * y + t[5]));
    let first = [corners[0].0, corners[0].1, corners[0].0, corners[0].1];
    corners.iter().fold(first, |b, &(x, y)| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)])
}
