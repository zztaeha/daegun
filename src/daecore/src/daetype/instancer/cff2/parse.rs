use alloc::string::String;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::read_u16_be;
use crate::daecore::daetype::format::cff::{resolve_fd_select, walk_cff_dict, DictFlow, DictKind, DictOp};
use crate::daecore::daetype::format::ivs::ItemVariationStore;
use crate::daecore::daetype::subsetter::{parse_cff_index, parse_cff_index_refs};

pub(crate) struct Cff2Fd<'a> {
    pub vsindex: u16,
    // Into `Cff2Font::subr_sets`: FDs naming one Subrs INDEX share it.
    pub subrs:   usize,
    pub private: &'a [u8],
}

pub(crate) struct Cff2Font<'a> {
    pub charstrings:     Vec<&'a [u8]>,
    pub global_subrs:    Vec<&'a [u8]>,
    pub fd_select:        Vec<u16>,
    pub fds:              Vec<Cff2Fd<'a>>,
    pub subr_sets:        Vec<Vec<&'a [u8]>>,
    pub vstore:            Option<ItemVariationStore>,
    pub font_matrix_raw:  Option<Vec<u8>>,
}

struct Cff2TopDict {
    font_matrix_raw: Option<Vec<u8>>,
    charstrings_off: usize,
    fd_array_off:    usize,
    fd_select_off:   Option<usize>,
    vstore_off:      Option<usize>,
}

pub(crate) fn parse_cff2(cff2: &[u8]) -> Result<Cff2Font<'_>, String> {
    if cff2.len() < 5 { return Err("CFF2: header too short".into()); }
    let header_size     = cff2[2] as usize;
    let top_dict_length = read_u16_be(cff2, 3).ok_or("CFF2: header truncated")? as usize;
    let top_dict_end    = header_size.checked_add(top_dict_length).ok_or("CFF2: Top DICT length overflow")?;
    if top_dict_end > cff2.len() {
        return Err("CFF2: Top DICT out of bounds".into());
    }
    let top_dict = parse_cff2_top_dict(&cff2[header_size..top_dict_end])?;

    let (global_subrs, _) = parse_cff_index_refs(cff2, top_dict_end, true)?;

    let (charstrings, _) = parse_cff_index_refs(cff2, top_dict.charstrings_off, true)?;
    let n_glyphs = charstrings.len();

    let (fd_dicts, _) = parse_cff_index(cff2, top_dict.fd_array_off, true)?;
    if fd_dicts.is_empty() { return Err("CFF2: empty FDArray".into()); }

    let fd_select = match top_dict.fd_select_off {
        Some(off) => resolve_fd_select(cff2, off, n_glyphs)?,
        None => alloc::vec![0u16; n_glyphs],
    };

    // An FD no glyph selects is never drawn, and each distinct Subrs INDEX is parsed once, so the
    // work stays within what the glyphs and the table's bytes can name.
    let reachable = fd_select.iter().copied().max().map_or(1, |m| usize::from(m) + 1).min(fd_dicts.len());
    let mut fds = Vec::with_capacity(reachable);
    let mut subr_sets: Vec<Vec<&[u8]>> = alloc::vec![Vec::new()];
    let mut by_offset: BTreeMap<usize, usize> = BTreeMap::new();
    for fd_dict in &fd_dicts[..reachable] {
        let (priv_size, priv_off) = find_private_dict_ptr(fd_dict);
        let private = priv_off.checked_add(priv_size).and_then(|end| cff2.get(priv_off..end)).filter(|_| priv_off > 0);
        let (vsindex, subrs_offset) = private.map_or((0, 0), parse_cff2_private_dict);

        let subrs = match priv_off.checked_add(subrs_offset).filter(|&abs| subrs_offset > 0 && abs < cff2.len()) {
            Some(abs) => *by_offset.entry(abs).or_insert_with(|| {
                subr_sets.push(parse_cff_index_refs(cff2, abs, true).map(|(subrs, _)| subrs).unwrap_or_default());
                subr_sets.len() - 1
            }),
            None => 0,
        };
        fds.push(Cff2Fd { vsindex, subrs, private: private.unwrap_or_default() });
    }

    let vstore = match top_dict.vstore_off {
        Some(off) => Some(crate::daecore::daetype::format::ivs::parse_item_variation_store(cff2, off + 2)?),
        None => None,
    };

    Ok(Cff2Font {
        charstrings,
        global_subrs,
        fd_select,
        fds,
        subr_sets,
        vstore,
        font_matrix_raw: top_dict.font_matrix_raw,
    })
}

