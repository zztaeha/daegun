use std::collections::BTreeMap;

use daegun::{Font, OutlinePen};

use super::FONTS;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font")).expect("it parses")
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// The font's tables after `edit`, as a font.
fn rebuilt(f: &Font, edit: impl FnOnce(&mut BTreeMap<String, Vec<u8>>)) -> Vec<u8> {
    let mut tables: BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    edit(&mut tables);
    daegun::build_font(&tables)
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

fn outline(f: &Font, gid: u16) -> Option<Vec<String>> {
    let mut pen = Record::default();
    f.outline_glyph(gid, &mut pen).map(|_| pen.0)
}

fn reopen(out: &daegun::SubsetResult) -> Font {
    Font::from_vec(out.ttf.clone()).expect("the subset parses")
}

#[test]
fn a_glyph_subset_maps_the_characters_it_keeps_and_carries_names() {
    for rel in ["inter/InterVariable.ttf", "stix-two-math/STIX2Math.otf"] {
        let f = font(rel);
        let a = f.glyph_id('A' as u32).expect("A");
        let out = f.subset(&[a], &[]).expect("a subset");
        let sub = reopen(&out);
        assert_eq!(sub.glyph_id('A' as u32), out.new_gid(a), "{rel}");
        assert_eq!(sub.glyph_id('Z' as u32), None, "{rel}: Z maps to a glyph the subset dropped");
        assert_eq!(sub.family_name(), f.family_name(), "{rel}");
    }
}

#[test]
fn a_text_subset_keeps_the_variation_sequences_in_its_text() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    for (base, selector) in [(0x4FAE, 0xFE00), (0x845B, 0xE0100)] {
        let text: String = [base, selector].into_iter().filter_map(char::from_u32).collect();
        let want = f.variation_glyph_id(base, selector).expect("the source has the sequence");
        let out = f.subset_text(&text, &[]).expect("a subset");
        let sub = reopen(&out);
        let got = sub.variation_glyph_id(base, selector);
        assert_eq!(got, out.new_gid(want), "U+{base:X} U+{selector:X}");
        assert_eq!(outline(&sub, got.expect("mapped")), outline(&f, want));
    }
}

#[test]
fn a_text_subset_of_thousands_of_ideographs_maps_each() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let text: String = (0x4E00..0xA000).filter_map(char::from_u32).filter(|&c| f.glyph_id(c as u32).is_some()).take(8_300).collect();
    let out = f.subset_text(&text, &[]).expect("8,300 ideographs subset");
    let sub = reopen(&out);
    for c in text.chars().step_by(97) {
        assert_eq!(sub.glyph_id(c as u32), f.glyph_id(c as u32).and_then(|g| out.new_gid(g)), "{c}");
    }
}

#[test]
fn a_subsets_cmap_records_are_sorted() {
    let f = font("stix-two-math/STIX2Math.otf");
    let sub = reopen(&f.subset_text("A\u{1D400}", &[]).expect("a subset"));
    let cmap = sub.table("cmap").expect("cmap");
    let records: Vec<(u16, u16)> = (0..usize::from(u16::from_be_bytes([cmap[2], cmap[3]])))
        .map(|i| (u16::from_be_bytes([cmap[4 + 8 * i], cmap[5 + 8 * i]]), u16::from_be_bytes([cmap[6 + 8 * i], cmap[7 + 8 * i]])))
        .collect();
    assert!(records.is_sorted(), "{records:?}");
    assert!(sub.glyph_id(0x1D400).is_some(), "U+1D400 is unmapped");
}

#[test]
fn a_text_subset_keeps_the_instances_names_and_its_character_range() {
    let f = font("inter/InterVariable.ttf");
    let sub = reopen(&f.subset_text("Hamburg", &[("wght", 700.0)]).expect("a subset"));
    assert_eq!(sub.name_string(2).as_deref(), Some("Bold"));
    assert!(sub.name_string(6).is_some_and(|ps| ps.ends_with("-Bold")), "{:?}", sub.name_string(6));
    for id in [0, 5] {
        assert!(sub.name_string(id).is_some() && sub.name_string(id) == f.name_string(id), "name {id}");
    }
    let os2 = sub.table("OS/2").expect("OS/2");
    assert_eq!([&os2[64..66], &os2[66..68]], [&[0, 0x48][..], &[0, 0x75][..]], "first and last characters, H and u");
}

#[test]
fn a_truetype_subset_keeps_its_glyph_free_tables() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let a = f.glyph_id('A' as u32).expect("A");
    for out in [f.subset(&[a], &[]), f.subset_text("A", &[])] {
        assert_eq!(reopen(&out.expect("a subset")).table("gasp"), f.table("gasp"));
    }
}

// The glyph's offset in glyf, from loca.
fn glyph_at(f: &Font, gid: u16) -> usize {
    let (head, loca) = (f.table("head").expect("head"), f.table("loca").expect("loca"));
    let g = usize::from(gid);
    if head[51] == 0 { 2 * usize::from(u16::from_be_bytes([loca[2 * g], loca[2 * g + 1]])) }
    else { u32::from_be_bytes([loca[4 * g], loca[4 * g + 1], loca[4 * g + 2], loca[4 * g + 3]]) as usize }
}

fn a_composite(f: &Font) -> (u16, usize) {
    let gid = f.glyph_id('Á' as u32).expect("Á");
    let at = glyph_at(f, gid);
    assert_eq!(&f.table("glyf").expect("glyf")[at..at + 2], [0xFF, 0xFF], "Á is a composite");
    (gid, at)
}

