use alloc::vec::Vec;
use core::cmp::Ordering;

use super::{general_category, Script};
use crate::daecore::daeshaper::generated::word_lists as lists;

// Text no word covers is joined to a word shorter than this before it, unless it starts like a word this long.
const COMBINE_BELOW: usize = 3;

// Places a division keeps what starts there: its steps look at most three words ahead.
const MEMO: usize = 64;

const PAIYANNOI: char = '\u{0E2F}';
const MAIYAMOK: char = '\u{0E46}';

// A script's list, and the characters a word may end and start with where text no word covers is passed over.
struct Language {
    list: &'static [u8],
    ends: fn(char) -> bool,
    begins: fn(char) -> bool,
    thai: bool,
}

fn language(script: Script) -> Option<Language> {
    Some(match script {
        Script::THAI => Language {
            list: lists::THAI,
            ends: |c| !matches!(c, '\u{0E31}' | '\u{0E40}'..='\u{0E44}'),
            begins: |c| matches!(c, '\u{0E01}'..='\u{0E2E}' | '\u{0E40}'..='\u{0E44}'),
            thai: true,
        },
        Script::LAO => Language {
            list: lists::LAO,
            ends: |c| !matches!(c, '\u{0EC0}'..='\u{0EC4}'),
            begins: |c| matches!(c, '\u{0E81}'..='\u{0EAE}' | '\u{0EDC}'..='\u{0EDD}' | '\u{0EC0}'..='\u{0EC4}'),
            thai: false,
        },
        Script::KHMER => Language {
            list: lists::KHMER,
            ends: |c| c != '\u{17D2}',
            begins: |c| matches!(c, '\u{1780}'..='\u{17B3}'),
            thai: false,
        },
        Script::MYANMAR => Language {
            list: lists::MYANMAR,
            ends: |_| true,
            begins: |c| matches!(c, '\u{1000}'..='\u{102A}'),
            thai: false,
        },
        _ => return None,
    })
}

// A list as word-lists.py lays it out: its alphabet's base, its buckets' offsets, and its words, sorted and front
// coded, each bucket's first written whole.
struct WordList {
    base: u32,
    offsets: &'static [u8],
    words: &'static [u8],
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    let mut le = [0; 4];
    le.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(le)
}

impl WordList {
    fn new(data: &'static [u8]) -> WordList {
        let (base, count) = (u32_at(data, 0), u32_at(data, 4) as usize);
        let (offsets, words) = data[8..].split_at(4 * count);
        WordList { base, offsets, words }
    }

    fn byte(&self, c: char) -> Option<u8> {
        match c {
            '\u{200D}' => Some(0xFF),
            '\u{200C}' => Some(0xFE),
            _ => (c as u32).checked_sub(self.base).filter(|&d| d <= 0xFD).map(|d| d as u8),
        }
    }

