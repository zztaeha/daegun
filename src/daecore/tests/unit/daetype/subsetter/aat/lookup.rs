use super::*;

fn read_back(table: &[u8], entries: &[(u16, u16)]) {
    let lookup = Lookup::parse(table, u16::MAX.into()).expect("the lookup parses");
    assert_eq!(lookup.pairs(), entries, "format {}", table[1]);
    for &(g, v) in entries {
        assert_eq!(lookup.value(g.into()), Some(v));
    }
}

// Format 10 stays format 10 at its width, each of the unit sizes Apple's lookup tables allow, a gap between kept
// glyphs in class 1.
#[test]
fn a_wide_class_lookup_keeps_its_format_and_width() {
    for unit in [1, 2, 4, 8] {
        let table = build_class_lookup(&[(9, 6), (5, 4), (7, 5)], Some(unit)).expect("a lookup");
        assert_eq!(table[1], 10);
        let lookup = Lookup::parse(&table, u16::MAX.into()).expect("the lookup parses");
        assert_eq!(lookup.wide_unit(), Some(unit));
        assert_eq!(lookup.pairs(), [(5, 4), (6, 1), (7, 5), (8, 1), (9, 6)], "{unit}-byte values");
    }
}

#[test]
fn each_shape_takes_its_smallest_format_and_reads_back() {
    let runs: Vec<(u16, u16)> = (10..400).map(|g| (g, 1 + g / 100)).collect();
    let dense: Vec<(u16, u16)> = (10..400).map(|g| (g, g * 3)).collect();
    let pieces: Vec<(u16, u16)> = (0..40).flat_map(|r| (r * 100..r * 100 + 20).map(|g| (g, g ^ 0x55))).collect();
    let scattered: Vec<(u16, u16)> = (0..300).map(|i| (i * 7, i)).collect();
    for (entries, format) in [(runs, 2), (pieces, 4), (scattered, 6), (dense, 8), (Vec::new(), 8)] {
        let table = build_aat_lookup(&entries).expect("a lookup");
        assert_eq!(table[1], format);
        read_back(&table, &entries);
    }
}

// The smallest format is written, and past 16,383 units format 6's searchRange leaves 16 bits, so these pieces,
// each running with its own values, take format 4.
#[test]
fn a_lookup_past_format_6s_header_still_reads_back() {
    let entries: Vec<(u16, u16)> = (0..2_000u16).flat_map(|r| (r * 12..r * 12 + 10).map(|g| (g, g))).collect();
    assert!(format6(&entries).is_none(), "format 6 wrote a searchRange past 16 bits");
    let table = build_aat_lookup(&entries).expect("a lookup");
    assert_eq!(table[1], 4);
    read_back(&table, &entries);
}

// Each format ends with the 0xFFFF unit that ends a search, counted among its units: two entries make 3 units of 4
// bytes in format 6, searchRange 8, entrySelector 1 and rangeShift 4.
#[test]
fn segments_end_with_the_terminator_the_spec_requires() {
    let table = format6(&[(3, 9), (5, 1)]).expect("format 6");
    assert_eq!(&table[table.len() - 4..], [0xFF, 0xFF, 0, 0]);
    assert_eq!(&table[2..12], [0, 4, 0, 3, 0, 8, 0, 1, 0, 4]);
    let table = format2(&[(3, 9), (4, 9)]).expect("format 2");
    assert_eq!(&table[2..12], [0, 6, 0, 2, 0, 12, 0, 1, 0, 0]);
    assert_eq!(&table[table.len() - 6..], [0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
}

#[test]
fn an_offset_lookup_is_sized_by_its_glyphs_alone() {
    let glyphs: Vec<(u16, u16)> = (0..50).map(|i| (i * 2, 0)).collect();
    let valued: Vec<(u16, u16)> = glyphs.iter().map(|&(g, _)| (g, 1_000 + g)).collect();
    assert_eq!(build_aat_offset_lookup(&glyphs).map(|t| t.len()), build_aat_offset_lookup(&valued).map(|t| t.len()));
}
