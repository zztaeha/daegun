use super::*;
use alloc::string::ToString;
use crate::daecore::daetype::subsetter::cff::build;

fn encode_cff_index(objects: &[Vec<u8>]) -> Vec<u8> {
    build::encode_cff_index(objects).expect("an INDEX")
}

fn int(v: usize) -> Vec<u8> {
    [&[29u8][..], &(v as i32).to_be_bytes()].concat()
}

// A CID-keyed CFF of two empty glyphs: this FDSelect, `n_fds` FD dicts of 11 bytes each, which
// `fd` writes given where the Private DICTs start, and then `privates`.
fn cid_cff(fdselect: &[u8], n_fds: usize, fd: impl Fn(usize, usize) -> Vec<u8>, privates: Vec<u8>) -> Vec<u8> {
    cid_cff_of(&[vec![14], vec![14]], fdselect, n_fds, fd, privates)
}

// The same, of these glyphs.
fn cid_cff_of(
    glyphs: &[Vec<u8>],
    fdselect: &[u8],
    n_fds: usize,
    fd: impl Fn(usize, usize) -> Vec<u8>,
    privates: Vec<u8>,
) -> Vec<u8> {
    let name = encode_cff_index(&[b"T".to_vec()]);
    let empty = [0u8, 0];
    let charstrings = encode_cff_index(glyphs);
    let top_len = encode_cff_index(&[vec![0; 37]]).len();
    let fdselect_at = 4 + name.len() + top_len + 2 * empty.len();
    let charstrings_at = fdselect_at + fdselect.len();
    let fdarray_at = charstrings_at + charstrings.len();
    let private_at = fdarray_at + encode_cff_index(&vec![vec![0; 11]; n_fds]).len();
    let top = [int(0), int(0), int(0), vec![12, 30], int(charstrings_at), vec![17], int(fdarray_at), vec![12, 36],
               int(fdselect_at), vec![12, 37]].concat();
    let fds: Vec<Vec<u8>> = (0..n_fds).map(|i| fd(i, private_at)).collect();
    [vec![1u8, 0, 4, 2], name, encode_cff_index(&[top]), empty.to_vec(), empty.to_vec(), fdselect.to_vec(),
     charstrings, encode_cff_index(&fds), privates].concat()
}

const ONE_RANGE: [u8; 8] = [3, 0, 1, 0, 0, 0, 0, 2];

// 1,000 FDs that all name one Private DICT and its 10,000-byte Subrs.
#[test]
fn a_private_dict_shared_by_fds_is_written_once() {
    let privates = [[int(6), vec![19]].concat(), encode_cff_index(&[vec![11u8; 10_000]])].concat();
    let cff = cid_cff(&ONE_RANGE, 1_000, |_, at| [int(6), int(at), vec![18]].concat(), privates);
    let out = subset_cff(&cff, &[0, 1]).expect("a subset CFF");
    assert!(out.font.len() <= 2 * cff.len(), "a {}-byte CFF subset to {} bytes", cff.len(), out.font.len());
}

// 1,000 FDs naming one Private DICT at 1,000 lengths: each is distinct, and each carries the padding
// to its Subrs 5,000 bytes on and the 10,000-byte Subrs, 15 MB in all.
#[test]
fn private_dicts_past_the_subset_budget_are_refused() {
    let mut private = [int(5_000), vec![19], [139u8, 10].repeat(1_000)].concat();
    private.resize(5_000, 0);
    let privates = [private, encode_cff_index(&[vec![11u8; 10_000]])].concat();
    let cff = cid_cff(&ONE_RANGE, 1_000, |k, at| [int(6 + 2 * k), int(at), vec![18]].concat(), privates);
    assert!(subset_cff(&cff, &[0, 1]).is_err(), "15 MB of Private DICTs were written");
}

// A CFF of these glyphs keyed by name, with this Private DICT and these bytes after it.
fn named_cff(glyphs: &[Vec<u8>], private: &[u8], after: &[u8]) -> Vec<u8> {
    let name = encode_cff_index(&[b"T".to_vec()]);
    let empty = [0u8, 0];
    let charstrings = encode_cff_index(glyphs);
    let charstrings_at = 4 + name.len() + encode_cff_index(&[vec![0; 17]]).len() + 2 * empty.len();
    let private_at = charstrings_at + charstrings.len();
    let top = [int(charstrings_at), vec![17], int(private.len()), int(private_at), vec![18]].concat();
    [vec![1u8, 0, 4, 2], name, encode_cff_index(&[top]), empty.to_vec(), empty.to_vec(), charstrings,
     private.to_vec(), after.to_vec()].concat()
}

