use super::*;

// Offsets near u32::MAX, which on a 32-bit target would overflow an unchecked sum and trap in a
// debug build: the paint is past the table, and there is none.
#[test]
fn offsets_near_the_limit_read_as_absent() {
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&34u32.to_be_bytes());
    colr.extend(1u32.to_be_bytes());
    colr.extend(1u16.to_be_bytes());
    colr.extend(0xFFFF_FFF0u32.to_be_bytes());
    let map = testing::tables(&colr, [0, 0, 0, 255]);
    let var = parse_colr_v1_var_data(&colr);
    assert_eq!(colr_v1_paint_graph_with_scalars(&map, 1, 0, &var, &[]), None);
    let deltas = [Deltas::Measured(var.as_ref(), &[]), Deltas::Drawn(None)];
    assert!(deltas.iter().all(|&d| clip_box(&colr, 1, d).is_none()));
}

// A region whose axes all peak at 0 applies everywhere but the default, where coordinates that are all 0 vary
// nothing.
#[test]
fn nothing_varies_at_coordinates_that_are_all_0() {
    let words = [1u16, 0, 12, 1, 0, 22, 1, 1, 0, 0, 0, 1, 1, 1, 0, 2];
    let store: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let var = DeltaSource::read(&store, 0, None).ok();
    assert!(colr_v1_region_scalars(&var, &[0.0]).is_empty());
    assert_eq!(colr_v1_region_scalars(&var, &[0.25]), [65_536]);
}

// The minimums go down and the maximums up, however little past a whole unit: 1.00000001 takes 2.
#[test]
fn a_clip_box_floors_its_minimums_and_ceils_its_maximums() {
    let deltas = [-2.5, 2.5, -0.5, 1.000_000_01];
    assert_eq!(varied_box([0; 4], |k| deltas[k as usize]), [-3, 2, 0, 2]);
}

// 2,000 glyphs sharing 60 layers walk 242,000 paints between them, past one budget of 100,000;
// the instancer walks each paint once, so the last glyph's alpha still varies.
#[test]
fn instancing_reaches_every_glyph_of_a_shared_graph() {
    let colr = testing::shared_layers(2000, 60, true);
    let out = instance_colr_v1(&colr, &[1.0]).expect("an instance");
    let map = testing::tables(&out, [0, 0, 0, 255]);
    let var = parse_colr_v1_var_data(&out);
    assert!(var.is_none(), "the store is left");
    let graph = colr_v1_paint_graph_with_scalars(&map, 2000, 0, &var, &[]).expect("glyph 2000");
    assert_eq!(graph.root().node(), PaintNode::Layers);
    let layers: Vec<_> = graph.root().children().collect();
    assert_eq!(layers.len(), 60);
    let solid = |l: &PaintRef| {
        l.children().next().is_some_and(|c| matches!(c.node(), PaintNode::Solid { color: Rgba { a: 0, .. }, .. }))
    };
    assert!(layers.iter().all(|l| matches!(l.node(), PaintNode::Glyph { .. }) && solid(l)));
}

// A PaintSolid (glyph 1) and a gradient's one stop (glyph 2) naming a palette entry of alpha 200, at
// alpha 4536/16384: 0.2769 * 200 = 55.37 for both, rounded once.
#[test]
fn a_solid_and_a_stop_of_one_color_take_one_alpha() {
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&34u32.to_be_bytes());
    colr.extend(2u32.to_be_bytes());
    colr.extend([&1u16.to_be_bytes()[..], &16u32.to_be_bytes(), &2u16.to_be_bytes(), &21u32.to_be_bytes()].concat());
    colr.extend([&[2u8][..], &0u16.to_be_bytes(), &4536i16.to_be_bytes()].concat());
    colr.extend([&[4u8, 0, 0, 16][..], &[0u16, 0, 100, 0, 0, 100].map(u16::to_be_bytes).concat()].concat());
    colr.extend([&[0u8][..], &1u16.to_be_bytes(), &[0u16, 0, 4536].map(u16::to_be_bytes).concat()].concat());
    let map = testing::tables(&colr, [0, 0, 255, 200]);
    let var = parse_colr_v1_var_data(&map["COLR"]);
    let [one, two] = [1, 2].map(|gid| colr_v1_paint_graph_with_scalars(&map, gid, 0, &var, &[]));
    let solid = match one.as_ref().map(|g| g.root().node()) {
        Some(PaintNode::Solid { color, .. }) => color.a,
        other => panic!("glyph 1: {other:?}"),
    };
    let stop = match two.as_ref().map(|g| g.root().node()) {
        Some(PaintNode::LinearGradient { stops, .. }) => stops[0].color.a,
        other => panic!("glyph 2: {other:?}"),
    };
    assert_eq!((solid, stop), (55, 55));
}

