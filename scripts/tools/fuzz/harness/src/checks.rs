use daegun::color::{DisplayList, Op, Paint, PaintNode, PaintVisitor, Rgba, SceneOptions};
use daegun::layout::{Adjustment, Align, JustifyOptions, LayoutOptions, TextLayout, WritingMode};
use daegun::outline::{FillRule, HintMode, OutlineOptions, StrokeStyle};
use daegun::raster::{Field, Gamma, Image, RasterOptions, Rasterizer, StripeOrder, SubpixelLayout, Target, MAX_SIDE};
use daegun::shape::{ClusterLevel, Direction, Feature, Ignorables, ShapeOptions, ShapedRun, Shaper};
use daegun::unicode::BaseDirection;
use daegun::{Cache, Font, GlyphId, Location, Orientation, Tag};

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}


pub fn bounded(f: &Font) -> bool {
    f.glyph_count() > 0
}

pub fn check_run(run: &daegun::shape::ShapedRun, text: &str, what: &str) {
    check_run_at(run, text, what, ClusterLevel::MonotoneGraphemes);
}

// As `check_run`, for a run shaped at `level`: its clusters run one way only where the level is monotone.
pub fn check_run_at(run: &daegun::shape::ShapedRun, text: &str, what: &str, level: ClusterLevel) {
    let g = run.glyphs.len();
    must!(run.advances.len() == g, "{what}: advances {} vs glyphs {g}", run.advances.len());
    must!(run.offsets.len() == g, "{what}: offsets {} vs glyphs {g}", run.offsets.len());
    must!(run.clusters.len() == g, "{what}: clusters {} vs glyphs {g}", run.clusters.len());
    must!(run.unsafe_to_break.len() == g, "{what}: flags {} vs glyphs {g}", run.unsafe_to_break.len());
    for (name, v) in [("unsafe_to_concat", &run.unsafe_to_concat),
                      ("safe_to_insert_tatweel", &run.safe_to_insert_tatweel)] {
        must!(v.is_empty() || v.len() == g, "{what}: {name} {} vs glyphs {g}", v.len());
    }

    const SYLLABIC: [Shaper; 5] =
        [Shaper::Indic, Shaper::Khmer, Shaper::Myanmar, Shaper::MyanmarZawgyi, Shaper::Universal];
    must!(
        !run.has_broken_syllable || SYLLABIC.contains(&run.shaper),
        "{what}({text:?}): shaper {} reported a broken syllable", run.shaper.name(),
    );
    let _ = run.complete;
    for (i, &a) in run.advances.iter().enumerate() {
        must!(a.is_finite(), "{what}: advance[{i}] is {a}");
    }
    for (i, o) in run.offsets.iter().enumerate() {
        must!(o.x.is_finite() && o.y.is_finite(), "{what}: offset[{i}] is ({}, {})", o.x, o.y);
    }
    for (i, &c) in run.clusters.iter().enumerate() {
        must!(text.is_char_boundary(c as usize), "{what}({text:?}): cluster[{i}] is {c}, not a character's start");
    }
    let ascends = run.clusters.windows(2).all(|w| w[0] <= w[1]);
    let descends = run.clusters.windows(2).all(|w| w[0] >= w[1]);
    must!(
        !level.is_monotone() || ascends || descends,
        "{what}({text:?}): at {level:?}, of graphemes {}, clusters turned round mid-run: {:?}", level.is_graphemes(),
        run.clusters,
    );
}

// The run's promise where it calls a break safe: the text broken at byte `at` and each side shaped apart, the other
// as its context, as a layout shapes a line's pieces, joins into the run. What differs where it doesn't; None where it
// does, or where the run calls that break unsafe or can't be split there.
pub fn broken_apart(
    f: &Font, loc: &Location, text: &str, options: &ShapeOptions, run: &ShapedRun, at: usize,
) -> Option<String> {
    let forward = run.clusters.windows(2).all(|w| w[0] <= w[1]);
    let backward = run.clusters.windows(2).all(|w| w[0] >= w[1]);
    let ours = |k: &u32| *k as usize == at;
    let safe = run.clusters.iter().zip(&run.unsafe_to_break).filter(|(k, _)| ours(k)).all(|(_, &unsafe_)| !unsafe_);
    if !run.complete || forward == backward || at == 0 || !run.clusters.iter().any(ours) || !safe {
        return None;
    }
    let (head, tail) = (text.get(..at)?, text.get(at..)?);
    // Held to the whole run's script and direction: a side alone may read as another.
    let script = text.chars().map(daegun::unicode::script).find(|s| !s.is_context_dependent());
    let mut alike = *options;
    alike.seed_script = options.seed_script.or(script);
    alike.direction = options.direction.or(Some(if forward { Direction::LeftToRight } else { Direction::RightToLeft }));
    let (after, before) = (format!("{tail}{}", options.after), format!("{}{head}", options.before));
    let (head_features, tail_features) =
        (features_over(options.features, 0, at), features_over(options.features, at, text.len()));
    let first = f.shape(head, loc, &alike.with_context(options.before, &after).with_features(&head_features));
    let rest = alike.with_context(&before, options.after).with_features(&tail_features).with_beginning_of_text(false);
    let second = f.shape(tail, loc, &rest);
    if !first.complete || !second.complete {
        return None;
    }
    let glyphs = |r: &ShapedRun, by: u32| {
        let g = |i: usize| {
            (r.glyphs[i].0, r.clusters[i].saturating_add(by), r.advances[i], r.offsets[i].x, r.offsets[i].y)
        };
        (0..r.glyphs.len()).map(g).collect::<Vec<_>>()
    };
    let by = u32::try_from(at).unwrap_or(u32::MAX);
    let whole = glyphs(run, 0);
    let (first, second) = (glyphs(&first, 0), glyphs(&second, by));
    let joined = if forward { [first, second] } else { [second, first] }.concat();
    let differs = whole.iter().zip(&joined).position(|(a, b)| a != b);
    let k = differs.or((whole.len() != joined.len()).then(|| whole.len().min(joined.len())))?;
    Some(format!("broken at byte {at}: glyph {k} (id, cluster, advance, offset) is {:?} whole, {:?} apart, of {} \
                  and {}", whole.get(k), joined.get(k), whole.len(), joined.len()))
}

