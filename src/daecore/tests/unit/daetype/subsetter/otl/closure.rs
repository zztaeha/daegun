use super::*;
use alloc::vec;

fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn u24s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()[1..].to_vec()).collect()
}

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn coverage(glyphs: &[u16]) -> Vec<u8> {
    [u16s(&[1, glyphs.len() as u16]), u16s(glyphs)].concat()
}

// SingleSubst 2 of (from, to) pairs.
fn single(pairs: &[(u16, u16)]) -> Vec<u8> {
    let n = pairs.len() as u16;
    let to: Vec<u16> = pairs.iter().map(|p| p.1).collect();
    [u16s(&[2, 6 + 2 * n, n]), u16s(&to), coverage(&pairs.iter().map(|p| p.0).collect::<Vec<_>>())].concat()
}

fn gsub(lookups: &[(u16, Vec<u8>)]) -> Vec<u8> {
    gsub_naming(0, lookups)
}

// A GSUB of one script and one feature naming lookup `named`, each lookup of one subtable.
fn gsub_naming(named: u16, lookups: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let features = [u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, named])].concat();
    let bodies: Vec<Vec<u8>> = lookups.iter().map(|(kind, sub)| [u16s(&[*kind, 0, 1, 8]), sub.clone()].concat())
        .collect();
    let mut list = u16s(&[bodies.len() as u16]);
    let mut at = 2 + 2 * bodies.len();
    for body in &bodies {
        list.extend(u16s(&[at as u16]));
        at += body.len();
    }
    let head = u16s(&[1, 0, 10, 10 + scripts.len() as u16, 10 + (scripts.len() + features.len()) as u16]);
    [head, scripts, features, list, bodies.concat()].concat()
}

// `seed` closed over `table`'s GSUB, in a font of `glyphs` glyphs.
fn closed(table: &[u8], seed: &[u32], glyphs: u32) -> Vec<u32> {
    let mut set = GlyphSet::new();
    seed.iter().for_each(|&g| {
        set.insert(g);
    });
    assert!(close(table, &feature_lookups(table).expect("within the budget"), &mut set, glyphs, budget(table)));
    set.iter().collect()
}

// Values of `w` bytes each.
fn uints(w: usize, v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()[4 - w..].to_vec()).collect()
}

// Coverage 1, or 3 of 24-bit glyphs.
fn cover(w: usize, glyphs: &[u32]) -> Vec<u8> {
    [u16s(&[w as u16 * 2 - 3]), uints(w, &[glyphs.len() as u32]), uints(w, glyphs)].concat()
}

// ClassDef 2, or 4 of 24-bit glyphs, of (first, last, class) ranges.
fn class_def(w: usize, ranges: &[(u32, u32, u16)]) -> Vec<u8> {
    let mut def = [u16s(&[w as u16 * 2 - 2]), uints(w, &[ranges.len() as u32])].concat();
    ranges.iter().for_each(|&(first, last, class)| def.extend([uints(w, &[first, last]), u16s(&[class])].concat()));
    def
}

// `parts` laid out from `at`: their offsets, an empty part's 0, and their bytes.
fn laid(at: usize, parts: &[Vec<u8>]) -> (Vec<u32>, Vec<u8>) {
    let (mut at, mut offsets) = (at, Vec::new());
    for part in parts {
        offsets.push(if part.is_empty() { 0 } else { at as u32 });
        at += part.len();
    }
    (offsets, parts.concat())
}

fn records(records: &[(u16, u16)]) -> Vec<u8> {
    records.iter().flat_map(|&(seq, lookup)| u16s(&[seq, lookup])).collect()
}

// A rule set, or a ligature set, of `rules` at `o`-byte offsets.
fn rule_set(o: usize, rules: &[Vec<u8>]) -> Vec<u8> {
    let (offsets, body) = laid(2 + o * rules.len(), rules);
    [u16s(&[rules.len() as u16]), uints(o, &offsets), body].concat()
}

// A context rule of `w`-byte values, its first place left out.
fn rule(w: usize, rest: &[u32], r: &[(u16, u16)]) -> Vec<u8> {
    [u16s(&[rest.len() as u16 + 1, r.len() as u16]), uints(w, rest), records(r)].concat()
}

fn chained(w: usize, back: &[u32], rest: &[u32], ahead: &[u32], r: &[(u16, u16)]) -> Vec<u8> {
    let counted = |n: usize, v: &[u32]| [u16s(&[n as u16]), uints(w, v)].concat();
    [counted(back.len(), back), counted(rest.len() + 1, rest), counted(ahead.len(), ahead), counted(r.len(), &[]),
        records(r)].concat()
}

// A subtable of a set for each covered glyph, in `w`-byte glyphs, counts and offsets, its coverage at a long
// offset: MultipleSubst, AlternateSubst and LigatureSubst 1 or 2, Context and ChainContext 1 or 4.
fn covered_sets(format: u16, w: usize, covered: &[u32], sets: &[Vec<u8>]) -> Vec<u8> {
    let head = 2 + long(w) + w + w * sets.len();
    let cov = cover(w, covered);
    let (offsets, body) = laid(head + cov.len(), sets);
    let counted = [uints(w, &[sets.len() as u32]), uints(w, &offsets)].concat();
    [u16s(&[format]), uints(long(w), &[head as u32]), counted, cov, body].concat()
}

fn sequences(w: usize, covered: &[u32], lists: &[&[u32]]) -> Vec<u8> {
    let lists: Vec<Vec<u8>> = lists.iter().map(|l| [u16s(&[l.len() as u16]), uints(w, l)].concat()).collect();
    covered_sets(w as u16 - 1, w, covered, &lists)
}

