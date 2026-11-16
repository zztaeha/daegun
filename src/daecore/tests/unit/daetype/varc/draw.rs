use super::*;
use core::cell::RefCell;
use crate::daecore::daetype::instancer::FIXED_ONE as ONE;

// Records each leaf asked for and draws it as one point at the transformed origin, taking `work` to load; glyph 99
// draws a point and then fails, at any coordinates if `alike`.
type Call = (u32, Option<Vec<i64>>, Matrix, (i64, i64));

#[derive(Default)]
struct Mock {
    calls: RefCell<Vec<Call>>,
    work: usize,
    alike: bool,
}

impl Leaves for Mock {
    fn leaf(&self, gid: u32, coords: Option<&[i64]>, m: &Matrix, by: (i64, i64), out: &mut Path, work: &mut usize)
    -> Result<(), ()> {
        self.calls.borrow_mut().push((gid, coords.map(<[i64]>::to_vec), *m, by));
        *work += self.work;
        out.move_to(by.0 as f32, by.1 as f32);
        if gid == 99 { Err(()) } else { Ok(()) }
    }

    fn fails_alike(&self) -> bool {
        self.alike
    }
}

fn index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u32).to_be_bytes().to_vec();
    out.push(4);
    let mut at = 1u32;
    out.extend(at.to_be_bytes());
    for it in items {
        at += it.len() as u32;
        out.extend(at.to_be_bytes());
    }
    items.iter().for_each(|it| out.extend(it));
    out
}

// A component naming `gid`, moved `tx` units across.
fn moved_by(gid: u16, tx: i16) -> Vec<u8> {
    let mut out = vec![0x10];
    out.extend(gid.to_be_bytes());
    out.extend(tx.to_be_bytes());
    out
}

// Records by covered glyph, the coverage listing each glyph in `extra` past the records.
fn varc(records: &[(u16, Vec<u8>)], extra: &[u16]) -> Vec<u8> {
    let mut gids: Vec<u16> = records.iter().map(|r| r.0).chain(extra.iter().copied()).collect();
    gids.sort_unstable();
    let mut coverage = [1u16, gids.len() as u16].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    gids.iter().for_each(|g| coverage.extend(g.to_be_bytes()));
    let mut sorted: Vec<_> = records.to_vec();
    sorted.sort_by_key(|r| r.0);
    let records = index(&sorted.into_iter().map(|r| r.1).collect::<Vec<_>>());
    let mut out = 0x10000u32.to_be_bytes().to_vec();
    out.extend(24u32.to_be_bytes());
    out.extend([0u8; 12]);
    out.extend((24 + coverage.len() as u32).to_be_bytes());
    out.extend(coverage);
    out.extend(records);
    out
}

fn run(table: &[u8], gid: u32, mock: &Mock) -> Option<Path> {
    let mut out = Path::default();
    draw(&Varc::new(table).unwrap(), mock, Some(&[0]), gid, &mut out).map(|_| out)
}

// A glyph inside itself is refused before its record is read: its own outline under the transform its parent gave.
#[test]
fn a_cycle_draws_the_glyph_s_own_outline_moved() {
    let t = varc(&[(10, [moved_by(10, 5), moved_by(1, 0)].concat())], &[]);
    let mock = Mock::default();
    assert!(run(&t, 10, &mock).is_some());
    let calls = mock.calls.borrow();
    assert_eq!(calls.iter().map(|c| (c.0, c.3)).collect::<Vec<_>>(), [(10, (5, 0)), (1, (0, 0))]);
}

// A child whose record doesn't read is its own outline under the transform its parent gave, as a cycle is; the same
// at the top draws nothing from VARC.
#[test]
fn a_record_that_fails_draws_the_own_outline_where_its_parent_put_it() {
    let t = varc(&[(10, moved_by(11, 7))], &[11]);
    let mock = Mock::default();
    assert!(run(&t, 10, &mock).is_some());
    assert_eq!(mock.calls.borrow()[0].3, (7, 0));
    assert!(run(&t, 11, &Mock::default()).is_none());
}