// The features over bytes `start..end` of the text, counted from `start`, as a layout gives a piece it shapes alone:
// one over all of it is global.
fn features_over(features: &[Feature], start: usize, end: usize) -> Vec<Feature> {
    let over = features.iter().filter(|f| f.bytes.start < end && f.bytes.end > start);
    over.map(|f| match f.bytes.start <= start && f.bytes.end >= end {
        true => Feature::new(f.tag, f.value),
        false => {
            let mut piece = *f;
            piece.bytes = (f.bytes.start.max(start) - start..f.bytes.end.min(end) - start).into();
            piece
        }
    })
    .collect()
}

// The run's promise kept at up to `most` of the breaks it calls safe, spread through it: a cluster's start, past the
// text's, where none of its glyphs is unsafe.
pub fn check_breaks(f: &Font, loc: &Location, text: &str, options: &ShapeOptions, run: &ShapedRun, most: usize) {
    let mut unsafe_at = std::collections::BTreeMap::new();
    for (&k, &flag) in run.clusters.iter().zip(&run.unsafe_to_break) {
        *unsafe_at.entry(k as usize).or_insert(false) |= flag;
    }
    let breaks: Vec<usize> = unsafe_at.into_iter().filter(|&(k, flag)| k > 0 && !flag).map(|(k, _)| k).collect();
    let step = breaks.len().div_ceil(most.max(1)).max(1);
    for &at in breaks.iter().step_by(step) {
        let apart = broken_apart(f, loc, text, options, run, at);
        let what = "a break called safe isn't";
        must!(apart.is_none(), "shape({text:?}) with {options:?}: {what}: {}", apart.unwrap_or_default());
    }
}

pub fn finite(points: &[daegun::outline::Point]) -> bool {
    points.iter().all(|p| p.x.is_finite() && p.y.is_finite())
}

// The helpers a rasterizer of the caller's own is handed, under every option: finite, within their
// documented bounds, and the same answer twice.
pub fn check_helpers(f: &Font, g: GlyphId, at: &Location) {
    use daegun::outline::{Cap, Join, OutlineOptions, StrokeStyle};
    check_quads(f, g, at);
    let plain = OutlineOptions::default();
    let options = [
        plain,
        plain.with_hinting(HintMode::Auto),
        plain.with_transform([0.9, 0.1, -0.2, 1.1, 3.0, -2.0]),
        plain.with_oblique(0.2),
        plain.with_stroke(StrokeStyle::new(30.0).with_join(Join::Round).with_cap(Cap::Round)),
        plain.with_stroke(StrokeStyle::new(30.0).with_cap(Cap::Square)),
        plain.with_embolden(40.0),
        plain.with_hinting(HintMode::Auto).with_embolden(40.0),
        // Auto leaves a font with its own hints to them, so only AutoForce takes every font to the autohinter.
        plain.with_hinting(HintMode::AutoForce),
    ];
    let mut plain_box = None;
    for (i, opts) in options.iter().enumerate() {
        check_prepared(f, g, at, 18.0, i, opts, &mut plain_box);
    }

    check_scene(f, g, at);
}

// Quads twice and into a list still holding some, the same each way, finite, a cubic's at most 256.
pub fn check_quads(f: &Font, g: GlyphId, at: &Location) {
    use daegun::outline::Path;
    let quads = f.quads(g, at);
    must!(quads == f.quads(g, at), "quads {g} is not idempotent");
    if let Ok(q) = &quads {
        // A segment gives one quad, a cubic up to 256, and a contour one more where it's closed by a line.
        let mut path = Path::default();
        let segments = f.outline(g, at, &mut path).map_or(0, |()| path.segments().count());
        must!(!q.is_empty() && q.len() <= 256 * segments, "quads {g}: {} curves of {segments} segments", q.len());
        let finite = |p: daegun::outline::Point| p.x.is_finite() && p.y.is_finite();
        let all_finite = q.iter().all(|q| [q.start, q.control, q.end].into_iter().all(finite));
        must!(all_finite, "quads {g}: a point is not finite");
    }
    // Over a list still holding curves, which quads_into must not keep.
    let mut kept = quads.clone().unwrap_or_default();
    let into = f.quads_into(g, at, &mut kept);
    must!(into.is_ok() == quads.is_ok() && kept == quads.as_deref().unwrap_or_default(), "quads_into {g} != quads");
}