    fn first_of(&self, bucket: usize) -> &'static [u8] {
        let at = u32_at(self.offsets, 4 * bucket) as usize;
        &self.words[at + 2..at + 2 + usize::from(self.words[at + 1])]
    }

    // Whether `key` is a word, starts one, and starts a longer one, read on from where `at` stands for the key before
    // it. Only a word's bytes past what the word before and the key share are read; a bucket's first is read whole.
    fn probe(&self, key: &[u8], at: &mut Cursor) -> (bool, bool, bool) {
        let (mut l, mut h) = (at.lo, at.hi);
        while l < h {
            let mid = (l + h) / 2;
            let first = self.first_of(mid);
            if first <= key {
                l = mid + 1;
            } else {
                h = mid;
                if !first.starts_with(key) {
                    at.hi = mid;
                }
            }
        }
        at.lo = l;
        let n = key.len();
        let start = u32_at(self.offsets, 4 * l.saturating_sub(1)) as usize;
        let (mut word, (mut shared, mut rest)) = match at.word {
            Some(word) if word >= start => (word, self.record(word)),
            _ => (start, self.record(start)),
        };
        // A word read on from starts the key before and shares less than that with the word before it, which sorts
        // below that key, so its byte past the key before is its own.
        let (mut common, mut order) =
            if word == start { compare(rest, 0, key) } else { compare(&rest[n - 1 - shared..], n - 1, key) };
        loop {
            let next = word + 2 + rest.len();
            match order {
                Ordering::Greater => {
                    at.word = Some(word);
                    return (false, common == n, common == n);
                }
                Ordering::Equal => {
                    let longer = next < self.words.len() && match self.record(next) {
                        (0, rest) => rest.starts_with(key),
                        (shared, _) => shared == n,
                    };
                    at.word = Some(next);
                    return (true, true, longer);
                }
                Ordering::Less if next >= self.words.len() => return (false, false, false),
                Ordering::Less => {
                    word = next;
                    (shared, rest) = self.record(word);
                    // A word sharing less with the word before than that word shares with the key sorts past the
                    // key; sharing more, it sorts below with it.
                    (common, order) = if shared == 0 {
                        compare(rest, 0, key)
                    } else {
                        match shared.cmp(&common) {
                            Ordering::Less => (shared, Ordering::Greater),
                            Ordering::Greater => (common, Ordering::Less),
                            Ordering::Equal => compare(rest, shared, key),
                        }
                    };
                }
            }
        }
    }

    // A word's record at `at`: how much it shares with the word before, and the rest of it.
    fn record(&self, at: usize) -> (usize, &'static [u8]) {
        let rest = usize::from(self.words[at + 1]);
        (usize::from(self.words[at]), &self.words[at + 2..at + 2 + rest])
    }

    // The words `chars` starts with, and how far it reads as the start of one: to the character no word continues
    // with, counted, or to a word none continues.
    fn matches(&self, chars: &[char]) -> Found {
        let mut found = Found::NONE;
        let mut key = [0; 256];
        let mut at = Cursor { lo: 0, hi: self.offsets.len() / 4, word: None };
        for (i, &c) in chars.iter().enumerate() {
            let (Some(b), Some(slot)) = (self.byte(c), key.get_mut(i)) else { return found.with_prefix(i + 1) };
            *slot = b;
            let (is_word, prefix, longer) = self.probe(&key[..=i], &mut at);
            if !prefix {
                return found.with_prefix(i + 1);
            }
            if is_word {
                found.add(i + 1);
                if !longer {
                    return found.with_prefix(i + 1);
                }
            }
        }
        found.with_prefix(chars.len())
    }
}

// Where a growing key's search stands: buckets before `lo` start no later than the key, those from `hi` past every
// key it starts; every word before `word`, which the key starts, sorts below every longer key.
struct Cursor {
    lo: usize,
    hi: usize,
    word: Option<usize>,
}

// How far a word shares `key`, and how it sorts against it, the word being `key`'s first `from` bytes and then `rest`.
fn compare(rest: &[u8], from: usize, key: &[u8]) -> (usize, Ordering) {
    let tail = &key[from..];
    let same = rest.iter().zip(tail).take_while(|(a, b)| a == b).count();
    (from + same, rest.get(same).cmp(&tail.get(same)))
}

// The lengths of the words starting at one place, every one, and how far the text there reads as one's start. A word
// of k characters is bit k - 1, as `matches` reads at most 256.
#[derive(Clone, Copy)]
struct Found {
    lengths: [u64; 4],
    prefix: u16,
}

impl Found {
    const NONE: Found = Found { lengths: [0; 4], prefix: 0 };

    fn with_prefix(mut self, prefix: usize) -> Found {
        self.prefix = prefix as u16;
        self
    }

    fn add(&mut self, length: usize) {
        self.lengths[(length - 1) / 64] |= 1 << ((length - 1) % 64);
    }

    fn count(&self) -> usize {
        self.lengths.iter().map(|bits| bits.count_ones() as usize).sum()
    }

