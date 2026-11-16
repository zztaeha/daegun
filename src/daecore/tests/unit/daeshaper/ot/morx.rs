use super::*;

// In place, each verb on spans of up to six glyphs gives what copying out the front, middle and back and joining
// them back, middle, front gives.
#[test]
fn rearranging_in_place_gives_each_verbs_permutation() {
    for &(l, r, rev_l, rev_r) in &VERBS {
        for n in l + r..=6 {
            let span: Vec<usize> = (0..n).collect();
            let mut front = span[..l].to_vec();
            let mut back = span[n - r..].to_vec();
            if rev_l {
                front.reverse();
            }
            if rev_r {
                back.reverse();
            }
            let middle = span[l..n - r].iter().copied();
            let expected: Vec<usize> = back.into_iter().chain(middle).chain(front).collect();
            let mut got = span.clone();
            rearrange(&mut got, (l, r, rev_l, rev_r));
            assert_eq!(got, expected, "verb ({l}, {r}, {rev_l}, {rev_r}) on {n} glyphs");
        }
    }
}

// A deleted glyph puts the sentinel in the set, so a later subtable whose class table starts on it still runs.
#[test]
fn a_deletion_puts_its_sentinel_in_the_set() {
    let mut buffer = Run::default();
    for (i, glyph) in [1, 2, 3, 4].into_iter().enumerate() {
        buffer.push(glyph, i as u32);
    }
    let mut cx = Context::new(&mut buffer, 8, None);
    assert!(!cx.reaches(&buffer, |g| g == DELETED_GLYPH));
    buffer.start_rewrite();
    cx.delete(&mut buffer);
    assert!(cx.reaches(&buffer, |g| g == DELETED_GLYPH));
    cx.finish(&mut buffer);
}

// A glyph goes by its deletion mark, as removal finds them, not by the sentinel's id.
#[test]
fn deleted_glyphs_go_by_their_mark() {
    let mut buffer = Run::default();
    for (i, glyph) in [1, 2, 3].into_iter().enumerate() {
        buffer.push(glyph, i as u32);
    }
    buffer.glyphs[0].id = DELETED_GLYPH;
    Context { classes: None, seen: None }.replace_at(&mut buffer, 1, DELETED_GLYPH);
    remove_deleted_glyphs(&mut buffer);
    assert!(buffer.glyphs[..buffer.len].iter().map(|info| info.id).eq([DELETED_GLYPH, 3]));
}

// 20,000 f's stack themselves, then each i stacks itself and acts, 19,000 actions running to a last one whose
// ligature isn't there. Each walk stops past the 64 components the ring holds, not 19,000 steps on: no glyph changes.
#[test]
fn a_ligature_action_walks_no_further_than_the_ring_holds() {
    let (n, actions) = (20_000usize, 19_000u32);
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let u32s = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let components = 80 + 4 * actions;
    let mut body = u32s(&[6, 28, 38, 62, 80, components, components + 8]);
    body.extend(u16s(&[8, 1, 2, 4, 5, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 1, 2]));
    body.extend(u16s(&[0, 0, 0, 0, 0x8000, 0, 0, 0x8000 | 0x2000, 0]));
    body.extend(u32s(&alloc::vec![0; actions as usize - 1]));
    body.extend(u32s(&[0x8000_0000, 0, 0]));
    let mut buffer = Run::default();
    (0..2 * n).for_each(|i| buffer.push(if i < n { 1 } else { 2 }, i as u32));
    let ranges = [RangeFlags { flags: 1, first: 0, last: u32::MAX }];
    let mut cx = Context::new(&mut buffer, 8, None);
    ligature(&body, &mut buffer, 8, None, &mut cx, Switches { ranges: &ranges, flags: 1 });
    cx.finish(&mut buffer);
    let ids = buffer.glyphs[..buffer.len].iter().map(|info| info.id);
    assert!(ids.eq((0..2 * n).map(|i| if i < n { 1 } else { 2 })));
}

// A morx chain and a kerx that count 2^24 subtables, of which one fits, each have the font's cache keep a slot for
// that one, not 128 MB of slots for those they count.
#[test]
fn aat_tables_are_cached_by_the_subtables_that_fit_not_those_they_count() {
    use super::super::kerx::{self, Kind};
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let u32s = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/carets.ttf");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("the fixture"));
    let cache = crate::daecore::daekit::cache::FontCache::new(tables.expect("its tables")).expect("a font");
    let face = Face::new(&cache);

    // One noncontextual subtable, f to l.
    let mut morx = [u16s(&[2, 0]), u32s(&[1, 1, 36, 0, 1 << 24, 20, 4, 1])].concat();
    morx.extend(u16s(&[8, 1, 1, 3]));
    let flags = [alloc::vec![RangeFlags { flags: 1, first: 0, last: u32::MAX }]];
    assert_eq!(plan_sets(&face, &morx, &flags).len(), 1);
    // One format 0 subtable, f before i by -40.
    let pair = u16s(&[1, 2, (-40i16).cast_unsigned()]);
    let kerx = [u16s(&[2, 0]), u32s(&[1 << 24, 34, 0, 0, 1, 6, 0, 0]), pair].concat();
    assert_eq!(kerx::plan_sets(&face, &kerx, Kind::Kerx, true).len(), 1);
    assert!(cache.held_bytes() < 1 << 20, "{} bytes held", cache.held_bytes());
}

// 2,000 morx chains of 16 bytes that each count every subtable there is, so each reads the rest of the table: the
// font is shaped without it, rather than with a plan holding a slot for each of the 1,999,000 subtables they yield.
#[test]
fn a_morx_whose_chains_read_each_other_has_no_plan() {
    use crate::daecore::daeshaper::run::Direction;
    let u32s = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/carets.ttf");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("the fixture"));
    let mut tables = tables.expect("its tables");
    let header = [2u16, 0].map(u16::to_be_bytes).concat();
    tables.insert("morx", [header, u32s(&[2_000]), u32s(&[16, 16, 0, u32::MAX]).repeat(2_000)].concat().into());
    let cache = crate::daecore::daekit::cache::FontCache::new(tables).expect("a font");
    let face = Face::new(&cache);
    let plan = cache.shape_plan_cached(None, &face, Direction::LeftToRight, None, None, &[], &[]);
    assert!(plan.aat.morx_sets.is_empty(), "the plan holds {} slots", plan.aat.morx_sets.len());
    assert!(face.aat(AatTable::Morx).is_none(), "the face reads it");
}