// One prepared outline at `px` under `opts`, the `i`th tried: the same twice, finite, within a stroke's reach of the
// plain outline's box, which the plain one, first, leaves in `plain_box`; then flattened and its overlaps resolved.
pub fn check_prepared(
    f: &Font, g: GlyphId, at: &Location, px: f32, i: usize, opts: &OutlineOptions,
    plain_box: &mut Option<(f32, f32, f32, f32)>,
) {
    use daegun::outline::{Join, Path};
    let (mut a, mut b) = (Path::default(), Path::default());
    let (ga, gb) = (f.prepared_outline(g, at, px, opts, &mut a), f.prepared_outline(g, at, px, opts, &mut b));
    must!(ga == gb && a == b, "prepared_outline {g}, options {i}: not idempotent");
    let Ok(prepared) = ga else { return };
    // The box of the plain outline's points, its control points too, so holding its curves and what an embolden
    // replays of them.
    if i == 0 && !a.is_empty() {
        let grow = |(x0, y0, x1, y1): (f32, f32, f32, f32), p: &daegun::outline::Point| {
            (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y))
        };
        *plain_box = Some(a.parts().1.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), grow));
    }
    // A stroke, or an embolden above 0, reaches past the outline by half its width, its sign ignored: √2 of that at
    // a square end and the limit at a miter's tip, 4 where it's not above 0. The lean shears the outline first.
    let miter = |limit: f32| if limit > 0.0 { limit } else { 4.0 };
    let ink = opts.stroke.map(|s| (s.width.abs(), if let Join::Miter { limit } = s.join { miter(limit) } else { 1.0 }));
    let (width, limit) = ink.or(opts.embolden.filter(|e| *e > 0.0).map(|e| (e, 1.0))).unwrap_or((0.0, 1.0));
    if let (Some(b), HintMode::None, None) = (*plain_box, opts.hinting, opts.transform) {
        let upm = f32::from(f.units_per_em());
        let reach = f64::from(width * 0.5 * limit.max(core::f32::consts::SQRT_2) * px / upm) + 0.01;
        let (x0, y0, x1, y1) = (f64::from(b.0), f64::from(b.1), f64::from(b.2), f64::from(b.3));
        let k = f64::from(opts.oblique.unwrap_or(0.0));
        let (xs, ys) = (x0 + (k * y0).min(k * y1) - reach..=x1 + (k * y0).max(k * y1) + reach, y0 - reach..=y1 + reach);
        let within = |p: &daegun::outline::Point| xs.contains(&f64::from(p.x)) && ys.contains(&f64::from(p.y));
        must!(a.parts().1.iter().all(within), "prepared_outline {g}, options {i}: a point past the stroke's reach");
    }
    must!(prepared.advance_width.is_finite() && prepared.advance_height.is_finite(),
          "prepared_outline {g}, options {i}: advance {:?}", prepared);
    must!(finite(a.parts().1), "prepared_outline {g}, options {i}: a point is not finite");
    must!(a.segments().map(|(verb, _)| verb).eq(a.parts().0.iter().copied())
              && a.segments().flat_map(|(_, points)| points.iter().copied()).eq(a.parts().1.iter().copied()),
          "segments {g}, options {i}: not the path's parts");
    let flat = a.flatten(0.05);
    must!(a.flatten(0.05) == flat, "flatten {g}, options {i}: not idempotent");
    let Ok(contours) = flat else { return };
    let points: usize = contours.iter().map(Vec::len).sum();
    must!(points <= daegun::outline::MAX_FLATTEN_POINTS, "flatten {g}, options {i}: {points} points");
    must!(contours.iter().all(|c| c.len() >= 3 && finite(c)), "flatten {g}, options {i}: a degenerate contour");
    let resolved = daegun::outline::resolve_overlaps(&contours);
    must!(daegun::outline::resolve_overlaps(&contours) == resolved,
          "resolve_overlaps {g}, options {i}: not idempotent");
    if let Some(resolved) = resolved {
        must!(!resolved.is_empty() && resolved.iter().all(|c| finite(c)), "resolve_overlaps {g}, options {i}");
    }
}

// A caret resolves or not whichever axis it is read on.
pub fn check_carets(f: &Font, g: GlyphId, at: &Location) {
    let across = f.ligature_carets(g, at, Orientation::Horizontal);
    let upright = f.ligature_carets(g, at, Orientation::Vertical);
    must!(across.iter().chain(&upright).flatten().all(|v| v.is_finite()),
          "ligature_carets {g} not finite: {across:?} {upright:?}");
    must!(across.iter().map(Option::is_some).eq(upright.iter().map(Option::is_some)),
          "ligature_carets {g} resolves different carets by direction: {across:?} {upright:?}");
}

// Pairs each enter with a leave, as the trait promises.
#[derive(Default)]
struct Balance {
    entered: usize,
    depth: usize,
}

impl PaintVisitor for Balance {
    fn enter(&mut self, _: PaintNode<'_>) {
        self.entered += 1;
        self.depth += 1;
    }
    fn leave(&mut self) {
        must!(self.depth > 0, "a paint left before it was entered");
        self.depth -= 1;
    }
}

pub fn check_paints(f: &Font, g: GlyphId, at: &Location) {
    let graph = f.paint_graph(g, at, 0);
    let mut seen = Balance::default();
    must!(f.visit_paint_graph(g, at, 0, &mut seen) == graph.is_some(), "visit_paint_graph {g} != paint_graph");
    must!(seen.depth == 0, "visit_paint_graph {g}: {} paints never left", seen.depth);
    must!(graph.is_some() || seen.entered == 0, "visit_paint_graph {g}: paints of no graph");
    if let Some(b) = f.colr_clip_box(g, at) {
        must!([b.x_min, b.y_min, b.x_max, b.y_max].iter().all(|v| v.is_finite()), "colr_clip_box {g}: not finite");
    }
}

pub fn check_scene(f: &Font, g: GlyphId, at: &Location) {
    check_paints(f, g, at);
    let options = SceneOptions::default();
    let first = f.color_scene(g, at, &options);
    must!(f.color_scene(g, at, &options) == first, "color_scene {g} is not idempotent");
    if let Ok(Some(scene)) = first {
        let points: usize = scene.paths().map(|p| p.parts().1.len()).sum();
        let table = |t| f.table(daegun::Tag::new(t)).map_or(0, <[u8]>::len);
        let bytes: usize = [b"glyf", b"CFF ", b"CFF2", b"VARC"].map(table).iter().sum();
        must!(points <= (1 << 20usize).max(16 * bytes), "color_scene {g}: {points} points");
        check_ops(&scene, &format!("color_scene {g}"), true);
    }
}

// A display list's ops name paths and gradients it holds, its pushes and pops balanced, 128 deep at most; and where
// it was made from finite numbers alone, every number in it finite.
pub fn check_ops(list: &DisplayList, what: &str, finite_in: bool) {
    let finite = |points: &[daegun::outline::Point]| !finite_in || finite(points);
    let finite_t = |t: &[f64; 6]| !finite_in || t.iter().all(|v| v.is_finite());
    let (mut depth, mut deepest) = (0i64, 0i64);
    for op in list.ops() {
        match op {
            Op::Fill { path, paint, transform, .. } => {
                must!(list.path(*path).is_some_and(|p| finite(p.parts().1)) && finite_t(transform),
                      "{what}: a bad fill path");
                if let Paint::Gradient(id) = paint {
                    must!(list.gradient(*id).is_some(), "{what}: a fill names a missing gradient");
                }
            }
            Op::PushClip { shapes } => {
                must!(list.shapes(*shapes).iter()
                          .all(|s| list.path(s.path).is_some_and(|p| finite(p.parts().1)) && finite_t(&s.transform)),
                      "{what}: a bad clip path");
                depth += 1;
            }
            Op::PushLayer { opacity, .. } => {
                must!(!finite_in || opacity.is_finite(), "{what}: a layer's opacity is {opacity}");
                depth += 1;
            }
            Op::PopClip | Op::PopLayer => depth -= 1,
            other => must!(false, "{what}: an op the fuzzer doesn't know, {other:?}"),
        }
        must!(depth >= 0, "{what}: a pop before its push");
        deepest = deepest.max(depth);
    }
    must!(depth == 0, "{what}: {depth} pushes never popped");
    must!(deepest <= 128, "{what}: nested {deepest} deep");
}

