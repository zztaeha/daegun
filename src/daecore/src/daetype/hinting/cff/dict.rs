use alloc::vec::Vec;

use crate::daecore::daetype::format::cff::decode_cff_number;

// Private DICT values are kept in 32.32: every DICT integer exactly, a real to the nearest 2^-32.
const WHOLE: i64 = 1 << 32;
// BlueScale's default, 0.039625 (TN 5176, table 23), in 32.32.
const BLUE_SCALE: i64 = 170_188_079;
// TN 5176 4 allows 48 operands before an operator, the CFF2 chapter 513.
const CFF_OPERANDS: usize = 48;
const CFF2_OPERANDS: usize = 513;

// A DICT number as written, digits × 10^power, its sign apart: exact for every integer and for a real's first 19
// significant digits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Decimal {
    negative: bool,
    digits: u64,
    power: i64,
}

impl Decimal {
    pub(super) const ONE: Decimal = Decimal { negative: false, digits: 1, power: 0 };

    pub(super) fn is_zero(self) -> bool {
        self.digits == 0
    }

    pub(super) fn abs(self) -> Decimal {
        Decimal { negative: false, ..self }
    }

    // This number over `by`, which isn't 0, in 32.32: exactly rounded, a half away from zero, and past i64's range
    // its largest value of that sign.
    pub(super) fn over(self, by: Decimal) -> i64 {
        let (n, d, e) = (u128::from(self.digits) << 32, u128::from(by.digits), self.power.saturating_sub(by.power));
        let ten = u32::try_from(e.unsigned_abs()).ok().and_then(|e| 10u128.checked_pow(e));
        let magnitude = match ten {
            _ if self.digits == 0 => 0,
            Some(p) if e >= 0 => n.checked_mul(p).and_then(|n| n.checked_add(d / 2)).map_or(u128::MAX, |n| n / d),
            Some(p) => d.checked_mul(p).map_or(0, |d| (n + d / 2) / d),
            None if e >= 0 => u128::MAX,
            None => 0,
        };
        let v = i64::try_from(magnitude).unwrap_or(i64::MAX);
        if self.negative != by.negative { -v } else { v }
    }

    pub(super) fn fixed(self) -> i64 {
        self.over(Decimal::ONE)
    }
}

// The operand at `at` and the bytes it takes; None for a reserved byte or one cut short.
pub(super) fn operand(dict: &[u8], at: usize) -> Option<(Decimal, usize)> {
    match *dict.get(at)? {
        30 => real(dict, at),
        _ => decode_cff_number(dict, at).ok().map(|(v, len)| {
            (Decimal { negative: v < 0, digits: u64::from(v.unsigned_abs()), power: 0 }, len)
        }),
    }
}

#[derive(PartialEq)]
enum Part {
    Whole,
    Fraction,
    Exponent,
}

// A real (TN 5176, table 5) from the nibbles after its 30 byte, and the bytes it takes; None where a nibble is
// reserved or out of place, or the number runs past the DICT.
fn real(dict: &[u8], at: usize) -> Option<(Decimal, usize)> {
    let (mut part, mut negative, mut first) = (Part::Whole, false, true);
    let (mut digits, mut power, mut exponent, mut exponent_negative) = (0u64, 0i64, 0i64, false);
    for (i, &byte) in dict.get(at + 1..)?.iter().enumerate() {
        for nibble in [byte >> 4, byte & 15] {
            match (nibble, &part) {
                // Saturated, as is a power past i64: 10 to either is past any 32.32 value whatever the digits.
                (0..=9, Part::Exponent) => exponent = exponent.saturating_mul(10).saturating_add(i64::from(nibble)),
                (0..=9, _) if digits < 1_000_000_000_000_000_000 => {
                    digits = digits * 10 + u64::from(nibble);
                    power -= i64::from(part == Part::Fraction);
                }
                // Past 19 digits the rest are dropped, each one before the point standing for a power of ten.
                (0..=9, _) => power += i64::from(part == Part::Whole),
                (0xA, Part::Whole) => part = Part::Fraction,
                (0xB | 0xC, Part::Whole | Part::Fraction) => {
                    (part, exponent_negative) = (Part::Exponent, nibble == 0xC);
                }
                (0xE, _) if first => negative = true,
                (0xF, _) => {
                    power = power.saturating_add(if exponent_negative { -exponent } else { exponent });
                    return Some((Decimal { negative, digits, power }, i + 2));
                }
                _ => return None,
            }
            first = false;
        }
    }
    None
}

