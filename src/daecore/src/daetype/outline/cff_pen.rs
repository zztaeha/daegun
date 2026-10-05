use alloc::string::String;
use alloc::vec::Vec;
#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daemachine::float::FloatExt;
use super::pen::OutlinePen;
use super::super::format::cff::{decode_charstring_number, resolve_fd_select, subr_bias};
use super::super::subsetter::{
    cff_index_end, cff_index_spans, parse_cff_index_refs, parse_top_dict, parse_private_subrs_offset, parse_fd_dict_private,
    standard_encoding_sid, SidMap,
};
use crate::daecore::sync::{read, write, Mutable};

type Span = (u32, u32);

pub struct CffOutlines {
    charstrings:  Vec<Span>,
    global_subrs: Vec<Span>,
    local_subrs:  Vec<Vec<Span>>,
    fd_select:    Option<Vec<u16>>,
    charset_off:  Option<usize>,
    charset_predefined: u8,
    // Built on the first seac, which most fonts never draw.
    sid_map:      Mutable<Option<SidMap>>,
}

impl CffOutlines {
    pub fn parse(cff: &[u8]) -> Result<CffOutlines, String> {
        if cff.len() < 4 {
            return Err("CFF: file too short".into());
        }
        let hdr_size = cff[2] as usize;

        let after_name             = cff_index_end(cff, hdr_size, false)?;
        let (top_dicts, after_top) = parse_cff_index_refs(cff, after_name, false)?;
        let top_dict = top_dicts.into_iter().next().ok_or("CFF: empty Top DICT INDEX")?;
        let fields = parse_top_dict(top_dict)?;

        let after_strings      = cff_index_end(cff, after_top, false)?;
        let (global_subrs, _)  = cff_index_spans(cff, after_strings, false)?;

        let (charstrings, _) = cff_index_spans(cff, fields.charstrings_off, false)?;
        let n_glyphs = charstrings.len();

        let (fd_select, local_subrs) = if let Some(fd_array_off) = fields.fd_array_off {
            let fd_select_off = fields.fd_select_off.ok_or("CFF CID: missing FDSelect offset")?;
            let fd_select = resolve_fd_select(cff, fd_select_off, n_glyphs)?;
            let (fd_dicts, _) = parse_cff_index_refs(cff, fd_array_off, false)?;
            let mut per_fd = Vec::with_capacity(fd_dicts.len());
            for fd_dict in &fd_dicts {
                let (priv_size, priv_off, _) = parse_fd_dict_private(fd_dict);
                per_fd.push(local_subrs_at(cff, priv_off, priv_size)?);
            }
            (Some(fd_select), per_fd)
        } else {
            (None, vec![local_subrs_at(cff, fields.private_off, fields.private_size)?])
        };

        Ok(CffOutlines {
            charstrings,
            global_subrs,
            local_subrs,
            fd_select,
            charset_off: fields.charset_off,
            charset_predefined: fields.charset_predefined,
            sid_map: Mutable::new(None),
        })
    }

    pub fn num_glyphs(&self) -> usize {
        self.charstrings.len()
    }

    // A seac locates its components by name, and a CID-keyed font names no glyph.
    fn glyph_of_sid(&self, cff: &[u8], sid: u16) -> Option<u16> {
        if self.fd_select.is_some() {
            return None;
        }
        if let Some(map) = read(&self.sid_map).as_ref() {
            return map.gid(sid);
        }
        let map = SidMap::new(cff, self.charset_off, self.charset_predefined, self.charstrings.len());
        let gid = map.gid(sid);
        *write(&self.sid_map) = Some(map);
        gid
    }

    // The base and accent glyphs a seac draws, wherever its endchar sits: after hints, inside a
    // subroutine. Each is None when the charset names no such glyph, as in every CID-keyed font.
    pub fn seac_glyphs(&self, cff: &[u8], gid: u16) -> Option<[Option<u16>; 2]> {
        if self.fd_select.is_some() {
            return None;
        }
        let (charstring, local_subrs) = glyph_program(self, cff, gid).ok()?;
        let (_, seac) = draw_charstring(cff, charstring, &self.global_subrs, local_subrs, None, &mut NullPen).ok()?;
        let (_, _, bchar, achar) = seac?;
        Some([bchar, achar].map(|code| self.glyph_of_sid(cff, standard_encoding_sid(code))))
    }

    // Where each global subroutine sits in the font, and each of an FD's local ones.
    pub(crate) fn global_subr_spans(&self) -> &[Span] {
        &self.global_subrs
    }

    pub(crate) fn local_subr_spans(&self, fd: usize) -> &[Span] {
        self.local_subrs.get(fd).map_or(&[], |l| &l[..])
    }

