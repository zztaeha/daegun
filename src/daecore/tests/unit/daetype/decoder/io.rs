use super::*;

// [1, 2] and [1, 2, 0] share a fingerprint, as a record's last word is padded with zeros, so only their
// bytes tell them apart. A thousand more grow the index several times over.
#[test]
fn a_pool_places_each_distinct_record_once() {
    let print = |record: &[u8]| {
        let mut print = Fingerprint::default();
        print.mix(record);
        print.finish()
    };
    assert_eq!(print(&[1, 2]), print(&[1, 2, 0]));
    let mut pool = Placed::new(4);
    let placed = [pool.place(&[1, 2]), pool.place(&[1, 2, 0]), pool.place(&[1, 2]), pool.place(&[1, 2, 0])];
    assert_eq!(placed, [Some(4), Some(6), Some(4), Some(6)]);
    let records: Vec<[u8; 2]> = (0x1000..0x1000 + 1_000u16).map(u16::to_be_bytes).collect();
    for _ in 0..2 {
        let at: Vec<Option<u16>> = records.iter().map(|r| pool.place(r)).collect();
        assert!(at.iter().zip(0u16..).all(|(&at, i)| at == Some(9 + 2 * i)), "a record moved");
    }
    assert_eq!(pool.bytes.len(), 5 + 2 * records.len());

    let mut full = Placed::new(0xFFFF);
    assert_eq!([full.place(&[1]), full.place(&[2]), full.place(&[1])], [Some(0xFFFF), None, Some(0xFFFF)]);
}

#[test]
fn an_offset24_past_24_bits_writes_nothing() {
    let mut data = [9u8; 4];
    write_offset24(&mut data, 0, 0x100_0000);
    assert_eq!(data, [9; 4]);
    write_offset24(&mut data, 1, 0xFF_FFFF);
    assert_eq!(data, [9, 0xFF, 0xFF, 0xFF]);
}

// 2,000 glyphs naming one 1 MB record: placed again from where it lies, it's found without hashing its bytes again.
#[test]
fn a_record_placed_again_from_where_it_lies_is_not_read_again() {
    let record = alloc::vec![7u8; 1 << 20];
    let mut pool = Placed::new(0);
    let started = std::time::Instant::now();
    assert!((0..2_000).all(|_| pool.place(&record) == Some(0)));
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    assert_eq!(pool.bytes.len(), 1 << 20);
}
