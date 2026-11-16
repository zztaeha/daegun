use super::aat_shaping::{cmap, gdef};
use super::aat_subset::{carets_with, F, F_F_I, F_F_L, I, L};
use super::gpos_lookups::{cursive_pos, marks, Anchor};
use super::layout_build::{anchor, coverage, layout, list, lookup, run, table, to, w};

const RIGHT_TO_LEFT: u16 = 0x0001;
const IGNORE_MARKS: u16 = 0x0008;

// f, i and l, with Hebrew alef as f and sheva as i, so a run of those goes right to left.
fn hebrew() -> (&'static str, Vec<u8>) {
    ("cmap", cmap(&[(0x66, 0x66, F), (0x69, 0x69, I), (0x6C, 0x6C, L), (0x5B0, 0x5B0, I), (0x5D0, 0x5D0, F)]))
}

// i a mark that attaches its origin at f's (300, 500), under `script`: Hebrew takes GPOS only under its own.
fn mark_on_f(script: &[u8; 4]) -> Vec<(&'static str, Vec<u8>)> {
    let (marks, bases) = (to(marks(&[(0, Some((0, 0)))])), to(list(vec![to(anchor(300, 500))])));
    let subtable = table(vec![w(&[1]), to(coverage(&[I])), to(coverage(&[F])), w(&[1]), marks, bases]);
    let gpos = layout(script, &[(b"mark", &[0])], vec![lookup(4, 0, vec![to(subtable)])]);
    vec![("GDEF", gdef(&[(I, I, 3)])), ("GPOS", gpos), hebrew()]
}

const JOIN: (Anchor, Anchor) = (Some((0, 0)), Some((500, 50)));

// Cursive lookups over f under `script`, each with its flags, entering at the origin and leaving at (500, 50).
fn joined(script: &[u8; 4], flags: &[u16]) -> Vec<(&'static str, Vec<u8>)> {
    let lookups = flags.iter().map(|&f| lookup(3, f, vec![to(cursive_pos(&[F], &[JOIN]))])).collect();
    let indices: Vec<u16> = (0..flags.len() as u16).collect();
    vec![("GPOS", layout(script, &[(b"curs", &indices)], lookups)), hebrew()]
}

// The f joined by a lookup flagged right to left, then l joined to the first f by an unflagged one.
fn rejoined() -> Vec<(&'static str, Vec<u8>)> {
    let head = cursive_pos(&[F, L], &[(JOIN.0, None), (None, JOIN.1)]);
    let lookups = vec![lookup(3, RIGHT_TO_LEFT, vec![to(cursive_pos(&[F], &[JOIN]))]), lookup(3, 0, vec![to(head)])];
    vec![("GPOS", layout(b"DFLT", &[(b"curs", &[0, 1])], lookups))]
}

// Fifteen f make f_f_i, then nineteen f_f_i past marks make f_f_l, and l, a mark, attaches to the component its
// ligature count gives, each of sixteen 100 further along.
fn ligated() -> Vec<(&'static str, Vec<u8>)> {
    let ligature = |first: u16, rest: &[u16], glyph: u16| {
        let lig = table(vec![w(&[glyph, rest.len() as u16 + 1]), w(rest)]);
        table(vec![w(&[1]), to(coverage(&[first])), w(&[1]), to(list(vec![to(lig)]))])
    };
    let lookups = vec![
        lookup(4, 0, vec![to(ligature(F, &[F; 14], F_F_I))]),
        lookup(4, IGNORE_MARKS, vec![to(ligature(F_F_I, &[F_F_I; 18], F_F_L))]),
    ];
    let gsub = layout(b"DFLT", &[(b"liga", &[0, 1])], lookups);
    let attach = list((1..=285).map(|c| to(anchor(10 * c, 0))).collect());
    let fields = vec![w(&[1]), to(coverage(&[L])), to(coverage(&[F_F_L])), w(&[1]), to(marks(&[(0, Some((0, 0)))]))];
    let mark_lig = table(fields.into_iter().chain([to(list(vec![to(attach)]))]).collect());
    let gpos = layout(b"DFLT", &[(b"mark", &[0])], vec![lookup(5, 0, vec![to(mark_lig)])]);
    vec![("GSUB", gsub), ("GPOS", gpos), ("GDEF", gdef(&[(L, L, 3), (F_F_I, F_F_I, 2), (F_F_L, F_F_L, 2)]))]
}