    // The global and each FD's local subroutines drawing these glyphs calls. None where the font has none,
    // or a glyph does not draw or draws with `random`, whose values another reader need not share.
    pub(crate) fn subrs_called(&self, cff: &[u8], glyphs: &[u16]) -> Option<(Vec<bool>, Vec<Vec<bool>>)> {
        if self.global_subrs.is_empty() && self.local_subrs.iter().all(Vec::is_empty) {
            return None;
        }
        let mut global = vec![false; self.global_subrs.len()];
        let mut local: Vec<Vec<bool>> = self.local_subrs.iter().map(|l| vec![false; l.len()]).collect();
        for &gid in glyphs {
            let charstring = span_bytes(cff, *self.charstrings.get(usize::from(gid))?).ok()?;
            let fd = self.fd_select.as_ref().map_or(Some(0), |s| s.get(usize::from(gid)).map(|&f| usize::from(f)))?;
            let local_subrs = self.local_subrs.get(fd)?;
            let mut state = State::new(None);
            state.called = Some((core::mem::take(&mut global), core::mem::take(&mut local[fd])));
            let ran = draw_with(cff, charstring, &self.global_subrs, local_subrs, &mut state, &mut NullPen);
            (global, local[fd]) = state.called.take()?;
            if ran.is_err() || state.random != 0 {
                return None;
            }
        }
        Some((global, local))
    }
}

struct NullPen;

impl OutlinePen for NullPen {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

// A glyph's charstring and the local Subrs its FD gives it.
fn glyph_program<'a>(outlines: &'a CffOutlines, cff: &'a [u8], gid: u16) -> Result<(&'a [u8], &'a [Span]), String> {
    let charstring = span_bytes(cff, *outlines
        .charstrings
        .get(gid as usize)
        .ok_or("CFF: glyph index out of range")?)?;
    let local_subrs = match &outlines.fd_select {
        Some(fd_select) => {
            let fd_idx = *fd_select.get(gid as usize).unwrap_or(&0) as usize;
            outlines
                .local_subrs
                .get(fd_idx)
                .ok_or("CFF CID: FDSelect references an FD index out of range")?
        }
        None => outlines.local_subrs.first().ok_or("CFF: no Private DICT")?,
    };
    Ok((charstring, local_subrs))
}

pub fn outline_cff_glyph_with(
    outlines: &CffOutlines,
    cff: &[u8],
    gid: u16,
    pen: &mut dyn OutlinePen,
) -> Result<(), String> {
    let (charstring, local_subrs) = glyph_program(outlines, cff, gid)?;
    draw_glyph(outlines, cff, charstring, local_subrs, None, pen).map(|_| ())
}

pub fn outline_cff_glyph_hinted(
    outlines: &CffOutlines,
    cff: &[u8],
    gid: u16,
    pen: &mut dyn OutlinePen,
) -> Result<CffHints, String> {
    let (charstring, local_subrs) = glyph_program(outlines, cff, gid)?;
    draw_glyph(outlines, cff, charstring, local_subrs, Some(CffHints::default()), pen).map(Option::unwrap_or_default)
}

// A glyph's own program, then, when its endchar names a base and an accent (seac), those two.
fn draw_glyph(
    outlines: &CffOutlines,
    cff: &[u8],
    charstring: &[u8],
    local_subrs: &[Span],
    hints: Option<CffHints>,
    pen: &mut dyn OutlinePen,
) -> Result<Option<CffHints>, String> {
    let (hints, seac) = draw_charstring(cff, charstring, &outlines.global_subrs, local_subrs, hints, pen)?;
    if let Some((adx, ady, bchar, achar)) = seac {
        draw_seac(outlines, cff, local_subrs, adx, ady, bchar, achar, pen)?;
        return Ok(hints.map(|_| CffHints::default()));
    }
    Ok(hints)
}

#[allow(clippy::too_many_arguments, reason = "a seac's four operands travel with the glyph's programs")]
fn draw_seac(
    outlines:     &CffOutlines,
    cff:          &[u8],
    local_subrs:  &[Span],
    adx: f64, ady: f64, bchar: u8, achar: u8,
    pen: &mut dyn OutlinePen,
) -> Result<(), String> {
    let (charstrings, global_subrs) = (&outlines.charstrings, &outlines.global_subrs[..]);
    let base_gid = outlines.glyph_of_sid(cff, standard_encoding_sid(bchar))
        .ok_or("CFF: seac base character not found in charset")?;
    let accent_gid = outlines.glyph_of_sid(cff, standard_encoding_sid(achar))
        .ok_or("CFF: seac accent character not found in charset")?;

    let base_cs = span_bytes(cff, *charstrings.get(base_gid as usize)
        .ok_or("CFF: seac base glyph index out of range")?)?;
    let (_, nested) = draw_charstring(cff, base_cs, global_subrs, local_subrs, None, pen)?;

    let accent_cs = span_bytes(cff, *charstrings.get(accent_gid as usize)
        .ok_or("CFF: seac accent glyph index out of range")?)?;
    let mut offset_pen = SeacOffsetPen { inner: pen, dx: adx as f32, dy: ady as f32 };
    let (_, nested_accent) = draw_charstring(cff, accent_cs, global_subrs, local_subrs, None, &mut offset_pen)?;
    if nested.is_some() || nested_accent.is_some() {
        return Err("CFF: seac base or accent is itself a seac".into());
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

struct SeacOffsetPen<'a> { inner: &'a mut dyn OutlinePen, dx: f32, dy: f32 }

impl OutlinePen for SeacOffsetPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) { self.inner.move_to(x + self.dx, y + self.dy) }
    fn line_to(&mut self, x: f32, y: f32) { self.inner.line_to(x + self.dx, y + self.dy) }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.inner.quad_to(cx + self.dx, cy + self.dy, x + self.dx, y + self.dy)
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.inner.curve_to(c1x + self.dx, c1y + self.dy, c2x + self.dx, c2y + self.dy, x + self.dx, y + self.dy)
    }
    fn close(&mut self) { self.inner.close() }
}

