use super::*;
use crate::daecore::daetype::{TableBytes, Tables};

// One strike whose 2,000 index records all name one subtable of one 1,000-byte image.
#[test]
fn an_image_named_many_times_is_copied_once() {
    const N: usize = 2_000;
    let mut cblc = [0x0003_0000u32, 1, 56, 0, N as u32, 0].map(u32::to_be_bytes).concat();
    cblc.resize(56, 0);
    for _ in 0..N {
        cblc.extend([1u16, 1].map(u16::to_be_bytes).concat());
        cblc.extend(((N * 8) as u32).to_be_bytes());
    }
    cblc.extend([1u16, 17].map(u16::to_be_bytes).concat());
    cblc.extend([4u32, 0, 1_000].map(u32::to_be_bytes).concat());
    let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
    cbdt.resize(4 + 1_000, 7);
    let mut active = GlyphSet::new();
    active.insert(1);

    let (_, data) = subset_bitmap_strikes(&cblc, &cbdt, &active, &[0, 1]).expect("a subset strike");
    assert!(data.len() <= cbdt.len(), "one image named {N} times took {} bytes", data.len());
}

// Strikes whose subtables hold the same images share one copy, and one array of subtables, the smaller table; the
// same bytes in another image format, or split otherwise, are copied again.
#[test]
fn the_same_images_in_other_strikes_are_written_once() {
    let two = |format: u16, split: u32| {
        let (format, split) = (format.to_be_bytes(), split.to_be_bytes());
        [&[0, 1][..], &format, &4u32.to_be_bytes(), &[0, 0, 0, 0], &split, &[0, 0, 0, 4]].concat()
    };
    let records = [(1, 2, two(17, 2)), (1, 2, two(17, 2)), (1, 2, two(18, 2)), (1, 2, two(17, 1))];
    let cbdt = [0, 3, 0, 0, 1, 2, 3, 4];
    let mut active = GlyphSet::new();
    [1, 2].iter().for_each(|&g| { active.insert(g); });
    let at = |loc: &[u8], strike: usize| {
        let array = read_u32_be(loc, 8 + 48 * strike).expect("an array") as usize;
        let subtable = array + read_u32_be(loc, array + 4).expect("a subtable") as usize;
        read_u32_be(loc, subtable + 4).expect("an image offset")
    };
    let table = cblc_per_strike(&records);
    let arrays = |loc: &[u8]| [0, 1, 2].map(|strike| read_u32_be(loc, 8 + 48 * strike).expect("an array"));
    let (loc, data) = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1, 2]).expect("a subset");
    assert_eq!(data, [&cbdt[..], &cbdt[4..], &cbdt[4..]].concat());
    assert_eq!([0, 1, 2, 3].map(|strike| at(&loc, strike)), [4, 4, 8, 12]);
    let [first, second, third] = arrays(&loc);
    assert_eq!((first == second, first == third), (true, false), "the first two strikes' arrays are the same");
}

