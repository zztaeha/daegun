use super::*;
use alloc::vec;

// A cmap whose records are (platform, encoding, index into `subtables`).
fn cmap(records: &[(u16, u16, usize)], subtables: &[Vec<u8>]) -> Vec<u8> {
    let mut at = 4 + records.len() * 8;
    let offsets: Vec<usize> = subtables.iter().map(|s| { at += s.len(); at - s.len() }).collect();
    let mut out = vec![0, 0];
    out.extend((records.len() as u16).to_be_bytes());
    for &(platform, encoding, i) in records {
        out.extend(platform.to_be_bytes());
        out.extend(encoding.to_be_bytes());
        out.extend((offsets[i] as u32).to_be_bytes());
    }
    subtables.iter().for_each(|s| out.extend(s));
    out
}

fn format12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut out = vec![0, 12, 0, 0];
    out.extend(((16 + groups.len() * 12) as u32).to_be_bytes());
    out.extend(0u32.to_be_bytes());
    out.extend((groups.len() as u32).to_be_bytes());
    for &(start, end, gid) in groups {
        [start, end, gid].iter().for_each(|v| out.extend(v.to_be_bytes()));
    }
    out
}

fn format13(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut out = format12(groups);
    out[1] = 13;
    out
}

// Format 4 from its columns: (start, end, delta, range offset) segments, then the glyph array.
fn format4_columns(language: u16, segments: &[(u16, u16, u16, u16)], glyphs: &[u16]) -> Vec<u8> {
    let n = segments.len() as u16;
    let mut out = vec![0, 4];
    out.extend((16 + n * 8 + glyphs.len() as u16 * 2).to_be_bytes());
    out.extend(language.to_be_bytes());
    out.extend((n * 2).to_be_bytes());
    out.extend([0; 6]);
    segments.iter().for_each(|s| out.extend(s.1.to_be_bytes()));
    out.extend([0, 0]);
    segments.iter().for_each(|s| out.extend(s.0.to_be_bytes()));
    segments.iter().for_each(|s| out.extend(s.2.to_be_bytes()));
    segments.iter().for_each(|s| out.extend(s.3.to_be_bytes()));
    glyphs.iter().for_each(|g| out.extend(g.to_be_bytes()));
    out
}

// Segments of (start, end, first glyph), closed by the 0xFFFF segment.
fn format4(language: u16, segments: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut columns: Vec<(u16, u16, u16, u16)> =
        segments.iter().map(|&(start, end, first)| (start, end, first.wrapping_sub(start), 0)).collect();
    columns.push((0xFFFF, 0xFFFF, 1, 0));
    format4_columns(language, &columns, &[])
}

fn format6(language: u16, first: u16, glyphs: &[u16]) -> Vec<u8> {
    let mut out = vec![0, 6];
    out.extend((10 + glyphs.len() as u16 * 2).to_be_bytes());
    out.extend(language.to_be_bytes());
    out.extend(first.to_be_bytes());
    out.extend((glyphs.len() as u16).to_be_bytes());
    glyphs.iter().for_each(|g| out.extend(g.to_be_bytes()));
    out
}

fn format10(first: u32, glyphs: &[u16]) -> Vec<u8> {
    let mut out = vec![0, 10, 0, 0];
    out.extend((20 + glyphs.len() as u32 * 2).to_be_bytes());
    out.extend(0u32.to_be_bytes());
    out.extend(first.to_be_bytes());
    out.extend((glyphs.len() as u32).to_be_bytes());
    glyphs.iter().for_each(|g| out.extend(g.to_be_bytes()));
    out
}

fn format0(glyphs: &[(u8, u8)]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 6, 0, 0];
    let mut array = [0u8; 256];
    glyphs.iter().for_each(|&(code, gid)| array[usize::from(code)] = gid);
    out.extend(array);
    out
}

// A format 14 of one selector's default ranges, as (start, additional count), and glyph mappings.
fn format14(selector: u32, defaults: &[(u32, u8)], glyphs: &[(u32, u16)]) -> Vec<u8> {
    let glyphs: Vec<(u32, u32)> = glyphs.iter().map(|&(base, g)| (base, g.into())).collect();
    variations(14, selector, defaults, &glyphs)
}

