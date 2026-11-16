use super::aat_shaping::gdef;
use super::aat_subset::{carets_with, font, glyph_ids, u16s, F, F_F_I, F_F_L, F_I, I, L};
use super::cmap_reading::{cmap, format4};
use super::layout_build::{
    aliased, anchor, classes, coverage, layout, list, lookup, maybe, repeated, run, single, table, to, w, Field, NULL,
};
use daegun::{GlyphId, Location};
use daegun::shape::ShapeOptions;

// A GSUB or GPOS whose `feature`, the default script's one, runs one lookup of `kind` over `subtables`.
pub(super) fn layout_lookup(feature: &[u8; 4], kind: u16, subtables: &[Vec<u8>]) -> Vec<u8> {
    layout(b"DFLT", &[(feature, &[0])], vec![lookup(kind, 0, subtables.iter().cloned().map(to).collect())])
}

fn coverage1(glyphs: &[u16]) -> Vec<u8> {
    [u16s(&[1, glyphs.len() as u16]), u16s(glyphs)].concat()
}

fn coverage2(ranges: &[[u16; 3]]) -> Vec<u8> {
    [u16s(&[2, ranges.len() as u16]), ranges.iter().flat_map(|r| u16s(r)).collect()].concat()
}

// A format 2 single substitution, its coverage after the substitutes.
fn single2(coverage: Vec<u8>, substitutes: &[u16]) -> Vec<u8> {
    let n = substitutes.len() as u16;
    [u16s(&[2, 6 + 2 * n, n]), u16s(substitutes), coverage].concat()
}

// Coverages repeating i, both formats under and over 64 records, take the first matching midpoint, and a range's
// index sums past 16 bits.
#[test]
fn coverage_finds_a_glyph_as_a_midpoint_search_does() {
    let long: Vec<u16> = [[F_F_L; 32].as_slice(), &[F_F_I, F_I], &[F_F_L; 32]].concat();
    let ranges: Vec<[u16; 3]> = (0..66).map(|k| [I, I, k]).collect();
    let cases = [
        (single2(coverage1(&[F, I, I, I, L]), &[F_F_I, F_I, F_F_L, F, I]), [F_F_I, F_F_L, I]),
        (single2(coverage1(&[I; 66]), &long), [F, F_F_I, L]),
        (single2(coverage2(&[[I, I, 0], [I, I, 1], [I, I, 2]]), &[F_F_I, F_I, F_F_L]), [F, F_I, L]),
        (single2(coverage2(&ranges), &long), [F, F_F_I, L]),
        ([u16s(&[1, 6, 3]), coverage2(&[[F, I, 65535]])].concat(), [F_F_I, F_I, L]),
    ];
    for (i, (subtable, want)) in cases.into_iter().enumerate() {
        let f = carets_with(&[("GSUB", layout_lookup(b"ccmp", 1, &[subtable]))]);
        assert_eq!(f.shape("fil", &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(&want), "case {i}");
    }
}

// 100 lookups naming one SingleSubst of 30,000 subtables, then lookup 100 turning f to f_i: ccmp runs all 101, its
// plan's reading out before the last, and ss01 lookup 100 alone. Each shapes as on a fresh font, whichever ran first.
#[test]
fn each_plan_applies_its_lookups_whichever_plan_indexed_them_first() {
    use daegun::shape::Feature;
    use daegun::Tag;
    let own = vec![lookup(1, 0, vec![to(single(F, F_I))])];
    let gsub = aliased(&[(b"ccmp", 0..101), (b"ss01", 100..101)], 100, repeated(1, 0, 30_000, single(L, I)), own);
    let alone = [Feature::new(Tag::new(b"ccmp"), 0), Feature::new(Tag::new(b"ss01"), 1)];
    let plans = [ShapeOptions::default(), ShapeOptions::default().with_features(&alone)];
    let shaped = |f: &daegun::Font, k: usize| f.shape("f", &Location::default(), &plans[k]).glyphs.clone();
    let font = || carets_with(&[("GSUB", gsub.clone())]);
    let fresh = [0, 1].map(|k| shaped(&font(), k));
    assert_eq!(fresh, [glyph_ids(&[F]), glyph_ids(&[F_I])]);
    for first in [0, 1] {
        let f = font();
        shaped(&f, first);
        assert_eq!(shaped(&f, 1 - first), fresh[1 - first], "after plan {first}");
    }
}

// A multiple substitution covering f and i, whose sequence for i is missing: its count stops at f's, or its offset
// is 0. Both read the empty sequence, which deletes i, shaping [3=0+400|3=0+400|3=2+400].
#[test]
fn a_missing_sequence_deletes_its_glyph() {
    let coverage = u16s(&[1, 2, F, I]);
    let f_to_ll = u16s(&[2, L, L]);
    let short = [u16s(&[1, 8, 1, 16]), coverage.clone(), f_to_ll.clone()].concat();
    let null = [u16s(&[1, 10, 2, 18, 0]), coverage, f_to_ll].concat();
    for (what, subtable) in [("short", short), ("null", null)] {
        let f = carets_with(&[("GSUB", layout_lookup(b"ccmp", 2, &[subtable]))]);
        let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
        let want = glyph_ids(&[L, L, L]);
        assert_eq!((run.glyphs.as_slice(), run.clusters.as_slice()), (&want[..], &[0, 0, 2][..]), "{what}");
    }
}

type Rule<'a> = (&'a [u16], &'a [(u16, u16)]);

// A context substitution of format 1 for f whose rules each match their input glyphs after f and run their (position,
// lookup) records, a null offset for None; lookup 1 makes f l.
fn rules_for_f(rules: Vec<Option<Rule>>) -> daegun::Font {
    let rule = |(input, records): Rule| {
        let records: Vec<u16> = records.iter().flat_map(|r| [r.0, r.1]).collect();
        let count = if input.is_empty() { 0 } else { input.len() as u16 + 1 };
        table(vec![w(&[count, (records.len() / 2) as u16]), w(input), w(&records)])
    };
    let set = list(rules.into_iter().map(|r| maybe(r.map(rule))).collect());
    let context = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let lookups = vec![lookup(5, 0, vec![to(context)]), lookup(1, 0, vec![to(single(F, L))])];
    carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], lookups))])
}

