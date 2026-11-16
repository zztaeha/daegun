use super::*;
use alloc::vec;

fn index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u32).to_be_bytes().to_vec();
    out.push(4);
    let mut at = 1u32;
    out.extend(at.to_be_bytes());
    for item in items {
        at += item.len() as u32;
        out.extend(at.to_be_bytes());
    }
    items.iter().for_each(|i| out.extend(i));
    out
}

// A CFF2 of one charstring a glyph, each glyph's FD from `fd_of` in FDSelect format 4, a Font DICT for each of
// `privates` naming it, and `store` and `subrs` as the variation store and global subroutines where they're any.
fn font(charstrings: &[Vec<u8>], fd_of: &[u16], privates: &[Vec<u8>], store: &[u8], subrs: &[Vec<u8>]) -> Vec<u8> {
    let int = |v: usize| [vec![29], (v as u32).to_be_bytes().to_vec()].concat();
    let charstrings = index(charstrings);
    let mut select = vec![4];
    select.extend((fd_of.len() as u32).to_be_bytes());
    for (gid, &fd) in fd_of.iter().enumerate() {
        select.extend((gid as u32).to_be_bytes());
        select.extend(fd.to_be_bytes());
    }
    select.extend((fd_of.len() as u32).to_be_bytes());
    let fd_array_len = index(&vec![vec![0; 11]; privates.len()]).len();
    let top_len = if store.is_empty() { 20 } else { 26 };
    let subrs = if subrs.is_empty() { vec![0; 4] } else { index(subrs) };
    let charstrings_at = 5 + top_len + subrs.len();
    let select_at = charstrings_at + charstrings.len();
    let fd_array_at = select_at + select.len();
    let mut private_at = fd_array_at + fd_array_len;
    let fonts: Vec<Vec<u8>> = privates.iter().map(|p| {
        let dict = [int(p.len()), int(private_at), vec![18]].concat();
        private_at += p.len();
        dict
    }).collect();
    let top = [int(charstrings_at), vec![17], int(fd_array_at), vec![12, 36], int(select_at), vec![12, 37]];
    let mut top = top.concat();
    if !store.is_empty() {
        top.extend([int(private_at), vec![24]].concat());
    }
    let header = vec![2, 0, 5, 0, top_len as u8];
    let store = if store.is_empty() { vec![] } else { [&(store.len() as u16).to_be_bytes()[..], store].concat() };
    [header, top, subrs, charstrings, select, index(&fonts), privates.concat(), store].concat()
}

fn written(font: &[u8], glyphs: usize) -> Vec<u8> {
    let slots: Vec<Option<u32>> = (0..glyphs as u32).map(Some).collect();
    Cff2Source::read(font, glyphs, false).expect("reads").write(&[], &slots).expect("writes")
}

// Each glyph's FD, None without FDSelect.
fn fds_of(source: &Cff2Source, glyphs: usize) -> Option<Vec<u16>> {
    let (select, cff2) = source.fd_select.as_ref()?;
    Some((0..glyphs).map(|g| select.get(cff2, g).expect("a glyph's FD")).collect())
}

fn fd_select_format(cff2: &[u8]) -> u8 {
    let top = read_dict(&cff2[5..5 + usize::from(read_u16_be(cff2, 3).unwrap())], Kind::Top).unwrap();
    cff2[top.fd_select as usize]
}

// A charstring's operands, and its operators as "op" and their number, hint masks' bytes skipped.
fn program(charstring: &[u8]) -> Vec<String> {
    let (mut out, mut at, mut stems, mut operands) = (Vec::new(), 0, 0usize, 0);
    while at < charstring.len() {
        let op = fetch_op(charstring, &mut at);
        let fixed = (op == 255).then(|| take::<4>(charstring, &mut at).expect("16.16"));
        let fixed = fixed.map(|b| f64::from(i32::from_be_bytes(b)) / 65536.0);
        if let Some(v) = fixed.map_or_else(|| number(op, charstring, &mut at).expect("reads"), Some) {
            out.push(format!("{v}"));
            operands += 1;
            continue;
        }
        if matches!(op, 1 | 3 | 18 | 19 | 20 | 23) {
            stems += operands / 2;
        }
        if matches!(op, 19 | 20) {
            at += stems.div_ceil(8);
        }
        out.push(format!("op{op}"));
        operands = 0;
    }
    out
}

