use daegun::{Font, GlyphId, Location, Tag};

use super::aat_shaping::gdef;
use super::aat_subset::{carets_with, shapes_alike, F, F_I, I, L};
use super::layout_build::{classes, coverage, list, lookup, maybe, run, single, table, tag, to, w, Field, NULL};

// A language system running the features at `indices`.
fn lang_sys(indices: &[u16]) -> Vec<u8> {
    table(vec![w(&[0, 0xFFFF, indices.len() as u16]), w(indices)])
}

// A script of a default language system and tagged ones, a null offset for None.
fn script(default: Option<Vec<u8>>, tagged: Vec<(&[u8; 4], Option<Vec<u8>>)>) -> Vec<u8> {
    let count = w(&[tagged.len() as u16]);
    let records = tagged.into_iter().flat_map(|(t, l)| [tag(t), maybe(l)]);
    table([maybe(default), count].into_iter().chain(records).collect())
}

// Each record a tag and an offset, after the count of `counted` records.
fn records(counted: usize, records: Vec<(&[u8; 4], Option<Vec<u8>>)>) -> Vec<u8> {
    let fields = records.into_iter().flat_map(|(t, s)| [tag(t), maybe(s)]);
    table([w(&[counted as u16])].into_iter().chain(fields).collect())
}

fn feature(lookups: &[u16]) -> Vec<u8> {
    table(vec![w(&[0, lookups.len() as u16]), w(lookups)])
}

// A GSUB of these scripts and features over lookups, the first f to i.
fn gsub(scripts: Vec<u8>, features: Vec<u8>, lookups: Vec<Field>) -> Font {
    carets_with(&[("GSUB", table(vec![w(&[1, 0]), to(scripts), to(features), to(list(lookups))]))])
}

fn f_to_i() -> Field {
    to(lookup(1, 0, vec![to(single(F, I))]))
}

fn ccmp() -> Vec<u8> {
    records(1, vec![(b"ccmp", Some(feature(&[0])))])
}

fn dflt(default: Option<Vec<u8>>) -> Vec<u8> {
    records(1, vec![(b"DFLT", Some(script(default, vec![])))])
}

// A null script, language system, feature or lookup holds nothing: f stays f, and the script lists no language.
// Each subset shapes as its font.
#[test]
fn a_null_layout_record_holds_nothing() {
    let null_script = gsub(records(1, vec![(b"DFLT", None)]), ccmp(), vec![f_to_i()]);
    assert_eq!(run(&null_script, "f", false).glyphs, [F].map(|g| GlyphId(g.into())));
    assert!(null_script.language_system_tags(Tag::new(b"DFLT")).is_empty());
    assert!(null_script.feature_tags(None, None).is_empty());
    shapes_alike(&null_script, "f");

    let scripts = records(1, vec![(b"DFLT", Some(script(None, vec![(b"TRK ", None)])))]);
    let null_lang_sys = gsub(scripts, ccmp(), vec![f_to_i()]);
    let opts = daegun::shape::ShapeOptions::default().with_language("tr");
    assert_eq!(null_lang_sys.shape("f", &Location::default(), &opts).glyphs, [F].map(|g| GlyphId(g.into())));
    assert!(null_lang_sys.feature_tags(Some(Tag::new(b"DFLT")), Some(Tag::new(b"TRK "))).is_empty());
    shapes_alike(&null_lang_sys, "f");

    let null_feature = gsub(dflt(Some(lang_sys(&[0]))), records(1, vec![(b"ccmp", None)]), vec![f_to_i()]);
    assert_eq!(run(&null_feature, "f", false).glyphs, [F].map(|g| GlyphId(g.into())));
    shapes_alike(&null_feature, "f");

    let null_lookup = gsub(dflt(Some(lang_sys(&[0]))), ccmp(), vec![NULL, f_to_i()]);
    assert_eq!(run(&null_lookup, "f", false).glyphs, [F].map(|g| GlyphId(g.into())));
    shapes_alike(&null_lookup, "f");
}

