use crate::Hint;
use std::collections::BTreeMap;

use daegun::outline::OutlinePen;
use daegun::shape::ShapeOptions;
use daegun::{Font, GlyphId, Location, NameId, Tag};

use super::FONTS;
use crate::located;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font"), 0).expect("it parses")
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// The font's tables after `edit`, as a font.
fn rebuilt(f: &Font, edit: impl FnOnce(&mut BTreeMap<Tag, Vec<u8>>)) -> Vec<u8> {
    let mut tables: BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    edit(&mut tables);
    daegun::raw::build_font(&tables).unwrap()
}

#[derive(Default)]
struct Record(Vec<String>);

impl OutlinePen for Record {
    fn move_to(&mut self, x: f32, y: f32) { self.0.push(format!("M{x},{y}")); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.push(format!("L{x},{y}")); }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("Q{x},{y}")); }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("C{x},{y}")); }
    fn close(&mut self) { self.0.push("Z".into()); }
}

fn outline(f: &Font, gid: u32) -> Option<Vec<String>> {
    let mut pen = Record::default();
    f.outline(GlyphId(gid), &Location::default(), &mut pen).ok().map(|()| pen.0)
}

fn reopen(out: &daegun::tables::Subset) -> Font {
    Font::from_vec(out.font.clone(), 0).expect("the subset parses")
}

#[test]
fn a_glyph_subset_maps_the_characters_it_keeps_and_carries_names() {
    for rel in ["inter/InterVariable.ttf", "stix-two-math/STIX2Math.otf"] {
        let f = font(rel);
        let a = f.glyph_id('A').expect("A");
        let out = f.subset([a], &Location::default()).expect("a subset");
        let sub = reopen(&out);
        assert_eq!(sub.glyph_id('A'), out.renumbered(a), "{rel}");
        assert_eq!(sub.glyph_id('Z'), None, "{rel}: Z maps to a glyph the subset dropped");
        assert_eq!(sub.family_name(&Location::default()), f.family_name(&Location::default()), "{rel}");
    }
}

// A text subset keeps each sequence the font gives for a selector and a base the text holds, wherever the
// selector stands.
#[test]
fn a_text_subset_keeps_the_variation_sequences_in_its_text() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    for (base, selector) in [('\u{4FAE}', '\u{FE00}'), ('\u{845B}', '\u{E0100}')] {
        let text: String = [base, selector].into_iter().collect();
        let want = f.variation_glyph_id(base, selector).expect("the source has the sequence");
        let out = f.subset_text(&text, &Location::default()).expect("a subset");
        let sub = reopen(&out);
        let got = sub.variation_glyph_id(base, selector);
        assert_eq!(got, out.renumbered(want), "{base:?} {selector:?}");
        assert_eq!(outline(&sub, got.expect("mapped").0), outline(&f, want.0));
    }
    let out = f.subset_text("\u{4FAE}\u{845B} \u{FE00}\u{E0100}", &Location::default()).expect("a subset");
    let sub = reopen(&out);
    let sequences =
        [('\u{4FAE}', '\u{FE00}'), ('\u{845B}', '\u{E0100}'), ('\u{4FAE}', '\u{E0100}'), ('\u{845B}', '\u{FE00}')];
    for (base, selector) in sequences {
        let want = f.variation_glyph_id(base, selector).and_then(|g| out.renumbered(g));
        assert_eq!(sub.variation_glyph_id(base, selector), want, "{base:?} {selector:?}");
    }
    assert!(sequences[..2].iter().all(|&(base, selector)| sub.variation_glyph_id(base, selector).is_some()));
}

#[test]
fn a_text_subset_of_thousands_of_ideographs_maps_each() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let text: String =
        (0x4E00..0xA000).filter_map(char::from_u32).filter(|&c| f.glyph_id(c).is_some()).take(8_300).collect();
    let out = f.subset_text(&text, &Location::default()).expect("8,300 ideographs subset");
    let sub = reopen(&out);
    for c in text.chars().step_by(97) {
        assert_eq!(sub.glyph_id(c), f.glyph_id(c).and_then(|g| out.renumbered(g)), "{c}");
    }
}

#[test]
fn a_subsets_cmap_records_are_sorted() {
    let f = font("stix-two-math/STIX2Math.otf");
    let sub = reopen(&f.subset_text("A\u{1D400}", &Location::default()).expect("a subset"));
    let cmap = sub.table(Tag::new(b"cmap")).expect("cmap");
    let records: Vec<(u16, u16)> = (0..usize::from(u16::from_be_bytes([cmap[2], cmap[3]])))
        .map(|i| {
            (
                u16::from_be_bytes([cmap[4 + 8 * i], cmap[5 + 8 * i]]),
                u16::from_be_bytes([cmap[6 + 8 * i], cmap[7 + 8 * i]]),
            )
        })
        .collect();
    assert!(records.is_sorted(), "{records:?}");
    assert!(sub.glyph_id('\u{1D400}').is_some(), "U+1D400 is unmapped");
}

