use super::*;
use crate::daecore::daetype::TableBytes;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// STAT 1.1 with one axis record at 20 and one value offset at 28, 30 bytes: read at major version 1 with its header,
// whatever lies past the table, which its readers find absent.
#[test]
fn has_data_takes_major_1_with_its_header() {
    let stat = words(&[1, 1, 8, 1, 0, 20, 1, 0, 28, 2, 0x7767, 0x6874, 256, 0, 0]);
    assert!(has_data(&stat));
    let changed = |at: usize, v: u16| {
        let mut t = stat.clone();
        t[at..at + 2].copy_from_slice(&v.to_be_bytes());
        has_data(&t)
    };
    assert!(changed(2, 0), "version 1.0");
    assert!(!changed(0, 2), "version 2.1");
    assert!(changed(6, 2) && changed(12, 2), "axis records or value offsets past the end");
    assert!(!has_data(&stat[..19]), "a 1.1 header cut short");
    assert!(has_data(&[&words(&[1, 0])[..], &stat[4..18]].concat()), "1.0's 18 bytes");
}

// Mac Roman names by ID.
fn name_table(names: &[(u16, &[u8])]) -> Vec<u8> {
    let mut t = words(&[0, names.len() as u16, 6 + 12 * names.len() as u16]);
    let mut storage: Vec<u8> = Vec::new();
    for (id, s) in names {
        t.extend(words(&[1, 0, 0, *id, s.len() as u16, storage.len() as u16]));
        storage.extend(*s);
    }
    t.extend(storage);
    t
}

// STAT 1.2 over the axes `axes` (tag and name ID), its axis value offsets `offsets` counted from
// their own start and the value tables `tables` after them.
fn tables(
    names: Vec<u8>, axes: &[(&[u8; 4], u16)], offsets: &[u16], tables: &[u8],
) -> Tables {
    let values_at = 20 + 8 * axes.len() as u32;
    let mut stat = words(&[1, 2, 8, axes.len() as u16]);
    stat.extend(20u32.to_be_bytes());
    stat.extend(words(&[offsets.len() as u16]));
    stat.extend(values_at.to_be_bytes());
    stat.extend(words(&[0]));
    for (i, (tag, name)) in axes.iter().enumerate() {
        stat.extend(*tag);
        stat.extend(words(&[*name, i as u16]));
    }
    stat.extend(offsets.iter().flat_map(|o| o.to_be_bytes()));
    stat.extend(tables);
    [("name", names), ("STAT", stat)].into_iter().map(|(t, b)| (String::from(t), TableBytes::from(b))).collect()
}

// A format 1 value on `axis` with `flags`, named 257, at weight 700.
fn single(axis: u16, flags: u16) -> Vec<u8> {
    words(&[1, axis, flags, 257, 700, 0])
}

// 1,024 values all naming one 65,535-byte name, 196,605 bytes once decoded: copied for each, they
// would hold 201 MB, from a 2 KB STAT.
#[test]
fn a_long_name_named_by_every_value_is_not_copied_past_a_bound() {
    let long = alloc::vec![0xA0u8; 65_535];
    let value = words(&[1, 0, 0, 256, 700, 0]);
    let map = tables(name_table(&[(256, &long)]), &[(b"wght", 256)], &[2048; 1024], &value);
    let (axes, values, _) = parse_stat(&map).expect("a STAT");
    let value_name = |v: &StatAxisValue| match v { StatAxisValue::Single { name, .. } => name.clone(), _ => None };
    let held: usize = axes.iter().filter_map(|a| a.name.as_ref()).map(String::len).sum::<usize>()
        + values.iter().filter_map(value_name).map(|n| n.len()).sum::<usize>();
    let bound = 16 * map.get("STAT").map_or(0, |t| t.len()) + 196_605;
    assert!(held <= bound, "held {held} bytes of names, past {bound}");
    assert_eq!(values.len(), 1024);
}

// One design axis: a value on axis 7, or a combination naming it, is left out rather than handed
// to a caller who would index the axes with it.
#[test]
fn a_value_on_no_axis_is_left_out() {
    let names = name_table(&[(256, b"Weight"), (257, b"Bold")]);
    let combo = words(&[4, 2, 0, 257, 0, 700, 0, 7, 1, 0]);
    let map = tables(names, &[(b"wght", 256)], &[6, 18, 30], &[single(0, 0), single(7, 0), combo].concat());
    let (_, values, _) = parse_stat(&map).expect("a STAT");
    assert_eq!(values.len(), 1);
    assert!(matches!(values[0], StatAxisValue::Single { axis_index: 0, .. }));
}

#[test]
fn values_describing_older_siblings_say_so() {
    let names = name_table(&[(256, b"Weight"), (257, b"Bold")]);
    let map = tables(names, &[(b"wght", 256)], &[4, 16], &[single(0, 0x0001), single(0, 0x0002)].concat());
    let (_, values, _) = parse_stat(&map).expect("a STAT");
    let flags: Vec<_> = values
        .iter()
        .map(|v| match v {
            StatAxisValue::Single { elidable, older_sibling, .. } => (*elidable, *older_sibling),
            _ => (true, true),
        })
        .collect();
    assert_eq!(flags, [(false, true), (true, false)]);
}
