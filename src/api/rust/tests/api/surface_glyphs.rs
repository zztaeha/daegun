use daegun::color::SceneOptions;
use daegun::shape::{Direction, ShapeOptions};
use daegun::unicode::BaseDirection;
use daegun::{Font, GlyphId, Location, Orientation, Tag};

fn bytes_of(rel: &str) -> Vec<u8> {
    let path = format!("{}/{}", crate::FONTS, rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

fn font(rel: &str) -> Font {
    Font::from_bytes(&bytes_of(rel)).unwrap_or_else(|e| panic!("{rel} did not parse: {e}"))
}

fn colr_table() -> Vec<u8> {
    font(COLR).table(Tag::new(b"COLR")).expect("the COLR fixture carries a COLR table").to_vec()
}

fn scene_of(f: &Font, gid: GlyphId) -> Option<daegun::color::DisplayList> {
    f.color_scene(gid, &Location::default(), &SceneOptions::default()).expect("the scene reads")
}

fn bidi(f: &Font, text: &str, options: &ShapeOptions) -> Vec<daegun::shape::BidiRun> {
    f.shape_bidi(text, &Location::default(), BaseDirection::Auto, options)
}

const CARETS: &str = "test-fixtures/carets.ttf";
const COLR: &str = "colr-v1-test-glyphs/test_glyphs.ttf";
const SOURCE_HAN: &str = "source-han-sans/SourceHanSansJP-VF.otf";

#[test]
fn ligature_carets_come_back_for_a_font_that_declares_them() {
    let f = font(CARETS);
    let three = 4u32;
    let two = 5u32;

    // The table's stored values: f_f_i's two in format 1, f_i's in format 2 at its point 1, (420, 0), 850 below the
    // vertical origin it's drawn from.
    assert_eq!(
        f.ligature_carets(GlyphId(three), &Location::default(), Orientation::Horizontal),
        [Some(300.0), Some(600.0)]
    );
    assert_eq!(f.ligature_carets(GlyphId(two), &Location::default(), Orientation::Horizontal), [Some(420.0)]);
    assert_eq!(f.ligature_carets(GlyphId(two), &Location::default(), Orientation::Vertical), [Some(-850.0)]);
    assert_eq!(
        f.ligature_carets(GlyphId(6), &Location::default(), Orientation::Horizontal),
        [Some(250.0), Some(500.0), Some(750.0)]
    );
    assert!(
        f.ligature_carets(GlyphId(1), &Location::default(), Orientation::Horizontal).is_empty(),
        "a non-ligature glyph reported carets"
    );
}

#[test]
fn caret_positions_span_the_text_they_are_asked_about() {
    let f = font(CARETS);
    let carets = f.caret_positions("ffi", &Location::default(), Orientation::Horizontal);
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

// With its ligatures under ccmp, which vertical text applies, "fi" forms f_i: its caret at point 1's y of 0 sits 850
// down from the top shaping sets for it, measured from the baseline as its other formats are.
#[test]
fn a_vertical_caret_on_a_point_sits_down_from_the_glyph_top() {
    let f = font(CARETS);
    let mut t: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let gsub = t.get_mut(&Tag::new(b"GSUB")).expect("a GSUB");
    let at = gsub.windows(4).position(|w| w == b"liga").expect("a liga feature");
    gsub[at..at + 4].copy_from_slice(b"ccmp");
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    assert_eq!(
        f.shape("fi", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom)).glyphs,
        [GlyphId(5)]
    );
    assert_eq!(f.caret_positions("fi", &Location::default(), Orientation::Vertical), [0.0, 850.0, 1000.0]);
}

// A base with a combining mark, a pre-base matra and a right-to-left mark each shape to several glyphs in
// one cluster. The caret before the cluster sits at its edge, and bytes inside the grapheme share it.
#[test]
fn a_cluster_of_several_glyphs_places_its_carets_at_its_whole_extent() {
    let inter = font("inter/InterVariable.ttf");
    assert_eq!(
        inter.caret_positions("ax\u{301}b", &Location::default(), Orientation::Horizontal),
        [0.0, 1150.0, 1150.0, 1150.0, 2268.0, 3522.0]
    );
    let deva = font("noto-devanagari/NotoSansDevanagari.ttf");
    assert_eq!(
        deva.caret_positions("\u{915}\u{93F}\u{915}", &Location::default(), Orientation::Horizontal),
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1027.0, 1027.0, 1027.0, 1795.0]
    );
    assert_eq!(
        deva.caret_positions("\u{915}\u{915}\u{93F}", &Location::default(), Orientation::Horizontal),
        [0.0, 0.0, 0.0, 768.0, 768.0, 768.0, 768.0, 768.0, 768.0, 1795.0]
    );
    let arabic = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    assert_eq!(
        arabic.caret_positions("\u{628}\u{64E}\u{628}", &Location::default(), Orientation::Horizontal),
        [2520.0, 2520.0, 2520.0, 2520.0, 1998.0, 1998.0, 0.0]
    );
}

// Mixed-direction text resolves its bidi runs: digits in Arabic run left to right, and each run's first
// character sits at the edge layout starts that run from.
#[test]
fn carets_follow_the_bidi_runs_layout_draws() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    assert_eq!(
        f.caret_positions("\u{628}\u{628} 123", &Location::default(), Orientation::Horizontal),
        [5848.0, 5848.0, 5326.0, 5326.0, 3328.0, 0.0, 926.0, 1852.0, 2778.0],
    );
    for text in ["\u{628}\u{628} 123", "abc \u{628}\u{628}\u{628} def", "a\u{301}\u{628}\u{64E}c"] {
        let carets = f.caret_positions(text, &Location::default(), Orientation::Horizontal);
        for run in f
            .layout(text, &Location::default(), &daegun::layout::LayoutOptions::default())
            .lines
            .iter()
            .flat_map(|l| &l.runs)
        {
            let width: f64 = run.run.advances.iter().sum();
            let leading = if run.level % 2 == 1 { run.offset.x + width } else { run.offset.x };
            assert_eq!(carets[run.bytes.start], leading, "{text:?}: the run over {:?}", run.bytes);
        }
    }
}

