use super::*;

pub(super) fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A table of `parts` after the header in order: coverage, store, conditions, axis indices, records, each offset 0
// where its part is empty.
pub(super) fn table(parts: [&[u8]; 5]) -> Vec<u8> {
    let mut out = 0x10000u32.to_be_bytes().to_vec();
    let mut at = 24;
    for p in &parts {
        out.extend((if p.is_empty() { 0 } else { at as u32 }).to_be_bytes());
        at += p.len();
    }
    parts.iter().for_each(|p| out.extend(*p));
    out
}

pub(super) fn index(items: &[&[u8]]) -> Vec<u8> {
    let mut out = (items.len() as u32).to_be_bytes().to_vec();
    out.push(1);
    let mut at = 1u8;
    out.push(at);
    for it in items {
        at += it.len() as u8;
        out.push(at);
    }
    items.iter().for_each(|it| out.extend(*it));
    out
}

#[test]
fn a_number_counts_its_bytes_by_its_leading_ones() {
    let read = |b: &[u8]| uint32var(b, &mut 0);
    assert_eq!(read(&[0x7F]), Some(0x7F));
    assert_eq!(read(&[0x81, 0x02]), Some(0x102));
    assert_eq!(read(&[0xC1, 0x02, 0x03]), Some(0x10203));
    assert_eq!(read(&[0xE1, 0x02, 0x03, 0x04]), Some(0x1020304));
    assert_eq!(read(&[0xF0, 0xFF, 0xFF, 0xFF, 0xFF]), Some(u32::MAX));
    assert_eq!(read(&[0xF7, 0, 0, 0, 1]), Some(1), "a lead past 0xF0 still takes four bytes");
    assert_eq!(read(&[0xC1, 0x02]), None, "cut short");
}

// Runs cut to the values asked for, of every width, signed or not; a run cut short by the data ends the read.
#[test]
fn tuple_value_runs_are_cut_to_what_is_asked() {
    let d = [0x01, 0xFF, 0x80, 0x82, 0x40, 0xFF, 0xFE, 0xC0, 0xFF, 0xFF, 0xFF, 0xFD];
    assert_eq!(runs(&d, &mut 0, 9, true), Ok(vec![-1, -128, 0, 0, 0, -2, -3]));
    assert_eq!(runs(&d, &mut 0, 9, false), Ok(vec![255, 128, 0, 0, 0, 65534, 4294967293]));
    let mut p = 0;
    assert_eq!(runs(&d, &mut p, 3, true), Ok(vec![-1, -128, 0]));
    assert_eq!(p, 4, "the zeros run read whole, its values cut to what's asked");
    assert_eq!(runs(&[0x41, 0, 1, 0], &mut 0, 9, true), Err(0), "two 16-bit values in three bytes");
    assert_eq!(runs(&[0x00, 5, 0x41, 0], &mut 0, 9, true), Err(1));
}

// Coverage formats 1 to 4, a range's index counted on from its start without a look at the records.
#[test]
fn coverage_reads_every_format() {
    let records = index(&[b"x"]);
    let at = |coverage: Vec<u8>, gid| Varc::new(&table([&coverage, &[], &[], &[], &records])).unwrap().coverage(gid);
    assert_eq!(at(u16s(&[1, 3, 4, 9, 70]), 9), Some(1));
    assert_eq!(at(u16s(&[1, 3, 4, 9, 70]), 10), None);
    assert_eq!(at(u16s(&[2, 2, 4, 6, 0, 10, 20, 900]), 15), Some(905));
    let mut wide = vec![0, 3, 0, 0, 2, 1, 0, 0, 2, 0, 1];
    assert_eq!(at(wide.clone(), 0x20001), Some(1));
    wide = vec![0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 9, 0, 0, 7];
    assert_eq!(at(wide.clone(), 0x10004), Some(11));
    assert_eq!(at(u16s(&[5, 1, 4]), 4), None, "format 5");
    assert_eq!(at(u16s(&[1, 400, 4]), 4), None, "a count past the table");
}

