use daegun::{Font, Location};

use super::aat_subset::{base_with, u16s};
use super::cmap_reading::{cmap, format4};
use super::layout_build::{aliased, layout, repeated, table, to, w};
use daegun::shape::ShapeOptions;

// colr-clip.ttf's 30 glyphs with `map` as the cmap and `gsub`.
fn font(map: &[(u16, u16)], gsub: Vec<u8>) -> Font {
    let mut map = map.to_vec();
    map.sort();
    base_with("test-fixtures/colr-clip.ttf", &[("cmap", cmap(&[(3, 1, &format4(&map))])), ("GSUB", gsub)])
}

// A GSUB under `script` whose features each run the lookups they name, one subtable a lookup.
fn gsub(script: &[u8; 4], features: &[(&[u8; 4], &[u16])], lookups: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let n = features.len() as u16;
    let scripts = [u16s(&[1]), script.to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, n]), u16s(&(0..n).collect::<Vec<_>>())];
    let mut records = u16s(&[n]);
    let mut tables = Vec::new();
    for (tag, indices) in features {
        records.extend([tag.to_vec(), u16s(&[(2 + 6 * features.len() + tables.len()) as u16])].concat());
        tables.extend([u16s(&[0, indices.len() as u16]), u16s(indices)].concat());
    }
    let mut offsets = u16s(&[lookups.len() as u16]);
    let mut bodies = Vec::new();
    for (kind, subtable) in lookups {
        offsets.extend(u16s(&[(2 + 2 * lookups.len() + bodies.len()) as u16]));
        bodies.extend([u16s(&[*kind, 0, 1, 8]), subtable.clone()].concat());
    }
    let (scripts, features) = (scripts.concat(), [records, tables].concat());
    let at_features = 10 + scripts.len() as u16;
    let at_lookups = at_features + features.len() as u16;
    [u16s(&[1, 0, 10, at_features, at_lookups]), scripts, features, offsets, bodies].concat()
}

fn coverage(glyphs: &[u16]) -> Vec<u8> {
    [u16s(&[1, glyphs.len() as u16]), u16s(glyphs)].concat()
}

// A ligature substitution with one ligature for each first glyph, given in glyph order.
fn ligatures(ligatures: &[(&[u16], u16)]) -> Vec<u8> {
    let mut offsets = Vec::new();
    let mut sets = Vec::new();
    for (parts, ligature) in ligatures {
        offsets.push((6 + 2 * ligatures.len() + sets.len()) as u16);
        sets.extend([u16s(&[1, 4, *ligature, parts.len() as u16]), u16s(&parts[1..])].concat());
    }
    let firsts: Vec<u16> = ligatures.iter().map(|l| l.0[0]).collect();
    let at_coverage = (6 + 2 * ligatures.len() + sets.len()) as u16;
    [u16s(&[1, at_coverage, ligatures.len() as u16]), u16s(&offsets), sets, coverage(&firsts)].concat()
}

fn single(from: u16, to: u16) -> Vec<u8> {
    [u16s(&[2, 8, 1, to]), coverage(&[from])].concat()
}

// A multiple substitution that deletes `glyph`.
fn deletion(glyph: u16) -> Vec<u8> {
    [u16s(&[1, 10, 1, 8, 0]), coverage(&[glyph])].concat()
}

// A chain context rule over `input` before `lookahead`, one glyph a position, running each (position, lookup).
fn chain(input: &[u16], lookahead: &[u16], records: &[(u16, u16)]) -> Vec<u8> {
    let first = 10 + 2 * (input.len() + lookahead.len()) + 4 * records.len();
    let at = |k: usize| (first + 6 * k) as u16;
    let inputs: Vec<u16> = (0..input.len()).map(at).collect();
    let lookaheads: Vec<u16> = (input.len()..input.len() + lookahead.len()).map(at).collect();
    let records: Vec<u16> = records.iter().flat_map(|r| [r.0, r.1]).collect();
    [
        u16s(&[3, 0, input.len() as u16]),
        u16s(&inputs),
        u16s(&[lookahead.len() as u16]),
        u16s(&lookaheads),
        u16s(&[(records.len() / 2) as u16]),
        u16s(&records),
        input.iter().chain(lookahead).flat_map(|&g| coverage(&[g])).collect(),
    ]
    .concat()
}

fn shaped(f: &Font, text: &str) -> (Vec<u32>, Vec<u32>, Vec<bool>) {
    let run = f.shape(text, &Location::default(), &ShapeOptions::default());
    (run.glyphs.iter().map(|g| g.0).collect(), run.clusters.to_vec(), run.unsafe_to_break.to_vec())
}

