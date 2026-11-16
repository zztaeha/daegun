use super::*;

// 2,000 vertical glyphs sharing one construction of 100 variants.
fn math() -> Vec<u8> {
    const N: u16 = 2_000;
    const V: u16 = 100;
    let construction = 20 + 2 * N;
    let coverage = construction + 4 + 4 * V;
    let mut t = [1u16, 0, 0, 0, 10].map(u16::to_be_bytes).concat();
    t.extend([0u16, coverage - 10, 0, N, 0].map(u16::to_be_bytes).concat());
    (0..N).for_each(|_| t.extend((construction - 10).to_be_bytes()));
    t.extend([0u16, V].map(u16::to_be_bytes).concat());
    (0..V).for_each(|v| t.extend([N + 1 + v, 100].map(u16::to_be_bytes).concat()));
    t.extend([1u16, N].map(u16::to_be_bytes).concat());
    t.extend((1..=N).flat_map(u16::to_be_bytes));
    t
}

fn everything() -> (GlyphSet, Vec<u32>) {
    let mut active = GlyphSet::new();
    (0..2_200).for_each(|g| { active.insert(g); });
    (active, (0..2_200).collect())
}

// A run of (0, 100) records reads as a construction of 100 variants at every 4-byte step, so 200
// glyphs naming offsets 4 bytes apart rebuild 80 KB from 2 KB: past what 16-bit offsets reach.
#[test]
fn variants_past_16_bits_are_refused_not_left_without_coverage() {
    const N: usize = 200;
    const V: usize = 100;
    let region = 10 + 2 * N;
    let coverage = region + 4 * (N + V);
    let mut t = [1u16, 0, 0, 0, 10].map(u16::to_be_bytes).concat();
    t.extend([0, coverage, 0, N, 0].map(|v| (v as u16).to_be_bytes()).concat());
    (0..N).for_each(|i| t.extend(((region + 4 * i) as u16).to_be_bytes()));
    (0..N + V).for_each(|_| t.extend([0u16, 100].map(u16::to_be_bytes).concat()));
    t.extend([1u16, N as u16].map(u16::to_be_bytes).concat());
    t.extend((1..=N as u16).flat_map(u16::to_be_bytes));
    let (active, gid_map) = everything();

    if let Some(out) = subset_math(&t, &active, &gid_map) {
        let variants = usize::from(read_u16_be(&out, 8).expect("a variants offset"));
        let declared = read_u16_be(&out, variants + 6).unwrap_or(0);
        let covered = read_u16_be(&out, variants + 2).unwrap_or(0);
        assert!(declared == 0 || covered != 0, "{declared} glyphs declared with no coverage");
    }
}

#[test]
fn a_shared_construction_is_closed_over_once() {
    let (active, _) = everything();
    let found = math_closure(&math(), &active);
    assert!(found.len() <= 100, "100 variants shared by 2,000 glyphs found {}", found.len());
}

#[test]
fn a_shared_construction_is_written_once() {
    let t = math();
    let (active, gid_map) = everything();
    let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
    assert!(out.len() <= t.len(), "a {}-byte MATH subset to {} bytes", t.len(), out.len());
}

// math() with constants appended: each i16 and MathValueRecord value its own index.
fn with_constants(mut t: Vec<u8>) -> (Vec<u8>, usize) {
    let at = t.len();
    write_u16_be(&mut t, 4, at as u16);
    (0..4u16).for_each(|k| t.extend(k.to_be_bytes()));
    (4..55u16).for_each(|k| t.extend([k, 0].map(u16::to_be_bytes).concat()));
    t.extend(55u16.to_be_bytes());
    (t, at)
}

#[test]
fn a_subset_without_glyph_info_or_variants_keeps_the_constants() {
    let (t, at) = with_constants(math());
    let mut active = GlyphSet::new();
    active.insert(0);
    let out = subset_math(&t, &active, &[0]).expect("MATH is kept for its constants");
    let constants = usize::from(read_u16_be(&out, 4).expect("a constants offset"));
    assert_eq!(out.get(constants..constants + CONSTANTS_LEN), t.get(at..at + CONSTANTS_LEN));
}

