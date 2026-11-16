use super::*;
use alloc::string::String;
use alloc::vec;

// Each glyph's FD expanded into a list, and where the FDSelect ends: the tests' oracle for `FdSelect`.
pub(crate) fn read_fd_select(
    table: &[u8], off: usize, n_glyphs: usize, kind: DictKind,
) -> Result<(Vec<u16>, usize), String> {
    let format = *table.get(off).ok_or("CFF FDSelect: offset out of bounds")?;
    let wide = match (format, kind) {
        (0, _) => {
            let end = off + 1 + n_glyphs;
            let fds = table.get(off + 1..end).ok_or("CFF FDSelect format 0: truncated")?;
            return Ok((fds.iter().map(|&fd| u16::from(fd)).collect(), end));
        }
        (3, _) => false,
        (4, DictKind::Cff2) => true,
        _ => return Err(format!("CFF FDSelect: unsupported format {format}")),
    };
    let truncated = || format!("CFF FDSelect format {format}: truncated");
    let (glyph, record) = if wide { (4, 6) } else { (2, 3) };
    let read_glyph = |at: usize| match wide {
        true => read_u32_be(table, at).map(|v| v as usize),
        false => read_u16_be(table, at).map(usize::from),
    };
    let read_fd = |at: usize| if wide { read_u16_be(table, at) } else { table.get(at).map(|&fd| u16::from(fd)) };
    let n_ranges = read_glyph(off + 1).ok_or_else(truncated)?;
    let ranges = off + 1 + glyph;
    let mut out = alloc::vec![0u16; n_glyphs];
    let mut prev_end = 0usize;
    for i in 0..n_ranges {
        let at = i.checked_mul(record).and_then(|r| r.checked_add(ranges)).ok_or_else(truncated)?;
        let first = read_glyph(at).ok_or_else(truncated)?;
        let fd = at.checked_add(glyph).and_then(read_fd).ok_or_else(truncated)?;
        let next_first = at.checked_add(record).and_then(read_glyph).ok_or_else(truncated)?;
        if first < prev_end || next_first <= first {
            continue;
        }
        let end = next_first.min(n_glyphs);
        prev_end = end;
        out.iter_mut().take(end).skip(first).for_each(|slot| *slot = fd);
    }
    let end = n_ranges.checked_mul(record).and_then(|r| r.checked_add(ranges + glyph));
    Ok((out, end.filter(|&end| end <= table.len()).ok_or_else(truncated)?))
}

// A blend adds each delta times its 16.16 weight: 10 plus 4 and 6 at half weight is 15, and a product of half a
// 16.16 unit rounds away from 0 to a whole one, either way.
#[test]
fn a_blend_sums_16_16_products_each_rounded_half_away_from_0() {
    assert_eq!(blend_value(10.0, &[4.0, 6.0], &[32_768, 32_768]), 15.0);
    let unit = 1.0 / 65_536.0;
    assert_eq!(blend_value(1.0, &[unit], &[32_768]), 1.0 + unit);
    assert_eq!(blend_value(1.0, &[-unit], &[32_768]), 1.0 - unit);
}

fn ops(dict: &[u8], kind: DictKind) -> Vec<(DictOp, Vec<i32>)> {
    let mut out = Vec::new();
    walk_cff_dict(dict, kind, |op, operands, _, _| out.push((op, operands.to_vec())));
    out
}

#[test]
fn a_cff2_dict_walks_past_a_blend() {
    let private = [129, 149, 140, 141, 141, 23, 6, 247, 0, 19];
    assert_eq!(
        ops(&private, DictKind::Cff2),
        [(DictOp::Single(23), vec![-10, 10, 1, 2, 2]), (DictOp::Single(6), vec![]), (DictOp::Single(19), vec![108])],
    );
}

