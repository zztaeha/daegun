use alloc::vec::Vec;

use crate::daecore::daeshaper::generated::unicode_tables as t;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
#[allow(clippy::upper_case_acronyms, reason = "these are UAX #9's names for its own classes")]
pub(crate) enum Class {
    L = 0, R, AL, EN, ES, ET, AN, CS, NSM, BN,
    B, S, WS, ON, LRE, LRO, RLE, RLO, PDF, LRI, RLI, FSI, PDI,
}

const CLASSES: [Class; 23] = [
    Class::L, Class::R, Class::AL, Class::EN, Class::ES, Class::ET, Class::AN, Class::CS,
    Class::NSM, Class::BN, Class::B, Class::S, Class::WS, Class::ON, Class::LRE, Class::LRO,
    Class::RLE, Class::RLO, Class::PDF, Class::LRI, Class::RLI, Class::FSI, Class::PDI,
];

impl Class {
    fn is_isolate_initiator(self) -> bool {
        matches!(self, Class::LRI | Class::RLI | Class::FSI)
    }
    fn is_explicit(self) -> bool {
        matches!(self, Class::LRE | Class::LRO | Class::RLE | Class::RLO | Class::PDF)
    }
    fn is_removed_by_x9(self) -> bool {
        self.is_explicit() || self == Class::BN
    }
    fn is_neutral_or_isolate(self) -> bool {
        matches!(self, Class::B | Class::S | Class::WS | Class::ON)
            || self.is_isolate_initiator()
            || self == Class::PDI
    }
}

pub(crate) fn class_of(c: char) -> Class {
    let raw = super::props(c).bidi_class;
    CLASSES[raw as usize % CLASSES.len()]
}

fn bracket(c: char) -> Option<(char, bool)> {
    let cp = c as u32;
    let i = t::BIDI_BRACKETS.binary_search_by_key(&cp, |&(k, _, _)| k).ok()?;
    let (_, paired, is_open) = t::BIDI_BRACKETS[i];
    Some((char::from_u32(paired)?, is_open == 1))
}

fn canonical_bracket(c: char) -> char {
    match c {
        '\u{3008}' => '\u{2329}',
        '\u{3009}' => '\u{232A}',
        other => other,
    }
}

const MAX_DEPTH: u8 = 125;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub base_level: u8,
    pub levels: Vec<u8>,
    pub visual_order: Vec<usize>,
}

pub fn resolve(text: &str, base: Option<bool>) -> Paragraph {
    let chars: Vec<char> = text.chars().collect();
    let classes: Vec<Class> = chars.iter().map(|&c| class_of(c)).collect();
    resolve_classes(&chars, &classes, base)
}

pub(crate) fn resolve_classes(chars: &[char], classes: &[Class], base: Option<bool>) -> Paragraph {
    let n = classes.len();
    let base_level = match base {
        Some(true) => 1,
        Some(false) => 0,
        None => paragraph_level(classes, 0, n),
    };

    let mut levels = alloc::vec![base_level; n];
    // X9 removes explicit formatting characters and BN from the runs, but their *levels* still get
    // reported, so they are tracked rather than deleted.
    let mut result = classes.to_vec();
    explicit_levels(classes, base_level, &mut levels, &mut result);

    for seq in isolating_run_sequences(classes, &levels, base_level) {
        resolve_sequence(chars, &seq, &mut result);
    }

    implicit_levels(&result, &mut levels);
    let visual_order = reorder(classes, &mut levels, base_level);

    Paragraph { base_level, levels, visual_order }
}

fn paragraph_level(classes: &[Class], from: usize, to: usize) -> u8 {
    let mut depth = 0usize;
    for &c in &classes[from..to] {
        if c.is_isolate_initiator() { depth += 1; continue; }
        if c == Class::PDI { depth = depth.saturating_sub(1); continue; }
        if depth > 0 { continue; }
        match c {
            Class::L => return 0,
            Class::R | Class::AL => return 1,
            _ => {}
        }
    }
    0
}

#[derive(Clone, Copy)]
struct Status {
    level: u8,
    override_status: Option<Class>,
    isolate: bool,
}

