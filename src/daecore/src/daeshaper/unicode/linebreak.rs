use alloc::vec::Vec;

use super::{general_category, is_extended_pictographic, script, GeneralCategory, Script, DOTTED_CIRCLE};
use crate::daecore::daeshaper::generated::unicode_tables::{east_asian_width as eaw, LineBreakClass as Lb};

fn east_asian(c: char) -> bool {
    matches!(super::props(c).east_asian_width, eaw::F | eaw::H | eaw::W)
}

// LB1: the classes that never reach a later rule, with whether the character is SA. AI, SG and XX behave as AL; CJ
// as NS; SA as CM when the character is a mark and AL otherwise. CB is left alone, as LB20 acts on it.
fn resolved_class(c: char) -> (Lb, bool) {
    match super::props(c).line_break {
        Lb::AI | Lb::SG | Lb::XX => (Lb::AL, false),
        Lb::CJ => (Lb::NS, false),
        Lb::SA => match general_category(c) {
            GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark => (Lb::CM, true),
            _ => (Lb::AL, true),
        },
        other => (other, false),
    }
}

// `at` is the byte offset the next line starts at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineBreak {
    pub at: usize,
    pub mandatory: bool,
}

// Finds the word boundaries inside one run of text written without spaces between words, pushing their byte
// offsets from the run's start; one that isn't a char boundary the rules see is ignored.
pub type Segmenter<'a> = &'a (dyn Fn(&str, &mut Vec<usize>) + Sync);

const NONE: usize = usize::MAX;

// A character the later rules see, LB9 and LB10 having folded each combining sequence into its base, with what
// they read of the text before it.
#[derive(Clone, Copy)]
struct Sig {
    at: usize,
    ch: char,
    cls: Lb,
    // LB8a: a ZWJ just before it, folded into the base before or not.
    after_zwj: bool,
    // LB30a: an odd run of regional indicators before it.
    ri_odd: bool,
    // LB8, LB14, LB15 and LB25 read `SP*` back from here: the last one before it that isn't a space, or `NONE`.
    prev_non_sp: usize,
    // LB1: a word of SA text starts here.
    sa_break: bool,
}

pub fn line_break_opportunities(text: &str, segmenter: Option<Segmenter>) -> Vec<LineBreak> {
    if text.is_empty() {
        return Vec::new();
    }
    // LB9/LB10: `X (CM | ZWJ)*` behaves as X. One pass folds each sequence into its base, which also gives LB9's other
    // half: a folded character is no position the rules visit, so it can never be a break.
    let mut sigs: Vec<Sig> = Vec::with_capacity(text.chars().count());
    let mut sa = SaRuns { text, segmenter, run: None, chars: Vec::new(), found: Vec::new() };
    let (mut base, mut before) = (None, Lb::XX);
    let (mut ri_run, mut last_non_sp) = (0usize, NONE);
    for (at, c) in text.char_indices() {
        let (raw, is_sa) = resolved_class(c);
        sa.next(at, c, is_sa, &mut sigs);
        let attaches = matches!(raw, Lb::CM | Lb::ZWJ)
            && base.is_some_and(|b| !matches!(b, Lb::BK | Lb::CR | Lb::LF | Lb::NL | Lb::SP | Lb::ZW));
        if !attaches {
            let cls = if matches!(raw, Lb::CM | Lb::ZWJ) { Lb::AL } else { raw };
            let (ri_odd, prev_non_sp) = (ri_run % 2 == 1, last_non_sp);
            sigs.push(Sig { at, ch: c, cls, after_zwj: before == Lb::ZWJ, ri_odd, prev_non_sp, sa_break: false });
            ri_run = if cls == Lb::RI { ri_run + 1 } else { 0 };
            if cls != Lb::SP {
                last_non_sp = sigs.len() - 1;
            }
            base = Some(raw);
        }
        before = raw;
    }
    sa.end_run(text.len(), &mut sigs);

    let ctx = Ctx { sigs: &sigs };
    let mut out = Vec::with_capacity(sigs.len() + 1);
    for (k, sig) in sigs.iter().enumerate().skip(1) {
        match ctx.break_before(k) {
            Some(mandatory) => out.push(LineBreak { at: sig.at, mandatory }),
            None if sig.sa_break => out.push(LineBreak { at: sig.at, mandatory: false }),
            None => {}
        }
    }
    // LB3: always break at the end of text.
    out.push(LineBreak { at: text.len(), mandatory: true });
    out
}

// LB1: SA text breaks between words, found per run by a caller's segmenter or, with `dictionaries`, its script's word
// list. Each run is divided as it ends, its breaks marked on the characters the rules see, never inside a sequence.
struct SaRuns<'a> {
    text: &'a str,
    segmenter: Option<Segmenter<'a>>,
    // Where the run being gathered starts, and its script.
    run: Option<(usize, Script)>,
    chars: Vec<char>,
    found: Vec<usize>,
}

