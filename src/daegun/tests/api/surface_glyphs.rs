use daegun::Font;

fn bytes_of(rel: &str) -> Vec<u8> {
    let path = format!("{}/{}", crate::FONTS, rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

fn font(rel: &str) -> Font {
    Font::from_bytes(&bytes_of(rel)).unwrap_or_else(|e| panic!("{rel} did not parse: {e}"))
}

fn colr_table() -> Vec<u8> {
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes_of(COLR)).expect("parses");
    map.get("COLR").expect("the COLR fixture carries a COLR table").to_owned_vec()
}

const CARETS: &str = "test-fixtures/carets.ttf";
const COLR: &str = "colr-v1-test-glyphs/test_glyphs.ttf";
const SOURCE_HAN: &str = "source-han-sans/SourceHanSansJP-VF.otf";

#[test]
fn ligature_carets_come_back_for_a_font_that_declares_them() {
    let f = font(CARETS);
    let three = 4u16;
    let two = 5u16;

    // fontTools' values: f_f_i's two in format 1, f_i's in format 2 at its point 1, (420, 0).
    assert_eq!(f.ligature_carets(three, &[], false), [Some(300.0), Some(600.0)]);
    assert_eq!(f.ligature_carets(two, &[], false), [Some(420.0)]);
    assert_eq!(f.ligature_carets(two, &[], true), [Some(0.0)]);
    assert_eq!(f.ligature_carets(6, &[], false), [Some(250.0), Some(500.0), Some(750.0)]);
    assert!(f.ligature_carets(1, &[], false).is_empty(), "a non-ligature glyph reported carets");
}

#[test]
fn caret_positions_span_the_text_they_are_asked_about() {
    let f = font(CARETS);
    let carets = f.caret_positions("ffi", &[], false).expect("the fixture shapes 'ffi'");
    assert!(
        carets.len() >= "ffi".chars().count(),
        "expected at least one caret per character, got {} for 3 characters",
        carets.len(),
    );
    assert!(carets.iter().all(|v| v.is_finite()), "a caret position is not finite");
    assert!(
        carets.windows(2).all(|w| w[0] <= w[1]),
        "caret positions are not monotonic across the run: {carets:?}",
    );
}

// Values from fontTools. The pairs sit first, last and deep in their lists, so a lookup that
// misreads a record stride or swaps the default and non-default lists lands elsewhere.
#[test]
fn variation_sequences_resolve_through_format_14() {
    let f = font(SOURCE_HAN);
    assert_eq!(f.glyph_id(0x4FAE), Some(2695));
    let cases = [
        (0x4FAE, 0xFE00, Some(15200), "the first non-default mapping of VS1"),
        (0x2A61A, 0xE0101, Some(17423), "the last of VS18's 987 non-default mappings"),
        (0x9089, 0xE010E, Some(17243), "the last selector record"),
        (0x2E6EA, 0xE0100, Some(16144), "a default sequence gives the base glyph"),
        (0x4FAE, 0xE0104, None, "a selector this base does not use"),
        (0x4FAE, 0xFE0F, None, "a selector the font lacks"),
        (0x000F_FFFD, 0xFE00, None, "an unmapped base"),
    ];
    for (base, selector, want, what) in cases {
        assert_eq!(f.variation_glyph_id(base, selector), want, "{what}: U+{base:04X} U+{selector:04X}");
    }
}

#[test]
fn a_color_glyph_becomes_a_scene() {
    use daegun::paint::Op;
    let f = font(COLR);
    let colr = colr_table();

    let off = daegun::daecore::daetype::decoder::read_u32_be(&colr, 14).expect("baseGlyphList offset") as usize;
    let gid = daegun::daecore::daetype::decoder::read_u16_be(&colr, off + 4).expect("first base glyph");

    let scene = f.colr_scene(gid, &[], 0).expect("a declared base glyph has a scene");
    let (mut fills, mut clips, mut layers) = (0usize, 0i32, 0i32);
    for op in scene.ops() {
        match op {
            Op::Fill { path, .. } => {
                assert!(scene.path(*path).is_some_and(|p| !p.is_empty()), "a fill names an empty path");
                fills += 1;
            }
            Op::PushClip { shapes } => {
                assert!(shapes.iter().all(|s| scene.path(s.path).is_some()), "a clip names a missing path");
                clips += 1;
            }
            Op::PopClip => clips -= 1,
            Op::PushLayer { .. } => layers += 1,
            Op::PopLayer => layers -= 1,
        }
        assert!(clips >= 0 && layers >= 0, "a pop came before its push");
    }
    assert!(fills > 0, "the scene fills nothing");
    assert_eq!((clips, layers), (0, 0), "pushes and pops do not balance");
    assert!(f.colr_scene(0, &[], 0).is_none(), ".notdef has a color scene");
}

// A glyph painted through another clips to the outer one, with a composite under it keeping both fills
// inside. The clip glyph's xMin is 100 and lsb 0, so it draws at 0, as HarfBuzz and FreeType draw it.
#[test]
fn a_glyph_painted_through_a_glyph_is_clipped() {
    use daegun::paint::Op;
    let f = font("test-fixtures/colr-clip.ttf");
    for (ch, fills) in [('N', 1), ('C', 2)] {
        let gid = f.glyph_id(ch as u32).expect("the fixture maps N and C");
        let scene = f.colr_scene(gid, &[], 0).expect("a color glyph has a scene");
        let ops = scene.ops();
        let Some(Op::PushClip { shapes }) = ops.first() else { panic!("{ch}: the scene opens with {:?}", ops.first()) };
        let clip = scene.path(shapes[0].path).expect("the clip names a path");
        assert_eq!((shapes.len(), clip.bounds()), (1, Some((0.0, 0.0, 500.0, 700.0))), "{ch}: not the outer glyph");
        assert!(matches!(ops.last(), Some(Op::PopClip)), "{ch}: the clip is never popped");
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Fill { .. })).count(), fills, "{ch}: {ops:?}");
    }
}

