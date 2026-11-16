use super::*;
use super::super::cmap::{CmapLookup, UvsLookup};
use super::super::super::decoder::read_u32_be;

fn records(cmap: &[u8]) -> Vec<(u16, u16, u16)> {
    (0..usize::from(read_u16_be(cmap, 2).expect("count")))
        .map(|i| {
            let off = read_u32_be(cmap, 8 + 8 * i).expect("offset") as usize;
            (
                read_u16_be(cmap, 4 + 8 * i).expect("platform"),
                read_u16_be(cmap, 6 + 8 * i).expect("encoding"),
                read_u16_be(cmap, off).expect("format"),
            )
        })
        .collect()
}

fn listed(cmap: &[u8]) -> Option<Vec<Sequence>> {
    let tables = CharTables { cmap, dmap: &[] };
    CharMap::new(tables).sequences(tables).map(|s| s.listed())
}

fn listing(sequences: &[Sequence]) -> Sequences {
    Sequences::from_listed(sequences.iter().copied())
}

fn maps_all(cmap: &[u8], mappings: &[(u32, u32)]) {
    let lookup = CmapLookup::new(cmap);
    for &(cp, gid) in mappings {
        assert_eq!(lookup.glyph_id(cmap, cp), Some(gid), "U+{cp:X}");
    }
}

// Glyphs advancing with their codes share one segment; glyphs that do not share a glyph array;
// codes too scattered for format 4 are left to format 12 alone.
#[test]
fn format_4_holds_runs_and_gives_way_to_format_12() {
    let runs: Vec<(u32, u32)> = (0x4E00..0x4E00 + 10_000).map(|cp| (cp, cp - 0x4D00)).collect();
    let cmap = build_cmap(&runs, &Sequences::default(), false);
    assert!(cmap.len() < 100, "one run took {} bytes", cmap.len());
    maps_all(&cmap, &runs);
    let shuffled: Vec<(u32, u32)> = (0..9_000u32).map(|i| (0x4E00 + i, (i * 7_919) % 9_000 + 1)).collect();
    let cmap = build_cmap(&shuffled, &Sequences::default(), false);
    assert_eq!(records(&cmap), [(0, 3, 4), (3, 1, 4)]);
    maps_all(&cmap, &shuffled);
    let scattered: Vec<(u32, u32)> = (0..9_000u32).map(|i| (0x100 + 3 * i, i + 1)).collect();
    let cmap = build_cmap(&scattered, &Sequences::default(), false);
    assert_eq!(records(&cmap), [(0, 4, 12), (3, 10, 12)]);
    maps_all(&cmap, &scattered);
    assert_eq!(CmapLookup::new(&cmap).glyph_id(&cmap, 0x101), None);
}

// U+FFFF mapped lives in the final segment the spec requires, ending codes rising throughout.
#[test]
fn u_ffff_is_mapped_by_the_final_segment() {
    let mappings = [(0x41, 3), (0xFFFF, 5)];
    let cmap = build_cmap(&mappings, &Sequences::default(), false);
    maps_all(&cmap, &mappings);
    let sub = read_u32_be(&cmap, 8).expect("offset") as usize;
    let n = usize::from(read_u16_be(&cmap, sub + 6).expect("segCountX2")) / 2;
    let ends: Vec<u16> = (0..n).map(|i| read_u16_be(&cmap, sub + 14 + 2 * i).expect("endCode")).collect();
    assert!(ends.windows(2).all(|w| w[0] < w[1]) && ends.last() == Some(&0xFFFF), "{ends:x?}");
}

#[test]
fn records_run_in_platform_and_encoding_order() {
    let cmap = build_cmap(&[(0x41, 1), (0x1D400, 2)], &listing(&[(0xFE00, 0x41, None)]), false);
    assert_eq!(records(&cmap), [(0, 3, 4), (0, 4, 12), (0, 5, 14), (3, 1, 4), (3, 10, 12)]);
    let cmap = build_cmap(&[(0x41, 1), (0x1D400, 1), (0x1D401, 1)], &listing(&[(0xFE00, 0x41, None)]), false);
    assert_eq!(records(&cmap), [(0, 3, 4), (0, 5, 14), (0, 6, 13), (3, 1, 4), (3, 10, 13)]);
}

