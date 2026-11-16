use alloc::vec::Vec;
use core::ops::Range;

use crate::daecore::daeshaper::generated::unicode_tables as t;

pub(crate) use t::BidiClass as Class;

/// The direction a paragraph takes before its text has a say, as Unicode's bidi algorithm starts it.
#[expect(clippy::exhaustive_enums, reason = "UAX #9 gives a paragraph these three")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub enum BaseDirection {
    /// The direction of the paragraph's first strong character, left to right if it has none.
    #[default]
    Auto,
    /// Left to right, whatever the text.
    LeftToRight,
    /// Right to left, whatever the text.
    RightToLeft,
}

impl BaseDirection {
    // As `resolve` takes it: None to find it in the text, or whether it is right to left.
    pub(crate) fn rtl(self) -> Option<bool> {
        match self {
            BaseDirection::Auto => None,
            BaseDirection::LeftToRight => Some(false),
            BaseDirection::RightToLeft => Some(true),
        }
    }
}

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
    // L1 returns these to the paragraph level before a separator and at a line's end.
    fn is_reset_by_l1(self) -> bool {
        matches!(self, Class::WS | Class::LRI | Class::RLI | Class::FSI | Class::PDI) || self.is_removed_by_x9()
    }
    // The direction N0 to N2 read from a resolved class, numbers counting as R.
    fn strong(self) -> Option<Class> {
        match self {
            Class::L => Some(Class::L),
            Class::R | Class::EN | Class::AN => Some(Class::R),
            _ => None,
        }
    }
}

pub(crate) fn class_of(c: char) -> Class {
    super::props(c).bidi_class
}

pub(crate) fn bracket(c: char) -> Option<(char, bool)> {
    if super::props(c).flags & t::flag::BRACKET == 0 {
        return None;
    }
    let i = t::BIDI_BRACKETS.binary_search_by_key(&(c as u32), |&(k, _)| k).ok()?;
    Some((super::mirrored(c)?, t::BIDI_BRACKETS[i].1))
}

// BD16 pairs brackets up to canonical equivalence: U+2329 and U+232A are singletons for U+3008 and U+3009.
fn canonical_bracket(c: char) -> char {
    match super::decompose(c) {
        Some((single, None)) => single,
        _ => c,
    }
}

// UAX #9's max_depth (BD2): an embedding or isolate past it overflows, as X1 to X8 count it.
const MAX_DEPTH: u8 = 125;

// The text's levels and visual order, with each paragraph's start, end and level, in bytes: a level per byte, each
// character's start in visual order. `base_level` is the first paragraph's, and each reorders as a line of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub base_level: u8,
    pub levels: Vec<u8>,
    pub visual_order: Vec<usize>,
    pub paragraphs: Vec<(usize, usize, u8)>,
}

impl Paragraph {
    // The level of the paragraph holding byte `at`, or the first paragraph's past the end.
    pub fn paragraph_level(&self, at: usize) -> u8 {
        let k = self.paragraphs.partition_point(|&(_, end, _)| end <= at);
        self.paragraphs.get(k).map_or(self.base_level, |&(_, _, level)| level)
    }
}

// Resolved a character at a time, as UAX #9 reads them, then given in bytes. Levels spread from the end, as no
// character starts before its own index, then each paragraph's characters reorder by their first bytes.
pub fn resolve(text: &str, base: Option<bool>) -> Paragraph {
    let mut chars = Vec::with_capacity(text.len());
    chars.extend(text.chars());
    let classes: Vec<Class> = chars.iter().map(|&c| class_of(c)).collect();
    let n = chars.len();
    let mut levels = Vec::with_capacity(text.len());
    levels.resize(n, 0);
    let mut paragraphs = Vec::new();
    let mut start = 0;
    while start < n {
        let end = paragraph_end(&chars, &classes, start);
        let level = resolve_paragraph(&chars[start..end], &classes[start..end], base, &mut levels[start..end]);
        paragraphs.push((start, end, level));
        start = end;
    }
    if n != text.len() {
        let mut end = text.len();
        levels.resize(end, 0);
        for k in (0..n).rev() {
            let (level, start) = (levels[k], end - chars[k].len_utf8());
            // Indexed: as a slice the loop becomes a memset call, dearer than writing 1 to 4 bytes.
            #[allow(clippy::needless_range_loop)]
            for b in start..end {
                levels[b] = level;
            }
            end = start;
        }
    }
    let mut visual_order = Vec::with_capacity(n);
    let mut at = 0;
    for (start, end, _) in &mut paragraphs {
        let (from, first) = (visual_order.len(), at);
        for c in &chars[*start..*end] {
            visual_order.push(at);
            at += c.len_utf8();
        }
        reorder(&mut visual_order[from..], |b| levels[b]);
        (*start, *end) = (first, at);
    }
    let base_level = paragraphs.first().map_or(u8::from(base == Some(true)), |&(_, _, level)| level);
    Paragraph { base_level, levels, visual_order, paragraphs }
}

