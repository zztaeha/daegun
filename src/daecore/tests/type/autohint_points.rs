use std::collections::BTreeMap;

use daegun::daecore::daetype::hinting::auto::{AutoPoints, CollectPen, CONIC, CUBIC, ON_CURVE};
use daegun::daecore::daetype::outline::OutlinePen;
use daegun::daecore::daetype::TableBytes;

fn table_map_of(rel: &str) -> BTreeMap<String, TableBytes> {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes)
        .unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

fn loca_of(map: &BTreeMap<String, TableBytes>) -> Vec<usize> {
    let head = map.get("head").expect("fixture carries head");
    let maxp = map.get("maxp").expect("fixture carries maxp");
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(head, 50).expect("head carries indexToLocFormat");
    let n = daegun::daecore::daetype::decoder::read_u16_be(maxp, 4).expect("maxp carries numGlyphs") as usize;
    daegun::daecore::daetype::instancer::parse_loca(map, fmt, n).expect("loca parses")
}

fn collect_glyf(rel: &str, gid: u16) -> AutoPoints {
    let map = table_map_of(rel);
    let loca = loca_of(&map);
    let mut pen = CollectPen::new();
    daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut pen)
        .unwrap_or_else(|e| panic!("{rel} gid {gid}: {e:?}"));
    pen.finish()
}

fn collect_cff(rel: &str, gid: u16) -> AutoPoints {
    let map = table_map_of(rel);
    let cff = map.get("CFF ").expect("fixture carries CFF");
    let outlines = daegun::daecore::daetype::outline::CffOutlines::parse(cff).expect("CFF navigation parses");
    let mut pen = CollectPen::new();
    daegun::daecore::daetype::outline::outline_cff_glyph_with(&outlines, cff, gid, &mut pen)
        .unwrap_or_else(|e| panic!("{rel} gid {gid}: {e:?}"));
    pen.finish()
}

fn assert_well_formed(name: &str, p: &AutoPoints) {
    assert!(!p.is_empty(), "{name}: collected no points");
    assert_eq!(p.x.len(), p.y.len(), "{name}: x and y disagree in length");
    assert_eq!(p.x.len(), p.flags.len(), "{name}: flags disagree with coordinates");
    assert!(!p.contour_ends.is_empty(), "{name}: points but no contours");

    for (i, &f) in p.flags.iter().enumerate() {
        let kinds = (f & ON_CURVE != 0) as u32 + (f & CONIC != 0) as u32 + (f & CUBIC != 0) as u32;
        assert_eq!(kinds, 1, "{name}: point {i} has flags {f:#04x}, expected exactly one kind bit");
    }

    let mut expected_start = 0usize;
    for i in 0..p.contour_ends.len() {
        let (start, end) = p.contour(i).unwrap_or_else(|| panic!("{name}: contour {i} out of range"));
        assert_eq!(start, expected_start, "{name}: contour {i} does not start where {} ended", i.wrapping_sub(1));
        assert!(end > start, "{name}: contour {i} is empty");
        expected_start = end;
    }
    assert_eq!(expected_start, p.len(), "{name}: contours do not cover every point");
}

#[test]
fn glyf_outlines_collect_with_no_cubic_points() {
    let p = collect_glyf("scheherazade-new/ScheherazadeNew-Regular.ttf", 1583);
    assert_well_formed("scheherazade gid 1583", &p);
    assert!(
        p.flags.iter().all(|f| f & CUBIC == 0),
        "a glyf outline produced a cubic control point",
    );
    assert!(
        p.flags.iter().any(|f| f & CONIC != 0),
        "a 1683-point Arabic glyph with no quadratic control point is not being read as curves",
    );
}

#[test]
fn cff_outlines_collect_with_cubic_points() {
    let p = collect_cff("stix-two-math/STIX2Math.otf", 2257);
    assert_well_formed("stix gid 2257", &p);
    assert!(
        p.flags.iter().any(|f| f & CUBIC != 0),
        "a Type 2 charstring produced no cubic control point",
    );
}

