use alloc::vec::Vec;

use super::ot::apply::{apply_string, ApplyContext};
use super::buffer::{glyph_flag, glyph_props, scratch_flags, Buffer, ClusterLevel, Direction, GlyphInfo};
use super::face::Face;
use super::fallback;
use super::ot::gpos;
use super::ot::gsub;
use super::ot::kern;
use super::ot::kerx;
use super::ot::map::TableIndex;
use super::normalize;
use super::ot::LayoutTable;
use super::plan::ShapePlan;
use super::script::ZeroWidthMarks;
use super::unicode::{self, GeneralCategory};

pub fn shape(
    face: &Face,
    plan: &ShapePlan,
    buffer: &mut Buffer,
    target_direction: Direction,
) {
    buffer.enter();
    buffer.direction = target_direction;

    buffer.reset_masks(plan.map.global_mask);
    set_unicode_props(buffer);
    insert_dotted_circle(buffer, face);
    form_clusters(buffer);
    ensure_native_direction(buffer);

    if let Some(func) = plan.shaper.preprocess_text {
        func(plan, face, buffer);
    }

    let used_morx = substitute(face, plan, buffer, target_direction);
    position(face, plan, buffer);

    if used_morx && !plan.apply_gpos {
        super::ot::morx::remove_deleted_glyphs(buffer);
    }

    deal_with_variation_selectors(buffer);
    hide_default_ignorables(buffer, face);

    if let Some(func) = plan.shaper.postprocess_glyphs {
        func(plan, face, buffer);
    }

    propagate_flags(buffer);

    buffer.direction = target_direction;
    buffer.leave();
}

fn substitute(face: &Face, plan: &ShapePlan, buffer: &mut Buffer, target_direction: Direction) -> bool {
    rotate_chars(face, plan, buffer, target_direction);

    let defaults = normalize::Hooks::default();
    let hooks = normalize::Hooks {
        mode: plan.shaper.normalization_preference,
        decompose: plan.shaper.decompose.unwrap_or(defaults.decompose),
        compose: plan.shaper.compose.unwrap_or(defaults.compose),
        reorder_marks: plan.shaper.reorder_marks,
    };
    normalize::normalize(buffer, face, &hooks, plan.has_gpos_mark);

    setup_masks(face, plan, buffer);

    if plan.fallback_mark_positioning {
        fallback::recategorize_marks(buffer);
    }

    map_glyphs(buffer);

    let len = buffer.len;
    for info in &mut buffer.info[..len] {
        info.glyph_props = face.glyph_props(info.id as u16);
        info.lig_props = 0;
    }
    if plan.fallback_glyph_classes {
        synthesize_glyph_classes(buffer);
    }

    let vertical = matches!(target_direction, Direction::TopToBottom | Direction::BottomToTop);
    let morx_data = face.table("morx");
    let use_morx = morx_data.is_some() && (!vertical || face.table("GSUB").is_none());

    if use_morx {
        if let Some(data) = morx_data {
            let num_glyphs = face
                .table("maxp")
                .and_then(|maxp| crate::daecore::daetype::decoder::read_u16_be(maxp, 4))
                .unwrap_or(u16::MAX);
            super::ot::morx::apply(data, buffer, num_glyphs);
            if plan.apply_gpos {
            // Only when GPOS will run: it is indexed by glyph and would read the deletion sentinel
            // as a real one. When AAT positions instead they stay, because a deletion is a separator
            // and the glyphs either side of one are not adjacent for kerning.
                super::ot::morx::remove_deleted_glyphs(buffer);
            }
        }
    } else {
        // Deliberately ungated by `plan.apply_gsub`: the table is optional and the walk is not. A
        // shaper's pauses – Indic's reordering, the syllabic dotted circle – are stages of the
        // *plan*, and skipping the walk would leave a broken cluster in a GSUB-less font.
        let data = face.table("GSUB");
        let table = data.and_then(LayoutTable::parse);
        run_lookups(face, plan, buffer, table.as_ref(), TableIndex::Gsub);
    }

    use_morx
}

