use super::*;
use crate::daecore::daetype::decoder::read_u32_be;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn all(n: u16) -> (GlyphSet, Vec<u32>) {
    let mut active = GlyphSet::new();
    (0..n).for_each(|g| { active.insert(g.into()); });
    (active, (0..n).map(u32::from).collect())
}

// Each of `n` mark glyph sets kept at its index.
fn sets(n: u16) -> Vec<Option<u16>> {
    (0..n).map(Some).collect()
}

fn devanagari_gsub() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/noto-devanagari/NotoSansDevanagari.ttf");
    let ttf = std::fs::read(path).expect("the fixture font");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&ttf).expect("its tables");
    tables.get("GSUB").expect("a GSUB").to_owned_vec()
}

// The same table at version 1.1, four header bytes longer for the FeatureVariations offset `fv`.
fn as_1_1(gsub: &[u8], fv: u32) -> Vec<u8> {
    let mut t = words(&[1, 1]);
    for at in [4, 6, 8] {
        t.extend((read_u16_be(gsub, at).expect("a header offset") + 4).to_be_bytes());
    }
    t.extend(fv.to_be_bytes());
    t.extend(&gsub[10..]);
    t
}

fn lookups(t: &[u8]) -> &[u8] {
    &t[usize::from(read_u16_be(t, 8).expect("a lookup list"))..]
}

// Version 1.1 with no FeatureVariations, as the instancer leaves a table once it has applied them,
// and a kilobyte after it: kept whole ahead of the lookups, the table would pass 64 KB and GSUB go.
#[test]
fn a_version_1_1_table_is_trimmed_as_1_0_is() {
    let gsub = devanagari_gsub();
    let (active, gid_map) = all(1117);
    let want = subset_gsub(&gsub, &active, &gid_map, &sets(10), false).expect("the 1.0 table subsets");
    let mut t = as_1_1(&gsub, 0);
    t.extend([0u8; 1024]);
    let got = subset_gsub(&t, &active, &gid_map, &sets(10), false).expect("the 1.1 table subsets");
    assert!(got.len() <= want.len() + 4, "{} bytes where 1.0 gives {}", got.len(), want.len());
    assert_eq!(lookups(&got), lookups(&want));
}

// FeatureVariations at the table's end, whose alternate for feature 0 runs lookup 0 on axis 0 from
// 0.5 to 1, move whole after the lookups, not after a copy of the whole table.
#[test]
fn feature_variations_move_whole_after_the_lookups() {
    let gsub = devanagari_gsub();
    let (active, gid_map) = all(1117);
    let want = subset_gsub(&gsub, &active, &gid_map, &sets(10), false).expect("the 1.0 table subsets");
    let fv = [
        words(&[1, 0, 0, 1, 0, 16, 0, 30]),
        words(&[1, 0, 6, 1, 0, 0x2000, 0x4000]),
        words(&[1, 0, 1, 0, 0, 12, 0, 1, 0]),
    ]
    .concat();
    let mut t = as_1_1(&gsub, gsub.len() as u32 + 4);
    t.extend(&fv);
    let got = subset_gsub(&t, &active, &gid_map, &sets(10), false).expect("the 1.1 table subsets");
    let at = read_u32_be(&got, 10).expect("a FeatureVariations offset") as usize;
    assert_eq!(read_u16_be(&got, 2), Some(1));
    assert_eq!(got.get(at..at + fv.len()), Some(&fv[..]));
    assert!(got.len() <= want.len() + 4 + fv.len(), "{} bytes where 1.0 gives {}", got.len(), want.len());
}

// A version 1.1 FeatureVariations whose lookup variations give feature 0 lookup 0 on axis 0 from 0.5 to 1, and
// lookup 1 everywhere by a null condition, moves whole with them after the lookups.
#[test]
fn feature_variations_move_whole_with_their_lookup_variations() {
    let gsub = devanagari_gsub();
    let (active, gid_map) = all(1117);
    let fv = [
        words(&[1, 1, 0, 1, 0, 26, 0, 40, 0, 1, 0, 0, 58]),
        words(&[1, 0, 6, 1, 0, 0x2000, 0x4000]),
        words(&[1, 0, 1, 0, 0, 12, 0, 1, 0]),
        words(&[1, 0, 1, 0, 2, 0, 34, 0, 26, 0, 0, 0, 30, 1, 0, 1, 1, 1, 0, 0x2000, 0x4000]),
    ]
    .concat();
    let mut t = as_1_1(&gsub, gsub.len() as u32 + 4);
    t.extend(&fv);
    let got = subset_gsub(&t, &active, &gid_map, &sets(10), false).expect("the 1.1 table subsets");
    let at = read_u32_be(&got, 10).expect("a FeatureVariations offset") as usize;
    assert_eq!(got.get(at..), Some(&fv[..]));
}

