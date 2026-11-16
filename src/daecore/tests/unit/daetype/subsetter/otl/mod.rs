use super::*;

// Which format each takes: a coverage's list where it's no longer than its ranges, a class definition's smallest
// format by strict size in format order, each 24-bit where a glyph needs it, ClassDef 4 only for 16-bit classes.
#[test]
fn coverages_and_class_definitions_widen_past_16_bits() {
    let u24 = |v: &[u32]| v.iter().flat_map(|v| v.to_be_bytes()[1..].to_vec()).collect::<Vec<u8>>();
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    assert_eq!(build_coverage(&[1u32, 2, 3]), u16s(&[1, 3, 1, 2, 3]));
    assert_eq!(build_coverage(&[0xFFFFu32]), u16s(&[1, 1, 0xFFFF]));
    let two: Vec<u32> = (1..=10).chain(20..=29).collect();
    assert_eq!(build_coverage(&two), u16s(&[2, 2, 1, 10, 0, 20, 29, 10]));
    let list = [u16s(&[3]), u24(&[3, 70_000, 70_001, 70_002])].concat();
    assert_eq!(build_coverage(&[70_000u32, 70_001, 70_002]), list);
    let run: Vec<u32> = (70_000..70_010).collect();
    assert_eq!(build_coverage(&run), [u16s(&[4]), u24(&[1, 70_000, 70_009, 0])].concat());

    assert_eq!(build_classdef(&[(1u32, 1u32), (2, 1)]), u16s(&[1, 1, 2, 1, 1]));
    let mixed = [(70_000u32, 1u32), (70_001, 2), (70_002, 1)];
    assert_eq!(build_classdef(&mixed), [u16s(&[3]), u24(&[70_000, 3, 1, 2, 1])].concat());
    let wide: Vec<(u32, u32)> = (70_000..70_100).map(|g| (g, 1)).collect();
    assert_eq!(build_classdef(&wide), [u16s(&[4]), u24(&[1, 70_000, 70_099]), u16s(&[1])].concat());
    let runs: Vec<(u32, u32)> = (70_000..70_005).map(|g| (g, 1 + u32::from(g >= 70_003))).collect();
    assert_eq!(build_classdef(&runs), [u16s(&[4]), u24(&[2, 70_000, 70_002]), u16s(&[1]), u24(&[70_003, 70_004]),
        u16s(&[2])].concat());
    let spread = [(5u32, 1u32), (100, 1)];
    assert_eq!(build_classdef(&spread), u16s(&[2, 2, 5, 5, 1, 100, 100, 1]));
    assert_eq!(build_classdef(&[(5u32, 70_000u32)]), [u16s(&[3]), u24(&[5, 1, 70_000])].concat());
    let deep: Vec<(u32, u32)> = (5..15).map(|g| (g, 70_000)).collect();
    assert_eq!(build_classdef(&deep), [u16s(&[3]), u24(&[5, 10]), u24(&[70_000; 10])].concat());
}

// A GSUB 1.2's FeatureVariations, whose alternate ss01 holds parameters, reaches their end through the
// FeatureList at its 32-bit field: the tag that says how long they are is on that list's record.
#[test]
fn feature_variations_of_1_2_read_their_tags_from_its_feature_list() {
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let head = u16s(&[1, 2, 0, 0, 0, 0, 42, 0, 26, 0, 28, 0, 40]);
    let lists = [u16s(&[0, 1]), b"ss01".to_vec(), u16s(&[8, 0, 0, 0])].concat();
    let variations = u16s(&[1, 0, 0, 1, 0, 0, 0, 16, 1, 0, 1, 0, 0, 12, 4, 0, 0, 300]);
    let gsub = [head, lists, variations].concat();
    assert_eq!(gsub.len(), 78);
    assert_eq!(feature_variations(&gsub), Some(Some(42..78)));
}

// Classes at glyphs 0 and 65,535: the format 1 length, 65,536, overflows 16 bits.
#[test]
fn a_class_definition_can_span_every_glyph() {
    let entries = [(0u16, 1u16), (u16::MAX, 3)];
    let classes = crate::daecore::daetype::format::class_def::classes(&build_classdef(&entries));
    assert_eq!(classes, Some(entries.map(|(g, c)| (u32::from(g), u32::from(c))).to_vec()));
}

// GSUB's one lookup filters by set 1 of a GDEF holding only mark glyph sets, {5} and {6}. A subset of glyphs 0 to 4
// keeps neither, so GDEF goes, and the lookup ignores every mark as filtering by set 1 did.
#[test]
fn a_gdef_that_goes_for_keeping_nothing_leaves_its_emptied_sets_ignoring_marks() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = words(&[1, 0, 0, 0, 10, 1, 4, 1, 0x10, 1, 10, 1, 1, 6, 1, 1, 1, 1]);
    let gdef = words(&[1, 2, 0, 0, 0, 0, 14, 1, 2, 0, 12, 0, 18, 1, 1, 5, 1, 1, 6]);
    let tables: [(&str, &[u8]); 3] = [("GSUB", &gsub), ("GDEF", &gdef), ("JSTF", &[])];
    let table = |tag: &str| tables.iter().find(|t| t.0 == tag).map(|t| t.1);
    let active: GlyphSet = (0..5).collect();
    let [new_gdef, new_gsub, _] = subset_layout(table, &Reads::of(table), &active, &[0, 1, 2, 3, 4]);
    assert_eq!(new_gdef, None);
    let out = new_gsub.expect("a subset GSUB");
    let list = usize::from(read_u16_be(&out, 8).expect("a lookup list"));
    let at = list + usize::from(read_u16_be(&out, list + 2).expect("a lookup"));
    assert_eq!(read_u16_be(&out, at + 2), Some(0x08));
}