// Format 14's layout, its glyphs in 24 bits for format 15.
fn variations(format: u8, selector: u32, defaults: &[(u32, u8)], glyphs: &[(u32, u32)]) -> Vec<u8> {
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let glyph = |g: u32| if format == 15 { u24(g) } else { (g as u16).to_be_bytes().to_vec() };
    let mut ranges = (defaults.len() as u32).to_be_bytes().to_vec();
    defaults.iter().for_each(|&(start, more)| { ranges.extend(u24(start)); ranges.push(more); });
    let mut mappings = (glyphs.len() as u32).to_be_bytes().to_vec();
    glyphs.iter().for_each(|&(base, g)| { mappings.extend(u24(base)); mappings.extend(glyph(g)); });
    let mut out = vec![0, format];
    out.extend((21 + ranges.len() as u32 + mappings.len() as u32).to_be_bytes());
    out.extend(1u32.to_be_bytes());
    out.extend(u24(selector));
    out.extend(21u32.to_be_bytes());
    out.extend((21 + ranges.len() as u32).to_be_bytes());
    out.extend(ranges);
    out.extend(mappings);
    out
}

fn glyphs(table: &[u8], codes: &[u32]) -> Vec<Option<u32>> {
    let lookup = CmapLookup::new(table);
    codes.iter().map(|&code| lookup.glyph_id(table, code)).collect()
}

fn entries(table: &[u8]) -> Vec<(u32, u32)> {
    CmapLookup::new(table).entries(table)
}

fn indexed(entries: &[(u32, u32)], code: u32) -> Option<u32> {
    entries.binary_search_by_key(&code, |&(c, _)| c).ok().map(|i| entries[i].1)
}

// Inter's layout: two records each for one format 4 and one format 12. Subtables are exclusive and the 32-bit one
// is to be used, so format 12 is read alone and a code mapped only by format 4 maps nothing.
#[test]
fn the_best_record_answers_alone() {
    let bmp = format4(0, &[(0x41, 0x42, 7)]);
    let full = format12(&[(0x41, 0x41, 9), (0x10000, 0x10000, 10)]);
    let table = cmap(&[(0, 3, 0), (0, 4, 1), (3, 1, 0), (3, 10, 1)], &[bmp, full]);
    assert_eq!(glyphs(&table, &[0x41, 0x42, 0x10000]), [Some(9), None, Some(10)]);
    assert_eq!(entries(&table), [(0x41, 9), (0x10000, 10)]);
}

// Records are sorted, so they're found by binary search: out of order, (3,10) and (0,3) hide from it and it finds
// (3,1). A record of offset 0 is no record.
#[test]
fn records_are_found_by_binary_search() {
    let subtables = [format12(&[(0x41, 0x41, 9)]), format4(0, &[(0x41, 0x41, 7)]), format4(0, &[(0x41, 0x41, 8)])];
    let table = cmap(&[(3, 10, 0), (0, 3, 1), (3, 1, 2)], &subtables);
    assert_eq!(glyphs(&table, &[0x41]), [Some(8)]);

    let subtables = [format4(0, &[(0x41, 0x41, 7)]), format4(0, &[(0xF041, 0xF041, 5)])];
    let mut table = cmap(&[(0, 3, 0), (3, 0, 1)], &subtables);
    table[16..20].copy_from_slice(&[0; 4]);
    assert_eq!(glyphs(&table, &[0x41, 0xF041]), [Some(7), None]);
}