#[test]
fn a_text_subset_keeps_the_instances_names_and_its_character_range() {
    let f = font("inter/InterVariable.ttf");
    let sub = reopen(&f.subset_text("Hamburg", &located(&f, &[("wght", 700.0)])).expect("a subset"));
    assert_eq!(sub.name(NameId::SUBFAMILY_NAME, &Location::default()).as_deref(), Some("Bold"));
    assert!(
        sub.name(NameId::POSTSCRIPT_NAME, &Location::default()).is_some_and(|ps| ps.ends_with("-Bold")),
        "{:?}",
        sub.name(NameId::POSTSCRIPT_NAME, &Location::default())
    );
    for id in [NameId::COPYRIGHT_NOTICE, NameId::VERSION_STRING] {
        assert!(
            sub.name(id, &Location::default()).is_some()
                && sub.name(id, &Location::default()) == f.name(id, &Location::default()),
            "name {id}"
        );
    }
    let os2 = sub.table(Tag::new(b"OS/2")).expect("OS/2");
    assert_eq!([&os2[64..66], &os2[66..68]], [&[0, 0x48][..], &[0, 0x75][..]], "first and last characters, H and u");
}

// OS/2's Unicode ranges keep only blocks a subset maps a character in: "Aé" Basic Latin and Latin-1, "Ж→" Cyrillic
// and Arrows; with none mapped none stays, and the first and last characters are 0xFFFF.
#[test]
fn a_subsets_unicode_ranges_keep_the_blocks_it_maps() {
    let f = font("inter/InterVariable.ttf");
    let fields = |out: daegun::tables::Subset| {
        let os2 = reopen(&out).table(Tag::new(b"OS/2")).expect("OS/2").to_vec();
        let ranges = [42, 46, 50, 54].map(|at| u32::from_be_bytes(os2[at..at + 4].try_into().expect("four bytes")));
        (ranges, [64, 66].map(|at| u16::from_be_bytes([os2[at], os2[at + 1]])))
    };
    assert_eq!(
        fields(f.subset_text("A\u{E9}", &Location::default()).expect("a subset")),
        ([0x3, 0, 0, 0], [0x41, 0xE9])
    );
    let cyrillic = f.subset_text("\u{416}\u{2192}", &Location::default()).expect("a subset");
    assert_eq!(fields(cyrillic), ([0x200, 0x20, 0, 0], [0x416, 0x2192]));
    assert_eq!(fields(f.subset([GlyphId(0)], &Location::default()).expect("a subset")), ([0; 4], [0xFFFF, 0xFFFF]));
}

#[test]
fn a_truetype_subset_keeps_its_glyph_free_tables() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let a = f.glyph_id('A').expect("A");
    for out in [f.subset([a], &Location::default()), f.subset_text("A", &Location::default())] {
        assert_eq!(reopen(&out.expect("a subset")).table(Tag::new(b"gasp")), f.table(Tag::new(b"gasp")));
    }
}

// The glyph's offset in glyf, from loca.
fn glyph_at(f: &Font, gid: u32) -> usize {
    let (head, loca) = (f.table(Tag::new(b"head")).expect("head"), f.table(Tag::new(b"loca")).expect("loca"));
    let g = gid as usize;
    if head[51] == 0 { 2 * usize::from(u16::from_be_bytes([loca[2 * g], loca[2 * g + 1]])) }
    else { u32::from_be_bytes([loca[4 * g], loca[4 * g + 1], loca[4 * g + 2], loca[4 * g + 3]]) as usize }
}

fn a_composite(f: &Font) -> (u32, usize) {
    let gid = f.glyph_id('Á').expect("Á");
    let at = glyph_at(f, gid.0);
    assert_eq!(&f.table(Tag::new(b"glyf")).expect("glyf")[at..at + 2], [0xFF, 0xFF], "Á is a composite");
    (gid.0, at)
}

// The spec reads any negative contour count as a composite, -1 being only the one to write.
#[test]
fn a_composite_of_any_negative_count_keeps_its_components() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let (gid, at) = a_composite(&f);
    let g = Font::from_vec(rebuilt(&f, |t| t.get_mut(&Tag::new(b"glyf")).expect("glyf")[at + 1] = 0xFE), 0)
        .expect("parses");
    let out = g.subset([GlyphId(gid)], &Location::default()).expect("a subset");
    let sub = reopen(&out);
    assert!(sub.glyph_count() > 2, "the components were dropped");
    assert_eq!(outline(&sub, out.renumbered(GlyphId(gid)).expect("kept").0), outline(&g, gid));
}

// A component past the font's glyphs fails the composite in the source; the subset fails it too
// rather than drawing .notdef in its place.
#[test]
fn a_component_past_the_font_stays_past_it() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let (gid, at) = a_composite(&f);
    let g = Font::from_vec(rebuilt(&f, |t| {
        t.get_mut(&Tag::new(b"glyf")).expect("glyf")[at + 12..at + 14].copy_from_slice(&65_000u16.to_be_bytes())
    }), 0)
    .expect("parses");
    assert_eq!(outline(&g, gid), None);
    let out = g.subset([GlyphId(gid)], &Location::default()).expect("a subset");
    assert_eq!(outline(&reopen(&out), out.renumbered(GlyphId(gid)).expect("kept").0), None);
}

