use super::*;
use alloc::vec;

// A one-axis store whose first data offset is NULL and whose second holds one delta of 100, with
// other table bytes after it that a NULL read as an offset would land among.
fn store_with_null_data() -> Vec<u8> {
    let mut t = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 0, 0, 0, 0, 0, 0, 26];
    t.extend([0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0]);
    t.extend([0, 1, 0, 0, 0, 1, 0, 0, 100]);
    t.extend([0x11; 40]);
    t
}

#[test]
fn a_null_data_offset_has_no_variation() {
    let store = ItemVariationStore::parse(&store_with_null_data(), 0).expect("parses");
    let scalars = store.region_scalars(&[1.0]);
    let index = |outer| DeltaSetIndex { outer, inner: 0 };
    assert_eq!(store.delta(index(0), &scalars), 0.0);
    assert_eq!(store.delta(index(1), &scalars), 100.0);
}

#[test]
fn a_store_or_map_of_an_unknown_format_is_refused() {
    let mut t = store_with_null_data();
    t[1] = 2;
    assert!(ItemVariationStore::parse(&t, 0).is_err());
    assert!(parse_delta_set_index_map(&[7, 0, 0, 0, 0, 1, 0x40], 0).is_err());
}

#[test]
fn an_empty_delta_set_index_map_maps_nothing() {
    let map = parse_delta_set_index_map(&[0, 0, 0, 0], 0).expect("parses");
    assert!(map.is_empty());
    assert_eq!(delta_set_index_map_lookup(&map, 5), DeltaSetIndex { outer: 0, inner: 5 });
    let one = parse_delta_set_index_map(&[0, 0x17, 0, 1, 0x01, 0x02], 0).expect("parses");
    let first = DeltaSetIndex { outer: 1, inner: 2 };
    assert_eq!([0, 9].map(|i| delta_set_index_map_lookup(&one, i)), [first, first]);
}

// The store above with a map after it: an empty map leaves the table's own mapping, and past a
// map's end its last entry answers.
#[test]
fn a_delta_source_with_an_empty_map_takes_the_tables_own_mapping() {
    let with_map = |map: &[u8]| [store_with_null_data(), map.to_vec()].concat();
    let map_at = store_with_null_data().len();
    let empty = with_map(&[0, 0, 0, 0]);
    let source = DeltaSource::read(&empty, 0, Some(map_at)).expect("reads");
    let scalars = source.scalars(&[1.0]);
    assert_eq!(source.delta(5, (1, 0), &scalars), 100 << 16);
    let one = with_map(&[0, 0x10, 0, 1, 0, 2]);
    let source = DeltaSource::read(&one, 0, Some(map_at)).expect("reads");
    assert_eq!([0, 9].map(|i| source.delta(i, (0, 0), &scalars)), [100 << 16; 2]);
}

fn var_tables(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
    for path in std::fs::read_dir(dir).expect("a fixture directory").flatten().map(|e| e.path()) {
        if path.is_dir() {
            var_tables(&path, out);
        } else if path.extension().is_some_and(|e| e == "ttf" || e == "otf") {
            let bytes = std::fs::read(&path).expect("a fixture");
            let Ok(map) = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes) else { continue };
            out.extend(["HVAR", "VVAR"].iter().filter_map(|tag| map.get(tag)).map(|t| t.to_vec()));
        }
    }
}

// Read in place under each mapping the table names, or none, as `DeltaSource` reads it: both fail, or both answer
// alike to the bit for the indices `sampled` keeps.
fn reads_alike(table: &[u8], sampled: impl Fn(usize) -> bool) {
    let at = |field| read_u32_be(table, field).map(|v| v as usize);
    let Some(store_at) = at(4) else { return };
    let bytes = TableBytes::from_vec(table.to_vec());
    let named = [8, 12, 16, 20].into_iter().filter_map(at).filter(|&m| m != 0).map(Some);
    for map in core::iter::once(None).chain(named) {
        let (view, source) = (DeltaView::read(&bytes, store_at, map), DeltaSource::read(table, store_at, map));
        assert_eq!(view.is_some(), source.is_ok(), "map {map:?}");
        let (Some(view), Ok(source)) = (view, source) else { continue };
        assert_eq!(view.extent(), source.extent());
        // Each decoded value as the in-place reader reads it, a row past the table 0.
        let buf = view.store.table.as_slice();
        for (set, ivd) in view.store.layout.subtables.iter().zip(&source.store.ivd_data) {
            for inner in 0..set.items {
                let row = set.row(buf, inner);
                let values: Vec<i32> = (0..set.regions).map(|j| row.map_or(0, |r| set.value(r, j))).collect();
                assert_eq!(ivd.row(inner), Some(&values[..]), "map {map:?} row {inner}");
            }
        }
        for location in [&[0.0][..], &[1.0], &[-1.0], &[0.3, -0.7, 1.0], &[0.61, 0.5, -0.25, 0.9]] {
            let (in_place, decoded) = (view.scalars(location), source.scalars(location));
            assert_eq!(in_place, decoded);
            for index in (0..view.extent() + 3).filter(|&i| sampled(i)) {
                let implicit = (index % 3, index);
                let pair = [view.delta(index, implicit, &in_place), source.delta(index, implicit, &decoded)];
                assert_eq!(pair[0], pair[1], "map {map:?} index {index} at {location:?}");
            }
        }
    }
}

