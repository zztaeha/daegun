use alloc::collections::BTreeMap;
use alloc::vec::Vec;
#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use super::pen::{OutlinePen, TransformPen};
use super::super::format::cff::{
    decode_charstring_number, read_dict, subr_bias, subr_index, Charset, Dict, DictKind, FdSelect, MAX_SUBR_DEPTH,
};
use super::super::subsetter::cff::parse::{read_header, read_private, read_subrs};
use super::super::subsetter::{parse_cff_index_refs, subset_budget, CffIndex, standard_encoding_sid, SidMap};
use super::super::hinting::cff::{
    EmBox, FontMatrix, Hinter, Outline, Placement, Private, PrivateDict, Scale, Zones, MAX_STEMS,
};
use super::super::hinting::HintedOutline;
use crate::daecore::daekit::base::sync::{read, write, Mutable};
use crate::daecore::{Error, ErrorKind};

type Span = (u32, u32);

// A glyph past the charstrings has no outline to draw.
const PAST_CFF: Error = Error::new(ErrorKind::NoOutline, "CFF: glyph index out of range");
const PAST_CFF2: Error = Error::new(ErrorKind::NoOutline, "CFF2: glyph index out of range");

// A Subrs INDEX as where each subroutine starts, then where the last ends: 4 bytes a subroutine, read with two
// adjacent loads. Read where they lie, its offsets cost drawing a CJK font 3%.
fn subr_starts(table: &[u8], index: CffIndex) -> Vec<u32> {
    let Some(last) = index.len().checked_sub(1) else { return Vec::new() };
    let mut starts = Vec::with_capacity(index.len() + 1);
    starts.extend((0..index.len()).filter_map(|i| index.span(table, i)).map(|(start, _)| start));
    starts.extend(index.span(table, last).map(|(_, end)| end));
    starts
}

// The subroutines of one Subrs INDEX as `subr_starts` lists them, an INDEX of none as one end, so the count is one
// less than the list's length without a test on each call.
#[derive(Clone, Copy)]
pub(crate) struct Subrs<'a>(&'a [u32]);

impl Default for Subrs<'_> {
    fn default() -> Self {
        Subrs(&[0])
    }
}

impl<'a> Subrs<'a> {
    fn of(starts: &'a [u32]) -> Subrs<'a> {
        match starts.is_empty() {
            true => Subrs::default(),
            false => Subrs(starts),
        }
    }

    pub(crate) fn len(self) -> usize {
        self.0.len() - 1
    }

    pub(crate) fn span(self, i: usize) -> Option<Span> {
        match *self.0.get(i..i + 2)? {
            [start, end] => Some((start, end)),
            _ => None,
        }
    }

    // From the first subroutine's start to the last's end, which the readable spans cover, so it never counts more
    // than they read.
    fn bytes(self) -> usize {
        self.0[self.0.len() - 1].saturating_sub(self.0[0]) as usize
    }
}

pub struct CffOutlines {
    charstrings:  CffIndex,
    global_subrs: Vec<u32>,
    // Every local Subrs INDEX once, however many FDs name it; each FD's, and where its subroutines start among all
    // of them, numbered one after another for those a subset marks, `local_count` in all.
    local_subrs:  Vec<Vec<u32>>,
    fd_subrs:     Vec<(usize, usize)>,
    local_count:  usize,
    // Each FD's hinting values, read with the outlines; None where its Private DICT doesn't read.
    fd_hints:     Vec<Option<Private>>,
    // Each FD's FontMatrix placement, or the Top DICT's alone without FDs.
    placements:   Vec<Option<Placement>>,
    // The font's ideographic em box, which an ideographic FD's glyphs are hinted in.
    em_box:       Option<EmBox>,
    fd_select:    Option<FdSelect>,
    charset:      Charset,
    // Built on the first seac, which most fonts never draw.
    sid_map:      Mutable<Option<SidMap>>,
}

impl CffOutlines {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let CffOutlines {
            charstrings: _, global_subrs, local_subrs, fd_subrs, local_count: _, fd_hints, placements, em_box: _,
            fd_select, charset: _, sid_map,
        } = self;
        let locals = heap_of(local_subrs) + local_subrs.iter().map(heap_of).sum::<usize>();
        heap_of(global_subrs) + locals + heap_of(fd_subrs) + heap_of(fd_hints) + heap_of(placements)
            + fd_select.as_ref().map_or(0, FdSelect::heap_bytes)
            + read(sid_map).as_ref().map_or(0, SidMap::heap_bytes)
    }

    // A bare CFF's outlines, whose Top DICT FontMatrix defaults to an em of 1000.
    #[cfg(test)]
    pub fn parse(cff: &[u8]) -> Result<CffOutlines, Error> {
        CffOutlines::parse_in(cff, 1000, None)
    }

    // A font's CFF outlines, `em` its head table's.
    pub(crate) fn parse_in(cff: &[u8], em: u16, em_box: Option<EmBox>) -> Result<CffOutlines, Error> {
        let header = read_header(cff)?;
        let global_subrs = subr_starts(cff, CffIndex::read(cff, header.gsubrs, false)?.0);
        let (charstrings, _) = CffIndex::read(cff, header.charstrings, false)?;
        let n_glyphs = charstrings.len();

        let matrix = |dict: &[u8], at: &Dict| at.font_matrix.clone().and_then(|at| FontMatrix::read(dict, at));
        let top_matrix = matrix(header.top_dict, &header.top);
        let (fd_select, privates, placements) = match header.top.fd_array {
            Some(fd_array) => {
                // An FDSelect that doesn't read is absent, every glyph then taking FD 0.
                let read = header.top.fd_select.and_then(|at| FdSelect::read(cff, at, n_glyphs, DictKind::Cff1).ok());
                let fd_select = read.map_or_else(|| FdSelect::zero(n_glyphs), |(select, _)| select);
                let (fds, _) = parse_cff_index_refs(cff, fd_array, false)?;
                let dicts: Vec<Dict> = fds.iter().map(|fd| read_dict(fd, DictKind::Cff1)).collect();
                let matrices: Vec<_> = fds.iter().zip(&dicts).map(|(fd, dict)| matrix(fd, dict)).collect();
                let placements = Placement::of(top_matrix, Some(&matrices), em);
                (Some(fd_select), dicts.into_iter().map(|dict| dict.private).collect(), placements)
            }
            None => (None, vec![header.top.private.clone()], Placement::of(top_matrix, None, em)),
        };
        let (mut local_subrs, mut local_count) = (Vec::new(), 0);
        let mut by_offset = BTreeMap::new();
        let read = fd_privates(cff, privates, |private| {
            let at = read_private(cff, private.clone(), DictKind::Cff1).subrs;
            let subrs = *by_offset.entry(at).or_insert_with(|| {
                let index = at.map_or_else(CffIndex::default, |at| read_subrs(cff, at, DictKind::Cff1));
                local_subrs.push(subr_starts(cff, index));
                local_count += index.len();
                (local_subrs.len() - 1, local_count - index.len())
            });
            (fd_hints(cff, private), subrs)
        })?;
        let (fd_hints, fd_subrs) = read.into_iter().unzip();

        Ok(CffOutlines {
            charstrings,
            global_subrs,
            local_subrs,
            fd_subrs,
            local_count,
            fd_hints,
            placements,
            em_box,
            fd_select,
            charset: header.top.charset(),
            sid_map: Mutable::new(None),
        })
    }

