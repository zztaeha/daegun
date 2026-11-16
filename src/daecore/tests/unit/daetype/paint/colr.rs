use super::*;
use crate::daecore::daekit::draw::blend::{composite, Premultiplied};
use crate::daecore::daetype::colr_v1::PaintNode;

// An ill-formed color line, its stops at one offset under repeat or reflect, leaves no paint in the
// display list; under pad it stays a gradient.
#[test]
fn a_color_line_at_one_offset_lowers_to_nothing_unless_padded() {
    let kind = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
    let stop = |r| ColorStop { offset: 0.5, color: Rgba { r, g: 0, b: 0, a: 255 }, foreground: false };
    let stops = [stop(255), stop(0)];
    let black = Rgba::opaque(0, 0, 0);
    let lowered = |extend| gradient(kind, &stops, extend, black, IDENTITY, &mut DisplayList::default());
    assert_eq!([Extend::Repeat, Extend::Reflect].map(lowered), [None, None]);
    assert!(matches!(lowered(Extend::Pad), Some(Paint::Gradient(_))));
}

// What a color glyph paints, boxed: a fill covers its clip outline through the transforms, a referenced glyph's clip
// box narrows it, each composite mode keeps its share, and a fill outside every outline is unbounded, leaving no box.
#[test]
fn a_painted_box_follows_clips_transforms_and_modes() {
    let solid = |p: &mut PaintGraph| {
        p.with(PaintNode::Solid { color: Rgba::opaque(0, 0, 0), foreground: false }, |_| {});
    };
    let glyph = |glyph_id| move |p: &mut PaintGraph| {
        p.with(PaintNode::Glyph { glyph: GlyphId(glyph_id) }, solid);
    };
    let boxed = |build: &dyn Fn(&mut PaintGraph)| {
        let mut p = PaintGraph::new();
        build(&mut p);
        let mut outline = |g: u32| Some(if g == 1 { [0.0, 0.0, 100.0, 50.0] } else { [50.0, 25.0, 300.0, 200.0] });
        painted_box(&mut Tree::new(&p), Tree::ROOT, &|g| (g == 9).then_some([0, 0, 10, 10]), &mut outline)
    };
    let composite = |mode| move |p: &mut PaintGraph| {
        p.with(PaintNode::Composite { mode }, |p| {
            glyph(2)(p);
            glyph(1)(p);
        });
    };
    let translate = |p: &mut PaintGraph| {
        p.with(PaintNode::Translate { dx: 10.0, dy: -5.0 }, glyph(1));
    };
    let scale = |p: &mut PaintGraph| {
        p.with(PaintNode::Scale { sx: 2.0, sy: 3.0, center: None }, glyph(1));
    };
    let referenced = |p: &mut PaintGraph| {
        p.with(PaintNode::ColrGlyph { glyph: GlyphId(9) }, solid);
    };
    let layers = |p: &mut PaintGraph| {
        p.with(PaintNode::Layers, |p| {
            glyph(1)(p);
            solid(p);
        });
    };

    assert_eq!(boxed(&glyph(1)), Some([0.0, 0.0, 100.0, 50.0]));
    assert_eq!(boxed(&translate), Some([10.0, -5.0, 110.0, 45.0]));
    assert_eq!(boxed(&scale), Some([0.0, 0.0, 200.0, 150.0]));
    assert_eq!(boxed(&composite(Blend::SrcOver)), Some([0.0, 0.0, 300.0, 200.0]));
    assert_eq!(boxed(&composite(Blend::SrcIn)), Some([50.0, 25.0, 100.0, 50.0]));
    assert_eq!(boxed(&composite(Blend::Dest)), Some([0.0, 0.0, 100.0, 50.0]));
    assert_eq!(boxed(&composite(Blend::Clear)), None);
    assert_eq!(boxed(&referenced), Some([0.0, 0.0, 10.0, 10.0]));
    assert_eq!(boxed(&layers), None, "a fill outside every outline");
}

// A referenced color glyph with a clip box clips what it paints to the box, under the transform it is
// reached through, and bounds a fill nothing else bounds. Without a box it clips and bounds nothing.
#[test]
fn a_referenced_color_glyph_clips_to_its_clip_box() {
    let solid = |p: &mut PaintGraph| {
        p.with(PaintNode::Solid { color: Rgba::opaque(0, 0, 0), foreground: false }, |_| {});
    };
    let lowered = |paint: &PaintGraph| {
        let mut square = |_, pen: &mut dyn OutlinePen| {
            rectangle(pen, [0.0, 0.0, 100.0, 100.0]);
            true
        };
        let mut out = DisplayList::default();
        let boxes = |g: GlyphId| (g.0 == 9).then_some(Bounds { x_min: 0.0, y_min: 0.0, x_max: 10.0, y_max: 10.0 });
        let drawn = lower(GlyphId(1), paint, IDENTITY, &boxes, &mut square, Rgba::opaque(0, 0, 0), &mut out);
        (drawn, out)
    };
    let colr_glyph = |glyph_id| move |p: &mut PaintGraph| {
        p.with(PaintNode::ColrGlyph { glyph: GlyphId(glyph_id) }, |p| {
            p.with(PaintNode::Glyph { glyph: GlyphId(2) }, solid);
        });
    };
    let graph = |build: &dyn Fn(&mut PaintGraph)| {
        let mut p = PaintGraph::new();
        build(&mut p);
        p
    };
    let scaled = graph(&|p| {
        p.with(PaintNode::Scale { sx: 2.0, sy: 2.0, center: None }, colr_glyph(9));
    });
    let (drawn, out) = lowered(&scaled);
    let &[Op::PushClip { shapes }, Op::Fill { .. }, Op::PopClip] = out.ops() else { panic!("{:?}", out.ops()) };
    let shape = out.shapes(shapes)[0];
    assert!(drawn);
    let square = Bounds { x_min: 0.0, y_min: 0.0, x_max: 10.0, y_max: 10.0 };
    assert_eq!(out.path(shape.path).and_then(|p| p.bounds()), Some(square));
    assert_eq!(shape.transform, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let unclipped = lowered(&graph(&colr_glyph(8)));
    assert!(matches!(unclipped.1.ops(), [Op::Fill { .. }]), "a color glyph without a box clipped");

    let bare = |glyph_id| graph(&|p| {
        p.with(PaintNode::ColrGlyph { glyph: GlyphId(glyph_id) }, solid);
    });
    assert!(lowered(&bare(9)).0, "the clip box bounds a bare fill");
    assert!(!lowered(&bare(8)).0, "nothing bounds this fill");
}

// As blend.rs composites each mode: an empty source leaves the backdrop or nothing, and a source
// over an empty backdrop leaves itself or nothing.
#[test]
fn what_a_mode_keeps_is_what_it_composites() {
    let (color, alpha) = ([0.2f32, 0.5, 0.9], 0.6f32);
    let alone = Premultiplied { color: color.map(|v| v * alpha), alpha };
    for mode in (0..28).map(Blend::from_colr) {
        let under_empty = composite(mode, [0.0; 3], 0.0, color, alpha);
        let over_empty = composite(mode, color, alpha, [0.0; 3], 0.0);
        for (kept, result, what) in
            [(keeps_backdrop(mode), under_empty, "backdrop"), (keeps_source(mode), over_empty, "source")]
        {
            let want = if kept { alone } else { Premultiplied::default() };
            assert_eq!(result, want, "{mode:?} keeps the {what}: {kept}");
        }
    }
}
