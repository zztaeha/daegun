use super::*;
use super::super::dict::Private;

// BlueValues -15 0 500 515, at 16 px at an em of 1,000 units.
fn zones() -> Zones {
    let p = Private::read(&[124, 154, 248, 136, 154, 6]).expect("a Private DICT");
    Zones::new(&p, Scale::new(16 * 64 * 65536 / 1000).expect("a scale"), None)
}

fn hinter() -> Hinter {
    Hinter::new(zones(), (0.0, 0.0), Outline::default())
}

// A box from 0 to 300 units across and 0 to `top` up.
fn draw_box(h: &mut Hinter, top: f64) {
    h.move_to(0.0, 0.0);
    h.line_to(300.0, 0.0);
    h.line_to(300.0, top);
    h.line_to(0.0, top);
    h.line_to(0.0, 0.0);
}

// Stems declared once the list is fixed aren't hints, though a mask still counts them.
#[test]
fn stems_after_the_first_mask_are_counted_but_not_kept() {
    let mut h = hinter();
    h.stems(&[0.0, 20.0], false);
    h.mask(&[0x80], false);
    h.stems(&[100.0, 20.0, 200.0, 20.0], false);
    assert_eq!((h.stems.len(), h.declared), (1, 3));
}

// The box's top at 507 units, in the x-height zone, lands on its flat pixel, 8 px at 16 px; its bottom on 0; and
// the line back to the start is left to the close.
#[test]
fn a_box_aligns_to_its_zones() {
    let mut h = hinter();
    h.stems(&[21.0, -21.0, 507.0, -20.0], false);
    draw_box(&mut h, 507.0);
    let o = h.finish().finish().expect("hinted");
    assert_eq!((o.y, o.contour_ends), (alloc::vec![0, 0, 512, 512], alloc::vec![3]));
}

// A point drawn after a hintmask maps by the new set; the moveto before it keeps where the old set put it, as a
// hintmask doesn't move the current point (TN 5177, hintmask). The stem, 100.5 to 140.5 units, falls a quarter pixel.
#[test]
fn a_hintmask_applies_from_the_next_point() {
    let mut h = hinter();
    h.stems(&[100.5, 40.0], false);
    h.mask(&[0x00], false);
    h.move_to(0.0, 120.0);
    h.mask(&[0x80], false);
    h.line_to(10.0, 120.0);
    let o = h.finish().finish().expect("hinted");
    assert_eq!(o.y, [123, 107]);
}

// CFF2 sets no limit on stems, so a glyph of 200 stems 15 units apart hints by stem 151, at 2,265 to 2,270 units, as
// one declaring only it and the stems the zones align (0, 33 and 34, which anchor the free ones) does.
#[test]
fn a_glyph_of_more_than_96_stems_hints_by_each() {
    let drawn = |stems: &[usize], bit: usize| {
        let mut h = hinter();
        // Each pair's edge counts from the last pair's top.
        let tops = core::iter::once(0.0).chain(stems.iter().map(|&i| 15.0 * i as f64 + 5.0));
        h.stems(&stems.iter().zip(tops).flat_map(|(&i, top)| [15.0 * i as f64 - top, 5.0]).collect::<Vec<_>>(), false);
        let mut mask = alloc::vec![0u8; stems.len().div_ceil(8)];
        mask[bit / 8] = 0x80 >> (bit % 8);
        h.mask(&mask, false);
        h.move_to(0.0, 2265.0);
        h.line_to(10.0, 2270.0);
        h.finish().finish().expect("hinted").y
    };
    let all: Vec<usize> = (0..200).collect();
    let y = drawn(&all, 151);
    assert_eq!(y, drawn(&[0, 33, 34, 151], 3));
    assert_ne!(y, [2319, 2325], "stem 151 left the points where scaling put them");
}

// Any number of hint sets is placed, each charged to the charstring's steps by the stems it looks over and takes:
// 2 for the placement and 2 for each stem taken, here 4 a set, after 10 for fixing the list and placing every stem.
#[test]
fn every_hint_set_is_placed_and_charged() {
    let mut h = hinter();
    h.stems(&[0.0, 10.0, 100.0, 10.0], false);
    h.move_to(0.0, 0.0);
    let frozen = h.take_work();
    for i in 0..2_000 {
        h.mask(&[if i % 2 == 0 { 0x80 } else { 0x40 }], false);
        h.line_to(i as f64, 0.0);
    }
    assert_eq!((frozen, h.take_work()), (2 * 2 + 2 + 2 * 2, 2_000 * 4));
    assert_eq!(h.finish().finish().map(|o| o.x.len()), Some(2_000));
}

// Values past 16.16's range and NaN, in stems, masks and segments, hint without a panic under debug's overflow
// checks; a NaN lands where 0 does.
#[test]
fn wild_values_hint_without_a_panic() {
    let draw = |v: f64| {
        let mut h = Hinter::new(zones(), (v, v), Outline::default());
        h.stems(&[v, 32767.0, -32768.0, v, 500.0, 15.0, v, v], false);
        h.mask(&[0xFF, 0xFF], true);
        h.move_to(v, -v);
        h.line_to(32767.99, v);
        h.curve_to((v, 32767.99), (-32768.0, v), (v, v));
        h.mask(&[0x40], false);
        h.line_to(v, 0.0);
        h.finish().finish()
    };
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 32767.99, -32768.0, 1e300, 0.0] {
        draw(v);
    }
    let zero = draw(0.0);
    assert!(zero.as_ref().is_some_and(|o| o.x.len() > 3));
    assert_eq!(draw(f64::NAN), zero);
}