// X1–X8: explicit embeddings, overrides and isolates.
fn explicit_levels(classes: &[Class], base_level: u8, levels: &mut [u8], result: &mut [Class]) {
    let mut stack = alloc::vec![Status { level: base_level, override_status: None, isolate: false }];
    let mut overflow_isolate = 0usize;
    let mut overflow_embedding = 0usize;
    let mut valid_isolate = 0usize;

    for i in 0..classes.len() {
        let class = classes[i];
        match class {
            Class::RLE | Class::LRE | Class::RLO | Class::LRO => {
                // X2–X5: the embedding's own level is the last one reported for it.
                levels[i] = stack.last().map_or(base_level, |s| s.level);
                if let Some(s) = stack.last()
                    && let Some(o) = s.override_status { result[i] = o; }
                let rtl = matches!(class, Class::RLE | Class::RLO);
                let last = stack.last().map_or(base_level, |s| s.level);
                let next = if rtl { (last + 1) | 1 } else { (last + 2) & !1 };
                if next <= MAX_DEPTH && overflow_isolate == 0 && overflow_embedding == 0 {
                    stack.push(Status {
                        level: next,
                        override_status: match class {
                            Class::RLO => Some(Class::R),
                            Class::LRO => Some(Class::L),
                            _ => None,
                        },
                        isolate: false,
                    });
                } else if overflow_isolate == 0 {
                    overflow_embedding += 1;
                }
            }
            Class::RLI | Class::LRI | Class::FSI => {
                // X5a–X5c. An FSI takes the direction of its own contents.
                let rtl = match class {
                    Class::RLI => true,
                    Class::LRI => false,
                    _ => paragraph_level(classes, i + 1, matching_pdi(classes, i)) == 1,
                };
                let last = stack.last().copied()
                    .unwrap_or(Status { level: base_level, override_status: None, isolate: false });
                levels[i] = last.level;
                if let Some(o) = last.override_status { result[i] = o; }
                let next = if rtl { (last.level + 1) | 1 } else { (last.level + 2) & !1 };
                if next <= MAX_DEPTH && overflow_isolate == 0 && overflow_embedding == 0 {
                    valid_isolate += 1;
                    stack.push(Status { level: next, override_status: None, isolate: true });
                } else {
                    overflow_isolate += 1;
                }
            }
            Class::PDI => {
                // X6a.
                if overflow_isolate > 0 {
                    overflow_isolate -= 1;
                } else if valid_isolate > 0 {
                    overflow_embedding = 0;
                    while let Some(s) = stack.last() {
                        if s.isolate { break; }
                        stack.pop();
                    }
                    stack.pop();
                    valid_isolate -= 1;
                }
                let last = stack.last().copied()
                    .unwrap_or(Status { level: base_level, override_status: None, isolate: false });
                levels[i] = last.level;
                if let Some(o) = last.override_status { result[i] = o; }
            }
            Class::PDF => {
                // X7.
                levels[i] = stack.last().map_or(base_level, |s| s.level);
                if overflow_isolate > 0 {
                } else if overflow_embedding > 0 {
                    overflow_embedding -= 1;
                } else if stack.last().is_some_and(|s| !s.isolate) && stack.len() >= 2 {
                    stack.pop();
                }
            }
            Class::B => {
                // X8: a paragraph separator resets everything and takes the base level.
                stack.truncate(1);
                overflow_isolate = 0;
                overflow_embedding = 0;
                valid_isolate = 0;
                levels[i] = base_level;
            }
            _ => {
                // X6.
                let last = stack.last().copied()
                    .unwrap_or(Status { level: base_level, override_status: None, isolate: false });
                levels[i] = last.level;
                if let Some(o) = last.override_status { result[i] = o; }
            }
        }
    }
}

fn matching_pdi(classes: &[Class], from: usize) -> usize {
    let mut depth = 1usize;
    for (i, &c) in classes.iter().enumerate().skip(from + 1) {
        if c.is_isolate_initiator() { depth += 1; }
        else if c == Class::PDI {
            depth -= 1;
            if depth == 0 { return i; }
        }
    }
    classes.len()
}

struct Sequence {
    indices: Vec<usize>,
    level: u8,
    sos: Class,
    eos: Class,
}

