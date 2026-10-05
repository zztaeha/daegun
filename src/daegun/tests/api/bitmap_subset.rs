use std::collections::BTreeMap;

use daegun::Font;

use super::FONTS;

// carets.ttf without its layout tables, seven simple glyphs, so only the bitmap tables pull glyphs in.
fn carets_with(bitmaps: &[(&str, Vec<u8>)]) -> Font {
    let path = format!("{FONTS}/test-fixtures/carets.ttf");
    let source = Font::from_vec(std::fs::read(&path).expect("the fixture font")).expect("it parses");
    let mut tables: BTreeMap<String, Vec<u8>> = source
        .table_tags()
        .into_iter()
        .filter(|t| !matches!(*t, "GSUB" | "GDEF"))
        .map(|t| (t.to_string(), source.table(t).expect("listed").to_vec()))
        .collect();
    tables.extend(bitmaps.iter().map(|(t, b)| (t.to_string(), b.clone())));
    Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses")
}

// Subset to `requested`: the closure must be `closure`, and each requested glyph must draw at 16 ppem
// what it drew in the source.
fn draws_alike(font: &Font, requested: &[u16], closure: &[u16]) {
    assert_eq!(font.glyph_closure(requested, &[]).expect("a closure"), closure);
    let out = font.subset(requested, &[]).expect("a subset");
    let sub = Font::from_vec(out.ttf.clone()).expect("the subset parses");
    assert_eq!(usize::from(sub.num_glyphs()), closure.len(), "the subset kept another closure");
    for &g in requested {
        let new = out.new_gid(g).expect("a requested glyph is kept");
        let want = font.glyph_bitmap(g, 16);
        assert!(want.is_some(), "glyph {g} draws nothing in the source");
        assert_eq!(sub.glyph_bitmap(new, 16), want, "glyph {g}, now {new}");
    }
}

// A PNG as far as its IHDR, which is all daegun reads of one.
fn png(height: u32) -> Vec<u8> {
    [&b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"[..], &7u32.to_be_bytes(), &height.to_be_bytes(), &[0; 9]].concat()
}

// One 16 ppem strike over carets' seven glyphs: per glyph, its graphic type and data, or none.
fn sbix(glyphs: [Option<(&[u8; 4], Vec<u8>)>; 7]) -> Vec<u8> {
    let head = 4 + 4 * (glyphs.len() + 1);
    let mut out = [&[0u8, 1, 0, 1][..], &1u32.to_be_bytes(), &12u32.to_be_bytes(), &[0, 16, 0, 72]].concat();
    let mut data = Vec::new();
    for g in &glyphs {
        out.extend(((head + data.len()) as u32).to_be_bytes());
        if let Some((kind, body)) = g {
            data.extend([&[0u8, 2, 0, 3][..], &kind[..], body].concat());
        }
    }
    out.extend(((head + data.len()) as u32).to_be_bytes());
    [out, data].concat()
}

// Glyph 4 is a 'dupe' of 3 and glyph 5 a 'flip' of 2. Their targets join the closure, and dropping
// glyph 1 shifts every id, so a target left unrenumbered names the wrong image.
#[test]
fn sbix_dupes_and_flips_keep_and_renumber_their_targets() {
    let font = carets_with(&[(
        "sbix",
        sbix([
            None,
            Some((b"png ", png(30))),
            Some((b"png ", png(10))),
            Some((b"png ", png(20))),
            Some((b"dupe", 3u16.to_be_bytes().to_vec())),
            Some((b"flip", 2u16.to_be_bytes().to_vec())),
            None,
        ]),
    )]);
    draws_alike(&font, &[2, 3, 4, 5], &[0, 2, 3, 4, 5]);
    draws_alike(&font, &[4, 5], &[0, 2, 3, 4, 5]);
}

// One 16 ppem, 1-bit strike, each glyph its own format 1 index subtable: (glyph, image format, image).
fn eblc_ebdt(glyphs: &[(u16, u16, Vec<u8>)]) -> [(&'static str, Vec<u8>); 2] {
    let mut loc = [0x0002_0000u32, 1, 56, 0, glyphs.len() as u32].map(u32::to_be_bytes).concat();
    loc.extend([0; 28]);
    loc.extend([0, 0, 0xFF, 0xFF, 16, 16, 1, 1]);
    let (mut data, mut subtables) = (0x0002_0000u32.to_be_bytes().to_vec(), Vec::new());
    for (gid, format, image) in glyphs {
        loc.extend([gid.to_be_bytes(), gid.to_be_bytes()].concat());
        loc.extend(((glyphs.len() * 8 + subtables.len()) as u32).to_be_bytes());
        subtables.extend([1u16, *format].map(u16::to_be_bytes).concat());
        subtables.extend([data.len() as u32, 0, image.len() as u32].map(u32::to_be_bytes).concat());
        data.extend(image);
    }
    loc.extend(subtables);
    [("EBLC", loc), ("EBDT", data)]
}

// Glyph 5 is a composite (EBDT image format 8) of glyphs 3 and 1. Both join the closure, and the
// composite's component ids are renumbered with them.
#[test]
fn ebdt_composites_keep_and_renumber_their_components() {
    let small = |w: u8, x: i8, y: i8| vec![2, w, x as u8, y as u8, w];
    let font = carets_with(&eblc_ebdt(&[
        (1, 1, [small(3, 1, 5), vec![0b1010_0000, 0b0100_0000]].concat()),
        (3, 1, [small(3, 0, 2), vec![0b1110_0000, 0b1010_0000]].concat()),
        (5, 8, [small(4, 0, 5), vec![0, 0, 2, 0, 3, 0, 0, 0, 1, 1, 0]].concat()),
    ]));
    draws_alike(&font, &[5], &[0, 1, 3, 5]);
    draws_alike(&font, &[1, 3, 5], &[0, 1, 3, 5]);
}
