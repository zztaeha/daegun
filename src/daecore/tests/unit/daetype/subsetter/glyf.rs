use super::*;

// A GLYF composite's 24-bit ids take their new glyphs in 24 bits still, one now under 65,536 too, so the glyph keeps
// its length; in glyf the flag is left alone and ids are 16 bits.
#[test]
fn a_24_bit_component_keeps_its_width() {
    let header = [0xFFu8, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut gid_map = alloc::vec![0u32; 0x1_000A];
    (gid_map[0x1_0005], gid_map[0x1_0009], gid_map[5]) = (3, 0x1_0001, 2);
    let patched = |glyph: &[u8], extended| {
        let mut out = glyph.to_vec();
        patch_compound_gids(glyph, &mut out, extended, &gid_map);
        out
    };
    let glyph = [&header[..], &[0x20, 0x22, 1, 0, 5, 1, 2], &[0x21, 0x02, 1, 0, 9, 3, 4], &[0xAA, 0xBB]].concat();
    let want = [&header[..], &[0x20, 0x22, 0, 0, 3, 1, 2], &[0x21, 0x02, 1, 0, 1, 3, 4], &[0xAA, 0xBB]].concat();
    assert_eq!(patched(&glyph, true), want);
    let lower = [&header[..], &[0x20, 0x02, 0, 5, 1, 2]].concat();
    assert_eq!(patched(&lower, false), [&header[..], &[0x20, 0x02, 0, 2, 1, 2]].concat());
}

// Up to 65,535 glyphs loca is read for every one counted, past the table's end too; beyond, only as far as it
// holds entries.
#[test]
fn loca_past_16_bits_is_read_only_as_far_as_it_holds() {
    assert_eq!(parse_loca(&[0; 8], 1, 0xFFFF).len(), 0x1_0000);
    assert_eq!(parse_loca(&[0; 8], 1, 0x10_0000).len(), 0x1_0000);
    let long = alloc::vec![0; 4 * 0x1_0003];
    assert_eq!(parse_loca(&long, 1, 0x10_0000).len(), 0x1_0003);
    assert_eq!(Loca::new(&long, 1, 0x1_0001).at(0x1_0001), Some(0));
    assert_eq!(parse_loca(&long[..2 * 0x1_0003], 0, 0x10_0000).len(), 0x1_0003);
}

// Short and long loca cut short, read entry by entry, give what the whole decode gives: 0 past the bytes.
#[test]
fn loca_read_per_glyph_is_the_whole_decode() {
    let short = [0u8, 0, 0, 3, 0, 9, 0];
    let long = [0u8, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0, 18, 0, 0];
    for (bytes, format) in [(&short[..], 0), (&long[..], 1)] {
        let whole = parse_loca(bytes, format, 5);
        let loca = Loca::new(bytes, format, 5);
        assert_eq!((0..whole.len()).map(|i| loca.at(i)).collect::<Option<Vec<_>>>(), Some(whole));
        assert_eq!(loca.at(6), None);
    }
    let (glyf, loca) = ([7u8; 20], Loca::new(&short, 0, 5));
    assert_eq!((loca.glyph(&glyf, 1), loca.glyph(&glyf, 2)), (Some(&glyf[6..18]), None));
}

// The even glyphs 2 to 2,000 share one composite naming them all, and the odd glyphs read backward and hold nothing:
// each reads the composite again, which past the subset budget refuses the closure, as copying them would the subset.
#[test]
fn glyphs_sharing_one_composite_read_it_within_the_budget() {
    const K: usize = 1_000;
    let mut composite = [0xFFu8, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0].to_vec();
    for i in 1..=K {
        let more: u16 = if i < K { 0x0020 } else { 0 };
        composite.extend([more.to_be_bytes(), (2 * i as u16).to_be_bytes(), [0, 0]].concat());
    }
    let glyphs = 2 * K + 2;
    let loca: Vec<u8> = (0..=glyphs).flat_map(|i| ((i % 2 * composite.len()) as u32).to_be_bytes()).collect();
    let (loca, mut set) = (Loca::new(&loca, 1, glyphs), GlyphSet::new());
    let mut left = crate::daecore::daetype::subsetter::subset_budget(composite.len());
    let read = active_gids_into(&[2], &composite, false, loca, glyphs, &mut set, &mut left);
    assert_eq!(read.map_err(|e| e.kind()), Err(crate::daecore::ErrorKind::LimitExceeded));
    let mut left = usize::MAX;
    assert!(active_gids_into(&[2], &composite, false, loca, glyphs, &mut GlyphSet::new(), &mut left).is_ok());
    assert_eq!(usize::MAX - left, K * composite.len(), "each glyph sharing the composite reads it once");
}