// Eleven lookups each adding 32,767 to f's advance, then a context rule running the first 65,535 times more.
fn widened() -> Vec<(&'static str, Vec<u8>)> {
    let wider = || lookup(1, 0, vec![to(table(vec![w(&[1]), to(coverage(&[F])), w(&[4, 32_767])]))]);
    let rule = table(vec![w(&[1, 65_535]), w(&[0; 2 * 65_535])]);
    let context = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(list(vec![to(rule)]))]);
    let mut lookups: Vec<Vec<u8>> = (0..11).map(|_| wider()).collect();
    lookups.push(lookup(7, 0, vec![to(context)]));
    let indices: Vec<u16> = (0..12).collect();
    vec![("GPOS", layout(b"DFLT", &[(b"kern", &indices)], lookups))]
}

// i, a mark, after f, and Hebrew sheva after alef, 40,000 and 70,000 times: the 32,768th still attaches, as an i16
// holds its distance, and those further keep their place.
#[test]
fn a_mark_further_than_an_i16_reaches_keeps_its_place() {
    for n in [40_000, 70_000] {
        let latin = run(&carets_with(&mark_on_f(b"DFLT")), &format!("f{}", "i".repeat(n)), false);
        let mut want = vec![(-100.0, 500.0); n + 1];
        want[0] = (0.0, 0.0);
        want[32_769..].fill((0.0, 0.0));
        assert_eq!(crate::xy(&latin.offsets), want, "{n} after f");

        let hebrew = run(&carets_with(&mark_on_f(b"hebr")), &format!("\u{5D0}{}", "\u{5B0}".repeat(n)), false);
        let mut want = vec![(0.0, 0.0); n + 1];
        want[n - 32_768..n].fill((300.0, 500.0));
        assert_eq!(crate::xy(&hebrew.offsets), want, "{n} after alef");
    }
}

// A hundred f joined against the run's order: each moves only by the joins as deep as lookups nest, so the first
// sits 65 up and the 66th starts over. Alef right to left, unflagged, as well.
#[test]
fn a_cursive_chain_moves_a_glyph_as_deep_as_lookups_nest() {
    let depth = |k: usize| 50.0 * if k <= 64 { 65 - k } else { 99 - k } as f64;
    let latin = run(&carets_with(&joined(b"DFLT", &[RIGHT_TO_LEFT])), &"f".repeat(100), false);
    assert_eq!(crate::xy(&latin.offsets), (0..100).map(|k| (0.0, -depth(k))).collect::<Vec<_>>());
    let hebrew = run(&carets_with(&joined(b"hebr", &[0])), &"\u{5D0}".repeat(100), false);
    let x = |k: usize| if k == 0 { 0.0 } else { -500.0 };
    assert_eq!(crate::xy(&hebrew.offsets), (0..100).map(|k| (x(k), depth(k))).collect::<Vec<_>>());
}

// l joined to the first of those f by an unflagged lookup, which reverses the chain as deep as lookups nest: 66 f
// hang from l, 50 apart, and the rest from the last f.
#[test]
fn rejoining_a_cursive_chain_reverses_it_as_deep_as_lookups_nest() {
    let joined = run(&carets_with(&rejoined()), &format!("l{}", "f".repeat(100)), false);
    let y = |k: usize| if k <= 66 { 50.0 * k as f64 } else { -50.0 * (100 - k) as f64 };
    assert_eq!(crate::xy(&joined.offsets), (0..101).map(|k| (0.0, y(k))).collect::<Vec<_>>());
}

// l between the 18th and 19th f_f_i of an f_f_l, 15 components each, takes component 270 of 285, and before the 18th,
// 255: it attaches at that component's anchor, 10 units each, less the f_f_l's 900 advance.
#[test]
fn a_mark_in_a_ligature_of_ligatures_counts_every_component() {
    let f = carets_with(&ligated());
    let mark = |before: usize, after: usize| run(&f, &format!("{}l{}", "f".repeat(before), "f".repeat(after)), false);
    assert_eq!(crate::xy(&mark(270, 15).offsets), [(0.0, 0.0), (1800.0, 0.0)]);
    assert_eq!(crate::xy(&mark(255, 30).offsets), [(0.0, 0.0), (1650.0, 0.0)]);
}

// Each f's advance passes i32::MAX and stops there. The 65,535 records of each f's rule cost two of the run's
// budget apiece, a lookup entered and its subtable tried, so 64 i's after f or ff give the run room for them.
#[test]
fn an_advance_saturates_at_the_i32_limit() {
    let f = carets_with(&widened());
    let i = "i".repeat(64);
    assert_eq!(run(&f, &format!("f{i}"), false).advances[..1], [f64::from(i32::MAX)]);
    assert_eq!(run(&f, &format!("ff{i}"), false).advances[..2], [f64::from(i32::MAX); 2]);
}
