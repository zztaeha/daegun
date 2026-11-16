use daegun::shape::Feature;
use super::aat_shaping::SET_MARK;
use super::aat_subset::{
    carets_with, glyph_ids, lookup, shapes_alike, stx, u16s, u32s, F, F_F_I, F_F_L, F_I, I, L, PERFORM_ACTION,
    SET_COMPONENT,
};
use daegun::{GlyphId, Location, Tag};

// A morx of one chain: its default flags, its features as (type, setting, enable, disable) and its subtables as
// (type, flags, body).
fn chain(default: u32, features: &[(u16, u16, u32, u32)], subtables: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for &(kind, setting, enable, disable) in features {
        body.extend(u16s(&[kind, setting]));
        body.extend(u32s(&[enable, disable]));
    }
    for (kind, flags, subtable) in subtables {
        body.extend(u32s(&[(12 + subtable.len()) as u32, *kind, *flags]));
        body.extend(subtable);
    }
    let mut out = u16s(&[2, 0]);
    out.extend(u32s(&[1, default, (16 + body.len()) as u32, features.len() as u32, subtables.len() as u32]));
    out.extend(body);
    out
}

// A feat naming (type, settings, flags) features, in the order given.
fn feat(features: &[(u16, &[u16], u16)]) -> Vec<u8> {
    let mut settings_at = 12 + 12 * features.len();
    let (mut names, mut settings) = (Vec::new(), Vec::new());
    for &(kind, list, flags) in features {
        names.extend(u16s(&[kind, list.len() as u16]));
        names.extend(u32s(&[settings_at as u32]));
        names.extend(u16s(&[flags, 256]));
        list.iter().for_each(|&s| settings.extend(u16s(&[s, 256])));
        settings_at += 4 * list.len();
    }
    [u32s(&[0x0001_0000]), u16s(&[features.len() as u16, 0]), u32s(&[0]), names, settings].concat()
}

// An ltag of these language tags.
fn ltag(tags: &[&str]) -> Vec<u8> {
    let mut at = 12 + 4 * tags.len();
    let mut ranges = Vec::new();
    for t in tags {
        ranges.extend(u16s(&[at as u16, t.len() as u16]));
        at += t.len();
    }
    [u32s(&[1, 0, tags.len() as u32]), ranges, tags.concat().into_bytes()].concat()
}

const EXCLUSIVE: u16 = 0x8000;

// One chain whose features each set a flag that turns one glyph into another, as the subtables say; ltag names
// only "tr", so its ninth entry is missing. `kinds` are the feature types feat names.
fn featured(kinds: &[(u16, &[u16], u16)]) -> daegun::Font {
    const ALL: u32 = 0xFFFF_FFFF;
    let features = [
        (1, 2, 0x01, ALL),
        (1, 3, 0, !0x01),
        (37, 1, 0x02, ALL),
        (3, 3, 0x04, ALL),
        (39, 1, 0x08, ALL),
        (39, 9, 0x10, ALL),
        (17, 1, 0x40, ALL),
        (17, 2, 0x20, ALL),
    ];
    let subtables = [
        (4, 0x08, lookup(&[(F, F_F_I)])),
        (4, 0x01, lookup(&[(F, F_I)])),
        (4, 0x20, lookup(&[(I, L)])),
        (4, 0x40, lookup(&[(I, F_F_L)])),
        (4, 0x02, lookup(&[(I, F_F_I)])),
        (4, 0x10, lookup(&[(L, F_I)])),
        (4, 0x04, lookup(&[(L, F_F_L)])),
    ];
    carets_with(&[("morx", chain(0x01, &features, &subtables)), ("feat", feat(kinds)), ("ltag", ltag(&["tr"]))])
}

fn glyphs(f: &daegun::Font, features: &[Feature], language: Option<&str>) -> Vec<GlyphId> {
    let options = daegun::shape::ShapeOptions::default().with_features(features);
    let options = language.map_or(options, |language| options.with_language(language));
    f.shape("fil", &Location::default(), &options).glyphs.clone()
}