// A last loca entry 8 bytes past glyf's end, as a cut after the last glyph's padding leaves it: the range leaves glyf,
// so the glyph holds nothing, drawn, subset and instanced, and hinting has nothing to hint.
#[test]
fn a_last_glyph_past_glyf_is_empty() {
    let overrun = |f: &Font| rebuilt(f, |t| {
        let (long, n) = (t[&Tag::new(b"head")][51] == 1, f.glyph_count() as usize);
        let loca = t.get_mut(&Tag::new(b"loca")).expect("loca");
        if long {
            let end = u32::from_be_bytes([loca[4 * n], loca[4 * n + 1], loca[4 * n + 2], loca[4 * n + 3]]) + 8;
            loca[4 * n..4 * n + 4].copy_from_slice(&end.to_be_bytes());
        } else {
            let end = u16::from_be_bytes([loca[2 * n], loca[2 * n + 1]]) + 4;
            loca[2 * n..2 * n + 2].copy_from_slice(&end.to_be_bytes());
        }
    });
    let hinted = font("test-fixtures/hinted.ttf");
    let cut = Font::from_vec(overrun(&hinted), 0).expect("parses");
    let last = hinted.glyph_count() - 1;
    let hint =
        |f: &Font| f.hinted(GlyphId(last), &Location::default(), 16.0, daegun::outline::HintMode::Font);
    assert!(hint(&hinted).is_some(), "the fixture's last glyph no longer hints");
    assert_eq!(outline(&cut, last), Some(Vec::new()), "drawn");
    assert!(hint(&cut).is_none(), "hinted");
    let out = cut.subset([GlyphId(last)], &Location::default()).expect("a subset");
    assert_eq!(outline(&reopen(&out), out.renumbered(GlyphId(last)).expect("kept").0), Some(Vec::new()), "subset");

    let inter = font("inter/InterVariable.ttf");
    let last = inter.glyph_count() - 1;
    let bold = |f: &Font| {
        outline(
            &Font::from_bytes(&f.instance(&located(f, &[("wght", 900.0)])).expect("instanced")).expect("an instance"),
            last,
        )
    };
    assert_ne!(bold(&inter), Some(Vec::new()), "the fixture's last glyph is empty at 900");
    assert_eq!(bold(&Font::from_vec(overrun(&inter), 0).expect("parses")), Some(Vec::new()), "instance");
}

// hinted.ttf's E (glyph 5), its loca range ending 2 bytes before it starts: a range running backward holds nothing, so
// E is empty, drawn and subset, and hinting has nothing to hint.
#[test]
fn a_backward_glyph_is_empty() {
    let hinted = font("test-fixtures/hinted.ttf");
    assert_eq!(hinted.table(Tag::new(b"head")).expect("head")[51], 0, "the fixture's loca is no longer short");
    let back = Font::from_vec(rebuilt(&hinted, |t| {
        let loca = t.get_mut(&Tag::new(b"loca")).expect("loca");
        let end = u16::from_be_bytes([loca[10], loca[11]]) - 1;
        loca[12..14].copy_from_slice(&end.to_be_bytes());
    }), 0)
    .expect("parses");
    let hint = |f: &Font| f.hinted(GlyphId(5), &Location::default(), 16.0, daegun::outline::HintMode::Font);
    assert!(hint(&hinted).is_some(), "the fixture's E no longer hints");
    assert_ne!(outline(&hinted, 5), Some(Vec::new()), "the fixture's E is empty");
    assert_eq!(outline(&back, 5), Some(Vec::new()), "drawn");
    assert!(hint(&back).is_none(), "hinted");
    let out = back.subset([GlyphId(5)], &Location::default()).expect("a subset");
    assert_eq!(outline(&reopen(&out), out.renumbered(GlyphId(5)).expect("kept").0), Some(Vec::new()), "subset");
}

