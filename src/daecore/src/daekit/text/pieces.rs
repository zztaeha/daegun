use alloc::vec::Vec;
use core::ops::Range;

use crate::daecore::daeshaper::unicode::itemize::script_runs;
use crate::daecore::daeshaper::unicode::Script;

// A stretch of text shaped as one run: one script, one bidi level and one orientation, in bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Piece {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) upright: bool,
    pub(crate) level: u8,
    pub(crate) script: Script,
}

// Text cut into pieces, in logical order, wherever the script, the bidi level or the orientation changes.
pub(crate) struct Pieces {
    pub(crate) all: Vec<Piece>,
}

impl Pieces {
    // Bytes `start..end` of `text` cut, `levels` giving a level for each byte of the text.
    pub(crate) fn new(
        text: &str, levels: &[u8], (start, end): (usize, usize), upright: impl Fn(char) -> bool,
    ) -> Pieces {
        let mut all: Vec<Piece> = Vec::new();
        let cut = &text[start..end.max(start)];
        for sr in script_runs(cut) {
            for (rel, c) in cut[sr.bytes].char_indices() {
                let at = start + sr.bytes.start + rel;
                let (level, up, next) = (levels.get(at).copied().unwrap_or(0), upright(c), at + c.len_utf8());
                match all.last_mut() {
                    Some(p) if (p.script, p.level, p.upright) == (sr.script, level, up) => p.end = next,
                    _ => all.push(Piece { start: at, end: next, upright: up, level, script: sr.script }),
                }
            }
        }
        Pieces { all }
    }

    pub(crate) fn at(&self, at: usize) -> Option<&Piece> {
        self.all.get(self.all.partition_point(|p| p.end <= at)).filter(|p| p.start <= at)
    }

    // One visual run's text split where the piece changes, as (piece, start, end) in visual order, text in no piece a
    // part of its own. Right to left they come out last first, or a Latin space lands on an Arabic word's wrong side.
    pub(crate) fn split(&self, run: Range<usize>, level: u8) -> Vec<(Option<&Piece>, usize, usize)> {
        let mut parts = Vec::new();
        let (mut at, mut next) = (run.start, self.all.partition_point(|p| p.end <= run.start));
        while at < run.end {
            let end = match self.all.get(next) {
                Some(p) if p.start <= at => {
                    next += 1;
                    parts.push((Some(p), at, p.end.min(run.end)));
                    p.end.min(run.end)
                }
                piece => {
                    let end = piece.map_or(run.end, |p| p.start.min(run.end));
                    parts.push((None, at, end));
                    end
                }
            };
            at = end;
        }
        if level % 2 == 1 {
            parts.reverse();
        }
        parts
    }
}
