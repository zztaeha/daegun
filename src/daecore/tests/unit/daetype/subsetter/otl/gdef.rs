use super::*;

// What an instance keeps of a GDEF its font doesn't read: no classes, and sets that cover every glyph.
#[test]
fn sets_of_every_glyph_read_as_sets_covering_every_glyph() {
    assert_eq!(sets_of_every_glyph(0, 8), None);
    let gdef = sets_of_every_glyph(3, 8).expect("a GDEF");
    assert_eq!(mark_glyph_set_count(&gdef), 3);
    let read = crate::daecore::daeshaper::ot::Gdef::new(&gdef).expect("it reads");
    assert!(!read.has_glyph_classes());
    assert!((0..3).all(|set| (0..8).all(|g| read.is_mark_glyph(g, set))));
}

// GDEF 1.2 of four mark glyph sets: {5}, a null offset, {7, 2} and {3}. A subset of glyphs 0 to 6 but 3 keeps
// sets 0 and 2, numbered 0 and 1, each covering its kept glyphs by their new IDs.
#[test]
fn mark_sets_keep_those_covering_a_kept_glyph() {
    let longs = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let sets = [words(&[1, 4]), longs(&[20, 0, 26, 34]), words(&[1, 1, 5, 1, 2, 7, 2, 1, 1, 3])].concat();
    let gdef = [words(&[1, 2, 0, 0, 0, 0, 14]), sets].concat();
    let mut active = GlyphSet::new();
    [0, 1, 2, 4, 5, 6].into_iter().for_each(|g| { active.insert(g); });
    let gid_map = [0, 1, 2, 0, 3, 4, 5, 0];
    let (map, coverages) = mark_sets(&gdef, &active, &gid_map);
    assert_eq!(map, [Some(0), None, Some(1), None]);
    assert_eq!(coverages, [words(&[1, 1, 4]), words(&[1, 1, 2])]);
    let out = subset_gdef(&gdef, &active, &gid_map).expect("a subset GDEF");
    assert_eq!(mark_glyph_set_count(&out), 2);
    let read = crate::daecore::daeshaper::ot::Gdef::new(&out).expect("it reads");
    assert_eq!([(4, 0), (5, 0), (2, 1), (4, 1)].map(|(g, set)| read.is_mark_glyph(g, set)), [true, false, true, false]);
}

// GDEF 1.2 whose MarkGlyphSets have an undefined format: the class definition stays without them,
// rather than the whole GDEF going.
#[test]
fn unreadable_mark_glyph_sets_leave_the_rest() {
    let gdef = [1u16, 2, 14, 0, 0, 0, 22, 1, 5, 1, 3, 2, 0].map(u16::to_be_bytes).concat();
    let mut active = GlyphSet::new();
    (0..8).for_each(|g| { active.insert(g); });
    let out = subset_gdef(&gdef, &active, &(0..8).collect::<Vec<u32>>()).expect("a subset GDEF");
    assert_eq!(glyph_class(&out, 5), 3);
    assert_eq!(mark_glyph_set_count(&out), 0);
}

// GDEF 1.4 naming glyph classes and ligature carets at 32-bit offsets, of glyphs 70,000 and 70,001, renumbered 1 and 2:
// 1.4 still, each through its 32-bit field. At the 16-bit field they're written as 1.0.
#[test]
fn parts_at_32_bit_offsets_keep_gdef_1_4() {
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let longs = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let classes = [words(&[3]), u24(70_000), u24(1), u24(3)].concat();
    let carets = [longs(&[10]), u24(1), u24(18), words(&[3]), u24(1), u24(70_001), words(&[1, 4, 1, 300])].concat();
    let lists = longs(&[0, 38, 0, 38 + classes.len() as u32, 0, 0]);
    let gdef = [words(&[1, 4, 0, 0, 0, 0, 0]), lists, classes, carets].concat();
    let mut active = GlyphSet::new();
    let mut gid_map = vec![0; 70_002];
    for (new, g) in [0u32, 70_000, 70_001].into_iter().enumerate() {
        active.insert(g);
        gid_map[g as usize] = new as u32;
    }
    let out = subset_gdef(&gdef, &active, &gid_map).expect("a subset GDEF");
    assert_eq!((&out[..4], &out[4..14]), (&words(&[1, 4])[..], &[0; 10][..]));
    assert_eq!(GdefHeader::read(&out).map(|h| h.long), Some([true, false, true, false, false]));
    assert_eq!(glyph_class(&out, 1), 3);
    let carets = crate::daecore::daetype::lig_caret::ligature_carets(&out, 2, || None, &[], false, || None);
    assert_eq!(carets, [Some(300.0)]);

    let gdef = [words(&[1, 4, 38, 0, 0, 0, 0]), longs(&[0; 6]), words(&[1, 1, 1, 3])].concat();
    let mut active = GlyphSet::new();
    [0, 1].into_iter().for_each(|g| { active.insert(g); });
    let out = subset_gdef(&gdef, &active, &[0, 1]).expect("a subset GDEF");
    assert_eq!((&out[..4], glyph_class(&out, 1)), (&words(&[1, 0])[..], 3));
}

