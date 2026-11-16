use super::*;

// A real operand spelled in decimal.
fn real(s: &str) -> Vec<u8> {
    let mut nibbles: Vec<u8> = s.bytes().map(|c| if c == b'.' { 0xA } else { c - b'0' }).collect();
    nibbles.extend([0xF, 0xF]);
    let mut b = alloc::vec![30u8];
    b.extend(nibbles.as_chunks::<2>().0.iter().map(|&[hi, lo]| hi << 4 | lo));
    b
}

fn matrix(ops: &[&[u8]]) -> Option<FontMatrix> {
    let dict = ops.concat();
    FontMatrix::read(&dict, 0..dict.len())
}

const ZERO: &[u8] = &[139];
const ONE_OP: &[u8] = &[140];

// 0.001 with head's em of 1000, or whole numbers, place nothing; a shear of .000167 over .001 is 0.167 in 16.16.
#[test]
fn a_top_matrix_is_measured_in_its_own_em() {
    let thousandth = real(".001");
    let plain = matrix(&[&thousandth, ZERO, ZERO, &thousandth, ZERO, ZERO]);
    assert_eq!(Placement::of(plain, None, 1000), [None]);
    assert_eq!(Placement::of(matrix(&[ONE_OP, ZERO, ZERO, ONE_OP, ZERO, ZERO]), None, 1000), [None]);
    let shear = real(".000167");
    let sheared = Placement::of(matrix(&[&thousandth, ZERO, &shear, &thousandth, ZERO, ZERO]), None, 1000);
    assert_eq!(sheared[0].map(|p| (p.matrix, p.y_scale)), Some(([ONE, 0, 10_945, ONE], ONE)));
    assert_eq!(matrix(&[ONE_OP, ZERO, ZERO, ONE_OP, ZERO]), None, "five operands read");
}

// A CIDFont's FD matrix applies before the Top DICT's: halved under a 0.001 Top DICT, it halves the glyph, which the
// hinter hints at half the size. Without a Top DICT matrix an FD of 1 unit to the em scales by head's em.
#[test]
fn an_fd_matrix_goes_before_the_top_dicts() {
    let thousandth = real(".001");
    let top = matrix(&[&thousandth, ZERO, ZERO, &thousandth, ZERO, ZERO]);
    let half = real(".5");
    let halved = matrix(&[&half, ZERO, ZERO, &half, ZERO, ZERO]);
    let fds = [halved, matrix(&[ONE_OP, ZERO, ZERO, ONE_OP, ZERO, ZERO]), None];
    let placed = Placement::of(top, Some(&fds), 2048);
    assert_eq!(placed[0].map(|p| (p.matrix, p.y_scale)), Some(([ONE / 2, 0, 0, ONE / 2], ONE / 2)));
    assert_eq!((placed[1], placed[2]), (None, None));
    let alone = Placement::of(None, Some(&fds[1..2]), 1000);
    assert_eq!(alone[0].map(|p| p.hint_scale(ONE)), Some(1000 * ONE));
}

// A singular FD matrix falls back to the Top DICT's placement.
#[test]
fn an_unusable_fd_matrix_falls_back_to_the_top_dicts() {
    let thousandth = real(".001");
    let top = matrix(&[&thousandth, ZERO, &thousandth, &thousandth, ZERO, ZERO]);
    let singular = [matrix(&[ONE_OP, ONE_OP, ONE_OP, ONE_OP, ZERO, ZERO])];
    let placed = Placement::of(top, Some(&singular), 1000);
    assert_eq!(placed, Placement::of(top, None, 1000));
    assert!(placed[0].is_some());
}

// A drawn y back in glyph space: 1,000 units drawn at y scale 2 and offset 100 are 450 glyph units, and under a y
// scale of 1/65536 it holds at 2^31 units. A matrix that turns x into y or flips y has no glyph y for a drawn one.
#[test]
fn a_drawn_y_maps_back_only_where_it_follows_glyph_y_alone() {
    let p = Placement { matrix: [ONE, 0, ONE, 2 * ONE], offset: [0, 100 * ONE], y_scale: 2 * ONE };
    assert_eq!(p.glyph_y(1000 * ONE), Some(450 * ONE));
    let tiny = Placement { matrix: [ONE, 0, 0, 1], offset: [0, -(1 << 47)], y_scale: 1 };
    assert_eq!(tiny.glyph_y(30_000 * ONE), Some(1 << 47));
    assert_eq!(Placement { matrix: [ONE, ONE, 0, ONE], ..p }.glyph_y(ONE), None);
    assert_eq!(Placement { matrix: [ONE, 0, 0, -ONE], ..p }.glyph_y(ONE), None);
}

// Hinted at twice the size, a point goes through the matrix with that scale taken out, then the offset moves it 3
// units at the size: (64, 64) in 26.6 sheared by 0.5 and moved 3 units at 1 px a unit.
#[test]
fn place_takes_the_y_scale_out_and_moves_by_the_offset_at_the_size() {
    let p = Placement { matrix: [2 * ONE, 0, ONE, 2 * ONE], offset: [3 * ONE, 0], y_scale: 2 * ONE };
    assert_eq!(p.hint_scale(64 * ONE), 128 * ONE);
    let (x, y, flags, contour_ends) = (alloc::vec![64], alloc::vec![64], alloc::vec![1], alloc::vec![0]);
    let out = HintedOutline { x, y, flags, contour_ends };
    let placed = p.place(out.clone(), 64 * ONE).expect("in range");
    assert_eq!((placed.x[0], placed.y[0]), (64 + 32 + 3 * 64, 64));
    let far = Placement { offset: [1 << 40, 0], ..p };
    assert!(far.place(out, 64 * ONE).is_none(), "a point past 26.6's range was placed");
}
