use super::*;
use alloc::string::String;
use crate::daecore::daetype::TableBytes;

fn map(tables: &[(&str, Vec<u8>)]) -> Tables {
    tables.iter().map(|(t, b)| (String::from(*t), TableBytes::from(b.clone()))).collect()
}

fn maxp(n: u16) -> Vec<u8> {
    [&[0, 0, 0x50, 0][..], &n.to_be_bytes()].concat()
}

// A PNG as far as its IHDR, which is all daegun reads of one.
fn png(height: u32) -> Vec<u8> {
    [&b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"[..], &7u32.to_be_bytes(), &height.to_be_bytes(), &[0; 9]].concat()
}

// An sbix of three glyphs: each strike a ppem and, per glyph, its record or none.
fn sbix(strikes: &[(u16, [Option<Vec<u8>>; 3])]) -> Vec<u8> {
    let mut out = [1u16, 1].map(u16::to_be_bytes).concat();
    out.extend((strikes.len() as u32).to_be_bytes());
    let mut at = out.len() + strikes.len() * 4;
    let mut bodies = Vec::new();
    for (ppem, glyphs) in strikes {
        out.extend((at as u32).to_be_bytes());
        let mut body = [ppem.to_be_bytes(), 72u16.to_be_bytes()].concat();
        let mut offset = 4 + 4 * 4;
        let mut data: Vec<u8> = Vec::new();
        for g in glyphs {
            body.extend((offset as u32).to_be_bytes());
            if let Some(g) = g {
                data.extend(g);
                offset += g.len();
            }
        }
        body.extend((offset as u32).to_be_bytes());
        body.extend(data);
        at += body.len();
        bodies.push(body);
    }
    out.extend(bodies.concat());
    out
}

fn record(x: i16, y: i16, kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    [&x.to_be_bytes()[..], &y.to_be_bytes(), kind, data].concat()
}

// An sbix image's top edge is its bottom (originOffsetY) plus its height, as CBDT's bearingY is
// its top: the two read alike. 'dupe' and 'flip' hand over to another glyph's image.
#[test]
fn sbix_places_dupes_flips_and_tops() {
    let image = record(4, -27, b"png ", &png(128));
    let dupe = record(0, 0, b"dupe", &1u16.to_be_bytes());
    let t = map(&[("maxp", maxp(3)), ("sbix", sbix(&[(109, [None, Some(image), Some(dupe)])]))]);
    let b = glyph_bitmap(&t, 1, 109).expect("glyph 1");
    assert_eq!((b.left, b.top, b.mirrored, b.ppem), (4, 101, false, 109));
    let dupe = glyph_bitmap(&t, 2, 109).expect("glyph 2, a dupe of 1");
    assert_eq!(dupe, b);

    let image = record(4, -27, b"png ", &png(128));
    let flip = record(0, 0, b"flip", &1u16.to_be_bytes());
    let t = map(&[("maxp", maxp(3)), ("sbix", sbix(&[(109, [None, Some(image), Some(flip)])]))]);
    assert!(glyph_bitmap(&t, 2, 109).expect("glyph 2, a flip of 1").mirrored);
}

// A strike without the glyph defers to the next nearest, and a strike whose header cannot be
// read is passed over rather than failing the font.
#[test]
fn another_strike_serves_a_glyph_the_nearest_lacks() {
    let image = || Some(record(0, 0, b"png ", &png(20)));
    let mut table = sbix(&[(40, [None, None, None]), (20, [None, image(), None])]);
    let t = map(&[("maxp", maxp(3)), ("sbix", table.clone())]);
    assert_eq!(glyph_bitmap(&t, 1, 40).map(|b| b.ppem), Some(20));
    table[8..12].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
    let t = map(&[("maxp", maxp(3)), ("sbix", table)]);
    assert_eq!(glyph_bitmap(&t, 1, 40).map(|b| b.ppem), Some(20), "a broken strike failed the rest");
}

// An EBLC of one strike at `depth` bits, with these index subtables (first glyph, last glyph,
// the subtable's bytes after its 8-byte header, its index and image formats).
fn eblc(depth: u8, subtables: &[(u16, u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
    let list = 8 + 48;
    let mut out = [0x0002_0000u32, 1].map(u32::to_be_bytes).concat();
    let mut size = Vec::new();
    size.extend((list as u32).to_be_bytes());
    size.extend([0; 4]);
    size.extend((subtables.len() as u32).to_be_bytes());
    size.extend([0; 4 + 24]);
    size.extend([0, 0, 0xFF, 0xFF, 16, 16, depth, 1]);
    out.extend(size);
    let mut at = subtables.len() * 8;
    let mut bodies = Vec::new();
    for (first, last, index_format, image_format, rest) in subtables {
        out.extend([first.to_be_bytes(), last.to_be_bytes()].concat());
        out.extend((at as u32).to_be_bytes());
        let mut body = [index_format.to_be_bytes(), image_format.to_be_bytes()].concat();
        body.extend(4u32.to_be_bytes());
        body.extend(rest);
        at += body.len();
        bodies.push(body);
    }
    out.extend(bodies.concat());
    out
}

fn coverage<'b>(b: &'b GlyphBitmap<'_>) -> (u16, u16, &'b [u8]) {
    match &b.image {
        BitmapImage::Coverage { width, height, pixels } => (*width, *height, pixels),
        BitmapImage::Png(_) => panic!("a PNG"),
    }
}

// EBDT's raw formats decoded: byte-aligned rows (1), bit-aligned (2), metrics from a format 2
// index (5) at 2 bits a pixel, and a composite (8) drawing two glyphs into its own box.
#[test]
fn ebdt_strikes_decode_to_coverage() {
    let small = |h: u8, w: u8, x: i8, y: i8| alloc::vec![h, w, x as u8, y as u8, w];
    let g1 = [small(2, 3, 1, 5), alloc::vec![0b1010_0000, 0b0100_0000]].concat();
    let g2 = [small(2, 3, 0, 2), alloc::vec![0b1010_1000]].concat();
    let g3 = [
        small(2, 4, 0, 5),
        alloc::vec![0, 0, 2],
        1u16.to_be_bytes().to_vec(),
        alloc::vec![0, 0],
        2u16.to_be_bytes().to_vec(),
        alloc::vec![1, 0],
    ]
    .concat();
    let ebdt = [&[0, 2, 0, 0][..], &g1, &g2, &g3].concat();
    let offsets = |sizes: &[usize]| -> Vec<u8> {
        let mut at = 0;
        let mut out = Vec::new();
        for s in sizes {
            out.extend((at as u32).to_be_bytes());
            at += s;
        }
        out.extend((at as u32).to_be_bytes());
        out
    };
    let mut rest = offsets(&[g1.len()]);
    let loc = eblc(1, &[(1, 1, 1, 1, rest.clone())]);
    let t = map(&[("EBLC", loc), ("EBDT", ebdt.clone())]);
    let b = glyph_bitmap(&t, 1, 16).expect("glyph 1");
    assert_eq!(coverage(&b), (3, 2, &[255, 0, 255, 0, 255, 0][..]));
    assert_eq!((b.left, b.top), (1, 5));

    // Glyphs 2 and 3 follow glyph 1's data, so their offsets start past it.
    rest = offsets(&[g2.len(), g3.len()])
        .chunks(4)
        .map(|c| u32::from_be_bytes(c.try_into().unwrap()) + g1.len() as u32)
        .flat_map(u32::to_be_bytes)
        .collect();
    let loc = eblc(
        1,
        &[(1, 1, 1, 1, offsets(&[g1.len()])), (2, 2, 1, 2, rest[..8].to_vec()), (3, 3, 1, 8, rest[4..].to_vec())],
    );
    let t = map(&[("EBLC", loc), ("EBDT", ebdt)]);
    assert_eq!(coverage(&glyph_bitmap(&t, 2, 16).expect("glyph 2")), (3, 2, &[255, 0, 255, 0, 255, 0][..]));
    let composite = glyph_bitmap(&t, 3, 16).expect("glyph 3");
    assert_eq!(coverage(&composite), (4, 2, &[255, 255, 255, 255, 0, 255, 255, 0][..]));

    // Two bits a pixel, metrics in a format 2 index: 3 and 1 of 3 are 255 and 85.
    let big = alloc::vec![1, 2, 0xFE, 4, 2, 0, 0, 0];
    let loc = eblc(2, &[(7, 9, 2, 5, [&1u32.to_be_bytes()[..], &big].concat())]);
    let t = map(&[("EBLC", loc), ("EBDT", alloc::vec![0, 2, 0, 0, 0, 0b1101_0000])]);
    let b = glyph_bitmap(&t, 8, 16).expect("glyph 8");
    assert_eq!((coverage(&b), b.left, b.top), ((2, 1, &[255, 85][..]), -2, 4));
}

// Sparse index formats: 4 pairs glyphs with offsets, 5 lists glyphs of one size and metrics, and
// a CBDT format 19 PNG takes its placement from that index.
#[test]
fn sparse_indexes_and_format_19_find_their_glyphs() {
    let image = png(6);
    let data = [&[0u8, 3, 0, 0][..], &(image.len() as u32).to_be_bytes(), &image].concat();
    let size = (data.len() - 4) as u32;
    let big = alloc::vec![6, 7, 2, 9, 8, 0, 0, 0];
    let five = [&size.to_be_bytes()[..], &big, &1u32.to_be_bytes(), &40u16.to_be_bytes(), &[0, 0]].concat();
    let t = map(&[("CBLC", eblc(32, &[(10, 50, 5, 19, five)])), ("CBDT", data)]);
    let b = glyph_bitmap(&t, 40, 16).expect("glyph 40");
    assert_eq!((&b.image, b.left, b.top), (&BitmapImage::Png(&image), 2, 9));
    assert!(glyph_bitmap(&t, 41, 16).is_none(), "a glyph the index leaves out");

    let small = [&[0u8, 3, 0, 0][..], &[6, 7, 1, 3, 7], &(image.len() as u32).to_be_bytes(), &image].concat();
    let pairs = [30u16, 0, 31, (small.len() - 4) as u16].map(u16::to_be_bytes).concat();
    let cblc = eblc(32, &[(30, 31, 4, 17, [&1u32.to_be_bytes()[..], &pairs].concat())]);
    let t2 = map(&[("CBLC", cblc), ("CBDT", small)]);
    let b = glyph_bitmap(&t2, 30, 16).expect("glyph 30");
    assert_eq!((b.left, b.top), (1, 3));
}

// A color bitmap's box in font units, its glyph's extents: an sbix PNG's from its origin
// offsets and IHDR size, a CBDT image's from its metrics, each scaled from its strike's ppem.
#[test]
fn color_bitmaps_give_their_box_in_font_units() {
    let image = record(4, -27, b"png ", &png(128));
    let t = map(&[("maxp", maxp(3)), ("sbix", sbix(&[(100, [None, Some(image), None]), (50, [None, None, None])]))]);
    assert_eq!(color_bitmap_extents(&t, 1, 1000), Some([40, 1010, 70, -1280]));
    assert_eq!(color_bitmap_extents(&t, 2, 1000), None, "a glyph no strike holds");

    let image = png(6);
    let data = [&[0u8, 3, 0, 0][..], &[6, 7, 1, 3, 7], &(image.len() as u32).to_be_bytes(), &image].concat();
    let pairs = [30u16, 0, 31, (data.len() - 4) as u16].map(u16::to_be_bytes).concat();
    let t = map(&[("CBLC", eblc(32, &[(30, 31, 4, 17, [&1u32.to_be_bytes()[..], &pairs].concat())])), ("CBDT", data)]);
    assert_eq!(color_bitmap_extents(&t, 30, 1000), Some([63, 188, 438, -375]), "7 by 6 at (1, 3), 16 ppem");
}

// Apple's bloc and bdat read as EBLC and EBDT do, after them. A glyph states its advances, none down in small
// metrics, and its box; the largest strike's line is its ascender and descender as stored. 16 ppem, 2048 em.
#[test]
fn strikes_state_advances_boxes_and_lines() {
    let big = [&[6u8, 3, 1, 9, 12, 0xFF, 2, 14][..], &[0b1110_0000; 6]].concat();
    let small = [&[2u8, 2, 0, 2, 6][..], &[0b1100_0000; 2]].concat();
    let offsets = |at: usize, len: usize| [at as u32, (at + len) as u32].map(u32::to_be_bytes).concat();
    let loc = eblc(1, &[(1, 1, 1, 6, offsets(0, big.len())), (2, 2, 1, 1, offsets(big.len(), small.len()))]);
    let data = [&[0u8, 2, 0, 0][..], &big, &small].concat();
    let t = map(&[("maxp", maxp(3)), ("bloc", loc.clone()), ("bdat", data.clone())]);
    let b = glyph_bitmap(&t, 1, 16).expect("glyph 1, from bdat");
    assert_eq!((coverage(&b), b.left, b.top), ((3, 6, &[255; 18][..]), 1, 9));
    let mut ebdt = data.clone();
    ebdt[12..18].fill(0b1010_0000);
    let both = map(&[("bloc", loc.clone()), ("bdat", data), ("EBLC", loc.clone()), ("EBDT", ebdt)]);
    let first = glyph_bitmap(&both, 1, 16).expect("glyph 1");
    assert_eq!(coverage(&first).2, [255, 0, 255].repeat(6), "bdat beat EBDT");

    let advance = |g, vertical| bitmap_advance(&t, g, 2048, vertical);
    let advances = [advance(1, false), advance(1, true), advance(2, false), advance(2, true)];
    assert_eq!(advances, [Some(1536), Some(1792), Some(768), None]);
    assert_eq!(advance(3, false), None, "a glyph no strike holds");
    assert_eq!(mono_bitmap_extents(&t, 1, 2048), Some([128, 1152, 384, -768]));
    assert_eq!(color_bitmap_extents(&t, 1, 2048), None, "a monochrome strike read as color");

    let lines = |hori: [i8; 4], vert: [i8; 4]| {
        let mut loc = loc.clone();
        for (at, [a, d, before, after]) in [(24, hori), (36, vert)] {
            (loc[at], loc[at + 1], loc[at + 8], loc[at + 9]) = (a as u8, d as u8, before as u8, after as u8);
        }
        let t = map(&[("bloc", loc), ("bdat", alloc::vec![0, 2, 0, 0])]);
        (strike_line_extents(&t, 2048, false), strike_line_extents(&t, 2048, true))
    };
    assert_eq!(lines([18, -2, 0, 0], [10, -10, 0, 0]), (Some((2304, -256)), Some((1280, -1280))));
    assert_eq!(lines([18, 2, 0, -2], [0; 4]).0, Some((2304, 256)), "a positive descender, whatever minAfterBL");
    assert_eq!(lines([0, 0, 14, -3], [0; 4]).0, Some((0, 0)), "maxBeforeBL and minAfterBL unread");
    assert_eq!(lines([5, 5, 0, 0], [0; 4]).0, Some((640, 640)), "an empty line");
    assert_eq!(strike_line_extents(&map(&[]), 2048, false), None);
}

// A 0x0 composite naming itself 16 times decodes no pixels, so only the cost of finding each
// component stops its 16^16 recursion. A watchdog thread catches the hang.
#[test]
fn a_self_naming_empty_composite_stops_at_the_work_budget() {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let n = 16u16;
        let glyph = [&[0u8; 8][..], &n.to_be_bytes(), &[0, 1, 0, 0].repeat(n.into())].concat();
        let offsets = [0, glyph.len() as u32].map(u32::to_be_bytes).concat();
        let t = map(&[("EBLC", eblc(1, &[(1, 1, 1, 9, offsets)])), ("EBDT", [&[0u8, 2, 0, 0][..], &glyph].concat())]);
        let _ = done.send(glyph_bitmap(&t, 1, 16).map(|b| coverage(&b).0));
    });
    let width = finished.recv_timeout(std::time::Duration::from_secs(30)).expect("the decode returns");
    assert_eq!(width, Some(0));
}