// Every HVAR and VVAR among the fixtures, whole, cut short and with a byte changed, early bytes changed most.
#[test]
fn a_store_read_in_place_answers_as_the_decoded_one() {
    let mut tables = Vec::new();
    var_tables(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts")), &mut tables);
    assert!(tables.len() > 8, "the fixtures are missing");
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below.max(1) as u64) as usize
    };
    // Repeated offsets to one subtable of 65,535 rows: of no regions, 15 fit the row budget; of one region, 4 fit the
    // values budget. A set past them is absent, the store still read.
    let repeated = |copies: u16, regions: u16| {
        let regions_at = 8 + 4 * u32::from(copies);
        let mut t: Vec<u8> = [&[0, 1][..], &regions_at.to_be_bytes(), &copies.to_be_bytes()].concat();
        (0..copies).for_each(|_| t.extend((regions_at + 4).to_be_bytes()));
        t.extend([0, 0, 0, 0, 0xFF, 0xFF, 0, 0]);
        t.extend(regions.to_be_bytes());
        t.extend(vec![0; usize::from(regions) * (2 + 65_535)]);
        t
    };
    for (copies, regions, present) in [(15, 0, 15), (16, 0, 15), (3, 1, 3), (5, 1, 4)] {
        let header = [0, 1, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let table = [&header[..], &repeated(copies, regions)].concat();
        let store = ItemVariationStore::parse(&table, 20).expect("reads");
        let read = store.ivd_data.iter().filter(|d| d.rows() > 0).count();
        assert_eq!(read, present, "{copies} copies of {regions} regions");
        reads_alike(&table, |i| i < 64);
    }
    for table in &tables {
        reads_alike(table, |_| true);
        for _ in 0..40 {
            reads_alike(&table[..next(table.len())], |i| i < 64 || i % 97 == 0);
        }
        for round in 0..120 {
            let mut changed = table.clone();
            let at = next(if round % 2 == 0 { table.len().min(96) } else { table.len() });
            changed[at] = next(256) as u8;
            reads_alike(&changed, |i| i < 64 || i % 97 == 0);
        }
    }
}

// Two regions on one axis, peaking at 1.0 and 0.5: set 0's rows a word for region 1, then a byte for region 0 (256 and
// 10, -256 and -20), set 1's a byte for region 0 (7); then, at 57, a map of `size`-byte entries, inner 1 bit wide.
fn store_and_map(entries: &[u32], size: usize) -> Vec<u8> {
    let mut t = vec![0u8, 1, 0, 0, 0, 16, 0, 2, 0, 0, 0, 32, 0, 0, 0, 48];
    t.extend([0, 1, 0, 2, 0, 0, 0x40, 0, 0x40, 0, 0, 0, 0x20, 0, 0x40, 0]);
    t.extend([0, 2, 0, 1, 0, 2, 0, 1, 0, 0, 1, 0, 10, 0xFF, 0, 0xEC]);
    t.extend([0, 1, 0, 0, 0, 1, 0, 0, 7]);
    t.extend([0, (size as u8 - 1) << 4]);
    t.extend((entries.len() as u16).to_be_bytes());
    entries.iter().for_each(|e| t.extend(&e.to_be_bytes()[4 - size..]));
    t
}

// Each index's delta at 0.5, rounded once, in place and decoded: through the map at 57 where `mapped`, else as
// (outer, inner) pairs packed in 16 bits each.
fn deltas_at_half(t: &[u8], mapped: bool, indices: &[usize]) -> Vec<i32> {
    let map = mapped.then_some(57);
    let view = DeltaView::read(&TableBytes::from_vec(t.to_vec()), 0, map).expect("a store");
    let source = DeltaSource::read(t, 0, map).expect("a store");
    let (in_place, decoded) = (view.scalars_at(&[ONE / 2]), source.scalars(&[0.5]));
    let read = |i: usize| {
        let implicit = (i >> 16, i & 0xFFFF);
        let (a, b) = (view.delta(i, implicit, &in_place), source.delta(i, implicit, &decoded));
        assert_eq!(a, b, "index {i:#x} in place and decoded");
        round_fixed(a)
    };
    indices.iter().map(|&i| read(i)).collect()
}