// A subtable whose check fails is absent and the rest are read: beside the Unicode one it maps nothing, and as the
// best it gives way to the next best, Mac Roman here. A header of another version maps nothing at all.
#[test]
fn a_subtable_that_fails_its_check_is_absent() {
    let unicode = format4(0, &[(0x41, 0x41, 7)]);
    let mut past_the_end = format4(0, &[(0x80, 0x80, 8)]);
    past_the_end[6..8].copy_from_slice(&40u16.to_be_bytes());
    let truncated = format0(&[(0x41, 3)])[..200].to_vec();
    let mut uvs_past_the_end = format14(0xFE00, &[(0x41, 0)], &[]);
    uvs_past_the_end[13..17].copy_from_slice(&100u32.to_be_bytes());
    for bad in [past_the_end.clone(), truncated, uvs_past_the_end] {
        let table = cmap(&[(0, 5, 1), (3, 1, 0)], &[unicode.clone(), bad]);
        assert_eq!(glyphs(&table, &[0x41]), [Some(7)]);
        assert_eq!(entries(&table), [(0x41, 7)]);
        assert!(CmapLookup::new(&table).variation(&table, 0x41, 0xFE00).is_none());
    }
    let mac = format0(&[(0x41, 3)]);
    assert_eq!(glyphs(&cmap(&[(1, 0, 0), (3, 1, 1)], &[mac, past_the_end]), &[0x41]), [Some(3)]);

    let mut table = cmap(&[(3, 1, 0)], core::slice::from_ref(&unicode));
    assert_eq!(glyphs(&table, &[0x41]), [Some(7)]);
    table[1] = 1;
    assert_eq!(glyphs(&table, &[0x41]), [None]);
    let unknown = vec![0, 7, 0, 0];
    assert_eq!(glyphs(&cmap(&[(1, 1, 1), (3, 1, 0)], &[unicode, unknown]), &[0x41]), [Some(7)]);
}

// The lookup checks only the subtables it reads, so records sharing one large format 4, however many, read it whole.
#[test]
fn the_lookup_checks_only_the_subtables_it_reads() {
    let segments: Vec<(u16, u16, u16)> = (0..999).map(|i| (0x100 + 2 * i, 0x100 + 2 * i, 1)).collect();
    let large = format4(0, &segments);
    for records in [1, 69, 5_000] {
        let table = cmap(&vec![(3, 1, 0); records], core::slice::from_ref(&large));
        assert_eq!(glyphs(&table, &[0x100]), [Some(1)], "{records} records");
    }
}

// Mac Roman's byte 0xDB is the euro sign, U+20AC; a code past the BMP is no Mac Roman character, so U+120AC maps
// nothing.
#[test]
fn mac_roman_maps_only_its_own_characters() {
    let table = cmap(&[(1, 0, 0)], &[format0(&[(0x41, 3), (0xDB, 7)])]);
    let codes = [0x41, 0x10041, 0x20AC, 0x120AC, 0x10_20AC, 0x80];
    assert_eq!(glyphs(&table, &codes), [Some(3), None, Some(7), None, None, None]);
    assert_eq!(entries(&table), [(0x41, 3), (0x20AC, 7)]);
}

// A Mac Roman table is keyed by Mac bytes whatever its format: byte 0x80 is Ä, not U+0080.
#[test]
fn mac_tables_are_keyed_by_mac_bytes_in_every_format() {
    let roman6 = cmap(&[(1, 0, 0)], &[format6(0, 0x80, &[10, 11])]);
    let roman4 = cmap(&[(1, 0, 0)], &[format4(0, &[(0x80, 0x81, 10)])]);
    for table in [&roman6, &roman4] {
        assert_eq!(glyphs(table, &[0x80, 'Ä' as u32, 'Å' as u32]), [None, Some(10), Some(11)]);
        assert_eq!(indexed(&entries(table), 'Ä' as u32), Some(10));
    }

    let turkish = cmap(&[(1, 0, 0)], &[format6(MAC_LANG_TURKISH, 0xDA, &[20])]);
    assert_eq!(glyphs(&turkish, &['Ğ' as u32]), [Some(20)]);
    assert_eq!(entries(&turkish), [('Ğ' as u32, 20)]);
}

// Mac Japanese is read for ASCII alone, and Windows PRC not at all.
#[test]
fn any_other_mac_subtable_answers_for_ascii() {
    let table = cmap(&[(1, 1, 0), (3, 3, 1)], &[format0(&[(0x41, 5), (0xC4, 6)]), format4(0, &[(0x42, 0x42, 7)])]);
    assert_eq!(glyphs(&table, &[0x41, 0x42, 0xC4]), [Some(5), None, None]);
    assert_eq!(entries(&table), [(0x41, 5)]);
}