// INDEX entries hold where offsets are 1-based, in order and inside the table; a record of any length reads.
#[test]
fn an_index_entry_must_lie_inside_the_table() {
    let coverage = u16s(&[1, 1, 7]);
    let record = |records: &[u8]| {
        let t = table([&coverage, &[], &[], &[], records]);
        Varc::new(&t).and_then(|v| v.record(7).map(<[u8]>::to_vec))
    };
    assert_eq!(record(&index(&[b"abc"])), Some(b"abc".to_vec()));
    assert_eq!(record(&[0, 0, 0, 1, 1, 1, 9, b'a']), None, "an end past the table");
    assert_eq!(record(&[0, 0, 0, 1, 1, 0, 1, b'a']), None, "an offset of 0");
    assert_eq!(record(&[0, 0, 0, 1, 1, 3, 2, b'a']), None, "offsets out of order");
    assert_eq!(record(&[0, 0, 0, 1, 5, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]), None, "an offset size of 5");
    assert_eq!(record(&[0, 0, 0, 0, 1, 1]), None, "no entries");
    let mut big = vec![0, 0, 0, 1, 3, 0, 0, 1, 1, 0, 1];
    big.resize(big.len() + 0x10000, 0);
    assert_eq!(record(&big).map(|r| r.len()), Some(0x1_0000), "64K and a byte");
}

#[test]
fn the_table_loads_only_whole_at_version_1_0() {
    let good = table([&u16s(&[1, 0]), &[], &[], &[], &index(&[b"x"])]);
    assert!(Varc::new(&good).is_some());
    let mut minor = good.clone();
    minor[3] = 1;
    assert!(Varc::new(&minor).is_none(), "version 1.1");
    assert!(Varc::new(&good[..23]).is_none());
    let mut past = good.clone();
    past[20..24].copy_from_slice(&1000u32.to_be_bytes());
    assert!(Varc::new(&past).is_none(), "records past the end");
}

// One component of every field: its gid in 24 bits, a condition, axis values from a list of two, both variations,
// all nine transform fields, and two reserved bits each skipping a number. ScaleY absent mirrors ScaleX.
#[test]
fn a_component_reads_every_field() {
    let axis_list = index(&[&[0x01, 3, 7]]);
    let t = table([&u16s(&[1, 0]), &[], &[], &axis_list, &index(&[b"x"])]);
    let varc = Varc::new(&t).unwrap();
    let flags: u32 = 0x1FFF & !0x200 | 1 << 13 | 1 << 14 | 1 << 16 | 1 << 31;
    let mut r = vec![0xF0];
    r.extend(flags.to_be_bytes());
    r.extend([0x01, 0x02, 0x03, 0x05, 0x00, 0x01, 0x05, 0xFF, 0x09, 0x0A]);
    for v in [10i16, -20, 2048, 1536, 100, -100, 30, 40] {
        r.extend(v.to_be_bytes());
    }
    r.extend([0x81, 0x00, 0x7F]);
    let mut p = 0;
    let c = varc.component(&r, &mut p).unwrap();
    assert_eq!(p, r.len());
    assert_eq!((c.gid, c.condition, c.axis_indices, c.axis_var, c.transform_var), (0x10203, 5, 0, 9, 10));
    assert_eq!(c.axis_values, [5 * 4, -4]);
    assert_eq!(c.fields, [640, -1280, 2048 * 16, 1536 * 64, 1536 * 64, 1600, -1600, 1920, 2560]);
    assert!(varc.component(&r[..r.len() - 1], &mut 0).is_none(), "the last reserved number cut short");
}

// Without a store each value's delta is 0, a condition's its default's sign; past a data set's count, nothing.
#[test]
fn deltas_without_a_store() {
    let t = table([&u16s(&[1, 0]), &[], &[], &[], &index(&[b"x"])]);
    let varc = Varc::new(&t).unwrap();
    assert_eq!(varc.deltas(0, 3, 4, Some(&[0x4000]), None), Some(vec![0, 0, 0]));
    assert_eq!(varc.deltas(0, 1, 0, None, Some(3)), Some(vec![1]));
    assert_eq!(varc.deltas(0, 1, 0, None, Some(0)), Some(vec![0]));
}

