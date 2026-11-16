use std::collections::BTreeMap;

use daegun::{Font, GlyphId, Location};

use super::FONTS;
use daegun::Tag;

// carets.ttf without its layout tables, seven simple glyphs, so only the bitmap tables pull glyphs in.
fn carets_with(bitmaps: &[(&str, Vec<u8>)]) -> Font {
    let path = format!("{FONTS}/test-fixtures/carets.ttf");
    let source = Font::from_vec(std::fs::read(&path).expect("the fixture font"), 0).expect("it parses");
    let mut tables: BTreeMap<Tag, Vec<u8>> = source
        .table_tags()
        .filter(|t| ![Tag::new(b"GSUB"), Tag::new(b"GDEF")].contains(t))
        .map(|t| (t, source.table(t).expect("listed").to_vec()))
        .collect();
    tables.extend(bitmaps.iter().map(|(t, b)| (t.parse().expect("a tag"), b.clone())));
    Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses")
}

// Subset to `requested`: the closure must be `closure`, and each requested glyph must draw at 16 ppem
// what it drew in the source.
fn draws_alike(font: &Font, requested: &[u32], closure: &[u32]) {
    let glyphs: Vec<GlyphId> = requested.iter().copied().map(GlyphId).collect();
    let kept: Vec<u32> =
        font.glyph_closure(&glyphs, &Location::default()).expect("a closure").iter().map(|g| g.0).collect();
    assert_eq!(kept, closure);
    let out = font.subset(&glyphs, &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font.clone(), 0).expect("the subset parses");
    assert_eq!(sub.glyph_count() as usize, closure.len(), "the subset kept another closure");
    for &g in requested {
        let new = out.renumbered(GlyphId(g)).expect("a requested glyph is kept");
        let want = font.bitmap(GlyphId(g), 16);
        assert!(want.is_some(), "glyph {g} draws nothing in the source");
        assert_eq!(sub.bitmap(new, 16), want, "glyph {g}, now {new}");
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

// A strike table and its data of two strikes, at 16 and 32 ppem, each with its own array and subtable for glyph 1's
// one image.
fn two_strikes(tags: [&'static str; 2], version: u32, format: u16, image: &[u8]) -> [(&'static str, Vec<u8>); 2] {
    let mut loc = [version, 2].map(u32::to_be_bytes).concat();
    for (k, ppem) in [16u8, 32].into_iter().enumerate() {
        loc.extend([(8 + 2 * 48 + 24 * k) as u32, 24, 1].map(u32::to_be_bytes).concat());
        loc.extend([0; 28]);
        loc.extend([0, 1, 0, 1, ppem, ppem, if version >> 16 == 3 { 32 } else { 1 }, 1]);
    }
    for _ in 0..2 {
        loc.extend([[0, 1, 0, 1, 0, 0, 0, 8], [0, 1, (format >> 8) as u8, format as u8, 0, 0, 0, 4]].concat());
        loc.extend([0, image.len() as u32].map(u32::to_be_bytes).concat());
    }
    [(tags[0], loc), (tags[1], [&version.to_be_bytes()[..], image].concat())]
}

// Two strikes whose own subtables hold glyph 1's one image keep one copy of it, in CBDT and EBDT alike.
#[test]
fn an_image_two_strikes_hold_is_written_once() {
    let color = [&[9u8, 7, 0, 9, 7][..], &(png(9).len() as u32).to_be_bytes(), &png(9)].concat();
    let mono = [&[9u8, 7, 0, 9, 7][..], &[0; 9]].concat();
    let cases = [(["CBLC", "CBDT"], 0x0003_0000, 17, color), (["EBLC", "EBDT"], 0x0002_0000, 1, mono)];
    for (tags, version, format, image) in cases {
        let font = carets_with(&two_strikes(tags, version, format, &image));
        assert!(font.bitmap(GlyphId(1), 32).is_some(), "{tags:?}: glyph 1 draws");
        let sub = Font::from_vec(font.subset([GlyphId(1)], &Location::default()).expect("a subset").font, 0)
            .expect("the subset parses");
        assert_eq!(
            sub.table(tags[1].parse().expect("a tag")).map(<[u8]>::len),
            Some(4 + image.len()),
            "{tags:?}"
        );
    }
}
