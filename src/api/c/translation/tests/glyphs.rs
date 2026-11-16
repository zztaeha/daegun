use super::*;
use alloc::vec::Vec;
use crate::Orientation;
use crate::ffi::testing::{Diffs, PROBES, SAMPLE, bits, each_font, filled2, found, glyph_list, take_strs};
use core::ptr::null_mut;

// The longest name and its NUL.
const GLYPH_NAME_ROOM: usize = crate::daecore::daetype::glyph_names::MAX_GLYPH_NAME + 1;

// A name fills exactly its room and its NUL; one byte short is refused, the count still set, and nothing written.
#[test]
fn a_glyph_name_fills_the_callers_buffer() {
    let c = crate::ffi::testing::CFont::of(crate::ffi::testing::asset("inter/InterVariable.ttf"));
    let font = crate::Font::from_bytes(&c.bytes).unwrap();
    let a = font.glyph_id('a').unwrap();
    let name = font.glyph_name(a).unwrap();
    let mut buf = [0x7f as core::ffi::c_char; GLYPH_NAME_ROOM];
    let mut n = 0;
    unsafe {
        assert_eq!(daegun_font_glyph_name(c.handle, a.0, buf.as_mut_ptr(), name.len() + 1, &mut n), Status::Ok);
        assert_eq!(n, name.len() + 1);
        assert_eq!(core::ffi::CStr::from_ptr(buf.as_ptr()).to_str(), Ok(name));
        buf.fill(0x7f);
        let st = daegun_font_glyph_name(c.handle, a.0, buf.as_mut_ptr(), name.len(), &mut n);
        assert!(st == Status::InvalidArgument && n == name.len() + 1, "a short buffer answered {st:?}, {n}");
        assert!(buf.iter().all(|&b| b == 0x7f), "a short buffer was written");
        let past = font.glyph_count();
        assert_eq!(daegun_font_glyph_name(c.handle, past, buf.as_mut_ptr(), buf.len(), &mut n), Status::Absent);
        assert_eq!(daegun_font_glyph_name(c.handle, a.0, buf.as_mut_ptr(), buf.len(), null_mut()), Status::Null);
    }
}

// A C string can't hold a NUL, so a name holding one is none through C, where Rust gives it.
#[test]
fn a_glyph_name_holding_a_nul_is_none_in_c() {
    let font = crate::Font::from_bytes(&crate::ffi::testing::asset("inter/InterVariable.ttf")).unwrap();
    let mut tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> =
        font.table_tags().map(|t| (t, font.table(t).unwrap().to_vec())).collect();
    let mut post = [&[0, 2, 0, 0][..], &[0; 28], &[0, 2, 1, 2, 1, 3], b"\x03a\x00b\x01c"].concat();
    post[4..8].copy_from_slice(&tables[&crate::Tag::new(b"post")][4..8]);
    tables.insert(crate::Tag::new(b"post"), post);
    let c = crate::ffi::testing::CFont::of(crate::raw::build_font(&tables).unwrap());
    let rust = unsafe { &*c.handle };
    assert_eq!((rust.glyph_name(GlyphId(0)), rust.glyph_name(GlyphId(1))), (Some("a\0b"), Some("c")));
    let (mut buf, mut n) = ([0 as core::ffi::c_char; GLYPH_NAME_ROOM], 0);
    unsafe {
        assert_eq!(daegun_font_glyph_name(c.handle, 0, buf.as_mut_ptr(), buf.len(), &mut n), Status::Absent);
        assert_eq!(daegun_font_glyph_name(c.handle, 1, buf.as_mut_ptr(), buf.len(), &mut n), Status::Ok);
        let mut names = null_mut();
        assert_eq!(daegun_font_glyph_names(c.handle, &mut names), Status::Ok);
        assert_eq!(take_strs(names)[..3], ["", "c", ""]);
    }
}

