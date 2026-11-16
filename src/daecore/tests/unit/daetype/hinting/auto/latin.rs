use super::*;
use super::super::blues::BlueZone;

// A zone is active at a size only while its overshoot is at most 3/4 pixel: 50 units of 1000 are
// 0.75 px at 15 px and 0.77 px at 15.4, so only the exact size turns it off.
#[test]
fn a_zone_is_judged_at_the_exact_size() {
    let mut blues = BlueZones { zones: alloc::vec![BlueZone::new(500.0, 550.0, true)] };
    let mut active = |px| {
        let size = f26dot6::Size::from_px(px).expect("a size");
        fit_zones(&mut blues, &|v| f26dot6::scale_f32(v, size, 1000));
        blues.zones.iter().filter(|z| z.fit.is_some()).count()
    };
    assert_eq!((active(15.0), active(15.4)), (1, 0));
}

// Zones are fitted once a size and kept, so a size met again must not read another's: 50 units of overshoot keep
// the top zone at 15 px and drop it at 15.4, and a box's top 6 units above it snaps down to it or rounds up.
#[test]
fn zones_kept_for_one_size_are_never_read_at_another() {
    let zones = alloc::vec![BlueZone::new(0.0, -10.0, false), BlueZone::new(497.0, 547.0, true)];
    let blues = BlueZones { zones };
    let pts = AutoPoints {
        x: alloc::vec![100.0, 100.0, 400.0, 400.0],
        y: alloc::vec![0.0, 503.0, 503.0, 0.0],
        flags: alloc::vec![super::super::points::ON_CURVE; 4],
        contour_ends: alloc::vec![3],
    };
    let fitted = |px, (blues, at, s): &mut (BlueZones, Option<core::num::NonZeroI32>, Scratch)| {
        let mut y = Vec::new();
        fit(&pts, blues, at, f26dot6::Size::from_px(px).expect("a size"), 1000, s, &mut y);
        y
    };
    let fresh = || (blues.clone(), None, Scratch::default());
    assert_ne!(fitted(15.0, &mut fresh()), fitted(15.4, &mut fresh()), "the sizes agree");
    let mut kept = fresh();
    for px in [15.0, 15.4, 15.0, 22.0, 15.4, 15.4] {
        assert_eq!(fitted(px, &mut kept), fitted(px, &mut fresh()), "at {px} px");
    }
}

// The last run starts at 10 and comes back round past 0 to the 20s, all within 10 of its start.
// Bounded by the contour rather than by what is left of it, it would take points 0 and 1 from the first run.
#[test]
fn no_point_falls_in_two_segments() {
    let pts = AutoPoints {
        x: alloc::vec![0.0, 100.0, 100.0, 0.0, 0.0, 100.0],
        y: alloc::vec![20.0, 20.0, 60.0, 60.0, 10.0, 0.0],
        flags: alloc::vec![super::super::points::ON_CURVE; 6],
        contour_ends: alloc::vec![5],
    };
    let mut s = Scratch::default();
    compute_segments(&pts, 10.0, 1000.0, &mut s);
    assert_eq!(s.seg_of, [0, 0, 1, 1, 2, 2]);
}

// Two facing segments with runs between them, off to the side: they link while LINK_SCAN - 1
// stand in the way and not past that, so a crafted outline costs segments times LINK_SCAN.
#[test]
fn linking_looks_past_at_most_link_scan_segments() {
    let seg = |pos, dir, x_min| Segment { pos, x_min, x_max: x_min + 100.0, link: NIL, dir, round: false };
    let linked = |between: usize| {
        let mut s = Scratch::default();
        s.segments.push(seg(0.0, -1, 0.0));
        s.segments.extend((0..between).map(|i| seg(1.0 + i as f32 * 0.1, -1, 200.0)));
        s.segments.push(seg(100.0, 1, 0.0));
        link_segments(333.0, &mut s);
        s.segments[0].link != NIL
    };
    assert_eq!((linked(LINK_SCAN - 1), linked(LINK_SCAN)), (true, false));
}

// A position's key orders as `total_cmp` does, signed zeros, subnormals, infinities and NaNs among them.
#[test]
fn position_keys_order_as_total_cmp() {
    let values = [
        f32::NEG_INFINITY, -1e30, -1.0, -f32::MIN_POSITIVE, -1e-45, -0.0, 0.0, 1e-45, f32::MIN_POSITIVE, 1.0, 3e38,
        f32::INFINITY, f32::NAN, -f32::NAN,
    ];
    for a in values {
        for b in values {
            assert_eq!(ordered(a).cmp(&ordered(b)), a.total_cmp(&b), "{a} against {b}");
        }
    }
}

// Placed by counting up to RANKED segments and by sorting past that, the order is the sort's: by position, then
// index, repeated positions and signed zeros among them.
#[test]
fn segments_are_placed_by_position_then_index() {
    for m in [3, RANKED, RANKED + 1, 3 * RANKED] {
        let mut s = Scratch::default();
        s.segments.extend((0..m).map(|i| {
            let pos = [0.0, -0.0, 1.0, -1.0, 0.5][i % 5] * (i % 3) as f32;
            Segment { pos, x_min: 0.0, x_max: 0.0, link: NIL, dir: 1, round: false }
        }));
        link_segments(1.0, &mut s);
        let mut want: Vec<usize> = (0..m).collect();
        want.sort_by(|&a, &b| s.segments[a].pos.total_cmp(&s.segments[b].pos).then(a.cmp(&b)));
        let got: Vec<usize> = s.placed.iter().map(|p| p.seg as usize).collect();
        assert_eq!(got, want, "{m} segments");
    }
}

// A distance at or past the bound a zone is skipped from scales past `reach`, at every em and size tried, so no
// zone skipped could have been the nearest.
#[test]
fn a_skipped_zone_is_never_the_nearest() {
    for upm in [1u16, 7, 1000, 1024, 2048, 65535] {
        for units in [32, 64, 77, 832, 6400, 64000, 1 << 21] {
            let size = f26dot6::Size::from_px(units as f32 / 64.0).expect("a size");
            let scale = |v: f32| f26dot6::scale_f32(v, size, upm);
            let reach = scale(f32::from(upm / 40)).min(f26dot6::HALF);
            let beyond = skipped_from(reach, size, upm);
            let mut d = beyond as f32;
            while f64::from(d) < beyond {
                d = d.next_up();
            }
            for _ in 0..256 {
                assert!(scale(d) > reach, "upm {upm}, size {units}: {d} scales to {}, reach {reach}", scale(d));
                d = d.next_up();
            }
        }
    }
}

// The walk starts at the first break, point 3, and comes round to point 2 last, a lone point and a minimum: the
// extrema still follow the runs in point order, 2 before 4.
#[test]
fn lone_extrema_follow_the_runs_in_point_order() {
    let pts = AutoPoints {
        x: (0..8).map(|i| i as f32 * 10.0).collect(),
        y: alloc::vec![30.0, 22.0, 14.0, 100.0, 200.0, 100.0, 46.0, 38.0],
        flags: alloc::vec![super::super::points::ON_CURVE; 8],
        contour_ends: alloc::vec![7],
    };
    let mut s = Scratch::default();
    compute_segments(&pts, 10.0, 1000.0, &mut s);
    let positions: Vec<f32> = s.segments.iter().map(|g| g.pos).collect();
    assert_eq!(positions, [42.0, 26.0, 14.0, 200.0]);
    assert_eq!(s.seg_of, [1, 1, 2, NIL, 3, NIL, 0, 0]);
}
