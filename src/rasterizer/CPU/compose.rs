// Color glyphs: a scene's ops drawn in linear light, premultiplied, through a stack of clips and layers, then kept as
// premultiplied sRGB bytes.
use alloc::vec::Vec;

use super::coverage::Edges;
use super::{fits, Drawn, Frame, Rasterizer, PAST, TOO_LARGE, UNFIT};
#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::daekit::base::error::{Error, Result};
use crate::daecore::daekit::draw::bitmap::MAX_SIDE;
use crate::daecore::daekit::draw::blend::{composite_row, paint_row};
use crate::daecore::daekit::draw::gradient::{premultiplied, Ramp};
use crate::daecore::daekit::draw::matrix::{concat, invert, transform_box, Matrix};
use crate::daecore::daekit::draw::srgb::encoded;
use crate::daecore::daekit::draw::Blend;
use crate::daecore::daetype::outline::{FillRule, OutlinePen, PathView};
use crate::daecore::daetype::paint::{DisplayList, Op, Paint};

// What a scene keeps between calls: its layers, the modes they composite by, its clips, each with the layer it was
// pushed in, a path's coverage, then a fill's weights, and a gradient's row.
#[derive(Default)]
pub(crate) struct Scene {
    layers: Vec<Vec<[f32; 4]>>,
    modes: Vec<(f32, Blend)>,
    clips: Vec<(usize, Vec<f32>)>,
    weights: Vec<f32>,
    row: Vec<[f32; 4]>,
}

// What a scene may take a pixel of its frame, in passes over a pixel of some 16 instructions: Noto Color Emoji's
// costliest glyph takes 73 a pixel at 1,368 px, and a crafted scene stays within seconds a megapixel.
pub(super) const PASSES: u64 = 1 << 10;
// What a scene may take besides, for its points and overlaps where its pixels are few: some two seconds' work.
pub(super) const BASE: u64 = 1 << 29;
// A point's share before its fill's: read, placed and cut.
const POINT: u64 = 16;
// An op's share past its pixels and points.
const OP: u64 = 192;
// A pixel's share of the frame, cleared and encoded.
const FRAME: u64 = 8;
const TOO_SLOW: Error = Error::limit("composing the scene would pass over each pixel more than 1,024 times");

