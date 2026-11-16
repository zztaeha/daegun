use super::*;

// COLR v0 whose base glyphs 1 to `bases` all name the same 40,000 layers of glyph `bases + 1`.
fn shared(bases: u16) -> (Vec<u8>, GlyphSet, Vec<u32>) {
    const LAYERS: u16 = 40_000;
    let records = 14 + 6 * usize::from(bases);
    let mut colr = [0u16, bases].map(u16::to_be_bytes).concat();
    colr.extend(14u32.to_be_bytes());
    colr.extend((records as u32).to_be_bytes());
    colr.extend(LAYERS.to_be_bytes());
    for gid in 1..=bases {
        colr.extend([gid, 0, LAYERS].map(u16::to_be_bytes).concat());
    }
    for _ in 0..LAYERS {
        colr.extend([bases + 1, 0].map(u16::to_be_bytes).concat());
    }
    let mut active = GlyphSet::new();
    (0..=bases + 1).for_each(|g| { active.insert(g.into()); });
    (colr, active, (0..=bases + 1).map(u32::from).collect())
}

// Version 1 holding glyph 2's layer of glyph 3 and glyph 5's PaintSolid.
fn painted() -> Vec<u8> {
    let header = [[1u16, 1].map(u16::to_be_bytes).concat(), [34u32, 40].map(u32::to_be_bytes).concat()].concat();
    let rest = [1u16.to_be_bytes().to_vec(), [44u32, 0, 0, 0, 0].map(u32::to_be_bytes).concat()].concat();
    let v0 = [2u16, 0, 1, 3, 0].map(u16::to_be_bytes).concat();
    let list = [1u32.to_be_bytes().to_vec(), 5u16.to_be_bytes().to_vec(), 10u32.to_be_bytes().to_vec()].concat();
    [header, rest, v0, list, vec![2, 0, 0, 0x40, 0]].concat()
}

// The subset of `colr` keeping `active` under `gid_map`, its closure over `active` as the subset's.
fn subset(colr: &[u8], active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    subset_colr(colr, &colr_closure(colr, active, u32::MAX).1, active, gid_map)
}

// The glyphs `wanted` close over in `colr`, .notdef among them, each its rank among them, as a subset keeps them.
fn kept(colr: &[u8], wanted: &[u32]) -> (GlyphSet, Vec<u32>, ColrKept) {
    let asked: GlyphSet = core::iter::once(0).chain(wanted.iter().copied()).collect();
    let (found, colr_kept) = colr_closure(colr, &asked, u32::MAX);
    let mut active = asked;
    active.union(&found);
    let mut gid_map = vec![0; active.iter().last().map_or(0, |g| g as usize + 1)];
    active.iter().enumerate().for_each(|(new, old)| gid_map[old as usize] = new as u32);
    (active, gid_map, colr_kept)
}

// Version 0 once no kept glyph has a paint, its layers after the header and its base records last.
#[test]
fn a_subset_keeping_no_paint_is_version_0() {
    let colr = painted();
    for (glyphs, version, layers) in [(&[0u16, 2, 3][..], 0, 14), (&[0, 2, 3, 5], 1, 49)] {
        let active: GlyphSet = glyphs.iter().copied().map(u32::from).collect();
        let mut gid_map = vec![0u32; 6];
        glyphs.iter().enumerate().for_each(|(new, &old)| gid_map[usize::from(old)] = new as u32);
        let out = subset(&colr, &active, &gid_map).expect("a subset COLR");
        assert_eq!(read_u16_be(&out, 0), Some(version), "{glyphs:?}");
        assert_eq!([read_u32_be(&out, 8), read_u32_be(&out, 4)], [Some(layers), Some(layers + 4)], "{glyphs:?}");
        assert_eq!(read_u16_be(&out, 2), Some(1), "glyph 2 keeps its layer");
    }
}

// Base glyph 1 painting `paint` then a PaintSolid. Format 33 writes an id past 65,535 whole; a caller's map
// renumbering format 10's glyph past 16 bits leaves glyph 1 an empty PaintColrLayers, as an unreadable paint does.
#[test]
fn a_paint_glyph_is_written_in_its_width() {
    let colr = |paint: &[u8]| {
        let header = [&[0, 1, 0, 0][..], &[0; 10], &34u32.to_be_bytes(), &[0; 16]].concat();
        [&header[..], &[0, 0, 0, 1, 0, 1, 0, 0, 0, 10], paint, &[2, 0, 0, 0x40, 0]].concat()
    };
    let (narrow, wide) = (colr(&[10, 0, 0, 6, 0, 4]), colr(&[33, 0, 0, 7, 1, 0, 4]));
    let active: GlyphSet = [0u32, 1, 4, 0x1_0004].into_iter().collect();
    let mut gid_map = vec![0; 0x1_0005];
    (gid_map[1], gid_map[4], gid_map[0x1_0004]) = (1, 2, 0x1_0002);
    let has = |t: Option<Vec<u8>>, paint: &[u8]| t.is_some_and(|t| t.windows(paint.len()).any(|w| w == paint));
    assert!(has(subset(&narrow, &active, &gid_map), &[10, 0, 0, 6, 0, 2]));
    assert!(has(subset(&wide, &active, &gid_map), &[33, 0, 0, 7, 1, 0, 2]));
    gid_map[4] = 0x1_0000;
    assert!(has(subset(&narrow, &active, &gid_map), &[0, 1, 0, 0, 0, 10, 1, 0, 0, 0, 0, 0]));
}