// A leaf that fails leaves nothing behind, and the components after it still draw; one cut short ends the record.
#[test]
fn a_failed_leaf_is_left_out() {
    let t = varc(&[(10, [moved_by(99, 3), moved_by(1, 4), moved_by(2, 6)[..4].to_vec()].concat())], &[]);
    let mock = Mock::default();
    let out = run(&t, 10, &mock).unwrap();
    assert_eq!(out.parts().1, [(4.0, 0.0).into()]);
    assert_eq!(mock.calls.borrow().len(), 2);
}

// A glyph one level deep holds no frame of its own, and components setting no coordinates keep none: the walk
// allocates nothing beside its leaves, which here draw into room already made.
#[cfg(any(debug_assertions, daegun_counting))]
#[test]
fn a_glyph_one_level_deep_allocates_nothing() {
    let t = varc(&[(10, [moved_by(1, 5), moved_by(2, 0)].concat())], &[]);
    let mock = Mock { calls: RefCell::new(Vec::with_capacity(2)), ..Mock::default() };
    let (varc, mut out) = (Varc::new(&t).unwrap(), Path::default());
    out.reserve(2, 2);
    crate::allocations::set_counting(true);
    let (before, _, _) = crate::allocations::tally();
    let drawn = draw(&varc, &mock, None, 10, &mut out);
    let (after, _, _) = crate::allocations::tally();
    crate::allocations::set_counting(false);
    assert_eq!((drawn, after - before), (Some(()), 0));
}

// A parent's offset goes through its own matrix and the child's adds to it, cut to whole units at the leaf.
#[test]
fn transforms_compose_parent_first() {
    let mut turned = vec![0x40, 0, 1];
    turned.extend(0x0800i16.to_be_bytes());
    let t = varc(&[(10, moved_by(11, 100)), (11, turned)], &[]);
    let mock = Mock::default();
    run(&t, 10, &mock).unwrap();
    let call = &mock.calls.borrow()[0];
    assert_eq!((call.0, call.2, call.3), (1, [0, -ONE, ONE, 0], (100, 0)));
}

// A point moved by the matrix with each product rounded as 16.16; one off the grid takes the plain product.
#[test]
fn a_point_moves_by_rounded_products() {
    assert_eq!(moved(&[ONE / 3, 0, 0, ONE], (2, -1), 3.0, 5.0), (3.0, 4.0));
    assert_eq!(moved(&[ONE / 2, 0, 0, ONE], (0, 0), 3.5, 0.0), (1.75, 0.0));
}

// 300 components each naming a glyph of 300 leaves load 90,300 times, each a component and a load (64), past the
// budget: the glyph draws its own outline. 200 of 300 load 60,200 times and draw.
#[test]
fn loads_past_the_budget_fail_the_glyph() {
    let many = |n: usize, gid: u16| (0..n).map(|_| moved_by(gid, 0)).collect::<Vec<_>>().concat();
    let t = varc(&[(10, many(300, 11)), (11, many(300, 1))], &[]);
    assert!(run(&t, 10, &Mock::default()).is_none());
    let t = varc(&[(10, many(200, 11)), (11, many(300, 1))], &[]);
    assert!(run(&t, 10, &Mock::default()).is_some());
}

// Three components skewed 90 degrees both ways compose a matrix near 2^61 across; a fourth's offset through it passes
// i64 and is pinned there rather than wrapped.
#[test]
fn an_offset_through_nested_skews_saturates() {
    let skew = 2048i16.to_be_bytes();
    let skewed = |gid: u16| [&[0xC0, 0x60, 0][..], &gid.to_be_bytes(), &skew, &skew].concat();
    let far = [&[0x30, 0, 1][..], &(-32768i16).to_be_bytes(), &32767i16.to_be_bytes()].concat();
    let t = varc(&[(10, skewed(11)), (11, skewed(12)), (12, skewed(13)), (13, far)], &[]);
    let mock = Mock::default();
    assert!(run(&t, 10, &mock).is_some());
    assert_eq!(mock.calls.borrow()[0].3.0, i64::MAX >> 6);
}