// Each glyph of `charstrings` pinned at 0.5 of the way to the one region of a one-axis store, as written.
fn pinned(charstrings: &[Vec<u8>]) -> Result<Vec<Vec<String>>, crate::daecore::Error> {
    let regions = [0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0];
    let store = [&[0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22][..], &regions, &[0, 0, 0, 0, 0, 1, 0, 0]].concat();
    let glyphs = charstrings.len();
    let font = font(charstrings, &vec![0; glyphs], &[vec![]], &store, &[vec![]]);
    let slots: Vec<Option<u32>> = (0..glyphs as u32).map(Some).collect();
    let out = Cff2Source::read(&font, glyphs, false).expect("reads").write(&[0.5], &slots)?;
    let back = Cff2Source::read(&out, glyphs, false).expect("reads back");
    Ok((0..glyphs).map(|g| program(back.charstrings.get(g).expect("a charstring"))).collect())
}

// A glyph whose operators take a blend is rounded at absolute positions (four halves step 1, 1, 0, 0); with a flex1 or
// no blend taken, each blend alone, at the end or in a subroutine too; bad operands fail if rounded.
#[test]
fn a_glyph_that_takes_a_blend_is_rounded_at_its_absolute_positions() {
    let halves = [139, 139, 139, 139, 140, 140, 140, 140, 143, 16];
    let half = [255, 0, 0, 0x80, 0];
    let flex1 = [&[139; 11][..], &[12, 37]].concat();
    let glyphs = [
        [&halves[..], &[5]].concat(),
        [&halves[..], &[5], &flex1].concat(),
        [&half[..], &half, &half, &half, &[5]].concat(),
        [&half[..], &half, &[5], &[139, 140, 140, 16]].concat(),
        [&halves[..], &[1], &halves, &[5]].concat(),
        [&half[..], &half, &[32, 139, 140, 16, 29, 5]].concat(),
    ];
    let written = pinned(&glyphs).expect("writes");
    let [ones, flexed, unblended, left, stems, called] = written.try_into().expect("six glyphs");
    assert_eq!(ones, ["1", "1", "0", "0", "op5"]);
    assert_eq!(flexed[..5], ["1", "1", "1", "1", "op5"], "a flex1");
    assert_eq!(unblended, ["0.5", "0.5", "0.5", "0.5", "op5"], "no blend");
    assert_eq!(left, ["0.5", "0.5", "op5"], "a blend no operator takes");
    assert_eq!(called, ["0.5", "0.5", "op5"], "a blend called as a subroutine");
    let odd = |operands: &[u8]| pinned(&[[operands, &[139, 5]].concat()]).map(drop);
    assert_eq!((odd(&halves).is_err(), odd(&[139; 4]).is_ok()), (true, true));
    assert_eq!(stems, ["1", "0", "1", "0", "op1", "1", "1", "0", "0", "op5"], "stem edges start over");
}