// P1: a paragraph runs through the separator that ends it, CR and LF counting as one.
fn paragraph_end(chars: &[char], classes: &[Class], start: usize) -> usize {
    let Some(at) = classes[start..].iter().position(|&c| c == Class::B).map(|b| start + b) else {
        return classes.len();
    };
    if chars[at] == '\r' && chars.get(at + 1) == Some(&'\n') { at + 2 } else { at + 1 }
}

// Resolves one paragraph's levels, through L1, and gives its level. One not set right to left, with no R, AL, AN or
// explicit formatting, is 0 throughout: W7 makes its European numbers L, and N1, N2 and L1 raise nothing.
fn resolve_paragraph(chars: &[char], classes: &[Class], base: Option<bool>, levels: &mut [u8]) -> u8 {
    let plain = |&c: &Class| (c as u8) < Class::LRE as u8 && !matches!(c, Class::R | Class::AL | Class::AN);
    if base != Some(true) && classes.iter().all(plain) {
        levels.fill(0);
        return 0;
    }
    resolve_fully(chars, classes, base, levels)
}

fn resolve_fully(chars: &[char], classes: &[Class], base: Option<bool>, levels: &mut [u8]) -> u8 {
    let isolates = Isolates::find(classes);
    let base_level = match base {
        Some(rtl) => u8::from(rtl),
        None => u8::from(isolates.paragraph_rtl),
    };

    levels.fill(base_level);
    // X9 removes explicit formatting characters and BN from the runs, but their *levels* still get
    // reported, so they are tracked rather than deleted.
    let mut result = classes.to_vec();
    explicit_levels(classes, &isolates, base_level, levels, &mut result);

    for seq in isolating_run_sequences(classes, &isolates, levels, base_level) {
        resolve_sequence(chars, &seq, &mut result);
    }

    implicit_levels(&result, levels);
    separator_levels(classes, levels, base_level);
    base_level
}

// BD9's isolate pairs, both ways, and P2's first strong class of each isolate and of the paragraph, from one pass
// with a stack. A character inside a nested isolate counts only for the innermost.
struct Isolates {
    partner: Vec<Option<usize>>,
    rtl: Vec<bool>,
    paragraph_rtl: bool,
}

impl Isolates {
    fn find(classes: &[Class]) -> Isolates {
        let mut partner = alloc::vec![None; classes.len()];
        let mut first: Vec<Option<bool>> = alloc::vec![None; classes.len()];
        let mut paragraph = None;
        let mut open: Vec<usize> = Vec::new();
        for (i, &c) in classes.iter().enumerate() {
            match c {
                Class::LRI | Class::RLI | Class::FSI => open.push(i),
                Class::PDI => {
                    if let Some(o) = open.pop() {
                        partner[o] = Some(i);
                        partner[i] = Some(o);
                    }
                }
                Class::L | Class::R | Class::AL => {
                    let slot = match open.last() {
                        Some(&o) => &mut first[o],
                        None => &mut paragraph,
                    };
                    if slot.is_none() {
                        *slot = Some(c != Class::L);
                    }
                }
                _ => {}
            }
        }
        let rtl = first.into_iter().map(|f| f == Some(true)).collect();
        Isolates { partner, rtl, paragraph_rtl: paragraph == Some(true) }
    }
}

#[derive(Clone, Copy)]
struct Status {
    level: u8,
    override_status: Option<Class>,
    isolate: bool,
}