// SingleSubst 2 or 4.
fn substitutes(w: usize, pairs: &[(u32, u32)]) -> Vec<u8> {
    let head = 2 + long(w) + w + w * pairs.len();
    let (from, to): (Vec<u32>, Vec<u32>) = pairs.iter().copied().unzip();
    let counted = [uints(w, &[pairs.len() as u32]), uints(w, &to)].concat();
    [u16s(&[w as u16 * 2 - 2]), uints(long(w), &[head as u32]), counted, cover(w, &from)].concat()
}

// Context 2 or 5 with one ClassDef, ChainContext 2 or 5 with three: a set of 16-bit class rules for each class,
// none for an empty one. ChainContext counts its sets in 16 bits.
fn by_class(format: u16, w: usize, covered: &[u32], defs: &[Vec<u8>], sets: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let count = if defs.len() == 3 { 2 } else { w };
    let head = 2 + long(w) * (1 + defs.len()) + count + w * sets.len();
    let cov = cover(w, covered);
    let (def_at, defs) = laid(head + cov.len(), defs);
    let sets: Vec<Vec<u8>> =
        sets.iter().map(|rules| if rules.is_empty() { Vec::new() } else { rule_set(w, rules) }).collect();
    let (set_at, sets) = laid(head + cov.len() + defs.len(), &sets);
    let counted = [uints(count, &[set_at.len() as u32]), uints(w, &set_at)].concat();
    [u16s(&[format]), uints(long(w), &[head as u32]), uints(long(w), &def_at), counted, cov, defs, sets].concat()
}

// Context 3 or 6: a coverage for each place, at `w`-byte offsets.
fn by_coverage(w: usize, places: &[&[u32]], r: &[(u16, u16)]) -> Vec<u8> {
    let covs: Vec<Vec<u8>> = places.iter().map(|g| cover(w, g)).collect();
    let (offsets, body) = laid(6 + w * places.len() + 4 * r.len(), &covs);
    [u16s(&[w as u16 * 3 - 3, places.len() as u16, r.len() as u16]), uints(w, &offsets), records(r), body].concat()
}

// ReverseChainSingleSubst 1 or 2 of one backtrack and one lookahead coverage.
fn reverse(w: usize, covered: &[u32], back: &[u32], ahead: &[u32], to: &[u32]) -> Vec<u8> {
    let head = 2 + long(w) + 2 * (2 + w) + w * (1 + to.len());
    let (at, body) = laid(head, &[cover(w, covered), cover(w, back), cover(w, ahead)]);
    let context = [u16s(&[1]), uints(w, &at[1..2]), u16s(&[1]), uints(w, &at[2..])].concat();
    [u16s(&[w as u16 - 1]), uints(long(w), &at[..1]), context, uints(w, &[to.len() as u32]), uints(w, to), body]
        .concat()
}

// Glyph, class and chain rules, and a rule after a MultipleSubst: each nested lookup closes only over the glyphs
// its rule's places can hold.
#[test]
fn a_context_closes_its_nested_lookups_where_its_rule_can_match() {
    let rule_set = u16s(&[1, 4, 2, 1, 2, 1, 1]);
    let context = [u16s(&[1, 8, 1, 14]), coverage(&[1]), rule_set].concat();
    let t = gsub(&[(5, context), (1, single(&[(2, 10), (3, 11)]))]);
    assert_eq!(closed(&t, &[1], 20), [1]);
    assert_eq!(closed(&t, &[1, 2, 3], 20), [1, 2, 3, 10]);

    let classes = u16s(&[2, 2, 1, 1, 1, 2, 2, 2]);
    let context = [u16s(&[2, 12, 18, 2, 0, 34]), coverage(&[1]), classes, u16s(&[1, 4, 2, 1, 0, 1, 1])].concat();
    let t = gsub(&[(5, context), (1, [u16s(&[1, 6, 10]), coverage(&[3, 4, 6])].concat())]);
    assert_eq!(closed(&t, &[1, 3], 20), [1, 3, 13]);
    assert_eq!(closed(&t, &[1, 2, 3, 5], 20), [1, 2, 3, 5, 13]);

    let chain = [u16s(&[3, 1, 20, 1, 26, 1, 32, 1, 0, 1]), coverage(&[4]), coverage(&[1]), coverage(&[2])].concat();
    let t = gsub(&[(6, chain), (1, single(&[(1, 9)]))]);
    assert_eq!(closed(&t, &[1, 2], 20), [1, 2]);
    assert_eq!(closed(&t, &[1, 2, 4], 20), [1, 2, 4, 9]);

    // After a MultipleSubst at the first place, the second place takes the whole set: 3 reaches 9.
    let context = [u16s(&[3, 2, 2, 18, 24, 0, 1, 1, 2]), coverage(&[1]), coverage(&[2])].concat();
    let multiple = [u16s(&[1, 8, 1, 14]), coverage(&[1]), u16s(&[2, 5, 6])].concat();
    let t = gsub(&[(5, context), (2, multiple), (1, single(&[(2, 8), (3, 9)]))]);
    assert_eq!(closed(&t, &[1, 2, 3], 20), [1, 2, 3, 5, 6, 8, 9]);
}

