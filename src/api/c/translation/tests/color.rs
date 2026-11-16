use super::*;
use alloc::string::String;
use crate::{Location, NameId};
use crate::ffi::testing::{CFont, asset};

#[test]
fn every_interpolation_has_a_code() {
    for v in [Interpolation::LinearLight, Interpolation::Srgb] {
        let code = match v {
            Interpolation::LinearLight => INTERPOLATE_LINEAR_LIGHT,
            Interpolation::Srgb => INTERPOLATE_SRGB,
        };
        assert_eq!(interpolation_of(code), Ok(v));
    }
    assert!(interpolation_of(2).is_err());
}

// Ops and composite nodes hand C a mode's discriminant, which must be its COLR number.
#[test]
fn blend_modes_are_numbered_as_colr_numbers_them() {
    for code in 0..=27 {
        let mode = blend_mode(code).unwrap();
        assert_eq!((mode, mode as i32), (Blend::from_colr(code as u8), code));
    }
    assert!(blend_mode(28).is_err() && blend_mode(-1).is_err());
}

#[test]
fn the_default_scene_options_are_rusts() {
    let c = SceneOptionsC::defaults();
    let SceneOptions { palette, foreground } = SceneOptions::default();
    assert_eq!((c.size, c.palette, c.foreground), (size_of::<SceneOptionsC>(), palette, foreground));
}

// Every scene of the COLR test font, built through C with a palette and foreground of its own, against the display
// list Rust builds: each op, clip shape, gradient and stop where C's layout puts it, and each ramp as Rust's.
#[test]
fn a_scene_reaches_c_as_its_display_list_holds_it() {
    let c = CFont::of(asset("colr-v1-test-glyphs/test_glyphs.ttf"));
    let font = unsafe { &*c.handle };
    let red = Rgba { r: 255, g: 0, b: 0, a: 255 };
    let opts = SceneOptionsC { size: size_of::<SceneOptionsC>(), foreground: red, palette: 2 };
    let options = SceneOptions { palette: 2, foreground: red };
    let to_device = [0.05, 0.0, 0.0, -0.05, 10.0, 90.0];
    let (mut kinds, mut ops, mut foreground) = ([false; 3], [false; 4], false);
    // Miri reads every 4th glyph, which reaches each call; that the font covers every part is read without it.
    for gid in (0..font.glyph_count()).step_by(if cfg!(miri) { 4 } else { 1 }) {
        let Some(list) = font.color_scene(GlyphId(gid), &Location::default(), &options).unwrap() else { continue };
        let mut handle = core::ptr::null_mut();
        let st = unsafe { daegun_font_color_scene(c.handle, gid, core::ptr::null(), &opts, &mut handle) };
        assert_eq!(st, Status::Ok, "glyph {gid}");
        let scene = unsafe { &*handle };
        assert_eq!(list.paths().len(), scene.paths);

        assert_eq!(scene.ops.len(), list.ops().len(), "glyph {gid}");
        for (op, c) in list.ops().iter().zip(&scene.ops) {
            match *op {
                Op::Fill { path, paint, rule, transform } => {
                    assert_eq!((c.kind, c.path, c.rule, c.transform), (OP_FILL, path.0, rule_code(rule), transform));
                    match paint {
                        Paint::Solid(color) => assert_eq!((c.color, c.has_gradient), (color, false)),
                        Paint::Gradient(id) => assert_eq!((c.gradient, c.has_gradient), (id.0, true)),
                    }
                    ops[usize::from(c.has_gradient)] = true;
                    foreground |= paint == Paint::Solid(red);
                }
                Op::PushClip { shapes } => {
                    assert_eq!(c.kind, OP_PUSH_CLIP);
                    let (want, got) = (list.shapes(shapes), &scene.shapes[c.shapes_start..c.shapes_end]);
                    assert_eq!(got.len(), want.len());
                    for (g, w) in got.iter().zip(want) {
                        assert_eq!((g.path, g.rule, g.transform), (w.path.0, rule_code(w.rule), w.transform));
                    }
                    ops[2] = true;
                }
                Op::PopClip => assert_eq!(c.kind, OP_POP_CLIP),
                Op::PushLayer { opacity, blend } => {
                    assert_eq!((c.kind, c.opacity, c.blend), (OP_PUSH_LAYER, opacity, blend as i32));
                    ops[3] |= blend != Blend::SrcOver;
                }
                Op::PopLayer => assert_eq!(c.kind, OP_POP_LAYER),
            }
        }

        assert_eq!(list.gradients().len(), scene.gradients.len());
        for (id, (g, c)) in list.gradients().zip(&scene.gradients).enumerate() {
            let (kind, values) = match g.kind {
                GradientKind::Linear { x0, y0, x1, y1 } => (GRADIENT_LINEAR, [x0, y0, x1, y1, 0.0, 0.0]),
                GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => (GRADIENT_RADIAL, [x0, y0, r0, x1, y1, r1]),
                GradientKind::Sweep { cx, cy, start_angle, end_angle } => {
                    (GRADIENT_SWEEP, [cx, cy, start_angle, end_angle, 0.0, 0.0])
                }
            };
            assert_eq!((c.kind, c.values, c.extend, c.transform), (kind, values, extend_code(g.extend), g.transform));
            assert_eq!(&scene.stops[c.stops_start..][..c.stops_count], g.stops);
            kinds[kind as usize] = true;

            let want = Ramp::with_interpolation(&g, &to_device, Interpolation::Srgb);
            let mut ramp = core::ptr::null_mut();
            let st = unsafe { daegun_scene_ramp(scene, id, to_device.as_ptr(), INTERPOLATE_SRGB, &mut ramp) };
            assert_eq!(st, Status::Ok);
            for (x, y) in (0..15).flat_map(|i| (0..15).map(move |j| (f64::from(i) * 7.0, f64::from(j) * 7.0))) {
                let mut got = Rgba::default();
                let st = unsafe { daegun_ramp_at(ramp, x, y, &mut got) };
                assert_eq!((st == Status::Ok).then_some(got), want.at(x, y), "glyph {gid} gradient {id} at {x}, {y}");
            }
            unsafe { daegun_ramp_free(ramp) };
        }
        unsafe { daegun_scene_free(handle) };
    }
    if !cfg!(miri) {
        assert_eq!((kinds, ops, foreground), ([true; 3], [true; 4], true), "the font covers every part of a scene");
    }
}

