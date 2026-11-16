use daegun::color::{Blend, DisplayList, Op, PaintGraph, PaintNode, PaintRef, PaintVisitor, PathId, Rgba, SceneOptions};
use daegun::{Bounds, Font, GlyphId, Location, Tag};

use crate::located;

const STATIC: &str = "colr-v1-test-glyphs/test_glyphs.ttf";
const VARIABLE: &str = "colr-v1-test-glyphs/test_glyphs_variable.ttf";

fn bytes(rel: &str) -> Vec<u8> {
    let path = format!("{}/{}", crate::FONTS, rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

fn font(rel: &str) -> Font {
    Font::from_vec(bytes(rel), 0).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// The font with its `tag` table edited in place.
fn edited(rel: &str, tag: &[u8; 4], edit: impl FnOnce(&mut [u8])) -> Font {
    let mut b = bytes(rel);
    let n = usize::from(u16::from_be_bytes([b[4], b[5]]));
    let rec = (0..n).map(|i| 12 + i * 16).find(|&r| &b[r..r + 4] == tag).expect("the table");
    let at = |o: usize| u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (off, len) = (at(rec + 8), at(rec + 12));
    edit(&mut b[off..off + len]);
    Font::from_vec(b, 0).expect("still a font")
}

// A table's bytes in a font file.
fn table<'f>(font: &'f [u8], tag: &[u8; 4]) -> &'f [u8] {
    let n = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let rec = (0..n).map(|i| 12 + i * 16).find(|&r| &font[r..r + 4] == tag).expect("the table");
    &font[u32_at(font, rec + 8)..u32_at(font, rec + 8) + u32_at(font, rec + 12)]
}

fn u24_at(t: &[u8], o: usize) -> usize {
    (usize::from(t[o]) << 16) | (usize::from(t[o + 1]) << 8) | usize::from(t[o + 2])
}

fn u32_at(t: &[u8], o: usize) -> usize {
    u32::from_be_bytes(t[o..o + 4].try_into().unwrap()) as usize
}

// Where a base glyph's root paint starts in a COLR table.
fn root_paint(colr: &[u8], gid: u32) -> usize {
    let list = u32_at(colr, 14);
    let n = u32_at(colr, list);
    (0..n)
        .map(|i| list + 4 + i * 6)
        .find(|&r| u32::from(u16::from_be_bytes([colr[r], colr[r + 1]])) == gid)
        .map(|r| list + u32_at(colr, r + 2))
        .expect("a base glyph record")
}

// The glyph's paint graph at `axes` in palette 0.
fn graph(f: &Font, gid: u32, axes: &[(&str, f64)]) -> Option<PaintGraph> {
    f.paint_graph(GlyphId(gid), &located(f, axes), 0)
}

// The glyph's scene at `axes` in palette 0; None where it has none or it doesn't read.
fn scene(f: &Font, gid: GlyphId, axes: &[(&str, f64)]) -> Option<DisplayList> {
    f.color_scene(gid, &located(f, axes), &SceneOptions::default()).ok().flatten()
}

fn boxed([x_min, y_min, x_max, y_max]: [f64; 4]) -> Bounds {
    Bounds { x_min, y_min, x_max, y_max }
}

// The first paint `pick` takes, a paint before those below it.
fn first_of<'p>(paint: PaintRef<'p>, pick: &dyn Fn(&PaintNode) -> bool) -> Option<PaintNode<'p>> {
    let node = paint.node();
    if pick(&node) {
        return Some(node);
    }
    paint.children().find_map(|child| first_of(child, pick))
}