#[test]
fn constants_cut_short_are_left_out_not_zeroed() {
    let (mut t, _) = with_constants(math());
    t.truncate(t.len() - 2);
    let (active, gid_map) = everything();
    let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
    assert_eq!(read_u16_be(&out, 4), Some(0));
}

// 20 glyphs naming constructions 4 bytes apart in a run of (0, 65,535) records, then one naming
// a construction of glyph 7: 1.3M records, past the 590K budget, so glyph 7 is never reached.
#[test]
fn the_closure_stops_reading_constructions_at_the_budget() {
    const N: u16 = 20;
    let coverage = 10 + 2 * (N + 1);
    let single = coverage + 4 + 2 * (N + 1);
    let region = single + 8;
    let mut t = [1u16, 0, 0, 0, 10, 0, coverage, 0, N + 1, 0].map(u16::to_be_bytes).concat();
    (0..N).for_each(|i| t.extend((region + 4 * i).to_be_bytes()));
    t.extend(single.to_be_bytes());
    t.extend([1u16, N + 1].map(u16::to_be_bytes).concat());
    t.extend((1..=N + 1).flat_map(u16::to_be_bytes));
    t.extend([0u16, 1, 7, 0].map(u16::to_be_bytes).concat());
    (0..u32::from(N) + 65_535).for_each(|_| t.extend([0u16, 0xFFFF].map(u16::to_be_bytes).concat()));
    let (active, _) = everything();
    assert!(!math_closure(&t, &active).contains(&7));
}

// Glyph 5's construction offset is NULL, which the OpenType spec reads as no table; read as an offset,
// it parses the MathVariants header as 14 variants, which glyph 6's nine make fit.
#[test]
fn a_null_construction_offset_is_no_construction() {
    use crate::daecore::daetype::math_table::math_glyph_construction;
    let mut t = [1u16, 0, 0, 0, 10, 0, 14, 0, 2, 0, 0, 22, 1, 2, 5, 6, 0, 9].map(u16::to_be_bytes).concat();
    (0..9u16).for_each(|v| t.extend([100 + v, 500].map(u16::to_be_bytes).concat()));
    let map: crate::daecore::daetype::Tables =
        [("MATH", t.clone().into())].into_iter().collect();
    assert!(math_glyph_construction(&map, 5, true, None).is_none(), "glyph 5 read a construction");
    assert_eq!(math_glyph_construction(&map, 6, true, None).map(|c| c.variants.len()), Some(9));

    let mut active = GlyphSet::new();
    active.insert(5);
    assert_eq!(math_closure(&t, &active), Vec::<u32>::new());
    let (active, gid_map) = everything();
    let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
    let variants = usize::from(read_u16_be(&out, 8).expect("a variants offset"));
    assert_eq!(read_u16_be(&out, variants + 6), Some(1), "glyph 5 kept a construction");
}

fn table_map(math: Vec<u8>) -> crate::daecore::daetype::Tables {
    [("MATH", math.into())].into_iter().collect()
}

// Glyph 5's construction has one variant and an assembly stating three parts with one in place,
// then one stating three variants with one in place. The font and its subset read the same.
#[test]
fn a_construction_cut_short_reads_alike_in_the_font_and_its_subset() {
    use crate::daecore::daetype::math_table::math_glyph_construction;
    let read = |math: Vec<u8>| {
        math_glyph_construction(&table_map(math), 5, true, None).map(|c| (c.variants.len(), c.assembly.is_some()))
    };
    let (active, gid_map) = everything();
    let words = |w: &[u16]| {
        let head = [1u16, 0, 0, 0, 10, 0, 12, 0, 1, 0, 18, 1, 1, 5];
        head.iter().chain(w).flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>()
    };
    let assembly = words(&[8, 1, 9, 500, 0, 0, 3, 10, 0, 0, 100, 0]);
    let variants = words(&[0, 3, 9, 500]);
    for t in [assembly, variants] {
        assert_eq!(read(t.clone()), Some((1, false)));
        assert_eq!(read(subset_math(&t, &active, &gid_map).expect("a subset MATH")), Some((1, false)));
    }
}