// The box `list`'s fills cover once `m` places them, (x_min, y_min, x_max, y_max), and the box its fills and clips
// together reach; none where it fills nothing.
fn scene_boxes(list: &DisplayList, m: &Matrix) -> Option<([f64; 4], [f64; 4])> {
    let placed = |path, transform: &Matrix| {
        let b = list.path(path).and_then(|p: PathView<'_>| p.bounds())?;
        Some(transform_box(&concat(transform, m), [b.x_min, b.y_min, b.x_max, b.y_max]))
    };
    let add = |out: &mut Option<[f64; 4]>, t: [f64; 4]| *out = Some(out.map_or(t, |o| join(o, t)));
    let (mut fills, mut all) = (None, None);
    for op in list.ops() {
        match op {
            Op::Fill { path, transform, .. } => {
                if let Some(t) = placed(*path, transform) {
                    add(&mut fills, t);
                    add(&mut all, t);
                }
            }
            Op::PushClip { shapes } => {
                for shape in list.shapes(*shapes) {
                    if let Some(t) = placed(shape.path, &shape.transform) {
                        add(&mut all, t);
                    }
                }
            }
            _ => {}
        }
    }
    Some((fills?, all?))
}

fn join(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn meet(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

// A box, (x min, y min, x max, y max), drawn into `pen` as a rectangle.
fn rect(pen: &mut Edges, [x0, y0, x1, y1]: [f64; 4]) {
    for (k, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].into_iter().enumerate() {
        let (x, y) = (x as f32, y as f32);
        if k == 0 { pen.move_to(x, y) } else { pen.line_to(x, y) }
    }
    pen.close();
}

// What a scene shows once `m` places it: the box its fills cover inside `clip`, and that clip where it cuts them;
// none where nothing shows. Unfit where a box isn't finite.
#[allow(clippy::type_complexity, reason = "the box shown and the clip cutting it, each (x min, y min, x max, y max)")]
fn shown(list: &DisplayList, m: &Matrix, clip: Option<[f64; 4]>) -> Result<Option<([f64; 4], Option<[f64; 4]>)>> {
    let Some((fills, reach)) = scene_boxes(list, m) else { return Ok(None) };
    // A clip box holding the fills with a pixel to spare cuts nothing: it's left out.
    let grown = [fills[0] - 1.0, fills[1] - 1.0, fills[2] + 1.0, fills[3] + 1.0];
    let spare = invert(m).map(|inv| transform_box(&inv, grown));
    let holds = |c: &[f64; 4]| spare.is_some_and(|s| c[0] <= s[0] && c[1] <= s[1] && s[2] <= c[2] && s[3] <= c[3]);
    let clip = clip.filter(|c| !holds(c));
    let boxed = clip.map(|c| transform_box(m, c));
    let shown = boxed.map_or(fills, |c| meet(fills, c));
    if !reach.iter().chain(&shown).chain(boxed.iter().flatten()).all(|v| v.is_finite()) {
        return Err(UNFIT);
    }
    Ok((shown[0] <= shown[2] && shown[1] <= shown[3]).then_some((shown, clip)))
}

// A frame a scene is painted over, its top left corner `left` and `top` in pixels, y up, and its size.
#[derive(Clone, Copy)]
struct Area {
    left: f64,
    top: f64,
    width: usize,
    height: usize,
}

impl Rasterizer {
    /// `list` drawn into premultiplied sRGB RGBA, `m` taking its units to pixels, y up, inside `clip`, a box in its
    /// units, where there's one. A layer's fills take only the clips pushed inside it; its composite takes the clip
    /// it was pushed under, the box below them all.
    pub(crate) fn compose(&mut self, list: &DisplayList, m: &Matrix, clip: Option<[f64; 4]>) -> Result<Drawn> {
        let Some((shown, clip)) = shown(list, m, clip)? else { return Ok(Drawn::EMPTY_COLOR) };
        // The frame over what's shown, a pixel spare each way, within MAX_SIDE and the bytes a draw holds.
        let (left, bottom) = (shown[0].floor() - 1.0, shown[1].floor() - 1.0);
        let (right, top) = (shown[2].ceil() + 1.0, shown[3].ceil() + 1.0);
        if right - left > f64::from(MAX_SIDE) || top - bottom > f64::from(MAX_SIDE) {
            return Err(TOO_LARGE);
        }
        let within = |v: f64| (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&v);
        if !within(left) || !within(top) {
            return Err(PAST);
        }
        let area = Area { left, top, width: (right - left) as usize, height: (top - bottom) as usize };
        let n = area.width * area.height;
        fits(bytes(n, 1, usize::from(clip.is_some())))?;
        let mut allowance = PASSES.saturating_mul(n as u64).saturating_add(BASE);
        self.paint(list, m, clip, area, Some(n), &mut allowance)?;
        // The image: the pixels the fills cover inside the clip, cut from the work.
        let (x0, x1) = ((shown[0].floor() - left).max(0.0) as usize, (shown[2].ceil() - left).min(area.width as f64));
        let (y0, y1) = ((top - shown[3].ceil()).max(0.0) as usize, (top - shown[1].floor()).min(area.height as f64));
        let (across, down) = ((x1.max(0.0) as usize).saturating_sub(x0), (y1.max(0.0) as usize).saturating_sub(y0));
        if across == 0 || down == 0 {
            return Ok(Drawn::EMPTY_COLOR);
        }
        if self.pixels.len() < across * down * 4 {
            self.pixels.resize(across * down * 4, 0);
        }
        let image = self.pixels[..across * down * 4].chunks_exact_mut(across * 4);
        let work = self.scene.layers[0][..n].chunks_exact(area.width).skip(y0);
        for (out, row) in image.zip(work) {
            encode(&row[x0..x0 + across], out);
        }
        let (left, top) = (left as i32, top as i32);
        Ok(Drawn { left: left + x0 as i32, top: top - y0 as i32, width: across, height: down, channels: 4 })
    }

    /// `list` drawn as `compose` draws it into `pixels`, rows `stride` bytes apart of a region `width` by `height`
    /// pixels whose top left corner lies `left` across and `top` up from the origin, in bands of rows each worked
    /// out within BAND_BYTES; transparent where nothing shows.
    pub(crate) fn compose_into(
        &mut self, list: &DisplayList, m: &Matrix, clip: Option<[f64; 4]>, (left, top, width, height): Region,
        pixels: &mut [u8], stride: usize,
    ) -> Result<()> {
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return Ok(());
        }
        let Some((_, clip)) = shown(list, m, clip)? else {
            for y in 0..height {
                pixels[y * stride..][..width * 4].fill(0);
            }
            return Ok(());
        };
        let (mut depth, mut deepest, mut clips, mut most) = (1usize, 1usize, usize::from(clip.is_some()), 0usize);
        for op in list.ops() {
            match op {
                Op::PushLayer { .. } => depth += 1,
                Op::PopLayer => depth = depth.saturating_sub(1).max(1),
                Op::PushClip { .. } => clips += 1,
                Op::PopClip => clips = clips.saturating_sub(1),
                Op::Fill { .. } => {}
            }
            (deepest, most) = (deepest.max(depth), most.max(clips));
        }
        let rows = (BAND_BYTES / bytes(width, deepest, most).max(1)).clamp(1, height as u64) as usize;
        let mut allowance = PASSES.saturating_mul((width as u64).saturating_mul(height as u64)).saturating_add(BASE);
        for y in (0..height).step_by(rows) {
            let down = rows.min(height - y);
            let area = Area { left: f64::from(left), top: f64::from(top) - y as f64, width, height: down };
            self.paint(list, m, clip, area, None, &mut allowance)?;
            let work = self.scene.layers[0][..width * down].chunks_exact(width);
            for (r, row) in work.enumerate() {
                encode(row, &mut pixels[(y + r) * stride..][..width * 4]);
            }
        }
        Ok(())
    }

    // The scene painted over `area` into the first layer, its work out of `allowance`; `held` the pixels a lent
    // image's frame counts into MAX_DRAW_BYTES, as each layer and clip pushed is.
    fn paint(
        &mut self, list: &DisplayList, m: &Matrix, clip: Option<[f64; 4]>, area: Area, held: Option<usize>,
        allowance: &mut u64,
    ) -> Result<()> {
        let (width, n) = (area.width, area.width * area.height);
        let held = |layers: usize, clips: usize| held.map_or(Ok(()), |n| fits(bytes(n, layers, clips)));
        // Pixels from the frame's top left, y down, which a gradient samples at.
        let device = [m[0], -m[1], m[2], -m[3], m[4] - area.left, area.top - m[5]];
        let scene = &mut self.scene;
        scene.weights.resize(n.max(scene.weights.len()), 0.0);
        scene.row.resize(width.max(scene.row.len()), [0.0; 4]);
        scene.modes.clear();
        let (mut depth, mut clips) = (1usize, 0usize);
        spend(allowance, FRAME * n as u64)?;
        layer(&mut scene.layers, 0, n);
        if let Some(clip) = clip {
            if self.scene.clips.is_empty() {
                self.scene.clips.push((0, Vec::new()));
            }
            spend(allowance, n as u64)?;
            let mask = &mut self.scene.clips[0];
            mask.0 = 0;
            mask.1.clear();
            mask.1.resize(n, 0.0);
            let box_of = |e: &mut Edges| rect(e, clip);
            if let Some(at) = cover(self, &box_of, clip, FillRule::NonZero, m, area, allowance)? {
                let scene = &mut self.scene;
                for (r, w) in at.rows(&scene.weights) {
                    scene.clips[0].1[r * width + at.x..][..w.len()].copy_from_slice(w);
                }
            }
            clips = 1;
        }
        // The clip a layer's fills take: the last pushed, if it was pushed inside that layer.
        let clip_of = |clips: &[(usize, Vec<f32>)], at: usize, layer: usize| {
            at.checked_sub(1).filter(|&c| clips[c].0 == layer)
        };
        for op in list.ops() {
            spend(allowance, OP)?;
            match *op {
                Op::Fill { path, paint, rule, transform } => {
                    let Some(path) = list.path(path) else { continue };
                    let t = concat(&transform, m);
                    let drawn = |e: &mut Edges| path.replay(None, e);
                    let Some(at) = cover(self, &drawn, controls(path), rule, &t, area, allowance)? else {
                        continue;
                    };
                    spend(allowance, at.size())?;
                    let (color, ramp) = match paint {
                        Paint::Solid(color) => (premultiplied(color), None),
                        Paint::Gradient(id) => {
                            let Some(g) = list.gradient(id) else { continue };
                            ([0.0; 4], Some(Ramp::new(&g, &device)))
                        }
                    };
                    let scene = &mut self.scene;
                    let clip = clip_of(&scene.clips, clips, depth - 1);
                    for (r, w) in at.rows_mut(&mut scene.weights) {
                        let (from, to) = (r * width + at.x, r * width + at.x + w.len());
                        if let Some(c) = clip {
                            w.iter_mut().zip(&scene.clips[c].1[from..to]).for_each(|(w, k)| *w *= k);
                        }
                        let target = &mut scene.layers[depth - 1][from..to];
                        let Some(ramp) = &ramp else {
                            paint_row(color, w, target);
                            continue;
                        };
                        let Some(first) = w.iter().position(|&k| k > 0.0) else { continue };
                        let last = w.iter().rposition(|&k| k > 0.0).unwrap_or(first) + 1;
                        let row = &mut scene.row[..last - first];
                        ramp.fill_row(at.x + first, r, row);
                        composite_row(Blend::SrcOver, row, &w[first..last], &mut target[first..last]);
                    }
                }
                Op::PushClip { shapes } => {
                    held(depth, clips + 1)?;
                    spend(allowance, n as u64)?;
                    if self.scene.clips.len() <= clips {
                        self.scene.clips.push((0, Vec::new()));
                    }
                    let base = clip_of(&self.scene.clips, clips, depth - 1);
                    {
                        let (_, mask) = &mut self.scene.clips[clips];
                        mask.clear();
                        mask.resize(n, 0.0);
                    }
                    // A clip's shapes join: each pixel takes the most any of them covers.
                    for shape in list.shapes(shapes) {
                        let Some(path) = list.path(shape.path) else { continue };
                        let t = concat(&shape.transform, m);
                        let shape_of = |e: &mut Edges| path.replay(None, e);
                        if let Some(at) = cover(self, &shape_of, controls(path), shape.rule, &t, area, allowance)? {
                            spend(allowance, at.size())?;
                            let scene = &mut self.scene;
                            for (r, w) in at.rows(&scene.weights) {
                                let mask = &mut scene.clips[clips].1[r * width + at.x..][..w.len()];
                                mask.iter_mut().zip(w).for_each(|(k, &v)| *k = k.max(v));
                            }
                        }
                    }
                    let (below, above) = self.scene.clips.split_at_mut(clips);
                    if let Some(base) = base {
                        above[0].1[..n].iter_mut().zip(&below[base].1[..n]).for_each(|(k, b)| *k *= b);
                    }
                    above[0].0 = depth - 1;
                    clips += 1;
                }
                Op::PopClip => clips = clips.saturating_sub(1),
                Op::PushLayer { opacity, blend } => {
                    held(depth + 1, clips)?;
                    spend(allowance, 2 * n as u64)?;
                    layer(&mut self.scene.layers, depth, n);
                    self.scene.modes.push((opacity.clamp(0.0, 1.0), blend));
                    depth += 1;
                }
                Op::PopLayer => {
                    if depth > 1 {
                        pop(&mut self.scene, &mut depth, &mut clips, n);
                    }
                }
            }
        }
        while depth > 1 {
            pop(&mut self.scene, &mut depth, &mut clips, n);
        }
        Ok(())
    }
}

/// A region drawn into: its top left corner's pixels across and up from the origin, and its width and height.
pub(crate) type Region = (i32, i32, u32, u32);

// The bytes a band of a region drawn into is worked out within.
pub(crate) const BAND_BYTES: u64 = 1 << 24;


// A row of linear premultiplied RGBA as premultiplied sRGB bytes.
fn encode(row: &[[f32; 4]], out: &mut [u8]) {
    for (o, p) in out.as_chunks_mut::<4>().0.iter_mut().zip(row) {
        let a = p[3].clamp(0.0, 1.0);
        *o = if a > 0.0 {
            let level = |v: f32| (encoded((v / a).clamp(0.0, 1.0)) * a + 0.5) as u8;
            [level(p[0]), level(p[1]), level(p[2]), (a * 255.0 + 0.5) as u8]
        } else {
            [0; 4]
        };
    }
}

// What `n` pixels hold: each one's weight, coverage's sum and mark and its image, 4 bytes each, then its color in each
// of `layers` and its mask in each of `clips`.
fn bytes(n: usize, layers: usize, clips: usize) -> u64 {
    n as u64 * (16 + 16 * layers + 4 * clips) as u64
}

// Layer `at` of the pool, cleared to `n` transparent pixels.
fn layer(layers: &mut Vec<Vec<[f32; 4]>>, at: usize, n: usize) {
    if layers.len() <= at {
        layers.resize_with(at + 1, Vec::new);
    }
    layers[at].clear();
    layers[at].resize(n, [0.0; 4]);
}

// The top layer composited onto the one below by its mode and opacity, through the clip it was pushed under; clips
// pushed inside it and never popped go with it.
fn pop(scene: &mut Scene, depth: &mut usize, clips: &mut usize, n: usize) {
    *depth -= 1;
    let (opacity, mode) = scene.modes.pop().unwrap_or((1.0, Blend::SrcOver));
    while *clips > 0 && scene.clips[*clips - 1].0 >= *depth {
        *clips -= 1;
    }
    let clip = clips.checked_sub(1).filter(|&c| scene.clips[c].0 == *depth - 1);
    let weights = &mut scene.weights[..n];
    match clip {
        Some(c) => weights.copy_from_slice(&scene.clips[c].1[..n]),
        None => weights.fill(1.0),
    }
    let (below, above) = scene.layers.split_at_mut(*depth);
    let source = &mut above[0][..n];
    if opacity < 1.0 {
        source.iter_mut().for_each(|p| *p = p.map(|v| v * opacity));
    }
    composite_row(mode, source, weights, &mut below[*depth - 1][..n]);
}

// Where a fill's coverage lies in the frame: its box's left column and top row, and its width and height; the
// weights hold its rows packed.
#[derive(Clone, Copy)]
struct Patch {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Patch {
    fn size(&self) -> u64 {
        (self.width * self.height) as u64
    }

    // Each of its rows of `weights` with its frame row.
    fn rows<'a>(&self, weights: &'a [f32]) -> impl Iterator<Item = (usize, &'a [f32])> + use<'a> {
        let y = self.y;
        weights[..self.width * self.height].chunks_exact(self.width).enumerate().map(move |(r, w)| (y + r, w))
    }

    fn rows_mut<'a>(&self, weights: &'a mut [f32]) -> impl Iterator<Item = (usize, &'a mut [f32])> + use<'a> {
        let y = self.y;
        weights[..self.width * self.height].chunks_exact_mut(self.width).enumerate().map(move |(r, w)| (y + r, w))
    }
}

// The box of a path's points, its curves' controls among them, (x min, y min, x max, y max) in its own units.
fn controls(path: PathView<'_>) -> [f64; 4] {
    let b = path.parts().1.iter().fold([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], |b, p| {
        [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]
    });
    b.map(f64::from)
}

// f32 places a point within 2^-22 of its terms' magnitudes summed (the matrix, two products and two sums each
// rounded), so under NEAR px of them it's within 2^-10 px of where f64 puts it, a 32nd of what a chord may stray.
const NEAR: f64 = 4096.0;

// Whether `m` places the box `b` within NEAR, and inside `clip` by more than f32 may stray, so f32 places what's in it.
fn near(m: &Matrix, b: [f64; 4], clip: [f64; 4]) -> bool {
    let (x, y) = (b[0].abs().max(b[2].abs()), b[1].abs().max(b[3].abs()));
    let terms = |a: f64, c: f64, d: f64| a.abs() * x + c.abs() * y + d.abs();
    let [x0, y0, x1, y1] = transform_box(m, b);
    let e = 1.0 / 1024.0;
    terms(m[0], m[2], m[4]).max(terms(m[1], m[3], m[5])) < NEAR
        && x0 > clip[0] + e
        && y0 > clip[1] + e
        && x1 < clip[2] - e
        && y1 < clip[3] - e
}

// What `draw` draws within the box `controls` covers under `rule` once `m` places it, into the scene's weights over
// the pixels it reaches; none where it covers nothing. Placed in f32 as a lent draw is where that's `near`, else in
// f64 and cut to `area`. Its points, and its fill's work, are paid from `allowance`.
fn cover(
    r: &mut Rasterizer, draw: &dyn Fn(&mut Edges), controls: [f64; 4], rule: FillRule, m: &Matrix, area: Area,
    allowance: &mut u64,
) -> Result<Option<Patch>> {
    let local = [m[0], m[1], m[2], m[3], m[4] - area.left, m[5] - area.top];
    let clip = [0.0, -(area.height as f64), area.width as f64, 0.0];
    let near = near(&local, controls, clip);
    if near {
        r.edges.begin(local.map(|v| v as f32));
        draw(&mut r.edges);
        r.edges.finish();
    }
    // Past the points a lent draw holds, it's held cut as far ones are.
    if !near || r.edges.too_many() {
        r.edges.begin_cut(local, clip);
        draw(&mut r.edges);
        r.edges.finish();
    }
    if r.edges.unfit() {
        return Err(UNFIT);
    }
    spend(allowance, POINT * r.edges.points() as u64)?;
    let Some([x0, y0, x1, y1]) = r.edges.bounds() else { return Ok(None) };
    let (x, y) = ((x0.floor().max(0.0) as usize).min(area.width), (-y1.ceil()).max(0.0) as usize);
    let right = (x1.ceil().max(0.0) as usize).min(area.width);
    let bottom = ((-y0.floor()).max(0.0) as usize).min(area.height);
    let at = Patch { x, y: y.min(area.height), width: right.saturating_sub(x), height: bottom.saturating_sub(y) };
    if at.width == 0 || at.height == 0 {
        return Ok(None);
    }
    let frame = Frame { left: at.x as i32, top: -(at.y as i32), width: at.width, height: at.height };
    let weights = &mut r.scene.weights[..at.width * at.height];
    if !r.coverage.draw(&r.edges, rule, frame, Some((usize::MAX, allowance)), weights, at.width) {
        return Err(TOO_SLOW);
    }
    Ok(Some(at))
}

// `work` out of `allowance`, refused past it.
fn spend(allowance: &mut u64, work: u64) -> Result<()> {
    *allowance = allowance.checked_sub(work).ok_or(TOO_SLOW)?;
    Ok(())
}