// A graph built by hand: a paint and the graphs below it, in order.
struct G(PaintNode<'static>, Vec<G>);

impl G {
    fn add_to(&self, paint: &mut PaintGraph) {
        paint.with(self.0, |p| self.1.iter().for_each(|below| below.add_to(p)));
    }

    fn built(&self) -> PaintGraph {
        let mut paint = PaintGraph::new();
        self.add_to(&mut paint);
        paint
    }
}

fn fills(scene: &DisplayList) -> usize {
    scene.ops().iter().filter(|o| matches!(o, Op::Fill { .. })).count()
}

// Options with a color build in a const, as the builders and the color's constructor are const.
const INK: SceneOptions = SceneOptions::DEFAULT.with_foreground(Rgba::opaque(20, 30, 40));

#[test]
fn scene_options_with_a_color_build_in_a_const() {
    assert_eq!(INK.foreground, Rgba { r: 20, g: 30, b: 40, a: 255 });
}

// Children borrow the graph alone, so the paint they came from can be a temporary.
#[test]
fn children_outlive_the_paint_they_came_from() {
    let g = graph(&font(VARIABLE), 103, &[]).expect("glyph 103 paints");
    let below = g.root().children();
    assert!(below.count() > 0);
}

// Drawn, varied radii, scales and stop offsets leave their stored type, which a clamp or a wrap to 16 bits would
// cut short: glyph 19's first stop, 0.25 moved by its 32-bit delta of 2, lands at 2.25.
#[test]
fn varied_values_may_leave_their_stored_range() {
    let f = font(VARIABLE);
    let g = graph(&f, 93, &[("GRR0", -1000.0)]).expect("radial");
    let radial = first_of(g.root(), &|p| matches!(p, PaintNode::RadialGradient { .. }));
    let Some(PaintNode::RadialGradient { r0, .. }) = radial else { panic!("no radial") };
    assert_eq!(r0, -1000.0);

    let max = 1.999_938_964_843_75;
    let g = graph(&f, 87, &[("SCSX", max), ("SCSY", max)]).expect("scale");
    let scale = first_of(g.root(), &|p| matches!(p, PaintNode::ScaleUniform { .. }));
    let Some(PaintNode::ScaleUniform { s, .. }) = scale else { panic!("no scale") };
    assert_eq!(s, 57_343.0 / 16_384.0);

    let g = graph(&f, 19, &[("SWC1", 2.0)]).expect("sweep");
    let sweep = first_of(g.root(), &|p| matches!(p, PaintNode::SweepGradient { .. }));
    let Some(PaintNode::SweepGradient { stops, .. }) = sweep else { panic!("no sweep") };
    assert_eq!(stops[0].offset, 2.25);
}

// Drawn, a delta is a whole unit, rounded half up: glyph 105's x skew moves 7,712.5 units at SKXA 84.732 and is
// drawn 7,713.
#[test]
fn a_drawn_delta_is_a_whole_unit() {
    let g = graph(&font(VARIABLE), 105, &[("SKXA", 84.732)]).expect("skew");
    let skew = first_of(g.root(), &|p| matches!(p, PaintNode::Skew { .. }));
    let Some(PaintNode::Skew { x_angle, .. }) = skew else { panic!("no skew") };
    assert_eq!(x_angle, 7713.0 / 16384.0 * 180.0);
}

// An index map past the table is absent, and a map named but absent leaves no way to the deltas, so nothing varies:
// glyphs draw and clip at a location as at the default.
#[test]
fn an_index_map_past_the_table_leaves_the_glyphs_unvaried() {
    let f = edited(VARIABLE, b"COLR", |colr| colr[26..30].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes()));
    let at = [("SWPS", 90.0)];
    let drawn = |at: &[(&str, f64)]| {
        (graph(&f, 19, at), scene(&f, GlyphId(19), at).is_some(), f.colr_clip_box(GlyphId(156), &located(&f, at)))
    };
    assert!(drawn(&at).0.is_some() && drawn(&at).1);
    assert_eq!(drawn(&at), drawn(&[]));
}

// An x skew leans by -tan of its angle: the spec's PaintSkew matrix has xy = -tan(φ).
#[test]
fn an_x_skew_leans_the_way_the_spec_says() {
    let f = font(STATIC);
    let scene = scene(&f, GlyphId(103), &[]).expect("skew_25_0");
    let skewed = scene.ops().iter().find_map(|o| match o {
        Op::Fill { transform, .. } if transform[2] != 0.0 => Some(*transform),
        _ => None,
    });
    let t = skewed.expect("a skewed fill");
    assert!((t[2] - -(25.0048828125f64.to_radians().tan())).abs() < 1e-6 && t[1] == 0.0, "{t:?}");
}

// A gradient under transforms inside a PaintGlyph fills that glyph; without it the nested-glyph
// conformance glyphs draw nothing at all.
#[test]
fn a_fill_under_transforms_fills_its_glyph() {
    let f = font(STATIC);
    for gid in 205..=220 {
        let scene = scene(&f, GlyphId(gid), &[]).unwrap_or_else(|| panic!("gid {gid} has no scene"));
        let gradient =
            scene.ops().iter().any(|o| matches!(o, Op::Fill { paint: daegun::color::Paint::Gradient(_), .. }));
        assert!(gradient, "gid {gid} lost its gradient");
    }
}

// The conformance glyphs whose stops share one offset: under reflect and repeat the color line is
// ill-formed and draws nothing, yet the glyph is still a color glyph; under pad it is a hard edge.
#[test]
fn a_color_line_at_one_offset_draws_nothing_unless_padded() {
    let f = font(STATIC);
    for gid in [194, 195, 197, 198, 200, 201, 203, 204] {
        let scene = scene(&f, GlyphId(gid), &[]).unwrap_or_else(|| panic!("gid {gid} is a color glyph"));
        assert_eq!(scene.ops(), [], "gid {gid}");
    }
    for gid in [193, 196, 199, 202] {
        let scene = scene(&f, GlyphId(gid), &[]).expect("a color glyph");
        let gradient =
            scene.ops().iter().any(|o| matches!(o, Op::Fill { paint: daegun::color::Paint::Gradient(_), .. }));
        assert!(gradient, "gid {gid} lost its gradient");
    }
}

