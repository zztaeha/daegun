#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::string::String;
use core::cell::RefCell;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::range::Range;

use crate::daecore::daekit::cache::{AxisKey, FontCache, RunContext};
use crate::daecore::daekit::text::linebreak::{break_lines, Breakpoint, Cut, Extent, Fit, Reshaped};
use crate::daecore::daekit::text::shape::{Feature, Offset, ShapedRun};
use crate::daecore::daeshaper::unicode::bidi::{self, BaseDirection};
use crate::daecore::daeshaper::unicode::{general_category, is_upright, vertical_form, GeneralCategory};
use crate::daecore::daekit::text::pieces::{Piece, Pieces};

pub use crate::daecore::daekit::text::linebreak::BreakStrategy;

/// Where lines sit in the measure.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// At the start edge: the left in a left-to-right paragraph, the right in a right-to-left one.
    #[default]
    Start,
    /// At the end edge.
    End,
    /// Centered.
    Center,
    /// Spread to fill the measure, but for a paragraph's last line, which takes the start edge.
    Justify,
}

/// Which way lines run, and which way they stack.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WritingMode {
    /// Lines run across and stack downward.
    #[default]
    Horizontal,
    /// Lines run downward and stack leftward, as Japanese is set.
    VerticalRl,
    /// Lines run downward and stack rightward, as Mongolian is set.
    VerticalLr,
}

impl WritingMode {
    /// Whether lines run downward.
    pub const fn is_vertical(self) -> bool {
        !matches!(self, WritingMode::Horizontal)
    }
}

/// How characters stand in vertical lines.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextOrientation {
    /// Upright where Unicode says so, as CJK is, and turned sideways otherwise.
    #[default]
    Mixed,
    /// All upright, laid out left to right.
    Upright,
    /// All turned sideways.
    Sideways,
}

/// How to lay text out in lines. `LayoutOptions::default()` sets one line a paragraph, unwrapped, across.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions<'a> {
    /// The measure lines wrap at, in font units; infinite, the default, to wrap only at hard breaks. Below 0 it wraps
    /// as 0 does, and NaN as infinity.
    pub max_inline_size: f64,
    /// Where lines sit in the measure.
    pub align: Align,
    /// Which way lines run.
    pub writing_mode: WritingMode,
    /// How characters stand in vertical lines.
    pub text_orientation: TextOrientation,
    /// The paragraphs' direction.
    pub base_direction: BaseDirection,
    /// The language, as a BCP 47 tag, whose lookups apply.
    pub language: Option<&'a str>,
    /// From one baseline to the next, in font units; the font's line metrics' when none.
    pub line_height: Option<f64>,
    /// How lines are chosen.
    pub strategy: BreakStrategy,
    /// How many lines to lay out at most; [`TextLayout::truncated`] says where the rest begin.
    pub max_lines: Option<usize>,
    /// The distance between tab stops, in advances of the space, as CSS's tab-size counts it.
    pub tab_size: f64,
    /// Where text written without spaces between words, Thai or Khmer, breaks between them, in place of the
    /// `dictionaries` feature's word lists.
    pub segmenter: Option<crate::daecore::daekit::text::WordSegmenter<'a>>,
    /// OpenType features to turn on, off or set, each over its bytes of the text: small capitals, tabular figures.
    pub features: &'a [Feature],
}

impl Default for LayoutOptions<'_> {
    fn default() -> Self {
        LayoutOptions::DEFAULT
    }
}

impl<'a> LayoutOptions<'a> {
    /// The defaults, as `default()` gives them, for options of your own built as a constant.
    pub const DEFAULT: LayoutOptions<'static> = LayoutOptions {
        max_inline_size: f64::INFINITY,
        align: Align::Start,
        writing_mode: WritingMode::Horizontal,
        text_orientation: TextOrientation::Mixed,
        base_direction: BaseDirection::Auto,
        language: None,
        line_height: None,
        strategy: BreakStrategy::Greedy,
        max_lines: None,
        tab_size: 8.0,
        segmenter: None,
        features: &[],
    };

    /// These options wrapping at a measure.
    #[must_use]
    pub const fn with_max_inline_size(mut self, size: f64) -> Self {
        self.max_inline_size = size;
        self
    }

    /// These options aligning lines so.
    #[must_use]
    pub const fn with_align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// These options in a writing mode.
    #[must_use]
    pub const fn with_writing_mode(mut self, mode: WritingMode) -> Self {
        self.writing_mode = mode;
        self
    }

    /// These options standing characters so in vertical lines.
    #[must_use]
    pub const fn with_text_orientation(mut self, orientation: TextOrientation) -> Self {
        self.text_orientation = orientation;
        self
    }

    /// These options with a paragraph direction.
    #[must_use]
    pub const fn with_base_direction(mut self, direction: BaseDirection) -> Self {
        self.base_direction = direction;
        self
    }

    /// These options with a language.
    #[must_use]
    pub const fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    /// These options with a line height.
    #[must_use]
    pub const fn with_line_height(mut self, height: f64) -> Self {
        self.line_height = Some(height);
        self
    }

    /// These options choosing lines so.
    #[must_use]
    pub const fn with_strategy(mut self, strategy: BreakStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// These options laying out at most so many lines.
    #[must_use]
    pub const fn with_max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines);
        self
    }

    /// These options with tab stops so many spaces apart.
    #[must_use]
    pub const fn with_tab_size(mut self, spaces: f64) -> Self {
        self.tab_size = spaces;
        self
    }

    /// These options with features.
    #[must_use]
    pub const fn with_features(mut self, features: &'a [Feature]) -> Self {
        self.features = features;
        self
    }

    /// These options breaking text without spaces between words where a segmenter finds words.
    #[must_use]
    pub const fn with_segmenter(mut self, segmenter: crate::daecore::daekit::text::WordSegmenter<'a>) -> Self {
        self.segmenter = Some(segmenter);
        self
    }
}

