use std::collections::BTreeMap;

use daegun::shape::{Direction, ShapeOptions};
use daegun::{Font, GlyphClass, GlyphId, Location, Orientation, Tag};

use crate::located;

type Tables = BTreeMap<Tag, Vec<u8>>;

fn tables(rel: &str) -> Tables {
    let f = Font::from_vec(std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("the fixture"), 0)
        .expect("it parses");
    f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect()
}

fn with_count(mut t: Tables, count: u16) -> Font {
    t.get_mut(&Tag::new(b"maxp")).expect("maxp")[4..6].copy_from_slice(&count.to_be_bytes());
    Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses")
}

// carets.ttf (.notdef, f, i, l, f_f_i, f_i, f_f_l, with f and i forming f_i) with maxp counting 2 glyphs, hhea
// `long` long metrics, and its GSUB where `gsub` holds.
fn carets_counting_2(long: u16, gsub: bool) -> Font {
    let mut t = tables("test-fixtures/carets.ttf");
    t.get_mut(&Tag::new(b"hhea")).expect("hhea")[34..36].copy_from_slice(&long.to_be_bytes());
    if !gsub {
        t.remove(&Tag::new(b"GSUB"));
    }
    with_count(t, 2)
}

// A glyph past the count shapes as the cmap maps it and advances by hmtx up to the larger of the count and the long
// metrics, 0 past them; f_i, glyph 5, lies past both.
#[test]
fn shaping_keeps_a_glyph_past_the_count() {
    let shaped = |f: &Font, vertical: bool| {
        let o = ShapeOptions::default();
        let run =
            f.shape("fil", &Location::default(), &if vertical { o.with_direction(Direction::TopToBottom) } else { o });
        (run.glyphs.iter().map(|g| g.0).collect::<Vec<_>>(), run.advances.to_vec(), crate::xy(&run.offsets))
    };
    let (glyphs, advances, _) = shaped(&carets_counting_2(5, true), false);
    assert_eq!((glyphs, advances), (vec![5, 3], vec![0.0, 400.0]));
    let (glyphs, advances, _) = shaped(&carets_counting_2(5, false), false);
    assert_eq!((glyphs, advances), (vec![1, 2, 3], vec![400.0; 3]));
    let (glyphs, advances, _) = shaped(&carets_counting_2(1, true), false);
    assert_eq!((glyphs, advances), (vec![5, 3], vec![0.0, 0.0]));
    let vertical = shaped(&carets_counting_2(5, true), true);
    assert_eq!(vertical, (vec![1, 2, 3], vec![1000.0; 3], vec![(-200.0, -850.0), (-200.0, -800.0), (-200.0, -800.0)]));
    let (_, _, offsets) = shaped(&carets_counting_2(1, true), true);
    assert_eq!(offsets, [(-200.0, -850.0), (0.0, -800.0), (0.0, -800.0)]);
}

// A glyph past the count answers as the tables do, cmap, hmtx, the line's height down, the ascender up and GDEF, and
// has no outline past glyf's count.
#[test]
fn every_query_answers_a_glyph_past_the_count_as_its_tables_do() {
    let f = carets_counting_2(5, true);
    assert_eq!(f.glyph_count(), 2);
    assert_eq!(['f', 'i', 'l'].map(|c| f.glyph_id(c)), [Some(GlyphId(1)), Some(GlyphId(2)), Some(GlyphId(3))]);
    assert!(f.has_glyph('l') && f.coverage().any(|pair| pair == ('l', GlyphId(3))));
    assert_eq!(
        crate::advances(
            &f,
            &[GlyphId(2), GlyphId(3), GlyphId(4), GlyphId(5), GlyphId(6), GlyphId(7)],
            &Location::default()
        ),
        [400.0, 400.0, 900.0, 0.0, 0.0, 0.0]
    );
    assert!((2..=7).all(|g| f.advance(GlyphId(g), &Location::default(), Orientation::Vertical) == 1000.0));
    assert_eq!([2, 4, 7].map(|g| f.vertical_origin(GlyphId(g), &Location::default())), [800.0; 3]);
    assert_eq!(
        [2, 4, 7].map(|g| f.glyph_class(GlyphId(g))),
        [Some(GlyphClass::Base), Some(GlyphClass::Ligature), None]
    );
    assert_eq!(f.ligature_carets(GlyphId(6), &Location::default(), Orientation::Horizontal).len(), 3);
    assert_eq!(f.glyph_name(GlyphId(2)), Some("i"));
    assert_eq!(
        (4..=6).map(|g| f.glyph_name(GlyphId(g))).collect::<Vec<_>>(),
        ["f_f_i", "f_i", "f_f_l"].map(Some)
    );
    assert!(
        f.glyph_bounds(GlyphId(2), &Location::default()).is_none()
            && f.outline(GlyphId(2), &Location::default(), &mut daegun::outline::Path::default()).is_err()
    );
    assert_eq!(
        crate::advances(&carets_counting_2(1, true), &[GlyphId(1), GlyphId(2), GlyphId(3)], &Location::default()),
        [400.0, 0.0, 0.0]
    );
}

