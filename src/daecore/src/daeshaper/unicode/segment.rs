use alloc::vec::Vec;

use super::props;
use crate::daecore::daeshaper::generated::unicode_tables::{
    flag, grapheme_break as gcb, indic_conjunct_break as incb, word_break as wb, Props,
};

fn pictograph(p: &Props) -> bool {
    p.flags & flag::EXTENDED_PICTOGRAPHIC != 0
}

// Where `each_grapheme_boundary` puts each offset: Rust's list, or C's count and buffer.
pub(crate) trait Boundaries {
    fn put(&mut self, at: usize);
}

impl Boundaries for Vec<usize> {
    fn put(&mut self, at: usize) {
        self.push(at);
    }
}

pub fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let mut out = Vec::with_capacity(text.chars().count() + 1);
    each_grapheme_boundary(text, &mut out);
    out
}

// Byte offsets, each at a char boundary, as every position in the API is: each read from the text left before a
// character is taken, which costs less than char_indices' own count.
pub(crate) fn each_grapheme_boundary(text: &str, out: &mut impl Boundaries) {
    out.put(0);
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return };

    // What GB11, GB9c and GB12/13 read behind a position, carried so none counts back: a pictograph before
    // `before`'s Extends, a linker past the conjunct Extends through `before`, and the RI run ending at `before`.
    let (mut before, mut pictograph_behind, mut linked, mut ri_run) = (props(first), false, false, 0usize);
    let second = first.len_utf8();
    loop {
        let at = text.len() - chars.as_str().len();
        let Some(c) = chars.next() else { break };
        let after = props(c);
        ri_run = if before.grapheme_break == gcb::REGIONAL_INDICATOR { ri_run + 1 } else { 0 };
        if before.indic_conjunct_break != incb::EXTEND {
            linked = before.indic_conjunct_break == incb::LINKER;
        }
        if grapheme_break_between(before, after, pictograph_behind, linked, ri_run) {
            out.put(at);
        }
        if before.grapheme_break != gcb::EXTEND || at == second {
            pictograph_behind = pictograph(before);
        }
        before = after;
    }
    out.put(text.len());
}

fn grapheme_break_between(before: &Props, after: &Props, pictograph_behind: bool, linked: bool, ri_run: usize) -> bool {
    let (b, a) = (before.grapheme_break, after.grapheme_break);

    // GB3, GB4, GB5: CRLF is indivisible, and any other control breaks on both sides.
    if b == gcb::CR && a == gcb::LF { return false; }
    if matches!(b, gcb::CONTROL | gcb::CR | gcb::LF) { return true; }
    if matches!(a, gcb::CONTROL | gcb::CR | gcb::LF) { return true; }

    // GB6, GB7, GB8: Hangul syllable sequences.
    if b == gcb::L && matches!(a, gcb::L | gcb::V | gcb::LV | gcb::LVT) { return false; }
    if matches!(b, gcb::LV | gcb::V) && matches!(a, gcb::V | gcb::T) { return false; }
    if matches!(b, gcb::LVT | gcb::T) && a == gcb::T { return false; }

    // GB9, GB9a, GB9b: marks and Prepend attach to their neighbor.
    if matches!(a, gcb::EXTEND | gcb::ZWJ) { return false; }
    if a == gcb::SPACING_MARK { return false; }
    if b == gcb::PREPEND { return false; }

    // GB11: an emoji ZWJ sequence continues only if the ZWJ was preceded by a pictograph, possibly through a run of
    // Extends.
    if b == gcb::ZWJ && pictograph(after) && pictograph_behind { return false; }

    // GB9c: a linker, past any extends, holds the consonant after it to its conjunct.
    if after.indic_conjunct_break == incb::CONSONANT && linked { return false; }

    // GB12, GB13: regional indicators pair up, so a break falls between pairs rather than between every two.
    if b == gcb::REGIONAL_INDICATOR && a == gcb::REGIONAL_INDICATOR {
        return ri_run.is_multiple_of(2);
    }

    // GB999.
    true
}

// Byte offsets, read as `grapheme_boundaries` reads them, in one pass; the class past the ignorables ahead is found
// when a rule reads it.
pub fn word_boundaries(text: &str) -> Vec<usize> {
    let mut out = Vec::with_capacity(text.chars().count() + 1);
    out.push(0);
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return out };

    // WB4 makes Extend, Format and ZWJ invisible to almost every rule, so the rules read *significant* characters:
    // the two behind and WB15/16's RI run carried forward, beside the class just behind, ignorable or not.
    let ignorable = |k: u8| matches!(k, wb::EXTEND | wb::FORMAT | wb::ZWJ);
    let after_newline = |k: u8| matches!(k, wb::CR | wb::LF | wb::NEWLINE);
    let mut prev_raw = props(first).word_break;
    let (mut prev, mut prev2) = (prev_raw, wb::OTHER);
    let mut ri_run = usize::from(prev == wb::REGIONAL_INDICATOR);
    loop {
        let at = text.len() - chars.as_str().len();
        let Some(c) = chars.next() else { break };
        let cur = props(c);
        let next = || {
            let mut before = cur.word_break;
            for c in chars.clone() {
                let k = props(c).word_break;
                if !ignorable(k) || after_newline(before) {
                    return k;
                }
                before = k;
            }
            wb::OTHER
        };
        if word_break_at(prev_raw, cur, prev, prev2, ri_run, next) {
            out.push(at);
        }
        if !ignorable(cur.word_break) || after_newline(prev_raw) {
            (prev2, prev) = (prev, cur.word_break);
            ri_run = if prev == wb::REGIONAL_INDICATOR { ri_run + 1 } else { 0 };
        }
        prev_raw = cur.word_break;
    }
    out.push(text.len());
    out
}