    // A glyph's charstring, as the CharStrings INDEX holds it.
    pub(crate) fn charstring<'c>(&self, cff: &'c [u8], gid: u32) -> Option<&'c [u8]> {
        let (start, end) = self.charstrings.span(cff, gid as usize)?;
        cff.get(start as usize..end as usize)
    }

    pub fn num_glyphs(&self) -> usize {
        self.charstrings.len()
    }

    // The glyph the charset gives a seac's SID; none in a CID-keyed font, whose charset holds CIDs, not SIDs, so a
    // seac there fails its glyph.
    fn glyph_of_sid(&self, cff: &[u8], sid: u16) -> Option<u16> {
        if self.fd_select.is_some() {
            return None;
        }
        if let Some(map) = read(&self.sid_map).as_ref() {
            return map.gid(sid);
        }
        let map = SidMap::new(cff, self.charset, self.charstrings.len());
        let gid = map.gid(sid);
        *write(&self.sid_map) = Some(map);
        gid
    }

    // The base and accent glyphs a seac draws, wherever its endchar sits, None where the charset names none. The calls
    // it walks are marked in `gathered`, so a subset of the glyphs the closure walked needn't walk them again.
    pub(crate) fn seac_glyphs(
        &self, cff: &[u8], gid: u32, gathered: &mut SubrsGathered,
    ) -> Option<[Option<u16>; 2]> {
        let span = self.charstrings.span(cff, gid as usize)?;
        gathered.walked += 1;
        let fd = fd_of(self, cff, gid);
        let (Ok(charstring), Some((local, base))) = (span_bytes(cff, span), self.fd_locals(fd)) else {
            gathered.called = None;
            return None;
        };
        let steps = allowed(cff, charstring, self.global_subrs(), local, 0);
        let mut state = State::new(None, TYPE2_OPERANDS, (), steps);
        state.called = gathered.called.take().map(|called| Called { base, ..called });
        let ran = draw_with(cff, charstring, self.global_subrs(), local, &mut state, &mut NullPen);
        gathered.called = state.called.take().filter(|_| ran.is_ok() && state.random.is_none());
        let (_, _, bchar, achar) = ran.ok().and(state.seac)?;
        Some([bchar, achar].map(|code| self.glyph_of_sid(cff, standard_encoding_sid(code))))
    }

    pub(crate) fn gather_subrs(&self) -> SubrsGathered {
        SubrsGathered { called: Some(self.uncalled()), walked: 0 }
    }

    // Every subroutine of the font, none marked.
    fn uncalled(&self) -> Called {
        Called { global: vec![false; self.global_subrs().len()], local: vec![false; self.local_count], base: 0 }
    }

    // The global Subrs, and an FD's local ones (none past the FDs).
    pub(crate) fn global_subrs(&self) -> Subrs<'_> {
        Subrs::of(&self.global_subrs)
    }

    pub(crate) fn local_subrs(&self, fd: usize) -> Subrs<'_> {
        self.fd_locals(fd).map_or_else(Subrs::default, |(local, _)| local)
    }

    // Where an FD's local subroutines lie among the font's, the same for every FD naming one INDEX.
    pub(crate) fn subr_range(&self, fd: usize) -> Option<(usize, usize)> {
        self.fd_locals(fd).map(|(local, base)| (base, base + local.len()))
    }

    // An FD's local Subrs and where they start among the font's.
    fn fd_locals(&self, fd: usize) -> Option<(Subrs<'_>, usize)> {
        self.fd_subrs.get(fd).map(|&(which, base)| (Subrs::of(&self.local_subrs[which]), base))
    }

    // The global and local subroutines these glyphs call, local ones placed by `subr_range`. None when the
    // font has none, or a glyph doesn't draw or draws with `random`, whose values could steer which ones it calls.
    pub(crate) fn subrs_called(&self, cff: &[u8], glyphs: &[u32]) -> Option<(Vec<bool>, Vec<bool>)> {
        if self.global_subrs.is_empty() && self.local_count == 0 {
            return None;
        }
        let mut called = self.uncalled();
        for &gid in glyphs {
            let charstring = span_bytes(cff, self.charstrings.span(cff, gid as usize)?).ok()?;
            let (local, base) = self.fd_locals(fd_of(self, cff, gid))?;
            let steps = allowed(cff, charstring, self.global_subrs(), local, 0);
            let mut state = State::new(None, TYPE2_OPERANDS, (), steps);
            called.base = base;
            state.called = Some(called);
            let ran = draw_with(cff, charstring, self.global_subrs(), local, &mut state, &mut NullPen);
            called = state.called.take()?;
            if ran.is_err() || state.random.is_some() {
                return None;
            }
        }
        Some((called.global, called.local))
    }
}

// A CFF2 font's charstrings, each FD's subroutines and vsindex, and the store blends weigh by when drawn. A store that
// doesn't read refuses drawing; an FDSelect that doesn't is absent, every glyph then taking FD 0.
pub struct Cff2Outlines {
    charstrings:  CffIndex,
    global_subrs: Vec<u32>,
    // Every local Subrs INDEX once, however many FDs name it; each FD's, and its vsindex.
    local_subrs:  Vec<Vec<u32>>,
    fds:          Vec<(usize, u32)>,
    // Each FD's Private DICT as hinting reads it, weighed where a glyph is drawn; None where it doesn't read.
    fd_private:   Vec<Option<PrivateDict>>,
    fd_select:    FdSelect,
    // Each FD's FontMatrix placement.
    placements:   Vec<Option<Placement>>,
    // The font's ideographic em box, which an ideographic FD's glyphs are hinted in.
    em_box:       Option<EmBox>,
    store:        Result<Option<crate::daecore::daetype::instancer::Cff2Store>, Error>,
    // The store's length, which a blend reads.
    store_bytes:  usize,
}

impl Cff2Outlines {
    pub fn parse(cff2: &[u8]) -> Result<Cff2Outlines, Error> {
        Cff2Outlines::parse_in(cff2, 1000, None)
    }

    // A font's CFF2 outlines, `em` its head table's.
    pub(crate) fn parse_in(cff2: &[u8], em: u16, em_box: Option<EmBox>) -> Result<Cff2Outlines, Error> {
        let header_size = usize::from(*cff2.get(2).ok_or(Error::malformed("CFF2: header too short"))?);
        let top_len =
            crate::daecore::daetype::decoder::read_u16_be(cff2, 3).ok_or(Error::malformed("CFF2: header truncated"))?;
        let top_len = usize::from(top_len);
        let top_dict =
            cff2.get(header_size..header_size + top_len).ok_or(Error::malformed("CFF2: Top DICT out of bounds"))?;
        let top = read_dict(top_dict, DictKind::Cff2);
        let global_subrs = subr_starts(cff2, CffIndex::read(cff2, header_size + top_len, true)?.0);
        let charstrings_at = top.charstrings.ok_or(Error::missing("CFF2 Top DICT: missing CharStrings"))?;
        let (charstrings, _) = CffIndex::read(cff2, charstrings_at, true)?;
        let (fd_dicts, _) =
            parse_cff_index_refs(cff2, top.fd_array.ok_or(Error::missing("CFF2 Top DICT: missing FDArray"))?, true)?;
        let read = top.fd_select.and_then(|at| FdSelect::read(cff2, at, charstrings.len(), DictKind::Cff2).ok());
        let fd_select = read.map_or_else(|| FdSelect::zero(charstrings.len()), |(select, _)| select);
        let (mut local_subrs, mut by_offset) = (Vec::new(), BTreeMap::new());
        let dicts: Vec<Dict> = fd_dicts.iter().map(|fd| read_dict(fd, DictKind::Cff2)).collect();
        let matrix = |dict: &[u8], at: &Dict| at.font_matrix.clone().and_then(|at| FontMatrix::read(dict, at));
        let matrices: Vec<_> = fd_dicts.iter().zip(&dicts).map(|(fd, dict)| matrix(fd, dict)).collect();
        let placements = Placement::of(matrix(top_dict, &top), Some(&matrices), em);
        let privates: Vec<_> = dicts.into_iter().map(|dict| dict.private).collect();
        let store_bytes = top.vstore.and_then(|at| super::super::decoder::read_u16_be(cff2, at)).map_or(0, usize::from);
        let store = match top.vstore {
            // A VariationStore of length 0 is none (CFF2, "VariationStore").
            Some(at) if super::super::decoder::read_u16_be(cff2, at) == Some(0) => Ok(None),
            Some(at) => crate::daecore::daetype::instancer::Cff2Store::load(cff2, at)
                .map(Some)
                .ok_or(Error::malformed("CFF2: variation store")),
            None => Ok(None),
        };
        let count = |v| store.as_ref().ok()?.as_ref()?.weight_count(v);
        let read = fd_privates(cff2, privates, |at| {
            let hints = match private_span(at.clone()) {
                Some(span) => span_bytes(cff2, span).ok().and_then(|d| PrivateDict::read(d, true, &mut |v| count(v))),
                None => Some(PrivateDict::default()),
            };
            let private = read_private(cff2, at, DictKind::Cff2);
            let subrs = *by_offset.entry(private.subrs).or_insert_with(|| {
                let index = private.subrs.map_or_else(CffIndex::default, |at| read_subrs(cff2, at, DictKind::Cff2));
                local_subrs.push(subr_starts(cff2, index));
                local_subrs.len() - 1
            });
            (hints, (subrs, u32::from(private.vsindex)))
        })?;
        let (fd_private, fds) = read.into_iter().unzip();
        Ok(Cff2Outlines {
            charstrings, global_subrs, local_subrs, fds, fd_private, fd_select, placements, em_box, store, store_bytes,
        })
    }

    // Glyph `gid`'s FD: the one FDSelect names, FD 0 where it names none or one past the FDArray.
    fn fd_of(&self, cff2: &[u8], gid: u32) -> usize {
        let fd = usize::from(self.fd_select.get(cff2, gid as usize).unwrap_or(0));
        if fd < self.fds.len() { fd } else { 0 }
    }

    // An FD's local Subrs and vsindex.
    fn fd(&self, fd: usize) -> Option<(Subrs<'_>, u32)> {
        self.fds.get(fd).map(|&(which, vsindex)| (Subrs::of(&self.local_subrs[which]), vsindex))
    }

    // How many axes the drawn variation store's regions span, 0 without one.
    pub(crate) fn store_axes(&self) -> usize {
        self.store.as_ref().ok().and_then(Option::as_ref).map_or(0, |s| s.axes())
    }

    // Whether drawing reads the font: its variation store loads.
    pub(crate) fn drawable(&self) -> bool {
        self.store.is_ok()
    }

    pub(crate) fn num_glyphs(&self) -> usize {
        self.charstrings.len()
    }
}

// The store's weights at `coords`, None at the default.
struct Located<'a> {
    store:  Option<&'a crate::daecore::daetype::instancer::Cff2Store>,
    coords: Option<&'a [i64]>,
}

impl Cff2Weights for Located<'_> {
    fn weights(&self, vsindex: u32) -> Result<Vec<i32>, Error> {
        self.store.ok_or(Error::malformed("CFF2: a blend without a variation store"))?.weights(vsindex, self.coords)
    }

    fn work(&self, vsindex: u32) -> usize {
        self.store.map_or(0, |store| store.work(vsindex, self.coords))
    }
}