#[test]
fn substitutions_close_over_the_glyphs_they_can_make() {
    let ligature = [u16s(&[1, 8, 1, 14]), coverage(&[1]), u16s(&[1, 4, 12, 2, 2])].concat();
    let t = gsub(&[(4, ligature)]);
    assert_eq!((closed(&t, &[1], 20), closed(&t, &[1, 2], 20)), (vec![1], vec![1, 2, 12]));

    // A delta mapping a run back into itself closes a glyph a pass, to one past the run.
    let t = gsub(&[(1, [u16s(&[1, 6, 1]), coverage(&[1, 2, 3])].concat())]);
    let runs = [closed(&t, &[1], 20), closed(&t, &[1, 2], 20), closed(&t, &[1, 3], 20)];
    assert_eq!(runs, [[1, 2, 3, 4]; 3]);

    // Lookup 1, which no feature names, takes 1 to nothing.
    let t = gsub(&[(1, single(&[(1, 5)])), (1, single(&[(1, 6)]))]);
    assert_eq!(closed(&t, &[1], 20), [1, 5]);
}

// A GSUB of four SingleSubsts and `features`, each a tag and its lookups, under one script whose language
// systems are each a required feature and feature indices, the first its default; 1.1 where `variations` follow.
fn collected(systems: &[(u16, Vec<u16>)], features: &[(&[u8; 4], &[u16])], variations: &[u8]) -> Vec<u16> {
    collected_from(&[systems.to_vec()], features, variations)
}

// As `collected`, over `scripts`, each its language systems; an empty one has no default and no tagged system.
fn collected_from(
    scripts: &[Vec<(u16, Vec<u16>)>], features: &[(&[u8; 4], &[u16])], variations: &[u8],
) -> Vec<u16> {
    let system = |(required, indices): &(u16, Vec<u16>)| {
        [u16s(&[0, *required, indices.len() as u16]), u16s(indices)].concat()
    };
    let script = |systems: &[(u16, Vec<u16>)]| {
        let Some((default, tagged)) = systems.split_first() else { return u16s(&[0, 0]) };
        let at = 4 + 6 * tagged.len();
        let (mut head, mut body) = (u16s(&[at as u16, tagged.len() as u16]), system(default));
        for (i, s) in tagged.iter().enumerate() {
            head.extend([format!("{i:04}").into_bytes(), u16s(&[(at + body.len()) as u16])].concat());
            body.extend(system(s));
        }
        [head, body].concat()
    };
    let (mut list, mut bodies) = (u16s(&[scripts.len() as u16]), Vec::new());
    for (i, s) in scripts.iter().enumerate() {
        let at = 2 + 6 * scripts.len() + bodies.len();
        list.extend([format!("S{i:03}").into_bytes(), u16s(&[at as u16])].concat());
        bodies.extend(script(s));
    }
    let scripts = [list, bodies].concat();
    let (mut list, mut bodies) = (u16s(&[features.len() as u16]), Vec::new());
    for (tag, lookups) in features {
        list.extend([tag.to_vec(), u16s(&[(2 + 6 * features.len() + bodies.len()) as u16])].concat());
        bodies.extend([u16s(&[0, lookups.len() as u16]), u16s(lookups)].concat());
    }
    let features = [list, bodies].concat();
    let singles = (0..4).flat_map(|i| u16s(&[1, 0, 1, 8, 1, 6, 1 + i, 1, 1, 1]));
    let lookups = [u16s(&[4, 10, 30, 50, 70]), singles.collect()].concat();
    let head = if variations.is_empty() { 10 } else { 14 };
    let (f_at, l_at) = (head + scripts.len(), head + scripts.len() + features.len());
    let offsets = u16s(&[head as u16, f_at as u16, l_at as u16]);
    let header = match variations.is_empty() {
        true => [u32s(&[0x1_0000]), offsets].concat(),
        false => [u32s(&[0x1_0001]), offsets, u32s(&[(l_at + lookups.len()) as u32])].concat(),
    };
    feature_lookups(&[header, scripts, features, lookups, variations.to_vec()].concat()).expect("within the budget")
}

// Feature variations, 1.1 where `lookup_records` has any: each record substituting a feature by an alternate's
// lookups under no condition, then each lookup variation's lists for a feature.
fn variations(records: &[(u16, &[u16])], lookup_records: &[(u16, &[&[u16]])]) -> Vec<u8> {
    let v11 = !lookup_records.is_empty();
    let mut at = 8 + 8 * records.len() + if v11 { 4 + 6 * lookup_records.len() } else { 0 };
    let (mut head, mut bodies) = (u32s(&[if v11 { 0x1_0001 } else { 0x1_0000 }, records.len() as u32]), Vec::new());
    for &(feature, lookups) in records {
        let substitution =
            [u32s(&[0x1_0000]), u16s(&[1, feature]), u32s(&[12]), u16s(&[0, lookups.len() as u16]), u16s(lookups)];
        head.extend(u32s(&[0, at as u32]));
        at += substitution.concat().len();
        bodies.extend(substitution.concat());
    }
    if v11 {
        head.extend(u32s(&[lookup_records.len() as u32]));
        for &(feature, lists) in lookup_records {
            let (mut table, mut tail) = ([u16s(&[1, 0, 0]), u32s(&[lists.len() as u32])].concat(), Vec::new());
            for list in lists {
                table.extend(u32s(&[0, (10 + 8 * lists.len() + tail.len()) as u32]));
                tail.extend([u16s(&[list.len() as u16]), u16s(list)].concat());
            }
            table.extend(tail);
            head.extend([u16s(&[feature]), u32s(&[at as u32])].concat());
            at += table.len();
            bodies.extend(table);
        }
    }
    [head, bodies].concat()
}

