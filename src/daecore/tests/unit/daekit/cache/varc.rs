use super::*;

fn cache(name: &str) -> FontCache {
    let path = alloc::format!("{}/assets/test-fonts/varc/{name}", env!("CARGO_MANIFEST_DIR"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    FontCache::new(map.expect("parses")).expect("loads")
}

// Glyph 42's record fails, so it draws its own outline, but VARC still covers it, keeping it from the autohinter;
// the triangle, glyph 4, VARC doesn't cover.
#[test]
fn varc_covers_a_glyph_whose_record_fails() {
    let fc = cache("varc-hostile.ttf");
    assert!(fc.varc_covers(42) && !fc.varc_draws(42));
    assert!(!fc.varc_covers(4));
}

// A VARC glyph's leaves at a location borrow the thread's outline scratch, which keeps what they grew it to for the
// next draw; at the default they draw through the font's own, unless threads share it, where each draw is the thread's.
#[cfg(feature = "std")]
#[test]
fn varc_leaves_draw_through_the_threads_scratch() {
    use crate::daecore::daekit::cache::Scratch;
    let fc = cache("varc-hangul.ttf");
    drop(Scratch::take());
    let bold = fc.axis_key(&[("wght", 700.0)]);
    fc.at_location(&bold, |at| at.draw_varc(2, &mut 0, &mut Path::default()).expect("drawn"));
    let kept = Scratch::take();
    assert!(kept.x_coords.capacity() > 0 && kept.decoded.is_none(), "no leaf drew through the thread's scratch");
    fc.draw_varc(2, &mut 0, &mut Path::default()).expect("drawn");
    if !cfg!(feature = "threading") {
        assert_eq!(Scratch::take().x_coords.capacity(), 0, "the default drew through the thread's");
    }
}

// A glyf font whose glyph 3, drawn by VARC, loads glyph 1 four times: a composite of 1,000 components naming the empty
// glyph 2. Each load, beside its component's 32 and its own 32, reads the 1,000 components for its origin, then 1,001
// glyphs, all charged to the draw's budget though none draws a point.
#[test]
fn a_leaf_s_glyphs_read_are_charged_to_the_draw() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let mut composite = words(&[0xFFFF, 0, 0, 0, 0]);
    for k in 0..1_000 {
        composite.extend(words(&[if k < 999 { 0x22 } else { 0x02 }, 2, 0]));
    }
    let n = composite.len() as u32;
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let record = [0x00, 0x00, 0x01].repeat(4);
    let mut varc = [0x10000u32, 24, 0, 0, 0, 30].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    varc.extend(words(&[1, 1, 3]));
    varc.extend([&1u32.to_be_bytes()[..], &[1, 1, 1 + record.len() as u8], &record].concat());
    let tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> = [
        (*b"head", head),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 4]),
        (*b"loca", [0, 0, n, n, n].iter().flat_map(|v: &u32| v.to_be_bytes()).collect()),
        (*b"glyf", composite),
        (*b"VARC", varc),
    ]
    .into_iter()
    .map(|(tag, t)| (crate::Tag::new(&tag), t))
    .collect();
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&crate::raw::build_font(&tables).unwrap());
    let fc = FontCache::new(map.expect("parses")).expect("loads");
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, None, 3, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 4 * (32 + 32) + 4 * (1_000 + 1_001));
}

// The short CFF2 fixture's glyph 3 loads its triangle, glyph 1, once: one component and its load, then the triangle's
// charstring, `100 hmoveto 100 0 -100 100 rlineto`, a step for each number and operator.
#[test]
fn a_cff2_leaf_s_charstring_steps_are_charged_to_the_draw() {
    let fc = cache("varc-short-cff2.otf");
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, fc.varc_coords(), 3, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 32 + 32 + 7);
}