fn parse_cff2_top_dict(dict: &[u8]) -> Result<Cff2TopDict, String> {
    let mut charstrings_off: Option<usize> = None;
    let mut fd_array_off:    Option<usize> = None;
    let mut fd_select_off:   Option<usize> = None;
    let mut vstore_off:      Option<usize> = None;
    let mut font_matrix_raw: Option<Vec<u8>> = None;

    walk_cff_dict(dict, DictKind::Cff2, |op, operands, operand_start, op_off| {
        match op {
            DictOp::Escaped(36) => { if let Some(&v) = operands.last() && v >= 0 { fd_array_off  = Some(v as usize); } }
            DictOp::Escaped(37) => { if let Some(&v) = operands.last() && v >= 0 { fd_select_off = Some(v as usize); } }
            DictOp::Escaped(7) => {
                let mut raw = dict[operand_start..op_off].to_vec();
                raw.extend_from_slice(&[12, 7]);
                font_matrix_raw = Some(raw);
            }
            DictOp::Single(17) => { if let Some(&v) = operands.last() && v >= 0 { charstrings_off = Some(v as usize); } }
            DictOp::Single(24) => { if let Some(&v) = operands.last() && v >= 0 { vstore_off = Some(v as usize); } }
            _ => {}
        }
        DictFlow::Continue
    });

    Ok(Cff2TopDict {
        font_matrix_raw,
        charstrings_off: charstrings_off.ok_or("CFF2 Top DICT: missing CharStrings offset")?,
        fd_array_off:    fd_array_off.ok_or("CFF2 Top DICT: missing FDArray offset")?,
        fd_select_off,
        vstore_off,
    })
}

fn find_private_dict_ptr(fd_dict: &[u8]) -> (usize, usize) {
    let mut priv_size = 0usize;
    let mut priv_off  = 0usize;
    walk_cff_dict(fd_dict, DictKind::Cff2, |op, operands, _, _| {
        if op == DictOp::Single(18) && operands.len() >= 2 {
            let sz = operands[operands.len() - 2];
            let po = operands[operands.len() - 1];
            if sz >= 0 && po >= 0 {
                priv_size = sz as usize;
                priv_off  = po as usize;
            }
        }
        DictFlow::Continue
    });
    (priv_size, priv_off)
}

fn parse_cff2_private_dict(private_dict: &[u8]) -> (u16, usize) {
    let mut vsindex: u16 = 0;
    let mut subrs_offset: usize = 0;
    walk_cff_dict(private_dict, DictKind::Cff2, |op, operands, _, _| {
        match op {
            DictOp::Single(19) => { if let Some(&v) = operands.last() { subrs_offset = if v > 0 { v as usize } else { 0 }; } }
            DictOp::Single(22) => { if let Some(&v) = operands.last() { vsindex = v.max(0) as u16; } }
            _ => {}
        }
        DictFlow::Continue
    });
    (vsindex, subrs_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_dict_with_blended_hints_still_names_its_subroutines() {
        let private = [139, 22, 129, 149, 140, 141, 141, 23, 6, 247, 0, 19];
        assert_eq!(parse_cff2_private_dict(&private), (0, 108));
    }

    // A CFF2 whose `fds` FD DICTs all name one Private DICT and its Subrs INDEX of 1,000 entries,
    // with FDSelect sending both glyphs to FD `selected`.
    fn many_fds(fds: usize, selected: u8) -> Vec<u8> {
        let int = |v: usize| { let mut b = alloc::vec![29]; b.extend((v as u32).to_be_bytes()); b };
        let index = |items: &[Vec<u8>]| {
            let mut b = (items.len() as u32).to_be_bytes().to_vec();
            b.push(4);
            let mut at = 1u32;
            b.extend(at.to_be_bytes());
            for item in items { at += item.len() as u32; b.extend(at.to_be_bytes()); }
            items.iter().for_each(|i| b.extend(i));
            b
        };
        let top_len = 6 + 7 + 7;
        let global = index(&[]);
        let charstrings = index(&[alloc::vec![], alloc::vec![]]);
        let fd_select_at = 5 + top_len + global.len() + charstrings.len();
        let fd_select = alloc::vec![3, 0, 1, 0, 0, selected, 0, 2];
        let fd_array_at = fd_select_at + fd_select.len();
        let fd_array_len = index(&alloc::vec![alloc::vec![0; 11]; fds]).len();
        let private_at = fd_array_at + fd_array_len;
        let mut private = int(6);
        private.push(19);
        let fd = { let mut d = int(private.len()); d.extend(int(private_at)); d.push(18); d };
        let mut out = alloc::vec![2, 0, 5, 0, top_len as u8];
        out.extend(int(5 + top_len + global.len()));
        out.push(17);
        out.extend(int(fd_array_at));
        out.extend([12, 36]);
        out.extend(int(fd_select_at));
        out.extend([12, 37]);
        out.extend(global);
        out.extend(charstrings);
        out.extend(fd_select);
        out.extend(index(&alloc::vec![fd; fds]));
        out.extend(private);
        out.extend(index(&alloc::vec![alloc::vec![11]; 1000]));
        out
    }

    #[test]
    fn fds_sharing_one_subrs_index_parse_it_once() {
        let font = many_fds(200, 199);
        let cff2 = parse_cff2(&font).expect("parses");
        assert_eq!((cff2.fds.len(), cff2.subr_sets.len(), cff2.subr_sets[1].len()), (200, 2, 1000));
        assert!(cff2.fds.iter().all(|fd| fd.subrs == 1));

        let font = many_fds(200, 0);
        assert_eq!(parse_cff2(&font).expect("parses").fds.len(), 1, "no glyph selects the other 199");
    }
}