// An image's rows lie within its pixels, each pixel the bytes its kind has, a color one premultiplied; packed.
pub fn sound(image: Image<'_>, channels: &[u8], what: &str) -> Vec<u8> {
    must!(image.width <= MAX_SIDE && image.height <= MAX_SIDE && channels.contains(&image.channels),
          "{what}: {}x{}, {} channels", image.width, image.height, image.channels);
    let row = image.width as usize * usize::from(image.channels);
    let rows = |h: u32| image.stride * (h as usize - 1) + row;
    must!(image.height == 0 || (image.stride >= row && image.pixels.len() >= rows(image.height)),
          "{what}: {} bytes for {} rows of {row}, {} apart", image.pixels.len(), image.height, image.stride);
    let packed: Vec<u8> = (0..image.height).flat_map(|y| image.row(y).iter().copied()).collect();
    if image.channels == 4 {
        must!(packed.chunks(4).all(|p| p[..3].iter().all(|&c| c <= p[3])), "{what}: a pixel isn't premultiplied");
    }
    packed
}

// Every kind of image from a hostile font: an image or an error, never a panic, the same twice, and a raster cache
// hit the image drawn cold.
pub fn check_raster(f: &Font, r: &mut Rasterizer, g: GlyphId, at: &Location) {
    let rgb = SubpixelLayout::horizontal(StripeOrder::Rgb);
    let srgb = Gamma::Srgb { text: Rgba::opaque(30, 30, 30), contrast: 0.5 };
    let (o, outline) = (RasterOptions::DEFAULT, OutlineOptions::DEFAULT);
    let sets = [
        o,
        o.with_layout(&rgb).with_darken(true).with_offset(0.5, 0.25),
        o.with_gamma(srgb).with_outline(outline.with_hinting(HintMode::Auto)),
        o.with_outline(outline.with_stroke(StrokeStyle::new(40.0)).with_oblique(0.3)),
        o.with_outline(outline.with_embolden(30.0).with_transform([1.0, 0.5, -0.5, 1.0, 3.0, 3.0])),
    ];
    for (i, options) in sets.iter().enumerate() {
        let what = format!("glyph {g}, options {i}");
        let first = r.glyph(f, g, at, 13.5, options).map(|im| sound(im, &[1, 3], &what));
        let again = r.glyph(f, g, at, 13.5, options).map(|im| sound(im, &[1, 3], &what));
        must!(first.as_ref().ok() == again.as_ref().ok(), "{what} drew twice apart");
    }
    for px in [0.0, -4.0, f32::NAN, f32::INFINITY] {
        must!(r.glyph(f, g, at, px, &o).is_err(), "glyph {g} drew at {px} px");
    }
    let _ = r.glyph(f, g, at, 1e6, &o).map(|im| sound(im, &[1], "a glyph at a million px"));
    let scene = SceneOptions::default().with_palette(1).with_foreground(Rgba::opaque(200, 40, 0));
    let _ = r.color_glyph(f, g, at, 17.0, &o.with_offset(0.25, 0.75), &scene)
        .map(|im| im.map(|im| sound(im, &[4], &format!("color glyph {g}"))));
    // Drawn into a window across the origin, at sizes no lent image reaches too.
    let mut window = vec![0u8; 48 * 48 * 4];
    for px in [13.5, 1e6] {
        let _ = r.glyph_into(f, g, at, px, &o, Target::new(-24, 24, 48, 48, 48, &mut window[..48 * 48]));
        let _ = r.color_glyph_into(f, g, at, px, &o, &scene, Target::new(-24, 24, 48, 48, 48 * 4, &mut window));
    }
    for (field, channels) in [(Field::Sdf { spread: 2.0 }, 1), (Field::Msdf { spread: 2.0 }, 3)] {
        let _ = r.distance_field(f, g, at, 12.0, field).map(|im| sound(im, &[channels], &format!("{field:?} {g}")));
    }
    let mut path = daegun::outline::Path::default();
    if f.outline(g, at, &mut path).is_ok() {
        let m = [0.02, 0.003, -0.004, 0.02, 1.5, 2.5];
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let _ = r.path(&path, rule, Some(&m)).map(|im| sound(im, &[1], &format!("glyph {g}'s path")));
            let _ = r.path_into(&path, rule, Some(&m), Target::new(-24, 24, 48, 48, 48, &mut window[..48 * 48]));
        }
    }

    let cold = r.glyph(f, g, at, 15.0, &sets[1]).map(|im| sound(im, &[3], "a cold draw"));
    f.set_cache_budget(Cache::Raster, 1 << 20);
    for _ in 0..2 {
        let kept = r.glyph(f, g, at, 15.0, &sets[1]).map(|im| sound(im, &[3], "a cached draw"));
        must!(kept.as_ref().ok() == cold.as_ref().ok(), "glyph {g} from the raster cache isn't the image drawn cold");
    }
    must!(f.cache_stats(Cache::Raster).bytes <= 1 << 20, "the raster cache holds more than its budget");
    f.clear_cache(Cache::Raster);
    f.set_cache_budget(Cache::Raster, 0);
    must!(f.cache_stats(Cache::Raster).entries == 0, "a raster cache of no budget holds images");
}

// What a font says of itself that no drawing or shaping call reads: palettes, scripts, baselines, justification.
pub fn check_tables(f: &Font, at: &Location, texts: &[String]) {
    let _ = (f.math_constants(at), f.localized_names(daegun::NameId::POSTSCRIPT_NAME, at));
    let _ = (f.palette_entry_labels(), f.has_glyph_free_base(), f.feature_tags(None, None));
    for palette in [0, 1, u16::MAX] {
        let _ = f.palette_colors(palette);
    }
    let scripts = f.script_tags();
    for &script in scripts.iter().take(4).chain(&[Tag::new(b"latn"), Tag::new(b"arab")]) {
        let _ = (f.language_system_tags(script), f.feature_tags(Some(script), None), f.justification_glyphs(script));
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let _ = f.base_script(script, at, orientation);
            let _ = f.base_extents(script, at, orientation, None, None);
        }
        let Some(levels) = f.justification_priorities(script, None) else { continue };
        for level in levels.iter().take(2) {
            for adjustment in [Adjustment::Shrink, Adjustment::Extend] {
                for text in texts.iter().take(2) {
                    let run = f.shape_justified(text, at, Orientation::Horizontal, level, adjustment);
                    check_run(&run, text, "shape_justified");
                }
            }
        }
    }
}