// Each operator's operands rounded by its own pattern: alternating from x, or from the axis it names; curves'
// own patterns; stem edges from 0 each operator; a flex's depth alone; an hflex back at the y it started from.
#[test]
fn each_operator_rounds_its_operands_by_its_pattern() {
    let rounded = |op: u16, args: &[f64]| {
        let (mut at, mut args) = (Rounding::default(), args.to_vec());
        at.round(op, &mut args).map(|()| args)
    };
    let half = |n: usize| vec![0.5; n];
    assert_eq!(rounded(5, &half(4)), Ok(vec![1.0, 1.0, 0.0, 0.0]));
    assert_eq!(rounded(7, &half(3)), Ok(vec![1.0, 1.0, 0.0]), "vlineto from y");
    assert_eq!(rounded(5, &[-0.5, -1.5]), Ok(vec![0.0, -1.0]), "half up, below 0 too");
    assert_eq!(rounded(27, &half(5)), Ok(vec![1.0, 1.0, 0.0, 0.0, 1.0]), "hhcurveto, dy1 first");
    assert_eq!(rounded(31, &half(5)), Ok(vec![1.0, 0.0, 1.0, 0.0, 1.0]), "hvcurveto, dxf on x");
    assert_eq!(rounded(31, &[0.5, 0.5, 0.5, 0.0, 0.5]), Ok(vec![1.0, 0.0, 1.0, 0.0, 1.0]), "dxf, not on y");
    // From a pen off the grid on x alone, so an operand on the wrong axis rounds otherwise.
    let after = |op: u16, args: &[f64]| {
        let mut at = Rounding::default();
        at.round(5, &mut [0.5, 0.0]).expect("a line");
        let mut args = args.to_vec();
        at.round(op, &mut args).map(|()| args)
    };
    assert_eq!(after(7, &[0.5]), Ok(vec![1.0]), "vlineto on y");
    assert_eq!(after(1, &[0.5, 0.5]), Ok(vec![1.0, 0.0]), "stem edges from 0, not the pen");
    let flex = [vec![0.0; 12], vec![50.5]].concat();
    assert_eq!(after(0x0C23, &flex).map(|f| f[12]), Ok(51.0), "a flex's depth alone, not on x");
    assert_eq!(rounded(30, &half(8)), Ok(vec![1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0]), "vhcurveto");
    assert_eq!(rounded(1, &half(4)), Ok(vec![1.0, 0.0, 1.0, 0.0]), "stem edges");
    let mut flex = half(12);
    flex.push(50.5);
    let flexed = rounded(0x0C23, &flex).expect("a flex");
    assert_eq!((flexed[..4].to_vec(), flexed[12]), (vec![1.0, 1.0, 0.0, 0.0], 51.0));
    let mut at = Rounding::default();
    at.round(0x0C22, &mut half(7)).expect("an hflex");
    assert_eq!(at.0, [(3.0, 3.0), (0.0, 0.0)], "hflex moves y back");
    at.round(0x0C24, &mut half(9)).expect("an hflex1");
    assert_eq!(at.0, [(6.0, 6.0), (0.0, 0.0)], "hflex1 too");
    let unfit = [(5, 3), (21, 1), (4, 2), (8, 5), (24, 7), (25, 7), (27, 6), (0x0C23, 12), (0x0C22, 8), (18, 3)];
    for (op, n) in unfit {
        assert!(rounded(op, &half(n)).is_err(), "{op} with {n}");
    }
    assert_eq!(rounded(19, &[]), Ok(vec![]), "a hint mask of no stems");
}

// At wght 900 and opsz 60 (2.14 coordinates 1, 1), Source Serif 4's I stems at 594, and its minute's second
// vstem edge steps 134; at wght 900 Source Han Sans's U+3330 curves -29 and -12: daegun's recorded output.
#[test]
fn variable_fonts_instance_with_their_positions_rounded() {
    let glyph = |rel: &str, coords: &[f64], gid: u32| {
        let path = alloc::format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"));
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
        let map = map.expect("parses");
        let (cff2, glyphs) = (map["CFF2"].to_vec(), usize::from(read_u16_be(&map["maxp"], 4).expect("a count")));
        let source = Cff2Source::read(&cff2, glyphs, false).expect("reads");
        let out = source.write(coords, &[Some(0), Some(gid)]).expect("writes");
        program(Cff2Source::read(&out, 2, false).expect("reads back").charstrings.get(1).expect("the glyph"))
    };
    let serif = "source-serif/SourceSerif4Variable-Roman.otf";
    assert_eq!(glyph(serif, &[1.0, 1.0], 10)[..5], ["0", "29", "594", "29", "op1"]);
    assert_eq!(glyph(serif, &[1.0, 1.0], 759)[3..6], ["50", "134", "op3"]);
    let han = glyph("source-han-sans/SourceHanSansJP-VF.otf", &[1.0], 1987);
    assert!(han.windows(6).any(|w| w == ["-108", "op6", "2", "-19", "2", "-29"]), "{han:?}");
}

