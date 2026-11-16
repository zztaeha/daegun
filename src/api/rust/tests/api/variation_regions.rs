use daegun::{Font, GlyphId};

use super::aat_subset::{base_bytes, carets_with, cut, font, one_face_collection, u16s, u32s};
use daegun::Tag;
use daegun::shape::ShapeOptions;
use crate::located;

// Axes AAAA and BBBB from -1 to 1.
fn fvar() -> Vec<u8> {
    let fixed = |v: i32| u32s(&[(v << 16) as u32]);
    let mut fvar = u16s(&[1, 0, 16, 2, 2, 20, 0, 12]);
    for tag in [b"AAAA", b"BBBB"] {
        fvar.extend(tag);
        fvar.extend([fixed(-1), fixed(0), fixed(1)].concat());
        fvar.extend(u16s(&[0, 256]));
    }
    fvar
}

// A store over `regions`, each a (start, peak, end) on AAAA and a full rise to 1 on BBBB, and one row of deltas, one
// a region, for each item in turn.
fn store<const N: usize>(regions: [[i16; 3]; N], rows: &[[i8; N]]) -> Vec<u8> {
    let mut t = u16s(&[1]);
    t.extend(u32s(&[12]));
    t.extend(u16s(&[1]));
    t.extend(u32s(&[(16 + 12 * N) as u32]));
    t.extend(u16s(&[2, N as u16]));
    for region in regions {
        t.extend(region.iter().flat_map(|v| v.to_be_bytes()));
        t.extend(u16s(&[0, 0x4000, 0x4000]));
    }
    t.extend(u16s(&[rows.len() as u16, 0, N as u16]));
    t.extend(u16s(&(0..N as u16).collect::<Vec<_>>()));
    t.extend(rows.iter().flatten().map(|&v| v as u8));
    t
}

// An HVAR whose store's rows go to the glyphs in turn, and none past them.
fn varied<const N: usize>(regions: [[i16; 3]; N], rows: &[[i8; N]]) -> daegun::Font {
    let mut hvar = u16s(&[1, 0]);
    hvar.extend(u32s(&[20, 0, 0, 0]));
    hvar.extend(store(regions, rows));
    carets_with(&[("fvar", fvar()), ("HVAR", hvar)])
}

// One region on AAAA from 0 through 16383/16384 to 1, adding 1: at 8191/16384 it adds 8191/16383, a little under
// a half.
const UNDER_A_HALF: ([[i16; 3]; 1], [[i8; 1]; 1]) = ([[0, 0x3FFF, 0x4000]], [[1]]);

// A gvar moving i's outline 100 across, its phantom points staying, under one intermediate tuple: from -1 to 1
// through 0.5 on AAAA, which crosses zero, and up to 1 on BBBB.
fn moved() -> daegun::Font {
    let mut data = u16s(&[1, 20, 8, 0xE000, 0x2000, 0x4000, 0xC000, 0, 0x4000, 0x4000]);
    data.extend([0, 3, 100, 100, 100, 100, 0x83, 0x87]);
    let mut gvar = u16s(&[1, 0, 2, 0]);
    gvar.extend(u32s(&[36]));
    gvar.extend(u16s(&[7, 0]));
    gvar.extend(u32s(&[36]));
    gvar.extend(u16s(&[0, 0, 0, 14, 14, 14, 14, 14]));
    gvar.extend(data);
    carets_with(&[("fvar", fvar()), ("gvar", gvar)])
}

// A region that runs backward or across zero on an axis is ignored there, with AAAA at 0 as anywhere: BBBB at 1 alone
// decides, adding the whole 100.
#[test]
fn a_region_ignores_an_axis_it_is_malformed_on() {
    for first in [[-0x4000, 0x2000, 0x4000], [0x3000, 0x2000, 0x4000]] {
        let f = varied([first], &[[100]; 7]);
        let advance = |a: f64, b: f64| {
            f.shape("f", &located(&f, &[("AAAA", a), ("BBBB", b)]), &ShapeOptions::default()).advances[0]
        };
        assert_eq!([advance(0.0, 1.0), advance(0.25, 1.0), advance(0.0, 0.0)], [500.0, 500.0, 400.0], "{first:?}");
    }
}

// The tuple's intermediate region crosses zero on AAAA, so that axis is ignored and BBBB at 1 moves i the whole 100,
// from 20 to 120, with AAAA at 0 or 0.25.
#[test]
fn a_tuple_ignores_an_axis_its_region_crosses_zero_on() {
    let f = moved();
    let left = |a: f64, b: f64| f.glyph_bounds(GlyphId(2), &located(&f, &[("AAAA", a), ("BBBB", b)])).map(|b| b.x_min);
    assert_eq!([left(0.0, 1.0), left(0.25, 1.0)], [Some(120.0), Some(120.0)]);
}

// Scalars and sums in 32-bit float, each product added in one rounding: f's sum lands just under 1.5 and i's just past
// -5.5, where products rounded first give -5.5.
#[test]
fn a_rows_deltas_are_summed_in_single_precision() {
    let f = varied([[0, 0x3000, 0x4000], [0, 0x2800, 0x4000]], &[[0, 0], [-86, 125], [54, -85]]);
    let advance = |text: &str, a: f64| {
        f.shape(text, &located(&f, &[("AAAA", a), ("BBBB", 1.0)]), &ShapeOptions::default()).advances[0]
    };
    assert_eq!([advance("f", 288.0 / 16384.0), advance("i", 1408.0 / 16384.0)], [401.0, 394.0]);
}