// A fallback font maps all of Unicode to one glyph: one format 13 group, where format 12 would take
// one per code.
#[test]
fn codes_sharing_a_glyph_take_format_13() {
    let all: Vec<(u32, u32)> = (0x20..0x11_0000).map(|cp| (cp, 1)).collect();
    let cmap = build_cmap(&all, &Sequences::default(), false);
    assert_eq!(records(&cmap), [(0, 6, 13), (3, 10, 13)]);
    assert!(cmap.len() < 100, "one group took {} bytes", cmap.len());
    assert_eq!(CmapLookup::new(&cmap).entries(&cmap), all);
}

#[test]
fn variation_sequences_read_back_as_written() {
    let sequences = [(0xFE00, 0x41, None), (0xFE00, 0x42, None), (0xFE00, 0x43, Some(9)), (0xE0100, 0x845B, Some(7))];
    let cmap = build_cmap(&[(0x41, 1), (0x42, 2), (0x43, 3), (0x845B, 4)], &listing(&sequences), false);
    let lookup = CmapLookup::new(&cmap);
    assert!(matches!(lookup.variation(&cmap, 0x41, 0xFE00), Some(UvsLookup::UseDefault)));
    assert!(matches!(lookup.variation(&cmap, 0x43, 0xFE00), Some(UvsLookup::Explicit(9))));
    assert!(matches!(lookup.variation(&cmap, 0x845B, 0xE0100), Some(UvsLookup::Explicit(7))));
    assert!(lookup.variation(&cmap, 0x44, 0xFE00).is_none());
    assert_eq!(listed(&cmap), Some(sequences.to_vec()));
}

// A glyph past 65,535 takes format 12 even in the BMP, format 4 leaving its code out rather than writing the
// glyph's low 16 bits there; a sequence's takes format 15.
#[test]
fn a_glyph_past_65_535_takes_formats_12_and_15() {
    let (mappings, sequences) = ([(0x41, 1), (0x42, 70_000)], [(0xFE0F, 0x41, Some(70_001))]);
    let cmap = build_cmap(&mappings, &listing(&sequences), false);
    assert_eq!(records(&cmap), [(0, 3, 4), (0, 4, 12), (0, 5, 15), (3, 1, 4), (3, 10, 12)]);
    maps_all(&cmap, &mappings);
    assert_eq!(listed(&cmap), Some(sequences.to_vec()));
    let at = read_u32_be(&cmap, 8 + 8 * 3).expect("offset") as usize;
    let bmp = [&[0, 0, 0, 1, 0, 3, 0, 1, 0, 0, 0, 12][..], &cmap[at..]].concat();
    let lookup = CmapLookup::new(&bmp);
    assert_eq!([lookup.glyph_id(&bmp, 0x41), lookup.glyph_id(&bmp, 0x42)], [Some(1), None]);
    let narrow = build_cmap(&[(0x41, 1)], &listing(&[(0xFE0F, 0x41, Some(2))]), false);
    assert_eq!(records(&narrow), [(0, 3, 4), (0, 5, 14), (3, 1, 4)]);
    let symbol = build_cmap(&[(0xF041, 70_000)], &Sequences::default(), true);
    assert_eq!(records(&symbol), [(0, 3, 4), (0, 4, 12), (3, 1, 4), (3, 10, 12)], "a symbol subtable is 16-bit");
    maps_all(&symbol, &[(0xF041, 70_000)]);
}

// 20,000 selectors of one sequence each: each selector's sequences are read once, where reading them all for each
// selector would be 800 million steps.
#[test]
fn each_selector_s_sequences_are_read_once() {
    let sequences: Vec<Sequence> = (0..20_000).map(|i| (0x1000 + i, 0x41, None)).collect();
    let started = std::time::Instant::now();
    let table = variations(&listing(&sequences));
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    assert_eq!(read_u32_be(&table, 6), Some(20_000));
}