// The spec reads any negative contour count as a composite, -1 being only the one to write.
#[test]
fn a_composite_of_any_negative_count_keeps_its_components() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let (gid, at) = a_composite(&f);
    let g = Font::from_vec(rebuilt(&f, |t| t.get_mut("glyf").expect("glyf")[at + 1] = 0xFE)).expect("parses");
    let out = g.subset(&[gid], &[]).expect("a subset");
    let sub = reopen(&out);
    assert!(sub.num_glyphs() > 2, "the components were dropped");
    assert_eq!(outline(&sub, out.new_gid(gid).expect("kept")), outline(&g, gid));
}

// A component past the font's glyphs fails the composite in the source; the subset fails it too
// rather than drawing .notdef in its place.
#[test]
fn a_component_past_the_font_stays_past_it() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let (gid, at) = a_composite(&f);
    let g = Font::from_vec(rebuilt(&f, |t| t.get_mut("glyf").expect("glyf")[at + 12..at + 14].copy_from_slice(&65_000u16.to_be_bytes())))
        .expect("parses");
    assert_eq!(outline(&g, gid), None);
    let out = g.subset(&[gid], &[]).expect("a subset");
    assert_eq!(outline(&reopen(&out), out.new_gid(gid).expect("kept")), None);
}

#[test]
fn a_source_without_hhea_gets_no_horizontal_metrics() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let bytes = rebuilt(&f, |t| { t.remove("hhea"); });
    let out = daegun::daecore::daetype::subsetter::subset_ttf(&bytes, &[36]).expect("a subset");
    let tables = daegun::daecore::daetype::decoder::extract_ttf_tables(&out.ttf).expect("it parses");
    assert!(!tables.contains_key("hhea") && !tables.contains_key("hmtx"), "{:?}", tables.keys().collect::<Vec<_>>());
}

// A GSUB of one SingleSubst taking each of glyphs 1 to 100 to the next: a pass follows one step.
#[test]
fn a_substitution_chain_longer_than_64_closes() {
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
    let g = Font::from_vec(rebuilt(&f, |t| { t.insert("GSUB".into(), gsub); })).expect("parses");
    let closure = g.glyph_closure(&[1], &[]).expect("the chain closes");
    assert!(closure.contains(&(N + 1)), "{} glyphs", closure.len());
    assert!(g.subset(&[1], &[]).is_ok());
}

// 12,000 pairs, V A the last: past 10,920 a format 0 subtable's length wraps 16 bits, so it is
// read to the table's end, as HarfBuzz reads the last subtable, and a subset writes it so too.
#[test]
fn a_kern_subtable_past_64_kb_kerns_and_subsets_whole() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let (a, v) = (f.glyph_id('A' as u32).expect("A"), f.glyph_id('V' as u32).expect("V"));
    let n = u32::from(f.num_glyphs());
    let at = u32::from(v) * n + u32::from(a);
    assert!(at >= 11_999, "V A sorts too early to sit past 64 KB");
    let mut pairs: Vec<u8> = Vec::new();
    for p in at - 11_999..=at {
        pairs.extend(u16s(&[(p / n) as u16, (p % n) as u16]));
        pairs.extend((if p == at { -100i16 } else { 0 }).to_be_bytes());
    }
    let length = 14 + pairs.len();
    let kern = [u16s(&[0, 1, 0, length as u16, 0x0001, 12_000, 6 * 8192, 13, 6 * (12_000 - 8192)]), pairs].concat();
    let plain = Font::from_vec(rebuilt(&f, |t| { for tag in ["GSUB", "GPOS", "GDEF", "kern"] { t.remove(tag); } })).expect("parses");
    let g = Font::from_vec(rebuilt(&plain, |t| { t.insert("kern".into(), kern); })).expect("parses");
    let width = |f: &Font| f.shape("VA", &[], false).expect("shapes").advances.iter().sum::<f64>();
    assert_eq!((width(&g) - width(&plain)).round(), -100.0, "the source's last pair was not read");
    let all: Vec<u16> = (0..f.num_glyphs()).collect();
    let sub = reopen(&g.subset(&all, &[]).expect("a subset"));
    assert_eq!(width(&sub), width(&g));
}

// A subset carries no variations: a variable font handed straight to the subsetter is subset at its
// default instance, so GDEF names no variation data whose fvar is gone.
#[test]
fn a_variable_font_is_subset_at_its_default_instance() {
    let bytes = std::fs::read(format!("{FONTS}/inter/InterVariable.ttf")).expect("the fixture font");
    let out = daegun::daecore::daetype::subsetter::subset_ttf(&bytes, &[0, 36]).expect("a subset");
    let sub = reopen(&out);
    assert!(sub.table("fvar").is_none() && sub.table("gvar").is_none());
    let gdef = sub.table("GDEF").expect("GDEF");
    let store = (gdef.len() >= 18 && gdef[2..4] == [0, 3]).then(|| u32::from_be_bytes([gdef[14], gdef[15], gdef[16], gdef[17]]));
    assert!(store.is_none_or(|at| at == 0), "GDEF names a variation store at {store:?}");
    let f = font("inter/InterVariable.ttf");
    assert_eq!(outline(&sub, out.new_gid(36).expect("kept")), outline(&f, 36));
}