// A subtable per run of consecutive glyphs, in index format 1 whatever the source's: of glyphs 1 to 4, 2 and 3 with
// no image, 1 and 4 take one each. A strike keeping no image leaves the tables out.
#[test]
fn each_run_of_glyphs_takes_a_format_1_subtable() {
    let index = |format: u16| {
        let header = [&format.to_be_bytes()[..], &17u16.to_be_bytes(), &4u32.to_be_bytes()].concat();
        match format {
            1 => [header, [0u32, 2, 2, 2, 4].map(u32::to_be_bytes).concat()].concat(),
            _ => [header, [0u16, 2, 2, 2, 4, 0].map(u16::to_be_bytes).concat()].concat(),
        }
    };
    let subtables = |loc: &[u8]| -> Vec<(u16, u16, u16, Vec<u32>)> {
        let array = read_u32_be(loc, 8).expect("an array") as usize;
        (0..read_u32_be(loc, 16).expect("a count") as usize).map(|j| {
            let record = array + 8 * j;
            let [first, last] = [record, record + 2].map(|at| read_u16_be(loc, at).expect("a glyph"));
            let at = array + read_u32_be(loc, record + 4).expect("a subtable") as usize;
            let format = read_u16_be(loc, at).expect("a format");
            let offset = |k: usize| match format {
                1 => read_u32_be(loc, at + 8 + 4 * k),
                _ => read_u16_be(loc, at + 8 + 2 * k).map(u32::from),
            };
            (first, last, format, (0..usize::from(last - first) + 2).map_while(offset).collect())
        }).collect()
    };
    let cbdt = [0, 3, 0, 0, 1, 2, 3, 4];
    let mut active = GlyphSet::new();
    [1, 2, 3, 4].iter().for_each(|&g| { active.insert(g); });
    let gids = [0, 1, 2, 3, 4];
    for format in [1, 3] {
        let table = cblc(1, &[(1, 4, index(format))]);
        let (loc, data) = subset_bitmap_strikes(&table, &cbdt, &active, &gids).expect("a subset");
        assert_eq!((subtables(&loc), &data[..]), (vec![(1, 1, 1, vec![0, 2]), (4, 4, 1, vec![0, 2])], &cbdt[..]));
    }
    // The same block of images split with no empty entries is another, so it is written again.
    let pair = [&[0, 1, 0, 17][..], &4u32.to_be_bytes(), &[0u32, 2, 4].map(u32::to_be_bytes).concat()].concat();
    let table = cblc_per_strike(&[(1, 4, index(1)), (5, 6, pair)]);
    [5, 6].iter().for_each(|&g| { active.insert(g); });
    let (_, data) = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1, 2, 3, 4, 5, 6]).expect("a subset");
    assert_eq!(data, [&cbdt[..], &cbdt[4..]].concat());
    let mut blank = GlyphSet::new();
    blank.insert(2);
    let table = cblc(1, &[(1, 4, index(1))]);
    assert_eq!(subset_bitmap_strikes(&table, &cbdt, &blank, &gids), None);
}

// 2,000 strike offsets naming one strike of one 1,000-byte image.
#[test]
fn a_strike_named_many_times_is_kept_once() {
    const N: usize = 2_000;
    let strike = 8 + 4 * N;
    let mut sbix = [1u16, 1].map(u16::to_be_bytes).concat();
    sbix.extend((N as u32).to_be_bytes());
    (0..N).for_each(|_| sbix.extend((strike as u32).to_be_bytes()));
    sbix.extend([72u16, 72].map(u16::to_be_bytes).concat());
    sbix.extend([16u32, 16, 16 + 8 + 1_000].map(u32::to_be_bytes).concat());
    sbix.extend([0u16, 0].map(u16::to_be_bytes).concat());
    sbix.extend(b"png ");
    sbix.extend([7u8; 1_000]);

    let out = subset_sbix(&sbix, 2, &[0, 1], &[0, 1]).expect("a subset sbix");
    assert!(out.len() <= sbix.len(), "one strike named {N} times took {} bytes", out.len());
}

// Images are copied once, but 2,000 strikes of one 5,000-glyph subtable still name 40 MB of index.
#[test]
fn a_subtable_named_past_the_budget_is_refused() {
    const GLYPHS: u16 = 5_000;
    let offsets: Vec<u8> = (0..=u32::from(GLYPHS)).flat_map(u32::to_be_bytes).collect();
    let body = [&[0u8, 1, 0, 17][..], &4u32.to_be_bytes(), &offsets].concat();
    let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
    cbdt.resize(4 + usize::from(GLYPHS), 7);
    let mut active = GlyphSet::new();
    (1..=GLYPHS).for_each(|g| { active.insert(g.into()); });
    let gid_map: Vec<u32> = (0..=GLYPHS).map(u32::from).collect();

    let table = cblc(2_000, &[(1, GLYPHS, body)]);
    assert!(subset_bitmap_strikes(&table, &cbdt, &active, &gid_map).is_none(), "40 MB of index was built");
}

