use super::*;

fn map(tables: &[(&str, &[u8])]) -> Tables {
    tables.iter().map(|&(tag, bytes)| (String::from(tag), TableBytes::from(bytes.to_vec()))).collect()
}

// MAXP counts in 24 bits wherever it reads whole, 0 included, as far as GLYF and LOCA or CFF2's CharStrings hold
// glyphs; past them, or where it doesn't read whole, maxp counts, and a font with neither counts none.
#[test]
fn maxp_s_24_bit_count_comes_first_where_it_reads_whole_and_the_glyphs_are_held() {
    let maxp: &[u8] = &[0, 0, 0x50, 0, 0, 7];
    let wide = |version: u32, count: u32, len: usize| {
        let mut t = [&version.to_be_bytes()[..], &count.to_be_bytes()[1..]].concat();
        t.resize(len, 0);
        t
    };
    let mut head = vec![0; 54];
    head[51] = 1;
    let loca = vec![0; 4 * 70_001];
    // A CFF2 header, a Top DICT naming CharStrings at 7, and an INDEX of 70,000 empty glyphs.
    let cff2 = [&[2, 0, 5, 0, 2, 7 + 139, 17][..], &70_000u32.to_be_bytes(), &[1], &vec![1; 70_001]].concat();
    let count = |tables: &[(&str, &[u8])]| num_glyphs(&map(tables));
    let glyf = |wide: &[u8]| count(&[("maxp", maxp), ("MAXP", wide), ("head", &head), ("GLYF", &[0]), ("LOCA", &loca)]);
    let cff = |wide: &[u8]| count(&[("maxp", maxp), ("MAXP", wide), ("CFF2", &cff2)]);
    assert_eq!(glyf(&wide(0x5000, 70_000, 7)), Some(70_000));
    assert_eq!(glyf(&wide(0x1_0000, 70_000, 33)), Some(70_000));
    assert_eq!(cff(&wide(0x5000, 70_000, 7)), Some(70_000));
    assert_eq!(count(&[("maxp", maxp), ("MAXP", &wide(0x1_0000, 0, 33))]), Some(0));
    for unheld in [glyf(&wide(0x5000, 70_001, 7)), cff(&wide(0x5000, 70_001, 7))] {
        assert_eq!(unheld, Some(7));
    }
    assert_eq!(count(&[("maxp", maxp), ("MAXP", &wide(0x5000, 0xFF_FFFF, 7))]), Some(7), "no glyph data");
    for torn in [wide(0x1_0000, 70_000, 32), wide(0x2_0000, 70_000, 33), wide(0x5000, 70_000, 6)] {
        assert_eq!(count(&[("maxp", maxp), ("MAXP", &torn)]), Some(7));
    }
    assert_eq!(count(&[("MAXP", &wide(0x1_0000, 70_000, 32))]), None);
}

// GLYF and LOCA are read only together and holding bytes; else glyf and loca, whatever else is there.
#[test]
fn glyf_and_loca_give_way_only_to_both_uppercase_tables() {
    let tags = |tables: &[(&str, &[u8])]| glyf_tags(&map(tables));
    let (lower, upper) = (GlyfTags::of(|_| false), GlyfTags::of(|_| true));
    assert_eq!((lower.glyf, lower.loca, lower.extended), ("glyf", "loca", false));
    assert_eq!((upper.glyf, upper.loca, upper.extended), ("GLYF", "LOCA", true));
    assert_eq!(tags(&[("glyf", &[1]), ("loca", &[0]), ("GLYF", &[1]), ("LOCA", &[0])]), upper);
    assert_eq!(tags(&[("GLYF", &[1]), ("LOCA", &[0])]), upper);
    assert_eq!(tags(&[("glyf", &[1]), ("GLYF", &[1])]), lower);
    assert_eq!(tags(&[("GLYF", &[]), ("LOCA", &[0])]), lower);
}

fn be(words: &[u32], width: usize) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()[4 - width..].to_vec()).collect()
}

