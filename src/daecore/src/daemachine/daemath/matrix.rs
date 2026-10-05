pub type Matrix = [f64; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

// `x' = a*x + c*y + e`, `y' = b*x + d*y + f` – the same convention as `TransformPen`,
// `Path::replay` and `OutlineOptions::transform`, so a matrix never needs converting between them.
pub fn invert(t: &Matrix) -> Option<Matrix> {
    let [a, b, c, d, e, f] = *t;
    let det = a * d - b * c;
    if !det.is_finite() || det == 0.0 {
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

pub fn concat(first: &Matrix, second: &Matrix) -> Matrix {
    [
        first[0] * second[0] + first[1] * second[2],
        first[0] * second[1] + first[1] * second[3],
        first[2] * second[0] + first[3] * second[2],
        first[2] * second[1] + first[3] * second[3],
        first[4] * second[0] + first[5] * second[2] + second[4],
        first[4] * second[1] + first[5] * second[3] + second[5],
    ]
}