// Every level run must land in some isolating run sequence, or W1-W7 and N0-N2 never run on it and
// its characters keep their raw classes. Checked rather than argued: zero orphans over BidiTest,
// BidiCharacterTest and an exhaustive sweep – 4,195,278 cases – and asserted below so a future
// change to level assignment cannot quietly reintroduce one.
fn isolating_run_sequences(classes: &[Class], levels: &[u8], base_level: u8) -> Vec<Sequence> {
    let n = classes.len();
    let kept: Vec<usize> = (0..n).filter(|&i| !classes[i].is_removed_by_x9()).collect();

    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (at, &i) in kept.iter().enumerate() {
        match runs.last_mut() {
            Some((a, end)) if levels[kept[*a]] == levels[i] => *end = at + 1,
            _ => runs.push((at, at + 1)),
        }
    }

    let run_starting_at =
        |at: usize| runs.binary_search_by_key(&at, |&(a, _)| kept[a]).ok();

    let mut used = alloc::vec![false; runs.len()];
    let mut out = Vec::new();
    for start in 0..runs.len() {
        if used[start] { continue; }
        let first = kept[runs[start].0];
        // BD13: a sequence begins at a run whose first character is not a PDI matching an earlier
        // initiator.
        if classes[first] == Class::PDI && matched_initiator(classes, first).is_some() { continue; }

        let mut indices = Vec::new();
        let mut cur = start;
        loop {
            used[cur] = true;
            let (a, b) = runs[cur];
            indices.extend_from_slice(&kept[a..b]);
            let last = kept.get(b.wrapping_sub(1)).copied().unwrap_or(0);
            if !classes[last].is_isolate_initiator() { break; }
            let pdi = matching_pdi(classes, last);
            if pdi >= n { break; }
            match run_starting_at(pdi) {
                Some(next) if !used[next] => cur = next,
                _ => break,
            }
        }

        let level = levels[indices[0]];
        // X10: sos/eos come from the higher of this sequence's level and its neighbor's.
        let before = prev_kept_level(&kept, levels, indices[0], base_level);
        let last = *indices.last().unwrap_or(&0);
        let after = if classes[last].is_isolate_initiator() && matching_pdi(classes, last) >= n {
            base_level
        } else {
            next_kept_level(&kept, levels, last, base_level)
        };
        let sos = if !level.max(before).is_multiple_of(2) { Class::R } else { Class::L };
        let eos = if !level.max(after).is_multiple_of(2) { Class::R } else { Class::L };
        out.push(Sequence { indices, level, sos, eos });
    }

    debug_assert!(
        used.iter().all(|&u| u),
        "a level run was assigned to no isolating run sequence; see BD13 and X6a above",
    );
    out
}

fn matched_initiator(classes: &[Class], pdi: usize) -> Option<usize> {
    let mut depth = 0usize;
    for i in (0..pdi).rev() {
        if classes[i] == Class::PDI { depth += 1; }
        else if classes[i].is_isolate_initiator() {
            if depth == 0 { return Some(i); }
            depth -= 1;
        }
    }
    None
}

fn prev_kept_level(kept: &[usize], levels: &[u8], at: usize, base: u8) -> u8 {
    let at_or_after = kept.partition_point(|&i| i < at);
    match at_or_after.checked_sub(1) {
        Some(k) => levels[kept[k]],
        None => base,
    }
}

fn next_kept_level(kept: &[usize], levels: &[u8], at: usize, base: u8) -> u8 {
    let after = kept.partition_point(|&i| i <= at);
    kept.get(after).map_or(base, |&i| levels[i])
}