// Composites `depth` deep, each 2 by 2 placing the next at its origin, over a 2 by 2 glyph all inked: glyph 1 is the
// top. `fan` copies of each component make the walk fan out, `fan`^`depth` paths to the last.
fn nested(depth: u16, fan: u16) -> Tables {
    let composite = |next: u16| {
        let parts = [next.to_be_bytes(), [0, 0]].concat().repeat(fan.into());
        [&[2u8, 2, 0, 2, 2, 0][..], &fan.to_be_bytes(), &parts].concat()
    };
    let leaf = [2u8, 2, 0, 2, 2, 0b1100_0000, 0b1100_0000];
    let glyphs: Vec<Vec<u8>> = (1..=depth).map(|g| composite(g + 1)).chain([leaf.to_vec()]).collect();
    let offsets = |glyphs: &[Vec<u8>], from: usize| {
        let mut at = from;
        let mut out = (at as u32).to_be_bytes().to_vec();
        for g in glyphs {
            at += g.len();
            out.extend((at as u32).to_be_bytes());
        }
        out
    };
    let (composites, leaf) = glyphs.split_at(usize::from(depth));
    let after = composites.iter().map(Vec::len).sum();
    let loc = eblc(1, &[(1, depth, 1, 8, offsets(composites, 0)), (depth + 1, depth + 1, 1, 1, offsets(leaf, after))]);
    map(&[("EBLC", loc), ("EBDT", [alloc::vec![0, 2, 0, 0], glyphs.concat()].concat())])
}

// Composites nest as deep as their glyphs go: the leaf under 40 of them draws.
#[test]
fn composites_nest_as_deep_as_their_glyphs() {
    let t = nested(40, 1);
    assert_eq!(coverage(&glyph_bitmap(&t, 1, 16).expect("glyph 1")), (2, 2, &[255; 4][..]));
}

// 20 composites each naming the next 8 times would draw the leaf 8^20 times; the work stops at 64 units a byte of the
// tables, leaving what was drawn by then.
#[test]
fn composites_fanning_out_stop_at_the_work_their_bytes_allow() {
    let t = nested(20, 8);
    let (loc, data) = (&t.get("EBLC").expect("EBLC")[..], &t.get("EBDT").expect("EBDT")[..]);
    let mut work = WORK_PER_BYTE * (loc.len() + data.len());
    let size = &eblc::bitmap_sizes(loc).expect("a strike")[0];
    let strike = Strike { loc, data, size };
    let at = strike.locate(1, &mut work).expect("glyph 1");
    let (_, pixels) = strike.coverage(&at, 1, &mut work).expect("drawn");
    assert_eq!((pixels, work < COMPONENT_WORK), (alloc::vec![255; 4], true));
}