// A sequence listed both ways is written once, as the default, since the default ranges are searched first.
#[test]
fn a_sequence_listed_both_ways_stays_default() {
    let cmap = build_cmap(&[(0x41, 1)], &listing(&[(0xFE00, 0x41, Some(5)), (0xFE00, 0x41, None)]), false);
    let lookup = CmapLookup::new(&cmap);
    assert!(matches!(lookup.variation(&cmap, 0x41, 0xFE00), Some(UvsLookup::UseDefault)));
    assert_eq!(listed(&cmap), Some([(0xFE00, 0x41, None)].to_vec()));
}

// A symbol font's subset is one (3,0) subtable read through its byte codes' U+F0xx: a code whose U+F0xx has the
// same glyph is left to it, and one whose U+F0xx has another glyph, or none, keeps its own.
#[test]
fn a_symbol_font_keeps_one_symbol_subtable() {
    let mappings = [(0x41, 3), (0x42, 4), (0x43, 6), (0xF042, 4), (0xF043, 5)];
    let cmap = build_cmap(&mappings, &Sequences::default(), true);
    assert_eq!(records(&cmap), [(3, 0, 4)]);
    let lookup = CmapLookup::new(&cmap);
    assert_eq!(lookup.entries(&cmap), mappings);
    assert_eq!(lookup.glyph_id(&cmap, 0xF041), None);

    let cmap = build_cmap(&mappings, &listing(&[(0xFE00, 0x41, None)]), true);
    assert_eq!(records(&cmap), [(0, 5, 14), (3, 0, 4)]);
}

fn name_table(records: &[(u16, u16, &str)]) -> Vec<u8> {
    let mut strings: Vec<u8> = Vec::new();
    let header = [0u16, records.len() as u16, 6 + 12 * records.len() as u16];
    let mut out = header.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    for &(platform, id, text) in records {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let (encoding, language) = if platform == 1 { (0, 0) } else { (1, 0x409) };
        for v in [platform, encoding, language, id, bytes.len() as u16, strings.len() as u16] {
            out.extend(v.to_be_bytes());
        }
        strings.extend(bytes);
    }
    out.extend(strings);
    out
}

// ss01 names 300; cv01 names 301 to 303 and its two parameters 310 and 311; STAT 1.1's wght axis 410, its format 1
// value 420 and its elided fallback 500, a value of a format it doesn't know naming none.
#[test]
fn the_names_tables_refer_to_are_found() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = [u16s(&[1, 0, 10, 12, 0, 0, 2]), b"ss01".to_vec(), u16s(&[14]), b"cv01".to_vec(), u16s(&[22]),
                u16s(&[4, 0, 0, 300, 4, 0, 0, 301, 302, 303, 2, 310])].concat();
    let stat = [u16s(&[1, 1, 8, 1, 0, 20, 2, 0, 28, 500]), b"wght".to_vec(), u16s(&[410, 0, 4, 16]),
                u16s(&[1, 0, 0, 420, 4, 0]), u16s(&[9, 0, 0, 430])].concat();
    let ids = names_in_use(|tag| match tag { "GSUB" => Some(&gsub[..]), "STAT" => Some(&stat[..]), _ => None });
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), [300, 301, 302, 303, 310, 311, 410, 420, 500]);
}

