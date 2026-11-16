use super::*;
use alloc::vec::Vec;

fn bytes<const N: usize>(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()[4 - N..].to_vec()).collect()
}

// GDEF 1.4's 32-bit tables answer for glyph 65,537: class 3, mark attachment class 5 and mark glyph set 0. Its
// 16-bit ones, all 1.3 reads, class glyph 1 alone.
#[test]
fn a_gdef_of_1_4_answers_from_its_32_bit_tables() {
    let head = [bytes::<4>(&[0x0001_0004]), bytes::<2>(&[38, 0, 0, 0, 0]), bytes::<4>(&[0, 46, 0, 0, 57, 70])];
    let classes = [bytes::<2>(&[1, 1, 1, 1, 3]), bytes::<3>(&[0x1_0001, 1, 3])].concat();
    let attachment = [bytes::<2>(&[4]), bytes::<3>(&[1, 0x1_0001, 0x1_0001]), bytes::<2>(&[5])].concat();
    let sets = [bytes::<2>(&[1, 1]), bytes::<4>(&[8]), bytes::<2>(&[3]), bytes::<3>(&[1, 0x1_0001])].concat();
    let mut gdef = [head.concat(), classes, attachment, sets].concat();
    for (minor, wide, narrow) in [(4, (3, 5, true), 0), (3, (0, 0, false), 1)] {
        gdef[3] = minor;
        let g = Gdef::new(&gdef).expect("a GDEF");
        let class = |c: Option<ClassDef>, glyph| c.map_or(0, |c| c.class_of(glyph));
        let read = (class(g.glyph_classes, 0x1_0001), class(g.mark_attach_classes, 0x1_0001));
        assert_eq!((read.0, read.1, g.is_mark_glyph(0x1_0001, 0)), wide, "GDEF 1.{minor}");
        assert_eq!(class(g.glyph_classes, 1), narrow, "GDEF 1.{minor}");
    }
}