#[test]
fn fds_no_glyph_selects_are_dropped_and_the_rest_renumbered_in_order() {
    let privates: Vec<Vec<u8>> = (0..4).map(|i| vec![139 + i, 10]).collect();
    let out = written(&font(&vec![vec![]; 4], &[0, 3, 3, 1], &privates, &[], &[]), 4);
    let back = Cff2Source::read(&out, 4, false).expect("the instance reads");
    assert_eq!(fds_of(&back, 4), Some(vec![0, 2, 2, 1]));
    let stdhw: Vec<&[u8]> = back.fds.iter().map(|fd| back.privates[fd.private][0].1).collect();
    assert_eq!(stdhw, [&[139, 10][..], &[140, 10], &[142, 10]], "FD 2 is dropped and FD 3 becomes 2");
    assert_eq!(fd_select_format(&out), 3);
}

// FDSelect format 3, the smaller, holds at most 255 FontDICTs by the CFF2 chapter; past them, format 4.
#[test]
fn past_255_fds_fdselect_takes_format_4() {
    for (fds, format) in [(255, 3), (256, 4)] {
        let privates: Vec<Vec<u8>> = (0..fds).map(|i| vec![28, (i >> 8) as u8, i as u8, 10]).collect();
        let fd_of: Vec<u16> = (0..fds as u16).collect();
        let out = written(&font(&vec![vec![]; fds], &fd_of, &privates, &[], &[]), fds);
        assert_eq!(fd_select_format(&out), format, "{fds} FDs");
        assert_eq!(fds_of(&Cff2Source::read(&out, fds, false).expect("reads"), fds), Some(fd_of));
    }
}

// An FD whose Private DICT is another's, byte for byte, points at that one, so the dict is written once.
#[test]
fn fds_with_one_private_dict_share_it() {
    let privates = [vec![139, 10], vec![140, 10], vec![139, 10]];
    let out = written(&font(&vec![vec![]; 3], &[0, 1, 2], &privates, &[], &[]), 3);
    let fonts = index_at(&out, 0, read_dict(&out[5..25], Kind::Top).unwrap().fd_array.into()).unwrap();
    let at: Vec<u32> =
        (0..fonts.len()).map(|i| read_dict(fonts.get(i).unwrap(), Kind::Font).unwrap().private.1).collect();
    assert_eq!(at[0], at[2]);
    assert_eq!(at[0] + 2, at[1], "the shared one is laid out first, in FD order");
}

// An unknown operator drops its operands, a fixed number cut short runs as operators, and so does a hint mask
// cut short.
#[test]
fn a_charstring_flattens_its_unknown_and_short_parts() {
    let glyphs = [vec![139, 139, 0, 139, 21], vec![139, 139, 21, 255, 1], vec![139, 139, 1, 19]];
    let out = written(&font(&glyphs, &[0, 0, 0], &[vec![]], &[], &[]), 3);
    let back = Cff2Source::read(&out, 3, false).expect("reads");
    let charstrings: Vec<&[u8]> = (0..3).filter_map(|g| back.charstrings.get(g)).collect();
    assert_eq!(charstrings, [&[139, 21][..], &[139, 139, 21, 1], &[139, 139, 1]]);
}

// A lone escape byte ends a Private DICT as an operator of no operands, written back.
#[test]
fn a_private_dict_cut_short_in_an_escape_keeps_it() {
    let ops = read_dict(&[139, 6, 12], Kind::Private).unwrap().ops;
    assert_eq!(write_private(&ops, None, &[], &mut 1_000), Ok(vec![139, 6, 12, 255]));
}