// A CFF2 glyph drawn at `coords`, None at the default, and its hints when asked for.
pub(crate) fn outline_cff2_glyph(
    outlines: &Cff2Outlines,
    cff2: &[u8],
    gid: u32,
    coords: Option<&[i64]>,
    hints: Option<CffHints>,
    pen: &mut dyn OutlinePen,
) -> Result<Option<CffHints>, Error> {
    outline_cff2_glyph_counted(outlines, cff2, gid, coords, hints, &mut 0, pen)
}

// As `outline_cff2_glyph`, the charstring steps it took, drawn or not, added to `steps`.
pub(crate) fn outline_cff2_glyph_counted(
    outlines: &Cff2Outlines,
    cff2: &[u8],
    gid: u32,
    coords: Option<&[i64]>,
    hints: Option<CffHints>,
    steps: &mut usize,
    pen: &mut dyn OutlinePen,
) -> Result<Option<CffHints>, Error> {
    let span = outlines.charstrings.span(cff2, gid as usize).ok_or(PAST_CFF2)?;
    let charstring = span_bytes(cff2, span)?;
    let fd = outlines.fd_of(cff2, gid);
    let (local, vsindex) = outlines.fd(fd).ok_or(Error::malformed("CFF2: no FDArray"))?;
    let located = Located { store: outlines.store.as_ref().map_err(Error::clone)?.as_ref(), coords };
    let global = Subrs::of(&outlines.global_subrs);
    let allowed = allowed(cff2, charstring, global, local, outlines.store_bytes);
    let mut state = State::new(hints, OPERAND_LIMIT, (), allowed);
    state.width_taken = true;
    let blending = Blending { weights: &located, built: None, vsindex_read: false };
    state.cff2 = Some(Cff2Run { blending, vsindex });
    let mut placed;
    let pen: &mut dyn OutlinePen = match outlines.placements.get(fd).copied().flatten() {
        Some(p) => {
            placed = TransformPen::new(pen, p.pen_transform());
            &mut placed
        }
        None => pen,
    };
    let drawn = draw_with(cff2, charstring, global, local, &mut state, pen);
    *steps += allowed - state.budget;
    drawn.map(|()| state.hints)
}

// A pen that draws nothing, for a charstring run for what it calls or ends with.
struct NullPen;

impl OutlinePen for NullPen {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

// A glyph's charstring, the local Subrs its FD gives it and the seed its `random` starts from.
fn glyph_program<'a>(outlines: &'a CffOutlines, cff: &'a [u8], gid: u32) -> Result<(Program<'a>, usize), Error> {
    let span = outlines.charstrings.span(cff, gid as usize).ok_or(PAST_CFF)?;
    let charstring = span_bytes(cff, span)?;
    let fd = fd_of(outlines, cff, gid);
    let (local, _) = outlines.fd_locals(fd).ok_or(Error::malformed("CFF CID: no FDArray"))?;
    let seed = outlines.fd_hints.get(fd).and_then(Option::as_ref).map_or(0, |p| p.random_seed);
    Ok(((charstring, local, seed), fd))
}

type Program<'a> = (&'a [u8], Subrs<'a>, u32);

// Glyph `gid`'s FD: the one FDSelect names, FD 0 where it names none or one past the FDArray.
fn fd_of(outlines: &CffOutlines, cff: &[u8], gid: u32) -> usize {
    let fd = outlines.fd_select.as_ref().map_or(0, |s| usize::from(s.get(cff, gid as usize).unwrap_or(0)));
    if fd < outlines.fd_subrs.len() { fd } else { 0 }
}

// A Private DICT's place, None where there's none to read: at offset 0 or of no size.
fn private_span(at: Option<core::ops::Range<usize>>) -> Option<Span> {
    let at = at.filter(|at| at.start != 0 && !at.is_empty())?;
    Some((u32::try_from(at.start).ok()?, u32::try_from(at.end).ok()?))
}

// What `read` gives each FD's Private DICT `privates`, read once a span however many FDs name it. Spans can overlap,
// and FDs each reading theirs whole could read the table's square, so those read share the subset budget, past which
// the font is refused.
fn fd_privates<T: Clone>(
    cff: &[u8], privates: Vec<Option<core::ops::Range<usize>>>,
    mut read: impl FnMut(Option<core::ops::Range<usize>>) -> T,
) -> Result<Vec<T>, Error> {
    let (mut left, mut read_once) = (subset_budget(cff.len()), BTreeMap::new());
    privates
        .into_iter()
        .map(|at| {
            let span = at.as_ref().map(|at| (at.start, at.end));
            if let Some(read) = read_once.get(&span) {
                return Ok(T::clone(read));
            }
            let size = at.clone().and_then(|at| cff.get(at)).map_or(0, <[u8]>::len);
            let over = Error::limit("CFF: the FDs' Private DICTs run past the font's budget");
            left = left.checked_sub(size).ok_or(over)?;
            Ok(read_once.entry(span).or_insert(read(at)).clone())
        })
        .collect()
}

// A CFF FD's hinting values; None where its Private DICT doesn't read.
fn fd_hints(cff: &[u8], at: Option<core::ops::Range<usize>>) -> Option<Private> {
    match private_span(at) {
        Some(at) => Private::read(span_bytes(cff, at).ok()?),
        None => Some(Private::default()),
    }
}

// A CFF glyph hinted, `scale16` from font units to 26.6 in 16.16; a seac draws its base, then its accent raised by
// its offset, each with its own local subrs, in the glyph's zones. None where hinting fails.
pub(crate) fn hint_cff_glyph(outlines: &CffOutlines, cff: &[u8], gid: u32, scale16: i64) -> Option<HintedOutline> {
    let ((charstring, local_subrs, seed), fd) = glyph_program(outlines, cff, gid).ok()?;
    let placement = outlines.placements.get(fd).copied().flatten();
    let scale = Scale::new(placement.map_or(scale16, |p| p.hint_scale(scale16)))?;
    let em_box = outlines.em_box.and_then(|b| b.placed(placement));
    let zones = Zones::new(outlines.fd_hints.get(fd)?.as_ref()?, scale, em_box);
    let mut out = Outline::for_charstring(charstring.len());
    let mut seac = None;
    for part in 0..3 {
        let ((charstring, local_subrs, _), at) = match (part, seac) {
            (0, _) => ((charstring, local_subrs, seed), (0.0, 0.0)),
            (_, Some((adx, ady, bchar, achar))) => {
                let code = if part == 1 { bchar } else { achar };
                let gid = outlines.glyph_of_sid(cff, standard_encoding_sid(code))?;
                (glyph_program(outlines, cff, gid.into()).ok()?.0, if part == 2 { (adx, ady) } else { (0.0, 0.0) })
            }
            (_, None) => break,
        };
        let mut hinter = Hinter::new(zones, at, out);
        let steps = allowed(cff, charstring, outlines.global_subrs(), local_subrs, 0);
        let mut state = State::new(None, TYPE2_OPERANDS, &mut hinter, steps);
        (state.x, state.y, state.seed) = (at.0, at.1, seed);
        draw_with(cff, charstring, outlines.global_subrs(), local_subrs, &mut state, &mut NullPen).ok()?;
        match (part, state.seac) {
            (0, found) => seac = found,
            (_, Some(_)) => return None,
            _ => {}
        }
        out = hinter.finish();
    }
    placed(out.finish()?, placement, scale16)
}

// An outline as hinting leaves it, through its FD's placement.
fn placed(out: HintedOutline, placement: Option<Placement>, scale16: i64) -> Option<HintedOutline> {
    match placement {
        Some(p) => p.place(out, scale16),
        None => Some(out),
    }
}

// A CFF2 glyph hinted at `coords`, None at the default, as `hint_cff_glyph` hints a CFF glyph; its FD's Private DICT
// blended there.
pub(crate) fn hint_cff2_glyph(
    outlines: &Cff2Outlines, cff2: &[u8], gid: u32, coords: Option<&[i64]>, scale16: i64,
) -> Option<HintedOutline> {
    let charstring = span_bytes(cff2, outlines.charstrings.span(cff2, gid as usize)?).ok()?;
    let fd = outlines.fd_of(cff2, gid);
    let placement = outlines.placements.get(fd).copied().flatten();
    let scale = Scale::new(placement.map_or(scale16, |p| p.hint_scale(scale16)))?;
    let (local, vsindex) = outlines.fd(fd)?;
    let located = Located { store: outlines.store.as_ref().ok()?.as_ref(), coords };
    let dict = outlines.fd_private.get(fd)?.as_ref()?;
    // Weighing the Private DICT's blends comes out of the charstring's steps, as a blend's weighing does.
    let weighed = if dict.blends() { located.work(dict.vsindex) } else { 0 };
    let global = Subrs::of(&outlines.global_subrs);
    let budget = allowed(cff2, charstring, global, local, outlines.store_bytes).checked_sub(weighed)?;
    let weights = match dict.blends() {
        true => Some(located.weights(dict.vsindex).ok()?),
        false => None,
    };
    let em_box = outlines.em_box.and_then(|b| b.placed(placement));
    let zones = Zones::new(&dict.at(weights.as_deref())?, scale, em_box);
    let mut hinter = Hinter::new(zones, (0.0, 0.0), Outline::for_charstring(charstring.len()));
    let mut state = State::new(None, OPERAND_LIMIT, &mut hinter, budget);
    state.width_taken = true;
    state.cff2 = Some(Cff2Run { blending: Blending { weights: &located, built: None, vsindex_read: false }, vsindex });
    draw_with(cff2, charstring, global, local, &mut state, &mut NullPen).ok()?;
    placed(hinter.finish().finish()?, placement, scale16)
}