// A table whose store has one region of one axis, `axis` (index, start, peak, end in 2.14), and one data set of a
// row (100, -3).
fn one_region(axis: [u16; 4]) -> Vec<u8> {
    one_region_row(axis, &[0x41, 0, 100, 0xFF, 0xFD])
}

fn one_region_row(axis: [u16; 4], row: &[u8]) -> Vec<u8> {
    let mut region = 1u16.to_be_bytes().to_vec();
    region.extend(6u32.to_be_bytes());
    region.extend(u16s(&axis));
    let mut list = 1u32.to_be_bytes().to_vec();
    list.extend(8u32.to_be_bytes());
    list.extend(&region);
    let mut data = vec![1, 0, 1, 0, 0];
    data.extend(9u32.to_be_bytes());
    data.extend(index(&[row]));
    let mut store = u16s(&[1]);
    store.extend(12u32.to_be_bytes());
    store.extend(u16s(&[1]));
    store.extend((12 + list.len() as u32).to_be_bytes());
    store.extend(&list);
    store.extend(&data);
    table([&u16s(&[1, 0]), &store, &[], &[], &index(&[b"x"])])
}

// The region peaking at axis 0's maximum: half way, the deltas halve, kept to the bits asked for; without
// coordinates the first region applies whole.
#[test]
fn deltas_from_the_store() {
    let t = one_region([0, 0, 0x4000, 0x4000]);
    let varc = Varc::new(&t).unwrap();
    assert_eq!(varc.deltas(0, 2, 0, Some(&[0x8000]), None), Some(vec![50, -1]));
    assert_eq!(varc.deltas(0, 2, 4, Some(&[0x8000]), None), Some(vec![800, -24]));
    assert_eq!(varc.deltas(0, 2, 0, None, None), Some(vec![100, -3]));
    assert_eq!(varc.deltas(0, 2, 0, Some(&[0]), None), Some(vec![0, 0]));
    assert_eq!(varc.deltas(0, 1, 0, Some(&[0x8000]), Some(-50)), Some(vec![0]));
    assert_eq!(varc.deltas(0, 1, 0, Some(&[0x8000]), Some(-49)), Some(vec![1]));
    assert_eq!(varc.deltas(0x10000, 2, 0, Some(&[0x8000]), None), None, "outer past the sets");
    assert_eq!(varc.deltas(1, 2, 0, Some(&[0x8000]), None), None, "inner past the rows");
}

// A row of 200 values whose last run is cut short: three runs of 64 zeros are walked before the cut, and spent.
#[test]
fn a_row_cut_short_spends_the_values_walked() {
    let t = one_region_row([0, 0, 0x4000, 0x4000], &[0xBF, 0xBF, 0xBF, 0x41, 0]);
    let varc = Varc::new(&t).unwrap();
    assert_eq!(varc.deltas(0, 200, 0, Some(&[0x8000]), None), None);
    assert_eq!(varc.work.get(), 192);
}

// An axis whose range spans 0 around a peak of 0.5 is ill-formed, so OpenType's per-axis rule leaves it out and the
// row applies whole, at the default too; an axis past the coordinates sits at 0, short of a peak of 1.
#[test]
fn a_region_axis_spanning_0_is_skipped() {
    let t = one_region([0, 0xF000, 0x2000, 0x4000]);
    let varc = Varc::new(&t).unwrap();
    assert_eq!(varc.deltas(0, 2, 0, Some(&[0x4000]), None), Some(vec![100, -3]));
    assert_eq!(varc.deltas(0, 2, 0, Some(&[0]), None), Some(vec![100, -3]));
    let past = one_region([1, 0, 0x4000, 0x4000]);
    assert_eq!(Varc::new(&past).unwrap().deltas(0, 2, 0, Some(&[0x8000]), None), Some(vec![0, 0]));
}

