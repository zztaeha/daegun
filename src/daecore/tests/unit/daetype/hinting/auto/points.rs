use super::*;

// A charstring may draw before it moves, or after it closes: the strays open a contour of their
// own at the pen rather than joining the next one or none.
#[test]
fn drawing_without_a_move_opens_a_contour() {
    let mut pen = CollectPen::new();
    pen.line_to(10.0, 0.0);
    pen.line_to(10.0, 10.0);
    pen.move_to(0.0, 0.0);
    pen.line_to(5.0, 5.0);
    pen.line_to(0.0, 5.0);
    pen.close();
    pen.line_to(20.0, 0.0);
    let pts = pen.finish();
    assert_eq!(pts.contour_ends, [2, 5, 7]);
    assert_eq!((pts.x[6], pts.y[6]), (0.0, 0.0), "the strays after the close start where it closed");
}