impl SaRuns<'_> {
    // A run holds SA characters of its first's script, or of any with a segmenter.
    fn next(&mut self, at: usize, c: char, is_sa: bool, sigs: &mut [Sig]) {
        if let Some((_, s)) = self.run {
            if is_sa && (self.segmenter.is_some() || script(c) == s) {
                self.gather(c);
                return;
            }
            self.end_run(at, sigs);
        }
        if is_sa {
            self.run = Some((at, script(c)));
            self.gather(c);
        }
    }

    fn gather(&mut self, c: char) {
        if cfg!(feature = "dictionaries") && self.segmenter.is_none() {
            self.chars.push(c);
        }
    }

    // The run's characters the rules see are the last of `sigs`, all of them there by the time it ends.
    fn end_run(&mut self, end: usize, sigs: &mut [Sig]) {
        let Some((mut start, s)) = self.run.take() else { return };
        let first = sigs.partition_point(|sig| sig.at < start);
        // Marks LB9 folds into the preceding character belong to no word, so where the rules break before the run's
        // first letter its words are found from there. A letter is AL, whose rules read nothing past it.
        let mut skip = 0;
        if let Some(sig) = sigs.get(first)
            && sig.at > start
            && (Ctx { sigs: &*sigs }).break_before(first).is_some()
        {
            skip = self.text[start..sig.at].chars().count();
            start = sig.at;
        }
        let run = &self.text[start..end];
        self.found.clear();
        match self.segmenter {
            Some(segment) => segment(run, &mut self.found),
            None => {
                let chars = self.chars.get(skip..).unwrap_or_default();
                dictionary_breaks(chars, s, &mut self.found);
                to_bytes(chars, &mut self.found);
            }
        }
        self.chars.clear();
        let run = &mut sigs[first..];
        for at in self.found.iter().filter(|&&g| 0 < g && g < end - start).map(|g| start + g) {
            if let Ok(k) = run.binary_search_by_key(&at, |sig| sig.at) {
                run[k].sa_break = true;
            }
        }
    }
}

// The word lists count characters, in order: each boundary becomes its byte offset in the run, summed from the
// characters' own widths.
fn to_bytes(chars: &[char], found: &mut [usize]) {
    let (mut k, mut at) = (0, 0);
    for g in found {
        debug_assert!(*g >= k, "the word lists' boundaries come in order");
        at += chars.get(k..*g).unwrap_or_default().iter().map(|c| c.len_utf8()).sum::<usize>();
        (k, *g) = (*g, at);
    }
}

#[cfg(feature = "dictionaries")]
fn dictionary_breaks(run: &[char], script: Script, out: &mut Vec<usize>) {
    super::words::word_boundaries(run, script, out);
}

#[cfg(not(feature = "dictionaries"))]
fn dictionary_breaks(_: &[char], _: Script, _: &mut Vec<usize>) {}

struct Ctx<'a> {
    sigs: &'a [Sig],
}

impl Ctx<'_> {
    fn cls(&self, k: usize) -> Option<Lb> {
        self.sigs.get(k).map(|s| s.cls)
    }

    fn ch(&self, k: usize) -> char {
        self.sigs[k].ch
    }

    fn gc_is(&self, k: usize, want: GeneralCategory) -> bool {
        general_category(self.ch(k)) == want
    }

    fn skip_spaces(&self, k: usize) -> Option<usize> {
        self.sigs.get(k).map(|s| s.prev_non_sp).filter(|&j| j != NONE)
    }

    // LB28a writes `(AK | [◌] | AS)` in three of its four forms.
    fn brahmic(&self, k: usize) -> bool {
        matches!(self.sigs[k].cls, Lb::AK | Lb::AS) || self.ch(k) == DOTTED_CIRCLE
    }

    fn number_run_ends(&self, k: usize) -> bool {
        let mut j = k;
        loop {
            match self.sigs[j].cls {
                Lb::NU => return true,
                Lb::SY | Lb::IS if j > 0 => j -= 1,
                _ => return false,
            }
        }
    }

    fn break_before(&self, k: usize) -> Option<bool> {
        let a = self.sigs[k - 1].cls;
        let b = self.sigs[k].cls;

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
        if self.skip_spaces(k).is_some_and(|j| self.sigs[j].cls == Lb::ZW) {
            return Some(false);
        }
        // LB8a: `ZWJ ×`. The ZWJ was absorbed into its base, so this reads the raw text.
        if self.sigs[k].after_zwj {
            return None;
        }
        // LB9 and LB10 are already folded into `cls` and `sig`.

        // LB11: `× WJ`, `WJ ×`.
        if a == Lb::WJ || b == Lb::WJ {
            return None;
        }
        // LB12: `GL ×`. LB12a: `[^SP HY HH] × GL`.
        if a == Lb::GL {
            return None;
        }
        if b == Lb::GL && !matches!(a, Lb::SP | Lb::HY | Lb::HH) {
            return None;
        }
        // LB13: `× CL`, `× CP`, `× EX`, `× SY`.
        if matches!(b, Lb::CL | Lb::CP | Lb::EX | Lb::SY) {
            return None;
        }
        // LB14: `OP SP* ×`.
        if self.skip_spaces(k).is_some_and(|j| self.sigs[j].cls == Lb::OP) {
            return None;
        }
        // LB15a: `(sot | BK | CR | LF | NL | OP | QU | GL | SP | ZW) [\p{Pi}&QU] SP* ×`.
        if let Some(j) = self.skip_spaces(k) {
            let after_opener = j == 0
                || matches!(
                    self.sigs[j - 1].cls,
                    Lb::BK | Lb::CR | Lb::LF | Lb::NL | Lb::OP | Lb::QU | Lb::GL | Lb::SP | Lb::ZW
                );
            if self.sigs[j].cls == Lb::QU
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
                .is_some_and(|j| matches!(self.sigs[j].cls, Lb::CL | Lb::CP))
        {
            return None;
        }
        // LB17: `B2 SP* × B2`.
        if b == Lb::B2 && self.skip_spaces(k).is_some_and(|j| self.sigs[j].cls == Lb::B2) {
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
                || k + 1 >= self.sigs.len()
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
                    self.sigs[k - 2].cls,
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
        if k >= 2 && self.sigs[k - 2].cls == Lb::HL && b != Lb::HL && matches!(a, Lb::HY | Lb::HH) {
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
        if a == Lb::RI && b == Lb::RI && self.sigs[k].ri_odd {
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

    // LB25's fifteen forms, kept together as they only mean anything as one rule, and out of line as inlining them
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