// Base glyphs sharing a run of layers each take their own copy. A layer index is 16 bits, so one base's 40,000
// layers are written and two bases' 80,000 can't be: version 0 holds no COLR then.
#[test]
fn each_base_glyph_takes_its_own_layers_within_16_bits() {
    let (colr, active, gid_map) = shared(1);
    let out = subset(&colr, &active, &gid_map).expect("a subset COLR");
    assert_eq!(read_u16_be(&out, 12), Some(40_000), "the layer count");
    let (colr, active, gid_map) = shared(2);
    assert_eq!(subset(&colr, &active, &gid_map), None);
}

// 2,000 glyphs sharing 60 layers walk 242,000 paints between them, past one budget of 100,000 for
// the whole subset: each paint is read once, and written once.
#[test]
fn a_shared_graph_subsets_whole() {
    let colr = super::super::super::colr_v1::testing::shared_layers(2000, 60, false);
    let mut active = GlyphSet::new();
    (0..=2000u32).for_each(|g| { active.insert(g); });
    let (found, _) = colr_closure(&colr, &active, u32::MAX);
    assert!((1000..1060u32).all(|g| found.contains(&g)), "the closure lost layer glyphs");
    (1000..1060u32).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..=2000).collect();
    let out = subset(&colr, &active, &gid_map).expect("a subset COLR");
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    assert_eq!(read_u32_be(&out, list), Some(2000));
    assert!(out.len() < colr.len() + 2000 * 6, "the shared layers were written more than once");
}

// Glyph 5 names two layers when the table holds one, so drawing reads none for it and the subset leaves its record
// out; glyph 4 keeps its layer, renumbered 3 beside 5 and 2 without it.
#[test]
fn a_record_whose_run_passes_the_layers_is_left_out() {
    let mut colr = [0u16, 2].map(u16::to_be_bytes).concat();
    colr.extend([14u32, 26].map(u32::to_be_bytes).concat());
    colr.extend(1u16.to_be_bytes());
    colr.extend([4u16, 0, 1, 5, 0, 2].map(u16::to_be_bytes).concat());
    colr.extend([7u16, 0, 8, 0].map(u16::to_be_bytes).concat());
    let (active, gid_map, colr_kept) = kept(&colr, &[4, 5]);
    let out = subset_colr(&colr, &colr_kept, &active, &gid_map).expect("a subset COLR");
    assert_eq!(out, [&[0, 0, 0, 1, 0, 0, 0, 18, 0, 0, 0, 14, 0, 1, 0, 3, 0, 0][..], &[0, 1, 0, 0, 0, 1]].concat());
    let (active, gid_map, colr_kept) = kept(&colr, &[4]);
    let out = subset_colr(&colr, &colr_kept, &active, &gid_map).expect("a subset COLR");
    assert_eq!(out, [&[0, 0, 0, 1, 0, 0, 0, 18, 0, 0, 0, 14, 0, 1, 0, 2, 0, 0][..], &[0, 1, 0, 0, 0, 1]].concat());
}

// Version 1 with `bases`' root paints, then `layers`' paints, each list's paints after its offsets.
fn version_1(bases: &[(u16, Vec<u8>)], layers: &[Vec<u8>]) -> Vec<u8> {
    let u32s = |v: &[u32]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let (mut list, mut at) = (u32s(&[bases.len() as u32]), 4 + 6 * bases.len());
    for (gid, paint) in bases {
        list.extend([&gid.to_be_bytes()[..], &u32s(&[at as u32])].concat());
        at += paint.len();
    }
    list.extend(bases.iter().flat_map(|b| b.1.clone()));
    let (mut layer_list, mut at) = (u32s(&[layers.len() as u32]), 4 + 4 * layers.len());
    for paint in layers {
        layer_list.extend(u32s(&[at as u32]));
        at += paint.len();
    }
    layer_list.extend(layers.concat());
    let header = [vec![0, 1, 0, 0], u32s(&[0, 0]), vec![0, 0], u32s(&[34, 34 + list.len() as u32, 0, 0, 0])];
    [header.concat(), list, layer_list].concat()
}

fn closed(colr: &[u8], glyphs: &[u32]) -> Vec<u32> {
    colr_closure(colr, &glyphs.iter().copied().collect(), u32::MAX).0.iter().collect()
}

