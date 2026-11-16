use super::*;

// The closed form is the distance the quad strays from the cubic at t = 1/4 and 3/4, sampled.
#[test]
fn the_deviation_is_the_distance_at_the_quarter_points() {
    let at = |p: &[Point; 4], q: &Quad, t: f32| {
        let u = 1.0 - t;
        let cubic = |a: f32, b: f32, c: f32, d: f32| {
            u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
        };
        let quad = |a: f32, b: f32, c: f32| u * u * a + 2.0 * u * t * b + t * t * c;
        let dx = cubic(p[0].x, p[1].x, p[2].x, p[3].x) - quad(q.start.x, q.control.x, q.end.x);
        let dy = cubic(p[0].y, p[1].y, p[2].y, p[3].y) - quad(q.start.y, q.control.y, q.end.y);
        dx.powi(2) + dy.powi(2)
    };
    for k in 0..200u16 {
        let v = |i: u16| f32::from((k * 37 + i * 101) % 211) - 100.0;
        let p = [(v(0), v(1)), (v(2), v(3)), (v(4), v(5)), (v(6), v(7))].map(Point::from);
        let q = quad_for(&p);
        let sampled = at(&p, &q, 0.25).max(at(&p, &q, 0.75));
        let d = deviation(&p);
        assert!((d - sampled).abs() <= 1e-3 * sampled.max(1.0), "{p:?}: {d} against {sampled}");
    }
}