// GDEF 1.4 whose 66 KB glyph classes sit at their 32-bit offset ahead of 70 KB of mark attachment classes at the
// 16-bit one: those go first, so their offset fits, though they're the larger.
#[test]
fn a_16_bit_part_goes_ahead_of_a_32_bit_one() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let classes = |n: u16| [words(&[1, 0, n]), (0..n).flat_map(|g| (1 + g % 2).to_be_bytes()).collect()].concat();
    let (glyph, attach) = (classes(33_000), classes(35_000));
    let longs = [0u32, 38 + attach.len() as u32, 0, 0, 0, 0].map(u32::to_be_bytes).concat();
    let gdef = [words(&[1, 4, 0, 0, 0, 38, 0]), longs, attach, glyph].concat();
    let mut active = GlyphSet::new();
    (0..35_000).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..35_000).collect();
    let out = subset_gdef(&gdef, &active, &gid_map).expect("a subset GDEF");
    assert_eq!((glyph_class(&out, 32_998), mark_attach_class(&out, 34_999)), (1, 2));
}

// A source GDEF that keeps its 80 KB class definition last, so its own offsets fit 16 bits. Rebuilt
// class definition first, the attach list behind it would sit past 64 KB.
#[test]
fn a_large_part_does_not_push_another_past_16_bit_offsets() {
    const N: u16 = 40_000;
    let attach = [10u16, 1, 6, 1, 0, 1, 1, 5].map(u16::to_be_bytes).concat();
    let mut gdef = [1u16, 0, 12 + attach.len() as u16, 12, 0, 0].map(u16::to_be_bytes).concat();
    gdef.extend(&attach);
    gdef.extend([1u16, 0, N].map(u16::to_be_bytes).concat());
    gdef.extend((0..N).flat_map(|g| (1 + g % 2).to_be_bytes()));
    let mut active = GlyphSet::new();
    (0..N).for_each(|g| { active.insert(g.into()); });
    let gid_map: Vec<u32> = (0..N).map(u32::from).collect();

    let out = subset_gdef(&gdef, &active, &gid_map).expect("a subset GDEF");
    let mut work = generic::Work::for_table(gdef.len());
    let kept = generic::Kept::Subset(&active, &gid_map);
    let want =
        generic::subset_subtable(&gdef, 12, &schemas::gdef::ATTACH_LIST, kept, generic::Devices::Keep, &mut work);
    let want = want.expect("the attach list subsets");
    let attach_at = usize::from(read_u16_be(&out, 6).expect("an attach list offset"));
    assert_eq!(out.get(attach_at..attach_at + want.len()), Some(&want[..]), "the attach list offset is wrong");
    assert_eq!(glyph_class(&out, 7), 2, "the class definition moved");
}

// GDEF 1.2 of 1,000 mark glyph sets all naming one coverage of glyphs 0 to 65,534, glyphs 0 and 1 kept: it's read
// once, within the work a 4 KB table gives. A set naming a coverage of 2^24 glyphs is past that work: none is read.
#[test]
fn a_coverage_many_mark_sets_name_is_read_once_within_the_budget() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gdef_of = |count: u16, coverage: &[u8]| {
        let offsets = (0..count).flat_map(|_| (4 + 4 * u32::from(count)).to_be_bytes());
        [words(&[1, 2, 0, 0, 0, 0, 14, 1, count]), offsets.collect(), coverage.to_vec()].concat()
    };
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let gdef = gdef_of(1_000, &words(&[2, 1, 0, 0xFFFE, 0]));
    let (map, coverages) = mark_sets(&gdef, &active, &[0, 1]);
    assert!(map.iter().zip(0..).all(|(&set, i)| set == Some(i)) && map.len() == 1_000, "the sets were dropped");
    assert!(coverages.iter().all(|c| *c == words(&[1, 2, 0, 1])) && coverages.len() == 1_000);
    let wide = [words(&[4, 0]), alloc::vec![1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0, 0, 0]].concat();
    assert_eq!(mark_sets(&gdef_of(1, &wide), &active, &[0, 1]), (Vec::new(), Vec::new()));
}