/// A run in a line: its shaping, where it starts, and the text it covers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PositionedRun {
    /// The run as shaped, its advances widened where a line is justified. Its clusters count from `bytes.start`, as
    /// the run was shaped from there.
    pub run: ShapedRun,
    /// Where it starts from the line's start.
    pub offset: Offset,
    /// Its bidi level: odd right to left.
    pub level: u8,
    /// The text it covers, as byte offsets.
    pub bytes: Range<usize>,
    /// Whether it stands upright, as horizontal text always does, rather than on its side in a vertical line.
    pub upright: bool,
}

/// One line, in font units.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutLine {
    /// Its runs, in visual order.
    pub runs: Vec<PositionedRun>,
    /// The text it covers, as byte offsets.
    pub bytes: Range<usize>,
    /// Where its baseline lies: from the block's top edge, or in vertical text from its left edge.
    pub baseline: f64,
    /// Its extent along the line.
    pub inline_size: f64,
    /// Its ascent, up from the baseline.
    pub ascent: f64,
    /// Its descent, negative below the baseline.
    pub descent: f64,
    /// Whether a hard break ends it.
    pub hard_break: bool,
}

/// Text laid out in lines, in font units.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLayout {
    /// The lines, in order.
    pub lines: Vec<LayoutLine>,
    /// The widest line's extent.
    pub inline_size: f64,
    /// The lines' extent across, first ascent to last descent.
    pub block_size: f64,
    /// Where the text left out past [`LayoutOptions::max_lines`] begins, a byte offset.
    pub truncated: Option<usize>,
}

fn is_expandable_space(c: char) -> bool {
    general_category(c) == GeneralCategory::SpaceSeparator
}

const SOFT_HYPHEN: char = '\u{00AD}';

fn is_break_char(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

// The character starting at byte `at`, and the one ending there; none past either end or off a char boundary.
fn char_at(text: &str, at: usize) -> Option<char> {
    text.get(at..)?.chars().next()
}

fn char_before(text: &str, at: usize) -> Option<char> {
    text.get(..at)?.chars().next_back()
}

// Tab stops along a line, from its start edge: every `interval`, skipping one nearer than `min`, as CSS
// does. An interval of 0, or one whose stop isn't finite, draws tabs at no width.
#[derive(Clone, Copy, Default)]
struct Tabs {
    interval: f64,
    min: f64,
}

impl Tabs {
    fn next_stop(self, x: f64) -> f64 {
        if !(self.interval > 0.0 && self.interval.is_finite()) {
            return x;
        }
        let stop = ((x / self.interval).floor() + 1.0) * self.interval;
        let stop = if stop - x < self.min { stop + self.interval } else { stop };
        if stop.is_finite() { stop } else { x }
    }
}

// The extent of the line between `start` and `end` that holds a tab, walked from its start edge; only spaces after
// its last tab justify, so no stop moves. A breaker asks one start for ever later ends, so a walk resumes at `walked`.
fn tabbed_extent(
    text: &str,
    advances: &[f64],
    (start, end): (usize, usize),
    tabs: Tabs,
    justify: bool,
    walked: &mut Option<(usize, f64, f64)>,
) -> Extent {
    let end = ink_end(text, start, end);
    let (from, mut x, mut spaces) = walked.filter(|w| w.0 <= end).unwrap_or((start, 0.0, 0.0));
    for (rel, c) in text[from..end].char_indices() {
        let i = from + rel;
        if c == '\t' {
            x = tabs.next_stop(x);
            spaces = 0.0;
        } else {
            x += advances[i];
            if is_expandable_space(c) {
                spaces += advances[i];
            }
        }
    }
    *walked = Some((end, x, spaces));
    let (stretch, shrink) = if justify { (spaces / 2.0, spaces / SHRINK_DIVISOR) } else { (0.0, 0.0) };
    Extent { width: x, stretch, shrink }
}

// A line's tabs reach their stops from its start edge (left in LTR, else right): at the paragraph's level, walking from
// that edge meets each tab where its stop is measured. Gives the index of the line's last tab.
fn set_tabs(runs: &mut [PositionedRun], text: &str, rtl: bool, tabs: Tabs) -> Option<usize> {
    let mut glyphs: Vec<(usize, usize)> =
        runs.iter().enumerate().flat_map(|(r, run)| (0..run.run.advances.len()).map(move |i| (r, i))).collect();
    if rtl {
        glyphs.reverse();
    }
    let (mut x, mut last) = (0.0, None);
    for (r, i) in glyphs {
        let run = &mut runs[r];
        let at = run.bytes.start + run.run.clusters[i] as usize;
        if text.as_bytes().get(at) == Some(&b'\t') {
            run.run.advances[i] = tabs.next_stop(x) - x;
            last = last.max(Some(at));
        }
        x += run.run.advances[i];
    }
    last
}

fn upright_of(c: char, opts: &LayoutOptions) -> bool {
    match (opts.writing_mode.is_vertical(), opts.text_orientation) {
        (false, _) | (true, TextOrientation::Upright) => true,
        (true, TextOrientation::Sideways) => false,
        (true, TextOrientation::Mixed) => is_upright(c, vertical_form(c).is_some()),
    }
}

// What a layout's ranges shape with: the text with tabs as spaces at the same offsets, each advancing to its stop;
// `hyphen`, what a soft hyphen break shows; `scratch`, a hyphenated range, the others sliced from the text.
struct Shaping<'a> {
    fc: &'a FontCache,
    axes: &'a AxisKey,
    text: &'a str,
    scratch: RefCell<String>,
    vertical: bool,
    language: Option<&'a str>,
    hyphen: Option<char>,
    features: &'a [Feature],
}

