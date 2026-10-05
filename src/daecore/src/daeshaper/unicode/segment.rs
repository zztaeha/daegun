use alloc::vec::Vec;

use super::is_extended_pictographic;

mod gcb {
    pub(super) const CR: u8 = 1;
    pub(super) const LF: u8 = 2;
    pub(super) const CONTROL: u8 = 3;
    pub(super) const EXTEND: u8 = 4;
    pub(super) const ZWJ: u8 = 5;
    pub(super) const REGIONAL_INDICATOR: u8 = 6;
    pub(super) const PREPEND: u8 = 7;
    pub(super) const SPACING_MARK: u8 = 8;
    pub(super) const L: u8 = 9;
    pub(super) const V: u8 = 10;
    pub(super) const T: u8 = 11;
    pub(super) const LV: u8 = 12;
    pub(super) const LVT: u8 = 13;
}

mod wb {
    pub(super) const OTHER: u8 = 0;
    pub(super) const CR: u8 = 1;
    pub(super) const LF: u8 = 2;
    pub(super) const NEWLINE: u8 = 3;
    pub(super) const EXTEND: u8 = 4;
    pub(super) const ZWJ: u8 = 5;
    pub(super) const REGIONAL_INDICATOR: u8 = 6;
    pub(super) const FORMAT: u8 = 7;
    pub(super) const KATAKANA: u8 = 8;
    pub(super) const HEBREW_LETTER: u8 = 9;
    pub(super) const ALETTER: u8 = 10;
    pub(super) const SINGLE_QUOTE: u8 = 11;
    pub(super) const DOUBLE_QUOTE: u8 = 12;
    pub(super) const MID_NUM_LET: u8 = 13;
    pub(super) const MID_LETTER: u8 = 14;
    pub(super) const MID_NUM: u8 = 15;
    pub(super) const NUMERIC: u8 = 16;
    pub(super) const EXTEND_NUM_LET: u8 = 17;
    pub(super) const WSEG_SPACE: u8 = 18;
}

pub(crate) fn grapheme_break(c: char) -> u8 {
    super::props(c).grapheme_break
}

pub(crate) fn word_break(c: char) -> u8 {
    super::props(c).word_break
}

fn conjunct_break(c: char) -> u8 {
    super::props(c).indic_conjunct_break
}

mod incb {
    pub(super) const CONSONANT: u8 = 1;
    pub(super) const EXTEND: u8 = 2;
    pub(super) const LINKER: u8 = 3;
}

pub fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = alloc::vec![0usize];
    if chars.is_empty() { return out; }

    for i in 1..chars.len() {
        if grapheme_break_between(&chars, i) {
            out.push(i);
        }
    }
    out.push(chars.len());
    out
}

fn grapheme_break_between(chars: &[char], i: usize) -> bool {
    let before = grapheme_break(chars[i - 1]);
    let after = grapheme_break(chars[i]);

    // GB3, GB4, GB5: CRLF is indivisible, and any other control breaks on both sides.
    if before == gcb::CR && after == gcb::LF { return false; }
    if matches!(before, gcb::CONTROL | gcb::CR | gcb::LF) { return true; }
    if matches!(after, gcb::CONTROL | gcb::CR | gcb::LF) { return true; }

    // GB6, GB7, GB8: Hangul syllable sequences.
    if before == gcb::L && matches!(after, gcb::L | gcb::V | gcb::LV | gcb::LVT) { return false; }
    if matches!(before, gcb::LV | gcb::V) && matches!(after, gcb::V | gcb::T) { return false; }
    if matches!(before, gcb::LVT | gcb::T) && after == gcb::T { return false; }

    // GB9, GB9a, GB9b: marks and Prepend attach to their neighbor.
    if matches!(after, gcb::EXTEND | gcb::ZWJ) { return false; }
    if after == gcb::SPACING_MARK { return false; }
    if before == gcb::PREPEND { return false; }

    // GB11: an emoji ZWJ sequence continues only if the ZWJ was preceded by a pictograph, possibly
    // through a run of Extends. Needs history, not just the adjacent pair.
    if before == gcb::ZWJ && is_extended_pictographic(chars[i]) {
        let mut j = i - 1;
        while j > 0 {
            j -= 1;
            match grapheme_break(chars[j]) {
                gcb::EXTEND => continue,
                _ => break,
            }
        }
        if is_extended_pictographic(chars[j]) { return false; }
    }

    // GB9c: a Devanagari-style conjunct holds together – consonant, then linkers and extends with
    // at least one linker among them, then consonant. Added in Unicode 15.1, and the only rule here
    // that has to look back past an arbitrary run for a *specific* property rather than any of them.
    if conjunct_break(chars[i]) == incb::CONSONANT {
        let mut j = i;
        let mut seen_linker = false;
        while j > 0 {
            j -= 1;
            match conjunct_break(chars[j]) {
                incb::LINKER => seen_linker = true,
                incb::EXTEND => {}
                incb::CONSONANT => {
                    if seen_linker { return false; }
                    break;
                }
                _ => break,
            }
        }
    }

    // GB12, GB13: regional indicators pair up, so a break falls between pairs rather than between
    // every two. Counting back over an unbroken run is what distinguishes the second RI from the
    // third.
    if before == gcb::REGIONAL_INDICATOR && after == gcb::REGIONAL_INDICATOR {
        let mut count = 0usize;
        let mut j = i;
        while j > 0 && grapheme_break(chars[j - 1]) == gcb::REGIONAL_INDICATOR {
            count += 1;
            j -= 1;
        }
        return count.is_multiple_of(2);
    }

    // GB999.
    true
}

