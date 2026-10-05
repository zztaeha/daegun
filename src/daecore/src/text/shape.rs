use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::cache::FontCache;
use crate::daecore::daetype::jstf::JstfModLists;
use crate::daecore::daeshaper::ot::map::{LookupOverride, TableIndex};
use crate::daecore::daeshaper::plan::UserFeature;

#[derive(Debug, Clone)]
pub struct ShapedRun {
    pub glyphs:   Vec<u16>,
    pub advances: Vec<f64>,
    pub offsets:  Vec<(f64, f64)>,
    pub unsafe_to_break: Vec<bool>,
    pub unsafe_to_concat: Vec<bool>,
    pub safe_to_insert_tatweel: Vec<bool>,
    pub clusters: Vec<u32>,
    pub complete: bool,
    pub has_broken_syllable: bool,
    pub shaper: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Ignorables {
    #[default]
    Hide,
    Remove,
    Preserve,
}

#[derive(Clone, Copy, Default)]
pub struct ShapeOptions<'a> {
    pub cluster_level: crate::daecore::daeshaper::buffer::ClusterLevel,
    pub before: &'a str,
    pub after: &'a str,
    pub beginning_of_text: bool,
    pub point_size: Option<f64>,
    pub features: &'a [(&'a str, u32)],
    pub script: Option<&'a str>,
    pub language: Option<&'a str>,
    pub report_unsafe_to_concat: bool,
    pub report_tatweel_positions: bool,
    pub ignorables: Ignorables,
    pub suppress_dotted_circle: bool,
    pub invisible_glyph: Option<u16>,
    pub seed_script: Option<crate::daecore::daeshaper::unicode::Script>,
}

pub fn shape_run_directional(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool, rtl: bool,
) -> Option<ShapedRun> {
    shape_run_with_features(fc, axis_values, text, vertical, &[], &[], None, None, Some(rtl), &ShapeOptions::default())
}

pub fn shape_run_directional_with_options(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool, rtl: bool,
    opts: &ShapeOptions,
) -> Option<ShapedRun> {
    shape_run_stated_with_options(fc, axis_values, text, vertical, Some(rtl), opts)
}

pub fn shape_run_stated_with_options(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool,
    stated_rtl: Option<bool>, opts: &ShapeOptions,
) -> Option<ShapedRun> {
    let features: Vec<UserFeature> = opts.features
        .iter()
        .filter_map(|(t, value)| Some(UserFeature::global(parse_tag(t)?, *value)))
        .collect();
    shape_run_with_features(
        fc, axis_values, text, vertical,
        &features, &[], opts.language, opts.script.and_then(parse_tag), stated_rtl, opts,
    )
}

fn parse_tag(s: &str) -> Option<crate::daecore::daeshaper::ot::tag::Tag> {
    let bytes: [u8; 4] = s.as_bytes().try_into().ok()?;
    Some(crate::daecore::daeshaper::ot::tag::Tag::from_bytes(&bytes))
}

pub fn shape_run_with_language(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool, language: &str,
) -> Option<ShapedRun> {
    shape_run_with_features(fc, axis_values, text, vertical, &[], &[], Some(language), None, None, &ShapeOptions::default())
}

pub(crate) fn shape_run_justified(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool,
    mods: &JstfModLists, shrink: bool,
) -> Option<ShapedRun> {
    let overrides = jstf_mods_to_overrides(mods, shrink);
    shape_run_with_features(fc, axis_values, text, vertical, &[], &overrides, None, None, None, &ShapeOptions::default())
}