// No table is read past its declared length, a store's or loca's no more than any other's, in a font or a
// collection: the bytes after it belong to the next table or to none.
#[test]
fn every_table_is_read_at_its_declared_length() {
    let parts: [(&str, &[u8]); 5] =
        [("HVAR", &[1; 24]), ("OS/2", &[0; 8]), ("VVAR", &[1; 24]), ("loca", &[0; 4]), ("zzzz", &[2; 50])];
    let tables = map(&parts);
    let font = build_ttf(&tables).unwrap();
    let read = extract_tables_in_place(Source::Owned(font.clone()), 0).expect("its tables");
    let mut ttc = [&b"ttcf"[..], &be(&[1, 0], 2), &be(&[1, 16], 4), &font].concat();
    for at in (0..5).map(|i| 36 + 16 * i) {
        let moved = u32::from_be_bytes(ttc[at..at + 4].try_into().expect("4 bytes")) + 16;
        ttc[at..at + 4].copy_from_slice(&moved.to_be_bytes());
    }
    let collected = extract_ttc_tables(&ttc, 0).expect("its tables");
    for (tag, len) in [("HVAR", 24), ("OS/2", 8), ("VVAR", 24), ("loca", 4)] {
        assert_eq!([read[tag].len(), collected[tag].len()], [len; 2], "{tag}");
    }
}

// 4,000 tables each naming the whole 128 KiB file, 500 MiB in all, read as windows onto it: overlapping entries cost
// nothing, until a file is written from them, which copies each and is held to 4x the file. A collection's face
// whose tables overlap like that keeps the whole collection, held to 4x of it; two of them copy the bytes they cover.
#[test]
fn tables_naming_the_file_many_times_over_are_windows_onto_it() {
    let (count, len) = (4_000usize, 128 << 10);
    let mut font = alloc::vec![0u8; len];
    font[..6].copy_from_slice(&[0, 1, 0, 0, (count >> 8) as u8, count as u8]);
    for i in 0..count {
        let e = 12 + 16 * i;
        font[e..e + 4].copy_from_slice(alloc::format!("t{i:03x}").as_bytes());
        font[e + 12..e + 16].copy_from_slice(&(len as u32).to_be_bytes());
    }
    let buf = Shared::new(Source::Owned(font.clone()));
    for read in [by_directory(&buf, 0), in_order(&buf, 0).ok_or(Error::malformed("not in order"))] {
        let read = read.expect("its tables");
        assert_eq!(read.iter().count(), count);
        assert!(windows(&buf, &read).iter().all(|&(_, at, held)| at == 0 && held == len));
        assert_eq!(copies_held(&read).map_err(|e| e.kind()), Err(crate::daecore::ErrorKind::LimitExceeded));
    }
    let mut two = font.clone();
    two[4..6].copy_from_slice(&[0, 2]);
    assert!(copies_held(&by_directory(&Shared::new(Source::Owned(two)), 0).expect("its tables")).is_ok());

    let mut ttc = [&b"ttcf"[..], &be(&[1, 0], 2), &be(&[1, 16], 4), &font].concat();
    for i in 0..count {
        let at = 16 + 12 + 16 * i + 8;
        ttc[at..at + 4].copy_from_slice(&16u32.to_be_bytes());
    }
    let face = extract_ttc_tables(&ttc, 0).expect("the face");
    let first = face.iter().next().expect("a table").1.as_ptr();
    assert!(face.iter().all(|(_, t)| t.as_ptr() == first && t[..] == ttc[16..]));
    assert_eq!(face.iter().next().expect("a table").1.source_len(), ttc.len());
    assert!(copies_held(&face).is_err());
    ttc[20..22].copy_from_slice(&[0, 2]);
    let pair = extract_ttc_tables(&ttc, 0).expect("the face");
    assert!(pair.iter().all(|(_, t)| t.source_len() == len) && copies_held(&pair).is_ok());
}