// A glyph's bounds as drawn.
fn bounds(cff: &[u8], gid: u32) -> Result<Option<(f64, f64, f64, f64)>, crate::daecore::Error> {
    use crate::daecore::daetype::outline::{outline_cff_glyph_with, path::TightBounds};
    let mut pen = TightBounds::default();
    outline_cff_glyph_with(&CffOutlines::parse(cff)?, cff, gid, &mut pen)?;
    Ok(pen.finish().map(|b| (b.x_min, b.y_min, b.x_max, b.y_max)))
}

const CALLS_SUBR_0: [u8; 6] = [139, 139, 21, 32, 10, 14];

// A Private DICT naming Subrs twice: the last names them, as in CFF2, in the font and in its subsets.
#[test]
fn a_private_dicts_last_subrs_operator_wins() {
    let line = |dx: u8| encode_cff_index(&[vec![139 + dx, 139, 5, 11]]);
    let (first, last) = (line(10), line(20));
    let private = [int(12), vec![19], int(12 + first.len()), vec![19]].concat();
    let cff = named_cff(&[vec![14], CALLS_SUBR_0.to_vec()], &private, &[first, last].concat());
    assert_eq!(bounds(&cff, 1), Ok(Some((0.0, 0.0, 20.0, 0.0))));
    for out in [subset_cff(&cff, &[1]), subset_cff_compacting(&cff, &[1])] {
        assert_eq!(bounds(&out.expect("a subset").font, 1), Ok(Some((0.0, 0.0, 20.0, 0.0))));
    }
}

// A Private DICT whose size takes in its Subrs: they're its local subroutines all the same, so a
// subset keeps only the one its glyph calls.
#[test]
fn subrs_inside_their_private_dict_are_subset_like_any_others() {
    let subrs = [vec![149u8, 139, 5, 11], vec![159, 139, 5, 11]];
    let private = [int(6), vec![19], encode_cff_index(&subrs)].concat();
    let cff = named_cff(&[vec![14], vec![139, 139, 21, 33, 10, 14], CALLS_SUBR_0.to_vec()], &private, &[]);
    let out = subset_cff(&cff, &[1]).expect("a subset").font;
    assert_eq!(subrs_of(&out).1, [vec![vec![11], subrs[1].clone()]]);
    assert_eq!(bounds(&out, 1), bounds(&cff, 1));
}

// Subrs that don't read as an INDEX, and a Private DICT past the table's end, are none: only the
// glyphs that call a subroutine fail, in the font and in its subsets.
#[test]
fn unreadable_subrs_cost_only_the_glyphs_that_call_them() {
    let glyphs = [vec![14], vec![139, 139, 21, 149, 139, 5, 14], CALLS_SUBR_0.to_vec()];
    let bad_index = named_cff(&glyphs, &[int(6), vec![19]].concat(), &[0, 1, 9, 0, 0]);
    let mut cut_private = named_cff(&glyphs, &[int(6), vec![19]].concat(), &[]);
    cut_private.truncate(cut_private.len() - 3);
    for cff in [bad_index, cut_private] {
        assert!(bounds(&cff, 1).is_ok_and(|b| b.is_some()) && bounds(&cff, 2).is_err());
        for out in [subset_cff(&cff, &[1, 2]), subset_cff_compacting(&cff, &[1, 2])] {
            assert_eq!(bounds(&out.expect("a subset").font, 1), bounds(&cff, 1));
        }
    }
}