// A rule for f then i that fails on f then l marks the pair unsafe to concat, by glyph and by class, and so do Noto
// Sans Devanagari's abvs rules for a lone short a, each needing a sign after it.
#[test]
fn a_context_rule_that_fails_is_unsafe_to_concat() {
    let by_glyph = rules_for_f(vec![Some((&[I], &[(0, 1)]))]);
    assert_eq!(run(&by_glyph, "fl", false).unsafe_to_concat, [true, true]);

    let rule = table(vec![w(&[2, 1, 2, 0, 1])]);
    let sets = vec![NULL, to(list(vec![to(rule)]))];
    let mut fields = vec![w(&[2]), to(coverage(&[F])), to(classes(&[(F, F, 1), (I, I, 2)])), w(&[2])];
    fields.extend(sets);
    let lookups = vec![lookup(5, 0, vec![to(table(fields))]), lookup(1, 0, vec![to(single(F, L))])];
    let by_class = carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], lookups))]);
    assert_eq!(run(&by_class, "fl", false).unsafe_to_concat, [true, true]);

    let short_a = run(&font("noto-devanagari/NotoSansDevanagari.ttf"), "\u{904}", false);
    assert_eq!((&short_a.glyphs[..], &short_a.unsafe_to_concat[..]), (&[GlyphId(22)][..], &[true][..]));
}

// A rule of no input, given or at a null offset, matches f alone and runs nothing, so the rule after it, f then i to
// l i, never runs. The same in a chain rule.
#[test]
fn a_rule_of_no_input_matches_one_glyph() {
    let f_i: Rule = (&[I], &[(0, 1)]);
    assert_eq!(run(&rules_for_f(vec![Some(f_i)]), "fi", false).glyphs, [L, I].map(|g| GlyphId(g.into())));
    assert_eq!(
        run(&rules_for_f(vec![Some((&[], &[])), Some(f_i)]), "fi", false).glyphs,
        [F, I].map(|g| GlyphId(g.into()))
    );
    assert_eq!(run(&rules_for_f(vec![None, Some(f_i)]), "fi", false).glyphs, [F, I].map(|g| GlyphId(g.into())));

    let chain_rule = |input: &[u16]| {
        let count = if input.is_empty() { 0 } else { input.len() as u16 + 1 };
        table(vec![w(&[0, count]), w(input), w(&[0, 1, 0, 1])])
    };
    let set = list(vec![to(chain_rule(&[])), to(chain_rule(&[I]))]);
    let chain = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let lookups = vec![lookup(6, 0, vec![to(chain)]), lookup(1, 0, vec![to(single(F, L))])];
    let f = carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], lookups))]);
    assert_eq!(run(&f, "fi", false).glyphs, [F, I].map(|g| GlyphId(g.into())));
}

// f, a mark, l: ccmp splits f to f and i, liga joins f_i, then f_i and l past the mark. f_i counts one component, so
// the mark attaches at f_f_l's first anchor.
#[test]
fn a_ligature_counts_the_components_within_its_ligation() {
    let map = format4(&[(0x66, F), (0x69, I), (0x6C, L), (0x301, F_F_I)]);
    let split = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(table(vec![w(&[2, F, I])]))]);
    let ligature = |first: u16, rest: &[u16], glyph: u16| {
        let lig = table(vec![w(&[glyph, rest.len() as u16 + 1]), w(rest)]);
        table(vec![w(&[1]), to(coverage(&[first])), w(&[1]), to(list(vec![to(lig)]))])
    };
    let lookups = vec![
        lookup(2, 0, vec![to(split)]),
        lookup(4, 0, vec![to(ligature(F, &[I], F_I))]),
        lookup(4, 8, vec![to(ligature(F_I, &[L], F_F_L))]),
    ];
    let gsub = layout(b"DFLT", &[(b"ccmp", &[0]), (b"liga", &[1, 2])], lookups);
    let attach = list(vec![to(anchor(100, 0)), to(anchor(200, 0)), to(anchor(300, 0))]);
    let marks = table(vec![w(&[1, 0]), to(anchor(0, 0))]);
    let lig_array = to(list(vec![to(attach)]));
    let mark_lig = table(vec![w(&[1]), to(coverage(&[F_F_I])), to(coverage(&[F_F_L])), w(&[1]), to(marks), lig_array]);
    let gpos = layout(b"DFLT", &[(b"mark", &[0])], vec![lookup(5, 0, vec![to(mark_lig)])]);
    let gdef = gdef(&[(F_F_I, F_F_I, 3), (F_I, F_F_L, 2)]);
    let f = carets_with(&[("cmap", cmap(&[(3, 1, &map)])), ("GSUB", gsub), ("GPOS", gpos), ("GDEF", gdef)]);
    let run = run(&f, "f\u{301}l", false);
    assert_eq!(run.glyphs, [F_F_L, F_F_I].map(|g| GlyphId(g.into())));
    assert_eq!(crate::xy(&run.offsets), [(0.0, 0.0), (-800.0, 0.0)]);
}

