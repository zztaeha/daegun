use super::*;
use alloc::string::String;

#[test]
fn every_cache_has_a_code() {
    for v in [Cache::Shape, Cache::Instance, Cache::Outline, Cache::Raster] {
        let code = match v {
            Cache::Shape => CACHE_SHAPE,
            Cache::Instance => CACHE_INSTANCE,
            Cache::Outline => CACHE_OUTLINE,
            Cache::Raster => CACHE_RASTER,
        };
        assert_eq!(cache_of(code), Ok(v));
    }
    assert!(cache_of(4).is_err());
}

// Tables set out of order walk in tag order, a second set of a tag replacing its bytes and a remove closing the gap.
#[test]
fn a_table_map_walks_in_tag_order() {
    let tag = |t: &[u8; 4]| u32::from_be_bytes(*t);
    let map = daegun_table_map_new();
    unsafe {
        for (i, t) in [b"glyf", b"OS/2", b"head", b"cmap"].into_iter().enumerate() {
            assert_eq!(daegun_table_map_set(map, tag(t), [i as u8; 3].as_ptr(), 3), Status::Ok);
        }
        assert_eq!(daegun_table_map_set(map, tag(b"head"), [9u8].as_ptr(), 1), Status::Ok);
        assert_eq!(daegun_table_map_remove(map, tag(b"glyf")), Status::Ok);
        assert_eq!(daegun_table_map_remove(map, tag(b"glyf")), Status::Ok);
        let mut walked = Vec::new();
        for i in 0..4 {
            let (mut t, mut view) = (0, Bytes { data: core::ptr::null(), len: 0 });
            let st = (daegun_table_map_tag_at(map, i, &mut t), daegun_table_map_bytes_at(map, i, &mut view));
            walked.push((st, t.to_be_bytes(), crate::ffi::testing::lent(view.data, view.len).to_vec()));
        }
        let (ok, past) = ((Status::Ok, Status::Ok), (Status::InvalidArgument, Status::InvalidArgument));
        let want = [
            (ok, *b"OS/2", alloc::vec![1; 3]),
            (ok, *b"cmap", alloc::vec![3; 3]),
            (ok, *b"head", alloc::vec![9]),
            (past, [0; 4], alloc::vec![]),
        ];
        assert_eq!(walked, want);
        let mut view = Bytes { data: core::ptr::null(), len: 0 };
        assert_eq!(daegun_table_map_get(map, tag(b"cmap"), &mut view), Status::Ok);
        assert_eq!(crate::ffi::testing::lent(view.data, view.len), [3; 3]);
        assert_eq!(daegun_table_map_get(map, tag(b"glyf"), &mut view), Status::Absent);
        daegun_table_map_free(map);
    }
}

// The bytes outlive every font opened from them, as the static open asks; each face against the same face copied.
#[test]
fn a_static_open_reads_each_face_as_a_copying_open_does() {
    use core::ptr::{null, null_mut};
    for rel in ["test-fixtures/EBGaramond-InterVariable.ttc", "inter/InterVariable.ttf"] {
        let held = alloc::boxed::Box::into_raw(crate::ffi::testing::asset(rel).into_boxed_slice());
        let bytes: &[u8] = unsafe { &*held };
        let mut faces = 0;
        assert_eq!(unsafe { daegun_face_count(bytes.as_ptr(), bytes.len(), &mut faces) }, Status::Ok);
        assert_eq!(faces, Font::face_count(bytes), "{rel}");
        for i in 0..faces {
            let (mut placed, mut copied) = (null_mut(), null_mut());
            let st = unsafe { daegun_font_open_static(bytes.as_ptr(), bytes.len(), i, &mut placed) };
            assert_eq!(st, Status::Ok, "{rel}#{i}");
            let st = unsafe { daegun_font_open_collection(bytes.as_ptr(), bytes.len(), i, &mut copied) };
            assert_eq!(st, Status::Ok, "{rel}#{i}");
            let (p, c) = unsafe { (&*placed, &*copied) };
            assert!(p.table_tags().eq(c.table_tags()), "{rel}#{i}: the tables differ");
            for t in c.table_tags() {
                assert_eq!(p.table(t), c.table(t), "{rel}#{i}: {t:?}");
            }
            unsafe { (daegun_font_free(placed), daegun_font_free(copied)) };
        }
        let mut past = null_mut();
        let st = unsafe { daegun_font_open_static(bytes.as_ptr(), bytes.len(), faces, &mut past) };
        assert!(st == Status::InvalidArgument && past.is_null(), "{rel}: face {faces} opened");
        // Each font is freed, so the bytes go back, as Miri wants nothing leaked.
        drop(unsafe { alloc::boxed::Box::from_raw(held) });
    }
    let mut none = null_mut();
    assert_eq!(unsafe { daegun_font_open_static(null(), 4, 0, &mut none) }, Status::Null);
}