impl Shaping<'_> {
    // The layout's features over a range, counted from its start, as it is shaped alone. One over all of it is
    // global, so its run is the same wherever in the text it lies, and the shaper need not mask its glyphs.
    fn features_over(&self, (start, end): (usize, usize)) -> Vec<Feature> {
        let over = self.features.iter().filter(|f| f.bytes.start < end && f.bytes.end > start);
        over.map(|f| match f.bytes.start <= start && f.bytes.end >= end {
            true => Feature::new(f.tag, f.value),
            false => {
                let bytes = Range { start: f.bytes.start.max(start) - start, end: f.bytes.end.min(end) - start };
                Feature { bytes, ..*f }
            }
        })
        .collect()
    }

    // Measuring and drawing both shape here, so breaking measures what it draws. The bidi level sets direction, as an
    // all-Common piece like ") " has nothing to guess from and would stay unmirrored. `hyphenated` ends in a hyphen.
    fn shape(
        &self,
        (start, end): (usize, usize),
        level: u8,
        upright: bool,
        seed_script: Option<crate::daecore::daeshaper::unicode::Script>,
        hyphenated: bool,
    ) -> Arc<ShapedRun> {
        let (text, context) = (self.text, crate::daecore::daeshaper::run::CONTEXT_CHARS);
        let before = text[..start].char_indices().rev().take(context).last().map_or(start, |(at, _)| at);
        let after = text[end..].char_indices().nth(context).map_or(text.len(), |(at, _)| end + at);
        let features = self.features_over((start, end));
        let ctx = RunContext {
            rtl: Some(level % 2 == 1),
            before: &text[before..start],
            after: &text[end..after],
            language: self.language,
            seed_script,
            features: &features,
        };
        let vertical = self.vertical && upright;
        match self.hyphen.filter(|_| hyphenated) {
            Some(hyphen) => {
                let mut buf = self.scratch.borrow_mut();
                buf.clear();
                buf.push_str(&text[start..end]);
                buf.pop();
                buf.push(hyphen);
                self.fc.shaped_run_in_context(self.axes, &buf, vertical, &ctx)
            }
            None => self.fc.shaped_run_in_context(self.axes, &text[start..end], vertical, &ctx),
        }
    }

    // Whether a run shaped at `level` puts its glyphs last character first: right to left, not top to bottom.
    fn backward(&self, level: u8, upright: bool) -> bool {
        level % 2 == 1 && !(self.vertical && upright)
    }

    // One character's advance on its own, with the features over all the text.
    fn advance(&self, c: char, upright: bool) -> f64 {
        let features = self.features_over((0, self.text.len()));
        let features: Vec<Feature> = features.into_iter().filter(|f| f.bytes.end == usize::MAX).collect();
        let ctx = RunContext { language: self.language, features: &features, ..Default::default() };
        let run = self.fc.shaped_run_in_context(self.axes, c.encode_utf8(&mut [0; 4]), self.vertical && upright, &ctx);
        run.width()
    }
}

