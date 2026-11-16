use super::*;

fn be16(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at + 2).map(|s| usize::from(u16::from_be_bytes([s[0], s[1]])))
}

fn be32(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

// A region's axes, each bound as its bits.
fn axes_of(region: &[format::RegionAxis]) -> Vec<[u64; 3]> {
    region.iter().map(|a| [a.start, a.peak, a.end].map(f64::to_bits)).collect()
}

// Indices a check walks: the first 64, the last few and two past the end.
fn some_of(n: usize) -> impl Iterator<Item = usize> {
    (0..n.min(64)).chain(n.saturating_sub(3).max(64)..n + 2)
}

// A freed lookup is the thread's next, its copy's room kept and its bytes the new ones: a trimmed array of one
// glyph's value, then of another's.
#[test]
fn a_freed_lookup_is_the_next_one() {
    let trimmed = |first: u8, value: u8| [0, 8, 0, first, 0, 1, 0, value];
    let parse = |bytes: &[u8]| unsafe {
        let mut lookup = core::ptr::null_mut();
        assert_eq!(daegun_aat_lookup_parse(bytes.as_ptr(), bytes.len(), 100, &mut lookup), Status::Ok);
        lookup
    };
    let value = |lookup, glyph| unsafe {
        let mut v = 0;
        (daegun_aat_lookup_value(lookup, glyph, &mut v), v)
    };
    let first = parse(&trimmed(5, 7));
    assert_eq!(value(first, 5), (Status::Ok, 7));
    unsafe { daegun_aat_lookup_free(first) };
    let second = parse(&trimmed(9, 3));
    assert!(second == first, "the lookup freed last was not reused");
    assert_eq!((value(second, 9), value(second, 5).0), ((Status::Ok, 3), Status::Absent));
    unsafe { daegun_aat_lookup_free(second) };
    // One whose bytes don't read is handed back, and the next parse gets its box.
    let mut broken = core::ptr::null_mut();
    assert_eq!(unsafe { daegun_aat_lookup_parse([0, 3].as_ptr(), 2, 100, &mut broken) }, Status::Unsupported);
    assert!(broken.is_null());
    let third = parse(&trimmed(5, 7));
    assert!(third == first, "a failed parse kept the spare");
    unsafe { daegun_aat_lookup_free(third) };
}

// A Coverage table of each format, at an offset, then each way one fails: C's glyphs and status as Rust's, and a
// malformed table answered before a NULL buffer with room, as it was when C copied Rust's list.
#[test]
fn coverage_glyphs_answers_as_rust() {
    use crate::ffi::testing::{filled, status};
    let tables: [&[u8]; 11] = [
        &[0, 1, 0, 3, 0, 3, 0, 7, 0, 9],
        &[0, 2, 0, 2, 0, 1, 0, 3, 0, 0, 0, 6, 0, 6, 0, 3],
        &[0, 3, 0, 0, 2, 1, 0, 0, 1, 0, 5],
        &[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0, 0, 0],
        &[0, 1, 0, 0],
        &[0, 1, 0, 3, 0, 3],
        &[0, 2, 0, 1, 0, 1],
        &[0, 2, 0, 1, 0, 5, 0, 4, 0, 0],
        &[0, 2, 0, 2, 0, 1, 0, 3, 0, 0, 0, 3, 0, 4, 0, 3],
        &[0, 5, 0, 0],
        &[0],
    ];
    for (i, table) in tables.into_iter().enumerate() {
        let padded = [&[9, 9, 9][..], table].concat();
        let rust = format::coverage_glyphs(&padded, 3);
        let got = unsafe { filled(|o, l, c| daegun_coverage_glyphs(padded.as_ptr(), padded.len(), 3, o, l, c)) };
        assert_eq!(got, (status(&rust), rust.clone().unwrap_or_default()), "table {i}");
        let (mut count, none) = (0, core::ptr::null_mut());
        let st = unsafe { daegun_coverage_glyphs(padded.as_ptr(), padded.len(), 3, none, 1, &mut count) };
        assert_eq!(st, if rust.is_ok() { Status::Null } else { status(&rust) }, "table {i} with a NULL buffer");
    }
    assert_eq!(format::coverage_glyphs(tables[3], 0), Ok(alloc::vec![65536, 65537, 65538]));
}

// A map of 65,536 empty tables, one more than a font's directory counts: refused as out of range with the out left
// NULL, and built once one goes.
#[test]
#[cfg_attr(miri, ignore = "65,536 tables take hours under Miri; a_table_map_walks_in_tag_order checks the same calls")]
fn a_map_of_more_tables_than_a_directory_counts_builds_nothing() {
    use crate::ffi::font::{daegun_table_map_free, daegun_table_map_new, daegun_table_map_remove, daegun_table_map_set};
    unsafe {
        let map = daegun_table_map_new();
        for tag in 0..=u32::from(u16::MAX) {
            assert_eq!(daegun_table_map_set(map, tag, [].as_ptr(), 0), Status::Ok);
        }
        let mut blob = core::ptr::null_mut();
        assert_eq!(daegun_table_map_build(map, &mut blob), Status::InvalidArgument);
        assert!(blob.is_null());
        assert_eq!(daegun_table_map_remove(map, 0), Status::Ok);
        assert_eq!(daegun_table_map_build(map, &mut blob), Status::Ok);
        assert_eq!(be16(&crate::ffi::testing::take_blob(blob), 4), Some(65_535));
        daegun_table_map_free(map);
    }
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::Tag;
    use crate::ffi::font::{daegun_table_map_free, daegun_table_map_new, daegun_table_map_set};
    use crate::ffi::testing::{
        Diffs, buffered, drawn, each_font, found, glyph_list, lent, rust_path, short_of, status,
    };
    use crate::ffi::testing::{filled, take_blob};
    use crate::outline::Path;
    use core::ptr::null_mut;
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        let name = &f.name;
        let table = |tag: &[u8; 4]| f.rust.table(Tag::new(tag));
        let tags: Vec<Tag> = f.rust.table_tags().collect();
        let map = daegun_table_map_new();
        let mut tables = alloc::collections::BTreeMap::new();
        for &tag in &tags {
            let bytes = f.rust.table(tag).unwrap_or(&[]);
            let _ = daegun_table_map_set(map, tag.0, bytes.as_ptr(), bytes.len());
            tables.insert(tag, bytes);
        }
        let mut blob = null_mut();
        let st = daegun_table_map_build(map, &mut blob);
        daegun_table_map_free(map);
        d.eq(name, "table_map_build", (st, take_blob(blob)), (Status::Ok, crate::raw::build_font(&tables).unwrap()));

        if let (Some(head), Some(maxp), Some(loca), Some(glyf)) =
            (table(b"head"), table(b"maxp"), table(b"loca"), table(b"glyf"))
        {
            let format = head.get(50..52).map_or(0, |s| i16::from_be_bytes([s[0], s[1]]));
            let num = be16(maxp, 4).unwrap_or(0);
            let got = filled(|o, l, c| daegun_parse_loca(loca.as_ptr(), loca.len(), format, num, o, l, c));
            let rust = crate::raw::parse_loca(loca, format, num);
            d.eq(name, "parse_loca", got, (Status::Ok, rust.clone()));
            for g in glyph_list(&f.rust) {
                for extended in [false, true] {
                    let c = drawn(|pen| {
                        daegun_outline_glyf_bytes(
                            glyf.as_ptr(),
                            glyf.len(),
                            extended,
                            rust.as_ptr(),
                            rust.len(),
                            g,
                            pen,
                        )
                    });
                    let mut p = Path::default();
                    let r = crate::raw::outline_glyf_bytes(glyf, extended, &rust, g, &mut p);
                    d.eq(name, "outline_glyf_bytes", (g, extended, c), (g, extended, (status(&r), rust_path(&p))));
                }
            }
        }

        let places = f.places();
        let gdef = table(b"GDEF").filter(|t| be16(t, 0) == Some(1) && be16(t, 2) >= Some(3));
        let gdef_store = gdef.and_then(|t| be32(t, 14).filter(|&o| o != 0).map(|o| (t, o)));
        let stores = [(b"HVAR", 4, true), (b"VVAR", 4, true), (b"MVAR", 10, false)]
            .into_iter()
            .filter_map(|(tag, at, wide)| {
                let t = table(tag)?;
                let off = if wide { be32(t, at) } else { be16(t, at) }?;
                (off != 0).then_some((tag, t, off))
            })
            .chain(gdef_store.map(|(t, o)| (b"GDEF", t, o)));
        for (tag, t, off) in stores {
            let at = &alloc::format!("{name} {}", core::str::from_utf8(tag).unwrap_or("?"));
            let mut ivs = null_mut();
            let st = daegun_ivs_parse(t.as_ptr(), t.len(), off, &mut ivs);
            let r = format::ItemVariationStore::parse(t, off);
            d.eq(at, "ivs_parse", st, status(&r));
            let Ok(store) = r else { continue };
            let (mut axes, mut regions, mut ivds) = (0, 0, 0);
            let _ = (daegun_ivs_axis_count(ivs, &mut axes), daegun_ivs_region_count(ivs, &mut regions));
            let _ = daegun_ivs_ivd_count(ivs, &mut ivds);
            d.eq(
                at,
                "ivs counts",
                (axes, regions, ivds),
                (store.axis_count, store.regions.len(), store.ivd_data.len()),
            );
            for i in some_of(store.regions.len()) {
                let mut n = 0;
                let region = daegun_ivs_region(ivs, i, &mut n);
                let got = (!region.is_null()).then(|| axes_of(lent(region, n)));
                d.eq(at, "ivs_region", (i, got), (i, store.regions.get(i).map(|r| axes_of(r))));
            }
            for i in 0..store.ivd_data.len() + 1 {
                let mut rows = 0;
                let st = daegun_ivs_ivd_rows(ivs, i, &mut rows);
                let ivd = store.ivd_data.get(i);
                d.eq(
                    at,
                    "ivs_ivd_rows",
                    (i, st, rows),
                    (i, if ivd.is_some() { Status::Ok } else { Status::InvalidArgument }, ivd.map_or(0, |v| v.rows())),
                );
                let mut n = 0;
                let indices = daegun_ivs_ivd_region_indices(ivs, i, &mut n);
                let got = (!indices.is_null()).then(|| lent(indices, n).to_vec());
                d.eq(at, "ivs_ivd_region_indices", (i, got), (i, ivd.map(|v| v.region_indices.clone())));
                let Some(ivd) = ivd else { continue };
                for row in some_of(ivd.rows()) {
                    let mut n = 0;
                    let deltas = daegun_ivs_ivd_row(ivs, i, row, &mut n);
                    let got = (!deltas.is_null()).then(|| lent(deltas, n).to_vec());
                    d.eq(at, "ivs_ivd_row", (i, row, got), (i, row, ivd.row(row).map(<[i32]>::to_vec)));
                }
            }
            for place in &places {
                let coords = place.rust.normalized();
                let got = buffered(
                    |out, len, count| daegun_ivs_region_scalars(ivs, coords.as_ptr(), coords.len(), out, len, count),
                    store.regions.len() + 1,
                );
                let scalars = store.region_scalars(coords);
                let (short, count) = short_of(scalars.len());
                let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                let want = (short, count, Status::Ok, bits(&scalars));
                d.eq(
                    at,
                    "ivs_region_scalars",
                    (place.label.as_str(), (got.0, got.1, got.2, bits(&got.3))),
                    (place.label.as_str(), want),
                );
                for outer in 0..store.ivd_data.len() + 1 {
                    let rows = store.ivd_data.get(outer).map_or(1, |v| v.rows().min(32) + 1);
                    for inner in 0..rows {
                        let mut delta = 0.0;
                        let st = daegun_ivs_delta(ivs, outer, inner, scalars.as_ptr(), scalars.len(), &mut delta);
                        let index = format::DeltaSetIndex { outer: outer as u32, inner: inner as u32 };
                        let want = store.delta(index, &scalars);
                        d.eq(
                            at,
                            "ivs_delta",
                            (place.label.as_str(), outer, inner, st, delta.to_bits()),
                            (place.label.as_str(), outer, inner, Status::Ok, want.to_bits()),
                        );
                    }
                }
            }
            daegun_ivs_free(ivs);
        }

        let maps = [(b"HVAR", [8, 12, 16, 0]), (b"VVAR", [8, 12, 16, 20])];
        for (tag, offsets) in maps {
            let Some(t) = table(tag) else { continue };
            for at in offsets.into_iter().filter(|&a| a != 0) {
                let Some(base) = be32(t, at).filter(|&o| o != 0) else { continue };
                let what = &alloc::format!("{name} {} @{at}", core::str::from_utf8(tag).unwrap_or("?"));
                let mut map = null_mut();
                let st = daegun_delta_set_index_map_parse(t.as_ptr(), t.len(), base, &mut map);
                let r = format::parse_delta_set_index_map(t, base);
                d.eq(what, "delta_set_index_map_parse", st, status(&r));
                let Ok(entries) = r else { continue };
                let mut count = 0;
                let _ = daegun_delta_set_index_map_count(map, &mut count);
                d.eq(what, "delta_set_index_map_count", count, entries.len());
                for i in some_of(entries.len()) {
                    let (mut outer, mut inner) = (0, 0);
                    let st = daegun_delta_set_index_map_lookup(map, i, &mut outer, &mut inner);
                    d.eq(
                        what,
                        "delta_set_index_map_lookup",
                        (i, st, (outer, inner)),
                        (i, Status::Ok, format::delta_set_index_map_lookup(&entries, i).into()),
                    );
                }
                daegun_delta_set_index_map_free(map);
            }
        }

        let (mut c_store, rust_store) =
            (null_mut(), gdef_store.and_then(|(t, o)| format::ItemVariationStore::parse(t, o).ok()));
        if let Some((t, o)) = gdef_store {
            let _ = daegun_ivs_parse(t.as_ptr(), t.len(), o, &mut c_store);
        }
        for tag in [b"GSUB", b"GPOS"] {
            let Some(t) = table(tag) else { continue };
            let what = &alloc::format!("{name} {}", core::str::from_utf8(tag).unwrap_or("?"));
            let mut v = null_mut();
            let st = daegun_feature_variations_parse(t.as_ptr(), t.len(), &mut v);
            let r = format::FeatureVariations::parse(t);
            d.eq(what, "feature_variations_parse", st, found(&r));
            let Some(rust) = r else { continue };
            let mut at = null_mut();
            let _ = daegun_feature_variations_at(t.as_ptr(), t.len(), rust.offset(), &mut at);
            let features = be16(t, 6).and_then(|list| be16(t, list)).unwrap_or(0).min(300);
            for place in &places {
                let coords: Vec<i32> = place.rust.normalized().iter().map(|&x| (x * 16384.0).round() as i32).collect();
                for (c_ivs, r_ivs) in [(core::ptr::null(), None), (c_store.cast_const(), rust_store.as_ref())] {
                    let want = rust.find(&coords, r_ivs);
                    for handle in [v, at] {
                        let mut index = 0;
                        let st =
                            daegun_feature_variations_find(handle, coords.as_ptr(), coords.len(), c_ivs, &mut index);
                        let got = (st, (st == Status::Ok).then_some(index));
                        d.eq(
                            what,
                            "feature_variations_find",
                            (place.label.as_str(), got),
                            (place.label.as_str(), (found(&want), want)),
                        );
                    }
                    for variation in want.into_iter().chain([0, 1]) {
                        for feature in 0..features as u16 + 1 {
                            let mut offset = 0;
                            let st = daegun_feature_variations_substitute(v, variation, feature, &mut offset);
                            let want = rust.substitute(variation, feature);
                            let w = (found(&want), want.map(|s| match s {
                                format::Substitution::Feature(at) => at,
                                format::Substitution::Empty => 0,
                            }));
                            d.eq(
                                what,
                                "feature_variations_substitute",
                                (variation, feature, st, (st == Status::Ok).then_some(offset)),
                                (variation, feature, w.0, w.1),
                            );
                        }
                    }
                }
            }
            daegun_feature_variations_free(v);
            daegun_feature_variations_free(at);
        }
        daegun_ivs_free(c_store);
    });
    d.done("raw");
}