// A collection of one face, `sfnt` moved past the collection's 16-byte header.
fn collection(sfnt: &[u8]) -> Vec<u8> {
    let mut face = sfnt.to_vec();
    for i in 0..usize::from(u16::from_be_bytes([face[4], face[5]])) {
        let at = 20 + 16 * i;
        let offset = u32::from_be_bytes(face[at..at + 4].try_into().expect("four")) + 16;
        face[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    [&b"ttcf"[..], &[0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 16], &face].concat()
}

// hinted.ttf's loca with one entry more than maxp counts glyphs, its glyph 6 now running past glyf's end, so it draws
// empty; glyph 7 is past maxp's count.
#[test]
fn a_glyph_past_glyf_before_loca_s_last_entry_draws_nothing() {
    let hinted = font("test-fixtures/hinted.ttf");
    assert_eq!(
        (hinted.table(Tag::new(b"head")).expect("head")[51], hinted.glyph_count()),
        (0, 7),
        "the fixture changed"
    );
    let long = Font::from_vec(rebuilt(&hinted, |t| {
        let loca = t.get_mut(&Tag::new(b"loca")).expect("loca");
        let end = u16::from_be_bytes([loca[14], loca[15]]) + 2;
        loca[14..16].copy_from_slice(&end.to_be_bytes());
        loca.extend_from_slice(&end.to_be_bytes());
    }), 0)
    .expect("parses");
    assert_eq!(outline(&long, 6), Some(Vec::new()), "glyph 6");
    assert_eq!(outline(&long, 5), outline(&hinted, 5), "glyph 5");
    assert_eq!(outline(&long, 7), None, "glyph 7");
}

// hinted.ttf's loca one entry short: loca is read at its own length, so glyph 6 has no end and draws nothing, in a
// font or collection, and nothing hints it; two short, glyph 5 has none either.
#[test]
fn a_short_loca_ends_its_glyphs_where_it_does() {
    let hinted = font("test-fixtures/hinted.ttf");
    let cut = |words: usize| rebuilt(&hinted, |t| t.get_mut(&Tag::new(b"loca")).expect("loca").truncate(2 * words));
    let short = Font::from_vec(cut(7), 0).expect("parses");
    let hint = |f: &Font| f.hinted(GlyphId(6), &Location::default(), 16.0, daegun::outline::HintMode::Font);
    assert!(hint(&hinted).is_some(), "the fixture's last glyph no longer hints");
    assert_eq!([outline(&short, 5), outline(&short, 6)], [outline(&hinted, 5), None], "drawn");
    assert!(hint(&short).is_none(), "hinted");
    let ttc = collection(&cut(7));
    assert_eq!(outline(&Font::from_collection(&ttc, 0).expect("parses"), 6), None, "in a collection");
    let shorter = Font::from_vec(cut(6), 0).expect("parses");
    assert_eq!([outline(&shorter, 5), outline(&shorter, 6)], [None, None], "two short");
}

// head's formats past those the spec defines: indexToLocFormat 2 or glyphDataFormat 2 leaves glyf unread, so no glyph
// draws, and origins are the ascender, 800 of 1024.
#[test]
fn a_head_of_an_unknown_format_holds_no_glyph() {
    let hinted = font("test-fixtures/hinted.ttf");
    let with = |at: usize| {
        let bytes =
            rebuilt(&hinted, |t| t.get_mut(&Tag::new(b"head")).expect("head")[at..at + 2].copy_from_slice(&[0, 2]));
        Font::from_vec(bytes, 0).expect("parses")
    };
    let drawn = |f: &Font| (0..7).map(|g| outline(f, g)).collect::<Vec<_>>();
    assert_eq!(drawn(&with(50)), vec![None; 7], "indexToLocFormat 2");
    let data = with(52);
    assert_eq!(drawn(&data), vec![None; 7], "glyphDataFormat 2");
    assert_eq!(data.vertical_origin(GlyphId(1), &Location::default()), 800.0, "glyphDataFormat 2's origin");
}

// numberOfHMetrics 0 leaves hmtx no metrics, so every glyph advances half an em at every location, which HVAR
// doesn't move: 1,024 units in Inter. Its subsets agree.
#[test]
fn no_long_metrics_advance_half_an_em_everywhere() {
    let f = font("inter/InterVariable.ttf");
    let zeroed = rebuilt(&f, |t| t.get_mut(&Tag::new(b"hhea")).expect("hhea")[34..36].copy_from_slice(&[0, 0]));
    let g = Font::from_vec(zeroed, 0).expect("parses");
    let gids = [GlyphId(0), GlyphId(10), GlyphId(50)];
    assert_ne!(
        crate::advances(&f, &gids, &Location::default())[0],
        1024.0,
        "the fixture's .notdef already advances half an em"
    );
    assert_eq!(
        [&[][..], &[("wght", 700.0)]].map(|axes| crate::advances(&g, &gids, &located(&g, axes))),
        [[1024.0; 3]; 2]
    );
    for axes in [&[][..], &[("wght", 700.0)]] {
        let out = g.subset(gids, &located(&g, axes)).expect("a subset");
        let kept: Vec<GlyphId> = gids.iter().map(|&gid| out.renumbered(gid).expect("kept")).collect();
        let subset = crate::advances(&reopen(&out), &kept, &Location::default());
        assert_eq!(subset, crate::advances(&g, &gids, &located(&g, axes)), "at {axes:?}");
    }
}

// A VORG claiming 45 more records than it holds, which no reader takes, so each glyph has one origin, from glyf or its
// box, in the font and in its subset.
#[test]
fn a_cut_vorg_is_read_alike_everywhere() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let vorg = f.table(Tag::new(b"VORG")).expect("VORG").to_vec();
    let count = usize::from(u16::from_be_bytes([vorg[6], vorg[7]]));
    let g =
        Font::from_vec(rebuilt(&f, |t| t.get_mut(&Tag::new(b"VORG")).expect("VORG").truncate(8 + 4 * (count - 45))), 0)
            .expect("parses");
    let gids: Vec<GlyphId> =
        (0..count).map(|i| GlyphId(u16::from_be_bytes([vorg[8 + 4 * i], vorg[9 + 4 * i]]).into())).collect();
    let out = g.subset(&gids, &Location::default()).expect("a subset");
    let sub = reopen(&out);
    for gid in gids {
        let origin = g.vertical_origin(gid, &Location::default());
        let kept = out.renumbered(gid).expect("kept");
        assert_eq!(sub.vertical_origin(kept, &Location::default()), origin, "glyph {gid}");
    }
}

// Ids keep their order in every subset, glyph or text, TrueType or CFF, compacted or not: rebuilt class arrays rely on
// it to hold no glyph the font's didn't.
#[test]
fn a_subset_keeps_its_glyphs_in_order() {
    for rel in [
        "inter/InterVariable.ttf",
        "stix-two-math/STIX2Math.otf",
        "source-serif/SourceSerif4Variable-Roman.otf",
    ] {
        let f = font(rel);
        let some = [900, 3, 40, 17, 2].map(GlyphId);
        let glyphs = f.subset(some, &Location::default()).expect("a glyph subset");
        let text = f.subset_text("Zebra 42, éclair → ok", &Location::default()).expect("a text subset");
        for out in [glyphs, text] {
            let kept: Vec<GlyphId> = (0..f.glyph_count()).filter_map(|g| out.renumbered(GlyphId(g))).collect();
            assert!(kept.windows(2).all(|w| w[0] < w[1]), "{rel}");
        }
    }
}

// A cmap of one Windows subtable.
fn one_subtable_cmap(encoding: u16, subtable: Vec<u8>) -> Vec<u8> {
    [u16s(&[0, 1, 3, encoding]), 12u32.to_be_bytes().to_vec(), subtable].concat()
}

// Each character's own glyph in `f`, read from a cmap holding nothing but (code, glyph).
fn mapped(f: &Font, text: &str) -> Vec<(char, GlyphId)> {
    text.chars().map(|c| (c, f.glyph_id(c).expect("the source maps it"))).collect()
}

// Each code holds its glyph, whatever the call or order, and a subset of the glyphs maps them all.
fn read_alike(g: &Font, text: &str, mappings: &[(char, GlyphId)]) {
    for &(c, gid) in mappings {
        assert_eq!(g.glyph_id(c), Some(gid), "{c:?} before shaping");
        assert!(g.has_glyph(c), "{c:?}");
    }
    assert_eq!(g.coverage().collect::<Vec<_>>(), mappings);
    let gids: Vec<GlyphId> = mappings.iter().map(|&(_, gid)| gid).collect();
    assert_eq!(g.shape(text, &Location::default(), &ShapeOptions::default()).glyphs.to_vec(), gids);
    for &(c, gid) in mappings {
        assert_eq!(g.glyph_id(c), Some(gid), "{c:?} after shaping");
    }
    let out = g.subset(&gids, &Location::default()).expect("a subset");
    let sub = reopen(&out);
    let kept: Vec<(char, GlyphId)> = mappings.iter().map(|&(c, gid)| (c, out.renumbered(gid).expect("kept"))).collect();
    assert_eq!(sub.coverage().collect::<Vec<_>>(), kept);
}

// A format 12 whose middle group runs backward: lookups, enumeration and a subset each pass over that group
// alone, so a and z stay mapped in all three.
#[test]
fn a_backward_cmap_group_costs_only_itself() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let mappings = mapped(&f, "az");
    let [(a, ga), (z, gz)] = [mappings[0], mappings[1]].map(|(c, g)| (u32::from(c), g.0));
    let groups = [(a, a, ga), (0x70, 0x60, 1), (z, z, gz)];
    let mut format12 = u16s(&[12, 0]);
    format12.extend((16 + 12 * groups.len() as u32).to_be_bytes());
    format12.extend([0u32, groups.len() as u32].iter().flat_map(|v| v.to_be_bytes()));
    format12.extend(groups.iter().flat_map(|&(start, end, gid)| [start, end, gid]).flat_map(u32::to_be_bytes));
    let g = Font::from_vec(rebuilt(&f, |t| {
        t.insert(Tag::new(b"cmap"), one_subtable_cmap(10, format12));
    }), 0)
    .expect("parses");
    read_alike(&g, "az", &mappings);
}