// The byte-level readers, which tools hand whatever a table holds, with ids and indexes past every end.
pub fn check_raw(f: &Font) {
    use daegun::raw::bytes::*;
    use daegun::raw::format::*;
    use daegun::raw::{outline_glyf_bytes, parse_loca};

    let n = f.glyph_count();
    let glyphs = [0, 1, n / 2, n, u32::MAX];
    let small = [0, 1, u16::MAX];
    let edges = [0, 1, 2, usize::MAX];
    let tags: Vec<Tag> = f.table_tags().collect();
    for table in tags.iter().filter_map(|&tag| f.table(tag)) {
        for start in [0, 4, 8, 12, 16, 32] {
            let Some(data) = table.get(start..) else { break };
            let data = &data[..data.len().min(1 << 16)];

            if let Ok(lookup) = Lookup::parse(data, n) {
                let _ = lookup.entries();
                for g in glyphs {
                    let _ = lookup.value(g);
                }
            }
            for extra in [0, 1, 2] {
                let Ok(states) = StateTable::parse(data, extra, n) else { continue };
                for g in glyphs {
                    let _ = states.class(g);
                }
                for state in small {
                    for class in small {
                        let _ = states.entry(state, class);
                    }
                }
            }
            let _ = Ankr::version(data);
            if let Ok(ankr) = Ankr::parse(data, n) {
                for g in glyphs {
                    let _ = ankr.point_count(g);
                    for index in small {
                        let _ = ankr.anchor_point(g, index);
                    }
                }
            }
            for g in glyphs {
                let _ = coverage_index(data, g);
            }

            let store = ItemVariationStore::parse(data, 0).ok();
            if let Some(store) = &store {
                for ivd in &store.ivd_data {
                    must!(ivd.row(ivd.rows()).is_none(), "an item variation data reads a row past its {}", ivd.rows());
                }
                for location in [&[][..], &[0.5], &[f64::NAN, f64::INFINITY, -2.0]] {
                    let scalars = store.region_scalars(location);
                    for index in glyphs {
                        let _ = store.delta(DeltaSetIndex { outer: index, inner: 0 }, &scalars);
                        let _ = store.delta(DeltaSetIndex { outer: 0, inner: index }, &scalars);
                        let _ = store.delta(DeltaSetIndex { outer: 0, inner: index }, &[]);
                    }
                }
            }
            if let Ok(map) = parse_delta_set_index_map(data, 0) {
                for index in edges {
                    let _ = delta_set_index_map_lookup(&map, index);
                }
            }
            let parsed = FeatureVariations::parse(data).into_iter();
            for variations in parsed.chain(edges.map(|at| FeatureVariations::at(data, at))) {
                for coords in [&[][..], &[0x4000], &[i32::MIN, i32::MAX]] {
                    let _ = variations.find(coords, None);
                    let _ = variations.find(coords, store.as_ref());
                }
                for feature in small {
                    let _ = variations.substitute(0, feature);
                    let _ = variations.substitute(u32::MAX, feature);
                }
            }

            for format in [0, 1, -1] {
                let offsets = parse_loca(data, format, n as usize);
                for g in glyphs {
                    let mut pen = daegun::outline::Path::default();
                    let _ = outline_glyf_bytes(data, format == 1, &offsets, g, &mut pen);
                }
            }

            let mut copy = data[..data.len().min(64)].to_vec();
            for off in edges.into_iter().chain([data.len(), data.len().saturating_sub(1)]) {
                let _ = (read_u16_be(data, off), read_i16_be(data, off), read_u24_be(data, off));
                let _ = (read_u32_be(data, off), search_records(off, 7, |i| read_u32_be(data, i)));
                let _ = (read_offset24(data, off), window::<4>(data, off), ot_round(off as f64));
                let _ = (control_point(data, off), records_fit(off, off, off, data.len()), coverage_glyphs(data, off));
                write_u16_be(&mut copy, off, 0xBEEF);
                write_i16_be(&mut copy, off, -1);
                write_u32_be(&mut copy, off, u32::MAX);
                write_offset24(&mut copy, off, usize::MAX);
            }
        }
    }
    let _ = (ot_round(f64::NAN), ot_round(f64::INFINITY), ot_round(f64::NEG_INFINITY));
}

pub fn check_layout(l: &TextLayout, text: &str) {
    must!(l.inline_size.is_finite() && l.block_size.is_finite(), "layout size not finite");
    for pair in l.lines.windows(2) {
        must!(pair[0].bytes.end == pair[1].bytes.start,
              "layout lines do not tile: {:?} then {:?}", pair[0].bytes, pair[1].bytes);
    }
    for ln in &l.lines {
        must!(text.get(ln.bytes).is_some(), "layout line {:?} is not a slice of the text", ln.bytes);
        must!(ln.baseline.is_finite() && ln.inline_size.is_finite(), "layout line geometry not finite");
        for r in &ln.runs {
            let slice = text.get(r.bytes);
            must!(slice.is_some(), "layout run {:?} is not a slice of the text", r.bytes);
            check_run(&r.run, slice.unwrap_or_default(), "layout run");
        }
    }
}

// Each way in to a face reads the same tables, and only these reach a collection's.
pub fn exercise(bytes: &[u8], texts: &[String]) {
    each_face(bytes, |f| exercise_font(f, texts));
}

