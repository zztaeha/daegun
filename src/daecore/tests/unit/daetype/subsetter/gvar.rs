use super::*;

// A gvar of one axis and one shared tuple over `data`, each glyph's bytes, its offsets long where `long`.
fn gvar(data: &[&[u8]], long: bool) -> Vec<u8> {
    let offsets = if long { 4 } else { 2 } * (data.len() + 1);
    let mut out = [0x0001_0000u32.to_be_bytes().as_slice(), &[0, 1, 0, 1]].concat();
    out.extend(((20 + offsets) as u32).to_be_bytes());
    out.extend([data.len() as u16, u16::from(long)].map(u16::to_be_bytes).concat());
    out.extend(((22 + offsets) as u32).to_be_bytes());
    let mut at = 0;
    for bytes in [&[][..]].iter().chain(data) {
        at += bytes.len();
        match long {
            true => out.extend((at as u32).to_be_bytes()),
            false => out.extend(((at / 2) as u16).to_be_bytes()),
        }
    }
    out.extend([0x40, 0]);
    out.extend(data.concat());
    out
}

#[test]
fn each_kept_glyph_takes_its_data_to_its_new_place() {
    let source = gvar(&[&[], &[1, 2, 3, 4, 5, 6], &[7, 8]], false);
    let out = subset_gvar(&source, false, 3, &[(0, 0), (1, 2), (2, 1)], 3).expect("a subset").expect("in budget");
    // Glyph 2's two bytes are too few for a header and are left out.
    assert_eq!(out, gvar(&[&[], &[], &[1, 2, 3, 4, 5, 6]], false));
    // Past maxp a glyph's end offset reads 0, and its data reads as nothing.
    let past = subset_gvar(&source, false, 1, &[(0, 1)], 1).expect("a subset").expect("in budget");
    assert_eq!(past, gvar(&[&[]], false));
}

#[test]
fn short_offsets_pad_each_glyph_to_an_even_length_and_a_gap_reads_empty() {
    let source = gvar(&[&[1, 2, 3, 4, 5]], true);
    let out = subset_gvar(&source, false, 1, &[(1, 0)], 2).expect("a subset").expect("in budget");
    assert_eq!(out, gvar(&[&[], &[1, 2, 3, 4, 5, 0]], false));
}

#[test]
fn data_past_what_short_offsets_reach_takes_long_ones() {
    let big = vec![9; 0x2_0000];
    let out = subset_gvar(&gvar(&[&big], true), false, 1, &[(0, 0)], 1).expect("a subset").expect("in budget");
    assert_eq!(out, gvar(&[&big], true));
    // Long offsets leave odd data unpadded.
    let odd = gvar(&[&vec![9; 0x2_0001]], true);
    assert_eq!(subset_gvar(&odd, false, 1, &[(0, 0)], 1), Some(Ok(odd)));
}

#[test]
fn ranges_overlapping_past_the_subset_budget_refuse_the_subset() {
    // One axis, no shared tuples, long offsets: each even glyph names all the data, each odd one ends before it starts.
    let (n, len) = (64, 4096);
    let mut source = [0x0001_0000u32.to_be_bytes().as_slice(), &[0, 1, 0, 0, 0, 0, 0, 0]].concat();
    source.extend([n as u16, 1].map(u16::to_be_bytes).concat());
    source.extend(((20 + 4 * (n + 1)) as u32).to_be_bytes());
    source.extend((0..=n).flat_map(|i| (if i % 2 == 0 { 0 } else { len as u32 }).to_be_bytes()));
    source.resize(source.len() + len, 1);
    let every: Vec<(usize, u32)> = (0..n).map(|i| (i, i as u32)).collect();
    let refused = Error::limit("subset: gvar ranges past the subset budget");
    assert_eq!(subset_gvar(&source, false, n, &every, n), Some(Err(refused)));
    assert!(subset_gvar(&source, false, n, &every[..2], 2).is_some_and(|out| out.is_ok()));
}