// Records are bisected as their tags sort: with latn listed before cyrl and DFLT, neither latn nor DFLT is found, so
// Latin text takes no script, and DEU after TRK isn't found for German.
#[test]
fn layout_records_are_found_by_bisection() {
    let some = |indices: &[u16]| Some(script(Some(lang_sys(indices)), vec![]));
    let scripts = records(3, vec![(b"latn", some(&[0])), (b"cyrl", some(&[])), (b"DFLT", some(&[]))]);
    let unsorted = gsub(scripts, ccmp(), vec![f_to_i()]);
    assert_eq!(run(&unsorted, "f", false).glyphs, [F].map(|g| GlyphId(g.into())));
    assert_eq!(unsorted.script_tags(), [b"DFLT", b"cyrl", b"latn"].map(Tag::new));
    assert!(unsorted.language_system_tags(Tag::new(b"latn")).is_empty());

    let languages = vec![(b"TRK ", Some(lang_sys(&[]))), (b"DEU ", Some(lang_sys(&[0])))];
    let unsorted = gsub(records(1, vec![(b"DFLT", Some(script(None, languages)))]), ccmp(), vec![f_to_i()]);
    let opts = daegun::shape::ShapeOptions::default().with_language("de");
    assert_eq!(unsorted.shape("f", &Location::default(), &opts).glyphs, [F].map(|g| GlyphId(g.into())));
}

// A language system tagged dflt serves a language the script lacks before its default one.
#[test]
fn a_language_system_tagged_dflt_comes_before_the_default() {
    let scripts = records(1, vec![(b"DFLT", Some(script(Some(lang_sys(&[])), vec![(b"dflt", Some(lang_sys(&[0])))])))]);
    let f = gsub(scripts, ccmp(), vec![f_to_i()]);
    assert_eq!(run(&f, "f", false).glyphs, [I].map(|g| GlyphId(g.into())));
    let opts = daegun::shape::ShapeOptions::default().with_language("tr");
    assert_eq!(f.shape("f", &Location::default(), &opts).glyphs, [I].map(|g| GlyphId(g.into())));
}

// vert in GSUB's language system, and in GPOS's feature list alone: found in one table, it isn't looked for in the
// other's list, so GPOS doesn't move i.
#[test]
fn vert_is_looked_for_in_the_lists_only_when_neither_language_system_has_it() {
    let vert = || records(1, vec![(b"vert", Some(feature(&[0])))]);
    let gsub = table(vec![w(&[1, 0]), to(dflt(Some(lang_sys(&[0])))), to(vert()), to(list(vec![f_to_i()]))]);
    let moved = table(vec![w(&[1]), to(coverage(&[I])), w(&[1, 100])]);
    let positions = list(vec![to(lookup(1, 0, vec![to(moved)]))]);
    let gpos = table(vec![w(&[1, 0]), to(dflt(Some(lang_sys(&[])))), to(vert()), to(positions)]);
    let f = carets_with(&[("GSUB", gsub), ("GPOS", gpos)]);
    let run = run(&f, "f", true);
    assert_eq!(run.glyphs, [I].map(|g| GlyphId(g.into())));
    assert_eq!(crate::xy(&run.offsets), [(-200.0, -850.0)]);
}

// The language system runs feature 1 of a list counting one, whose bytes after it read as a ccmp record: it's the
// empty record, so nothing runs and nothing is listed.
#[test]
fn a_feature_past_the_count_is_empty() {
    let features = records(1, vec![(b"liga", Some(feature(&[]))), (b"ccmp", Some(feature(&[0])))]);
    let f = gsub(dflt(Some(lang_sys(&[1]))), features, vec![f_to_i()]);
    assert_eq!(run(&f, "f", false).glyphs, [F].map(|g| GlyphId(g.into())));
    assert!(f.feature_tags(None, None).is_empty());
}

// GDEF 1.2 with i a mark and one mark glyph set, of `set`.
fn gdef_with_set(set: &[u16]) -> Vec<u8> {
    let sets = table(vec![w(&[1, 1]), Field::To32(Some(coverage(set)))]);
    table(vec![w(&[1, 2]), to(classes(&[(I, I, 3)])), w(&[0, 0, 0]), to(sets)])
}

