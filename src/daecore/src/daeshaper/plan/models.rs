// Which tables shape a run and what becomes of its marks: one table of decisions, read once a plan.
use super::{Features, Table};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::ot::{kern, kerx};
use crate::daecore::daeshaper::script::{MarkAdvances, Model};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Substitution {
    Gsub,
    Morx,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Positioning {
    pub(crate) gpos: bool,
    // Only where GPOS doesn't run.
    pub(crate) kerx: bool,
    pub(crate) kern: bool,
    pub(crate) trak: bool,
    pub(crate) zero_marks: Option<MarkAdvances>,
    // A zeroed mark's offset moves back by its advance, so it hangs over its base.
    pub(crate) marks_hang_back: bool,
    // Marks placed by their Unicode class, where nothing in the font places them.
    pub(crate) fallback_marks: bool,
}

// What the font has and the run asks, for `positioning`.
pub(super) struct Inputs<'p> {
    pub(super) face: &'p Face<'p>,
    pub(super) features: &'p Features,
    pub(super) horizontal: bool,
    pub(super) has_gsub: bool,
    pub(super) has_gpos: bool,
    pub(super) substitution: Substitution,
    // The model whose hooks run, and the script's own, which a morx font's model stands in for.
    pub(super) model: &'static dyn Model,
    pub(super) script_model: &'static dyn Model,
}

// kerx outranks GPOS unless the font substitutes through GSUB and has GPOS. GPOS without kern leaves kerning to kerx,
// else to kern where the script's model takes fallbacks: kern would set Devanagari's two full stops apart.
pub(super) fn positioning(inputs: &Inputs) -> Positioning {
    let Inputs { face, features, horizontal, has_gsub, has_gpos, substitution, model, script_model } = *inputs;
    let kern_tag = Tag::new(if horizontal { b"kern" } else { b"vkrn" });
    let gpos_kerns = features.index(Table::Gpos, kern_tag).is_some();
    // A model may need GPOS filed under its own script, as Hebrew's marks do.
    let gpos_usable = has_gpos && model.gpos_script().is_none_or(|t| features.script(Table::Gpos).tag == Some(t));
    let through_gsub = substitution == Substitution::Gsub && has_gsub;
    let has_kerx = kerx::has_data(face);

    let mut kerx_kerns = has_kerx && !(through_gsub && gpos_usable);
    let gpos = gpos_usable && !kerx_kerns;
    let mut kern = false;
    if !kerx_kerns && !(gpos && gpos_kerns) {
        if has_kerx {
            // A font that kerns through kerx keeps its marks' advances even where GPOS runs in its place.
            kerx_kerns = true;
        } else if kern::has_data(face) {
            kern = script_model.takes_fallbacks();
        }
    }

    let zeroes = script_model.mark_advances() != MarkAdvances::Keep
        && !kerx_kerns
        && (!kern || !kern::has_machine_kerning(face));
    let zero_marks = match model.mark_advances() {
        MarkAdvances::Keep => None,
        when => zeroes.then_some(when),
    };
    let unplaced = !gpos && !kerx_kerns && (!kern || !kern::has_cross_stream(face));
    // Only a font with a STAT, the mark of a modern font, is tracked.
    let trak = face.table("trak").is_some_and(crate::daecore::daetype::trak::has_data)
        && face.table("STAT").is_some_and(crate::daecore::daetype::stat::has_data);
    Positioning {
        gpos,
        kerx: kerx_kerns && !gpos,
        kern,
        trak,
        zero_marks,
        // Not under morx: Apple Color Emoji forms its sequences expecting no shift.
        marks_hang_back: unplaced && substitution != Substitution::Morx,
        fallback_marks: unplaced && script_model.takes_fallbacks(),
    }
}
