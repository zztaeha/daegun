//! Unicode text analysis, which needs no font: bidi, segmentation, line breaking, scripts and categories.

pub use crate::daecore::daeshaper::unicode::bidi::BaseDirection;
pub use crate::text::{
    general_category, grapheme_boundaries, line_break_opportunities, script, script_runs, vertical_form,
    vertical_orientation, word_boundaries, BidiParagraph, GeneralCategory, LineBreak, ParagraphRun, Script,
    ScriptDirection, ScriptRun, VerticalOrientation, VisualRun, WordSegmenter,
};
