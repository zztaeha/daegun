use super::aat_subset::{carets_with, shapes_alike, u16s, u32s, F, F_F_I, F_F_L, F_I, I, L};
use daegun::{Location, Tag};
use daegun::shape::ShapeOptions;

// A cmap of (platform, encoding, subtable) records, in the order given.
pub(super) fn cmap(records: &[(u16, u16, &[u8])]) -> Vec<u8> {
    let mut head = u16s(&[0, records.len() as u16]);
    let mut body = Vec::new();
    for &(platform, encoding, subtable) in records {
        head.extend(u16s(&[platform, encoding]));
        head.extend(u32s(&[(4 + 8 * records.len() + body.len()) as u32]));
        body.extend(subtable);
    }
    [head, body].concat()
}

// A format 4 mapping each code to its glyph, a segment each, in code order.
pub(super) fn format4(map: &[(u16, u16)]) -> Vec<u8> {
    let mut segments = map.to_vec();
    segments.push((0xFFFF, 0));
    let n = segments.len() as u16;
    let column = |f: fn(&(u16, u16)) -> u16| u16s(&segments.iter().map(f).collect::<Vec<_>>());
    [
        u16s(&[4, 16 + 8 * n, 0, 2 * n, 0, 0, 0]),
        column(|s| s.0),
        u16s(&[0]),
        column(|s| s.0),
        column(|s| if s.0 == 0xFFFF { 1 } else { s.1.wrapping_sub(s.0) }),
        u16s(&vec![0; segments.len()]),
    ]
    .concat()
}

// A format 0 mapping each byte to its glyph.
fn format0(map: &[(u8, u16)]) -> Vec<u8> {
    let mut glyphs = [0u8; 256];
    map.iter().for_each(|&(byte, g)| glyphs[usize::from(byte)] = g as u8);
    [u16s(&[0, 262, 0]), glyphs.to_vec()].concat()
}

// A format 14 of one selector, whose default ranges hold `default` and whose glyphs map `own`.
pub(super) fn format14(selector: u32, default: u32, own: (u32, u16)) -> Vec<u8> {
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let ranges = [u32s(&[1]), u24(default), vec![0]].concat();
    let glyphs = [u32s(&[1]), u24(own.0), u16s(&[own.1])].concat();
    [u16s(&[14]), u32s(&[38, 1]), u24(selector), u32s(&[21, 29]), ranges, glyphs].concat()
}

// carets.ttf's OS/2 as version 0, the version a font page is read from, naming `page`.
fn os2_page(page: u8) -> Vec<u8> {
    let carets = std::fs::read(format!("{}/test-fixtures/carets.ttf", super::FONTS)).expect("the fixture font");
    let mut os2 = daegun::Font::from_vec(carets, 0)
        .expect("it parses").table(Tag::new(b"OS/2")).expect("OS/2").to_vec();
    os2[..2].copy_from_slice(&[0, 0]);
    os2[62] = page;
    os2
}

fn glyphs(f: &daegun::Font, text: &str) -> Vec<u32> {
    f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs.iter().map(|g| g.0).collect()
}

// A symbol subtable is read before any Unicode one, finding a code it lacks at U+F000 more: A and B
// are the symbol subtable's, and C, which only the Unicode subtable maps, maps nothing.
#[test]
fn a_symbol_subtable_answers_before_a_unicode_one() {
    let symbol = format4(&[(0xF041, F_I), (0xF042, F_F_L)]);
    let unicode = format4(&[(0x41, F), (0x43, I)]);
    let f = carets_with(&[("cmap", cmap(&[(3, 0, &symbol), (3, 1, &unicode)]))]);
    assert_eq!(glyphs(&f, "ABC"), [F_I, F_F_L, 0].map(u32::from));
    shapes_alike(&f, "ABC");
}

// AppleGothic's É: the Unicode subtable answers alone, so ∞, which only the Mac subtable maps, maps nothing.
#[test]
fn a_unicode_subtable_answers_alone_where_a_mac_one_maps_more() {
    let mac = format0(&[(0x41, I), (0xB0, F_F_I)]);
    let unicode = format4(&[(0x41, F)]);
    let f = carets_with(&[("cmap", cmap(&[(1, 0, &mac), (3, 1, &unicode)]))]);
    assert_eq!(glyphs(&f, "A∞"), [F, 0].map(u32::from));
    shapes_alike(&f, "A∞");
}

