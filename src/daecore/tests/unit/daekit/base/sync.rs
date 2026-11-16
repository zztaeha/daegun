use super::*;

// A table given back comes out again emptied, glyph by glyph or, written past its list, whole; one of another count
// is laid out new; and past one location's worth, a table given back is dropped.
#[test]
fn spare_slots_come_back_emptied() {
    let spare = SpareSlots::default();
    let table = spare.take(3200);
    assert_eq!((table.len(), table.heap_bytes()), (3200, 4 * (1 + LISTED + 3200)));
    for gid in [7, 411, 3199, 411] {
        table.set(gid, gid + 1);
    }
    table.set(3200, 9);
    assert_eq!((table.get(411), table.get(3200), table.get(8)), (412, 0, 0));
    spare.give(table);
    let again = spare.take(3200);
    assert!((0..3200).all(|gid| again.get(gid) == 0), "a table came back holding what it was given with");
    assert_eq!(read(&spare.0).len(), 0);
    for gid in 0..LISTED as u32 + 1 {
        again.set(gid * 10, 1);
    }
    spare.give(again);
    let whole = spare.take(3200);
    assert!((0..3200).all(|gid| whole.get(gid) == 0), "a table written past its list came back holding some");
    assert_eq!(spare.take(10).len(), 10);
    for _ in 0..SPARES + 2 {
        spare.give(GlyphSlots::new(5));
    }
    assert_eq!(read(&spare.0).len(), SPARES);
}

// A spare table isn't waited for: where another thread holds the list, a table is made new and one given back dropped.
#[cfg(feature = "threading")]
#[test]
fn a_busy_list_of_spare_tables_is_passed_over() {
    let spare = &SpareSlots::default();
    spare.give(GlyphSlots::new(32));
    let held = write(&spare.0);
    let (tell, told) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(move || {
            let table = spare.take(32);
            spare.give(table);
            tell.send(()).expect("the test is waiting");
        });
        let past = told.recv_timeout(std::time::Duration::from_secs(20));
        drop(held);
        past.expect("a thread waited for the list");
    });
    assert_eq!(read(&spare.0).len(), 1, "a table was taken or given past the list another thread held");
}

// A draw takes all it asks or nothing, so threads racing for the last of an allowance never take more than it held,
// and what they give back leaves it where it began.
#[test]
fn an_allowance_is_drawn_whole_or_not_at_all() {
    let left = Counter::new(100);
    assert!(!left.draw(101));
    assert!(left.draw(60));
    assert_eq!(left.get(), 40);
    // Miri's schedules come from its seeds, which the gate runs many of, so a few rounds each.
    #[cfg(feature = "threading")]
    for _ in 0..if cfg!(miri) { 10 } else { 1000 } {
        let left = Counter::new(100);
        let start = std::sync::Barrier::new(8);
        let race = || {
            start.wait();
            if left.draw(30) { 30 } else { 0 }
        };
        let drawn: usize = std::thread::scope(|s| {
            let racers: alloc::vec::Vec<_> = (0..8).map(|_| s.spawn(race)).collect();
            racers.into_iter().map(|r| r.join().unwrap_or(0)).sum()
        });
        assert_eq!((drawn, left.get()), (90, 10), "racing draws took more than the allowance held");
        left.add(drawn);
        assert_eq!(left.get(), 100);
    }
}