// Where a line's ink ends, before the spaces, tabs and break characters that hang past it.
fn ink_end(text: &str, from: usize, to: usize) -> usize {
    let mut end = to;
    while end > from
        && let Some(c) = char_before(text, end)
        && (is_expandable_space(c) || c == '\t' || is_break_char(c))
    {
        end -= c.len_utf8();
    }
    end
}

// A justified space shrinks by at most a third of its width, in the breaker and when distributing.
const SHRINK_DIVISOR: f64 = 3.0;

// Each character's share of the piece's advances, whether it starts one of its glyph clusters, and whether the
// shaping marks that cluster unsafe to shape apart from the text before it, at its first byte. Gives the shaping.
fn measure_piece(sh: &Shaping, piece: &Piece, m: &mut Measured) -> Arc<ShapedRun> {
    let run = sh.shape((piece.start, piece.end), piece.level, piece.upright, Some(piece.script), false);
    let backward = sh.backward(piece.level, piece.upright);
    debug_assert!(
        run.clusters.windows(2).all(|w| if backward { w[0] >= w[1] } else { w[0] <= w[1] }),
        "slicing a piece's shaping by range assumes its clusters run in order",
    );
    for (i, &cluster) in run.clusters.iter().enumerate() {
        let at = piece.start + cluster as usize;
        if let (Some(slot), Some(&adv)) = (m.advances.get_mut(at), run.advances.get(i)) {
            *slot += adv;
        }
        if let Some(slot) = m.cluster_starts.get_mut(at) {
            *slot = true;
        }
        if run.unsafe_to_break.get(i) == Some(&true) && let Some(slot) = m.unsafe_starts.get_mut(at) {
            *slot = true;
        }
    }
    run
}

fn breakpoints(
    text: &str,
    advances: &[f64],
    ops: &[(usize, bool)],
    start: usize,
    justify: bool,
    has_tab: bool,
) -> Vec<Breakpoint> {
    let mut bps = Vec::with_capacity(ops.len() + 1);
    bps.push(Breakpoint { at: start, ..Breakpoint::start() });
    let mut prev = start;
    for &(at, forced) in ops {
        let trimmed = ink_end(text, prev, at);
        let ink: f64 = advances[prev..trimmed].iter().sum();
        let space: f64 = advances[trimmed..at].iter().sum();
        let (stretch, shrink) = if justify { (space / 2.0, space / SHRINK_DIVISOR) } else { (0.0, 0.0) };
        let tab = has_tab && text[prev..at].contains('\t');
        let hyphen = !forced && at > prev && text[prev..at].ends_with(SOFT_HYPHEN);
        let blank = !tab && advances[prev..at].iter().all(|&a| a == 0.0);
        bps.push(Breakpoint { at, ink, space, stretch, shrink, forced, tab, hyphen, blank, ..Breakpoint::start() });
        prev = at;
    }
    bps
}

// Where a line's shaping can't be cut at a break, its first glyphs are reshaped to the next cut and its last, hanging
// spaces and hyphen included, from the one before, with text out to the next opportunity; `alone` where they meet.
fn reshape_ends(sh: &Shaping, p: &Para, bps: &mut [Breakpoint]) {
    let m = p.measured;
    let sum = |from: usize, to: usize| m.advances[from..to].iter().sum::<f64>();
    let up = |at: usize, piece: &Piece| (at..piece.end).find(|&x| m.cluster_starts[x]).unwrap_or(piece.end);
    // A byte offset inside a character is never a cut, though it lies outside every piece.
    let may_cut = |at: usize| {
        sh.text.is_char_boundary(at)
            && (p.pieces.at(at).is_none_or(|q| q.start == at) || (m.cluster_starts[at] && !m.unsafe_starts[at]))
    };
    let reshape = |piece: &Piece, shaped: (usize, usize), range: (usize, usize), hyphen: bool| {
        let run = sh.shape(shaped, piece.level, piece.upright, Some(piece.script), hyphen);
        let drawn = glyphs_of(&run, shaped.0, range, range.0, sh.backward(piece.level, piece.upright));
        Reshaped { gain: drawn.width() - sum(range.0, range.1), range, shaped }
    };
    let mut opportunities = Vec::with_capacity(bps.len());
    opportunities.extend(bps.iter().filter(|b| b.cut == Cut::Opportunity).map(|b| b.at));
    let opportunity_after = |at: usize| opportunities.get(opportunities.partition_point(|&o| o <= at)).copied();
    let opportunity_before = |at: usize| opportunities[..opportunities.partition_point(|&o| o < at)].last().copied();
    for k in 0..bps.len() {
        let (bp, opportunity) = (bps[k], bps[k].cut == Cut::Opportunity);
        bps[k].lead = None;
        bps[k].tail = None;
        if let (true, false, Some(next)) = (opportunity, bp.forced, bps.get(k + 1))
            && let Some(piece) = p.pieces.at(bp.at).filter(|_| !may_cut(bp.at))
        {
            let cap = up(next.at.min(piece.end), piece);
            let to = (bp.at + 1..cap).find(|&x| may_cut(x)).unwrap_or(cap);
            let beside = opportunity_after(next.at).map_or(piece.end, |o| o.min(piece.end));
            let shaped_to = if to < cap { to } else { up(beside, piece) };
            bps[k].lead = Some(reshape(piece, (bp.at, shaped_to), (bp.at, to), false));
        }
        if k == 0 || !opportunity {
            continue;
        }
        let prev = bps[k - 1].at;
        let ink = ink_end(sh.text, prev, bp.at);
        let Some(piece) = p.pieces.at(ink.max(1) - 1).filter(|_| ink > prev) else { continue };
        let (stop, floor) = (bp.at.min(piece.end), up(prev.max(piece.start), piece));
        if !bp.hyphen && may_cut(stop) {
            continue;
        }
        let from = if bp.hyphen { floor } else { (floor..stop).rev().find(|&x| may_cut(x)).unwrap_or(floor) };
        let shaped_from = match opportunity_before(prev) {
            Some(o) if from == floor => up(o.max(piece.start), piece),
            _ => from,
        };
        if from < ink {
            bps[k].tail = Some(reshape(piece, (shaped_from, stop), (from, ink), bp.hyphen));
        }
    }
    for k in 1..bps.len() {
        bps[k].alone = bps[k - 1].lead.map(|_| {
            build_line(sh, p, &bps[k - 1], &bps[k]).iter().map(|r| r.run.width()).sum::<f64>() - bps[k].ink
        });
    }
}