// The directory read entry by entry in the file's order, the plain reading `directory` must agree with.
fn read_one_by_one(data: &[u8], dir_off: usize) -> Result<BTreeMap<String, (usize, usize)>, String> {
    if dir_off.checked_add(12).is_none_or(|end| data.len() < end) {
        return Err("TTF/OTF: file too short".into());
    }
    let sfversion = read_u32_be(data, dir_off).ok_or("TTF/OTF: header truncated")?;
    if sfversion != 0x0001_0000 && sfversion != 0x4F54_544F && sfversion != 0x7472_7565 {
        return Err(format!("TTF/OTF: not a font file (signature: 0x{:08X})", sfversion));
    }
    let num_tables = read_u16_be(data, dir_off + 4).ok_or("TTF/OTF: header truncated")? as usize;
    if (dir_off + 12).checked_add(num_tables * 16).is_none_or(|end| data.len() < end) {
        return Err("TTF/OTF: table directory truncated".into());
    }
    let (mut map, mut empty) = (BTreeMap::new(), alloc::collections::BTreeSet::new());
    for e in (0..num_tables).map(|i| dir_off + 12 + i * 16) {
        let tag_bytes = &data[e..e + 4];
        if !tag_bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
            continue;
        }
        let tag = String::from(core::str::from_utf8(tag_bytes).expect("printable"));
        let field = |at| read_u32_be(data, e + at).expect("a whole entry") as usize;
        let (offset, mut length) = (field(8), field(12));
        if offset > data.len() || empty.contains(&tag) || map.contains_key(&tag) {
            continue;
        }
        if length > data.len() - offset {
            if !matches!(tag.as_str(), "hmtx" | "HMTX" | "vmtx" | "VMTX") {
                continue;
            }
            length = data.len() - offset;
        }
        if length == 0 && tag != "glyf" {
            empty.insert(tag);
            continue;
        }
        map.insert(tag, (offset, length));
    }
    let has = |tag| map.contains_key(tag);
    if (has("glyf") || has("GLYF")) && (has("CFF ") || has("CFF2")) {
        let dropped: &[&str] = match sfversion {
            0x4F54_544F => &["glyf", "loca", "GLYF", "LOCA"],
            _ => &["CFF ", "CFF2"],
        };
        map.retain(|tag, _| !dropped.contains(&tag.as_str()));
    }
    Ok(map)
}

// Sorted once, a directory reads as reading it entry by entry does: the same tables and refusals, over repeated,
// clipped, empty, unprintable, overlapping and truncated entries.
#[test]
fn a_directory_reads_as_its_entries_one_by_one() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % n.max(1)
    };
    let tags: [&[u8; 4]; 14] = [
        b"glyf", b"loca", b"GLYF", b"LOCA", b"CFF ", b"CFF2", b"hmtx", b"HMTX", b"vmtx", b"VMTX", b"head", b"maxp",
        b"\x01\x02ab", b"OS/2",
    ];
    let (mut kept, mut refused, mut overlapping) = (0, 0, 0);
    for case in 0..20_000 {
        let (count, dir_off, wide) = (next(15), [0, 12][next(2)], case % 25 == 0);
        let len = dir_off + 12 + 16 * count + if wide { 20_000 } else { next(200) };
        let mut data = alloc::vec![0u8; len];
        let version = [0x0001_0000u32, 0x4F54_544F, 0x7472_7565, 0x1234_5678][next(4)];
        data[dir_off..dir_off + 4].copy_from_slice(&version.to_be_bytes());
        data[dir_off + 4..dir_off + 6].copy_from_slice(&(count as u16).to_be_bytes());
        for e in (0..count).map(|i| dir_off + 12 + 16 * i) {
            let (off, length) = match wide {
                true => (next(64), len - 64 - next(8)),
                false => (next(len + 8), [0, next(len + 8), next(16)][next(3)]),
            };
            data[e..e + 4].copy_from_slice(tags[next(tags.len())]);
            data[e + 8..e + 12].copy_from_slice(&(off as u32).to_be_bytes());
            data[e + 12..e + 16].copy_from_slice(&(length as u32).to_be_bytes());
        }
        let data = if case % 97 == 0 { &data[..next(len)] } else { &data[..] };
        let buf = Shared::new(Source::Owned(data.to_vec()));
        if let Some(read) = in_order(&buf, dir_off) {
            let plain = by_directory(&buf, dir_off).expect("a directory in order reads");
            assert_eq!(windows(&buf, &read), windows(&buf, &plain), "case {case}");
        }
        match (directory(data, dir_off), read_one_by_one(data, dir_off)) {
            (Ok(dir), Ok(map)) => {
                kept += 1;
                overlapping += usize::from(map.values().map(|&(_, len)| len).sum::<usize>() > 4 * data.len());
                assert!(dir.0.windows(2).all(|w| w[0].tag < w[1].tag), "case {case}: tag order");
                let tables: BTreeMap<String, (usize, usize)> =
                    dir.tables().map(|e| (String::from(tag_str(&e.tag)), (e.off, e.len))).collect();
                assert_eq!(tables, map, "case {case}");
                for tag in tags.map(|t| core::str::from_utf8(t).expect("UTF-8")) {
                    assert_eq!(dir.get(tag), map.get(tag).copied(), "case {case}: {tag}");
                }
            }
            (read, plain) => {
                refused += 1;
                assert_eq!(read.err().map(|e| String::from(e.message())), plain.err(), "case {case}");
            }
        }
    }
    let reached = kept > 10_000 && refused > 1_000 && overlapping > 100;
    assert!(reached, "{kept} kept, {refused} refused, {overlapping} overlap");
}