// maxp counts glyphs at 0.5 and 1.x, the versions the spec defines, a 1.0 cut to 20 bytes too, its count still in the
// table; else none, so f_i advances 0 and there's no subset: maxp at 2.0 or 0.6. A shorter maxp is refused at load.
#[test]
fn a_maxp_of_another_version_counts_no_glyphs() {
    let full = tables("test-fixtures/carets.ttf")[&Tag::new(b"maxp")].clone();
    let version = |major: u16, minor: u16| [&major.to_be_bytes()[..], &minor.to_be_bytes(), &full[4..]].concat();
    for (maxp, glyphs, advances) in [
        (full[..20].to_vec(), 7, [900.0, 400.0]),
        (version(2, 0), 0, [0.0, 400.0]),
        (version(0, 0x6000), 0, [0.0, 400.0]),
        (version(1, 0x5000), 7, [900.0, 400.0]),
        (version(0, 0x5000)[..6].to_vec(), 7, [900.0, 400.0]),
    ] {
        let mut t = tables("test-fixtures/carets.ttf");
        t.insert(Tag::new(b"maxp"), maxp.clone());
        let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
        let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
        let shaped = (f.glyph_count(), run.glyphs.iter().map(|g| g.0).collect::<Vec<_>>(), run.advances.to_vec());
        assert_eq!(shaped, (glyphs, vec![5, 3], advances.to_vec()), "maxp {:02x?}", &maxp[..4]);
        assert_eq!(f.subset_text("fil", &Location::default()).is_ok(), glyphs > 0, "maxp {:02x?}, subset", &maxp[..4]);
    }
}

// A subset keeps no glyph past the count, so its cmap maps neither i nor l.
#[test]
fn a_subset_drops_a_glyph_past_the_count() {
    let f = carets_counting_2(5, false);
    let sub = Font::from_vec(f.subset_text("fil", &Location::default()).expect("a subset").font, 0)
        .expect("the subset parses");
    assert_eq!(sub.glyph_count(), 2);
    let run = sub.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(
        (run.glyphs.to_vec(), run.advances.to_vec()),
        (super::aat_subset::glyph_ids(&[1, 0, 0]), vec![400.0; 3])
    );
    assert_eq!(
        f.glyph_closure([GlyphId(1), GlyphId(2), GlyphId(3)], &Location::default()).expect("a closure"),
        [GlyphId(0), GlyphId(1)]
    );
}

// At a location an advance past the count moves by HVAR's delta, or without HVAR is glyf's, which ends at the count:
// Inter counting 1,000 and TestGVAREight 2. Font units, 2,048 to the em.
#[test]
fn a_glyph_past_the_count_varies_as_its_tables_do() {
    let units = |f: &Font, text: &str, axes: &[(&str, f64)]| {
        (f.shape(text, &located(f, axes), &ShapeOptions::default()).advances[0]).round()
    };
    let inter = with_count(tables("inter/InterVariable.ttf"), 1000);
    assert_eq!(inter.glyph_id('z'), Some(GlyphId(1022)));
    assert_eq!([units(&inter, "z", &[]), units(&inter, "z", &[("wght", 900.0)])], [1131.0, 1208.0]);
    let gvar = with_count(tables("structure/TestGVAREight.ttf"), 2);
    assert_eq!([units(&gvar, "H", &[]), units(&gvar, "H", &[("HV", 1.0)])], [704.0, 0.0]);
}

