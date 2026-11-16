// What placed glyphs still owe the caller: no morx deletion marks, the stand-in for a variation selector the font
// couldn't apply, and default ignorables drawn invisible, kept or dropped as asked.
use crate::daecore::daekit::text::shape::Ignorables;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::morx;
use crate::daecore::daeshaper::plan::{Plan, Substitution};
use crate::daecore::daeshaper::run::{text, Fact, Glyph, Placement, Run};

// Where GPOS ran, they left before it.
pub(super) fn drop_deletions(plan: &Plan, run: &mut Run) {
    if plan.substitution == Substitution::Morx && !plan.positioning.gpos {
        morx::remove_deleted_glyphs(run);
    }
}

pub(super) fn stand_in_for_selectors(run: &mut Run) {
    let Some(stand_in) = run.settings.missing_selector else { return };
    if !run.facts.has(Fact::SelectorFallback) {
        return;
    }
    let pairs = run.glyphs[..run.len].iter_mut().zip(&mut run.places[..run.len]);
    for (glyph, place) in pairs.filter(|(glyph, _)| glyph.char_is(text::SELECTOR)) {
        glyph.id = stand_in;
        glyph.set_selector_fallback(false);
        *place = Placement::default();
    }
}

// Hidden ones take the invisible glyph, the font's space by default; without one they go, as removed ones do.
pub(super) fn settle_ignorables(face: &Face, run: &mut Run) {
    if !run.facts.has(Fact::Ignorables) {
        return;
    }
    let invisible = match run.settings.ignorables {
        Ignorables::Preserve => return,
        Ignorables::Hide => run.settings.invisible.or_else(|| face.glyph_of(0x20)),
        Ignorables::Remove => None,
    };
    match invisible {
        Some(invisible) => {
            let glyphs = run.glyphs[..run.len].iter_mut().filter(|g| g.is_ignorable());
            glyphs.for_each(|g| g.id = invisible);
        }
        None => run.delete_where(Glyph::is_ignorable),
    }
}
