use alloc::vec::Vec;

use super::{general_category, is_extended_pictographic, GeneralCategory};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(clippy::upper_case_acronyms, reason = "UAX #14's own names for its classes")]
#[rustfmt::skip]
enum Lb {
    XX = 0, AI, AK, AL, AP, AS, B2, BA, BB, BK, CB, CJ, CL, CM, CP,
    CR, EB, EM, EX, GL, H2, H3, HH, HL, HY, ID, IN, IS, JL, JT, JV,
    LF, NL, NS, NU, OP, PO, PR, QU, RI, SA, SG, SP, SY, VF, VI,
    WJ, ZW, ZWJ,
}

#[rustfmt::skip]
const LB_BY_CODE: [Lb; 49] = [
    Lb::XX, Lb::AI, Lb::AK, Lb::AL, Lb::AP, Lb::AS, Lb::B2, Lb::BA, Lb::BB, Lb::BK, Lb::CB,
    Lb::CJ, Lb::CL, Lb::CM, Lb::CP, Lb::CR, Lb::EB, Lb::EM, Lb::EX, Lb::GL, Lb::H2, Lb::H3,
    Lb::HH, Lb::HL, Lb::HY, Lb::ID, Lb::IN, Lb::IS, Lb::JL, Lb::JT, Lb::JV, Lb::LF, Lb::NL,
    Lb::NS, Lb::NU, Lb::OP, Lb::PO, Lb::PR, Lb::QU, Lb::RI, Lb::SA, Lb::SG, Lb::SP, Lb::SY,
    Lb::VF, Lb::VI, Lb::WJ, Lb::ZW, Lb::ZWJ,
];

// LB28a spells U+25CC out as a literal alongside the AK and AS classes, because a dotted circle
// stands in for a missing consonant when a cluster is shown in isolation.
const DOTTED_CIRCLE: char = '\u{25CC}';

fn east_asian(c: char) -> bool {
    matches!(super::props(c).east_asian_width, 2 | 3 | 5)
}

// LB1: the classes that never reach a later rule. AI, SG and XX behave as AL; CJ as NS; SA as CM
// when the character is a mark and AL otherwise. CB is deliberately left alone – LB20 acts on it.
fn resolved_class(c: char) -> Lb {
    let raw = LB_BY_CODE[super::props(c).line_break as usize];
    match raw {
        Lb::AI | Lb::SG | Lb::XX => Lb::AL,
        Lb::CJ => Lb::NS,
        Lb::SA => match general_category(c) {
            GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark => Lb::CM,
            _ => Lb::AL,
        },
        other => other,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineBreak {
    pub at: usize,
    pub mandatory: bool,
}

pub fn line_break_opportunities(text: &str) -> Vec<LineBreak> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }

    let raw: Vec<Lb> = chars.iter().map(|&c| resolved_class(c)).collect();

    // LB9/LB10: `X (CM | ZWJ)*` behaves as X, so a mark is not a character the later rules can see
    // at all. Folding that into a base index once, rather than re-deriving it inside thirty rules,
    // is the same shape WB4 forced in segment.rs – and it makes LB9's other half fall out: a
    // position whose base is not itself lies inside a sequence, so it can never be a break.
    let mut base: Vec<usize> = Vec::with_capacity(n);
    for i in 0..n {
        let attaches = i > 0
            && matches!(raw[i], Lb::CM | Lb::ZWJ)
            && !matches!(
                raw[base[i - 1]],
                Lb::BK | Lb::CR | Lb::LF | Lb::NL | Lb::SP | Lb::ZW
            );
        base.push(if attaches { base[i - 1] } else { i });
    }

    let mut sig = Vec::with_capacity(n);
    let mut cls = Vec::with_capacity(n);
    for i in 0..n {
        if base[i] != i {
            continue;
        }
        sig.push(i);
        cls.push(match raw[i] {
            Lb::CM | Lb::ZWJ => Lb::AL,
            other => other,
        });
    }

    // LB30a needs the parity of the regional-indicator run before each position. Counting it where
    // the rule is read rescans that run every time, which is quadratic on a text of flag halves; one
    // forward pass carries it instead.
    let mut ri_odd = Vec::with_capacity(cls.len());
    let mut run = 0usize;
    for &c in &cls {
        ri_odd.push(run % 2 == 1);
        run = if c == Lb::RI { run + 1 } else { 0 };
    }

    // LB8, LB14, LB15 and LB25 all read `SP*` backwards from the same position, so one forward pass
    // carries it for all four, as `ri_odd` above does for LB30a, not a scan each per break position.
    const NONE: u32 = u32::MAX;
    let mut prev_non_sp: Vec<u32> = Vec::with_capacity(cls.len());
    let mut last = NONE;
    for &c in &cls {
        prev_non_sp.push(last);
        if c != Lb::SP {
            last = (prev_non_sp.len() - 1) as u32;
        }
    }

    let ctx = Ctx {
        chars: &chars,
        raw: &raw,
        sig: &sig,
        cls: &cls,
        ri_odd: &ri_odd,
        prev_non_sp: &prev_non_sp,
    };
    for (k, &at) in sig.iter().enumerate().skip(1) {
        if let Some(mandatory) = ctx.break_before(k) {
            out.push(LineBreak { at, mandatory });
        }
    }
    // LB3: always break at the end of text.
    out.push(LineBreak { at: n, mandatory: true });
    out
}