// f, an acute (f_f_i, a mark) and i. A rule for f and i deletes f, then substitutes at its first position again,
// which holds the mark: a lookup that shrinks the run removes the positions after it.
#[test]
fn a_nested_lookup_that_deletes_moves_the_positions_after_it() {
    let map = format4(&[(0x66, F), (0x69, I), (0x6C, L), (0x301, F_F_I)]);
    let rule = table(vec![w(&[3, 2, 2]), to(coverage(&[F])), to(coverage(&[I])), w(&[0, 1, 0, 2])]);
    let delete = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(table(vec![w(&[0])]))]);
    let swap = table(vec![w(&[2]), to(coverage(&[I, F_F_I])), w(&[2, L, F_I])]);
    let lookups = vec![lookup(5, 8, vec![to(rule)]), lookup(2, 0, vec![to(delete)]), lookup(1, 0, vec![to(swap)])];
    let gsub = layout(b"DFLT", &[(b"ccmp", &[0])], lookups);
    let gdef = gdef(&[(F_F_I, F_F_I, 3)]);
    let f = carets_with(&[("cmap", cmap(&[(3, 1, &map)])), ("GSUB", gsub), ("GDEF", gdef)]);
    assert_eq!(run(&f, "f\u{301}i", false).glyphs, [F_I, I].map(|g| GlyphId(g.into())));
}

// A chain rule for f whose lookahead coverage lists l, f and i out of order: its search finds f and i but not l.
// Then one whose backtrack, input and lookahead name one coverage of f, i and l.
#[test]
fn a_context_coverage_answers_as_its_search_does() {
    let font = |chain: Vec<u8>, nested: Vec<u8>| {
        let lookups = vec![lookup(6, 0, vec![to(chain)]), lookup(1, 0, vec![to(nested)])];
        carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], lookups))])
    };
    let ahead = table(vec![w(&[3, 0, 1]), to(coverage(&[F])), w(&[1]), to(coverage(&[L, F, I])), w(&[1, 0, 1])]);
    let misordered = font(ahead, single(F, F_I));
    let shared = font([u16s(&[3, 1, 20, 1, 20, 1, 20, 1, 0, 1]), coverage(&[F, I, L])].concat(), single(I, F_I));
    let cases = [
        (&misordered, "fl", &[F, L][..]),
        (&misordered, "fi", &[F_I, I]),
        (&misordered, "ff", &[F_I, F]),
        (&shared, "fil", &[F, F_I, L]),
        (&shared, "lif", &[L, F_I, F]),
    ];
    for (f, text, want) in cases {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(want), "{text}");
    }
}

// f's ligatures f f i, f f l and f i, in that order, as in carets.ttf. Each f i forms after f f i fails on its
// second glyph and marks the pair unsafe to concat, and f i keeps the mark.
#[test]
fn a_ligature_formed_after_one_that_failed_is_unsafe_to_concat() {
    let lig = |glyph: u16, rest: &[u16]| to(table(vec![w(&[glyph, rest.len() as u16 + 1]), w(rest)]));
    let set = list(vec![lig(F_F_I, &[F, I]), lig(F_F_L, &[F, L]), lig(F_I, &[I])]);
    let gsub = layout_lookup(b"liga", 4, &[table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)])]);
    let f = carets_with(&[("GSUB", gsub)]);
    let fifi = run(&f, "fifi", false);
    assert_eq!(fifi.glyphs, [F_I, F_I].map(|g| GlyphId(g.into())));
    assert_eq!(fifi.unsafe_to_concat, [true, true]);
    let ifilfi = run(&f, "ifilfi", false);
    assert_eq!(ifilfi.glyphs, [I, F_I, L, F_I].map(|g| GlyphId(g.into())));
    assert_eq!(ifilfi.unsafe_to_concat, [false, true, false, true]);
}

// f's ligatures, as (ligature, the glyphs after f), in a liga lookup of `flag`, with `extra` tables beside it.
fn ligature_font(ligatures: &[(u16, &[u16])], flag: u16, extra: &[(&str, Vec<u8>)]) -> daegun::Font {
    let lig = |&(glyph, rest): &(u16, &[u16])| to(table(vec![w(&[glyph, rest.len() as u16 + 1]), w(rest)]));
    let set = list(ligatures.iter().map(lig).collect());
    let subtable = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let gsub = layout(b"DFLT", &[(b"liga", &[0])], vec![lookup(4, flag, vec![to(subtable)])]);
    carets_with(&[[("GSUB", gsub)].as_slice(), extra].concat())
}