#[cfg(test)]
pub fn outline_cff_glyph_with(
    outlines: &CffOutlines,
    cff: &[u8],
    gid: u32,
    pen: &mut dyn OutlinePen,
) -> Result<(), Error> {
    draw_glyph(outlines, cff, gid, None, true, &mut 0, pen).map(|_| ())
}

// As `outline_cff_glyph_with`, the charstring steps it took, drawn or not, added to `steps`.
pub(crate) fn outline_cff_glyph_counted(
    outlines: &CffOutlines,
    cff: &[u8],
    gid: u32,
    steps: &mut usize,
    pen: &mut dyn OutlinePen,
) -> Result<(), Error> {
    draw_glyph(outlines, cff, gid, None, true, steps, pen).map(drop)
}

pub fn outline_cff_glyph_hinted(
    outlines: &CffOutlines,
    cff: &[u8],
    gid: u32,
    pen: &mut dyn OutlinePen,
) -> Result<CffHints, Error> {
    draw_glyph(outlines, cff, gid, Some(CffHints::default()), true, &mut 0, pen)
        .map(Option::unwrap_or_default)
}

// A glyph's own program, then, when its endchar names a base and an accent (seac), those two, all placed by the
// glyph's FD where `placed`, the steps they take added to `steps`.
fn draw_glyph(
    outlines: &CffOutlines,
    cff: &[u8],
    gid: u32,
    hints: Option<CffHints>,
    placed: bool,
    steps: &mut usize,
    pen: &mut dyn OutlinePen,
) -> Result<Option<CffHints>, Error> {
    let (program, fd) = glyph_program(outlines, cff, gid)?;
    let placement = outlines.placements.get(fd).copied().flatten().filter(|_| placed);
    let mut placed;
    let pen: &mut dyn OutlinePen = match placement {
        Some(p) => {
            placed = TransformPen::new(pen, p.pen_transform());
            &mut placed
        }
        None => pen,
    };
    let (hints, seac) = draw_charstring(cff, program, outlines.global_subrs(), hints, steps, pen)?;
    if let Some((adx, ady, bchar, achar)) = seac {
        draw_seac(outlines, cff, adx, ady, bchar, achar, steps, pen)?;
        return Ok(hints.map(|_| CffHints::default()));
    }
    Ok(hints)
}

// Each component draws with its own FD's local subroutines, so it draws in a seac as it draws alone. The base
// comes first.
#[allow(clippy::too_many_arguments)]
fn draw_seac(
    outlines: &CffOutlines,
    cff:      &[u8],
    adx: f64, ady: f64, bchar: u8, achar: u8,
    steps: &mut usize,
    pen: &mut dyn OutlinePen,
) -> Result<(), Error> {
    let global_subrs = outlines.global_subrs();
    let base_gid = outlines.glyph_of_sid(cff, standard_encoding_sid(bchar))
        .ok_or(Error::malformed("CFF: seac base character not found in charset"))?;
    let accent_gid = outlines.glyph_of_sid(cff, standard_encoding_sid(achar))
        .ok_or(Error::malformed("CFF: seac accent character not found in charset"))?;

    let base = glyph_program(outlines, cff, base_gid.into())?.0;
    let (_, nested) = draw_charstring(cff, base, global_subrs, None, steps, pen)?;

    let accent = glyph_program(outlines, cff, accent_gid.into())?.0;
    let mut offset_pen = TransformPen::new(pen, [1.0, 0.0, 0.0, 1.0, adx, ady]);
    let (_, nested_accent) = draw_charstring(cff, accent, global_subrs, None, steps, &mut offset_pen)?;
    if nested.is_some() || nested_accent.is_some() {
        return Err(Error::malformed("CFF: seac base or accent is itself a seac"));
    }
    Ok(())
}

// A charstring may walk back to where its contour began, and that segment is the one `close`
// already draws. Held one step back so it can be dropped when the contour ends there.
struct CloseElidingPen<'a> {
    inner:   &'a mut dyn OutlinePen,
    start:   (f32, f32),
    pending: Option<(f32, f32)>,
}

impl CloseElidingPen<'_> {
    fn new(inner: &mut dyn OutlinePen) -> CloseElidingPen<'_> {
        CloseElidingPen { inner, start: (0.0, 0.0), pending: None }
    }

    fn flush(&mut self) {
        if let Some((x, y)) = self.pending.take() {
            self.inner.line_to(x, y);
        }
    }
}

impl OutlinePen for CloseElidingPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.start = (x, y);
        self.inner.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.pending = Some((x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.flush();
        self.inner.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.flush();
        self.inner.curve_to(c1x, c1y, c2x, c2y, x, y);
    }
    fn close(&mut self) {
        if self.pending == Some(self.start) {
            self.pending = None;
        }
        self.flush();
        self.inner.close();
    }
}

// Type 2 allows 48 operands (TN 5177, appendix B), but CFF that 1.1.7 converted from CFF2 carries up to the 513
// CFF2's maxstack allows.
const OPERAND_LIMIT: usize = 513;
const TYPE2_OPERANDS: usize = 48;

/// A CFF stem hint, its edges as the charstring states them in font units: an edge hint keeps its width of -20 or
/// -21, so its max is below its min. Laid out as C reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CffStem {
    /// The lower edge.
    pub min: f32,
    /// The upper edge.
    pub max: f32,
    /// Whether it hints a vertical stem, its edges x positions.
    pub vertical: bool,
}

/// A CFF glyph's stem hints and the hint masks its charstring sets as it draws.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CffHints {
    /// The stems, in the charstring's order.
    pub stems: Vec<CffStem>,
    /// The masks, in the order the charstring sets them.
    pub masks: Vec<CffMask>,
}

/// A hint mask a CFF charstring sets as it draws.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct CffMask {
    /// The point it takes effect from, counted as the glyph's points are drawn.
    pub point: usize,
    /// A bit a stem, in [`CffHints::stems`]' order, high bit first.
    pub bits: Vec<u8>,
}

type Seac = (f64, f64, u8, u8);

// The subroutines the glyphs walked call, as `subrs_called` gives them: None once one doesn't draw or draws with
// `random`. `walked` counts the glyphs of the font walked.
pub(crate) struct SubrsGathered {
    called: Option<Called>,
    pub(crate) walked: usize,
}

impl SubrsGathered {
    pub(crate) fn called(self, outlines: &CffOutlines) -> Option<(Vec<bool>, Vec<bool>)> {
        if outlines.global_subrs.is_empty() && outlines.local_count == 0 {
            return None;
        }
        self.called.map(|c| (c.global, c.local))
    }
}

// The global and local subroutines called, marked where a subset asks which it keeps; a glyph's
// local ones from `base`, where its Subrs start among the font's.
struct Called {
    global: Vec<bool>,
    local:  Vec<bool>,
    base:   usize,
}

// What a CFF2 charstring reads to blend: how its blends are weighed and the vsindex in force.
struct Cff2Run<'a> {
    blending: Blending<'a>,
    vsindex:  u32,
}

// 16.16 weights for a vsindex at the location, built at the first blend as the weights' count and each region not
// weighing 0, with its weight; and whether a vsindex has been read, as only one may be.
struct Blending<'a> {
    weights: &'a dyn Cff2Weights,
    built: Option<(usize, Vec<(usize, i32)>)>,
    vsindex_read: bool,
}

// The blend weights a CFF2 charstring reads at a location, 16.16: one for the default master, then one for each region
// the vsindex's data names. Err where a blend there fails the glyph.
pub(crate) trait Cff2Weights {
    fn weights(&self, vsindex: u32) -> Result<Vec<i32>, Error>;

    // What `weights` takes, in charstring steps.
    fn work(&self, vsindex: u32) -> usize;
}

// What the interpreter tells a hinter. Drawing alone takes `()`, so the `run` it builds holds no hinting: the
// checks cost unhinted drawing about 1.5%.
trait Hook {
    fn hinter(&mut self) -> Option<&mut Hinter>;
}

impl Hook for () {
    fn hinter(&mut self) -> Option<&mut Hinter> {
        None
    }
}

impl Hook for &mut Hinter {
    fn hinter(&mut self) -> Option<&mut Hinter> {
        Some(&mut **self)
    }
}

struct State<'a, H> {
    cff2:        Option<Cff2Run<'a>>,
    stack:       Stack,
    x:           f64,
    y:           f64,
    n_stems:     usize,
    width_taken: bool,
    open_path:   bool,
    depth:       usize,
    budget:      usize,
    hints:       Option<CffHints>,
    shadow:      Shadow,
    mask:        Option<Vec<u8>>,
    // A hint mask read since the last segment, which keeps a zero-length line.
    mask_fresh:  bool,
    ended:       bool,
    seac:        Option<Seac>,

    transient:   [i32; 32],
    // `random`'s state, seeded at its first use in each glyph, so a glyph draws alike whatever was drawn before it.
    random:      Option<u64>,
    seed:        u32,
    called:      Option<Called>,
    // The stem count fixed, as it is from the first hintmask or moveto; past 96 there, a failure.
    frozen:      bool,
    over_hints:  bool,
    hinter:      H,
}

// The operand stack, lent from the thread's last charstring to its next, so a glyph allocates none.
struct Stack(Vec<f64>);

