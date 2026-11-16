use daegun::layout::{Align, BreakStrategy, JustifyOptions, LayoutOptions, TextOrientation, WritingMode};
use daegun::outline::{Cap, HintMode, Join, OutlineOptions, StrokeStyle};
use daegun::raster::{Field, Gamma, RasterOptions, StripeOrder, SubpixelLayout, MAX_OVERSAMPLE, MAX_TAPS};
use daegun::shape::{ClusterLevel, Direction, Feature, ShapeOptions};
use daegun::unicode::{BaseDirection, Script, WordSegmenter};
use daegun::{Font, GlyphId, Location, Tag};

/// The bytes of choices an input starts with.
pub const CHOICES: usize = 32;

/// An input split: the choices, a text, and the font after them.
pub struct Input<'a> {
    pub choices: Choices,
    pub text: String,
    pub font: &'a [u8],
}

/// Where an input's choices, text and font lie: `[choices][u16 text length][text][font]`, cut short where it is.
pub fn sections(data: &[u8]) -> [core::ops::Range<usize>; 3] {
    let choices = 0..data.len().min(CHOICES);
    let len = data.get(CHOICES..CHOICES + 2).map_or(0, |b| usize::from(u16::from_be_bytes([b[0], b[1]])));
    let text_start = data.len().min(CHOICES + 2);
    let text = text_start..data.len().min(text_start + len);
    [choices, text.clone(), text.end..data.len()]
}

// What a font file starts with, which is read whole as the font so a plain file is an input too.
const MAGICS: [&[u8]; 5] = [b"\0\x01\0\0", b"OTTO", b"true", b"ttcf", b"typ1"];

impl<'a> Input<'a> {
    pub fn parse(data: &'a [u8]) -> Input<'a> {
        if MAGICS.iter().any(|m| data.starts_with(m)) {
            return Input { choices: Choices::default(), text: String::new(), font: data };
        }
        let [choices, text, font] = sections(data);
        let mut bytes = [0; CHOICES];
        bytes[..choices.len()].copy_from_slice(&data[choices]);
        Input { choices: Choices(bytes), text: String::from_utf8_lossy(&data[text]).into_owned(), font: &data[font] }
    }

    /// The input's own text and one mixing scripts, bidi and marks, which every target shapes.
    pub fn texts(&self) -> Vec<String> {
        vec![self.text.clone(), "fi سلام (a) हिन्दी 日本".to_string()]
    }
}

/// What a target does with a font, each byte deciding one thing so a mutation changes one decision.
#[derive(Clone, Copy, Default, Debug)]
pub struct Choices(pub [u8; CHOICES]);

// Sizes in pixels, the last three refused.
const PX: [f32; 16] =
    [0.5, 1.0, 6.0, 9.0, 12.0, 13.5, 16.0, 24.0, 48.0, 96.0, 256.0, 1000.0, 1e5, 0.0, -1.0, f32::NAN];
// Stroke widths and embolden strengths in font units, the last three ones a caller may pass.
const UNITS: [f32; 8] = [1.0, 12.0, 40.0, 200.0, 5000.0, -10.0, f32::NAN, f32::INFINITY];
// Features shapers and fonts name most, with ranges and values in `features`.
const FEATURES: [&[u8; 4]; 24] = [
    b"liga", b"kern", b"calt", b"smcp", b"c2sc", b"dlig", b"ss01", b"ss20", b"aalt", b"salt", b"swsh", b"frac",
    b"onum", b"tnum", b"zero", b"locl", b"ccmp", b"mark", b"mkmk", b"vert", b"rlig", b"init", b"fina", b"curs",
];
const SCRIPTS: [&[u8; 4]; 12] =
    [b"latn", b"arab", b"deva", b"dev2", b"thai", b"hani", b"hebr", b"khmr", b"mym2", b"bng2", b"DFLT", b"zzzz"];
const LANGUAGES: [&str; 6] = ["", "en", "ar", "hi", "tr", "x-not-a-language"];
// Scripts to seed shaping with, by ISO 15924 code, the last two no script daegun knows.
const ISO: [&str; 12] =
    ["Latn", "Arab", "Deva", "Thai", "Hang", "Hani", "Mymr", "Khmr", "Zyyy", "Zinh", "Zzzz", "Qaaa"];
const LEVELS: [ClusterLevel; 4] = [
    ClusterLevel::MonotoneGraphemes,
    ClusterLevel::MonotoneCharacters,
    ClusterLevel::Characters,
    ClusterLevel::Graphemes,
];

// Word segmenters a caller may hand layout: every byte a boundary, inside characters too; offsets out of order,
// repeated and past the run; none at all.
fn every_byte(run: &str, out: &mut Vec<usize>) {
    out.extend(0..=run.len());
}

fn disordered(run: &str, out: &mut Vec<usize>) {
    out.extend([run.len() + 1, usize::MAX, 3, 3, 0, run.len(), 1]);
}

fn nothing(_: &str, _: &mut Vec<usize>) {}

impl Choices {
    fn byte(&self, at: usize) -> u8 {
        self.0[at]
    }