pub fn word_boundaries(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = alloc::vec![0usize];
    if chars.is_empty() { return out; }

    // WB4 says Extend, Format and ZWJ are invisible to almost every other rule, so the rules run
    // over the *significant* characters and the boundaries are then mapped back to real indices.
    // Doing it any other way means every rule below repeating the same skip logic.
    let significant: Vec<usize> = (0..chars.len())
        .filter(|&i| {
            let w = word_break(chars[i]);
            let ignorable = matches!(w, wb::EXTEND | wb::FORMAT | wb::ZWJ);
            !ignorable || i == 0 || matches!(word_break(chars[i - 1]), wb::CR | wb::LF | wb::NEWLINE)
        })
        .collect();

    for i in 1..chars.len() {
        if word_break_at(&chars, &significant, i) {
            out.push(i);
        }
    }
    out.push(chars.len());
    out
}

fn word_break_at(chars: &[char], significant: &[usize], i: usize) -> bool {
    let prev_raw = word_break(chars[i - 1]);
    let cur_raw = word_break(chars[i]);

    // WB3, WB3a, WB3b: CRLF is indivisible; anything else newline-ish breaks on both sides.
    if prev_raw == wb::CR && cur_raw == wb::LF { return false; }
    if matches!(prev_raw, wb::NEWLINE | wb::CR | wb::LF) { return true; }
    if matches!(cur_raw, wb::NEWLINE | wb::CR | wb::LF) { return true; }

    // WB3c: ZWJ followed by a pictograph stays joined.
    if prev_raw == wb::ZWJ && is_extended_pictographic(chars[i]) { return false; }

    // WB3d: a run of whitespace is one unit.
    if prev_raw == wb::WSEG_SPACE && cur_raw == wb::WSEG_SPACE { return false; }

    // WB4: an ignorable never introduces a break of its own.
    if matches!(cur_raw, wb::EXTEND | wb::FORMAT | wb::ZWJ) { return false; }

    let Ok(pos) = significant.binary_search(&i) else {
        return false;
    };
    if pos == 0 { return true; }

    let at = |k: usize| word_break(chars[significant[k]]);
    let prev = at(pos - 1);
    let cur = at(pos);
    let next = if pos + 1 < significant.len() { at(pos + 1) } else { wb::OTHER };
    let prev2 = if pos >= 2 { at(pos - 2) } else { wb::OTHER };

    let is_letter = |w: u8| matches!(w, wb::ALETTER | wb::HEBREW_LETTER);

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
    if matches!(prev, wb::ALETTER | wb::HEBREW_LETTER | wb::NUMERIC | wb::KATAKANA | wb::EXTEND_NUM_LET)
        && cur == wb::EXTEND_NUM_LET { return false; }
    if prev == wb::EXTEND_NUM_LET
        && matches!(cur, wb::ALETTER | wb::HEBREW_LETTER | wb::NUMERIC | wb::KATAKANA) { return false; }

    // WB15, WB16: regional indicators pair up, counted over significant characters only.
    if prev == wb::REGIONAL_INDICATOR && cur == wb::REGIONAL_INDICATOR {
        let mut count = 0usize;
        let mut k = pos;
        while k > 0 && at(k - 1) == wb::REGIONAL_INDICATOR {
            count += 1;
            k -= 1;
        }
        return count.is_multiple_of(2);
    }

    // WB999.
    true
}
