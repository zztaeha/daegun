use alloc::collections::VecDeque;
use alloc::vec::Vec;

use super::{bidi, general_category, script, script_extensions, Script};

/// A run of one script.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScriptRun {
    /// The bytes of the text it covers.
    pub bytes: core::range::Range<usize>,
    /// Its script.
    pub script: Script,
}

// Open brackets kept for their closers: past this many the oldest is dropped, as a ring overwrites it.
const BRACKET_DEPTH: usize = 32;

// UAX #24's runs: brackets pair up and close in their opener's script, marks stay with their base, and Common or
// Inherited characters continue any run their Script_Extensions allow. Text with no script is one Common run.
pub fn script_runs(text: &str) -> Vec<ScriptRun> {
    let mut out = Vec::new();
    let mut brackets = Brackets::default();
    let mut run = Run::default();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let pair = bidi::bracket(c);
        let opener = match pair {
            Some((opener, false)) => brackets.find(opener),
            _ => None,
        };
        let claim = Claim::of(c, opener.filter(|s| !s.is_context_dependent()));
        if !run.take(claim, &mut brackets) {
            out.push(ScriptRun { bytes: core::range::Range { start, end: i }, script: run.finish(&mut brackets) });
            start = i;
            run = Run::default();
            run.take(claim, &mut brackets);
        }
        match pair {
            Some((_, true)) => brackets.push(c, run.resolved.unwrap_or(Script::COMMON)),
            Some((_, false)) if opener.is_some() => brackets.pop(),
            _ => {}
        }
    }
    if start < text.len() {
        out.push(ScriptRun { bytes: core::range::Range { start, end: text.len() }, script: run.finish(&mut brackets) });
    }
    out
}

// What a character asks of the run it joins. A mark stays with its base whatever its script, as an Inherited
// character does with what it follows.
#[derive(Clone, Copy)]
struct Claim {
    own: Option<Script>,
    extensions: Option<&'static [Script]>,
    stays: bool,
}

impl Claim {
    fn of(c: char, opener: Option<Script>) -> Claim {
        if let Some(s) = opener {
            return Claim { own: Some(s), extensions: None, stays: false };
        }
        let s = script(c);
        Claim {
            own: (!s.is_context_dependent()).then_some(s),
            extensions: script_extensions(c),
            stays: s == Script::INHERITED || general_category(c).is_mark(),
        }
    }

    // The scripts whose runs it may continue, none meaning any.
    fn scripts(&self) -> Option<&[Script]> {
        self.extensions.or(self.own.as_ref().map(core::slice::from_ref))
    }
}

// The script a run settled on, and until then the scripts all its characters allow, empty meaning any.
#[derive(Default)]
struct Run {
    resolved: Option<Script>,
    allowed: Vec<Script>,
}

impl Run {
    fn take(&mut self, claim: Claim, brackets: &mut Brackets) -> bool {
        let Some(scripts) = claim.scripts() else { return true };
        if let Some(r) = self.resolved {
            return claim.stays || scripts.contains(&r);
        }
        let kept: Vec<Script> = match self.allowed.is_empty() {
            true => scripts.to_vec(),
            false => self.allowed.iter().copied().filter(|s| scripts.contains(s)).collect(),
        };
        if kept.is_empty() {
            return claim.stays;
        }
        match claim.own.filter(|s| kept.contains(s)) {
            Some(own) => {
                self.resolved = Some(own);
                brackets.settle(own);
            }
            None => self.allowed = kept,
        }
        true
    }

    fn finish(&self, brackets: &mut Brackets) -> Script {
        let s = self.resolved.or(self.allowed.first().copied()).unwrap_or(Script::COMMON);
        brackets.settle(s);
        s
    }
}

// Open brackets with the script of their run. Those opened since the run began take its script once it has one.
#[derive(Default)]
struct Brackets {
    open: VecDeque<(char, Script)>,
    pending: usize,
}

impl Brackets {
    fn push(&mut self, opener: char, s: Script) {
        if self.open.len() == BRACKET_DEPTH {
            self.open.pop_front();
        }
        self.open.push_back((opener, s));
        self.pending = (self.pending + 1).min(BRACKET_DEPTH);
    }

    fn pop(&mut self) {
        if self.open.pop_back().is_some() {
            self.pending = self.pending.saturating_sub(1);
        }
    }

    // The script `opener`'s bracket opened in, dropping the unclosed ones after it; none when it isn't open.
    fn find(&mut self, opener: char) -> Option<Script> {
        while let Some(&(c, s)) = self.open.back() {
            if c == opener {
                return Some(s);
            }
            self.pop();
        }
        None
    }

    fn settle(&mut self, s: Script) {
        for bracket in self.open.iter_mut().rev().take(self.pending) {
            bracket.1 = s;
        }
        self.pending = 0;
    }
}