fn code(node: &PaintNode<'_>) -> i32 {
    match node {
        PaintNode::Layers => PAINT_LAYERS,
        PaintNode::Glyph { .. } => PAINT_GLYPH,
        PaintNode::ColrGlyph { .. } => PAINT_COLR_GLYPH,
        PaintNode::Solid { .. } => PAINT_SOLID,
        PaintNode::LinearGradient { .. } => PAINT_LINEAR_GRADIENT,
        PaintNode::RadialGradient { .. } => PAINT_RADIAL_GRADIENT,
        PaintNode::SweepGradient { .. } => PAINT_SWEEP_GRADIENT,
        PaintNode::Transform { .. } => PAINT_TRANSFORM,
        PaintNode::Translate { .. } => PAINT_TRANSLATE,
        PaintNode::Scale { .. } => PAINT_SCALE,
        PaintNode::ScaleUniform { .. } => PAINT_SCALE_UNIFORM,
        PaintNode::Rotate { .. } => PAINT_ROTATE,
        PaintNode::Skew { .. } => PAINT_SKEW,
        PaintNode::Composite { .. } => PAINT_COMPOSITE,
    }
}

// Each node's depth, code and stops as the walk meets them.
#[derive(Default)]
struct Walk {
    depth: usize,
    nodes: Vec<(usize, i32, core::ops::Range<usize>)>,
    stops: Vec<ColorStop>,
}

impl PaintVisitor for Walk {
    fn enter(&mut self, node: PaintNode<'_>) {
        let start = self.stops.len();
        if let PaintNode::LinearGradient { stops, .. }
        | PaintNode::RadialGradient { stops, .. }
        | PaintNode::SweepGradient { stops, .. } = node
        {
            self.stops.extend_from_slice(stops);
        }
        self.nodes.push((self.depth, code(&node), start..self.stops.len()));
        self.depth += 1;
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }
}