// At 0.5 region 0 weighs a half and region 1 whole: each item's delta sums its columns by their own regions, rounded
// once; an item past its set, or all 1s, varies nothing.
#[test]
fn a_store_reads_each_items_delta() {
    let items = [0, 1, 0x1_0000, 0x1_0001, 0x2_0000, 0xFFFF_FFFF];
    assert_eq!(deltas_at_half(&store_and_map(&[], 4), false, &items), [261, -266, 4, 0, 0, 0]);
}

// A map's entries one to four bytes wide read alike, a position past it taking its last entry; an entry naming no row,
// past its set's items or past the sets, all 1s included, varies nothing.
#[test]
fn a_map_entry_naming_no_row_varies_nothing() {
    for size in 1..=4 {
        let entries = [1, 2, 0, 3, u32::MAX >> (32 - 8 * size)];
        let got = deltas_at_half(&store_and_map(&entries, size), true, &[0, 1, 2, 3, 4, 9]);
        assert_eq!(got, [-266, 4, 261, 0, 0, 0], "{size}-byte entries");
    }
}

// A failing part of a store is absent, not the store, in place and decoded alike: a row cut by the table's end, a set
// past it or with more word deltas than regions, a column past the region list, a region list past the table.
#[test]
fn a_store_part_that_fails_is_absent() {
    let t = store_and_map(&[], 4);
    let with = |at: usize, bytes: &[u8]| {
        let mut t = t.clone();
        t[at..at + bytes.len()].copy_from_slice(bytes);
        t
    };
    let items = [0, 0x1_0000];
    assert_eq!(deltas_at_half(&t[..56], false, &items), [261, 0], "a row cut off");
    assert_eq!(deltas_at_half(&with(12, &[0, 0, 0x10, 0]), false, &items), [261, 0], "a set past the table");
    assert_eq!(deltas_at_half(&with(50, &[0, 2]), false, &items), [261, 0], "more word deltas than regions");
    assert_eq!(deltas_at_half(&with(54, &[0, 2]), false, &items), [261, 0], "a region past the list");
    assert_eq!(deltas_at_half(&with(2, &[0, 0, 0x10, 0]), false, &items), [0, 0], "the region list past the table");
}

// A store on one axis of `regions` regions peaking at 1.0 and one data set of one row, each delta 1, from byte
// `header` of a table whose map at `map_at`, the store's end, names that row for each of `glyphs` glyphs.
fn one_row_for_every_glyph(header: usize, regions: usize, glyphs: usize) -> (Vec<u8>, usize) {
    let data_at = 12 + 4 + 6 * regions;
    let mut store = [&[0, 1, 0, 0, 0, 12, 0, 1][..], &(data_at as u32).to_be_bytes(), &1u16.to_be_bytes()].concat();
    store.extend((regions as u16).to_be_bytes());
    (0..regions).for_each(|_| store.extend([0, 0, 0x40, 0, 0x40, 0]));
    store.extend([0, 1, 0, 0]);
    store.extend((regions as u16).to_be_bytes());
    (0..regions as u16).for_each(|r| store.extend(r.to_be_bytes()));
    store.extend(vec![1u8; regions]);
    let mut t = vec![0, 1, 0, 0];
    t.extend((header as u32).to_be_bytes());
    t.resize(header, 0);
    t.extend(store);
    let map_at = t.len();
    t.extend([0, 0]);
    t.extend((glyphs as u16).to_be_bytes());
    t.extend(vec![0u8; glyphs]);
    (t, map_at)
}

fn rows_summed(pass: impl FnOnce()) -> usize {
    ROWS_SUMMED.try_with(|n| n.set(0)).expect("a counter");
    pass();
    ROWS_SUMMED.try_with(core::cell::Cell::get).expect("a counter")
}

// One row of 100 regions the map names for each of 1,000 glyphs: a pass over them, through either reader, sums it
// once and gives every glyph its 100 units.
#[test]
fn a_pass_over_the_glyphs_sums_each_long_row_once() {
    let (t, map_at) = one_row_for_every_glyph(20, 100, 1_000);
    let source = DeltaSource::read(&t, 20, Some(map_at)).expect("reads");
    let scalars = source.scalars(&[1.0]);
    let mut delta = source.glyph_deltas(&scalars);
    assert_eq!(rows_summed(|| assert!((0..1_000).all(|gid| delta(gid) == 100 << 16))), 1);
    let view = DeltaView::read(&TableBytes::from_vec(t), 20, Some(map_at)).expect("reads");
    let scalars = view.scalars(&[1.0]);
    let mut delta = view.glyph_deltas(&scalars);
    assert_eq!(rows_summed(|| assert!((0..1_000).all(|gid| delta(gid) == 100 << 16))), 1);
}
