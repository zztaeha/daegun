// Placing the glyphs: the font's advances, then whichever tables the plan runs, then what nothing in the font did.
use crate::daecore::daekit::cache::Advances;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::fallback;
use crate::daecore::daeshaper::ot::digest::Digest;
use crate::daecore::daeshaper::ot::{gpos, kern, kerx};
use crate::daecore::daeshaper::plan::{Plan, Table};
use crate::daecore::daeshaper::run::{class, text, Fact, Run};
use crate::daecore::daeshaper::script::MarkAdvances;
use crate::daecore::daekit::text::shape::Ignorables;

pub(super) fn position(face: &Face, plan: &Plan, run: &mut Run, digest: Option<Digest>) {
    let p = plan.positioning;
    run.start_positions();
    font_advances(face, run);
    if run.facts.has(Fact::SpaceStandIns) {
        fallback::adjust_spaces(face, run);
    }
    // A backward run lays a mark's advance out after it, so there's nothing to hang back over.
    let hang_back = p.marks_hang_back && !run.direction.is_backward();
    if p.zero_marks == Some(MarkAdvances::ZeroBeforeGpos) {
        zero_marks(run, hang_back);
    }
    if p.gpos {
        // No glyph is written between GSUB and here, so its digest holds every one.
        let digest = digest.unwrap_or_else(|| Digest::of(&run.glyphs[..run.len]));
        super::layout::apply_table(face, plan, run, Table::Gpos, digest);
    } else if p.kerx {
        kerx::apply(face, run, plan.masks.kern, &plan.aat.kerx_sets);
    }
    if p.kern {
        kern::apply(face, run, plan.masks.kern, &plan.aat.kern_sets);
    }
    if p.trak {
        track(run, face.tracking(run.direction.is_horizontal()));
    }
    if p.zero_marks == Some(MarkAdvances::ZeroAfterGpos) {
        zero_marks(run, hang_back);
    }
    zero_ignorables(run);
    gpos::resolve_attachments(run);
    if p.fallback_marks {
        fallback::position_marks(plan, face, run, hang_back);
    }
    if run.direction.is_backward() {
        run.reverse();
    }
}

// One loop for each source of advances, so a table's, read in place, stays as lean.
fn font_advances(face: &Face, run: &mut Run) {
    match face.advances(run.direction.is_vertical()) {
        Advances::Table(table) => place(face, run, |glyph| table.of(glyph)),
        Advances::Varied(varied) => place(face, run, |glyph| varied.of(glyph)),
        Advances::Listed(listed) => place(face, run, |glyph| listed.of(glyph)),
    }
}

// Along a horizontal run, the advance; down a vertical one, the advance negated, the glyph centered across and
// dropped by its vertical origin.
fn place(face: &Face, run: &mut Run, advance: impl Fn(u32) -> u32) {
    let pairs = run.glyphs[..run.len].iter().zip(&mut run.places[..run.len]);
    if run.direction.is_horizontal() {
        pairs.for_each(|(glyph, place)| place.advance_x = advance(glyph.id) as i32);
        return;
    }
    let fallback = face.line_height();
    let widths = face.advances(false);
    face.with_origins(|origins| {
        for (glyph, place) in pairs {
            place.advance_y = -face.height_or(glyph.id, advance(glyph.id), fallback);
            place.offset_x -= widths.of(glyph.id) as i32 / 2;
            place.offset_y -= origins.of(glyph.id);
        }
    });
}

fn zero_marks(run: &mut Run, hang_back: bool) {
    let pairs = run.glyphs[..run.len].iter().zip(&mut run.places[..run.len]);
    pairs.filter(|(glyph, _)| glyph.glyph_is(class::MARK)).for_each(|(_, place)| place.zero_advance(hang_back));
}

// Each grapheme's first glyph advances `adjust` further along the run, saturating. Glyphs that continue a grapheme
// and open the run share a cluster with its first only where reordering put them first, as a pre-base matra.
pub(super) fn track(run: &mut Run, adjust: i32) {
    if adjust == 0 {
        return;
    }
    let horizontal = run.direction.is_horizontal();
    let glyphs = &run.glyphs[..run.len];
    let opening = glyphs.first().map(|g| g.cluster);
    let first = glyphs.iter().take_while(|g| Some(g.cluster) == opening).position(|g| !g.char_is(text::CONTINUES));
    let mut start = first.unwrap_or(0);
    while start < run.len {
        let (advance, _) = run.places[start].along_mut(horizontal);
        *advance = advance.saturating_add(adjust);
        start = run.grapheme_end(start);
    }
}

// Default ignorables to be drawn invisible take no room: no advance and no offset along the run. Across it, a vertical
// origin's shift stays, as does the attachment resolved after this.
fn zero_ignorables(run: &mut Run) {
    if !run.facts.has(Fact::Ignorables) || run.settings.ignorables != Ignorables::Hide {
        return;
    }
    let horizontal = run.direction.is_horizontal();
    let pairs = run.glyphs[..run.len].iter().zip(&mut run.places[..run.len]);
    for (_, place) in pairs.filter(|(glyph, _)| glyph.is_ignorable()) {
        (place.advance_x, place.advance_y) = (0, 0);
        *place.along_mut(horizontal).1 = 0;
    }
}
