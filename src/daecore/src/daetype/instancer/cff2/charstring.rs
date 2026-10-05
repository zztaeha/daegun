use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daetype::format::cff::{decode_charstring_number_fx, charstring_number_error, subr_bias};
use crate::daecore::daetype::format::ivs::{ItemVariationStore, region_scalars};
use crate::daecore::daetype::format::round::banker_round_i64;

pub(crate) struct Scratch {
    stack:        [Fx; MAX_OPERANDS],
    scalar_cache: Vec<ScalarSet>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self { stack: [0; MAX_OPERANDS], scalar_cache: Vec::new() }
    }
}

const MAX_OPERANDS: usize = 513;

type Fx = i64;
const FX_SHIFT: u32 = 16;
const FX_ONE: Fx = 1 << FX_SHIFT;
const FX_TO_F64: f64 = 1.0 / FX_ONE as f64;

#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_charstring_into(
    charstring:      &[u8],
    global_subrs:    &[&[u8]],
    local_subrs:     &[&[u8]],
    default_vsindex: u16,
    vstore:          Option<&ItemVariationStore>,
    location:        &[f64],
    budget:          &mut u32,
    scratch:         &mut Scratch,
    out:             &mut Vec<u8>,
) -> Result<(), String> {
    // Both apply, and the outer one is the caller's on purpose: a per-charstring ceiling does not
    // compose, since a fresh budget per glyph makes the font's cost the ceiling times the glyph
    // count, from an input that need not grow at all.
    let per_call = (*budget).min(MAX_CHARSTRING_STEPS);
    let mut state = State {
        stack:      &mut scratch.stack,
        sp:         0,
        vsindex:    default_vsindex,
        hint_count: 0,
        depth:      0,
        budget:     per_call,
    };
    let global_bias = subr_bias(global_subrs.len());
    let local_bias  = subr_bias(local_subrs.len());
    let result = run(charstring, global_subrs, local_subrs, global_bias, local_bias, vstore, location, &mut state, out, &mut scratch.scalar_cache);
    *budget -= per_call - state.budget;
    result
}

struct ScalarSet {
    vsindex:  usize,
    scalars:  Vec<f64>,
    all_zero: bool,
}

const MAX_SUBR_DEPTH: usize = 10;

const MAX_CHARSTRING_STEPS: u32 = 1_000_000;

struct State<'a> {
    stack:      &'a mut [Fx; MAX_OPERANDS],
    sp:         usize,
    vsindex:    u16,
    hint_count: usize,
    depth:      usize,
    budget:     u32,
}