// Wingdings' layout: the symbol table maps U+F020..U+F0FF, which answers for 0x20..0xFF too.
// A code the table maps itself still wins over its U+F0xx alias.
#[test]
fn a_symbol_table_answers_for_its_byte_codes() {
    let table = cmap(&[(3, 0, 0)], &[format4(0, &[(0x41, 0x41, 500), (0xF020, 0xF0FF, 10)])]);
    let codes = [0x42, 0xE0, 0xF041, 0x41, 0x141];
    assert_eq!(glyphs(&table, &codes), [Some(10 + 0x22), Some(10 + 0xC0), Some(10 + 0x21), Some(500), None]);
    let entries = entries(&table);
    assert_eq!(indexed(&entries, 0x42), Some(10 + 0x22));
    assert_eq!(indexed(&entries, 0x41), Some(500));
}

// A symbol font answers for byte codes at U+F0xx alone: OS/2's fsSelection holds no font page, the OS/2 chapter having
// bits 7 to 15 ignored in versions 0 to 3, so the cmap is read without it.
#[test]
fn a_symbol_font_reads_no_font_page() {
    let table = cmap(&[(3, 0, 0)], &[format4(0, &[(0xF041, 0xF041, 3), (0xF14A, 0xF14A, 4), (0xF24C, 0xF24C, 5)])]);
    assert_eq!(glyphs(&table, &[0x41, 0x628]), [Some(3), None]);
}

// A range offset counts bytes from its own place, wherever that lands: U+0041's 2 reads the next range offset, 4,
// and U+0050's 4 reads the glyph array's 9 and 0. A glyph of 0 is no glyph, and the delta adds to any other.
#[test]
fn format_4_reads_its_glyph_where_the_range_offset_points() {
    let segments = [(0x41, 0x41, 0, 2), (0x50, 0x51, 0x10, 4), (0xFFFF, 0xFFFF, 1, 0)];
    let table = cmap(&[(3, 1, 0)], &[format4_columns(0, &segments, &[9, 0])]);
    assert_eq!(glyphs(&table, &[0x41, 0x50, 0x51]), [Some(4), Some(25), None]);
    assert_eq!(entries(&table), [(0x41, 4), (0x50, 25)]);
}

// Formats 6 and 10 index by the code less the first, unsigned: format 6 runs past the BMP, and format 10
// wraps past 0xFFFFFFFF to 0.
#[test]
fn trimmed_arrays_index_from_their_first_code() {
    let table = cmap(&[(3, 1, 0)], &[format6(0, 0xFFFE, &[3, 4, 5, 6])]);
    assert_eq!(glyphs(&table, &[0xFFFE, 0x10001, 0x10002]), [Some(3), Some(6), None]);
    let table = cmap(&[(3, 10, 0)], &[format10(0xFFFF_FFFF, &[3, 4, 5])]);
    assert_eq!(glyphs(&table, &[0, 1, 2]), [Some(4), Some(5), None]);
    assert_eq!(entries(&table), [(0, 4), (1, 5)]);
}

// Segments out of order hide a code from the binary search, as U+0030 here. Format 4 compares a code with endCode
// first and format 12 with startCharCode, so a segment running backward sends them apart: only format 4 finds U+0055.
#[test]
fn segments_out_of_order_map_what_the_search_finds() {
    let table = cmap(&[(3, 1, 0)], &[format4(0, &[(0x41, 0x41, 7), (0x50, 0x50, 8), (0x30, 0x30, 9)])]);
    assert_eq!(glyphs(&table, &[0x30, 0x41, 0x50]), [None, Some(7), Some(8)]);
    assert_eq!(entries(&table), [(0x41, 7), (0x50, 8)]);

    let segments = [(0x41, 0x41, 7), (0x60, 0x50, 8), (0x55, 0x55, 9)];
    let table = cmap(&[(3, 1, 0)], &[format4(0, &segments)]);
    assert_eq!(glyphs(&table, &[0x55]), [Some(9)]);
    let groups = segments.map(|(start, end, first)| (u32::from(start), u32::from(end), u32::from(first)));
    let table = cmap(&[(3, 10, 0)], &[format12(&groups)]);
    assert_eq!(glyphs(&table, &[0x41, 0x55]), [Some(7), None]);
}