// Layers and clips nest at most 128 deep, a stack a rasterizer can size. Of 64 nested composites, as
// deep as a graph parses, all but the innermost two (paints past that depth) open two layers each.
#[test]
fn a_scene_nests_at_most_128_deep() {
    use daegun::paint::Op;
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('D' as u32).expect("the fixture maps D");
    let scene = f.colr_scene(gid, &[], 0).expect("the deep glyph has a scene");
    let (mut depth, mut deepest) = (0usize, 0usize);
    for op in scene.ops() {
        match op {
            Op::PushClip { .. } | Op::PushLayer { .. } => depth += 1,
            Op::PopClip | Op::PopLayer => depth -= 1,
            Op::Fill { .. } => {}
        }
        deepest = deepest.max(depth);
    }
    assert_eq!(deepest, 124, "64 nested composites held {deepest} layers open");
}

// A scene holds each glyph's outline once, however many layers name it, and is refused past
// MAX_FLATTEN_POINTS points in all: 17 composites of 64,000 points come to 1,088,000.
#[test]
fn a_scene_holds_each_outline_once_within_a_bound() {
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = |c: char| f.glyph_id(c as u32).expect("the fixture maps P and R");
    let repeated = f.colr_scene(gid('R'), &[], 0).expect("eight layers of one glyph draw");
    let paths = (0..).take_while(|&i| repeated.path(i).is_some()).count();
    let fills = repeated.ops().iter().filter(|op| matches!(op, daegun::paint::Op::Fill { .. })).count();
    assert_eq!((paths, fills), (1, 8), "eight layers of one glyph held {paths} outlines and {fills} fills");
    assert!(f.colr_scene(gid('P'), &[], 0).is_none(), "a scene past MAX_FLATTEN_POINTS points was built");
}