// W1–W7, N0–N2 over one isolating run sequence.
fn resolve_sequence(chars: &[char], seq: &Sequence, result: &mut [Class]) {
    let idx = &seq.indices;
    let n = idx.len();
    if n == 0 { return; }
    let mut cls: Vec<Class> = idx.iter().map(|&i| result[i]).collect();

    // W1: NSM takes the class of what precedes it; after an isolate it becomes ON.
    let mut prev = seq.sos;
    for c in cls.iter_mut() {
        if *c == Class::NSM {
            *c = if prev.is_isolate_initiator() || prev == Class::PDI { Class::ON } else { prev };
        }
        prev = *c;
    }

    // W2: EN becomes AN when the last strong type was AL.
    let mut strong = seq.sos;
    for c in cls.iter_mut() {
        match *c {
            Class::L | Class::R | Class::AL => strong = *c,
            Class::EN if strong == Class::AL => *c = Class::AN,
            _ => {}
        }
    }

    // W3.
    for c in cls.iter_mut() {
        if *c == Class::AL { *c = Class::R; }
    }

    // W4: a single separator between two numbers of the same kind joins them.
    for i in 1..n.saturating_sub(1) {
        let (before, after) = (cls[i - 1], cls[i + 1]);
        if cls[i] == Class::ES && before == Class::EN && after == Class::EN { cls[i] = Class::EN; }
        if cls[i] == Class::CS && before == after && matches!(before, Class::EN | Class::AN) {
            cls[i] = before;
        }
    }

    // W5: a run of ET adjacent to EN becomes EN.
    let mut i = 0;
    while i < n {
        if cls[i] != Class::ET { i += 1; continue; }
        let start = i;
        while i < n && cls[i] == Class::ET { i += 1; }
        let before = if start > 0 { cls[start - 1] } else { seq.sos };
        let after = if i < n { cls[i] } else { seq.eos };
        if before == Class::EN || after == Class::EN {
            for c in cls.iter_mut().take(i).skip(start) { *c = Class::EN; }
        }
    }

    // W6.
    for c in cls.iter_mut() {
        if matches!(*c, Class::ES | Class::ET | Class::CS) { *c = Class::ON; }
    }

    // W7: EN becomes L when the last strong type was L.
    let mut strong = seq.sos;
    for c in cls.iter_mut() {
        match *c {
            Class::L | Class::R => strong = *c,
            Class::EN if strong == Class::L => *c = Class::L,
            _ => {}
        }
    }

    // N0: paired brackets take the embedding direction when it appears inside them.
    resolve_brackets(chars, idx, &mut cls, seq);

    // N1, N2: a run of neutrals between matching strong types takes that type, otherwise the
    // embedding direction.
    let embedding = if !seq.level.is_multiple_of(2) { Class::R } else { Class::L };
    let strong_of = |c: Class| match c {
        Class::L => Some(Class::L),
        Class::R | Class::EN | Class::AN => Some(Class::R),
        _ => None,
    };
    let mut i = 0;
    while i < n {
        if !cls[i].is_neutral_or_isolate() { i += 1; continue; }
        let start = i;
        while i < n && cls[i].is_neutral_or_isolate() { i += 1; }
        let before = if start > 0 { strong_of(cls[start - 1]) } else { strong_of(seq.sos) };
        let after = if i < n { strong_of(cls[i]) } else { strong_of(seq.eos) };
        let fill = match (before, after) {
            (Some(a), Some(b)) if a == b => a,
            _ => embedding,
        };
        for c in cls.iter_mut().take(i).skip(start) { *c = fill; }
    }

    for (k, &i) in idx.iter().enumerate() {
        result[i] = cls[k];
    }
}

fn resolve_brackets(chars: &[char], idx: &[usize], cls: &mut [Class], seq: &Sequence) {
    if chars.is_empty() { return; }
    let embedding = if !seq.level.is_multiple_of(2) { Class::R } else { Class::L };
    let opposite = if embedding == Class::R { Class::L } else { Class::R };

    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (k, &i) in idx.iter().enumerate() {
        if cls[k] != Class::ON { continue; }
        let Some(c) = chars.get(i).copied() else { continue };
        let Some((paired, is_open)) = bracket(c) else { continue };
        if is_open {
            if stack.len() >= 63 { break; }
            stack.push((canonical_bracket(paired), k));
        } else {
            let want = canonical_bracket(c);
            if let Some(pos) = stack.iter().rposition(|&(close, _)| close == want) {
                pairs.push((stack[pos].1, k));
                stack.truncate(pos);
            }
        }
    }
    pairs.sort_unstable();

    let strong_of = |c: Class| match c {
        Class::L => Some(Class::L),
        Class::R | Class::EN | Class::AN => Some(Class::R),
        _ => None,
    };

    for &(open, close) in &pairs {
        // N0 b: the embedding direction inside the pair wins.
        let mut found_embedding = false;
        let mut found_opposite = false;
        for c in cls.iter().take(close).skip(open + 1) {
            match strong_of(*c) {
                Some(s) if s == embedding => { found_embedding = true; break; }
                Some(_) => found_opposite = true,
                None => {}
            }
        }
        let set = if found_embedding {
            Some(embedding)
        } else if found_opposite {
            // N0 c: opposite direction inside, so the context before the pair decides.
            let mut prior = seq.sos;
            for c in cls.iter().take(open).rev() {
                if let Some(s) = strong_of(*c) { prior = s; break; }
            }
            if prior == opposite { Some(opposite) } else { Some(embedding) }
        } else {
            None
        };

        if let Some(dir) = set {
            cls[open] = dir;
            cls[close] = dir;
            for k in [open, close] {
                for (j, slot) in cls.iter_mut().enumerate().skip(k + 1) {
                    let orig = idx.get(j).map(|&i| class_of_original(chars, i));
                    if orig != Some(Class::NSM) { break; }
                    *slot = dir;
                }
            }
        }
    }
}

fn class_of_original(chars: &[char], i: usize) -> Class {
    chars.get(i).map_or(Class::L, |&c| class_of(c))
}