// Gurmukhi KA, halant, YA with a post-base form, and Yakash: the base search takes Yakash, a consonant medial, so
// YA is the base; passing over it would give YA its post form after KA.
#[test]
fn the_base_search_takes_a_consonant_medial() {
    let map = [(0xA15, 1), (0xA4D, 2), (0xA2F, 3), (0xA75, 4)];
    let f = font(&map, gsub(b"gur2", &[(b"pstf", &[0])], &[(4, ligatures(&[(&[2, 3], 8)]))]));
    let run = shaped(&f, "\u{A15}\u{A4D}\u{A2F}\u{A75}");
    assert_eq!(run, (vec![1, 2, 3, 4], vec![0, 0, 6, 6], vec![false, false, true, true]));
}

// A post-base form only before the aa matra: Malayalam fonts of both specs look at the context, so YA after the
// virama takes it, where a new-spec Kannada font matches the form without it.
#[test]
fn malayalam_forms_are_found_in_their_context() {
    let rule = |script| {
        let lookups = [(6, chain(&[2, 3], &[4], &[(0, 1), (1, 2)])), (1, single(2, 6)), (1, single(3, 7))];
        gsub(script, &[(b"pstf", &[0])], &lookups)
    };
    let malayalam = font(&[(0xD15, 1), (0xD4D, 2), (0xD2F, 3), (0xD3E, 4)], rule(b"mlm2"));
    let kannada = font(&[(0xC95, 1), (0xCCD, 2), (0xCAF, 3), (0xCBE, 4)], rule(b"knd2"));
    let flags = vec![false, false, true, true];
    assert_eq!(shaped(&malayalam, "\u{D15}\u{D4D}\u{D2F}\u{D3E}"), (vec![1, 6, 7, 4], vec![0, 0, 6, 6], flags.clone()));
    assert_eq!(shaped(&kannada, "\u{C95}\u{CCD}\u{CAF}\u{CBE}"), (vec![1, 2, 3, 4], vec![0, 0, 6, 6], flags));
}

// KA and virama ligated by ccmp before medial RA and another KA: the Myanmar base search passes over the ligature
// to the second KA rather than take it for a consonant.
#[test]
fn a_ligated_consonant_is_no_myanmar_base() {
    let map = [(0x1000, 1), (0x1039, 2), (0x103C, 3)];
    let f = font(&map, gsub(b"mym2", &[(b"ccmp", &[0])], &[(4, ligatures(&[(&[1, 2], 10)]))]));
    let run = shaped(&f, "\u{1000}\u{1039}\u{1000}\u{103C}");
    assert_eq!(run, (vec![10, 3, 1], vec![0, 6, 6], vec![false, true, true]));
}

// Ligated by ccmp, a nukta and halant still travel with KA, a matra still counts toward the reph's place, and
// pre-base matras still flip: only the category is read, since taken as a ligature none of them would.
#[test]
fn a_ligated_sign_keeps_its_category_where_it_moves() {
    let devanagari = |map: &[(u16, u16)], features: &[(&[u8; 4], &[u16])], lookups: &[(u16, Vec<u8>)]| {
        font(map, gsub(b"dev2", features, lookups))
    };
    let nukta_halant = devanagari(
        &[(0x915, 1), (0x93C, 2), (0x94D, 3), (0x930, 4)],
        &[(b"ccmp", &[0]), (b"blwf", &[1])],
        &[(4, ligatures(&[(&[2, 3], 8)])), (4, ligatures(&[(&[3, 4], 9)]))],
    );
    let run = shaped(&nukta_halant, "\u{915}\u{93C}\u{94D}\u{930}");
    assert_eq!(run, (vec![1, 8, 4], vec![0, 0, 9], vec![false, false, true]));

    let o_nukta = devanagari(
        &[(0x930, 1), (0x94D, 2), (0x915, 3), (0x94B, 4), (0x93C, 5)],
        &[(b"ccmp", &[0]), (b"rphf", &[1])],
        &[(4, ligatures(&[(&[4, 5], 9)])), (4, ligatures(&[(&[1, 2], 10)]))],
    );
    let run = shaped(&o_nukta, "\u{930}\u{94D}\u{915}\u{94B}\u{93C}\u{94D}");
    assert_eq!(run, (vec![3, 9, 10, 2], vec![0; 4], vec![false; 4]));

    let matra_nukta = devanagari(
        &[(0x915, 1), (0x93F, 2), (0x93C, 3), (0x94E, 4)],
        &[(b"ccmp", &[0])],
        &[(4, ligatures(&[(&[2, 3], 10), (&[4, 3], 9)]))],
    );
    assert_eq!(shaped(&matra_nukta, "\u{915}\u{94E}\u{94E}\u{93C}"), (vec![9, 4, 1], vec![0; 3], vec![false; 3]));
    assert_eq!(shaped(&matra_nukta, "\u{915}\u{93F}\u{93F}\u{93C}"), (vec![10, 2, 1], vec![0; 3], vec![false; 3]));
}

