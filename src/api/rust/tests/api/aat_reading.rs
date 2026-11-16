use super::aat_shaping::kerx;
use super::aat_subset::{carets_with, lookup, morx, shapes_alike, stx_classed, u16s, u32s, F, F_I, I, L};
use daegun::Location;
use daegun::shape::ShapeOptions;

fn glyphs(f: &daegun::Font, text: &str) -> Vec<u32> {
    f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs.iter().map(|g| g.0).collect()
}

// A format 10 lookup of `unit`-byte values for the glyphs from `first` on.
fn format10(unit: u16, first: u16, values: &[u32]) -> Vec<u8> {
    let mut out = u16s(&[10, unit, first, values.len() as u16]);
    values.iter().for_each(|v| out.extend(&v.to_be_bytes()[4 - usize::from(unit)..]));
    out
}

// A contextual subtable turning i into l in class 5, classed by format 10 (f 4, i 5, l 6): one-byte units read, so i
// turns into l; units of 3 bytes, a size Apple's manual doesn't allow, read nothing, every glyph out of bounds.
#[test]
fn a_format_10_class_table_reads_the_unit_sizes_the_manual_allows() {
    let row: &[u16] = &[0, 0, 0, 0, 0, 1, 0];
    let entries: [&[u16]; 2] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0]];
    let substitutions = [u32s(&[4]), lookup(&[(I, L)])].concat();
    for (unit, want) in [(1, [F, L, L]), (3, [F, I, L])] {
        let classes = format10(unit, F, &[4, 5, 6]);
        let contextual = stx_classed(7, classes, &[row, row], &entries, std::slice::from_ref(&substitutions));
        let f = carets_with(&[("morx", morx(2, &[(1, contextual)]))]);
        assert_eq!(glyphs(&f, "fil"), want.map(u32::from), "{unit}-byte units");
        shapes_alike(&f, "fil");
    }
}

// A format 10 lookup in a non-contextual subtable substitutes by its two-byte units: f turns into f_i.
#[test]
fn a_format_10_non_contextual_lookup_substitutes_by_its_units() {
    let f = carets_with(&[("morx", morx(2, &[(4, format10(2, F, &[u32::from(F_I)]))]))]);

    assert_eq!(glyphs(&f, "fil"), [F_I, I, L].map(u32::from));
    shapes_alike(&f, "fil");
}

// kerx format 2 classes f on the left and i on the right through format 10 lookups, 4 and 2, which sum to byte 6,
// the array's fourth value, -10; l, which no lookup lists, isn't kerned.
#[test]
fn kerx_format_2_reads_format_10_classes() {
    let mut body = u32s(&[4, 28, 38, 48]);
    body.extend(format10(2, F, &[4]));
    body.extend(format10(2, I, &[2]));
    body.extend(u16s(&[-50i16, 0, 0, -10].map(|v| v as u16)));
    let f = carets_with(&[("kerx", kerx(2, body))]);

    for (text, advances) in [("fi", &[390.0, 400.0][..]), ("fl", &[400.0, 400.0])] {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, advances, "{text}");
        shapes_alike(&f, text);
    }
}