// A bidi run of two scripts is cut where the script changes, as layout cuts lines, each piece shaped as its own: Arabic
// after a Hebrew letter keeps its joining, Latin, Greek and Cyrillic each take a run, and the carets follow.
#[test]
fn shape_bidi_cuts_each_bidi_run_by_script_as_layout_does() {
    let arabic = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let alone = bidi(&arabic, "\u{645}\u{631}\u{62D}\u{628}\u{627}", &ShapeOptions::default());
    let after_hebrew = bidi(&arabic, "\u{5E9} \u{645}\u{631}\u{62D}\u{628}\u{627}", &ShapeOptions::default());
    let word = after_hebrew.iter().find(|r| r.bytes.start == 3).expect("a run that starts at the Arabic word");
    assert_eq!(word.run.glyphs, alone[0].run.glyphs, "the Arabic after a Hebrew letter lost its joining forms");

    let garamond = font("eb-garamond/EBGaramond.ttf");
    for (f, text) in [
        (&arabic, "\u{5E9} \u{645}\u{631}\u{62D}\u{628}\u{627}"),
        (&garamond, "Ab \u{391}\u{3B2} \u{412}\u{432}"),
    ] {
        let runs = bidi(f, text, &ShapeOptions::default());
        let line = &f.layout(text, &Location::default(), &daegun::layout::LayoutOptions::default()).lines[0];
        assert_eq!(runs.len(), line.runs.len(), "{text:?}: {} runs where layout has {}", runs.len(), line.runs.len());
        let carets = f.caret_positions(text, &Location::default(), Orientation::Horizontal);
        for (r, l) in runs.iter().zip(&line.runs) {
            assert_eq!(r.bytes, l.bytes, "{text:?}");
            let (ours, layouts) = ((&r.run.glyphs, &r.run.advances), (&l.run.glyphs, &l.run.advances));
            assert_eq!(ours, layouts, "{text:?} over {:?}", l.bytes);
            let leading = if l.level % 2 == 1 { l.offset.x + l.run.width() } else { l.offset.x };
            assert_eq!(carets[l.bytes.start], leading, "{text:?}: the caret at {}", l.bytes.start);
        }
    }

    let named = daegun::shape::ShapeOptions::default().with_script(Tag::new(b"latn"));
    let whole = bidi(&garamond, "Ab \u{391}\u{3B2} \u{412}\u{432}", &named);
    assert_eq!(whole.len(), 1, "a script the caller names keeps the bidi run whole");
    assert_eq!(whole[0].bytes, (0..12).into());
}

