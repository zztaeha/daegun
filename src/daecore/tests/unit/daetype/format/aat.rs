use super::*;
use alloc::vec;

fn words(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

// A binary-searched lookup of the given format with `units` of `size` words each.
fn searched(format: u16, units: &[&[u16]], tail: &[u16]) -> Vec<u8> {
    let size = units.first().map_or(0, |u| u.len() as u16 * 2);
    let mut t = words(&[format, size, units.len() as u16, 0, 0, 0]);
    units.iter().for_each(|u| t.extend(words(u)));
    t.extend(words(tail));
    t
}

// Glyphs read in any order into bits make the set their single-glyph ranges make, bits or ranges alike.
#[test]
fn a_set_of_bits_holds_what_its_ranges_hold() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % u64::from(n)) as u32
    };
    for round in 0..300 {
        let (count, spread) = ([0, 1, 40, 900][round % 4], [70, 3_000, 65_536][round % 3]);
        let last = [65_535].into_iter().take(round % 5 / 4);
        let glyphs: Vec<u16> = (0..count).map(|_| next(spread) as u16).chain(last).collect();
        let mut bits = super::super::index::GlyphBits::new();
        glyphs.iter().for_each(|&g| bits.insert(g.into()));
        let ranges = GlyphRanges::new(glyphs.iter().map(|&g| (u32::from(g), u32::from(g))).collect());
        assert_eq!(GlyphRanges::of_bits(&bits), ranges, "round {round}");
        assert_eq!(ranges.heap_bytes(), GlyphRanges::of_bits(&bits).heap_bytes());
    }
}

#[test]
fn overlapping_segments_list_each_glyph_once() {
    let segments = vec![&[0xFFFE, 0, 1][..]; 50];
    let bytes = searched(2, &segments, &[]);
    let lookup = Lookup::parse(&bytes, 0xFFFF).unwrap();
    let entries = lookup.pairs();
    assert_eq!(entries.len(), 0xFFFF, "every glyph below 0xFFFF once");
    assert!(entries.iter().all(|&(g, v)| lookup.value(g.into()) == Some(v)));
}

// Segments for 3, 1 and 2, then a terminator: the binary search sorted segments call for finds 1 and 2 but not 3,
// its first midpoint, 1's segment, sending a search for 3 right. Every list of the lookup's glyphs agrees with it.
#[test]
fn units_out_of_order_answer_only_what_the_search_finds() {
    let bytes = searched(2, &[&[3, 3, 6], &[1, 1, 4], &[2, 2, 5], &[0xFFFF, 0xFFFF, 0]], &[]);
    let lookup = Lookup::parse(&bytes, 10).unwrap();
    assert_eq!([1, 2, 3].map(|g| lookup.value(g)), [Some(4), Some(5), None]);
    assert!(lookup.glyph_set().glyphs().eq([1, 2]));
    let mut sixes = Vec::new();
    lookup.collect_where(&mut sixes, &|v| v == 6);
    assert!(sixes.is_empty());
    assert_eq!(lookup.pairs(), [(1, 4), (2, 5)]);
    assert_eq!(lookup.entries(), [GlyphValue { glyph: 1, value: 4 }, GlyphValue { glyph: 2, value: 5 }]);
}

#[test]
fn a_segment_ending_at_ffff_lists_the_glyphs_value_answers_for() {
    let bytes = searched(2, &[&[0xFFFF, 0xFFF0, 7], &[0xFFFF, 0xFFFF, 0]], &[]);
    let lookup = Lookup::parse(&bytes, 0xFFFF).unwrap();
    assert_eq!(lookup.value(0xFFF5), Some(7));
    assert_eq!(lookup.pairs(), (0xFFF0..=0xFFFE).map(|g| (g, 7)).collect::<Vec<_>>());
}