// Every paint graph of the COLR test font laid out as the walk met it: a node's end is one past its last
// descendant, and its stops are its own.
#[test]
fn a_paint_graph_reaches_c_as_the_walk_meets_it() {
    let font = Font::from_bytes(&asset("colr-v1-test-glyphs/test_glyphs.ttf")).unwrap();
    let mut seen = alloc::collections::BTreeSet::new();
    for gid in 0..font.glyph_count() {
        let (mut walk, mut graph) = (Walk::default(), PaintGraphC::empty());
        if !font.visit_paint_graph(GlyphId(gid), &Location::default(), 0, &mut walk) {
            continue;
        }
        assert!(font.visit_paint_graph(GlyphId(gid), &Location::default(), 0, &mut graph));
        assert_eq!((graph.nodes.len(), &graph.stops, walk.depth), (walk.nodes.len(), &walk.stops, 0));
        for (i, (n, (depth, kind, stops))) in graph.nodes.iter().zip(&walk.nodes).enumerate() {
            let end = i + 1 + walk.nodes[i + 1..].iter().take_while(|(d, ..)| d > depth).count();
            assert_eq!((n.kind, n.end, n.stops_start..n.stops_start + n.stops_count), (*kind, end, stops.clone()));
            seen.insert(*kind);
        }
    }
    assert_eq!(seen.len(), 14, "the font covers every paint");
}

// A graph and a scene freed are the thread's next of their kind, their lists' room kept, so a caller building and
// freeing them in a loop allocates nothing; what each then holds is the next glyph's alone, as a fresh one holds it.
#[test]
fn a_freed_graph_and_scene_are_the_next_ones() {
    use crate::ffi::testing::lent;
    use core::ptr::{null, null_mut};
    let font = CFont::of(asset("colr-v1-test-glyphs/test_glyphs.ttf"));
    let f = font.handle.cast_const();
    let graph = |gid| unsafe {
        let mut g = null_mut();
        (daegun_font_paint_graph(f, gid, null(), 0, &mut g) == Status::Ok).then_some(g)
    };
    let read = |g: *mut PaintGraphC| unsafe {
        let (mut n, mut s) = (0, 0);
        let stops = lent(daegun_paint_graph_stops(g, &mut s), s);
        lent(daegun_paint_graph_nodes(g, &mut n), n).iter().map(|node| decode(node, stops)).collect::<Vec<_>>()
    };
    let mut colored = (0..font_glyphs(f)).filter_map(|gid| graph(gid).map(|g| (gid, g)));
    let ((a, first), (b, other)) = (colored.next().unwrap(), colored.next().unwrap());
    drop(colored);
    unsafe { daegun_paint_graph_free(other) };
    let a_fresh = read(first);
    unsafe { daegun_paint_graph_free(first) };
    let second = graph(b).unwrap();
    assert_eq!(second, first, "the graph freed last was not reused");
    let fresh = graph(b).unwrap();
    assert!(fresh != second && read(second) == read(fresh) && read(second) != a_fresh);
    unsafe { (daegun_paint_graph_free(second), daegun_paint_graph_free(fresh)) };

    let scene = |gid| unsafe {
        let mut s = null_mut();
        assert_eq!(daegun_font_color_scene(f, gid, null(), null(), &mut s), Status::Ok);
        s
    };
    let ops = |s: *mut Scene| unsafe {
        let mut n = 0;
        lent(daegun_scene_ops(s, &mut n), n).iter().map(|op| (op.kind, op.path, op.gradient)).collect::<Vec<_>>()
    };
    let first = scene(a);
    unsafe { daegun_scene_free(first) };
    let (second, fresh) = (scene(b), scene(b));
    assert!(second == first && fresh != second && ops(second) == ops(fresh), "the scene freed last was not reused");
    // Its display list too: drawn again and again, it neither moves nor grows.
    let held = |s: *mut Scene| unsafe {
        let mut view = PathViewC { verbs: null(), verbs_len: 0, points: null(), points_len: 0 };
        assert_eq!(daegun_scene_path_at(s, 0, &mut view), Status::Ok);
        ((*s).list.ops().as_ptr(), view.verbs, view.points, (*s).list.heap_bytes())
    };
    let kept = held(second);
    unsafe { daegun_scene_free(second) };
    for _ in 0..3 {
        let again = scene(b);
        assert_eq!(held(again), kept, "a reused scene's list moved or grew");
        unsafe { daegun_scene_free(again) };
    }
    unsafe { daegun_scene_free(fresh) };
}