// X1–X8: explicit embeddings, overrides and isolates.
fn explicit_levels(classes: &[Class], isolates: &Isolates, base_level: u8, levels: &mut [u8], result: &mut [Class]) {
    let base = Status { level: base_level, override_status: None, isolate: false };
    let mut stack = alloc::vec![base];
    let mut overflow_isolate = 0usize;
    let mut overflow_embedding = 0usize;
    let mut valid_isolate = 0usize;
    // A character takes the level of the entry on top, and its override.
    let take = |i: usize, top: Status, levels: &mut [u8], result: &mut [Class]| {
        levels[i] = top.level;
        if let Some(o) = top.override_status {
            result[i] = o;
        }
    };

    for (i, &class) in classes.iter().enumerate() {
        let top = stack.last().copied().unwrap_or(base);
        match class {
            Class::RLE | Class::LRE | Class::RLO | Class::LRO => {
                // X2–X5: the embedding's own level is the last one reported for it.
                take(i, top, levels, result);
                let rtl = matches!(class, Class::RLE | Class::RLO);
                let next = if rtl { (top.level + 1) | 1 } else { (top.level + 2) & !1 };
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
                    _ => isolates.rtl[i],
                };
                take(i, top, levels, result);
                let next = if rtl { (top.level + 1) | 1 } else { (top.level + 2) & !1 };
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
                take(i, stack.last().copied().unwrap_or(base), levels, result);
            }
            Class::PDF => {
                // X7.
                levels[i] = top.level;
                if overflow_isolate > 0 {
                } else if overflow_embedding > 0 {
                    overflow_embedding -= 1;
                } else if !top.isolate && stack.len() >= 2 {
                    stack.pop();
                }
            }
            // X8: the separator ending the paragraph takes its level.
            Class::B => levels[i] = base_level,
            // X6.
            _ => take(i, top, levels, result),
        }
    }
}

struct Sequence {
    indices: Vec<usize>,
    level: u8,
    sos: Class,
    eos: Class,
}

impl Sequence {
    fn embedding_direction(&self) -> Class {
        if self.level.is_multiple_of(2) { Class::L } else { Class::R }
    }
}

