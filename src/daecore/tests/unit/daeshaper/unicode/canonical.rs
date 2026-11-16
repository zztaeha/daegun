use alloc::collections::BTreeSet;

use super::*;
use crate::daecore::daeshaper::unicode::props;

// Every code point's two flags against the searches they stand before: a character decomposes, or is the
// second of a composition (a pair's, or Hangul's after a leading consonant or LV syllable), where its flag says.
#[test]
fn a_characters_normalization_flags_say_what_the_searches_find() {
    let pairs = t::DECOMPOSE_PAIR_BMP.iter().map(|&(k, a, b)| (u32::from(k), u32::from(a), u32::from(b)));
    let pairs = pairs.chain(t::DECOMPOSE_PAIR_SUPP.iter().copied());
    let mut seconds: BTreeSet<u32> = pairs.filter(|&(k, a, b)| join(a, b) == Some(k)).map(|(_, _, b)| b).collect();
    let hangul = |c: u32| [0x1100, 0xAC00].iter().any(|&a| join(a, c).is_some());
    seconds.extend((0..=0x10FFFF).filter(|&c| hangul(c)));
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        let (cp, flags) = (u32::from(c), props(c).flags);
        assert_eq!(flags & t::flag::DECOMPOSES != 0, split(cp).is_some(), "U+{cp:04X}");
        assert_eq!(flags & t::flag::COMPOSES_SECOND != 0, seconds.contains(&cp), "U+{cp:04X}");
    }
}

// Each Hangul syllable is its jamo's (Unicode 3.12), splits into its LV syllable and trailing consonant or its two
// jamo, and composes back from them; nothing else makes a syllable.
#[test]
fn hangul_syllables_are_their_jamo_by_arithmetic() {
    let (first, last) = (0xAC00, 0xD7A3);
    for s in first..=last {
        let (l, v, t) = hangul_jamo(s).expect("a syllable");
        assert_eq!(hangul_syllable(l, v, t), Some(s), "U+{s:04X}");
        let (a, b) = match t {
            Some(t) => (hangul_syllable(l, v, None).expect("an LV syllable"), t),
            None => (l, v),
        };
        assert_eq!(split(s), Some((a, Some(b))), "U+{s:04X}");
        assert_eq!(join(a, b), Some(s), "U+{s:04X}");
    }
    assert_eq!(hangul_jamo(0xAC00), Some((0x1100, 0x1161, None)));
    assert_eq!(hangul_jamo(last), Some((0x1112, 0x1175, Some(0x11C2))));
    assert_eq!([hangul_jamo(first - 1), hangul_jamo(last + 1)], [None, None]);
    assert_eq!(hangul_syllable(0x1100, 0x1161, Some(0x11A7)), None, "TBase is no trailing consonant");
    assert_eq!(hangul_syllable(0x1113, 0x1161, None), None, "an old leading consonant has no syllable");
    assert_eq!(join(0xAC01, 0x11A8), None, "an LVT syllable takes no second trailing consonant");
}

// Each Hebrew letter or form with each Hebrew mark: the table's form is the one a scan of the decompositions finds.
#[test]
fn hebrew_presentation_forms_are_the_decompositions() {
    let scan = |a: char, b: char| {
        t::DECOMPOSE_PAIR_BMP
            .iter()
            .filter(|&&(k, _, _)| (0xFB1D..=0xFB4F).contains(&k))
            .find(|&&(_, x, y)| (u32::from(x), u32::from(y)) == (u32::from(a), u32::from(b)))
            .and_then(|&(k, _, _)| char::from_u32(u32::from(k)))
    };
    for a in (0x0590..=0x05FF).chain(0xFB1D..=0xFB4F).filter_map(char::from_u32) {
        for b in (0x0590..=0x05FF).filter_map(char::from_u32) {
            assert_eq!(hebrew_presentation_form(a, b), scan(a, b), "{a:?} {b:?}");
        }
    }
}