// Features reach chains through feat, small caps falling back to the deprecated letter case a font made before lower
// case has, and the last asked of one setting or exclusive type stands, as an OpenType feature asked twice does.
#[test]
fn requested_features_set_each_chains_flags_through_feat() {
    let with_lower_case: &[(u16, &[u16], u16)] = &[(1, &[2, 3], 0), (17, &[1, 2], EXCLUSIVE), (37, &[0, 1], EXCLUSIVE)];
    let with_letter_case: &[(u16, &[u16], u16)] = &[(1, &[2, 3], 0), (3, &[0, 3], EXCLUSIVE)];
    let ligatures_only: &[(u16, &[u16], u16)] = &[(1, &[2, 3], 0)];
    let lower = featured(with_lower_case);
    let en = Some("en");
    for (f, features, want) in [
        (&lower, &[][..], [F_I, I, L]),
        (&lower, &[Feature::new(Tag::new(b"liga"), 0)], [F, I, L]),
        (&lower, &[Feature::new(Tag::new(b"liga"), 0), Feature::new(Tag::new(b"liga"), 1)], [F_I, I, L]),
        (&lower, &[Feature::new(Tag::new(b"liga"), 1), Feature::new(Tag::new(b"liga"), 0)], [F, I, L]),
        (&lower, &[Feature::new(Tag::new(b"smcp"), 1)], [F_I, F_F_I, F_F_L]),
        (&featured(with_letter_case), &[Feature::new(Tag::new(b"smcp"), 1)], [F_I, F_F_I, F_F_L]),
        (&featured(ligatures_only), &[Feature::new(Tag::new(b"smcp"), 1)], [F_I, I, L]),
        (&featured(ligatures_only), &[Feature::new(Tag::new(b"liga"), 0)], [F, I, L]),
        (&lower, &[Feature::new(Tag::new(b"aalt"), 2)], [F_I, L, L]),
        (&lower, &[Feature::new(Tag::new(b"aalt"), 1), Feature::new(Tag::new(b"aalt"), 2)], [F_I, L, L]),
        (&lower, &[Feature::new(Tag::new(b"aalt"), 2), Feature::new(Tag::new(b"aalt"), 1)], [F_I, F_F_L, L]),
        (&featured(&[]), &[Feature::new(Tag::new(b"liga"), 0)], [F_I, I, L]),
    ] {
        assert_eq!(glyphs(f, features, en), glyph_ids(&want), "{features:?}");
    }
}

// "tr" matches a language equal to it or starting with it and a hyphen, case and `_` aside; an entry ltag lacks
// matches only no language.
#[test]
fn a_chains_language_feature_matches_through_ltag() {
    let f = featured(&[(1, &[2, 3], 0)]);
    for (language, want) in [
        (Some("tr"), [F_F_I, I, L]),
        (Some("TR_tr"), [F_F_I, I, L]),
        (Some("tr-Latn"), [F_F_I, I, L]),
        (Some("trk"), [F_I, I, L]),
        (Some("en"), [F_I, I, L]),
        (None, [F_I, I, F_I]),
    ] {
        assert_eq!(glyphs(&f, &[], language), glyph_ids(&want), "{language:?}");
    }
    shapes_alike(&f, "fil");
}

// A chain whose ligature machine, run under liga, forms f_i from f then i and f_f_l from f then l.
fn ligated() -> daegun::Font {
    let start = [0, 0, 0, 0, 1, 0];
    let after_f = [0, 0, 0, 0, 1, 2];
    let entries: [&[u16]; 3] = [&[0, 0, 0], &[2, SET_COMPONENT, 0], &[0, SET_COMPONENT | PERFORM_ACTION, 0]];
    let tails = [u32s(&[0x3FFF_FFFE, 0x8000_0000]), u16s(&[0, 1, 1, F_F_I]), u16s(&[0, F_I, F_F_L])];
    let machine = stx(6, &[(F, 4), (I, 5), (L, 5)], &[&start, &start, &after_f], &entries, &tails);
    let features = [(1, 2, 0x01, 0xFFFF_FFFF), (1, 3, 0, !0x01)];
    let morx = chain(0x01, &features, &[(2, 0x01, machine)]);
    carets_with(&[("morx", morx), ("feat", feat(&[(1, &[2, 3], 0)]))])
}

fn shaped(f: &daegun::Font, text: &str, features: &[Feature]) -> Vec<GlyphId> {
    let options = daegun::shape::ShapeOptions::default().with_features(features).with_language("en");
    f.shape(text, &Location::default(), &options).glyphs.clone()
}