// 200 cv01 records name one feature whose labels run 1,000 to 65,535; cv02 and cv03 label 300 to 310 and 305 to 320.
// Each label is taken once however many records name it, where taking each record's would be 13 million steps.
#[test]
fn character_variant_labels_are_taken_once() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    const R: u16 = 200;
    let feature = |i: u16| 2 + 6 * (R + 2) + 18 * i;
    let records: Vec<u8> = (0..R).flat_map(|_| [&b"cv01"[..], &u16s(&[feature(0)])].concat()).collect();
    let tables: Vec<u8> = [(0xFFFF, 1_000), (11, 300), (16, 305)]
        .iter()
        .flat_map(|&(n, first)| u16s(&[4, 0, 0, 0, 0, 0, n, first, 0]))
        .collect();
    let gsub = [u16s(&[1, 0, 0, 10, 0, R + 2]), records, b"cv02".to_vec(), u16s(&[feature(1)]), b"cv03".to_vec(),
                u16s(&[feature(2)]), tables].concat();
    let started = std::time::Instant::now();
    let ids = names_in_use(|tag| (tag == "GSUB").then_some(&gsub[..]));
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    let want: Vec<u16> = core::iter::once(0).chain(300..=320).chain(1_000..=u16::MAX).collect();
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), want);
}

// A GSUB 1.2's ss01 naming 300, its FeatureList at the 32-bit field alone.
#[test]
fn a_version_1_2_feature_list_is_read() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = [u16s(&[1, 2, 0, 0, 0, 0, 0, 0, 26, 0, 28, 0, 0, 0, 1]), b"ss01".to_vec(), u16s(&[8, 4, 0, 0, 300])]
        .concat();
    let ids = names_in_use(|tag| (tag == "GSUB").then_some(&gsub[..]));
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), [300]);
}

// ssty is math script style, not a stylistic set: whatever its FeatureParams offset names, no name ID.
#[test]
fn only_a_two_digit_tag_is_a_stylistic_set() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = [u16s(&[1, 0, 10, 12, 0, 0, 1]), b"ssty".to_vec(), u16s(&[8, 4, 0, 0, 300])].concat();
    let ids = names_in_use(|tag| (tag == "GSUB").then_some(&gsub[..]));
    assert!(ids.is_empty(), "ssty named {ids:?}");
}

// IDs 0 to 6, 16, 17, 21, 22 and those a kept table names stay; a Mac record goes where a Windows one says the same.
#[test]
fn a_subset_keeps_the_names_it_needs() {
    let name = name_table(&[
        (1, 1, "Fam"),
        (3, 1, "Fam"),
        (1, 9, "Designer"),
        (3, 13, "License"),
        (3, 16, "Typographic family"),
        (3, 300, "Swash set"),
        (1, 301, "Mac only"),
    ]);
    let kept = kept_names(&name, &[300, 301].into_iter().collect()).expect("a name table");
    let ids: Vec<(u16, u16)> = (0..usize::from(read_u16_be(&kept, 2).expect("count")))
        .map(|i| (read_u16_be(&kept, 6 + 12 * i).expect("platform"), read_u16_be(&kept, 12 + 12 * i).expect("id")))
        .collect();
    assert_eq!(ids, [(1, 301), (3, 1), (3, 16), (3, 300)]);
}

// A count past the table's end costs a subset only the records that don't fit, not its names.
#[test]
fn a_name_table_cut_short_keeps_the_names_that_read() {
    let mut name = name_table(&[(3, 1, "A"), (3, 2, "B")]);
    name[2..4].copy_from_slice(&3u16.to_be_bytes());
    let kept = kept_names(&name, &BTreeSet::new()).expect("a name table");
    assert_eq!(name_records(&kept).map(|(key, _)| key.name_id).collect::<Vec<_>>(), [1, 2]);
}

// STAT's axis records lie designAxisSize apart: at 12 bytes each, axes naming 256 and 257 keep both names, with the
// elided fallback's 2.
#[test]
fn stat_axis_names_are_read_designaxissize_apart() {
    let words = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let header = [words(&[1, 1, 12, 2, 0, 20, 0, 0, 0, 2])].concat();
    let axes = [b"wght".to_vec(), words(&[256, 0, 0, 0]), b"wdth".to_vec(), words(&[257, 1, 0, 0])].concat();
    let stat = [header, axes].concat();
    let ids = names_in_use(|tag| (tag == "STAT").then_some(&stat[..]));
    assert_eq!(ids.into_iter().collect::<Vec<u16>>(), [2, 256, 257]);
}
