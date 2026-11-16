use crate::daecore::daeshaper::unicode::bidi::BaseDirection;

/// Text resolved for bidi (UAX #9): each character's level and the visual order, each paragraph reordered as a line
/// of its own. Positions are byte offsets into the text, as everywhere in daegun.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct BidiParagraph {
    /// The first paragraph's level: 0 left to right, 1 right to left.
    pub base_level: u8,
    /// A level for each byte, every byte of a character at its level, so any position indexes it; odd is right to
    /// left.
    pub levels: alloc::vec::Vec<u8>,
    /// Each character's start, in visual order, left to right.
    pub visual_order: alloc::vec::Vec<usize>,
    /// The paragraphs, in order.
    pub paragraphs: alloc::vec::Vec<ParagraphRun>,
}

/// A paragraph: its text, ending with the separator that ends it, and its level.
// `start` and `end` rather than a `Range`, whose layout isn't fixed, as C reads the list in place.
#[non_exhaustive]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ParagraphRun {
    /// Where it starts, a byte offset into the text.
    pub start: usize,
    /// Where it ends, the byte offset past its separator.
    pub end: usize,
    /// Its level: 0 left to right, 1 right to left.
    pub level: u8,
}

/// A run of one level on a line, in visual order.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VisualRun {
    /// The text it covers, as byte offsets, which an odd level draws last to first.
    pub bytes: core::range::Range<usize>,
    /// Its level, odd for right to left.
    pub level: u8,
}

impl BidiParagraph {
    /// The text resolved for bidi, each paragraph's direction set by `base`, or with `Auto` by its first strong
    /// character.
    ///
    /// ```
    /// use daegun::unicode::{BaseDirection, BidiParagraph};
    ///
    /// let text = "abc שלום";
    /// let para = BidiParagraph::new(text, BaseDirection::Auto);
    /// assert_eq!(para.base_level, 0, "the first strong character is Latin");
    /// let runs = para.visual_runs(text, ..);
    /// assert_eq!((&text[runs[0].bytes], runs[0].level), ("abc ", 0));
    /// assert_eq!((&text[runs[1].bytes], runs[1].level), ("שלום", 1));
    /// ```
    #[doc(alias("resolve_bidi"))]
    pub fn new(text: &str, base: BaseDirection) -> BidiParagraph {
        let p = crate::daecore::daeshaper::unicode::bidi::resolve(text, base.rtl());
        let paragraphs = p.paragraphs.iter().map(|&(start, end, level)| ParagraphRun { start, end, level }).collect();
        BidiParagraph { base_level: p.base_level, levels: p.levels, visual_order: p.visual_order, paragraphs }
    }

    /// A line's runs left to right: the bytes `line` covers of `text`, which this resolved, reordered by the levels
    /// of the paragraph holding the line's start. No runs for an empty or reversed range, or one that doesn't fall
    /// on char boundaries.
    #[doc(alias("line_visual_runs"))]
    pub fn visual_runs(&self, text: &str, line: impl core::ops::RangeBounds<usize>) -> alloc::vec::Vec<VisualRun> {
        use core::ops::Bound::{Excluded, Included, Unbounded};
        let start = match line.start_bound() {
            Included(&at) => at,
            Excluded(&at) => at.saturating_add(1),
            Unbounded => 0,
        };
        let end = match line.end_bound() {
            Included(&at) => at.saturating_add(1),
            Excluded(&at) => at,
            Unbounded => text.len(),
        };
        let k = self.paragraphs.partition_point(|p| p.end <= start);
        let base_level = self.paragraphs.get(k).map_or(self.base_level, |p| p.level);
        let runs =
            crate::daecore::daeshaper::unicode::bidi::line_visual_runs(base_level, &self.levels, text, start, end);
        runs.iter().map(|(bytes, level)| VisualRun { bytes: bytes.into(), level }).collect()
    }
}