// loca read from the buffer it's parsed into, or a count set inside the buffer, answers INVALID_ARGUMENT before
// anything is written; buffers apart parse.
#[test]
fn a_buffer_overlapping_what_a_call_reads_is_refused() {
    let loca = [0u8, 0, 0, 2, 0, 4, 0, 6];
    let (mut buf, mut count) = ([0usize; 8], 0);
    let out = buf.as_mut_ptr();
    unsafe { core::ptr::copy_nonoverlapping(loca.as_ptr(), out.cast::<u8>(), 8) };
    let in_place = unsafe { daegun_parse_loca(out.cast::<u8>(), 8, 0, 3, out, 8, &mut count) };
    assert_eq!((in_place, count), (Status::InvalidArgument, 0));
    assert!(super::super::testing::last_error().contains("shares bytes"));
    assert_eq!(unsafe { core::slice::from_raw_parts(out.cast::<u8>(), 8) }, loca, "nothing written");
    let counted_inside = unsafe { daegun_parse_loca(loca.as_ptr(), 8, 0, 3, out, 8, out.wrapping_add(2)) };
    assert_eq!(counted_inside, Status::InvalidArgument);
    let apart = unsafe { daegun_parse_loca(loca.as_ptr(), 8, 0, 3, out, 8, &mut count) };
    assert_eq!((apart, count, &buf[..4]), (Status::Ok, 4, &[0, 4, 8, 12][..]));
}