// The lookups of every script, language and feature: a required feature, a tagged system's, every feature index
// however many, and a feature variation's alternates and lookup lists.
#[test]
fn feature_lookups_collect_over_every_script_language_and_feature() {
    let none = 0xFFFF;
    assert_eq!(collected(&[(1, vec![0])], &[(b"ccmp", &[0]), (b"rqd1", &[2])], &[]), [0, 2]);
    assert_eq!(collected(&[(none, vec![]), (none, vec![1])], &[(b"ccmp", &[0]), (b"liga", &[3])], &[]), [3]);
    assert_eq!(collected(&[(none, vec![0; 9_000])], &[(b"ccmp", &[1])], &[]), [1]);
    let systems = [(none, vec![0; 9_000]), (none, vec![1, 1])];
    assert_eq!(collected(&systems, &[(b"ccmp", &[1]), (b"liga", &[2])], &[]), [1, 2]);
    let features: [(&[u8; 4], &[u16]); 2] = [(b"ccmp", &[0]), (b"liga", &[1])];
    assert_eq!(collected(&[(none, vec![0])], &features, &variations(&[(0, &[3])], &[])), [0, 3]);
    assert_eq!(collected(&[(none, vec![0])], &features, &variations(&[(1, &[3])], &[])), [0]);
    let lists = variations(&[], &[(0, &[&[2], &[3]])]);
    assert_eq!(collected(&[(none, vec![0])], &[(b"ccmp", &[0])], &lists), [0, 2, 3]);
}

// Context 4 rules of 24-bit glyphs: a later place's glyph is read whole, not cut to 16 bits (65,538 as 256, 512 as
// 2), so [1, 65,538] reaches 65,541 and [1, 512] reaches 8, not 7.
#[test]
fn a_24_bit_rule_s_later_glyph_is_read_in_24_bits() {
    let context = |glyph: u32| {
        let sets = [u16s(&[4, 0]), u16s(&[12]), u24s(&[1, 20]), u16s(&[3]), u24s(&[1, 1])].concat();
        [sets, u16s(&[1, 4, 2, 1]), u24s(&[glyph]), u16s(&[1, 1])].concat()
    };
    let wide = [u16s(&[4, 0, 12]), u24s(&[1, 0x1_0005]), u16s(&[3]), u24s(&[1, 0x1_0002])].concat();
    let t = gsub(&[(5, context(0x1_0002)), (1, wide)]);
    assert_eq!(closed(&t, &[1, 0x1_0002], 0x1_0010), [1, 0x1_0002, 0x1_0005]);
    let t = gsub(&[(5, context(512)), (1, single(&[(2, 7), (512, 8)]))]);
    assert_eq!(closed(&t, &[1, 512], 0x300), [1, 8, 512]);
}

// Context 5 picks a rule set by the whole class its glyphs take: of 65,538 sets, the one for class 65,537 names
// lookup 0, which no feature names. Glyph 1 takes that class, glyph 2 class 1.
#[test]
fn a_24_bit_class_picks_its_rule_set_past_65_535() {
    let n = 0x1_0002;
    let at = 13 + 3 * n;
    let (cov, classes) = (coverage(&[1, 2]), [u16s(&[3]), u24s(&[1, 2, 0x1_0001, 1])].concat());
    let mut sets = vec![0; n];
    sets[0x1_0001] = (at + cov.len() + classes.len()) as u32;
    let set = [u16s(&[1]), u24s(&[5]), u16s(&[1, 1, 0, 0])].concat();
    let head = [u16s(&[5]), u32s(&[at as u32, (at + cov.len()) as u32]), u24s(&[n as u32]), u24s(&sets)].concat();
    let t = gsub_naming(1, &[(1, single(&[(1, 9), (2, 10)])), (5, [head, cov, classes, set].concat())]);
    assert_eq!(closed(&t, &[1], 20), [1, 9]);
    assert_eq!(closed(&t, &[1, 2], 20), [1, 2, 9]);
}

// The closures of each 24-bit substitution: SingleSubst 3 wrapping at 24 bits and 4, MultipleSubst,
// AlternateSubst and LigatureSubst 2.
#[test]
fn substitutions_of_24_bit_glyphs_close() {
    let n = 0x1_0020;
    let t = gsub(&[(1, [u16s(&[3]), u32s(&[9]), u24s(&[1]), cover(3, &[0x1_0001])].concat())]);
    assert_eq!(closed(&t, &[0x1_0001], n), [0x1_0001, 0x1_0002]);
    let t = gsub(&[(1, substitutes(3, &[(1, 0x1_0005), (2, 0x1_0006)]))]);
    assert_eq!(closed(&t, &[1, 2], n), [1, 2, 0x1_0005, 0x1_0006]);
    assert_eq!(closed(&t, &[2], n), [2, 0x1_0006]);
    let t = gsub(&[(2, sequences(3, &[1, 2], &[&[0x1_0003, 0x1_0004], &[5]]))]);
    assert_eq!((closed(&t, &[1], n), closed(&t, &[2], n)), (vec![1, 0x1_0003, 0x1_0004], vec![2, 5]));
    let t = gsub(&[(3, sequences(3, &[1], &[&[0x1_0007, 7]]))]);
    assert_eq!(closed(&t, &[1], n), [1, 7, 0x1_0007]);
    let ligature = |glyph: u32, rest: &[u32]| [u24s(&[glyph]), u16s(&[rest.len() as u16 + 1]), u24s(rest)].concat();
    let set = rule_set(3, &[ligature(0x1_0010, &[2]), ligature(0x1_0011, &[3, 0x1_0002])]);
    let t = gsub(&[(4, covered_sets(2, 3, &[1], &[set]))]);
    assert_eq!((closed(&t, &[1], n), closed(&t, &[1, 2], n)), (vec![1], vec![1, 2, 0x1_0010]));
    assert_eq!(closed(&t, &[1, 2, 3, 0x1_0002], n), [1, 2, 3, 0x1_0002, 0x1_0010, 0x1_0011]);
}