// Glyph 10 loads glyph 11 `loads` times; 11's one component sets 64,000 axes, its axis list entry and its record each
// 1,000 bytes of 0xBF (runs of 64 zeros), so each load decodes 128,001 values. 33 loads pass the budget.
#[test]
fn values_decoded_past_the_budget_fail_the_glyph() {
    let table = |loads: usize| {
        let a: Vec<u8> = (0..loads).flat_map(|_| [0x00, 0x00, 0x0B]).collect();
        let b = [&[0x02, 0x00, 0x01, 0x00][..], &[0xBF; 1000]].concat();
        let coverage: Vec<u8> = [1u16, 2, 10, 11].iter().flat_map(|v| v.to_be_bytes()).collect();
        let axes = index(&[vec![0xBF; 1000]]);
        let (axes_at, records_at) = (24 + coverage.len() as u32, 24 + (coverage.len() + axes.len()) as u32);
        let mut t = 0x10000u32.to_be_bytes().to_vec();
        [24, 0, 0, axes_at, records_at].iter().for_each(|v: &u32| t.extend(v.to_be_bytes()));
        [coverage, axes, index(&[a, b])].concat().into_iter().for_each(|v| t.push(v));
        t
    };
    assert!(run(&table(40), 10, &Mock::default()).is_none());
    let mock = Mock::default();
    assert!(run(&table(30), 10, &mock).is_some());
    assert_eq!(mock.calls.borrow().len(), 30);
}

// Glyph 10 loads glyph 11 three times, whose 100 components each fail a format 1 condition (axis 0 at 1, where the
// draw is at 0): each component read (32) and each condition worked out (8) is charged, though nothing loads.
#[test]
fn components_a_condition_hides_are_decoded_against_the_budget() {
    let hidden: Vec<u8> = (0..100).flat_map(|_| [0x80, 0x80, 0x00, 0x01, 0x00]).collect();
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let (coverage, records) = (u16s(&[1, 2, 10, 11]), index(&[[0x00, 0x00, 0x0B].repeat(3), hidden]));
    let conditions = [&1u32.to_be_bytes()[..], &8u32.to_be_bytes(), &u16s(&[1, 0, 1, 1])].concat();
    let conditions_at = 24 + coverage.len() as u32;
    let mut t = 0x10000u32.to_be_bytes().to_vec();
    for at in [24, 0, conditions_at, 0, conditions_at + conditions.len() as u32] {
        t.extend(at.to_be_bytes());
    }
    t.extend([coverage, conditions, records].concat());
    let (varc, mock) = (Varc::new(&t).unwrap(), Mock::default());
    assert!(draw(&varc, &mock, Some(&[0]), 10, &mut Path::default()).is_some());
    assert!(mock.calls.borrow().is_empty());
    assert_eq!(varc.work.get(), 3 * (32 + 32) + 3 * 100 * (32 + 8));
}

// The leaves' work loading counts against what one glyph's draw may take: 30 leaves of 50,000,000 pass 2^30, 20 don't.
#[test]
fn a_leaf_s_work_counts_against_the_budget() {
    let many = |n: usize| (0..n).map(|_| moved_by(1, 0)).collect::<Vec<_>>().concat();
    assert!(run(&varc(&[(10, many(30))], &[]), 10, &Mock { work: 50_000_000, ..Mock::default() }).is_none());
    assert!(run(&varc(&[(10, many(20))], &[]), 10, &Mock { work: 50_000_000, ..Mock::default() }).is_some());
}

