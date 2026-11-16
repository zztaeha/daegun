use super::*;
use super::super::super::decoder::{extract_ttf_tables, read_font_family_name};

// `ttf` with one more directory entry, after the others, naming `length` bytes at `offset`.
fn with_entry(ttf: &[u8], tag: &[u8; 4], offset: u32, length: u32) -> Vec<u8> {
    let n = usize::from(read_u16_be(ttf, 4).expect("a table count"));
    let mut out = ttf[..12].to_vec();
    write_u16_be(&mut out, 4, n as u16 + 1);
    for entry in ttf[12..12 + 16 * n].chunks(16) {
        let at = out.len();
        out.extend(entry);
        write_u32_be(&mut out, at + 8, read_u32_be(entry, 8).expect("an offset") + 16);
    }
    out.extend(tag);
    out.extend([0; 4]);
    out.extend(offset.to_be_bytes());
    out.extend(length.to_be_bytes());
    out.extend(&ttf[12 + 16 * n..]);
    out
}

// A second, empty name entry: the font reads the first, and so does a subset of its bytes, rather
// than taking the empty one and naming itself DaegunSubset.
#[test]
fn a_subset_of_raw_bytes_reads_the_directory_as_the_font_does() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/hinted.ttf");
    let ttf = with_entry(&std::fs::read(path).expect("a fixture"), b"name", 0, 0);
    let family = |bytes: &[u8]| read_font_family_name(&extract_ttf_tables(bytes).expect("a font"));
    let subset = subset_ttf(&ttf, &[0, 1]).expect("a subset");
    assert!(family(&ttf).is_some());
    assert_eq!(family(&subset.font), family(&ttf));
}

// A glyph VARC draws takes its drawing's room, not glyf's: varc-hostile's glyph 34, whose components name axes an
// instance pins, drawn with 20,000 points, past twice the font's 3 KB glyf plus 64 KB, is written whole.
#[test]
fn a_drawn_glyph_is_held_to_its_drawing_not_to_glyf() {
    use crate::daecore::daetype::outline::{OutlinePen, Path};
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/varc/varc-hostile.ttf");
    let map = extract_ttf_tables(&std::fs::read(path).expect("a fixture")).expect("a font");
    let table = |tag: &str| map.get(tag).filter(|_| tag != "fvar").map(|t| t.as_slice());
    let mut drawn = Path::default();
    drawn.move_to(0.0, 0.0);
    (1..20_000).for_each(|i| drawn.line_to((i * 7_919 % 30_000) as f32, (i * 4_729 % 30_000) as f32));
    drawn.close();
    let draw = |gids: &[u32]| Ok(gids.iter().map(|&g| (g, drawn.clone())).collect());
    let entries = || Shared::new(CmapEntries::default());
    let subset = subset_glyf(table, None, &[34], None, &entries, true, &draw).expect("a subset");
    let glyf = extract_ttf_tables(&subset.font).expect("the subset")["glyf"].len();
    assert!(glyf > 2 * map["glyf"].len() + (64 << 10), "{glyf} bytes");
}