// A PaintColrGlyph names its glyph where it has a base record, and walks into that record's paint; a chain of
// layers reaches a glyph 63 paints down, not 64.
#[test]
fn a_paint_walk_follows_colr_glyphs_to_a_bounded_depth() {
    let solid = [2u8, 0, 0, 0x40, 0];
    let glyph = |gid: u16| [&[10u8, 0, 0, 6][..], &gid.to_be_bytes(), &solid].concat();
    let colr_glyph = |gid: u16| [&[11u8][..], &gid.to_be_bytes()].concat();
    let layers = |first: u32| [&[1u8, 1][..], &first.to_be_bytes()].concat();
    let t = version_1(&[(1, colr_glyph(9)), (2, colr_glyph(10)), (9, glyph(12))], &[]);
    assert_eq!(closed(&t, &[0, 1, 2]), [9, 12]);
    for (k, want) in [(62, &[13][..]), (63, &[])] {
        let chain: Vec<Vec<u8>> = (1..=k).map(layers).chain([glyph(13)]).collect();
        assert_eq!(closed(&version_1(&[(3, layers(0))], &chain), &[0, 3]), want, "{k} layers");
    }
}

// A PaintColrGlyph 63 paints down names glyph 9, whose own paint, a PaintSolid of palette 7, is walked from its root
// as the subset writes it there: its palette is kept and so is version 1.
#[test]
fn a_colr_glyph_s_paint_is_walked_from_its_own_root() {
    let layers = |first: u32| [&[1u8, 1][..], &first.to_be_bytes()].concat();
    let chain: Vec<Vec<u8>> = (1..=62).map(layers).chain([[&[11u8][..], &9u16.to_be_bytes()].concat()]).collect();
    let colr = version_1(&[(1, layers(0)), (9, solid(7))], &chain);
    let (active, gid_map, colr_kept) = kept(&colr, &[1]);
    assert!(active.contains(&9u32));
    let out = subset_colr(&colr, &colr_kept, &active, &gid_map).expect("a subset COLR");
    assert_eq!(read_u16_be(&out, 0), Some(1));
}

// A BaseGlyphList whose count passes the table is absent, as drawing reads it: its records name no glyph.
#[test]
fn a_base_glyph_list_past_the_table_names_nothing() {
    let colr_glyph = |gid: u16| [&[11u8][..], &gid.to_be_bytes()].concat();
    let mut t = version_1(&[(1, colr_glyph(9)), (9, solid(0))], &[]);
    assert_eq!(closed(&t, &[0, 1]), [9]);
    t[34..38].copy_from_slice(&1_000u32.to_be_bytes());
    assert_eq!(closed(&t, &[0, 1]), [0u32; 0]);
}

// A run past the layer records has no layers, as drawing reads it, and a record the base record search misses
// among misordered ones names none: glyph 5 reaches nothing, and of glyphs 6 then 2, only 6 reaches its layer.
#[test]
fn version_0_layers_are_found_by_the_record_search() {
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    assert_eq!(closed(&u16s(&[0, 1, 0, 14, 0, 20, 1, 5, 0, 2, 7, 0, 8, 0]), &[5]), []);
    let t = u16s(&[0, 2, 0, 14, 0, 26, 2, 6, 0, 1, 2, 1, 1, 9, 0, 10, 0]);
    assert_eq!([closed(&t, &[2, 6]), closed(&t, &[2])], [vec![9], vec![]]);
}

#[test]
fn the_v0_closure_reads_each_layer_once() {
    let (colr, active, _) = shared(200);
    let (found, _) = colr_closure(&colr, &active, u32::MAX);
    assert!(found.len() <= 40_000, "200 glyphs sharing 40,000 layers found {}", found.len());
}

// Base glyphs 1 to 4,000 each naming the same 4,000 layers, of glyphs 1 to 4,000 in palette entries 1 to 4,000:
// palettes read each layer once, where reading each kept glyph's run takes 16 million reads.
#[test]
fn palettes_read_each_shared_layer_once() {
    const L: u16 = 4_000;
    let mut colr = [0u16, L].map(u16::to_be_bytes).concat();
    colr.extend([14, 14 + 6 * u32::from(L)].map(u32::to_be_bytes).concat());
    colr.extend(L.to_be_bytes());
    (1..=L).for_each(|g| colr.extend([g, 0, L].map(u16::to_be_bytes).concat()));
    (1..=L).for_each(|g| colr.extend([g, g].map(u16::to_be_bytes).concat()));
    let started = std::time::Instant::now();
    let (found, kept) = colr_closure(&colr, &[1].into_iter().collect(), u32::MAX);
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    assert_eq!((found.len(), kept.palettes.len()), (L.into(), L.into()));
}

fn solid(palette: u16) -> Vec<u8> {
    [&[2][..], &palette.to_be_bytes(), &[0x40, 0]].concat()
}