fn font_glyphs(font: *const Font) -> u32 {
    let mut n = 0;
    assert_eq!(unsafe { crate::ffi::font::daegun_font_glyph_count(font, &mut n) }, Status::Ok);
    n
}

// A C paint node read back as the Rust node it stands for, so a field put in the wrong place shows.
fn decode(n: &NodeC, stops: &[ColorStop]) -> String {
    let extend = match n.extend {
        EXTEND_PAD => Extend::Pad,
        EXTEND_REPEAT => Extend::Repeat,
        EXTEND_REFLECT => Extend::Reflect,
        code => return alloc::format!("extend {code}"),
    };
    let (v, center) = (n.values, n.has_center.then_some(crate::color::Center { x: n.center[0], y: n.center[1] }));
    let node = match n.kind {
        PAINT_LAYERS => PaintNode::Layers,
        PAINT_GLYPH => PaintNode::Glyph { glyph: GlyphId(n.glyph) },
        PAINT_COLR_GLYPH => PaintNode::ColrGlyph { glyph: GlyphId(n.glyph) },
        PAINT_SOLID => PaintNode::Solid { color: n.color, foreground: n.foreground },
        PAINT_LINEAR_GRADIENT => {
            PaintNode::LinearGradient { extend, stops, x0: v[0], y0: v[1], x1: v[2], y1: v[3], x2: v[4], y2: v[5] }
        }
        PAINT_RADIAL_GRADIENT => {
            PaintNode::RadialGradient { extend, stops, x0: v[0], y0: v[1], r0: v[2], x1: v[3], y1: v[4], r1: v[5] }
        }
        PAINT_SWEEP_GRADIENT => {
            PaintNode::SweepGradient { extend, stops, cx: v[0], cy: v[1], start_angle: v[2], end_angle: v[3] }
        }
        PAINT_TRANSFORM => PaintNode::Transform { matrix: v },
        PAINT_TRANSLATE => PaintNode::Translate { dx: v[0], dy: v[1] },
        PAINT_SCALE => PaintNode::Scale { sx: v[0], sy: v[1], center },
        PAINT_SCALE_UNIFORM => PaintNode::ScaleUniform { s: v[0], center },
        PAINT_ROTATE => PaintNode::Rotate { angle: v[0], center },
        PAINT_SKEW => PaintNode::Skew { x_angle: v[0], y_angle: v[1], center },
        PAINT_COMPOSITE => match u8::try_from(n.mode) {
            Ok(mode @ 0..=27) => PaintNode::Composite { mode: Blend::from_colr(mode) },
            _ => return alloc::format!("mode {}", n.mode),
        },
        kind => return alloc::format!("kind {kind}"),
    };
    alloc::format!("{node:?}")
}

// A C graph walked as a visitor sees it: each node, its children from the next node to its end, then a leave.
fn walk_c(nodes: &[NodeC], stops: &[ColorStop], i: usize, out: &mut Vec<String>) -> usize {
    let n = &nodes[i];
    out.push(decode(n, stops.get(n.stops_start..n.stops_start + n.stops_count).unwrap_or(&[])));
    let (mut j, end) = (i + 1, n.end.clamp(i + 1, nodes.len()));
    while j < end {
        j = walk_c(nodes, stops, j, out);
    }
    out.push(String::from("leave"));
    end
}