// A segment no line can hold is cut as `overflow-wrap: anywhere` cuts, at a grapheme boundary starting a cluster,
// measured by the whole segment's shaping; never just before a soft hyphen, where it would break without showing it.
fn cut_wide_segments(
    bps: &[Breakpoint],
    text: &str,
    m: &Measured,
    graphemes: &[usize],
    target: f64,
) -> Vec<Breakpoint> {
    let width = |from: usize, to: usize| m.advances[from..to].iter().sum::<f64>();
    let mut out = Vec::with_capacity(bps.len());
    out.push(bps[0]);
    for pair in bps.windows(2) {
        let (prev, bp) = (pair[0], pair[1]);
        if bp.ink + prev.gain_to(&bp, true) <= target {
            out.push(bp);
            continue;
        }
        let end = ink_end(text, prev.at, bp.at);
        let (from, to) = (graphemes.partition_point(|&g| g <= prev.at), graphemes.partition_point(|&g| g < end));
        let inner = graphemes.get(from..to).unwrap_or_default();
        let tabbed = |from: usize, to: usize| bp.tab && text.get(from..to).is_some_and(|t| t.contains('\t'));
        let mut last = prev.at;
        for &at in inner.iter().filter(|&&g| m.cluster_starts[g] && char_at(text, g) != Some(SOFT_HYPHEN)) {
            let (ink, tab) = (width(last, at), tabbed(last, at));
            out.push(Breakpoint { at, ink, tab, cut: Cut::Anywhere, ..Breakpoint::start() });
            last = at;
        }
        out.push(Breakpoint { ink: width(last, end), tab: tabbed(last, bp.at), ..bp });
    }
    out
}

// The glyphs of `run`, shaped from byte `start` and `backward` or not, whose clusters lie in `from..to`, counted
// from `base`.
fn glyphs_of(run: &ShapedRun, start: usize, (from, to): (usize, usize), base: usize, backward: bool) -> ShapedRun {
    let before = |at: usize| move |&c: &u32| (start + c as usize) < at;
    let (lo, hi) = if backward {
        let after = |at: usize| move |&c: &u32| (start + c as usize) >= at;
        (run.clusters.partition_point(after(to)), run.clusters.partition_point(after(from)))
    } else {
        (run.clusters.partition_point(before(from)), run.clusters.partition_point(before(to)))
    };
    // A list the shaping wasn't asked for stays empty.
    fn pick<T: Copy>(v: &[T], (lo, hi): (usize, usize)) -> Vec<T> {
        v.get(lo..hi).map_or(Vec::new(), <[T]>::to_vec)
    }
    ShapedRun {
        glyphs: pick(&run.glyphs, (lo, hi)),
        advances: pick(&run.advances, (lo, hi)),
        offsets: pick(&run.offsets, (lo, hi)),
        unsafe_to_break: pick(&run.unsafe_to_break, (lo, hi)),
        unsafe_to_concat: pick(&run.unsafe_to_concat, (lo, hi)),
        safe_to_insert_tatweel: pick(&run.safe_to_insert_tatweel, (lo, hi)),
        clusters: pick(&run.clusters, (lo, hi)).iter().map(|&c| (start + c as usize).saturating_sub(base) as u32)
            .collect(),
        complete: run.complete,
        has_broken_syllable: run.has_broken_syllable,
        shaper: run.shaper,
    }
}