// Each condition format at axis 0 = 0.5: a range, an and, an or with a null child, a negation of a null, 17 negations
// of a range that holds, an unknown format, an index the list lacks.
#[test]
fn every_condition_format_holds_by_its_own_rule() {
    let range = |lo: u16, hi: u16| u16s(&[1, 0, lo, hi]);
    let group = |fmt: u8, kids: &[&[u8]]| {
        let mut out = vec![0, fmt, kids.len() as u8];
        let mut at = 3 + 3 * kids.len();
        let mut body: Vec<u8> = Vec::new();
        for k in kids {
            out.extend(if k.is_empty() { [0, 0, 0] } else { [0, 0, at as u8] });
            at += k.len();
            body.extend(*k);
        }
        out.extend(body);
        out
    };
    let mut deep = range(0, 0x4000);
    for _ in 0..17 {
        let mut n = vec![0, 5, 0, 0, 5];
        n.extend(deep);
        deep = n;
    }
    let conds: Vec<Vec<u8>> = vec![
        range(0x1000, 0x3000),
        range(0x3000, 0x4000),
        group(3, &[&range(0, 0x4000), &range(0x1000, 0x2000)]),
        group(4, &[&[], &range(0x3000, 0x4000)]),
        vec![0, 5, 0, 0, 0],
        deep,
        u16s(&[9, 0]),
    ];
    let mut list = (conds.len() as u32).to_be_bytes().to_vec();
    let mut at = 4 + 4 * conds.len();
    for c in &conds {
        list.extend((at as u32).to_be_bytes());
        at += c.len();
    }
    conds.iter().for_each(|c| list.extend(c));
    let t = table([&u16s(&[1, 0]), &[], &list, &[], &index(&[b"x"])]);
    let varc = Varc::new(&t).unwrap();
    let holds = |i| varc.condition(i).map(|at| varc.holds(at, Some(&[0x8000])));
    let held: Vec<_> = (0..8).map(holds).collect();
    assert_eq!(held, [Some(true), Some(false), Some(true), Some(true), Some(false), Some(false), Some(false), None]);
}

// The coordinates a component draws at: its values set over its parent's, each cut to a 2.14 step half away from 0,
// an axis past them ignored; reset, the rest the font's.
#[test]
fn a_component_s_coordinates() {
    let axis_list = index(&[&[0x02, 0, 1, 5]]);
    let t = table([&u16s(&[1, 0]), &[], &[], &axis_list, &index(&[b"x"])]);
    let varc = Varc::new(&t).unwrap();
    let mut c = Component {
        flags: HAVE_AXES, gid: 0, condition: 0, axis_indices: 0, axis_values: vec![6, -6, 9], axis_var: 0,
        transform_var: 0, fields: [0, 0, 0, ONE, ONE, 0, 0, 0, 0],
    };
    // What it leaves there from before is written over.
    let coords = |c: &Component, parent: &[i64], font: &[i64]| {
        let mut out = vec![55; 5];
        varc.coords(c, parent, font, &mut out).then_some(out)
    };
    assert_eq!(coords(&c, &[100, 200], &[7, 7]), Some(vec![8, -8]));
    c.flags |= RESET_UNSPECIFIED_AXES;
    c.axis_values = vec![1, 2, 3];
    assert_eq!(coords(&c, &[100, 200, 300], &[7]), Some(vec![0, 4, 0]));
    assert_eq!(coords(&c, &[], &[7]), None, "a font with no coordinates sets none");
    c.axis_values = vec![1, 2, 3, 4];
    assert_eq!(coords(&c, &[100], &[7]), None, "more values than indices");
}

// A quarter turn, then a doubled ScaleX, a skew whose tangent is 1, and the center held where it was.
#[test]
fn a_transform_turns_scales_skews_about_its_center() {
    let mut f = [0, 0, 0, ONE, ONE, 0, 0, 0, 0];
    assert_eq!(transform(&f), ([ONE, 0, 0, ONE], (0, 0)));
    f[ROTATION] = ONE / 2;
    assert_eq!(transform(&f).0, [0, -ONE, ONE, 0]);
    f[SX] = 2 * ONE;
    assert_eq!(transform(&f).0, [0, -ONE, 2 * ONE, 0]);
    f = [0, 0, 0, ONE, ONE, ONE / 4, 0, 64 * 10, 0];
    let (m, offset) = transform(&f);
    assert_eq!(m, [ONE, -ONE, 0, ONE]);
    assert_eq!(offset, (0, 0), "a skew along x keeps a center on the x axis where it was");
}