// At a location a glyph past hmtx advances by HVAR's delta alone, at least 0, and one past the advance map by its last
// entry's: Inter with hmtx cut to 1,000 glyphs, and Inter itself.
#[test]
fn a_glyph_past_every_metric_advances_by_the_delta_alone() {
    let units = |f: &Font, axes: &[(&str, f64)]| {
        (f.shape("z", &located(f, axes), &ShapeOptions::default()).advances[0]).round()
    };
    let mut t = tables("inter/InterVariable.ttf");
    t.get_mut(&Tag::new(b"hhea")).expect("hhea")[34..36].copy_from_slice(&1000u16.to_be_bytes());
    t.get_mut(&Tag::new(b"hmtx")).expect("hmtx").truncate(4000);
    let cut = with_count(t, 1000);
    let at = [&[][..], &[("wght", 900.0)], &[("wght", 100.0)]];
    assert_eq!(at.map(|axes| units(&cut, axes)), [0.0, 77.0, 0.0]);
    let inter = Font::from_vec(std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("Inter"), 0)
        .expect("Inter parses");
    assert_eq!(
        crate::advances(&inter, &[GlyphId(2937), GlyphId(60000), GlyphId(65535)], &Location::default()),
        [0.0; 3]
    );
    assert_eq!(
        crate::advances(&inter, &[GlyphId(2937), GlyphId(60000), GlyphId(65535)], &located(&inter, &[("wght", 900.0)])),
        [64.0; 3]
    );
}

// At a location every VORG origin moves by VVAR's delta, a record past the count too, one past the map by its last
// entry's: Source Han Sans with a record for glyph 20,000.
#[test]
fn an_origin_past_the_count_varies_as_its_tables_do() {
    let mut t = tables("source-han-sans/SourceHanSansJP-VF.otf");
    let vorg = t.get_mut(&Tag::new(b"VORG")).expect("VORG");
    let count = u16::from_be_bytes([vorg[6], vorg[7]]);
    vorg[6..8].copy_from_slice(&(count + 1).to_be_bytes());
    vorg.extend_from_slice(&[0x4E, 0x20, 0x03, 0x84]);
    let vvar = t.get_mut(&Tag::new(b"VVAR")).expect("VVAR");
    let map = u32::from_be_bytes(vvar[20..24].try_into().expect("four bytes")) as usize;
    let entries = usize::from(u16::from_be_bytes([vvar[map + 2], vvar[map + 3]]));
    let size = usize::from(vvar[map + 1] >> 4 & 3) + 1;
    let varying = map + 4 + 480 * size;
    vvar.copy_within(varying..varying + size, map + 4 + (entries - 1) * size);
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    let origins =
        |axes: &[(&str, f64)]| [17942, 20000, 30000].map(|g| f.vertical_origin(GlyphId(g), &located(&f, axes)));
    assert_eq!(origins(&[]), [880.0, 900.0, 880.0]);
    assert_eq!(origins(&[("wght", 900.0)]), [890.0, 910.0, 890.0]);
}

// The same font with VORG at version 2, its glyphs and count in 24 bits and a record for glyph 70,000 at 900: that
// origin varies by the map's last entry too, and an instance writes VORG 2.
#[test]
fn a_version_2_origin_past_65_535_varies_as_its_tables_do() {
    let mut t = tables("source-han-sans/SourceHanSansJP-VF.otf");
    let vorg = t.get_mut(&Tag::new(b"VORG")).expect("VORG");
    let count = usize::from(u16::from_be_bytes([vorg[6], vorg[7]]));
    let mut v2 = [&[0, 2, 0, 0], &vorg[4..6], &(count as u32 + 1).to_be_bytes()[1..]].concat();
    for record in vorg[8..8 + 4 * count].chunks(4) {
        v2.extend([&[0][..], record].concat());
    }
    v2.extend([1, 0x11, 0x70, 0x03, 0x84]);
    *vorg = v2;
    let vvar = t.get_mut(&Tag::new(b"VVAR")).expect("VVAR");
    let map = u32::from_be_bytes(vvar[20..24].try_into().expect("four bytes")) as usize;
    let entries = usize::from(u16::from_be_bytes([vvar[map + 2], vvar[map + 3]]));
    let size = usize::from(vvar[map + 1] >> 4 & 3) + 1;
    let varying = map + 4 + 480 * size;
    vvar.copy_within(varying..varying + size, map + 4 + (entries - 1) * size);
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    let origins =
        |axes: &[(&str, f64)]| [17942, 20000, 30000, 70000].map(|g| f.vertical_origin(GlyphId(g), &located(&f, axes)));
    assert_eq!(origins(&[]), [880.0, 880.0, 880.0, 900.0]);
    assert_eq!(origins(&[("wght", 900.0)]), [890.0, 890.0, 890.0, 910.0]);
    let instance =
        Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("an instance")).expect("it parses");
    assert_eq!(instance.table(Tag::new(b"VORG")).map(|v| &v[..2]), Some(&[0, 2][..]));
    assert_eq!([17942, 70000].map(|g| instance.vertical_origin(GlyphId(g), &Location::default())), [890.0, 910.0]);
}