// S's 255 layers share one gradient of 4,000 stops, a small table naming 1,020,000. A subset writes it
// once, where a copy for each layer would pass the paint budget's million stops.
#[test]
fn subsetting_a_shared_gradient_stays_within_the_stop_budget() {
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('S' as u32).expect("the fixture maps S");
    let colr = f.table("COLR").expect("COLR").len();
    let sub = f.subset(&[gid], &[]).expect("subsets");
    assert!(sub.ttf.len() < colr + (1 << 20), "a {colr}-byte COLR became a subset of {} bytes", sub.ttf.len());
}

// The bound is the lowering's own, not only the parser's: a graph built by hand, deeper than any font
// parses, still nests no more than 128.
#[test]
fn a_hand_built_paint_graph_nests_at_most_128_deep() {
    use daegun::paint::Op;
    let solid = || Box::new(daegun::Paint::Solid { is_foreground: false, r: 0, g: 0, b: 0, alpha: 255 });
    let mut paint = daegun::Paint::Composite { source: solid(), mode: 0, backdrop: solid() };
    for _ in 1..70 {
        paint = daegun::Paint::Composite { source: Box::new(paint), mode: 0, backdrop: solid() };
    }
    let mut scene = daegun::paint::DisplayList::default();
    daegun::paint::lower(&paint, daegun::paint::IDENTITY, None, &mut |_| None, daegun::paint::Rgba { r: 0, g: 0, b: 0, a: 255 }, &mut scene);
    let (mut depth, mut deepest) = (0usize, 0usize);
    for op in scene.ops() {
        match op {
            Op::PushClip { .. } | Op::PushLayer { .. } => depth += 1,
            Op::PopClip | Op::PopLayer => depth -= 1,
            Op::Fill { .. } => {}
        }
        deepest = deepest.max(depth);
    }
    assert!(deepest <= 128, "70 composites built by hand nested {deepest} deep");
}

// L's first layer is itself a layer list. A subset must keep its own layers ahead of the nested ones,
// or its record names the nested layers in their place.
#[test]
fn a_nested_layer_list_survives_a_subset() {
    use daegun::paint::Op;
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('L' as u32).expect("the fixture maps L");
    let fills = |f: &Font, gid: u16| {
        let scene = f.colr_scene(gid, &[], 0).expect("L has a scene");
        scene.ops().iter().filter_map(|op| match op {
            Op::Fill { path, paint, .. } => Some((format!("{paint:?}"), scene.path(*path).map(|p| p.parts().1.to_vec()))),
            _ => None,
        }).collect::<Vec<_>>()
    };
    let subset = f.subset(&[gid], &[]).expect("a subset");
    let small = Font::from_vec(subset.ttf).expect("the subset parses");
    let new_gid = subset.gid_map.get(usize::from(gid)).copied().expect("L survives");
    let want = fills(&f, gid);
    assert_eq!(want.len(), 3, "L fills inner, other and clip");
    assert_eq!(fills(&small, new_gid), want, "the subset's L fills other layers");
}

// A substitution naming a glyph past the font's end is ignored, as a requested one is, so the closure
// still ends once a pass adds nothing.
#[test]
fn a_substitute_past_the_end_does_not_stall_the_closure() {
    let f = font("test-fixtures/hinted.ttf");
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = f.table_tags().into_iter()
        .map(|t| (t.to_string(), f.table(t).expect("listed").to_vec()))
        .collect();
    let gsub = [1u16, 0, 0, 0, 10, 1, 4, 1, 0, 1, 8, 2, 8, 1, 60_000, 1, 1, 1];
    tables.insert("GSUB".into(), gsub.iter().flat_map(|v| v.to_be_bytes()).collect());
    let font = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    assert!(font.glyph_closure(&[1], &[]).is_ok(), "the closure did not converge");
    assert!(font.subset(&[1], &[]).is_ok(), "the subset did not converge");
}