// Every other glyph of 4,000 names one 20,000-byte image, each a run of its own: one shared copy is written, but
// 2,000 copies made to compare pass the budget, so the strike is refused.
#[test]
fn overlapping_images_shared_still_spend_the_budget() {
    const GLYPHS: u16 = 4_000;
    const IMAGE: u32 = 20_000;
    let offsets: Vec<u8> = (0..=u32::from(GLYPHS)).flat_map(|g| (IMAGE * (g % 2)).to_be_bytes()).collect();
    let body = [&[0u8, 1, 0, 17][..], &4u32.to_be_bytes(), &offsets].concat();
    let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
    cbdt.resize(4 + IMAGE as usize, 7);
    let mut active = GlyphSet::new();
    (1..=GLYPHS).for_each(|g| { active.insert(g.into()); });
    let gid_map: Vec<u32> = (0..=GLYPHS).map(u32::from).collect();

    let table = cblc(1, &[(1, GLYPHS, body)]);
    assert!(subset_bitmap_strikes(&table, &cbdt, &active, &gid_map).is_none(), "40 MB of copies were made");
}

// A CBLC of `strikes` strikes all naming one array of these records (first, last, the
// subtable's bytes). Bodies are laid end to end, so a record with none names the next one.
fn cblc(strikes: usize, records: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let array = 8 + strikes * 48;
    let mut out = [0x0003_0000u32, strikes as u32].map(u32::to_be_bytes).concat();
    for _ in 0..strikes {
        out.extend([array as u32, 0, records.len() as u32].map(u32::to_be_bytes).concat());
        out.extend([0; 28]);
        out.extend([0, 0, 0xFF, 0xFF, 16, 16, 32, 1]);
    }
    let mut at = records.len() * 8;
    for (first, last, body) in records {
        out.extend([first.to_be_bytes(), last.to_be_bytes()].concat());
        out.extend((at as u32).to_be_bytes());
        at += body.len();
    }
    records.iter().for_each(|r| out.extend(&r.2));
    out
}

// A CBLC of one strike per record (first, last, the subtable's bytes), each the one record of its
// strike's array. Bodies are laid end to end, so a record with none names the next one.
fn cblc_per_strike(records: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let n = records.len();
    let arrays = 8 + 48 * n;
    let mut out = [0x0003_0000u32, n as u32].map(u32::to_be_bytes).concat();
    for j in 0..n {
        out.extend([(arrays + 8 * j) as u32, 0, 1].map(u32::to_be_bytes).concat());
        out.extend([0; 28]);
        out.extend([0, 0, 0xFF, 0xFF, 16, 16, 32, 1]);
    }
    let mut at = 8 * n;
    for (j, (first, last, body)) in records.iter().enumerate() {
        out.extend([first.to_be_bytes(), last.to_be_bytes()].concat());
        out.extend(((at - 8 * j) as u32).to_be_bytes());
        at += body.len();
    }
    records.iter().for_each(|r| out.extend(&r.2));
    out
}

fn format2_empty() -> Vec<u8> {
    [&[0, 2, 0, 17, 0, 0, 0, 4][..], &[0; 12]].concat()
}

fn format1(image_format: u16, len: u32) -> Vec<u8> {
    [&[0, 1][..], &image_format.to_be_bytes(), &[0, 0, 0, 4], &0u32.to_be_bytes(), &len.to_be_bytes()].concat()
}

// A real font walks each glyph of a strike once. These 200 strikes name one empty subtable of
// about 65,535 glyphs, a different last glyph each, so the cache cannot help and the walk is refused.
#[test]
fn index_walks_past_their_steps_are_refused() {
    let mut records: Vec<(u16, u16, Vec<u8>)> = (1..200).map(|j| (1, 65_535 - j, Vec::new())).collect();
    records.push((1, 65_535, format2_empty()));
    records.push((1, 1, format1(17, 4)));
    let cbdt = [0, 3, 0, 0, 1, 2, 3, 4];
    let mut active = GlyphSet::new();
    active.insert(1);
    assert!(subset_bitmap_strikes(&cblc_per_strike(&records), &cbdt, &active, &[0, 1]).is_none());
}

