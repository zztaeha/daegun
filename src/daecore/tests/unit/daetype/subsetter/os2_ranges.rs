use super::*;

// Basic Latin's last code point and Latin-1's first each keep their own bit; one past the BMP keeps bit 57 with its
// block's, Mathematical Alphanumeric Symbols' 89, or alone in an unnamed block; a bit the font left clear stays clear.
#[test]
fn each_kept_code_point_keeps_its_blocks_bit() {
    let pruned = |stated: u8, codes: &[u32]| {
        let mut os2 = vec![stated; 58];
        prune_unicode_ranges(&mut os2, codes.iter().copied());
        [42, 46, 50, 54].map(|at| read_u32_be(&os2, at).expect("a range"))
    };
    assert_eq!(pruned(0xFF, &[0x7F]), [1, 0, 0, 0]);
    assert_eq!(pruned(0xFF, &[0x80]), [2, 0, 0, 0]);
    assert_eq!(pruned(0xFF, &[0x1D400]), [0, 1 << 25, 1 << 25, 0]);
    assert_eq!(pruned(0xFF, &[0x1F642]), [0, 1 << 25, 0, 0]);
    assert_eq!(pruned(0, &[0x41, 0x1D400]), [0; 4]);
}
