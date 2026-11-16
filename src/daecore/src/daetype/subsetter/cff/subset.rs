use super::*;
use crate::daecore::Error;
use super::build::{encode_cff_index_with, write_cff, CffParts, Fonts, TOO_MANY};
use super::expert_charsets::predefined_charset_sid;
use super::parse::{kept_sids, read_header, read_private};
use crate::daecore::daetype::format::cff::{read_dict, walk_cff_dict, Charset, DictKind, DictOp, FdSelect};
use crate::daecore::daetype::glyph_names::N_STD_STRINGS;
use crate::daecore::daetype::outline::{CffOutlines, Subrs, SubrsGathered};
use alloc::borrow::Cow;

#[cfg(test)]
pub fn subset_cff(cff: &[u8], requested: &[u32]) -> Result<Subset, Error> {
    subset_cff_requested(cff, requested, false)
}

#[cfg(test)]
pub fn subset_cff_compacting(cff: &[u8], requested: &[u32]) -> Result<Subset, Error> {
    subset_cff_requested(cff, requested, true)
}

// .notdef, the requested glyphs, and the base and accent glyphs their seacs draw.
#[cfg(test)]
fn subset_cff_requested(cff: &[u8], requested: &[u32], compact: bool) -> Result<Subset, Error> {
    let outlines = CffOutlines::parse(cff).ok();
    let mut set = GlyphSet::new();
    let frontier: Vec<u32> = core::iter::once(0).chain(requested.iter().copied()).filter(|&g| set.insert(g)).collect();
    let gathered = outlines.as_ref().map(|outlines| {
        let mut gathered = outlines.gather_subrs();
        close_over_seacs(outlines, cff, frontier, &mut set, &mut gathered);
        gathered
    });
    subset_cff_closed(cff, outlines.as_ref(), &set.iter().collect::<Vec<_>>(), compact, gathered, &BTreeMap::new())
}

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
            return;
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
    });
    out
}

// A Subrs INDEX with each subroutine no kept glyph calls cut to a bare `return`: every index stays,
// and with the count the bias.
fn pruned_subrs(cff: &[u8], subrs: Subrs<'_>, called: &[bool]) -> Result<Vec<u8>, Error> {
    let body = |i: usize| match called.get(i) {
        Some(true) => subrs.span(i).and_then(|(start, end)| cff.get(start as usize..end as usize)),
        _ => Some(&[11][..]),
    };
    if (0..subrs.len()).any(|i| body(i).is_none()) {
        return Err(Error::malformed("CFF: a subroutine past the font"));
    }
    encode_cff_index_with(|| (0..subrs.len()).filter_map(body)).ok_or(TOO_MANY)
}

// A Private DICT's entries but Subrs, then, when it has local Subrs, a Subrs operator naming them
// right after it: their offset is its own length, which counts the bytes that offset is written in.
fn private_dict(private: &[u8], subrs: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(private.len() + 6);
    walk_cff_dict(private, DictKind::Cff1, |op, _, operand_start, op_off| {
        if op != DictOp::Single(19) {
            let end = op_off + if matches!(op, DictOp::Escaped(_)) { 2 } else { 1 };
            out.extend_from_slice(&private[operand_start..end]);
        }
    });
    if subrs {
        let entries = out.len() + 1;
        let len = (1..=5)
            .map(|width| entries + width)
            .find(|&len| entries + encode_cff_int_short(len as i32).len() == len)
            .unwrap_or(entries + 5);
        out.extend(encode_cff_int_short(len as i32));
        out.push(19);
    }
    out
}

// A CFF2 table's subset of the closed glyphs pinned at `coords`, as static CFF2: kept glyphs in their slots and the
// rest empty, or with `compact` renumbered in order. A font with `varc` may hold fewer charstrings than glyphs.
pub(crate) fn subset_cff2_closed(
    source: crate::daecore::daetype::instancer::Cff2Source, n_glyphs: usize, closed: &[u32], compact: bool,
    coords: &[f64], flattened: Option<&mut crate::daecore::daetype::instancer::Flattened>,
) -> Result<Subset, Error> {
    let kept = |&g: &u32| (g as usize) < n_glyphs;
    let active: BTreeSet<u32> = core::iter::once(0).chain(closed.iter().copied().filter(kept)).collect();
    let (slots, gid_map) = if compact {
        let mut gid_map = vec![0u32; active.last().map_or(0, |&g| g as usize) + 1];
        for (new, &old) in active.iter().enumerate() {
            gid_map[old as usize] = new as u32;
        }
        (active.iter().map(|&g| Some(g)).collect::<Vec<_>>(), gid_map)
    } else {
        ((0..n_glyphs as u32).map(|g| active.contains(&g).then_some(g)).collect(), vec![])
    };
    Ok(Subset { font: source.write_kept(coords, &slots, flattened)?, gid_map })
}

