use super::*;

// A 16-bit offset to a child 70,004 bytes in fails the table rather than wrapping; at 60,004 it fits.
#[test]
fn an_offset_past_its_width_fails_the_table() {
    let table = |n: u32| {
        let glyphs = |first: u32| Rc::new(Value::Glyphs(Width::W16, (first..first + n).collect()));
        Value::Struct(alloc::vec![
            Value::Offset(OffsetWidth::W16, Some(glyphs(0))),
            Value::Offset(OffsetWidth::W16, Some(glyphs(n))),
        ])
    };
    let mut work = Work::for_table(0);
    assert!(generic_build(&table(30_000), &mut work).is_some());
    assert!(generic_build(&table(35_000), &mut work).is_none());
}

// Tables crafted to one fingerprint are told apart by their bytes, past the probe limit too.
#[test]
fn tables_of_one_fingerprint_are_never_confused() {
    let table = |n: u16| Built { hash: 0, bytes: n.to_be_bytes().to_vec(), devices: Vec::new() };
    let mut placed = Placed::default();
    for n in 0..100 {
        if placed.get(&table(n)).is_none() {
            placed.insert(Rc::new(table(n)), usize::from(n));
        }
    }
    for n in 0..100 {
        let got = placed.get(&table(n));
        assert!(got.is_none() || got == Some(usize::from(n)), "table {n} found at {got:?}");
    }
    assert_eq!(placed.get(&table(0)), Some(0));
}