// A ranged feature sets its chain's flags there alone: machines skip and restart around a glyph it's off for, and of
// two settings meeting the last asked stands.
#[test]
fn a_ranged_feature_sets_its_chains_flags_over_its_range() {
    let ranged = |tag, value, start: usize, end: usize| Feature { tag, value, bytes: (start..end).into() };
    let liga = |value, start, end| ranged(Tag::new(b"liga"), value, start, end);
    let all = usize::MAX;
    let ligated = ligated();
    for (features, fifi, fifl, ffil) in [
        (vec![], &[F_I, F_I][..], &[F_I, F_F_L][..], &[F, F_I, L][..]),
        (vec![liga(0, 0, 2)], &[F, I, F_I], &[F, I, F_F_L], &[F, F, I, L]),
        (vec![liga(0, 1, 3)], &[F, I, F, I], &[F, I, F, L], &[F, F, I, L]),
        (vec![liga(0, 2, 4)], &[F_I, F, I], &[F_I, F, L], &[F, F, I, L]),
        (vec![liga(0, 1, 2)], &[F, I, F_I], &[F, I, F_F_L], &[F, F, I, L]),
        (vec![liga(0, 3, all)], &[F_I, F, I], &[F_I, F, L], &[F, F_I, L]),
        (vec![liga(0, 0, 2), liga(1, 1, 3)], &[F, I, F_I], &[F, I, F_F_L], &[F, F_I, L]),
        (vec![Feature::new(Tag::new(b"liga"), 0), liga(1, 2, 4)], &[F, I, F_I], &[F, I, F_F_L], &[F, F, I, L]),
    ] {
        let got = ["fifi", "fifl", "ffil"].map(|text| shaped(&ligated, text, &features));
        assert_eq!(got, [fifi, fifl, ffil].map(glyph_ids), "{features:?}");
    }

    let lookups = featured(&[(1, &[2, 3], 0)]);
    for (features, fff, fil, lif) in [
        (vec![], [F_I, F_I, F_I], [F_I, I, L], [L, I, F_I]),
        (vec![liga(0, 1, 2)], [F_I, F, F_I], [F_I, I, L], [L, I, F_I]),
        (vec![liga(0, 0, 1), liga(0, 2, all)], [F, F_I, F], [F, I, L], [L, I, F]),
    ] {
        let got = ["fff", "fil", "lif"].map(|text| shaped(&lookups, text, &features));
        assert_eq!(got, [fff, fil, lif].map(|g| glyph_ids(&g)), "{features:?}");
    }

    let lower = featured(&[(1, &[2, 3], 0), (17, &[1, 2], EXCLUSIVE), (37, &[0, 1], EXCLUSIVE)]);
    for (features, fil, lif) in [
        (vec![Feature::new(Tag::new(b"smcp"), 1)], [F_I, F_F_I, F_F_L], [F_F_L, F_F_I, F_I]),
        (vec![ranged(Tag::new(b"smcp"), 1, 1, 2)], [F_I, F_F_I, L], [L, F_F_I, F_I]),
        (vec![ranged(Tag::new(b"smcp"), 1, 2, all), liga(0, 0, 1)], [F, I, F_F_L], [L, I, F_I]),
        (vec![ranged(Tag::new(b"aalt"), 2, 1, 2)], [F_I, L, L], [L, L, F_I]),
    ] {
        let want = [fil, lif].map(|g| glyph_ids(&g));
        assert_eq!(["fil", "lif"].map(|text| shaped(&lower, text, &features)), want, "{features:?}");
    }
}

// A ranged feature stops a machine where a text would end, without the end's action: the machine marks f to become
// f_i at the end of the text, and liga off over i stops it short. "f" shaped apart does end there, so the break before
// i can't be safe.
#[test]
fn a_break_where_a_ranged_feature_stops_a_machine_is_unsafe_where_its_end_would_act() {
    let (start, after_f): (&[u16], &[u16]) = (&[0, 0, 0, 0, 1, 0], &[2, 0, 0, 0, 1, 0]);
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF, 0xFFFF], &[2, SET_MARK, 0xFFFF, 0xFFFF], &[0, 0, 0, 0xFFFF]];
    let substitutions = [u32s(&[4]), lookup(&[(F, F_I)])].concat();
    let machine = stx(6, &[(F, 4), (I, 5)], &[start, start, after_f], &entries, &[substitutions]);
    let morx = chain(0x01, &[(1, 2, 0x01, 0xFFFF_FFFF), (1, 3, 0, !0x01)], &[(1, 0x01, machine)]);
    let f = carets_with(&[("morx", morx), ("feat", feat(&[(1, &[2, 3], 0)]))]);
    let off = [Feature { tag: Tag::new(b"liga"), value: 0, bytes: (1..2).into() }];
    let run = f.shape("fi", &Location::default(), &daegun::shape::ShapeOptions::default().with_features(&off));
    assert_eq!((run.glyphs.clone(), shaped(&f, "f", &[])), (glyph_ids(&[F, I]), glyph_ids(&[F_I])));
    assert!(run.unsafe_to_break[1], "the break before i is called safe: {:?}", run.unsafe_to_break);
}

// smcp over bytes 2 to 1, ending before it starts: like an empty range it sets the chain's flags nowhere, as GSUB
// applies such a range nowhere, where on from byte 2 it would turn fil's l into f_f_l.
#[test]
fn a_reversed_feature_range_sets_its_chains_flags_nowhere() {
    let lower = featured(&[(1, &[2, 3], 0), (17, &[1, 2], EXCLUSIVE), (37, &[0, 1], EXCLUSIVE)]);
    let reversed = Feature { tag: Tag::new(b"smcp"), value: 1, bytes: core::range::Range { start: 2, end: 1 } };
    let want = [[F_I, I, L], [L, I, F_I]].map(|g| glyph_ids(&g));
    assert_eq!(["fil", "lif"].map(|text| shaped(&lower, text, &[reversed])), want);
}