// A collection stating two faces that ends after the first's offset: daegun_face_count counts one, so each open
// refuses index 1 as an argument past the faces, not as a malformed file.
#[test]
fn a_collection_index_past_the_offsets_it_holds_is_an_invalid_argument() {
    use core::ptr::null_mut;
    let ttc = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\x0c";
    let mut faces = 0;
    assert_eq!(unsafe { daegun_face_count(ttc.as_ptr(), ttc.len(), &mut faces) }, Status::Ok);
    assert_eq!(faces, 1);
    let (mut copied, mut placed, mut owned) = (null_mut(), null_mut(), null_mut());
    let buffer = daegun_font_buffer_new(ttc.len());
    let opens = unsafe {
        core::ptr::copy_nonoverlapping(ttc.as_ptr(), buffer, ttc.len());
        [
            daegun_font_open_collection(ttc.as_ptr(), ttc.len(), 1, &mut copied),
            daegun_font_open_static(ttc.as_ptr(), ttc.len(), 1, &mut placed),
            daegun_font_open_owned(buffer, ttc.len(), 1, &mut owned),
        ]
    };
    assert_eq!(opens, [Status::InvalidArgument; 3]);
    assert!(copied.is_null() && placed.is_null() && owned.is_null(), "a refused open delivered a font");
}

// Two blobs of one location's instance read the one buffer its cache holds. The font is a small one, so Miri
// instances it in seconds.
#[test]
fn an_instance_is_lent_from_its_cache() {
    use core::ptr::null_mut;
    let font = crate::ffi::testing::CFont::of(crate::ffi::testing::asset("test-fixtures/gvar-points.ttf"));
    let coord = Coord { tag: u32::from_be_bytes(*b"wght"), value: 560.0 };
    let (mut at, mut a, mut b) = (null_mut(), null_mut(), null_mut());
    unsafe {
        assert_eq!(daegun_font_location(font.handle, &coord, 1, &mut at), Status::Ok);
        assert_eq!(daegun_font_instance(font.handle, at, &mut a), Status::Ok);
        assert_eq!(daegun_font_instance(font.handle, at, &mut b), Status::Ok);
        let (mut n, mut m) = (0, 0);
        let (x, y) = (crate::ffi::base::daegun_blob_data(a, &mut n), crate::ffi::base::daegun_blob_data(b, &mut m));
        assert!(n > 0 && n == m && core::ptr::eq(x, y), "two instances at one location were copied apart");
        crate::ffi::base::daegun_blob_free(a);
        crate::ffi::base::daegun_blob_free(b);
        daegun_location_free(at);
    }
}

// A location freed is the thread's next, and holds the next one's coordinates alone, though the last read its own.
#[test]
fn a_freed_location_is_the_next_one() {
    use core::ptr::null_mut;
    let font = crate::ffi::testing::CFont::of(crate::ffi::testing::asset("inter/InterVariable.ttf"));
    let wght = |value| Coord { tag: u32::from_be_bytes(*b"wght"), value };
    let at = |value| unsafe {
        let mut at = null_mut();
        assert_eq!(daegun_font_location(font.handle, &wght(value), 1, &mut at), Status::Ok);
        at
    };
    let coords = |at| unsafe {
        let mut k = 0;
        crate::ffi::testing::lent(daegun_location_coords(at, &mut k), k).iter().map(|c| c.value).collect::<Vec<_>>()
    };
    let first = at(300.0);
    assert_eq!(coords(first), [300.0]);
    unsafe { daegun_location_free(first) };
    let second = at(700.0);
    assert!(second == first, "the location freed last was not reused");
    assert_eq!(coords(second), [700.0], "a reused location kept the last one's coordinates");
    unsafe { daegun_location_free(second) };
}

// A font freed leaves the thread's next its box, holding the next font alone.
#[test]
fn a_freed_font_is_the_next_ones_box() {
    let open = |rel| unsafe {
        let bytes = crate::ffi::testing::asset(rel);
        let mut font = core::ptr::null_mut();
        assert_eq!(daegun_font_open(bytes.as_ptr(), bytes.len(), &mut font), Status::Ok);
        font
    };
    let glyphs = |font| unsafe {
        let mut n = 0;
        assert_eq!(daegun_font_glyph_count(font, &mut n), Status::Ok);
        n
    };
    let first = open("inter/InterVariable.ttf");
    let inter = glyphs(first);
    unsafe { daegun_font_free(first) };
    let second = open("eb-garamond/EBGaramond.ttf");
    assert!(second == first, "the font freed last left no box");
    let garamond = crate::Font::from_bytes(&crate::ffi::testing::asset("eb-garamond/EBGaramond.ttf")).unwrap();
    assert_ne!(garamond.glyph_count(), inter);
    assert_eq!(glyphs(second), garamond.glyph_count(), "the reused box holds the last font");
    unsafe { daegun_font_free(second) };
}