// FDSelect format 4 is CFF2's, so in a CFF table it fails its check: each glyph takes FD 0, drawn and subset alike.
#[test]
fn an_fdselect_in_cff2s_format_leaves_each_glyph_in_fd_0() {
    let fdselect = [4u8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let glyphs = [vec![14], vec![139, 139, 21, 149, 139, 5, 14]];
    let cff = cid_cff_of(&glyphs, &fdselect, 1, |_, at| [int(2), int(at), vec![18]].concat(), vec![139, 139]);
    assert_eq!(bounds(&cff, 1), Ok(Some((0.0, 0.0, 10.0, 0.0))));
    for out in [subset_cff(&cff, &[1]), subset_cff_compacting(&cff, &[1])] {
        assert_eq!(bounds(&out.expect("a subset").font, 1), bounds(&cff, 1));
    }
}

// A CFF of these glyphs keyed by name, its one Private DICT 6 bytes long and its Subrs `gap` bytes on.
fn plain_cff(glyphs: &[Vec<u8>], gap: usize, subrs: &[Vec<u8>]) -> Vec<u8> {
    named_cff(glyphs, &[int(6 + gap), vec![19]].concat(), &[vec![0; gap], encode_cff_index(subrs)].concat())
}

// Subrs 5,000 bytes past a 6-byte Private DICT follow it directly in the subset, named or CID-keyed,
// the gap not copied as zeros.
#[test]
fn a_private_dicts_subrs_follow_it_without_a_gap() {
    let subrs = [vec![140u8, 139, 5, 11]];
    let plain = plain_cff(&[vec![14], vec![139, 139, 21, 32, 10, 14]], 4_994, &subrs);
    let privates = [int(5_000), vec![19], vec![0; 4_994], encode_cff_index(&subrs)].concat();
    let cid = cid_cff_of(
        &[vec![14], vec![139, 139, 21, 32, 10, 14]],
        &ONE_RANGE,
        1,
        |_, at| [int(6), int(at), vec![18]].concat(),
        privates,
    );
    for cff in [plain, cid] {
        let out = subset_cff(&cff, &[1]).expect("a subset CFF").font;
        assert!(out.len() + 4_000 < cff.len(), "a {}-byte CFF subset to {} bytes", cff.len(), out.len());
        assert_eq!(subrs_of(&out).1, [subrs.to_vec()]);
    }
}

// Glyph 1 calls subroutine 0 under FD 0 and glyph 2 subroutine 1 under FD 1, both FDs naming one
// Private DICT: a subset of glyph 2 keeps subroutine 1, called through FD 1 though written under FD 0.
#[test]
fn fds_sharing_subroutines_keep_what_any_of_them_calls() {
    let glyphs = [vec![14], vec![139, 139, 21, 32, 10, 14], vec![139, 139, 21, 33, 10, 14]];
    let fdselect = [3u8, 0, 2, 0, 0, 0, 0, 2, 1, 0, 3];
    let subrs = [vec![140u8, 139, 5, 11], vec![139, 140, 5, 11]];
    let privates = [int(6), vec![19], encode_cff_index(&subrs)].concat();
    let cff = cid_cff_of(&glyphs, &fdselect, 2, |_, at| [int(6), int(at), vec![18]].concat(), privates);
    let (_, local) = subrs_of(&subset_cff(&cff, &[2]).expect("a subset").font);
    assert_eq!(local, [vec![vec![11], subrs[1].clone()], vec![vec![11], subrs[1].clone()]]);
}

// A format 3 FDSelect whose second range starts past the sentinel: the drawing path skips it, and
// so must the compacting subset rather than refusing the font as an unknown format.
#[test]
fn an_fdselect_range_out_of_order_is_skipped_as_drawing_skips_it() {
    let fdselect = [3u8, 0, 2, 0, 0, 0, 0, 5, 0, 0, 2];
    let cff = cid_cff(&fdselect, 1, |_, at| [int(2), int(at), vec![18]].concat(), vec![139, 139]);
    assert!(subset_cff(&cff, &[1]).is_ok());
    assert!(subset_cff_compacting(&cff, &[1]).is_ok());
}

fn fixture_cff(rel: &str) -> Vec<u8> {
    let path = alloc::format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).expect("a fixture");
    crate::daecore::daetype::decoder::extract_ttf_tables(&bytes)
        .expect("it parses")
        .get("CFF ")
        .expect("CFF")
        .to_owned_vec()
}

// Each Top DICT and FD DICT entry but the moved offsets: SIDs as their strings, the rest as bytes.
// The SID operators are listed apart from `sid_operands`, which is under test.
fn dict_entries(cff: &[u8]) -> Vec<(DictOp, Vec<String>)> {
    let sids = |op| match op {
        DictOp::Single(0..=4) | DictOp::Escaped(0 | 21 | 22 | 38) => 1,
        DictOp::Escaped(30) => 2,
        _ => 0,
    };
    let header = read_header(cff).expect("a header");
    let (strings, _) = parse_cff_index_refs(cff, header.strings, false).expect("strings");
    let fds = header.top.fd_array.map_or(Vec::new(), |at| parse_cff_index_refs(cff, at, false).expect("FDArray").0);
    let mut out = Vec::new();
    for dict in core::iter::once(header.top_dict).chain(fds) {
        walk_cff_dict(dict, DictKind::Cff1, |op, operands, start, at| {
            if !matches!(op, DictOp::Single(15 | 17 | 18) | DictOp::Escaped(36 | 37)) {
                let text: Vec<String> = match sids(op) {
                    0 => vec![alloc::format!("{:?}", &dict[start..at])],
                    n => operands.iter().take(n).map(|&v| match u16::try_from(v) {
                        Ok(sid) if sid >= N_STD_STRINGS => {
                            String::from_utf8_lossy(strings[usize::from(sid - N_STD_STRINGS)]).to_string()
                        }
                        _ => v.to_string(),
                    }).collect(),
                };
                out.push((op, text));
            }
        });
    }
    out
}