// Each string ends at a NUL of its own; one holding a NUL is mended as its list says.
#[test]
fn a_string_holding_a_nul_is_mended_as_its_list_says() {
    use crate::ffi::base::{OnNul, StrList, daegun_str_list_at};
    let items = ["a", "b\0c", "", "\0", "\u{fffd}"];
    for (on_nul, want) in [
        (OnNul::Replace, ["a", "b\u{fffd}c", "", "\u{fffd}", "\u{fffd}"]),
        (OnNul::Empty, ["a", "", "", "", "\u{fffd}"]),
    ] {
        let list = crate::ffi::handle::new(StrList::new(items.into_iter(), on_nul));
        for i in 0..items.len() {
            let mut s = crate::ffi::handle::Str::EMPTY;
            assert_eq!(unsafe { daegun_str_list_at(list, i, &mut s) }, Status::Ok);
            assert_eq!(unsafe { *s.data.add(s.len) }, 0, "string {i} has no NUL after it");
        }
        assert_eq!(unsafe { take_strs(list) }, want);
    }
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        let at = f.name.as_str();
        let n = SAMPLE.chars().count();
        let (mut gids, mut present, mut count) = (vec![7u32; n], vec![7u8; n], 0);
        let st = daegun_font_glyph_ids(
            f.c,
            SAMPLE.as_ptr().cast(),
            SAMPLE.len(),
            gids.as_mut_ptr(),
            present.as_mut_ptr(),
            n,
            &mut count,
        );
        let c: Vec<_> = (0..count).map(|i| (present[i] != 0).then_some(gids[i])).collect();
        let want: Vec<_> = f.rust.glyph_ids(SAMPLE).map(|g| g.map(|g| g.0)).collect();
        d.eq(at, "glyph_ids", (st, c), (Status::Ok, want));
        let got = filled2(|cps, gids, l, c| daegun_font_coverage(f.c, cps, gids, l, c));
        let coverage: Vec<_> = f.rust.coverage().collect();
        let (cps, gids) = coverage.iter().map(|&(c, g)| (u32::from(c), g.0)).unzip();
        d.eq(at, "coverage", got, (Status::Ok, cps, gids));
        for cp in PROBES {
            let (mut g, mut has) = (0, false);
            let st = daegun_font_glyph_id(f.c, cp, &mut g);
            let sh = daegun_font_has_glyph(f.c, cp, &mut has);
            let want = match char::from_u32(cp) {
                Some(c) => {
                    let r = f.rust.glyph_id(c);
                    ((found(&r), r.map_or(0, |g| g.0)), (Status::Ok, f.rust.has_glyph(c)))
                }
                None => ((Status::InvalidArgument, 0), (Status::InvalidArgument, false)),
            };
            d.eq(at, "glyph_id, has_glyph", ((st, g), (sh, has)), want);
            for sel in [0xfe00, 0xfe0e, 0xfe0f, 0xe0100, 0x41, 0xd800] {
                let mut g = 0;
                let st = daegun_font_variation_glyph_id(f.c, cp, sel, &mut g);
                let want = match (char::from_u32(cp), char::from_u32(sel)) {
                    (Some(c), Some(s)) => {
                        let r = f.rust.variation_glyph_id(c, s);
                        (found(&r), r.map_or(0, |g| g.0))
                    }
                    _ => (Status::InvalidArgument, 0),
                };
                d.eq(at, "variation_glyph_id", (st, g), want);
            }
        }
        let mut dvo = 0.0;
        let st = daegun_font_default_vertical_origin(f.c, &mut dvo);
        d.eq(at, "default_vertical_origin", (st, bits(dvo)), (Status::Ok, bits(f.rust.default_vertical_origin())));
        let list = glyph_list(&f.rust);
        for &g in &list {
            let (mut class, mut mark) = (-1, 0);
            let st = daegun_font_glyph_class(f.c, g, &mut class);
            let rust = f.rust.glyph_class(crate::GlyphId(g)).map(|c| match c {
                GlyphClass::Base => GLYPH_CLASS_BASE,
                GlyphClass::Ligature => GLYPH_CLASS_LIGATURE,
                GlyphClass::Mark => GLYPH_CLASS_MARK,
                GlyphClass::Component => GLYPH_CLASS_COMPONENT,
            });
            d.eq(at, "glyph_class", (g, st, (st == Status::Ok).then_some(class)), (g, found(&rust), rust));
            let st = daegun_font_mark_attachment_class(f.c, g, &mut mark);
            let rust = f.rust.mark_attachment_class(crate::GlyphId(g)).map(core::num::NonZeroU32::get);
            d.eq(at, "mark_attachment_class", (g, st, (st == Status::Ok).then_some(mark)), (g, found(&rust), rust));
            // Into room for the longest name and its NUL, which every name fits, the NUL counted.
            let (mut name, mut n) = ([0 as core::ffi::c_char; GLYPH_NAME_ROOM], usize::MAX);
            let st = daegun_font_glyph_name(f.c, g, name.as_mut_ptr(), name.len(), &mut n);
            let got = (st == Status::Ok).then(|| name.get(..n).map(|b| b.iter().map(|&c| c as u8).collect::<Vec<_>>()));
            let rust = f.rust.glyph_name(crate::GlyphId(g)).filter(|s| !s.contains('\0'));
            let want = rust.map(|s| Some([s.as_bytes(), &[0]].concat()));
            d.eq(at, "glyph_name", (g, st, got), (g, found(&rust), want));
        }
        let mut names = null_mut();
        let st = daegun_font_glyph_names(f.c, &mut names);
        let c = take_strs(names);
        let want: Vec<_> = f.rust.glyph_names().map(|n| n.filter(|s| !s.contains('\0')).unwrap_or("").into()).collect();
        d.eq(at, "an empty name", f.rust.glyph_names().any(|n| n == Some("")), false);
        d.eq(at, "glyph_names", (st, c), (Status::Ok, want));

        for place in f.places() {
            let at = &alloc::format!("{at} @{}", place.label);
            for (code, o) in
                [(crate::ffi::HORIZONTAL, Orientation::Horizontal), (crate::ffi::VERTICAL, Orientation::Vertical)]
            {
                let mut c = vec![0.0; list.len()];
                let st = daegun_font_advances(f.c, list.as_ptr(), list.len(), place.c, code, c.as_mut_ptr());
                let glyphs: Vec<_> = list.iter().map(|&g| crate::GlyphId(g)).collect();
                let mut want = Vec::new();
                f.rust.advances(&glyphs, &place.rust, o, &mut want);
                d.eq(
                    at,
                    "advances",
                    (st, c.iter().map(|&v| bits(v)).collect::<Vec<_>>()),
                    (Status::Ok, want.iter().map(|&v| bits(v)).collect()),
                );
                for &g in &list {
                    let mut a = 0.0;
                    let st = daegun_font_advance(f.c, g, place.c, code, &mut a);
                    d.eq(
                        at,
                        "advance",
                        (g, st, bits(a)),
                        (g, Status::Ok, bits(f.rust.advance(crate::GlyphId(g), &place.rust, o))),
                    );
                    let (st, values, present) =
                        filled2(|v, p, l, c| daegun_font_ligature_carets(f.c, g, place.c, code, v, p, l, c));
                    let c: Vec<_> = values.into_iter().zip(present).map(|(v, p)| (p != 0).then_some(bits(v))).collect();
                    let want: Vec<_> =
                        f.rust.ligature_carets(crate::GlyphId(g), &place.rust, o).iter().map(|v| v.map(bits)).collect();
                    d.eq(at, "ligature_carets", (g, st, c), (g, Status::Ok, want));
                }
            }
            for &g in &list {
                let mut b = crate::Bounds { x_min: 0.0, y_min: 0.0, x_max: 0.0, y_max: 0.0 };
                let st = daegun_font_glyph_bounds(f.c, g, place.c, &mut b);
                let rust = f.rust.glyph_bounds(crate::GlyphId(g), &place.rust);
                let c = (st == Status::Ok).then_some([bits(b.x_min), bits(b.y_min), bits(b.x_max), bits(b.y_max)]);
                let want = rust.map(|b| [bits(b.x_min), bits(b.y_min), bits(b.x_max), bits(b.y_max)]);
                d.eq(at, "glyph_bounds", (g, st, c), (g, found(&rust), want));
                let mut o = 0.0;
                let st = daegun_font_vertical_origin(f.c, g, place.c, &mut o);
                let want = bits(f.rust.vertical_origin(crate::GlyphId(g), &place.rust));
                d.eq(at, "vertical_origin", (g, st, bits(o)), (g, Status::Ok, want));
            }
        }
    });
    d.done("glyphs");
}