// The clip boxes of the conformance glyphs, read and applied: each scene starts with its box as a clip, and a
// variable box's edges each move by their delta, rounded outward (COLR, "Clip boxes").
#[test]
fn a_clip_box_bounds_its_glyph() {
    let f = font(STATIC);
    let boxes = [
        (156, [0, 500, 500, 1000]),
        (157, [0, 0, 500, 500]),
        (158, [500, 0, 1000, 500]),
        (159, [500, 500, 1000, 1000]),
        (160, [250, 250, 750, 750]),
    ];
    for (gid, b) in boxes {
        assert_eq!(f.colr_clip_box(GlyphId(gid), &Location::default()), Some(boxed(b.map(f64::from))), "gid {gid}");
        let scene = scene(&f, GlyphId(gid), &[]).expect("a scene");
        let Some(&Op::PushClip { shapes }) = scene.ops().first() else { panic!("gid {gid} is not clipped") };
        let bounds = scene.path(scene.shapes(shapes)[0].path).and_then(|p| p.bounds()).expect("a box");
        assert_eq!(bounds, boxed(b.map(f64::from)), "gid {gid}");
    }
    assert_eq!(f.colr_clip_box(GlyphId(1), &Location::default()), None, "a glyph with no box");
    // 4095/16384 of each axis, exact in 2.14 and 16.16, leaves every edge 0.03 past a whole unit: each grows by 1.
    let v = font(VARIABLE);
    let q = 4095.0 / 16384.0 * 500.0;
    let at = [("CLXI", q), ("CLYI", q), ("CLXA", -q), ("CLYA", -q), ("CLIO", q)];
    assert_eq!(v.colr_clip_box(GlyphId(160), &located(&v, &at)), Some(boxed([374.0, 374.0, 626.0, 626.0])));
    assert_eq!(v.colr_clip_box(GlyphId(166), &located(&v, &at)), Some(boxed([224.0, 224.0, 776.0, 776.0])));
}

// A fill outside every PaintGlyph fills the clip box, or with none, a rectangle around the glyph's
// bounded content; a glyph the spec calls unbounded and gives no box is not drawn.
#[test]
fn a_fill_outside_every_glyph_is_bounded_as_the_spec_says() {
    let mut square = |_, p: &mut dyn daegun::outline::OutlinePen| {
        daegun::outline::OutlinePen::move_to(p, 100.0, 100.0);
        for (x, y) in [(300.0, 100.0), (300.0, 300.0), (100.0, 300.0)] {
            daegun::outline::OutlinePen::line_to(p, x, y);
        }
        daegun::outline::OutlinePen::close(p);
        true
    };
    let solid = |a| G(PaintNode::Solid { color: Rgba { r: 0, g: 0, b: 255, a }, foreground: false }, vec![]);
    let shape = || G(PaintNode::Glyph { glyph: GlyphId(1) }, vec![solid(255)]);
    let mut lower = |paint: &PaintGraph, clip: Option<Bounds>| {
        let mut scene = DisplayList::default();
        let drawn = daegun::color::lower(
            GlyphId(0),
            paint,
            daegun::color::IDENTITY,
            &|_| clip,
            &mut square,
            daegun::color::Rgba::default(),
            &mut scene,
        );
        (drawn, scene)
    };
    let rect_of = |scene: &DisplayList, alpha: u8| {
        scene.ops().iter().find_map(|o| match o {
            Op::Fill { path, paint: daegun::color::Paint::Solid(c), transform, .. } if c.a == alpha => {
                let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } = scene.path(*path)?.bounds()?;
                Some([
                    x0 * transform[0] + transform[4],
                    y0 * transform[3] + transform[5],
                    x1 * transform[0] + transform[4],
                    y1 * transform[3] + transform[5],
                ])
            }
            _ => None,
        })
    };

    // SrcIn keeps the source where the backdrop is: a group at half opacity, Noto's idiom.
    let group = G(PaintNode::Composite { mode: Blend::SrcIn }, vec![shape(), solid(128)]).built();
    let (drawn, scene) = lower(&group, None);
    assert!(drawn);
    assert_eq!(rect_of(&scene, 128), Some([100.0, 100.0, 300.0, 300.0]), "the backdrop has no fill around the shape");
    let (_, scene) = lower(&group, Some(boxed([0.0, 0.0, 1000.0, 1000.0])));
    assert_eq!(rect_of(&scene, 128), Some([0.0, 0.0, 1000.0, 1000.0]), "the backdrop does not fill the clip box");

    let unbounded = G(PaintNode::Layers, vec![shape(), solid(255)]).built();
    let (drawn, scene) = lower(&unbounded, None);
    assert!(!drawn && scene.ops().is_empty(), "an unbounded glyph was drawn");
    assert!(lower(&unbounded, Some(boxed([0.0, 0.0, 500.0, 500.0]))).0, "a clip box bounds it");
}

// An empty backdrop under SrcIn leaves nothing to draw, under SrcOver the source as drawn, and an
// empty source under Xor the backdrop as drawn: none opens a layer.
#[test]
fn composites_showing_nothing_or_their_backdrop_open_no_layers() {
    let mut square = |_, p: &mut dyn daegun::outline::OutlinePen| {
        daegun::outline::OutlinePen::move_to(p, 0.0, 0.0);
        daegun::outline::OutlinePen::line_to(p, 100.0, 0.0);
        daegun::outline::OutlinePen::line_to(p, 0.0, 100.0);
        daegun::outline::OutlinePen::close(p);
        true
    };
    let solid = PaintNode::Solid { color: Rgba { r: 0, g: 0, b: 255, a: 255 }, foreground: false };
    let shape = || G(PaintNode::Glyph { glyph: GlyphId(1) }, vec![G(solid, vec![])]);
    let empty = || G(PaintNode::Layers, vec![]);
    let mut lower = |paint: PaintGraph| {
        let mut scene = DisplayList::default();
        let drawn = daegun::color::lower(
            GlyphId(0),
            &paint,
            daegun::color::IDENTITY,
            &|_| None,
            &mut square,
            daegun::color::Rgba::default(),
            &mut scene,
        );
        assert!(drawn);
        scene
    };

    let composite = |mode, below| G(PaintNode::Composite { mode }, below).built();
    let nothing = lower(composite(Blend::SrcIn, vec![shape(), empty()]));
    assert!(nothing.ops().is_empty(), "SrcIn over nothing drew {:?}", nothing.ops());
    let source = lower(composite(Blend::SrcOver, vec![shape(), empty()]));
    assert!(matches!(source.ops(), [Op::Fill { .. }]), "SrcOver over nothing drew {:?}", source.ops());
    let backdrop = lower(composite(Blend::Xor, vec![empty(), shape()]));
    assert!(matches!(backdrop.ops(), [Op::Fill { .. }]), "Xor of nothing drew {:?}", backdrop.ops());
}