fn jstf_mods_to_overrides(mods: &JstfModLists, shrink: bool) -> Vec<LookupOverride> {
    let (enable_gsub, disable_gsub, enable_gpos, disable_gpos) = if shrink {
        (&mods.shrinkage_enable_gsub, &mods.shrinkage_disable_gsub, &mods.shrinkage_enable_gpos, &mods.shrinkage_disable_gpos)
    } else {
        (&mods.extension_enable_gsub, &mods.extension_disable_gsub, &mods.extension_enable_gpos, &mods.extension_disable_gpos)
    };

    let mut out = Vec::new();
    let mut push = |list: &Option<Vec<u16>>, table: TableIndex, enable: bool| {
        for &index in list.iter().flatten() {
            out.push(LookupOverride { table, index, enable });
        }
    };
    push(disable_gsub, TableIndex::Gsub, false);
    push(disable_gpos, TableIndex::Gpos, false);
    push(enable_gsub, TableIndex::Gsub, true);
    push(enable_gpos, TableIndex::Gpos, true);
    out
}

pub fn shape_run_with_options(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool,
    opts: &ShapeOptions,
) -> Option<ShapedRun> {
    shape_run_stated_with_options(fc, axis_values, text, vertical, None, opts)
}

pub fn shape_run_with_user_features(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool,
    script: Option<&str>, features: &[(String, u32)],
) -> Option<ShapedRun> {
    let features: Vec<UserFeature> = features
        .iter()
        .filter_map(|(t, value)| Some(UserFeature::global(parse_tag(t)?, *value)))
        .collect();
    shape_run_with_features(fc, axis_values, text, vertical, &features, &[], None, script.and_then(parse_tag), None, &ShapeOptions::default())
}