// A CFF2 INDEX of `items`, offSize 4.
fn index(items: &[&[u8]]) -> Vec<u8> {
    let mut out = [&(items.len() as u32).to_be_bytes()[..], &[4]].concat();
    let mut at = 1u32;
    out.extend(at.to_be_bytes());
    for item in items {
        at += item.len() as u32;
        out.extend(at.to_be_bytes());
    }
    items.iter().for_each(|item| out.extend_from_slice(item));
    out
}

// A charstring number from -107 to 107, or a 16-bit one.
fn num(v: i32) -> Vec<u8> {
    match v {
        -107..=107 => alloc::vec![(v + 139) as u8],
        _ => [&[28u8][..], &(v as i16).to_be_bytes()].concat(),
    }
}

// A CFF2 table of one glyph drawn by `charstring`, with local subroutines `subrs` and BlueValues -15 0 500 515, at
// 16 px on an em of 1,000 units: the glyph hinted, if it is.
fn hinted_cff2(charstring: &[u8], subrs: &[&[u8]]) -> Option<crate::daecore::daetype::hinting::HintedOutline> {
    use crate::daecore::daetype::outline::{hint_cff2_glyph, Cff2Outlines};
    let int = |v: usize| [&[29u8][..], &(v as u32).to_be_bytes()].concat();
    let blues = [num(-15), num(15), num(500), num(15), alloc::vec![6]].concat();
    let private = [blues, int(blues_len() + 6), alloc::vec![19]].concat();
    let cs_at = 5 + 13 + 4;
    let charstrings = index(&[charstring]);
    let fd_at = cs_at + charstrings.len();
    let fd = |private_at: usize| [int(private.len()), int(private_at), alloc::vec![18]].concat();
    let private_at = fd_at + index(&[&fd(0)]).len();
    let mut t = alloc::vec![2, 0, 5, 0, 13];
    t.extend([int(cs_at), alloc::vec![17], int(fd_at), alloc::vec![12, 36]].concat());
    t.extend(0u32.to_be_bytes());
    t.extend(charstrings);
    t.extend(index(&[&fd(private_at)]));
    t.extend(&private);
    t.extend(index(subrs));
    let outlines = Cff2Outlines::parse_in(&t, 1000, None).expect("the table parses");
    hint_cff2_glyph(&outlines, &t, 0, None, crate::daecore::daetype::hinting::f26dot6::Size::from(16).scale16(1000))
}

fn blues_len() -> usize {
    [num(-15), num(15), num(500), num(15)].concat().len() + 1
}

// `stems` 5 units tall at 15 units each, by one hstemhm: each pair's edge counts from the last pair's top. CFF2's 513
// operands hold 256 of them.
fn stems_at(stems: &[usize]) -> Vec<u8> {
    let tops = core::iter::once(0).chain(stems.iter().map(|&i| 15 * i as i32 + 5));
    let pairs = stems.iter().zip(tops).flat_map(|(&i, top)| [num(15 * i as i32 - top), num(5)].concat());
    [pairs.collect(), alloc::vec![18]].concat()
}

fn hintmask(stems: usize, bits: &[usize]) -> Vec<u8> {
    let mut mask = alloc::vec![0u8; stems.div_ceil(8)];
    bits.iter().for_each(|&b| mask[b / 8] |= 0x80 >> (b % 8));
    [alloc::vec![19], mask].concat()
}

// CFF2 sets no limit on stems: a glyph of 200 hints by stem 151, as one declaring only the four that matter does.
#[test]
fn a_cff2_glyph_of_200_stems_hints() {
    let drawn = |stems: &[usize], bit: usize| {
        let path = [num(0), num(2265), alloc::vec![21], num(10), num(5), alloc::vec![5]].concat();
        hinted_cff2(&[stems_at(stems), hintmask(stems.len(), &[bit]), path].concat(), &[]).map(|o| o.y)
    };
    let all: Vec<usize> = (0..200).collect();
    let y = drawn(&all, 151).expect("200 stems left the glyph unhinted");
    assert_eq!(Some(y), drawn(&[0, 33, 34, 151], 3));
}

// Hint sets are paid from the charstring's steps, 64 a byte it reads: 2,000 sets of 2 stems hint, as do 10 calls of
// a subroutine placing two 500-stem sets at about 1,070 steps each; 100 calls, 217,000 steps for 86,000, don't.
#[test]
fn hint_sets_are_paid_for_by_the_charstring_s_steps() {
    let pair = |n: usize, a: &[usize], b: &[usize]| {
        [hintmask(n, a), num(1), num(0), alloc::vec![5], hintmask(n, b), num(-1), num(0), alloc::vec![5]].concat()
    };
    let start = [num(0), num(0), alloc::vec![21]].concat();
    let two = [stems_at(&[0, 10]), start.clone(), pair(2, &[0], &[1]).repeat(1_000)].concat();
    assert_eq!(hinted_cff2(&two, &[]).map(|o| o.y.len()), Some(2_000), "2,000 hint sets left the glyph unhinted");
    let all: Vec<usize> = (0..500).collect();
    let subr = [pair(500, &all, &all[1..]), alloc::vec![11]].concat();
    let calls = |k: usize| [num(-107), alloc::vec![10]].concat().repeat(k);
    let stems = [stems_at(&all[..250]), stems_at(&all[250..])].concat();
    let hinted = |k: usize| hinted_cff2(&[&stems[..], &start, &calls(k)].concat(), &[&subr]).is_some();
    assert_eq!([10, 100].map(hinted), [true, false]);
}