// A fill reached through layers inside a PaintGlyph fills that glyph too, not only one under
// transforms alone, so the clip it fills stays too.
#[test]
fn a_fill_through_layers_fills_its_glyph() {
    let mut square = |_, p: &mut dyn daegun::outline::OutlinePen| {
        daegun::outline::OutlinePen::move_to(p, 0.0, 0.0);
        daegun::outline::OutlinePen::line_to(p, 100.0, 0.0);
        daegun::outline::OutlinePen::line_to(p, 0.0, 100.0);
        daegun::outline::OutlinePen::close(p);
        true
    };
    let solid = G(PaintNode::Solid { color: Rgba { r: 9, g: 9, b: 9, a: 255 }, foreground: false }, vec![]);
    let translated = G(PaintNode::Translate { dx: 5.0, dy: 0.0 }, vec![solid]);
    let graph = G(PaintNode::Glyph { glyph: GlyphId(1) }, vec![G(PaintNode::Layers, vec![translated])]).built();
    let mut scene = DisplayList::default();
    let drawn = daegun::color::lower(
        GlyphId(0), &graph, daegun::color::IDENTITY, &|_| None, &mut square, daegun::color::Rgba::default(), &mut scene,
    );
    assert!(drawn);
    assert_eq!(fills(&scene), 1, "{:?}", scene.ops());
}

// The spec: "If an unrecognized value is encountered, COMPOSITE_CLEAR must be used."
#[test]
fn an_unknown_composite_mode_clears() {
    assert_eq!(daegun::color::Blend::from_colr(200), daegun::color::Blend::Clear);
}

// A layer pointing past the table is absent and the rest of its run drawn: the last of five or the first, four
// layers either way. The cycle glyphs keep what precedes their cycle.
#[test]
fn a_layer_past_the_table_is_absent_and_the_rest_drawn() {
    let whole = fills(&scene(&font(STATIC), GlyphId(180), &[]).expect("no_cycle_multi_colrglyph"));
    let broken = |last: bool| {
        edited(STATIC, b"COLR", |colr| {
            let root = root_paint(colr, 180);
            assert_eq!((colr[root], colr[root + 1]), (1, 5), "a PaintColrLayers of five");
            let entry = u32_at(colr, 18) + 4 + (u32_at(colr, root + 2) + if last { 4 } else { 0 }) * 4;
            colr[entry..entry + 4].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        })
    };
    let drawn = |last| fills(&scene(&broken(last), GlyphId(180), &[]).expect("a color glyph"));
    assert_eq!((drawn(true) * 5, drawn(false) * 5), (whole * 4, whole * 4), "of {whole}");

    let cycle = graph(&font(STATIC), 178, &[]).expect("the cycle glyph parses");
    let inner = G(PaintNode::ColrGlyph { glyph: GlyphId(178) }, vec![G(PaintNode::Layers, vec![])]);
    let want = G(PaintNode::ColrGlyph { glyph: GlyphId(179) }, vec![inner]);
    assert_eq!(cycle, want.built());
}

// A subset keeps its glyphs' clip boxes under their new IDs, and a COLR with a part past its end, that part absent.
#[test]
fn a_subset_keeps_clip_boxes_and_readable_glyphs() {
    let f = font(STATIC);
    let sub = f
        .subset([GlyphId(156), GlyphId(157), GlyphId(158), GlyphId(159), GlyphId(160)], &Location::default())
        .expect("a subset");
    let s = Font::from_vec(sub.font.clone(), 0).expect("the subset reads");
    for gid in 156..=160 {
        let new = sub.renumbered(GlyphId(gid)).expect("kept");
        assert_eq!(
            s.colr_clip_box(new, &Location::default()),
            f.colr_clip_box(GlyphId(gid), &Location::default()),
            "gid {gid}"
        );
        let drawn = |f: &Font, gid| scene(f, gid, &[]).map(|sc| fills(&sc));
        assert_eq!(drawn(&s, new), drawn(&f, GlyphId(gid)), "gid {gid}");
    }

    let broken = edited(STATIC, b"COLR", |colr| {
        let list = u32_at(colr, 14);
        let rec = (0..u32_at(colr, list))
            .map(|i| list + 4 + i * 6)
            .find(|&r| colr[r..r + 2] == 8u16.to_be_bytes())
            .expect("gid 8");
        colr[rec + 2..rec + 6].copy_from_slice(&0x00FF_FFF0u32.to_be_bytes());
    });
    // gid 8's root lies past the table, absent, and 9 is drawn; a subset keeps COLR and draws 9 as the font does.
    assert_eq!([8, 9].map(|g| graph(&broken, g, &[]).is_some()), [false, true]);
    let sub = broken.subset([GlyphId(8), GlyphId(9)], &Location::default()).expect("a subset");
    let s = Font::from_vec(sub.font.clone(), 0).expect("the subset reads");
    let nine = sub.renumbered(GlyphId(9)).expect("kept");
    assert!(graph(&s, nine.0, &[]).is_some(), "the subset draws gid 9");

    // A variable font subset at its default location keeps no fvar, so it is the default master, and its COLR
    // names no variation data to go with axes it does not have.
    let sub = font(VARIABLE).subset([GlyphId(99), GlyphId(160)], &Location::default()).expect("a subset");
    assert_eq!(&table(&sub.font, b"COLR")[26..34], &[0; 8]);

    // colr-clip's glyph 6 nests 65 paints and draws the first 64, its 65th level unread; a subset writes that level
    // absent, so it keeps COLR and draws glyph 6 as the font does.
    let deep = font("test-fixtures/colr-clip.ttf");
    let sub = deep.subset([GlyphId(6)], &Location::default()).expect("a subset");
    let s = Font::from_vec(sub.font.clone(), 0).expect("the subset reads");
    let six = sub.renumbered(GlyphId(6)).expect("kept");
    let drawn = |f: &Font, gid| scene(f, gid, &[]).map(|sc| fills(&sc));
    assert!(drawn(&deep, GlyphId(6)).is_some_and(|n| n > 0), "glyph 6 drawn");
    assert_eq!(drawn(&s, six), drawn(&deep, GlyphId(6)), "the subset draws glyph 6 as the font does");
}