// A fallback font's cmap: one format 13 group from U+0020 to past Unicode's end. Every character to U+10FFFF
// is listed and kept, none past it.
#[test]
fn a_cmap_over_all_of_unicode_is_listed_and_kept_whole() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let gid = f.glyph_id('a').expect("a");
    let mut format13 = u16s(&[13, 0]);
    format13.extend([28, 0, 1, 0x20, u32::MAX, gid.0].iter().flat_map(|v: &u32| v.to_be_bytes()));
    let g = Font::from_vec(rebuilt(&f, |t| {
        t.insert(Tag::new(b"cmap"), one_subtable_cmap(10, format13));
    }), 0)
    .expect("parses");
    let every = |f: &Font, gid: GlyphId| {
        let coverage: Vec<_> = f.coverage().collect();
        assert_eq!(coverage.len(), 0x11_0000 - 0x20 - 0x800, "every character from U+0020, surrogates being none");
        assert!(coverage.iter().all(|&(_, g)| g == gid));
        assert_eq!(f.glyph_id(char::MAX), Some(gid));
        let _ = f.shape("a", &Location::default(), &ShapeOptions::default());
        assert_eq!(f.glyph_id(char::MAX), Some(gid), "once the index is built");
    };
    every(&g, gid);
    let out = g.subset([gid], &Location::default()).expect("a subset");
    every(&reopen(&out), out.renumbered(gid).expect("kept"));
}

