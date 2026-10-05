use crate::daecore::daeshaper::buffer::{scratch_flags, Buffer, GlyphInfo};
use crate::daecore::daeshaper::face::Face;
use super::indic_category::{category, position};
use crate::daecore::daeshaper::ot::map::{feature_flags as ff, MapBuilder, TableIndex};
use crate::daecore::daeshaper::normalize;
use crate::daecore::daeshaper::plan::ShapePlan;
use super::{PauseFn, Shaper, ZeroWidthMarks};
use super::syllabic;
use super::syllable::{self};
use crate::daecore::daeshaper::generated::syllable_tables::{indic_accept, IndicSyllable, INDIC_TRANSITIONS};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::unicode::{self, Script};

pub(crate) const SHAPER: Shaper = Shaper {
    name: "indic",
    collect_features: Some(collect_features),
    pauses: PAUSES,
    override_features: Some(override_features),
    preprocess_text: Some(preprocess_text),
    postprocess_glyphs: None,
    normalization_preference: normalize::Mode::ComposedDiacriticsNoShortCircuit,
    decompose: Some(decompose),
    compose: Some(compose),
    setup_masks: Some(setup_masks),
    gpos_tag: None,
    reorder_marks: None,
    zero_width_marks: ZeroWidthMarks::Never,
    fallback_position: false,
};

const PAUSES: &[PauseFn] = &[setup_syllables, initial_reordering, final_reordering, clear_syllables];

const PAUSE_SETUP_SYLLABLES: usize = 0;
const PAUSE_INITIAL_REORDERING: usize = 1;
const PAUSE_FINAL_REORDERING: usize = 2;
const PAUSE_CLEAR_SYLLABLES: usize = 3;