// The first two faces, each opened both ways and checked alike, then handed to `body`, which drops it; about every
// other input's caches draw from a small pool, which a dropped font gives back whole.
pub fn each_face(bytes: &[u8], mut body: impl FnMut(Font)) {
    let faces = Font::face_count(bytes);
    must!(faces > 0 || Font::from_bytes(bytes).is_err(), "from_bytes opened what face_count counts none of");
    must!(Font::from_collection(bytes, faces.max(1)).is_err(), "a face past the {faces} counted opened");
    if let Err(e) = Font::from_bytes(bytes) {
        must!(!e.message().is_empty(), "an error of kind {:?} says nothing", e.kind());
    }
    let pool = (bytes.len() % 2 == 1).then(|| daegun::CachePool::new(256 << 10));
    for index in 0..faces.min(2) {
        let (copied, kept) = (Font::from_collection(bytes, index), Font::from_vec(bytes.to_vec(), index));
        must!(copied.is_ok() == kept.is_ok(), "face {index}: from_collection and from_vec disagree");
        let (Ok(copied), Ok(kept)) = (copied, kept) else { continue };
        must!(copied.table_tags().eq(kept.table_tags())
                  && copied.table_tags().all(|t| copied.table(t) == kept.table(t)),
              "face {index}: from_collection and from_vec read different tables");
        for cache in [Cache::Shape, Cache::Instance, Cache::Outline, Cache::Raster] {
            copied.set_cache_pool(cache, pool.as_ref());
        }
        body(copied);
        if let Some(pool) = &pool {
            must!(pool.stats() == daegun::CacheStats::default(), "a dropped font left the pool {:?}", pool.stats());
        }
    }
}

pub fn exercise_font(f: Font, texts: &[String]) {
    describe(&f);
    let coords: Vec<(Tag, f64)> = f.axes().into_iter().map(|a| (a.tag, a.max)).collect();
    let at = f.location(&coords);
    must!(f.location(&coords).normalized() == at.normalized(), "a location resolves two ways");
    must!(f.typographic_metrics(&at) == f.typographic_metrics(&at), "typographic_metrics not idempotent");
    check_tables(&f, &at, texts);
    check_raw(&f);
    let inst = check_instance(&f, &at);
    let probes = probes(f.glyph_count());
    // The probes rarely land on a color glyph or a ligature, so a spread of the font's glyphs is drawn
    // as scenes and has its carets read too.
    let mut rasterizer = Rasterizer::new();
    // Small, so what it learned of the glyphs is let go and learned again as they go by.
    rasterizer.set_cache_budget(64 << 10);
    check_spread(&f, &mut rasterizer, &at);
    for &g in &probes {
        check_glyph(&f, &mut rasterizer, g, &at);
    }
    check_derived(&f, &probes, &at);
    check_outline_cache(&f, &at);
    if !coords.is_empty() {
        check_instance_shapes(&f, &at, &inst, texts);
    }
    check_shaping(&f, &at, texts);
}

// What a font says of itself at its default: metrics, names, coverage, tables.
pub fn describe(f: &Font) {
    let n = f.glyph_count();
    let origin = Location::default();
    let _ = (f.units_per_em(), f.ascender(&origin), f.descender(&origin), f.cap_height(&origin), f.bounds());
    let _ = (f.pdf_flags(&origin), f.italic_angle(&origin), f.style(&origin), f.is_variable(), f.palettes());
    let _ = (f.line_metrics(&origin, Orientation::Horizontal), f.line_metrics(&origin, Orientation::Vertical));
    let _ = (f.os2(&origin), f.typographic_metrics(&origin), f.names(&origin), f.stat(&origin));
    must!(f.palette_count() == f.palettes().len(), "palette_count isn't the palettes listed");
    let mut visited = std::collections::BTreeMap::new();
    f.visit_names(&origin, |id, name| drop(visited.insert(id, name.to_string())));
    must!(visited == f.names(&origin), "visit_names lent other names than names gives");
    let _ = (f.table_tags().count(), f.glyph_names().count(), f.named_instances(), f.axes());
    let _ = f.default_vertical_origin();
    let _ = (f.tracking(12.0, Orientation::Horizontal), f.math_min_connector_overlap(), f.math_constants(&origin));
    // Ids past maxp's count are the cmap's, so coverage is checked against glyph_id rather than the count; and an
    // index of no allowance only costs time.
    let allowance = f.index_allowance();
    for budget in [allowance, 0] {
        f.set_index_allowance(budget);
        for (c, gid) in f.coverage().take(400) {
            must!(f.glyph_id(c) == Some(gid), "coverage: {c:?} -> {gid}, glyph_id {:?}, {budget} bytes", f.glyph_id(c));
            must!(f.has_glyph(c), "has_glyph({c:?}) is false but coverage maps it, allowance {budget}");
            let _ = f.variation_glyph_id(c, '\u{FE00}');
        }
    }
    f.set_index_allowance(allowance);
    must!(f.glyph_names().len() == n as usize,
          "glyph_names has {} entries for {n} glyphs", f.glyph_names().len());
    for orientation in [Orientation::Horizontal, Orientation::Vertical] {
        let m = f.line_metrics(&origin, orientation);
        must!(m.ascent.is_finite() && m.descent.is_finite() && m.line_height().is_finite(),
              "line_metrics({orientation:?}) is not finite");
    }

    for tag in [b"head", b"hhea", b"maxp", b"OS/2", b"post", b"cmap", b"glyf", b"loca", b"CFF ", b"GSUB", b"MATH"] {
        let _ = f.table(Tag::new(tag));
        let _ = f.has_table(Tag::new(tag));
    }

}

// The instance at `at`, the same error twice where it can't be built, and what it reads as the font reads there.
pub fn check_instance(f: &Font, at: &Location) -> Result<std::sync::Arc<[u8]>, String> {
    let origin = Location::default();
    // A location the instancer can't build is an answer, the same error from both calls.
    let inst = f.instance(at).map_err(|e| e.to_string());
    must!(inst == f.instance(at).map_err(|e| e.to_string()), "instance not idempotent");
    let built = f.instance_tables(at).and_then(|t| daegun::raw::build_font(&t)).map_err(|e| e.to_string());
    must!(built.as_deref() == inst.as_deref(), "instance_tables + build_font != instance");
    // What MVAR varies reads at a location as the instance has it there.
    if let Some(instance) = inst.as_ref().ok().and_then(|b| Font::from_bytes(b).ok()) {
        let metrics = |f: &Font, at: &Location| {
            let os2 = f.os2(at).map(|o| (o.win_metrics, o.typo_metrics));
            let lines = (f.line_metrics(at, Orientation::Horizontal), f.line_metrics(at, Orientation::Vertical));
            (f.ascender(at), f.descender(at), f.cap_height(at), lines, os2, f.typographic_metrics(at))
        };
        must!(metrics(f, at) == metrics(&instance, &origin), "metrics at a location differ from its instance's");
        // So does what the style pass rewrites, away from the default, where the font describes itself.
        if at.normalized().iter().any(|&v| v != 0.0) {
            let described = |f: &Font, at: &Location| {
                let names = (f.family_name(at), f.names(at), f.name(daegun::NameId::POSTSCRIPT_NAME, at));
                (f.italic_angle(at), f.pdf_flags(at), f.style(at), names, f.stat(at))
            };
            must!(described(f, at) == described(&instance, &origin), "names or style differ from the instance's");
        }
    }

    inst
}

