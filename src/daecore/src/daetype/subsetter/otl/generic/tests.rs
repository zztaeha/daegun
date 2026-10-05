use alloc::vec::Vec;

use super::schemas::{gdef, gpos, gsub};
use super::{subset_subtable, Devices, Schema, Work};
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be};
use crate::daecore::daetype::subsetter::otl::parse_coverage;
use crate::daecore::daetype::subsetter::GlyphSet;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn subset(table: &[u8], schema: Schema) -> Option<Vec<u8>> {
    let mut active = GlyphSet::new();
    (0..=u16::MAX).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u16> = (0..=u16::MAX).collect();
    subset_subtable(table, 0, &schema, &active, &gid_map, Devices::Keep, &mut Work::for_table(table.len()))
}

fn at(t: &[u8], o: usize) -> usize {
    usize::from(read_u16_be(t, o).expect("in the table"))
}

type Body = (Vec<(u16, i16, u16)>, Vec<u8>);

// A PairPos 1 covering glyphs 1 on, glyph g's PairSet `bodies[order[g - 1]]`. A body is its records
// (second glyph, x advance, device offset or 0), then Device tables; format 0x44 if any names one.
fn pair_pos1(order: &[usize], bodies: &[Body]) -> Vec<u8> {
    let devices = bodies.iter().any(|(r, _)| r.iter().any(|p| p.2 != 0));
    let n = order.len() as u16;
    let header = 10 + 2 * usize::from(n);
    let coverage = words(&[&[1, n][..], &(1..=n).collect::<Vec<_>>()].concat());
    let mut t = words(&[1, header as u16, if devices { 0x44 } else { 4 }, 0, n]);
    let mut at = header + coverage.len();
    let mut starts = Vec::new();
    let mut tail = Vec::new();
    for (records, extra) in bodies {
        starts.push(at);
        let mut body = (records.len() as u16).to_be_bytes().to_vec();
        for &(second, x, device) in records {
            body.extend(words(&[second, x as u16]));
            if devices {
                body.extend(device.to_be_bytes());
            }
        }
        body.extend(extra);
        at += body.len();
        tail.extend(body);
    }
    order.iter().for_each(|&b| t.extend((starts[b] as u16).to_be_bytes()));
    t.extend(coverage);
    t.extend(tail);
    t
}

// A PairSet's record `i`: its x advance and its device's bytes, if any.
fn pair(out: &[u8], set: usize, i: usize, device_len: usize) -> (i16, Option<Vec<u8>>) {
    let format = read_u16_be(out, 4).expect("a value format");
    let set_at = at(out, 10 + set * 2);
    let rec = set_at + 2 + i * if format & 0x40 != 0 { 6 } else { 4 };
    let x = read_i16_be(out, rec + 2).expect("an advance");
    let device = (format & 0x40 != 0)
        .then(|| at(out, rec + 4))
        .filter(|&o| o != 0)
        .map(|o| out[set_at + o..set_at + o + device_len].to_vec());
    (x, device)
}

const VARIATION_INDEX: u16 = 0x8000;

// 40 PairSets alike except glyph 2's, whose pair 1 names VariationIndex (0, 2) where the rest name
// (0, 1). Sharing matched bytes before the Device offsets are written would give glyph 2 the others'.
#[test]
fn pair_sets_differing_in_a_device_are_not_shared() {
    let set = |inner_at_1: u16| {
        let records = (0..300u16).map(|p| (100 + p, p as i16, if p == 1 { 1808 } else { 1802 })).collect();
        (records, words(&[0, 1, VARIATION_INDEX, 0, inner_at_1, VARIATION_INDEX]))
    };
    let order: Vec<usize> = (1..=40u16).map(|g| usize::from(g == 2)).collect();
    let out = subset(&pair_pos1(&order, &[set(1), set(2)]), gpos::pair_pos_schema()).expect("a subset");
    assert_eq!(pair(&out, 1, 1, 6).1, Some(words(&[0, 2, VARIATION_INDEX])), "glyph 2 lost its own VariationIndex");
    assert_eq!(pair(&out, 0, 1, 6).1, Some(words(&[0, 1, VARIATION_INDEX])));
}

