use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::*;
use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daeshaper::script::{self, Shaper};
use crate::daecore::daeshaper::unicode::SpaceFallback;
use crate::daecore::Tag;

fn u16s(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// carets.ttf with a cmap mapping each (character, glyph) and each (base, selector, glyph) variation sequence.
fn font(chars: &[(u32, u16)], sequences: &[(u32, u32, u16)]) -> FontCache {
    let mut chars = chars.to_vec();
    chars.sort_unstable();
    let groups: Vec<u8> = chars.iter().flat_map(|&(c, g)| u32s(&[c, c, u32::from(g)])).collect();
    let format12 = [u16s(&[12, 0]), u32s(&[16 + groups.len() as u32, 0, chars.len() as u32]), groups].concat();
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let mut records = Vec::new();
    let mut lists = Vec::new();
    let head = 10 + 11 * sequences.len() as u32;
    for &(base, selector, glyph) in sequences {
        records.extend([u24(selector), u32s(&[0, head + lists.len() as u32])].concat());
        lists.extend([u32s(&[1]), u24(base), u16s(&[glyph])].concat());
    }
    let length = head + lists.len() as u32;
    let format14 = [u16s(&[14]), u32s(&[length, sequences.len() as u32]), records, lists].concat();
    let header = [u16s(&[0, 2, 0, 5]), u32s(&[20]), u16s(&[3, 10]), u32s(&[20 + format14.len() as u32])].concat();
    let cmap = [header, format14, format12].concat();

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/carets.ttf");
    let source = crate::Font::from_vec(std::fs::read(path).expect("the fixture"), 0).expect("it parses");
    let mut tables: BTreeMap<Tag, Vec<u8>> =
        source.table_tags().map(|t| (t, source.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"cmap"), cmap);
    let bytes = crate::raw::build_font(&tables).unwrap();
    FontCache::new(crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("tables")).expect("a font")
}

// A model asking only for a normalization.
struct Asking(Normalization);

impl Model for Asking {
    fn kind(&self) -> Shaper {
        Shaper::Default
    }

    fn normalization(&self) -> Normalization {
        self.0
    }
}

// `text` read as the pipeline reads it, then normalized; each glyph's character and the glyph found for it.
fn normalized(cache: &FontCache, text: &str, model: &dyn Model, setup: impl Fn(&mut Run)) -> (Run, Vec<(u32, u32)>) {
    let mut run = Run::default();
    run.push_str(text);
    run.begin();
    setup(&mut run);
    for i in 0..run.len {
        let facts = run.glyphs[i].read_char();
        run.facts.add(facts);
    }
    normalize(&mut run, &Face::new(cache), model, false);
    let out = run.glyphs[..run.len].iter().map(|g| (g.id, g.mapped)).collect();
    (run, out)
}

fn plain(cache: &FontCache, text: &str, model: &dyn Model) -> Vec<(u32, u32)> {
    normalized(cache, text, model, |_| ()).1
}

// U+1E69 is U+1E63 and a dot above, and U+1E63 is s and a dot below. A character the font lacks takes the first level
// it can draw, or under `Recomposed` the last, composed again as far as the font maps; a level whose second part the
// font lacks stops the descent.
#[test]
fn a_character_takes_the_shallowest_or_deepest_level_the_font_draws() {
    let all = font(&[(0x73, 1), (0x323, 2), (0x307, 3), (0x1E63, 4)], &[]);
    let precomposed = &script::GENERAL;
    assert_eq!(plain(&all, "\u{1E69}", precomposed), [(0x1E63, 4), (0x307, 3)]);
    let recomposed = &Asking(Normalization::Recomposed);
    assert_eq!(plain(&all, "\u{1E69}", recomposed), [(0x1E63, 4), (0x307, 3)]);
    let no_dot_below = font(&[(0x73, 1), (0x323, 2), (0x307, 3)], &[]);
    assert_eq!(plain(&no_dot_below, "\u{1E69}", recomposed), [(0x73, 1), (0x323, 2), (0x307, 3)]);

    let no_dot_above = font(&[(0x73, 1), (0x323, 2), (0x1E63, 4)], &[]);
    assert_eq!(plain(&no_dot_above, "\u{1E69}", recomposed), [(0x1E69, 0)]);
    let (run, _) = normalized(&no_dot_above, "\u{1E69}", recomposed, |run| run.settings.missing = 6);
    assert_eq!(run.glyphs[0].mapped, 6, "the missing glyph asked for");
}

// A character the font maps comes out as its glyph; a model that decomposes everything composes it again.
#[test]
fn a_mapped_character_comes_out_whole() {
    let f = font(&[(0x61, 1), (0x301, 2), (0xE1, 3)], &[]);
    assert_eq!(plain(&f, "\u{E1}", &script::GENERAL), [(0xE1, 3)]);
    assert_eq!(plain(&f, "\u{E1}", &Asking(Normalization::Unrecomposed)), [(0xE1, 3)]);
    assert_eq!(plain(&f, "\u{E1}", &Asking(Normalization::Recomposed)), [(0xE1, 3)], "decomposed and composed again");
}

// A character comes out the same whatever stands beside it, whether or not another cluster in the run has a mark:
// under `Recomposed`, á decomposes and composes again; by default, ǭ, which the font lacks, is ō and an ogonek.
#[test]
fn a_character_normalizes_alike_alone_and_beside_marks() {
    let f = font(&[(0x61, 1), (0x301, 2), (0xE1, 3), (0x62, 4)], &[]);
    let recomposed = &Asking(Normalization::Recomposed);
    assert_eq!(plain(&f, "b\u{301}\u{E1}", recomposed)[2..], [(0xE1, 3)]);
    assert_eq!(plain(&f, "b\u{E1}", recomposed)[1..], [(0xE1, 3)]);
    let f = font(&[(0x6F, 1), (0x304, 2), (0x328, 3), (0x14D, 4), (0x62, 5)], &[]);
    let precomposed = &Asking(Normalization::Precomposed);
    assert_eq!(plain(&f, "b\u{301}\u{1ED}", precomposed)[2..], [(0x14D, 4), (0x328, 3)]);
    assert_eq!(plain(&f, "b\u{1ED}", precomposed)[1..], [(0x14D, 4), (0x328, 3)]);
}

// Marks sort by class, the dot below (220) before the acute (230), and a mark joins its base where the font has the
// composite: a with the dot below does, and the acute after it finds no composite of the two.
#[test]
fn marks_sort_by_class_then_compose_with_their_base() {
    let f = font(&[(0x61, 1), (0x301, 2), (0x323, 3), (0x1EA1, 4)], &[]);
    let (run, out) = normalized(&f, "a\u{301}\u{323}", &script::GENERAL, |_| ());
    assert_eq!(out, [(0x1EA1, 4), (0x301, 2)]);
    assert_eq!([run.glyphs[0].cluster, run.glyphs[1].cluster], [0, 0]);
    assert_eq!(plain(&f, "a\u{301}\u{323}", &Asking(Normalization::Unrecomposed)), [(0x61, 1), (0x323, 3), (0x301, 2)]);
}

// A circumflex the font can't compose with a stands between it and an acute of the same class, which then can't
// compose either (UAX #15's blocking).
#[test]
fn a_mark_of_the_same_class_between_blocks_composition() {
    let f = font(&[(0x61, 1), (0x301, 2), (0x302, 3), (0xE1, 4)], &[]);
    assert_eq!(plain(&f, "a\u{302}\u{301}", &script::GENERAL), [(0x61, 1), (0x302, 3), (0x301, 2)]);
    assert_eq!(plain(&f, "a\u{301}\u{302}", &script::GENERAL), [(0xE1, 4), (0x302, 3)]);
}

// A CGJ between marks that sort apart keeps them apart and stays hidden; between marks in order it shows.
#[test]
fn a_cgj_shows_where_it_holds_nothing_apart() {
    let f = font(&[(0x61, 1), (0x301, 2), (0x323, 3), (0x34F, 4)], &[]);
    let (run, out) = normalized(&f, "a\u{301}\u{34F}\u{323}", &script::GENERAL, |_| ());
    assert_eq!(out.iter().map(|g| g.0).collect::<Vec<_>>(), [0x61, 0x301, 0x34F, 0x323]);
    assert!(run.glyphs[2].char_is(text::HIDDEN), "held apart");
    let (run, _) = normalized(&f, "a\u{323}\u{34F}\u{301}", &script::GENERAL, |_| ());
    assert!(!run.glyphs[2].char_is(text::HIDDEN), "in order anyway");
}

// A space the font lacks takes the space glyph and keeps its kind for its width; a non-breaking hyphen takes the
// hyphen.
#[test]
fn spaces_and_hyphens_the_font_lacks_take_stand_ins() {
    let f = font(&[(0x20, 1), (0x2010, 2)], &[]);
    let (run, out) = normalized(&f, "\u{2002}\u{2011}", &script::GENERAL, |_| ());
    assert_eq!(out, [(0x2002, 1), (0x2011, 2)]);
    assert_eq!(run.glyphs[0].space_fallback(), Some(SpaceFallback::EmDiv(2)));
    assert!(run.facts.has(Fact::SpaceStandIns));
}

// A base and selector the font has a glyph for become that one glyph; where it has none, both stay, the selector
// marked for its stand-in and drawn, like any character the font lacks, with the missing glyph.
#[test]
fn a_selector_picks_its_sequences_glyph_or_stays_for_its_stand_in() {
    let f = font(&[(0x845B, 1)], &[(0x845B, 0xE0100, 5)]);
    let (run, out) = normalized(&f, "\u{845B}\u{E0100}", &script::GENERAL, |_| ());
    assert_eq!(out, [(0x845B, 5)]);
    assert!(!run.facts.has(Fact::SelectorFallback));

    let ask = |run: &mut Run| (run.settings.missing, run.settings.missing_selector) = (6, Some(3));
    let (run, out) = normalized(&f, "\u{845B}\u{FE00}", &script::GENERAL, ask);
    assert_eq!(out, [(0x845B, 1), (0xFE00, 6)]);
    assert!(run.facts.has(Fact::SelectorFallback));
    assert!(run.glyphs[1].char_is(text::SELECTOR) && !run.glyphs[1].char_is(text::IGNORABLE));
}

// Every character's decompositions under every model, followed down, stay within the levels followed.
#[test]
fn every_decomposition_fits_the_levels_followed() {
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        for model in script::ALL {
            let (mut at, mut depth) = (c, 0);
            while let Some((first, _)) = model.decompose(at) {
                (at, depth) = (first, depth + 1);
                assert!(depth <= MOST_LEVELS, "U+{:04X} under {:?}", u32::from(c), model.kind());
            }
        }
    }
}
