// Canonical decomposition a level at a time and the primary composite of two (UAX #15, from UnicodeData.txt and
// CompositionExclusions.txt); Hangul syllables are computed, not listed (Unicode 3.12).
use core::ops::Range;

use super::{flagged, t};

// Unicode 3.12's jamo and syllable blocks. A syllable's index is its leading consonant's, then its vowel's, then its
// trailing consonant's, which counts from 1 above `NO_TRAILING`, as 0 stands for none.
const LEADING: Range<u32> = 0x1100..0x1100 + 19;
const VOWELS: Range<u32> = 0x1161..0x1161 + 21;
const NO_TRAILING: u32 = 0x11A7;
const TRAILING: Range<u32> = NO_TRAILING + 1..NO_TRAILING + 28;
const PER_VOWEL: u32 = TRAILING.end - NO_TRAILING;
const PER_LEADING: u32 = (VOWELS.end - VOWELS.start) * PER_VOWEL;
const SYLLABLES: Range<u32> = 0xAC00..0xAC00 + (LEADING.end - LEADING.start) * PER_LEADING;

// The syllable of a leading consonant, a vowel and a trailing consonant if any; None where one isn't such a jamo.
pub(crate) fn hangul_syllable(l: u32, v: u32, t: Option<u32>) -> Option<u32> {
    if !LEADING.contains(&l) || !VOWELS.contains(&v) || t.is_some_and(|t| !TRAILING.contains(&t)) {
        return None;
    }
    let trailing = t.map_or(0, |t| t - NO_TRAILING);
    Some(SYLLABLES.start + (l - LEADING.start) * PER_LEADING + (v - VOWELS.start) * PER_VOWEL + trailing)
}

// A syllable's jamo: leading consonant, vowel and trailing consonant if it has one.
pub(crate) fn hangul_jamo(s: u32) -> Option<(u32, u32, Option<u32>)> {
    let index = s.checked_sub(SYLLABLES.start).filter(|_| SYLLABLES.contains(&s))?;
    let (l, rest) = (index / PER_LEADING, index % PER_LEADING);
    let (v, trailing) = (rest / PER_VOWEL, rest % PER_VOWEL);
    Some((LEADING.start + l, VOWELS.start + v, (trailing > 0).then_some(NO_TRAILING + trailing)))
}

// One level of `c`'s canonical decomposition: its two parts, or the one character it is equivalent to.
pub(crate) fn decompose(c: char) -> Option<(char, Option<char>)> {
    if !flagged(c, t::flag::DECOMPOSES) {
        return None;
    }
    let (first, second) = split(u32::from(c))?;
    Some((char::from_u32(first)?, second.and_then(char::from_u32)))
}

// The primary composite of `a` and `b`; None where they have none or composition excludes it.
pub(crate) fn compose(a: char, b: char) -> Option<char> {
    if !flagged(b, t::flag::COMPOSES_SECOND) {
        return None;
    }
    char::from_u32(join(u32::from(a), u32::from(b))?)
}

// Whether `c` is the second of a pair `compose` or `hebrew_presentation_form` joins.
pub(crate) fn composes_second(c: char) -> bool {
    flagged(c, t::flag::COMPOSES_SECOND) || t::HEBREW_PRESENTATION_FORMS.iter().any(|r| u32::from(r.1) == u32::from(c))
}

// The Hebrew presentation form `a` and `b` decompose from, which canonical composition excludes.
pub(crate) fn hebrew_presentation_form(a: char, b: char) -> Option<char> {
    let parts = (u16::try_from(u32::from(a)).ok()?, u16::try_from(u32::from(b)).ok()?);
    let form = row(t::HEBREW_PRESENTATION_FORMS, parts, |&(a, b, _)| (a, b))?.2;
    char::from_u32(u32::from(form))
}

// `decompose` without the flag that rules most characters out first.
fn split(cp: u32) -> Option<(u32, Option<u32>)> {
    if let Some((l, v, t)) = hangul_jamo(cp) {
        // An LV syllable splits into its two jamo; an LVT one into its LV syllable and its trailing consonant.
        return Some(match t {
            None => (l, Some(v)),
            Some(t) => (cp - (t - NO_TRAILING), Some(t)),
        });
    }
    let Ok(key) = u16::try_from(cp) else {
        let pair = row(t::DECOMPOSE_PAIR_SUPP, cp, |r| r.0).map(|(_, a, b)| (a, Some(b)));
        let cjk = cp.checked_sub(t::DECOMPOSE_SINGLE_CJK_BASE).and_then(|i| t::DECOMPOSE_SINGLE_CJK.get(i as usize));
        return pair.or(cjk.map(|&a| (a, None)));
    };
    let pair = || row(t::DECOMPOSE_PAIR_BMP, key, |r| r.0).map(|(_, a, b)| (u32::from(a), Some(u32::from(b))));
    let single = || row(t::DECOMPOSE_SINGLE, key, |r| r.0).map(|(_, a)| (u32::from(a), None));
    let astral = || t::DECOMPOSE_SINGLE_ASTRAL.iter().find(|r| r.0 == key).map(|&(_, a)| (a, None));
    pair().or_else(single).or_else(astral)
}

// `compose` without the flag that rules most characters out first.
fn join(a: u32, b: u32) -> Option<u32> {
    if LEADING.contains(&a) && VOWELS.contains(&b) {
        return hangul_syllable(a, b, None);
    }
    if hangul_jamo(a).is_some_and(|(_, _, t)| t.is_none()) && TRAILING.contains(&b) {
        return Some(a + (b - NO_TRAILING));
    }
    match (u16::try_from(a), u16::try_from(b)) {
        (Ok(a), Ok(b)) => row(t::COMPOSE_BMP, (a, b), |&(a, b, _)| (a, b)).map(|r| u32::from(r.2)),
        _ => row(t::COMPOSE_SUPP, (a, b), |&(a, b, _)| (a, b)).map(|r| r.2),
    }
}

// The row of a table sorted by `key_of` whose key is `key`.
fn row<R: Copy, K: Ord>(rows: &[R], key: K, key_of: impl FnMut(&R) -> K) -> Option<R> {
    rows.binary_search_by_key(&key, key_of).ok().map(|i| rows[i])
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/canonical.rs"]
mod tests;