// A single name Rust hands out has a byte to spare, so C's NUL takes it and the string moves in without a copy; one
// holding a NUL of its own has it replaced, as a copy does.
#[test]
fn a_name_moves_into_c_without_a_copy() {
    use crate::ffi::handle::OwnedStr;
    let font = crate::Font::from_bytes(&crate::ffi::testing::asset("inter/InterVariable.ttf")).unwrap();
    let at = font.location(&[]);
    let mut strings: Vec<String> = (0..26).filter_map(|id| font.name(NameId(id), &at)).collect();
    strings.extend(font.family_name(&at));
    assert!(strings.len() > 10);
    for (given, want) in strings.into_iter().map(|s| (s, None)).chain([(String::from("b\0c"), Some("b\u{fffd}c"))]) {
        let (text, data) = (String::from(want.unwrap_or(&given)), given.as_ptr());
        let raw = OwnedStr::from_string(given).into_raw();
        assert!(want.is_some() || raw.cast::<u8>().cast_const() == data, "{text:?} was copied");
        let bytes = unsafe { core::slice::from_raw_parts(raw.cast::<u8>(), text.len() + 1) };
        assert_eq!(bytes, [text.as_bytes(), &[0]].concat());
        drop(unsafe { alloc::ffi::CString::from_raw(raw) });
    }
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::ffi::testing::{Diffs, bits, c_text, each_font, found, lent, status, str_of, take_blob, take_text};
    use crate::ffi::testing::{filled, take_localized_names, take_names};
    use core::ptr::null_mut;
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        let at = f.name.as_str();
        let (mut upm, mut count, mut variable) = (0u16, 0u32, false);
        let st = daegun_font_units_per_em(f.c, &mut upm);
        d.eq(at, "units_per_em", (st, upm), (Status::Ok, f.rust.units_per_em()));
        let st = daegun_font_glyph_count(f.c, &mut count);
        d.eq(at, "glyph_count", (st, count), (Status::Ok, f.rust.glyph_count()));
        let st = daegun_font_is_variable(f.c, &mut variable);
        d.eq(at, "is_variable", (st, variable), (Status::Ok, f.rust.is_variable()));

        let (st, axes) = filled(|o, l, c| daegun_font_axes(f.c, o, l, c));
        let c: Vec<_> = (axes.iter())
            .map(|a| (a.tag, a.name_id, a.hidden, bits(a.min), bits(a.default_value), bits(a.max)))
            .collect();
        let rust: Vec<_> = (f.rust.axes().iter())
            .map(|a| (a.tag.0, a.name_id.0, a.hidden, bits(a.min), bits(a.default), bits(a.max)))
            .collect();
        d.eq(at, "axes", (st, c), (Status::Ok, rust));

        let mut list = null_mut();
        let st = daegun_font_named_instances(f.c, &mut list);
        d.eq(at, "named_instances", st, Status::Ok);
        let rust = f.rust.named_instances();
        let mut n = 0;
        let _ = daegun_named_instances_count(list, &mut n);
        d.eq(at, "named instance count", n, rust.len());
        for (i, inst) in rust.iter().enumerate() {
            let mut s = Str::EMPTY;
            let st = daegun_named_instances_name(list, i, &mut s);
            let name = (st == Status::Ok).then(|| str_of(s));
            d.eq(at, "instance name", (st, name), (found(&inst.name), inst.name.as_deref().map(c_text)));
            let st = daegun_named_instances_postscript_name(list, i, &mut s);
            let ps = (st == Status::Ok).then(|| str_of(s));
            let want = inst.postscript_name.as_deref().map(c_text);
            d.eq(at, "instance postscript name", (st, ps), (found(&inst.postscript_name), want));
            let mut k = 0;
            let coords: Vec<_> = lent(daegun_named_instances_coords(list, i, &mut k), k)
                .iter()
                .map(|c| (c.tag, bits(c.value)))
                .collect();
            let want: Vec<_> = inst.coords.iter().map(|&(t, v)| (t.0, bits(v))).collect();
            d.eq(at, "instance coords", coords, want);
        }
        daegun_named_instances_free(list);

        let want: Vec<u32> = f.rust.table_tags().map(|t| t.0).collect();
        d.eq(at, "table_tags", filled(|o, l, c| daegun_font_table_tags(f.c, o, l, c)), (Status::Ok, want.clone()));
        for tag in want.iter().copied().chain([u32::from_be_bytes(*b"zzzz")]) {
            let mut has = false;
            let st = daegun_font_has_table(f.c, tag, &mut has);
            d.eq(at, "has_table", (st, has), (Status::Ok, f.rust.has_table(crate::Tag(tag))));
            let mut view = Bytes { data: core::ptr::null(), len: 0 };
            let st = daegun_font_table(f.c, tag, &mut view);
            let c = (st == Status::Ok).then(|| lent(view.data, view.len).to_vec());
            let rust = f.rust.table(crate::Tag(tag));
            d.eq(at, "table", (st, c), (found(&rust), rust.map(<[u8]>::to_vec)));
        }

        for place in f.places() {
            let at = &alloc::format!("{at} @{}", place.label);
            let mut k = 0;
            let c: Vec<_> =
                lent(daegun_location_coords(place.c, &mut k), k).iter().map(|c| (c.tag, bits(c.value))).collect();
            let rust: Vec<_> = place.rust.coords().iter().map(|&(t, v)| (t.0, bits(v))).collect();
            d.eq(at, "location coords", c, rust);
            let c: Vec<_> = lent(daegun_location_normalized(place.c, &mut k), k).iter().map(|&v| bits(v)).collect();
            let rust: Vec<_> = place.rust.normalized().iter().map(|&v| bits(v)).collect();
            d.eq(at, "location normalized", c, rust);

            let mut t = null_mut();
            let st = daegun_font_family_name(f.c, place.c, &mut t);
            let rust = f.rust.family_name(&place.rust);
            d.eq(at, "family_name", (st, take_text(t)), (found(&rust), rust.as_deref().map(c_text)));
            let mut style = -1;
            let st = daegun_font_style(f.c, place.c, &mut style);
            let want = match f.rust.style(&place.rust) {
                FontStyle::Normal => STYLE_NORMAL,
                FontStyle::Italic => STYLE_ITALIC,
                FontStyle::Oblique => STYLE_OBLIQUE,
            };
            d.eq(at, "style", (st, style), (Status::Ok, want));
            for id in [0, 1, 2, 3, 4, 5, 6, 16, 17, 25, 256, 300] {
                let mut t = null_mut();
                let st = daegun_font_name(f.c, id, place.c, &mut t);
                let rust = f.rust.name(NameId(id), &place.rust);
                d.eq(at, "name", (id, st, take_text(t)), (id, found(&rust), rust.as_deref().map(c_text)));
            }
            let mut names = null_mut();
            let st = daegun_font_names(f.c, place.c, &mut names);
            let want: Vec<(u16, String)> = f.rust.names(&place.rust).iter().map(|(id, s)| (id.0, c_text(s))).collect();
            d.eq(at, "names", (st, take_names(names)), (Status::Ok, want));
            for id in [1, 2, 4, 16, 300] {
                let mut names = null_mut();
                let st = daegun_font_localized_names(f.c, id, place.c, &mut names);
                let rust = f.rust.localized_names(NameId(id), &place.rust);
                let want: Vec<_> = rust.iter().map(|n| (c_text(&n.language), c_text(&n.name))).collect();
                d.eq(at, "localized_names", (id, st, take_localized_names(names)), (id, Status::Ok, want));
            }
            if !place.whole {
                continue;
            }
            let mut blob = null_mut();
            let st = daegun_font_instance(f.c, place.c, &mut blob);
            let rust = f.rust.instance(&place.rust);
            d.eq(at, "instance", (st, take_blob(blob)), (status(&rust), rust.map(|b| b.to_vec()).unwrap_or_default()));
            let mut map = null_mut();
            let st = daegun_font_instance_tables(f.c, place.c, &mut map);
            let rust = f.rust.instance_tables(&place.rust);
            let mut c = Vec::new();
            let mut n = 0;
            let _ = daegun_table_map_count(map, &mut n);
            for i in 0..n {
                let (mut tag, mut view) = (0, Bytes { data: core::ptr::null(), len: 0 });
                let _ = daegun_table_map_tag_at(map, i, &mut tag);
                let _ = daegun_table_map_bytes_at(map, i, &mut view);
                c.push((tag, lent(view.data, view.len).to_vec()));
            }
            daegun_table_map_free(map);
            let want: Vec<_> = rust.as_ref().map_or(Vec::new(), |m| m.iter().map(|(t, b)| (t.0, b.to_vec())).collect());
            d.eq(at, "instance_tables", (st, c), (status(&rust), want));
        }
    });
    for path in crate::ffi::testing::corpus() {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let mut faces = usize::MAX;
        let st = unsafe { daegun_face_count(bytes.as_ptr(), bytes.len(), &mut faces) };
        d.eq(&alloc::format!("{}", path.display()), "face_count", (st, faces), (Status::Ok, Font::face_count(&bytes)));
    }
    d.done("font");
}