// A format 13 group from U+0020 runs past Unicode, as a fallback font's may. Every code to U+10FFFF
// is listed, none past it is, and a lookup agrees.
#[test]
fn codes_past_unicode_map_nothing() {
    let table = cmap(&[(3, 10, 0)], &[format13(&[(0x20, 0xFFFF_FFFF, 3)])]);
    let entries = entries(&table);
    assert_eq!(entries.len(), 0x11_0000 - 0x20);
    assert!(entries.iter().all(|&(_, g)| g == 3));
    assert_eq!(entries[entries.len() - 1].0, 0x10FFFF);
    assert_eq!(glyphs(&table, &[0x10FFFF, 0x11_0000, u32::MAX]), [Some(3), None, None]);
}

// Overlapping groups that each cover all of Unicode list every code once.
#[test]
fn overlapping_groups_list_each_code_once() {
    let table = cmap(&[(3, 10, 0)], &[format13(&[(0, 0x10FFFF, 1); 5])]);
    let entries = entries(&table);
    assert_eq!(entries.len(), 0x11_0000);
    assert!(entries.iter().enumerate().all(|(i, &(code, glyph))| code == i as u32 && glyph == 1));
}

// Every way of reading a cmap, the Unicode ones with segments out of order. The index must give what the
// direct lookup gives for every code.
#[test]
fn the_index_agrees_with_the_lookup_everywhere() {
    let mixed = [(0x41, 0x5A, 1), (0x60, 0x50, 2), (0x55, 0x55, 3), (0x30, 0x30, 4)];
    let groups = mixed.map(|(start, end, first)| (u32::from(start), u32::from(end), u32::from(first)));
    let tables = [
        cmap(&[(3, 0, 0)], &[format4(0, &[(0x30, 0x30, 70), (0xF020, 0xF0FF, 100)])]),
        cmap(&[(3, 0, 0)], &[format4(0, &[(0x30, 0x30, 3), (0xF000, 0xF2FF, 10)])]),
        cmap(&[(3, 1, 0)], &[format4(0, &mixed)]),
        cmap(&[(3, 10, 0)], &[format12(&[groups.as_slice(), &[(0x1F600, 0x1F64F, 400)]].concat())]),
        cmap(&[(1, 0, 0)], &[format6(0, 0x3E, &[300, 301, 302, 303, 304])]),
        cmap(&[(1, 0, 0)], &[format4(0, &[(0x41, 0xFF, 1)])]),
        cmap(&[(1, 0, 0)], &[format6(MAC_LANG_TURKISH, 0x40, &[1; 0xC0])]),
        cmap(&[(1, 1, 0)], &[format0(&[(0x42, 9), (0xC0, 10)])]),
        // Runs that wrap to glyph 0, start on it, run past glyph 65,535 (which the index refuses) or name none.
        cmap(&[(3, 1, 0)], &[format4(0, &[(0x100, 0x1FF, 0xFFF0), (0x300, 0x310, 0)])]),
        cmap(&[(3, 10, 0)], &[format12(&[(0x20, 0x30, 0), (0x1000, 0x1FFF, 0xFFF8), (0x3000, 0x3005, 0x1_0000),
                                        (0x4000, 0x4003, 0xFFFF_FFFE)])]),
        cmap(&[(3, 10, 0)], &[format13(&[(0x41, 0x50, 0), (0x60, 0x70, 0x1_0005), (0x80, 0x2_0090, 7)])]),
    ];
    for table in &tables {
        let (entries, lookup) = (entries(table), CmapLookup::new(table));
        let index = crate::daecore::daetype::format::index::SparseIndex::build(
            &|e| { lookup.each_entry(table, e); Some(()) }, 0, usize::MAX,
        );
        assert_eq!(index.is_none(), entries.iter().any(|&(_, g)| g > 0xFFFF));
        for code in 0..=0x11_0000 {
            assert_eq!(indexed(&entries, code), lookup.glyph_id(table, code), "U+{code:04X}");
            if let Some(index) = &index {
                let from_index = Some(u32::from(index.lookup(code))).filter(|&g| g != 0);
                assert_eq!(from_index, lookup.glyph_id(table, code), "U+{code:04X} from the index");
            }
        }
    }
}