fn position(face: &Face, plan: &ShapePlan, buffer: &mut Buffer) {
    buffer.clear_positions();
    position_default(face, buffer);

    if buffer.scratch_flags & scratch_flags::HAS_SPACE_FALLBACK != 0 {
        fallback::adjust_spaces(face, buffer);
    }

    let adjust_offsets =
        plan.adjust_mark_positioning_when_zeroing && !buffer.direction.is_backward();

    gpos::position_start(buffer);

    if plan.zero_marks && plan.shaper.zero_width_marks == ZeroWidthMarks::ByGdefEarly {
        zero_mark_widths(buffer, adjust_offsets);
    }

    if plan.apply_gpos {
        let data = face.table("GPOS");
        let table = data.and_then(LayoutTable::parse);
        run_lookups(face, plan, buffer, table.as_ref(), TableIndex::Gpos);
    // `kerx` runs only when GPOS does not. The two flags being set together is not the same as both
    // running: the plan sets `apply_kerx` whenever GPOS has no kerning of its own, and running both
    // kerns pairs GPOS deliberately leaves alone, as in Hiragino.
    } else if plan.apply_kerx {
        kerx::apply(face, buffer, plan.kern_mask, plan.kern_mask != 0);
    }

    if plan.apply_kern {
        kern::apply(face, buffer, plan.kern_mask, plan.kern_mask != 0);
    }

    if plan.zero_marks && plan.shaper.zero_width_marks == ZeroWidthMarks::ByGdefLate {
        zero_mark_widths(buffer, adjust_offsets);
    }

    zero_width_default_ignorables(buffer);
    gpos::position_finish_offsets(buffer);

    if plan.fallback_mark_positioning {
        fallback::position_marks(plan, face, buffer, adjust_offsets);
    }

    apply_tracking(face, buffer);

    if buffer.direction.is_backward() {
        buffer.reverse();
    }
}

fn apply_tracking(face: &Face, buffer: &mut Buffer) {
    let horizontal = buffer.direction.is_horizontal();
    let adjust = face.tracking(horizontal);
    if adjust == 0 {
        return;
    }

    let mut start = 0;
    while start < buffer.len {
        let end = buffer.group_end(start, |_, b| b.is_continuation());
        if horizontal {
            buffer.pos[start].x_advance += adjust;
        } else {
            buffer.pos[start].y_advance += adjust;
        }
        start = end;
    }
}