// A subset of glyphs closed over their seacs, as `subset_cff_requested` and `cff_color_closure` close them, with the
// outlines and subroutine calls that closure read, if any; each glyph `drawn` holds is written as that charstring.
pub(crate) fn subset_cff_closed(
    cff: &[u8], outlines: Option<&CffOutlines>, closed: &[u32], compact: bool, gathered: Option<SubrsGathered>,
    drawn: &BTreeMap<u32, Vec<u8>>,
) -> Result<Subset, Error> {
    let header = read_header(cff)?;
    // The outlines read the CharStrings INDEX as the subset would, from these bytes; without them it's read here.
    let parsed = match outlines {
        Some(_) => Vec::new(),
        None => parse_cff_index_refs(cff, header.charstrings, false)?.0,
    };
    let n_glyphs = outlines.map_or(parsed.len(), CffOutlines::num_glyphs);
    let charstring = |g: u32| match outlines {
        _ if let Some(drawn) = drawn.get(&g) => Some(drawn.as_slice()),
        Some(outlines) => outlines.charstring(cff, g),
        None => parsed.get(g as usize).copied(),
    };

    let mut active = BTreeSet::<u32>::new();
    active.insert(0);
    for &gid in closed {
        if (gid as usize) < n_glyphs { active.insert(gid); }
    }

    let active_sorted: Vec<u32> = active.iter().copied().collect();
    let gid_map: Vec<u32> = if compact {
        let mut m = vec![0u32; *active_sorted.last().unwrap_or(&0) as usize + 1];
        for (new, &orig) in active_sorted.iter().enumerate() { m[orig as usize] = new as u32; }
        m
    } else {
        vec![]
    };

    let endchar: &[u8] = &[0x0eu8];
    let new_charstrings: Vec<&[u8]> = if compact {
        active_sorted.iter().map(|&g| charstring(g).unwrap_or(endchar)).collect()
    } else {
        (0..n_glyphs as u32).map(|g| active.contains(&g).then(|| charstring(g)).flatten().unwrap_or(endchar)).collect()
    };
    let charstring_ends: Vec<usize> = new_charstrings
        .iter()
        .scan(0, |end, c| {
            *end += c.len();
            Some(*end)
        })
        .collect();

    // The subroutines kept glyphs call, marked; None keeps them all, as does keeping every glyph (what none calls the
    // source never called either). The closure walked each kept glyph where it walked as many.
    let called = outlines.filter(|_| active_sorted.len() < n_glyphs).and_then(|o| {
        let called = match gathered.filter(|g| g.walked == active_sorted.len()) {
            Some(gathered) => gathered.called(o),
            None => o.subrs_called(cff, &active_sorted),
        };
        Some((o, called?))
    });
    let gsubr_index: Cow<[u8]> = match &called {
        Some((o, (global, _))) => Cow::Owned(pruned_subrs(cff, o.global_subrs(), global)?),
        None => Cow::Borrowed(&cff[header.gsubrs..header.end]),
    };

    let is_cid = header.top.fd_array.is_some();
    let fd_dicts: Vec<&[u8]> = match header.top.fd_array {
        Some(off) => parse_cff_index_refs(cff, off, false)?.0,
        None => Vec::new(),
    };

    // A compacting subset keeps the strings its glyph names and DICTs name, renumbered in order.
    let (charset_bytes, string_index_bytes, kept) = if compact {
        let (strings, _) = parse_cff_index_refs(cff, header.strings, false)?;
        // Each kept glyph's SID, in `active_sorted`'s order past .notdef.
        let sids = match header.top.charset() {
            Charset::At(off) => kept_sids(cff, off, n_glyphs, active_sorted.get(1..).unwrap_or(&[]))?,
            // A glyph its predefined charset doesn't name gets .notdef's SID.
            Charset::Predefined(id) => active_sorted.iter().skip(1)
                .map(|&g| predefined_charset_sid(id, g, n_glyphs).unwrap_or(0))
                .collect(),
        };

        let mut needed: BTreeSet<u16> = BTreeSet::new();
        dict_sids(header.top_dict, &mut needed);
        fd_dicts.iter().for_each(|fd| dict_sids(fd, &mut needed));
        if !is_cid {
            needed.extend(sids.iter().copied().filter(|&sid| sid >= N_STD_STRINGS));
        }
        let kept: Vec<u16> = needed.into_iter().collect();
        let renumber = |sid: u16| kept.binary_search(&sid).map_or(sid, |i| N_STD_STRINGS + i as u16);
        let refs: Vec<&[u8]> = kept.iter()
            .map(|&sid| strings.get((sid - N_STD_STRINGS) as usize).map_or(&[][..], |v| *v))
            .collect();

        let mut b = Vec::with_capacity(1 + 2 * active_sorted.len());
        b.push(0);
        for &sid in &sids {
            b.extend_from_slice(&if is_cid { sid } else { renumber(sid) }.to_be_bytes());
        }
        (b, encode_cff_index_refs(&refs).ok_or(TOO_MANY)?, kept)
    } else {
        let charset = match header.top.charset() {
            Charset::At(off) => cff[off..parse_charset_sids(cff, off, n_glyphs)?].to_vec(),
            Charset::Predefined(_) => Vec::new(),
        };
        (charset, cff[header.strings..header.gsubrs].to_vec(), Vec::new())
    };
    let renumber = |sid: u16| kept.binary_search(&sid).map_or(sid, |i| N_STD_STRINGS + i as u16);

    // The FDSelect written and the FDs kept, a compacting subset keeping only those its glyphs select, in order. A
    // glyph whose FD is absent or past the FDArray takes FD 0, as do all where FDSelect doesn't read.
    let (fd_select, kept_fds) = if is_cid {
        if !header.top.ros { return Err(Error::missing("CFF CID: missing ROS")); }
        let n_fds = fd_dicts.len();
        if n_fds == 0 { return Err(Error::malformed("CFF CID: empty FDArray")); }
        let read_at = |at| Some((at, FdSelect::read(cff, at, n_glyphs, DictKind::Cff1).ok()?));
        let read = header.top.fd_select.and_then(read_at);
        let select = read.as_ref().map_or_else(|| FdSelect::zero(n_glyphs), |(_, (select, _))| select.clone());
        let fd_of = |g: u32| select.get(cff, g as usize).map(usize::from).filter(|&fd| fd < n_fds).unwrap_or(0);
        if compact {
            let selected: BTreeSet<usize> = active_sorted.iter().map(|&g| fd_of(g)).collect();
            let kept: Vec<usize> = selected.into_iter().collect();
            let mut b = Vec::with_capacity(1 + active_sorted.len());
            b.push(0);
            for &orig in &active_sorted {
                let fd = kept.binary_search(&fd_of(orig)).unwrap_or(kept.len());
                b.push(u8::try_from(fd).map_err(|_| Error::malformed("CFF CID: an FD index past 255"))?);
            }
            (b, kept)
        } else {
            let bytes = read.map_or_else(|| alloc::vec![0; n_glyphs + 1], |(at, (_, end))| cff[at..end].to_vec());
            (bytes, (0..n_fds).collect())
        }
    } else {
        (Vec::new(), vec![0])
    };

    // Each kept FD's Private DICT, the Top DICT's when the font isn't CID-keyed: FDs may share one and
    // its Subrs, read once and written once, with the subroutines any FD naming them calls.
    let mut first_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut subrs_end: BTreeMap<usize, Option<usize>> = BTreeMap::new();
    let mut privates: Vec<(Vec<u8>, Cow<[u8]>)> = Vec::new();
    let mut fd_private = Vec::with_capacity(kept_fds.len());
    let mut read = 0usize;
    for &fd in &kept_fds {
        let at = if is_cid { read_dict(fd_dicts[fd], DictKind::Cff1).private } else { header.top.private.clone() };
        let key = at.as_ref().map_or((0, 0), |at| (at.start, at.end));
        if let Some(&first) = first_of.get(&key) {
            fd_private.push(first);
            continue;
        }
        let private = read_private(cff, at, DictKind::Cff1);
        // A copy needs only where the Subrs end: it reads back as the source's do, readable or not.
        let subrs = private.subrs.and_then(|at| {
            let end = *subrs_end.entry(at).or_insert_with(|| cff_index_end(cff, at, false).ok());
            Some(at..end?)
        });
        read = read.saturating_add(private.dict.len() + subrs.as_ref().map_or(0, |s| s.len()));
        if read > super::super::subset_budget(cff.len()) {
            return Err(Error::limit("CFF: Private DICTs past the subset budget"));
        }
        let local: Cow<[u8]> = match (&subrs, &called) {
            (None, _) => Cow::Borrowed(&[]),
            (Some(span), None) => Cow::Borrowed(&cff[span.clone()]),
            (Some(_), Some((o, (_, marks)))) => {
                let marks = o.subr_range(fd).map_or(&[][..], |(start, end)| &marks[start..end]);
                Cow::Owned(pruned_subrs(cff, o.local_subrs(fd), marks)?)
            }
        };
        first_of.insert(key, privates.len());
        fd_private.push(privates.len());
        privates.push((private_dict(private.dict, subrs.is_some()), local));
    }
    let fd_entries: Vec<Vec<u8>> = if is_cid {
        kept_fds.iter().map(|&fd| copy_dict(fd_dicts[fd], &renumber)).collect()
    } else {
        Vec::new()
    };

    let font = write_cff(&CffParts {
        header: &cff[..header.names],
        names: &cff[header.names..header.top_index],
        top: &copy_dict(header.top_dict, &renumber),
        strings: &string_index_bytes,
        gsubrs: &gsubr_index,
        charset: &charset_bytes,
        // A predefined charset keeps its id: written as 0, an Expert font would read as ISOAdobe.
        predefined: match header.top.charset() {
            Charset::Predefined(id) => id,
            Charset::At(_) => 0,
        },
        charstrings: (&new_charstrings, &charstring_ends),
        fonts: if is_cid {
            Fonts::Cid { fd_select: &fd_select, fds: fd_entries.iter().map(Vec::as_slice).zip(fd_private).collect() }
        } else {
            Fonts::Named
        },
        privates: privates.iter().map(|(dict, subrs)| (dict.as_slice(), &subrs[..])).collect(),
    })?;
    Ok(Subset { font, gid_map })
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/cff/subset.rs"]
mod tests;