#[test]
fn cff_point_count_is_the_charstrings_own() {
    let p = collect_cff("stix-two-math/STIX2Math.otf", 2257);
    assert_eq!(
        p.contour_ends.len(),
        211,
        "STIX2Math gid 2257 has 211 contours",
    );
    assert_eq!(
        p.len(),
        2628,
        "STIX2Math gid 2257 has 2628 points: \
         211 movetos + 12 lines + 872 curves x 3 = 2839, less the 211 coincident contour-start points",
    );
}

#[test]
fn a_closing_point_that_repeats_the_start_is_dropped() {
    let mut pen = CollectPen::new();
    pen.move_to(0.0, 0.0);
    pen.line_to(10.0, 0.0);
    pen.line_to(10.0, 10.0);
    pen.line_to(0.0, 0.0);
    pen.close();
    let p = pen.finish();
    assert_eq!(p.len(), 3, "the repeated start point should have been dropped");
    assert_eq!(p.contour_ends, vec![2]);
}

#[test]
fn an_unclosed_contour_still_lands() {
    let mut pen = CollectPen::new();
    pen.move_to(0.0, 0.0);
    pen.line_to(5.0, 5.0);
    let p = pen.finish();
    assert_eq!(p.len(), 2);
    assert_eq!(p.contour_ends, vec![1]);
}

#[test]
fn multiple_contours_tile_the_buffer() {
    let mut pen = CollectPen::new();
    pen.move_to(0.0, 0.0);
    pen.quad_to(1.0, 1.0, 2.0, 0.0);
    pen.close();
    pen.move_to(10.0, 10.0);
    pen.curve_to(11.0, 11.0, 12.0, 12.0, 13.0, 10.0);
    pen.close();
    let p = pen.finish();
    assert_well_formed("synthetic two-contour", &p);
    assert_eq!(p.contour_ends.len(), 2);
    assert_eq!(p.contour(0), Some((0, 3)));
    assert_eq!(p.contour(1), Some((3, 7)));
    assert_eq!(p.flags[1] & CONIC, CONIC, "quad_to's control must be conic");
    assert_eq!(p.flags[4] & CUBIC, CUBIC, "curve_to's controls must be cubic");
}

#[test]
fn horizontal_bars_never_collapse_to_nothing() {
    use daegun::daecore::daetype::hinting::auto::AutoHinter;

    let map = table_map_of("inter/InterVariable.ttf");
    let cmap = map.get("cmap").expect("Inter carries cmap").clone();
    let upm = daegun::daecore::daetype::decoder::read_u16_be(map.get("head").unwrap(), 18).unwrap();
    let loca = loca_of(&map);

    let pts_of = |gid: u16| {
        let mut pen = CollectPen::new();
        daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut pen).ok()?;
        let p = pen.finish();
        (!p.is_empty()).then_some(p)
    };
    let gid_of = |c: char| daegun::daecore::daetype::subsetter::cmap_glyph_id(&cmap, c as u32);

    let mut hinter = AutoHinter::new(upm, &mut |c| gid_of(c), &mut |g| pts_of(g)).expect("Inter yields zones");
    let e = pts_of(gid_of('E').expect("Inter maps E")).expect("E has an outline");

    for ppem in [8u16, 10, 12, 16, 24] {
        let out = hinter.hint(&e, ppem.into());
        let mut levels: Vec<i32> = out.y.clone();
        levels.sort_unstable();
        levels.dedup();
        assert!(
            levels.len() >= 4,
            "at {ppem}ppem E hinted to only {} distinct y levels; its three bars need six edges, \
             so this many means bars collapsed into each other",
            levels.len(),
        );
        let span = levels[levels.len() - 1] - levels[0];
        assert!(
            span >= 64 * 3,
            "at {ppem}ppem E spans only {span}/64 vertically, under three pixels",
        );
    }
}