// The sequences come from the first format 14 subtable among the (0, 5) records, read in order whatever order
// the records are in.
#[test]
fn the_first_variation_subtable_is_read_however_the_records_lie() {
    let (five, six) = (format14(0xFE00, &[], &[(0x41, 5)]), format14(0xFE00, &[], &[(0x41, 6)]));
    let explicit = |table: &[u8]| match CmapLookup::new(table).variation(table, 0x41, 0xFE00) {
        Some(UvsLookup::Explicit(g)) => Some(g),
        _ => None,
    };
    let unsorted = cmap(&[(3, 1, 0), (0, 5, 1)], &[format4(0, &[(0x41, 0x41, 1)]), five.clone()]);
    assert_eq!(explicit(&unsorted), Some(5));
    assert_eq!(explicit(&cmap(&[(0, 5, 0), (0, 5, 1)], &[five, six])), Some(5));
}

// A sequence listed both as a default and with its own glyph is a default one: its record names the default table
// first.
#[test]
fn default_ranges_answer_before_glyphs() {
    let table = cmap(&[(0, 5, 0)], &[format14(0xFE00, &[(0x41, 1)], &[(0x41, 5), (0x43, 6)])]);
    let lookup = CmapLookup::new(&table);
    assert!(matches!(lookup.variation(&table, 0x41, 0xFE00), Some(UvsLookup::UseDefault)));
    assert!(matches!(lookup.variation(&table, 0x43, 0xFE00), Some(UvsLookup::Explicit(6))));
    let listed = [(0xFE00, 0x41, None), (0xFE00, 0x42, None), (0xFE00, 0x43, Some(6))];
    let tables = CharTables { cmap: &table, dmap: &[] };
    assert_eq!(CharMap::new(tables).sequences(tables).map(|s| s.listed()), Some(listed.to_vec()));
}

// A non-default mapping to glyph 0, the missing glyph, reads as no variant, so the base glyph stays.
#[test]
fn a_variant_mapped_to_notdef_is_no_variant() {
    let table = cmap(&[(0, 5, 0)], &[format14(0xE0100, &[], &[(0x845B, 0)])]);
    assert!(CmapLookup::new(&table).variation(&table, 0x845B, 0xE0100).is_none());
}

fn explicit(found: Option<UvsLookup>) -> Option<u32> {
    match found {
        Some(UvsLookup::Explicit(g)) => Some(g),
        _ => None,
    }
}

// Format 15 is format 14 with 24-bit glyphs, mappings of 6 bytes: format 14's bytes labeled 15 read the next
// byte into the glyph, and are absent where the array then runs past it, the rest of the cmap read.
#[test]
fn format_15_reads_its_glyphs_in_24_bits() {
    let mappings = [(0x41, 0x1_0001), (0x42, 0x1_0002)];
    let table = cmap(&[(0, 5, 0)], &[variations(15, 0xFE0F, &[], &mappings)]);
    let lookup = CmapLookup::new(&table);
    assert_eq!(explicit(lookup.variation(&table, 0x42, 0xFE0F)), Some(0x1_0002));
    let tables = CharTables { cmap: &table, dmap: &[] };
    let listed = mappings.map(|(base, glyph)| (0xFE0F, base, Some(glyph)));
    assert_eq!(CharMap::new(tables).sequences(tables).map(|s| s.listed()), Some(listed.to_vec()));

    let mut relabeled = format14(0xFE0F, &[], &[(0x41, 7)]);
    relabeled[1] = 15;
    let nominal = format4(0, &[(0x41, 0x41, 1)]);
    let cut = cmap(&[(0, 5, 1), (3, 1, 0)], &[nominal.clone(), relabeled.clone()]);
    assert_eq!(glyphs(&cut, &[0x41]), [Some(1)]);
    assert!(CmapLookup::new(&cut).variation(&cut, 0x41, 0xFE0F).is_none());
    relabeled.push(0);
    let read = cmap(&[(0, 5, 1), (3, 1, 0)], &[nominal, relabeled]);
    assert_eq!(explicit(CmapLookup::new(&read).variation(&read, 0x41, 0xFE0F)), Some(0x700));
}

