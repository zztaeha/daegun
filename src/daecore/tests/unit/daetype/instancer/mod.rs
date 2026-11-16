use super::*;

// A simple glyph of 2 contours and 5 points, a composite of it, one of that and the simple glyph (2 deep), and one
// naming itself, which the walk passes over. A side bearing off its xMin clears head's flag.
#[test]
fn maxp_s_maxima_are_counted_through_nested_composites() {
    let composite = |parts: &[u8]| {
        let mut record = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 10, 0, 10];
        for (i, &gid) in parts.iter().enumerate() {
            record.extend([0, if i + 1 < parts.len() { 0x22 } else { 0x02 }, 0, gid, 0, 0]);
        }
        record
    };
    let simple = vec![0, 2, 0, 0, 0, 0, 0, 10, 0, 10, 0, 1, 0, 4, 0, 0];
    let (mut glyf, mut loca) = (Vec::new(), vec![0]);
    for record in [simple, composite(&[0]), composite(&[1, 0]), composite(&[3, 0])] {
        glyf.extend(record);
        loca.push(glyf.len());
    }
    let bounds = glyf_bounds(&glyf, &loca, 4);
    let counts = |lsb: u16| {
        let hmtx: Vec<u8> = [500, 0, 500, lsb, 500, 0, 500, 0].iter().flat_map(|v: &u16| v.to_be_bytes()).collect();
        let hmtx = MetricsTable::new(&hmtx, 4);
        GlyfCounts::of(bounds.iter().copied().enumerate(), &glyf, false, &loca, &hmtx).expect("counted")
    };
    let at_x_min = counts(0);
    assert_eq!((at_x_min.maxima, at_x_min.lsb_at_x_min), ([5, 2, 10, 4, 2, 2], true));
    assert!(!counts(3).lsb_at_x_min);
}