// A ZWJ mapped to f_f_l, which f's first ligature takes as its second glyph: the ignorable is matched, not
// skipped.
#[test]
fn a_default_ignorable_can_be_a_ligatures_second_glyph() {
    let map = format4(&[(0x66, F), (0x69, I), (0x6C, L), (0x200D, F_F_L)]);
    let f = ligature_font(&[(F_F_I, &[F_F_L]), (F_I, &[I])], 0, &[("cmap", cmap(&[(3, 1, &map)]))]);
    let got = run(&f, "f\u{200D}i", false);
    assert_eq!(got.glyphs, [F_F_I, I].map(|g| GlyphId(g.into())));
    assert_eq!(got.unsafe_to_concat, [false, false]);
}

// A ligature of 65 glyphs fails before it reads one, so it marks nothing, and f i forms after it.
#[test]
fn a_ligature_past_64_glyphs_fails_unmarked() {
    let f = ligature_font(&[(F_F_I, &[L; 64]), (F_I, &[I])], 0, &[]);
    let got = run(&f, "fi", false);
    assert_eq!(got.glyphs, [F_I].map(|g| GlyphId(g.into())));
    assert_eq!(got.unsafe_to_concat, [false]);
}

// An f that ends the run: each ligature reads to the end and marks it.
#[test]
fn a_ligature_set_at_the_runs_end_marks_what_each_read() {
    let f = ligature_font(&[(F_F_I, &[F, I]), (F_I, &[I])], 0, &[]);
    let got = run(&f, "if", false);
    assert_eq!(got.glyphs, [I, F].map(|g| GlyphId(g.into())));
    assert_eq!(got.unsafe_to_concat, [false, true]);
}

// f, an acute the lookup skips as a mark, and i: f f i fails on i, marking all three, and f i forms over the mark.
#[test]
fn a_ligature_set_reads_past_a_skipped_mark() {
    let map = format4(&[(0x66, F), (0x69, I), (0x6C, L), (0x301, F_F_L)]);
    let extra = [("cmap", cmap(&[(3, 1, &map)])), ("GDEF", gdef(&[(F_F_L, F_F_L, 3)]))];
    let f = ligature_font(&[(F_F_I, &[F, I]), (F_I, &[I])], 8, &extra);
    let got = run(&f, "f\u{301}i", false);
    assert_eq!(got.glyphs, [F_I, F_F_L].map(|g| GlyphId(g.into())));
    assert_eq!(got.unsafe_to_concat, [true, true]);
}

type Ligature<'a> = (u16, &'a [u16]);

// On fi, a ligature taking i first leaves the rest untried; one skipped before a lone f's ligature marks f and i;
// one failing at the run's end and one skipped after it mark to the end.
#[test]
fn a_ligature_set_tries_its_ligatures_in_order() {
    let cases: [(&[Ligature], &[u16], &[bool]); 3] = [
        (&[(F_I, &[I]), (F_F_I, &[F, I])], &[F_I], &[false]),
        (&[(F_F_L, &[L]), (F_F_I, &[]), (F_I, &[I])], &[F_F_I, I], &[true, true]),
        (&[(F_F_I, &[I, L]), (F_F_L, &[F, L])], &[F, I], &[true, true]),
    ];
    for (i, (ligatures, glyphs, flags)) in cases.into_iter().enumerate() {
        let got = run(&ligature_font(ligatures, 0, &[]), "fi", false);
        let want = glyph_ids(glyphs);
        assert_eq!((got.glyphs.as_slice(), got.unsafe_to_concat.as_slice()), (&want[..], flags), "case {i}");
    }
}

// Sets past four ligatures, which their index can pass whole: none taking i, so f and i are marked; f i last, formed
// over the marks before it; f alone last, which the index can't rule out.
#[test]
fn a_long_ligature_set_is_passed_only_where_none_can_form() {
    let head: [Ligature; 4] = [(F_F_L, &[L]), (F_F_L, &[L, L]), (F_F_I, &[F]), (F_F_I, &[F, I])];
    let cases: [(Ligature, &[u16], &[bool]); 3] = [
        ((F_F_L, &[F, L]), &[F, I], &[true, true]),
        ((F_I, &[I]), &[F_I], &[true]),
        ((F_F_I, &[]), &[F_F_I, I], &[true, true]),
    ];
    for (i, (last, glyphs, flags)) in cases.into_iter().enumerate() {
        let ligatures: Vec<Ligature> = head.iter().copied().chain([last]).collect();
        let got = run(&ligature_font(&ligatures, 0, &[]), "fi", false);
        let want = glyph_ids(glyphs);
        assert_eq!((got.glyphs.as_slice(), got.unsafe_to_concat.as_slice()), (&want[..], flags), "case {i}");
    }
}

// A context rule: its backtrack, its input after the first glyph, its lookahead, and the lookup it runs at the first.
type ContextRule<'a> = (&'a [u16], &'a [u16], &'a [u16], u16);

// Glyphs, clusters and flags as [glyph=cluster#flags|...], where flag 1 is unsafe to break and 2 unsafe to concat.
fn serialized(r: &daegun::shape::ShapedRun) -> String {
    let glyphs = (0..r.glyphs.len()).map(|i| {
        let flags = u8::from(r.unsafe_to_break[i]) | (u8::from(r.unsafe_to_concat[i]) << 1);
        let flags = if flags == 0 { String::new() } else { format!("#{flags}") };
        format!("{}={}{flags}", r.glyphs[i], r.clusters[i])
    });
    format!("[{}]", glyphs.collect::<Vec<_>>().join("|"))
}