// A format 4 listing 'z' before 'a'. The segment search lands on 'a', then on the final segment, so it maps 'a'
// alone, and so does every reader here.
#[test]
fn an_unsorted_cmap_is_read_alike_everywhere() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let mappings = mapped(&f, "az");
    let [a, z] = [mappings[0], mappings[1]].map(|(c, gid)| (u32::from(c) as u16, gid.0 as u16));
    let segments = [z, a, (0xFFFF, 0)];
    let n = segments.len() as u16;
    let format4 = [
        u16s(&[4, 16 + 8 * n, 0, 2 * n, 0, 0, 0]),
        u16s(&segments.map(|(code, _)| code)), u16s(&[0]),
        u16s(&segments.map(|(code, _)| code)),
        u16s(&segments.map(|(code, gid)| gid.wrapping_sub(code))),
        u16s(&[0; 3]),
    ].concat();
    let g = Font::from_vec(rebuilt(&f, |t| {
        t.insert(Tag::new(b"cmap"), one_subtable_cmap(1, format4));
    }), 0)
    .expect("parses");
    read_alike(&g, "a", &mappings[..1]);
    assert_eq!(
        g.shape("az", &Location::default(), &ShapeOptions::default()).glyphs.to_vec(),
        [mappings[0].1, GlyphId(0)]
    );
    assert_eq!(g.glyph_id('z'), None);
}

// A GDEF listing its class ranges out of order: far glyphs Base, then D Base, then O Mark. The public classes,
// shaping and a subset each read D and O as its search finds them.
#[test]
fn out_of_order_glyph_classes_are_read_alike_everywhere() {
    use daegun::GlyphClass::{Base, Mark};
    let f = font("eb-garamond/EBGaramond.ttf");
    let (d, o) = (f.glyph_id('D').expect("D"), f.glyph_id('O').expect("O"));
    assert!(d < o);
    let (d16, o16) = (d.0 as u16, o.0 as u16);
    let gdef = u16s(&[1, 0, 12, 0, 0, 12, 2, 3, o16 + 100, o16 + 110, 1, d16, d16, 1, o16, o16, 3]);
    let g = Font::from_vec(rebuilt(&f, |t| { t.insert(Tag::new(b"GDEF"), gdef); }), 0).expect("parses");
    assert_eq!([d, o, GlyphId(o.0 + 105)].map(|gid| g.glyph_class(gid)), [Some(Base), Some(Mark), None]);
    assert_eq!([d, o].map(|gid| g.mark_attachment_class(gid).map(|c| c.get())), [Some(1), Some(3)]);

    let shaped = g.shape("DO", &Location::default(), &ShapeOptions::default());
    assert_eq!(shaped.advances[1], 0.0, "O shapes as a mark");
    let out = g.subset([d, o], &Location::default()).expect("a subset");
    let sub = reopen(&out);
    let kept = [d, o].map(|gid| out.renumbered(gid).expect("kept"));
    assert_eq!(kept.map(|gid| sub.glyph_class(gid)), [Some(Base), Some(Mark)]);
    assert_eq!(sub.shape("DO", &Location::default(), &ShapeOptions::default()).advances, shaped.advances);
}

// One LigatureSet holding a CompCount-0 ligature to x before f + i to f_i. Shaping, the closure and the subsetter
// each skip the bad one alone, so a subset keeps f_i and still ligates.
#[test]
fn a_bad_ligature_costs_the_closure_only_itself() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let [gf, gi, gx] = ['f', 'i', 'x'].map(|c| f.glyph_id(c).expect("mapped"));
    let fi = f.shape("fi", &Location::default(), &ShapeOptions::default()).glyphs[0];
    let gsub = [
        u16s(&[1, 0, 10, 30, 44, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
        u16s(&[1]), b"liga".to_vec(), u16s(&[8, 0, 1, 0, 1, 4, 4, 0, 1, 8]),
        u16s(&[1, 24, 1, 8, 2, 6, 10, gx.0 as u16, 0, fi.0 as u16, 2, gi.0 as u16, 1, 1, gf.0 as u16]),
    ].concat();
    let g = Font::from_vec(rebuilt(&f, |t| { t.insert(Tag::new(b"GSUB"), gsub); }), 0).expect("parses");
    assert_eq!(g.shape("fi", &Location::default(), &ShapeOptions::default()).glyphs.to_vec(), [fi]);
    assert!(
        g.glyph_closure([gf, gi], &Location::default()).expect("a closure").contains(&fi),
        "f_i is out of the closure"
    );
    for out in [g.subset([gf, gi], &Location::default()), g.subset_text("fi", &Location::default())] {
        let out = out.expect("a subset");
        let shaped = reopen(&out).shape("fi", &Location::default(), &ShapeOptions::default()).glyphs.to_vec();
        assert_eq!(shaped, [out.renumbered(fi).expect("f_i kept")]);
    }
}

// A font keeps what it reads for the last location closed over. Moving between locations, one font answers each
// closure and subset as a font opened for it alone does.
#[test]
fn a_kept_font_closes_as_a_fresh_one_at_every_location() {
    for rel in ["inter/InterVariable.ttf", "source-serif/SourceSerif4Variable-Roman.otf"] {
        let kept = font(rel);
        let gids: Vec<GlyphId> = "office ffi".chars().filter_map(|c| kept.glyph_id(c)).collect();
        for axes in [&[][..], &[("wght", 900.0)], &[("wght", 300.0)], &[], &[("wght", 900.0)]] {
            let fresh = font(rel);
            let closure = |f: &Font| f.glyph_closure(&gids, &located(f, axes)).ok();
            assert_eq!(closure(&kept), closure(&fresh), "{rel} at {axes:?}");
            let bytes = |f: &Font| f.subset_text("office ffi", &located(f, axes)).expect("a subset").font;
            assert!(bytes(&kept) == bytes(&fresh), "{rel}: the subsets at {axes:?} differ");
        }
    }
}

#[test]
fn a_source_without_hhea_gets_no_horizontal_metrics() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let bytes = rebuilt(&f, |t| { t.remove(&Tag::new(b"hhea")); });
    let sub = reopen(&Font::from_vec(bytes, 0)
        .expect("parses").subset([GlyphId(36)], &Location::default()).expect("a subset"));
    let tags: Vec<_> = sub.table_tags().collect();
    assert!(!tags.contains(&Tag::new(b"hhea")) && !tags.contains(&Tag::new(b"hmtx")), "{tags:?}");
}

