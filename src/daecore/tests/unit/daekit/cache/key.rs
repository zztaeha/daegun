use super::*;

fn tables(path: &str) -> Tables {
    let bytes = std::fs::read(alloc::format!("{}/assets/test-fonts/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).unwrap()
}

fn entries(key: &AxisKey) -> Vec<(AxisTag, f64)> {
    key.to_vec()
}

fn pairs(given: &[(&str, f64)]) -> Vec<(AxisTag, f64)> {
    given.iter().map(|&(tag, value)| (tag.parse().expect("a tag"), value)).collect()
}

// test_glyphs_variable's 31 axes, each at its maximum, named in fvar's order, backwards and from the middle: one
// key, in fvar's order. Of an axis named twice the last value counts, a default included.
#[test]
fn a_location_is_one_key_however_its_axes_are_named() {
    let map = tables("colr-v1-test-glyphs/test_glyphs_variable.ttf");
    let axes = crate::daecore::daetype::decoder::parse_fvar_axes(&map).unwrap();
    let cache = FontCache::new(map).unwrap();
    let at_max: Vec<(AxisTag, f64)> = axes.iter().map(|a| (a.tag, a.max)).collect();
    let key = cache.axis_key(&at_max);
    let moved: Vec<(AxisTag, f64)> = axes.iter().filter(|a| a.max != a.default).map(|a| (a.tag, a.max)).collect();
    assert_eq!(entries(&key), moved);
    assert!(key.len() > 20, "{} axes set", key.len());
    let backwards: Vec<_> = at_max.iter().rev().copied().collect();
    let from_middle: Vec<_> = at_max[15..].iter().chain(&at_max[..15]).copied().collect();
    assert_eq!(cache.axis_key(&backwards), key);
    assert_eq!(cache.axis_key(&from_middle), key);
    let (first, default) = (axes[0].tag, axes[0].default);
    let again: Vec<_> = at_max.iter().copied().chain([(first, default)]).collect();
    assert_eq!(entries(&cache.axis_key(&again)), entries(&key)[1..]);
    let twice = [(first, axes[0].max), (first, default), (first, axes[0].min)];
    assert_eq!(entries(&cache.axis_key(&twice)), [(first, axes[0].min)]);
}

// fvar listing AAAA twice, around BBBB: AAAA's value is kept as given, past either range, where BBBB's is clamped,
// and named as fvar lists the axes, the second AAAA is the first's. A value that isn't finite names nothing.
#[test]
fn a_tag_fvar_lists_twice_keeps_its_value_as_given() {
    let mut map = tables("colr-v1-test-glyphs/test_glyphs_variable.ttf");
    let mut fvar: Vec<u8> = [1u16, 0, 16, 2, 3, 20, 0, 12].iter().flat_map(|v| v.to_be_bytes()).collect();
    for (tag, max) in [(b"AAAA", 1), (b"BBBB", 1), (b"AAAA", 2)] {
        fvar.extend(tag);
        fvar.extend([-1i32 << 16, 0, max << 16].iter().flat_map(|v| v.to_be_bytes()));
        fvar.extend([0u16, 256].iter().flat_map(|v| v.to_be_bytes()));
    }
    map.insert("fvar", TableBytes::from_vec(fvar));
    map.remove("avar");
    let cache = FontCache::new(map).unwrap();
    assert_eq!(entries(&cache.axis_key(&[("AAAA", 5.0), ("BBBB", 5.0)])), pairs(&[("AAAA", 5.0), ("BBBB", 1.0)]));
    let as_listed = [("AAAA", 0.25), ("BBBB", 0.5), ("AAAA", 0.75)];
    assert_eq!(entries(&cache.axis_key(&as_listed)), pairs(&[("AAAA", 0.75), ("BBBB", 0.5)]));
    assert_eq!(entries(&cache.axis_key(&[("AAAA", 0.0)])), pairs(&[("AAAA", 0.0)]));
    assert!(cache.axis_key(&[("BBBB", f64::NAN), ("AAAA", f64::INFINITY)]).is_empty());
}