// Subsets by glyph ids keep COLR and CPAL, each pinned by its FNV-1a 64: palette indices by rank, the palettes left
// alike sharing their records, CPAL's arrays written in field order.
#[test]
fn a_subset_s_color_tables_match_their_pinned_hashes() {
    let fnv = |t: &[u8]| {
        t.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
    };
    let every: Vec<u32> = (1..221).collect();
    let cases: [(&[u32], u64, u64); 5] = [
        (&[10], 0x7598_e963_0880_a1eb, 0x6951_1089_f3b0_79bc),
        (&[10, 11, 12, 13], 0x4ddc_a714_9a7d_8573, 0x3401_7089_e669_07bf),
        (&[60, 61, 62], 0x2857_42ca_c38e_3a37, 0x3401_7089_e669_07bf),
        (&[100, 150, 200], 0x0b71_070c_85df_811f, 0x612d_5dde_a6ce_ec94),
        (&every, 0x15ad_2e7d_d3ee_f560, 0x8281_6e00_ba55_766b),
    ];
    let f = font(STATIC);
    for (gids, colr, cpal) in cases {
        let glyphs: Vec<GlyphId> = gids.iter().copied().map(GlyphId).collect();
        let sub = f.subset(&glyphs, &Location::default()).expect("a subset");
        assert_eq!([fnv(table(&sub.font, b"COLR")), fnv(table(&sub.font, b"CPAL"))], [colr, cpal], "{gids:?}");
    }
}