// FontBBox, the names, the underline, ROS, CIDCount and each FD's FontName all stay, the SIDs
// among them renumbered with the strings a compacting subset keeps.
#[test]
fn every_dict_entry_survives_both_subsets() {
    for rel in ["test-fixtures/cff-subrs.otf", "test-fixtures/cff-cid.otf", "stix-two-math/STIX2Math.otf"] {
        let cff = fixture_cff(rel);
        let want = dict_entries(&cff);
        assert!(want.len() >= 4, "{rel} has {} entries", want.len());
        for out in [subset_cff(&cff, &[0, 2, 5]), subset_cff_compacting(&cff, &[0, 2, 5])] {
            assert_eq!(dict_entries(&out.expect("a subset").font), want, "{rel}");
        }
    }
}

// The global subroutines and each FD's local ones, or the one Private DICT's.
fn subrs_of(cff: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<Vec<u8>>>) {
    let header = read_header(cff).expect("a header");
    let (global, _) = parse_cff_index_refs(cff, header.gsubrs, false).expect("global subrs");
    let privates = match header.top.fd_array {
        Some(at) => parse_cff_index_refs(cff, at, false).expect("FDArray").0.iter()
            .map(|fd| read_dict(fd, DictKind::Cff1).private)
            .collect(),
        None => vec![header.top.private.clone()],
    };
    let local = privates.into_iter().map(|at| match read_private(cff, at, DictKind::Cff1).subrs {
        None => Vec::new(),
        Some(at) => parse_cff_index_refs(cff, at, false).expect("local subrs").0.iter().map(|s| s.to_vec()).collect(),
    }).collect();
    (global.iter().map(|s| s.to_vec()).collect(), local)
}

// Glyph 2 calls local subroutine 0, and glyphs 7 and 10 local 1 and global 0: a subset keeps those
// its glyphs call and cuts the rest to a bare return, each index in place.
#[test]
fn a_subset_keeps_only_the_subroutines_its_glyphs_call() {
    let cff = fixture_cff("test-fixtures/cff-subrs.otf");
    let (global, local) = subrs_of(&cff);
    let stub = vec![11u8];
    for (glyph, want) in [(2, (vec![stub.clone()], vec![vec![local[0][0].clone(), stub.clone()]])),
                          (10, (global.clone(), vec![vec![stub.clone(), stub.clone()]]))] {
        for out in [subset_cff(&cff, &[glyph]), subset_cff_compacting(&cff, &[glyph])] {
            assert_eq!(subrs_of(&out.expect("a subset").font), want, "glyph {glyph}");
        }
    }
}

// Glyphs alternate between two FDs: a compacted subset of FD 0's glyphs keeps that FD alone, and a
// plain one keeps both, FD 1's subroutines cut to a bare return.
#[test]
fn a_cid_subset_keeps_the_fds_and_subroutines_its_glyphs_use() {
    let cff = fixture_cff("test-fixtures/cff-cid.otf");
    let (_, local) = subrs_of(&cff);
    assert_eq!(subrs_of(&subset_cff_compacting(&cff, &[2, 4]).expect("a subset").font).1, [local[0].clone()]);
    let plain = subrs_of(&subset_cff(&cff, &[2, 4]).expect("a subset").font).1;
    assert_eq!(plain, [local[0].clone(), vec![vec![11]; 3]]);
}

fn cids(cff: &[u8]) -> Vec<u16> {
    let header = read_header(cff).expect("a header");
    let n = parse_cff_index_refs(cff, header.charstrings, false).expect("CharStrings").0.len();
    let Charset::At(at) = header.top.charset() else { panic!("no charset") };
    let mut out = vec![0; n];
    walk_charset(cff, at, n, |gid, cid| out[usize::from(gid)] = cid).expect("it walks");
    out
}

#[test]
fn a_compacted_cid_subset_keeps_each_glyphs_cid() {
    let cff = fixture_cff("test-fixtures/cff-cid.otf");
    let out = subset_cff_compacting(&cff, &[2, 9, 10]).expect("a subset");
    assert_eq!(cids(&out.font), [0, cids(&cff)[2], 34, 125]);
}

// An Expert charset kept as charset 0 reads as ISOAdobe: space, exclam and on.
#[test]
fn a_predefined_charset_keeps_its_id() {
    let cff = fixture_cff("test-fixtures/cff-expert.otf");
    let out = subset_cff(&cff, &[2, 3]).expect("a subset");
    assert_eq!(read_header(&out.font).expect("a header").top.charset(), Charset::Predefined(1));
}