#[allow(clippy::too_many_arguments)]
fn run(
    cs:           &[u8],
    global_subrs: &[&[u8]],
    local_subrs:  &[&[u8]],
    global_bias:  i32,
    local_bias:   i32,
    vstore:       Option<&ItemVariationStore>,
    location:     &[f64],
    state:        &mut State<'_>,
    out:          &mut Vec<u8>,
    scalar_cache: &mut Vec<ScalarSet>,
) -> Result<(), String> {
    if state.depth > MAX_SUBR_DEPTH {
        return Err("CFF2 charstring: subroutine nesting too deep".into());
    }
    state.budget = state.budget
        .checked_sub(u32::try_from(cs.len()).unwrap_or(u32::MAX))
        .ok_or("CFF2 charstring: work budget exhausted")?;
    let mut sp = state.sp;
    let mut pos = 0usize;
    while pos < cs.len() {
        let b0 = cs[pos];

        if b0 >= 32 {
            if sp >= MAX_OPERANDS {
                return Err("CFF2 charstring: operand stack overflow".into());
            }
            if b0 <= 246 {
                state.stack[sp] = ((b0 as Fx) - 139) << FX_SHIFT;
                sp += 1;
                pos += 1;
                continue;
            }
            let Some((v, sz)) = decode_charstring_number_fx(cs, pos) else {
                return Err(charstring_number_error(cs, pos));
            };
            state.stack[sp] = v;
            sp += 1;
            pos += sz;
            continue;
        }
        if b0 == 28 {
            if sp >= MAX_OPERANDS {
                return Err("CFF2 charstring: operand stack overflow".into());
            }
            let Some((v, sz)) = decode_charstring_number_fx(cs, pos) else {
                return Err(charstring_number_error(cs, pos));
            };
            state.stack[sp] = v;
            sp += 1;
            pos += sz;
            continue;
        }

        match b0 {
            1 | 3 | 18 | 23 => {
                state.hint_count += sp / 2;
                emit(out, &state.stack[..sp], &mut sp, &[b0]);
                pos += 1;
            }
            19 | 20 => {
                if sp != 0 {
                    state.hint_count += sp / 2;
                }
                // The stems a mask declares with it are vstems, written apart when there are too many.
                if sp > TYPE2_OPERANDS {
                    emit(out, &state.stack[..sp], &mut sp, &[23]);
                }
                flush(out, &state.stack[..sp], &mut sp, &[b0]);
                let mask_bytes = state.hint_count.div_ceil(8);
                let mask = cs.get(pos + 1..pos + 1 + mask_bytes)
                    .ok_or("CFF2 charstring: hintmask/cntrmask truncated")?;
                out.extend_from_slice(mask);
                pos += 1 + mask_bytes;
            }
            10 => {
                let idx = pop(state.stack, &mut sp).ok_or("CFF2 charstring: callsubr with empty stack")?;
                let real_idx = i64::from(fx_to_i32(idx)) + i64::from(local_bias);
                let subr = usize::try_from(real_idx).ok().and_then(|i| local_subrs.get(i))
                    .ok_or("CFF2 charstring: local subr index out of range")?;
                state.depth += 1;
                state.sp = sp;
                run(subr, global_subrs, local_subrs, global_bias, local_bias, vstore, location, state, out, scalar_cache)?;
                sp = state.sp;
                state.depth -= 1;
                pos += 1;
            }
            29 => {
                let idx = pop(state.stack, &mut sp).ok_or("CFF2 charstring: callgsubr with empty stack")?;
                let real_idx = i64::from(fx_to_i32(idx)) + i64::from(global_bias);
                let subr = usize::try_from(real_idx).ok().and_then(|i| global_subrs.get(i))
                    .ok_or("CFF2 charstring: global subr index out of range")?;
                state.depth += 1;
                state.sp = sp;
                run(subr, global_subrs, local_subrs, global_bias, local_bias, vstore, location, state, out, scalar_cache)?;
                sp = state.sp;
                state.depth -= 1;
                pos += 1;
            }
            15 => {
                let v = pop(state.stack, &mut sp).ok_or("CFF2 charstring: vsindex with empty stack")?;
                state.vsindex = (v.max(0) >> FX_SHIFT).min(u16::MAX as Fx) as u16;
                sp = 0;
                pos += 1;
            }
            16 => {
                let vstore = vstore.ok_or("CFF2 charstring: blend operator with no vstore")?;
                let n = pop(state.stack, &mut sp).ok_or("CFF2 charstring: blend missing operand count")?;
                if n < 0 { return Err("CFF2 charstring: blend negative operand count".into()); }
                let n = (n >> FX_SHIFT) as usize;
                let vsindex = state.vsindex as usize;
                let set: &ScalarSet = if let Some(i) = scalar_cache.iter().position(|s| s.vsindex == vsindex) {
                    &scalar_cache[i]
                } else {
                    let scalars = region_scalars(vstore, vsindex, location)
                        .ok_or("CFF2 charstring: vsindex out of range in vstore")?;
                    let all_zero = scalars.iter().all(|&s| s == 0.0);
                    scalar_cache.push(ScalarSet { vsindex, scalars, all_zero });
                    &scalar_cache[scalar_cache.len() - 1]
                };
                let scalars: &[f64] = &set.scalars;
                let k = scalars.len();
                let need = n.checked_add(n.checked_mul(k).ok_or("CFF2 charstring: blend operand count overflow")?)
                    .ok_or("CFF2 charstring: blend operand count overflow")?;
                if sp < need {
                    return Err("CFF2 charstring: blend stack underflow".into());
                }
                let deltas_start   = sp - n * k;
                let defaults_start = deltas_start - n;
                if set.all_zero {
                    sp = defaults_start + n;
                    pos += 1;
                    continue;
                }
                if k == 1 {
                    let s = scalars[0];
                    for i in 0..n {
                        let delta = s * (state.stack[deltas_start + i] as f64 * FX_TO_F64);
                        state.stack[defaults_start + i] = state.stack[defaults_start + i]
                            .saturating_add(banker_round_i64(delta * FX_ONE as f64));
                    }
                } else {
                    for i in 0..n {
                        let mut delta = 0.0;
                        for (r, &s) in scalars.iter().enumerate() {
                            delta += s * (state.stack[deltas_start + i * k + r] as f64 * FX_TO_F64);
                        }
                        state.stack[defaults_start + i] = state.stack[defaults_start + i]
                            .saturating_add(banker_round_i64(delta * FX_ONE as f64));
                    }
                }
                sp = defaults_start + n;
                pos += 1;
            }
            12 => {
                let b1 = *cs.get(pos + 1).ok_or("CFF2 charstring: truncated escape operator")?;
                if !matches!(b1, 34..=37) {
                    return Err(format!("CFF2 charstring: operator 12 {b1} is not in CFF2"));
                }
                flush(out, &state.stack[..sp], &mut sp, &[b0, b1]);
                pos += 2;
            }
            11 | 14 => { state.sp = sp; return Ok(()); }
            4 | 5 | 6 | 7 | 8 | 21 | 22 | 24 | 25 | 26 | 27 | 30 | 31 => {
                emit(out, &state.stack[..sp], &mut sp, &[b0]);
                pos += 1;
            }
            _ => return Err(format!("CFF2 charstring: operator {b0} is not in CFF2")),
        }
    }
    state.sp = sp;
    Ok(())
}

