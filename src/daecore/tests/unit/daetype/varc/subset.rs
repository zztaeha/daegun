use super::*;
use super::super::tests::{index as records, table, u16s};
use super::super::Varc;

// Glyph 1's record of one component, `component`'s bytes, beside `axes`' lists.
fn one_record_beside(component: &[u8], axes: &[u8]) -> Vec<u8> {
    table([&u16s(&[1, 1, 1]), &[], &[], axes, &records(&[component])])
}

fn one_record(component: &[u8]) -> Vec<u8> {
    one_record_beside(component, &[])
}

#[test]
fn the_closure_leaves_out_a_component_past_the_font() {
    let t = one_record(&[0x00, 0, 5]);
    let mut closed: GlyphSet = [1].into_iter().collect();
    closure(&t, &mut closed, 5);
    assert_eq!(closed.iter().collect::<Vec<_>>(), [1]);
    closure(&t, &mut closed, 6);
    assert_eq!(closed.iter().collect::<Vec<_>>(), [1, 5]);
}

#[test]
fn an_axis_named_where_every_axis_is_pinned_refuses_the_table() {
    let t = one_record_beside(&[0x02, 0, 2, 0x00, 0x40, 0x40, 0x00], &records(&[&[0x00, 0]]));
    let kept: GlyphSet = [1, 2].into_iter().collect();
    assert_eq!(subset(&t, &kept, Some, true, true), Subset::Refused);
    assert!(matches!(subset(&t, &kept, Some, true, false), Subset::Written(_)));
}

// A record whose flags take two bytes where one holds them: copied where ids are retained, else written anew, its flags
// and its id at their shortest, the id 24 bits only past 65,535, so one stored in 24 bits narrows to 16.
#[test]
fn a_record_is_copied_where_ids_are_retained_and_written_anew_else() {
    let record = |component: &[u8], new_gid: &dyn Fn(u32) -> Option<u32>, retain| {
        let (t, kept): (_, GlyphSet) = (one_record(component), [1, 2].into_iter().collect());
        match subset(&t, &kept, new_gid, retain, false) {
            Subset::Written(out) => Varc::new(&out).and_then(|v| v.record(1)).map(<[u8]>::to_vec),
            _ => None,
        }
    };
    let long = [0x80, 0x00, 0, 2];
    assert_eq!(record(&long, &Some, true), Some(long.to_vec()));
    assert_eq!(record(&long, &Some, false), Some(vec![0x00, 0, 2]));
    let last = |gid| Some(if gid == 2 { 0xFFFF } else { gid });
    assert_eq!(record(&long, &last, false), Some(vec![0x00, 0xFF, 0xFF]));
    let past = |gid| Some(if gid == 2 { 0x1_0000 } else { gid });
    assert_eq!(record(&long, &past, false), Some(vec![0x90, 0x00, 1, 0, 0]));
    assert_eq!(record(&[0x90, 0x00, 0, 0, 2], &Some, false), Some(vec![0x00, 0, 2]));
}

// Condition 0's offset is null, so it holds: the component naming it is written without a condition, and only
// condition 1, a format 1 axis range, stays in the list, renumbered 0.
#[test]
fn a_null_condition_is_dropped() {
    let conditions = [&[0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 12][..], &u16s(&[1, 0, 0xC000, 0x4000])].concat();
    let record = [0x80, 0x80, 0, 2, 0, 0x80, 0x80, 0, 3, 1];
    let t = table([&u16s(&[1, 1, 1]), &[], &conditions, &[], &records(&[&record])]);
    let kept: GlyphSet = [1, 2, 3].into_iter().collect();
    let Subset::Written(out) = subset(&t, &kept, Some, true, false) else { panic!("no VARC written") };
    let written = Varc::new(&out).and_then(|v| v.record(1)).map(<[u8]>::to_vec);
    assert_eq!(written, Some(vec![0x00, 0, 2, 0x80, 0x80, 0, 3, 0]));
    let list = u32::from_be_bytes(out[12..16].try_into().expect("an offset")) as usize;
    assert_eq!(out[list..list + 4], [0, 0, 0, 1]);
}

// An And of 255 conditions all naming the next, 40 deep, is read and written once a condition, where walking each
// naming would take 255^40 steps.
#[test]
fn conditions_shared_by_their_parents_are_walked_once() {
    let and = [&[0, 3, 255][..], &[0, 3, 0].repeat(255)].concat();
    let conditions = [&[0, 0, 0, 1, 0, 0, 0, 8][..], &and.repeat(40), &u16s(&[1, 0, 0xC000, 0x4000])].concat();
    let t = table([&u16s(&[1, 1, 1]), &[], &conditions, &[], &records(&[&[0x80, 0x80, 0, 2, 0]])]);
    let kept: GlyphSet = [1, 2].into_iter().collect();
    let Subset::Written(out) = subset(&t, &kept, Some, true, false) else { panic!("no VARC written") };
    let list = u32::from_be_bytes(out[12..16].try_into().expect("an offset")) as usize;
    assert_eq!(out[list..list + 4], [0, 0, 0, 1]);
    assert!(out.len() < conditions.len() + 64, "each condition written once: {} bytes", out.len());
}