// The table's stored values. The pairs sit first, last and deep in their lists, so a lookup that misreads a record
// stride or swaps the default and non-default lists finds another glyph.
#[test]
fn variation_sequences_resolve_through_format_14() {
    let f = font(SOURCE_HAN);
    assert_eq!(f.glyph_id('\u{4FAE}'), Some(GlyphId(2695)));
    let cases = [
        ('\u{4FAE}', '\u{FE00}', Some(15200), "the first non-default mapping of VS1"),
        ('\u{2A61A}', '\u{E0101}', Some(17423), "the last of VS18's 987 non-default mappings"),
        ('\u{9089}', '\u{E010E}', Some(17243), "the last selector record"),
        ('\u{2E6EA}', '\u{E0100}', Some(16144), "a default sequence gives the base glyph"),
        ('\u{4FAE}', '\u{E0104}', None, "a selector this base does not use"),
        ('\u{4FAE}', '\u{FE0F}', None, "a selector the font lacks"),
        ('\u{FFFFD}', '\u{FE00}', None, "an unmapped base"),
    ];
    for (base, selector, want, what) in cases {
        let (b, v) = (u32::from(base), u32::from(selector));
        assert_eq!(f.variation_glyph_id(base, selector), want.map(GlyphId), "{what}: U+{b:04X} U+{v:04X}");
    }
}

