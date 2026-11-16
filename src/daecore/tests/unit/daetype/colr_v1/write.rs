use super::*;

// A key inserted twice, as after a search past the index's probes builds its table again, is listed once, with the
// table made first.
#[test]
fn seen_lists_each_key_once_with_its_first_table() {
    let mut seen = Seen::default();
    for (key, table) in [(5, 0), (3, 1), (5, 2)] {
        seen.insert(key, table);
    }
    assert_eq!(seen.get(5), Some(0));
    assert_eq!(seen.sorted().collect::<Vec<_>>(), [(3, 1), (5, 0)]);
}

// A subset keeps the box its font reads for each glyph: of records for 1 to 2, 65,535 back to 3 and 2 alone, a binary
// search finds the first for glyphs 1 and 2, and the second holds no glyph. A null box is none.
#[test]
fn a_subset_keeps_each_glyph_s_clip_box_as_its_font_reads_it() {
    let colr = |records: &[(u16, u16, u32)]| {
        let mut colr = alloc::vec![0u8; 34];
        colr[1] = 1;
        colr[22..26].copy_from_slice(&34u32.to_be_bytes());
        colr.push(1);
        colr.extend((records.len() as u32).to_be_bytes());
        for &(first, last, rel) in records {
            colr.extend([first.to_be_bytes(), last.to_be_bytes()].concat());
            colr.extend(&rel.to_be_bytes()[1..]);
        }
        colr.resize(colr.len() + 27, 1);
        colr
    };
    let boxes = read_clips(&colr(&[(1, 2, 26), (0xFFFF, 3, 35), (2, 2, 44)]), &Some);
    assert_eq!(boxes.into_iter().collect::<Vec<_>>(), [(1, 60), (2, 60)]);
    assert!(read_clips(&colr(&[(1, 1, 0)]), &Some).is_empty(), "a null box");
}

// A glyph's chain of 66 PaintTranslates down to a PaintSolid is written with its first 64, the 64th's child absent, as
// drawing reads no paint 64 levels below its root; the COLR is kept.
#[test]
fn a_paint_past_the_depth_limit_is_written_absent() {
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&34u32.to_be_bytes());
    colr.extend([1u32.to_be_bytes().as_slice(), &1u16.to_be_bytes(), &10u32.to_be_bytes()].concat());
    colr.extend([14, 0, 0, 8, 0, 0, 0, 0].repeat(66));
    colr.extend([2, 0, 0, 0x40, 0]);
    let built = rebuild(&colr, &Some, &|g| g, None, None, usize::MAX).expect("a COLR");
    let out = assemble(1, None, Some(built)).expect("written");
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    let mut at = list + read_u32_be(&out, list + 6).expect("a root") as usize;
    let mut translates = 1;
    while let Some(rel) = read_offset24(&out, at + 1).filter(|&rel| rel != 0) {
        at += rel;
        translates += 1;
    }
    assert_eq!((translates, out[at]), (64, 14));
}

// A LayerList stating 1,000 layers with 10 in the table: drawing reads the 10 a glyph's PaintColrLayers names, and
// the glyph's instance or subset draws them alike.
#[test]
fn a_layer_list_counting_past_the_table_keeps_the_layers_it_holds() {
    use super::super::{colr_v1_paint_graph_with_scalars as graph, testing::{shared_layers, tables}};
    let mut colr = shared_layers(1, 10, false);
    let list = read_u32_be(&colr, 18).expect("a LayerList") as usize;
    colr[list..list + 4].copy_from_slice(&1_000u32.to_be_bytes());
    let out = assemble(1, None, rebuild(&colr, &Some, &|g| g, None, None, usize::MAX)).expect("written");
    let drawn = |colr: &[u8]| graph(&tables(colr, [0, 0, 0, 255]), 1, 0, &None, &[]);
    assert_eq!(drawn(&colr).map(|g| g.root().children().count()), Some(10));
    assert_eq!(drawn(&out), drawn(&colr));
}

// Records for glyphs 5, 3, 7, 9 and 11, out of order: the search finds 5, 7, 9 and 11, but 9's paint passes the table
// and 11's offset of 0 is the null paint. A subset of each glyph alone draws it as the font does.
#[test]
fn a_subset_keeps_a_glyph_s_record_where_drawing_finds_it() {
    use super::super::{colr_v1_paint_graph_with_scalars as graph, testing::tables};
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&34u32.to_be_bytes());
    let records = [(5u16, 34u32), (3, 34), (7, 34), (9, 1_000), (11, 0)];
    colr.extend((records.len() as u32).to_be_bytes());
    for (gid, rel) in records {
        colr.extend([&gid.to_be_bytes()[..], &rel.to_be_bytes()].concat());
    }
    colr.extend([2, 0xFF, 0xFF, 0x40, 0]);
    let drawn = |colr: &[u8], gid: u16| graph(&tables(colr, [0, 0, 0, 255]), gid.into(), 0, &None, &[]);
    assert_eq!(records.map(|(gid, _)| drawn(&colr, gid).is_some()), [true, false, true, false, true]);
    for (kept, _) in records {
        let built = rebuild(&colr, &|g| (g == kept).then_some(g), &|g| g, None, None, usize::MAX);
        let out = built.and_then(|b| assemble(1, None, Some(b)));
        assert_eq!(out.and_then(|out| drawn(&out, kept)), drawn(&colr, kept), "glyph {kept}");
    }
}