// Each glyph's FD and the FDSelect's end in each format, format 4 in CFF2 alone.
#[test]
fn fdselect_maps_every_glyph_in_each_format() {
    let mut f0 = vec![0u8];
    f0.extend([2, 0, 1, 1, 9]);
    assert_eq!(read_fd_select(&f0, 0, 4, DictKind::Cff1), Ok((vec![2, 0, 1, 1], 5)));

    let f3 = [3, 0, 2, 0, 0, 1, 0, 3, 4, 0, 5, 9];
    assert_eq!(read_fd_select(&f3, 0, 5, DictKind::Cff1), Ok((vec![1, 1, 1, 4, 4], 11)));

    let mut f4 = vec![4, 0, 0, 0, 2];
    f4.extend([0, 0, 0, 0, 0, 7, 0, 0, 0, 2, 0x01, 0x02]);
    f4.extend([0, 0, 0, 5, 9]);
    assert_eq!(read_fd_select(&f4, 0, 5, DictKind::Cff2), Ok((vec![7, 7, 0x0102, 0x0102, 0x0102], 21)));
    assert!(read_fd_select(&f4, 0, 5, DictKind::Cff1).is_err());
    assert!(read_fd_select(&[3, 0, 0, 0], 0, 5, DictKind::Cff1).is_err(), "a sentinel cut short");
}

fn f3(ranges: &[(u16, u8)], sentinel: u16) -> Vec<u8> {
    let mut t = [&[3u8][..], &(ranges.len() as u16).to_be_bytes()].concat();
    ranges.iter().for_each(|&(first, fd)| t.extend([&first.to_be_bytes()[..], &[fd]].concat()));
    t.extend(sentinel.to_be_bytes());
    t
}

// FDSelect read in place gives each glyph the expanded list's FD, None one past, and the same end and errors: ranges
// out of order, overlapping, empty, past the glyphs, none and gaps, in both formats, and every CFF fixture's glyph.
#[test]
fn fd_select_in_place_reads_as_the_expanded_list() {
    let u16s = |v: &[u16]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    let f4 = [&[4u8][..], &[0, 0, 0, 2], &[0, 0, 0, 0], &u16s(&[7]), &[0, 0, 0, 4], &u16s(&[1]), &[0, 0, 0, 9]];
    let f4 = f4.concat();
    let cases: Vec<(Vec<u8>, usize, DictKind)> = alloc::vec![
        ([&[0u8][..], &[2, 0, 1, 1, 9]].concat(), 5, DictKind::Cff1),
        ([&[0u8][..], &[2, 0, 1]].concat(), 5, DictKind::Cff1),
        (f3(&[(0, 1), (10, 2), (5, 3)], 20), 20, DictKind::Cff1),
        (f3(&[(2, 1), (4, 2)], 9), 12, DictKind::Cff1),
        (f3(&[(0, 1), (3, 2), (3, 4), (8, 5)], 30), 12, DictKind::Cff1),
        (f3(&[(0, 1), (40, 2)], 50), 12, DictKind::Cff1),
        (f3(&[], 0), 6, DictKind::Cff1),
        (f3(&[(0, 9)], 0), 4, DictKind::Cff1),
        (f4.clone(), 12, DictKind::Cff2),
        (f4, 12, DictKind::Cff1),
    ];
    let fixtures = ["test-fixtures/cff-cid.otf", "source-han-sans/SourceHanSansJP-VF.otf"].map(|rel| {
        let path = alloc::format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"));
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("fixture"));
        map.expect("parses")
    });
    let mut whole = Vec::new();
    for (map, (tag, kind)) in fixtures.iter().zip([("CFF ", DictKind::Cff1), ("CFF2", DictKind::Cff2)]) {
        let table = map.get(tag).expect("the table").to_vec();
        use crate::daecore::daetype::subsetter::{cff::parse::read_header, CffIndex};
        let top = match kind {
            DictKind::Cff1 => read_header(&table).expect("header").top,
            DictKind::Cff2 => read_dict(&table[5..5 + usize::from(read_u16_be(&table, 3).expect("top"))], kind),
        };
        let at = top.fd_select.expect("an FDSelect");
        let count = |at: Option<usize>| CffIndex::read(&table, at.expect("an INDEX"), kind == DictKind::Cff2);
        let glyphs = count(top.charstrings).expect("CharStrings").0.len();
        whole.push((table, at, glyphs, kind));
    }
    let synthetic = cases.into_iter().map(|(t, n, kind)| (t, 0, n, kind));
    for (table, at, glyphs, kind) in synthetic.chain(whole) {
        let read = FdSelect::read(&table, at, glyphs, kind);
        let expanded = read_fd_select(&table, at, glyphs, kind);
        assert_eq!(
            read.as_ref().map(|r| r.1).map_err(|e| e.message()),
            expanded.as_ref().map(|e| e.1).map_err(String::as_str),
            "{table:?}"
        );
        let (Ok((select, _)), Ok((list, _))) = (read, expanded) else { continue };
        let got: Vec<u16> = (0..glyphs).map(|g| select.get(&table, g).expect("a glyph")).collect();
        assert_eq!(got, list, "{:?}", &table[at..table.len().min(at + 40)]);
        assert_eq!(select.get(&table, glyphs), None);
    }
    let none = FdSelect::zero(3);
    assert_eq!((none.get(&[], 2), none.get(&[], 3)), (Some(0), None));
}