fn pop(stack: &[Fx; MAX_OPERANDS], sp: &mut usize) -> Option<Fx> {
    let v = *stack.get(sp.checked_sub(1)?)?;
    *sp -= 1;
    Some(v)
}

fn fx_to_i32(v: Fx) -> i32 {
    (v / FX_ONE).clamp(i32::MIN as Fx, i32::MAX as Fx) as i32
}

// Type 2 holds 48 operands where CFF2 holds 513, so a longer operator is written as several that draw the
// same. Each split falls on a whole curve, line pair or h/v alternation; a stem list starts again from 0.
const TYPE2_OPERANDS: usize = 48;

fn emit(out: &mut Vec<u8>, operands: &[Fx], sp: &mut usize, op: &[u8]) {
    if operands.len() <= TYPE2_OPERANDS {
        return flush(out, operands, sp, op);
    }
    let mut rest = operands;
    match op {
        // 48 is a whole number of line pairs, curves and alternations.
        [5] | [6] | [7] | [8] => {
            while !rest.is_empty() {
                let n = rest.len().min(TYPE2_OPERANDS);
                write(out, &rest[..n], op);
                rest = &rest[n..];
            }
        }
        // vvcurveto, hhcurveto: an odd first value belongs to the first curve only.
        [26] | [27] => {
            let first = rest.len() % 4 + 44;
            write(out, &rest[..first], op);
            emit(out, &rest[first..], &mut 0, op);
        }
        // vhcurveto, hvcurveto: eight curves keep the alternation, and the last may carry one more value.
        [30] | [31] => {
            while rest.len() > 33 {
                write(out, &rest[..32], op);
                rest = &rest[32..];
            }
            write(out, rest, op);
        }
        // rcurveline and rlinecurve: the leading curves or lines go out as rrcurveto or rlineto.
        [24] => {
            while rest.len() > 44 {
                write(out, &rest[..42], &[8]);
                rest = &rest[42..];
            }
            write(out, rest, op);
        }
        [25] => {
            while rest.len() > 46 {
                write(out, &rest[..40], &[5]);
                rest = &rest[40..];
            }
            write(out, rest, op);
        }
        [1] | [3] | [18] | [23] => {
            let mut edge: Fx = 0;
            let mut chunk: Vec<Fx> = Vec::with_capacity(TYPE2_OPERANDS);
            for (i, &v) in operands.iter().enumerate() {
                edge = edge.saturating_add(v);
                chunk.push(if chunk.is_empty() && i > 0 { edge } else { v });
                if chunk.len() == TYPE2_OPERANDS {
                    write(out, &chunk, op);
                    chunk.clear();
                }
            }
            if !chunk.is_empty() {
                write(out, &chunk, op);
            }
        }
        _ => write(out, operands, op),
    }
    *sp = 0;
}