// Each value of the matrix lies within half a unit of 16.16 of the true one, here a double's: it's rounded once, not
// after each step of the product.
#[test]
fn a_transform_matrix_is_rounded_once() {
    let half_turns = |v: i64| v as f64 / 65536.0 * core::f64::consts::PI;
    for rotation in (-ONE..=ONE).step_by(4099) {
        for skew in [-ONE / 3, -1234, 0, 777, ONE / 5] {
            for scale in [ONE / 3, ONE, 3 * ONE / 2 + 17] {
                let f = [0, 0, rotation, scale, ONE + 101, skew, -skew / 2, 0, 0];
                let (m, _) = transform(&f);
                let (r, kx, ky) = (half_turns(rotation), half_turns(skew).tan(), half_turns(-skew / 2).tan());
                let (sx, sy) = (scale as f64 / 65536.0, (ONE + 101) as f64 / 65536.0);
                let (c, s) = (r.cos(), r.sin());
                let want = [c * sx - s * sy * ky, -c * sx * kx - s * sy, s * sx + c * sy * ky, c * sy - s * sx * kx];
                for (got, want) in m.iter().zip(want) {
                    assert!((*got as f64 - want * 65536.0).abs() <= 0.5 + 1e-6, "{f:?}: {m:?}");
                }
            }
        }
    }
}

// Sixteen levels of ors, each naming the level below 255 times: 255^16 visits worked out one by one, a few hundred
// once each is kept.
#[test]
fn conditions_naming_one_child_many_times_finish() {
    let mut cond = vec![0, 1, 0, 0, 0x40, 0, 0x40, 0];
    for _ in 0..16 {
        let mut group = vec![0, 4, 255];
        let at = (3 + 3 * 255) as u32;
        (0..255).for_each(|_| group.extend(&at.to_be_bytes()[1..]));
        group.extend(&cond);
        cond = group;
    }
    let mut list = 1u32.to_be_bytes().to_vec();
    list.extend(8u32.to_be_bytes());
    list.extend(&cond);
    let t = table([&u16s(&[1, 0]), &[], &list, &[], &index(&[b"x"])]);
    let varc = Varc::new(&t).unwrap();
    let at = varc.condition(0).unwrap();
    assert!(!varc.holds(at, Some(&[0])));
}

// A range alone is worked out keeping nothing, as no group names it again.
#[cfg(any(debug_assertions, daegun_counting))]
#[test]
fn a_range_alone_keeps_nothing() {
    let list = [&[0, 0, 0, 1, 0, 0, 0, 8][..], &u16s(&[1, 0, 0xC000, 0x4000])].concat();
    let t = table([&u16s(&[1, 0]), &[], &list, &[], &index(&[b"x"])]);
    let varc = Varc::new(&t).unwrap();
    let at = varc.condition(0).unwrap();
    crate::allocations::set_counting(true);
    let (before, _, _) = crate::allocations::tally();
    let held = varc.holds(at, Some(&[0]));
    let (after, _, _) = crate::allocations::tally();
    crate::allocations::set_counting(false);
    assert_eq!((held, after - before), (true, 0));
}

// A range holding at 0 under 50,000 negations holds, and under 50,001 doesn't: no depth cuts the chain short.
#[test]
fn conditions_nest_without_end() {
    let holds = |n: usize| {
        let negations = [0, 5, 0, 0, 5].repeat(n);
        let chain = [&[0, 0, 0, 1, 0, 0, 0, 8][..], &negations, &u16s(&[1, 0, 0xC000, 0x4000])].concat();
        let t = table([&u16s(&[1, 0]), &[], &chain, &[], &index(&[b"x"])]);
        let varc = Varc::new(&t).unwrap();
        varc.condition(0).map(|at| varc.holds(at, Some(&[0])))
    };
    assert_eq!((holds(50_000), holds(50_001)), (Some(true), Some(false)));
}