    fn word(&self, at: usize) -> u16 {
        u16::from_be_bytes([self.0[at], self.0[at + 1]])
    }

    /// Two glyph ids, each as given, wrapped into the font, or at its edges and past them.
    pub fn glyphs(&self, n: u32) -> Vec<GlyphId> {
        let pick = |raw: u16, mode: u8| match mode % 8 {
            0 | 1 => u32::from(raw),
            2 | 3 => u32::from(raw) % n.max(1),
            4 => n.saturating_sub(1),
            5 => n,
            6 => n.wrapping_add(1),
            _ => 0,
        };
        let mode = self.byte(4);
        let mut out = vec![GlyphId(pick(self.word(0), mode)), GlyphId(pick(self.word(2), mode >> 4))];
        out.dedup();
        out
    }

    /// A location over the font's first six axes, each at an edge, past one, at its default, between, or not finite.
    pub fn location(&self, f: &Font) -> Location {
        let coords: Vec<(Tag, f64)> = f.axes().into_iter().take(6).enumerate().map(|(i, a)| {
            let (lo, hi) = (a.min, a.max);
            let v = match self.byte(5 + i) {
                0 => lo,
                255 => hi,
                128 => a.default,
                1 => lo - 1.0,
                254 => hi + 1.0,
                2 => f64::NAN,
                253 => f64::INFINITY,
                b => lo + (hi - lo) * f64::from(b) / 255.0,
            };
            (a.tag, v)
        }).collect();
        f.location(&coords)
    }

    pub fn px(&self) -> f32 {
        PX[usize::from(self.byte(11)) % PX.len()]
    }

    pub fn hinting(&self) -> HintMode {
        [HintMode::None, HintMode::Font, HintMode::Auto, HintMode::AutoForce][usize::from(self.byte(12) & 3)]
    }

    pub fn outline_options(&self) -> OutlineOptions {
        let (bits, units) = (self.byte(12), UNITS[usize::from(self.byte(13)) % UNITS.len()]);
        let mut o = OutlineOptions::default().with_hinting(self.hinting());
        if bits & 4 != 0 {
            let style = self.byte(14);
            let join = match style % 4 {
                0 => Join::Round,
                1 => Join::Bevel,
                2 => Join::Miter { limit: 4.0 },
                _ => Join::Miter { limit: f32::from(style >> 2) / 2.0 },
            };
            let cap = [Cap::Butt, Cap::Round, Cap::Square][usize::from(style >> 6) % 3];
            o = o.with_stroke(StrokeStyle::new(units).with_join(join).with_cap(cap));
        }
        if bits & 8 != 0 {
            o = o.with_embolden(units);
        }
        if bits & 16 != 0 {
            let s = f32::from(self.byte(14)) / 64.0 - 2.0;
            o = o.with_transform([1.0 + s, s / 3.0, -s / 2.0, 1.0 - s / 4.0, s * 100.0, -s * 50.0]);
        }
        if bits & 32 != 0 {
            o = o.with_oblique(f32::from(self.byte(14)) / 128.0 - 1.0);
        }
        o
    }

    pub fn subpixel(&self) -> SubpixelLayout {
        let order = if self.byte(15) & 2 != 0 { StripeOrder::Bgr } else { StripeOrder::Rgb };
        if self.byte(15) & 4 != 0 { SubpixelLayout::vertical(order) } else { SubpixelLayout::horizontal(order) }
    }