// An instance draws what the variable font draws there, its out-of-range values written another way: each delta is
// worked in 16.16 and rounded once into its field, so at SKXA 60 glyph 103's skew lands on the same 2.14 step.
#[test]
fn an_instance_draws_what_the_variable_font_draws_there() {
    let f = font(VARIABLE);
    let at = [("SKXA", 60.0)];
    let skew = |paint: Option<PaintGraph>| {
        let paint = paint.expect("glyph 103");
        assert!(matches!(paint.root().node(), PaintNode::Composite { .. }), "glyph 103: {paint:?}");
        match paint.root().children().next().map(|source| source.node()) {
            Some(PaintNode::Skew { x_angle, .. }) => x_angle,
            other => panic!("glyph 103's source: {other:?}"),
        }
    };
    let inst = Font::from_bytes(&f.instance(&located(&f, &at)).expect("instanced")).expect("an instance");
    assert_eq!(skew(graph(&inst, 103, &[])), skew(graph(&f, 103, &at)));
    // A location, and the glyphs to compare there (every one when empty).
    type At<'a> = (&'a [(&'a str, f64)], &'a [u32]);
    let locations: [At; 6] = [
        (
            &[
                ("SCSX", 1.99), ("SCSY", 1.99), ("GRR0", -1000.0), ("COL1", 1.99), ("ROTA", 539.0), ("SKXA", 67.5),
                ("CLXI", 125.0), ("APH1", -0.5),
            ],
            &[],
        ),
        (
            &[
                ("SCSX", -2.0), ("SCOX", 200.0), ("GRR1", -1000.0), ("COL2", -2.0), ("SWC1", 1.99), ("TRDX", 0.4),
                ("CLIO", -500.0), ("SKYA", -60.0),
            ],
            &[],
        ),
        (
            &[
                ("SWC1", -1.0), ("SWC4", 1.0), ("SWPE", 45.0), ("GRR0", 1000.0), ("GRR1", -1000.0), ("COL3", 1.99),
                ("TRXX", 0.4), ("ROTX", 500.0),
            ],
            &[],
        ),
        // Glyph 59's padded sweep spans 2,986 degrees here, past a static table, but only 1,102 of
        // them once the stops never seen are left out.
        (&[("SWPS", -90.0), ("SWPE", -90.0), ("SWC1", -2.0), ("SWC4", -2.0), ("SWC2", 1.99)], &[59]),
        // These sweeps' stops reach past 1,260 degrees, held only with the angles running backward,
        // and these only with them running forward from -180.
        (&[("SWPS", -90.0), ("SWPE", -90.0), ("SWC1", 1.99), ("SWC4", 1.99)], &[35, 47, 71, 83]),
        (
            &[("SWPS", -90.0), ("SWPE", -90.0), ("SWC1", -2.0), ("SWC2", -2.0), ("SWC3", -2.0), ("SWC4", -2.0)],
            &[71, 83],
        ),
    ];
    for (at, only) in locations {
        let bytes = f.instance(&located(&f, at)).expect("instanced");
        let colr = table(&bytes, b"COLR");
        assert_eq!(&colr[26..34], &[0; 8], "the index map or store is still named");
        let mut stack: Vec<usize> = (0..u32_at(colr, u32_at(colr, 14))).map(|i| {
            let list = u32_at(colr, 14);
            list + u32_at(colr, list + 4 + i * 6 + 2)
        }).collect();
        while let Some(off) = stack.pop() {
            let format = colr[off];
            assert!(
                !matches!(format, 3 | 5 | 7 | 9 | 13 | 15 | 17 | 19 | 21 | 23 | 25 | 27 | 29 | 31),
                "a Var paint (format {format}) is left"
            );
            match format {
                1 => {
                    let list = u32_at(colr, 18);
                    stack.extend(
                        (0..usize::from(colr[off + 1]))
                            .map(|i| list + u32_at(colr, list + 4 + (u32_at(colr, off + 2) + i) * 4)),
                    );
                }
                10 | 12 | 14 | 16 | 18 | 20 | 22 | 24 | 26 | 28 | 30 => stack.push(off + u24_at(colr, off + 1)),
                32 => stack.extend([off + u24_at(colr, off + 1), off + u24_at(colr, off + 5)]),
                _ => {}
            }
        }
        let inst = Font::from_bytes(&bytes).expect("an instance");
        let glyphs: Vec<u32> = if only.is_empty() { (0..f.glyph_count()).collect() } else { only.to_vec() };
        for gid in glyphs {
            assert_eq!(
                inst.colr_clip_box(GlyphId(gid), &Location::default()),
                f.colr_clip_box(GlyphId(gid), &located(&f, at)),
                "gid {gid} at {at:?}"
            );
            let (a, b) = (scene(&f, GlyphId(gid), at), scene(&inst, GlyphId(gid), &[]));
            if let Err(why) = same_scene(a.as_ref(), b.as_ref()) {
                panic!("gid {gid} at {at:?}: {why}");
            }
        }
    }
}

fn same_scene(a: Option<&DisplayList>, b: Option<&DisplayList>) -> Result<(), String> {
    let (a, b) = match (a, b) {
        (None, None) => return Ok(()),
        (Some(a), Some(b)) => (a, b),
        _ => return Err(format!("one scene only: {} and {}", a.is_some(), b.is_some())),
    };
    if a.ops().len() != b.ops().len() {
        return Err(format!("{} ops against {}", a.ops().len(), b.ops().len()));
    }
    let near = |x: f64, y: f64, tol: f64| (x - y).abs() <= tol;
    // Angles held to 1/16384 of a half turn: a steep skew's tangent, and the offset its center gives,
    // move by a part in a thousand.
    let matrix = |m: &[f64; 6], n: &[f64; 6]| {
        (0..4).all(|i| near(m[i], n[i], 2e-3 * m[i].abs().max(1.0)))
            && (4..6).all(|i| near(m[i], n[i], (2e-3 * m[i].abs()).max(1.0)))
    };
    let path = |p: PathId, q: PathId| a.path(p).map(|p| p.parts().1) == b.path(q).map(|q| q.parts().1);
    for (i, (x, y)) in a.ops().iter().zip(b.ops()).enumerate() {
        let same = match (x, y) {
            (
                Op::Fill { path: p, paint: pa, rule: r, transform: t },
                Op::Fill { path: q, paint: pb, rule: s, transform: u },
            ) => r == s && matrix(t, u) && path(*p, *q) && same_paint((a, pa), (b, pb)),
            (Op::PushClip { shapes: s }, Op::PushClip { shapes: t }) => {
                let (s, t) = (a.shapes(*s), b.shapes(*t));
                s.len() == t.len()
                    && s.iter()
                        .zip(t)
                        .all(|(s, t)| s.rule == t.rule && matrix(&s.transform, &t.transform) && path(s.path, t.path))
            }
            (Op::PushLayer { opacity: o, blend: l }, Op::PushLayer { opacity: p, blend: m }) => o == p && l == m,
            (Op::PopClip, Op::PopClip) | (Op::PopLayer, Op::PopLayer) => true,
            _ => false,
        };
        if !same {
            return Err(format!("op {i}: {x:?} against {y:?}"));
        }
    }
    Ok(())
}

fn same_paint((sa, a): (&DisplayList, &daegun::color::Paint), (sb, b): (&DisplayList, &daegun::color::Paint)) -> bool {
    use daegun::color::Paint::{Gradient, Solid};
    let color = |c: &daegun::color::Rgba, d: &daegun::color::Rgba| {
        [(c.r, d.r), (c.g, d.g), (c.b, d.b), (c.a, d.a)].iter().all(|&(x, y)| x.abs_diff(y) <= 1)
    };
    match (a, b) {
        (Solid(c), Solid(d)) => color(c, d),
        // Compared by what they paint on a grid: angles and stops held to 1/16384 shift colors a
        // fraction of a unit, so each sample need only lie within the other's range 2 units around.
        (Gradient(g), Gradient(h)) => {
            let (Some(g), Some(h)) = (sa.gradient(*g), sb.gradient(*h)) else { return false };
            let ramp = |g| daegun::color::gradient::Ramp::new(&g, &daegun::color::IDENTITY);
            let (ra, rb) = (ramp(g), ramp(h));
            let channels = |c: daegun::color::Rgba| [c.r, c.g, c.b, c.a];
            let steps = [-2.0, -1.0, 0.0, 1.0, 2.0];
            (0..17).all(|i| {
                (0..17).all(|j| {
                    let (x, y) = (f64::from(i) * 80.0 - 160.0 + 0.37, f64::from(j) * 80.0 - 160.0 + 0.61);
                    let Some(c) = ra.at(x, y) else { return rb.at(x, y).is_none() };
                    let around: Vec<[u8; 4]> = steps
                        .iter()
                        .flat_map(|dx| steps.map(|dy| rb.at(x + dx, y + dy)))
                        .flatten()
                        .map(channels)
                        .collect();
                    (0..4).all(|k| {
                        let lo = around.iter().map(|v| v[k]).min().unwrap_or(0);
                        let hi = around.iter().map(|v| v[k]).max().unwrap_or(0);
                        (lo.saturating_sub(3)..=hi.saturating_add(3)).contains(&channels(c)[k])
                    })
                })
            })
        }
        _ => false,
    }
}

// test_glyphs with glyph 166's clip box shrunk to 400..600. Glyph 160 reaches 166 through a PaintColrGlyph, and a
// reused glyph keeps the bounds its box gives it, so the radial fill is drawn inside that box, not across 160's.
#[test]
fn a_referenced_color_glyph_is_clipped_to_its_clip_box() {
    let f = font(STATIC);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let colr = tables.get_mut(&Tag::new(b"COLR")).expect("test_glyphs has COLR");
    let list = u32::from_be_bytes(colr[22..26].try_into().unwrap()) as usize;
    let count = u32::from_be_bytes(colr[list + 1..list + 5].try_into().unwrap()) as usize;
    let record = (0..count).map(|i| list + 5 + 7 * i).find(|&r| colr[r..r + 4] == [0, 166, 0, 166]);
    let record = record.expect("166's clip");
    let at = list + usize::from(u16::from_be_bytes([colr[record + 5], colr[record + 6]]))
        + (usize::from(colr[record + 4]) << 16);
    for (i, v) in [400i16, 400, 600, 600].into_iter().enumerate() {
        colr[at + 1 + 2 * i..at + 3 + 2 * i].copy_from_slice(&v.to_be_bytes());
    }
    let g = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads");
    let clip = boxed([400.0, 400.0, 600.0, 600.0]);
    assert_eq!(g.colr_clip_box(GlyphId(166), &Location::default()), Some(clip));

    let scene = scene(&g, GlyphId(160), &[]).expect("a scene");
    let mut clips: Vec<Bounds> = Vec::new();
    let mut gradients = 0;
    for op in scene.ops() {
        match op {
            Op::PushClip { shapes } => {
                clips.push(scene.path(scene.shapes(*shapes)[0].path).and_then(|p| p.bounds()).expect("a box"));
            }
            Op::PopClip => drop(clips.pop()),
            Op::Fill { paint: daegun::color::Paint::Gradient(_), .. } => {
                gradients += 1;
                assert!(clips.contains(&clip), "drawn under {clips:?}");
            }
            _ => {}
        }
    }
    assert_eq!(gradients, 1, "{:?}", scene.ops());
}

// A COLR v1 whose base glyph f fills f with `fill`, no clip box, and one palette of one color.
fn filling_f(fill: &[u8]) -> Vec<u8> {
    use super::aat_subset::{u16s, u32s, F};
    let mut t = u16s(&[1, 0]);
    t.extend(u32s(&[0, 0]));
    t.extend(u16s(&[0]));
    t.extend(u32s(&[34, 0, 0, 0, 0]));
    t.extend(u32s(&[1]));
    t.extend(u16s(&[F]));
    t.extend(u32s(&[10]));
    t.extend([10, 0, 0, 6]);
    t.extend(u16s(&[F]));
    t.extend(fill);
    t
}

// A linear gradient through `points` over a color line of `extend` and `stops` (offset, palette entry, alpha).
fn linear(points: [u16; 6], extend: u8, stops: &[[u16; 3]]) -> Vec<u8> {
    use super::aat_subset::u16s;
    let mut t = vec![4, 0, 0, 16];
    t.extend(u16s(&points));
    t.push(extend);
    t.extend(u16s(&[stops.len() as u16]));
    t.extend(stops.iter().flat_map(|s| u16s(s)));
    t
}

// Extents count every fill as covering its clip, even these that draw nothing: no stops, stops at one offset, ends
// that meet, a color past the palette, no palette table.
#[test]
fn a_fill_that_draws_nothing_still_covers_its_clip_in_extents() {
    use super::aat_subset::{carets_with, u16s, F};
    let cpal = [&u16s(&[0, 1, 1, 1])[..], &14u32.to_be_bytes(), &u16s(&[0]), &[0, 0, 0xFF, 0xFF]].concat();
    let across = [0, 0, 100, 0, 0, 100];
    let fills = [
        ("a solid", [&[2u8][..], &u16s(&[0, 0x4000])].concat()),
        ("no stops", linear(across, 0, &[])),
        ("one offset repeated", linear(across, 1, &[[0x2000, 0, 0x4000], [0x2000, 0, 0x4000]])),
        ("ends that meet", linear([0, 0, 0, 0, 0, 100], 0, &[[0, 0, 0x4000], [0x4000, 0, 0x4000]])),
        ("past the palette", [&[2u8][..], &u16s(&[7, 0x4000])].concat()),
    ];
    let origin = |f: Font| f.vertical_origin(GlyphId(F.into()), &Location::default());
    let mut origins: Vec<(&str, f64)> = fills.iter().map(|(what, fill)| {
        (*what, origin(carets_with(&[("COLR", filling_f(fill)), ("CPAL", cpal.clone())])))
    }).collect();
    let no_palette = origin(carets_with(&[("COLR", filling_f(&fills[0].1))]));
    origins.push(("no palette table", no_palette));
    let all = ["a solid", "no stops", "one offset repeated", "ends that meet", "past the palette", "no palette table"];
    assert_eq!(origins, all.map(|what| (what, 850.0)));
}

// An empty layer covers nothing, wherever it is moved: F's box (20–320, 0–700) alone sets the origin beside glyph 0,
// at (-30, -40) too, and beside glyph 6, whose range runs past glyf's end and so holds nothing, drawn or measured.
#[test]
fn a_layer_that_draws_nothing_adds_nothing_to_extents() {
    use super::aat_subset::{carets_with, u16s, F};
    let cpal = [u16s(&[0, 1, 1, 1, 0, 14, 0]), vec![0, 0, 0xFF, 0xFF]].concat();
    let v0 = |second| [u16s(&[0, 1, 0, 14, 0, 20, 2]), u16s(&[F, 0, 2]), u16s(&[F, 0, second, 0])].concat();
    let glyph = |gid: u16| [vec![10, 0, 0, 6], u16s(&[gid]), vec![2], u16s(&[0, 0x4000])].concat();
    let v1 = |second: Vec<u8>| {
        let mut t = u16s(&[1, 0, 0, 0, 0, 0, 0, 0, 34, 0, 50, 0, 0, 0, 0, 0, 0]);
        t.extend([u16s(&[0, 1, F, 0, 10]), vec![1, 2, 0, 0, 0, 0], u16s(&[0, 2, 0, 12, 0, 23])].concat());
        [t, glyph(F), second].concat()
    };
    let moved = [vec![14, 0, 0, 8], u16s(&[(-30i16) as u16, (-40i16) as u16]), glyph(0)].concat();
    let origin = |colr| {
        carets_with(&[("COLR", colr), ("CPAL", cpal.clone())]).vertical_origin(GlyphId(F.into()), &Location::default())
    };
    let past = u16s(&[0, 0, 13, 25, 37, 50, 63, 126]);
    let cut = carets_with(&[("COLR", v0(6)), ("CPAL", cpal.clone()), ("loca", past)]);
    let cut = (
        cut.vertical_origin(GlyphId(F.into()), &Location::default()),
        cut.glyph_bounds(GlyphId(6), &Location::default()),
    );
    assert_eq!([origin(v0(0)), origin(v1(glyph(0))), origin(v1(moved)), cut.0], [850.0; 4]);
    assert_eq!(cut.1, None, "glyph 6 holds nothing");
}

// What a visitor is handed, in order: each paint entered, as it prints, and each left, as None.
#[derive(Default)]
struct Recorded(Vec<Option<String>>);

impl PaintVisitor for Recorded {
    fn enter(&mut self, node: PaintNode<'_>) {
        self.0.push(Some(format!("{node:?}")));
    }

    fn leave(&mut self) {
        self.0.push(None);
    }
}

// A graph handed to `to` as a visitor would be, read back through the graph's view.
fn replay(paint: PaintRef, to: &mut Recorded) {
    to.enter(paint.node());
    paint.children().for_each(|child| replay(child, to));
    to.leave();
}

// Every glyph of both test fonts, at the default and every axis at its maximum, under two palettes: `paint_graph`'s
// graph reads back paint for paint as the visitor is handed it, the cycle glyphs' unread paints as empty layers.
#[test]
fn the_graph_reads_back_as_the_visitor_walks_it() {
    for rel in [STATIC, VARIABLE] {
        let f = font(rel);
        let at_max: Vec<(Tag, f64)> = f.axes().iter().map(|a| (a.tag, a.max)).collect();
        let mut painted = 0;
        for location in [Location::default(), f.location(&at_max)] {
            for (gid, palette) in (0..f.glyph_count()).flat_map(|g| [(GlyphId(g), 0), (GlyphId(g), 1)]) {
                let (mut visited, mut read) = (Recorded::default(), Recorded::default());
                let walked = f.visit_paint_graph(gid, &location, palette, &mut visited);
                let graph = f.paint_graph(gid, &location, palette);
                if let Some(graph) = &graph {
                    replay(graph.root(), &mut read);
                }
                assert_eq!((walked, &visited.0), (graph.is_some(), &read.0), "{rel} {gid} {palette}");
                painted += usize::from(walked);
            }
        }
        assert!(painted > 700, "{rel}: {painted} painted");
    }
}