    // The lengths, longest first.
    fn descending(self) -> impl Iterator<Item = usize> {
        (0..4).rev().flat_map(move |w| {
            let mut bits = self.lengths[w];
            core::iter::from_fn(move || {
                let bit = bits.checked_ilog2()?;
                bits &= !(1 << bit);
                Some(w * 64 + bit as usize + 1)
            })
        })
    }

    fn longest(self) -> usize {
        self.descending().next().unwrap_or(0)
    }
}

// The words starting at each place of a run, kept by place, as a division asks again where it already looked.
struct Words<'a> {
    list: WordList,
    chars: &'a [char],
    memo: [(usize, Found); MEMO],
}

impl<'a> Words<'a> {
    fn new(list: WordList, chars: &'a [char]) -> Words<'a> {
        Words { list, chars, memo: [(usize::MAX, Found::NONE); MEMO] }
    }

    fn at(&mut self, at: usize) -> Found {
        let slot = &mut self.memo[at % MEMO];
        if slot.0 != at {
            *slot = (at, self.list.matches(&self.chars[at..]));
        }
        slot.1
    }
}

// Word boundaries in `chars`, a listed script's SA run, pushed to `out`: each word lets the most words follow, up to
// three, marks stay with their base. Out of line: inlined into the line breaker, it cost all other text 2.5%.
#[inline(never)]
pub(crate) fn word_boundaries(chars: &[char], script: Script, out: &mut Vec<usize>) {
    if let Some(lang) = language(script) {
        divide(chars, &lang, out);
    }
}

fn divide(chars: &[char], lang: &Language, out: &mut Vec<usize>) {
    let n = chars.len();
    if n < 4 || (lang.thai && n == 4) {
        return;
    }
    let mut words = Words::new(WordList::new(lang.list), chars);
    let mut pos = 0;
    while pos < n {
        let first = words.at(pos);
        let mut len = match first.count() {
            0 | 1 => first.longest(),
            _ => best_first(first, pos, n, &mut words),
        };

        if pos + len < n && len < COMBINE_BELOW {
            let next = words.at(pos + len);
            if next.count() == 0 && (len == 0 || usize::from(next.prefix) < COMBINE_BELOW) {
                let mut at = pos + len + 1;
                while at < n && !((lang.ends)(chars[at - 1]) && (lang.begins)(chars[at]) && words.at(at).count() > 0) {
                    at += 1;
                }
                len = at - pos;
            }
        }
        while pos + len < n && general_category(chars[pos + len]).is_mark() {
            len += 1;
        }
        if lang.thai && pos + len < n && len > 0 {
            len += thai_suffixes(chars, pos + len, &mut words);
        }
        pos += len;
        if pos < n {
            out.push(pos);
        }
    }
}

// Which of the words starting at `pos` lets two more follow, from the longest down; else the shortest one followed
// by any word, else the longest. Every word is weighed, each against every word after it.
fn best_first(first: Found, pos: usize, n: usize, words: &mut Words) -> usize {
    let mut mark = first.longest();
    if pos + mark >= n {
        return mark;
    }
    for length in first.descending() {
        let after = pos + length;
        let second = words.at(after);
        if second.count() == 0 {
            continue;
        }
        mark = length;
        if after + second.longest() >= n {
            return mark;
        }
        if second.descending().any(|k| words.at(after + k).count() > 0) {
            return mark;
        }
    }
    mark
}

// A Thai word takes the elision mark after it, unless one came just before, and the repeat mark after that.
fn thai_suffixes(chars: &[char], mut at: usize, words: &mut Words) -> usize {
    let start = at;
    if words.at(at).count() > 0 || !matches!(chars[at], PAIYANNOI | MAIYAMOK) {
        return 0;
    }
    if chars[at] == PAIYANNOI && !matches!(chars[at - 1], PAIYANNOI | MAIYAMOK) {
        at += 1;
    }
    if chars.get(at) == Some(&MAIYAMOK) && chars[at - 1] != MAIYAMOK {
        at += 1;
    }
    at - start
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/words.rs"]
mod tests;
