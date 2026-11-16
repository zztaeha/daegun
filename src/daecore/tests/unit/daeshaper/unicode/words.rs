use alloc::boxed::Box;
use alloc::vec::Vec;

use super::{divide, language, Found, Language, Script, WordList, Words, MEMO};

// The plain probe, every bucket bisected from the root and its words decoded into a fresh list: the oracle for
// the one that narrows and reads on.
fn probe_from_root(list: &WordList, key: &[u8]) -> (bool, bool, bool) {
    let (mut lo, mut hi) = (0, list.offsets.len() / 4);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if list.first_of(mid) <= key { lo = mid + 1 } else { hi = mid }
    }
    let mut at = super::u32_at(list.offsets, 4 * lo.saturating_sub(1)) as usize;
    let (mut word, mut is_word) = (Vec::new(), false);
    while at < list.words.len() {
        let (shared, rest) = (usize::from(list.words[at]), usize::from(list.words[at + 1]));
        word.truncate(shared);
        word.extend_from_slice(&list.words[at + 2..at + 2 + rest]);
        at += 2 + rest;
        if is_word {
            return (true, true, word.starts_with(key));
        }
        if word.as_slice() >= key {
            if word != key {
                return (false, word.starts_with(key), word.starts_with(key));
            }
            is_word = true;
        }
    }
    (is_word, is_word, false)
}

fn matches_from_root(list: &WordList, chars: &[char]) -> (Vec<usize>, usize) {
    let (mut key, mut lengths) = (Vec::new(), Vec::new());
    for (i, &c) in chars.iter().enumerate() {
        let Some(b) = list.byte(c) else { return (lengths, i + 1) };
        key.push(b);
        let (word, prefix, longer) = probe_from_root(list, &key);
        if !prefix {
            return (lengths, i + 1);
        }
        if word {
            lengths.push(i + 1);
            if !longer {
                return (lengths, i + 1);
            }
        }
    }
    (lengths, chars.len())
}

// The lengths found, shortest first.
fn lengths(found: Found) -> Vec<usize> {
    let mut lengths: Vec<usize> = found.descending().collect();
    lengths.reverse();
    lengths
}

fn agree(list: &WordList, chars: &[char]) {
    let found = list.matches(chars);
    assert_eq!((lengths(found), usize::from(found.prefix)), matches_from_root(list, chars), "{chars:?}");
}

fn character(list: &WordList, b: u8) -> char {
    match b {
        0xFF => '\u{200D}',
        0xFE => '\u{200C}',
        _ => char::from_u32(list.base + u32::from(b)).expect("a character"),
    }
}

// Every prefix of every listed word, and the byte after it, as the oracle probes them.
#[test]
fn narrowed_probes_find_what_probes_from_the_root_find() {
    for script in [Script::THAI, Script::LAO, Script::KHMER, Script::MYANMAR] {
        let list = WordList::new(language(script).expect("a listed script").list);
        let (mut at, mut word, mut n) = (0, Vec::new(), 0usize);
        while at < list.words.len() {
            let (shared, rest) = (usize::from(list.words[at]), usize::from(list.words[at + 1]));
            word.truncate(shared);
            word.extend_from_slice(&list.words[at + 2..at + 2 + rest]);
            at += 2 + rest;
            let mut chars: Vec<char> = word.iter().map(|&b| character(&list, b)).collect();
            chars.push(character(&list, (n % 0x60) as u8));
            agree(&list, &chars);
            n += 1;
        }
        assert!(n > 20_000, "{n} words");
    }
}

// A place kept reads as the words found there afresh, wherever the division looks next: ahead, back, or at a
// place sharing its slot.
#[test]
fn a_place_kept_reads_as_its_words() {
    let text = "ภาษาไทยเป็นภาษาที่มีวรรณยุกต์และใช้อักษรไทยในการเขียนหนังสือ";
    let chars: Vec<char> = text.repeat(3).chars().collect();
    let list = WordList::new(language(Script::THAI).expect("listed").list);
    let mut words = Words::new(WordList::new(language(Script::THAI).expect("listed").list), &chars);
    let n = chars.len();
    assert!(n > 2 * MEMO);
    for at in (0..n).chain((0..n).rev()).chain((0..n).step_by(MEMO)) {
        let (kept, fresh) = (words.at(at), list.matches(&chars[at..]));
        assert_eq!((lengths(kept), kept.prefix), (lengths(fresh), fresh.prefix), "at {at}");
    }
}

// A list of 25 words of ก, each the one before and one more: a word list nested past 20 at a place.
fn nested_list() -> &'static [u8] {
    let mut words = Vec::new();
    let mut offsets = Vec::new();
    for len in 1..=25 {
        if (len - 1) % 16 == 0 {
            offsets.extend_from_slice(&(words.len() as u32).to_le_bytes());
            words.extend_from_slice(&[0, len as u8]);
            words.extend(core::iter::repeat_n(1u8, len));
        } else {
            words.extend_from_slice(&[(len - 1) as u8, 1, 1]);
        }
    }
    let mut data = Vec::new();
    data.extend_from_slice(&0x0E00u32.to_le_bytes());
    data.extend_from_slice(&((offsets.len() / 4) as u32).to_le_bytes());
    data.extend_from_slice(&offsets);
    data.extend_from_slice(&words);
    Box::leak(data.into_boxed_slice())
}

// Every word nested at one place is found, however many: all 25, past which the text reads on as no word's start.
#[test]
fn every_nested_word_is_found() {
    let list = WordList::new(nested_list());
    let chars: Vec<char> = core::iter::repeat_n('\u{0E01}', 34).collect();
    agree(&list, &chars);
    let found = list.matches(&chars);
    assert_eq!((found.count(), found.longest(), usize::from(found.prefix)), (25, 25, 25));
}

// Text that is one word of 25 stays whole, as its longest word takes it to the end.
#[test]
fn the_longest_of_many_nested_words_is_weighed() {
    let lang = Language { list: nested_list(), ends: |_| true, begins: |_| true, thai: false };
    let mut out = Vec::new();
    divide(&core::iter::repeat_n('\u{0E01}', 25).collect::<Vec<_>>(), &lang, &mut out);
    assert_eq!(out, []);
}
