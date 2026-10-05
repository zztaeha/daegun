use alloc::vec::Vec;

use super::unicode::GeneralCategory;

pub(crate) type Mask = u32;

const IS_LIG_BASE: u8 = 0x10;

pub(crate) mod glyph_flag {
    pub(crate) const UNSAFE_TO_BREAK: u32 = 0x0000_0001;
    pub(crate) const UNSAFE_TO_CONCAT: u32 = 0x0000_0002;
    pub(crate) const SAFE_TO_INSERT_TATWEEL: u32 = 0x0000_0004;
    pub(crate) const DEFINED: u32 = 0x0000_0007;
}

pub(crate) mod scratch_flags {
    pub(crate) const HAS_NON_ASCII: u32 = 0x0000_0001;
    pub(crate) const HAS_DEFAULT_IGNORABLES: u32 = 0x0000_0002;
    pub(crate) const HAS_SPACE_FALLBACK: u32 = 0x0000_0004;
    pub(crate) const HAS_GPOS_ATTACHMENT: u32 = 0x0000_0008;
    pub(crate) const HAS_CGJ: u32 = 0x0000_0010;
    pub(crate) const HAS_GLYPH_FLAGS: u32 = 0x0000_0020;
    pub(crate) const HAS_BROKEN_SYLLABLE: u32 = 0x0000_0040;
    pub(crate) const HAS_VARIATION_SELECTOR_FALLBACK: u32 = 0x0000_0080;
    pub(crate) const ARABIC_HAS_STCH: u32 = 0x0000_0100;
    pub(crate) const HAS_CONTINUATIONS: u32 = 0x0000_0200;
}

pub(crate) mod unicode_props {
    pub(crate) const GENERAL_CATEGORY: u16 = 0x001F;
    pub(crate) const IGNORABLE: u16 = 0x0020;
    pub(crate) const HIDDEN: u16 = 0x0040;
    pub(crate) const CONTINUATION: u16 = 0x0080;
    pub(crate) const CF_ZWJ: u16 = 0x0100;
    pub(crate) const CF_ZWNJ: u16 = 0x0200;
    pub(crate) const CF_VS: u16 = 0x0400;
    pub(crate) const CCC_SHIFT: u32 = 8;
}

pub(crate) mod glyph_props {
    pub(crate) const BASE_GLYPH: u16 = 0x02;
    pub(crate) const LIGATURE: u16 = 0x04;
    pub(crate) const MARK: u16 = 0x08;