// Glyph 5's top right MathKern states two heights and holds only the first of its three kerns.
// Read in part it pairs a height with another's kern, and its subset drops it, so neither reads it.
#[test]
fn a_math_kern_cut_short_reads_as_none() {
    use crate::daecore::daetype::math_table::{math_kern, MathKernCorner};
    let t = [1u16, 0, 0, 10, 0, 0, 0, 0, 8, 12, 1, 18, 0, 0, 0, 1, 1, 5, 2, 100, 0, 200, 0, 50, 0]
        .map(u16::to_be_bytes)
        .concat();
    let (active, gid_map) = everything();
    let subset = subset_math(&t, &active, &gid_map).expect("a subset MATH");
    for math in [t, subset] {
        assert_eq!(math_kern(&table_map(math), 5, MathKernCorner::TopRight, 50.0, None), 0.0);
    }
}

// 2,000 glyphs whose four corners all name one MathKern of 38 bytes: rebuilt per corner, that
// is 304 KB, past what MathKernInfo's 16-bit offsets reach.
#[test]
fn a_math_kern_shared_by_many_corners_is_written_once() {
    const N: u16 = 2_000;
    let kern_info = 18u16;
    let kern = 4 + 8 * N;
    let coverage = kern + 38;
    let mut t = [1u16, 0, 0, 10, 0, 0, 0, 0, kern_info - 10, coverage, N].map(u16::to_be_bytes).concat();
    (0..4 * N).for_each(|_| t.extend(kern.to_be_bytes()));
    t.extend(4u16.to_be_bytes());
    (0..9u16).for_each(|k| t.extend([k, 0].map(u16::to_be_bytes).concat()));
    t.extend([1u16, N].map(u16::to_be_bytes).concat());
    t.extend((1..=N).flat_map(u16::to_be_bytes));
    let (active, gid_map) = everything();
    let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
    let glyph_info = usize::from(read_u16_be(&out, 6).expect("a glyph info offset"));
    assert_ne!(read_u16_be(&out, glyph_info + 6), Some(0), "MathKernInfo was refused");
    assert!(out.len() <= t.len(), "a {}-byte MATH subset to {} bytes", t.len(), out.len());
}

// Glyph 1's construction keeps glyph 1; the rest start four bytes apart in a run of 16,384 records of a glyph the
// subset lacks, each read to the run's end: 200 read 3.3 million records from 66 KB, past the budget, so refused.
#[test]
fn constructions_that_overlap_are_read_within_the_budget() {
    const RUN: usize = 16_384;
    let overlapping = |n: usize| {
        let kept = 10 + 2 * (n + 1);
        let coverage = kept + 8;
        let run = coverage + 4 + 2 * (n + 1);
        let mut t = [1u16, 0, 0, 0, 10].map(u16::to_be_bytes).concat();
        t.extend([0, coverage, 0, n + 1, 0, kept].map(|v| (v as u16).to_be_bytes()).concat());
        (0..n).for_each(|i| t.extend(((run + 4 * i) as u16).to_be_bytes()));
        t.extend([0u16, 1, 1, 0, 1, n as u16 + 1].map(u16::to_be_bytes).concat());
        t.extend((1..=n as u16 + 1).flat_map(u16::to_be_bytes));
        (0..RUN).for_each(|_| t.extend([0x7FFFu16, 0xFFFF].map(u16::to_be_bytes).concat()));
        t
    };
    let (active, gid_map) = everything();
    assert!(subset_variants(&overlapping(5), 10, &active, &gid_map).is_some_and(|v| v.len() > 10), "nothing kept");
    assert_eq!(subset_variants(&overlapping(200), 10, &active, &gid_map), None);
}