// Two PairSets alike byte for byte at different places in the source are written once, as HarfBuzz
// and fontTools write identical tables.
#[test]
fn identical_pair_sets_are_written_once() {
    let body = || ((0..50u16).map(|p| (p, 7, 0)).collect(), Vec::new());
    let out = subset(&pair_pos1(&[0, 1], &[body(), body()]), gpos::pair_pos_schema()).expect("a subset");
    assert_eq!(at(&out, 10), at(&out, 12));
    assert_eq!(pair(&out, 1, 49, 0).0, 7);
}

// 17 PairSets of 1,000 pairs, the last at 64,114: the source fits 16-bit offsets with its coverage
// first. Written after the sets, the coverage would sit past 64 KB and drop the whole subtable.
#[test]
fn a_coverage_goes_before_the_sets_it_covers() {
    let bodies: Vec<_> = (0..17i16).map(|s| ((0..1000u16).map(|p| (p, s, 0)).collect(), Vec::new())).collect();
    let table = pair_pos1(&(0..17).collect::<Vec<_>>(), &bodies);
    let out = subset(&table, gpos::pair_pos_schema()).expect("a subset that fits");
    assert_eq!(pair(&out, 16, 999, 0).0, 16);
}

// Two PairSets of 800 and 1,000 pairs, each pair naming its own 40-byte Device table placed after
// its set, as the source does. Pooled after both sets, 233 of the second's offsets would come back NULL.
#[test]
fn every_device_stays_in_reach_of_its_pair_set() {
    let device = |s: u16, p: u16| [words(&[8, 41, 3, p, s]), alloc::vec![0; 30]].concat();
    let set = |s: u16, n: u16| {
        let records = (0..n).map(|p| (p, 1, 2 + 6 * n + 40 * p)).collect();
        (records, (0..n).flat_map(|p| device(s, p)).collect::<Vec<u8>>())
    };
    let out = subset(&pair_pos1(&[0, 1], &[set(0, 800), set(1, 1000)]), gpos::pair_pos_schema()).expect("a subset");
    for (s, n) in [(0, 800), (1, 1000)] {
        for p in 0..n {
            assert_eq!(pair(&out, s, usize::from(p), 40).1, Some(device(s as u16, p)), "set {s} pair {p}");
        }
    }
}

// 15,000 records of an x advance and a VariationIndex fill 60 KB, so the Device table only fits
// right after them, ahead of the 30 KB coverage.
#[test]
fn a_subtables_own_devices_go_first_when_only_that_fits() {
    const N: u16 = 15_000;
    let cov_at = 8 + 4 * usize::from(N) + 6;
    let mut t = words(&[2, cov_at as u16, 0x44, N]);
    (0..N).for_each(|i| t.extend(words(&[i, (8 + 4 * usize::from(N)) as u16])));
    t.extend(words(&[0, 9, VARIATION_INDEX]));
    t.extend(words(&[&[1, N][..], &(0..N).map(|g| g * 2).collect::<Vec<_>>()].concat()));
    let out = subset(&t, gpos::single_pos_schema()).expect("a subset that fits");
    let device = at(&out, 8 + 2);
    assert_eq!(out.get(device..device + 6), Some(&words(&[0, 9, VARIATION_INDEX])[..]));
}

// A ValueFormat's high byte is reserved, so records hold only the low byte's fields: 0x0104 is one
// x advance a record, and is written back as 0x0004.
#[test]
fn reserved_value_format_bits_hold_no_field() {
    let t = words(&[2, 14, 0x0104, 3, 10, 20, 30, 1, 3, 1, 2, 3]);
    let out = subset(&t, gpos::single_pos_schema()).expect("a subset");
    assert_eq!(read_u16_be(&out, 4), Some(0x0004));
    assert_eq!((2..5).map(|i| read_i16_be(&out, 2 * i + 4)).collect::<Vec<_>>(), [Some(10), Some(20), Some(30)]);
}

