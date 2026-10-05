use daegun::paint::{DisplayList, Op};
use daegun::{Font, Paint};

const STATIC: &str = "colr-v1-test-glyphs/test_glyphs.ttf";
const VARIABLE: &str = "colr-v1-test-glyphs/test_glyphs_variable.ttf";

fn bytes(rel: &str) -> Vec<u8> {
    let path = format!("{}/{}", crate::FONTS, rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

fn font(rel: &str) -> Font {
    Font::from_vec(bytes(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// The font with its `tag` table edited in place.
fn edited(rel: &str, tag: &[u8; 4], edit: impl FnOnce(&mut [u8])) -> Font {
    let mut b = bytes(rel);
    let n = usize::from(u16::from_be_bytes([b[4], b[5]]));
    let rec = (0..n).map(|i| 12 + i * 16).find(|&r| &b[r..r + 4] == tag).expect("the table");
    let at = |o: usize| u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (off, len) = (at(rec + 8), at(rec + 12));
    edit(&mut b[off..off + len]);
    Font::from_vec(b).expect("still a font")
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
fn root_paint(colr: &[u8], gid: u16) -> usize {
    let list = u32_at(colr, 14);
    let n = u32_at(colr, list);
    (0..n)
        .map(|i| list + 4 + i * 6)
        .find(|&r| u16::from_be_bytes([colr[r], colr[r + 1]]) == gid)
        .map(|r| list + u32_at(colr, r + 2))
        .expect("a base glyph record")
}

fn first_of<'p>(paint: &'p Paint, pick: &dyn Fn(&'p Paint) -> bool) -> Option<&'p Paint> {
    if pick(paint) {
        return Some(paint);
    }
    match paint {
        Paint::Layers(c) => c.iter().find_map(|p| first_of(p, pick)),
        Paint::Composite { source, backdrop, .. } => first_of(source, pick).or_else(|| first_of(backdrop, pick)),
        Paint::Glyph { child, .. }
        | Paint::ColrGlyph { child, .. }
        | Paint::Transform { child, .. }
        | Paint::Translate { child, .. }
        | Paint::Scale { child, .. }
        | Paint::ScaleUniform { child, .. }
        | Paint::Rotate { child, .. }
        | Paint::Skew { child, .. } => first_of(child, pick),
        _ => None,
    }
}

fn fills(scene: &DisplayList) -> usize {
    scene.ops().iter().filter(|o| matches!(o, Op::Fill { .. })).count()
}

// The spec lets varied angles, scales, radii and stop offsets leave their stored type; these are
// fontTools' values for the conformance font (its location held in 2.14), which a clamp would cut short.
#[test]
fn varied_values_may_leave_their_stored_range() {
    let f = font(VARIABLE);
    let angle = |p: &Paint| match p { Paint::Rotate { angle, .. } => Some(*angle), _ => None };
    let g = f.colr_v1_paint(99, &[("ROTA", 450.0)], 0).expect("rotate_10");
    let a = first_of(&g, &|p| angle(p).is_some()).and_then(angle).expect("a rotation");
    assert!((a - 451.6787).abs() < 1e-3, "rotation {a}");

    let g = f.colr_v1_paint(93, &[("GRR0", -500.0)], 0).expect("radial");
    let Some(Paint::RadialGradient { r0, .. }) = first_of(&g, &|p| matches!(p, Paint::RadialGradient { .. })) else { panic!("no radial") };
    assert_eq!(*r0, -500.0);

    let g = f.colr_v1_paint(87, &[("SCSX", 1.99), ("SCSY", 1.99)], 0).expect("scale");
    let Some(Paint::ScaleUniform { s, .. }) = first_of(&g, &|p| matches!(p, Paint::ScaleUniform { .. })) else { panic!("no scale") };
    assert!((s - 3.49).abs() < 1e-3, "scale {s}");

    let g = f.colr_v1_paint(19, &[("SWC1", 2.0)], 0).expect("sweep");
    let Some(Paint::SweepGradient { stops, .. }) = first_of(&g, &|p| matches!(p, Paint::SweepGradient { .. })) else { panic!("no sweep") };
    assert!((stops[0].offset - 2.25).abs() < 1e-3, "first stop {}", stops[0].offset);
}

// A map that is named and cannot be read leaves no way to find the right deltas: nothing varies,
// where the implicit mapping would hand gid 19 another field's row (0.75 for 0.25).
#[test]
fn an_unreadable_index_map_varies_nothing() {
    let f = edited(VARIABLE, b"COLR", |colr| colr[26..30].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes()));
    let g = f.colr_v1_paint(19, &[("SWPS", 90.0)], 0).expect("sweep");
    let Some(Paint::SweepGradient { stops, start_angle, .. }) = first_of(&g, &|p| matches!(p, Paint::SweepGradient { .. })) else { panic!("no sweep") };
    assert!((stops[0].offset - 0.25).abs() < 1e-3, "first stop {}", stops[0].offset);
    assert!((start_angle - -45.0).abs() < 1e-3, "start {start_angle}");
}

// An x skew leans by -tan of its angle, as the spec, fontTools and HarfBuzz have it.
#[test]
fn an_x_skew_leans_the_way_the_spec_says() {
    let f = font(STATIC);
    let scene = f.colr_scene(103, &[], 0).expect("skew_25_0");
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
        let scene = f.colr_scene(gid, &[], 0).unwrap_or_else(|| panic!("gid {gid} has no scene"));
        let gradient = scene.ops().iter().any(|o| matches!(o, Op::Fill { paint: daegun::paint::Paint::Gradient(_), .. }));
        assert!(gradient, "gid {gid} lost its gradient");
    }
}

// The clip boxes of the conformance glyphs, read and applied: each scene starts with its box as
// a clip, and a variable box is rounded outward at the location, as fontTools gives it.
#[test]
fn a_clip_box_bounds_its_glyph() {
    let f = font(STATIC);
    let boxes = [(156, [0, 500, 500, 1000]), (157, [0, 0, 500, 500]), (158, [500, 0, 1000, 500]), (159, [500, 500, 1000, 1000]), (160, [250, 250, 750, 750])];
    for (gid, b) in boxes {
        assert_eq!(f.colr_clip_box(gid, &[]), Some(b), "gid {gid}");
        let scene = f.colr_scene(gid, &[], 0).expect("a scene");
        let Some(Op::PushClip { shapes }) = scene.ops().first() else { panic!("gid {gid} is not clipped") };
        let bounds = scene.path(shapes[0].path).and_then(|p| p.bounds()).expect("a box");
        assert_eq!(bounds, (f64::from(b[0]), f64::from(b[1]), f64::from(b[2]), f64::from(b[3])), "gid {gid}");
    }
    assert_eq!(f.colr_clip_box(1, &[]), None, "a glyph with no box");
    // 4095/16384 of each axis, exact in 2.14, leaves every edge 0.03 off a whole unit: rounding to
    // nearest would give 375 and 625.
    let v = font(VARIABLE);
    let q = 4095.0 / 16384.0 * 500.0;
    let at = [("CLXI", q), ("CLYI", q), ("CLXA", -q), ("CLYA", -q), ("CLIO", q)];
    assert_eq!(v.colr_clip_box(160, &at), Some([374, 374, 626, 626]));
    assert_eq!(v.colr_clip_box(166, &at), Some([224, 224, 776, 776]));
}

// A fill outside every PaintGlyph fills the clip box, or with none, a rectangle around the glyph's
// bounded content; a glyph the spec calls unbounded and gives no box is not drawn.
#[test]
fn a_fill_outside_every_glyph_is_bounded_as_the_spec_says() {
    let mut square = |_| {
        let mut p = daegun::Path::default();
        daegun::OutlinePen::move_to(&mut p, 100.0, 100.0);
        for (x, y) in [(300.0, 100.0), (300.0, 300.0), (100.0, 300.0)] {
            daegun::OutlinePen::line_to(&mut p, x, y);
        }
        daegun::OutlinePen::close(&mut p);
        Some(p)
    };
    let solid = |alpha| Box::new(Paint::Solid { is_foreground: false, r: 0, g: 0, b: 255, alpha });
    let shape = || Box::new(Paint::Glyph { child: solid(255), glyph_id: 1 });
    let mut lower = |paint: &Paint, clip| {
        let mut scene = DisplayList::default();
        let drawn = daegun::paint::lower(paint, daegun::paint::IDENTITY, clip, &mut square, daegun::paint::Rgba::default(), &mut scene);
        (drawn, scene)
    };
    let rect_of = |scene: &DisplayList, alpha: u8| {
        scene.ops().iter().find_map(|o| match o {
            Op::Fill { path, paint: daegun::paint::Paint::Solid(c), transform, .. } if c.a == alpha => {
                let (x0, y0, x1, y1) = scene.path(*path)?.bounds()?;
                Some([x0 * transform[0] + transform[4], y0 * transform[3] + transform[5], x1 * transform[0] + transform[4], y1 * transform[3] + transform[5]])
            }
            _ => None,
        })
    };

    // SrcIn keeps the source where the backdrop is: a group at half opacity, Noto's idiom.
    let group = Paint::Composite { source: shape(), mode: 5, backdrop: solid(128) };
    let (drawn, scene) = lower(&group, None);
    assert!(drawn);
    assert_eq!(rect_of(&scene, 128), Some([100.0, 100.0, 300.0, 300.0]), "the backdrop has no fill around the shape");
    let (_, scene) = lower(&group, Some([0, 0, 1000, 1000]));
    assert_eq!(rect_of(&scene, 128), Some([0.0, 0.0, 1000.0, 1000.0]), "the backdrop does not fill the clip box");

    let unbounded = Paint::Layers(vec![*shape(), *solid(255)]);
    let (drawn, scene) = lower(&unbounded, None);
    assert!(!drawn && scene.ops().is_empty(), "an unbounded glyph was drawn");
    assert!(lower(&unbounded, Some([0, 0, 500, 500])).0, "a clip box bounds it");
}

// An empty backdrop under SrcIn leaves nothing to draw, under SrcOver the source as drawn, and an
// empty source under Xor the backdrop as drawn: none opens a layer.
#[test]
fn composites_showing_nothing_or_their_backdrop_open_no_layers() {
    let mut square = |_| {
        let mut p = daegun::Path::default();
        daegun::OutlinePen::move_to(&mut p, 0.0, 0.0);
        daegun::OutlinePen::line_to(&mut p, 100.0, 0.0);
        daegun::OutlinePen::line_to(&mut p, 0.0, 100.0);
        daegun::OutlinePen::close(&mut p);
        Some(p)
    };
    let shape = || Box::new(Paint::Glyph { child: Box::new(Paint::Solid { is_foreground: false, r: 0, g: 0, b: 255, alpha: 255 }), glyph_id: 1 });
    let empty = || Box::new(Paint::Layers(vec![]));
    let mut lower = |paint: Paint| {
        let mut scene = DisplayList::default();
        assert!(daegun::paint::lower(&paint, daegun::paint::IDENTITY, None, &mut square, daegun::paint::Rgba::default(), &mut scene));
        scene
    };

    let nothing = lower(Paint::Composite { source: shape(), mode: 5, backdrop: empty() });
    assert!(nothing.ops().is_empty(), "SrcIn over nothing drew {:?}", nothing.ops());
    let source = lower(Paint::Composite { source: shape(), mode: 3, backdrop: empty() });
    assert!(matches!(source.ops(), [Op::Fill { .. }]), "SrcOver over nothing drew {:?}", source.ops());
    let backdrop = lower(Paint::Composite { source: empty(), mode: 11, backdrop: shape() });
    assert!(matches!(backdrop.ops(), [Op::Fill { .. }]), "Xor of nothing drew {:?}", backdrop.ops());
}

// A fill reached through layers inside a PaintGlyph fills that glyph too, not only one under
// transforms alone, so the clip it fills stays too.
#[test]
fn a_fill_through_layers_fills_its_glyph() {
    let mut square = |_| {
        let mut p = daegun::Path::default();
        daegun::OutlinePen::move_to(&mut p, 0.0, 0.0);
        daegun::OutlinePen::line_to(&mut p, 100.0, 0.0);
        daegun::OutlinePen::line_to(&mut p, 0.0, 100.0);
        daegun::OutlinePen::close(&mut p);
        Some(p)
    };
    let solid = Paint::Solid { is_foreground: false, r: 9, g: 9, b: 9, alpha: 255 };
    let graph = Paint::Glyph { child: Box::new(Paint::Layers(vec![Paint::Translate { child: Box::new(solid), dx: 5.0, dy: 0.0 }])), glyph_id: 1 };
    let mut scene = DisplayList::default();
    assert!(daegun::paint::lower(&graph, daegun::paint::IDENTITY, None, &mut square, daegun::paint::Rgba::default(), &mut scene));
    assert_eq!(fills(&scene), 1, "{:?}", scene.ops());
}

// The spec: "If an unrecognized value is encountered, COMPOSITE_CLEAR must be used."
#[test]
fn an_unknown_composite_mode_clears() {
    assert_eq!(daegun::paint::Blend::from_colr(200), daegun::paint::Blend::Clear);
}

// One layer pointing past the table is left out and the other four still draw, the glyph not refused
// whole; the cycle glyphs keep what precedes their cycle.
#[test]
fn an_ill_formed_paint_leaves_the_rest_of_the_glyph() {
    let whole = fills(&font(STATIC).colr_scene(180, &[], 0).expect("no_cycle_multi_colrglyph"));
    let f = edited(STATIC, b"COLR", |colr| {
        let root = root_paint(colr, 180);
        assert_eq!(colr[root], 1, "a PaintColrLayers");
        let entry = u32_at(colr, 18) + 4 + u32_at(colr, root + 2) * 4;
        colr[entry..entry + 4].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
    });
    let scene = f.colr_scene(180, &[], 0).expect("the other layers");
    assert_eq!(fills(&scene) * 5, whole * 4, "{} fills of {whole}", fills(&scene));

    let cycle = font(STATIC).colr_v1_paint(178, &[], 0).expect("the cycle glyph parses");
    let Paint::ColrGlyph { glyph_id: 179, child } = &cycle else { panic!("{cycle:?}") };
    assert!(matches!(&**child, Paint::ColrGlyph { glyph_id: 178, child } if **child == Paint::Layers(vec![])), "{cycle:?}");
}

// A subset keeps its glyphs' clip boxes under their new IDs, keeps every glyph whose graph reads
// when another's does not, and keeps a graph as deep as the reader reads.
#[test]
fn a_subset_keeps_clip_boxes_and_readable_glyphs() {
    let f = font(STATIC);
    let sub = f.subset(&[156, 157, 158, 159, 160], &[]).expect("a subset");
    let s = Font::from_vec(sub.ttf.clone()).expect("the subset reads");
    for gid in 156..=160 {
        let new = sub.new_gid(gid).expect("kept");
        assert_eq!(s.colr_clip_box(new, &[]), f.colr_clip_box(gid, &[]), "gid {gid}");
        assert_eq!(s.colr_scene(new, &[], 0).map(|sc| fills(&sc)), f.colr_scene(gid, &[], 0).map(|sc| fills(&sc)), "gid {gid}");
    }

    let broken = edited(STATIC, b"COLR", |colr| {
        let list = u32_at(colr, 14);
        let rec = (0..u32_at(colr, list)).map(|i| list + 4 + i * 6).find(|&r| colr[r..r + 2] == 8u16.to_be_bytes()).expect("gid 8");
        colr[rec + 2..rec + 6].copy_from_slice(&0x00FF_FFF0u32.to_be_bytes());
    });
    let sub = broken.subset(&[8, 9], &[]).expect("a subset");
    let s = Font::from_vec(sub.ttf.clone()).expect("the subset reads");
    assert!(s.colr_scene(sub.new_gid(9).expect("kept"), &[], 0).is_some(), "gid 9 lost its color with gid 8");

    // A variable font subset as it is keeps no fvar, so it is the default master, and its COLR
    // names no variation data to go with axes it does not have.
    let sub = daegun::daecore::daetype::subsetter::subset_ttf(&bytes(VARIABLE), &[99, 160]).expect("a subset");
    assert_eq!(&table(&sub.ttf, b"COLR")[26..34], &[0; 8]);

    let deep = font("test-fixtures/colr-clip.ttf");
    let sub = deep.subset(&[6], &[]).expect("a subset");
    let s = Font::from_vec(sub.ttf.clone()).expect("the subset reads");
    assert_eq!(s.colr_v1_paint(sub.new_gid(6).expect("kept"), &[], 0).is_some(), deep.colr_v1_paint(6, &[], 0).is_some());
}

// An instance draws what the variable font draws there, its out-of-range scales, radii, stops and
// angles written another way. (A padded sweep spanning over 2,880 degrees has no static form.)
#[test]
fn an_instance_draws_what_the_variable_font_draws_there() {
    let f = font(VARIABLE);
    // A location, and the glyphs to compare there (every one when empty).
    type At<'a> = (&'a [(&'a str, f64)], &'a [u16]);
    let locations: [At; 6] = [
        (&[("SCSX", 1.99), ("SCSY", 1.99), ("GRR0", -1000.0), ("COL1", 2.0), ("ROTA", 539.0), ("SKXA", 60.0), ("CLXI", 125.0), ("APH1", -0.5)], &[]),
        (&[("SCSX", -2.0), ("SCOX", 200.0), ("GRR1", -1000.0), ("COL2", -2.0), ("SWC1", 2.0), ("TRDX", 500.0), ("CLIO", -500.0), ("SKYA", -60.0)], &[]),
        (&[("SWC1", -1.0), ("SWC4", 1.0), ("SWPE", 45.0), ("GRR0", 1000.0), ("GRR1", -1000.0), ("COL3", 2.0), ("TRXX", 2.0), ("ROTX", 500.0)], &[]),
        // Glyph 59's padded sweep spans 2,992 degrees here, past a static table, but only 1,102 of
        // them once the stops never seen are left out.
        (&[("SWPS", -90.0), ("SWPE", -90.0), ("SWC1", -2.0), ("SWC4", -2.0), ("SWC2", 2.0)], &[59]),
        // These sweeps' stops reach past 1,260 degrees, held only with the angles running backward,
        // and these only with them running forward from -180.
        (&[("SWPS", -90.0), ("SWPE", -90.0), ("SWC1", 2.0), ("SWC4", 2.0)], &[35, 47, 71, 83]),
        (&[("SWPS", -90.0), ("SWPE", -90.0), ("SWC1", -2.0), ("SWC2", -2.0), ("SWC3", -2.0), ("SWC4", -2.0)], &[71, 83]),
    ];
    for (at, only) in locations {
        let bytes = f.instance(at);
        let colr = table(&bytes, b"COLR");
        assert_eq!(&colr[26..34], &[0; 8], "the index map or store is still named");
        let mut stack: Vec<usize> = (0..u32_at(colr, u32_at(colr, 14))).map(|i| {
            let list = u32_at(colr, 14);
            list + u32_at(colr, list + 4 + i * 6 + 2)
        }).collect();
        while let Some(off) = stack.pop() {
            let format = colr[off];
            assert!(!matches!(format, 3 | 5 | 7 | 9 | 13 | 15 | 17 | 19 | 21 | 23 | 25 | 27 | 29 | 31), "a Var paint (format {format}) is left");
            match format {
                1 => {
                    let list = u32_at(colr, 18);
                    stack.extend((0..usize::from(colr[off + 1])).map(|i| list + u32_at(colr, list + 4 + (u32_at(colr, off + 2) + i) * 4)));
                }
                10 | 12 | 14 | 16 | 18 | 20 | 22 | 24 | 26 | 28 | 30 => stack.push(off + u24_at(colr, off + 1)),
                32 => stack.extend([off + u24_at(colr, off + 1), off + u24_at(colr, off + 5)]),
                _ => {}
            }
        }
        let inst = Font::from_vec(bytes).expect("an instance");
        let glyphs: Vec<u16> = if only.is_empty() { (0..f.num_glyphs()).collect() } else { only.to_vec() };
        for gid in glyphs {
            assert_eq!(inst.colr_clip_box(gid, &[]), f.colr_clip_box(gid, at), "gid {gid} at {at:?}");
            let (a, b) = (f.colr_scene(gid, at, 0), inst.colr_scene(gid, &[], 0));
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
        (0..4).all(|i| near(m[i], n[i], 2e-3 * m[i].abs().max(1.0))) && (4..6).all(|i| near(m[i], n[i], (2e-3 * m[i].abs()).max(1.0)))
    };
    let path = |p: usize, q: usize| a.path(p).map(|p| p.parts().1.to_vec()) == b.path(q).map(|q| q.parts().1.to_vec());
    for (i, (x, y)) in a.ops().iter().zip(b.ops()).enumerate() {
        let same = match (x, y) {
            (Op::Fill { path: p, paint: pa, rule: r, transform: t }, Op::Fill { path: q, paint: pb, rule: s, transform: u }) => {
                r == s && matrix(t, u) && path(*p, *q) && same_paint(pa, pb)
            }
            (Op::PushClip { shapes: s }, Op::PushClip { shapes: t }) => {
                s.len() == t.len() && s.iter().zip(t).all(|(s, t)| s.rule == t.rule && matrix(&s.transform, &t.transform) && path(s.path, t.path))
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

fn same_paint(a: &daegun::paint::Paint, b: &daegun::paint::Paint) -> bool {
    use daegun::paint::Paint::{Gradient, Solid};
    let color = |c: &daegun::paint::Rgba, d: &daegun::paint::Rgba| {
        [(c.r, d.r), (c.g, d.g), (c.b, d.b), (c.a, d.a)].iter().all(|&(x, y)| x.abs_diff(y) <= 1)
    };
    match (a, b) {
        (Solid(c), Solid(d)) => color(c, d),
        // Compared by what they paint on a grid: angles and stops held to 1/16384 shift colors a
        // fraction of a unit, so each sample need only lie within the other's range 2 units around.
        (Gradient(g), Gradient(h)) => {
            let ramp = |g| daegun::paint::gradient::Ramp::new(g, &daegun::paint::IDENTITY);
            let (ra, rb) = (ramp(g), ramp(h));
            let channels = |c: daegun::paint::Rgba| [c.r, c.g, c.b, c.a];
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