fn glyph(gid: u16, child: Vec<u8>) -> Vec<u8> {
    [&[10, 0, 0, 6][..], &gid.to_be_bytes(), &child].concat()
}

// A linear gradient whose color line follows it, each stop (offset, palette index) at full alpha.
fn linear(stops: &[(u16, u16)]) -> Vec<u8> {
    let line = stops.iter().flat_map(|&(at, palette)| [at, palette, 0x4000].map(u16::to_be_bytes).concat());
    [&[4, 0, 0, 16][..], &[0; 12], &[0], &(stops.len() as u16).to_be_bytes(), &line.collect::<Vec<u8>>()].concat()
}

fn colr_layers(n: u8, first: u32) -> Vec<u8> {
    [&[1, n][..], &first.to_be_bytes()].concat()
}

// Version 1 (0 with no paint) holding version 0's records and layers, the base glyphs' paints, a LayerList and
// clips, in that order, each list's paints after it.
fn colr(
    v0: &[[u16; 3]], layers: &[[u16; 2]], bases: &[(u16, Vec<u8>)], list: &[Vec<u8>], clips: &[(u16, u16, i16)],
) -> Vec<u8> {
    let painted = !bases.is_empty() || !list.is_empty();
    let mut t = vec![0u8; if painted { 34 } else { 14 }];
    t[1] = u8::from(painted);
    let put = |t: &mut Vec<u8>, field: usize, bytes: &[u8]| {
        let at = (t.len() as u32).to_be_bytes();
        t[field..field + 4].copy_from_slice(&at);
        t.extend_from_slice(bytes);
    };
    if !v0.is_empty() {
        t[2..4].copy_from_slice(&(v0.len() as u16).to_be_bytes());
        put(&mut t, 4, &v0.iter().flat_map(|r| r.map(u16::to_be_bytes).concat()).collect::<Vec<u8>>());
        put(&mut t, 8, &layers.iter().flat_map(|r| r.map(u16::to_be_bytes).concat()).collect::<Vec<u8>>());
        t[12..14].copy_from_slice(&(layers.len() as u16).to_be_bytes());
    }
    if !bases.is_empty() {
        let mut at = 4 + 6 * bases.len();
        let mut records = (bases.len() as u32).to_be_bytes().to_vec();
        for (gid, paint) in bases {
            records.extend([&gid.to_be_bytes()[..], &(at as u32).to_be_bytes()].concat());
            at += paint.len();
        }
        put(&mut t, 14, &[records, bases.iter().flat_map(|b| b.1.clone()).collect()].concat());
    }
    if !list.is_empty() {
        let mut at = 4 + 4 * list.len();
        let mut offsets = (list.len() as u32).to_be_bytes().to_vec();
        for paint in list {
            offsets.extend((at as u32).to_be_bytes());
            at += paint.len();
        }
        put(&mut t, 18, &[offsets, list.concat()].concat());
    }
    if !clips.is_empty() {
        let mut bytes = [&[1][..], &(clips.len() as u32).to_be_bytes()].concat();
        for (i, &(first, last, _)) in clips.iter().enumerate() {
            let at = (5 + 7 * clips.len() + 9 * i) as u32;
            bytes.extend([&first.to_be_bytes()[..], &last.to_be_bytes(), &at.to_be_bytes()[1..]].concat());
        }
        for &(.., x_max) in clips {
            bytes.extend([&[1][..], &[0, 0, 0, 0], &x_max.to_be_bytes(), &x_max.to_be_bytes()].concat());
        }
        put(&mut t, 22, &bytes);
    }
    t
}

// CPAL 1 of 8 colors in 3 palettes, palettes 0 and 2 alike in colors 3, 5 and 7, with types, palette labels 300,
// 301 and none, and entry labels 310 to 317.
fn cpal() -> Vec<u8> {
    cpal_labelled([300, 301, 0xFFFF])
}

fn cpal_labelled(palette_labels: [u16; 3]) -> Vec<u8> {
    let mut colors: Vec<[u8; 4]> = (0..24u8).map(|i| [i * 4, i * 4 + 1, i * 4 + 2, i * 4 + 3]).collect();
    for c in [3, 5, 7] {
        colors[16 + c] = colors[c];
    }
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let head = [u16s(&[1, 8, 3, 24, 0, 64]), u16s(&[0, 8, 16]), [30u32, 42, 48].map(u32::to_be_bytes).concat()];
    let labels = u16s(&[310, 311, 312, 313, 314, 315, 316, 317]);
    let arrays = [[1u32, 2, 0].map(u32::to_be_bytes).concat(), u16s(&palette_labels), labels];
    [head.concat(), arrays.concat(), colors.concat()].concat()
}

fn tables(colr: &[u8], wanted: &[u32]) -> (Option<Vec<u8>>, Option<Vec<u8>>, ColrKept) {
    let (active, gid_map, kept) = kept(colr, wanted);
    (subset_colr(colr, &kept, &active, &gid_map), subset_cpal(&cpal(), &kept), kept)
}