// f's rule set in a ccmp lookup of `kind` 5 or 6 and `format` 1 by glyph or 2 by class, where f, i and l are classes
// 1, 2 and 3, the values their glyphs have, so one rule reads alike in both. Lookup 1 makes f f_i, lookup 2 f_f_l.
fn rule_set_font(kind: u16, format: u16, rules: &[ContextRule]) -> daegun::Font {
    let rule = |&(back, input, ahead, nested): &ContextRule| {
        let count = input.len() as u16 + 1;
        if kind == 5 {
            return table(vec![w(&[count, 1]), w(input), w(&[0, nested])]);
        }
        let (back_len, ahead_len) = (back.len() as u16, ahead.len() as u16);
        table(vec![w(&[back_len]), w(back), w(&[count]), w(input), w(&[ahead_len]), w(ahead), w(&[1, 0, nested])])
    };
    let set = list(rules.iter().map(|r| to(rule(r))).collect());
    let class_def = || to(classes(&[(F, F, 1), (I, I, 2), (L, L, 3)]));
    let mut fields = vec![w(&[format]), to(coverage(&[F]))];
    match (kind, format) {
        (_, 1) => fields.extend([w(&[1]), to(set)]),
        (5, _) => fields.extend([class_def(), w(&[2]), NULL, to(set)]),
        _ => fields.extend([class_def(), class_def(), class_def(), w(&[2]), NULL, to(set)]),
    }
    let nested = |by: u16| lookup(1, 0, vec![to(single(F, by))]);
    let lookups = vec![lookup(kind, 0, vec![to(table(fields))]), nested(F_I), nested(F_F_L)];
    carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], lookups))])
}

const CHAIN_RULES: [ContextRule; 4] =
    [(&[], &[L], &[], 2), (&[], &[I], &[L], 2), (&[L], &[I], &[], 2), (&[], &[I], &[], 1)];
const CONTEXT_RULES: [ContextRule; 2] = [(&[], &[L], &[], 2), (&[], &[I], &[], 1)];

// f's rules failing on the second input, the lookahead or the backtrack, then matching, in chain and context lookups:
// whatever part fails, chained or not, the glyphs it read are unsafe to concatenate.
#[test]
fn a_rule_set_marks_each_failed_rule() {
    let chain = [
        ("fi", "[5=0#2|2=1#3]"),
        ("fii", "[5=0#2|2=1#3|2=2#2]"),
        ("fil", "[6=0#2|2=1#3|3=2#3]"),
        ("lfi", "[3=0|6=1#3|2=2#3]"),
        ("fl", "[6=0|3=1#3]"),
        ("ff", "[1=0#2|1=1#2]"),
        ("f", "[1=0#2]"),
        ("f\u{200D}i", "[5=0#2|2=4#3]"),
    ];
    let context = [
        ("fi", "[5=0#2|2=1#3]"),
        ("fii", "[5=0#2|2=1#3|2=2]"),
        ("fil", "[5=0#2|2=1#3|3=2]"),
        ("lfi", "[3=0|5=1#2|2=2#3]"),
        ("fl", "[6=0|3=1#3]"),
        ("ff", "[1=0#2|1=1#2]"),
        ("f", "[1=0#2]"),
        ("f\u{200D}i", "[5=0#2|2=4#3]"),
    ];
    let one_input: [ContextRule; 2] = [(&[], &[L], &[], 2), (&[], &[], &[], 1)];
    for format in [1, 2] {
        for (kind, rules, cases) in [(6, &CHAIN_RULES[..], &chain), (5, &CONTEXT_RULES[..], &context)] {
            let f = rule_set_font(kind, format, rules);
            for (text, want) in cases {
                assert_eq!(serialized(&run(&f, text, false)), *want, "kind {kind}, format {format}, {text:?}");
            }
        }
        for (kind, want, alone, one_rule_fi) in [
            (6, "[5=0#2|2=1#2]", "[5=0#2]", "[1=0#2|2=1#2]"),
            (5, "[5=0#2|2=1#2]", "[5=0#2]", "[1=0#2|2=1#2]"),
        ] {
            let f = rule_set_font(kind, format, &one_input);
            assert_eq!(serialized(&run(&f, "fi", false)), want, "one input, kind {kind}, format {format}");
            assert_eq!(serialized(&run(&f, "f", false)), alone, "one input alone, kind {kind}, format {format}");
            let one = rule_set_font(kind, format, &[(&[], &[L], &[], 2)]);
            for (text, want) in [("fi", one_rule_fi), ("fl", "[6=0|3=1#3]"), ("f\u{200D}l", "[6=0|3=4#3]")] {
                let got = serialized(&run(&one, text, false));
                assert_eq!(got, want, "one rule, kind {kind}, format {format}, {text:?}");
            }
            let none = rule_set_font(kind, format, &[]);
            assert_eq!(serialized(&run(&none, "fi", false)), "[1=0|2=1]", "no rules, kind {kind}, format {format}");
        }
    }
}