// A glyf font on gvar's two axes: leaf glyph 1, a triangle or empty, has `tuples` tuples each moving every point;
// VARC's glyph 3 loads glyph 4 `outer` times and glyph 4 loads glyph 1 `inner` times, each at its own location.
fn gvar_leaf(triangle: bool, tuples: u16, outer: u16, inner: u16) -> FontCache {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let longs = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let mut shape = words(&[1, 0, 0, 100, 100, 2, 0]);
    shape.extend([1, 1, 1]);
    shape.extend(words(&[0, 100, (-50i16) as u16, 0, 0, 100, 0]));
    shape.truncate(30);
    let s = shape.len() as u32;
    let (leaf, glyf) = if triangle { (s, [shape.clone(), shape].concat()) } else { (0, shape) };
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let n = if triangle { 7 } else { 4 };
    let moves = [&[n as u8 - 1][..], &vec![1; n]].concat().repeat(2);
    let mut data = words(&[tuples, 4 + 4 * tuples]);
    for _ in 0..tuples {
        data.extend(words(&[moves.len() as u16, 0]));
    }
    for _ in 0..tuples {
        data.extend(&moves);
    }
    let d = data.len() as u32;
    let mut gvar = longs(&[0x10000]);
    gvar.extend(words(&[2, 1]));
    gvar.extend(longs(&[44]));
    gvar.extend(words(&[5, 1]));
    gvar.extend(longs(&[48, 0, 0, d, d, d, d, 0x4000_0000]));
    gvar.extend(data);
    let component = |gid: u16, axis: u8, v: u16| [&[0x02, 0, gid as u8, axis, 0x40][..], &v.to_be_bytes()].concat();
    let r3: Vec<u8> = (0..outer).flat_map(|k| component(4, 1, 1 + k)).collect();
    let r4: Vec<u8> = (0..inner).flat_map(|j| component(1, 0, 1 + j)).collect();
    let mut varc = longs(&[0x10000, 24, 0, 0, 32, 44]);
    varc.extend(words(&[1, 2, 3, 4]));
    varc.extend([&longs(&[2])[..], &[1, 1, 3, 5, 0, 0, 0, 1]].concat());
    let offsets = longs(&[1, 1 + r3.len() as u32, 1 + (r3.len() + r4.len()) as u32]);
    varc.extend([&longs(&[2])[..], &[4], &offsets].concat());
    varc.extend([r3, r4].concat());
    let tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> = [
        (*b"head", head),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 5]),
        (*b"loca", longs(&[0, 0, leaf, leaf + s, leaf + s, leaf + s])),
        (*b"glyf", glyf),
        (*b"gvar", gvar),
        (*b"VARC", varc),
    ]
    .into_iter()
    .map(|(tag, t)| (crate::Tag::new(&tag), t))
    .collect();
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&crate::raw::build_font(&tables).unwrap());
    FontCache::new(map.expect("parses")).expect("loads")
}


// One load of a leaf of 4,095 gvar tuples is charged each tuple, an axis each of the two for its scalar, and, as each
// applies, its 16 bytes and 7 points: 4,095 times 26, past the two components' and their loads' 68 each (an axis value,
// two coordinates and an index beside them), its origin's decode of 4 and glyf's 5. A thousand loads draw too.
#[test]
fn a_leaf_s_gvar_tuples_are_charged_to_the_draw() {
    let fc = gvar_leaf(true, 4_095, 1, 1);
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, fc.varc_coords(), 3, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 2 * (32 + 1 + 2 + 1 + 32) + 4 + 5 + 4_095 * 26);
    let fc = gvar_leaf(true, 4_095, 1, 1_000);
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, fc.varc_coords(), 3, &mut Path::default()).is_some());
    assert_eq!(varc.leaves.get(), 1_000 * (4 + 5 + 4_095 * 26));
}

// The store fixture's glyph 2 loads its square, glyph 1, at wght 1.0: a component and its load, its axis value, index
// and one coordinate set, then the square's 18 steps, its two blends' one default each, and one region over one axis.
#[test]
fn a_cff2_leaf_s_blend_weighing_is_charged_to_the_draw() {
    let fc = cache("varc-cff2-store.otf");
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, fc.varc_coords(), 2, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 32 + 32 + 3 + 18 + 2 + 1);
}

// A glyf font whose VARC glyph 3 loads glyph 4, a VARC record loading glyph 1, the glyf glyph `leaf`, `loads` times;
// with `ivds`, an fvar of wght and an HVAR whose store lists that many null data sets.
fn leaves_loaded(leaf: Vec<u8>, loads: usize, ivds: Option<u16>) -> FontCache {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let longs = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let n = leaf.len() as u32;
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let (r3, r4) = ([0x00u8, 0x00, 0x04].to_vec(), [0x00u8, 0x00, 0x01].repeat(loads));
    let mut varc = longs(&[0x10000, 24, 0, 0, 0, 32]);
    varc.extend(words(&[1, 2, 3, 4]));
    let offsets = longs(&[1, 1 + r3.len() as u32, 1 + (r3.len() + r4.len()) as u32]);
    varc.extend([&longs(&[2])[..], &[4], &offsets, &r3, &r4].concat());
    let mut tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> = [
        (*b"head", head),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 5]),
        (*b"loca", longs(&[0, 0, n, n, n, n])),
        (*b"glyf", leaf),
        (*b"VARC", varc),
    ]
    .into_iter()
    .map(|(tag, t)| (crate::Tag::new(&tag), t))
    .collect();
    if let Some(ivds) = ivds {
        let mut fvar = words(&[1, 0, 16, 2, 1, 20, 0, 4]);
        fvar.extend(b"wght");
        fvar.extend(longs(&[100 << 16, 400 << 16, 900 << 16]));
        fvar.extend(words(&[0, 256]));
        let mut hvar = words(&[1, 0]);
        hvar.extend(longs(&[20, 0, 0, 0]));
        hvar.extend(words(&[1]));
        hvar.extend(longs(&[8 + 4 * u32::from(ivds)]));
        hvar.extend(words(&[ivds]));
        hvar.extend(vec![0u8; 4 * usize::from(ivds)]);
        hvar.extend(words(&[1, 0]));
        tables.extend([(crate::Tag::new(b"fvar"), fvar), (crate::Tag::new(b"HVAR"), hvar)]);
    }
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&crate::raw::build_font(&tables).unwrap());
    FontCache::new(map.expect("parses")).expect("loads")
}