#[allow(clippy::too_many_arguments, reason = "everything one shaped run is keyed on")]
fn shape_run_with_features(
    fc: &FontCache, axis_values: &[(String, f64)], text: &str, vertical: bool,
    features: &[UserFeature], lookup_overrides: &[LookupOverride], language: Option<&str>,
    script_override: Option<crate::daecore::daeshaper::ot::tag::Tag>, stated_rtl: Option<bool>,
    opts: &ShapeOptions,
) -> Option<ShapedRun> {
    use crate::daecore::daeshaper::buffer::Direction;
    use crate::daecore::daeshaper::face::Face;
    use crate::daecore::daeshaper::shape::{guess_segment_properties, shape, shaped_glyphs};
    use crate::daecore::daeshaper::ot::tag::{language_tags, script_tags};

    // Instanced first, so the face needs no axes of its own; the instance is also where upm, the buffer
    // and the metrics below come from.
    let instanced = fc.instanced_font_cache(axis_values);
    let face = Face::new(&instanced, &[]).with_point_size(opts.point_size);

    let declared = instanced.table_map.get("head")
        .filter(|h| h.len() >= 20)
        .and_then(|h| crate::daecore::daetype::decoder::read_u16_be(h, 18));
    // A font whose head declares no usable unitsPerEm is refused rather than measured on a guess.
    // The face substitutes a default so shaping can proceed at all, which is right for it and wrong
    // here – a caller asking for measurements deserves to be told the font is unusable.
    if declared.is_none_or(|v| v == 0) { return None; }

    let upm = face.units_per_em() as f64;
    if upm <= 0.0 { return None; }
    let scale = 1000.0 / upm;

    let mut buf = instanced.take_buffer();
    buf.push_str(text);
    // Set before guessing, which is what keeps it: `guess_segment_properties` is
    // `buffer.script.or_else(scan)`, so a seeded script survives and only an unset one is scanned for.
    buf.script = opts.seed_script;
    buf.cluster_level = opts.cluster_level;
    buf.beginning_of_text = opts.beginning_of_text;
    buf.produce_unsafe_to_concat = opts.report_unsafe_to_concat;
    buf.produce_safe_to_insert_tatweel = opts.report_tatweel_positions;
    buf.preserve_default_ignorables = opts.ignorables == Ignorables::Preserve;
    buf.remove_default_ignorables = opts.ignorables == Ignorables::Remove;
    buf.invisible = opts.invisible_glyph;
    buf.insert_dotted_circle = !opts.suppress_dotted_circle;
    let before: Vec<u32> = opts.before.chars().map(u32::from).collect();
    let after: Vec<u32> = opts.after.chars().map(u32::from).collect();
    buf.set_pre_context(&before);
    buf.set_post_context(&after);

    let guessed = guess_segment_properties(&mut buf);
    let direction = if vertical {
        Direction::TopToBottom
    } else {
        match stated_rtl {
            Some(true) => Direction::RightToLeft,
            Some(false) => Direction::LeftToRight,
            None => guessed,
        }
    };
    buf.direction = direction;

    let tags = buf.script.map(script_tags);
    let forced = script_override.map(|t| [t]);
    let script = match &forced {
        Some(one) => &one[..],
        None => tags.as_ref().map_or(&[][..], |t| t.as_slice()),
    };
    let language = language.map(language_tags);
    let language: &[_] = language.as_ref().map_or(&[][..], |t| t.as_slice());

    let plan = instanced.shape_plan_cached(buf.script, &face, direction, script, language, features,
                                           lookup_overrides, &[]);
    shape(&face, &plan, &mut buf, direction);

    let ascii = text.is_ascii();
    let byte_offsets: Vec<u32> = if ascii {
        Vec::new()
    } else {
        text.char_indices().map(|(bi, _)| bi as u32).collect()
    };
    let to_char = |cluster: u32| -> u32 {
        if ascii {
            return if (cluster as usize) < text.len() { cluster } else { 0 };
        }
        byte_offsets.binary_search(&cluster).map_or(0, |i| i as u32)
    };

    let out = shaped_glyphs(&buf);
    let complete = buf.successful && !buf.shaping_failed;
    let has_broken_syllable = buf.scratch_flags
        & crate::daecore::daeshaper::buffer::scratch_flags::HAS_BROKEN_SYLLABLE
        != 0;
    instanced.give_buffer(buf);
    let mut glyphs   = Vec::with_capacity(out.len());
    let mut advances = Vec::with_capacity(out.len());
    let mut offsets  = Vec::with_capacity(out.len());
    let mut unsafe_to_break = Vec::with_capacity(out.len());
    let mut unsafe_to_concat =
        Vec::with_capacity(if opts.report_unsafe_to_concat { out.len() } else { 0 });
    let mut safe_to_insert_tatweel =
        Vec::with_capacity(if opts.report_tatweel_positions { out.len() } else { 0 });
    let mut clusters = Vec::with_capacity(out.len());
    let in_range = |gid: u16| match instanced.font_num_glyphs() {
        Some(n) if gid >= n => 0,
        _ => gid,
    };
    for g in &out {
        glyphs.push(in_range(g.glyph_id));
        let raw = if vertical { g.y_advance.saturating_neg() } else { g.x_advance };
        advances.push(raw as f64 * scale);
        offsets.push((g.x_offset as f64 * scale, g.y_offset as f64 * scale));
        unsafe_to_break
            .push(g.flags & crate::daecore::daeshaper::buffer::glyph_flag::UNSAFE_TO_BREAK != 0);
        if opts.report_unsafe_to_concat {
            unsafe_to_concat
                .push(g.flags & crate::daecore::daeshaper::buffer::glyph_flag::UNSAFE_TO_CONCAT != 0);
        }
        if opts.report_tatweel_positions {
            safe_to_insert_tatweel.push(
                g.flags & crate::daecore::daeshaper::buffer::glyph_flag::SAFE_TO_INSERT_TATWEEL != 0,
            );
        }
        clusters.push(to_char(g.cluster));
    }
    Some(ShapedRun {
        glyphs, advances, offsets, unsafe_to_break, unsafe_to_concat, safe_to_insert_tatweel,
        clusters, complete, has_broken_syllable, shaper: plan.shaper_name(),
    })
}

pub fn run_is_rtl(text: &str, vertical: bool) -> bool {
    if vertical { return false; }
    use crate::daecore::daeshaper::buffer::{Buffer, Direction};
    use crate::daecore::daeshaper::shape::guess_segment_properties;
    let mut buf = Buffer::new();
    buf.push_str(text);
    guess_segment_properties(&mut buf) == Direction::RightToLeft
}