// Glyph 0 names a chain 101 deep of glyphs of 20,000 components each naming the next: past its visits the walk reads
// no component below the top, so each of 20 roots costs MAX_VISITS reads, not 101 × 20,000 more.
#[test]
fn a_glyph_s_count_stops_below_the_top_once_its_visits_are_spent() {
    use crate::daecore::daetype::format::glyf::MORE_COMPONENTS;
    let composite = |next: u16, count: usize| {
        let mut g = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for i in 0..count {
            let flags = if i + 1 < count { MORE_COMPONENTS } else { 0 };
            g.extend([flags.to_be_bytes(), next.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    let (mut glyf, mut loca) = (Vec::new(), vec![0]);
    for g in 0..=101u16 {
        glyf.extend(composite(g + 1, if g == 0 { 1 } else { 20_000 }));
        loca.push(glyf.len());
    }
    let roots = (0..20).map(|_| (0, Some([0i16; 4])));
    let counts = GlyfCounts::of(roots, &glyf, false, &loca, &MetricsTable::new(&[], 0)).expect("counted");
    assert_eq!(counts.maxima[4], 1);
}

// A described location moving post's underline keeps its header alone, as the whole instance writes it, and builds its
// name on first read, cut to the records read; one moving nothing in post shares the font's.
#[test]
fn a_described_location_keeps_what_is_read_of_post_and_name() {
    use crate::daecore::daetype::decoder::{extract_ttf_tables, parse_all_name_strings};
    let path = alloc::format!("{}/assets/test-fonts/inter/InterVariable.ttf", env!("CARGO_MANIFEST_DIR"));
    let map = extract_ttf_tables(&std::fs::read(path).expect("fixture")).expect("parses");
    let (bold, reads) = ([(AxisTag::new(b"wght"), 900.0)], Reads::of_tables(&map));
    let described = instance_tables_described(&map, &reads, &bold).expect("instances");
    let whole = instance_tables_from_map(&map, &reads, &bold).expect("instances");
    assert_eq!(described["post"][..], whole["post"][..32], "post's whole header");
    assert!(!described.contains_key("name"));
    let name = described_name(&map, &bold).expect("the location names itself");
    assert!(name.len() < whole["name"].len());
    let read = |table: &[u8]| {
        parse_all_name_strings::<u16>(&Tables::from([("name", TableBytes::from_vec(table.to_vec()))]))
    };
    assert_eq!(read(&name), read(&whole["name"]));
    let regular = instance_tables_described(&map, &reads, &[(AxisTag::new(b"wght"), 400.0)]).expect("instances");
    assert!(core::ptr::eq(&regular["post"][..], &map["post"][..]), "an unmoved post is the font's");
}

// Glyph 0 names composite 1 a thousand times, then a 5-point glyph; 1's thousand components name itself, read on
// each visit and never followed: each read spends a visit, so the walk runs out before the 5-point glyph.
#[test]
fn a_composite_s_components_spend_visits_where_they_are_not_followed() {
    use crate::daecore::daetype::format::glyf::MORE_COMPONENTS;
    let composite = |parts: &[u16]| {
        let mut g = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for (i, &gid) in parts.iter().enumerate() {
            let flags = if i + 1 < parts.len() { MORE_COMPONENTS } else { 0 };
            g.extend([flags.to_be_bytes(), gid.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    let simple = vec![0, 2, 0, 0, 0, 0, 0, 10, 0, 10, 0, 1, 0, 4, 0, 0];
    let (mut glyf, mut loca) = (Vec::new(), vec![0]);
    for record in [composite(&[[1; 1_000].as_slice(), &[2]].concat()), composite(&[1; 1_000]), simple] {
        glyf.extend(record);
        loca.push(glyf.len());
    }
    let counts = GlyfCounts::of([(0, Some([0i16; 4]))].into_iter(), &glyf, false, &loca, &MetricsTable::new(&[], 0));
    let counts = counts.expect("counted");
    assert_eq!((counts.maxima[2], counts.maxima[4]), (0, 1_001));
}

// A simple glyph of 65,536 points from 526 bytes of repeated flags, which loca names for every even glyph: at two
// such glyphs the points kept pass the budget over glyf's bytes, and varying the glyphs is refused.
#[test]
fn glyphs_sharing_their_bytes_past_the_points_budget_are_not_varied() {
    let glyph = [vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0], [0x39, 255].repeat(256)].concat();
    let varied = |glyphs: usize| {
        let loca: Vec<usize> = (0..=glyphs).map(|i| if i % 2 == 0 { 0 } else { glyph.len() }).collect();
        apply_gvar(&Tables::default(), &glyph, &loca, &[0.5]).err().map(|e| e.kind())
    };
    assert_eq!(varied(2), None);
    assert_eq!(varied(4), Some(crate::daecore::ErrorKind::LimitExceeded));
}

// A 1-point glyph, `depth` composites each naming the one before twice, and `tops` more naming the last once, every
// component with `flags`.
fn composite_dag(depth: u16, tops: usize, flags: u16) -> (Vec<u8>, Vec<usize>) {
    use crate::daecore::daetype::format::glyf::MORE_COMPONENTS;
    let composite = |parts: &[u16]| {
        let mut g = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for (i, &gid) in parts.iter().enumerate() {
            let more = if i + 1 < parts.len() { MORE_COMPONENTS } else { 0 };
            g.extend([(flags | more).to_be_bytes(), gid.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    let (mut glyf, mut loca) = (vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31], vec![0]);
    loca.push(glyf.len());
    for record in (1..=depth).map(|k| composite(&[k - 1, k - 1])).chain((0..tops).map(|_| composite(&[depth]))) {
        glyf.extend(record);
        loca.push(glyf.len());
    }
    (glyf, loca)
}

// Glyph `first`, a 60,000-point glyph of one contour, every point's flags repeated, and `tops` composites naming it,
// each walk of which places its 60,000 points.
pub(crate) fn heavy_composites(first: &[u8], tops: usize) -> (Vec<u8>, Vec<usize>) {
    let header = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0xEA, 0x5F, 0, 0];
    let (mut glyf, mut loca) = (first.to_vec(), vec![0, first.len()]);
    glyf.extend([&header[..], &[0x39, 255].repeat(234), &[0x39, 95]].concat());
    loca.push(glyf.len());
    for _ in 0..tops {
        glyf.extend([0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 1, 0, 0]);
        loca.push(glyf.len());
    }
    (glyf, loca)
}

// 24,000 composites of one 60,000-point glyph: each glyph's walks stay within its own, but together pass the
// instance's budget, so the outlines stay as stored. 1,000 vary.
#[test]
fn glyphs_walking_one_glyph_past_the_instance_s_budget_are_not_varied() {
    let varied = |tops| {
        let (glyf, loca) = heavy_composites(&[], tops);
        apply_gvar(&Tables::default(), &glyf, &loca, &[0.5]).err().map(|e| e.kind())
    };
    assert_eq!(varied(1_000), None);
    assert_eq!(varied(24_000), Some(crate::daecore::ErrorKind::LimitExceeded));
}

// Four composites over composites 16 deep, each naming the one before twice: each count stops at its own visits, but
// together they pass the font's, so head and maxp stay as stored. Three deep, they're counted.
#[test]
fn counts_past_the_font_s_visits_leave_head_and_maxp_as_stored() {
    use crate::daecore::daetype::format::glyf::ARGS_ARE_XY_VALUES;
    let deepest = |depth: u16| {
        let (glyf, loca) = composite_dag(depth, 4, ARGS_ARE_XY_VALUES);
        let tops = (usize::from(depth) + 1..loca.len() - 1).map(|gid| (gid, Some([0i16; 4])));
        GlyfCounts::of(tops, &glyf, false, &loca, &MetricsTable::new(&[], 0)).map(|counts| counts.maxima[5])
    };
    assert_eq!(deepest(3), Some(4));
    assert_eq!(deepest(16), None);
}

// A chain of 30,000 composites over a simple glyph, each component spending a visit as each glyph does: maxp's
// maxComponentDepth counts all 30,000.
#[test]
fn component_depth_is_counted_as_deep_as_composites_go() {
    use crate::daecore::daetype::format::glyf::ARGS_ARE_XY_VALUES;
    let (mut glyf, mut loca) = (Vec::new(), vec![0]);
    for next in 1..=30_000u16 {
        glyf.extend([0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
        glyf.extend([ARGS_ARE_XY_VALUES.to_be_bytes(), next.to_be_bytes(), [0, 0]].concat());
        loca.push(glyf.len());
    }
    glyf.extend([0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
    loca.push(glyf.len());
    let counts = GlyfCounts::of([(0, Some([0i16; 4]))].into_iter(), &glyf, false, &loca, &MetricsTable::new(&[], 0));
    assert_eq!(counts.expect("counted").maxima[5], 30_000);
}

// A one-glyph font whose VORG's one record is at glyph 65,535 and whose VVAR moves every origin alike: the instance
// writes that record alone, not the 65,535 glyphs before it, and a location holds an origin for its one glyph.
#[test]
fn a_vorg_record_far_past_the_glyphs_is_varied_alone() {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let store = [words(&[1, 0, 12, 1, 0, 22, 1, 1, 0, 0x4000, 0x4000, 1, 0, 1, 0]), vec![50]].concat();
    let vvar = [words(&[1, 0, 0, 24, 0, 0, 0, 0, 0, 0, 0, 55]), store, vec![0, 0, 0, 1, 0]].concat();
    let table = |t: Vec<u8>| TableBytes::from_vec(t);
    let map = Tables::from([
        ("VORG", table(words(&[1, 0, 880, 1, 0xFFFF, 100]))),
        ("VVAR", table(vvar)),
        ("maxp", table(words(&[0, 0x5000, 1]))),
    ]);
    let vorg = hvar::vary_vorg(&map, &[1.0]).expect("varied");
    assert_eq!(vorg.len(), 12);
    assert_eq!(vorg, words(&[1, 0, 930, 1, 0xFFFF, 150]));
    let spare = crate::daecore::daekit::base::sync::SpareSlots::default();
    let origins = hvar::VariedOrigins::new(&map, &[1.0], &spare).expect("varied");
    assert_eq!([0, 30_000, 65_535].map(|g| origins.origin(g)), [930, 930, 150]);
    assert!(origins.heap_bytes() < 1_024, "{} bytes", origins.heap_bytes());
}