// Closures in both widths, glyph g being g + 65,536 in 24 bits: first places, second records, classes walked,
// coverage rules, reverse chains and backtracks, places after a MultipleSubst, a glyph past the sets.
#[test]
fn contexts_close_alike_in_both_widths() {
    for w in [2, 3] {
        let g = |x: u32| if w == 2 { x } else { x + 0x1_0000 };
        let gs = |xs: &[u32]| xs.iter().map(|&x| g(x)).collect::<Vec<u32>>();
        let n = if w == 2 { 30 } else { 0x1_0020 };
        let sub = |pairs: &[(u32, u32)]| {
            substitutes(w, &pairs.iter().map(|&(a, b)| (g(a), g(b))).collect::<Vec<_>>())
        };
        let glyph_rules = |covered: &[u32], sets: &[Vec<Vec<u8>>]| {
            let sets: Vec<Vec<u8>> = sets.iter().map(|rules| rule_set(2, rules)).collect();
            covered_sets(if w == 2 { 1 } else { 4 }, w, &gs(covered), &sets)
        };
        let check = |t: &[u8], seed: &[u32], want: &[u32]| assert_eq!(closed(t, &gs(seed), n), gs(want), "{w}");

        let first = glyph_rules(&[1, 2], &[vec![rule(w, &[], &[(0, 1)])], vec![]]);
        let t = gsub(&[(5, first), (1, sub(&[(1, 8), (2, 9)]))]);
        check(&t, &[1, 2], &[1, 2, 8]);
        let twice = glyph_rules(&[1], &[vec![rule(w, &gs(&[2]), &[(1, 1), (1, 1)])]]);
        let t = gsub(&[(5, twice), (1, sub(&[(2, 7), (3, 8)]))]);
        check(&t, &[1, 2, 3], &[1, 2, 3, 7, 8]);

        let def = class_def(w, &[(g(1), g(1), 1), (g(3), g(3), 2), (g(4), g(4), 1)]);
        let sets = [vec![], vec![], vec![rule(2, &[1], &[(1, 1)])]];
        let t = gsub(&[(5, by_class(w as u16 * 3 - 4, w, &gs(&[1, 3]), &[def], &sets)), (1, sub(&[(4, 9)]))]);
        check(&t, &[1, 4], &[1, 4]);
        check(&t, &[1, 3, 4], &[1, 3, 4, 9]);

        let t = gsub(&[(5, by_coverage(w, &[&gs(&[1]), &gs(&[2])], &[(0, 1)])), (1, sub(&[(1, 9)]))]);
        check(&t, &[1], &[1]);
        check(&t, &[1, 2], &[1, 2, 9]);
        let t = gsub(&[(8, reverse(w, &gs(&[1]), &gs(&[2]), &gs(&[3]), &gs(&[9])))]);
        check(&t, &[1, 2], &[1, 2]);
        check(&t, &[1, 2, 3], &[1, 2, 3, 9]);

        let rules = rule_set(w, &[chained(w, &gs(&[4]), &[], &gs(&[2]), &[(0, 1)])]);
        let t = gsub(&[(6, covered_sets(if w == 2 { 1 } else { 4 }, w, &gs(&[1]), &[rules])), (1, sub(&[(1, 9)]))]);
        check(&t, &[1, 2], &[1, 2]);
        check(&t, &[1, 2, 4], &[1, 2, 4, 9]);
        let defs = [4, 1, 2].map(|x| class_def(w, &[(g(x), g(x), 1)]));
        let sets = [vec![], vec![chained(2, &[1], &[], &[1], &[(0, 1)])]];
        let t = gsub(&[(6, by_class(w as u16 * 3 - 4, w, &gs(&[1]), &defs, &sets)), (1, sub(&[(1, 9)]))]);
        check(&t, &[1, 2], &[1, 2]);
        check(&t, &[1, 2, 4], &[1, 2, 4, 9]);

        let multiple = sequences(w, &gs(&[1]), &[&gs(&[5, 6])]);
        let rules = [vec![rule(w, &gs(&[2, 3]), &[(0, 1), (1, 2)])]];
        let t = gsub(&[(5, glyph_rules(&[1], &rules)), (2, multiple.clone()), (1, sub(&[(2, 7), (4, 8)]))]);
        check(&t, &[1, 2, 3, 4], &[1, 2, 3, 4, 5, 6, 7, 8]);
        let rules = [vec![rule(w, &[], &[(0, 1), (1, 2)])]];
        let t = gsub(&[(5, glyph_rules(&[1], &rules)), (2, multiple), (1, sub(&[(2, 7)]))]);
        check(&t, &[1, 2], &[1, 2, 5, 6]);

        let t = gsub(&[(2, sequences(w, &gs(&[1, 2]), &[&gs(&[8])]))]);
        assert_eq!(closed(&t, &gs(&[1, 2]), 0xFF_FFFF), gs(&[1, 2, 8]));
    }
}