// The first format 15 of the (0, 5) records, the 24-bit twin, answers wherever format 14s lie, and a format 14 only
// where there is none.
#[test]
fn the_first_format_15_answers_before_any_format_14() {
    let low = format14(0xFE0F, &[], &[(0x41, 2)]);
    let first = variations(15, 0xFE0F, &[], &[(0x41, 0x1_0001)]);
    let second = variations(15, 0xFE0F, &[], &[(0x41, 3)]);
    let table = cmap(&[(0, 5, 0), (0, 5, 1), (0, 5, 2)], &[low.clone(), first.clone(), second.clone()]);
    assert_eq!(explicit(CmapLookup::new(&table).variation(&table, 0x41, 0xFE0F)), Some(0x1_0001));
    let table = cmap(&[(0, 5, 0), (0, 5, 1), (0, 5, 2)], &[second, first, low]);
    assert_eq!(explicit(CmapLookup::new(&table).variation(&table, 0x41, 0xFE0F)), Some(3));
}

// A format 12 or 13 group's glyphs map in all 32 bits, past 65,535 as below it.
#[test]
fn a_glyph_past_65_535_maps() {
    let table = cmap(&[(3, 10, 0)], &[format12(&[(0x41, 0x42, 70_000), (0x1_0000, 0x1_0000, 0xFF_FFFF)])]);
    assert_eq!(glyphs(&table, &[0x41, 0x42, 0x1_0000]), [Some(70_000), Some(70_001), Some(0xFF_FFFF)]);
    assert_eq!(entries(&table), [(0x41, 70_000), (0x42, 70_001), (0x1_0000, 0xFF_FFFF)]);
    let table = cmap(&[(3, 10, 0)], &[format13(&[(0x41, 0x43, 70_000)])]);
    assert_eq!(glyphs(&table, &[0x41, 0x43]), [Some(70_000), Some(70_000)]);
}

fn char_map<'a>(cmap: &'a [u8], dmap: &'a [u8]) -> (CharMap, CharTables<'a>) {
    let tables = CharTables { cmap, dmap };
    (CharMap::new(tables), tables)
}

// DMAP answers first, its glyph 0 or a code it leaves out falling through to cmap. A DMAP the sanitizer refuses
// (version 1) is read as missing, and DMAP alone maps.
#[test]
fn dmap_answers_before_cmap() {
    let base = cmap(&[(3, 10, 0)], &[format12(&[(0x41, 0x42, 1)])]);
    let dmap = cmap(&[(3, 10, 0)], &[format12(&[(0x41, 0x41, 5)])]);
    let (map, tables) = char_map(&base, &dmap);
    assert_eq!([map.glyph_id(tables, 0x41), map.glyph_id(tables, 0x42)], [Some(5), Some(2)]);
    assert_eq!(map.entries(tables), [(0x41, 5), (0x42, 2)]);

    let zero = cmap(&[(3, 10, 0)], &[format12(&[(0x41, 0x41, 0)])]);
    let (map, tables) = char_map(&base, &zero);
    assert_eq!(map.glyph_id(tables, 0x41), Some(1));
    let mut refused = dmap.clone();
    refused[1] = 1;
    let (map, tables) = char_map(&base, &refused);
    assert_eq!(map.glyph_id(tables, 0x41), Some(1));
    let (map, tables) = char_map(&[], &dmap);
    assert_eq!([map.glyph_id(tables, 0x41), map.glyph_id(tables, 0x42)], [Some(5), None]);
    let bmp = cmap(&[(3, 1, 0)], &[format4(0, &[(0x41, 0x41, 9)])]);
    let (map, tables) = char_map(&base, &bmp);
    assert_eq!(map.glyph_id(tables, 0x41), Some(9));
}