    pub fn raster_options<'l>(&self, layout: &'l SubpixelLayout) -> RasterOptions<'l> {
        let bits = self.byte(15);
        let mut o = RasterOptions::DEFAULT.with_outline(self.outline_options());
        if bits & 1 != 0 {
            o = o.with_layout(layout);
        }
        if bits & 8 != 0 {
            let contrast = f32::from(self.byte(16)) / 255.0;
            o = o.with_gamma(Gamma::Srgb { text: daegun::color::Rgba::opaque(30, 30, 30), contrast });
        }
        if bits & 16 != 0 {
            o = o.with_darken(true);
        }
        if bits & 32 != 0 {
            o = o.with_offset(f32::from(self.byte(16)) / 256.0, f32::from(self.byte(17)) / 256.0);
        }
        o
    }

    pub fn field(&self) -> Field {
        let spread = [0.5, 2.0, 8.0, 64.0, 0.0, -1.0, f32::NAN, 1e6][usize::from(self.byte(16) >> 5)];
        if self.byte(16) & 1 != 0 { Field::Msdf { spread } } else { Field::Sdf { spread } }
    }

    /// A window into the caller's pixels: its origin anywhere, a size up to 64, and a stride that may fall short.
    pub fn window(&self) -> (i32, i32, u32, u32, usize) {
        let origin = |b: u8| match b {
            0 => i32::MIN,
            255 => i32::MAX,
            b => (i32::from(b) - 128) * 4,
        };
        let (w, h) = (u32::from(self.byte(19) % 65), u32::from(self.byte(20) % 65));
        let short = usize::from(self.byte(20) >> 7);
        (origin(self.byte(17)), origin(self.byte(18)), w, h, (w as usize).saturating_sub(short))
    }

    /// Up to six features, each on, off, a value, or over a range of the text.
    pub fn features(&self) -> Vec<Feature> {
        (0..6).filter_map(|i| {
            let b = self.byte(21 + i / 2) >> (4 * (i % 2)) & 15;
            (b != 0).then(|| {
                let tag = Tag::new(FEATURES[(usize::from(self.byte(24)) + i * 5) % FEATURES.len()]);
                let mut f = Feature::new(tag, u32::from(b % 4));
                if b >= 8 {
                    f.bytes = (usize::from(b % 3)..usize::from(b)).into();
                }
                f
            })
        }).collect()
    }

    pub fn shape_options<'f>(&self, features: &'f [Feature]) -> ShapeOptions<'f> {
        let bits = self.byte(25);
        let direction =
            [Direction::LeftToRight, Direction::RightToLeft, Direction::TopToBottom, Direction::BottomToTop];
        let language = LANGUAGES[usize::from(bits >> 5) % LANGUAGES.len()];
        let mut o = ShapeOptions::default().with_features(features).with_language(language);
        if bits & 4 != 0 {
            o = o.with_direction(direction[usize::from(bits & 3)]);
        }
        if bits & 8 != 0 {
            o = o.with_script(Tag::new(SCRIPTS[usize::from(self.byte(26)) % SCRIPTS.len()]));
        }
        if bits & 16 != 0 {
            o = o.with_context("a\u{0644}", "\u{200D}b");
        }
        // From bytes the shape target reads for nothing else: the cluster level, the text's ends, a size, the glyphs
        // a missing character and selector take, and a script to start from.
        let more = self.byte(13);
        o = o.with_cluster_level(self.cluster_level()).with_beginning_of_text(more & 4 != 0)
            .with_suppress_dotted_circle(more & 8 != 0).with_point_size(f64::from(self.px()));
        if more & 16 != 0 {
            o = o.with_not_found_glyph(GlyphId(u32::from(self.word(0))))
                .with_not_found_variation_selector_glyph(GlyphId(u32::from(self.word(2))));
        }
        if let Some(script) = Script::from_iso15924(ISO[usize::from(self.byte(14)) % ISO.len()]) {
            o = o.with_seed_script(script);
        }
        o
    }

    pub fn cluster_level(&self) -> ClusterLevel {
        LEVELS[usize::from(self.byte(13) & 3)]
    }

    pub fn justify_options(&self) -> JustifyOptions {
        let script = Tag::new(SCRIPTS[usize::from(self.byte(26)) % SCRIPTS.len()]);
        let system = Tag::new(SCRIPTS[usize::from(self.byte(15)) % SCRIPTS.len()]);
        JustifyOptions::new(script, f64::from(self.word(29))).with_language_system(system)
    }

    pub fn layout_options<'f>(&self, features: &'f [Feature]) -> LayoutOptions<'f> {
        let bits = self.byte(27);
        let width = [f64::INFINITY, 3000.0, 400.0, 1.0, 0.0, -5.0, f64::NAN, 1e12][usize::from(bits & 7)];
        let align = [Align::Start, Align::End, Align::Center, Align::Justify][usize::from(bits >> 3) & 3];
        let modes = [WritingMode::Horizontal, WritingMode::VerticalRl, WritingMode::VerticalLr];
        let mode = modes[usize::from(bits >> 5) % 3];
        // From bytes the shape target reads for nothing else: orientation, base direction, language, strategy, line
        // and tab limits, and a segmenter that may answer anything.
        let more = self.byte(16);
        let orientation = [TextOrientation::Mixed, TextOrientation::Upright, TextOrientation::Sideways];
        let base = [BaseDirection::Auto, BaseDirection::LeftToRight, BaseDirection::RightToLeft];
        let strategy = if more & 16 != 0 { BreakStrategy::Optimal } else { BreakStrategy::Greedy };
        let lines = [usize::MAX, 0, 1, 2, 3, 7, 64, 1 << 20][usize::from(self.byte(18) & 7)];
        let tab = [8.0, 4.0, 0.0, -1.0, f64::NAN, f64::INFINITY, 1e9, 0.25][usize::from(self.byte(19) & 7)];
        let o = LayoutOptions::default().with_max_inline_size(width).with_align(align).with_writing_mode(mode)
            .with_features(features).with_line_height(f64::from(self.byte(26)) / 16.0)
            .with_text_orientation(orientation[usize::from(more & 3) % 3])
            .with_base_direction(base[usize::from(more >> 2 & 3) % 3])
            .with_language(LANGUAGES[usize::from(self.byte(17)) % LANGUAGES.len()])
            .with_strategy(strategy).with_max_lines(lines).with_tab_size(tab);
        match more >> 5 {
            1 => o.with_segmenter(WordSegmenter(&every_byte)),
            2 => o.with_segmenter(WordSegmenter(&disordered)),
            3 => o.with_segmenter(WordSegmenter(&nothing)),
            _ => o,
        }
    }

    /// A subpixel filter of the caller's, from bytes the raster target reads for nothing else: oversampling and taps
    /// within their limits, or anywhere where byte 27 says so, an origin, each channel's weights at the length asked,
    /// and now and then a channel short or a weight that isn't finite.
    pub fn custom_subpixel(&self) -> daegun::Result<SubpixelLayout> {
        let (b, taps, wild) = (self.byte(27), self.byte(29), self.byte(27) & 64 != 0);
        let size = |v: u8, most: u8| if wild { v } else { v % most + 1 };
        let (tx, ty) = (size(taps & 15, MAX_TAPS as u8), size(taps >> 4, MAX_TAPS as u8));
        let over = (size(b & 7, MAX_OVERSAMPLE), size(b >> 3 & 7, MAX_OVERSAMPLE));
        let n = usize::from(tx) * usize::from(ty);
        let weight = |i: usize| f32::from(self.byte(30).wrapping_add(i as u8)) / 255.0 - 0.2;
        let mut weights: Vec<Vec<f32>> = (0..3).map(|c| (0..n).map(|i| weight(i + c * n)).collect()).collect();
        match self.byte(31) % 8 {
            0 => drop(weights[1].pop()),
            1 => weights[2].push(0.5),
            2 if n > 0 => weights[0][0] = f32::NAN,
            _ => {}
        }
        let origin = ((self.byte(31) >> 3 & 7) as i8 - 4, (self.byte(31) >> 6) as i8 - 2);
        SubpixelLayout::from_weights(over, (tx, ty), origin, [&weights[0], &weights[1], &weights[2]])
    }

    /// A cache budget from nothing to a few megabytes.
    pub fn budget(&self) -> usize {
        [0, 1, 4 << 10, 64 << 10, 256 << 10, 1 << 20, 4 << 20, usize::MAX][usize::from(self.byte(28) & 7)]
    }

    /// The operation thread `thread` runs at its `step`th turn, of `ops`.
    pub fn op(&self, thread: usize, step: usize, ops: usize) -> usize {
        usize::from(self.byte(29 + thread % 3)).wrapping_mul(31).wrapping_add(step * 7 + thread) % ops
    }
}