// A class context's answers, kept per subtable: a first place's among the covered glyphs, a later one's among the
// whole set, so 2 reaches 9 only second; a nested context keeps its own, and none outlive the subtable's visit.
#[test]
fn a_class_based_context_keeps_its_answers_for_its_subtable() {
    for w in [2, 3] {
        let g = |x: u32| if w == 2 { x } else { x + 0x1_0000 };
        let gs = |xs: &[u32]| xs.iter().map(|&x| g(x)).collect::<Vec<u32>>();
        let n = if w == 2 { 30 } else { 0x1_0020 };
        let format = w as u16 * 3 - 4;
        let sub = |pairs: &[(u32, u32)]| {
            substitutes(w, &pairs.iter().map(|&(a, b)| (g(a), g(b))).collect::<Vec<_>>())
        };
        let check = |t: &[u8], seed: &[u32], want: &[u32]| assert_eq!(closed(t, &gs(seed), n), gs(want), "{w}");

        let one = || [class_def(w, &[(g(1), g(2), 1)])];
        let first = by_class(format, w, &gs(&[1]), &one(), &[vec![], vec![rule(2, &[1], &[(0, 1), (1, 1)])]]);
        check(&gsub(&[(5, first), (1, sub(&[(1, 8), (2, 9)]))]), &[1, 2], &[1, 2, 8, 9]);
        let later = by_class(format, w, &gs(&[1]), &one(), &[vec![], vec![rule(2, &[1], &[(1, 2), (0, 1)])]]);
        check(&gsub(&[(5, later), (1, sub(&[(2, 9)])), (1, sub(&[(5, 6)]))]), &[1, 2], &[1, 2]);
        let nested = by_class(format, w, &gs(&[1]), &one(), &[vec![], vec![rule(2, &[1], &[(1, 3), (0, 1)])]]);
        let inner = [class_def(w, &[(g(1), g(1), 1)])];
        let inner = by_class(format, w, &gs(&[1]), &inner, &[vec![], vec![rule(2, &[], &[(0, 2)])]]);
        let t = gsub(&[(5, nested), (1, sub(&[(2, 9)])), (1, sub(&[(6, 7)])), (5, inner)]);
        check(&t, &[1, 2], &[1, 2]);

        let defs = [2, 1, 4].map(|x| class_def(w, &[(g(x), g(x), 1)]));
        let sets = [vec![], vec![chained(2, &[1], &[], &[1], &[(0, 1)])]];
        let t = gsub(&[(6, by_class(format, w, &gs(&[1]), &defs, &sets)), (1, sub(&[(1, 9)]))]);
        check(&t, &[1, 2], &[1, 2]);
        check(&t, &[1, 2, 4], &[1, 2, 4, 9]);

        let def = class_def(w, &[(g(1), g(1), 1), (g(3), g(3), 3), (g(5), g(5), 2)]);
        let sets = [vec![], vec![rule(2, &[2], &[(0, 1)])], vec![], vec![rule(2, &[], &[(0, 2)])]];
        let staged = by_class(format, w, &gs(&[1, 3]), &[def], &sets);
        check(&gsub(&[(5, staged), (1, sub(&[(1, 9)])), (1, sub(&[(3, 5)]))]), &[1, 3], &[1, 3, 5, 9]);
    }
}

// Tables gone wrong: a substitute past the glyph count; a SingleSubst 1 mapping back into its run, a glyph a pass;
// format 2 coverages jumping indices or out of order, each range for itself; class 0 past a format 1 ClassDef's array.
#[test]
fn tables_gone_wrong_still_close() {
    assert_eq!(closed(&gsub(&[(1, single(&[(1, 19), (2, 20), (3, 500)]))]), &[1, 2, 3], 20), [1, 2, 3, 19]);
    let t = gsub(&[(1, u16s(&[1, 6, 1, 2, 1, 0, 20, 0]))]);
    assert_eq!(closed(&t, &[1], 30), (1..=21).collect::<Vec<u32>>());
    let lists: Vec<Vec<u8>> = [8, 10, 10, 10, 10, 9].iter().map(|&g| u16s(&[1, g])).collect();
    let (offsets, body) = laid(34, &lists);
    let ranges = u16s(&[2, 2, 1, 1, 0, 3, 3, 5]);
    let t = gsub(&[(2, [u16s(&[1, 18, 6]), uints(2, &offsets), ranges, body].concat())]);
    assert_eq!(closed(&t, &[1, 3], 30), [1, 3, 8, 9]);
    let t = gsub(&[(1, u16s(&[1, 6, 10, 2, 2, 5, 6, 0, 2, 3, 2]))]);
    assert_eq!((closed(&t, &[2], 30), closed(&t, &[5], 30)), (vec![2, 12], vec![5, 15]));
    let class_0 = vec![vec![rule(2, &[], &[(0, 1)])], vec![]];
    let t = gsub(&[(5, by_class(2, 2, &[2, 5], &[u16s(&[1, 2, 1, 1])], &class_0)), (1, single(&[(5, 9)]))]);
    assert_eq!(closed(&t, &[2, 5], 30), [2, 5, 9]);
    let t = gsub(&[(5, by_class(2, 2, &[1, 3], &[u16s(&[1, 3, 1, 1])], &class_0)), (1, single(&[(1, 8)]))]);
    assert_eq!(closed(&t, &[1, 3], 30), [1, 3, 8]);
}

// A rule naming lookup 0 40,000 times still reaches its last record's lookup 1: only the work budget bounds a pass.
#[test]
fn a_rule_reaches_its_last_record_however_many_come_before() {
    let r: Vec<(u16, u16)> = [vec![(0, 0); 40_000], vec![(0, 1)]].concat();
    let context = covered_sets(1, 2, &[1], &[rule_set(2, &[rule(2, &[], &r)])]);
    let t = gsub_naming(2, &[(1, single(&[(1, 5)])), (1, single(&[(1, 9)])), (5, context)]);
    assert_eq!(closed(&t, &[1], 30), [1, 5, 9]);
}

