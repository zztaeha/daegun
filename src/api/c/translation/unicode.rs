// unicode.h: segmentation, line breaking, scripts, characters and bidi, none of which needs a font.

use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

use crate::daecore::daeshaper::unicode::segment::{Boundaries, each_grapheme_boundary};
use crate::ffi::handle::{
    Status, Str, apart, borrow, cleared, deliver_reused, field_view, into_buffer, invalid, or_return, put,
    put_some, reused_box, room_for, slice_mut_of, span, text_of,
};
use crate::ffi::char_of;
use crate::unicode::{BidiParagraph, LineBreak, ParagraphRun, Script, WordSegmenter};
use crate::Tag;

// segmentation

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_grapheme_boundaries(
    text: *const c_char,
    text_len: usize,
    out: *mut usize,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let text = or_return!(unsafe { text_of(text, text_len) });
    or_return!(apart(&[span(text.as_ptr(), text.len()), span(out, out_len), span(out_count, 1)]));
    unsafe { graphemes_into(text, out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_word_boundaries(
    text: *const c_char,
    text_len: usize,
    out: *mut usize,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let text = or_return!(unsafe { text_of(text, text_len) });
    or_return!(apart(&[span(text.as_ptr(), text.len()), span(out, out_len), span(out_count, 1)]));
    // Rust's list copied out: walked into C's buffer as graphemes are, words took 8% to 14% longer in cycles.
    unsafe { into_buffer(words(text).into_iter(), out, out_len, out_count) }
}

// The walk out of line: inlined into the entry above, it took 6% longer.
#[inline(never)]
fn words(text: &str) -> Vec<usize> {
    crate::unicode::word_boundaries(text)
}

// The caller's buffer as a walk's sink: each offset written while there is room, and every one counted.
struct Offsets<'a> {
    slots: core::slice::IterMut<'a, usize>,
    count: usize,
}

impl Boundaries for Offsets<'_> {
    fn put(&mut self, at: usize) {
        if let Some(slot) = self.slots.next() {
            *slot = at;
        }
        self.count += 1;
    }
}

// Boundaries into the caller's buffer as base.h's BUFFERS says, with no list between. A buffer under the text's length
// plus one may run out, and the walk then only counts: one walk serves both, as two stayed out of line.
unsafe fn graphemes_into(text: &str, out: *mut usize, out_len: usize, out_count: *mut usize) -> Status {
    if out_count.is_null() || (out.is_null() && out_len > 0) {
        return Status::Null;
    }
    let mut room = !out.is_null() && out_len > text.len();
    loop {
        let buf = if room { unsafe { slice_mut_of(out, out_len) }.unwrap_or_default() } else { &mut [] };
        let mut sink = Offsets { slots: buf.iter_mut(), count: 0 };
        each_grapheme_boundary(text, &mut sink);
        let count = sink.count;
        if room {
            unsafe { *out_count = count };
            return Status::Ok;
        }
        if !or_return!(unsafe { room_for(count, !out.is_null(), out_len, out_count) }) {
            return Status::Ok;
        }
        room = true;
    }
}

// line breaking

pub type SegmentFn = unsafe extern "C" fn(*mut c_void, *const c_char, usize, *mut usize, usize) -> usize;

// The caller's pointer, Sync as Rust's segmenter must be: it's built inside one C call and runs on that call's thread.
struct User(*mut c_void);
unsafe impl Sync for User {}

impl User {
    fn get(&self) -> *mut c_void {
        self.0
    }
}

// A caller's segmenter as Rust takes one: each run handed over as UTF-8, with room in `out` for every boundary
// inside it, and what it reports past that room ignored.
pub(crate) fn segment_with(segment: SegmentFn, user: *mut c_void) -> impl Fn(&str, &mut Vec<usize>) + Sync {
    let user = User(user);
    move |run: &str, out: &mut Vec<usize>| {
        let (start, cap) = (out.len(), run.chars().count().saturating_sub(1));
        out.resize(start + cap, 0);
        let found = unsafe { segment(user.get(), run.as_ptr().cast(), run.len(), out[start..].as_mut_ptr(), cap) };
        out.truncate(start + found.min(cap));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_line_break_opportunities(
    text: *const c_char,
    text_len: usize,
    segmenter: Option<SegmentFn>,
    user: *mut c_void,
    out: *mut LineBreak,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let text = or_return!(unsafe { text_of(text, text_len) });
    or_return!(apart(&[span(text.as_ptr(), text.len()), span(out, out_len), span(out_count, 1)]));
    let segment = segmenter.map(|f| segment_with(f, user));
    let breaks = crate::unicode::line_break_opportunities(text, segment.as_ref().map(|s| WordSegmenter(s)));
    unsafe { into_buffer(breaks.into_iter(), out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_has_dictionaries() -> bool {
    cfg!(feature = "dictionaries")
}

// scripts

// A script named by its ISO 15924 code packed as a tag.
pub(crate) fn script_of(tag: Tag) -> Result<Script, Status> {
    Script::from_iso_tag(tag.0)
        .ok_or_else(|| invalid(format_args!("{tag:?} is no ISO 15924 code of a script this build knows")))
}

fn script_tag(script: Script) -> u32 {
    let code = script.iso15924().and_then(|c| <[u8; 4]>::try_from(c.as_bytes()).ok());
    code.map_or(0, u32::from_be_bytes)
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScriptRunC {
    start: usize,
    end: usize,
    script: u32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_script_runs(
    text: *const c_char,
    text_len: usize,
    out: *mut ScriptRunC,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let text = or_return!(unsafe { text_of(text, text_len) });
    or_return!(apart(&[span(text.as_ptr(), text.len()), span(out, out_len), span(out_count, 1)]));
    let runs = crate::unicode::script_runs(text);
    let runs = runs.iter().map(|r| ScriptRunC { start: r.bytes.start, end: r.bytes.end, script: script_tag(r.script) });
    unsafe { into_buffer(runs, out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_script_name(script: u32, out: *mut Str) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let script = or_return!(script_of(Tag(script)));
    unsafe { put(out, Str::of_static(script.c_name())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_script_direction(script: u32, out: *mut i32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let script = or_return!(script_of(Tag(script)));
    unsafe { put(out, script.direction() as i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_script_opentype_tags(
    script: u32,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    if out_count.is_null() {
        return Status::Null;
    }
    let script = or_return!(script_of(Tag(script)));
    unsafe { into_buffer(script.opentype_tags().map(|t| t.0), out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_script_is_context_dependent(script: u32, out: *mut bool) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let script = or_return!(script_of(Tag(script)));
    unsafe { put(out, script.is_context_dependent()) }
}

// characters

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_general_category(codepoint: u32, out: *mut i32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let c = or_return!(char_of(codepoint));
    // The enum's discriminants are the DAEGUN_GC_* values, both in Unicode's order of the two-letter codes.
    unsafe { put(out, crate::unicode::general_category(c) as i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_script(codepoint: u32, out: *mut u32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let c = or_return!(char_of(codepoint));
    unsafe { put(out, script_tag(crate::unicode::script(c))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_vertical_form(codepoint: u32, out: *mut u32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let c = or_return!(char_of(codepoint));
    unsafe { put_some(out, crate::unicode::vertical_form(c).map(u32::from)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_vertical_orientation(codepoint: u32, out: *mut i32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let c = or_return!(char_of(codepoint));
    unsafe { put(out, crate::unicode::vertical_orientation(c) as i32) }
}

// bidi

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bidi_paragraph_new(
    text: *const c_char,
    text_len: usize,
    base: i32,
    out: *mut *mut BidiParagraph,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let text = or_return!(unsafe { text_of(text, text_len) });
    let base = or_return!(crate::ffi::base_direction(base));
    unsafe { deliver_reused(out, BidiParagraph::new(text, base)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bidi_paragraph_base_level(paragraph: *const BidiParagraph, out: *mut u8) -> Status {
    let Some(paragraph) = (unsafe { borrow(paragraph) }) else { return Status::Null };
    unsafe { put(out, paragraph.base_level) }
}

field_view!(daegun_bidi_paragraph_levels, BidiParagraph, levels, u8);
field_view!(daegun_bidi_paragraph_visual_order, BidiParagraph, visual_order, usize);
field_view!(daegun_bidi_paragraph_paragraphs, BidiParagraph, paragraphs, ParagraphRun);
reused_box!(daegun_bidi_paragraph_free, BidiParagraph, BidiParagraph {
    base_level: 0,
    levels: Vec::new(),
    visual_order: Vec::new(),
    paragraphs: Vec::new(),
});

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisualRunC {
    start: usize,
    end: usize,
    level: u8,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bidi_paragraph_visual_runs(
    paragraph: *const BidiParagraph,
    text: *const c_char,
    text_len: usize,
    start: usize,
    end: usize,
    out: *mut VisualRunC,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(paragraph) = (unsafe { borrow(paragraph) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let text = or_return!(unsafe { text_of(text, text_len) });
    or_return!(apart(&[span(text.as_ptr(), text.len()), span(out, out_len), span(out_count, 1)]));
    let runs = paragraph.visual_runs(text, start..end);
    let runs = runs.iter().map(|r| VisualRunC { start: r.bytes.start, end: r.bytes.end, level: r.level });
    unsafe { into_buffer(runs, out, out_len, out_count) }
}

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<BidiParagraph>();
}

#[cfg(test)]
#[path = "tests/unicode.rs"]
mod tests;