// Glyph ids at the edges of the font and past them.
pub fn probes(n: u32) -> Vec<GlyphId> {
    [0, 1, n / 2, n.saturating_sub(1), n, n.wrapping_add(1), u16::MAX.into()]
        .into_iter().map(GlyphId).collect()
}

// A spread of the font's glyphs drawn as scenes and color images, their carets read.
pub fn check_spread(f: &Font, rasterizer: &mut Rasterizer, at: &Location) {
    let n = f.glyph_count();
    for g in (0..n).step_by((n / 64).max(1) as usize).map(GlyphId) {
        check_scene(f, g, at);
        check_carets(f, g, at);
        let _ = rasterizer.color_glyph(f, g, at, 9.0, &RasterOptions::DEFAULT, &SceneOptions::default())
            .map(|im| im.map(|im| sound(im, &[4], &format!("color glyph {g}"))));
    }
}

// One glyph through every per-glyph call.
pub fn check_glyph(f: &Font, rasterizer: &mut Rasterizer, g: GlyphId, at: &Location) {
    let origin = Location::default();
    let _ = (f.glyph_name(g), f.advance(g, at, Orientation::Vertical), f.glyph_class(g), f.has_overlap_flag(g));
    let _ = f.mark_attachment_class(g);
    if let Some(b) = f.glyph_bounds(g, at) {
        must!(b.x_min <= b.x_max && b.y_min <= b.y_max, "glyph_bounds {g}: inverted {b:?}");
        must!(b.x_min.is_finite() && b.y_min.is_finite() && b.x_max.is_finite() && b.y_max.is_finite(),
              "glyph_bounds {g}: not finite");
    }
    let _ = (f.vertical_origin(g, &origin), f.cff_hints(g, at));
    check_carets(f, g, at);
    f.advances(&[g], at, Orientation::Horizontal, &mut Vec::new());
    let _ = f.bitmap(g, 16);
    let _ = (f.colr_layers(g, 0), f.paint_graph(g, at, 0));
    let _ = (f.math_glyph_variants(g, at, Orientation::Vertical), f.math_italics_correction(g, at));
    let _ = (f.math_top_accent_attachment(g, at), f.math_is_extended_shape(g));
    let _ = f.math_kern(g, at, daegun::math::MathKernCorner::TopRight, 100.0);
    let _ = f.hinted_outline(g, at, 16.0, HintMode::Auto).expect("a finite px above zero");
    let _ = f.hinted_outline(g, at, 16.0, HintMode::AutoForce).expect("a finite px above zero");
    check_helpers(f, g, at);
    check_raster(f, rasterizer, g, at);
}

// The closure and subset of `probes`, every glyph in range of what they give.
pub fn check_derived(f: &Font, probes: &[GlyphId], at: &Location) {
    let n = f.glyph_count();
    let bound = bounded(f);
    if let Ok(closed) = f.glyph_closure(probes, at) {
        for &g in &closed {
            must!(!bound || g.0 < n, "glyph_closure -> {g} of {n}");
        }
        must!(closed.windows(2).all(|w| w[0] < w[1]), "glyph_closure is not ascending and unique");
        must!(n == 0 || closed.first() == Some(&GlyphId(0)), "glyph_closure dropped .notdef: {closed:?}");
    }
    if let Ok(sub) = f.subset(probes, at)
        && let Ok(out) = Font::from_bytes(&sub.font) {
        for &g in probes.iter().filter(|g| g.0 < n) {
            if let Some(new) = sub.renumbered(g) {
                must!(new.0 < out.glyph_count(), "subset: {g} -> {new} of {}", out.glyph_count());
            }
        }
    }

}

// A drawn glyph replayed, cleared and refused a budget, the same outline each way.
pub fn check_outline_cache(f: &Font, at: &Location) {
    let n = f.glyph_count();
    let drawn = |f: &Font, g: GlyphId| {
        let mut p = daegun::outline::Path::default();
        f.outline(g, at, &mut p).ok().map(|()| p)
    };
    if let Some(g) = (0..n.min(8)).map(GlyphId).find(|&g| drawn(f, g).is_some_and(|p| !p.is_empty())) {
        let cold = drawn(f, g);
        f.prewarm([g], at);
        must!(drawn(f, g) == cold, "a prewarmed replay of {g} differs from its decode");
        f.clear_cache(Cache::Outline);
        must!(drawn(f, g) == cold, "clearing the outline cache changed the outline of {g}");
        f.set_cache_budget(Cache::Outline, 0);
        must!(drawn(f, g) == cold, "no budget for outlines changed the outline of {g}");
        must!(f.cache_stats(Cache::Outline).bytes == 0, "an outline cache of no budget holds bytes");
    }

}

// A font at a location shapes as its instance does at its default.
pub fn check_instance_shapes(f: &Font, at: &Location, inst: &Result<std::sync::Arc<[u8]>, String>, texts: &[String]) {
    let origin = Location::default();
    let plain = ShapeOptions::default();
    if let Ok(inst) = inst
        && let Ok(inst_font) = Font::from_bytes(inst) {
        for text in texts.iter().take(3) {
            let a = f.shape(text, at, &plain).glyphs.clone();
            let b = inst_font.shape(text, &origin, &plain).glyphs.clone();
            must!(a == b, "shape at axes != shape of the instance on {text:?}: {a:?} vs {b:?}");
        }
    }

}