fn write(out: &mut Vec<u8>, operands: &[Fx], op_bytes: &[u8]) {
    flush(out, operands, &mut 0, op_bytes);
}

fn flush(out: &mut Vec<u8>, operands: &[Fx], sp: &mut usize, op_bytes: &[u8]) {
    out.reserve(operands.len() * 5 + op_bytes.len());
    for &v in operands {
        encode_charstring_number(out, v);
    }
    out.extend_from_slice(op_bytes);
    *sp = 0;
}

fn encode_charstring_number(out: &mut Vec<u8>, v: Fx) {
    if v & (FX_ONE - 1) == 0 {
        let iv = v >> FX_SHIFT;
        if (-32768..=32767).contains(&iv) {
            let iv = iv as i32;
            if (-107..=107).contains(&iv) {
                out.push((iv + 139) as u8);
            } else if (108..=1131).contains(&iv) {
                out.push((((iv - 108) / 256) + 247) as u8);
                out.push(((iv - 108) % 256) as u8);
            } else if (-1131..=-108).contains(&iv) {
                out.push((((-iv - 108) / 256) + 251) as u8);
                out.push(((-iv - 108) % 256) as u8);
            } else {
                out.push(28);
                out.extend_from_slice(&(iv as i16).to_be_bytes());
            }
            return;
        }
    }
    out.push(255);
    out.extend_from_slice(&(v.clamp(i32::MIN as Fx, i32::MAX as Fx) as i32).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each operator as its operands and its bytes, read back from a charstring.
    fn ops(cs: &[u8]) -> Vec<(Vec<f64>, Vec<u8>)> {
        let (mut out, mut stack, mut pos) = (Vec::new(), Vec::new(), 0);
        while pos < cs.len() {
            let b0 = cs[pos];
            if b0 >= 32 || b0 == 28 || b0 == 255 {
                let (v, n) = decode_charstring_number_fx(cs, pos).expect("a number");
                stack.push(v as f64 * FX_TO_F64);
                pos += n;
            } else if b0 == 19 || b0 == 20 {
                out.push((core::mem::take(&mut stack), alloc::vec![b0]));
                pos = cs.len();
            } else {
                let n = if b0 == 12 { 2 } else { 1 };
                out.push((core::mem::take(&mut stack), cs[pos..pos + n].to_vec()));
                pos += n;
            }
        }
        out
    }

    // The points a Type 2 path draws and the stem edges it declares, each stem list from 0, so a split
    // that changes either shows.
    fn geometry(ops: &[(Vec<f64>, Vec<u8>)]) -> (Vec<(f64, f64)>, Vec<f64>) {
        let (mut x, mut y, mut pts, mut edges) = (0.0, 0.0, Vec::new(), Vec::new());
        for (a, op) in ops {
            let mut at = |dx: f64, dy: f64, pts: &mut Vec<(f64, f64)>| {
                x += dx;
                y += dy;
                pts.push((x, y));
            };
            match op.as_slice() {
                [21] => at(a[0], a[1], &mut pts),
                [5] => a.chunks(2).for_each(|p| at(p[0], p[1], &mut pts)),
                [6] | [7] => {
                    for (i, &d) in a.iter().enumerate() {
                        if (i % 2 == 0) == (op[0] == 6) { at(d, 0.0, &mut pts) } else { at(0.0, d, &mut pts) }
                    }
                }
                [8] => a.chunks(6).for_each(|c| (0..3).for_each(|k| at(c[2 * k], c[2 * k + 1], &mut pts))),
                [24] => {
                    let (curves, line) = a.split_at(a.len() - 2);
                    curves.chunks(6).for_each(|c| (0..3).for_each(|k| at(c[2 * k], c[2 * k + 1], &mut pts)));
                    at(line[0], line[1], &mut pts);
                }
                [25] => {
                    let (lines, c) = a.split_at(a.len() - 6);
                    lines.chunks(2).for_each(|p| at(p[0], p[1], &mut pts));
                    (0..3).for_each(|k| at(c[2 * k], c[2 * k + 1], &mut pts));
                }
                [26] | [27] => {
                    let lead = a.len() % 4;
                    for (i, c) in a[lead..].chunks(4).enumerate() {
                        let first = if i == 0 && lead == 1 { a[0] } else { 0.0 };
                        if op[0] == 26 {
                            at(first, c[0], &mut pts);
                            at(c[1], c[2], &mut pts);
                            at(0.0, c[3], &mut pts);
                        } else {
                            at(c[0], first, &mut pts);
                            at(c[1], c[2], &mut pts);
                            at(c[3], 0.0, &mut pts);
                        }
                    }
                }
                [30] | [31] => {
                    let curves = a.len() / 4;
                    for i in 0..curves {
                        let c = &a[4 * i..4 * i + 4];
                        let last = if i + 1 == curves && a.len() % 4 == 1 { a[a.len() - 1] } else { 0.0 };
                        if (i % 2 == 0) == (op[0] == 31) {
                            at(c[0], 0.0, &mut pts);
                            at(c[1], c[2], &mut pts);
                            at(last, c[3], &mut pts);
                        } else {
                            at(0.0, c[0], &mut pts);
                            at(c[1], c[2], &mut pts);
                            at(c[3], last, &mut pts);
                        }
                    }
                }
                [1] | [3] | [18] | [23] | [19] => {
                    let mut edge = 0.0;
                    for &v in a {
                        edge += v;
                        edges.push(edge);
                    }
                }
                _ => {}
            }
        }
        (pts, edges)
    }

    // One operator of each kind past Type 2's 48 operands, odd forms included, and a mask's vstems.
    #[test]
    fn a_long_operator_is_written_as_several_that_draw_the_same() {
        let mut cs = Vec::new();
        let op = |n: usize, bytes: &[u8], cs: &mut Vec<u8>| {
            cs.extend((0..n).map(|i| 139 + (i % 7) as u8 + 1));
            cs.extend_from_slice(bytes);
        };
        op(60, &[18], &mut cs);
        op(2, &[21], &mut cs);
        for (n, code) in [(100, 5), (51, 6), (50, 7), (96, 8), (50, 24), (50, 25), (53, 26), (52, 26), (53, 27),
                          (53, 30), (52, 30), (53, 31), (56, 31)] {
            op(n, &[code], &mut cs);
        }
        op(60, &[19], &mut cs);
        cs.extend([0u8; 8]);
        let original = ops(&cs);

        let (mut out, mut budget, mut scratch) = (Vec::new(), u32::MAX, Scratch::default());
        evaluate_charstring_into(&cs, &[], &[], 0, None, &[], &mut budget, &mut scratch, &mut out).expect("converts");
        let written = ops(&out);
        assert!(written.iter().all(|(a, _)| a.len() <= TYPE2_OPERANDS), "an operator kept more than 48 operands");
        assert!(written.len() > original.len(), "nothing was split");
        assert_eq!(geometry(&written), geometry(&original), "the split drew or hinted differently");
    }

    // A region whose peak is 0 on every axis applies in full everywhere, so each blend adds its
    // delta: 65,538 of them carry a subroutine index past i32, which the bias would then overflow.
    #[test]
    fn a_subroutine_index_grown_past_i32_is_refused() {
        let store = alloc::vec![0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0];
        let store = crate::daecore::daetype::format::ivs::parse_item_variation_store(&store, 0).expect("parses");
        let mut cs = alloc::vec![28, 0x7F, 0xFF];
        for _ in 0..65_538 {
            cs.extend([28, 0x7F, 0xFF, 140, 16]);
        }
        cs.push(10);
        let (mut budget, mut scratch, mut out) = (u32::MAX, Scratch::default(), Vec::new());
        let done = evaluate_charstring_into(&cs, &[], &[], 0, Some(&store), &[0.0], &mut budget, &mut scratch, &mut out);
        assert!(done.is_err());
    }
}