// A field of 1100 with that delta, summed in 16.16 to just under 1100.5 and rounded once to 1100, where metrics and
// vertical layout read it.
#[test]
fn a_field_takes_its_16_16_delta_rounded_once() {
    let (regions, rows) = UNDER_A_HALF;
    let at = [("AAAA", 8191.0 / 16384.0), ("BBBB", 1.0)];

    let mut os2 = font("test-fixtures/carets.ttf").table(Tag::new(b"OS/2")).expect("an OS/2").to_vec();
    os2[88..90].copy_from_slice(&1100u16.to_be_bytes());
    let mut mvar = u16s(&[1, 0, 0, 8, 1, 20]);
    mvar.extend(b"cpht");
    mvar.extend(u16s(&[0, 0]));
    mvar.extend(store(regions, &rows));
    let f = carets_with(&[("fvar", fvar()), ("OS/2", os2), ("MVAR", mvar)]);

    let mut vvar = u16s(&[1, 0]);
    vvar.extend(u32s(&[24, 0, 0, 0]));
    let ivs = store(regions, &rows);
    vvar.extend(u32s(&[(24 + ivs.len()) as u32]));
    vvar.extend(ivs);
    vvar.extend([0, 0, 0, 1, 0]);
    let g = carets_with(&[("fvar", fvar()), ("VORG", u16s(&[1, 0, 1100, 0])), ("VVAR", vvar)]);

    assert_eq!([f.cap_height(&located(&f, &at)), g.vertical_origin(GlyphId(1), &located(&g, &at))], [1100.0, 1100.0]);
}

// A store of one region, rising on AAAA to 1, and a word delta for each item.
fn rising(items: &[i16]) -> Vec<u8> {
    let mut t = [u16s(&[1]), u32s(&[12]), u16s(&[1]), u32s(&[28]), u16s(&[2, 1, 0, 0x4000, 0x4000, 0, 0, 0])].concat();
    t.extend(u16s(&[items.len() as u16, 1, 1, 0]));
    t.extend(items.iter().flat_map(|v| v.to_be_bytes()));
    t
}

// A gvar moving i's four points 100 across at AAAA 1, and its first phantom point `pp1`.
fn moving(pp1: u8) -> Vec<u8> {
    let mut data = u16s(&[1, 12, 9, 0xA000, 0x4000, 0]);
    data.extend([0, 4, 100, 100, 100, 100, pp1, 0x82, 0x87, 0]);
    [u16s(&[1, 0, 2, 0]), u32s(&[36]), u16s(&[7, 0]), u32s(&[36]), u16s(&[0, 0, 0, 11, 11, 11, 11, 11]), data].concat()
}

// Nothing is read past a table's length, so a store, or an index map, that its table cuts short is absent and i draws
// as without it, at 70: with the whole table 120 at AAAA 1 with HVAR, 83 at 0.5 where avar 2 moves AAAA to 0.625.
#[test]
fn a_store_its_table_cuts_short_is_absent() {
    let left = |bytes: &[u8], a: f64| {
        let read = [Font::from_bytes(bytes), Font::from_collection(&one_face_collection(bytes), 0)];
        read.map(|f| {
            let f = f.expect("it parses");
            f.glyph_bounds(GlyphId(2), &located(&f, &[("AAAA", a)])).map(|b| b.x_min)
        })
    };
    let hvar = [u16s(&[1, 0]), u32s(&[20, 0, 0, 0]), rising(&[0; 7])].concat();
    let identity = u16s(&[3, 0xC000, 0xC000, 0, 0, 0x4000, 0x4000]);
    let avar = [u16s(&[2, 0, 0, 2]), identity.clone(), identity, u32s(&[0, 44]), rising(&[4096, 0])].concat();
    let cases = [("HVAR", hvar, 50, 1.0, 22, 120.0, 70.0), ("avar", avar, 0, 0.5, 46, 83.0, 70.0)];
    for (tag, table, pp1, a, len, with, without) in cases {
        let varied = [("fvar", fvar()), ("gvar", moving(pp1))];
        let whole = base_bytes("test-fixtures/carets.ttf", &[&varied[..], &[(tag, table)]].concat());
        let tag_bytes = tag.as_bytes().try_into().expect("a tag");
        assert_eq!(left(&whole, a), [Some(with); 2], "{tag}");
        assert_eq!(left(&cut(whole, tag_bytes, len), a), [Some(without); 2], "{tag} cut short");
        assert_eq!(left(&base_bytes("test-fixtures/carets.ttf", &varied), a), [Some(without); 2], "without {tag}");
    }
    let mapped = [&u16s(&[1, 0])[..], &u32s(&[20, 70, 0, 0]), &rising(&[0; 7]), &[0, 0, 0, 7], &[0; 7]].concat();
    let whole = base_bytes("test-fixtures/carets.ttf", &[("fvar", fvar()), ("gvar", moving(50)), ("HVAR", mapped)]);
    for len in [8, 6] {
        assert_eq!(left(&cut(whole.clone(), b"HVAR", len), 1.0), [Some(70.0); 2], "HVAR's map at {len} bytes");
    }
}