// A GSUB of one SingleSubst taking each of glyphs 1 to 100 to the next: a pass follows one step, and the closure runs
// until a pass adds nothing, to glyph 101 with the components of glyphs 1 to 101, EBGaramond's as daegun closes them.
#[test]
fn a_substitution_chain_closes_whole() {
    const N: u16 = 100;
    let coverage = 6 + 2 * N;
    let gsub = [
        u16s(&[1, 0, 10, 30, 44]),
        u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
        u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0]),
        u16s(&[1, 4, 1, 0, 1, 8]),
        u16s(&[2, coverage, N]), (2..=N + 1).flat_map(u16::to_be_bytes).collect(),
        u16s(&[1, N]), (1..=N).flat_map(u16::to_be_bytes).collect(),
    ].concat();
    let f = font("eb-garamond/EBGaramond.ttf");
    let g = Font::from_vec(rebuilt(&f, |t| { t.insert(Tag::new(b"GSUB"), gsub); }), 0).expect("parses");
    let components = [1625, 3090, 3092, 3093, 3094, 3095, 3097, 3098, 3099, 3100, 3101, 3102, 3111, 3112, 3125, 3185];
    let want: Vec<GlyphId> = (0..=101).chain(components).map(GlyphId).collect();
    assert_eq!(g.glyph_closure([GlyphId(1)], &Location::default()).expect("the chain closes"), want);
    assert!(g.subset([GlyphId(1)], &Location::default()).is_ok());
}

// 12,000 pairs, V A the last: past 10,920 a format 0 subtable's length wraps 16 bits, so the last subtable is
// read to the table's end rather than by its length, and a subset writes it so too.
#[test]
fn a_kern_subtable_past_64_kb_kerns_and_subsets_whole() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let (a, v) = (f.glyph_id('A').expect("A"), f.glyph_id('V').expect("V"));
    let n = f.glyph_count();
    let at = v.0 * n + a.0;
    assert!(at >= 11_999, "V A sorts too early to sit past 64 KB");
    let mut pairs: Vec<u8> = Vec::new();
    for p in at - 11_999..=at {
        pairs.extend(u16s(&[(p / n) as u16, (p % n) as u16]));
        pairs.extend((if p == at { -100i16 } else { 0 }).to_be_bytes());
    }
    let length = 14 + pairs.len();
    let kern = [u16s(&[0, 1, 0, length as u16, 0x0001, 12_000, 6 * 8192, 13, 6 * (12_000 - 8192)]), pairs].concat();
    let plain = Font::from_vec(rebuilt(&f, |t| {
        for tag in [b"GSUB", b"GPOS", b"GDEF", b"kern"] {
            t.remove(&Tag::new(tag));
        }
    }), 0)
    .expect("parses");
    let g = Font::from_vec(rebuilt(&plain, |t| { t.insert(Tag::new(b"kern"), kern); }), 0).expect("parses");
    let width = |f: &Font| f.shape("VA", &Location::default(), &ShapeOptions::default()).advances.iter().sum::<f64>();
    assert_eq!((width(&g) - width(&plain)).round(), -100.0, "the source's last pair was not read");
    let all: Vec<GlyphId> = (0..f.glyph_count()).map(GlyphId).collect();
    let sub = reopen(&g.subset(&all, &Location::default()).expect("a subset"));
    assert_eq!(width(&sub), width(&g));
}

// A subset carries no variations: a variable font subset at its default location is that instance, so GDEF names
// no variation data whose fvar is gone.
#[test]
fn a_variable_font_is_subset_at_its_default_instance() {
    let f = font("inter/InterVariable.ttf");
    let out = f.subset([GlyphId(0), GlyphId(36)], &Location::default()).expect("a subset");
    let sub = reopen(&out);
    assert!(sub.table(Tag::new(b"fvar")).is_none() && sub.table(Tag::new(b"gvar")).is_none());
    let gdef = sub.table(Tag::new(b"GDEF")).expect("GDEF");
    let store = (gdef.len() >= 18 && gdef[2..4] == [0, 3])
        .then(|| u32::from_be_bytes([gdef[14], gdef[15], gdef[16], gdef[17]]));
    assert!(store.is_none_or(|at| at == 0), "GDEF names a variation store at {store:?}");
    assert_eq!(outline(&sub, out.renumbered(GlyphId(36)).expect("kept").0), outline(&f, 36));
}