// One-input chain rules, which read lookahead first, on f, f i, f l and f before ZWJ or ZWNJ: whatever part fails, the
// glyphs it read are unsafe to concatenate.
#[test]
fn a_rule_of_one_input_glyph_fails_on_its_lookahead() {
    let texts = ["f", "fi", "fl", "fii", "ff", "f\u{200D}l", "f\u{200D}i", "f\u{200C}l", "f\u{200C}i"];
    let cases: [(&[ContextRule], [&str; 9]); 3] = [
        (
            &[(&[], &[], &[L], 2)],
            [
                "[1=0#2]", "[1=0#2|2=1#2]", "[6=0|3=1#3]", "[1=0#2|2=1#2|2=2]", "[1=0#2|1=1#2]", "[6=0|3=4#3]",
                "[1=0#2|2=4#2]", "[6=0|3=4#3]", "[1=0#2|2=4#2]",
            ],
        ),
        (
            &[(&[], &[], &[L], 2), (&[], &[], &[I], 1)],
            [
                "[1=0#2]", "[5=0#2|2=1#3]", "[6=0|3=1#3]", "[5=0#2|2=1#3|2=2]", "[1=0#2|1=1#2]", "[6=0|3=4#3]",
                "[5=0#2|2=4#3]", "[6=0|3=4#3]", "[5=0#2|2=4#3]",
            ],
        ),
        (
            &[(&[], &[I], &[], 1), (&[], &[], &[L], 2)],
            [
                "[1=0#2]", "[5=0|2=1#3]", "[6=0#2|3=1#3]", "[5=0|2=1#3|2=2]", "[1=0#2|1=1#2]", "[6=0#2|3=4#3]",
                "[5=0|2=4#3]", "[6=0#2|3=4#3]", "[1=0#2|2=4#2]",
            ],
        ),
    ];
    for format in [1, 2] {
        for (rules, wants) in &cases {
            let f = rule_set_font(6, format, rules);
            for (text, want) in texts.iter().zip(wants) {
                assert_eq!(serialized(&run(&f, text, false)), *want, "format {format}, {rules:?}, {text:?}");
            }
        }
    }
}

// A chain rule for f and l runs a context lookup no feature names, which runs an Extension lookup on f, its first
// subtable for i alone, and on l a single substitution whose coverage lists l, f and i out of order.
fn nested_records_font() -> daegun::Font {
    let chain = table(vec![w(&[3, 0, 1]), to(coverage(&[F, L])), w(&[0, 1, 0, 1])]);
    let rule_set = |nested: u16| list(vec![to(u16s(&[1, 1, 0, nested]))]);
    let context = table(vec![w(&[1]), to(coverage(&[F, L])), w(&[2]), to(rule_set(2)), to(rule_set(3))]);
    let extension = |subtable: Vec<u8>| table(vec![w(&[1, 1]), Field::To32(Some(subtable))]);
    let lookups = vec![
        lookup(6, 0, vec![to(chain)]),
        lookup(5, 0, vec![to(context)]),
        lookup(7, 0, vec![to(extension(single(I, L))), to(extension(single(F, F_I)))]),
        lookup(1, 0, vec![to(single2(coverage1(&[L, F, I]), &[F_F_L, F_F_I, F_I]))]),
    ];
    carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], lookups))])
}

// The lookups the chain rule reaches run through the plan's records for them: the Extension's first subtable passes
// f by, and the index of the misordered coverage misses l as its search does.
#[test]
fn lookups_reached_only_through_context_records_shape() {
    let f = nested_records_font();
    let cases = [("fl", &[F_I, L][..]), ("lf", &[L, F_I]), ("if", &[I, F_I]), ("fil", &[F_I, I, L]), ("ll", &[L, L])];
    for (text, want) in cases {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(want), "{text}");
    }
}

// f becomes f_i after i l and before i: two backtrack coverages and a lookahead coverage, each read through its own
// index.
#[test]
fn a_reverse_chain_reads_each_context_coverage_through_its_own_index() {
    let reverse = table(vec![
        w(&[1]),
        to(coverage(&[F])),
        w(&[2]),
        to(coverage(&[L])),
        to(coverage(&[I])),
        w(&[1]),
        to(coverage(&[I])),
        w(&[1, F_I]),
    ]);
    let f = carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0])], vec![lookup(8, 0, vec![to(reverse)])]))]);
    let cases = [
        ("ilfi", &[I, L, F_I, I][..]),
        ("lfi", &[L, F, I]),
        ("ilfl", &[I, L, F, L]),
        ("flfi", &[F, L, F, I]),
        ("ilfilfi", &[I, L, F_I, I, L, F_I, I]),
    ];
    for (text, want) in cases {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(want), "{text}");
    }
}