// A Context 1 rule set of `n` rules sharing one rule of 10,000 records, each naming a place past the rule's one: each
// record read is spent, so 200 rules' 2 million reads take the closure past a million, as one rule's don't.
#[test]
fn records_past_a_rules_places_spend_each_read() {
    let skipped = rule(2, &[], &vec![(1, 0); 10_000]);
    let closes = |n: u16| {
        let shared = [u16s(&[n]), u16s(&vec![2 + 2 * n; n.into()]), skipped.clone()].concat();
        let t = gsub(&[(5, covered_sets(1, 2, &[1], &[shared]))]);
        let mut set: GlyphSet = [1].into_iter().collect();
        close(&t, &feature_lookups(&t).expect("within the budget"), &mut set, 10, 1_000_000)
    };
    assert!(closes(1));
    assert!(!closes(200));
}

// A SingleSubst 1 adding 1 over every 16-bit glyph adds one a pass, and its closure from glyph 1 fails past its
// work budget rather than walking 65,533 passes.
#[test]
fn a_chain_past_the_budget_fails_its_closure() {
    let t = gsub(&[(1, u16s(&[1, 6, 1, 2, 1, 0, 65_534, 0]))]);
    let mut set: GlyphSet = [1].into_iter().collect();
    assert!(!close(&t, &feature_lookups(&t).expect("within the budget"), &mut set, 65_535, budget(&t)));
}

// A Context 1 whose range coverage names all 65,535 glyphs, 60,000 of them in the set, given work for its visit but
// not its coverage, stops at the charge rather than gathering a rule set for each of the 60,000, megabytes.
#[cfg(any(debug_assertions, daegun_counting))]
#[test]
fn a_walk_past_the_budget_is_not_taken() {
    let t = gsub(&[(5, [u16s(&[1, 6, 0]), u16s(&[2, 1, 0, 65_534, 0])].concat())]);
    let lookups = feature_lookups(&t).expect("within the budget");
    let mut set: GlyphSet = (0..60_000).collect();
    let (_, before, _) = crate::allocations::tally();
    assert!(!close(&t, &lookups, &mut set, 65_535, 10_000));
    let (_, after, _) = crate::allocations::tally();
    assert!(after - before < 256 << 10, "{} bytes allocated", after - before);
}

// A Context 5 of 40,000 classes, each its own glyph's and each glyph in the set, charged a ClassDef read a class but
// given less: it stops at the charge, where reading the ClassDef for each class takes 1.6 billion range reads.
#[test]
fn a_class_walk_past_the_budget_is_not_taken() {
    extern crate std;
    let n = 40_000;
    let def = class_def(3, &(1..=n).map(|g| (g, g, g as u16)).collect::<Vec<_>>());
    let sets: Vec<Vec<Vec<u8>>> = (0..=n).map(|c| if c == 0 { vec![] } else { vec![rule(2, &[], &[])] }).collect();
    let glyphs: Vec<u32> = (1..=n).collect();
    let t = gsub(&[(5, by_class(5, 3, &glyphs, &[def], &sets))]);
    let lookups = feature_lookups(&t).expect("within the budget");
    let mut set: GlyphSet = glyphs.iter().copied().collect();
    let start = std::time::Instant::now();
    assert!(!close(&t, &lookups, &mut set, n + 1, 1_000_000));
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
}

// FeatureVariations records naming substitution tables six bytes apart in one run of (feature 0, 1,024) records, so
// each reads 1,024 of them: past twice the table's bytes at 600 tables, though not at 20, the closure is refused.
#[test]
fn feature_variations_read_past_their_budget_refuse_the_closure() {
    let lookups = |tables: usize| {
        let head = [u32s(&[0x1_0001]), u16s(&[14, 34, 46]), u32s(&[48])].concat();
        let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
        let features = [u16s(&[1]), b"liga".to_vec(), u16s(&[8, 0, 0])].concat();
        let run = 8 + 8 * tables;
        let records = (0..tables).flat_map(|i| u32s(&[0, (run + 6 * i) as u32]));
        let variations = [u32s(&[0x1_0000, tables as u32]), records.collect()].concat();
        let substitutions = (0..tables + 1_025).flat_map(|_| [u16s(&[0]), u32s(&[1_024])].concat());
        feature_lookups(&[head, scripts, features, u16s(&[0]), variations, substitutions.collect()].concat())
    };
    assert_eq!(lookups(20), Some(vec![]));
    assert_eq!(lookups(600), None);
}

// Every script and language system is read, 600 scripts and 2,100 systems alike, an empty one among them.
#[test]
fn feature_lookups_read_every_script_and_language_system() {
    let none = 0xFFFF;
    let features: [(&[u8; 4], &[u16]); 2] = [(b"ccmp", &[0]), (b"liga", &[1])];
    let systems = |empty: usize, n: usize| -> Vec<(u16, Vec<u16>)> {
        let last_liga = (1..=n).map(|i| (none, vec![u16::from(i == n)]));
        core::iter::repeat_n((none, vec![]), empty).chain(last_liga).collect()
    };
    let scripts = |empty: usize, n: usize| -> Vec<Vec<(u16, Vec<u16>)>> {
        core::iter::repeat_n(vec![], empty).chain(systems(0, n).into_iter().map(|s| vec![s])).collect()
    };
    assert_eq!(collected_from(&scripts(5, 600), &features, &[]), [0, 1]);
    assert_eq!(collected(&systems(5, 2_100), &features, &[]), [0, 1]);
}