// PDF's Symbolic and Nonsymbolic: a font is Symbolic once it maps a character outside the standard Latin
// set, or is a Windows symbol font, and control codes don't count.
#[test]
fn flags_mark_a_font_symbolic_past_the_standard_latin_set() {
    use daegun::metrics::PdfFlags;
    let kind = |f: &Font| {
        let flags = f.pdf_flags(&Location::default());
        (flags.contains(PdfFlags::SYMBOLIC), flags.contains(PdfFlags::NONSYMBOLIC))
    };
    let f = font("eb-garamond/EBGaramond.ttf");
    let latin = reopen(&f.subset_text("Œuvre – fin\r\0", &Location::default()).expect("a subset"));
    assert!(latin.glyph_id('\r').is_some() && latin.glyph_id('\0').is_some(), "the subset dropped its control codes");
    assert_eq!(kind(&latin), (false, true));
    let cyrillic = reopen(&f.subset_text("Œuvre Ж", &Location::default()).expect("a subset"));
    assert_eq!(kind(&cyrillic), (true, false));
    let symbol = Font::from_vec(rebuilt(&latin, |t| {
        let cmap = t.get_mut(&Tag::new(b"cmap")).expect("cmap");
        for i in 0..usize::from(u16::from_be_bytes([cmap[2], cmap[3]])) {
            cmap[4 + 8 * i..8 + 8 * i].copy_from_slice(&u16s(&[3, 0]));
        }
    }), 0)
    .expect("parses");
    assert_eq!(kind(&symbol), (true, false));
    assert_eq!(kind(&font("inter/InterVariable.ttf")), (true, false));
}

// A glyf subset copies each glyph cut after its coordinates, padded with a 0 to an even length while all of them then
// fit short loca, 0x1FFFE bytes, else not padded, with loca's format to match.
#[test]
fn a_glyf_subset_pads_glyphs_to_2_bytes_while_short_loca_holds_them() {
    // One point and `n` instruction bytes, 17 + n long, then 3 bytes of padding.
    let glyph = |n: usize| [&u16s(&[1, 0, 0, 1, 1, 0, n as u16])[..], &vec![0; n], &[0x37, 1, 1, 0, 0, 0]].concat();
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..20].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5, 0, 0, 0x03, 0xE8]);
    head[51] = 1;
    let subset = |n: usize| {
        let (notdef, other) = (glyph(65_535), glyph(n));
        let ends = [0, notdef.len(), notdef.len() + other.len()];
        let tables = BTreeMap::from([
            (Tag::new(b"maxp"), u16s(&[0, 0x5000, 2])),
            (Tag::new(b"head"), head.clone()),
            (Tag::new(b"glyf"), [notdef, other].concat()),
            (Tag::new(b"loca"), ends.iter().flat_map(|&o| (o as u32).to_be_bytes()).collect()),
        ]);
        let f = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the face parses");
        let sub = reopen(&f.subset([GlyphId(0), GlyphId(1)], &Location::default()).expect("a subset"));
        let table = |tag| sub.table(tag).expect("written").to_vec();
        let glyf = table(Tag::new(b"glyf"));
        (glyf.len(), glyf.last().copied(), table(Tag::new(b"loca")), table(Tag::new(b"head"))[50..52].to_vec())
    };
    assert_eq!(subset(65_500), (0x1_FFFE, Some(0), u16s(&[0, 0x8008, 0xFFFF]), vec![0, 0]));
    let long: Vec<u8> = [0u32, 0x1_0010, 0x1_FFFF].iter().flat_map(|o| o.to_be_bytes()).collect();
    assert_eq!(subset(65_502), (0x1_FFFF, Some(1), long, vec![0, 1]));
}

// Overlapping glyphs are charged their source length before each is trimmed, bounding the trim's walk: even glyphs
// naming one 200 KB glyph whose flags overcount its points stop at the subset budget, though each trims to nothing.
#[test]
fn overlapping_glyphs_are_charged_before_they_are_trimmed() {
    const GLYF: u32 = 200_000;
    let mut glyph = [&u16s(&[1, 0, 0, 0, 0, 0xFFFE, 0])[..], &[0x08, 0xFF].repeat(256)].concat();
    glyph.resize(GLYF as usize, 0);
    let f = font("eb-garamond/EBGaramond.ttf");
    let g = Font::from_vec(rebuilt(&f, |t| {
        t.insert(Tag::new(b"glyf"), glyph);
        t.insert(Tag::new(b"loca"), (0..=f.glyph_count()).flat_map(|i| (GLYF * (i % 2)).to_be_bytes()).collect());
        t.get_mut(&Tag::new(b"head")).expect("head")[50..52].copy_from_slice(&[0, 1]);
    }), 0)
    .expect("parses");
    let gids: Vec<GlyphId> = (0..400).map(GlyphId).collect();
    assert!(g.subset(&gids, &Location::default()).is_err());
}