#[cfg(feature = "std")]
std::thread_local! {
    static SPARE_STACK: core::cell::Cell<Vec<f64>> = const { core::cell::Cell::new(Vec::new()) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE_STACK.try_with(core::cell::Cell::take);
}

impl Stack {
    fn new(operands: usize) -> Stack {
        #[cfg(feature = "std")]
        let mut stack = SPARE_STACK.try_with(core::cell::Cell::take).unwrap_or_default();
        #[cfg(feature = "std")]
        stack.reserve(operands);
        #[cfg(not(feature = "std"))]
        let stack = Vec::with_capacity(operands);
        Stack(stack)
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        let _ = SPARE_STACK.try_with(|spare| {
            self.0.clear();
            spare.set(core::mem::take(&mut self.0));
        });
    }
}

impl core::ops::Deref for Stack {
    type Target = Vec<f64>;
    fn deref(&self) -> &Vec<f64> {
        &self.0
    }
}

impl core::ops::DerefMut for Stack {
    fn deref_mut(&mut self) -> &mut Vec<f64> {
        &mut self.0
    }
}

impl<H: Hook> State<'_, H> {
    // `operands` deep at first: Type 2's 48, or CFF2's limit, which blends reach; `budget` steps to take.
    fn new<'a>(hints: Option<CffHints>, operands: usize, hinter: H, budget: usize) -> State<'a, H> {
        State {
            cff2: None, stack: Stack::new(operands), x: 0.0, y: 0.0, n_stems: 0, width_taken: false,
            open_path: false, depth: 0, budget, hints, shadow: Shadow::default(),
            mask: None, mask_fresh: false, ended: false, seac: None,
            transient: [0; 32], random: None, seed: 0, called: None,
            frozen: false, over_hints: false, hinter,
        }
    }

    fn clear(&mut self) {
        self.stack.clear();
    }

    // The top operand in 16.16, an integer shifted up.
    fn pop_fixed(&mut self) -> Result<i32, Error> {
        let v = self.stack.pop().ok_or(Error::missing("CFF charstring: an operand missing"))?;
        Ok(to_fixed(v))
    }

    // `v` pushed as a 16.16 where `fixed`, else as a whole number.
    fn push_typed(&mut self, v: i32, fixed: bool) {
        self.stack.push(if fixed { f64::from(v) / 65536.0 } else { f64::from(v) });
    }

    // A hint mask applies from the first point of the construction after it, numbered as a
    // collecting pen keeps the points (see `Shadow`).
    fn stamp_mask(&mut self) {
        if let (Some(mask), Some(h)) = (self.mask.take(), self.hints.as_mut()) {
            h.masks.push(CffMask { point: self.shadow.collected + self.shadow.open, bits: mask });
        }
    }
}

// The points a collecting pen keeps: `CloseElidingPen` drops a closing line back to the start, then
// `CollectPen` a final on-curve point equal to it. Counted only while hints, which number them, are kept.
#[derive(Default)]
struct Shadow {
    collected: usize,
    open:      usize,
    start:     (f32, f32),
    ends:      [Option<(f32, f32)>; 2],
    last_line: bool,
}

impl Shadow {
    fn close(&mut self) {
        if self.open == 0 { return; }
        if self.last_line && self.ends[1] == Some(self.start) {
            self.open -= 1;
            self.ends[1] = self.ends[0];
        }
        if self.open > 1 && self.ends[1] == Some(self.start) {
            self.open -= 1;
        }
        self.collected += self.open;
        self.open = 0;
    }

    fn segment(&mut self, end: (f32, f32), points: usize, line: bool) {
        self.open += points;
        self.ends = [self.ends[1], Some(end)];
        self.last_line = line;
    }
}

// Steps a charstring may take for each byte it can read: its own, its subroutines' and a CFF2 store's. Calls nest
// 10 deep, so a program's steps grow exponentially with its bytes; real fonts take 4.5 a byte at most.
const STEPS_PER_BYTE: usize = 64;

const SPENT: Error = Error::limit("CFF charstring: work budget exhausted");

// The steps a charstring may take, calling `global` and `local` and reading `more` bytes besides.
fn allowed(cff: &[u8], charstring: &[u8], global: Subrs<'_>, local: Subrs<'_>, more: usize) -> usize {
    let subrs = (global.bytes() + local.bytes()).min(cff.len());
    STEPS_PER_BYTE.saturating_mul(charstring.len() + subrs + more)
}

// The charstring drawn, then its hints (when asked for) and any seac its endchar named, the steps it took added to
// `steps`.
pub(crate) fn draw_charstring(
    cff:          &[u8],
    (charstring, local_subrs, seed): Program<'_>,
    global_subrs: Subrs<'_>,
    hints:        Option<CffHints>,
    steps:        &mut usize,
    pen:          &mut dyn OutlinePen,
) -> Result<(Option<CffHints>, Option<Seac>), Error> {
    let allowed = allowed(cff, charstring, global_subrs, local_subrs, 0);
    let mut state = State::new(hints, TYPE2_OPERANDS, (), allowed);
    state.seed = seed;
    let drawn = draw_with(cff, charstring, global_subrs, local_subrs, &mut state, pen);
    *steps += allowed - state.budget;
    drawn?;
    Ok((state.hints, state.seac))
}

fn draw_with(
    cff:          &[u8],
    charstring:   &[u8],
    global_subrs: Subrs<'_>,
    local_subrs:  Subrs<'_>,
    state:        &mut State<'_, impl Hook>,
    pen:          &mut dyn OutlinePen,
) -> Result<(), Error> {
    let pen = &mut CloseElidingPen::new(pen);
    let global_bias = subr_bias(global_subrs.len());
    let local_bias  = subr_bias(local_subrs.len());
    match state.cff2.is_some() {
        true => run::<true>(cff, charstring, global_subrs, local_subrs, global_bias, local_bias, state, pen)?,
        false => run::<false>(cff, charstring, global_subrs, local_subrs, global_bias, local_bias, state, pen)?,
    }
    if state.over_hints {
        return Err(Error::malformed("CFF charstring: more than 96 stem hints"));
    }
    if state.open_path {
        pen.close();
    }
    Ok(())
}

fn take_width_fixed(state: &mut State<'_, impl Hook>, expected: &[usize]) {
    if state.width_taken { return; }
    state.width_taken = true;
    let n = state.stack.len();
    if expected.iter().any(|&c| n == c + 1) {
        state.stack.remove(0);
    }
}

fn record_stems(state: &mut State<'_, impl Hook>, vertical: bool) {
    let State { stack, hints, .. } = state;
    let Some(h) = hints.as_mut() else { return };
    let mut edge = 0.0f64;
    h.stems.extend(stack.as_chunks::<2>().0.iter().map(|c| {
        let min = edge + c[0];
        let max = min + c[1];
        edge = max;
        CffStem { min: min as f32, max: max as f32, vertical }
    }));
}

fn take_width_stem(state: &mut State<'_, impl Hook>) {
    if state.width_taken { return; }
    state.width_taken = true;
    if state.stack.len() % 2 == 1 {
        state.stack.remove(0);
    }
}

fn moveto(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>, dx: f64, dy: f64) {
    start_path(state, pen, dx, dy);
    if let Some(h) = state.hinter.hinter() {
        h.move_to(state.x, state.y);
    }
}

fn start_path(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>, dx: f64, dy: f64) {
    if state.open_path {
        pen.close();
    }
    let hinting = state.hints.is_some();
    if hinting {
        state.shadow.close();
        state.stamp_mask();
    }
    state.x += dx;
    state.y += dy;
    state.mask_fresh = false;
    let at = (state.x as f32, state.y as f32);
    pen.move_to(at.0, at.1);
    if hinting {
        state.shadow.start = at;
        state.shadow.segment(at, 1, false);
    }
    state.open_path = true;
    if !state.frozen {
        (state.frozen, state.over_hints) = (true, state.n_stems > MAX_STEMS && state.cff2.is_none());
    }
}

