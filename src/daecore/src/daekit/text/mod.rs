mod bidi;
pub use bidi::{BidiParagraph, ParagraphRun, VisualRun};

pub(crate) mod justify;
pub(crate) mod layout;
pub(crate) mod linebreak;
pub(crate) mod pieces;
pub mod shape;

/// The text's runs of one script (UAX #24), a Common or Inherited character joining the run around it.
///
/// ```
/// use daegun::unicode::{script_runs, Script};
///
/// let text = "abc αβγ";
/// let runs: Vec<_> = script_runs(text).iter().map(|r| (&text[r.bytes], r.script)).collect();
/// assert_eq!(runs, [("abc ", Script::LATIN), ("αβγ", Script::GREEK)]);
/// ```
pub fn script_runs(text: &str) -> alloc::vec::Vec<ScriptRun> {
    crate::daecore::daeshaper::unicode::itemize::script_runs(text)
}

pub use crate::daecore::daeshaper::unicode::itemize::ScriptRun;
pub use crate::daecore::daeshaper::unicode::Script;
pub use crate::daecore::daeshaper::unicode::{general_category, script, GeneralCategory};
pub use crate::daecore::daeshaper::unicode::{vertical_orientation, ScriptDirection, VerticalOrientation};
pub use crate::daecore::daeshaper::unicode::vertical_form;

/// Where the text's grapheme clusters (UAX #29) begin, as byte offsets, with its start and end.
///
/// ```
/// use daegun::unicode::grapheme_boundaries;
///
/// // An `e` with a combining acute is one cluster, and a flag's two regional indicators another.
/// assert_eq!(grapheme_boundaries("e\u{301}🇰🇷"), [0, 3, 11]);
/// ```
pub fn grapheme_boundaries(text: &str) -> alloc::vec::Vec<usize> {
    crate::daecore::daeshaper::unicode::segment::grapheme_boundaries(text)
}

/// The text's word boundaries (UAX #29), as byte offsets, with its start and end.
///
/// ```
/// use daegun::unicode::word_boundaries;
///
/// assert_eq!(word_boundaries("Hello, world"), [0, 5, 6, 7, 12]);
/// ```
pub fn word_boundaries(text: &str) -> alloc::vec::Vec<usize> {
    crate::daecore::daeshaper::unicode::segment::word_boundaries(text)
}

/// A place a line may break (UAX #14).
#[non_exhaustive]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LineBreak {
    /// Where the next line starts, a byte offset into the text.
    pub at: usize,
    /// Whether the text demands the break rather than allows it, as after a newline and at the end.
    pub mandatory: bool,
}

/// A caller's word segmentation for text written without spaces between words, UAX #14's class SA: Thai, Lao,
/// Khmer, Myanmar and others. It's `Sync`, so options holding one can be shared between threads, and daegun calls it
/// on the thread breaking the text; one keeping state between calls keeps it in a `Mutex` or an atomic, not a `Cell`.
///
/// ```
/// use daegun::unicode::{line_break_opportunities, WordSegmenter};
///
/// // Thai, with no spaces between its words, split after the first by the caller's own segmenter.
/// let text = "สวัสดีครับ";
/// let first = "สวัสดี".len();
/// let split = |_run: &str, out: &mut Vec<usize>| out.push(first);
/// let breaks: Vec<usize> = line_break_opportunities(text, Some(WordSegmenter(&split))).iter().map(|b| b.at).collect();
/// assert_eq!(breaks, [first, text.len()]);
/// ```
#[expect(clippy::exhaustive_structs, reason = "a newtype over the caller's callback")]
#[derive(Clone, Copy)]
pub struct WordSegmenter<'a>(
    /// Given one such run, it pushes the byte offsets from the run's start of the word boundaries inside it; one
    /// that isn't a char boundary is ignored.
    pub crate::daecore::daeshaper::unicode::linebreak::Segmenter<'a>,
);

impl core::fmt::Debug for WordSegmenter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WordSegmenter")
    }
}

/// Where lines may break in the text, the end included. SA text breaks between words where `segmenter` finds them,
/// or with none, where the `dictionaries` feature's word lists do, and without them not at all.
///
/// ```
/// use daegun::unicode::line_break_opportunities;
///
/// let breaks: Vec<_> = line_break_opportunities("one two\nthree", None).iter().map(|b| (b.at, b.mandatory)).collect();
/// assert_eq!(breaks, [(4, false), (8, true), (13, true)]);
/// ```
#[doc(alias("line_break_opportunities_with"))]
pub fn line_break_opportunities(text: &str, segmenter: Option<WordSegmenter>) -> alloc::vec::Vec<LineBreak> {
    crate::daecore::daeshaper::unicode::linebreak::line_break_opportunities(text, segmenter.map(|s| s.0))
        .into_iter()
        .map(|b| LineBreak { at: b.at, mandatory: b.mandatory })
        .collect()
}