// A blended BlueScale keeps its fraction, 0.039625 in the 16.16 a blend sums in rather than 0; a blended BlueShift of
// 7.4 is rounded to 7.
#[test]
fn a_pinned_private_dict_rounds_all_but_its_fractions() {
    let ops = read_dict(&[30, 0x0A, 0x03, 0x96, 0x25, 0xFF, 140, 23, 12, 9], Kind::Private).unwrap().ops;
    let out = write_private(&ops, None, &[], &mut 1_000).expect("written");
    let mut at = 0;
    let op = fetch_op(&out, &mut at);
    assert_eq!(dict_number(op, &out, &mut at), Ok(Some(2596.0 / 65536.0)));
    assert_eq!(out[at..], [12, 9]);
    let ops = read_dict(&[30, 0x7A, 0x4F, 140, 23, 12, 10], Kind::Private).unwrap().ops;
    assert_eq!(write_private(&ops, None, &[], &mut 1_000), Ok(vec![146, 12, 10]));
}

#[test]
fn a_blend_rounds_half_away_from_0_and_a_number_past_16_bits_is_pinned() {
    let mut args = Args(vec![0.5, -0.5, 2.25, 1.0, 1.0, 1.0, 3.0], None);
    args.blend(1, Some(&vec![0]), true).expect("blends");
    assert_eq!(args.0, [1.0, -1.0, 2.0]);
    let mut out = Vec::new();
    write_charstring_number(&mut out, 40_000.0);
    write_charstring_number(&mut out, 1.5);
    assert_eq!(out, [255, 0x7F, 0xFF, 0xFF, 0xFF, 255, 0, 1, 0x80, 0]);
}

// The fewest bytes: a whole number in 16 bits as an integer, others as a real's fewest exact digits, plain or exponent,
// or a long integer where shorter: 0.039625 as .039625, 40,000 as 4E4, 123,456,789 long; past f64's range, refused.
#[test]
fn a_dict_number_is_written_in_its_fewest_bytes() {
    let cases: [(f64, &[u8]); 12] = [
        (0.039625, &[30, 0xA0, 0x39, 0x62, 0x5F]),
        (40_000.0, &[30, 0x4B, 0x4F]),
        (1_200_000.0, &[30, 0x12, 0xB5, 0xFF]),
        (-40_000.0, &[30, 0xE4, 0xB4, 0xFF]),
        (123_456_789.0, &[29, 0x07, 0x5B, 0xCD, 0x15]),
        (1_234_567.0, &[30, 0x12, 0x34, 0x56, 0x7F]),
        (-0.5, &[30, 0xEA, 0x5F]),
        (1e-5, &[30, 0x1C, 0x5F]),
        (0.0001234, &[30, 0x12, 0x34, 0xC7, 0xFF]),
        (1e300, &[30, 0x1B, 0x30, 0x0F]),
        (-2.5e-7, &[30, 0xE2, 0x5C, 0x8F]),
        (12_000.0, &[28, 0x2E, 0xE0]),
    ];
    for (v, want) in cases {
        let mut out = Vec::new();
        write_dict_number(&mut out, v).expect("a finite value");
        assert_eq!(out, want, "{v}");
        if out[0] == 30 {
            assert_eq!(parse_real(&out[1..], &mut 0), Ok(v), "{v} reads back");
        }
    }
    assert!(write_dict_number(&mut Vec::new(), f64::INFINITY).is_err());
}

// A real reads as the decimal its nibbles spell, to the nearest f64: 7,473,713,815,003,510,253 to 0x43D9EDFDC7820DAD.
// A lone point or sign, a point with no digits before E and an exponent past f64's range don't read.
#[test]
fn a_real_reads_as_the_nearest_f64_to_its_digits() {
    let read = |bytes: &[u8]| parse_real(bytes, &mut 0).ok().map(f64::to_bits);
    let nineteen = [0x74, 0x73, 0x71, 0x38, 0x15, 0x00, 0x35, 0x10, 0x25, 0x3F];
    assert_eq!(read(&nineteen), Some(0x43D9_EDFD_C782_0DAD));
    assert_eq!(read(&[0xAF]), None);
    assert_eq!(read(&[0xEF]), None);
    assert_eq!(read(&[0x5A, 0xEB, 0x3F]), None);
    assert_eq!(read(&[0x0C, 0x5F]), Some(0));
    assert_eq!(read(&[0x1B, 0x40, 0x0F]), None);
}