// `right` drawn after `left`, as one run.
fn join(mut left: ShapedRun, right: ShapedRun) -> ShapedRun {
    left.glyphs.extend(right.glyphs);
    left.advances.extend(right.advances);
    left.offsets.extend(right.offsets);
    left.unsafe_to_break.extend(right.unsafe_to_break);
    left.unsafe_to_concat.extend(right.unsafe_to_concat);
    left.safe_to_insert_tatweel.extend(right.safe_to_insert_tatweel);
    left.clusters.extend(right.clusters);
    left.complete &= right.complete;
    left.has_broken_syllable |= right.has_broken_syllable;
    left
}

// What the pieces' shapings say of each character, at its first byte, for `measure_piece` to fill.
struct Measured {
    advances: Vec<f64>,
    cluster_starts: Vec<bool>,
    unsafe_starts: Vec<bool>,
}

struct Para<'a> {
    // The whole text's, a level a byte, and the text they're for, tabs as they are.
    levels: &'a [u8],
    text: &'a str,
    base_level: u8,
    pieces: &'a Pieces,
    shaped: &'a [Arc<ShapedRun>],
    measured: &'a Measured,
}

// The line from `first` to `last`: each piece's part of it as the piece was shaped, but for the ends
// `reshape_ends` reshapes, and a part at another level than its piece's, past L1's reset, shaped there.
fn build_line(sh: &Shaping, p: &Para, first: &Breakpoint, last: &Breakpoint) -> Vec<PositionedRun> {
    let (from, to) = (first.at, last.at);
    let trimmed = ink_end(sh.text, from, to);
    let visual = bidi::line_visual_runs(p.base_level, p.levels, p.text, from, trimmed);
    let mut runs: Vec<PositionedRun> = Vec::new();
    for (bytes, level) in visual.iter() {
        for (piece, s, e) in p.pieces.split(bytes, level) {
            let upright = piece.is_none_or(|p| p.upright);
            let hyphen = last.hyphen && e == trimmed;
            let run = match piece {
                Some(piece) if piece.level == level => {
                    let shaped = &p.shaped[p.pieces.all.partition_point(|q| q.start < piece.start)];
                    let shape = |range, hyphen| sh.shape(range, level, upright, Some(piece.script), hyphen);
                    let (lead, tail) = (first.lead.filter(|_| s == from), last.tail.filter(|_| e == trimmed));
                    let (lo, hi) = (lead.map_or(s, |r| r.range.1), tail.map_or(e, |r| r.range.0));
                    let stop = if e == trimmed { to.min(piece.end) } else { e };
                    let back = sh.backward(level, upright);
                    if lo > hi {
                        glyphs_of(&shape((s, stop), hyphen), s, (s, e), s, back)
                    } else {
                        let lead = lead.map(|r| glyphs_of(&shape(r.shaped, false), r.shaped.0, (s, lo), s, back));
                        let tail = tail.map(|r| glyphs_of(&shape(r.shaped, hyphen), r.shaped.0, (hi, e), s, back));
                        let (left, right) = if back { (tail, lead) } else { (lead, tail) };
                        let mut run = glyphs_of(shaped, piece.start, (lo, hi), s, back);
                        if let Some(left) = left {
                            run = join(left, run);
                        }
                        if let Some(right) = right {
                            run = join(run, right);
                        }
                        run
                    }
                }
                _ => (*sh.shape((s, e), level, upright, piece.map(|p| p.script), hyphen)).clone(),
            };
            runs.push(PositionedRun { run, offset: Offset::default(), level, bytes: (s..e).into(), upright });
        }
    }
    runs
}

fn place(runs: &mut [PositionedRun], vertical: bool, offset: f64) -> f64 {
    let mut inline = offset;
    for r in runs.iter_mut() {
        r.offset = if vertical { Offset { x: 0.0, y: inline } } else { Offset { x: inline, y: 0.0 } };
        inline += r.run.width();
    }
    inline - offset
}

// Only the spaces from `from` on take the slack: those after a line's last tab, so its stops stay put.
fn distribute(runs: &mut [PositionedRun], text: &str, slack: f64, from: usize) -> bool {
    let space_width = |r: &PositionedRun, i: usize| -> f64 {
        let at = r.bytes.start + r.run.clusters.get(i).copied().unwrap_or(0) as usize;
        if at >= from && char_at(text, at).is_some_and(is_expandable_space) {
            r.run.advances.get(i).copied().unwrap_or(0.0)
        } else {
            0.0
        }
    };
    let total: f64 = runs
        .iter()
        .map(|r| (0..r.run.advances.len()).map(|i| space_width(r, i)).sum::<f64>())
        .sum();
    if total <= 0.0 {
        return false;
    }
    let scale = (slack / total).max(-1.0 / SHRINK_DIVISOR);
    for r in runs.iter_mut() {
        for i in 0..r.run.advances.len() {
            let w = space_width(r, i);
            if w > 0.0 {
                r.run.advances[i] += w * scale;
            }
        }
    }
    true
}