// An operand as a field keeps it: its value, and for a CFF2 blend where its deltas start in `PrivateDict::deltas`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Term {
    value: i64,
    deltas: Option<u32>,
}

// A whole number a plain operand spells.
fn whole(term: Term) -> Option<i64> {
    (term.deltas.is_none() && term.value % WHOLE == 0).then_some(term.value >> 32)
}

// A delta array's first N operands (TN 5176, table 6), the most the Type 1 spec allows the field.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Terms<const N: usize> {
    terms: [Term; N],
    len: usize,
}

impl<const N: usize> Default for Terms<N> {
    fn default() -> Self {
        Terms { terms: [Term::default(); N], len: 0 }
    }
}

impl<const N: usize> Terms<N> {
    fn of(stack: &[Term]) -> Terms<N> {
        let mut t = Terms { len: stack.len().min(N), ..Terms::default() };
        t.terms[..t.len].copy_from_slice(&stack[..t.len]);
        t
    }
}

// A Private DICT's hint fields, read once; a CFF2 one's blends are weighed for each location by `at`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PrivateDict {
    blue_values: Terms<14>,
    other_blues: Terms<10>,
    family_blues: Terms<14>,
    family_other_blues: Terms<10>,
    blue_scale: Option<Term>,
    blue_shift: Option<Term>,
    blue_fuzz: Option<Term>,
    language_group: Option<Term>,
    random_seed: Option<Term>,
    // Every blend's deltas, `regions` to a blended operand; `regions` is known from the first blend.
    deltas: Vec<i64>,
    regions: Option<usize>,
    pub(crate) vsindex: u32,
}

impl PrivateDict {
    pub(crate) fn blends(&self) -> bool {
        self.regions.is_some()
    }

    // None where the dictionary is malformed, which leaves its FD's glyphs unhinted; `weights` counts the weights a
    // vsindex blends by, the default master's among them.
    pub(crate) fn read(dict: &[u8], cff2: bool, weights: &mut dyn FnMut(u32) -> Option<usize>) -> Option<PrivateDict> {
        let limit = if cff2 { CFF2_OPERANDS } else { CFF_OPERANDS };
        let (mut read, mut stack, mut at) = (PrivateDict::default(), Vec::new(), 0);
        while let Some(&b0) = dict.get(at) {
            let operator = match b0 {
                12 => 1200 + u16::from(*dict.get(at + 1)?),
                0..=21 => u16::from(b0),
                22..=24 if cff2 => u16::from(b0),
                _ => {
                    let (value, len) = operand(dict, at)?;
                    if stack.len() == limit {
                        return None;
                    }
                    stack.push(Term { value: value.fixed(), deltas: None });
                    at += len;
                    continue;
                }
            };
            at += if b0 == 12 { 2 } else { 1 };
            let first = stack.first().copied();
            match operator {
                6 => read.blue_values = Terms::of(&stack),
                7 => read.other_blues = Terms::of(&stack),
                8 => read.family_blues = Terms::of(&stack),
                9 => read.family_other_blues = Terms::of(&stack),
                1209 => read.blue_scale = first.or(read.blue_scale),
                1210 => read.blue_shift = first.or(read.blue_shift),
                1211 => read.blue_fuzz = first.or(read.blue_fuzz),
                1217 => read.language_group = first.or(read.language_group),
                1219 => read.random_seed = first.or(read.random_seed),
                // vsindex names the regions every blend after it weighs by, so it can't follow one.
                22 if read.blends() => return None,
                22 => {
                    if let Some(v) = first {
                        read.vsindex = u32::try_from(whole(v)?).ok()?;
                    }
                }
                23 => {
                    read.blend(&mut stack, weights)?;
                    continue;
                }
                _ => {}
            }
            stack.clear();
        }
        Some(read)
    }

    // A blend (CFF2, "blend"): its count n on top, below it n defaults, then each default's deltas, one a region. The
    // n results stay on the stack; one fed by another blend's result is outside what the chapter defines.
    fn blend(&mut self, stack: &mut Vec<Term>, weights: &mut dyn FnMut(u32) -> Option<usize>) -> Option<()> {
        let n = usize::try_from(whole(stack.pop()?)?).ok()?;
        let regions = match self.regions {
            Some(k) => k,
            None => *self.regions.insert(weights(self.vsindex)?.checked_sub(1)?),
        };
        let base = stack.len().checked_sub(n.checked_mul(regions + 1)?)?;
        if stack[base..].iter().any(|t| t.deltas.is_some()) {
            return None;
        }
        for i in 0..n {
            let deltas = base + n + i * regions;
            let at = u32::try_from(self.deltas.len()).ok()?;
            self.deltas.extend(stack[deltas..deltas + regions].iter().map(|t| t.value));
            stack[base + i].deltas = Some(at);
        }
        stack.truncate(base + n);
        Some(())
    }