// I1, I2.
fn implicit_levels(result: &[Class], levels: &mut [u8]) {
    for (i, &c) in result.iter().enumerate() {
        let level = levels[i];
        levels[i] = if level.is_multiple_of(2) {
            match c {
                Class::R => level + 1,
                Class::AN | Class::EN => level + 2,
                _ => level,
            }
        } else {
            match c {
                Class::L | Class::AN | Class::EN => level + 1,
                _ => level,
            }
        };
    }
}

// L1–L2: reset separators and trailing whitespace, then reverse by level.
fn reorder(classes: &[Class], levels: &mut [u8], base_level: u8) -> Vec<usize> {
    let n = classes.len();

    for i in 0..n {
        if classes[i].is_removed_by_x9() {
            levels[i] = if i == 0 { base_level } else { levels[i - 1] };
        }
    }

    // L1: a segment or paragraph separator goes back to the paragraph level, and so does any
    // whitespace *immediately preceding* one, and any trailing whitespace at the end of the line.
    // Not everything after a separator – that is a different and much more destructive rule.
    // Uses the *original* classes, not the resolved ones: the one place the algorithm looks back
    // past its own work.
    let resettable = |c: Class| {
        matches!(c, Class::WS | Class::LRI | Class::RLI | Class::FSI | Class::PDI)
            || c.is_removed_by_x9()
    };
    let reset_run_before = |levels: &mut [u8], from: usize| {
        let mut j = from;
        while j > 0 && resettable(classes[j - 1]) {
            j -= 1;
            levels[j] = base_level;
        }
    };
    for i in 0..n {
        if matches!(classes[i], Class::B | Class::S) {
            levels[i] = base_level;
            reset_run_before(levels, i);
        }
    }
    reset_run_before(levels, n);

    reorder_levels(levels)
}

// L2 alone: from the highest level down to the lowest odd level, reverse each maximal run at or
// above that level. Returns positions in display order.
pub(crate) fn reorder_levels(levels: &[u8]) -> Vec<usize> {
    let n = levels.len();
    let mut order: Vec<usize> = (0..n).collect();
    let Some(&max) = levels.iter().max() else { return order };
    let min_odd = levels.iter().copied().filter(|l| !l.is_multiple_of(2)).min().unwrap_or(max + 1);
    let mut level = max;
    while level >= min_odd && level > 0 {
        let mut i = 0;
        while i < n {
            if levels[order[i]] < level { i += 1; continue; }
            let start = i;
            while i < n && levels[order[i]] >= level { i += 1; }
            order[start..i].reverse();
        }
        level -= 1;
    }
    order
}

// L2's reversal, both applied to the line rather than to the paragraph.
pub struct VisualRuns {
    indices: Vec<usize>,
    bounds: Vec<(usize, u8)>,
}

impl VisualRuns {
    fn push(&mut self, index: usize, level: u8) {
        match self.bounds.last_mut() {
            Some((end, l)) if *l == level => *end += 1,
            _ => self.bounds.push((self.indices.len() + 1, level)),
        }
        self.indices.push(index);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[usize], u8)> + '_ {
        let mut from = 0usize;
        self.bounds.iter().map(move |&(end, level)| {
            let run = &self.indices[from..end];
            from = end;
            (run, level)
        })
    }

}

pub fn line_visual_runs(
    base_level: u8,
    para_levels: &[u8],
    chars: &[char],
    start: usize,
    end: usize,
) -> VisualRuns {
    let mut out = VisualRuns { indices: Vec::new(), bounds: Vec::new() };
    if start >= end || end > para_levels.len() || end > chars.len() {
        return out;
    }
    let mut levels: Vec<u8> = para_levels[start..end].to_vec();
    // L1, final clause: trailing whitespace at the end of the line goes back to the paragraph level,
    // so a space closing an RTL line still sits where the reader expects it.
    let mut j = levels.len();
    while j > 0 && {
        let c = class_of(chars[start + j - 1]);
        matches!(c, Class::WS | Class::FSI | Class::LRI | Class::RLI | Class::PDI)
            || c.is_removed_by_x9()
    } {
        j -= 1;
        levels[j] = base_level;
    }

    let order = reorder_levels(&levels);
    out.indices.reserve(order.len());
    for i in order {
        out.push(start + i, levels[i]);
    }
    out
}

pub fn visual_runs(p: &Paragraph) -> VisualRuns {
    let mut out =
        VisualRuns { indices: Vec::with_capacity(p.visual_order.len()), bounds: Vec::new() };
    for &i in &p.visual_order {
        out.push(i, p.levels.get(i).copied().unwrap_or(p.base_level));
    }
    out
}