// 50 strikes of 50 records naming one empty subtable of 65,534 glyphs: walked once, it fits the
// steps. Each strike's first record decides the range, and the 48 after it decide nothing.
#[test]
fn an_empty_subtable_named_many_times_is_walked_once() {
    let mut records: Vec<(u16, u16, Vec<u8>)> = (0..49).map(|_| (2, 65_535, Vec::new())).collect();
    records.push((2, 65_535, format2_empty()));
    records.push((1, 1, format1(17, 4)));
    let table = cblc(50, &records);
    let mut active = GlyphSet::new();
    active.insert(1);
    let cbdt = [0, 3, 0, 0, 1, 2, 3, 4];
    let (_, data) = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1]).expect("a subset");
    assert_eq!(data, [0, 3, 0, 0, 1, 2, 3, 4]);
}

// An EBDT composite (image format 8) naming glyph 9, past this three-glyph font, has nothing to
// draw and is dropped; the rest of the strike stays.
#[test]
fn a_composite_naming_a_missing_glyph_is_dropped() {
    let simple = [2u8, 3, 0, 2, 3, 0b1110_0000, 0b1010_0000];
    let composite = [&[2u8, 3, 0, 2, 3, 0, 0, 1][..], &9u16.to_be_bytes(), &[0, 0]].concat();
    let cbdt = [&[0, 2, 0, 0][..], &simple, &composite].concat();
    let second = [
        &[0, 1, 0, 8][..],
        &(4 + simple.len() as u32).to_be_bytes(),
        &0u32.to_be_bytes(),
        &(composite.len() as u32).to_be_bytes(),
    ]
    .concat();
    let table = cblc(1, &[(1, 1, format1(1, simple.len() as u32)), (2, 2, second)]);
    let mut active = GlyphSet::new();
    [0, 1, 2].iter().for_each(|&g| { active.insert(g); });
    let subset = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1, 2]);
    let (loc, data) = subset.expect("the simple glyph stays");
    assert_eq!(read_u32_be(&loc, 16), Some(1), "the composite kept its index subtable");
    assert_eq!(data, [&[0, 2, 0, 0][..], &simple].concat());
}

// Strikes overlapping one window of zero offsets keep nothing, but each walks every glyph. Charged,
// 100 of them pass the budget before the one real strike is reached.
#[test]
fn strikes_that_keep_nothing_still_spend_the_budget() {
    const GLYPHS: usize = 1_000;
    const SHIFTED: usize = 100;
    let strikes = 8 + (SHIFTED + 1) * 4;
    let window = 4 + 4 * (GLYPHS + 1) + SHIFTED;
    let real = strikes + window;
    let mut sbix = [1u16, 1].map(u16::to_be_bytes).concat();
    sbix.extend(((SHIFTED + 1) as u32).to_be_bytes());
    (0..SHIFTED).for_each(|i| sbix.extend(((strikes + i) as u32).to_be_bytes()));
    sbix.extend((real as u32).to_be_bytes());
    sbix.resize(real, 0);
    let head = 4 + 4 * (GLYPHS + 1);
    sbix.extend([72u16, 72].map(u16::to_be_bytes).concat());
    (0..=GLYPHS).for_each(|g| sbix.extend(((head + if g < 2 { 0 } else { 12 }) as u32).to_be_bytes()));
    sbix.extend([0, 0, 0, 0]);
    sbix.extend(b"png ");
    sbix.extend([7; 4]);
    let active: Vec<u32> = (0..GLYPHS as u32).collect();
    assert!(subset_sbix(&sbix, GLYPHS, &active, &active).is_none());
}

