use crate::daecore::daeshaper::buffer::{scratch_flags, Buffer, GlyphInfo};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::vowel_constraints::{INVALID_VOWEL_PAIRS, INVALID_VOWEL_TRIPLES};

pub(crate) fn insert_dotted_circles(
    face: &Face,
    buffer: &mut Buffer,
    broken_type: u8,
    dotted_circle_category: u8,
    repha_category: Option<u8>,
    dotted_circle_position: Option<u8>,
) -> bool {
    if !buffer.insert_dotted_circle {
        return false;
    }
    if buffer.scratch_flags & scratch_flags::HAS_BROKEN_SYLLABLE == 0 {
        return false;
    }
    // Resolved to a glyph id, not left as the codepoint: every caller here is a pause, which runs
    // after glyph mapping, where U+25CC would read as gid 9676 and the circle come out blank.
    // `insert_vowel_constraints` is the opposite case – it runs from `preprocess_text`.
    let Some(dotted_circle) = face.glyph_index(0x25CC) else {
        return false;
    };

    let mut template = GlyphInfo::new(u32::from(dotted_circle), 0);
    template.shaper_category = dotted_circle_category;
    if let Some(position) = dotted_circle_position {
        template.shaper_auxiliary = position;
    }

    buffer.clear_output();
    buffer.idx = 0;
    let mut last_syllable = 0;

    while buffer.idx < buffer.len {
        let syllable = buffer.cur(0).syllable;
        if last_syllable != syllable && (syllable & 0x0F) == broken_type {
            last_syllable = syllable;

            let mut inserted = template;
            inserted.cluster = buffer.cur(0).cluster;
            inserted.mask = buffer.cur(0).mask;
            inserted.syllable = syllable;

            if let Some(repha) = repha_category {
                while buffer.idx < buffer.len
                    && last_syllable == buffer.cur(0).syllable
                    && buffer.cur(0).shaper_category == repha
                {
                    buffer.next_glyph();
                }
            }

            buffer.output_info(inserted);
        } else {
            buffer.next_glyph();
        }
    }

    buffer.sync();
    true
}

pub(crate) fn is_invalid_vowel_pair(first: u32, second: u32) -> bool {
    INVALID_VOWEL_PAIRS.binary_search(&(first, second)).is_ok()
}

pub(crate) fn is_invalid_vowel_triple(first: u32, second: u32, third: u32) -> bool {
    INVALID_VOWEL_TRIPLES.binary_search(&(first, second, third)).is_ok()
}

pub(crate) fn insert_vowel_constraints(face: &Face, buffer: &mut Buffer) {
    if !buffer.insert_dotted_circle || buffer.len < 2 || !face.has_glyph(0x25CC) {
        return;
    }

    buffer.clear_output();
    buffer.idx = 0;

    let mut owed = false;
    while buffer.idx < buffer.len {
        let here = buffer.cur(0).id;
        let pair = buffer.idx + 1 < buffer.len && is_invalid_vowel_pair(here, buffer.cur(1).id);
        let triple = !pair
            && buffer.idx + 2 < buffer.len
            && is_invalid_vowel_triple(here, buffer.cur(1).id, buffer.cur(2).id);

        buffer.next_glyph();

        if owed || pair {
            buffer.output_glyph(0x25CC);
            let at = buffer.out_len - 1;
            buffer.out_info_mut()[at].reset_continuation();
        }
        owed = triple;
    }

    buffer.sync();
}