// f's ligatures across a ZWJ: naming it then l or i i, skipping it for l l or i, f and the ZWJ alone, an indexed set
// with x i; each fails after the ZWJ (at x), further in or both sides, two ZWJs undecided.
#[test]
fn a_ligature_set_rules_out_its_ligatures_across_a_joiner() {
    let map = format4(&[(0x66, F), (0x69, I), (0x6C, L), (0x78, F_I), (0x79, F_F_I), (0x200D, F_F_L)]);
    let four: [(u16, &[u16]); 4] = [(F_F_I, &[F_F_L, L]), (F_F_I, &[F_F_L, I, I]), (F_I, &[L, L]), (F_I, &[I])];
    let five = [four.as_slice(), &[(F_F_I, &[F_F_L])]].concat();
    let seven = [four.as_slice(), &[(F_F_I, &[F_I, I]), (F_I, &[F_F_L, L, I]), (F_I, &[F_F_L, F_F_I])]].concat();
    let cases = [
        ("f\u{200D}l", ["[4=0]", "[4=0]", "[4=0]"]),
        ("f\u{200D}ii", ["[4=0#2]", "[4=0#2]", "[4=0#2]"]),
        ("f\u{200D}il", ["[5=0#2|3=5#2]", "[5=0#2|3=5#2]", "[5=0#2|3=5#2]"]),
        ("f\u{200D}ll", ["[4=0|3=5]", "[4=0|3=5]", "[4=0|3=5]"]),
        ("f\u{200D}f", ["[1=0#2|1=4#2]", "[4=0#2|1=4#2]", "[1=0#2|1=4#2]"]),
        ("f\u{200D}\u{200D}i", ["[5=0#2]", "[5=0#2]", "[5=0#2]"]),
        ("f\u{200D}", ["[1=0#2]", "[4=0#2]", "[1=0#2]"]),
        ("f\u{200D}lf", ["[4=0|1=5#2]", "[4=0|1=5#2]", "[4=0|1=5#2]"]),
        ("f\u{200D}x", ["[1=0#2|5=4#2]", "[4=0#2|5=4#2]", "[1=0#2|5=4#2]"]),
        ("f\u{200D}xi", ["[1=0#2|5=4#2|2=5]", "[4=0#2|5=4#2|2=5]", "[4=0#2]"]),
        ("f\u{200D}li", ["[4=0|2=5]", "[4=0|2=5]", "[4=0|2=5]"]),
        ("f\u{200D}i", ["[5=0#2]", "[5=0#2]", "[5=0#2]"]),
        ("fli", ["[1=0#2|3=1#2|2=2#2]", "[1=0#2|3=1#2|2=2#2]", "[1=0#2|3=1#2|2=2#2]"]),
        ("f\u{200D}y", ["[1=0#2|4=4#2]", "[4=0#2|4=4#2]", "[5=0#2]"]),
        ("f\u{200D}yi", ["[1=0#2|4=4#2|2=5]", "[4=0#2|4=4#2|2=5]", "[5=0#2|2=5]"]),
    ];
    for (which, ligatures) in [four.as_slice(), five.as_slice(), seven.as_slice()].into_iter().enumerate() {
        let f = ligature_font(ligatures, 0, &[("cmap", cmap(&[(3, 1, &map)]))]);
        for (text, want) in cases {
            assert_eq!(serialized(&run(&f, text, false)), want[which], "{} ligatures, {text:?}", ligatures.len());
        }
    }
}

// A chain rule set on f of 1,000 rules sharing one, whose lookahead is 299 more f and an i. Over 600 f, each try reads
// up to 300 glyphs and fails, so the run spends its budget on what it reads and stops. Charged one a try, it would read
// some 90 million glyphs and call the run complete.
#[test]
fn a_rule_s_reads_are_charged_to_the_run() {
    let rules = 1_000;
    let rule = [u16s(&[0, 1, 300]), u16s(&[F; 299]), u16s(&[I, 0])].concat();
    let set = [u16s(&[rules]), u16s(&vec![2 + 2 * rules; usize::from(rules)]), rule].concat();
    let chain = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let f = carets_with(&[("GSUB", layout_lookup(b"ccmp", 6, &[chain]))]);
    assert!(!f.shape(&"f".repeat(600), &Location::default(), &ShapeOptions::default()).complete);
}

// An acute, 300 l, an acute and 300 l: a set on the first acute whose every entry reads past the l to the second acute
// and on to the end. Past the run's budget the tries left come back unread, so 30,000 of them cost what 5,000 do.
#[test]
fn a_set_s_tries_past_a_failed_run_read_nothing() {
    let text = format!("\u{301}{}\u{301}{}", "l".repeat(300), "l".repeat(300));
    let shaped = |n: u16| {
        let set = [u16s(&[n]), u16s(&vec![2 + 2 * n; usize::from(n)]), u16s(&[F_I, 3, 0, 0])].concat();
        let subtable = table(vec![w(&[1]), to(coverage(&[0])), w(&[1]), to(set)]);
        let f = carets_with(&[("GSUB", aliased(&[(b"ccmp", 0..1)], 1, repeated(4, 2, 1, subtable), vec![]))]);
        let started = std::time::Instant::now();
        assert!(!complete(&f, &text), "{n} tries");
        started.elapsed()
    };
    let fastest = |n| (0..3).map(|_| shaped(n)).min().expect("three runs");
    let (few, many) = (fastest(5_000), fastest(30_000));
    assert!(many < few * 3, "30,000 tries took {many:?}, 5,000 took {few:?}");
}

fn complete(f: &daegun::Font, text: &str) -> bool {
    f.shape(text, &Location::default(), &ShapeOptions::default()).complete
}

// f's ligature set names one ligature throughout: on f alone 90,000 of two glyphs fail unread, and on f, 62 i and l,
// 10,000 of f and 63 i each read to the l. The run pays for every try and the 640,000 glyphs they read.
#[test]
fn a_ligature_sets_tries_are_charged_to_the_run() {
    let set_of = |n: u16, ligature: Vec<u16>| {
        let set = [u16s(&[n]), u16s(&vec![2 + 2 * n; usize::from(n)]), u16s(&ligature)].concat();
        let subtable = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
        carets_with(&[("GSUB", aliased(&[(b"ccmp", 0..1)], 1, repeated(4, 0, n, subtable), vec![]))])
    };
    assert!(!complete(&set_of(300, vec![F_I, 2, I]), "f"));
    let long = [vec![F_I, 64], vec![I; 63]].concat();
    assert!(!complete(&set_of(100, long), &format!("f{}l", "i".repeat(62))));
}