// Glyph 5 is past this two-glyph font. Read anyway, its offsets would land on glyph 1's record.
#[test]
fn a_glyph_past_the_font_reads_no_record() {
    let record = [&[0u8, 0, 0, 0][..], b"png ", &[7; 4]].concat();
    let mut sbix = [&[0u8, 1, 0, 1][..], &1u32.to_be_bytes(), &12u32.to_be_bytes(), &[0, 72, 0, 72]].concat();
    [32u32, 32, 32 + 12, 0, 0, 32, 32 + 12].iter().for_each(|o| sbix.extend(o.to_be_bytes()));
    sbix.resize(12 + 32, 0);
    sbix.extend(&record);
    let out = subset_sbix(&sbix, 2, &[0, 1, 5], &[0, 1, 0, 0, 0, 2]).expect("glyph 1 is kept");
    let offset = |g: usize| read_u32_be(&out, 12 + 4 + g * 4);
    assert_eq!(offset(2), offset(3), "glyph 5 took a record");
    assert_eq!(out.len(), 12 + 4 + 4 * 4 + record.len());
}

// One strike: per glyph, its whole record or none.
fn sbix_of(glyphs: &[Option<Vec<u8>>]) -> Vec<u8> {
    let head = 4 + 4 * (glyphs.len() + 1);
    let mut out = [&[0u8, 1, 0, 1][..], &1u32.to_be_bytes(), &12u32.to_be_bytes(), &[0, 16, 0, 72]].concat();
    let mut data = Vec::new();
    for g in glyphs {
        out.extend(((head + data.len()) as u32).to_be_bytes());
        data.extend(g.iter().flatten());
    }
    out.extend(((head + data.len()) as u32).to_be_bytes());
    [out, data].concat()
}

// Glyph 4 is a 'dupe' of 3 and glyph 5 a 'flip' of 2, and dropping glyph 1 shifts every id down.
#[test]
fn dupes_and_flips_name_their_targets_new_ids() {
    let record = |kind: &[u8; 4], data: &[u8]| Some([&[0u8, 0, 0, 0][..], kind, data].concat());
    let sbix = sbix_of(&[
        None,
        record(b"png ", &[1; 4]),
        record(b"png ", &[2; 4]),
        record(b"png ", &[3; 4]),
        record(b"dupe", &3u16.to_be_bytes()),
        record(b"flip", &2u16.to_be_bytes()),
    ]);
    let out = subset_sbix(&sbix, 6, &[0, 2, 3, 4, 5], &[0, 0, 1, 2, 3, 4]).expect("a subset");
    let named = |g: usize| {
        let at = 12 + read_u32_be(&out, 16 + g * 4).expect("an offset") as usize;
        (&out[at + 4..at + 8], read_u16_be(&out, at + 8))
    };
    assert_eq!(named(3), (&b"dupe"[..], Some(2)));
    assert_eq!(named(4), (&b"flip"[..], Some(1)));
}

// Once subset, each glyph keeps the image the first index covering it finds. 7 (not in a sparse index) and 12 (out of
// range) have none, 6 keeps its first, 21 runs past CBDT alone, and 1 and 2 stay though the last offsets are cut.
#[test]
fn a_subset_draws_what_its_strike_draws() {
    let image = |tag: u8| [&[1u8, 1, 0, 1, 1][..], &4u32.to_be_bytes(), &[tag; 4]].concat();
    let cbdt = [&[0u8, 3, 0, 0][..], &b"PSQqWEAB".map(image).concat()].concat();
    let format1 = |data: u32, offsets: &[u32]| {
        let offsets: Vec<u8> = offsets.iter().flat_map(|o| o.to_be_bytes()).collect();
        [&[0u8, 1, 0, 17][..], &data.to_be_bytes(), &offsets].concat()
    };
    let pairs = [5u16, 0, 12, 13, 6, 26, 6, 39, 0, 52].map(u16::to_be_bytes).concat();
    let format4 = [&[0u8, 4, 0, 17][..], &[4u32, 4].map(u32::to_be_bytes).concat(), &pairs].concat();
    let records = [
        (5, 9, format4),
        (7, 7, format1(56, &[0, 13])),
        (20, 21, format1(69, &[0, 13, 1_000])),
        (1, 3, format1(82, &[0, 13, 26])),
    ];
    let table = cblc(1, &records);
    let glyphs: Vec<u32> = (0..=30).collect();
    let mut active = GlyphSet::new();
    glyphs.iter().for_each(|&g| { active.insert(g); });

    let (loc, data) = subset_bitmap_strikes(&table, &cbdt, &active, &glyphs).expect("a subset");
    let draws = |loc: &[u8], data: &[u8]| {
        let t = Tables::from([("CBLC", TableBytes::from(loc.to_vec())), ("CBDT", TableBytes::from(data.to_vec()))]);
        let image = |g| crate::daecore::daetype::bitmap::glyph_bitmap(&t, g, 16).map(|b| format!("{:?}", b.image));
        glyphs.iter().map(|&g| image(g)).collect::<Vec<_>>()
    };
    let before = draws(&table, &cbdt);
    assert_eq!(draws(&loc, &data), before);
    assert_eq!(glyphs.iter().filter(|&&g| before[g as usize].is_some()).collect::<Vec<_>>(), [&1, &2, &5, &6, &20]);
    assert_eq!(data.len(), 4 + 5 * 13, "an image was copied twice");
}

