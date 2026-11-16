// Glyphs for the characters, then substitution: morx, or GSUB's stages with the model's pauses between them.
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::apply::TableRun;
use crate::daecore::daeshaper::ot::digest::Digest;
use crate::daecore::daeshaper::ot::morx;
use crate::daecore::daeshaper::plan::{Plan, Substitution, Table};
use crate::daecore::daeshaper::run::{class, flag, text, Ligature, Mask, Run};
use crate::daecore::daeshaper::unicode::GeneralCategory;
use crate::daecore::daetype::format::sanitize::AatTable;

// Each glyph takes the font's glyph for its character and the glyph's GDEF class, or one from its character where the
// font has none: a nonspacing mark that isn't ignorable is a mark, anything else a base. Gives the glyphs' digest.
pub(super) fn map_glyphs(face: &Face, plan: &Plan, run: &mut Run) -> Digest {
    let mut digest = Digest::default();
    for glyph in &mut run.glyphs[..run.len] {
        glyph.id = glyph.mapped;
        glyph.ligature = Ligature::default();
        (glyph.class, glyph.attach_class) = match plan.synthesized_classes {
            false => face.classes(glyph.id),
            true if glyph.category == GeneralCategory::NonspacingMark && glyph.text & text::IGNORABLE == 0 => {
                (class::MARK, 0)
            }
            true => (class::BASE, 0),
        };
        digest.add(glyph.id);
    }
    digest
}

// The glyphs GSUB leaves, as a digest GPOS starts from; none where morx substituted.
pub(super) fn substitute(face: &Face, plan: &Plan, run: &mut Run, digest: Digest) -> Option<Digest> {
    match plan.substitution {
        Substitution::Gsub => Some(apply_table(face, plan, run, Table::Gsub, digest)),
        Substitution::Morx => {
            if let Some(data) = face.aat(AatTable::Morx) {
                morx::apply(face, data, run, face.glyph_count(), &plan.aat.morx_flags, &plan.aat.morx_sets);
                // Deleted glyphs leave before GPOS, which would read the deletion mark as a glyph; kerx and kern
                // keep them, as a deletion parts the glyphs either side.
                if plan.positioning.gpos {
                    morx::remove_deleted_glyphs(run);
                }
            }
            None
        }
    }
}

// A table's stages in turn, each stage's lookups then its pause. The pauses run whether the font has the table or
// not, as a model's reordering is its own.
pub(super) fn apply_table(face: &Face, plan: &Plan, run: &mut Run, table: Table, digest: Digest) -> Digest {
    let mut lookups = TableRun::new(table, face, plan, run, digest);
    let mut masks = mask_union(lookups.run());
    for (stage, pause) in plan.schedule(table).stages() {
        for lookup in stage.iter().filter(|l| l.mask & masks != 0) {
            debug_assert!(
                mask_union(lookups.run()) & !flag::ALL & !masks == 0,
                "a glyph carries a feature bit outside the union; only pauses may set masks between lookups",
            );
            lookups.apply(lookup);
        }
        if let Some(pause) = pause {
            if pause(lookups.run(), plan, face) {
                lookups.glyphs_changed();
            }
            // Taken whatever the pause says: a reordering pause sets masks and reports only glyphs it put in.
            masks = mask_union(lookups.run());
        }
    }
    lookups.digest()
}

fn mask_union(run: &Run) -> Mask {
    run.glyphs[..run.len].iter().fold(0, |union, g| union | g.mask)
}