pub(crate) fn layout_text(
    fc: &FontCache,
    axes: &AxisKey,
    text: &str,
    opts: &LayoutOptions,
) -> TextLayout {
    let vertical = opts.writing_mode.is_vertical();
    let metrics = fc.described_at(axes, |at| at.line_metrics(vertical));
    let line_height = opts.line_height.unwrap_or_else(|| metrics.line_height());
    let justify = opts.align == Align::Justify;

    let mut out =
        TextLayout { lines: Vec::new(), inline_size: 0.0, block_size: 0.0, truncated: None };
    if text.is_empty() {
        return out;
    }
    // A measure below 0 wraps as 0 does, and one that isn't a number not at all, as infinity.
    let measure = if opts.max_inline_size.is_nan() { f64::INFINITY } else { opts.max_inline_size.max(0.0) };
    let wrapping = measure.is_finite();
    // Each line's direction, whether it ends its paragraph and where its justifiable spaces start, for
    // aligning once the widest is known.
    let mut unaligned: Vec<(bool, bool, usize)> = Vec::new();
    let has_tab = text.contains('\t');
    let as_spaces: String;
    let shaped = if has_tab {
        as_spaces = text.replace('\t', " ");
        &as_spaces
    } else {
        text
    };
    let hyphen = ['\u{2010}', '-'].into_iter().find(|&c| fc.glyph_id(u32::from(c)).is_some());
    let scratch = RefCell::default();
    let language = opts.language;
    let sh = Shaping { fc, axes, text: shaped, scratch, vertical, language, hyphen, features: opts.features };
    // CSS's: the stops are tab_size spaces apart, and a stop nearer than half a '0' (0.5em without one) is
    // skipped. Measured in the tab's own orientation.
    let tabs = has_tab.then(|| {
        let upright = upright_of('\t', opts);
        let em = f64::from(fc.font_upm());
        let zero = if fc.glyph_id(u32::from('0')).is_some() { sh.advance('0', upright) } else { em / 2.0 };
        Tabs { interval: opts.tab_size * sh.advance(' ', upright), min: zero / 2.0 }
    });

    let segmenter = opts.segmenter.map(|s| s.0);
    let opportunities = crate::daecore::daeshaper::unicode::linebreak::line_break_opportunities(text, segmenter);
    let n = text.len();
    // CSS Writing Modes 4 lays upright text out left to right, every character strong LTR. Else bidi paragraphs end
    // only at a paragraph separator, so a line separator or form feed keeps the direction.
    let bidi_text = if vertical && opts.text_orientation == TextOrientation::Upright {
        bidi::Paragraph {
            base_level: 0,
            levels: alloc::vec![0; n],
            visual_order: text.char_indices().map(|(at, _)| at).collect(),
            paragraphs: alloc::vec![(0, n, 0)],
        }
    } else {
        bidi::resolve(text, opts.base_direction.rtl())
    };
    let mut measured = Measured {
        advances: alloc::vec![0.0; n],
        cluster_starts: alloc::vec![false; n],
        unsafe_starts: alloc::vec![false; n],
    };
    let graphemes =
        if wrapping { crate::daecore::daeshaper::unicode::segment::grapheme_boundaries(text) } else { Vec::new() };
    let mut baseline = metrics.ascent;
    let mut para_start = 0usize;
    let mut op_cursor = 0usize;
    debug_assert!(
        opportunities.windows(2).all(|w| w[0].at <= w[1].at),
        "the cursor below assumes break opportunities arrive in order",
    );

    'paragraphs: for para_end in opportunities.iter().filter(|b| b.mandatory).map(|b| b.at) {
        if para_start >= para_end {
            continue;
        }
        let mut content_end = para_end;
        while content_end > para_start
            && let Some(c) = char_before(text, content_end).filter(|&c| is_break_char(c))
        {
            content_end -= c.len_utf8();
        }

        let base_level = bidi_text.paragraph_level(para_start);
        let pieces = Pieces::new(text, &bidi_text.levels, (para_start, content_end), |c| upright_of(c, opts));
        let shaped: Vec<Arc<ShapedRun>> = pieces.all.iter().map(|p| measure_piece(&sh, p, &mut measured)).collect();

        while op_cursor < opportunities.len() && opportunities[op_cursor].at <= para_start {
            op_cursor += 1;
        }
        let ops_from = op_cursor;
        while op_cursor < opportunities.len() && opportunities[op_cursor].at <= para_end {
            op_cursor += 1;
        }
        // A glyph cluster is one unit, so a break between graphemes the font made one cluster is no break. One
        // inside a grapheme, as UAX #14 puts before a mark following a space, stays.
        let fused = |at: usize| {
            pieces.at(at).is_some_and(|p| p.start < at)
                && !measured.cluster_starts[at]
                && graphemes.binary_search(&at).is_ok()
        };
        let ops: Vec<(usize, bool)> = opportunities[ops_from..op_cursor]
            .iter()
            .filter(|b| b.mandatory || !fused(b.at))
            .map(|b| (b.at, b.mandatory))
            .collect();
        let ctx = Para {
            levels: &bidi_text.levels,
            text,
            base_level,
            pieces: &pieces,
            shaped: &shaped,
            measured: &measured,
        };
        let mut bps = breakpoints(text, &measured.advances, &ops, para_start, justify, has_tab);
        reshape_ends(&sh, &ctx, &mut bps);
        if wrapping {
            let cut = cut_wide_segments(&bps, text, &measured, &graphemes, measure);
            if cut.len() > bps.len() {
                bps = cut;
                reshape_ends(&sh, &ctx, &mut bps);
            }
        }

        let fit = Fit {
            target: measure,
            line_end_stretch: if justify { 0.0 } else { measure * 0.25 },
        };

        let walks = RefCell::new(Vec::new());
        let tabbed = |from: usize, to: usize| {
            let mut walks = walks.borrow_mut();
            if walks.is_empty() {
                walks.resize(bps.len(), None);
            }
            let (span, tabs) = ((bps[from].at, bps[to].at), tabs.unwrap_or_default());
            tabbed_extent(text, &measured.advances, span, tabs, justify, &mut walks[from])
        };
        for line in break_lines(&bps, &fit, opts.strategy, tabbed) {
            if opts.max_lines.is_some_and(|n| out.lines.len() >= n) {
                out.truncated = Some(bps[line.from].at);
                break 'paragraphs;
            }
            let range = bps[line.from].at..bps[line.to].at;
            let mut runs = build_line(&sh, &ctx, &bps[line.from], &bps[line.to]);
            let (rtl, last) = (base_level % 2 == 1, bps[line.to].forced);
            let justify_from = tabs.and_then(|tabs| set_tabs(&mut runs, text, rtl, tabs)).map_or(0, |t| t + 1);
            let natural = place(&mut runs, vertical, 0.0);
            let line_end = (rtl, last, justify_from);
            let inline_size = if wrapping {
                align_line(&mut runs, text, natural, line_end, measure, opts.align, vertical)
            } else {
                unaligned.push(line_end);
                natural
            };

            out.inline_size = out.inline_size.max(inline_size);
            out.lines.push(LayoutLine {
                runs,
                bytes: range.into(),
                baseline,
                inline_size,
                ascent: metrics.ascent,
                descent: metrics.descent,
                hard_break: bps[line.to].forced,
            });
            baseline += line_height;
        }

        para_start = para_end;
    }

    // An infinite measure has no edge to align to, so every line aligns against the widest.
    for (line, &line_end) in out.lines.iter_mut().zip(&unaligned) {
        let (natural, measure) = (line.inline_size, out.inline_size);
        line.inline_size = align_line(&mut line.runs, text, natural, line_end, measure, opts.align, vertical);
    }
    finish(out, opts.writing_mode)
}