fn run_lookups(
    face: &Face,
    plan: &ShapePlan,
    buffer: &mut Buffer,
    table: Option<&LayoutTable>,
    which: TableIndex,
) {
    let dispatch = match which {
        TableIndex::Gsub => gsub::apply as super::ot::apply::LookupDispatch,
        TableIndex::Gpos => gpos::apply as super::ot::apply::LookupDispatch,
    };
    let in_place = match which {
        TableIndex::Gsub => gsub::IN_PLACE,
        TableIndex::Gpos => gpos::IN_PLACE,
    };

    let mut ctx = ApplyContext::new(which, face, buffer, dispatch);
    match which {
        TableIndex::Gsub => ctx.gsub = table,
        TableIndex::Gpos => ctx.gpos = table,
    }

    let mut buffer_digest = super::ot::digest::Digest::new();
    for info in &ctx.buffer.info[..ctx.buffer.len] {
        buffer_digest.add(info.id as u16);
    }

    let digests = &plan.lookup_digests[which.idx()];

    let stages = plan.map.stages(which).len();
    for stage in 0..stages {
        let mut mask_union = 0;
        for info in &ctx.buffer.info[..ctx.buffer.len] {
            mask_union |= info.mask;
        }

        for lookup in table.iter().flat_map(|_| plan.map.stage_lookups(which, stage)) {
            if !ctx.buffer.successful || ctx.buffer.shaping_failed {
                break;
            }

            debug_assert!(
                ctx.buffer.info[..ctx.buffer.len]
                    .iter()
                    .all(|i| i.mask & !glyph_flag::DEFINED & !mask_union == 0),
                "a glyph carries a feature bit the stage's mask union does not; something set masks \
                 within a stage without the union being widened",
            );

            if lookup.mask & mask_union == 0 {
                continue;
            }

            if digests
                .get(lookup.index as usize)
                .is_some_and(|d| !d.may_intersect(&buffer_digest))
            {
                continue;
            }

            let Some(l) = table.and_then(|t| t.lookup(lookup.index)) else { continue };

            ctx.lookup_index = lookup.index;
            ctx.glyph_digest = digests.get(lookup.index as usize).copied();
            ctx.subtable_indexes =
                plan.subtable_indexes[which.idx()].get(lookup.index as usize).cloned().flatten();
            ctx.set_joiners(lookup.auto_zwnj, lookup.auto_zwj);
            ctx.random = lookup.random;
            ctx.per_syllable = which == TableIndex::Gsub && lookup.per_syllable;
            ctx.set_lookup_mask(lookup.mask);

            let reverse = match which {
                TableIndex::Gsub => gsub::is_reverse(&l),
                TableIndex::Gpos => gpos::is_reverse(&l),
            };
            let applied = apply_string(&mut ctx, lookup.index, in_place, reverse);

            if which == TableIndex::Gsub && applied {
                for info in &ctx.buffer.info[..ctx.buffer.len] {
                    buffer_digest.add(info.id as u16);
                    mask_union |= info.mask;
                }
            }
        }

        if let Some(at) = plan.map.stages(which).get(stage).and_then(|s| s.pause_index)
            && let Some(func) = plan.shaper.pauses.get(at)
            && func(plan, face, ctx.buffer)
        {
            for info in &ctx.buffer.info[..ctx.buffer.len] {
                buffer_digest.add(info.id as u16);
            }
        }
    }
}

fn set_unicode_props(buffer: &mut Buffer) {
    let len = buffer.len;
    let mut flags = buffer.scratch_flags;
    let mut i = 0;

    while i < len {
        buffer.info[i].init_unicode_props(&mut flags);
        let gc = GeneralCategory::from_stored(buffer.info[i].general_category());
        let id = buffer.info[i].id;

        if gc.is_letter() || gc == GeneralCategory::SpaceSeparator {
            i += 1;
            continue;
        }

        if gc == GeneralCategory::ModifierSymbol && (0x1F3FB..=0x1F3FF).contains(&id) {
            buffer.info[i].set_continuation();
        } else if i != 0 && (0x1F1E6..=0x1F1FF).contains(&id) {
            let prev = &buffer.info[i - 1];
            if (0x1F1E6..=0x1F1FF).contains(&prev.id) && !prev.is_continuation() {
                buffer.info[i].set_continuation();
            }
        } else if buffer.info[i].is_zwj() {
            buffer.info[i].set_continuation();
            if i + 1 < len {
                let next_is_picto =
                    char::from_u32(buffer.info[i + 1].id).is_some_and(unicode::is_extended_pictographic);
                if next_is_picto {
                    buffer.info[i + 1].init_unicode_props(&mut flags);
                    buffer.info[i + 1].set_continuation();
                    i += 1;
                }
            }
        } else if matches!(id, 0xFF9E..=0xFF9F | 0xE0020..=0xE007F) {
            buffer.info[i].set_continuation();
        }

        i += 1;
    }

    if flags & scratch_flags::HAS_NON_ASCII != 0
        && buffer.info[..len].iter().any(|i| i.is_continuation())
    {
        flags |= scratch_flags::HAS_CONTINUATIONS;
    }

    buffer.scratch_flags = flags;
}