const FEATURES: [(&[u8; 4], u32); 17] = [
    (b"nukt", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"akhn", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"rphf", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"rkrf", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"pref", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"blwf", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"abvf", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"half", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"pstf", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"vatu", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"cjct", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"init", ff::MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"pres", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"abvs", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"blws", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"psts", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
    (b"haln", ff::GLOBAL_MANUAL_JOINERS | ff::PER_SYLLABLE),
];

const RPHF: usize = 2;
const PREF: usize = 4;
const BLWF: usize = 5;
const ABVF: usize = 6;
const HALF: usize = 7;
const PSTF: usize = 8;
const INIT: usize = 11;
const BASIC: usize = 11;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RephPosition {
    AfterMain,
    BeforeSub,
    AfterSub,
    BeforePost,
    AfterPost,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RephMode {
    Implicit,
    Explicit,
    LogRepha,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlwfMode {
    PreAndPost,
    PostOnly,
}

#[derive(Clone, Copy)]
struct Config {
    has_old_spec: bool,
    virama: u32,
    reph_pos: RephPosition,
    reph_mode: RephMode,
    blwf_mode: BlwfMode,
    reorders_zwj_halant: bool,
    disallow_double_halants: bool,
    old_spec_eyelash_ra: bool,
    skip_unformed_below_forms: bool,
    has_half_forms: bool,
}

const DEFAULT_CONFIG: Config = Config {
    has_old_spec: false,
    virama: 0,
    reph_pos: RephPosition::BeforePost,
    reph_mode: RephMode::Implicit,
    blwf_mode: BlwfMode::PreAndPost,
    reorders_zwj_halant: false,
    disallow_double_halants: false,
    old_spec_eyelash_ra: false,
    skip_unformed_below_forms: false,
    has_half_forms: true,
};

fn config_for(script: Option<Script>) -> Config {
    let c = |virama, reph_pos, reph_mode, blwf_mode| Config {
        has_old_spec: true,
        virama,
        reph_pos,
        reph_mode,
        blwf_mode,
        ..DEFAULT_CONFIG
    };
    use BlwfMode::{PostOnly, PreAndPost};
    use RephMode::{Explicit, Implicit, LogRepha};
    use RephPosition::{AfterMain, AfterPost, AfterSub, BeforePost, BeforeSub};

    match script.map(|s| s.name()) {
        Some("Devanagari") => Config {
            old_spec_eyelash_ra: true,
            ..c(0x094D, BeforePost, Implicit, PreAndPost)
        },
        Some("Bengali") => c(0x09CD, AfterSub, Implicit, PreAndPost),
        Some("Gurmukhi") => c(0x0A4D, BeforeSub, Implicit, PreAndPost),
        Some("Gujarati") => c(0x0ACD, BeforePost, Implicit, PreAndPost),
        Some("Oriya") => c(0x0B4D, AfterMain, Implicit, PreAndPost),
        Some("Tamil") => Config {
            has_half_forms: false,
            ..c(0x0BCD, AfterPost, Implicit, PreAndPost)
        },
        Some("Telugu") => c(0x0C4D, AfterPost, Explicit, PostOnly),
        Some("Kannada") => Config {
            reorders_zwj_halant: true,
            disallow_double_halants: true,
            ..c(0x0CCD, AfterPost, Implicit, PostOnly)
        },
        Some("Malayalam") => Config {
            has_half_forms: false,
            skip_unformed_below_forms: true,
            ..c(0x0D4D, AfterMain, LogRepha, PreAndPost)
        },
        Some("Sinhala") => Config {
            has_old_spec: false,
            virama: 0x0DCA,
            reph_pos: AfterPost,
            reph_mode: Explicit,
            blwf_mode: PreAndPost,
            ..DEFAULT_CONFIG
        },
        _ => DEFAULT_CONFIG,
    }
}

struct Plan<'a> {
    plan: &'a ShapePlan,
    face: &'a Face<'a>,
    config: Config,
    is_old_spec: bool,
    masks: [u32; FEATURES.len()],
    gsub: Option<crate::daecore::daeshaper::ot::LayoutTable<'a>>,
}

impl<'a> Plan<'a> {
    fn new(plan: &'a ShapePlan, face: &'a Face<'a>, script: Option<Script>) -> Self {
        let config = config_for(script);
        let chosen = plan.map.chosen_script[TableIndex::Gsub.idx()];
        // `is_none_or`, not `is_some_and`: a font whose GSUB matched no script tag – or that has no
        // GSUB – is old-spec, because the reference tests a sentinel tag rather than absence and no
        // sentinel ends in '2'. Backwards, a GSUB-less Indic font skips the post-base merge.
        let is_old_spec = config.has_old_spec && chosen.is_none_or(|t| t.to_bytes()[3] != b'2');

        let mut masks = [0; FEATURES.len()];
        for (i, (tag, flags)) in FEATURES.iter().enumerate() {
            if flags & ff::GLOBAL == 0 {
                masks[i] = plan.map.one_mask(Tag::from_bytes(tag));
            }
        }

        let gsub = face.table("GSUB").and_then(crate::daecore::daeshaper::ot::LayoutTable::parse);

        Plan { plan, face, config, is_old_spec, masks, gsub }
    }

    fn would_substitute(&self, tag: &[u8; 4], glyphs: &[u16]) -> bool {
        let Some(gsub) = self.gsub.as_ref() else { return false };
        let Some(feature) = self.plan.map.feature(Tag::from_bytes(tag)) else { return false };
        let stage = feature.stage[TableIndex::Gsub.idx()];
        let ctx = crate::daecore::daeshaper::ot::gsub::WouldApplyContext { glyphs, zero_context: true };

        let digests = &self.plan.lookup_digests[TableIndex::Gsub.idx()];
        let first = glyphs.first().copied().unwrap_or(0);
        self.plan
            .map
            .stage_lookups(TableIndex::Gsub, stage)
            .iter()
            .filter(|l| {
                digests.get(l.index as usize).is_none_or(|d| d.may_have(first))
            })
            .any(|l| crate::daecore::daeshaper::ot::gsub::would_apply(&ctx, gsub, l.index))
    }
}

impl GlyphInfo {
    fn indic_position(&self) -> u8 {
        self.shaper_auxiliary
    }

    fn set_indic_position(&mut self, position: u8) {
        self.shaper_auxiliary = position;
    }

    fn is_category(&self, categories: &[u8]) -> bool {
        !self.ligated() && categories.contains(&self.shaper_category)
    }

    fn is_joiner(&self) -> bool {
        self.is_category(&[category::ZWJ, category::ZWNJ])
    }

    fn is_halant(&self) -> bool {
        self.is_category(&[category::H])
    }

    fn is_matra(&self) -> bool {
        self.is_category(&[category::M, category::MPST])
    }

    fn is_consonant(&self) -> bool {
        self.is_category(&[
            category::C,
            category::CS,
            category::RA,
            category::CM,
            category::V,
            category::PLACEHOLDER,
            category::DOTTEDCIRCLE,
        ])
    }

    fn is_base_candidate(&self) -> bool {
        self.is_category(&[
            category::C,
            category::CS,
            category::RA,
            category::V,
            category::PLACEHOLDER,
            category::DOTTEDCIRCLE,
        ])
    }
}

fn collect_features(b: &mut MapBuilder, _: Option<Tag>) {
    b.add_gsub_pause(Some(PAUSE_SETUP_SYLLABLES));

    b.enable_feature(Tag::from_bytes(b"locl"), ff::PER_SYLLABLE, 1);
    b.enable_feature(Tag::from_bytes(b"ccmp"), ff::PER_SYLLABLE, 1);

    b.add_gsub_pause(Some(PAUSE_INITIAL_REORDERING));

    for (tag, flags) in FEATURES.iter().take(BASIC) {
        b.add_feature(Tag::from_bytes(tag), *flags, 1);
        b.add_gsub_pause(None);
    }

    b.add_gsub_pause(Some(PAUSE_FINAL_REORDERING));

    for (tag, flags) in FEATURES.iter().skip(BASIC) {
        b.add_feature(Tag::from_bytes(tag), *flags, 1);
    }
}

fn override_features(b: &mut MapBuilder) {
    b.disable_feature(Tag::from_bytes(b"liga"));
    b.add_gsub_pause(Some(PAUSE_CLEAR_SYLLABLES));
}

fn preprocess_text(_: &ShapePlan, face: &Face, buffer: &mut Buffer) {
    syllabic::insert_vowel_constraints(face, buffer);
}

fn decompose(_: &normalize::Context, ab: char) -> Option<(char, Option<char>)> {
    match ab {
        '\u{0931}'
        | '\u{09DC}'
        | '\u{09DD}'
        | '\u{0B94}' => None,
        _ => unicode::decompose(ab),
    }
}

fn compose(_: &normalize::Context, a: char, b: char) -> Option<char> {
    if unicode::general_category(a).is_mark() {
        return None;
    }
    if a == '\u{09AF}' && b == '\u{09BC}' {
        return Some('\u{09DF}');
    }
    unicode::compose(a, b)
}

fn setup_masks(_: &ShapePlan, _: &Face, buffer: &mut Buffer) {
    let len = buffer.len;
    for info in &mut buffer.info[..len] {
        let (cat, pos) = super::indic_category::lookup(info.id);
        info.shaper_category = cat;
        info.set_indic_position(pos);
    }
}

fn setup_syllables(_: &ShapePlan, _: &Face, buffer: &mut Buffer) -> bool {
    segment_into_syllables(buffer);

    let mut start = 0;
    while start < buffer.len {
        let end = next_syllable(buffer, start);
        buffer.unsafe_to_break(start, end);
        start = end;
    }

    false
}

fn segment_into_syllables(buffer: &mut Buffer) {
    let mut segments = syllable::Segments::new();

    let len = buffer.len;
    syllable::segment(
        len,
        &INDIC_TRANSITIONS,
        |i| buffer.info[i].shaper_category,
        indic_accept,
        |s| segments.push(s),
    );

    let segments = segments.as_slice();
    if segments.iter().any(|s| s.kind == u8::from(IndicSyllable::BrokenCluster)) {
        buffer.scratch_flags |= scratch_flags::HAS_BROKEN_SYLLABLE;
    }
    syllable::set_syllables(buffer, segments);
}

fn next_syllable(buffer: &Buffer, start: usize) -> usize {
    if start >= buffer.len {
        return start;
    }
    let syllable = buffer.info[start].syllable;
    let mut end = start + 1;
    while end < buffer.len && buffer.info[end].syllable == syllable {
        end += 1;
    }
    end
}

fn initial_reordering(plan: &ShapePlan, face: &Face, buffer: &mut Buffer) -> bool {
    let p = Plan::new(plan, face, buffer.script);

    update_consonant_positions(&p, buffer);

    let inserted = syllabic::insert_dotted_circles(
        face,
        buffer,
        IndicSyllable::BrokenCluster.into(),
        category::DOTTEDCIRCLE,
        Some(category::REPHA),
        Some(position::END),
    );

    let mut start = 0;
    while start < buffer.len {
        let end = next_syllable(buffer, start);
        let kind = buffer.info[start].syllable & 0x0F;
        if kind != IndicSyllable::SymbolCluster.into()
            && kind != IndicSyllable::NonIndicCluster.into()
        {
            initial_reordering_syllable(&p, start, end, buffer);
        }
        start = end;
    }

    inserted
}

fn update_consonant_positions(p: &Plan, buffer: &mut Buffer) {
    if p.config.virama == 0 {
        return;
    }
    let Some(virama) = p.face.glyph_index(p.config.virama) else { return };

    let len = buffer.len;
    for i in 0..len {
        if buffer.info[i].indic_position() != position::BASE_C {
            continue;
        }
        let consonant = buffer.info[i].id as u16;
        buffer.info[i].set_indic_position(consonant_position(p, consonant, virama));
    }
}

fn consonant_position(p: &Plan, consonant: u16, virama: u16) -> u8 {
    // Both orders are asked about, not one. Old spec writes the consonant before the virama and new
    // spec after, but fonts copied lookups across unchanged and Uniscribe honors them anyway.
    let both = |tag: &[u8; 4]| {
        p.would_substitute(tag, &[virama, consonant]) || p.would_substitute(tag, &[consonant, virama])
    };

    if both(b"blwf") || both(b"vatu") {
        return position::BELOW_C;
    }
    if both(b"pstf") || both(b"pref") {
        return position::POST_C;
    }
    position::BASE_C
}

fn initial_reordering_syllable(p: &Plan, start: usize, end: usize, buffer: &mut Buffer) {
    if p.config.reorders_zwj_halant
        && start + 3 <= end
        && buffer.info[start].is_category(&[category::RA])
        && buffer.info[start + 1].is_category(&[category::H])
        && buffer.info[start + 2].is_category(&[category::ZWJ])
    {
        buffer.merge_clusters(start + 1, start + 3);
        buffer.info.swap(start + 1, start + 2);
    }

    let (base, has_reph) = find_base(p, start, end, buffer);

    assign_positions(p, start, base, end, buffer);
    if has_reph {
        buffer.info[start].set_indic_position(position::RA_TO_BECOME_REPH);
    }

    if p.is_old_spec {
        move_post_base_halant(p, base, end, buffer);
    }

    attach_marks_to_neighbors(start, end, buffer);
    give_post_base_consonants_their_marks(base, end, buffer);

    let base = sort_syllable(p, start, base, end, buffer);
    setup_syllable_masks(p, start, base, end, buffer);
}

fn find_base(p: &Plan, start: usize, end: usize, buffer: &mut Buffer) -> (usize, bool) {
    let mut base = end;
    let mut has_reph = false;
    let mut limit = start;

    if p.masks[RPHF] != 0
        && start + 3 <= end
        && ((p.config.reph_mode == RephMode::Implicit && !buffer.info[start + 2].is_joiner())
            || (p.config.reph_mode == RephMode::Explicit
                && buffer.info[start + 2].shaper_category == category::ZWJ))
    {
        let pair = [buffer.info[start].id as u16, buffer.info[start + 1].id as u16];
        let triple = [pair[0], pair[1], buffer.info[start + 2].id as u16];

        if p.would_substitute(b"rphf", &pair)
            || (p.config.reph_mode == RephMode::Explicit && p.would_substitute(b"rphf", &triple))
        {
            limit += 2;
            while limit < end && buffer.info[limit].is_joiner() {
                limit += 1;
            }
            base = start;
            has_reph = true;
        }
    } else if p.config.reph_mode == RephMode::LogRepha
        && buffer.info[start].shaper_category == category::REPHA
    {
        limit += 1;
        while limit < end && buffer.info[limit].is_joiner() {
            limit += 1;
        }
        base = start;
        has_reph = true;
    }

    let mut i = end;
    let mut seen_below = false;
    loop {
        i -= 1;
        if buffer.info[i].is_base_candidate() {
            let pos = buffer.info[i].indic_position();
            if pos != position::BELOW_C && (pos != position::POST_C || seen_below) {
                base = i;
                break;
            }
            if pos == position::BELOW_C {
                seen_below = true;
            }
            base = i;
        } else if start < i
            && buffer.info[i].shaper_category == category::ZWJ
            && buffer.info[i - 1].shaper_category == category::H
        {
            break;
        }

        if i <= limit {
            break;
        }
    }

    if has_reph && base == start && limit - base <= 2 {
        has_reph = false;
    }

    (base, has_reph)
}

fn assign_positions(_p: &Plan, start: usize, base: usize, end: usize, buffer: &mut Buffer) {
    for i in start..base {
        let pos = buffer.info[i].indic_position();
        buffer.info[i].set_indic_position(pos.min(position::PRE_C));
    }
    if base < end {
        buffer.info[base].set_indic_position(position::BASE_C);
    }
}

fn move_post_base_halant(p: &Plan, base: usize, end: usize, buffer: &mut Buffer) {
    let disallow_double_halants = p.config.disallow_double_halants;

    for i in base + 1..end {
        if buffer.info[i].shaper_category != category::H {
            continue;
        }

        let mut j = end - 1;
        while j > i {
            if buffer.info[j].is_consonant()
                || (disallow_double_halants && buffer.info[j].shaper_category == category::H)
            {
                break;
            }
            j -= 1;
        }

        if buffer.info[j].shaper_category != category::H && j > i {
            let halant = buffer.info[i];
            for k in 0..j - i {
                buffer.info[k + i] = buffer.info[k + i + 1];
            }
            buffer.info[j] = halant;
        }

        break;
    }
}

fn attach_marks_to_neighbors(start: usize, end: usize, buffer: &mut Buffer) {
    let mut last_pos = position::START;

    for i in start..end {
        let travels = buffer.info[i].is_category(&[
            category::ZWJ,
            category::ZWNJ,
            category::N,
            category::RS,
            category::CM,
            category::H,
        ]);

        if travels {
            buffer.info[i].set_indic_position(last_pos);

            if buffer.info[i].shaper_category == category::H
                && buffer.info[i].indic_position() == position::PRE_M
            {
                for j in (start + 1..=i).rev() {
                    if buffer.info[j - 1].indic_position() != position::PRE_M {
                        let pos = buffer.info[j - 1].indic_position();
                        buffer.info[i].set_indic_position(pos);
                        break;
                    }
                }
            }
        } else if buffer.info[i].indic_position() != position::SMVD {
            if buffer.info[i].shaper_category == category::MPST
                && i > start
                && buffer.info[i - 1].shaper_category == category::SM
            {
                let pos = buffer.info[i].indic_position();
                buffer.info[i - 1].set_indic_position(pos);
            }

            last_pos = buffer.info[i].indic_position();
        }
    }
}

fn give_post_base_consonants_their_marks(base: usize, end: usize, buffer: &mut Buffer) {
    let mut last = base;

    for i in base + 1..end {
        if buffer.info[i].is_consonant() {
            let pos = buffer.info[i].indic_position();
            for j in last + 1..i {
                if buffer.info[j].indic_position() < position::SMVD {
                    buffer.info[j].set_indic_position(pos);
                }
            }
            last = i;
        } else if buffer.info[i].is_matra() {
            last = i;
        }
    }
}

fn sort_syllable(p: &Plan, start: usize, _base: usize, end: usize, buffer: &mut Buffer) -> usize {
    // The syllable number is borrowed as scratch to record where each glyph came from, which is what
    // lets the cluster merging below know what crossed what. Restored afterwards.
    let syllable = buffer.info[start].syllable;
    for i in start..end {
        buffer.info[i].syllable = (i - start) as u8;
    }

    buffer.info[start..end].sort_by_key(|a| a.indic_position());

    let mut base = end;
    let mut first_left_matra = end;
    let mut last_left_matra = end;

    for i in start..end {
        if buffer.info[i].indic_position() == position::BASE_C {
            base = i;
            break;
        } else if buffer.info[i].indic_position() == position::PRE_M {
            if first_left_matra == end {
                first_left_matra = i;
            }
            last_left_matra = i;
        }
    }

    if first_left_matra < last_left_matra {
        buffer.reverse_range(first_left_matra, last_left_matra + 1);

        let mut group_start = first_left_matra;
        for j in first_left_matra..=last_left_matra {
            if buffer.info[j].is_matra() {
                buffer.reverse_range(group_start, j + 1);
                group_start = j + 1;
            }
        }
    }

    merge_clusters_after_base(p, start, base, end, buffer);

    for info in &mut buffer.info[start..end] {
        info.syllable = syllable;
    }

    base
}

fn merge_clusters_after_base(
    p: &Plan,
    start: usize,
    base: usize,
    end: usize,
    buffer: &mut Buffer,
) {
    if p.is_old_spec || end - start > 127 {
        buffer.merge_clusters(base, end);
        return;
    }

    for i in base..end {
        if buffer.info[i].syllable == 255 {
            continue;
        }
        let mut min = i;
        let mut max = i;
        let mut j = start + buffer.info[i].syllable as usize;
        while j != i {
            min = min.min(j);
            max = max.max(j);
            let next = start + buffer.info[j].syllable as usize;
            buffer.info[j].syllable = 255;
            j = next;
        }
        buffer.merge_clusters(base.max(min), max + 1);
    }
}

fn setup_syllable_masks(p: &Plan, start: usize, base: usize, end: usize, buffer: &mut Buffer) {
    for info in &mut buffer.info[start..end] {
        if info.indic_position() != position::RA_TO_BECOME_REPH {
            break;
        }
        info.mask |= p.masks[RPHF];
    }

    let mut pre_base = p.masks[HALF];
    if !p.is_old_spec && p.config.blwf_mode == BlwfMode::PreAndPost {
        pre_base |= p.masks[BLWF];
    }
    for info in &mut buffer.info[start..base] {
        info.mask |= pre_base;
    }

    let post_base = p.masks[BLWF] | p.masks[ABVF] | p.masks[PSTF];
    for i in base + 1..end {
        buffer.info[i].mask |= post_base;
    }

    if p.is_old_spec && p.config.old_spec_eyelash_ra {
        mark_old_spec_eyelash_ra(p, start, base, end, buffer);
    }

    mark_pre_base_reordering_ra(p, base, end, buffer);
    apply_joiner_effects(p, start, end, buffer);
}

fn mark_old_spec_eyelash_ra(p: &Plan, start: usize, base: usize, _end: usize, buffer: &mut Buffer) {
    for i in start..base.saturating_sub(1) {
        if buffer.info[i].shaper_category == category::RA
            && buffer.info[i + 1].shaper_category == category::H
            && (i + 2 == base || buffer.info[i + 2].shaper_category != category::ZWJ)
        {
            buffer.info[i].mask |= p.masks[BLWF];
            buffer.info[i + 1].mask |= p.masks[BLWF];
        }
    }
}

fn mark_pre_base_reordering_ra(p: &Plan, base: usize, end: usize, buffer: &mut Buffer) {
    const PREF_LEN: usize = 2;
    if p.masks[PREF] == 0 || base + PREF_LEN >= end {
        return;
    }

    for i in base + 1..end - PREF_LEN + 1 {
        let pair = [buffer.info[i].id as u16, buffer.info[i + 1].id as u16];
        if p.would_substitute(b"pref", &pair) {
            buffer.info[i].mask |= p.masks[PREF];
            buffer.info[i + 1].mask |= p.masks[PREF];
            break;
        }
    }
}

fn apply_joiner_effects(p: &Plan, start: usize, end: usize, buffer: &mut Buffer) {
    for i in start + 1..end {
        if !buffer.info[i].is_joiner() {
            continue;
        }
        let non_joiner = buffer.info[i].shaper_category == category::ZWNJ;
        let mut j = i;

        loop {
            j -= 1;
            if non_joiner {
                buffer.info[j].mask &= !p.masks[HALF];
            }
            if j <= start || buffer.info[j].is_consonant() {
                break;
            }
        }
    }
}

fn final_reordering(plan: &ShapePlan, face: &Face, buffer: &mut Buffer) -> bool {
    if buffer.len == 0 {
        return false;
    }
    let p = Plan::new(plan, face, buffer.script);

    let mut start = 0;
    while start < buffer.len {
        let end = next_syllable(buffer, start);
        final_reordering_syllable(&p, start, end, buffer);
        start = end;
    }

    false
}

fn final_reordering_syllable(p: &Plan, start: usize, end: usize, buffer: &mut Buffer) {
    recover_lost_halants(p, start, end, buffer);

    let mut try_pref = p.masks[PREF] != 0;
    let mut base = find_base_again(p, start, end, buffer, &mut try_pref);

    reorder_pre_base_matra(p, start, &mut base, end, buffer);
    reorder_reph(p, start, &mut base, end, buffer);
    reorder_pre_base_reordering_consonant(p, start, base, end, buffer, try_pref);

    if buffer.info[start].indic_position() == position::PRE_M {
        let continues_word = start > 0 && {
            let gc = unicode::GeneralCategory::from_stored(buffer.info[start - 1].general_category());
            gc.is_letter()
                || gc.is_mark()
                || matches!(
                    gc,
                    unicode::GeneralCategory::Format
                        | unicode::GeneralCategory::Unassigned
                        | unicode::GeneralCategory::PrivateUse
                        | unicode::GeneralCategory::Surrogate
                )
        };
        if continues_word {
            buffer.unsafe_to_break(start - 1, start + 1);
        } else {
            buffer.info[start].mask |= p.masks[INIT];
        }
    }
}

fn recover_lost_halants(p: &Plan, start: usize, end: usize, buffer: &mut Buffer) {
    if p.config.virama == 0 {
        return;
    }
    let Some(virama) = p.face.glyph_index(p.config.virama) else { return };

    for info in &mut buffer.info[start..end] {
        if info.id == virama as u32 && info.ligated() && info.multiplied() {
            info.shaper_category = category::H;
            info.clear_ligated_and_multiplied();
        }
    }
}

fn find_base_again(
    p: &Plan,
    start: usize,
    end: usize,
    buffer: &mut Buffer,
    try_pref: &mut bool,
) -> usize {
    let mut base = start;

    while base < end {
        if buffer.info[base].indic_position() < position::BASE_C {
            base += 1;
            continue;
        }

        if *try_pref && base + 1 < end {
            for i in base + 1..end {
                if buffer.info[i].mask & p.masks[PREF] == 0 {
                    continue;
                }
                if !(buffer.info[i].substituted() && buffer.info[i].ligated_and_didnt_multiply())
                {
                    base = i;
                    while base < end && buffer.info[base].is_halant() {
                        base += 1;
                    }
                    if base < end {
                        buffer.info[base].set_indic_position(position::BASE_C);
                    }
                    *try_pref = false;
                }
                break;
            }
        }

        if p.config.skip_unformed_below_forms {
            base = skip_unformed_below_forms(base, end, buffer);
        }

        if start < base && buffer.info[base].indic_position() > position::BASE_C {
            base -= 1;
        }
        break;
    }

    if base == end && start < base && buffer.info[base - 1].is_category(&[category::ZWJ]) {
        base -= 1;
    }

    if base < end {
        while start < base && buffer.info[base].is_category(&[category::N, category::H]) {
            base -= 1;
        }
    }

    base
}

fn skip_unformed_below_forms(mut base: usize, end: usize, buffer: &mut Buffer) -> usize {
    let mut i = base + 1;
    while i < end {
        while i < end && buffer.info[i].is_joiner() {
            i += 1;
        }
        if i == end || !buffer.info[i].is_halant() {
            break;
        }
        i += 1;

        while i < end && buffer.info[i].is_joiner() {
            i += 1;
        }
        if i < end
            && buffer.info[i].is_consonant()
            && buffer.info[i].indic_position() == position::BELOW_C
        {
            base = i;
            buffer.info[base].set_indic_position(position::BASE_C);
        }
        i += 1;
    }
    base
}

fn reorder_pre_base_matra(
    p: &Plan,
    start: usize,
    base: &mut usize,
    end: usize,
    buffer: &mut Buffer,
) {
    if start + 1 >= end || start >= *base {
        return;
    }

    let mut new_pos = if *base == end { *base - 2 } else { *base - 1 };

    if p.config.has_half_forms {
        loop {
            while new_pos > start
                && !buffer.info[new_pos].is_category(&[category::M, category::MPST, category::H])
            {
                new_pos -= 1;
            }

            if buffer.info[new_pos].is_halant()
                && buffer.info[new_pos].indic_position() != position::PRE_M
            {
                if new_pos + 1 < end
                    && buffer.info[new_pos + 1].shaper_category == category::ZWJ
                    && new_pos > start
                {
                    new_pos -= 1;
                    continue;
                }
            } else {
                new_pos = start;
            }
            break;
        }
    }

    if start < new_pos && buffer.info[new_pos].indic_position() != position::PRE_M {
        for i in (start + 1..=new_pos).rev() {
            if buffer.info[i - 1].indic_position() != position::PRE_M {
                continue;
            }
            let old_pos = i - 1;
            if old_pos < *base && *base <= new_pos {
                *base -= 1;
            }

            let matra = buffer.info[old_pos];
            for k in 0..new_pos - old_pos {
                buffer.info[k + old_pos] = buffer.info[k + old_pos + 1];
            }
            buffer.info[new_pos] = matra;

            buffer.merge_clusters(new_pos, end.min(*base + 1));
            new_pos -= 1;
        }
    } else {
        for i in start..*base {
            if buffer.info[i].indic_position() == position::PRE_M {
                buffer.merge_clusters(i, end.min(*base + 1));
                break;
            }
        }
    }
}

fn reorder_reph(p: &Plan, start: usize, base: &mut usize, end: usize, buffer: &mut Buffer) {
    if start + 1 >= end || buffer.info[start].indic_position() != position::RA_TO_BECOME_REPH {
        return;
    }
    let is_repha_char = buffer.info[start].shaper_category == category::REPHA;
    if is_repha_char == buffer.info[start].ligated_and_didnt_multiply() {
        return;
    }

    let new_reph_pos = find_reph_position(p, start, *base, end, buffer);

    buffer.merge_clusters(start, new_reph_pos + 1);
    let reph = buffer.info[start];
    for i in 0..new_reph_pos - start {
        buffer.info[i + start] = buffer.info[i + start + 1];
    }
    buffer.info[new_reph_pos] = reph;

    if start < *base && *base <= new_reph_pos {
        *base -= 1;
    }
}

fn find_reph_position(p: &Plan, start: usize, base: usize, end: usize, buffer: &Buffer) -> usize {
    let after_first_halant = || {
        let mut at = start + 1;
        while at < base && !buffer.info[at].is_halant() {
            at += 1;
        }
        if at < base && buffer.info[at].is_halant() {
            if at + 1 < base && buffer.info[at + 1].is_joiner() {
                at += 1;
            }
            return Some(at);
        }
        None
    };

    if p.config.reph_pos != RephPosition::AfterPost {
        if let Some(at) = after_first_halant() {
            return at;
        }

        if p.config.reph_pos == RephPosition::AfterMain {
            let mut at = base;
            while at + 1 < end && buffer.info[at + 1].indic_position() <= position::AFTER_MAIN {
                at += 1;
            }
            if at < end {
                return at;
            }
        }

        if p.config.reph_pos == RephPosition::AfterSub {
            let mut at = base;
            while at + 1 < end
                && !matches!(
                    buffer.info[at + 1].indic_position(),
                    position::POST_C | position::AFTER_POST | position::SMVD
                )
            {
                at += 1;
            }
            if at < end {
                return at;
            }
        }
    }

    if let Some(at) = after_first_halant() {
        return at;
    }

    let mut at = end - 1;
    while at > start && buffer.info[at].indic_position() == position::SMVD {
        at -= 1;
    }

    if buffer.info[at].is_halant() {
        let matras = buffer.info[base + 1..at].iter().filter(|i| i.is_matra()).count();
        at -= matras;
    }
    at
}

fn reorder_pre_base_reordering_consonant(
    p: &Plan,
    start: usize,
    base: usize,
    end: usize,
    buffer: &mut Buffer,
    try_pref: bool,
) {
    if !try_pref || base + 1 >= end {
        return;
    }

    for i in base + 1..end {
        if buffer.info[i].mask & p.masks[PREF] == 0 {
            continue;
        }
        if !buffer.info[i].ligated_and_didnt_multiply() {
            break;
        }

        let mut new_pos = base;
        if p.config.has_half_forms {
            while new_pos > start
                && !buffer.info[new_pos - 1]
                    .is_category(&[category::M, category::MPST, category::H])
            {
                new_pos -= 1;
            }
        }

        if new_pos > start
            && buffer.info[new_pos - 1].is_halant()
            && new_pos < end
            && buffer.info[new_pos].is_joiner()
        {
            new_pos += 1;
        }

        let old_pos = i;
        buffer.merge_clusters(new_pos, old_pos + 1);
        let consonant = buffer.info[old_pos];
        for k in (0..old_pos - new_pos).rev() {
            buffer.info[k + new_pos + 1] = buffer.info[k + new_pos];
        }
        buffer.info[new_pos] = consonant;
        break;
    }
}

fn clear_syllables(_: &ShapePlan, _: &Face, buffer: &mut Buffer) -> bool {
    let len = buffer.len;
    for info in &mut buffer.info[..len] {
        info.syllable = 0;
    }
    false
}