// An sbix strike whose offsets run past the table: glyph 0's record, whole within the table, is read on its own
// and stays.
#[test]
fn a_cut_short_sbix_strike_keeps_the_records_it_holds() {
    let png = [&b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"[..], &7u32.to_be_bytes(), &9u32.to_be_bytes(), &[0; 9]].concat();
    let record = [&[0u8, 0, 0, 0][..], b"png ", &png].concat();
    let mut sbix = [&[0u8, 1, 0, 1][..], &1u32.to_be_bytes(), &12u32.to_be_bytes(), &[0, 16, 0, 72]].concat();
    sbix.extend([12, 12 + record.len() as u32].map(u32::to_be_bytes).concat());
    sbix.extend(&record);
    let out = subset_sbix(&sbix, 50, &[0], &[0]).expect("glyph 0 is kept");
    let draws = |sbix: &[u8], glyphs: u16| {
        let t: Tables = [("sbix", sbix.to_vec()), ("maxp", [&[0u8, 0, 0x50, 0][..], &glyphs.to_be_bytes()].concat())]
            .into_iter().map(|(t, b)| (t, TableBytes::from(b))).collect();
        crate::daecore::daetype::bitmap::glyph_bitmap(&t, 0, 16).map(|b| format!("{b:?}"))
    };
    assert!(draws(&sbix, 50).is_some());
    assert_eq!(draws(&out, 1), draws(&sbix, 50));
}

// Glyph 5 is a composite (EBDT image format 8) of glyphs 3 and 1, which become 2 and 1.
#[test]
fn a_composite_names_its_components_by_their_new_ids() {
    let simple = [2u8, 3, 0, 2, 3, 0b1110_0000, 0b1010_0000];
    let composite =
        [&[2u8, 3, 0, 2, 3, 0, 0, 2][..], &3u16.to_be_bytes(), &[0, 0], &1u16.to_be_bytes(), &[1, 0]].concat();
    let cbdt = [&[0, 2, 0, 0][..], &simple, &simple, &composite].concat();
    let index = |n: usize, len: usize, format: u16| {
        let from = (4 + n * simple.len()) as u32;
        [&[0u8, 1][..], &format.to_be_bytes(), &from.to_be_bytes(), &0u32.to_be_bytes(), &(len as u32).to_be_bytes()]
            .concat()
    };
    let table = cblc(
        1,
        &[(1, 1, index(0, simple.len(), 1)), (3, 3, index(1, simple.len(), 1)), (5, 5, index(2, composite.len(), 8))],
    );
    let mut active = GlyphSet::new();
    [0, 1, 3, 5].iter().for_each(|&g| { active.insert(g); });
    let (_, data) = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1, 0, 2, 0, 3]).expect("a subset");
    let image = &data[data.len() - composite.len()..];
    assert_eq!((read_u16_be(image, 8), read_u16_be(image, 12)), (Some(2), Some(1)));
}
