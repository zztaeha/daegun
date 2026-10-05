use alloc::vec::Vec;

pub use super::buffer::Direction;

use super::face::Face;
use crate::daecore::daetype::decoder::read_u16_be;
use super::ot::map::{feature_flags as ff, LookupOverride, Map, MapBuilder, TableIndex, MAX_VALUE};
use super::ot::LayoutTable;
use super::script::Shaper;
use super::ot::tag::Tag;
use super::unicode::Script;

enum Routing {
    FromScript(Option<Script>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserFeature {
    pub tag: Tag,
    pub(crate) value: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl UserFeature {
    pub(crate) fn global(tag: Tag, value: u32) -> Self {
        UserFeature { tag, value, start: 0, end: u32::MAX }
    }

    pub(crate) fn is_global(&self) -> bool {
        self.start == 0 && self.end == u32::MAX
    }
}

const COMMON_FEATURES: &[(&[u8; 4], u32)] = &[
    (b"abvm", ff::GLOBAL),
    (b"blwm", ff::GLOBAL),
    (b"ccmp", ff::GLOBAL),
    (b"locl", ff::GLOBAL),
    (b"mark", ff::GLOBAL_MANUAL_JOINERS),
    (b"mkmk", ff::GLOBAL_MANUAL_JOINERS),
    (b"rlig", ff::GLOBAL),
];

const HORIZONTAL_FEATURES: &[(&[u8; 4], u32)] = &[
    (b"calt", ff::GLOBAL),
    (b"clig", ff::GLOBAL),
    (b"curs", ff::GLOBAL),
    (b"dist", ff::GLOBAL),
    (b"kern", ff::GLOBAL_HAS_FALLBACK),
    (b"liga", ff::GLOBAL),
    (b"rclt", ff::GLOBAL),
];

#[derive(Clone, Copy)]
struct Kinds {
    extension: u16,
    context: u16,
    chain_context: u16,
    pair_pos: u16,
}

const GSUB_KINDS: Kinds = Kinds {
    extension: super::ot::gsub::EXTENSION,
    context: super::ot::gsub::CONTEXT,
    chain_context: super::ot::gsub::CHAIN_CONTEXT,
    pair_pos: u16::MAX,
};

const GPOS_KINDS: Kinds = Kinds {
    extension: super::ot::gpos::EXTENSION,
    context: super::ot::gpos::CONTEXT,
    chain_context: super::ot::gpos::CHAIN_CONTEXT,
    pair_pos: super::ot::gpos::PAIR,
};

fn table_digests(
    face: &Face,
    table: Option<&LayoutTable>,
    map: &Map,
    which: TableIndex,
    k: Kinds,
    indexes: &SubtableIndexes,
) -> alloc::vec::Vec<super::ot::digest::Digest> {
    use super::ot::digest::Digest;
    let Some(table) = table else { return alloc::vec::Vec::new() };

    let count = table.lookup_count() as usize;
    let mut digests = alloc::vec![Digest::full(); count];
    for stage in 0..map.stages(which).len() {
        for lookup in map.stage_lookups(which, stage) {
            let at = lookup.index as usize;
            if at >= count {
                continue;
            }
            digests[at] = face.lookup_digest(which.idx(), count, lookup.index, || {
                if let Some(subs) = indexes.get(at).and_then(Option::as_ref) {
                    let mut d = Digest::new();
                    for entry in subs.iter() {
                        d.union(&entry.digest);
                    }
                    return d;
                }
                table
                    .lookup(lookup.index)
                    .map(|l| super::ot::lookup_digest(&l, k.extension, k.context, k.chain_context))
                    .unwrap_or_else(Digest::full)
            });
        }
    }
    digests
}

type SubtableIndexes = alloc::vec::Vec<Option<crate::daecore::sync::Shared<Vec<super::ot::SubtableIndex>>>>;

fn table_subtable_indexes(
    face: &Face,
    table: Option<&LayoutTable>,
    map: &Map,
    which: TableIndex,
    k: Kinds,
) -> SubtableIndexes {
    let Some(table) = table else { return alloc::vec::Vec::new() };
    let count = table.lookup_count() as usize;
    let mut out: SubtableIndexes = alloc::vec![None; count];
    for stage in 0..map.stages(which).len() {
        for lookup in map.stage_lookups(which, stage) {
            let at = lookup.index as usize;
            if at >= count || out[at].is_some() {
                continue;
            }
            out[at] = face.subtable_indexes(which.idx(), count, lookup.index, || {
                table.lookup(lookup.index).map_or_else(alloc::vec::Vec::new, |l| {
                    super::ot::subtable_indexes(
                        &l, k.extension, Some(k.pair_pos), k.context, k.chain_context,
                        |entries, absent| face.build_index(entries, absent),
                    )
                })
            });
        }
    }
    out
}

pub struct ShapePlan {
    pub(crate) shaper: &'static Shaper,
    pub(crate) direction: Direction,
    pub(crate) map: Map,
    pub(crate) subtable_indexes: [SubtableIndexes; 2],
    pub(crate) lookup_digests: [alloc::vec::Vec<super::ot::digest::Digest>; 2],
    pub(crate) frac_mask: u32,
    pub(crate) numr_mask: u32,
    pub(crate) dnom_mask: u32,
    pub(crate) rtlm_mask: u32,
    pub(crate) kern_mask: u32,
    pub(crate) has_frac: bool,
    pub(crate) has_vert: bool,
    pub(crate) has_gpos_mark: bool,
    pub(crate) fallback_glyph_classes: bool,
    pub(crate) apply_gpos: bool,
    pub(crate) apply_kern: bool,
    pub(crate) apply_kerx: bool,
    pub(crate) zero_marks: bool,
    pub(crate) adjust_mark_positioning_when_zeroing: bool,
    pub(crate) fallback_mark_positioning: bool,
    pub(crate) user_features: Vec<UserFeature>,
}

impl ShapePlan {
    pub fn shaper_name(&self) -> &'static str {
        self.shaper.name
    }

    #[allow(clippy::too_many_arguments, reason = "everything a plan is keyed on, and no fewer")]
    pub fn with_script(
        script: Option<Script>,
        face: &Face,
        direction: Direction,
        script_tags: &[Tag],
        language_tags: &[Tag],
        user_features: &[UserFeature],
        lookup_overrides: &[LookupOverride],
        coords: &[i32],
    ) -> ShapePlan {
        Self::build(Routing::FromScript(script), face, direction, script_tags, language_tags,
                    user_features, lookup_overrides, coords)
    }

    #[allow(clippy::too_many_arguments, reason = "everything a plan is keyed on, and no fewer")]
    fn build(
        routing: Routing,
        face: &Face,
        direction: Direction,
        script_tags: &[Tag],
        language_tags: &[Tag],
        user_features: &[UserFeature],
        lookup_overrides: &[LookupOverride],
        coords: &[i32],
    ) -> ShapePlan {
        let gsub = face.table("GSUB").and_then(LayoutTable::parse);
        let gpos = face.table("GPOS").and_then(LayoutTable::parse);

        let mut b = MapBuilder::new(gsub.as_ref(), gpos.as_ref(), script_tags, language_tags);
        let shaper = match routing {
            Routing::FromScript(script) => {
                super::script::select(
                    script,
                    direction,
                    b.chosen_script(TableIndex::Gsub),
                    script_tags.first().copied(),
                )
            }
        };
        collect_features(&mut b, direction, user_features, shaper, script_tags.first().copied());
        for o in lookup_overrides {
            b.override_lookup(o.table, o.index, o.enable);
        }

        let axis_count = face.table("fvar").and_then(|f| read_u16_be(f, 8)).map_or(0, usize::from);
        let variations = [gsub.as_ref(), gpos.as_ref()].map(|t| t.and_then(|t| t.find_variation_index(coords, axis_count)));
        let map = b.compile(variations);

        let subtable_indexes = [
            table_subtable_indexes(face, gsub.as_ref(), &map, TableIndex::Gsub, GSUB_KINDS),
            table_subtable_indexes(face, gpos.as_ref(), &map, TableIndex::Gpos, GPOS_KINDS),
        ];
        let lookup_digests = [
            table_digests(face, gsub.as_ref(), &map, TableIndex::Gsub, GSUB_KINDS, &subtable_indexes[0]),
            table_digests(face, gpos.as_ref(), &map, TableIndex::Gpos, GPOS_KINDS, &subtable_indexes[1]),
        ];

        let tag = |s: &[u8; 4]| Tag::from_bytes(s);
        let frac_mask = map.one_mask(tag(b"frac"));
        let numr_mask = map.one_mask(tag(b"numr"));
        let dnom_mask = map.one_mask(tag(b"dnom"));
        let has_frac = frac_mask != 0 || (numr_mask != 0 && dnom_mask != 0);

        let kern_tag = if direction.is_horizontal() { tag(b"kern") } else { tag(b"vkrn") };
        let kern_mask = map.mask(kern_tag).0;

        let has_gpos_kern = map.feature_index(TableIndex::Gpos, kern_tag).is_some();
        let disable_gpos = shaper.gpos_tag.is_some()
            && shaper.gpos_tag != map.chosen_script[TableIndex::Gpos.idx()];
        let apply_gpos = gpos.is_some() && !disable_gpos;

        let vertical = matches!(direction, Direction::TopToBottom | Direction::BottomToTop);
        let use_morx = face.has_table("morx") && (!vertical || !face.has_table("GSUB"));
        let substitutes_through_gsub = !use_morx && gsub.is_some();

        let has_kerx = super::ot::kerx::is_usable(face);
        let has_gpos = apply_gpos;

        // `kerx` outranks the rest unless the font also ships GSUB and GPOS, which real fonts shape
        // better through. `is_usable`, not `has_table`: a `kerx` of only variable or unknown subtables
        // does nothing and would take `kern` and the fallback down with it. `kern` falls back only for a
        // script whose shaper takes it; under Devanagari it would set two full stops apart.
        let mut apply_kerx = has_kerx && !(substitutes_through_gsub && has_gpos);
        let apply_gpos = !apply_kerx && has_gpos;
        let mut apply_kern = false;

        if !apply_kerx && (!has_gpos_kern || !apply_gpos) {
            if has_kerx {
                apply_kerx = true;
            } else if face.has_table("kern") {
                apply_kern = shaper.fallback_position;
            }
        }

        let zero_marks = shaper.zero_width_marks != crate::daecore::daeshaper::script::ZeroWidthMarks::Never
            && !apply_kerx
            && (!apply_kern || !super::ot::kern::has_machine_kerning(face));

        let adjust_mark_positioning_when_zeroing = !apply_gpos
            && !apply_kerx
            && (!apply_kern || !super::ot::kern::has_cross_stream(face));

        ShapePlan {
            shaper,
            direction,
            adjust_mark_positioning_when_zeroing,
            fallback_mark_positioning: adjust_mark_positioning_when_zeroing
                && shaper.fallback_position,
            frac_mask,
            numr_mask,
            dnom_mask,
            rtlm_mask: map.one_mask(tag(b"rtlm")),
            kern_mask,
            has_frac,
            has_vert: map.one_mask(tag(b"vert")) != 0,
            has_gpos_mark: map.one_mask(tag(b"mark")) != 0,
            fallback_glyph_classes: !face.has_glyph_classes(),
            apply_gpos,
            apply_kern,
            apply_kerx,
            zero_marks,
            map,
            lookup_digests,
            subtable_indexes,
            user_features: user_features.to_vec(),
        }
    }
}

fn collect_features(
    b: &mut MapBuilder,
    direction: Direction,
    user_features: &[UserFeature],
    shaper: &Shaper,
    script: Option<Tag>,
) {
    let tag = |s: &[u8; 4]| Tag::from_bytes(s);

    b.enable_feature(tag(b"rvrn"), ff::NONE, 1);
    b.add_gsub_pause(None);

    match direction {
        Direction::LeftToRight => {
            b.enable_feature(tag(b"ltra"), ff::NONE, 1);
            b.enable_feature(tag(b"ltrm"), ff::NONE, 1);
        }
        Direction::RightToLeft => {
            b.enable_feature(tag(b"rtla"), ff::NONE, 1);
            b.add_feature(tag(b"rtlm"), ff::NONE, 1);
        }
        _ => {}
    }

    b.add_feature(tag(b"frac"), ff::NONE, 1);
    b.add_feature(tag(b"numr"), ff::NONE, 1);
    b.add_feature(tag(b"dnom"), ff::NONE, 1);

    b.enable_feature(tag(b"rand"), ff::RANDOM, MAX_VALUE);

    if let Some(func) = shaper.collect_features {
        func(b, script);
    }

    for &(t, flags) in COMMON_FEATURES {
        b.add_feature(tag(t), flags, 1);
    }

    if direction.is_horizontal() {
        for &(t, flags) in HORIZONTAL_FEATURES {
            b.add_feature(tag(t), flags, 1);
        }
    } else {
        b.enable_feature(tag(b"vert"), ff::GLOBAL_SEARCH, 1);
    }

    for f in user_features {
        let flags = if f.is_global() { ff::GLOBAL } else { ff::NONE };
        b.add_feature(f.tag, flags, f.value);
    }

    if let Some(func) = shaper.override_features {
        func(b);
    }
}