// Drawing with no moveto first starts a path at the current point, which recovers such a charstring.
fn open_if_needed(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>) {
    if !state.open_path {
        start_path(state, pen, 0.0, 0.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn curveto(
    state: &mut State<'_, impl Hook>,
    pen: &mut CloseElidingPen<'_>,
    c1x: f64,
    c1y: f64,
    c2x: f64,
    c2y: f64,
    ex: f64,
    ey: f64,
) {
    if let Some(h) = state.hinter.hinter() {
        h.curve_to((c1x, c1y), (c2x, c2y), (ex, ey));
    }
    open_if_needed(state, pen);
    state.mask_fresh = false;
    if state.hints.is_some() {
        state.stamp_mask();
        state.shadow.segment((ex as f32, ey as f32), 3, false);
    }
    pen.curve_to(c1x as f32, c1y as f32, c2x as f32, c2y as f32, ex as f32, ey as f32);
    state.x = ex;
    state.y = ey;
}

// A line that goes nowhere is dropped, unless a hint mask came since the last segment.
fn lineto(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>, dx: f64, dy: f64) {
    if let Some(h) = state.hinter.hinter() {
        h.line_to(state.x + dx, state.y + dy);
    }
    if dx == 0.0 && dy == 0.0 && !state.mask_fresh {
        return;
    }
    open_if_needed(state, pen);
    state.mask_fresh = false;
    state.x += dx;
    state.y += dy;
    if state.hints.is_some() {
        state.stamp_mask();
        state.shadow.segment((state.x as f32, state.y as f32), 1, true);
    }
    pen.line_to(state.x as f32, state.y as f32);
}

#[inline]
fn span_bytes(cff: &[u8], s: Span) -> Result<&[u8], Error> {
    cff.get(s.0 as usize..s.1 as usize).ok_or(Error::malformed("CFF charstring: subr span out of bounds"))
}

// Subroutine `i`, already checked against the count.
#[inline]
fn subr_bytes<'c>(cff: &'c [u8], subrs: Subrs<'_>, i: usize) -> Result<&'c [u8], Error> {
    span_bytes(cff, subrs.span(i).ok_or(Error::malformed("CFF charstring: subr index out of range"))?)
}

// Built once for Type 2 and once for CFF2, so neither tests which it reads at each operand: those tests cost Type 2
// drawing 4% more instructions.
#[allow(clippy::too_many_arguments)]
fn run<const CFF2: bool>(
    cff:          &[u8],
    cs:           &[u8],
    global_subrs: Subrs<'_>,
    local_subrs:  Subrs<'_>,
    global_bias:  i32,
    local_bias:   i32,
    state:        &mut State<'_, impl Hook>,
    pen:          &mut CloseElidingPen<'_>,
) -> Result<(), Error> {
    if state.depth > MAX_SUBR_DEPTH {
        return Err(Error::malformed("CFF charstring: subroutine nesting too deep"));
    }
    let cff2 = CFF2;
    let mut pos = 0usize;
    while pos < cs.len() {
        // A step pays for itself and for the hint sets the step before had the hinter place.
        let spent = 1 + state.hinter.hinter().map_or(0, |h| h.take_work() as usize);
        let Some(left) = state.budget.checked_sub(spent) else {
            return Err(SPENT);
        };
        state.budget = left;
        let b0 = cs[pos];

        if b0 >= 32 || b0 == 28 {
            // A run of operands read in one loop, each charged as the outer loop charges it. Most take one byte,
            // read here past the general decode.
            let mut b = b0;
            loop {
                let (v, sz) = match b {
                    32..=246 => (f64::from(i32::from(b) - 139), 1),
                    _ => decode_charstring_number(cs, pos)?,
                };
                if state.stack.len() == OPERAND_LIMIT {
                    return Err(Error::malformed("CFF charstring: operand stack overflow"));
                }
                state.stack.push(v);
                pos += sz;
                match cs.get(pos) {
                    Some(&next) if (next >= 32 || next == 28) && state.budget > 0 => {
                        state.budget -= 1;
                        b = next;
                    }
                    _ => break,
                }
            }
            continue;
        }

        // CFF2 has no width, and an operator it doesn't define, return and endchar among them, is ignored and clears
        // the stack (CFF2, "Stack-based CFF2 decoding"), as do escapes other than flex.
        if cff2 {
            match b0 {
                0 | 2 | 9 | 11 | 13 | 14 | 17 => {
                    state.clear();
                    pos += 1;
                    continue;
                }
                15 => {
                    vsindex(state)?;
                    pos += 1;
                    continue;
                }
                16 => {
                    blend(state)?;
                    pos += 1;
                    continue;
                }
                12 if !matches!(cs.get(pos + 1), Some(34..=37)) => {
                    cs.get(pos + 1).ok_or(Error::malformed("CFF charstring: truncated escape operator"))?;
                    state.clear();
                    pos += 2;
                    continue;
                }
                _ => {}
            }
        }

        match b0 {
            // Stems once the count is fixed are dropped.
            1 | 3 | 18 | 23 => {
                if !state.frozen {
                    take_width_stem(state);
                    if let Some(h) = state.hinter.hinter() {
                        h.stems(&state.stack, b0 == 3 || b0 == 23);
                    }
                    record_stems(state, b0 == 3 || b0 == 23);
                    state.n_stems += state.stack.len() / 2;
                }
                state.clear();
                pos += 1;
            }
            4 => {
                take_width_fixed(state, &[1]);
                let dy = *state.stack.first().ok_or(Error::missing("CFF charstring: vmoveto missing operand"))?;
                moveto(state, pen, 0.0, dy);
                state.clear();
                pos += 1;
            }
            5 => {
                let n = state.stack.len();
                if !n.is_multiple_of(2) {
                    return Err(Error::malformed("CFF charstring: rlineto needs an even operand count"));
                }
                let mut i = 0;
                while i + 2 <= n {
                    lineto(state, pen, state.stack[i], state.stack[i + 1]);
                    i += 2;
                }
                state.clear();
                pos += 1;
            }
            6 | 7 => {
                let mut horiz = b0 == 6;
                for i in 0..state.stack.len() {
                    let v = state.stack[i];
                    if horiz { lineto(state, pen, v, 0.0); } else { lineto(state, pen, 0.0, v); }
                    horiz = !horiz;
                }
                state.clear();
                pos += 1;
            }
            8 => {
                let n = state.stack.len();
                if !n.is_multiple_of(6) || n == 0 {
                    return Err(Error::malformed(
                        "CFF charstring: rrcurveto needs a nonzero multiple-of-6 operand count",
                    ));
                }
                let mut i = 0;
                while i + 6 <= n {
                    draw_rrcurve(state, pen, six(&state.stack, i));
                    i += 6;
                }
                state.clear();
                pos += 1;
            }
            10 => {
                let idx = pop_int(state)?;
                let i = subr_index(idx, local_bias, local_subrs.len())
                    .ok_or(Error::malformed("CFF charstring: local subr index out of range"))?;
                let subr = subr_bytes(cff, local_subrs, i)?;
                if let Some(slot) = state.called.as_mut().and_then(|c| c.local.get_mut(c.base + i)) { *slot = true; }
                state.depth += 1;
                run::<CFF2>(cff, subr, global_subrs, local_subrs, global_bias, local_bias, state, pen)?;
                state.depth -= 1;
                if state.ended {
                    return Ok(());
                }
                pos += 1;
            }
            11 => { return Ok(()); }
            // endchar ends the glyph wherever it stands; four operands are a seac's.
            14 => {
                // A seac's codes are whole numbers, checked before any width beneath them is taken.
                let n = state.stack.len();
                if n >= 4 && state.stack[n - 2..].iter().any(|&v| v != f64::from(v as i32)) {
                    return Err(Error::malformed("CFF charstring: a seac code that isn't an integer"));
                }
                take_width_fixed(state, &[0, 4]);
                match state.stack[..] {
                    [] => {}
                    [adx, ady, bchar, achar] if (0.0..=255.0).contains(&bchar) && (0.0..=255.0).contains(&achar) => {
                        state.seac = Some((adx, ady, bchar as u8, achar as u8));
                    }
                    _ => return Err(Error::malformed("CFF charstring: endchar has leftover operands")),
                }
                state.clear();
                state.ended = true;
                return Ok(());
            }
            19 | 20 => {
                // Operands once the count is fixed are stems declared late, dropped as any are; the mask still reads.
                if !state.stack.is_empty() {
                    if !state.frozen {
                        take_width_stem(state);
                        if let Some(h) = state.hinter.hinter() {
                            h.stems(&state.stack, true);
                        }
                        record_stems(state, true);
                        state.n_stems += state.stack.len() / 2;
                    }
                    state.clear();
                }
                // Type 2's limit; CFF2 sets none.
                if state.n_stems > MAX_STEMS && !CFF2 {
                    return Err(Error::malformed("CFF charstring: more than 96 stem hints"));
                }
                // Bytes past the charstring's end read as 0, and it ends there.
                let mask_bytes = state.n_stems.div_ceil(8);
                let end = pos + 1 + mask_bytes;
                if let Some(h) = state.hinter.hinter() {
                    h.mask(&cs[pos + 1..end.min(cs.len())], b0 == 20);
                }
                state.frozen |= b0 == 19;
                state.mask_fresh |= b0 == 19;
                // A mask kept is charged a step per byte, its entry's included, or one in a subroutine
                // called again and again keeps gigabytes. The budget holds a glyph's to 64 bytes for each it reads.
                if b0 == 19 && state.hints.is_some() {
                    let held = mask_bytes + core::mem::size_of::<(usize, Vec<u8>)>();
                    state.budget = state.budget.checked_sub(held).ok_or(SPENT)?;
                    let mut mask = cs[pos + 1..end.min(cs.len())].to_vec();
                    mask.resize(mask_bytes, 0);
                    state.mask = Some(mask);
                }
                pos = end;
            }
            21 => {
                take_width_fixed(state, &[2]);
                if state.stack.len() < 2 { return Err(Error::missing("CFF charstring: rmoveto missing operands")); }
                let (dx, dy) = (state.stack[0], state.stack[1]);
                moveto(state, pen, dx, dy);
                state.clear();
                pos += 1;
            }
            22 => {
                take_width_fixed(state, &[1]);
                let dx = *state.stack.first().ok_or(Error::missing("CFF charstring: hmoveto missing operand"))?;
                moveto(state, pen, dx, 0.0);
                state.clear();
                pos += 1;
            }
            24 => {
                let n = state.stack.len();
                if n < 8 || !(n - 2).is_multiple_of(6) {
                    return Err(Error::malformed("CFF charstring: rcurveline needs 6k+2 operands"));
                }
                let mut i = 0;
                while n - i > 2 {
                    draw_rrcurve(state, pen, six(&state.stack, i));
                    i += 6;
                }
                let (a, b) = (state.stack[i], state.stack[i + 1]);
                lineto(state, pen, a, b);
                state.clear();
                pos += 1;
            }
            25 => {
                let n = state.stack.len();
                if n < 8 || !(n - 6).is_multiple_of(2) {
                    return Err(Error::malformed("CFF charstring: rlinecurve needs 2k+6 operands"));
                }
                let mut i = 0;
                while n - i > 6 {
                    let (a, b) = (state.stack[i], state.stack[i + 1]);
                    lineto(state, pen, a, b);
                    i += 2;
                }
                draw_rrcurve(state, pen, six(&state.stack, i));
                state.clear();
                pos += 1;
            }
            26 => {
                let n = state.stack.len();
                let mut i = 0;
                let mut cur_x = state.x;
                if n % 4 == 1 { cur_x = state.x + state.stack[0]; i = 1; }
                if !(n - i).is_multiple_of(4) || n == i {
                    return Err(Error::malformed("CFF charstring: vvcurveto has a malformed operand count"));
                }
                while i + 4 <= n {
                    let [dya, dxb, dyb, dyc] = four(&state.stack, i);
                    let c1x = cur_x; let c1y = state.y + dya;
                    let c2x = c1x + dxb; let c2y = c1y + dyb;
                    let ex = c2x; let ey = c2y + dyc;
                    curveto(state, pen, c1x, c1y, c2x, c2y, ex, ey);
                    cur_x = state.x;
                    i += 4;
                }
                state.clear();
                pos += 1;
            }
            27 => {
                let n = state.stack.len();
                let mut i = 0;
                let mut cur_y = state.y;
                if n % 4 == 1 { cur_y = state.y + state.stack[0]; i = 1; }
                if !(n - i).is_multiple_of(4) || n == i {
                    return Err(Error::malformed("CFF charstring: hhcurveto has a malformed operand count"));
                }
                while i + 4 <= n {
                    let [dxa, dxb, dyb, dxc] = four(&state.stack, i);
                    let c1x = state.x + dxa; let c1y = cur_y;
                    let c2x = c1x + dxb; let c2y = c1y + dyb;
                    let ex = c2x + dxc; let ey = c2y;
                    curveto(state, pen, c1x, c1y, c2x, c2y, ex, ey);
                    cur_y = state.y;
                    i += 4;
                }
                state.clear();
                pos += 1;
            }
            29 => {
                let idx = pop_int(state)?;
                let i = subr_index(idx, global_bias, global_subrs.len())
                    .ok_or(Error::malformed("CFF charstring: global subr index out of range"))?;
                let subr = subr_bytes(cff, global_subrs, i)?;
                if let Some(slot) = state.called.as_mut().and_then(|c| c.global.get_mut(i)) { *slot = true; }
                state.depth += 1;
                run::<CFF2>(cff, subr, global_subrs, local_subrs, global_bias, local_bias, state, pen)?;
                state.depth -= 1;
                if state.ended {
                    return Ok(());
                }
                pos += 1;
            }
            30 | 31 => {
                let n = state.stack.len();
                if n < 4 || !(n.is_multiple_of(4) || n % 4 == 1) {
                    return Err(Error::malformed("CFF charstring: vh/hvcurveto has a malformed operand count"));
                }
                let mut horiz = b0 == 31;
                let mut i = 0;
                while i + 4 <= n {
                    let is_last_group = i + 8 > n;
                    let last_extra = if is_last_group && n % 4 == 1 { Some(state.stack[n - 1]) } else { None };
                    let [a, b, c, d] = four(&state.stack, i);
                    if horiz {
                        let (dx1, dx2, dy2, dy3) = (a, b, c, d);
                        let c1x = state.x + dx1; let c1y = state.y;
                        let c2x = c1x + dx2; let c2y = c1y + dy2;
                        let ex = c2x + last_extra.unwrap_or(0.0); let ey = c2y + dy3;
                        curveto(state, pen, c1x, c1y, c2x, c2y, ex, ey);
                    } else {
                        let (dy1, dx2, dy2, dx3) = (a, b, c, d);
                        let c1x = state.x; let c1y = state.y + dy1;
                        let c2x = c1x + dx2; let c2y = c1y + dy2;
                        let ex = c2x + dx3; let ey = c2y + last_extra.unwrap_or(0.0);
                        curveto(state, pen, c1x, c1y, c2x, c2y, ex, ey);
                    }
                    i += 4;
                    horiz = !horiz;
                }
                state.clear();
                pos += 1;
            }
            12 => {
                let b1 = *cs.get(pos + 1).ok_or(Error::malformed("CFF charstring: truncated escape operator"))?;
                match b1 {
                    // dotsection: obsolete hint suspension, a no-op as TN 5177 says.
                    0 => state.clear(),
                    34 => { draw_hflex(state, pen)?; state.clear(); }
                    35 => { draw_flex(state, pen)?; state.clear(); }
                    36 => { draw_hflex1(state, pen)?; state.clear(); }
                    37 => { draw_flex1(state, pen)?; state.clear(); }
                    _ => arithmetic(state, b1)?,
                }
                pos += 2;
            }
            _ => {
                return Err(Error::owned(
                    crate::daecore::ErrorKind::Unsupported,
                    format!("CFF charstring: unsupported operator {}", b0),
                ));
            }
        }
    }
    Ok(())
}

// A count `vsindex` or `blend` pops: a whole number, never below 0.
fn pop_count(state: &mut State<'_, impl Hook>) -> Result<i32, Error> {
    match pop_int(state)? {
        count if count < 0 => Err(Error::malformed("CFF2 charstring: a negative count")),
        count => Ok(count),
    }
}

// vsindex, which may come once in a charstring and before its first blend (CFF2, "vsindex"); a second, or one after
// a blend, fails the glyph.
fn vsindex(state: &mut State<'_, impl Hook>) -> Result<(), Error> {
    if let Some(c) = state.cff2.as_mut() {
        let read = core::mem::replace(&mut c.blending.vsindex_read, true);
        if read || c.blending.built.is_some() {
            return Err(Error::malformed("CFF2 charstring: vsindex after blend or vsindex"));
        }
    }
    let v = pop_count(state)?;
    if let Some(c) = state.cff2.as_mut() {
        c.vsindex = v as u32;
    }
    state.clear();
    Ok(())
}

// blend: each of the last n defaults plus its deltas, each scaled by its region's weight in 16.16, summed with 32-bit
// wrapping. The results are reals.
fn blend(state: &mut State<'_, impl Hook>) -> Result<(), Error> {
    use super::super::instancer::drawn_mul_fix;
    let Some(c) = state.cff2.as_ref() else { return Ok(()) };
    if c.blending.built.is_none() {
        let (weights, vsindex) = (c.blending.weights, c.vsindex);
        // Weighing reads every region's every axis, which one blend can make millions, so it's charged first.
        state.budget = state.budget.checked_sub(weights.work(vsindex)).ok_or(SPENT)?;
        let all = weights.weights(vsindex)?;
        // A region weighing 0 adds 0, as most do at the default and away from their axes.
        let varied = all.iter().skip(1).enumerate().filter(|&(_, &w)| w != 0).map(|(j, &w)| (j, w)).collect();
        if let Some(c) = state.cff2.as_mut() {
            c.blending.built = Some((all.len(), varied));
        }
    }
    let n = pop_count(state)? as usize;
    let State {
        cff2: Some(Cff2Run { blending: Blending { built: Some((per, varied)), .. }, .. }), stack, budget, ..
    } = state
    else {
        return Ok(());
    };
    let (count, per) = (stack.len(), *per);
    if per == 0 || n > count / per {
        return Err(Error::malformed("CFF2 charstring: blend underflows the stack"));
    }
    // The n defaults stay on the stack as the results, where the next blend reads them again, so each read is a step.
    *budget = budget.checked_sub(n).ok_or(SPENT)?;
    let base = count - n * per;
    for i in 0..n {
        let deltas = base + n + i * (per - 1);
        let sum = varied.iter().fold(to_fixed(stack[base + i]), |sum, &(j, w)| {
            let delta = drawn_mul_fix(i64::from(w), i64::from(to_fixed(stack[deltas + j])));
            sum.wrapping_add(held(delta.unsigned_abs(), delta < 0))
        });
        stack[base + i] = f64::from(sum) / 65536.0;
    }
    stack.truncate(base + n);
    Ok(())
}

// An operand where Type 2 wants an integer, a subroutine number, an index or a count: a whole number however it was
// written, and one with a fraction fails the glyph.
fn pop_int(state: &mut State<'_, impl Hook>) -> Result<i32, Error> {
    let v = state.stack.pop().ok_or(Error::missing("CFF charstring: an operand missing"))?;
    match v == f64::from(v as i32) {
        true => Ok(v as i32),
        false => Err(Error::malformed("CFF charstring: a fraction where an integer belongs")),
    }
}

// Type 2's arithmetic, storage and stack operators (TN 5177, 4.4 to 4.6) in 16.16, each result an integer or a
// 16.16 as its operator types it; any other escape clears the stack.
#[cold]
fn arithmetic(state: &mut State<'_, impl Hook>, op: u8) -> Result<(), Error> {
    let s = state;
    match op {
        3 | 4 | 15 => {
            let (b, a) = (s.pop_fixed()?, s.pop_fixed()?);
            let v = match op {
                3 => a != 0 && b != 0,
                4 => a != 0 || b != 0,
                _ => a == b,
            };
            s.push_typed(i32::from(v), false);
        }
        5 => {
            let a = s.pop_fixed()?;
            s.push_typed(i32::from(a == 0), false);
        }
        // The one value with no opposite gives the largest.
        9 | 14 => {
            let v = match s.pop_fixed()? {
                i32::MIN => i32::MAX,
                a if op == 9 => a.abs(),
                a => -a,
            };
            s.push_typed(v, true);
        }
        10 | 11 | 12 | 24 => {
            let (b, a) = (s.pop_fixed()?, s.pop_fixed()?);
            let v = match op {
                10 => a.saturating_add(b),
                11 => a.saturating_sub(b),
                12 => divide(a, b),
                _ => multiply(a, b),
            };
            s.push_typed(v, true);
        }
        18 => drop(s.pop_fixed()?),
        20 => {
            let i = pop_int(s)?;
            let v = s.pop_fixed()?;
            if let Some(slot) = s.transient.get_mut(i as u32 as usize) {
                *slot = v;
            }
        }
        21 => {
            let i = pop_int(s)?;
            if let Some(&v) = s.transient.get(i as u32 as usize) {
                s.push_typed(v, true);
            }
        }
        22 => {
            let (v2, v1, s2, s1) = (s.pop_fixed()?, s.pop_fixed()?, s.pop_fixed()?, s.pop_fixed()?);
            s.push_typed(if v1 <= v2 { s1 } else { s2 }, true);
        }
        // A number in (0, 1], as Type 2 asks: a 64-bit linear congruential step (Knuth's MMIX constants), its top 16
        // bits plus 1 in 65,536ths.
        23 => {
            let state = s.random.unwrap_or(u64::from(s.seed));
            let state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            s.random = Some(state);
            s.push_typed((state >> 48) as i32 + 1, true);
        }
        26 => {
            let a = s.pop_fixed()?;
            s.push_typed(if a > 0 { sqrt_fixed(a as u32) as i32 } else { 0 }, true);
        }
        27 => {
            let a = s.pop_fixed()?;
            s.push_typed(a, true);
            s.push_typed(a, true);
        }
        28 => {
            let (b, a) = (s.pop_fixed()?, s.pop_fixed()?);
            s.push_typed(b, true);
            s.push_typed(a, true);
        }
        // A negative index copies the top, one past the stack its bottom, and an empty stack nothing.
        29 => {
            let i = pop_int(s)?;
            if let Some(top) = s.stack.len().checked_sub(1) {
                let at = usize::try_from(i).map_or(top, |i| top.saturating_sub(i));
                let v = to_fixed(s.stack[at]);
                s.push_typed(v, true);
            }
        }
        30 => {
            let (j, n) = (pop_int(s)?, pop_int(s)?);
            roll(s, n, j)?;
        }
        _ => s.clear(),
    }
    if s.stack.len() > OPERAND_LIMIT {
        return Err(Error::malformed("CFF charstring: operand stack overflow"));
    }
    Ok(())
}

// The top n operands turned j places toward the top; n past the stack fails the glyph.
fn roll(s: &mut State<'_, impl Hook>, n: i32, j: i32) -> Result<(), Error> {
    let n = match usize::try_from(n) {
        Ok(n) if n >= 2 => n,
        _ => return Ok(()),
    };
    let len = s.stack.len();
    let base = len.checked_sub(n).ok_or(Error::malformed("CFF charstring: roll past the stack"))?;
    let shift = i64::from(j).rem_euclid(n as i64) as usize;
    s.stack[base..].rotate_right(shift);
    Ok(())
}

// Type 2's mul and div (TN 5177, 4.4) on 16.16 operands: the exact result to the nearest 16.16, a half away from
// zero, held to 16.16's range where the spec leaves an overflow undefined.
fn multiply(a: i32, b: i32) -> i32 {
    let product = i64::from(a) * i64::from(b);
    let magnitude = (product.unsigned_abs() + 0x8000) >> 16;
    held(magnitude, product < 0)
}

// Division by 0 gives the largest value of the dividend's sign, and 0 / 0 gives 0.
fn divide(a: i32, b: i32) -> i32 {
    let (n, d) = (u64::from(a.unsigned_abs()) << 16, u64::from(b.unsigned_abs()));
    let magnitude = match d {
        0 if n == 0 => 0,
        0 => u64::MAX,
        d => (n + d / 2) / d,
    };
    held(magnitude, (a < 0) != (b < 0))
}

fn held(magnitude: u64, negative: bool) -> i32 {
    let v = i32::try_from(magnitude).unwrap_or(i32::MAX);
    if negative { -v } else { v }
}

// A 16.16 square root rounded to the nearest 65,536th: √(v / 2^16) in 16.16 is √(v · 2^16), and of the whole roots
// around it the nearer is the one whose square v · 2^16 passes by no more than the root itself.
fn sqrt_fixed(v: u32) -> u32 {
    let x = u64::from(v) << 16;
    let r = x.isqrt();
    (if x - r * r > r { r + 1 } else { r }) as u32
}

// A stack value in 16.16, its fraction cut toward 0 and held to 16.16's range rather than wrapped.
fn to_fixed(v: f64) -> i32 {
    let v = (v * 65536.0) as i64;
    held(v.unsigned_abs(), v < 0)
}

#[inline]
fn six(s: &[f64], i: usize) -> [f64; 6] {
    [s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]]
}

#[inline]
fn four(s: &[f64], i: usize) -> [f64; 4] {
    [s[i], s[i + 1], s[i + 2], s[i + 3]]
}

fn draw_rrcurve(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>, ops: [f64; 6]) {
    let (dxa, dya, dxb, dyb, dxc, dyc) = (ops[0], ops[1], ops[2], ops[3], ops[4], ops[5]);
    let c1x = state.x + dxa; let c1y = state.y + dya;
    let c2x = c1x + dxb; let c2y = c1y + dyb;
    let ex = c2x + dxc; let ey = c2y + dyc;
    curveto(state, pen, c1x, c1y, c2x, c2y, ex, ey);
}

fn draw_hflex(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>) -> Result<(), Error> {
    let s = &state.stack;
    if s.len() != 7 { return Err(Error::malformed("CFF charstring: hflex needs exactly 7 operands")); }
    let (dx1, dx2, dy2, dx3, dx4, dx5, dx6) = (s[0], s[1], s[2], s[3], s[4], s[5], s[6]);
    let y0 = state.y;
    let c1x = state.x + dx1; let c1y = state.y;
    let c2x = c1x + dx2; let c2y = c1y + dy2;
    let ex1 = c2x + dx3; let ey1 = c2y;
    curveto(state, pen, c1x, c1y, c2x, c2y, ex1, ey1);
    let c3x = state.x + dx4; let c3y = state.y;
    let c4x = c3x + dx5; let c4y = y0;
    let ex2 = c4x + dx6; let ey2 = y0;
    curveto(state, pen, c3x, c3y, c4x, c4y, ex2, ey2);
    Ok(())
}

fn draw_flex(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>) -> Result<(), Error> {
    if state.stack.len() != 13 { return Err(Error::malformed("CFF charstring: flex needs exactly 13 operands")); }
    let (first, second) = (six(&state.stack, 0), six(&state.stack, 6));
    draw_rrcurve(state, pen, first);
    draw_rrcurve(state, pen, second);
    Ok(())
}

fn draw_hflex1(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>) -> Result<(), Error> {
    let s = &state.stack;
    if s.len() != 9 { return Err(Error::malformed("CFF charstring: hflex1 needs exactly 9 operands")); }
    let (dx1, dy1, dx2, dy2, dx3, dx4, dx5, dy5, dx6) = (s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8]);
    let y0 = state.y;
    let c1x = state.x + dx1; let c1y = state.y + dy1;
    let c2x = c1x + dx2; let c2y = c1y + dy2;
    let ex1 = c2x + dx3; let ey1 = c2y;
    curveto(state, pen, c1x, c1y, c2x, c2y, ex1, ey1);
    let c3x = state.x + dx4; let c3y = state.y;
    let c4x = c3x + dx5; let c4y = c3y + dy5;
    let ex2 = c4x + dx6; let ey2 = y0;
    curveto(state, pen, c3x, c3y, c4x, c4y, ex2, ey2);
    Ok(())
}

fn draw_flex1(state: &mut State<'_, impl Hook>, pen: &mut CloseElidingPen<'_>) -> Result<(), Error> {
    let s = &state.stack;
    if s.len() != 11 { return Err(Error::malformed("CFF charstring: flex1 needs exactly 11 operands")); }
    let (dx1, dy1, dx2, dy2, dx3, dy3, dx4, dy4, dx5, dy5, d6) =
        (s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8], s[9], s[10]);
    let (x0, y0) = (state.x, state.y);
    let c1x = state.x + dx1; let c1y = state.y + dy1;
    let c2x = c1x + dx2; let c2y = c1y + dy2;
    let ex1 = c2x + dx3; let ey1 = c2y + dy3;
    curveto(state, pen, c1x, c1y, c2x, c2y, ex1, ey1);
    let c3x = state.x + dx4; let c3y = state.y + dy4;
    let c4x = c3x + dx5; let c4y = c3y + dy5;
    let dx_total = dx1 + dx2 + dx3 + dx4 + dx5;
    let dy_total = dy1 + dy2 + dy3 + dy4 + dy5;
    let (ex2, ey2) = if dx_total.abs() > dy_total.abs() {
        (c4x + d6, y0)
    } else {
        (x0, c4y + d6)
    };
    curveto(state, pen, c3x, c3y, c4x, c4y, ex2, ey2);
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/outline/cff_pen.rs"]
mod tests;