// The Mac Roman table maps the characters Mac Roman encodes, each through its byte: the euro sign finds 0xDB, and
// U+120AC, which shares its low 16 bits but isn't in Mac Roman, maps nothing.
#[test]
fn mac_roman_maps_only_its_own_characters() {
    let f = carets_with(&[("cmap", cmap(&[(1, 0, &format0(&[(0x41, F), (0xDB, F_F_I)]))]))]);
    assert_eq!(glyphs(&f, "A€"), [F, F_F_I].map(u32::from));
    assert_eq!(glyphs(&f, "\u{120AC}"), [0]);
    shapes_alike(&f, "A€\u{120AC}");
}

// Any other Mac subtable maps ASCII, and nothing past it.
#[test]
fn any_other_mac_subtable_maps_ascii() {
    let f = carets_with(&[("cmap", cmap(&[(1, 1, &format0(&[(0x41, F), (0xB0, I)]))]))]);
    assert_eq!(glyphs(&f, "A∞"), [F, 0].map(u32::from));
    shapes_alike(&f, "A∞");
}

// A format 14 whose default ranges run past the table fails its check and is absent; the Unicode subtable beside it
// still maps f, i and l.
#[test]
fn a_subtable_that_fails_its_check_leaves_the_rest_read() {
    let unicode = format4(&[(0x66, F), (0x69, I), (0x6C, L)]);
    let mut uvs = format14(0xFE00, 0x66, (0x66, F_I));
    uvs[13..17].copy_from_slice(&u32s(&[100]));
    let f = carets_with(&[("cmap", cmap(&[(0, 5, &uvs), (3, 1, &unicode)]))]);
    assert_eq!(glyphs(&f, "fil"), [F, I, L].map(u32::from));
    shapes_alike(&f, "fil");
}

// A symbol subtable answers a byte code at U+F000 plus the byte, so A finds U+F041. The font page a version 0 OS/2
// once held is in bits the OS/2 chapter has applications ignore, so the Arabic comma maps nothing on either page.
#[test]
fn a_symbol_font_reads_its_byte_codes_at_f000() {
    let symbol = format4(&[(0xF041, F_F_L), (0xF12C, F_F_I), (0xF22C, F_I)]);
    for page in [0xB2, 0xB3] {
        let f = carets_with(&[("cmap", cmap(&[(3, 0, &symbol)])), ("OS/2", os2_page(page))]);
        assert_eq!(glyphs(&f, "\u{60C}A"), [0, u32::from(F_F_L)], "page {page:X}");
        shapes_alike(&f, "\u{60C}A");
    }
}

// A sequence listed both as a default one and with its own glyph takes the base's glyph, as the default ranges are
// searched first.
#[test]
fn a_sequence_listed_both_ways_takes_the_base_glyph() {
    let unicode = format4(&[(0x66, F)]);
    let f = carets_with(&[("cmap", cmap(&[(0, 5, &format14(0xFE00, 0x66, (0x66, F_I))), (3, 1, &unicode)]))]);
    assert_eq!(glyphs(&f, "f\u{FE00}"), [F].map(u32::from));
    shapes_alike(&f, "f\u{FE00}");
}

// Four selectors each taking the default glyph for 60,000 ideographs, their ranges one table they share: 240,000
// sequences in 2 KB, every one kept by a subset of all the glyphs, as ranges again.
#[test]
fn a_subset_keeps_every_sequence_a_table_names() {
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let (first, count) = (0x4E00, 60_000u32);
    let ranges: Vec<u8> = (first..first + count).step_by(256)
        .flat_map(|start| [u24(start), vec![(first + count - 1 - start).min(255) as u8]].concat())
        .collect();
    let ranges = [u32s(&[ranges.len() as u32 / 4]), ranges].concat();
    let records: Vec<u8> = (0xFE00..0xFE04).flat_map(|vs| [u24(vs), u32s(&[10 + 4 * 11, 0])].concat()).collect();
    let uvs = [u16s(&[14]), u32s(&[(54 + ranges.len()) as u32, 4]), records, ranges].concat();
    let ideographs = [u16s(&[13, 0]), u32s(&[28, 0, 1, first, first + count - 1, u32::from(F)])].concat();
    let f = carets_with(&[("cmap", cmap(&[(0, 5, &uvs), (3, 10, &ideographs)]))]);
    let all: Vec<daegun::GlyphId> = (0..f.glyph_count()).map(daegun::GlyphId).collect();
    let sub = daegun::Font::from_vec(f.subset(&all, &Location::default()).expect("a subset").font, 0).expect("parses");
    let last = char::from_u32(first + count - 1).expect("a character");
    for (c, vs) in [('\u{4E00}', '\u{FE00}'), ('\u{4E00}', '\u{FE03}'), (last, '\u{FE02}')] {
        assert_eq!(sub.variation_glyph_id(c, vs), Some(daegun::GlyphId(F.into())), "{c:?} {vs:?}");
    }
}