// The second component's flags, 0, written in five bytes under 0xF8: read from the last four, as measuring and a subset
// read them, it draws, and the third after it.
#[test]
fn flags_in_five_bytes_under_any_lead_from_0xf0_read_as_a_subset_reads_them() {
    let five = [&[0xF8, 0, 0, 0, 0][..], &2u16.to_be_bytes()].concat();
    let t = varc(&[(10, [moved_by(1, 0), five, moved_by(3, 0)].concat())], &[]);
    let mock = Mock::default();
    assert!(run(&t, 10, &mock).is_some());
    assert_eq!(mock.calls.borrow().iter().map(|c| c.0).collect::<Vec<_>>(), [1, 2, 3]);
}

// Glyph 99 named three times beside glyph 1: a leaf that fails alike everywhere is loaded once a draw, and one that
// may not, at each component.
#[test]
fn a_leaf_that_fails_alike_is_loaded_once_a_draw() {
    let t = varc(&[(10, [moved_by(99, 0), moved_by(1, 0), moved_by(99, 5), moved_by(99, 9)].concat())], &[]);
    for (alike, loads) in [(true, 1), (false, 3)] {
        let mock = Mock { alike, ..Mock::default() };
        assert!(run(&t, 10, &mock).is_some());
        assert_eq!(mock.calls.borrow().iter().filter(|c| c.0 == 99).count(), loads, "alike: {alike}");
    }
}

// A chain of 10,000 records, each moving the next a unit across, draws the leaf at its end 10,000 across: no depth
// stops it.
#[test]
fn a_chain_of_any_depth_draws_its_leaf() {
    let n = 10_000;
    let records: Vec<(u16, Vec<u8>)> =
        (0..n).map(|i| (10 + i, moved_by(if i + 1 < n { 11 + i } else { 1 }, 1))).collect();
    let mock = Mock::default();
    assert!(run(&varc(&records, &[]), 10, &mock).is_some());
    assert_eq!(mock.calls.borrow().iter().map(|c| (c.0, c.3)).collect::<Vec<_>>(), [(1, (10_000, 0))]);
}

// A chain of 40 whose last names the 21st, a glyph inside itself past the records scanned for one, draws that glyph's
// own outline there; drawn twice over, the chain's second walk finds none of the first's glyphs still open.
#[test]
fn a_cycle_deep_in_a_chain_draws_the_glyph_s_own_outline() {
    let records: Vec<(u16, Vec<u8>)> =
        (0..40).map(|i| (10 + i, moved_by(if i < 39 { 11 + i } else { 30 }, 1))).collect();
    let twice = [&records[..], &[(9, [moved_by(10, 0), moved_by(10, 100)].concat())]].concat();
    let mock = Mock::default();
    assert!(run(&varc(&twice, &[]), 9, &mock).is_some());
    assert_eq!(mock.calls.borrow().iter().map(|c| (c.0, c.3)).collect::<Vec<_>>(), [(30, (40, 0)), (30, (140, 0))]);
}

// 70,000 components naming one leaf, past 65,536 loads and 64 KB of record: each load is priced in the budget, which
// the record's 350,000 bytes raise past what they take, and every one draws.
#[test]
fn a_record_of_any_width_draws_each_component() {
    let t = varc(&[(10, moved_by(1, 0).repeat(70_000))], &[]);
    let mock = Mock::default();
    assert!(run(&t, 10, &mock).is_some());
    assert_eq!(mock.calls.borrow().len(), 70_000);
}

// Glyph 10 loads glyph 11 100 times and 11 loads a leaf 700 times, 70,100 loads of 64, past what DECODES and a table of
// some 4,000 bytes allow; beside another glyph's unread record of 20,000 bytes, the budget grows past them.
#[test]
fn the_budget_grows_with_the_table() {
    let wide = [(10, moved_by(11, 0).repeat(100)), (11, moved_by(1, 0).repeat(700))];
    assert!(run(&varc(&wide, &[]), 10, &Mock::default()).is_none());
    let padded = [&wide[..], &[(12, vec![0; 20_000])]].concat();
    let mock = Mock::default();
    assert!(run(&varc(&padded, &[]), 10, &mock).is_some());
    assert_eq!(mock.calls.borrow().len(), 70_000);
}