// Glyph 10 paints palette index 5, 11 a gradient of 3 and the text color, and 12's version 0 layers 7 and 5: each its
// rank among those kept, CPAL keeps those of every palette, and palettes 0 and 2 share.
#[test]
fn palette_indices_are_written_by_their_rank_among_those_kept() {
    let t = colr(&[[12, 0, 2]], &[[16, 7], [17, 5]], &[
        (10, glyph(14, solid(5))),
        (11, glyph(15, linear(&[(0, 3), (0x4000, 0xFFFF)]))),
    ], &[], &[]);
    let (out, cpal, kept) = tables(&t, &[10, 12]);
    assert_eq!(kept.palettes.into_iter().collect::<Vec<_>>(), [(5, 0), (7, 1)]);
    let (out, cpal) = (out.expect("a subset COLR"), cpal.expect("a subset CPAL"));
    assert_eq!(out.len(), 69);
    assert!(out.windows(5).any(|w| w == solid(0)), "palette 5 as 0");
    let layers = read_u32_be(&out, 8).expect("layers") as usize;
    assert_eq!(out[layers..layers + 8], [0, 4, 0, 1, 0, 5, 0, 0], "glyphs 16 and 17 renumbered, palettes 7 and 5");
    assert_eq!(cpal.len(), 68);
    assert_eq!([2, 4, 6, 12, 14, 16].map(|at| read_u16_be(&cpal, at)), [2, 3, 4, 0, 2, 0].map(Some));
    let (out, cpal, _) = tables(&t, &[11]);
    let out = out.expect("a subset COLR");
    assert_eq!((out.len(), cpal.map(|c| c.len())), (81, Some(58)));
    let stops = [0, 0, 0, 0, 0x40, 0, 0x40, 0, 0xFF, 0xFF];
    assert!(out.windows(10).any(|w| w == stops), "stops 3 and 0xFFFF as 0 and 0xFFFF");
}

// Glyph 10's PaintGlyph names 13, which has its own record: the subset keeps that record, so the closure walks its
// paint too, and its palette index 6 is kept beside 10's 2.
#[test]
fn a_glyph_a_paint_names_keeps_its_own_paint() {
    let t = colr(&[], &[], &[(10, glyph(13, solid(2))), (13, glyph(14, solid(6)))], &[], &[]);
    let (out, _, kept) = tables(&t, &[10]);
    let out = out.expect("a subset COLR");
    assert_eq!(read_u32_be(&out, read_u32_be(&out, 14).expect("a BaseGlyphList") as usize), Some(2));
    assert_eq!(kept.palettes.keys().copied().collect::<Vec<u16>>(), [2, 6]);
    assert_eq!(tables(&t, &[13]).0.map(|t| t.len()), Some(55));
}

// Clips go with every glyph COLR's stage keeps, glyph 12's with no paint too, each glyph with the box the reader's
// search finds: records for 10 alone and 10 to 12 overlap, so 10 takes box 10 and 11 and 12 box 20, two records.
#[test]
fn clips_follow_the_glyphs_colr_keeps() {
    let t = colr(&[[12, 0, 1]], &[[16, 1]], &[(10, glyph(14, solid(1))), (11, glyph(15, solid(1)))], &[],
        &[(10, 10, 10), (10, 12, 20)]);
    let cases = [(vec![10, 11, 12], 114, vec![(1, 1, 10), (2, 3, 20)]), (vec![10], 76, vec![(1, 1, 10)])];
    for (wanted, len, boxes) in cases {
        let out = tables(&t, &wanted).0.expect("a subset COLR");
        let clips = read_u32_be(&out, 22).expect("a ClipList") as usize;
        assert_eq!(out.len(), len, "{wanted:?}");
        assert!(clips < read_u32_be(&out, 14).expect("a BaseGlyphList") as usize, "the ClipList comes first");
        assert_eq!(read_u32_be(&out, clips + 1), Some(boxes.len() as u32), "{wanted:?}");
        for (i, &(first, last, x)) in boxes.iter().enumerate() {
            let record = clips + 5 + 7 * i;
            let glyphs = [read_u16_be(&out, record), read_u16_be(&out, record + 2)];
            assert_eq!(glyphs, [Some(first), Some(last)], "{wanted:?}");
            let x_max = clips + read_u32_be(&out, record + 3).expect("a box") as usize % 0x100_0000 + 5;
            assert_eq!(read_u16_be(&out, x_max), Some(x), "{wanted:?}");
        }
    }
}

