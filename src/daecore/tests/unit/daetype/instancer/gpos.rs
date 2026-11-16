use super::*;
use super::super::super::decoder::{read_i16_be, read_u16_be};

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A SinglePos whose x advance device has no x advance, a VariationIndex of -40 at the peak, in a
// lookup filtering marks by GDEF's one mark glyph set when `filter`: the GPOS and GDEF.
fn tables(filter: bool, gdef_major: u16) -> (Vec<u8>, Vec<u8>) {
    let mut gpos = words(&[1, 0, 10, 12, 14, 0, 0, 1, 4]);
    gpos.extend(if filter { words(&[1, 0x0010, 1, 10, 0]) } else { words(&[1, 0, 1, 8]) });
    gpos.extend(words(&[1, 8, 0x0040, 14, 1, 1, 5, 0, 0, 0x8000]));
    let mut gdef = words(&[gdef_major, 3, 0, 0, 0, 0, 18, 0, 32, 1, 1, 0, 8, 1, 1, 7]);
    gdef.extend(crate::daecore::daetype::colr_v1::testing::store(1, -40));
    (gpos, gdef)
}

// The lookup instanced at the peak.
fn instanced_lookup(filter: bool) -> (Vec<u8>, usize) {
    let (gpos, gdef) = tables(filter, 1);
    let out = apply_gpos_var(&gpos, &gdef, &[1.0]).expect("an instanced GPOS");
    let lookups = usize::from(read_u16_be(&out, 8).expect("a lookup list"));
    let lookup = lookups + usize::from(read_u16_be(&out, lookups + 2).expect("a lookup"));
    (out, lookup)
}

// The missing value is 0, so the instance kerns by -40 as daegun's shaper does, not dropping the delta.
#[test]
fn a_device_without_its_value_still_moves_the_glyph() {
    let (out, lookup) = instanced_lookup(false);
    let sub = lookup + usize::from(read_u16_be(&out, lookup + 6).expect("a subtable"));
    let format = read_u16_be(&out, sub + 4).expect("a value format");
    assert!(format & 0x0004 != 0, "no x advance in format {format:#06x}");
    assert_eq!(read_i16_be(&out, sub + 6), Some(-40));
}

// Rebuilt to hold that value, the lookup keeps filtering by the mark glyph set GDEF still holds.
#[test]
fn a_rebuilt_lookup_keeps_its_mark_filtering_set() {
    let (out, lookup) = instanced_lookup(true);
    assert_eq!(read_u16_be(&out, lookup + 2), Some(0x0010));
    assert_eq!(read_u16_be(&out, lookup + 8), Some(0));
}

// The shaper reads no GDEF of a major version other than 1, its store included, so the instance varies nothing.
#[test]
fn a_later_gdef_major_varies_nothing() {
    let (gpos, gdef) = tables(false, 2);
    assert!(apply_gpos_var(&gpos, &gdef, &[1.0]).is_none());
}