#[test]
fn every_lookup_format_reads_its_values() {
    let simple = words(&[0, 10, 11, 12]);
    let simple = Lookup::parse(&simple, 3).unwrap();
    assert_eq!((0..4).map(|g| simple.value(g)).collect::<Vec<_>>(), [Some(10), Some(11), Some(12), None]);
    assert_eq!(simple.pairs(), [(0, 10), (1, 11), (2, 12)]);

    let single = searched(2, &[&[5, 3, 0x11], &[9, 8, 0x22], &[0xFFFF, 0xFFFF, 0]], &[]);
    let single = Lookup::parse(&single, 20).unwrap();
    let want = [None, Some(0x11), Some(0x11), None, Some(0x22), Some(0x22), None];
    assert_eq!([2, 3, 5, 7, 8, 9, 10].map(|g| single.value(g)), want);
    assert_eq!(single.pairs(), [(3, 0x11), (4, 0x11), (5, 0x11), (8, 0x22), (9, 0x22)]);

    let array = searched(4, &[&[5, 3, 30], &[8, 8, 36], &[0xFFFF, 0xFFFF, 0]], &[0x31, 0x32, 0x33, 0x41]);
    let array = Lookup::parse(&array, 20).unwrap();
    assert_eq!(array.pairs(), [(3, 0x31), (4, 0x32), (5, 0x33), (8, 0x41)]);

    let table = searched(6, &[&[2, 0x52], &[7, 0x57], &[0xFFFF, 0]], &[]);
    assert_eq!(Lookup::parse(&table, 20).unwrap().pairs(), [(2, 0x52), (7, 0x57)]);

    let trimmed = words(&[8, 4, 2, 0x84, 0x85]);
    let trimmed = Lookup::parse(&trimmed, 20).unwrap();
    assert_eq!(trimmed.pairs(), [(4, 0x84), (5, 0x85)]);

    let mut wide = words(&[10, 1, 6, 3]);
    wide.extend([0xA6, 0xA7, 0xA8]);
    assert_eq!(Lookup::parse(&wide, 20).unwrap().pairs(), [(6, 0xA6), (7, 0xA7), (8, 0xA8)]);
    let mut long = words(&[10, 4, 6, 1]);
    long.extend([0, 0, 0xB0, 0x0B]);
    assert_eq!(Lookup::parse(&long, 20).unwrap().value(6), Some(0xB00B));
}

// Format 10's units are 1, 2, 4 or 8 bytes, as Apple's manual allows, each value read whole: one too wide for the
// table's 16 or 32 bits is none, as is every value of a unit of another size.
#[test]
fn format_10_reads_the_unit_sizes_the_manual_allows() {
    let lookup = |unit: u16, value: &[u8]| {
        let t = [words(&[10, unit, 6, 1]), value.to_vec()].concat();
        let l = Lookup::parse(&t, 20).unwrap();
        (l.value(6), l.value_u32(6), l.typed_value(6), l.pairs())
    };
    assert_eq!(lookup(1, &[7]), (Some(7), Some(7), Some(7), alloc::vec![(6, 7)]));
    assert_eq!(lookup(8, &[0, 0, 0, 0, 0, 0, 0xB0, 0x0B]).0, Some(0xB00B));
    assert_eq!(lookup(4, &[0, 1, 0xB0, 0x0B]), (None, Some(0x1_B00B), None, alloc::vec![]));
    assert_eq!(lookup(8, &[0, 0, 0, 1, 0, 0, 0, 0]).1, None, "past 32 bits");
    assert_eq!(lookup(3, &[0, 0xB0, 0x0B]), (None, None, None, alloc::vec![]));
    assert_eq!(lookup(0, &[]), (None, None, None, alloc::vec![]));
}