#[test]
fn a_number_takes_its_shortest_form() {
    let written = |v| {
        let mut out = Vec::new();
        varint(&mut out, v);
        out
    };
    assert_eq!(written(0x7F), [0x7F]);
    assert_eq!(written(0x80), [0x80, 0x80]);
    assert_eq!(written(0x3FFF), [0xBF, 0xFF]);
    assert_eq!(written(0x4000), [0xC0, 0x40, 0x00]);
    assert_eq!(written(0x0FFF_FFFF), [0xEF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(written(0x1234_5678), [0xF0, 0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn an_index_counts_its_offsets_from_1_in_the_fewest_bytes() {
    assert_eq!(index(&[]), [0, 0, 0, 0]);
    assert_eq!(index(&[&[7], &[8, 9]]), [0, 0, 0, 2, 1, 1, 2, 4, 7, 8, 9]);
    let long = vec![0; 255];
    assert_eq!(index(&[&long])[4..9], [2, 0, 1, 1, 0]);
}

#[test]
fn coverage_lists_ascending_glyphs_and_sorts_ranges_else() {
    assert_eq!(coverage(&[3, 5, 9]), Some(vec![0, 1, 0, 3, 0, 3, 0, 5, 0, 9]));
    assert_eq!(coverage(&[1, 2, 3, 4]), Some(vec![0, 2, 0, 1, 0, 1, 0, 4, 0, 0]));
    // Out of order, the ranges keep the index each started at.
    assert_eq!(coverage(&[9, 1]), Some(vec![0, 2, 0, 2, 0, 1, 0, 1, 0, 1, 0, 9, 0, 9, 0, 0]));
    assert_eq!(coverage(&[0x1_0000]).expect("a coverage")[..2], [0, 3]);
    assert_eq!(coverage(&[0x100_0000]), None);
}

#[test]
fn identical_objects_pack_once_and_lie_last_packed_first() {
    let mut p = Packer::within(3);
    let a = p.pack(Object::leaf(vec![1, 2])).expect("room for 2 bytes");
    let b = p.pack(Object::leaf(vec![3])).expect("room for 1 byte");
    assert_eq!(p.pack(Object::leaf(vec![1, 2])), Some(a));
    assert_eq!(p.pack(Object::leaf(vec![4])), None, "packed past the budget");
    let mut root = Object::leaf(vec![0; 4]);
    root.links = vec![(0, 2, a), (2, 2, b)];
    assert_eq!(p.finish(root), Some(vec![0, 5, 0, 4, 3, 1, 2]));
}

// Glyphs 0 to `glyphs - 1` whose records alternate offsets 1 and `r + 1`, the last `r + 1`, so every other glyph's
// record is the same `r` zero bytes, each 3 a component of glyph 0.
fn shared_records(glyphs: u16, r: u32) -> Vec<u8> {
    let end = u32::from(glyphs);
    let offsets = (0..=end).flat_map(|i| (if i % 2 == 0 && i < end { 1 } else { r + 1 }).to_be_bytes());
    let index = [&u32::from(glyphs).to_be_bytes()[..], &[4], &offsets.collect::<Vec<_>>(), &vec![0; r as usize]];
    table([&u16s(&[2, 1, 0, glyphs - 1, 0]), &[], &[], &[], &index.concat()])
}

// 1,000 glyphs sharing one record of 9,999 bytes: an instance writes four glyphs sharing a small one, and refuses
// these, which written per glyph come to 5 MB from 14 KB, past the budget. The record is read once.
#[test]
fn records_many_glyphs_share_are_written_within_the_budget() {
    let small = shared_records(4, 6);
    let Subset::Written(out) = subset(&small, &(0..4).collect(), Some, true, true) else { panic!("no VARC written") };
    assert_eq!(Varc::new(&out).and_then(|v| v.record(2)), Some(&[0; 6][..]));
    let t = shared_records(1_000, 9_999);
    let written = match subset(&t, &(0..1_000).collect(), Some, true, true) {
        Subset::Written(out) => out.len(),
        _ => 0,
    };
    assert_eq!(written, 0, "{} bytes of VARC written as {written}", t.len());
}

// A range under 10,000 negations is written whole, each condition once, and holds where the source's does: no depth
// refuses it.
#[test]
fn conditions_nested_without_end_are_written() {
    let negations = [0, 5, 0, 0, 5].repeat(10_000);
    let chain = [&[0, 0, 0, 1, 0, 0, 0, 8][..], &negations, &u16s(&[1, 0, 0xC000, 0x4000])].concat();
    let t = table([&u16s(&[1, 1, 1]), &[], &chain, &[], &records(&[&[0x80, 0x80, 0, 2, 0]])]);
    let kept: GlyphSet = [1, 2].into_iter().collect();
    let Subset::Written(out) = subset(&t, &kept, Some, true, false) else { panic!("no VARC written") };
    assert!(out.len() < chain.len() + 64, "each condition written once: {} bytes", out.len());
    let holds = |t: &[u8], at: &[i64]| Varc::new(t).and_then(|v| v.condition(0).map(|c| v.holds(c, Some(at))));
    assert_eq!((holds(&out, &[0]), holds(&out, &[0x2_0000])), (Some(true), Some(false)));
    assert_eq!((holds(&t, &[0]), holds(&t, &[0x2_0000])), (Some(true), Some(false)));
}