// Beside VARC, CharStrings may hold fewer glyphs than maxp counts: a glyph past them is written empty in FD 0.
#[test]
fn a_glyph_past_charstrings_beside_varc_is_written_empty() {
    let path = alloc::format!("{}/assets/test-fonts/varc/varc-short-cff2.otf", env!("CARGO_MANIFEST_DIR"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let cff2 = map.expect("parses").remove("CFF2").expect("CFF2");
    assert!(Cff2Source::read(&cff2, 4, false).is_err());
    let slots = [Some(0), Some(1), Some(2), Some(3)];
    let out = Cff2Source::read(&cff2, 4, true).expect("reads").write(&[], &slots).expect("writes");
    let back = Cff2Source::read(&out, 4, false).expect("holds every glyph");
    assert_eq!((back.charstrings.get(1).map(<[u8]>::len), back.charstrings.get(3)), (Some(7), Some(&[][..])));
}

// A VariationStore lies within its length, none at 0 (CFF2, "VariationStore"), in the instancer and drawing alike: 12
// bytes of one axis read; a byte short, its region list and axes are absent; cut inside its header, it's none.
#[test]
fn a_cff2_variation_store_reads_within_its_length() {
    use crate::daecore::daetype::instancer::Cff2Store;
    let store = [0u8, 1, 0, 0, 0, 8, 0, 0, 0, 1, 0, 0];
    let cff2 = |length: u16| [&[0xFF][..], &length.to_be_bytes(), &store].concat();
    let lengths = [12, 11, 6, 0];
    assert_eq!(lengths.map(|n| store_at(&cff2(n), 1).map(|s| s.axis_count)), [Some(1), Some(0), None, None]);
    assert_eq!(lengths.map(|n| Cff2Store::load(&cff2(n), 1).map(|s| s.axes())), [Some(1), Some(0), None, None]);
}

// A blend's regions vary only at coordinates for the store's own axes: for other axes, as where its region list is
// absent, each region weighs 0 and the glyph draws its default, as gvar's does. One axis and one region peaking at 1.
#[test]
fn a_cff2_store_for_other_axes_than_the_fonts_blends_nothing() {
    use crate::daecore::daetype::instancer::Cff2Store;
    let store = [0u8, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22, 0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0, 0, 0, 0, 0, 0, 1, 0, 0];
    let cff2 = [&[0xFF][..], &30u16.to_be_bytes(), &store].concat();
    let store = Cff2Store::load(&cff2, 1).expect("loads");
    let weights = |coords: Option<&[i64]>| store.weights(0, coords).expect("weighs");
    assert_eq!(weights(Some(&[0x8000])), [0x10000, 0x8000]);
    assert_eq!(weights(Some(&[0x8000, 0])), [0x10000, 0]);
    assert_eq!(weights(None), [0x10000, 0]);
}

// A Top DICT naming CharStrings `repeats` times at 0 before its real CharStrings and FDArray, each of the 2-byte pairs
// written again as 6 bytes: past 65,535 at 11,000 of them, though not at 10, the instance is refused, not cut short.
#[test]
fn a_top_dict_grown_past_16_bits_is_refused() {
    let written = |repeats: usize| {
        let int = |v: usize| [vec![29], (v as u32).to_be_bytes().to_vec()].concat();
        let top_len = 2 * repeats + 13;
        let (charstrings_at, fd_array_at) = (5 + top_len + 4, 5 + top_len + 4 + 13);
        let top = [[139, 17].repeat(repeats), int(charstrings_at), vec![17], int(fd_array_at), vec![12, 36]].concat();
        let header = [vec![2, 0, 5], (top_len as u16).to_be_bytes().to_vec()].concat();
        let font = [header, top, vec![0; 4], index(&[vec![]]), index(&[[int(0), int(0), vec![18]].concat()])].concat();
        Cff2Source::read(&font, 1, false).expect("reads").write(&[], &[Some(0)]).map(drop).map_err(|e| e.kind())
    };
    assert_eq!(written(10), Ok(()));
    assert_eq!(written(11_000), Err(crate::daecore::ErrorKind::LimitExceeded));
}

// `5 vsindex` names no data set, so a blend has no regions and leaves its defaults where the next reads them again:
// 10 `512 blend`s fit the 64 a byte a write may spend, 100 run twice, 102,400 defaults read, don't.
#[test]
fn a_blend_without_regions_is_charged_each_default_it_reads() {
    let regions = [0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0];
    let store = [&[0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22][..], &regions, &[0, 0, 0, 0, 0, 1, 0, 0]].concat();
    let written = |blends: usize, calls: usize| {
        let subr = [248u8, 148, 16].repeat(blends);
        let glyph = [&[144u8, 15][..], &[139; 512], &[32, 29].repeat(calls)].concat();
        let font = font(&[vec![], glyph], &[0, 0], &[vec![]], &store, &[subr]);
        let source = Cff2Source::read(&font, 2, false).expect("reads");
        source.write(&[0.5], &[Some(0), Some(1)]).map(drop).map_err(|e| e.kind())
    };
    assert_eq!(written(10, 1), Ok(()));
    assert_eq!(written(100, 2), Err(crate::daecore::ErrorKind::LimitExceeded));
}

// A CFF2 of one empty glyph whose FDArray of `fds` Font DICTs names Private DICTs of `size` bytes: one shared by every
// FD, or where `apart`, FD i's starting i bytes into the one run of them.
fn privates_named(fds: usize, size: usize, apart: bool) -> Vec<u8> {
    let int = |v: usize| [vec![29], (v as u32).to_be_bytes().to_vec()].concat();
    let charstrings = index(&[vec![]]);
    let (charstrings_at, top_len) = (5 + 13 + 4, 13);
    let fd_array_at = charstrings_at + charstrings.len();
    let private_at = fd_array_at + index(&vec![vec![0; 11]; fds]).len();
    let at = |i: usize| private_at + if apart { i } else { 0 };
    let fonts: Vec<Vec<u8>> = (0..fds).map(|i| [int(size), int(at(i)), vec![18]].concat()).collect();
    let top = [int(charstrings_at), vec![17], int(fd_array_at), vec![12, 36]].concat();
    let header = vec![2, 0, 5, 0, top_len as u8];
    [header, top, vec![0; 4], charstrings, index(&fonts), vec![0; size + fds]].concat()
}

// 100 FDs each reading their own 300 bytes of overlapping Private DICTs fit the subset budget the FDs' DICTs share,
// twice the table and 64 KB; 1,000 bytes each, 100,000 in all, don't, and the font is refused.
#[test]
fn private_dicts_read_past_the_subset_budget_refuse_the_font() {
    let read = |size: usize| Cff2Source::read(&privates_named(100, size, true), 1, false).map(drop);
    let read = |size: usize| read(size).map_err(|e| e.kind());
    assert_eq!(read(300), Ok(()));
    assert_eq!(read(1_000), Err(crate::daecore::ErrorKind::LimitExceeded));
}

// 200 FDs naming one Private DICT of 4,096 one-byte operators read it once, its operators held once rather than by
// each FD.
#[cfg(any(debug_assertions, daegun_counting))]
#[test]
fn fds_naming_one_private_dict_read_it_once() {
    let font = privates_named(200, 4_096, false);
    let (_, before, _) = crate::allocations::tally();
    let source = Cff2Source::read(&font, 1, false).expect("reads");
    let (_, after, _) = crate::allocations::tally();
    assert_eq!((source.fds.len(), source.privates.len()), (200, 1));
    assert!(after - before < 1 << 20, "{} bytes allocated", after - before);
}