// A PaintColrLayers keeps its count, past the list too, its first index its rank among those named, or 0 for none.
#[test]
fn a_colr_layers_paint_keeps_its_count_and_takes_its_first_by_rank() {
    let list = [glyph(14, solid(1)), glyph(15, solid(2))];
    let t = colr(&[], &[], &[(10, colr_layers(0, 1)), (11, colr_layers(3, 0))], &list, &[]);
    let out = tables(&t, &[10, 11]).0.expect("a subset COLR");
    assert_eq!(out.len(), 96);
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    let paint = |i: usize| list + read_u32_be(&out, list + 4 + 6 * i + 2).expect("a paint") as usize;
    assert_eq!([&out[paint(0)..paint(0) + 6], &out[paint(1)..paint(1) + 6]], [colr_layers(0, 0), colr_layers(3, 0)]);
    assert_eq!(read_u32_be(&out, read_u32_be(&out, 18).expect("a LayerList") as usize), Some(2));
}

// The text color alone keeps no CPAL.
#[test]
fn the_text_color_alone_keeps_no_cpal() {
    let (out, cpal, _) = tables(&colr(&[], &[], &[(10, glyph(14, solid(0xFFFF)))], &[], &[]), &[10]);
    assert!(out.is_some_and(|t| t.windows(5).any(|w| w == solid(0xFFFF))));
    assert_eq!(cpal, None);
}

// Base glyph records are kept as drawing finds them: of 11, 10 and 11, the search finds 10 and the second 11, so the
// first, which nothing draws, is left out, and its paint's glyph and palette index with it.
#[test]
fn base_glyph_records_are_kept_as_drawing_finds_them() {
    let bases = [(11, glyph(15, solid(1))), (10, glyph(14, solid(2))), (11, glyph(16, solid(3)))];
    let (out, _, kept) = tables(&colr(&[], &[], &bases, &[], &[]), &[10, 11]);
    let out = out.expect("a subset COLR");
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    assert_eq!(read_u32_be(&out, list), Some(2));
    assert_eq!([0, 1].map(|i| read_u16_be(&out, list + 4 + 6 * i)), [1, 2].map(Some));
    assert!(!kept.glyphs.contains(&15u32) && kept.glyphs.contains(&16u32));
    assert_eq!(kept.palettes.into_keys().collect::<Vec<_>>(), [2, 3]);
}

// Paints alike in their bytes and what they point to are one, wherever the source holds them.
#[test]
fn paints_alike_are_written_once() {
    let out = tables(&colr(&[], &[], &[(1, glyph(5, solid(0))), (2, glyph(5, solid(0)))], &[], &[]), &[1, 2]).0;
    let out = out.expect("a subset COLR");
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    assert_eq!(read_u32_be(&out, list + 6), read_u32_be(&out, list + 12));
}

// Nothing kept that COLR names: a version 1 source's subset is version 0's bare header.
#[test]
fn a_subset_keeping_no_color_glyph_is_a_bare_header() {
    let active: GlyphSet = [0u32, 3].into_iter().collect();
    assert_eq!(subset(&painted(), &active, &[0, 0, 0, 1]), Some(vec![0; 14]));
}

// A version 1 CPAL keeps its palette labels whole and the kept colors' labels, whether or not the table is written;
// version 0 names none.
#[test]
fn cpal_names_are_the_palette_labels_and_the_kept_colors_labels() {
    let (_, _, kept) = tables(&colr(&[], &[], &[(10, glyph(14, linear(&[(0, 3), (0x4000, 5)])))], &[], &[]), &[10]);
    assert_eq!(cpal_names(&cpal(), &kept).into_iter().collect::<Vec<_>>(), [300, 301, 313, 315, 0xFFFF]);
    let mut v0 = cpal();
    v0[1] = 0;
    assert!(cpal_names(&v0, &kept).is_empty());
}

// A PaintGlyph with no paint is kept, its offset written null, as the reader passes over that child alone: as a layer,
// as one side of a PaintComposite, or as a base glyph's root, beside the paints that draw.
#[test]
fn a_null_child_is_written_null() {
    let null = |gid: u16| [&[10, 0, 0, 0][..], &gid.to_be_bytes()].concat();
    let layers = colr(&[], &[], &[(10, colr_layers(2, 0))], &[null(14), glyph(15, solid(2))], &[]);
    let out = tables(&layers, &[10]).0.expect("a subset COLR");
    let list = read_u32_be(&out, 18).expect("a LayerList") as usize;
    assert_eq!((out.len(), read_u32_be(&out, list + 4)), (79, Some(23)));
    let composite = [&[32, 0, 0, 8, 3, 0, 0, 0][..], &glyph(14, solid(1))].concat();
    let out = tables(&colr(&[], &[], &[(10, composite), (11, glyph(15, solid(2)))], &[], &[]), &[10, 11]).0;
    assert_eq!(out.map(|t| t.len()), Some(80));
    let root = colr(&[], &[], &[(10, null(14)), (11, glyph(15, solid(2)))], &[], &[]);
    assert_eq!(tables(&root, &[10, 11]).0.map(|t| t.len()), Some(67));
    assert!(tables(&root, &[11]).0.is_some());
}

