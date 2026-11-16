use super::*;

// A version 1 COLR header of 34 bytes whose lists all lie past the table, and a CPAL header of `version`.
fn tables(cpal_version: u16) -> (alloc::vec::Vec<u8>, [u8; 12]) {
    let mut colr = [[0, 1].as_slice(), &[0xFF; 32]].concat();
    colr[2..4].fill(0);
    let mut cpal = [0; 12];
    cpal[..2].copy_from_slice(&cpal_version.to_be_bytes());
    (colr, cpal)
}

// Drawing takes COLR by its header beside a CPAL, the parts inside left to their readers.
#[test]
fn drawing_takes_a_colr_by_its_header_beside_a_cpal() {
    let (colr, cpal) = tables(0);
    assert!(colr_drawn(&colr, Some(&cpal)), "lists past the table");
    assert!(!colr_drawn(&colr, None), "no CPAL");
    assert!(!colr_drawn(&colr, Some(&tables(2).1)), "a CPAL of version 2");
    assert!(!colr_drawn(&colr[..33], Some(&cpal)), "a version 1 header cut short");
}
