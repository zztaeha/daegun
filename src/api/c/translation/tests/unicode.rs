use super::*;
use alloc::string::String;

// A caller's segmenter: a boundary after every second character of a run.
unsafe extern "C" fn every_second(
    _: *mut c_void,
    text: *const c_char,
    len: usize,
    out: *mut usize,
    cap: usize,
) -> usize {
    let run = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(text.cast::<u8>(), len)) };
    let mut found = 0;
    for at in (2..run.chars().count()).step_by(2).take(cap) {
        unsafe { out.add(found).write(at) };
        found += 1;
    }
    found
}

fn every_second_rust(run: &str, out: &mut Vec<usize>) {
    out.extend((2..run.chars().count()).step_by(2));
}

// C against Rust over every code point and a set of texts, as every area's ignored test does: run with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::ffi::testing::{Diffs, SAMPLE, TEXTS, buffered, lent, short_of};
    use crate::unicode::BaseDirection;
    let mut d = Diffs::default();
    let code = |s: Script| {
        s.iso15924().map_or(0, |c| u32::from_be_bytes(<[u8; 4]>::try_from(c.as_bytes()).unwrap_or([0; 4])))
    };
    for cp in (0..=0x11_0000).chain([u32::MAX]) {
        let (mut gc, mut vf, mut vo, mut sc) = (0, 0, -1, 0);
        let got = unsafe {
            (
                daegun_general_category(cp, &mut gc),
                daegun_vertical_form(cp, &mut vf),
                daegun_vertical_orientation(cp, &mut vo),
                daegun_script(cp, &mut sc),
            )
        };
        let (category, form) = ((got.0 == Status::Ok).then_some(gc), (got.1 == Status::Ok).then_some(vf));
        let got = (got.0, category, got.1, form, got.2, vo, got.3, sc);
        let want = match char::from_u32(cp) {
            Some(c) => {
                let form = crate::unicode::vertical_form(c).map(u32::from);
                let form_status = if form.is_some() { Status::Ok } else { Status::Absent };
                let vo = crate::unicode::vertical_orientation(c) as i32;
                let gc = crate::unicode::general_category(c) as i32;
                (Status::Ok, Some(gc), form_status, form, Status::Ok, vo, Status::Ok, code(crate::unicode::script(c)))
            }
            None => {
                let bad = Status::InvalidArgument;
                (bad, None, bad, None, bad, -1, bad, 0)
            }
        };
        d.eq("characters", "U+", (cp, got), (cp, want));
    }
    for script in (0u16..).map(Script).take_while(|s| s.iso15924().is_some()) {
        let code =
            u32::from_be_bytes(<[u8; 4]>::try_from(script.iso15924().unwrap_or("").as_bytes()).unwrap_or([0; 4]));
        let (mut name, mut direction, mut context) = (Str::EMPTY, -1, false);
        let got = unsafe {
            let n = daegun_script_name(code, &mut name);
            let r = daegun_script_direction(code, &mut direction);
            let c = daegun_script_is_context_dependent(code, &mut context);
            let tags = buffered(|out, len, count| daegun_script_opentype_tags(code, out, len, count), 8);
            (n, crate::ffi::testing::str_of(name), r, direction, c, context, tags)
        };
        let tags: Vec<u32> = script.opentype_tags().map(|t| t.0).collect();
        let (short, count) = short_of(tags.len());
        let want = (
            Status::Ok,
            String::from(script.name()),
            Status::Ok,
            script.direction() as i32,
            Status::Ok,
            script.is_context_dependent(),
            (short, count, Status::Ok, tags),
        );
        d.eq("scripts", script.iso15924().unwrap_or(""), got, want);
    }
    for code in [0, u32::from_be_bytes(*b"Xxxx"), u32::from_be_bytes(*b"latn"), u32::MAX] {
        let mut direction = -1;
        let st = unsafe { daegun_script_direction(code, &mut direction) };
        let want = match Script::from_iso15924(core::str::from_utf8(&code.to_be_bytes()).unwrap_or("")) {
            Some(_) => Status::Ok,
            None => Status::InvalidArgument,
        };
        d.eq("scripts", "a code", (code, st), (code, want));
    }
    let mut texts: Vec<&str> = TEXTS.to_vec();
    texts.extend([
        SAMPLE,
        "",
        "abc \u{5d0}\u{5d1}\u{5d2} 123 def",
        "\u{202b}abc \u{5d0}\u{202c} x\u{2067}y\u{2069}",
        "a\nb\r\nc\u{2029}d \u{5d0}\n\u{627}",
        "\u{feff}x \u{301}a \u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}\u{e04}\u{e23}\u{e31}\u{e1a}",
    ]);
    for (t, text) in texts.iter().enumerate() {
        let room = text.chars().count() + 2;
        let (ptr, len) = (text.as_ptr().cast::<c_char>(), text.len());
        unsafe {
            let got = buffered(|out, l, count| daegun_grapheme_boundaries(ptr, len, out, l, count), room);
            let want = crate::unicode::grapheme_boundaries(text);
            let (short, count) = short_of(want.len());
            d.eq("graphemes", "text", (t, got), (t, (short, count, Status::Ok, want)));
            let got = buffered(|out, l, count| daegun_word_boundaries(ptr, len, out, l, count), room);
            let want = crate::unicode::word_boundaries(text);
            let (short, count) = short_of(want.len());
            d.eq("words", "text", (t, got), (t, (short, count, Status::Ok, want)));
            for segmented in [false, true] {
                let seg: Option<SegmentFn> = segmented.then_some(every_second);
                let got = buffered(
                    |out, l, count| {
                        daegun_line_break_opportunities(ptr, len, seg, core::ptr::null_mut(), out, l, count)
                    },
                    room,
                );
                let got = (got.0, got.1, got.2, got.3.iter().map(|b| (b.at, b.mandatory)).collect::<Vec<_>>());
                let rust_seg = every_second_rust;
                let want =
                    crate::unicode::line_break_opportunities(text, segmented.then_some(WordSegmenter(&rust_seg)));
                let (short, count) = short_of(want.len());
                let want = (short, count, Status::Ok, want.iter().map(|b| (b.at, b.mandatory)).collect::<Vec<_>>());
                d.eq("line breaks", "text", (t, segmented, got), (t, segmented, want));
            }
            let got = buffered(|out, l, count| daegun_script_runs(ptr, len, out, l, count), room);
            let got = (got.0, got.1, got.2, got.3.iter().map(|r| (r.start, r.end, r.script)).collect::<Vec<_>>());
            let runs = crate::unicode::script_runs(text);
            let (short, count) = short_of(runs.len());
            let cut = runs.iter().map(|r| (r.bytes.start, r.bytes.end, code(r.script))).collect::<Vec<_>>();
            let want = (short, count, Status::Ok, cut);
            d.eq("script runs", "text", (t, got), (t, want));

            for (base, direction) in [
                (crate::ffi::BASE_AUTO, BaseDirection::Auto),
                (crate::ffi::BASE_LTR, BaseDirection::LeftToRight),
                (crate::ffi::BASE_RTL, BaseDirection::RightToLeft),
            ] {
                let mut p = core::ptr::null_mut();
                let st = daegun_bidi_paragraph_new(ptr, len, base, &mut p);
                let (mut level, mut n, mut k, mut m) = (0, 0, 0, 0);
                let _ = daegun_bidi_paragraph_base_level(p, &mut level);
                let levels = lent(daegun_bidi_paragraph_levels(p, &mut n), n).to_vec();
                let order = lent(daegun_bidi_paragraph_visual_order(p, &mut k), k).to_vec();
                let paragraphs: Vec<_> = lent(daegun_bidi_paragraph_paragraphs(p, &mut m), m)
                    .iter()
                    .map(|r| (r.start, r.end, r.level))
                    .collect();
                let rust = crate::unicode::BidiParagraph::new(text, direction);
                let want_paragraphs: Vec<_> = rust.paragraphs.iter().map(|r| (r.start, r.end, r.level)).collect();
                let want =
                    (Status::Ok, rust.base_level, rust.levels.clone(), rust.visual_order.clone(), want_paragraphs);
                d.eq("bidi", "text", (t, base, (st, level, levels, order, paragraphs)), (t, base, want));
                let chars = text.chars().count();
                for (start, end) in
                    [(0, chars), (1, chars.saturating_sub(1)), (chars / 2, chars), (chars, 0), (0, chars + 5)]
                {
                    let runs =
                        |out, l, count| daegun_bidi_paragraph_visual_runs(p, ptr, len, start, end, out, l, count);
                    let got = buffered(runs, room);
                    let got =
                        (got.0, got.1, got.2, got.3.iter().map(|r| (r.start, r.end, r.level)).collect::<Vec<_>>());
                    let runs = rust.visual_runs(text, start..end);
                    let (short, count) = short_of(runs.len());
                    let want = (
                        short,
                        count,
                        Status::Ok,
                        runs.iter().map(|r| (r.bytes.start, r.bytes.end, r.level)).collect::<Vec<_>>(),
                    );
                    d.eq("visual runs", "text", (t, base, start, end, got), (t, base, start, end, want));
                }
                daegun_bidi_paragraph_free(p);
            }
        }
    }
    d.done("unicode");
}

// Text read from the buffer its boundaries go into answers INVALID_ARGUMENT, the text left as it was; the same text
// in its own buffer has its boundaries.
#[test]
fn boundaries_into_the_text_s_own_buffer_are_refused() {
    let (mut buf, mut count) = ([0usize; 4], 0);
    let out = buf.as_mut_ptr();
    unsafe { core::ptr::copy_nonoverlapping(b"ab cd".as_ptr(), out.cast::<u8>(), 5) };
    let in_place = unsafe { daegun_word_boundaries(out.cast::<c_char>(), 5, out, 4, &mut count) };
    assert_eq!((in_place, count), (Status::InvalidArgument, 0));
    assert_eq!(unsafe { core::slice::from_raw_parts(out.cast::<u8>(), 5) }, b"ab cd");
    let apart = unsafe { daegun_word_boundaries(b"ab cd".as_ptr().cast::<c_char>(), 5, out, 4, &mut count) };
    assert_eq!((apart, count, &buf[..4]), (Status::Ok, 4, &[0, 2, 3, 5][..]));
}
