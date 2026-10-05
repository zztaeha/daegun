use super::Blend;
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;

pub type Rgb = [f32; 3];

// Per pixel, like `composite` and the helpers both call: `#[inline]` lets a caller built without LTO
// inline them, which more than halves a composite's time.
#[inline]
pub fn blend(mode: Blend, cb: Rgb, cs: Rgb) -> Rgb {
    use Blend::*;
    match mode {
        Multiply => channels(cb, cs, |b, s| b * s),
        Screen => channels(cb, cs, screen),
        Overlay => channels(cb, cs, |b, s| hard_light(s, b)),
        Darken => channels(cb, cs, f32::min),
        Lighten => channels(cb, cs, f32::max),
        ColorDodge => channels(cb, cs, color_dodge),
        ColorBurn => channels(cb, cs, color_burn),
        HardLight => channels(cb, cs, hard_light),
        SoftLight => channels(cb, cs, soft_light),
        Difference => channels(cb, cs, |b, s| (b - s).abs()),
        Exclusion => channels(cb, cs, |b, s| b + s - 2.0 * b * s),

        HslHue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        HslSaturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        HslColor => set_lum(cs, lum(cb)),
        HslLuminosity => set_lum(cb, lum(cs)),

        _ => cs,
    }
}

#[inline]
fn coefficients(mode: Blend, a_s: f32, a_b: f32) -> (f32, f32) {
    use Blend::*;
    match mode {
        Clear => (0.0, 0.0),
        Src => (1.0, 0.0),
        Dest => (0.0, 1.0),
        DestOver => (1.0 - a_b, 1.0),
        SrcIn => (a_b, 0.0),
        DestIn => (0.0, a_s),
        SrcOut => (1.0 - a_b, 0.0),
        DestOut => (0.0, 1.0 - a_s),
        SrcAtop => (a_b, 1.0 - a_s),
        DestAtop => (1.0 - a_b, a_s),
        Xor => (1.0 - a_b, 1.0 - a_s),
        Plus => (1.0, 1.0),
        _ => (1.0, 1.0 - a_s),
    }
}

// The specification's two stages: the source blends with the backdrop only where they overlap,
// `Cs' = (1 - ab)*Cs + ab*B(Cb, Cs)`. Colors go in straight and in 0..1 (past it the HSL modes can
// answer NaN) and come back premultiplied by the alpha returned with them.
#[inline]
pub fn composite(mode: Blend, cs: Rgb, a_s: f32, cb: Rgb, a_b: f32) -> (Rgb, f32) {
    let (a_s, a_b) = (a_s.clamp(0.0, 1.0), a_b.clamp(0.0, 1.0));
    // SrcOver, COLR's default and nearly every layer: B(Cb, Cs) = Cs, so the mix is Cs itself.
    if mode == Blend::SrcOver {
        let k = a_b * (1.0 - a_s);
        return ([0, 1, 2].map(|i| (a_s * cs[i] + k * cb[i]).clamp(0.0, 1.0)), (a_s + k).clamp(0.0, 1.0));
    }
    let b = blend(mode, cb, cs);
    let mixed = [
        (1.0 - a_b) * cs[0] + a_b * b[0],
        (1.0 - a_b) * cs[1] + a_b * b[1],
        (1.0 - a_b) * cs[2] + a_b * b[2],
    ];

    let (fa, fb) = coefficients(mode, a_s, a_b);
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        out[i] = (a_s * fa * mixed[i] + a_b * fb * cb[i]).clamp(0.0, 1.0);
    }
    (out, (a_s * fa + a_b * fb).clamp(0.0, 1.0))
}

#[inline]
fn channels(cb: Rgb, cs: Rgb, f: impl Fn(f32, f32) -> f32) -> Rgb {
    [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])]
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { b * (2.0 * s) } else { screen(b, 2.0 * s - 1.0) }
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b == 0.0 {
        0.0
    } else if s == 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b == 1.0 {
        1.0
    } else if s == 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

fn lum(c: Rgb) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: Rgb) -> Rgb {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 && l - n != 0.0 {
        for v in &mut out {
            *v = l + ((*v - l) * l) / (l - n);
        }
    }
    if x > 1.0 && x - l != 0.0 {
        for v in &mut out {
            *v = l + ((*v - l) * (1.0 - l)) / (x - l);
        }
    }
    out
}

fn set_lum(c: Rgb, l: f32) -> Rgb {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: Rgb, s: f32) -> Rgb {
    let (mut lo, mut mid, mut hi) = (0usize, 1usize, 2usize);
    if c[lo] > c[mid] {
        core::mem::swap(&mut lo, &mut mid);
    }
    if c[mid] > c[hi] {
        core::mem::swap(&mut mid, &mut hi);
    }
    if c[lo] > c[mid] {
        core::mem::swap(&mut lo, &mut mid);
    }

    let mut out = [0.0f32; 3];
    if c[hi] > c[lo] {
        out[mid] = ((c[mid] - c[lo]) * s) / (c[hi] - c[lo]);
        out[hi] = s;
    }
    out
}
