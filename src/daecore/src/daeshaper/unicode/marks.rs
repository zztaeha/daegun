// The class a mark sorts by in canonical ordering (Unicode 3.11): its combining class, moved where a script's fonts
// expect another order. Hebrew's and Arabic's reordering and fallback positioning name these values.
use super::props;

pub(crate) fn mark_class(c: char) -> u8 {
    match c {
        // Tibetan's TSA -PHRU sits on its consonant, so before the vowel signs (129–132) rather than after (216).
        '\u{0F39}' => 127,
        // Tibetan's PADMA GDAN goes after any vowel sign and Tai Tham's SAKOT after any tone mark: both last.
        '\u{0FC6}' | '\u{1A60}' => 254,
        _ => SORT_CLASS[usize::from(props(c).combining_class)],
    }
}

// Hebrew's fixed-position point classes (10–26) in the order the SBL Hebrew font manual gives: the shin and sin dots,
// dagesh, rafe, holam, the hatafs, the other vowel points, then meteg and varika.
const HEBREW: [u8; 17] = [24, 25, 21, 23, 19, 11, 12, 13, 15, 16, 17, 18, 10, 14, 20, 22, 26];

// Arabic's harakat classes (27–32) with shadda's (33) first, so ordering leaves a shadda before the vowel it carries.
const ARABIC: [u8; 7] = [33, 27, 28, 29, 30, 31, 32];

// Single moves, each to a class nothing else sorts by.
const MOVES: [(u8, u8); 5] = [
    // Telugu's length marks, the only vowel signs of the major Indic scripts with a class, before a virama (9).
    (84, 4),
    (91, 5),
    // Thai's SARA U and SARA UU before PHINTHU (9).
    (103, 3),
    // Tibetan's vowel sign U before I, as Dzongkha's stacked vowels need.
    (130, 132),
    (132, 131),
];

// Each Canonical_Combining_Class's sort class, built as the crate builds.
static SORT_CLASS: [u8; 256] = {
    let mut table = [0; 256];
    let mut class = 0;
    while class < table.len() {
        table[class] = class as u8;
        class += 1;
    }
    rank(&mut table, &HEBREW);
    rank(&mut table, &ARABIC);
    let mut i = 0;
    while i < MOVES.len() {
        table[MOVES[i].0 as usize] = MOVES[i].1;
        i += 1;
    }
    table
};

// The classes in `order` take the span of classes they cover, in that order.
const fn rank(table: &mut [u8; 256], order: &[u8]) {
    let mut least = order[0];
    let mut i = 1;
    while i < order.len() {
        if order[i] < least {
            least = order[i];
        }
        i += 1;
    }
    let mut i = 0;
    while i < order.len() {
        table[order[i] as usize] = least + i as u8;
        i += 1;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/marks.rs"]
mod tests;
