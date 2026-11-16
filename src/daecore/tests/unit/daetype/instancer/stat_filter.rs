use super::*;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// STAT 1.1 over the axes `tags`, each record `size` bytes, then format 1 values (axis, value).
fn stat(tags: &[&[u8; 4]], size: u16, values: &[(u16, u16)]) -> Vec<u8> {
    let offsets_at = 20 + u32::from(size) * tags.len() as u32;
    let mut t = words(&[1, 1, size, tags.len() as u16]);
    t.extend(20u32.to_be_bytes());
    t.extend(words(&[values.len() as u16]));
    t.extend(offsets_at.to_be_bytes());
    t.extend(words(&[2]));
    for (i, tag) in tags.iter().enumerate() {
        t.extend(*tag);
        t.extend(words(&[256 + i as u16, i as u16]));
    }
    t.extend(words(&(0..values.len()).map(|i| (2 * values.len() + 12 * i) as u16).collect::<Vec<_>>()));
    values.iter().for_each(|&(axis, value)| t.extend(words(&[1, axis, 0, 257, value, 0])));
    t
}

// One design axis: a value on axis 7 describes no axis the table has, so an instance leaves it out
// as the public API does.
#[test]
fn a_value_on_no_axis_is_left_out_of_an_instance() {
    let source = stat(&[b"wght"], 8, &[(0, 700), (7, 400)]);
    let out = filter_stat_to_instance(&source, &BTreeMap::from([(crate::daecore::Tag::new(b"wght"), 700.0)]))
        .expect("a STAT");
    assert_eq!(read_u16_be(&out, 12), Some(1));
}

// With no design axes, designAxisSize sizes no record, so the table is filtered rather than kept
// whole with values on axes it lacks.
#[test]
fn a_stat_with_no_axes_is_filtered() {
    let out = filter_stat_to_instance(&stat(&[], 0, &[(0, 700)]), &BTreeMap::new()).expect("a STAT");
    assert_eq!(read_u16_be(&out, 12), Some(0));
}
