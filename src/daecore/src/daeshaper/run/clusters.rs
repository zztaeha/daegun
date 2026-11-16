// Clusters: an edit that turns several characters' glyphs into one unit gives them one cluster, the least, where the
// cluster level keeps clusters whole; at other levels it only marks them unsafe to break.
use core::ops::Range;

use super::{flag, text, Glyph, Mask, Run};

// A glyph taking another cluster takes the output flags of what it took it from.
fn move_to_cluster(glyph: &mut Glyph, cluster: u32, flags: Mask) {
    if glyph.cluster != cluster {
        glyph.mask = (glyph.mask & !flag::ALL) | (flags & flag::ALL);
        glyph.cluster = cluster;
    }
}

fn least_cluster(glyphs: &[Glyph]) -> u32 {
    glyphs.iter().map(|g| g.cluster).min().unwrap_or(u32::MAX)
}

// A dropped glyph's cluster, lower than the last cluster of the glyphs `kept` before it, goes to that cluster's glyphs.
pub(super) fn hand_down(kept: &mut [Glyph], cluster: u32, flags: Mask) {
    let Some(last) = kept.last().map(|g| g.cluster) else { return };
    if cluster < last {
        let tail = kept.iter().rev().take_while(|g| g.cluster == last).count();
        let from = kept.len() - tail;
        kept[from..].iter_mut().for_each(|g| move_to_cluster(g, cluster, flags));
    }
}

impl Run {
    pub(crate) fn join_clusters(&mut self, span: Range<usize>) {
        self.join_if(span, self.settings.cluster_level.is_monotone());
    }

    // A grapheme's characters, which the grapheme levels keep in one cluster.
    pub(crate) fn join_graphemes(&mut self, span: Range<usize>) {
        self.join_if(span, self.settings.cluster_level.is_graphemes());
    }

    fn join_if(&mut self, span: Range<usize>, joins: bool) {
        if span.len() < 2 {
            return;
        }
        if joins {
            self.unite(span);
        } else {
            self.unsafe_to_break(span);
        }
    }

    // The span widened over the whole clusters at its ends, all given the least; where it starts at the cursor of a
    // rewrite, the written glyphs of its first cluster join too.
    fn unite(&mut self, span: Range<usize>) {
        self.spend(span.len());
        let least = least_cluster(&self.glyphs[span.clone()]);
        let head = self.glyphs[span.start].cluster;
        let tail = self.glyphs[span.end - 1].cluster;
        let mut end = span.end;
        if tail != least {
            end += self.glyphs[end..self.len].iter().take_while(|g| g.cluster == tail).count();
        }
        let mut start = span.start;
        if head != least {
            start -= self.glyphs[self.cursor.min(start)..start].iter().rev().take_while(|g| g.cluster == head).count();
        }
        if start == self.cursor && self.written > 0 && head != least {
            let written = self.written;
            let out = &mut self.out_mut()[..written];
            out.iter_mut().rev().take_while(|g| g.cluster == head).for_each(|g| move_to_cluster(g, least, 0));
        }
        self.glyphs[start..end].iter_mut().for_each(|g| move_to_cluster(g, least, 0));
    }

    pub(crate) fn join_out_clusters(&mut self, span: Range<usize>) {
        self.join_out_if(span, self.settings.cluster_level.is_monotone());
    }

    pub(crate) fn join_out_graphemes(&mut self, span: Range<usize>) {
        self.join_out_if(span, self.settings.cluster_level.is_graphemes());
    }

    // As `join_if` over written glyphs, the span running to the last of them.
    fn join_out_if(&mut self, span: Range<usize>, joins: bool) {
        debug_assert_eq!(span.end, self.written);
        if span.len() < 2 {
            return;
        }
        if joins {
            self.unite_out(span);
        } else {
            self.unsafe_to_break_across(span.start, self.cursor);
        }
    }

    // As `unite` over written glyphs; where the span reaches the last written glyph, the unread glyphs of its cluster
    // join too.
    fn unite_out(&mut self, span: Range<usize>) {
        self.spend(span.len());
        let written = self.written;
        let out = &self.out()[..written];
        let least = least_cluster(&out[span.clone()]);
        let (head, tail) = (out[span.start].cluster, out[span.end - 1].cluster);
        let mut start = span.start;
        if head != least {
            start -= out[..start].iter().rev().take_while(|g| g.cluster == head).count();
        }
        let mut end = span.end;
        if tail != least {
            end += out[end..].iter().take_while(|g| g.cluster == tail).count();
        }
        let last = out[end - 1].cluster;
        if end == written {
            let unread = &mut self.glyphs[self.cursor..self.len];
            unread.iter_mut().take_while(|g| g.cluster == last).for_each(|g| move_to_cluster(g, least, 0));
        }
        self.out_mut()[start..end].iter_mut().for_each(|g| move_to_cluster(g, least, 0));
    }

    // Drops every glyph `gone` picks, in place, its cluster handed on as `delete_current` hands it.
    pub(crate) fn delete_where(&mut self, gone: impl Fn(&Glyph) -> bool) {
        let mut kept = 0;
        for i in 0..self.len {
            let glyph = self.glyphs[i];
            if !gone(&glyph) {
                self.glyphs[kept] = glyph;
                if self.positioned {
                    self.places[kept] = self.places[i];
                }
                kept += 1;
                continue;
            }
            let has_next = i + 1 < self.len;
            if has_next && self.glyphs[i + 1].cluster == glyph.cluster {
                continue;
            }
            if kept > 0 {
                hand_down(&mut self.glyphs[..kept], glyph.cluster, glyph.mask);
            } else if has_next {
                self.join_clusters(i..i + 2);
            }
        }
        self.len = kept;
    }

    // A stable sort of the span by `key`, each glyph that moves joined in cluster with those it passes.
    pub(crate) fn sort_span_by(&mut self, span: Range<usize>, key: impl Fn(&Glyph) -> u8) {
        debug_assert!(!self.positioned);
        for i in span.start + 1..span.end {
            let k = key(&self.glyphs[i]);
            let passed = self.glyphs[span.start..i].iter().rev().take_while(|g| key(g) > k).count();
            if passed > 0 {
                let to = i - passed;
                self.join_clusters(to..i + 1);
                self.glyphs[to..=i].rotate_right(1);
            }
        }
    }

    pub(crate) fn reverse(&mut self) {
        self.reverse_span(0..self.len);
    }

    pub(crate) fn reverse_span(&mut self, span: Range<usize>) {
        if span.len() < 2 {
            return;
        }
        self.glyphs[span.clone()].reverse();
        if self.positioned {
            self.places[span].reverse();
        }
    }

    // Where the grapheme starting at `start` ends: past the glyphs that continue it.
    pub(crate) fn grapheme_end(&self, start: usize) -> usize {
        start + 1 + self.glyphs[start + 1..self.len].iter().take_while(|g| g.char_is(text::CONTINUES)).count()
    }
}