// Every level run must land in an isolating run sequence, or W1–W7 and N0–N2 never resolve it; BidiTest,
// BidiCharacterTest and a sweep of 4,195,278 cases leave none out, and the assert below keeps it so.
fn isolating_run_sequences(classes: &[Class], isolates: &Isolates, levels: &[u8], base_level: u8) -> Vec<Sequence> {
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
        if classes[first] == Class::PDI && isolates.partner[first].is_some() { continue; }

        let mut indices = Vec::new();
        let mut cur = start;
        loop {
            used[cur] = true;
            let (a, b) = runs[cur];
            indices.extend_from_slice(&kept[a..b]);
            let last = kept.get(b.wrapping_sub(1)).copied().unwrap_or(0);
            if !classes[last].is_isolate_initiator() { break; }
            let Some(pdi) = isolates.partner[last] else { break };
            match run_starting_at(pdi) {
                Some(next) if !used[next] => cur = next,
                _ => break,
            }
        }

        let level = levels[indices[0]];
        // X10: sos/eos come from the higher of this sequence's level and its neighbor's.
        let before = prev_kept_level(&kept, levels, indices[0], base_level);
        let last = *indices.last().unwrap_or(&0);
        let after = if classes[last].is_isolate_initiator() && isolates.partner[last].is_none() {
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
    let embedding = seq.embedding_direction();
    let mut i = 0;
    while i < n {
        if !cls[i].is_neutral_or_isolate() { i += 1; continue; }
        let start = i;
        while i < n && cls[i].is_neutral_or_isolate() { i += 1; }
        let before = if start > 0 { cls[start - 1].strong() } else { seq.sos.strong() };
        let after = if i < n { cls[i].strong() } else { seq.eos.strong() };
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
    let embedding = seq.embedding_direction();
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

    for &(open, close) in &pairs {
        // N0 b: the embedding direction inside the pair wins.
        let mut found_embedding = false;
        let mut found_opposite = false;
        for c in cls.iter().take(close).skip(open + 1) {
            match c.strong() {
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
                if let Some(s) = c.strong() { prior = s; break; }
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

// X9's removed characters take the level before them, and L1 returns separators, and the whitespace and isolate
// formatting before them or at the paragraph's end, to the paragraph level, by their original classes.
fn separator_levels(classes: &[Class], levels: &mut [u8], base_level: u8) {
    for i in 0..classes.len() {
        if classes[i].is_removed_by_x9() {
            levels[i] = if i == 0 { base_level } else { levels[i - 1] };
        }
    }
    let reset_before = |levels: &mut [u8], from: usize| {
        for j in (0..from).rev().take_while(|&j| classes[j].is_reset_by_l1()) {
            levels[j] = base_level;
        }
    };
    for i in 0..classes.len() {
        if matches!(classes[i], Class::B | Class::S) {
            levels[i] = base_level;
            reset_before(levels, i);
        }
    }
    reset_before(levels, classes.len());
}

// L2: `order`'s entries in display order, every run at or above each level from the highest down to the lowest odd
// one reversed; `level` gives an entry's. The entries can be indices or offsets, as long as `level` reads them.
pub(crate) fn reorder(order: &mut [usize], level: impl Fn(usize) -> u8) {
    let (mut max, mut min_odd) = (0, u8::MAX);
    for &e in order.iter() {
        let l = level(e);
        max = max.max(l);
        if l % 2 == 1 {
            min_odd = min_odd.min(l);
        }
    }
    let mut at = max;
    while at >= min_odd && at > 0 {
        let mut i = 0;
        while i < order.len() {
            if level(order[i]) < at {
                i += 1;
                continue;
            }
            let start = i;
            while i < order.len() && level(order[i]) >= at {
                i += 1;
            }
            order[start..i].reverse();
        }
        at -= 1;
    }
}

// Runs in display order, each one level and so one span of the text, in bytes: a reversal at level k takes whole
// runs at k or above, so the characters of a run are contiguous whichever way they read.
#[derive(Default)]
pub struct VisualRuns(Vec<(Range<usize>, u8)>);

impl VisualRuns {
    // A run ends where the level changes, or where `cut` starts a paragraph.
    fn push(&mut self, chars: Range<usize>, level: u8, cut: bool) {
        match self.0.last_mut() {
            Some((span, l)) if *l == level && !cut => {
                span.start = span.start.min(chars.start);
                span.end = span.end.max(chars.end);
            }
            _ => self.0.push((chars, level)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Range<usize>, u8)> + '_ {
        self.0.iter().cloned()
    }
}

// One line's runs: bytes `start` to `end` of `text`, whose levels `levels` gives a byte at a time; `base_level` is
// the level of the paragraph holding it. Reordered a character at a time, by each one's first byte.
pub fn line_visual_runs(base_level: u8, levels: &[u8], text: &str, start: usize, end: usize) -> VisualRuns {
    let mut out = VisualRuns::default();
    let Some(line) = text.get(start..end).filter(|_| start < end && end <= levels.len()) else { return out };
    // At the paragraph's level throughout, the line is one run, which L1 and reordering leave as it is.
    if levels[start..end].iter().all(|&l| l == base_level) {
        out.push(start..end, base_level, false);
        return out;
    }
    reordered(base_level, levels, line, start, out)
}

// Any line: L1's resets, then L2's reversals.
fn reordered(base_level: u8, levels: &[u8], line: &str, start: usize, mut out: VisualRuns) -> VisualRuns {
    let mut line_levels = levels[start..start + line.len()].to_vec();
    // L1, final clause: trailing whitespace at the end of the line goes back to the paragraph level,
    // so a space closing an RTL line still sits where the reader expects it.
    for (at, c) in line.char_indices().rev().take_while(|&(_, c)| class_of(c).is_reset_by_l1()) {
        line_levels[at..at + c.len_utf8()].fill(base_level);
    }
    let mut order: Vec<usize> = line.char_indices().map(|(at, _)| at).collect();
    reorder(&mut order, |at| line_levels[at]);
    for at in order {
        let width = line[at..].chars().next().map_or(1, char::len_utf8);
        out.push(start + at..start + at + width, line_levels[at], false);
    }
    out
}

// Each paragraph's runs, every paragraph reordered as a line of its own.
pub fn visual_runs(p: &Paragraph, text: &str) -> VisualRuns {
    let mut out = VisualRuns::default();
    for &(start, end, level) in &p.paragraphs {
        let first = p.visual_order.partition_point(|&at| at < start);
        let last = first + text.get(start..end).map_or(0, |t| t.chars().count());
        for (k, &at) in p.visual_order.get(first..last).unwrap_or_default().iter().enumerate() {
            let width = text[at..].chars().next().map_or(1, char::len_utf8);
            out.push(at..at + width, p.levels.get(at).copied().unwrap_or(level), k == 0);
        }
    }
    out
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/bidi.rs"]
mod tests;