// The FeatureVariations of `feature_variations_move_whole_after_the_lookups`, its condition in
// `format`, ahead of a FeatureList and LookupList: a 1.1 table of 100 bytes.
fn variations_first(format: u16) -> (Vec<u8>, Vec<u8>) {
    let fv = [
        words(&[1, 0, 0, 1, 0, 16, 0, 30]),
        words(&[1, 0, 6, format, 0, 0x2000, 0x4000]),
        words(&[1, 0, 1, 0, 0, 12, 0, 1, 0]),
    ]
    .concat();
    let mut t = words(&[1, 1, 0, 62, 76, 0, 14]);
    t.extend(&fv);
    t.extend(words(&[1, 0x6C69, 0x6761, 8, 0, 1, 0]));
    t.extend(words(&[1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]));
    (t, fv)
}

// FeatureVariations ahead of the lookups are written once, where the header points.
#[test]
fn feature_variations_are_written_once() {
    let (t, fv) = variations_first(1);
    let (active, gid_map) = all(8);
    let out = subset_gsub(&t, &active, &gid_map, &[], false).expect("a subset GSUB");
    let at = read_u32_be(&out, 10).map_or(0, |at| at as usize);
    assert_eq!(out.get(at..at + fv.len()), Some(&fv[..]));
    assert_eq!(out.windows(fv.len()).filter(|w| *w == &fv[..]).count(), 1);
}

// A condition format past 1 may reach further than this reader knows, so the whole source stays
// ahead of the lookups, FeatureVariations and all it could reach in place.
#[test]
fn feature_variations_not_understood_keep_the_whole_source() {
    let (t, _) = variations_first(2);
    let (active, gid_map) = all(8);
    let out = subset_gsub(&t, &active, &gid_map, &[], false).expect("a subset GSUB");
    assert_eq!(read_u32_be(&out, 10), Some(14));
    assert_eq!(out.get(14..t.len()), Some(&t[14..]));
}

// ScriptList and FeatureList offsets of 0 are empty lists. Read as lists at offset 0, they would make
// the whole table stay ahead of the lookups, which no feature names, so they're held by number.
#[test]
fn null_lists_are_empty() {
    let mut t = words(&[1, 0, 0, 0, 10, 1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]);
    t.extend([0u8; 1024]);
    let (active, gid_map) = all(8);
    let out = subset_gsub(&t, &active, &gid_map, &[], true).expect("a subset GSUB");
    assert!(out.len() < 64, "{} bytes kept ahead of the lookups", read_u16_be(&out, 8).unwrap_or(0));
}

// A lookup no feature names, held by number, filtering on mark set 1 keeps it at its index in the subset's GDEF,
// 0 once set 0 goes. A set left empty ignores every mark, as filtering by it does; one GDEF lacks filters by none.
#[test]
fn a_mark_filtering_set_follows_the_gdef_written() {
    let t = words(&[1, 0, 0, 0, 10, 1, 4, 1, 0x10, 1, 10, 1, 1, 6, 1, 1, 1, 5]);
    let (active, gid_map) = all(8);
    let flag = |sets: &[Option<u16>]| {
        let out = subset_gsub(&t, &active, &gid_map, sets, true).expect("a subset GSUB");
        let list = usize::from(read_u16_be(&out, 8).expect("a lookup list"));
        let at = list + usize::from(read_u16_be(&out, list + 2).expect("a lookup"));
        let flag = read_u16_be(&out, at + 2).expect("a lookup flag");
        (flag, (flag & 0x10 != 0).then(|| read_u16_be(&out, at + 8)).flatten())
    };
    assert_eq!(flag(&[Some(0), Some(1)]), (0x10, Some(1)));
    assert_eq!(flag(&[None, Some(0)]), (0x10, Some(0)));
    assert_eq!(flag(&[Some(0), None]), (0x08, None));
    assert_eq!(flag(&[Some(0)]), (0, None));
}

// Coverage [5, 5, 7] with substitutes 10, 11, 12: once the repeat goes, 7 is the coverage's second
// glyph and keeps its own substitute, 12, not the second, 11.
#[test]
fn a_repeated_glyph_keeps_one_substitute() {
    let t = words(&[1, 16, 0, 0, 3, 10, 11, 12, 1, 3, 5, 5, 7]);
    let (active, gid_map) = all(16);
    let work = &mut Work::for_table(t.len());
    let out = subset_reverse_chain_single_subst(&t, 0, &active, &gid_map, work).expect("a subset subtable");
    let coverage = usize::from(read_u16_be(&out, 2).expect("a coverage offset"));
    assert_eq!(parse_coverage(&out, coverage), Ok(alloc::vec![5, 7]));
    assert_eq!([8, 10, 12].map(|at| read_u16_be(&out, at)), [Some(2), Some(10), Some(12)]);
}