fn local_subrs_at(cff: &[u8], priv_off: usize, priv_size: usize) -> Result<Vec<Span>, String> {
    if priv_size == 0 {
        return Ok(vec![]);
    }
    let priv_end = priv_off.saturating_add(priv_size);
    let priv_data = cff.get(priv_off..priv_end).ok_or("CFF: Private DICT out of bounds")?;
    let subrs_rel = parse_private_subrs_offset(priv_data);
    if subrs_rel == 0 {
        return Ok(vec![]);
    }
    let abs = priv_off + subrs_rel;
    if abs >= cff.len() {
        return Ok(vec![]);
    }
    Ok(cff_index_spans(cff, abs, false)?.0)
}

const MAX_SUBR_DEPTH: usize = 10;

// Type 2 allows 48 operands, but CFF that 1.1.7 converted from CFF2 carries up to CFF2's 513, which still
// keeps the stack small.
const OPERAND_LIMIT: usize = 513;
const TYPE2_OPERANDS: usize = 48;

// Edges as the charstring states them: an edge hint keeps its width of -20 or -21, so max is below min.
#[derive(Clone, Copy, Debug)]
pub struct CffStem {
    pub min: f32,
    pub max: f32,
    pub vertical: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CffHints {
    pub stems: Vec<CffStem>,
    pub masks: Vec<(usize, Vec<u8>)>,
}

type Seac = (f64, f64, u8, u8);

struct State {
    stack:       Vec<f64>,
    x:           f64,
    y:           f64,
    n_stems:     usize,
    width_taken: bool,
    open_path:   bool,
    depth:       usize,
    budget:      u32,
    hints:       Option<CffHints>,
    shadow:      Shadow,
    mask:        Option<Vec<u8>>,
    ended:       bool,
    seac:        Option<Seac>,
    transient:   [f64; 32],
    random:      u32,
    // The global and local subroutines called, marked where a subset asks which it keeps.
    called:      Option<(Vec<bool>, Vec<bool>)>,
}

impl State {
    fn new(hints: Option<CffHints>) -> State {
        State {
            stack: Vec::with_capacity(TYPE2_OPERANDS), x: 0.0, y: 0.0, n_stems: 0, width_taken: false,
            open_path: false, depth: 0, budget: MAX_CHARSTRING_STEPS, hints, shadow: Shadow::default(),
            mask: None, ended: false, seac: None, transient: [0.0; 32], random: 0, called: None,
        }
    }

