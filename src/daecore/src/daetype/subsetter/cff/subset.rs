use super::*;
use crate::daecore::daetype::format::cff::{resolve_fd_select, walk_cff_dict, DictFlow, DictKind, DictOp};
use crate::daecore::daetype::outline::CffOutlines;
use alloc::borrow::Cow;

pub fn subset_cff(cff: &[u8], requested: &[u16]) -> Result<SubsetResult, String> {
    subset_cff_requested(cff, requested, false)
}

pub fn subset_cff_compacting(cff: &[u8], requested: &[u16]) -> Result<SubsetResult, String> {
    subset_cff_requested(cff, requested, true)
}

// .notdef, the requested glyphs, and the base and accent glyphs their seacs draw.
fn subset_cff_requested(cff: &[u8], requested: &[u16], compact: bool) -> Result<SubsetResult, String> {
    let outlines = CffOutlines::parse(cff).ok();
    let mut set = GlyphSet::new();
    let frontier: Vec<u16> = core::iter::once(0).chain(requested.iter().copied()).filter(|&g| set.insert(g)).collect();
    if let Some(outlines) = &outlines {
        close_over_seacs(outlines, cff, frontier, &mut set);
    }
    subset_cff_closed(cff, outlines.as_ref(), &set.iter().collect::<Vec<_>>(), compact)
}

const N_STD_STRINGS: u16 = 391;
const TOO_MANY: &str = "CFF: an INDEX of more than 65,535 objects";

// The operators whose operands are SIDs, ROS's first two among them.
fn sid_operands(op: DictOp) -> usize {
    match op {
        DictOp::Single(0..=4) | DictOp::Escaped(0 | 21 | 22 | 38) => 1,
        DictOp::Escaped(30) => 2,
        _ => 0,
    }
}

// The SIDs past the standard strings that a DICT names, which a compacting subset must keep.
fn dict_sids(dict: &[u8], out: &mut BTreeSet<u16>) {
    walk_cff_dict(dict, DictKind::Cff1, |op, operands, _, _| {
        for &v in operands.iter().take(sid_operands(op)) {
            if let Ok(sid) = u16::try_from(v) && sid >= N_STD_STRINGS { out.insert(sid); }
        }
        DictFlow::Continue
    });
}

// A DICT copied entry by entry, but for the offsets the caller rewrites and a custom Encoding, whose
// table is not carried. SIDs go through `sid`, which a compacting subset renumbers.
fn copy_dict(dict: &[u8], sid: &dyn Fn(u16) -> u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(dict.len());
    walk_cff_dict(dict, DictKind::Cff1, |op, operands, operand_start, op_off| {
        let end = op_off + if matches!(op, DictOp::Escaped(_)) { 2 } else { 1 };
        let moved = matches!(op, DictOp::Single(15 | 17 | 18) | DictOp::Escaped(36 | 37))
            || (op == DictOp::Single(16) && operands.last().is_some_and(|&v| v > 1));
        if moved {
            return DictFlow::Continue;
        }
        let n_sids = sid_operands(op).min(operands.len());
        if operands[..n_sids].iter().any(|&v| u16::try_from(v).is_ok_and(|s| sid(s) != s)) {
            for (k, &v) in operands.iter().enumerate() {
                let v = if k < n_sids { u16::try_from(v).map_or(v, |s| i32::from(sid(s))) } else { v };
                out.extend(encode_cff_int(v));
            }
            out.extend_from_slice(&dict[op_off..end]);
        } else {
            out.extend_from_slice(&dict[operand_start..end]);
        }
        DictFlow::Continue
    });
    out
}

// A Subrs INDEX with each subroutine no kept glyph calls cut to a bare `return`: every index stays,
// and with the count the bias.
fn pruned_subrs(cff: &[u8], subrs: &[(u32, u32)], called: &[bool]) -> Result<Vec<u8>, String> {
    let refs = subrs.iter().enumerate()
        .map(|(i, &(start, end))| match called.get(i) {
            Some(true) => cff.get(start as usize..end as usize).ok_or("CFF: a subroutine past the font"),
            _ => Ok(&[11][..]),
        })
        .collect::<Result<Vec<&[u8]>, _>>()?;
    Ok(encode_cff_index_refs(&refs).ok_or(TOO_MANY)?)
}