// Glyph 1's layer and glyph 2's two come out byte for byte as the two base records (palette 5 ranked 2 past glyph
// 3's 1), so the serializer writes them once, both offsets at it.
#[test]
fn layers_alike_to_the_base_records_share_their_bytes() {
    let t = colr(&[[1, 0, 1], [2, 1, 2]], &[[1, 0], [1, 5], [1, 5]], &[(3, solid(1))], &[], &[]);
    let out = tables(&t, &[1, 2, 3]).0.expect("a subset COLR");
    assert_eq!(read_u32_be(&out, 4), read_u32_be(&out, 8));
    let at = read_u32_be(&out, 4).expect("records") as usize;
    assert_eq!((out.len(), &out[at..]), (at + 12, &[0, 1, 0, 0, 0, 1, 0, 2, 0, 1, 0, 2][..]));
}

fn kept_palettes(indices: &[u16]) -> ColrKept {
    let palettes = indices.iter().enumerate().map(|(rank, &i)| (i, rank as u16)).collect();
    ColrKept { palettes, ..ColrKept::default() }
}

// A var gradient's stops are 10 bytes apart, and a v0 record of a glyph a paint reached gives its layers' indices.
#[test]
fn the_closure_reads_every_palette_index_the_subset_writes() {
    let stop = |at: u8, palette: u8| [at, 0, 0, palette, 0x40, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    let line = [&[0, 0, 2][..], &stop(0, 3), &stop(0x40, 4)];
    let var_linear = [&[5, 0, 0, 20][..], &[0; 12], &[0xFF; 4], &line.concat()].concat();
    let palettes = |t: &[u8], wanted: &[u32]| kept(t, wanted).2.palettes.into_keys().collect::<Vec<_>>();
    assert_eq!(palettes(&colr(&[], &[], &[(10, var_linear)], &[], &[]), &[10]), [3, 4]);
    let t = colr(&[[12, 0, 1]], &[[12, 7]], &[(10, glyph(12, solid(1)))], &[], &[]);
    assert_eq!(palettes(&t, &[10]), [1, 7]);
    let out = tables(&t, &[10]).0.expect("a subset COLR");
    let layers = read_u32_be(&out, 8).expect("layers") as usize;
    assert_eq!(out[layers..layers + 4], [0, 2, 0, 1]);
}

// A version 0 source: a kept record of no layers at the layers' end is kept, as is one of none anywhere, each writing
// no layers or offset; keeping no record writes no COLR; a colred glyph the subset drops is passed over.
#[test]
fn version_0_records_are_checked() {
    let at_end = colr(&[[4, 1, 0]], &[[7, 0]], &[], &[], &[]);
    let out = tables(&at_end, &[4]).0.expect("a subset COLR");
    assert_eq!(out, [&[0, 0, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0][..], &[0, 1, 0, 0, 0, 0]].concat());
    let none = colr(&[[1, 0, 0]], &[[1, 0]], &[], &[], &[]);
    let out = tables(&none, &[1]).0.expect("a subset COLR");
    assert_eq!(out, [&[0, 0, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0][..], &[0, 1, 0, 0, 0, 0]].concat());
    let two = colr(&[[4, 0, 1], [5, 0, 1]], &[[7, 0]], &[], &[], &[]);
    assert_eq!(tables(&two, &[6]).0, None);
    let (active, gid_map): (GlyphSet, Vec<u32>) = ([0u32, 5, 7].into_iter().collect(), vec![0, 0, 0, 0, 0, 1, 0, 2]);
    let colr_kept = ColrKept { glyphs: [0u32, 4, 5, 7].into_iter().collect(), ..kept_palettes(&[0]) };
    let out = subset_colr(&two, &colr_kept, &active, &gid_map).expect("a subset COLR");
    assert_eq!(read_u16_be(&out, 2), Some(1), "glyph 4 passed over");
}

// Layers 0 and 3 to 4 named: glyph 11's run from 3 is the second kept, so it starts at 1.
#[test]
fn a_first_layer_index_counts_every_range_below_it() {
    let list: Vec<Vec<u8>> = (0..5).map(|g| glyph(14 + g, solid(1))).collect();
    let out = tables(&colr(&[], &[], &[(10, colr_layers(1, 0)), (11, colr_layers(2, 3))], &list, &[]), &[10, 11]).0;
    let out = out.expect("a subset COLR");
    let base = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    let paint = base + read_u32_be(&out, base + 12).expect("glyph 11's paint") as usize;
    assert_eq!(out[paint..paint + 6], colr_layers(2, 1));
}

// A ClipList of a format the spec doesn't define is absent, so a subset writes none; a null box leaves out that
// glyph's box alone, the other's kept.
#[test]
fn a_clip_list_of_another_format_is_absent_and_a_null_box_drops_its_glyph_alone() {
    let bases = [(10, glyph(14, solid(1))), (11, glyph(15, solid(1)))];
    let clipped = colr(&[], &[], &bases, &[], &[(10, 10, 10), (11, 11, 20)]);
    let list = read_u32_be(&clipped, 22).expect("a ClipList") as usize;
    let mut two = clipped.clone();
    two[list] = 2;
    assert_eq!(tables(&two, &[10, 11]).0.map(|t| read_u32_be(&t, 22)), Some(Some(0)));
    let mut null = clipped;
    null[list + 9..list + 12].fill(0);
    let out = tables(&null, &[10, 11]).0.expect("a subset COLR");
    let clips = read_u32_be(&out, 22).expect("a ClipList") as usize;
    assert_ne!(clips, 0);
    assert_eq!(read_u32_be(&out, clips + 1), Some(1));
    assert_eq!([read_u16_be(&out, clips + 5), read_u16_be(&out, clips + 7)], [Some(2), Some(2)]);
}

// Version 1's tail goes with version 1 alone; a kept index past the colors has no label; labels the same as the
// palette labels share them; past 65,535 records there is no CPAL; and only version 1 names labels.
#[test]
fn cpal_keeps_its_tail_by_version_and_its_records_in_bounds() {
    let mut two = cpal();
    two[1] = 2;
    let out = subset_cpal(&two, &kept_palettes(&[3, 5])).expect("a subset CPAL");
    assert_eq!((read_u16_be(&out, 0), out.len()), (Some(2), 12 + 6 + 16));
    assert!(cpal_names(&two, &kept_palettes(&[3])).is_empty());
    let out = subset_cpal(&cpal(), &kept_palettes(&[3, 9])).expect("a subset CPAL");
    let field = |at: usize| read_u32_be(&out, at).expect("an offset") as usize;
    let entry_labels = (read_u16_be(&out, field(26)), field(26) - field(22));
    assert_eq!(entry_labels, (Some(313), 6), "palette labels, then entry labels");
    let out = subset_cpal(&cpal_labelled([310, 311, 312]), &kept_palettes(&[0, 1, 2])).expect("a subset CPAL");
    let field = |at: usize| read_u32_be(&out, at).expect("an offset") as usize;
    assert_eq!((field(8), field(22), field(26)), (30, 30 + 36 + 12, 30 + 36 + 12), "records, types, one label array");
    let n = 300u16;
    let records_at = (12 + 2 * u32::from(n)).to_be_bytes().to_vec();
    let mut wide = [[0, n, n, 2 * n].map(u16::to_be_bytes).concat(), records_at].concat();
    wide.extend((0..n).flat_map(u16::to_be_bytes));
    wide.extend((0..2 * u32::from(n)).flat_map(u32::to_be_bytes));
    assert_eq!(subset_cpal(&wide, &kept_palettes(&(0..n).collect::<Vec<_>>())), None);
    assert!(subset_cpal(&wide, &kept_palettes(&(0..200).collect::<Vec<_>>())).is_some());
}

// Glyphs 10 and 11 paint matching trees, 11's child 5 bytes further from its PaintGlyph: alike once their offsets are
// set aside, so both records name one paint, written in 61 bytes.
#[test]
fn alike_paints_at_different_offsets_are_written_once() {
    let padded = [&[10, 0, 0, 11][..], &14u16.to_be_bytes(), &[0; 5], &solid(1)].concat();
    let out = tables(&colr(&[], &[], &[(10, glyph(14, solid(1))), (11, padded)], &[], &[]), &[10, 11]).0;
    let out = out.expect("a subset COLR");
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    assert_eq!(out.len(), 61);
    assert_eq!(read_u32_be(&out, list + 6), read_u32_be(&out, list + 12), "the two records name one paint");
}

// Glyph 10's layer names glyph 13, and 13's own record names layer 1, just past 10's range: 13 keeps its record, so
// its paint is walked and the LayerList holds both layers.
#[test]
fn a_glyph_a_layer_names_keeps_its_own_layers() {
    let list = [glyph(13, solid(1)), glyph(15, solid(1))];
    let t = colr(&[], &[], &[(10, colr_layers(1, 0)), (13, colr_layers(1, 1))], &list, &[]);
    let out = tables(&t, &[10]).0.expect("a subset COLR");
    assert_eq!(read_u32_be(&out, read_u32_be(&out, 18).expect("a LayerList") as usize), Some(2));
    assert!(!out.windows(4).any(|w| w == [0xFF; 4]), "no layer is named past the kept ones");
}

// A version 1 record the search misses among misordered ones, as drawing reads them, walks nothing: of glyphs 5, 3
// and 7, the search finds 3 and 7, so only their paints' glyphs are kept.
#[test]
fn version_1_paints_are_found_by_the_record_search() {
    let t = version_1(&[(5, glyph(20, solid(0))), (3, glyph(21, solid(0))), (7, glyph(22, solid(0)))], &[]);
    assert_eq!([closed(&t, &[5]), closed(&t, &[3]), closed(&t, &[7])], [vec![], vec![21], vec![22]]);
}