fn walk_rust(p: crate::color::PaintRef<'_>, out: &mut Vec<String>) {
    out.push(alloc::format!("{:?}", p.node()));
    for child in p.children() {
        walk_rust(child, out);
    }
    out.push(String::from("leave"));
}

struct Record(Vec<String>);

impl PaintVisitor for Record {
    fn enter(&mut self, node: PaintNode<'_>) {
        self.0.push(alloc::format!("{node:?}"));
    }

    fn leave(&mut self) {
        self.0.push(String::from("leave"));
    }
}

unsafe extern "C" fn record_enter(user: *mut c_void, node: *const NodeC, stops: *const ColorStop, count: usize) {
    let (out, node) = unsafe { (&mut *user.cast::<Vec<String>>(), &*node) };
    assert_eq!(count, node.stops_count, "the count a visitor is handed is the node's");
    out.push(decode(node, unsafe { crate::ffi::testing::lent(stops, count) }));
}

unsafe extern "C" fn record_leave(user: *mut c_void) {
    unsafe { &mut *user.cast::<Vec<String>>() }.push(String::from("leave"));
}

fn rule_of(code: i32) -> Option<FillRule> {
    match code {
        FILL_NONZERO => Some(FillRule::NonZero),
        FILL_EVENODD => Some(FillRule::EvenOdd),
        _ => None,
    }
}

type SceneSig = (Vec<String>, Vec<String>, Vec<(Vec<u8>, Vec<(u32, u32)>, Vec<u8>, Vec<(u32, u32)>)>);

fn path_bits(points: &[Point]) -> Vec<(u32, u32)> {
    points.iter().map(|p| (p.x.to_bits(), p.y.to_bits())).collect()
}

// A C scene as the Rust ops, gradients and paths it stands for, each path also as its copy draws it.
unsafe fn c_scene(scene: *const Scene) -> SceneSig {
    use crate::ffi::testing::lent;
    let (mut n, mut k, mut g, mut s) = (0, 0, 0, 0);
    let ops = unsafe { lent(daegun_scene_ops(scene, &mut n), n) };
    let shapes = unsafe { lent(daegun_scene_shapes(scene, &mut k), k) };
    let gradients = unsafe { lent(daegun_scene_gradients(scene, &mut g), g) };
    let stops = unsafe { lent(daegun_scene_stops(scene, &mut s), s) };
    let ops = ops.iter().map(|o| match o.kind {
        OP_FILL => {
            let paint = if o.has_gradient { Paint::Gradient(GradientId(o.gradient)) } else { Paint::Solid(o.color) };
            let Some(rule) = rule_of(o.rule) else { return alloc::format!("rule {}", o.rule) };
            alloc::format!("{:?}", Op::Fill { path: PathId(o.path), paint, rule, transform: o.transform })
        }
        OP_PUSH_CLIP => {
            let shapes = shapes.get(o.shapes_start..o.shapes_end).unwrap_or(&[]).iter();
            let shapes: Vec<_> = shapes
                .map(|c| rule_of(c.rule).map(|rule| ClipShape { path: PathId(c.path), rule, transform: c.transform }))
                .collect();
            alloc::format!("PushClip {shapes:?}")
        }
        OP_POP_CLIP => String::from("PopClip"),
        OP_PUSH_LAYER => match u8::try_from(o.blend) {
            Ok(mode @ 0..=27) => {
                alloc::format!("{:?}", Op::PushLayer { opacity: o.opacity, blend: Blend::from_colr(mode) })
            }
            _ => alloc::format!("blend {}", o.blend),
        },
        OP_POP_LAYER => String::from("PopLayer"),
        kind => alloc::format!("kind {kind}"),
    });
    let gradients = gradients.iter().map(|g| {
        let kind = match g.kind {
            GRADIENT_LINEAR => {
                GradientKind::Linear { x0: g.values[0], y0: g.values[1], x1: g.values[2], y1: g.values[3] }
            }
            GRADIENT_RADIAL => GradientKind::Radial {
                x0: g.values[0],
                y0: g.values[1],
                r0: g.values[2],
                x1: g.values[3],
                y1: g.values[4],
                r1: g.values[5],
            },
            GRADIENT_SWEEP => GradientKind::Sweep {
                cx: g.values[0],
                cy: g.values[1],
                start_angle: g.values[2],
                end_angle: g.values[3],
            },
            kind => return alloc::format!("kind {kind}"),
        };
        let extend = match g.extend {
            EXTEND_PAD => Extend::Pad,
            EXTEND_REPEAT => Extend::Repeat,
            EXTEND_REFLECT => Extend::Reflect,
            code => return alloc::format!("extend {code}"),
        };
        let stops = stops.get(g.stops_start..g.stops_start + g.stops_count).unwrap_or(&[]);
        alloc::format!("{:?}", crate::color::Gradient { kind, stops, extend, transform: g.transform })
    });
    let mut count = 0;
    let _ = unsafe { daegun_scene_path_count(scene, &mut count) };
    let paths = (0..count).map(|i| {
        let mut v = core::mem::MaybeUninit::<PathViewC>::zeroed();
        let _ = unsafe { daegun_scene_path_at(scene, i, v.as_mut_ptr()) };
        let v = unsafe { v.assume_init() };
        let (verbs, points) = unsafe { (lent(v.verbs, v.verbs_len).to_vec(), lent(v.points, v.points_len)) };
        let mut copy = core::ptr::null_mut();
        let _ = unsafe { daegun_scene_path_copy(scene, i, &mut copy) };
        let (mut cv, mut cp) = (0, 0);
        let copied = unsafe {
            let verbs = lent(crate::ffi::outline::daegun_path_verbs(copy, &mut cv), cv).to_vec();
            (verbs, path_bits(lent(crate::ffi::outline::daegun_path_points(copy, &mut cp), cp)))
        };
        unsafe { crate::ffi::outline::daegun_path_free(copy) };
        (verbs, path_bits(points), copied.0, copied.1)
    });
    (ops.collect(), gradients.collect(), paths.collect())
}