// A Private DICT its Subrs follow directly: their offset is its own length, which counts the bytes
// that offset is written in.
fn private_before_subrs(private: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(private.len() + 6);
    walk_cff_dict(private, DictKind::Cff1, |op, _, operand_start, op_off| {
        if op != DictOp::Single(19) {
            let end = op_off + if matches!(op, DictOp::Escaped(_)) { 2 } else { 1 };
            out.extend_from_slice(&private[operand_start..end]);
        }
        DictFlow::Continue
    });
    let entries = out.len() + 1;
    let len = (1..=5).map(|width| entries + width).find(|&len| entries + encode_cff_int_short(len as i32).len() == len).unwrap_or(entries + 5);
    out.extend(encode_cff_int_short(len as i32));
    out.push(19);
    out
}

// A subset of glyphs already closed over their seacs, as `subset_cff_requested` and `cff_color_closure`
// close them, with the outlines that closure read when the font has any to read.
pub(crate) fn subset_cff_closed(cff: &[u8], outlines: Option<&CffOutlines>, closed: &[u16], compact: bool) -> Result<SubsetResult, String> {
    if cff.len() < 4 {
        return Err("CFF: file too short".into());
    }
    let hdr_size = cff[2] as usize;

    let after_name             = cff_index_end(cff, hdr_size, false)?;
    let (top_dicts, after_top) = parse_cff_index_refs(cff, after_name, false)?;
    let top_dict = top_dicts.into_iter().next()
        .ok_or("CFF: empty Top DICT INDEX")?;
    let fields = parse_top_dict(top_dict)?;

    let after_strings = cff_index_end(cff, after_top, false)?;
    let after_gsubrs  = cff_index_end(cff, after_strings, false)?;

    let name_index_bytes   = &cff[hdr_size..after_name];

    let (charstrings, _) = parse_cff_index_refs(cff, fields.charstrings_off, false)?;
    let n_glyphs = charstrings.len();

    let mut active = BTreeSet::<u16>::new();
    active.insert(0);
    for &gid in closed {
        if (gid as usize) < n_glyphs { active.insert(gid); }
    }

    let active_sorted: Vec<u16> = active.iter().copied().collect();
    let gid_map: Vec<u16> = if compact {
        let mut m = vec![0u16; *active_sorted.last().unwrap_or(&0) as usize + 1];
        for (new, &orig) in active_sorted.iter().enumerate() { m[orig as usize] = new as u16; }
        m
    } else {
        vec![]
    };

    let endchar: &[u8] = &[0x0eu8];
    let new_charstrings: Vec<&[u8]> = if compact {
        active_sorted
            .iter()
            .map(|&g| charstrings.get(g as usize).copied().unwrap_or(endchar))
            .collect()
    } else {
        (0..n_glyphs)
            .map(|gid| {
                if active.contains(&(gid as u16)) { charstrings[gid] }
                else { endchar }
            })
            .collect()
    };
    let new_charstrings_index = encode_cff_index_refs(&new_charstrings).ok_or(TOO_MANY)?;

    // Global then each FD's local subroutines, marked where a kept glyph calls them. None keeps them all,
    // as does keeping every glyph: what none of them calls the source never called either.
    let called = outlines.filter(|_| active_sorted.len() < n_glyphs)
        .and_then(|o| Some((o, o.subrs_called(cff, &active_sorted)?)));
    let gsubr_index: Cow<[u8]> = match &called {
        Some((o, (global, _))) => Cow::Owned(pruned_subrs(cff, o.global_subr_spans(), global)?),
        None => Cow::Borrowed(&cff[after_strings..after_gsubrs]),
    };

    let is_cid = fields.fd_array_off.is_some();
    let fd_dicts: Vec<&[u8]> = match fields.fd_array_off {
        Some(off) => parse_cff_index_refs(cff, off, false)?.0,
        None => Vec::new(),
    };

    // A compacting subset keeps the strings its glyph names and DICTs name, renumbered in order.
    let (charset_bytes, string_index_bytes, kept) = if compact {
        let (strings, _) = parse_cff_index_refs(cff, after_top, false)?;
        let mut sids = vec![0u16; n_glyphs];
        match fields.charset_off {
            Some(off) => {
                walk_charset(cff, off, n_glyphs, |gid, sid| {
                    if let Some(slot) = sids.get_mut(gid as usize) { *slot = sid; }
                    CharsetFlow::Continue
                })?;
            }
            None => {
                let table: &[u16] = match fields.charset_predefined {
                    1 => &expert_charsets::EXPERT_CHARSET,
                    2 => &expert_charsets::EXPERT_SUBSET_CHARSET,
                    _ => &[],
                };
                for (gid, slot) in sids.iter_mut().enumerate() {
                    *slot = if table.is_empty() { gid as u16 } else { table.get(gid).copied().unwrap_or(0) };
                }
            }
        }

        let mut needed: BTreeSet<u16> = BTreeSet::new();
        dict_sids(top_dict, &mut needed);
        fd_dicts.iter().for_each(|fd| dict_sids(fd, &mut needed));
        if !is_cid {
            needed.extend(active_sorted.iter().skip(1).map(|&g| sids[g as usize]).filter(|&sid| sid >= N_STD_STRINGS));
        }
        let kept: Vec<u16> = needed.into_iter().collect();
        let renumber = |sid: u16| kept.binary_search(&sid).map_or(sid, |i| N_STD_STRINGS + i as u16);
        let refs: Vec<&[u8]> = kept.iter()
            .map(|&sid| strings.get((sid - N_STD_STRINGS) as usize).map_or(&[][..], |v| *v))
            .collect();

        let mut b = Vec::with_capacity(1 + 2 * active_sorted.len());
        b.push(0);
        for &orig in active_sorted.iter().skip(1) {
            let sid = sids[orig as usize];
            b.extend_from_slice(&if is_cid { sid } else { renumber(sid) }.to_be_bytes());
        }
        (b, encode_cff_index_refs(&refs).ok_or(TOO_MANY)?, kept)
    } else {
        let charset = match fields.charset_off {
            Some(off) => cff[off..parse_charset_sids(cff, off, n_glyphs)?].to_vec(),
            None => Vec::new(),
        };
        (charset, cff[after_top..after_strings].to_vec(), Vec::new())
    };
    let renumber = |sid: u16| kept.binary_search(&sid).map_or(sid, |i| N_STD_STRINGS + i as u16);
    let string_index_bytes = &string_index_bytes[..];
    let top_entries = copy_dict(top_dict, &renumber);
    let base = |top_dict_len: usize| hdr_size + name_index_bytes.len() + cff_index_size(top_dict_len)
        + string_index_bytes.len() + gsubr_index.len();
    // A predefined charset keeps its id: written as 0, an Expert font would read as ISOAdobe.
    let charset_value = |base: usize| if charset_bytes.is_empty() { i32::from(fields.charset_predefined) } else { base as i32 };

    if is_cid {
        let fd_select_off = fields.fd_select_off
            .ok_or("CFF CID: missing FDSelect offset")?;
        if fields.ros.is_none() { return Err("CFF CID: missing ROS".into()); }

        let n_fds = fd_dicts.len();
        if n_fds == 0 { return Err("CFF CID: empty FDArray".into()); }

        // A compacting subset keeps only the FDs its glyphs select, renumbered in order. A glyph whose FD
        // the FDArray lacks names one past them, lacking it still.
        let (fdselect_bytes, kept_fds) = if compact {
            let per_glyph = resolve_fd_select(cff, fd_select_off, n_glyphs)?;
            let selected: BTreeSet<usize> = active_sorted.iter().map(|&g| usize::from(per_glyph[g as usize])).filter(|&fd| fd < n_fds).collect();
            let kept: Vec<usize> = selected.into_iter().collect();
            let mut b = Vec::with_capacity(1 + active_sorted.len());
            b.push(0);
            for &orig in &active_sorted {
                let fd = kept.binary_search(&usize::from(per_glyph[orig as usize])).unwrap_or(kept.len());
                b.push(u8::try_from(fd).map_err(|_| "CFF CID: an FD index past 255")?);
            }
            (b, kept)
        } else {
            (parse_fd_select_bytes(cff, fd_select_off, n_glyphs)?, (0..n_fds).collect())
        };
        let n_kept = kept_fds.len();
        let mut fd_entries = Vec::with_capacity(n_kept);
        // FDs may share a Private DICT and its Subrs: each is read once and written once, keeping the
        // subroutines any FD naming it calls, every such FD naming the first.
        let mut first_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        let mut index_end: BTreeMap<usize, usize> = BTreeMap::new();
        let mut read = 0usize;
        let mut fd_first = Vec::with_capacity(n_kept);
        // The first's Private DICT as written and the span of its Subrs, empty for the rest.
        let mut fd_privs: Vec<Vec<u8>> = Vec::with_capacity(n_kept);
        let mut fd_subrs: Vec<Option<(usize, usize)>> = Vec::with_capacity(n_kept);

        for (k, &fd) in kept_fds.iter().enumerate() {
            let fd_dict = fd_dicts[fd];
            let (priv_size, priv_off, _) = parse_fd_dict_private(fd_dict);
            let first = *first_of.entry((priv_off, priv_size)).or_insert(k);
            fd_first.push(first);
            fd_entries.push(copy_dict(fd_dict, &renumber));
            if first != k {
                fd_privs.push(Vec::new());
                fd_subrs.push(None);
                continue;
            }
            let priv_end = priv_off.saturating_add(priv_size);
            if priv_end > cff.len() {
                return Err("CFF CID: FD Private DICT out of bounds".into());
            }
            let priv_data = &cff[priv_off..priv_end];
            let subrs_rel = match parse_private_subrs_offset(priv_data) {
                r if r >= priv_size => r,
                _ => 0,
            };
            let abs = priv_off + subrs_rel;
            let subrs = if subrs_rel > 0 && abs < cff.len() {
                let end = match index_end.get(&abs) {
                    Some(&end) => end,
                    None => {
                        let end = cff_index_end(cff, abs, false)?;
                        index_end.insert(abs, end);
                        end
                    }
                };
                Some((abs, end))
            } else {
                None
            };
            read = read.saturating_add(subrs.map_or(priv_size, |(abs, end)| subrs_rel + end - abs));
            if read > super::super::subset_budget(cff.len()) {
                return Err("CFF CID: FD Private DICTs past the subset budget".into());
            }
            fd_privs.push(if subrs.is_some() { private_before_subrs(priv_data) } else { priv_data.to_vec() });
            fd_subrs.push(subrs);
        }
        let fd_lsubrs = (0..n_kept)
            .map(|k| Ok(match (fd_subrs[k], &called) {
                (None, _) => Cow::Borrowed(&[][..]),
                (Some((abs, end)), None) => Cow::Borrowed(&cff[abs..end]),
                (Some(_), Some((o, (_, local)))) => {
                    let spans = o.local_subr_spans(kept_fds[k]);
                    let mut union = vec![false; spans.len()];
                    for j in (0..n_kept).filter(|&j| fd_first[j] == k) {
                        let marks = local.get(kept_fds[j]).map_or(&[][..], |m| &m[..]);
                        union.iter_mut().zip(marks).for_each(|(u, &m)| *u |= m);
                    }
                    Cow::Owned(pruned_subrs(cff, spans, &union)?)
                }
            }))
            .collect::<Result<Vec<Cow<[u8]>>, String>>()?;

        // Each FD DICT is its copied entries and a Private operator of two 5-byte ints.
        let fd_placeholder: Vec<Vec<u8>> = fd_entries.iter().map(|e| vec![0u8; e.len() + 11]).collect();
        let fdarray_size = encode_cff_index(&fd_placeholder).ok_or(TOO_MANY)?.len();

        let top_dict_data_len = top_entries.len() + 26;
        let base = base(top_dict_data_len);
        let new_fdselect_off    = base + charset_bytes.len();
        let new_charstrings_off = new_fdselect_off + fdselect_bytes.len();
        let new_fdarray_off     = new_charstrings_off + new_charstrings_index.len();

        let mut fd_new_priv_offs = Vec::with_capacity(n_kept);
        let mut cur = new_fdarray_off + fdarray_size;
        for k in 0..n_kept {
            if fd_first[k] != k {
                fd_new_priv_offs.push(fd_new_priv_offs[fd_first[k]]);
                continue;
            }
            fd_new_priv_offs.push(cur);
            cur += fd_privs[k].len() + fd_lsubrs[k].len();
        }
        let new_priv_off = |k: usize| i32::try_from(fd_new_priv_offs[k]).map_err(|_| "CFF CID: offset past 2 GB");

        let fd_dict_entries: Vec<Vec<u8>> = (0..n_kept)
            .map(|k| {
                let mut d = fd_entries[k].clone();
                d.extend(encode_cff_int(fd_privs[fd_first[k]].len() as i32));
                d.extend(encode_cff_int(new_priv_off(k)?));
                d.push(18);
                Ok(d)
            })
            .collect::<Result<_, &str>>()?;
        let new_fdarray_index = encode_cff_index(&fd_dict_entries).ok_or(TOO_MANY)?;
        debug_assert_eq!(new_fdarray_index.len(), fdarray_size);

        let mut top_dict_data = top_entries;
        top_dict_data.extend(encode_cff_int(charset_value(base)));
        top_dict_data.push(15);
        top_dict_data.extend(encode_cff_int(new_charstrings_off as i32));
        top_dict_data.push(17);
        top_dict_data.extend(encode_cff_int(new_fdarray_off as i32));
        top_dict_data.extend_from_slice(&[12u8, 36u8]);
        top_dict_data.extend(encode_cff_int(new_fdselect_off as i32));
        top_dict_data.extend_from_slice(&[12u8, 37u8]);

        let new_top_dict_index = encode_cff_index(&[top_dict_data]).ok_or(TOO_MANY)?;
        debug_assert_eq!(new_top_dict_index.len(), cff_index_size(top_dict_data_len));

        let mut out = Vec::new();
        out.extend_from_slice(&cff[..hdr_size]);
        out.extend_from_slice(name_index_bytes);
        out.extend_from_slice(&new_top_dict_index);
        out.extend_from_slice(string_index_bytes);
        out.extend_from_slice(&gsubr_index);
        out.extend_from_slice(&charset_bytes);
        out.extend_from_slice(&fdselect_bytes);
        out.extend_from_slice(&new_charstrings_index);
        out.extend_from_slice(&new_fdarray_index);
        for k in (0..n_kept).filter(|&k| fd_first[k] == k) {
            out.extend_from_slice(&fd_privs[k]);
            out.extend_from_slice(&fd_lsubrs[k]);
        }

        return Ok(SubsetResult { ttf: out, gid_map });
    }

    let priv_end = fields.private_off.saturating_add(fields.private_size);
    if priv_end > cff.len() {
        return Err("CFF: Private DICT out of bounds".into());
    }
    let private_bytes = &cff[fields.private_off..priv_end];
    let subrs_rel = match parse_private_subrs_offset(private_bytes) {
        r if r >= fields.private_size => r,
        _ => 0,
    };
    let subrs_abs = fields.private_off + subrs_rel;
    let (private, local_subrs) = if subrs_rel > 0 && subrs_abs < cff.len() {
        let local_subrs: Cow<[u8]> = match &called {
            Some((o, (_, local))) => Cow::Owned(pruned_subrs(cff, o.local_subr_spans(0), local.first().map_or(&[][..], |m| &m[..]))?),
            None => Cow::Borrowed(&cff[subrs_abs..cff_index_end(cff, subrs_abs, false)?]),
        };
        (private_before_subrs(private_bytes), local_subrs)
    } else {
        (private_bytes.to_vec(), Cow::Borrowed(&[][..]))
    };

    let top_dict_data_len = top_entries.len() + 23;
    let base = base(top_dict_data_len);
    let new_charstrings_off = base + charset_bytes.len();
    let new_private_off     = new_charstrings_off + new_charstrings_index.len();

    let mut top_dict_data = top_entries;
    top_dict_data.extend(encode_cff_int(charset_value(base)));
    top_dict_data.push(15);
    top_dict_data.extend(encode_cff_int(new_charstrings_off as i32));
    top_dict_data.push(17);
    top_dict_data.extend(encode_cff_int(private.len() as i32));
    top_dict_data.extend(encode_cff_int(new_private_off as i32));
    top_dict_data.push(18);

    let new_top_dict_index = encode_cff_index(&[top_dict_data]).ok_or(TOO_MANY)?;
    debug_assert_eq!(new_top_dict_index.len(), cff_index_size(top_dict_data_len));

    let mut out = Vec::new();
    out.extend_from_slice(&cff[..hdr_size]);
    out.extend_from_slice(name_index_bytes);
    out.extend_from_slice(&new_top_dict_index);
    out.extend_from_slice(string_index_bytes);
    out.extend_from_slice(&gsubr_index);
    out.extend_from_slice(&charset_bytes);
    out.extend_from_slice(&new_charstrings_index);
    out.extend_from_slice(&private);
    out.extend_from_slice(&local_subrs);

    Ok(SubsetResult { ttf: out, gid_map })
}