    // A hint mask applies from the first point of the construction after it, numbered as a
    // collecting pen keeps the points (see `Shadow`).
    fn stamp_mask(&mut self) {
        if let (Some(mask), Some(h)) = (self.mask.take(), self.hints.as_mut()) {
            h.masks.push((self.shadow.collected + self.shadow.open, mask));
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

const MAX_CHARSTRING_STEPS: u32 = 1_000_000;

// The charstring drawn, then its hints (when asked for) and any seac its endchar named.
pub(crate) fn draw_charstring(
    cff:          &[u8],
    charstring:   &[u8],
    global_subrs: &[Span],
    local_subrs:  &[Span],
    hints:        Option<CffHints>,
    pen:          &mut dyn OutlinePen,
) -> Result<(Option<CffHints>, Option<Seac>), String> {
    let mut state = State::new(hints);
    draw_with(cff, charstring, global_subrs, local_subrs, &mut state, pen)?;
    Ok((state.hints, state.seac))
}

// The one caller of `run`: drawing loses a tenth of its speed when it has two.
fn draw_with(
    cff:          &[u8],
    charstring:   &[u8],
    global_subrs: &[Span],
    local_subrs:  &[Span],
    state:        &mut State,
    pen:          &mut dyn OutlinePen,
) -> Result<(), String> {
    let mut pen = CloseElidingPen { inner: pen, start: (0.0, 0.0), pending: None };
    let pen = &mut pen;
    let global_bias = subr_bias(global_subrs.len());
    let local_bias  = subr_bias(local_subrs.len());
    run(cff, charstring, global_subrs, local_subrs, global_bias, local_bias, state, pen)?;
    if state.open_path {
        pen.close();
    }
    Ok(())
}

fn take_width_fixed(state: &mut State, expected: &[usize]) {
    if state.width_taken { return; }
    state.width_taken = true;
    let n = state.stack.len();
    if expected.iter().any(|&c| n == c + 1) {
        state.stack.remove(0);
    }
}

fn record_stems(state: &mut State, vertical: bool) {
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

fn take_width_stem(state: &mut State) {
    if state.width_taken { return; }
    state.width_taken = true;
    if state.stack.len() % 2 == 1 {
        state.stack.remove(0);
    }
}

fn moveto(state: &mut State, pen: &mut dyn OutlinePen, dx: f64, dy: f64) {
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
    let at = (state.x as f32, state.y as f32);
    pen.move_to(at.0, at.1);
    if hinting {
        state.shadow.start = at;
        state.shadow.segment(at, 1, false);
    }
    state.open_path = true;
}

// Drawing with no moveto first starts a path at the current point, as FreeType, HarfBuzz and
// fontTools recover such a charstring.
fn open_if_needed(state: &mut State, pen: &mut dyn OutlinePen) {
    if !state.open_path {
        moveto(state, pen, 0.0, 0.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn curveto(state: &mut State, pen: &mut dyn OutlinePen, c1x: f64, c1y: f64, c2x: f64, c2y: f64, ex: f64, ey: f64) {
    open_if_needed(state, pen);
    if state.hints.is_some() {
        state.stamp_mask();
        state.shadow.segment((ex as f32, ey as f32), 3, false);
    }
    pen.curve_to(c1x as f32, c1y as f32, c2x as f32, c2y as f32, ex as f32, ey as f32);
    state.x = ex;
    state.y = ey;
}

fn lineto(state: &mut State, pen: &mut dyn OutlinePen, dx: f64, dy: f64) {
    open_if_needed(state, pen);
    state.x += dx;
    state.y += dy;
    if state.hints.is_some() {
        state.stamp_mask();
        state.shadow.segment((state.x as f32, state.y as f32), 1, true);
    }
    pen.line_to(state.x as f32, state.y as f32);
}

#[inline]
fn span_bytes(cff: &[u8], s: Span) -> Result<&[u8], String> {
    cff.get(s.0 as usize..s.1 as usize).ok_or_else(|| "CFF charstring: subr span out of bounds".into())
}

#[allow(clippy::too_many_arguments)]
fn run(
    cff:          &[u8],
    cs:           &[u8],
    global_subrs: &[Span],
    local_subrs:  &[Span],
    global_bias:  i32,
    local_bias:   i32,
    state:        &mut State,
    pen:          &mut dyn OutlinePen,
) -> Result<(), String> {
    if state.depth > MAX_SUBR_DEPTH {
        return Err("CFF charstring: subroutine nesting too deep".into());
    }
    let mut pos = 0usize;
    while pos < cs.len() {
        if state.budget == 0 {
            return Err("CFF charstring: work budget exhausted".into());
        }
        state.budget -= 1;
        let b0 = cs[pos];

        if b0 >= 32 || b0 == 28 {
            let (v, sz) = decode_charstring_number(cs, pos)?;
            if state.stack.len() == OPERAND_LIMIT {
                return Err("CFF charstring: operand stack overflow".into());
            }
            state.stack.push(v);
            pos += sz;
            continue;
        }

        match b0 {
            1 | 3 | 18 | 23 => {
                take_width_stem(state);
                record_stems(state, b0 == 3 || b0 == 23);
                state.n_stems += state.stack.len() / 2;
                state.stack.clear();
                pos += 1;
            }
            4 => {
                take_width_fixed(state, &[1]);
                let dy = *state.stack.first().ok_or("CFF charstring: vmoveto missing operand")?;
                moveto(state, pen, 0.0, dy);
                state.stack.clear();
                pos += 1;
            }
            5 => {
                let n = state.stack.len();
                if !n.is_multiple_of(2) { return Err("CFF charstring: rlineto needs an even operand count".into()); }
                let mut i = 0;
                while i + 2 <= n {
                    lineto(state, pen, state.stack[i], state.stack[i + 1]);
                    i += 2;
                }
                state.stack.clear();
                pos += 1;
            }
            6 | 7 => {
                let mut horiz = b0 == 6;
                for i in 0..state.stack.len() {
                    let v = state.stack[i];
                    if horiz { lineto(state, pen, v, 0.0); } else { lineto(state, pen, 0.0, v); }
                    horiz = !horiz;
                }
                state.stack.clear();
                pos += 1;
            }
            8 => {
                let n = state.stack.len();
                if !n.is_multiple_of(6) || n == 0 { return Err("CFF charstring: rrcurveto needs a nonzero multiple-of-6 operand count".into()); }
                let mut i = 0;
                while i + 6 <= n {
                    draw_rrcurve(state, pen, six(&state.stack, i));
                    i += 6;
                }
                state.stack.clear();
                pos += 1;
            }
            10 => {
                let idx = state.stack.pop().ok_or("CFF charstring: callsubr with empty stack")?;
                let real_idx = i64::from(idx as i32) + i64::from(local_bias);
                let i = usize::try_from(real_idx).ok().filter(|&i| i < local_subrs.len())
                    .ok_or("CFF charstring: local subr index out of range")?;
                let subr = span_bytes(cff, local_subrs[i])?;
                if let Some(slot) = state.called.as_mut().and_then(|(_, local)| local.get_mut(i)) { *slot = true; }
                state.depth += 1;
                run(cff, subr, global_subrs, local_subrs, global_bias, local_bias, state, pen)?;
                state.depth -= 1;
                if state.ended {
                    return Ok(());
                }
                pos += 1;
            }
            11 => { return Ok(()); }
            // endchar ends the glyph wherever it stands; four operands are a seac's.
            14 => {
                take_width_fixed(state, &[0, 4]);
                match state.stack[..] {
                    [] => {}
                    [adx, ady, bchar, achar] if (0.0..=255.0).contains(&bchar) && (0.0..=255.0).contains(&achar) => {
                        state.seac = Some((adx, ady, bchar as u8, achar as u8));
                    }
                    _ => return Err("CFF charstring: endchar has leftover operands".into()),
                }
                state.stack.clear();
                state.ended = true;
                return Ok(());
            }
            19 | 20 => {
                if !state.stack.is_empty() {
                    take_width_stem(state);
                    record_stems(state, true);
                    state.n_stems += state.stack.len() / 2;
                    state.stack.clear();
                }
                let mask_bytes = state.n_stems.div_ceil(8);
                if pos + 1 + mask_bytes > cs.len() {
                    return Err("CFF charstring: hintmask/cntrmask truncated".into());
                }
                // A mask kept is charged a step per byte, its entry's included, or one in a subroutine
                // called again and again keeps gigabytes. The budget holds a glyph's to a megabyte or two.
                if b0 == 19 && state.hints.is_some() {
                    state.budget = u32::try_from(mask_bytes + core::mem::size_of::<(usize, Vec<u8>)>())
                        .ok()
                        .and_then(|n| state.budget.checked_sub(n))
                        .ok_or("CFF charstring: work budget exhausted")?;
                    state.mask = Some(cs[pos + 1..pos + 1 + mask_bytes].to_vec());
                }
                pos += 1 + mask_bytes;
            }
            21 => {
                take_width_fixed(state, &[2]);
                if state.stack.len() < 2 { return Err("CFF charstring: rmoveto missing operands".into()); }
                let (dx, dy) = (state.stack[0], state.stack[1]);
                moveto(state, pen, dx, dy);
                state.stack.clear();
                pos += 1;
            }
            22 => {
                take_width_fixed(state, &[1]);
                let dx = *state.stack.first().ok_or("CFF charstring: hmoveto missing operand")?;
                moveto(state, pen, dx, 0.0);
                state.stack.clear();
                pos += 1;
            }
            24 => {
                let n = state.stack.len();
                if n < 8 || !(n - 2).is_multiple_of(6) { return Err("CFF charstring: rcurveline needs 6k+2 operands".into()); }
                let mut i = 0;
                while n - i > 2 {
                    draw_rrcurve(state, pen, six(&state.stack, i));
                    i += 6;
                }
                let (a, b) = (state.stack[i], state.stack[i + 1]);
                lineto(state, pen, a, b);
                state.stack.clear();
                pos += 1;
            }
            25 => {
                let n = state.stack.len();
                if n < 8 || !(n - 6).is_multiple_of(2) { return Err("CFF charstring: rlinecurve needs 2k+6 operands".into()); }
                let mut i = 0;
                while n - i > 6 {
                    let (a, b) = (state.stack[i], state.stack[i + 1]);
                    lineto(state, pen, a, b);
                    i += 2;
                }
                draw_rrcurve(state, pen, six(&state.stack, i));
                state.stack.clear();
                pos += 1;
            }
            26 => {
                let n = state.stack.len();
                let mut i = 0;
                let mut cur_x = state.x;
                if n % 4 == 1 { cur_x = state.x + state.stack[0]; i = 1; }
                if !(n - i).is_multiple_of(4) || n == i {
                    return Err("CFF charstring: vvcurveto has a malformed operand count".into());
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
                state.stack.clear();
                pos += 1;
            }
            27 => {
                let n = state.stack.len();
                let mut i = 0;
                let mut cur_y = state.y;
                if n % 4 == 1 { cur_y = state.y + state.stack[0]; i = 1; }
                if !(n - i).is_multiple_of(4) || n == i {
                    return Err("CFF charstring: hhcurveto has a malformed operand count".into());
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
                state.stack.clear();
                pos += 1;
            }
            29 => {
                let idx = state.stack.pop().ok_or("CFF charstring: callgsubr with empty stack")?;
                let real_idx = i64::from(idx as i32) + i64::from(global_bias);
                let i = usize::try_from(real_idx).ok().filter(|&i| i < global_subrs.len())
                    .ok_or("CFF charstring: global subr index out of range")?;
                let subr = span_bytes(cff, global_subrs[i])?;
                if let Some(slot) = state.called.as_mut().and_then(|(global, _)| global.get_mut(i)) { *slot = true; }
                state.depth += 1;
                run(cff, subr, global_subrs, local_subrs, global_bias, local_bias, state, pen)?;
                state.depth -= 1;
                if state.ended {
                    return Ok(());
                }
                pos += 1;
            }
            30 | 31 => {
                let n = state.stack.len();
                if n < 4 || !(n.is_multiple_of(4) || n % 4 == 1) {
                    return Err("CFF charstring: vh/hvcurveto has a malformed operand count".into());
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
                state.stack.clear();
                pos += 1;
            }
            12 => {
                let b1 = *cs.get(pos + 1).ok_or("CFF charstring: truncated escape operator")?;
                match b1 {
                    // dotsection: obsolete hint suspension, a no-op as TN 5177 says.
                    0 => state.stack.clear(),
                    34 => { draw_hflex(state, pen)?; state.stack.clear(); }
                    35 => { draw_flex(state, pen)?; state.stack.clear(); }
                    36 => { draw_hflex1(state, pen)?; state.stack.clear(); }
                    37 => { draw_flex1(state, pen)?; state.stack.clear(); }
                    _ => arithmetic(state, b1)?,
                }
                pos += 2;
            }
            _ => return Err(format!("CFF charstring: unsupported operator {}", b0)),
        }
    }
    Ok(())
}

// Type 2's arithmetic, storage and conditional operators (TN 5177, 4.4 to 4.6), on the stack.
fn arithmetic(state: &mut State, op: u8) -> Result<(), String> {
    let s = &mut state.stack;
    let pop = |s: &mut Vec<f64>| s.pop().ok_or_else(|| format!("CFF charstring: escape operator 12 {op} needs more operands"));
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    match op {
        3 => { let (b, a) = (pop(s)?, pop(s)?); s.push(flag(a != 0.0 && b != 0.0)); }
        4 => { let (b, a) = (pop(s)?, pop(s)?); s.push(flag(a != 0.0 || b != 0.0)); }
        5 => { let a = pop(s)?; s.push(flag(a == 0.0)); }
        9 => { let a = pop(s)?; s.push(a.abs()); }
        10 => { let (b, a) = (pop(s)?, pop(s)?); s.push(a + b); }
        11 => { let (b, a) = (pop(s)?, pop(s)?); s.push(a - b); }
        12 => { let (b, a) = (pop(s)?, pop(s)?); s.push(if b == 0.0 { 0.0 } else { a / b }); }
        14 => { let a = pop(s)?; s.push(-a); }
        15 => { let (b, a) = (pop(s)?, pop(s)?); s.push(flag(a == b)); }
        18 => { pop(s)?; }
        20 => {
            let (i, v) = (pop(s)?, pop(s)?);
            if let Some(slot) = state.transient.get_mut(i as usize).filter(|_| i >= 0.0) { *slot = v; }
        }
        21 => {
            let i = pop(s)?;
            s.push(if i >= 0.0 { state.transient.get(i as usize).copied().unwrap_or(0.0) } else { 0.0 });
        }
        22 => {
            let (v2, v1, s2, s1) = (pop(s)?, pop(s)?, pop(s)?, pop(s)?);
            s.push(if v1 <= v2 { s1 } else { s2 });
        }
        // A pseudo-random number in (0, 1], the same sequence for every glyph so drawing is repeatable.
        23 => {
            state.random = state.random.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            s.push(f64::from((state.random >> 16) & 0x7FFF).max(1.0) / 32_768.0);
        }
        24 => { let (b, a) = (pop(s)?, pop(s)?); s.push(a * b); }
        26 => { let a = pop(s)?; s.push(if a > 0.0 { a.sqrt() } else { 0.0 }); }
        27 => { let a = *s.last().ok_or("CFF charstring: dup with an empty stack")?; s.push(a); }
        28 => { let (b, a) = (pop(s)?, pop(s)?); s.push(b); s.push(a); }
        29 => {
            let i = pop(s)?;
            let len = s.len();
            let at = if i < 0.0 { 0 } else { i as usize };
            let v = *s.get(len.checked_sub(at + 1).ok_or("CFF charstring: index past the stack")?).ok_or("CFF charstring: index past the stack")?;
            s.push(v);
        }
        30 => {
            let (j, n) = (pop(s)?, pop(s)?);
            let n = n as usize;
            let len = s.len();
            if n > 0 && n <= len {
                let part = &mut s[len - n..];
                let shift = (j as i64).rem_euclid(n as i64) as usize;
                part.rotate_right(shift);
            }
        }
        _ => return Err(format!("CFF charstring: unsupported escape operator 12 {op}")),
    }
    if state.stack.len() > OPERAND_LIMIT {
        return Err("CFF charstring: operand stack overflow".into());
    }
    Ok(())
}

#[inline]
fn six(s: &[f64], i: usize) -> [f64; 6] {
    [s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]]
}

#[inline]
fn four(s: &[f64], i: usize) -> [f64; 4] {
    [s[i], s[i + 1], s[i + 2], s[i + 3]]
}

fn draw_rrcurve(state: &mut State, pen: &mut dyn OutlinePen, ops: [f64; 6]) {
    let (dxa, dya, dxb, dyb, dxc, dyc) = (ops[0], ops[1], ops[2], ops[3], ops[4], ops[5]);
    let c1x = state.x + dxa; let c1y = state.y + dya;
    let c2x = c1x + dxb; let c2y = c1y + dyb;
    let ex = c2x + dxc; let ey = c2y + dyc;
    curveto(state, pen, c1x, c1y, c2x, c2y, ex, ey);
}

fn draw_hflex(state: &mut State, pen: &mut dyn OutlinePen) -> Result<(), String> {
    let s = &state.stack;
    if s.len() != 7 { return Err("CFF charstring: hflex needs exactly 7 operands".into()); }
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

fn draw_flex(state: &mut State, pen: &mut dyn OutlinePen) -> Result<(), String> {
    if state.stack.len() != 13 { return Err("CFF charstring: flex needs exactly 13 operands".into()); }
    let (first, second) = (six(&state.stack, 0), six(&state.stack, 6));
    draw_rrcurve(state, pen, first);
    draw_rrcurve(state, pen, second);
    Ok(())
}

fn draw_hflex1(state: &mut State, pen: &mut dyn OutlinePen) -> Result<(), String> {
    let s = &state.stack;
    if s.len() != 9 { return Err("CFF charstring: hflex1 needs exactly 9 operands".into()); }
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

fn draw_flex1(state: &mut State, pen: &mut dyn OutlinePen) -> Result<(), String> {
    let s = &state.stack;
    if s.len() != 11 { return Err("CFF charstring: flex1 needs exactly 11 operands".into()); }
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
mod tests {
    use super::*;

    // Glyph 0 calls local subroutine 0, a bare return; glyph 1 does too after pushing a random number.
    // Another reader's random could call another subroutine, so a subset of glyph 1 keeps them all.
    #[test]
    fn a_glyph_drawing_with_random_keeps_every_subroutine() {
        let cff = [&[11u8][..], &[32, 10, 14], &[12, 23, 12, 18, 32, 10, 14]].concat();
        let outlines = CffOutlines {
            charstrings: vec![(1, 4), (4, 11)], global_subrs: vec![], local_subrs: vec![vec![(0, 1)]],
            fd_select: None, charset_off: None, charset_predefined: 0, sid_map: Mutable::new(None),
        };
        assert_eq!(outlines.subrs_called(&cff, &[0]), Some((vec![], vec![vec![true]])));
        assert_eq!(outlines.subrs_called(&cff, &[0, 1]), None);
    }

    // 4,000 stems make each hintmask 500 bytes, and four levels of subroutines, each calling the one
    // below twenty times, repeat one hintmask 160,000 times: 80 MB of masks from a few kilobytes.
    #[test]
    fn hint_masks_are_charged_to_the_work_budget() {
        let mut cff = Vec::new();
        let mut spans = Vec::new();
        let mut subr = |body: Vec<u8>, cff: &mut Vec<u8>| {
            spans.push((cff.len() as u32, (cff.len() + body.len()) as u32));
            cff.extend(body);
        };
        subr([vec![19], vec![0xFF; 500], vec![11]].concat(), &mut cff);
        for level in 0..3u8 {
            subr([[32 + level, 10].repeat(20), vec![11]].concat(), &mut cff);
        }
        let mut main = Vec::new();
        for _ in 0..4000 / 24 {
            main.extend([139u8; 48]);
            main.push(1);
        }
        main.extend([139u8; 32]);
        main.push(1);
        main.extend([35u8, 10].repeat(20));

        let mut state = State::new(Some(CffHints::default()));
        let _ = run(&cff, &main, &[], &spans, subr_bias(0), subr_bias(spans.len()), &mut state, &mut NullPen);
        assert_eq!(state.n_stems, 4000, "the stems did not all register");
        let held: usize = state.hints.map_or(0, |h| h.masks.iter().map(|(_, m)| m.len()).sum());
        assert!(held <= MAX_CHARSTRING_STEPS as usize, "{held} bytes of hint masks from one glyph");
    }

    // A mask of no stems keeps no bytes but still keeps an entry, so 990 calls of a subroutine of 1,000
    // such masks would hold 990,000 entries from a charstring of 3 KB.
    #[test]
    fn empty_hint_masks_are_charged_too() {
        let cff = [vec![19u8; 1000], vec![11]].concat();
        let spans = [(0u32, cff.len() as u32)];
        let main = [32u8, 10].repeat(990);
        let mut state = State::new(Some(CffHints::default()));
        let _ = run(&cff, &main, &[], &spans, subr_bias(0), subr_bias(spans.len()), &mut state, &mut NullPen);
        let masks = state.hints.map_or(0, |h| h.masks.len());
        let held = masks * core::mem::size_of::<(usize, Vec<u8>)>();
        assert!(held <= MAX_CHARSTRING_STEPS as usize, "{masks} empty hint masks hold {held} bytes");
    }

    // CFF that 1.1.7 converted from CFF2 wrote operators of up to 513 operands, past Type 2's 48: an
    // hlineto of 513 draws them all, and one of 514 is refused.
    #[test]
    fn an_operator_of_513_operands_draws_and_one_more_does_not() {
        struct Lines(usize);
        impl OutlinePen for Lines {
            fn move_to(&mut self, _: f32, _: f32) {}
            fn line_to(&mut self, _: f32, _: f32) {
                self.0 += 1;
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn close(&mut self) {}
        }
        for (n, draws) in [(OPERAND_LIMIT, true), (OPERAND_LIMIT + 1, false)] {
            let main = [vec![139u8, 139, 21], vec![140u8; n], vec![6, 14]].concat();
            let mut state = State::new(None);
            let mut lines = Lines(0);
            let ran = run(&[], &main, &[], &[], subr_bias(0), subr_bias(0), &mut state, &mut lines);
            assert_eq!(ran.is_ok(), draws, "an hlineto of {n} operands: {ran:?}");
            if draws {
                assert_eq!(lines.0, n, "the hlineto drew {} of its {n} lines", lines.0);
            }
        }
        assert_eq!(OPERAND_LIMIT, 513, "CFF2's limit, which 1.1.7's conversions reach");
    }

    // Subroutines that push 200 each, called 4,000 times without an operator to clear them, would
    // otherwise stack 800,000 operands for `record_stems` to make stems of.
    #[test]
    fn the_operand_stack_stops_at_513() {
        let mut cff = Vec::new();
        let mut spans = Vec::new();
        let mut subr = |body: Vec<u8>, cff: &mut Vec<u8>| {
            spans.push((cff.len() as u32, (cff.len() + body.len()) as u32));
            cff.extend(body);
        };
        subr([vec![139u8; 200], vec![11]].concat(), &mut cff);
        subr([[32u8, 10].repeat(20), vec![11]].concat(), &mut cff);
        subr([[33u8, 10].repeat(20), vec![11]].concat(), &mut cff);
        let main = [[34u8, 10].repeat(10), vec![1]].concat();
        let mut state = State::new(Some(CffHints::default()));
        let ran = run(&cff, &main, &[], &spans, subr_bias(0), subr_bias(spans.len()), &mut state, &mut NullPen);
        assert!(ran.is_err(), "one hstem declared {} stems", state.n_stems);
        assert!(state.stack.len() <= OPERAND_LIMIT, "{} operands stacked", state.stack.len());
    }

    struct Record(Vec<String>);
    impl OutlinePen for Record {
        fn move_to(&mut self, x: f32, y: f32) { self.0.push(format!("M{x},{y}")); }
        fn line_to(&mut self, x: f32, y: f32) { self.0.push(format!("L{x},{y}")); }
        fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("Q{x},{y}")); }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("C{x},{y}")); }
        fn close(&mut self) { self.0.push("Z".into()); }
    }

    // Small integers as Type 2 bytes, then the program's operator bytes.
    fn n(v: i32) -> u8 { (v + 139) as u8 }

    fn drawn(cs: &[u8], subrs: &[&[u8]]) -> Result<Vec<String>, String> {
        let mut cff = Vec::new();
        let spans: Vec<Span> = subrs.iter().map(|s| {
            let at = cff.len() as u32;
            cff.extend_from_slice(s);
            (at, cff.len() as u32)
        }).collect();
        let mut pen = Record(Vec::new());
        draw_charstring(&cff, cs, &[], &spans, None, &mut pen)?;
        Ok(pen.0)
    }

    #[test]
    fn dotsection_is_a_no_op() {
        let cs = [n(10), n(10), 21, n(20), 6, 12, 0, n(20), 7, 14];
        assert_eq!(drawn(&cs, &[]).unwrap(), ["M10,10", "L30,10", "L30,30", "Z"]);
    }

    #[test]
    fn drawing_before_a_moveto_starts_at_the_current_point() {
        let cs = [n(10), n(10), 5, n(20), n(0), 5, 14];
        assert_eq!(drawn(&cs, &[]).unwrap(), ["M0,0", "L10,10", "L30,10", "Z"]);
    }

    #[test]
    fn endchar_ends_the_glyph_wherever_it_stands() {
        let cs = [n(0), n(0), 21, n(10), n(0), 5, 14, n(50), n(50), 5];
        assert_eq!(drawn(&cs, &[]).unwrap(), ["M0,0", "L10,0", "Z"]);
        let subr = [n(10), n(0), 5, 14];
        let cs = [n(0), n(0), 21, n(-107), 10, n(0), n(50), 5, n(0), n(0), 21, n(5), n(5), 5];
        assert_eq!(drawn(&cs, &[&subr]).unwrap(), ["M0,0", "L10,0", "Z"], "endchar in a subroutine");
    }

    #[test]
    fn the_arithmetic_operators_compute() {
        let program = |ops: &[u8]| [&[n(0), n(0), 21][..], ops, &[n(0), 5, 14]].concat();
        let x_of = |ops: &[u8]| drawn(&program(ops), &[]).unwrap()[1].clone();
        assert_eq!(x_of(&[n(4), n(2), 12, 12]), "L2,0", "div");
        assert_eq!(x_of(&[n(3), n(4), 12, 24]), "L12,0", "mul");
        assert_eq!(x_of(&[n(-7), 12, 9]), "L7,0", "abs");
        assert_eq!(x_of(&[n(5), n(6), n(1), 12, 29, 12, 10, 12, 10]), "L16,0", "index copies 5 over 6, then two adds");
        assert_eq!(x_of(&[n(1), n(2), n(3), n(3), n(1), 12, 30, 12, 18, 12, 18]), "L3,0", "roll brings 3 to the bottom");
        assert_eq!(x_of(&[n(5), n(9), n(1), n(2), 12, 22]), "L5,0", "ifelse keeps the first when v1 <= v2");
        assert_eq!(x_of(&[n(6), n(3), 12, 20, n(3), 12, 21]), "L6,0", "put then get");
        assert_eq!(x_of(&[n(2), n(2), 12, 15, n(1), 12, 3]), "L1,0", "eq then and");
    }

    #[test]
    fn a_subroutine_that_calls_itself_is_refused() {
        let subr = [n(-107), 10];
        let err = drawn(&[n(-107), 10], &[&subr]).unwrap_err();
        assert!(err.contains("nesting too deep"), "{err}");
    }

    // Two contours, the first closed by a curve back to its start (a dropped duplicate) and a line to it
    // (elided), then a mask: it names the second contour's first point as a collecting pen numbers it.
    #[test]
    fn a_hint_mask_counts_the_points_a_collecting_pen_keeps() {
        use super::super::super::hinting::auto::CollectPen;
        let cs = [
            n(10), n(20), 1,
            n(0), n(0), 21, n(50), n(0), 5, n(0), n(50), 5, n(-50), n(-50), 5,
            n(0), n(10), n(10), n(10), n(10), n(-30), 8,
            19, 0x80,
            n(100), n(0), 21, n(10), n(0), 5, 14,
        ];
        let mut pen = CollectPen::new();
        let (hints, _) = draw_charstring(&[], &cs, &[], &[], Some(CffHints::default()), &mut pen).unwrap();
        let points = pen.finish();
        let second = points.contour_ends[0] + 1;
        assert_eq!(hints.unwrap().masks.iter().map(|m| m.0).collect::<Vec<_>>(), [second]);
    }
}