// Built as Apple's bitmap-only fonts are (NISC18030): bhed, bloc and bdat, no outlines, hhea or hmtx, its line its 16
// ppem strike's on a 2048 em. 'A' (1) has big metrics, 'B' (2) small, 3 advances of 0.
fn apple_bitmap_font() -> Font {
    let big = [&[6u8, 3, 1, 9, 12, 0xFF, 2, 14][..], &[0b1110_0000; 6]].concat();
    let small = [&[2u8, 2, 0, 2, 6][..], &[0b1100_0000; 2]].concat();
    let still = [&[1u8, 1, 0, 1, 0, 0, 0, 0][..], &[0x80]].concat();
    let images = [(1u16, 6u16, big), (2, 1, small), (3, 6, still)];
    let mut bloc = [0x0002_0000u32, 1, 56, 0, images.len() as u32, 0].map(u32::to_be_bytes).concat();
    bloc.extend([18, -2i8 as u8, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    bloc.extend([10, -10i8 as u8, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    bloc.extend([0, 0, 0xFF, 0xFF, 16, 16, 1, 1]);
    let (mut bdat, mut subtables) = (0x0002_0000u32.to_be_bytes().to_vec(), Vec::new());
    for (gid, format, image) in &images {
        bloc.extend([gid.to_be_bytes(), gid.to_be_bytes()].concat());
        bloc.extend(((images.len() * 8 + subtables.len()) as u32).to_be_bytes());
        subtables.extend([1u16, *format].map(u16::to_be_bytes).concat());
        subtables.extend([bdat.len() as u32, 0, image.len() as u32].map(u32::to_be_bytes).concat());
        bdat.extend(image);
    }
    bloc.extend(subtables);
    let cmap = super::aat_shaping::cmap(&[(0x41, 0x42, 1)]);
    let tables: std::collections::BTreeMap<Tag, Vec<u8>> = [
        ("bhed", font("inter/InterVariable.ttf").table(Tag::new(b"head")).expect("head").to_vec()),
        ("maxp", [0, 0, 0x50, 0, 0, 4].to_vec()),
        ("cmap", cmap),
        ("bloc", bloc),
        ("bdat", bdat),
    ]
    .into_iter()
    .map(|(t, b)| (t.parse().expect("a tag"), b))
    .collect();
    Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads")
}

#[test]
fn an_apple_bitmap_font_takes_its_metrics_from_its_strike() {
    let f = apple_bitmap_font();
    assert_eq!(f.units_per_em(), 2048);

    let a = f.bitmap(GlyphId(1), 16).expect("glyph 1 draws from bdat");
    assert_eq!((a.ppem, a.left, a.top), (16, 1, 9));
    assert_eq!(a.image, daegun::color::BitmapImage::Coverage { width: 3, height: 6, pixels: vec![255; 18] });

    let across = crate::advances(&f, &[GlyphId(0), GlyphId(1), GlyphId(2), GlyphId(3)], &Location::default());
    assert_eq!(across, [1024.0, 1536.0, 768.0, 0.0], "half an em without a bitmap, else 12, 6 and 0 px");
    assert_eq!(f.shape("AB", &Location::default(), &ShapeOptions::default()).advances, [1536.0, 768.0]);
    let down: Vec<f64> = (0..4).map(|g| f.advance(GlyphId(g), &Location::default(), Orientation::Vertical)).collect();
    assert_eq!(down, [2560.0, 1792.0, 2560.0, 0.0], "14 and 0 px down where big metrics say, else the line's 20 px");

    assert_eq!(
        (f.ascender(&Location::default()), f.descender(&Location::default()), f.cap_height(&Location::default())),
        (2304.0, -256.0, 2304.0)
    );
    let line = |vertical| {
        let m = f.line_metrics(&Location::default(), vertical);
        (m.ascent, m.descent, m.line_gap)
    };
    assert_eq!(
        (line(Orientation::Horizontal), line(Orientation::Vertical)),
        ((2304.0, -256.0, 0.0), (1280.0, -1280.0, 0.0))
    );
    assert_eq!(f.vertical_origin(GlyphId(1), &Location::default()), 2048.0, "the 6 px box centered on the 20 px line");
    assert_eq!(f.vertical_origin(GlyphId(0), &Location::default()), 2304.0, "the ascender, for a glyph with no box");
}

// Its subset reads bhed where head is missing, rather than refusing the font for want of one, and keeps bloc and bdat
// for the kept glyphs, so it draws, advances and lines up as its font.
#[test]
fn an_apple_bitmap_fonts_subset_draws_and_measures_as_its_font() {
    let f = apple_bitmap_font();
    let out = f.subset_text("AB", &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font.clone(), 0).expect("the subset loads");
    assert_eq!((sub.table(Tag::new(b"head")), sub.table(Tag::new(b"bhed")).is_some()), (None, true));
    assert_eq!(sub.units_per_em(), f.units_per_em());
    for g in [0, 1, 2] {
        let new = out.renumbered(GlyphId(g)).expect("kept");
        assert_eq!(sub.bitmap(new, 16), f.bitmap(GlyphId(g), 16), "glyph {g}'s bitmap");
        let (kept, own) = (
            crate::advances(&sub, &[GlyphId(new.0)], &Location::default()),
            crate::advances(&f, &[GlyphId(g)], &Location::default()),
        );
        assert_eq!(kept, own, "glyph {g}'s advance");
        let down = |f: &Font, g| f.advance(g, &Location::default(), Orientation::Vertical);
        assert_eq!(down(&sub, new), down(&f, GlyphId(g)), "glyph {g}'s vertical advance");
        assert_eq!(
            sub.vertical_origin(new, &Location::default()),
            f.vertical_origin(GlyphId(g), &Location::default()),
            "glyph {g}'s vertical origin"
        );
    }
    assert_eq!(
        sub.shape("AB", &Location::default(), &ShapeOptions::default()).advances,
        f.shape("AB", &Location::default(), &ShapeOptions::default()).advances
    );
    for o in [Orientation::Horizontal, Orientation::Vertical] {
        assert_eq!(sub.line_metrics(&Location::default(), o), f.line_metrics(&Location::default(), o), "{o:?}");
    }
    assert_eq!(
        (sub.ascender(&Location::default()), sub.descender(&Location::default()), sub.cap_height(&Location::default())),
        (2304.0, -256.0, 2304.0)
    );
}

// Text with no script of its own, as in text-rendering-tests' CMAP-2/2: cut between the symbol and its
// selector, each would shape alone and the sequence's glyph would be lost.
#[test]
fn a_selector_after_a_symbol_stays_in_its_run() {
    let f = font("stix-two-math/STIX2Math.otf");
    let text = "\u{2269}\u{FE00}";
    let want = f.variation_glyph_id('\u{2269}', '\u{FE00}').expect("STIX Two Math maps the sequence");
    let runs = daegun::unicode::script_runs(text);
    let cut: Vec<_> = runs.iter().map(|r| (r.bytes.start, r.bytes.end, r.script.name())).collect();
    assert_eq!(cut, [(0, 6, "Common")]);

    let runs = bidi(&f, text, &ShapeOptions::default());
    let got: Vec<_> = runs.iter().map(|r| (core::ops::Range::from(r.bytes), r.run.glyphs[0])).collect();
    assert_eq!(got, [(0..6, want)]);
    let line = &f.layout(text, &Location::default(), &daegun::layout::LayoutOptions::default()).lines[0];
    let got: Vec<_> = line.runs.iter().map(|r| (core::ops::Range::from(r.bytes), r.run.glyphs[0])).collect();
    assert_eq!(got, [(0..6, want)]);
    assert_eq!(daegun::unicode::script_runs("!\u{301}?").len(), 1, "a mark between symbols split the run");
}

#[test]
fn a_color_glyph_becomes_a_scene() {
    use daegun::color::Op;
    let f = font(COLR);
    let colr = colr_table();

    let off = u32::from_be_bytes(colr[14..18].try_into().expect("baseGlyphList offset")) as usize;
    let gid = GlyphId(u16::from_be_bytes([colr[off + 4], colr[off + 5]]).into());

    let scene = scene_of(&f, gid).expect("a declared base glyph has a scene");
    let (mut fills, mut clips, mut layers) = (0usize, 0i32, 0i32);
    for op in scene.ops() {
        match op {
            Op::Fill { path, .. } => {
                assert!(scene.path(*path).is_some_and(|p| !p.is_empty()), "a fill names an empty path");
                fills += 1;
            }
            Op::PushClip { shapes } => {
                let named = scene.shapes(*shapes).iter().all(|s| scene.path(s.path).is_some());
                assert!(named, "a clip names a missing path");
                clips += 1;
            }
            Op::PopClip => clips -= 1,
            Op::PushLayer { .. } => layers += 1,
            Op::PopLayer => layers -= 1,
            other => panic!("an op this test doesn't know: {other:?}"),
        }
        assert!(clips >= 0 && layers >= 0, "a pop came before its push");
    }
    assert!(fills > 0, "the scene fills nothing");
    assert_eq!((clips, layers), (0, 0), "pushes and pops do not balance");
    assert!(scene_of(&f, GlyphId(0)).is_none(), ".notdef has a color scene");
}

// A glyph painted through another clips to the outer one, with a composite under it keeping both fills
// inside. The clip glyph's xMin is 100 and lsb 0, so it draws at 0, where its bearing puts it.
#[test]
fn a_glyph_painted_through_a_glyph_is_clipped() {
    use daegun::color::Op;
    let f = font("test-fixtures/colr-clip.ttf");
    for (ch, fills) in [('N', 1), ('C', 2)] {
        let gid = f.glyph_id(ch).expect("the fixture maps N and C");
        let scene = scene_of(&f, gid).expect("a color glyph has a scene");
        let ops = scene.ops();
        let Some(&Op::PushClip { shapes }) = ops.first() else { panic!("{ch}: the scene opens {:?}", ops.first()) };
        let shapes = scene.shapes(shapes);
        let clip = scene.path(shapes[0].path).expect("the clip names a path");
        assert_eq!(
            (shapes.len(), clip.bounds()),
            (1, Some(daegun::Bounds { x_min: 0.0, y_min: 0.0, x_max: 500.0, y_max: 700.0 })),
            "{ch}: not the outer glyph"
        );
        assert!(matches!(ops.last(), Some(Op::PopClip)), "{ch}: the clip is never popped");
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Fill { .. })).count(), fills, "{ch}: {ops:?}");
    }
}

// Layers and clips nest at most 128 deep, a stack a rasterizer can size. Of 64 nested composites, as
// deep as a graph parses, all but the innermost two (paints past that depth) open two layers each.
#[test]
fn a_scene_nests_at_most_128_deep() {
    use daegun::color::Op;
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('D').expect("the fixture maps D");
    let scene = scene_of(&f, gid).expect("the deep glyph has a scene");
    let (mut depth, mut deepest) = (0usize, 0usize);
    for op in scene.ops() {
        match op {
            Op::PushClip { .. } | Op::PushLayer { .. } => depth += 1,
            Op::PopClip | Op::PopLayer => depth -= 1,
            Op::Fill { .. } => {}
            other => panic!("an op this test doesn't know: {other:?}"),
        }
        deepest = deepest.max(depth);
    }
    assert_eq!(deepest, 124, "64 nested composites held {deepest} layers open");
}

// A scene holds each glyph's outline once, however many layers name it, and is refused past the 2^20 points in all a
// small font's may hold: 17 composites of 64,000 points come to 1,088,000.
#[test]
fn a_scene_holds_each_outline_once_within_a_bound() {
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = |c: char| f.glyph_id(c).expect("the fixture maps P and R");
    let repeated = scene_of(&f, gid('R')).expect("eight layers of one glyph draw");
    let paths = repeated.paths().len();
    let fills = repeated.ops().iter().filter(|op| matches!(op, daegun::color::Op::Fill { .. })).count();
    assert_eq!((paths, fills), (1, 8), "eight layers of one glyph held {paths} outlines and {fills} fills");
    let past = f.color_scene(gid('P'), &Location::default(), &SceneOptions::default()).map_err(|e| e.kind());
    assert_eq!(past.err(), Some(daegun::ErrorKind::LimitExceeded), "a scene past 2^20 points was built");
}

// S's 255 layers share one gradient of 4,000 stops, a small table naming 1,020,000. A subset writes it
// once, where a copy for each layer would pass the paint budget's million stops.
#[test]
fn subsetting_a_shared_gradient_stays_within_the_stop_budget() {
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('S').expect("the fixture maps S");
    let colr = f.table(Tag::new(b"COLR")).expect("COLR").len();
    let sub = f.subset([gid], &Location::default()).expect("subsets");
    assert!(sub.font.len() < colr + (1 << 20), "a {colr}-byte COLR became a subset of {} bytes", sub.font.len());
}

// The bound is the lowering's own, not only the parser's: a graph built by hand, deeper than any font
// parses, still nests no more than 128.
#[test]
fn a_hand_built_paint_graph_nests_at_most_128_deep() {
    use daegun::color::Op;
    use daegun::color::PaintNode;
    // 70 composites, each the source of the one above it, over a solid backdrop.
    fn nest(paint: &mut daegun::color::PaintGraph, left: usize) {
        let solid = PaintNode::Solid { color: daegun::color::Rgba { r: 0, g: 0, b: 0, a: 255 }, foreground: false };
        paint.with(PaintNode::Composite { mode: daegun::color::Blend::Clear }, |p| {
            if left == 1 {
                p.with(solid, |_| {});
            } else {
                nest(p, left - 1);
            }
            p.with(solid, |_| {});
        });
    }
    let mut paint = daegun::color::PaintGraph::new();
    nest(&mut paint, 70);
    let mut scene = daegun::color::DisplayList::default();
    let black = daegun::color::Rgba { r: 0, g: 0, b: 0, a: 255 };
    daegun::color::lower(GlyphId(0), &paint, daegun::color::IDENTITY, &|_| None, &mut |_, _| false, black, &mut scene);
    let (mut depth, mut deepest) = (0usize, 0usize);
    for op in scene.ops() {
        match op {
            Op::PushClip { .. } | Op::PushLayer { .. } => depth += 1,
            Op::PopClip | Op::PopLayer => depth -= 1,
            Op::Fill { .. } => {}
            other => panic!("an op this test doesn't know: {other:?}"),
        }
        deepest = deepest.max(depth);
    }
    assert!(deepest <= 128, "70 composites built by hand nested {deepest} deep");
}

// colr-clip as the sanitizer passes it: D one composite shallower, 64 paints deep, and the 255 layers sharing S's
// gradient nulled, within its budget.
fn colr_clip_sanitized() -> Font {
    super::color_load::colr_clip(1, 0, |_, _, _| {})
}

// L's first layer is itself a layer list. A subset must keep its own layers ahead of the nested ones,
// or its record names the nested layers in their place.
#[test]
fn a_nested_layer_list_survives_a_subset() {
    use daegun::color::Op;
    let f = colr_clip_sanitized();
    let gid = f.glyph_id('L').expect("the fixture maps L");
    let fills = |f: &Font, gid| {
        let scene = scene_of(f, gid).expect("L has a scene");
        scene
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Fill { path, paint, .. } => {
                    Some((format!("{paint:?}"), scene.path(*path).map(|p| p.parts().1.to_vec())))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let subset = f.subset([gid], &Location::default()).expect("a subset");
    let new_gid = subset.renumbered(gid).expect("L survives");
    let small = Font::from_vec(subset.font, 0).expect("the subset parses");
    let want = fills(&f, gid);
    assert_eq!(want.len(), 3, "L fills inner, other and clip");
    assert_eq!(fills(&small, new_gid), want, "the subset's L fills other layers");
}

// A substitution naming a glyph past the font's end is ignored, as a requested one is, so the closure
// still ends once a pass adds nothing.
#[test]
fn a_substitute_past_the_end_does_not_stall_the_closure() {
    let f = font("test-fixtures/hinted.ttf");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = f.table_tags()
        .map(|t| (t, f.table(t).expect("listed").to_vec()))
        .collect();
    let gsub = [1u16, 0, 0, 0, 10, 1, 4, 1, 0, 1, 8, 2, 8, 1, 60_000, 1, 1, 1];
    tables.insert(Tag::new(b"GSUB"), gsub.iter().flat_map(|v| v.to_be_bytes()).collect());
    let font = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    assert!(font.glyph_closure([GlyphId(1)], &Location::default()).is_ok(), "the closure did not converge");
    assert!(font.subset([GlyphId(1)], &Location::default()).is_ok(), "the subset did not converge");
}

// A scene's paths and gradients walk in id order, each the one its id names, and stop where the ids do.
#[test]
fn a_scene_walks_its_paths_and_gradients_by_id() {
    use daegun::color::{GradientId, PathId};
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let (mut paths, mut gradients) = (0, 0);
    for gid in 0..f.glyph_count() {
        let Some(scene) = scene_of(&f, GlyphId(gid)) else { continue };
        for (i, p) in scene.paths().enumerate() {
            assert_eq!(scene.path(PathId(i)), Some(p), "glyph {gid}, path {i}");
        }
        for (i, g) in scene.gradients().enumerate() {
            assert_eq!(scene.gradient(GradientId(i)), Some(g), "glyph {gid}, gradient {i}");
        }
        assert!(scene.path(PathId(scene.paths().len())).is_none(), "glyph {gid}: a path past the walk");
        assert!(scene.gradient(GradientId(scene.gradients().len())).is_none(), "glyph {gid}: a gradient past it");
        (paths, gradients) = (paths + scene.paths().len(), gradients + scene.gradients().len());
    }
    assert!(paths > 0 && gradients > 0, "walked {paths} paths and {gradients} gradients");
}