#[cfg(test)]
mod tests {
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
    fn cid_cff_of(glyphs: &[Vec<u8>], fdselect: &[u8], n_fds: usize, fd: impl Fn(usize, usize) -> Vec<u8>, privates: Vec<u8>) -> Vec<u8> {
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
        assert!(out.ttf.len() <= 2 * cff.len(), "a {}-byte CFF subset to {} bytes", cff.len(), out.ttf.len());
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

    // A CFF of these glyphs keyed by name, its one Private DICT 6 bytes long and its Subrs `gap` bytes on.
    fn plain_cff(glyphs: &[Vec<u8>], gap: usize, subrs: &[Vec<u8>]) -> Vec<u8> {
        let name = encode_cff_index(&[b"T".to_vec()]);
        let empty = [0u8, 0];
        let charstrings = encode_cff_index(glyphs);
        let charstrings_at = 4 + name.len() + encode_cff_index(&[vec![0; 17]]).len() + 2 * empty.len();
        let private_at = charstrings_at + charstrings.len();
        let top = [int(charstrings_at), vec![17], int(6), int(private_at), vec![18]].concat();
        [vec![1u8, 0, 4, 2], name, encode_cff_index(&[top]), empty.to_vec(), empty.to_vec(), charstrings,
         int(6 + gap), vec![19], vec![0; gap], encode_cff_index(subrs)].concat()
    }

    // Subrs 5,000 bytes past a 6-byte Private DICT follow it directly in the subset, named or CID-keyed,
    // the gap not copied as zeros.
    #[test]
    fn a_private_dicts_subrs_follow_it_without_a_gap() {
        let subrs = [vec![140u8, 139, 5, 11]];
        let plain = plain_cff(&[vec![14], vec![139, 139, 21, 32, 10, 14]], 4_994, &subrs);
        let privates = [int(5_000), vec![19], vec![0; 4_994], encode_cff_index(&subrs)].concat();
        let cid = cid_cff_of(&[vec![14], vec![139, 139, 21, 32, 10, 14]], &ONE_RANGE, 1, |_, at| [int(6), int(at), vec![18]].concat(), privates);
        for cff in [plain, cid] {
            let out = subset_cff(&cff, &[1]).expect("a subset CFF").ttf;
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
        let (_, local) = subrs_of(&subset_cff(&cff, &[2]).expect("a subset").ttf);
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
        crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("it parses").get("CFF ").expect("CFF").to_owned_vec()
    }

    // Each Top DICT and FD DICT entry but the moved offsets: SIDs as their strings, the rest as bytes.
    // The SID operators are listed apart from `sid_operands`, which is under test.
    fn dict_entries(cff: &[u8]) -> Vec<(DictOp, Vec<String>)> {
        let sids = |op| match op {
            DictOp::Single(0..=4) | DictOp::Escaped(0 | 21 | 22 | 38) => 1,
            DictOp::Escaped(30) => 2,
            _ => 0,
        };
        let (_, after_name) = parse_cff_index_refs(cff, usize::from(cff[2]), false).expect("names");
        let (tops, after_top) = parse_cff_index_refs(cff, after_name, false).expect("Top DICT");
        let (strings, _) = parse_cff_index_refs(cff, after_top, false).expect("strings");
        let fields = parse_top_dict(tops[0]).expect("Top DICT fields");
        let fds = fields.fd_array_off.map_or(Vec::new(), |at| parse_cff_index_refs(cff, at, false).expect("FDArray").0);
        let mut out = Vec::new();
        for dict in core::iter::once(tops[0]).chain(fds) {
            walk_cff_dict(dict, DictKind::Cff1, |op, operands, start, at| {
                if !matches!(op, DictOp::Single(15 | 17 | 18) | DictOp::Escaped(36 | 37)) {
                    let text: Vec<String> = match sids(op) {
                        0 => vec![alloc::format!("{:?}", &dict[start..at])],
                        n => operands.iter().take(n).map(|&v| match u16::try_from(v) {
                            Ok(sid) if sid >= N_STD_STRINGS => String::from_utf8_lossy(strings[usize::from(sid - N_STD_STRINGS)]).to_string(),
                            _ => v.to_string(),
                        }).collect(),
                    };
                    out.push((op, text));
                }
                DictFlow::Continue
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
                assert_eq!(dict_entries(&out.expect("a subset").ttf), want, "{rel}");
            }
        }
    }

    // The global subroutines and each FD's local ones, or the one Private DICT's.
    fn subrs_of(cff: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<Vec<u8>>>) {
        let (_, after_name) = parse_cff_index_refs(cff, usize::from(cff[2]), false).expect("names");
        let (tops, after_top) = parse_cff_index_refs(cff, after_name, false).expect("Top DICT");
        let (_, after_strings) = parse_cff_index_refs(cff, after_top, false).expect("strings");
        let (global, _) = parse_cff_index_refs(cff, after_strings, false).expect("global subrs");
        let fields = parse_top_dict(tops[0]).expect("fields");
        let privates: Vec<(usize, usize)> = match fields.fd_array_off {
            Some(at) => parse_cff_index_refs(cff, at, false).expect("FDArray").0.iter()
                .map(|fd| { let (size, off, _) = parse_fd_dict_private(fd); (off, size) })
                .collect(),
            None => vec![(fields.private_off, fields.private_size)],
        };
        let local = privates.iter().map(|&(off, size)| match parse_private_subrs_offset(&cff[off..off + size]) {
            0 => Vec::new(),
            rel => parse_cff_index_refs(cff, off + rel, false).expect("local subrs").0.iter().map(|s| s.to_vec()).collect(),
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
                assert_eq!(subrs_of(&out.expect("a subset").ttf), want, "glyph {glyph}");
            }
        }
    }

    // Glyphs alternate between two FDs: a compacted subset of FD 0's glyphs keeps that FD alone, and a
    // plain one keeps both, FD 1's subroutines cut to a bare return.
    #[test]
    fn a_cid_subset_keeps_the_fds_and_subroutines_its_glyphs_use() {
        let cff = fixture_cff("test-fixtures/cff-cid.otf");
        let (_, local) = subrs_of(&cff);
        assert_eq!(subrs_of(&subset_cff_compacting(&cff, &[2, 4]).expect("a subset").ttf).1, [local[0].clone()]);
        assert_eq!(subrs_of(&subset_cff(&cff, &[2, 4]).expect("a subset").ttf).1, [local[0].clone(), vec![vec![11], vec![11]]]);
    }

    fn cids(cff: &[u8]) -> Vec<u16> {
        let (_, after_name) = parse_cff_index_refs(cff, usize::from(cff[2]), false).expect("names");
        let (tops, _) = parse_cff_index_refs(cff, after_name, false).expect("Top DICT");
        let fields = parse_top_dict(tops[0]).expect("fields");
        let n = parse_cff_index_refs(cff, fields.charstrings_off, false).expect("CharStrings").0.len();
        let mut out = vec![0; n];
        walk_charset(cff, fields.charset_off.expect("a charset"), n, |gid, cid| {
            out[usize::from(gid)] = cid;
            CharsetFlow::Continue
        }).expect("it walks");
        out
    }

    #[test]
    fn a_compacted_cid_subset_keeps_each_glyphs_cid() {
        let cff = fixture_cff("test-fixtures/cff-cid.otf");
        let out = subset_cff_compacting(&cff, &[2, 9, 10]).expect("a subset");
        assert_eq!(cids(&out.ttf), [0, cids(&cff)[2], 34, 125]);
    }

    // An Expert charset kept as charset 0 reads as ISOAdobe: space, exclam and on.
    #[test]
    fn a_predefined_charset_keeps_its_id() {
        let cff = fixture_cff("test-fixtures/cff-expert.otf");
        let out = subset_cff(&cff, &[2, 3]).expect("a subset");
        let (_, after_name) = parse_cff_index_refs(&out.ttf, usize::from(out.ttf[2]), false).expect("names");
        let (tops, _) = parse_cff_index_refs(&out.ttf, after_name, false).expect("Top DICT");
        let fields = parse_top_dict(tops[0]).expect("fields");
        assert_eq!((fields.charset_off, fields.charset_predefined), (None, 1));
    }
}
