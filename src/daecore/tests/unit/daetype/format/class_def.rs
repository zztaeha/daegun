use super::*;
use alloc::vec;

fn format2_of(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut out = [2u16, ranges.len() as u16].map(u16::to_be_bytes).concat();
    ranges.iter().for_each(|&(s, e, c)| out.extend([s, e, c].map(u16::to_be_bytes).concat()));
    out
}

fn format4_of(ranges: &[(u32, u32, u16)]) -> Vec<u8> {
    let mut out = vec![0, 4];
    out.extend(&(ranges.len() as u32).to_be_bytes()[1..]);
    for &(s, e, c) in ranges {
        [s, e].iter().for_each(|g| out.extend(&g.to_be_bytes()[1..]));
        out.extend(c.to_be_bytes());
    }
    out
}

// Format 1 from glyph 65,535 with two values: the second classes glyph 65,536, as `class_of` reads it.
#[test]
fn format_1_classes_run_past_glyph_65_535() {
    let t = [1u16, u16::MAX, 2, 1, 3].map(u16::to_be_bytes).concat();
    assert_eq!(classes(&t), Some(vec![(65_535, 1), (65_536, 3)]));
    assert_eq!((class_of(&t, 0), class_of(&t, 65_536)), (0, 3));
}

// The search's own answers on ranges out of order: glyph 35's range is never reached, glyph 15's is. Format 4
// answers the same past glyph 65,535.
#[test]
fn out_of_order_ranges_list_what_the_search_finds() {
    let runs = [(30, 40, 1), (5, 8, 1), (10, 20, 3)];
    let wide = |g: u16| u32::from(g) + 0x1_0000;
    let format4 = format4_of(&runs.map(|(first, last, class)| (wide(first), wide(last), class)));
    for (t, base) in [(format2_of(&runs), 0), (format4, 0x1_0000)] {
        assert_eq!((class_of(&t, base + 6), class_of(&t, base + 15), class_of(&t, base + 35)), (1, 3, 0));
        let listed = classes(&t).expect("a ClassDef");
        let want = (base + 5..=base + 8).map(|g| (g, 1)).chain((base + 10..=base + 20).map(|g| (g, 3)));
        assert_eq!(listed, want.collect::<Vec<_>>());
        let class = |g| listed.iter().find(|e| e.0 == g).map_or(0, |e| e.1);
        assert!((base..base + 0x100).all(|g| class(g) == class_of(&t, g)));
    }
}

// A count past the records held, and a backward range: the rest still read.
#[test]
fn records_past_the_table_or_backward_cost_only_themselves() {
    let mut t = format2_of(&[(1, 2, 4), (9, 3, 5), (10, 10, 6)]);
    t[3] = 9;
    assert_eq!(classes(&t), Some(vec![(1, 4), (2, 4), (10, 6)]));
    assert_eq!(classes(&t[..2]), None);
}

// Format 3's 24-bit start and classes, and format 4's 24-bit ranges of 16-bit classes, read and listed alike.
#[test]
fn formats_3_and_4_read_24_bits() {
    let mut format3 = vec![0, 3, 1, 0, 0, 0, 0, 3];
    [2u32, 0, 0x1_0003].iter().for_each(|c| format3.extend(&c.to_be_bytes()[1..]));
    let read = |t: &[u8], glyphs: [u32; 3]| glyphs.map(|g| class_of(t, g));
    assert_eq!(read(&format3, [0x1_0000, 0x1_0002, 0x1_0003]), [2, 0x1_0003, 0]);
    assert_eq!(classes(&format3), Some(vec![(0x1_0000, 2), (0x1_0002, 0x1_0003)]));
    let format4 = format4_of(&[(0x1_0000, 0x1_0001, 5), (0x2_0000, 0x2_0000, 6)]);
    assert_eq!(read(&format4, [0x1_0001, 0x2_0000, 0x2_0001]), [5, 6, 0]);
    assert_eq!(class_runs(&format4), Some(vec![(0x1_0000, 0x1_0001, 5), (0x2_0000, 0x2_0000, 6)]));
    assert_eq!((classes(&format3[..4]), classes(&format4[..4])), (None, None));
}