// Glyph 2's gradient comes first and builds the color line both share, so the line must move past
// glyph 1's gradient too, as an offset can't point back.
#[test]
fn a_color_line_two_gradients_share_is_placed_after_both() {
    let line = [&[0u8][..], &2u16.to_be_bytes(),
                &[0u16, 0xFFFF, 0x4000].map(u16::to_be_bytes).concat(), &u32::MAX.to_be_bytes(),
                &[0x4000u16, 0xFFFF, 0x4000].map(u16::to_be_bytes).concat(), &u32::MAX.to_be_bytes()].concat();
    let gradient =
        [&[5u8, 0, 0, 20][..], &[0u16, 0, 100, 0, 0, 100].map(u16::to_be_bytes).concat(), &0u32.to_be_bytes()].concat();
    let (list, glyph2) = (34usize, 50usize);
    let glyph1 = glyph2 + gradient.len() + line.len();
    let store_at = glyph1 + gradient.len() + line.len();
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&(list as u32).to_be_bytes());
    colr[30..34].copy_from_slice(&(store_at as u32).to_be_bytes());
    colr.extend(2u32.to_be_bytes());
    for (gid, paint) in [(1u16, glyph1), (2, glyph2)] {
        colr.extend(gid.to_be_bytes());
        colr.extend(((paint - list) as u32).to_be_bytes());
    }
    colr.extend([&gradient[..], &line, &gradient, &line].concat());
    colr.extend(testing::store(1, 50));

    let out = instance_colr_v1(&colr, &[1.0]).expect("an instance");
    let map = testing::tables(&out, [0, 0, 0, 255]);
    let var = parse_colr_v1_var_data(&out);
    for gid in [1, 2] {
        let graph = colr_v1_paint_graph_with_scalars(&map, gid, 0, &var, &[]);
        let x0 = match graph.as_ref().map(|g| g.root().node()) {
            Some(PaintNode::LinearGradient { x0, .. }) => x0,
            other => panic!("glyph {gid}: {other:?}"),
        };
        assert_eq!(x0, 50.0, "glyph {gid} lost its variation");
    }
}

// A PaintVarLinearGradient whose color line offset is 0 names no line: instanced, it stays null, rather than its own
// bytes read as a line whose extend is the paint's format.
#[test]
fn instancing_a_gradient_with_a_null_color_line_keeps_it_null() {
    let gradient =
        [&[5u8, 0, 0, 0][..], &[0u16, 0, 100, 0, 0, 100].map(u16::to_be_bytes).concat(), &0u32.to_be_bytes()].concat();
    let (list, paint) = (34usize, 44usize);
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&(list as u32).to_be_bytes());
    colr[30..34].copy_from_slice(&((paint + gradient.len()) as u32).to_be_bytes());
    colr.extend([&1u32.to_be_bytes()[..], &1u16.to_be_bytes(), &((paint - list) as u32).to_be_bytes()].concat());
    colr.extend(&gradient);
    colr.extend(testing::store(1, 50));
    let out = instance_colr_v1(&colr, &[1.0]).expect("an instance");
    let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
    let root = list + read_u32_be(&out, list + 6).expect("a root") as usize;
    assert_eq!((out[root], read_offset24(&out, root + 1)), (4, Some(0)));
}

// A tree of layers 3 deep, 2 by 255 by 255, its 130,050 leaves one shared solid: 130,563 paints read, past
// MAX_PAINT_VISITS. Its 525 KB table allows a visit for each 4 bytes more, so every leaf is read.
#[test]
fn a_table_s_bytes_allow_its_paints_a_visit_each() {
    let (wide, leaves) = (255usize, 2 * 255 * 255);
    let entries = 2 + 2 * wide + leaves;
    let (list, layer_list) = (34usize, 44usize);
    let paints = layer_list + 4 + 4 * entries;
    let inner = |i: usize| paints + 6 + 6 * i;
    let solid = inner(2 + 2 * wide);
    let mut colr = alloc::vec![0u8; 34];
    colr[1] = 1;
    colr[14..18].copy_from_slice(&(list as u32).to_be_bytes());
    colr[18..22].copy_from_slice(&(layer_list as u32).to_be_bytes());
    colr.extend([&1u32.to_be_bytes()[..], &1u16.to_be_bytes(), &((paints - list) as u32).to_be_bytes()].concat());
    colr.extend((entries as u32).to_be_bytes());
    let at = |p: usize| ((p - layer_list) as u32).to_be_bytes();
    (0..2 + 2 * wide).for_each(|i| colr.extend(at(inner(i))));
    (0..leaves).for_each(|_| colr.extend(at(solid)));
    let layers = |n: usize, first: usize| [&[1u8, n as u8][..], &(first as u32).to_be_bytes()].concat();
    colr.extend(layers(2, 0));
    (0..2).for_each(|i| colr.extend(layers(wide, 2 + i * wide)));
    (0..2 * wide).for_each(|i| colr.extend(layers(wide, 2 + 2 * wide + i * wide)));
    colr.extend([2, 0, 0, 0x40, 0]);
    let map = testing::tables(&colr, [0, 0, 255, 255]);
    let graph = colr_v1_paint_graph_with_scalars(&map, 1, 0, &None, &[]).expect("glyph 1");
    fn solids(p: PaintRef) -> usize {
        usize::from(matches!(p.node(), PaintNode::Solid { .. })) + p.children().map(solids).sum::<usize>()
    }
    assert_eq!(solids(graph.root()), leaves);
}