// Glyph 1 is one contour of 65,535 points whose last x is cut off, and glyph 4 loads it 16 times: the first load is
// charged its visit, its contour and every point it read before failing, and no other load reads it again, beside
// the 17 components and their loads.
#[test]
fn a_failing_glyf_leaf_is_charged_what_it_read_and_loaded_once() {
    let mut leaf = [1u16, 0, 0, 0, 0, 0xFFFE, 0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    (0..255).for_each(|_| leaf.extend([0x38, 255]));
    leaf.extend([0x38, 252, 0x30, 0x02]);
    let fc = leaves_loaded(leaf.clone(), 16, None);
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, None, 3, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 17 * (32 + 32) + 1 + 1 + 65_535);
    // At a location the leaf's origin decodes it first, charged its contour and every point it read.
    let fc = leaves_loaded(leaf, 16, Some(0));
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, Some(&[1 << 14]), 3, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 17 * (32 + 32) + 1 + 65_535);
}

// Glyph 1 is a composite of 1,000 components, the first naming a glyph past the font: the first load is charged the
// 1,000 components its origin read and its two glyphs visited, and no other of the 16 reads it again, beside the 17
// components and their loads.
#[test]
fn a_failing_composite_leaf_is_charged_the_components_its_origin_read() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let mut leaf = words(&[0xFFFF, 0, 0, 0, 0, 0x22, 9, 0]);
    (1..1_000).for_each(|k| leaf.extend(words(&[if k < 999 { 0x22 } else { 0x02 }, 2, 0])));
    let fc = leaves_loaded(leaf, 16, None);
    let varc = fc.varc().expect("VARC loads");
    assert!(draw(&varc, &fc, None, 3, &mut Path::default()).is_some());
    assert_eq!(varc.work.get(), 17 * (32 + 32) + 1_000 + 2);
}

// At a described location each of 255 leaves varies by the font's HVAR and VVAR as kept once a font, rather than
// reading HVAR's store of 4,096 data sets again for each.
#[cfg(any(debug_assertions, daegun_counting))]
#[test]
fn a_described_location_s_leaves_vary_by_the_font_s_kept_advances() {
    let fc = leaves_loaded(Vec::new(), 255, Some(4_096));
    let key = fc.axis_key(&[("wght", 500.0)]);
    let (_, before, _) = crate::allocations::tally();
    assert!(fc.described_at(&key, |at| at.glyph_extents(3)).is_some());
    let (_, after, _) = crate::allocations::tally();
    assert!(after - before < 8 << 20, "{} bytes allocated", after - before);
}

// Glyphs 0 and 2 share one record (glyph 1's runs backward and doesn't read) of 1,000 components, each hidden by a
// condition walking 4,096 zero deltas, about 4.1 million decodes a glyph: one draws, but two run past the budget.
#[test]
fn glyphs_drawn_in_varc_s_place_share_one_budget() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let longs = |l: &[u32]| l.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let regions = 4_096u16;
    let mut store = [words(&[1]), longs(&[12]), words(&[1]), longs(&[16, 0]), vec![1], words(&[regions])].concat();
    store.resize(store.len() + 2 * usize::from(regions), 0);
    let row = vec![0xBF; usize::from(regions) / 64];
    store.extend([longs(&[store.len() as u32 - 12, 1]), vec![4], longs(&[1, 1 + row.len() as u32]), row].concat());
    let conditions = [longs(&[1, 8]), words(&[2, 0, 0, 0])].concat();
    let record = [0x80, 0x80, 0, 1, 0].repeat(1_000);
    let end = 1 + record.len() as u32;
    let records = [longs(&[3]), vec![4], longs(&[1, end, 1, end]), record].concat();
    let coverage = words(&[2, 1, 0, 2, 0]);
    let mut varc = longs(&[0x10000]);
    let mut at = 24;
    for part in [&coverage, &store, &conditions, &vec![], &records] {
        varc.extend(longs(&[if part.is_empty() { 0 } else { at as u32 }]));
        at += part.len();
    }
    varc.extend([coverage, store, conditions, records].concat());
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> = [
        (*b"head", head),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 3]),
        (*b"loca", vec![0; 8]),
        (*b"glyf", vec![]),
        (*b"VARC", varc),
    ]
    .into_iter()
    .map(|(tag, t)| (crate::Tag::new(&tag), t))
    .collect();
    let bytes = crate::raw::build_font(&tables).unwrap();
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let drawn = |gids: &[u32]| varc_drawn_from(&map, &[], gids).map(|d| d.into_keys().collect::<Vec<_>>());
    assert_eq!(drawn(&[0, 1]).expect("within the budget"), [0]);
    assert_eq!(drawn(&[0, 1, 2]).map_err(|e| e.kind()), Err(crate::daecore::ErrorKind::LimitExceeded));
}
