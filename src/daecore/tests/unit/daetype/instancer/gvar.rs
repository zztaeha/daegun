use super::*;
use super::super::drawn::tests::gvar_of_tuples;
use crate::daecore::daetype::format::glyf::MORE_COMPONENTS;

// Glyph 1 names glyph 2 a thousand times by USE_MY_METRICS, and glyph 2's thousand components name glyph 3 plainly,
// read on every visit though none is followed: given 100,000 units of work, the walk spends them on those reads.
#[test]
fn a_phantom_walk_spends_work_on_components_it_does_not_follow() {
    let composite = |gid: u16, flags: u16| {
        let mut g = alloc::vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for i in 0..1_000 {
            let more = if i + 1 < 1_000 { MORE_COMPONENTS } else { 0 };
            g.extend([(flags | more).to_be_bytes(), gid.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    let (one, two) = (composite(2, USE_MY_METRICS), composite(3, 0));
    let glyf = [one.as_slice(), &two].concat();
    let loca = [0, 0, one.len(), glyf.len(), glyf.len()];
    let at = GlyfAt {
        source: None, glyf: &glyf, extended: false, loca: (&loca[..]).into(), horizontal: None, vertical: None,
        upem: 1000,
    };
    let mut reached = at.reached();
    let mut walk = Walk { at: &at, kept: &mut reached, work: 100_000, edges: 0 };
    assert_eq!(walk.phantoms(1, &mut 0), None);
}

// A chain of 60,000 composites, each naming the next by USE_MY_METRICS, over a simple glyph: the phantom walk reaches
// its points.
#[test]
fn a_phantom_walk_goes_as_deep_as_composites_go() {
    use crate::daecore::daetype::format::glyf::USE_MY_METRICS;
    let (mut glyf, mut loca) = (Vec::new(), alloc::vec![0]);
    for next in 1..=60_000u16 {
        glyf.extend([0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
        glyf.extend([USE_MY_METRICS.to_be_bytes(), next.to_be_bytes(), [0, 0]].concat());
        loca.push(glyf.len());
    }
    glyf.extend([0, 1, 0, 0, 0, 0, 0, 0, 0x01, 0xF4, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
    loca.push(glyf.len());
    let at = GlyfAt {
        source: None, glyf: &glyf, extended: false, loca: (&loca[..]).into(), horizontal: None, vertical: None,
        upem: 1000,
    };
    let mut reached = at.reached();
    let mut walk = Walk { at: &at, kept: &mut reached, work: 1 << 24, edges: 0 };
    assert!(walk.phantoms(0, &mut 0).is_some());
}

// A 60,000-point glyph, 40,000 composites of it and a glyph naming them all: reading the last's points walks each
// glyph it reaches whole, together past what one glyph may spend, so they read None; a single composite's read.
#[test]
fn a_glyph_s_points_share_one_glyph_s_spend_across_the_glyphs_they_reach() {
    use crate::daecore::daetype::format::glyf::ARGS_ARE_XY_VALUES;
    let composite = |parts: &[u16]| {
        let mut g = alloc::vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for (i, &gid) in parts.iter().enumerate() {
            let more = if i + 1 < parts.len() { MORE_COMPONENTS } else { 0 };
            g.extend([(ARGS_ARE_XY_VALUES | more).to_be_bytes(), gid.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    // One contour ending at point 59,999, every point's flags repeated.
    let header = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0xEA, 0x5F, 0, 0];
    let (mut glyf, mut loca) = ([&header[..], &[0x39, 255].repeat(234), &[0x39, 95]].concat(), alloc::vec![0]);
    loca.push(glyf.len());
    let parts: Vec<u16> = (1..=40_000).collect();
    for record in parts.iter().map(|_| composite(&[0])).chain([composite(&parts)]) {
        glyf.extend(record);
        loca.push(glyf.len());
    }
    let at = GlyfAt {
        source: None, glyf: &glyf, extended: false, loca: (&loca[..]).into(), horizontal: None, vertical: None,
        upem: 1000,
    };
    assert!(at.instanced(1).is_some(), "a composite of the 60,000-point glyph");
    assert!(at.instanced(40_001).is_none(), "40,000 composites of it");
}

// One contour of 256 points in 16 bytes, every point's flags one repeated.
const DENSE: [u8; 16] = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0x39, 255];

// The dense glyph, a 60,000-point glyph and 19,256 composites whose walks fill all but 1.1 million of the instance's
// budget: 4 tuples over the dense glyph fit beside them, while 4,095, 2.1 million of work, run past; alone, they fit.
#[test]
fn an_instance_s_tuples_share_its_walks_budget() {
    use super::super::tests::heavy_composites;
    let varied = |tops: usize, tuples: u16| {
        let (glyf, loca) = heavy_composites(&DENSE, tops);
        let map = Tables::from([("gvar", TableBytes::from_vec(gvar_of_tuples(tops + 2, tuples)))]);
        apply_gvar(&map, &glyf, &loca, &[1.0]).map(drop).map_err(|e| e.kind())
    };
    assert_eq!(varied(19_256, 4), Ok(()));
    assert_eq!(varied(19_256, 4_095), Err(crate::daecore::ErrorKind::LimitExceeded));
    assert_eq!(varied(0, 4_095), Ok(()));
}

// The dense glyph's walks and its 64 tuples share what a read of it may spend: 40,000 holds them, 30,000 doesn't, as
// GLYPH_SPEND holds one glyph's walks and tuples where its points or a measure of it are read.
#[test]
fn a_glyph_s_walks_and_tuples_share_its_read_s_budget() {
    let map = Tables::from([("gvar", TableBytes::from_vec(gvar_of_tuples(1, 64)))]);
    let gvar = super::super::drawn::Gvar::load(&map).expect("reads").expect("a gvar");
    let (mut coords, mut scalars) = (Vec::new(), Vec::new());
    let source = gvar_source(Some(&gvar), &[1.0], &mut coords, &mut scalars);
    let at = GlyfAt {
        source, glyf: &DENSE, extended: false, loca: (&[0, 16][..]).into(), horizontal: None, vertical: None,
        upem: 1000,
    };
    let read = |left: usize| {
        let mut reached = Reached { at: &at, variation: at.variation(left), own: BTreeMap::new(), points: Vec::new() };
        let (_, spent) = walk_glyph(&at, &mut reached, 0, &mut GlyphCoords::default());
        reached.variation.spend(spent).is_some()
    };
    assert!(read(40_000));
    assert!(!read(30_000));
}

// 400 glyphs, the even ones sharing the dense glyph and the odd ones none: reading the last even one's points decodes
// it alone, rather than each below it taking its 260 points again.
#[cfg(any(debug_assertions, daegun_counting))]
#[test]
fn a_glyph_s_points_decode_only_the_glyphs_it_reaches() {
    let loca: Vec<usize> = (0..=400).map(|i| if i % 2 == 0 { 0 } else { 16 }).collect();
    let at = GlyfAt {
        source: None, glyf: &DENSE, extended: false, loca: (&loca[..]).into(), horizontal: None, vertical: None,
        upem: 1000,
    };
    let (_, before, _) = crate::allocations::tally();
    assert!(at.instanced(398).is_some());
    let (_, after, _) = crate::allocations::tally();
    assert!(after - before < 64 << 10, "{} bytes allocated", after - before);
}

// A glyph of one point and 60,000 bytes of instructions, which its instance copies: three of their own are written,
// while 300 sharing its bytes, 18 MB to copy from 60 KB, are refused at glyf's subset budget.
#[test]
fn glyphs_sharing_their_bytes_copy_them_within_the_subset_budget() {
    let glyph = [&[0, 1][..], &[0; 8], &[0, 0], &60_000u16.to_be_bytes(), &[0; 60_000], &[0x31]].concat();
    let instanced = |glyf: &[u8], loca: &[usize]| {
        apply_gvar(&Tables::default(), glyf, loca, &[1.0]).map(|r| r.glyf_data.len()).map_err(|e| e.kind())
    };
    let own = glyph.repeat(3);
    assert!(instanced(&own, &[0, glyph.len(), 2 * glyph.len(), own.len()]).is_ok());
    let shared: Vec<usize> = (0..=600).map(|i| if i % 2 == 0 { 0 } else { glyph.len() }).collect();
    assert_eq!(instanced(&glyph, &shared), Err(crate::daecore::ErrorKind::LimitExceeded));
}