fn insert_dotted_circle(buffer: &mut Buffer, face: &Face) {
    if buffer.is_empty()
        || !buffer.insert_dotted_circle
        || !buffer.beginning_of_text
        || buffer.context_len[0] != 0
    {
        return;
    }
    if !GeneralCategory::from_stored(buffer.info[0].general_category()).is_mark() {
        return;
    }
    if !face.has_glyph(0x25CC) {
        return;
    }

    let mut info = GlyphInfo::new(0x25CC, buffer.info[0].cluster);
    info.mask = buffer.info[0].mask;
    let mut flags = buffer.scratch_flags;
    info.init_unicode_props(&mut flags);
    buffer.scratch_flags = flags;

    buffer.clear_output();
    buffer.output_info(info);
    buffer.sync();
}

fn form_clusters(buffer: &mut Buffer) {
    if buffer.scratch_flags & scratch_flags::HAS_CONTINUATIONS == 0 {
        return;
    }

    let mut start = 0;
    while start < buffer.len {
        let end = buffer.group_end(start, |_, b| b.is_continuation());
        buffer.merge_grapheme_clusters(start, end);
        start = end;
    }
}

fn ensure_native_direction(buffer: &mut Buffer) {
    let dir = buffer.direction;
    let script = buffer.script;
    let mut hor = script.and_then(unicode::horizontal_direction);

    if hor == Some(Direction::RightToLeft) && dir == Direction::LeftToRight {
        let mut found_number = false;
        let mut found_letter = false;
        let mut found_ri = false;

        for info in &buffer.info[..buffer.len] {
            let gc = GeneralCategory::from_stored(info.general_category());
            if gc == GeneralCategory::DecimalNumber {
                found_number = true;
            } else if gc.is_letter() {
                found_letter = true;
                break;
            } else if (0x1F1E6..=0x1F1FF).contains(&info.id) {
                found_ri = true;
            }
        }

        if (found_number || found_ri) && !found_letter {
            hor = Some(Direction::LeftToRight);
        }
    }

    let flip = match (dir.is_horizontal(), hor) {
        (true, Some(h)) => dir != h,
        (false, _) => dir != Direction::TopToBottom,
        (true, None) => false,
    };

    if flip {
        reverse_graphemes(buffer);
        buffer.direction = buffer.direction.reverse();
    }
}

fn reverse_graphemes(buffer: &mut Buffer) {
    let characters = buffer.cluster_level == ClusterLevel::MonotoneCharacters;
    let mut start = 0;
    while start < buffer.len {
        let end = buffer.group_end(start, |_, b| b.is_continuation());
        if characters {
            buffer.merge_clusters(start, end);
        }
        buffer.reverse_range(start, end);
        start = end;
    }
    buffer.reverse();
}

fn rotate_chars(face: &Face, plan: &ShapePlan, buffer: &mut Buffer, target: Direction) {
    let len = buffer.len;

    if target.is_backward() {
        for info in &mut buffer.info[..len] {
            let mirrored = char::from_u32(info.id)
                .and_then(unicode::mirrored)
                .map(|c| c as u32)
                .filter(|&c| face.has_glyph(c));
            match mirrored {
                Some(c) => info.id = c,
                None => info.mask |= plan.rtlm_mask,
            }
        }
    }

    if target.is_vertical() && !plan.has_vert {
        for info in &mut buffer.info[..len] {
            let vertical = char::from_u32(info.id)
                .and_then(unicode::vertical_form)
                .map(|c| c as u32)
                .filter(|&c| face.has_glyph(c));
            if let Some(c) = vertical {
                info.id = c;
            }
        }
    }
}

fn setup_masks(face: &Face, plan: &ShapePlan, buffer: &mut Buffer) {
    setup_fraction_masks(plan, buffer);

    if let Some(func) = plan.shaper.setup_masks {
        func(plan, face, buffer);
    }

    for feature in &plan.user_features {
        if !feature.is_global() {
            let (mask, shift) = plan.map.mask(feature.tag);
            buffer.set_masks(feature.value << shift, mask, feature.start, feature.end);
        }
    }
}