    pub(crate) const SUBSTITUTED: u16 = 0x10;
    pub(crate) const LIGATED: u16 = 0x20;
    pub(crate) const MULTIPLIED: u16 = 0x40;
    pub(crate) const PRESERVE: u16 = SUBSTITUTED | LIGATED | MULTIPLIED;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Direction {
    #[default]
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

impl Direction {
    pub(crate) fn is_horizontal(self) -> bool {
        matches!(self, Direction::LeftToRight | Direction::RightToLeft)
    }

    pub(crate) fn is_vertical(self) -> bool {
        !self.is_horizontal()
    }

    pub(crate) fn is_backward(self) -> bool {
        matches!(self, Direction::RightToLeft | Direction::BottomToTop)
    }

    pub(crate) fn reverse(self) -> Direction {
        match self {
            Direction::LeftToRight => Direction::RightToLeft,
            Direction::RightToLeft => Direction::LeftToRight,
            Direction::TopToBottom => Direction::BottomToTop,
            Direction::BottomToTop => Direction::TopToBottom,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClusterLevel {
    #[default]
    MonotoneGraphemes,
    MonotoneCharacters,
    Characters,
    Graphemes,
}

impl ClusterLevel {
    pub fn is_monotone(self) -> bool {
        matches!(self, Self::MonotoneGraphemes | Self::MonotoneCharacters)
    }

    pub fn is_graphemes(self) -> bool {
        matches!(self, Self::MonotoneGraphemes | Self::Graphemes)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlyphInfo {
    pub(crate) id: u32,
    pub(crate) mask: Mask,
    pub(crate) cluster: u32,

    pub(crate) unicode_props: u16,
    pub(crate) glyph_props: u16,
    pub(crate) lig_props: u8,
    pub(crate) syllable: u8,
    pub(crate) shaper_category: u8,
    pub(crate) shaper_auxiliary: u8,
    pub(crate) glyph_index: u32,
}

impl GlyphInfo {
    pub fn new(codepoint: u32, cluster: u32) -> Self {
        GlyphInfo { id: codepoint, cluster, ..GlyphInfo::default() }
    }

    pub fn general_category(&self) -> u16 {
        self.unicode_props & unicode_props::GENERAL_CATEGORY
    }

    pub(crate) fn set_general_category(&mut self, gc: u16) {
        self.unicode_props =
            (gc & unicode_props::GENERAL_CATEGORY) | (self.unicode_props & !unicode_props::GENERAL_CATEGORY);
    }

    pub(crate) fn modified_combining_class(&self) -> u8 {
        if !self.is_unicode_mark() {
            return 0;
        }
        (self.unicode_props >> unicode_props::CCC_SHIFT) as u8
    }

    pub(crate) fn is_unicode_mark(&self) -> bool {
        GeneralCategory::from_stored(self.general_category()).is_mark()
    }

    pub(crate) fn set_modified_combining_class(&mut self, ccc: u8) {
        let low = self.unicode_props & ((1 << unicode_props::CCC_SHIFT) - 1);
        self.unicode_props = low | ((ccc as u16) << unicode_props::CCC_SHIFT);
    }

    pub(crate) fn is_default_ignorable(&self) -> bool {
        self.unicode_props & unicode_props::IGNORABLE != 0 && !self.substituted()
    }

    pub(crate) fn clear_default_ignorable(&mut self) {
        self.unicode_props &= !unicode_props::IGNORABLE;
    }

    pub(crate) fn is_hidden(&self) -> bool {
        self.unicode_props & unicode_props::HIDDEN != 0
    }

    pub(crate) fn unhide(&mut self) {
        self.unicode_props &= !unicode_props::HIDDEN;
    }

    pub(crate) fn init_unicode_props(&mut self, scratch_flags: &mut u32) {
        let Some(u) = char::from_u32(self.id) else { return };
        let gc = super::unicode::general_category(u);
        let mut props = gc as u16;

        if self.id >= 0x80 {
            *scratch_flags |= scratch_flags::HAS_NON_ASCII;

            if super::unicode::is_default_ignorable(u) {
                props |= unicode_props::IGNORABLE;
                *scratch_flags |= scratch_flags::HAS_DEFAULT_IGNORABLES;

                match self.id {
                    0x200C => props |= unicode_props::CF_ZWNJ,
                    0x200D => props |= unicode_props::CF_ZWJ,
                    0x180B..=0x180D | 0x180F => props |= unicode_props::HIDDEN,
                    0xE0020..=0xE007F => props |= unicode_props::HIDDEN,
                    0x034F => {
                        props |= unicode_props::HIDDEN;
                        *scratch_flags |= scratch_flags::HAS_CGJ;
                    }
                    _ => {}
                }
            }

            if gc.is_mark() {
                props |= unicode_props::CONTINUATION;
                props |= u16::from(super::unicode::modified_combining_class(u)) << unicode_props::CCC_SHIFT;
            }
        }

        self.unicode_props = props;
    }

    fn is_unicode_space(&self) -> bool {
        self.general_category() == GeneralCategory::SpaceSeparator as u16
    }

    pub(crate) fn set_space_fallback(&mut self, s: super::unicode::SpaceFallback) {
        if !self.is_unicode_space() {
            return;
        }
        let low = self.unicode_props & ((1 << unicode_props::CCC_SHIFT) - 1);
        self.unicode_props = low | (u16::from(s.to_byte()) << unicode_props::CCC_SHIFT);
    }

    pub(crate) fn space_fallback(&self) -> Option<super::unicode::SpaceFallback> {
        if !self.is_unicode_space() {
            return None;
        }
        super::unicode::SpaceFallback::from_byte((self.unicode_props >> unicode_props::CCC_SHIFT) as u8)
    }

    pub(crate) fn is_variation_selector(&self) -> bool {
        self.general_category() == GeneralCategory::Format as u16
            && self.unicode_props & unicode_props::CF_VS != 0
    }

    pub(crate) fn set_variation_selector(&mut self, on: bool) {
        if on {
            self.set_general_category(GeneralCategory::Format as u16);
            self.unicode_props |= unicode_props::CF_VS;
        } else {
            self.set_general_category(GeneralCategory::NonspacingMark as u16);
        }
    }

    pub(crate) fn is_continuation(&self) -> bool {
        self.unicode_props & unicode_props::CONTINUATION != 0
    }

    pub(crate) fn reset_continuation(&mut self) {
        self.unicode_props &= !unicode_props::CONTINUATION;
    }

    pub(crate) fn set_continuation(&mut self) {
        self.unicode_props |= unicode_props::CONTINUATION;
    }

    pub(crate) fn is_mark(&self) -> bool {
        self.glyph_props & glyph_props::MARK != 0
    }

    pub(crate) fn is_base_glyph(&self) -> bool {
        self.glyph_props & glyph_props::BASE_GLYPH != 0
    }

    pub(crate) fn is_ligature(&self) -> bool {
        self.glyph_props & glyph_props::LIGATURE != 0
    }

    pub(crate) fn substituted(&self) -> bool {
        self.glyph_props & glyph_props::SUBSTITUTED != 0
    }

    pub(crate) fn ligated(&self) -> bool {
        self.glyph_props & glyph_props::LIGATED != 0
    }

    pub(crate) fn multiplied(&self) -> bool {
        self.glyph_props & glyph_props::MULTIPLIED != 0
    }

    pub(crate) fn ligated_and_didnt_multiply(&self) -> bool {
        self.ligated() && !self.multiplied()
    }

    pub(crate) fn clear_substituted(&mut self) {
        self.glyph_props &= !glyph_props::SUBSTITUTED;
    }

    pub(crate) fn clear_ligated_and_multiplied(&mut self) {
        self.glyph_props &= !(glyph_props::LIGATED | glyph_props::MULTIPLIED);
    }

    fn is_unicode_format(&self) -> bool {
        self.general_category() == GeneralCategory::Format as u16
    }

    pub(crate) fn is_zwnj(&self) -> bool {
        self.is_unicode_format() && self.unicode_props & unicode_props::CF_ZWNJ != 0
    }

    pub(crate) fn is_zwj(&self) -> bool {
        self.is_unicode_format() && self.unicode_props & unicode_props::CF_ZWJ != 0
    }

    pub(crate) fn lig_id(&self) -> u8 {
        self.lig_props >> 5
    }

    fn is_lig_base(&self) -> bool {
        self.lig_props & IS_LIG_BASE != 0
    }

    pub(crate) fn lig_comp(&self) -> u8 {
        if self.is_lig_base() {
            0
        } else {
            self.lig_props & 0x0F
        }
    }

    pub(crate) fn lig_num_comps(&self) -> u8 {
        if self.is_ligature() && self.is_lig_base() {
            self.lig_props & 0x0F
        } else {
            1
        }
    }

    pub(crate) fn lig_num_comps_in_ligation(&self) -> u8 {
        if self.multiplied() && self.lig_comp() != 0 { 0 } else { self.lig_num_comps() }
    }

    pub(crate) fn set_lig_props_for_ligature(&mut self, lig_id: u8, num_comps: u8) {
        self.lig_props = (lig_id << 5) | IS_LIG_BASE | (num_comps & 0x0F);
    }

    pub(crate) fn set_lig_props_for_mark(&mut self, lig_id: u8, lig_comp: u8) {
        self.lig_props = (lig_id << 5) | (lig_comp & 0x0F);
    }

    pub(crate) fn set_lig_props_for_component(&mut self, comp: u8) {
        self.set_lig_props_for_mark(0, comp);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlyphPosition {
    pub(crate) x_advance: i32,
    pub(crate) y_advance: i32,
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
    pub(crate) attach_chain: i16,
    pub(crate) attach_type: u8,
}

pub struct Buffer {
    pub(crate) info: Vec<GlyphInfo>,
    pub(crate) pos: Vec<GlyphPosition>,
    out_info: Vec<GlyphInfo>,
    out_lockstep: bool,

    pub(crate) idx: usize,
    pub(crate) len: usize,
    pub(crate) out_len: usize,

    pub(crate) have_output: bool,
    pub(crate) have_positions: bool,
    pub(crate) successful: bool,
    pub(crate) shaping_failed: bool,

    pub(crate) direction: Direction,
    pub(crate) invisible: Option<u16>,
    pub(crate) not_found_variation_selector: Option<u16>,
    pub script: Option<super::unicode::Script>,
    pub(crate) beginning_of_text: bool,
    pub(crate) insert_dotted_circle: bool,
    pub(crate) context: [[u32; Self::CONTEXT_LENGTH]; 2],
    pub(crate) context_len: [usize; 2],
    pub(crate) edit_journal: alloc::vec::Vec<(usize, isize)>,
    pub(crate) recording_edits: bool,
    pub(crate) preserve_default_ignorables: bool,
    pub(crate) remove_default_ignorables: bool,
    pub(crate) cluster_level: ClusterLevel,
    pub(crate) produce_unsafe_to_concat: bool,
    pub(crate) produce_safe_to_insert_tatweel: bool,
    pub(crate) scratch_flags: u32,
    pub(crate) max_len: usize,
    pub(crate) max_ops: i32,
    serial: u8,
}

impl Buffer {
    pub(crate) const CONTEXT_LENGTH: usize = 5;
    const MAX_LEN_FACTOR: usize = 64;
    const MAX_LEN_MIN: usize = 16_384;
    const MAX_LEN_DEFAULT: usize = 0x3FFF_FFFF;
    const MAX_OPS_FACTOR: i32 = 1024;
    const MAX_OPS_MIN: i32 = 16_384;
    const MAX_OPS_DEFAULT: i32 = 0x1FFF_FFFF;

    pub fn new() -> Self {
        Buffer {
            info: Vec::new(),
            pos: Vec::new(),
            out_info: Vec::new(),
            out_lockstep: false,
            idx: 0,
            len: 0,
            out_len: 0,
            have_output: false,
            have_positions: false,
            successful: true,
            shaping_failed: false,
            direction: Direction::default(),
            invisible: None,
            not_found_variation_selector: None,
            script: None,
            beginning_of_text: false,
            context: [[0; Self::CONTEXT_LENGTH]; 2],
            context_len: [0, 0],
            insert_dotted_circle: true,
            edit_journal: alloc::vec::Vec::new(),
            recording_edits: false,
            preserve_default_ignorables: false,
            remove_default_ignorables: false,
            cluster_level: ClusterLevel::default(),
            produce_unsafe_to_concat: false,
            produce_safe_to_insert_tatweel: false,
            scratch_flags: 0,
            max_len: Self::MAX_LEN_DEFAULT,
            max_ops: Self::MAX_OPS_DEFAULT,
            serial: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        let mut info = core::mem::take(&mut self.info);
        let mut pos = core::mem::take(&mut self.pos);
        let mut out_info = core::mem::take(&mut self.out_info);
        info.clear();
        pos.clear();
        out_info.clear();
        *self = Buffer { info, pos, out_info, ..Buffer::new() };
    }

    pub(crate) fn set_pre_context(&mut self, codepoints: &[u32]) {
        self.context_len[0] = 0;
        for &c in codepoints.iter().rev().take(Self::CONTEXT_LENGTH) {
            self.context[0][self.context_len[0]] = c;
            self.context_len[0] += 1;
        }
    }

    pub(crate) fn set_post_context(&mut self, codepoints: &[u32]) {
        self.context_len[1] = 0;
        for &c in codepoints.iter().take(Self::CONTEXT_LENGTH) {
            self.context[1][self.context_len[1]] = c;
            self.context_len[1] += 1;
        }
    }

    pub fn push_str(&mut self, text: &str) {
        let _ = self.ensure(self.len + text.chars().count());
        for (byte_idx, ch) in text.char_indices() {
            self.add(ch as u32, byte_idx as u32);
        }
    }

    pub(crate) fn add(&mut self, codepoint: u32, cluster: u32) {
        if !self.ensure(self.len + 1) {
            return;
        }
        self.info[self.len] = GlyphInfo::new(codepoint, cluster);
        self.len += 1;
    }

    #[must_use]
    pub(crate) fn ensure(&mut self, size: usize) -> bool {
        if size <= self.info.len() {
            return true;
        }
        if size > self.max_len {
            self.successful = false;
            return false;
        }
        self.info.resize(size, GlyphInfo::default());
        self.pos.resize(size, GlyphPosition::default());
        if self.out_info.len() < size {
            self.out_info.resize(size, GlyphInfo::default());
        }
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn cur(&self, offset: usize) -> &GlyphInfo {
        &self.info[self.idx + offset]
    }

    pub(crate) fn cur_mut(&mut self, offset: usize) -> &mut GlyphInfo {
        let i = self.idx + offset;
        &mut self.info[i]
    }

    pub(crate) fn out_info(&self) -> &[GlyphInfo] {
        if self.have_output && !self.out_lockstep { &self.out_info } else { &self.info }
    }

    pub(crate) fn out_info_mut(&mut self) -> &mut [GlyphInfo] {
        if self.have_output && !self.out_lockstep { &mut self.out_info } else { &mut self.info }
    }

    fn materialize_out(&mut self) {
        if !self.have_output || !self.out_lockstep {
            return;
        }
        self.out_lockstep = false;
        let n = self.out_len;
        while self.out_info.len() < n {
            let before = self.out_info.len();
            if !self.grow_out() || self.out_info.len() == before {
                break;
            }
        }
        let take = n.min(self.out_info.len()).min(self.info.len());
        self.out_info[..take].copy_from_slice(&self.info[..take]);
    }

    pub(crate) fn cur_pos_mut(&mut self) -> &mut GlyphPosition {
        let i = self.idx;
        &mut self.pos[i]
    }

    pub(crate) fn prev(&self) -> &GlyphInfo {
        let i = self.out_len.saturating_sub(1);
        &self.out_info()[i]
    }

    pub(crate) fn prev_mut(&mut self) -> &mut GlyphInfo {
        let i = self.out_len.saturating_sub(1);
        &mut self.out_info_mut()[i]
    }

    pub(crate) fn backtrack_len(&self) -> usize {
        if self.have_output { self.out_len } else { self.idx }
    }

    pub(crate) fn lookahead_len(&self) -> usize {
        self.len - self.idx
    }

    pub(crate) fn allocate_lig_id(&mut self) -> u8 {
        self.serial = self.serial.wrapping_add(1);
        let id = self.serial & 0x07;
        if id == 0 { self.allocate_lig_id() } else { id }
    }

    pub(crate) fn clear_output(&mut self) {
        self.have_output = true;
        self.have_positions = false;
        self.idx = 0;
        self.out_len = 0;
        self.out_lockstep = true;
    }

    pub(crate) fn clear_positions(&mut self) {
        self.have_output = false;
        self.have_positions = true;
        self.out_len = 0;
        for p in &mut self.pos {
            *p = GlyphPosition::default();
        }
    }

    pub(crate) fn sync(&mut self) -> bool {
        debug_assert!(self.have_output);
        debug_assert!(self.idx <= self.len);

        if !self.successful {
            self.have_output = false;
            self.out_lockstep = false;
            self.out_len = 0;
            self.idx = 0;
                return false;
        }

        self.next_glyphs(self.len - self.idx);

        if self.out_lockstep && self.out_len == self.len {
            self.out_lockstep = false;
        } else {
            self.materialize_out();
            core::mem::swap(&mut self.info, &mut self.out_info);
            self.len = self.out_len;
            if self.pos.len() < self.info.len() {
                self.pos.resize(self.info.len(), GlyphPosition::default());
            }
        }

        self.have_output = false;
        self.out_len = 0;
        self.idx = 0;
        // Not an unconditional `true`: the flush above can hit the ceiling, where `push_out` clears
        // `successful` and drops the rest, and success would hand back a truncated buffer as whole.
        self.successful
    }

    fn record_edit(&mut self, delta: isize) {
        if self.recording_edits && delta != 0 {
            self.edit_journal.push((self.out_len, delta));
        }
    }

    fn push_out(&mut self, info: GlyphInfo) {
        debug_assert!(!self.out_lockstep, "push_out writes out_info, which lockstep does not own");
        if self.out_len >= self.out_info.len() && !self.grow_out() {
            return;
        }
        self.out_info[self.out_len] = info;
        self.out_len += 1;
    }

    // Bounded by the input side's ceiling too, which `ensure` checks only on input: substitutions that
    // expand without limit would otherwise run until memory does, 294,915 glyphs from three.
    #[cold]
    #[inline(never)]
    fn grow_out(&mut self) -> bool {
        if self.out_len >= self.max_len {
            self.successful = false;
            return false;
        }
        let new_len = (self.out_len + (self.out_len >> 1) + 32).min(self.max_len);
        self.out_info.resize(new_len, GlyphInfo::default());
        true
    }

    pub(crate) fn next_glyph(&mut self) {
        if self.have_output {
            if self.out_lockstep && self.out_len == self.idx {
                self.out_len += 1;
            } else {
                self.materialize_out();
                let info = self.info[self.idx];
                self.push_out(info);
            }
        }
        self.idx += 1;
    }

    pub(crate) fn next_glyphs(&mut self, n: usize) {
        if !self.have_output {
            self.idx += n;
            return;
        }
        if self.out_lockstep && self.out_len == self.idx {
            let take = n.min(self.info.len().saturating_sub(self.idx));
            self.out_len += take;
            self.idx += n;
            return;
        }
        self.materialize_out();
        // The second exit is what makes this terminate: `grow_out` clamps to `max_len` and reports
        // success, so once the output is *at* the ceiling with `out_len` still under it, it keeps
        // returning true without growing and the length test never clears.
        while self.out_len + n > self.out_info.len() {
            let before = self.out_info.len();
            if !self.grow_out() || self.out_info.len() == before {
                break;
            }
        }
        let take = n
            .min(self.out_info.len().saturating_sub(self.out_len))
            .min(self.info.len().saturating_sub(self.idx));
        if take > 0 {
            let (src, dst) = (self.idx, self.out_len);
            self.out_info[dst..dst + take].copy_from_slice(&self.info[src..src + take]);
            self.out_len += take;
        }
        self.idx += n;
    }

    pub(crate) fn skip_glyph(&mut self) {
        self.materialize_out();
        self.record_edit(-1);
        self.idx += 1;
    }

    pub(crate) fn replace_glyph(&mut self, glyph_id: u32) {
        if self.out_lockstep && self.out_len == self.idx {
            self.info[self.idx].id = glyph_id;
            self.out_len += 1;
            self.idx += 1;
            return;
        }
        self.materialize_out();
        let mut info = self.info[self.idx];
        info.id = glyph_id;
        self.push_out(info);
        self.idx += 1;
    }

    pub(crate) fn output_glyph(&mut self, glyph_id: u32) {
        self.materialize_out();
        self.record_edit(1);
        let mut info = if self.idx < self.len {
            self.info[self.idx]
        } else if self.out_len > 0 {
            self.out_info[self.out_len - 1]
        } else {
            return;
        };
        info.id = glyph_id;
        self.push_out(info);
    }

    pub(crate) fn output_info(&mut self, info: GlyphInfo) {
        self.materialize_out();
        self.record_edit(1);
        self.push_out(info);
    }

    pub(crate) fn replace_glyphs(&mut self, num_in: usize, glyph_data: &[u32]) {
        debug_assert!(self.idx + num_in <= self.len);
        self.materialize_out();
        self.record_edit(glyph_data.len() as isize - num_in as isize);
        self.merge_clusters(self.idx, self.idx + num_in);
        let orig = self.info[self.idx];
        for &g in glyph_data {
            let mut info = orig;
            info.id = g;
            self.push_out(info);
        }
        self.idx += num_in;
    }

    fn shift_forward(&mut self, count: usize) -> bool {
        if !self.ensure(self.len + count) {
            return false;
        }
        self.info.copy_within(self.idx..self.len, self.idx + count);
        self.len += count;
        self.idx += count;
        true
    }

    pub(crate) fn move_to(&mut self, i: usize) -> bool {
        if !self.have_output {
            debug_assert!(i <= self.len);
            self.idx = i;
            return true;
        }
        if !self.successful {
            return false;
        }

        if self.out_len < i {
            let count = i - self.out_len;
            if self.idx + count > self.len {
                return false;
            }
            self.next_glyphs(count);
        } else if self.out_len > i {
            let count = self.out_len - i;
            let self_copy = self.out_lockstep && self.out_len == self.idx && count <= self.idx;
            if !self_copy {
                self.materialize_out();
            }
            if count > self.idx && !self.shift_forward(count - self.idx) {
                return false;
            }
            self.idx -= count;
            self.out_len -= count;
            if !self_copy {
                for j in 0..count {
                    self.info[self.idx + j] = self.out_info[self.out_len + j];
                }
            }
        }
        true
    }

    pub(crate) fn sort(&mut self, start: usize, end: usize, cmp: impl Fn(&GlyphInfo, &GlyphInfo) -> bool) {
        debug_assert!(!self.have_positions);

        for i in start + 1..end {
            let mut j = i;
            while j > start && cmp(&self.info[j - 1], &self.info[i]) {
                j -= 1;
            }
            if i == j {
                continue;
            }

            self.merge_clusters(j, i + 1);

            let t = self.info[i];
            for idx in (0..i - j).rev() {
                self.info[idx + j + 1] = self.info[idx + j];
            }
            self.info[j] = t;
        }
    }

    pub(crate) fn reverse(&mut self) {
        if self.is_empty() {
            return;
        }
        self.reverse_range(0, self.len);
    }

    pub(crate) fn reverse_range(&mut self, start: usize, end: usize) {
        if end - start < 2 {
            return;
        }
        self.info[start..end].reverse();
        if self.have_positions {
            self.pos[start..end].reverse();
        }
    }

    pub(crate) fn group_end<F>(&self, mut start: usize, group: F) -> usize
    where
        F: Fn(&GlyphInfo, &GlyphInfo) -> bool,
    {
        start += 1;
        while start < self.len && group(&self.info[start - 1], &self.info[start]) {
            start += 1;
        }
        start
    }

    pub(crate) fn set_cluster(info: &mut GlyphInfo, cluster: u32, mask: Mask) {
        if info.cluster != cluster {
            info.mask = (info.mask & !glyph_flag::DEFINED) | (mask & glyph_flag::DEFINED);
        }
        info.cluster = cluster;
    }

    pub(crate) fn merge_clusters(&mut self, start: usize, end: usize) {
        if end - start < 2 {
            return;
        }
        if !self.cluster_level.is_monotone() {
            self.unsafe_to_break(start, end);
            return;
        }
        self.merge_clusters_impl(start, end);
    }

    pub(crate) fn merge_grapheme_clusters(&mut self, start: usize, end: usize) {
        if end - start < 2 {
            return;
        }
        if !self.cluster_level.is_graphemes() {
            self.unsafe_to_break(start, end);
            return;
        }
        self.merge_clusters_impl(start, end);
    }

    fn merge_clusters_impl(&mut self, start: usize, end: usize) {
        let mut cluster = self.info[start].cluster;
        for i in start + 1..end {
            cluster = cluster.min(self.info[i].cluster);
        }

        let mut end = end;
        if cluster != self.info[end - 1].cluster {
            while end < self.len && self.info[end - 1].cluster == self.info[end].cluster {
                end += 1;
            }
        }
        let mut start = start;
        if self.info[start].cluster != cluster {
            while start > self.idx && self.info[start - 1].cluster == self.info[start].cluster {
                start -= 1;
            }
        }

        if self.idx == start && self.out_len != 0 && self.info[start].cluster != cluster {
            let mut i = self.out_len;
            while i > 0 && self.out_info()[i - 1].cluster == self.info[start].cluster {
                Self::set_cluster(&mut self.out_info_mut()[i - 1], cluster, 0);
                i -= 1;
            }
        }

        for i in start..end {
            Self::set_cluster(&mut self.info[i], cluster, 0);
        }
    }

    pub(crate) fn merge_out_clusters(&mut self, start: usize, end: usize) {
        if end - start < 2 || !self.cluster_level.is_monotone() {
            return;
        }
        self.merge_out_clusters_impl(start, end);
    }

    pub(crate) fn merge_out_grapheme_clusters(&mut self, start: usize, end: usize) {
        if end - start < 2 || !self.cluster_level.is_graphemes() {
            return;
        }
        self.merge_out_clusters_impl(start, end);
    }

    fn merge_out_clusters_impl(&mut self, start: usize, end: usize) {
        self.materialize_out();
        let mut cluster = self.out_info[start].cluster;
        for i in start + 1..end {
            cluster = cluster.min(self.out_info[i].cluster);
        }

        let mut start = start;
        if self.out_info[start].cluster != cluster {
            while start > 0 && self.out_info[start - 1].cluster == self.out_info[start].cluster {
                start -= 1;
            }
        }
        let mut end = end;
        if cluster != self.out_info[end - 1].cluster {
            while end < self.out_len && self.out_info[end - 1].cluster == self.out_info[end].cluster {
                end += 1;
            }
        }

        if end == self.out_len {
            let mut i = self.idx;
            while i < self.len && self.info[i].cluster == self.out_info[end - 1].cluster {
                Self::set_cluster(&mut self.info[i], cluster, 0);
                i += 1;
            }
        }

        for i in start..end {
            Self::set_cluster(&mut self.out_info[i], cluster, 0);
        }
    }

    pub(crate) fn delete_glyph(&mut self) {
        let cluster = self.info[self.idx].cluster;

        if (self.idx + 1 < self.len && cluster == self.info[self.idx + 1].cluster)
            || (self.out_len != 0 && cluster == self.out_info()[self.out_len - 1].cluster)
        {
            self.skip_glyph();
            return;
        }

        if self.out_len != 0 {
            if cluster < self.out_info()[self.out_len - 1].cluster {
                let mask = self.info[self.idx].mask;
                let old = self.out_info()[self.out_len - 1].cluster;
                let mut i = self.out_len;
                while i != 0 && self.out_info()[i - 1].cluster == old {
                    Self::set_cluster(&mut self.out_info_mut()[i - 1], cluster, mask);
                    i -= 1;
                }
            }
            self.skip_glyph();
            return;
        }

        if self.idx + 1 < self.len {
            self.merge_clusters(self.idx, self.idx + 2);
        }
        self.skip_glyph();
    }

    pub(crate) fn delete_glyphs_inplace(&mut self, filter: impl Fn(&GlyphInfo) -> bool) {
        let mut j = 0;
        for i in 0..self.len {
            if filter(&self.info[i]) {
                let cluster = self.info[i].cluster;
                if i + 1 < self.len && cluster == self.info[i + 1].cluster {
                    continue;
                }
                if j != 0 {
                    if cluster < self.info[j - 1].cluster {
                        let mask = self.info[i].mask;
                        let old = self.info[j - 1].cluster;
                        let mut k = j;
                        while k > 0 && self.info[k - 1].cluster == old {
                            Self::set_cluster(&mut self.info[k - 1], cluster, mask);
                            k -= 1;
                        }
                    }
                    continue;
                }
                if i + 1 < self.len {
                    self.merge_clusters(i, i + 2);
                }
                continue;
            }
            if j != i {
                self.info[j] = self.info[i];
                self.pos[j] = self.pos[i];
            }
            j += 1;
        }
        self.len = j;
    }

    pub(crate) fn unsafe_to_break(&mut self, start: usize, end: usize) {
        self.set_glyph_flags(
            glyph_flag::UNSAFE_TO_BREAK | glyph_flag::UNSAFE_TO_CONCAT,
            start,
            end,
            true,
            false,
        );
    }

    pub(crate) fn unsafe_to_break_from_outbuffer(&mut self, start: usize, end: usize) {
        self.set_glyph_flags(
            glyph_flag::UNSAFE_TO_BREAK | glyph_flag::UNSAFE_TO_CONCAT,
            start,
            end,
            true,
            true,
        );
    }

    pub(crate) fn safe_to_insert_tatweel(&mut self, start: usize, end: usize) {
        if !self.produce_safe_to_insert_tatweel {
            self.unsafe_to_break(start, end);
            return;
        }
        self.set_glyph_flags(glyph_flag::SAFE_TO_INSERT_TATWEEL, start, end, true, false);
    }

    pub(crate) fn unsafe_to_concat(&mut self, start: usize, end: usize) {
        if !self.produce_unsafe_to_concat {
            return;
        }
        self.set_glyph_flags(glyph_flag::UNSAFE_TO_CONCAT, start, end, false, false);
    }

    pub(crate) fn unsafe_to_concat_from_outbuffer(&mut self, start: usize, end: usize) {
        if !self.produce_unsafe_to_concat {
            return;
        }
        self.set_glyph_flags(glyph_flag::UNSAFE_TO_CONCAT, start, end, false, true);
    }

    fn set_glyph_flags(
        &mut self,
        mask: Mask,
        start: usize,
        end: usize,
        interior: bool,
        from_out_buffer: bool,
    ) {
        let end = end.min(self.len);
        if interior && !from_out_buffer && end.saturating_sub(start) < 2 {
            return;
        }
        self.scratch_flags |= scratch_flags::HAS_GLYPH_FLAGS;

        if !from_out_buffer || !self.have_output {
            if interior {
                let cluster = self.find_min_cluster(false, start, end, Mask::MAX);
                self.apply_glyph_flags(false, start, end, cluster, mask);
            } else {
                for i in start..end {
                    self.info[i].mask |= mask;
                }
            }
        } else if interior {
            let mut cluster = self.find_min_cluster(false, self.idx, end, Mask::MAX);
            cluster = self.find_min_cluster(true, start, self.out_len, cluster);
            let out_len = self.out_len;
            let idx = self.idx;
            self.apply_glyph_flags(true, start, out_len, cluster, mask);
            self.apply_glyph_flags(false, idx, end, cluster, mask);
        } else {
            for i in start..self.out_len {
                self.out_info_mut()[i].mask |= mask;
            }
            for i in self.idx..end {
                self.info[i].mask |= mask;
            }
        }
    }

    fn find_min_cluster(&self, out: bool, start: usize, end: usize, cluster: u32) -> u32 {
        if start >= end {
            return cluster;
        }
        let infos = if out { self.out_info() } else { &self.info };
        let mut cluster = cluster;
        if self.cluster_level == ClusterLevel::Characters {
            for info in &infos[start..end] {
                cluster = cluster.min(info.cluster);
            }
        }
        cluster.min(infos[start].cluster.min(infos[end - 1].cluster))
    }

    fn apply_glyph_flags(&mut self, out: bool, start: usize, end: usize, cluster: u32, mask: Mask) {
        if start >= end {
            return;
        }
        let cluster_level = self.cluster_level;
        let infos = if out { self.out_info_mut() } else { &mut self.info };
        let cluster_first = infos[start].cluster;
        let cluster_last = infos[end - 1].cluster;

        if cluster_level == ClusterLevel::Characters
            || (cluster != cluster_first && cluster != cluster_last)
        {
            for info in &mut infos[start..end] {
                if info.cluster != cluster {
                    info.mask |= mask;
                }
            }
            return;
        }

        if cluster == cluster_first {
            let mut i = end;
            while start < i && infos[i - 1].cluster != cluster_first {
                if cluster != infos[i - 1].cluster {
                    infos[i - 1].mask |= mask;
                }
                i -= 1;
            }
        } else {
            let mut i = start;
            while i < end && infos[i].cluster != cluster_last {
                if cluster != infos[i].cluster {
                    infos[i].mask |= mask;
                }
                i += 1;
            }
        }
    }

    pub(crate) fn reset_masks(&mut self, mask: Mask) {
        for info in &mut self.info[..self.len] {
            info.mask = mask;
        }
    }

    pub(crate) fn set_masks(&mut self, value: Mask, mask: Mask, cluster_start: u32, cluster_end: u32) {
        if mask == 0 {
            return;
        }
        let value = value & mask;
        for info in &mut self.info[..self.len] {
            if cluster_start <= info.cluster && info.cluster < cluster_end {
                info.mask = (info.mask & !mask) | value;
            }
        }
    }

    pub(crate) fn map_glyphs(&mut self) {
        for info in &mut self.info[..self.len] {
            info.id = info.glyph_index;
        }
    }

    pub(crate) fn enter(&mut self) {
        self.serial = 0;
        self.shaping_failed = false;
        self.scratch_flags = 0;
        if let Some(n) = self.len.checked_mul(Self::MAX_LEN_FACTOR) {
            self.max_len = n.max(Self::MAX_LEN_MIN);
        }
        if let Ok(n) = i32::try_from(self.len)
            && let Some(ops) = n.checked_mul(Self::MAX_OPS_FACTOR) {
                self.max_ops = ops.max(Self::MAX_OPS_MIN);
            }
    }

    pub(crate) fn leave(&mut self) {
        self.max_len = Self::MAX_LEN_DEFAULT;
        self.max_ops = Self::MAX_OPS_DEFAULT;
        self.serial = 0;
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new()
    }
}