// A format 1 coverage out of order, [3, 2] to [13, 12]: written sorted, each glyph keeps its own.
#[test]
fn a_coverage_out_of_order_keeps_each_glyph_with_its_data() {
    let t = words(&[2, 10, 2, 13, 12, 1, 2, 3, 2]);
    let out = subset(&t, gsub::single_subst_schema()).expect("a subset");
    assert_eq!(parse_coverage(&out, at(&out, 2)), Ok(alloc::vec![2, 3]));
    assert_eq!((read_u16_be(&out, 6), read_u16_be(&out, 8)), (Some(12), Some(13)));
}

// A ligature of componentCount 0 is malformed and left out; the set's other two stay, and the
// subtable with them.
#[test]
fn a_ligature_of_no_components_leaves_the_others() {
    let t = words(&[1, 8, 1, 14, 1, 1, 1, 3, 8, 14, 18, 50, 2, 5, 51, 0, 52, 2, 6]);
    let out = subset(&t, gsub::ligature_subst_schema()).expect("a subset");
    let set = at(&out, 6);
    assert_eq!(read_u16_be(&out, set), Some(2));
}

// MultipleSubst defines format 1 only; a format 2 is not read as format 1.
#[test]
fn an_undefined_format_is_not_read_as_format_1() {
    let t = words(&[2, 8, 1, 14, 1, 1, 1, 2, 7, 8]);
    assert!(subset(&t, gsub::multiple_subst_schema()).is_none());
}

// A ligature with no LigatureAttach (a NULL offset) keeps its slot, so the next ligature's anchors
// stay with it; dropped, it would hand glyph 20 glyph 21's.
#[test]
fn a_null_ligature_attach_keeps_its_place() {
    let mut t = words(&[1, 12, 18, 1, 26, 38]);
    t.extend(words(&[1, 1, 10]));
    t.extend(words(&[1, 2, 20, 21]));
    t.extend(words(&[1, 0, 6, 1, 0, 0]));
    t.extend(words(&[2, 0, 6, 1, 4, 1, 100, 200]));
    let out = subset(&t, gpos::mark_lig_pos_schema()).expect("a subset");
    let ligs = at(&out, 10);
    assert_eq!((read_u16_be(&out, ligs), read_u16_be(&out, ligs + 2)), (Some(2), Some(0)));
    let attach = ligs + at(&out, ligs + 4);
    let anchor = attach + at(&out, attach + 2);
    assert_eq!((read_i16_be(&out, anchor + 2), read_i16_be(&out, anchor + 4)), (Some(100), Some(200)));
}

// Glyph 30 in class 5 of 2: the shaper refuses the pair, and the subset keeps it past the count
// rather than in class 0, which would take class 0's 50.
#[test]
fn a_class_past_the_count_stays_past_it() {
    let mut t = words(&[2, 24, 4, 0, 30, 38, 2, 2, 0, 0, 50, 0]);
    t.extend(words(&[1, 1, 20]));
    t.extend(words(&[1, 20, 1, 1]));
    t.extend(words(&[1, 30, 1, 5]));
    let out = subset(&t, gpos::pair_pos_schema()).expect("a subset");
    let count2 = read_u16_be(&out, 14).expect("class2Count");
    let cd2 = at(&out, 10);
    let class = crate::daecore::daetype::subsetter::otl::parse_classdef(&out, cd2).expect("a class definition");
    let glyph30 = class.iter().find(|&&(g, _)| g == 30).map(|&(_, c)| c);
    assert!(glyph30.is_some_and(|c| c >= count2), "glyph 30 is in class {glyph30:?} of {count2}");
}