fn setup_fraction_masks(plan: &ShapePlan, buffer: &mut Buffer) {
    if buffer.scratch_flags & scratch_flags::HAS_NON_ASCII == 0 || !plan.has_frac {
        return;
    }

    let (pre_mask, post_mask) = if buffer.direction.is_backward() {
        (plan.frac_mask | plan.dnom_mask, plan.numr_mask | plan.frac_mask)
    } else {
        (plan.numr_mask | plan.frac_mask, plan.frac_mask | plan.dnom_mask)
    };

    let len = buffer.len;
    let is_digit = |info: &GlyphInfo| {
        GeneralCategory::from_stored(info.general_category()) == GeneralCategory::DecimalNumber
    };

    let mut i = 0;
    while i < len {
        if buffer.info[i].id != 0x2044 {
            i += 1;
            continue;
        }

        let mut start = i;
        while start > 0 && is_digit(&buffer.info[start - 1]) {
            start -= 1;
        }
        let mut end = i + 1;
        while end < len && is_digit(&buffer.info[end]) {
            end += 1;
        }

        if start == i || end == i + 1 {
            if start == i {
                buffer.unsafe_to_concat(start, start + 1);
            }
            if end == i + 1 {
                buffer.unsafe_to_concat(end - 1, end);
            }
            i += 1;
            continue;
        }

        buffer.unsafe_to_break(start, end);
        for info in &mut buffer.info[start..i] {
            info.mask |= pre_mask;
        }
        buffer.info[i].mask |= plan.frac_mask;
        for info in &mut buffer.info[i + 1..end] {
            info.mask |= post_mask;
        }
        i = end;
    }
}

fn map_glyphs(buffer: &mut Buffer) {
    buffer.map_glyphs();
}

fn synthesize_glyph_classes(buffer: &mut Buffer) {
    let len = buffer.len;
    for info in &mut buffer.info[..len] {
        let is_mark =
            GeneralCategory::from_stored(info.general_category()) == GeneralCategory::NonspacingMark
                && !info.is_default_ignorable();

        info.glyph_props = if is_mark { glyph_props::MARK } else { glyph_props::BASE_GLYPH };
    }
}

fn position_default(face: &Face, buffer: &mut Buffer) {
    let len = buffer.len;

    if buffer.direction.is_horizontal() {
        let advances = face.advances_table(false);
        for i in 0..len {
            let raw = advances.get(buffer.info[i].id as usize).copied().unwrap_or(0);
            buffer.pos[i].x_advance = raw as i32;
        }
    } else {
        let advances = face.advances_table(true);
        for i in 0..len {
            let glyph = buffer.info[i].id as u16;
            let raw = advances.get(glyph as usize).copied().unwrap_or(0);
            buffer.pos[i].y_advance = -face.v_advance_or_line_height(raw);
            buffer.pos[i].x_offset -= face.glyph_h_origin(glyph);
            buffer.pos[i].y_offset -= face.glyph_v_origin(glyph);
        }
    }
}

fn zero_mark_widths(buffer: &mut Buffer, adjust_offsets: bool) {
    let len = buffer.len;
    for i in 0..len {
        if !buffer.info[i].is_mark() {
            continue;
        }
        if adjust_offsets {
            buffer.pos[i].x_offset -= buffer.pos[i].x_advance;
            buffer.pos[i].y_offset -= buffer.pos[i].y_advance;
        }
        buffer.pos[i].x_advance = 0;
        buffer.pos[i].y_advance = 0;
    }
}