// Eight ranges over the same 256 glyphs meet each glyph eight times, and each walk spends all of them; one given
// less stops where its work runs out.
#[test]
fn ranges_that_overlap_spend_every_glyph_they_meet() {
    let set: GlyphSet = (0..256).collect();
    let cov = [u16s(&[2, 8]), u16s(&[0, 255, 0]).repeat(8)].concat();
    let def = class_def(2, &[(0, 255, 1); 8]);
    let spent = |budget: usize, walk: &dyn Fn(&mut GlyphSet, &Work)| {
        let (mut out, work) = (GlyphSet::empty(), Work::new(budget));
        walk(&mut out, &work);
        (out.len(), budget - work.left.get(), work.over())
    };
    let met = |out: &mut GlyphSet, work: &Work| intersect(&cov, &set, out, work);
    let classed = |out: &mut GlyphSet, work: &Work| class_glyphs(&def, &set, 1, out, work);
    for walk in [&met as &dyn Fn(&mut GlyphSet, &Work), &classed] {
        assert_eq!(spent(10_000, walk), (256, 8 * (4 + 256), false));
        assert_eq!(spent(100, walk), (96, 100, true));
    }
}

// Class 0's gap walks, which read on from the glyph they last found, against the gaps they stand for: each glyph above
// every range before one and below its first, or above every range.
#[test]
fn class_zero_s_gaps_are_those_between_its_ranges() {
    let defs: [&[(u32, u32, u16)]; 5] = [
        &[(3, 5, 1), (9, 9, 2), (20, 40, 1)],
        &[(10, 30, 1), (5, 12, 2), (40, 45, 1)],
        &[(20, 10, 1), (30, 35, 2), (25, 27, 1)],
        &[(0, 255, 1), (0, 255, 1), (100, 100, 2)],
        &[(50, 60, 0), (61, 62, 1)],
    ];
    let sets: [GlyphSet; 4] =
        [(0..256).collect(), [7, 33, 200].into_iter().collect(), [255].into_iter().collect(), GlyphSet::empty()];
    for ranges in defs {
        let gap = |x: u32| {
            let mut below = None::<u32>;
            for &(first, last, _) in ranges {
                if below.is_none_or(|b| b < x) && x < first {
                    return true;
                }
                below = Some(below.map_or(last, |b| b.max(last)));
            }
            below.is_none_or(|b| x > b)
        };
        let listed = |x: u32| ranges.iter().any(|&(first, last, c)| c == 0 && (first..=last).contains(&x));
        let def = class_def(2, ranges);
        for set in &sets {
            let want: Vec<u32> = set.iter().filter(|&x| gap(x) || listed(x)).collect();
            let (mut out, work) = (GlyphSet::empty(), Work::new(usize::MAX));
            class_glyphs(&def, set, 0, &mut out, &work);
            assert_eq!(out.iter().collect::<Vec<_>>(), want, "{ranges:?}");
            assert_eq!(class_intersects(&def, set, 0, &work), !want.is_empty(), "{ranges:?}");
        }
    }
}

// Lookup 0 a Context 1 rule of 65,535 glyphs whose 100 records each name another index of one MultipleSubst lookup:
// each visit leaves the rule's places from its own on to the whole set, 6.5 million in all, and spends each one.
#[test]
fn a_length_changing_lookup_spends_the_places_it_leaves_to_the_whole_set() {
    const M: u16 = 100;
    let (multiple, context) = (2 + 2 * (M + 1), 2 + 2 * (M + 1) + 26);
    let t = [
        u16s(&[1, 0, 0, 0, 10, M + 1, context]), u16s(&vec![multiple; M.into()]),
        u16s(&[2, 0, 1, 8, 1, 8, 1, 14, 1, 1, 0, 1, 0]),
        u16s(&[5, 0, 1, 8, 1, 8, 1, 14, 1, 1, 0, 1, 4, u16::MAX, M]), vec![0; 2 * (usize::from(u16::MAX) - 1)],
        (1..=M).flat_map(|i| u16s(&[1, i])).collect(),
    ].concat();
    let set: GlyphSet = [0].into_iter().collect();
    assert!(close(&t, &[0], &mut set.clone(), 10, 10_000_000));
    assert!(!close(&t, &[0], &mut set.clone(), 10, 1_000_000));
}

// A GSUB whose one LangSys names a feature for each of `rels`, each FeatureRecord naming the Feature at its offset in
// `features`, read from the FeatureList's start.
fn naming_features(rels: &[u16], features: Vec<u8>) -> Vec<u8> {
    let f = rels.len() as u16;
    [
        u16s(&[1, 0, 10, 28 + 2 * f, 0, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, f]),
        (0..f).flat_map(u16::to_be_bytes).collect(), u16s(&[f]),
        rels.iter().flat_map(|&rel| [b"test".to_vec(), u16s(&[rel])].concat()).collect(), features,
    ]
    .concat()
}

// 200 features naming one Feature of 65,535 lookups read it once, within the budget its 132 KB give; eight naming
// Features that start two bytes apart over 0xFFFF words read 524,280 lookup indices, past it.
#[test]
fn a_feature_many_records_name_is_read_once_within_the_budget() {
    let n = 200;
    let lookups = [u16s(&[0, 0xFFFF]), (0..0xFFFF).flat_map(u16::to_be_bytes).collect()].concat();
    let shared = naming_features(&vec![2 + 6 * n; n.into()], lookups);
    assert_eq!(feature_lookups(&shared).map(|l| l.len()), Some(0xFFFF));
    let starts: Vec<u16> = (0..8).map(|k| 2 + 6 * 8 + 2 * k).collect();
    assert_eq!(feature_lookups(&naming_features(&starts, vec![0xFF; 2 * (0xFFFF + 8) + 4])), None);
}
