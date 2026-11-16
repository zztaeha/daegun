use super::*;

// Two records of four glyphs, each row 8 bytes: pixel size, max width, four widths.
fn hdmx() -> Vec<u8> {
    [&[0u8, 0, 0, 2, 0, 0, 0, 8][..], &[12, 9, 5, 9, 7, 6], &[0, 0], &[24, 18, 10, 18, 14, 12], &[0, 0]].concat()
}

#[test]
fn hdmx_rows_keep_their_glyphs_widths_padded_to_four() {
    let out = subset_hdmx(&hdmx(), 4, &[0, 2]).expect("a subset hdmx");
    assert_eq!(out, [&[0u8, 0, 0, 2, 0, 0, 0, 4][..], &[12, 7, 5, 7], &[24, 14, 10, 14]].concat());
}

// A record holds widths for the font's glyphs only: a glyph past them would read the next record.
#[test]
fn a_glyph_past_the_font_refuses_hdmx_and_ltsh() {
    assert!(subset_hdmx(&hdmx(), 4, &[0, 4]).is_none());
    let ltsh = [0u8, 0, 0, 3, 1, 5, 9];
    assert_eq!(subset_ltsh(&ltsh, &[0, 2]), Some(vec![0, 0, 0, 2, 1, 9]));
    assert!(subset_ltsh(&ltsh, &[0, 3]).is_none());
}