fn rust_scene(list: &DisplayList) -> SceneSig {
    let ops = list.ops().iter().map(|op| match op {
        Op::PushClip { shapes } => {
            let shapes: Vec<_> = list.shapes(*shapes).iter().map(|&s| Some(s)).collect();
            alloc::format!("PushClip {shapes:?}")
        }
        Op::PopClip => String::from("PopClip"),
        Op::PopLayer => String::from("PopLayer"),
        op => alloc::format!("{op:?}"),
    });
    let gradients = list.gradients().map(|g| alloc::format!("{g:?}"));
    let paths = list.paths().map(|p| {
        let (verbs, points) = p.parts();
        let verbs: Vec<u8> = verbs.iter().map(|&v| v as u8).collect();
        (verbs.clone(), path_bits(points), verbs, path_bits(points))
    });
    (ops.collect(), gradients.collect(), paths.collect())
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::color::gradient::{Interpolation, Ramp};
    use crate::ffi::testing::{Diffs, each_font, filled, filled2, found, glyph_list, lent, status};
    use core::ptr::null_mut;
    let mut d = Diffs::default();
    let (backdrop, source) = ([0.2f32, 0.5, 0.9], [0.7f32, 0.1, 0.4]);
    for mode in -1..=28 {
        let mut out = [0.0f32; 3];
        let st = unsafe { daegun_blend(mode, backdrop.as_ptr(), source.as_ptr(), out.as_mut_ptr()) };
        let (mut color, mut alpha) = ([0.0f32; 3], 0.0f32);
        let cst = unsafe {
            daegun_composite(mode, source.as_ptr(), 0.6, backdrop.as_ptr(), 0.8, color.as_mut_ptr(), &mut alpha)
        };
        let composited = crate::color::Premultiplied { color, alpha };
        let c = (st, (st == Status::Ok).then_some(out), cst, (cst == Status::Ok).then_some(composited));
        let want = match u8::try_from(mode) {
            Ok(m @ 0..=27) => {
                let blend = Blend::from_colr(m);
                let composite = blend.composite(source, 0.6, backdrop, 0.8);
                (Status::Ok, Some(blend.blend(backdrop, source)), Status::Ok, Some(composite))
            }
            _ => (Status::InvalidArgument, None, Status::InvalidArgument, None),
        };
        d.eq("blend and composite", "mode", (mode, c), (mode, want));
    }
    let foreground = Rgba { r: 200, g: 10, b: 30, a: 128 };
    let c_opts = [SceneOptionsC::defaults(), SceneOptionsC { palette: 1, foreground, ..SceneOptionsC::defaults() }];
    let rust_opts = [SceneOptions::default(), SceneOptions::default().with_palette(1).with_foreground(foreground)];
    each_font(&mut d, |f, d| unsafe {
        let gids = glyph_list(&f.rust);
        let (st, palettes) = filled(|o, l, c| daegun_font_palettes(f.c, o, l, c));
        let got: Vec<_> = palettes
            .iter()
            .map(|p| (p.index, p.has_name_id.then_some(p.name_id), p.light_safe, p.dark_safe))
            .collect();
        let palette = |p: &crate::color::Palette| (p.index, p.name_id.map(|id| id.0), p.light_safe, p.dark_safe);
        let want: Vec<_> = f.rust.palettes().iter().map(palette).collect();
        d.eq(&f.name, "palettes", (st, got), (Status::Ok, want));
        let (st, ids, named) = filled2(|i, n, l, c| daegun_font_palette_entry_labels(f.c, i, n, l, c));
        let labels: Vec<Option<NameId>> =
            ids.into_iter().zip(named).map(|(id, named)| (named != 0).then_some(NameId(id))).collect();
        d.eq(&f.name, "palette_entry_labels", (st, labels), (Status::Ok, f.rust.palette_entry_labels()));
        for palette in [0, 1, u16::MAX] {
            let (st, colors) = filled(|o, l, c| daegun_font_palette_colors(f.c, palette, o, l, c));
            let want = (palette, Status::Ok, f.rust.palette_colors(palette));
            d.eq(&f.name, "palette_colors", (palette, st, colors), want);
        }
        for &g in &gids {
            for ppem in [16, 72, 136] {
                let mut b = null_mut();
                let st = daegun_font_bitmap(f.c, g, ppem, &mut b);
                let got = (!b.is_null()).then(|| {
                    let mut i = core::mem::MaybeUninit::<BitmapInfoC>::zeroed();
                    let _ = daegun_bitmap_info_of(b, i.as_mut_ptr());
                    let i = i.assume_init();
                    (i.format, i.ppem, i.left, i.top, i.width, i.height, i.mirrored, lent(i.data, i.data_len).to_vec())
                });
                daegun_bitmap_free(b);
                let r = f.rust.bitmap(GlyphId(g), ppem);
                let want = r.as_ref().map(|b| {
                    let (format, width, height, data) = match &b.image {
                        crate::color::BitmapImage::Png(png) => (BITMAP_PNG, 0, 0, png.to_vec()),
                        crate::color::BitmapImage::Coverage { width, height, pixels } => {
                            (BITMAP_COVERAGE, *width, *height, pixels.clone())
                        }
                    };
                    (format, b.ppem, b.left, b.top, width, height, b.mirrored, data)
                });
                d.eq(&f.name, "bitmap", (g, ppem, st, got), (g, ppem, found(&r), want));
            }
            for palette in [0, 1, u16::MAX] {
                let (st, layers) = filled(|o, l, c| daegun_font_colr_layers(f.c, g, palette, o, l, c));
                let got = (st == Status::Ok).then(|| alloc::format!("{:?}", &layers[..]));
                let r = f.rust.colr_layers(GlyphId(g), palette);
                let want = r.as_ref().map(|l| alloc::format!("{:?}", &l[..]));
                d.eq(&f.name, "colr_layers", (g, palette, st, got), (g, palette, found(&r), want));
            }
        }
        for place in f.places().into_iter().filter(|p| p.whole) {
            let at = &alloc::format!("{} @{}", f.name, place.label);
            for &g in &gids {
                let mut b = Bounds::default();
                let st = daegun_font_colr_clip_box(f.c, g, place.c, &mut b);
                let r = f.rust.colr_clip_box(GlyphId(g), &place.rust);
                d.eq(at, "colr_clip_box", (g, st, (st == Status::Ok).then_some(b)), (g, found(&r), r));
                for (i, (c, r)) in c_opts.iter().zip(&rust_opts).enumerate() {
                    let mut scene = null_mut();
                    let st = daegun_font_color_scene(f.c, g, place.c, c, &mut scene);
                    let got = (!scene.is_null()).then(|| c_scene(scene));
                    let mut ramps = Vec::new();
                    let rust = f.rust.color_scene(GlyphId(g), &place.rust, r);
                    let want_status = match &rust {
                        Ok(Some(_)) => Status::Ok,
                        Ok(None) => Status::Absent,
                        Err(_) => status(&rust),
                    };
                    let list = rust.ok().flatten();
                    if let Some(list) = &list {
                        let to_device = [2.0, 0.0, 0.0, 2.0, 5.0, 5.0];
                        for (id, gradient) in list.gradients().enumerate() {
                            for (code, how) in [
                                (INTERPOLATE_LINEAR_LIGHT, Interpolation::LinearLight),
                                (INTERPOLATE_SRGB, Interpolation::Srgb),
                            ] {
                                let mut ramp = null_mut();
                                let st = daegun_scene_ramp(scene, id, to_device.as_ptr(), code, &mut ramp);
                                let rust_ramp = Ramp::with_interpolation(&gradient, &to_device, how);
                                for (x, y) in [(0.0, 0.0), (10.0, 20.0), (-5.0, 3.0), (100.0, 100.0), (1e9, -1e9)] {
                                    let mut px = Rgba::default();
                                    let at_st = daegun_ramp_at(ramp, x, y, &mut px);
                                    let r = rust_ramp.at(x, y);
                                    ramps.push((
                                        (id, code, st, at_st, (at_st == Status::Ok).then_some(px)),
                                        (id, code, Status::Ok, found(&r), r),
                                    ));
                                }
                                daegun_ramp_free(ramp);
                            }
                        }
                    }
                    daegun_scene_free(scene);
                    d.eq(at, "color_scene", (g, i, st, got), (g, i, want_status, list.as_ref().map(rust_scene)));
                    for (c, r) in ramps {
                        d.eq(at, "scene_ramp", (g, i, c), (g, i, r));
                    }
                }
                for palette in [0, 1] {
                    let mut graph = null_mut();
                    let st = daegun_font_paint_graph(f.c, g, place.c, palette, &mut graph);
                    let got = (!graph.is_null()).then(|| {
                        let (mut n, mut s) = (0, 0);
                        let nodes = lent(daegun_paint_graph_nodes(graph, &mut n), n);
                        let stops = lent(daegun_paint_graph_stops(graph, &mut s), s);
                        let (mut out, mut i) = (Vec::new(), 0);
                        while i < nodes.len() {
                            i = walk_c(nodes, stops, i, &mut out);
                        }
                        out
                    });
                    daegun_paint_graph_free(graph);
                    let r = f.rust.paint_graph(GlyphId(g), &place.rust, palette);
                    let want = r.as_ref().map(|graph| {
                        let mut out = Vec::new();
                        walk_rust(graph.root(), &mut out);
                        out
                    });
                    d.eq(at, "paint_graph", (g, palette, st, got), (g, palette, found(&r), want));

                    let mut seen: Vec<String> = Vec::new();
                    let visitor =
                        VisitorC { enter: Some(record_enter), leave: Some(record_leave), user: (&raw mut seen).cast() };
                    let st = daegun_font_visit_paint_graph(f.c, g, place.c, palette, &visitor);
                    let mut record = Record(Vec::new());
                    let visited = f.rust.visit_paint_graph(GlyphId(g), &place.rust, palette, &mut record);
                    let want = (if visited { Status::Ok } else { Status::Absent }, record.0);
                    d.eq(at, "visit_paint_graph", (g, palette, (st, seen)), (g, palette, want));
                }
            }
        }
    });
    d.done("color");
}
