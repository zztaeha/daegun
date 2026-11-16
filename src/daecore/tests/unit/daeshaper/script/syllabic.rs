use super::*;

#[test]
fn a_character_opens_a_vowel_sequence_exactly_when_a_pair_or_triple_starts_with_it() {
    let firsts: alloc::collections::BTreeSet<u32> =
        INVALID_VOWEL_PAIRS.iter().map(|p| p.0).chain(INVALID_VOWEL_TRIPLES.iter().map(|t| t.0)).collect();
    for c in 0..=0x10FFFF {
        assert_eq!(opens_invalid_vowel_sequence(c), firsts.contains(&c), "U+{c:04X}");
    }
}