// f and i ligate under a lookup filtering marks by set 5, which no GDEF has: the flag is dropped, so i joins as any
// glyph does. Set 0 of l alone still filters i out.
#[test]
fn a_mark_filtering_set_the_font_lacks_filters_nothing() {
    let lig = table(vec![w(&[F_I, 2, I])]);
    let ligature = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(list(vec![to(lig)]))]);
    let filtered = |set: u16, gdef: Vec<u8>| {
        let lookups = vec![to(table(vec![w(&[4, 0x10, 1]), to(ligature.clone()), w(&[set])]))];
        let gsub = table(vec![w(&[1, 0]), to(dflt(Some(lang_sys(&[0])))), to(ccmp()), to(list(lookups))]);
        carets_with(&[("GSUB", gsub), ("GDEF", gdef)])
    };
    assert_eq!(run(&filtered(5, gdef(&[(I, I, 3)])), "fi", false).glyphs, [F_I].map(|g| GlyphId(g.into())));
    assert_eq!(run(&filtered(5, gdef_with_set(&[3])), "fi", false).glyphs, [F_I].map(|g| GlyphId(g.into())));
    assert_eq!(run(&filtered(0, gdef_with_set(&[3])), "fi", false).glyphs, [F, I].map(|g| GlyphId(g.into())));
}

// f to i in the language systems `systems` names, running ccmp's feature at its index, beside an empty default.
fn systems(tagged: &[(&[u8; 4], u16)]) -> Vec<(&'static str, Vec<u8>)> {
    let tagged = tagged.iter().map(|&(t, i)| (t, Some(lang_sys(&[i])))).collect();
    let scripts = records(1, vec![(b"DFLT", Some(script(Some(lang_sys(&[])), tagged)))]);
    let features = records(2, vec![(b"ccmp", Some(feature(&[0]))), (b"ccmp", Some(feature(&[1])))]);
    let lookups = list(vec![f_to_i(), to(lookup(1, 0, vec![to(single(F, L))]))]);
    vec![("GSUB", table(vec![w(&[1, 0]), to(scripts), to(features), to(lookups)]))]
}

// A language tag, as a locale or in any case, reaches each language system it names: f becomes i only where it
// names the font's one, and l where it names MOL before ROM. Hong Kong's Chinese falls back on Traditional, never
// Simplified.
#[test]
fn a_language_tag_reaches_every_system_it_names() {
    let cases: [(&[u8; 4], &str, u16); 22] = [
        (b"SRB ", "sr_RS", I), (b"TRK ", "tr_TR", I), (b"TRK ", "TR-tr", I), (b"ROM ", "ro_RO", I),
        (b"NLD ", "nl_NL", I), (b"ZHT ", "zh_TW", I), (b"ZHT ", "zh_HK", I), (b"ZHS ", "zh_HK", F),
        (b"INUK", "iu", I), (b"ROM ", "mo", I),
        (b"FAR ", "pes", I), (b"FAR ", "prs", I), (b"ZHH ", "zh-yue", I), (b"ZHH ", "yue", I), (b"ZHS ", "cmn", I),
        (b"QAA ", "qaa", I), (b"HRV ", "sh", I), (b"ZHH ", "zh-Hant-MO", I), (b"ARA ", "arz", I),
        (b"IPPH", "en-fonipa", I), (b"PGR ", "el-polyton", I), (b"SRB ", "en", F),
    ];
    let shaped = |f: &Font, language| {
        f.shape("f", &Location::default(), &daegun::shape::ShapeOptions::default().with_language(language))
    };
    for (system, language, glyph) in cases {
        assert_eq!(
            shaped(&carets_with(&systems(&[(system, 0)])), language).glyphs,
            [GlyphId(glyph.into())],
            "{language}"
        );
    }
    let both = carets_with(&systems(&[(b"MOL ", 1), (b"ROM ", 0)]));
    for (language, glyph) in [("mo", L), ("ro", I), ("ro-MD", L), ("mo-MD", L), ("en", F)] {
        assert_eq!(shaped(&both, language).glyphs, [GlyphId(glyph.into())], "{language}");
    }
}