// A format 3 caret keeps its VariationIndex, so a variable font's carets still vary.
#[test]
fn a_caret_keeps_its_variation_index() {
    let t = words(&[6, 1, 12, 1, 1, 30, 1, 4, 3, 500, 6, 0, 7, VARIATION_INDEX]);
    let out = subset(&t, gdef::lig_caret_list_schema()).expect("a subset");
    let lig = at(&out, 4);
    let caret = lig + at(&out, lig + 2);
    assert_eq!(read_u16_be(&out, caret), Some(3));
    let device = caret + at(&out, caret + 4);
    assert_eq!(out.get(device..device + 6), Some(&words(&[0, 7, VARIATION_INDEX])[..]));
}

// 30,000 inputs naming one coverage of every glyph: read once and shared, so the subtable is kept,
// where reading and remapping all 65,536 glyphs at each naming would take two billion steps.
#[test]
fn a_coverage_named_many_times_is_read_once() {
    const N: u16 = 30_000;
    let mut t = words(&[3, N, 0]);
    let cov = 6 + 2 * usize::from(N);
    (0..N).for_each(|_| t.extend((cov as u16).to_be_bytes()));
    t.extend(words(&[2, 1, 0, 0xFFFF, 0]));
    let started = std::time::Instant::now();
    assert!(subset(&t, gsub::context_subst_schema()).is_some(), "the subtable was refused");
    assert!(started.elapsed().as_secs_f64() < 2.0, "took {:?}", started.elapsed());
}

// 30,000 rule sets naming one set of 65,535 NULL rules: NULL slots cost work too, so the budget
// stops it before two billion slot reads.
#[test]
fn null_slots_spend_the_work_budget() {
    const N: u16 = 30_000;
    let set = 6 + 2 * usize::from(N);
    let mut t = words(&[1, 0, N]);
    (0..N).for_each(|_| t.extend((set as u16).to_be_bytes()));
    t.extend(u16::MAX.to_be_bytes());
    t.extend(alloc::vec![0u8; 2 * usize::from(u16::MAX)]);
    let started = std::time::Instant::now();
    let _ = subset(&t, gsub::context_subst_schema());
    assert!(started.elapsed().as_secs_f64() < 2.0, "took {:?}", started.elapsed());
}

// Stripped, a Device table is never read: one of 16 KB named 8,000 times, too much work to keep,
// still leaves the records, without their devices.
#[test]
fn stripped_devices_cost_no_work() {
    const N: u16 = 2000;
    let mut t = words(&[2, 8 + 8 * N, 0x00F0, N]);
    (0..4 * N).for_each(|_| t.extend((18 + 8 * N).to_be_bytes()));
    t.extend(words(&[2, 1, 0, N - 1, 0, 0, u16::MAX, 1]));
    t.extend(alloc::vec![0u8; 16_384]);
    let mut active = GlyphSet::new();
    (0..N).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u16> = (0..N).collect();
    let schema = gpos::single_pos_schema();
    let out = subset_subtable(&t, 0, &schema, &active, &gid_map, Devices::Strip, &mut Work::for_table(t.len()));
    assert_eq!(out.as_deref().and_then(|o| read_u16_be(o, 4)), Some(0));
}

// 1,000 subtables of one table, each naming the same coverage of every glyph: it is read once for
// the table, so all fit its budget, where each subtable reading it again would run out after eight.
#[test]
fn a_coverage_shared_by_subtables_is_read_once_per_table() {
    const N: usize = 1000;
    let mut t = Vec::new();
    for i in 0..N {
        t.extend(words(&[3, 1, 0, (8 * N - 8 * i) as u16]));
    }
    t.extend(words(&[2, 1, 0, 0xFFFF, 0]));
    let mut active = GlyphSet::new();
    (0..=u16::MAX).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u16> = (0..=u16::MAX).collect();
    let mut work = Work::for_table(t.len());
    let schema = gsub::context_subst_schema();
    let kept = (0..N)
        .filter(|i| subset_subtable(&t, 8 * i, &schema, &active, &gid_map, Devices::Keep, &mut work).is_some())
        .count();
    assert_eq!(kept, N);
}