// f and i ligate over an acute; then a rule set on an acute, of 6,000 entries naming a rule of two acutes, reads past
// 200 l and the ligature, all hidden, to the second acute, which sits on the ligature and fails each try unmarked.
#[test]
fn an_input_refused_on_another_ligatures_component_pays_for_its_read() {
    let ligature = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(list(vec![to(u16s(&[F_I, 2, I]))]))]);
    let rules = 6_000;
    let set = [u16s(&[rules]), u16s(&vec![2 + 2 * rules; usize::from(rules)]), u16s(&[2, 0, 0])].concat();
    let context = table(vec![w(&[1]), to(coverage(&[0])), w(&[1]), to(set)]);
    let lookups = vec![lookup(4, 8, vec![to(ligature)]), lookup(5, 2 | 4, vec![to(context)])];
    let f = carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &[0, 1])], lookups))]);
    assert!(!complete(&f, &format!("\u{301}{}f\u{301}i", "l".repeat(200))));
}

// An acute before 200 l: 6,000 lookups that skip the l, a rule set or a ligature set on the acute, each read past them
// all, which the run pays for. A lookup of 6,000 such rule sets reads past them once.
#[test]
fn a_sets_read_past_skipped_glyphs_is_paid_for() {
    let context = table(vec![w(&[1]), to(coverage(&[0])), w(&[1]), to(u16s(&[0]))]);
    let ligature = table(vec![w(&[1]), to(coverage(&[0])), w(&[1]), to(list(vec![to(u16s(&[F_I, 2, 0]))]))]);
    let text = format!("\u{301}{}", "l".repeat(200));
    for (kind, subtable) in [(5, context.clone()), (4, ligature)] {
        let lookups = aliased(&[(b"ccmp", 0..6_000)], 6_000, repeated(kind, 2, 1, subtable), vec![]);
        assert!(!complete(&carets_with(&[("GSUB", lookups)]), &text), "type {kind}");
    }
    let subtables = aliased(&[(b"ccmp", 0..1)], 1, repeated(5, 2, 6_000, context), vec![]);
    assert!(complete(&carets_with(&[("GSUB", subtables)]), &text), "one lookup of rule sets");
}

// An Extension lookup of mixed types, a single substitution of f into l then a ligature, run by a context's record
// on f: it applies nothing, whether the plan indexed it or the font's allowance was spent and it wasn't.
#[test]
fn a_nested_lookup_of_mixed_types_applies_nothing_however_indexed() {
    let context = table(vec![w(&[3, 1, 1]), to(coverage(&[F])), w(&[0, 1])]);
    let extension = |kind: u16, subtable: Vec<u8>| to(table(vec![w(&[1, kind]), Field::To32(Some(subtable))]));
    let ligature = table(vec![w(&[1]), to(coverage(&[I])), w(&[1]), to(list(vec![to(u16s(&[F_I, 2, F]))]))]);
    let mixed = lookup(7, 0, vec![extension(1, single(F, L)), extension(4, ligature)]);
    let gsub = layout(b"DFLT", &[(b"ccmp", &[0])], vec![lookup(5, 0, vec![to(context)]), mixed]);
    for allowance in [None, Some(0)] {
        let f = carets_with(&[("GSUB", gsub.clone())]);
        allowance.inspect(|&bytes| f.set_index_allowance(bytes));
        let got = f.shape("f", &Location::default(), &ShapeOptions::default());
        assert_eq!(got.glyphs, glyph_ids(&[F]), "allowance {allowance:?}");
    }
}

// 1,000 lookups of a feature asked for past the text name one Extension lookup of 2,000 subtables, and a later global
// feature makes f an i. The plan's work pays for telling each lookup's type; past it, the lookups left apply nothing.
#[test]
fn a_plan_pays_for_telling_extension_lookups_types() {
    let extension = table(vec![w(&[1, 1]), Field::To32(Some(single(F_F_L, F_F_L)))]);
    let (shared, f_to_i) = (repeated(7, 0, 2_000, extension), lookup(1, 0, vec![to(single(F, I))]));
    let gsub = aliased(&[(b"zz01", 0..1_000), (b"zz02", 1_000..1_001)], 1_000, shared, vec![f_to_i]);
    let features = ["zz01[100:101]", "zz02"].map(|f| f.parse().expect("a feature"));
    let options = ShapeOptions::default().with_features(&features);
    let got = carets_with(&[("GSUB", gsub)]).shape("f", &Location::default(), &options);
    assert_eq!(got.glyphs, glyph_ids(&[F]));
}

// 50 lookups name one Extension lookup of 200 subtables, each an Extension naming another, which reads as none. With
// the font's allowance spent they go unindexed, and the run pays for every subtable tried at each of 100 f.
#[test]
fn a_subtable_that_reads_as_none_still_costs_its_try() {
    let gsub = aliased(&[(b"ccmp", 0..50)], 50, repeated(7, 0, 200, u16s(&[1, 7, 0, 8])), vec![]);
    let f = carets_with(&[("GSUB", gsub)]);
    f.set_index_allowance(0);
    assert!(!complete(&f, &"f".repeat(100)));
}