// Merged ranges held as bits where those take no more room, and as ranges where they would take more, the deletion
// sentinel's far range among them: both hold exactly the ranges' glyphs.
#[test]
fn a_glyph_set_holds_its_ranges_glyphs_as_bits_or_as_ranges() {
    let sets: [&[(u32, u32)]; 4] =
        [&[(20, 30), (0, 10), (5, 12)], &[(3, 5), (70, 70), (200, 260)], &[(5, 5), (0xFFFF, 0xFFFF)], &[(9, 9)]];
    for ranges in sets {
        let set = GlyphRanges::new(ranges.to_vec());
        let held = |g: u32| ranges.iter().any(|&(a, b)| (a..=b).contains(&g));
        assert!((0..=0x1_0000).all(|g| set.contains(g) == held(g)));
        assert!(set.glyphs().eq((0..=0x1_0000).filter(|&g| held(g))));
    }
    assert!(matches!(GlyphRanges::new(sets[0].to_vec()).0, Held::Bits { .. }));
    assert!(matches!(GlyphRanges::new(sets[1].to_vec()).0, Held::Ranges(_)));
}

// Each searched format, the first with its units out of order, behind a state table of one row: the index answers
// all 65,536 glyphs as the search does.
#[test]
fn a_class_index_answers_every_glyph_as_the_search_does() {
    let lookups = [
        searched(2, &[&[3, 3, 6], &[1, 1, 4], &[2, 2, 5], &[0xFFFF, 0xFFFF, 0]], &[]),
        searched(4, &[&[5, 3, 30], &[8, 8, 36], &[0xFFFF, 0xFFFF, 0]], &[7, 8, 9, 3]),
        searched(6, &[&[2, 5], &[7, 9], &[0xFFFF, 0]], &[]),
    ];
    for lookup in lookups {
        let state = 16 + lookup.len() as u32;
        let mut t: Vec<u8> = [10, 16, state, state + 20].iter().flat_map(|v: &u32| v.to_be_bytes()).collect();
        t.extend(&lookup);
        t.extend([0; 24]);
        let table = StateTable::parse(&t, 0, 20).unwrap();
        let entries = table.searched_classes().unwrap();
        let each = |e: &mut crate::daecore::daetype::format::index::Entries| {
            entries.iter().for_each(|&(g, k)| e.put(g.into(), k.into()));
            Some(())
        };
        let index = SparseIndex::build(&each, class::OUT_OF_BOUNDS, usize::MAX).unwrap();
        let indexed = StateTable::parse(&t, 0, 20).unwrap().with_classes(Some(&index));
        assert!((0..=0xFFFF).all(|g| table.class(g) == indexed.class(g)));
    }
}

#[test]
fn a_state_entry_reads_both_its_extra_words() {
    let mut t = vec![0, 0, 0, 5, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0, 44];
    t.extend(words(&[8, 4, 1, 4]));
    t.extend(words(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0]));
    t.extend(words(&[0, 0, 0, 0, 1, 0x4000, 0x0111, 0x0222]));
    let table = StateTable::parse(&t, 2, 10).unwrap();
    assert_eq!(table.class(4), 4);
    let e = table.entry(0, 4).unwrap();
    assert_eq!((e.new_state, e.flags, e.word1, e.word2), (1, 0x4000, 0x0111, 0x0222));
}

// Byte classes for glyphs 3 and 4, byte cells, and new states as byte offsets: 19 to the state array, row 0,
// and 14 to the row stored before it, row -1, which reads nothing.
#[test]
fn a_16_bit_state_table_reads_byte_classes_and_offset_states() {
    let mut t = words(&[5, 8, 19, 24, 3, 2]);
    t.extend([4, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0]);
    t.extend(words(&[19, 0, 14, 0x8000]));
    let table = ObsoleteStateTable::parse(&t, 0).unwrap();
    assert_eq!([2, 3, 4, 5, 0xFFFF].map(|g| table.class(g)), [1, 4, 1, 1, 2]);
    let mut listed = Vec::new();
    table.collect_classes_where(&mut listed, &|k| k != u32::from(class::OUT_OF_BOUNDS));
    assert_eq!(listed, [(3, 3)]);

    let start = table.entry_at(0, 4).unwrap();
    assert_eq!(table.next_state(&start), 0);
    assert_eq!(table.next_state(&Entry { new_state: 14, ..Entry::default() }), -1);
    assert!(table.entry_at(-1, 4).is_none());
    assert!(table.entry_at(0, 5).is_none());
}