struct Ctx<'a> {
    chars: &'a [char],
    raw: &'a [Lb],
    sig: &'a [usize],
    cls: &'a [Lb],
    ri_odd: &'a [bool],
    prev_non_sp: &'a [u32],
}

impl Ctx<'_> {
    fn cls(&self, k: usize) -> Option<Lb> {
        self.cls.get(k).copied()
    }

    fn ch(&self, k: usize) -> char {
        self.chars[self.sig[k]]
    }

    fn gc_is(&self, k: usize, want: GeneralCategory) -> bool {
        general_category(self.ch(k)) == want
    }

    fn skip_spaces(&self, k: usize) -> Option<usize> {
        match self.prev_non_sp.get(k).copied() {
            Some(u32::MAX) | None => None,
            Some(j) => Some(j as usize),
        }
    }

    // LB28a writes `(AK | [◌] | AS)` in three of its four forms.
    fn brahmic(&self, k: usize) -> bool {
        matches!(self.cls[k], Lb::AK | Lb::AS) || self.ch(k) == DOTTED_CIRCLE
    }

    fn number_run_ends(&self, k: usize) -> bool {
        let mut j = k;
        loop {
            match self.cls[j] {
                Lb::NU => return true,
                Lb::SY | Lb::IS if j > 0 => j -= 1,
                _ => return false,
            }
        }
    }

    fn break_before(&self, k: usize) -> Option<bool> {
        let a = self.cls[k - 1];
        let b = self.cls[k];

        // LB4, LB5: the hard breaks. CRLF is one break, not two.
        if a == Lb::BK {
            return Some(true);
        }
        if a == Lb::CR && b == Lb::LF {
            return None;
        }
        if matches!(a, Lb::CR | Lb::LF | Lb::NL) {
            return Some(true);
        }
        // LB6: never break before a hard break.
        if matches!(b, Lb::BK | Lb::CR | Lb::LF | Lb::NL) {
            return None;
        }
        // LB7: never break before a space or ZW.
        if matches!(b, Lb::SP | Lb::ZW) {
            return None;
        }
        // LB8: `ZW SP* ÷`.
        if self.skip_spaces(k).is_some_and(|j| self.cls[j] == Lb::ZW) {
            return Some(false);
        }
        // LB8a: `ZWJ ×`. The ZWJ was absorbed into its base, so this reads the raw text.
        if self.raw[self.sig[k] - 1] == Lb::ZWJ {
            return None;
        }
        // LB9 and LB10 are already folded into `cls` and `sig`.

        // LB11: `× WJ`, `WJ ×`.
        if a == Lb::WJ || b == Lb::WJ {
            return None;
        }
        // LB12: `GL ×`. LB12a: `[^SP BA HY HH] × GL`.
        if a == Lb::GL {
            return None;
        }
        if b == Lb::GL && !matches!(a, Lb::SP | Lb::BA | Lb::HY | Lb::HH) {
            return None;
        }
        // LB13: `× CL`, `× CP`, `× EX`, `× SY`.
        if matches!(b, Lb::CL | Lb::CP | Lb::EX | Lb::SY) {
            return None;
        }
        // LB14: `OP SP* ×`.
        if self.skip_spaces(k).is_some_and(|j| self.cls[j] == Lb::OP) {
            return None;
        }
        // LB15a: `(sot | BK | CR | LF | NL | OP | QU | GL | SP | ZW) [\p{Pi}&QU] SP* ×`.
        if let Some(j) = self.skip_spaces(k) {
            let after_opener = j == 0
                || matches!(
                    self.cls[j - 1],
                    Lb::BK | Lb::CR | Lb::LF | Lb::NL | Lb::OP | Lb::QU | Lb::GL | Lb::SP | Lb::ZW
                );
            if self.cls[j] == Lb::QU
                && after_opener
                && self.gc_is(j, GeneralCategory::InitialPunctuation)
            {
                return None;
            }
        }
        // LB15b: `× [\p{Pf}&QU] (SP|GL|WJ|CL|QU|CP|EX|IS|SY|BK|CR|LF|NL|ZW|eot)`.
        if b == Lb::QU && self.gc_is(k, GeneralCategory::FinalPunctuation) {
            let closes = match self.cls(k + 1) {
                None => true,
                Some(next) => matches!(
                    next,
                    Lb::SP
                        | Lb::GL
                        | Lb::WJ
                        | Lb::CL
                        | Lb::QU
                        | Lb::CP
                        | Lb::EX
                        | Lb::IS
                        | Lb::SY
                        | Lb::BK
                        | Lb::CR
                        | Lb::LF
                        | Lb::NL
                        | Lb::ZW
                ),
            };
            if closes {
                return None;
            }
        }
        // LB15c: `SP ÷ IS NU` – a decimal mark after a space starts a new number.
        if a == Lb::SP && b == Lb::IS && self.cls(k + 1) == Some(Lb::NU) {
            return Some(false);
        }
        // LB15d: `× IS`.
        if b == Lb::IS {
            return None;
        }
        // LB16: `(CL | CP) SP* × NS`.
        if b == Lb::NS
            && self
                .skip_spaces(k)
                .is_some_and(|j| matches!(self.cls[j], Lb::CL | Lb::CP))
        {
            return None;
        }
        // LB17: `B2 SP* × B2`.
        if b == Lb::B2 && self.skip_spaces(k).is_some_and(|j| self.cls[j] == Lb::B2) {
            return None;
        }
        // LB18: `SP ÷`.
        if a == Lb::SP {
            return Some(false);
        }
        // LB19: `× [QU - \p{Pi}]`, `[QU - \p{Pf}] ×`.
        if b == Lb::QU && !self.gc_is(k, GeneralCategory::InitialPunctuation) {
            return None;
        }
        if a == Lb::QU && !self.gc_is(k - 1, GeneralCategory::FinalPunctuation) {
            return None;
        }
        // LB19a: a quotation mark binds unless East Asian characters sit on both sides of it.
        if b == Lb::QU
            && (!east_asian(self.ch(k - 1))
                || k + 1 >= self.cls.len()
                || !east_asian(self.ch(k + 1)))
        {
            return None;
        }
        if a == Lb::QU && (!east_asian(self.ch(k)) || k < 2 || !east_asian(self.ch(k - 2))) {
            return None;
        }
        // LB20: `÷ CB`, `CB ÷`.
        if a == Lb::CB || b == Lb::CB {
            return Some(false);
        }
        // LB20a: `(sot|BK|CR|LF|NL|SP|ZW|CB|GL) (HY | HH) × (AL | HL)`.
        if matches!(b, Lb::AL | Lb::HL)
            && matches!(a, Lb::HY | Lb::HH)
            && (k == 1
                || matches!(
                    self.cls[k - 2],
                    Lb::BK | Lb::CR | Lb::LF | Lb::NL | Lb::SP | Lb::ZW | Lb::CB | Lb::GL
                ))
        {
            return None;
        }
        // LB21: `× BA`, `× HH`, `× HY`, `× NS`, `BB ×`.
        if matches!(b, Lb::BA | Lb::HH | Lb::HY | Lb::NS) || a == Lb::BB {
            return None;
        }
        // LB21a: `HL (HY | HH) × [^HL]`.
        if k >= 2 && self.cls[k - 2] == Lb::HL && b != Lb::HL && matches!(a, Lb::HY | Lb::HH) {
            return None;
        }
        // LB21b: `SY × HL`.
        if a == Lb::SY && b == Lb::HL {
            return None;
        }
        // LB22: `× IN`.
        if b == Lb::IN {
            return None;
        }
        // LB23: `(AL | HL) × NU`, `NU × (AL | HL)`.
        if matches!(a, Lb::AL | Lb::HL) && b == Lb::NU {
            return None;
        }
        if a == Lb::NU && matches!(b, Lb::AL | Lb::HL) {
            return None;
        }
        // LB23a: `PR × (ID | EB | EM)`, `(ID | EB | EM) × PO`.
        if a == Lb::PR && matches!(b, Lb::ID | Lb::EB | Lb::EM) {
            return None;
        }
        if matches!(a, Lb::ID | Lb::EB | Lb::EM) && b == Lb::PO {
            return None;
        }
        // LB24: `(PR | PO) × (AL | HL)`, `(AL | HL) × (PR | PO)`.
        if matches!(a, Lb::PR | Lb::PO) && matches!(b, Lb::AL | Lb::HL) {
            return None;
        }
        if matches!(a, Lb::AL | Lb::HL) && matches!(b, Lb::PR | Lb::PO) {
            return None;
        }
        // LB25.
        if self.number_binds(k, a, b) {
            return None;
        }
        // LB26: `JL × (JL | JV | H2 | H3)`, `(JV | H2) × (JV | JT)`, `(JT | H3) × JT`.
        if a == Lb::JL && matches!(b, Lb::JL | Lb::JV | Lb::H2 | Lb::H3) {
            return None;
        }
        if matches!(a, Lb::JV | Lb::H2) && matches!(b, Lb::JV | Lb::JT) {
            return None;
        }
        if matches!(a, Lb::JT | Lb::H3) && b == Lb::JT {
            return None;
        }
        // LB27: `(JL | JV | JT | H2 | H3) × PO`, `PR × (JL | JV | JT | H2 | H3)`.
        if matches!(a, Lb::JL | Lb::JV | Lb::JT | Lb::H2 | Lb::H3) && b == Lb::PO {
            return None;
        }
        if a == Lb::PR && matches!(b, Lb::JL | Lb::JV | Lb::JT | Lb::H2 | Lb::H3) {
            return None;
        }
        // LB28: `(AL | HL) × (AL | HL)`.
        if matches!(a, Lb::AL | Lb::HL) && matches!(b, Lb::AL | Lb::HL) {
            return None;
        }
        // LB28a: the orthographic syllables of Brahmic scripts.
        if a == Lb::AP && self.brahmic(k) {
            return None;
        }
        if self.brahmic(k - 1) && matches!(b, Lb::VF | Lb::VI) {
            return None;
        }
        if a == Lb::VI
            && k >= 2
            && self.brahmic(k - 2)
            && (b == Lb::AK || self.ch(k) == DOTTED_CIRCLE)
        {
            return None;
        }
        if self.brahmic(k - 1) && self.brahmic(k) && self.cls(k + 1) == Some(Lb::VF) {
            return None;
        }
        // LB29: `IS × (AL | HL)`.
        if a == Lb::IS && matches!(b, Lb::AL | Lb::HL) {
            return None;
        }
        // LB30: `(AL | HL | NU) × [OP - $EastAsian]`, `[CP - $EastAsian] × (AL | HL | NU)`.
        if matches!(a, Lb::AL | Lb::HL | Lb::NU) && b == Lb::OP && !east_asian(self.ch(k)) {
            return None;
        }
        if a == Lb::CP && !east_asian(self.ch(k - 1)) && matches!(b, Lb::AL | Lb::HL | Lb::NU) {
            return None;
        }
        // LB30a: `(RI RI)* RI × RI` – flags pair up, so only an odd run before the break binds.
        if a == Lb::RI && b == Lb::RI && self.ri_odd[k] {
            return None;
        }
        // LB30b: `EB × EM`, `[\p{Extended_Pictographic}&\p{Cn}] × EM`.
        if b == Lb::EM
            && (a == Lb::EB
                || (is_extended_pictographic(self.ch(k - 1))
                    && general_category(self.ch(k - 1)) == GeneralCategory::Unassigned))
        {
            return None;
        }
        // LB31: break everywhere else.
        Some(false)
    }

    // LB25, whose fifteen forms are what a single "do not break numbers" regex used to be. Kept
    // together because they only mean anything as one rule, and split out because inlining them
    // buries the rules on either side.
    fn number_binds(&self, k: usize, a: Lb, b: Lb) -> bool {
        if matches!(b, Lb::PO | Lb::PR) {
            if self.number_run_ends(k - 1) {
                return true;
            }
            if matches!(a, Lb::CL | Lb::CP) && k >= 2 && self.number_run_ends(k - 2) {
                return true;
            }
        }
        if b == Lb::NU && self.number_run_ends(k - 1) {
            return true;
        }
        if matches!(a, Lb::PO | Lb::PR) {
            if b == Lb::NU {
                return true;
            }
            if b == Lb::OP {
                return self.cls(k + 1) == Some(Lb::NU)
                    || (self.cls(k + 1) == Some(Lb::IS) && self.cls(k + 2) == Some(Lb::NU));
            }
        }
        matches!(a, Lb::HY | Lb::IS) && b == Lb::NU
    }
}