fn word_break_at(prev_raw: u8, cur: &Props, prev: u8, prev2: u8, ri_run: usize, next: impl Fn() -> u8) -> bool {
    let cur_raw = cur.word_break;

    // WB3, WB3a, WB3b: CRLF is indivisible; anything else newline-ish breaks on both sides.
    if prev_raw == wb::CR && cur_raw == wb::LF { return false; }
    if matches!(prev_raw, wb::NEWLINE | wb::CR | wb::LF) { return true; }
    if matches!(cur_raw, wb::NEWLINE | wb::CR | wb::LF) { return true; }

    // WB3c: ZWJ followed by a pictograph stays joined.
    if prev_raw == wb::ZWJ && pictograph(cur) { return false; }

    // WB3d: a run of whitespace is one unit.
    if prev_raw == wb::W_SEG_SPACE && cur_raw == wb::W_SEG_SPACE { return false; }

    // WB4: an ignorable never introduces a break of its own, and past it every character here is significant.
    if matches!(cur_raw, wb::EXTEND | wb::FORMAT | wb::ZWJ) { return false; }

    let cur = cur_raw;
    let next = if matches!(cur, wb::MID_LETTER | wb::MID_NUM_LET | wb::SINGLE_QUOTE | wb::DOUBLE_QUOTE | wb::MID_NUM) {
        next()
    } else {
        wb::OTHER
    };

    let is_letter = |w: u8| matches!(w, wb::A_LETTER | wb::HEBREW_LETTER);

    // WB5.
    if is_letter(prev) && is_letter(cur) { return false; }
    // WB6, WB7: a letter, one mid-letter punctuation, a letter – the apostrophe in "don't".
    if is_letter(prev) && matches!(cur, wb::MID_LETTER | wb::MID_NUM_LET | wb::SINGLE_QUOTE)
        && is_letter(next) { return false; }
    if is_letter(prev2) && matches!(prev, wb::MID_LETTER | wb::MID_NUM_LET | wb::SINGLE_QUOTE)
        && is_letter(cur) { return false; }
    // WB7a, WB7b, WB7c: Hebrew's own quote handling.
    if prev == wb::HEBREW_LETTER && cur == wb::SINGLE_QUOTE { return false; }
    if prev == wb::HEBREW_LETTER && cur == wb::DOUBLE_QUOTE && next == wb::HEBREW_LETTER { return false; }
    if prev2 == wb::HEBREW_LETTER && prev == wb::DOUBLE_QUOTE && cur == wb::HEBREW_LETTER { return false; }
    // WB8, WB9, WB10: numbers, and letters running into numbers either way.
    if prev == wb::NUMERIC && cur == wb::NUMERIC { return false; }
    if is_letter(prev) && cur == wb::NUMERIC { return false; }
    if prev == wb::NUMERIC && is_letter(cur) { return false; }
    // WB11, WB12: one separator inside a number – the comma in "1,000".
    if prev2 == wb::NUMERIC && matches!(prev, wb::MID_NUM | wb::MID_NUM_LET | wb::SINGLE_QUOTE)
        && cur == wb::NUMERIC { return false; }
    if prev == wb::NUMERIC && matches!(cur, wb::MID_NUM | wb::MID_NUM_LET | wb::SINGLE_QUOTE)
        && next == wb::NUMERIC { return false; }
    // WB13, WB13a, WB13b: Katakana, and ExtendNumLet gluing either side.
    if prev == wb::KATAKANA && cur == wb::KATAKANA { return false; }
    if matches!(prev, wb::A_LETTER | wb::HEBREW_LETTER | wb::NUMERIC | wb::KATAKANA | wb::EXTEND_NUM_LET)
        && cur == wb::EXTEND_NUM_LET { return false; }
    if prev == wb::EXTEND_NUM_LET
        && matches!(cur, wb::A_LETTER | wb::HEBREW_LETTER | wb::NUMERIC | wb::KATAKANA) { return false; }

    // WB15, WB16: regional indicators pair up, counted over significant characters only.
    if prev == wb::REGIONAL_INDICATOR && cur == wb::REGIONAL_INDICATOR {
        return ri_run.is_multiple_of(2);
    }

    // WB999.
    true
}