// Every way of shaping each text, the runs sound, and the layouts over them.
pub fn check_shaping(f: &Font, at: &Location, texts: &[String]) {
    let plain = ShapeOptions::default();
    let vertical = plain.with_direction(Direction::TopToBottom);
    let liga = [Feature::new(Tag::new(b"liga"), 0)];
    let kern = [Feature::new(Tag::new(b"kern"), 1)];
    let ranged: Vec<Feature> = ["liga[1:3]=0", "kern[2:]=0", "smcp[:1]"].iter().flat_map(|s| s.parse()).collect();
    let mut kept = ShapedRun::default();
    for text in texts.iter().map(String::as_str) {
        check_run(&f.shape(text, at, &plain), text, "shape");
        f.shape_into(text, at, &plain, &mut kept);
        must!(kept == *f.shape(text, at, &plain), "shape_into({text:?}) != shape");
        check_run(&f.shape(text, at, &vertical), text, "shape vertical");
        let runs = f.shape_bidi(text, at, BaseDirection::Auto, &plain);
        let mut seen: Vec<usize> = runs.iter().flat_map(|b| b.bytes).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        must!(seen.len() == before, "shape_bidi({text:?}): a byte is in two runs");
        for b in &runs {
            let slice = text.get(b.bytes);
            must!(slice.is_some(), "shape_bidi({text:?}): run {:?} is not a slice of the text", b.bytes);
            check_run(&b.run, slice.unwrap_or_default(), "shape_bidi");
        }
        let r = f.shape(text, at, &plain.with_script(Tag::new(b"latn")).with_features(&liga));
        check_run(&r, text, "shape with features");
        let r = f.shape(text, at, &plain.with_context("x", "y").with_features(&kern));
        check_run(&r, text, "shape in context");
        let flags = plain.with_report_unsafe_to_concat(true).with_report_tatweel_positions(true);
        check_run(&f.shape(text, at, &flags), text, "shape with flags");
        for ig in [Ignorables::Remove, Ignorables::Preserve] {
            let r = f.shape(text, at, &plain.with_ignorables(ig).with_invisible_glyph(GlyphId(1)));
            check_run(&r, text, "shape ignorables");
        }
        let width = f.shape(text, at, &plain).width();
        must!(width.is_finite(), "width({text:?}) is {width}");
        let c = f.caret_positions(text, at, Orientation::Horizontal);
        must!(c.len() == text.len() + 1, "caret_positions: {} entries for {} bytes", c.len(), text.len());
        for (i, v) in c.iter().enumerate() { must!(v.is_finite(), "caret[{i}] is {v}"); }
        let _ = f.glyph_ids(text);
        let _ = f.subset_text(text, at);

        for w in [f64::INFINITY, 3000.0, 1.0] {
            let options = LayoutOptions::default().with_max_inline_size(w).with_align(Align::Justify);
            check_layout(&f.layout(text, at, &options), text);
            let options = options.with_features(&ranged);
            check_layout(&f.layout(text, at, &options), text);
            check_layout(&f.layout(text, at, &options.with_writing_mode(WritingMode::VerticalRl)), text);
        }
        let options = JustifyOptions::new(Tag::new(b"latn"), 5000.0).with_tolerance(1.0);
        let _ = f.justify(text, at, Orientation::Horizontal, &options);
    }
}

pub fn texts_for(rng: &mut Rng) -> Vec<String> {
    const ALPHABET: &[char] = &[
        'A', 'a', 'f', 'i', ' ', '\n', '\t',
        'ا', 'ل', 'م', 'ه',                        // Arabic, all joining
        'ि', 'क', '्', 'ा',                          // Devanagari, with a virama
        'ก', '่', '้',                                // Thai, with tone marks
        '日', '本',                                   // CJK
        '\u{0301}', '\u{0327}', '\u{064B}',          // combining marks
        '\u{200D}', '\u{200C}',                      // ZWJ, ZWNJ
        '\u{200E}', '\u{200F}', '\u{061C}',          // LRM, RLM, ALM
        '\u{2066}', '\u{2069}',                      // isolates
        '(', ')', '«', '»', '.', '1', '2',
        '\u{0}', '\u{FFFD}', '\u{FEFF}',             // the ones that are not text
    ];
    let mut out: Vec<String> = ["", "A", "fi", "سلام (a) سلام", "हिन्दी", "日本語", "  \n  "]
        .into_iter().map(String::from).collect();
    for _ in 0..4 {
        let len = match rng.below(4) { 0 => rng.below(3), 1 | 2 => rng.below(12), _ => rng.below(120) };
        out.push((0..len).map(|_| ALPHABET[rng.below(ALPHABET.len())]).collect());
    }
    out
}

// What the three parsers take, valid and nearly so: refused, or read back the same from what they print.
pub fn check_syntax(rng: &mut Rng) {
    const SEEDS: &[&str] = &[
        "", "a", "kern", "-liga", "+smcp", "aalt=2", "kern[3:5]=0", "liga[:]", "kern[3]", "kern[3:]", "smcp[:5]=1",
        "liga=off", "aalt=4294967295", "kern[18446744073709551615:]", "ltr", "rtl", "ttb", "btt", "    ", "OS/2",
    ];
    const SWAPS: &[char] = &['k', 'a', '[', ']', ':', '=', '+', '-', '0', '9', ' ', '\u{0}', '\u{7F}', 'é', '語'];
    for _ in 0..16 {
        let mut chars: Vec<char> = SEEDS[rng.below(SEEDS.len())].chars().collect();
        for _ in 0..rng.below(3) {
            let swap = SWAPS[rng.below(SWAPS.len())];
            match rng.below(3) {
                0 if !chars.is_empty() => { let at = rng.below(chars.len()); chars[at] = swap; }
                1 if !chars.is_empty() => { chars.remove(rng.below(chars.len())); }
                _ => chars.insert(rng.below(chars.len() + 1), swap),
            }
        }
        let text: String = chars.into_iter().collect();
        if let Ok(feature) = text.parse::<Feature>() {
            must!(feature.to_string().parse::<Feature>().ok() == Some(feature), "{text:?} -> {feature} reads apart");
        }
        if let Ok(tag) = text.parse::<Tag>() {
            must!(tag.to_string().parse::<Tag>().ok() == Some(tag), "{text:?} -> {tag} reads otherwise");
        }
        let _ = text.parse::<Direction>();
    }
}