// A reph after two matras and a final halant in Kannada: the matras before its place each pull it in, and the
// count stops where it lands, so the second matra stays before it.
#[test]
fn the_rephs_place_counts_the_matras_before_it() {
    let map = [(0xCB0, 1), (0xCCD, 2), (0xC95, 3), (0xCC1, 4), (0xCC6, 5)];
    let f = font(&map, gsub(b"knd2", &[(b"rphf", &[0])], &[(4, ligatures(&[(&[1, 2], 10)]))]));
    let run = shaped(&f, "\u{CB0}\u{CCD}\u{C95}\u{CC1}\u{CC6}\u{CCD}");
    assert_eq!(run, (vec![3, 4, 5, 10, 2], vec![0; 5], vec![false; 5]));
}

// pref marks halant and RA, then deletes RA, so the base search runs off the syllable's end. It stops there rather
// than read the space after it, and the i matra moves before the final halant.
#[test]
fn a_base_search_stops_at_the_syllables_end() {
    let map = [(0x915, 1), (0x94D, 2), (0x930, 3), (0x93F, 4), (0x20, 5)];
    let lookups = [(2, deletion(3)), (4, ligatures(&[(&[2, 3], 7)]))];
    let f = font(&map, gsub(b"dev2", &[(b"pref", &[0, 1])], &lookups));
    let run = shaped(&f, "\u{915}\u{94D}\u{930}\u{93F}\u{94D} ");
    assert_eq!(run, (vec![1, 2, 4, 2, 5], vec![0, 0, 0, 0, 15], vec![false; 5]));
}

// RA, virama and KA, and an rphf lookup naming one LigatureSubst `n` times, whose set on RA names one ligature of RA
// and `second` `n` times.
fn reph_font(n: u16, second: u16) -> Font {
    let set = [u16s(&[n]), u16s(&vec![2 + 2 * n; usize::from(n)]), u16s(&[9, 2, second])].concat();
    let subtable = table(vec![w(&[1]), to(coverage(&[1])), w(&[1]), to(set)]);
    let gsub = layout(b"dev2", &[(b"rphf", &[0])], vec![repeated(4, 0, n, subtable)]);
    font(&[(0x915, 3), (0x930, 1), (0x94D, 2)], gsub)
}

// Ten syllables of RA, virama and KA, each asking rphf for a reph: 90,000 ligatures none of which takes the virama, or
// 30,000 lookups whose coverage lacks RA, fail the run that pays for them. A reph formed at once costs little.
#[test]
fn a_reph_probe_is_charged_to_the_run() {
    let text = "\u{930}\u{94D}\u{915} ".repeat(10);
    let shaped = |f: Font| f.shape(&text, &Location::default(), &ShapeOptions::default());
    assert!(!shaped(reph_font(300, 3)).complete);
    let ka = table(vec![w(&[1]), to(coverage(&[3])), w(&[0])]);
    let mut passed = aliased(&[(b"rphf", 0..30_000)], 30_000, repeated(4, 0, 1, ka), vec![]);
    // The Indic model runs only under an Indic script tag.
    let script = passed.windows(4).position(|t| t == b"DFLT").expect("the script tag");
    passed[script..script + 4].copy_from_slice(b"dev2");
    assert!(!shaped(font(&[(0x915, 3), (0x930, 1), (0x94D, 2)], passed)).complete);
    let formed = shaped(reph_font(4, 2));
    assert!(formed.complete && formed.glyphs.iter().any(|g| g.0 == 9));
}

// A first-spec font joins a syllable's clusters from its base on, so a ZWNJ after the base, which the spec places in
// no syllable there, keeps its own cluster; after an anusvara it closes the syllable and joins it.
#[test]
fn a_zwnj_after_a_base_is_no_part_of_its_syllable() {
    let f = font(&[(0x20, 4), (0x915, 1), (0x902, 2), (0x200C, 3)], gsub(b"deva", &[], &[]));
    assert_eq!(shaped(&f, "\u{915}\u{200C}\u{915}").1, [0, 3, 6]);
    assert_eq!(shaped(&f, "\u{915}\u{902}\u{200C}").1, [0, 0, 0]);
}