// Each table as a window of the font's buffer: its tag, where it starts and its length.
fn windows(buf: &Shared<Source>, tables: &Tables) -> Vec<(String, usize, usize)> {
    let at = |t: &TableBytes| t.as_slice().as_ptr() as usize - buf.as_ptr() as usize;
    tables.iter().map(|(tag, t)| (String::from(tag), at(t), t.len())).collect()
}

// A directory laid out as the specification says, from `pool`'s tags in order: entries inside a file of `len`, each
// holding bytes but glyf, which may hold none, and none over a quarter of the file.
fn laid_out(pool: &[&[u8; 4]], len: usize, mut next: impl FnMut(usize) -> usize) -> Vec<u8> {
    let tags: Vec<&[u8; 4]> = pool.iter().copied().filter(|_| next(3) > 0).collect();
    let mut data = alloc::vec![0u8; len];
    data[..4].copy_from_slice(&[0x0001_0000u32, 0x4F54_544F, 0x7472_7565][next(3)].to_be_bytes());
    data[4..6].copy_from_slice(&(tags.len() as u16).to_be_bytes());
    let body = 12 + 16 * tags.len();
    for (i, tag) in tags.iter().enumerate() {
        let off = body + next(len - body);
        let most = (len - off).min(len / 4);
        let held = if *tag == b"glyf" { next(most + 1) } else { 1 + next(most) };
        let e = 12 + 16 * i;
        data[e..e + 4].copy_from_slice(*tag);
        data[e + 8..e + 12].copy_from_slice(&(off as u32).to_be_bytes());
        data[e + 12..e + 16].copy_from_slice(&(held as u32).to_be_bytes());
    }
    data
}

// A directory in order is read in one pass to exactly the tables `directory` gives, each read to its length alone.
// Any one irregularity leaves it to `directory`.
#[test]
fn a_directory_in_order_reads_as_the_directory_does() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = |n: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % n.max(1)
    };
    let ttf: [&[u8; 4]; 12] =
        [b"COLR", b"GLYF", b"HVAR", b"LOCA", b"OS/2", b"VVAR", b"avar", b"glyf", b"head", b"hmtx", b"loca", b"maxp"];
    let cff: [&[u8; 4]; 7] = [b"CFF ", b"CFF2", b"HVAR", b"OS/2", b"avar", b"head", b"maxp"];
    let read = |data: &[u8]| {
        let buf = Shared::new(Source::Owned(data.to_vec()));
        in_order(&buf, 0).map(|t| (windows(&buf, &t), windows(&buf, &by_directory(&buf, 0).expect("reads"))))
    };
    for case in 0..5_000 {
        let pool: &[&[u8; 4]] = if case % 2 == 0 { &ttf } else { &cff };
        let data = laid_out(pool, 300 + next(if case % 4 == 0 { 30_000 } else { 3_000 }), &mut next);
        let (one_pass, plain) = read(&data).unwrap_or_else(|| panic!("case {case}: a directory in order"));
        assert_eq!(one_pass, plain, "case {case}");
        let count = usize::from(u16::from_be_bytes([data[4], data[5]]));
        if count < 2 {
            continue;
        }
        let (i, entry) = (1 + next(count - 1), |k: usize| 12 + 16 * k);
        let mut swapped = data.clone();
        let (a, b) = swapped.split_at_mut(entry(i));
        a[entry(i - 1)..entry(i)].swap_with_slice(&mut b[..16]);
        let mut repeated = data.clone();
        repeated.copy_within(entry(i - 1)..entry(i - 1) + 4, entry(i));
        let mut empty = data.clone();
        empty[entry(i) + 12..entry(i) + 16].copy_from_slice(&[0; 4]);
        let mut outside = data.clone();
        outside[entry(i) + 8..entry(i) + 12].copy_from_slice(&(data.len() as u32 + 1).to_be_bytes());
        let mut unprintable = data.clone();
        unprintable[entry(i)] = 0x7F;
        let mut over = data.clone();
        for e in 0..count {
            over[entry(e) + 8..entry(e) + 12].copy_from_slice(&[0; 4]);
            over[entry(e) + 12..entry(e) + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        }
        let irregular = [&swapped, &repeated, &outside, &unprintable];
        for (what, d) in ["swapped", "repeated", "outside", "unprintable"].iter().zip(irregular) {
            assert!(read(d).is_none(), "case {case}: {what}");
        }
        let is_glyf = &data[entry(i)..entry(i) + 4] == b"glyf";
        assert_eq!(read(&empty).is_some(), is_glyf, "case {case}: empty");
        assert!(read(&over).is_some(), "case {case}: every entry the whole file");
    }
    let mut both = laid_out(&[b"CFF ", b"glyf", b"head"], 600, |_| 1);
    let by_tables = by_directory(&Shared::new(Source::Owned(both.clone())), 0);
    assert!(by_tables.is_ok() && read(&both).is_none(), "both outline formats");
    both[12..16].copy_from_slice(b"CFX ");
    assert!(read(&both).is_some(), "one outline format");
}