// A line in its measure: justified unless it ends its paragraph, else offset by the alignment, Start and End following
// its direction; `line_end` holds the direction, whether the line ends it and where its spaces start. Gives its size.
fn align_line(
    runs: &mut [PositionedRun],
    text: &str,
    natural: f64,
    (rtl, last, justify_from): (bool, bool, usize),
    measure: f64,
    align: Align,
    vertical: bool,
) -> f64 {
    let slack = measure - natural;
    if align == Align::Justify && !last && slack != 0.0 && distribute(runs, text, slack, justify_from) {
        return place(runs, vertical, 0.0);
    }
    let offset = match align {
        Align::Center => slack / 2.0,
        Align::End => if rtl { 0.0 } else { slack },
        Align::Start | Align::Justify => if rtl { slack } else { 0.0 },
    };
    place(runs, vertical, offset);
    natural
}

// Vertical baselines become physical, from the block's left edge. A line's ascent faces right in both modes, so
// vertical-lr starts a descent in from the left and grows, vertical-rl an ascent in from the right and shrinks.
fn finish(mut out: TextLayout, mode: WritingMode) -> TextLayout {
    let (Some(first), Some(last)) = (out.lines.first(), out.lines.last()) else {
        return out;
    };
    let block = last.baseline - first.baseline + first.ascent - last.descent;
    out.block_size = block;
    for line in &mut out.lines {
        line.baseline = match mode {
            WritingMode::Horizontal => line.baseline,
            WritingMode::VerticalLr => line.baseline - line.ascent - line.descent,
            WritingMode::VerticalRl => block - line.baseline,
        };
    }
    out
}