// maxp counting no glyphs, with head's loca format past 1, leaves the list drawing reads one entry, no glyph, and
// nothing to draw or measure, with no glyph before the first to look for.
#[test]
fn a_font_counting_no_glyphs_draws_nothing() {
    let mut t = tables("test-fixtures/carets.ttf");
    t.get_mut(&Tag::new(b"head")).expect("head")[50..52].copy_from_slice(&2u16.to_be_bytes());
    let f = with_count(t, 0);
    let at = Location::default();
    assert!(f.outline(GlyphId(0), &at, &mut daegun::outline::Path::default()).is_err());
    assert_eq!(f.glyph_bounds(GlyphId(0), &at), None);
}

// glyf holds the glyphs loca ends, whatever maxp counts, and one past them has no outline or extents, the ascender
// for an origin and at a location no advance. loca cut to 4 entries.
#[test]
fn a_glyph_past_loca_has_no_outline() {
    let mut t = tables("test-fixtures/carets.ttf");
    t.get_mut(&Tag::new(b"loca")).expect("loca").truncate(8);
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the cut font parses");
    let drawn = |g| {
        (
            f.outline(g, &Location::default(), &mut daegun::outline::Path::default()).is_ok(),
            f.glyph_bounds(g, &Location::default()).is_some(),
        )
    };
    let none = (false, false);
    assert_eq!([0, 1, 3, 4, 6].map(|g| drawn(GlyphId(g))), [(true, false), (true, true), none, none, none]);
    assert_eq!(
        [0, 1, 3, 4, 6].map(|g| f.vertical_origin(GlyphId(g), &Location::default())),
        [500.0, 850.0, 800.0, 800.0, 800.0]
    );
    let down = f.shape("fil", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert_eq!(crate::xy(&down.offsets), [(-200.0, -850.0), (-200.0, -850.0), (-200.0, -800.0)]);
    let mut t = tables("structure/TestGVAREight.ttf");
    t.get_mut(&Tag::new(b"loca")).expect("loca").truncate(8);
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the cut font parses");
    let units = |axes: &[(&str, f64)]| {
        f.shape("H ", &located(&f, axes), &ShapeOptions::default())
            .advances
            .iter()
            .map(|a| a.round())
            .collect::<Vec<_>>()
    };
    assert_eq!([units(&[]), units(&[("CK", -1.0)])], [[704.0, 330.0], [0.0, 0.0]]);
}

// fvar cut to its first `keep` axes, then given `extra`, with each instance's coordinates cut or given a 0 alike.
fn fvar_axes(fvar: &[u8], keep: usize, extra: Option<[u8; 20]>) -> Vec<u8> {
    let field = |at: usize| usize::from(u16::from_be_bytes([fvar[at], fvar[at + 1]]));
    let (start, count, instances, size) = (field(4), field(8), field(12), field(14));
    let axes = keep + usize::from(extra.is_some());
    let mut out = fvar[..start].to_vec();
    out[8..10].copy_from_slice(&(axes as u16).to_be_bytes());
    out[14..16].copy_from_slice(&((size - 4 * count + 4 * axes) as u16).to_be_bytes());
    out.extend_from_slice(&fvar[start..start + 20 * keep]);
    out.extend(extra.iter().flatten());
    for i in 0..instances {
        let at = start + 20 * count + size * i;
        out.extend_from_slice(&fvar[at..at + 4 + 4 * keep]);
        out.extend(extra.map(|_| [0; 4]).iter().flatten());
        out.extend_from_slice(&fvar[at + 4 + 4 * count..at + size]);
    }
    out
}

// Drawing reads gvar by its own glyph count and refuses other axes than fvar's or a major version past 1; advances
// read glyphs by maxp and tuples by gvar's axes, HVAR's past a major 2.
#[test]
fn gvar_is_drawn_by_its_own_glyph_count_and_fvars_axes() {
    let inter = tables("inter/InterVariable.ttf");
    let edited = |edit: &dyn Fn(&mut Tables)| {
        let mut t = inter.clone();
        edit(&mut t);
        Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses")
    };
    let at = |f: &Font, axes: &[(&str, f64)]| {
        (f.glyph_bounds(GlyphId(161), &located(f, axes)), crate::advances(f, &[GlyphId(161)], &located(f, axes))[0])
    };
    let (whole, bold, wide) = (edited(&|_| {}), [("wght", 900.0)], [("opsz", 32.0)]);
    let stored = at(&whole, &[]).0;
    assert_ne!(at(&whole, &bold).0, stored);
    let gvar = |at: usize, value: u16| move |t: &mut Tables| {
        t.get_mut(&Tag::new(b"gvar")).expect("gvar")[at..at + 2].copy_from_slice(&value.to_be_bytes());
    };
    assert_eq!(at(&edited(&gvar(12, 100)), &bold), (stored, at(&whole, &bold).1));
    let mut zzzz = [0; 20];
    zzzz[..4].copy_from_slice(b"ZZZZ");
    zzzz[12..14].copy_from_slice(&1u16.to_be_bytes());
    let more = edited(&|t| {
        *t.get_mut(&Tag::new(b"fvar")).expect("fvar") = fvar_axes(&inter[&Tag::new(b"fvar")], 2, Some(zzzz))
    });
    assert_eq!(at(&more, &bold), (stored, at(&whole, &bold).1));
    let fewer =
        edited(&|t| *t.get_mut(&Tag::new(b"fvar")).expect("fvar") = fvar_axes(&inter[&Tag::new(b"fvar")], 1, None));
    assert_eq!(at(&fewer, &wide), (stored, at(&whole, &wide).1));
    let none = edited(&gvar(0, 2));
    assert_eq!(at(&none, &bold), (stored, at(&whole, &bold).1));
}

// A CFF counts its glyphs by its CharStrings whatever maxp says, cff-subrs.otf's 21 drawing, named and boxed as where
// the counts agree, and one past them drawing nothing with no box. Only a subset refuses it.
#[test]
fn a_cff_counts_its_glyphs_by_its_charstrings_whatever_maxp_says() {
    let whole = with_count(tables("test-fixtures/cff-subrs.otf"), 21);
    let outline = |f: &Font, gid| {
        let mut path = daegun::outline::Path::default();
        f.outline(gid, &Location::default(), &mut path).map(|()| path)
    };
    assert!(outline(&whole, GlyphId(2)).is_ok() && whole.glyph_name(GlyphId(2)).is_some());
    for count in [10, 30] {
        let f = with_count(tables("test-fixtures/cff-subrs.otf"), count);
        for gid in [1, 2, 9, 15].map(GlyphId) {
            let at = (count, gid);
            assert_eq!(outline(&f, gid), outline(&whole, gid), "{at:?}");
            assert_eq!(f.glyph_name(gid), whole.glyph_name(gid), "{at:?}");
            let bounds = |f: &Font| f.glyph_bounds(gid, &Location::default());
            assert_eq!(bounds(&f), bounds(&whole), "{at:?}");
        }
        for gid in [1, 2, 9].map(GlyphId) {
            let origin = |f: &Font| f.vertical_origin(gid, &Location::default());
            assert_eq!(origin(&f), origin(&whole), "count {count}, {gid:?}");
        }
        assert!(outline(&f, GlyphId(21)).is_err(), "count {count}");
        assert_eq!(f.glyph_bounds(GlyphId(21), &Location::default()), None, "count {count}");
        assert!(f.subset_text("A", &Location::default()).is_err(), "count {count}");
    }
}

// A CFF2 so counted draws too, at any location, while HVAR still varies its advances: Source Han Sans's A, glyph 34,
// with maxp one short of its 17,944 CharStrings. Its subset still fails.
#[test]
fn a_cff2_whose_charstrings_count_differs_from_maxp_draws_and_varies() {
    let whole = daegun::raw::build_font(&tables("source-han-sans/SourceHanSansJP-VF.otf")).unwrap();
    let whole = Font::from_vec(whole, 0).expect("opens");
    let f = with_count(tables("source-han-sans/SourceHanSansJP-VF.otf"), 17943);
    for axes in [&[][..], &[("wght", 900.0)]] {
        assert_eq!(
            f.glyph_bounds(GlyphId(34), &located(&f, axes)),
            whole.glyph_bounds(GlyphId(34), &located(&whole, axes)),
            "{axes:?}"
        );
        assert_eq!(f.vertical_origin(GlyphId(34), &located(&f, axes)), 880.0, "{axes:?}");
    }
    assert!(f.glyph_bounds(GlyphId(34), &Location::default()).is_some());
    assert_eq!(
        [&[][..], &[("wght", 900.0)]].map(|axes| crate::advances(&f, &[GlyphId(34)], &located(&f, axes))),
        [[574.0], [660.0]]
    );
    assert!(
        f.subset_text("A", &Location::default()).is_err()
            && f.subset_text("A", &located(&f, &[("wght", 900.0)])).is_err()
    );
}