// Every font of the corpus whose directory is in order reads in one pass to the tables `directory` gives.
#[test]
fn the_corpus_reads_in_one_pass_as_by_its_directory() {
    let mut fonts = alloc::vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts"))];
    let (mut in_order_count, mut all) = (0, 0);
    while let Some(path) = fonts.pop() {
        if path.is_dir() {
            fonts.extend(std::fs::read_dir(&path).expect("a folder").map(|e| e.expect("an entry").path()));
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        if ext != "ttf" && ext != "otf" {
            continue;
        }
        all += 1;
        let buf = Shared::new(Source::Owned(std::fs::read(&path).expect("a font")));
        if let Some(read) = in_order(&buf, 0) {
            in_order_count += 1;
            let plain = by_directory(&buf, 0).expect("a directory in order reads");
            assert_eq!(windows(&buf, &read), windows(&buf, &plain), "{}", path.display());
        }
    }
    assert!(in_order_count * 10 > all * 9, "{in_order_count} of {all} fonts in order");
}

// The directory comes out in tag order whatever order the tables go in, as the spec sorts it for a binary search, and
// of a tag given twice the last is kept, as a map collecting them keeps it.
#[test]
fn a_built_directory_is_sorted_and_keeps_a_tags_last() {
    let tables: [(&str, &[u8]); 4] = [("post", b"P"), ("OS/2", b"O"), ("cmap", b"one"), ("cmap", b"two")];
    let font = build_ttf(tables.iter().map(|(tag, bytes)| (*tag, bytes))).unwrap();
    let entry = |i: usize| {
        let at = 12 + 16 * i;
        let field = |k: usize| read_u32_be(&font, at + k).expect("a whole entry") as usize;
        let (offset, len) = (field(8), field(12));
        (&font[at..at + 4], &font[offset..offset + len])
    };
    assert_eq!(read_u16_be(&font, 4), Some(3));
    assert_eq!([entry(0), entry(1), entry(2)], [(&b"OS/2"[..], &b"O"[..]), (b"cmap", b"two"), (b"post", b"P")]);
}

// 65,536 empty tables, one more than numTables counts: refused as out of range, where 65,535 build a directory
// counting them all.
#[test]
fn a_font_of_more_tables_than_its_directory_counts_is_refused() {
    let tags: Vec<String> = (0..=u16::MAX).map(|i| alloc::format!("{i:04x}")).collect();
    let empty: &[u8] = &[];
    let build = |n: usize| build_ttf(tags[..n].iter().map(|tag| (tag.as_str(), &empty)));
    let refused = build(65_536).expect_err("65,536 tables");
    assert_eq!(refused.kind(), crate::daecore::ErrorKind::InvalidArgument);
    assert_eq!(read_u16_be(&build(65_535).expect("65,535 tables"), 4), Some(65_535));
}

// A directory's entries worked out from table lengths alone, as building past 4 GiB would take it: the last table
// may run past 4 GiB, but none may start 4 GiB in or be that long.
#[cfg(target_pointer_width = "64")]
#[test]
fn a_table_starting_4_gib_into_its_file_or_that_long_is_refused() {
    let gib = 1 << 30;
    let entries = |lengths: &[usize]| directory_entries(12, lengths.iter().copied());
    assert_eq!(entries(&[5, 8]), Some((alloc::vec![[12, 5], [20, 8]], 28)));
    let last = Some((alloc::vec![[12, u32::MAX - 15], [u32::MAX - 3, u32::MAX]], 8 * gib - 4));
    assert_eq!(entries(&[4 * gib - 16, 4 * gib - 1]), last);
    assert_eq!(entries(&[4 * gib]), None);
    assert_eq!(entries(&[4 * gib - 12, 0]), None);
}
