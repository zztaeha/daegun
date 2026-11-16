use super::*;

// An avar of these maps, `tail` after them.
fn avar(major: u16, maps: &[&[(i16, i16)]], tail: &[u8]) -> Vec<u8> {
    let mut out = [major, 0, 0, maps.len() as u16].map(u16::to_be_bytes).concat();
    for map in maps {
        out.extend((map.len() as u16).to_be_bytes());
        map.iter().for_each(|&(from, to)| out.extend([from.to_be_bytes(), to.to_be_bytes()].concat()));
    }
    [out, tail.to_vec()].concat()
}

// An axis from -1 to 1, default 0, through `avar` where given, in 2.14.
fn through(avar: Option<Vec<u8>>, value: f32) -> i32 {
    let mut location = [normalize_axis(value, -1.0, 0.0, 1.0)];
    apply_avar(avar.as_deref().and_then(Avar::read).as_ref(), &mut location);
    location[0]
}

const Q: i16 = 16384;

// Every step in 16.16, then 2.14 by adding 2 and shifting: wght 407 on 100 to 900 is 7/500 of the way, 917.504 in
// 16.16, rounded to 918 and shifted to 230; 399.9 is 26,207,846 in 16.16, 6,554 short of the default, so -22 and -5.
#[test]
fn normalization_rounds_through_16_16() {
    let wght = |v: f32| to_f2dot14(normalize_axis(v, 100.0, 400.0, 900.0));
    let got = [407.0, 700.0, 100.0, 900.0, 400.0, 250.0, 50.0, 1000.0, 399.9].map(wght);
    assert_eq!(got, [230, 9831, -16384, 16384, 0, -8192, -16384, 16384, -5]);
    // Halfway between two 2.14 steps rounds up on either side of zero.
    assert_eq!([-6, -2, 2, 6].map(to_f2dot14), [-1, 0, 1, 2]);
    // A half in 16.16 rounds up too, so -2.5 and -6.5 there land on 0 and -1.
    assert_eq!([-2.5f32, 2.5, -6.5].map(|v| through(None, v / 65536.0)), [0, 1, -1]);
}

// A subset pins each axis where shaping reads it: Source Serif 4 at wght 400.0122 is 800 past the default in 16.16,
// 1.6 of a 16.16 step along the axis, rounded to 2 and mapped to one 2.14 step.
#[test]
fn a_subset_pins_the_location_shaping_reads() {
    let path = "assets/test-fonts/source-serif/SourceSerif4Variable-Roman.otf";
    let bytes = std::fs::read(alloc::format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).expect("a fixture");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("it parses");
    let at = |v: f64| [(AxisTag::new(b"wght"), v)];
    assert_eq!(pinned_location(&map, &at(400.0122)), [1.0 / 16384.0, 0.0]);
    for v in [400.0122, 204.55, 900.0] {
        assert_eq!(Ok(pinned_location(&map, &at(v))), super::super::compute_location(&map, &at(v)));
    }
}

// An axis can span the whole Fixed range, -32768 to 32767.99, and halfway along either side is still half.
#[test]
fn an_axis_across_the_fixed_range_normalizes() {
    let wdth = |v: f32| to_f2dot14(normalize_axis(v, -32768.0, 0.0, 32767.99998));
    assert_eq!([-16384.0, -32768.0, 16384.0, 32767.99998].map(wdth), [-8192, -16384, 8192, 16384]);
}

