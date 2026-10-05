#[allow(dead_code, reason = "each method is unused whenever std provides the inherent one")]
pub trait FloatExt {
    fn round(self) -> Self;
    fn floor(self) -> Self;
    fn ceil(self) -> Self;
    fn trunc(self) -> Self;
    fn abs(self) -> Self;
    fn round_ties_even(self) -> Self;
    fn sqrt(self) -> Self;
}

// `$wide` picks each rounding's faster form, as measured: clearing fraction bits wins on f64, where a
// round trip through an integer costs more, and loses on f32.
macro_rules! impl_float_ext {
    ($t:ty, $bits:ty, $sign_mask:expr, $int:ty, $precision_limit:expr, $wide:expr,
     $sqrt_bias:expr, $sqrt_iters:expr, $sqrt_scale:expr, $sqrt_unscale:expr) => {
        macro_rules! is_already_integral {
            ($x:expr) => { !(FloatExt::abs($x) < $precision_limit) || $x == 0.0 };
        }

        macro_rules! with_sign_of {
            ($mag:expr, $sgn:expr) => {{
                let mag: $t = $mag;
                let sgn: $t = $sgn;
                <$t>::from_bits((mag.to_bits() & $sign_mask) | (sgn.to_bits() & !$sign_mask))
            }};
        }

        macro_rules! nearest_even {
            ($x:expr) => {{
                let x = $x;
                // The constant carries the argument's sign, which is not cosmetic: with a positive
                // one a negative argument lands in the binade *below* it, where an ulp is a half,
                // and comes back a half-integer – -268.622 returned -268.5.
                let magic = with_sign_of!($precision_limit, x);
                (x + magic) - magic
            }};
        }

        // The bits below the binary point: None when there are none, all of them when |x| < 1.
        macro_rules! frac_mask {
            ($x:expr) => {{
                const FRAC: u32 = <$t>::MANTISSA_DIGITS - 1;
                const BIAS: i32 = <$t>::MAX_EXP - 1;
                let e = (($x.to_bits() >> FRAC) as i32 & (2 * BIAS + 1)) - BIAS;
                if e >= FRAC as i32 {
                    None
                } else if e < 0 {
                    Some(<$bits>::MAX)
                } else {
                    Some((((1 as $bits) << FRAC) - 1) >> e)
                }
            }};
        }

        #[allow(clippy::neg_cmp_op_on_partial_ord, reason = "the negated form is what passes NaN through")]
        impl FloatExt for $t {
            #[inline]
            fn abs(self) -> Self {
                <$t>::from_bits(self.to_bits() & $sign_mask)
            }

            #[inline]
            fn trunc(self) -> Self {
                if $wide {
                    let b = self.to_bits();
                    return match frac_mask!(self) {
                        None => self,
                        Some(<$bits>::MAX) => <$t>::from_bits(b & !$sign_mask),
                        Some(m) => <$t>::from_bits(b & !m),
                    };
                }
                if is_already_integral!(self) { return self; }
                let t = (self as $int) as $t;
                if t == 0.0 && self < 0.0 { -0.0 } else { t }
            }

            #[inline]
            fn floor(self) -> Self {
                if $wide {
                    let b = self.to_bits();
                    let neg = (b >> (<$bits>::BITS - 1)) != 0;
                    return match frac_mask!(self) {
                        None => self,
                        Some(<$bits>::MAX) => if b << 1 == 0 { self } else if neg { -1.0 } else { 0.0 },
                        Some(m) => if b & m == 0 { self } else { <$t>::from_bits((if neg { b + m } else { b }) & !m) },
                    };
                }
                if is_already_integral!(self) { return self; }
                let t = (self as $int) as $t;
                if t > self { t - 1.0 } else { t }
            }

            #[inline]
            fn ceil(self) -> Self {
                if is_already_integral!(self) { return self; }
                let r = if $wide { (self as $int) as $t } else { nearest_even!(self) };
                let r = if r < self { r + 1.0 } else { r };
                with_sign_of!(r, self)
            }

            #[inline]
            fn round(self) -> Self {
                if $wide {
                    const FRAC: u32 = <$t>::MANTISSA_DIGITS - 1;
                    const BIAS: i32 = <$t>::MAX_EXP - 1;
                    let b = self.to_bits();
                    let e = ((b >> FRAC) as i32 & (2 * BIAS + 1)) - BIAS;
                    if e >= FRAC as i32 { return self; }
                    let sign = b & !$sign_mask;
                    if e < -1 { return <$t>::from_bits(sign); }
                    if e == -1 { return <$t>::from_bits(sign | (1.0 as $t).to_bits()); }
                    let m = (((1 as $bits) << FRAC) - 1) >> e;
                    let half = ((1 as $bits) << (FRAC - 1)) >> e;
                    return <$t>::from_bits((b + half) & !m);
                }
                if is_already_integral!(self) { return self; }
                let ti = self as $int;
                let t = ti as $t;
                let step = if self < 0.0 { -1.0 } else { 1.0 };
                let r = if FloatExt::abs(self - t) >= 0.5 { t + step } else { t };
                if r == 0.0 && self < 0.0 { -0.0 } else { r }
            }

            #[inline]
            fn round_ties_even(self) -> Self {
                if is_already_integral!(self) { return self; }
                // Load-bearing: without carrying the sign, every negative value rounding to zero
                // comes back `+0.0` – 748,634 mismatches against std in a six-million-value sweep.
                with_sign_of!(nearest_even!(self), self)
            }

            #[inline]
            fn sqrt(self) -> Self {
                if self.is_nan() || self < 0.0 { return <$t>::NAN; }
                if self == 0.0 || self == <$t>::INFINITY { return self; }

                let (x, undo) = if self < <$t>::MIN_POSITIVE {
                    (self * $sqrt_scale, $sqrt_unscale)
                } else {
                    (self, 1.0)
                };

                let mut y = <$t>::from_bits((x.to_bits() >> 1) + $sqrt_bias);
                let mut i = 0;
                // The pass counts are the minimum for the ulp bound, not a margin: one fewer drifts
                // 7,183 ulps on f64 and 19 on f32, and one more is a fixed point of the iteration.
                // `tests/machine/float_accuracy.rs` fails if they are shaved.
                while i < $sqrt_iters {
                    y = 0.5 * (y + x / y);
                    i += 1;
                }
                y * undo
            }
        }
    };
}

impl_float_ext!(f32, u32, 0x7fff_ffff, i32, 8_388_608.0, false,
                0x1fc0_0000, 3, 268_435_456.0, 6.103_515_6e-5);
impl_float_ext!(f64, u64, 0x7fff_ffff_ffff_ffff, i64, 4_503_599_627_370_496.0, true,
                0x1ff8_0000_0000_0000, 4, 1.844_674_407_370_955_2e19, 2.328_306_436_538_696_3e-10);