// A sequence DMAP lists takes DMAP's answer, a default from DMAP over cmap's glyph and DMAP's glyph over cmap's
// default or its own; one DMAP leaves out takes cmap's. Only the answers are listed.
#[test]
fn dmap_answers_sequences_before_cmap() {
    let base = cmap(&[(0, 5, 0)], &[format14(0xFE0F, &[(0x43, 0)], &[(0x41, 3), (0x42, 3), (0x44, 3)])]);
    let dmap = cmap(&[(0, 5, 0)], &[variations(15, 0xFE0F, &[(0x41, 0)], &[(0x43, 0x1_0001), (0x44, 4)])]);
    let (map, tables) = char_map(&base, &dmap);
    assert!(matches!(map.variation(tables, 0x41, 0xFE0F), Some(UvsLookup::UseDefault)));
    assert_eq!(explicit(map.variation(tables, 0x42, 0xFE0F)), Some(3));
    assert_eq!(explicit(map.variation(tables, 0x43, 0xFE0F)), Some(0x1_0001));
    assert_eq!(explicit(map.variation(tables, 0x44, 0xFE0F)), Some(4));
    let listed = [(0x41, None), (0x42, Some(3)), (0x43, Some(0x1_0001)), (0x44, Some(4))];
    let listed = listed.map(|(base, glyph)| (0xFE0F, base, glyph)).to_vec();
    assert_eq!(map.sequences(tables).map(|s| s.listed()), Some(listed));
}

// A hostile cmap's code can be any u32, and a caller may have overflow checks on.
#[test]
fn the_symbol_remap_reads_a_low_byte_alone() {
    assert_eq!(symbol_code(0x41), Some(0xF041));
    assert_eq!(symbol_code(0x100), None);
    assert_eq!(symbol_code(u32::MAX), None);
}

// A format 14 of records from U+E0100 on, each naming as its default ranges the array at one of `words`, which holds
// its count; a word as a range is its first 3 bytes' base and its last byte's more.
fn defaults_at(words: &[u32], starts: &[u32]) -> Vec<u8> {
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let at = 10 + 11 * starts.len() as u32;
    let record = |(&word, vs): (&u32, u32)| [u24(vs), (at + 4 * word).to_be_bytes().to_vec(), vec![0; 4]].concat();
    let records: Vec<u8> = starts.iter().zip(0xE_0100..).flat_map(record).collect();
    let length = (at + 4 * words.len() as u32).to_be_bytes();
    let words: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    [&[0, 14][..], &length, &(starts.len() as u32).to_be_bytes(), &records, &words].concat()
}

// Records sharing one array search it once: 200 sharing 4,000 ranges read whole, 204.8 million sequences as ranges.
// Records naming arrays that overlap search each, so 100 naming arrays of 256 to 25,600 of 25,701 ranges, 1.3 million
// steps from 103 KB, are refused at the table's subset budget.
#[test]
fn records_naming_one_array_search_it_once() {
    let read = |format14: Vec<u8>| {
        let table = cmap(&[(0, 5, 0)], &[format14]);
        let tables = CharTables { cmap: &table, dmap: &[] };
        CharMap::new(tables).sequences(tables)
    };
    let shared: Vec<u32> = core::iter::once(4_000).chain((0..4_000).map(|r| (256 * r) << 8 | 255)).collect();
    let sequences = read(defaults_at(&shared, &[0; 200])).expect("read whole");
    assert_eq!(sequences.defaults.len(), 200);
    assert!(sequences.defaults.iter().all(|&(_, first, last)| (first, last) == (0, 256 * 4_000 - 1)));
    let overlapping: Vec<u32> = (0..25_701).map(|k| k << 8).collect();
    let starts: Vec<u32> = (1..=100).collect();
    assert_eq!(read(defaults_at(&overlapping, &starts)), None);
}