// The overview's worked example: -0.75 maps to -0.5, -0.5 a third of the way on to -21,845 in 16.16 (-5,461), 0.5
// halfway from 0.4 to 0.6 onto 0.65; and Source Serif's weight map, 2,982 of the way, onto -64,438 (-16,109).
#[test]
fn avar_maps_interpolate_between_their_pairs() {
    type Case = (&'static [(i16, i16)], &'static [f32], &'static [i32]);
    let cases: [Case; 2] = [
        (&[(-Q, -Q), (-12288, -8192), (0, 0), (6554, 6554), (9830, 14746), (Q, Q)], &[-0.75, -0.5, 0.0, 0.5, 1.0],
            &[-8192, -5461, 0, 10650, 16384]),
        (&[(-Q, -Q), (-8192, -10354), (0, 0), (6554, 5407), (9830, 11599), (Q, Q)], &[-64045.0 / 65536.0],
            &[-16109]),
    ];
    for (map, values, want) in cases {
        let got: Vec<i32> = values.iter().map(|&v| through(Some(avar(1, &[map], &[])), v)).collect();
        assert_eq!(got, want, "{map:?}");
    }
}

// A pair whose from doesn't rise past the last kept, or whose to falls below it, is left out; a map that then lacks
// -1 to -1, 0 to 0 or 1 to 1 modifies nothing, as do one pair and a map a 2.14 step wide.
#[test]
fn avar_maps_leave_out_the_pairs_the_spec_lets_them_ignore() {
    type Case = (&'static [(i16, i16)], &'static [f32], &'static [i32]);
    let cases: [Case; 5] = [
        (&[(-Q, -Q), (-Q / 2, -Q / 4), (-Q / 2, -Q / 8), (0, 0), (Q / 2, Q / 4), (Q / 4, Q / 2), (Q, Q)],
            &[-0.5, 0.25, 0.75], &[-4096, 2048, 10240]),
        (&[(-Q, -Q), (0, -Q / 4), (0, 0), (0, Q / 4), (Q, Q)], &[0.0, 0.5], &[0, 8192]),
        (&[(-Q, -Q), (-Q / 2, Q / 2), (0, 0), (Q, Q)], &[-0.5, 0.5], &[-8192, 8192]),
        (&[(Q / 4, Q / 2)], &[0.0, 0.5, -1.0], &[0, 8192, -16384]),
        (&[(0, i16::MAX), (1, i16::MIN)], &[-1.0, 0.00001], &[-16384, 0]),
    ];
    for (map, values, want) in cases {
        let got: Vec<i32> = values.iter().map(|&v| through(Some(avar(1, &[map], &[])), v)).collect();
        assert_eq!(got, want, "{map:?}");
    }
}

// A map that takes 0.25 to 0.5.
const QUARTER_TO_HALF: &[(i16, i16)] = &[(-Q, -Q), (0, 0), (Q / 4, Q / 2), (Q, Q)];

// avar 2's tail: its index map's and store's offsets, then a store of `format` whose region list and one data
// offset are given, holding one region peaking at 1 and a delta of 2048 for it.
fn tail(map_at: u32, store_at: u32, format: u16, regions_at: u32, data_at: u32) -> Vec<u8> {
    let region = [1u16, 1, 0, 16384, 16384].map(u16::to_be_bytes).concat();
    let data = [1u16, 1, 1, 0, 2048].map(u16::to_be_bytes).concat();
    let offsets = [map_at, store_at].map(u32::to_be_bytes).concat();
    let head = [&format.to_be_bytes()[..], &regions_at.to_be_bytes(), &1u16.to_be_bytes(), &data_at.to_be_bytes()];
    [offsets, head.concat(), region, data].concat()
}

// What doesn't read is absent and the rest kept: a map cut short or a version 3 table maps nothing; a version 2 tail
// cut short, or a store past the table or of an unknown format, adds no deltas; an index map past it leaves each axis
// its own index, so the store's delta, 2048 at the peak and half that at 0.5, still adds.
#[test]
fn a_malformed_avar_keeps_what_reads() {
    let map = avar(1, &[QUARTER_TO_HALF], &[]);
    assert_eq!(through(Some(map.clone()), 0.25), 8192);
    let unmapped = [map[..map.len() - 2].to_vec(), avar(3, &[QUARTER_TO_HALF], &[])];
    for (n, table) in unmapped.into_iter().enumerate() {
        assert_eq!(through(Some(table), 0.25), 4096, "unmapped case {n}");
    }
    let mapped_alone = [
        avar(2, &[QUARTER_TO_HALF], &[0; 4]),
        avar(2, &[QUARTER_TO_HALF], &tail(0, 0xFFFF, 1, 12, 22)),
        avar(2, &[QUARTER_TO_HALF], &tail(0, 34, 7, 12, 22)),
    ];
    for (n, table) in mapped_alone.into_iter().enumerate() {
        assert_eq!(through(Some(table), 0.25), 8192, "mapped case {n}");
    }
    assert_eq!(through(Some(avar(2, &[QUARTER_TO_HALF], &tail(0xFFFF, 34, 1, 12, 22))), 0.25), 9216);
    assert_eq!(through(Some(avar(2, &[QUARTER_TO_HALF], &tail(0, 34, 1, 12, 22))), 0.25), 9216);
}

// avar 2 with no index map and one region peaking at 1, its delta 2048: read at the mapped 2.14 coordinate, rounded and
// added there, clamped to 1. 0.3 is 4,915 in 2.14, where the delta is 614.375, so 5,529.
#[test]
fn avar_2_adds_its_deltas_in_2_14() {
    let v2 = avar(2, &[&[]], &tail(0, 18, 1, 12, 22));
    let got = [0.0, 0.5, 1.0, -0.5, 0.3].map(|v| through(Some(v2.clone()), v));
    assert_eq!(got, [0, 9216, 16384, -8192, 5529]);
}
