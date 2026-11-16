use super::*;

// Version 2's count and glyphs are 24-bit and its records 5 bytes; any minor reads; another major is no table, and
// records whose count leaves the table are absent, every glyph taking the default.
#[test]
fn version_2_names_glyphs_in_24_bits() {
    let v2 = [0, 2, 0, 7, 0, 123, 0, 0, 2, 0, 0, 5, 0, 9, 1, 0, 1, 1, 65];
    let t = Vorg::read(&v2).expect("version 2");
    assert_eq!(t.records().collect::<alloc::vec::Vec<_>>(), [(5, 9), (0x1_0001, 321)]);
    assert_eq!([5, 0x1_0001, 1].map(|g| t.origin(g)), [9, 321, 123]);
    let v1 = [0, 1, 0, 0, 0, 123, 0, 1, 0, 1, 1, 65];
    assert_eq!(Vorg::read(&v1).map(|t| [t.origin(1), t.origin(2)]), Some([321, 123]));
    assert!(Vorg::read(&[[0, 3].as_slice(), &v2[2..]].concat()).is_none());
    assert_eq!(Vorg::read(&v2[..18]).map(|t| [t.origin(5), t.origin(0x1_0001)]), Some([123, 123]));
}