    // The values at a location: a blended operand its default plus each delta times its region's 16.16 weight,
    // `weights` listing the default master's first. None where the dictionary blends and has no weights.
    pub(crate) fn at(&self, weights: Option<&[i32]>) -> Option<Private> {
        let (weights, regions) = match self.regions {
            Some(k) => (weights?.get(1..)?, k),
            None => (&[][..], 0),
        };
        let value = |t: Term| match t.deltas {
            None => t.value,
            Some(at) => {
                let deltas = &self.deltas[at as usize..at as usize + regions];
                let sum = deltas.iter().zip(weights).fold(i128::from(t.value), |sum, (&d, &w)| {
                    sum + ((i128::from(d) * i128::from(w) + (1 << 15)) >> 16)
                });
                sum.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
            }
        };
        let mut p = Private {
            blue_values: pairs(&self.blue_values, value),
            other_blues: pairs(&self.other_blues, value),
            family_blues: pairs(&self.family_blues, value),
            family_other_blues: pairs(&self.family_other_blues, value),
            ..Private::default()
        };
        if let Some(t) = self.blue_scale {
            p.blue_scale = value(t);
        }
        // A negative distance past a zone or around one means none.
        if let Some(t) = self.blue_shift {
            p.blue_shift = units(value(t)).max(0);
        }
        if let Some(t) = self.blue_fuzz {
            p.blue_fuzz = units(value(t)).max(0);
        }
        if let Some(t) = self.language_group {
            p.language_group = value(t) >> 32;
        }
        if let Some(t) = self.random_seed {
            p.random_seed = (value(t) >> 32) as u32;
        }
        Some(p)
    }
}

// A 32.32 value in 16.16, to the nearest.
fn units(v: i64) -> i64 {
    v.saturating_add(1 << 15) >> 16
}

// A delta array summed as it goes, then taken in pairs, a lone last value left out.
fn pairs<const N: usize, const P: usize>(terms: &Terms<N>, value: impl Fn(Term) -> i64) -> Pairs<P> {
    let (mut out, mut sum) = (Pairs::default(), 0i64);
    let mut edges = terms.terms[..terms.len].iter().map(|&t| {
        sum = sum.saturating_add(value(t));
        units(sum)
    });
    while out.len < P
        && let (Some(low), Some(high)) = (edges.next(), edges.next())
    {
        out.pairs[out.len] = (low, high);
        out.len += 1;
    }
    out
}

// Zones as a Private DICT lists them, (low, high) in 16.16 font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pairs<const N: usize> {
    pairs: [(i64, i64); N],
    len: usize,
}

impl<const N: usize> Default for Pairs<N> {
    fn default() -> Self {
        Pairs { pairs: [(0, 0); N], len: 0 }
    }
}

impl<const N: usize> Pairs<N> {
    pub(super) fn get(&self) -> &[(i64, i64)] {
        &self.pairs[..self.len]
    }
}

// What hinting reads of an FD's Private DICT at a location: zones in 16.16 font units, BlueScale in 32.32, BlueShift
// and BlueFuzz in 16.16 font units, LanguageGroup, and initialRandomSeed, where `random` starts in a charstring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Private {
    pub(super) blue_values: Pairs<7>,
    pub(super) other_blues: Pairs<5>,
    pub(super) family_blues: Pairs<7>,
    pub(super) family_other_blues: Pairs<5>,
    pub(super) blue_scale: i64,
    pub(super) blue_shift: i64,
    pub(super) blue_fuzz: i64,
    pub(super) language_group: i64,
    pub(crate) random_seed: u32,
}

// TN 5176's defaults (table 23), which an FD without a Private DICT keeps.
impl Default for Private {
    fn default() -> Private {
        Private {
            blue_values: Pairs::default(),
            other_blues: Pairs::default(),
            family_blues: Pairs::default(),
            family_other_blues: Pairs::default(),
            blue_scale: BLUE_SCALE,
            blue_shift: 7 << 16,
            blue_fuzz: 1 << 16,
            language_group: 0,
            random_seed: 0,
        }
    }
}

impl Private {
    // A CFF Private DICT's values; None where it doesn't read.
    pub(crate) fn read(dict: &[u8]) -> Option<Private> {
        PrivateDict::read(dict, false, &mut |_| None)?.at(None)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/dict.rs"]
mod tests;