fn zero_width_default_ignorables(buffer: &mut Buffer) {
    if buffer.scratch_flags & scratch_flags::HAS_DEFAULT_IGNORABLES == 0
        || buffer.preserve_default_ignorables
        || buffer.remove_default_ignorables
    {
        return;
    }

    let len = buffer.len;
    for i in 0..len {
        if buffer.info[i].is_default_ignorable() {
            buffer.pos[i] = Default::default();
        }
    }
}

fn deal_with_variation_selectors(buffer: &mut Buffer) {
    if buffer.scratch_flags & scratch_flags::HAS_VARIATION_SELECTOR_FALLBACK == 0 {
        return;
    }
    let Some(nf) = buffer.not_found_variation_selector else { return };

    let len = buffer.len;
    for i in 0..len {
        if buffer.info[i].is_variation_selector() {
            buffer.info[i].id = u32::from(nf);
            buffer.pos[i] = Default::default();
            buffer.info[i].set_variation_selector(false);
        }
    }
}

fn hide_default_ignorables(buffer: &mut Buffer, face: &Face) {
    if buffer.scratch_flags & scratch_flags::HAS_DEFAULT_IGNORABLES == 0
        || buffer.preserve_default_ignorables
    {
        return;
    }

    if !buffer.remove_default_ignorables
        && let Some(invisible) = buffer.invisible.or_else(|| face.glyph_index(0x20)) {
            let len = buffer.len;
            for info in &mut buffer.info[..len] {
                if info.is_default_ignorable() {
                    info.id = u32::from(invisible);
                }
            }
            return;
        }

    buffer.delete_glyphs_inplace(GlyphInfo::is_default_ignorable);
}

fn propagate_flags(buffer: &mut Buffer) {
    if buffer.scratch_flags & scratch_flags::HAS_GLYPH_FLAGS == 0 {
        return;
    }

    let clear_concat = !buffer.produce_unsafe_to_concat;
    let tatweel = buffer.produce_safe_to_insert_tatweel;
    let mut start = 0;
    while start < buffer.len {
        let end = buffer.group_end(start, |a, b| a.cluster == b.cluster);

        let mut mask = 0;
        for info in &buffer.info[start..end] {
            mask |= info.mask & glyph_flag::DEFINED;
        }

        if tatweel {
            if mask & glyph_flag::UNSAFE_TO_BREAK != 0 {
                mask &= !glyph_flag::SAFE_TO_INSERT_TATWEEL;
            }
            if mask & glyph_flag::SAFE_TO_INSERT_TATWEEL != 0 {
                mask |= glyph_flag::UNSAFE_TO_BREAK | glyph_flag::UNSAFE_TO_CONCAT;
            }
        }

        if clear_concat {
            mask &= !glyph_flag::UNSAFE_TO_CONCAT;
        }

        for info in &mut buffer.info[start..end] {
            info.mask = mask;
        }
        start = end;
    }
}

pub fn guess_segment_properties(buffer: &mut Buffer) -> Direction {
    let script = buffer.script.or_else(|| {
        buffer.info[..buffer.len].iter().find_map(|info| {
            let c = char::from_u32(info.id)?;
            let s = unicode::script(c);
            match s.name() {
                "Common" | "Inherited" | "Unknown" => None,
                _ => Some(s),
            }
        })
    });

    buffer.script = script;
    script
        .and_then(unicode::horizontal_direction)
        .unwrap_or(Direction::LeftToRight)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub flags: u32,
}

pub fn shaped_glyphs(buffer: &Buffer) -> Vec<ShapedGlyph> {
    (0..buffer.len)
        .map(|i| ShapedGlyph {
            glyph_id: buffer.info[i].id as u16,
            cluster: buffer.info[i].cluster,
            x_advance: buffer.pos[i].x_advance,
            y_advance: buffer.pos[i].y_advance,
            x_offset: buffer.pos[i].x_offset,
            y_offset: buffer.pos[i].y_offset,
            flags: buffer.info[i].mask & glyph_flag::DEFINED,
        })
        .collect()
}
