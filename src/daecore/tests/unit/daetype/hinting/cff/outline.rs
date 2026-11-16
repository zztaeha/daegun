use super::*;

const PX: i64 = 1 << 16;

// A contour's last on-curve point on its first is dropped at the close, then a contour left with one point.
#[test]
fn a_close_drops_a_last_point_on_the_first_then_a_lone_point() {
    let mut out = Outline::default();
    for (x, y) in [(0, 0), (PX, 0), (0, 0)] {
        out.push((x, y), true);
    }
    out.close(false);
    out.push((5 * PX, 5 * PX), true);
    out.close(false);
    let o = out.finish().expect("an outline");
    assert_eq!((o.x, o.contour_ends), (alloc::vec![0, 64], alloc::vec![1]));
}

// A line drawn back to the start is the close's to draw, so its point goes even where it didn't land on the start.
#[test]
fn a_line_back_to_the_start_is_dropped_at_the_close() {
    let mut out = Outline::default();
    for (x, y) in [(0, 0), (PX, 0), (PX, PX), (0, PX / 2)] {
        out.push((x, y), true);
    }
    out.close(true);
    let o = out.finish().expect("an outline");
    assert_eq!((o.y, o.contour_ends), (alloc::vec![0, 0, 64], alloc::vec![2]));
}

// Points round to the nearest 1/64 px, a half up.
#[test]
fn points_round_to_the_nearest_64th() {
    let mut out = Outline::default();
    for x in [511, 512, -512, -513] {
        out.push((x, 0), true);
    }
    out.push((0, 0), false);
    out.close(false);
    assert_eq!(out.finish().expect("an outline").x, [0, 1, 0, -1, 0]);
}

// A point past 2^23 px fails the outline.
#[test]
fn a_point_past_the_26_6_range_fails_the_outline() {
    let mut out = Outline::default();
    out.push((0, (1 << 39) + 1), true);
    out.push((0, 0), true);
    out.close(false);
    assert!(out.finish().is_none());
}

// An outline holds as many points and contours as its charstring draws, which its steps bound: 70,000 contours of
// three points each, past glyf's 16-bit numbering.
#[test]
fn an_outline_holds_what_its_charstring_draws() {
    let mut out = Outline::default();
    for k in 0..70_000 {
        for (x, y) in [(k, 0), (k, PX), (k + PX, 0)] {
            out.push((x, y), true);
        }
        out.close(false);
    }
    let o = out.finish().expect("an outline");
    assert_eq!((o.x.len(), o.contour_ends.len()), (210_000, 70_000));
}