// The last of each operator wins, and an offset of 0 or a negative one is none.
#[test]
fn a_dicts_last_operator_wins() {
    let dict = [149, 19, 159, 19, 139, 17, 32, 17, 140, 149, 18, 140, 139, 18];
    let d = read_dict(&dict, DictKind::Cff1);
    assert_eq!((d.subrs, d.charstrings, d.private), (Some(20), None, None));
    let d = read_dict(&[139, 15, 140, 15, 141, 140, 18], DictKind::Cff1);
    assert_eq!((d.charset(), d.private), (Charset::Predefined(1), Some(1..3)));
}

#[test]
fn dict_and_charstring_numbers_read_their_values() {
    assert_eq!(decode_cff_number(&[28, 0xFF, 0x38], 0), Ok((-200, 3)));
    assert_eq!(decode_cff_number(&[29, 0xFF, 0xFF, 0xFF, 0x38], 0), Ok((-200, 5)));
    assert_eq!(decode_cff_number(&[30, 0x1A, 0x5F], 0), Ok((0, 3)), "a real reads as 0");
    assert_eq!(decode_charstring_number(&[255, 0, 1, 0x80, 1], 0), Ok((1.5 + 1.0 / 65536.0, 5)));
    assert_eq!(decode_charstring_number_fx(&[255, 0xFF, 0xFE, 0x40, 0], 0), Some((-0x1_C000, 5)));
    assert_eq!(decode_charstring_number(&[28, 0x80, 0], 0), Ok((-32768.0, 3)));
}

// Each integer form's edges, written by the shared short encoder and read back by both decoders.
#[test]
fn short_integers_round_trip_at_each_forms_edges() {
    let edges = [(-32769, 5), (-32768, 3), (-1132, 3), (-1131, 2), (-108, 2), (-107, 1), (107, 1), (108, 2),
                 (1131, 2), (1132, 3), (32767, 3), (32768, 5)];
    for (n, width) in edges {
        let bytes = encode_cff_int_short(n);
        assert_eq!(decode_cff_number(&bytes, 0), Ok((n, width)), "{n} in a DICT");
        if width < 5 {
            assert_eq!(decode_charstring_number(&bytes, 0), Ok((f64::from(n), width)), "{n} in a charstring");
        }
    }
}

#[test]
fn a_subroutine_bias_changes_at_its_thresholds() {
    assert_eq!([1239, 1240, 33899, 33900].map(subr_bias), [107, 1131, 1131, 32768]);
}
