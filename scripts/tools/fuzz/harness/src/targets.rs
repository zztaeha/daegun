use daegun::metrics::PdfFlags;
use daegun::outline::{HintMode, Path};
use daegun::raster::{RasterOptions, Rasterizer, StripeOrder, SubpixelLayout, Target};
use daegun::shape::Feature;
use daegun::unicode::{self, BaseDirection, BidiParagraph, Script};
use daegun::{Font, Location, Orientation, Tag};

use crate::checks::*;
use crate::input::Input;
use crate::stack::{guard_thread, stack_size};
use crate::tools::*;

// A panic on one exact text, only where asked for, so a run can prove a finding goes all the way through.
fn canary(input: &Input) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("DAEGUN_FUZZ_CANARY").is_some()) && input.text == "canary!" {
        panic!("invariant: the canary");
    }
}

/// Opening a font and everything it says of itself, at a chosen location.
pub fn load(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    let texts = input.texts();
    each_face(input.font, |f| {
        describe(&f);
        let at = input.choices.location(&f);
        must!(f.typographic_metrics(&at) == f.typographic_metrics(&at), "typographic_metrics not idempotent");
        check_tables(&f, &at, &texts);
        check_raw(&f);
        let _ = (f.glyph_ids(&input.text), f.names(&at), f.style(&at), f.stat(&at));
        if let Some(os2) = f.os2(&at) {
            let _ = (os2.is_bold(), os2.is_italic(), os2.is_oblique(), os2.is_regular(), os2.uses_typo_metrics());
        }
        let flags = f.pdf_flags(&at);
        must!(flags.is_empty() == (flags.bits() == 0) && flags.contains(PdfFlags::empty()), "pdf_flags {flags:?}");
        must!(f.location(at.coords()).normalized() == at.normalized(), "a location made again from its coords moved");
        must!(f.table_tags().all(|t| Tag::new(&t.to_bytes()) == t), "a table's tag read back as bytes is another");
        let _ = [HintMode::None, HintMode::Font, HintMode::Auto, HintMode::AutoForce].map(HintMode::may_autohint);
    });
}

/// Chosen glyphs as outlines, plain and under the chosen options at the chosen size, as quads, hinted, and kept.
pub fn outline(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    each_face(input.font, |f| {
        let at = input.choices.location(&f);
        let (px, options) = (input.choices.px(), input.choices.outline_options());
        for g in input.choices.glyphs(f.glyph_count()) {
            check_quads(&f, g, &at);
            let mut plain_box = None;
            check_prepared(&f, g, &at, px, 0, &daegun::outline::OutlineOptions::default(), &mut plain_box);
            check_prepared(&f, g, &at, px, 1, &options, &mut plain_box);
            let hinted = f.hinted_outline(g, &at, px, input.choices.hinting());
            must!(hinted.is_ok() == (px.is_finite() && px > 0.0), "hinted_outline {g} at {px} px: {:?}",
                  hinted.as_ref().err());
            if let Ok(Some(h)) = &hinted {
                check_hinted(h, &format!("glyph {g} hinted at {px} px"));
            }
            check_path_tools(&f, g, &at, &input.choices);
            let _ = f.cff_hints(g, &at);
            if let Some(b) = f.glyph_bounds(g, &at) {
                must!(b.x_min <= b.x_max && b.y_min <= b.y_max, "glyph_bounds {g}: inverted {b:?}");
            }
        }
        check_outline_cache(&f, &at);
    });
}

/// Chosen glyphs drawn under the chosen options: twice alike, from the raster cache as drawn cold, into the caller's
/// pixels, as a field, in color and as a scene.
pub fn raster(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    each_face(input.font, |f| {
        let at = input.choices.location(&f);
        let mut r = Rasterizer::new();
        r.set_cache_budget(input.choices.budget());
        let layout = input.choices.subpixel();
        let options = input.choices.raster_options(&layout);
        let px = input.choices.px();
        let (x, y, w, h, stride) = input.choices.window();
        let custom = input.choices.custom_subpixel();
        if let Ok(custom) = &custom {
            check_subpixel(custom, "the input's filter");
            let again = input.choices.custom_subpixel().map(|l| l.key());
            must!(again.ok() == Some(custom.key()), "the input's filter made twice has two keys");
        }
        check_subpixel(&layout, "the chosen layout");
        check_subpixel(&SubpixelLayout::grayscale(), "grayscale");
        check_subpixel(&SubpixelLayout::unfiltered(StripeOrder::Bgr, Orientation::Vertical), "unfiltered");
        let mut sizes = vec![(0, 0), (1, usize::MAX), (usize::MAX, 1), (7, 7), (300, 40)];
        for g in input.choices.glyphs(f.glyph_count()) {
            let what = format!("glyph {g} at {px} px");
            let first = r.glyph(&f, g, &at, px, &options).map(|im| sound(im, &[1, 3], &what));
            let again = r.glyph(&f, g, &at, px, &options).map(|im| sound(im, &[1, 3], &what));
            must!(first.as_ref().ok() == again.as_ref().ok(), "{what} drew twice apart");
            must!(first.is_err() || (px.is_finite() && px > 0.0), "{what} drew at a size it should refuse");
            f.set_cache_budget(daegun::Cache::Raster, 1 << 20);
            for _ in 0..2 {
                let kept = r.glyph(&f, g, &at, px, &options).map(|im| sound(im, &[1, 3], &what));
                let same = kept.as_ref().ok() == first.as_ref().ok();
                must!(same, "{what} from the raster cache isn't the image drawn cold");
            }
            f.set_cache_budget(daegun::Cache::Raster, 0);
            let field = input.choices.field();
            let _ = r.distance_field(&f, g, &at, px, field).map(|im| sound(im, &[1, 3], &format!("{field:?} {g}")));
            let mut pixels = vec![0u8; stride.max(1) * 4 * h as usize];
            let gray = &mut pixels[..stride * h as usize];
            let _ = r.glyph_into(&f, g, &at, px, &options, Target::new(x, y, w, h, stride, gray));
            let scene = daegun::color::SceneOptions::default().with_palette(u16::from(input.choices.0[26] % 4));
            let _ = r.color_glyph(&f, g, &at, px, &options, &scene).map(|im| im.map(|im| sound(im, &[4], &what)));
            let target = Target::new(x, y, w, h, stride * 4, &mut pixels);
            let _ = r.color_glyph_into(&f, g, &at, px, &options, &scene, target);
            check_scene(&f, g, &at);
            check_paint_tools(&f, g, &at, &input.choices);
            if let Ok(image) = r.glyph(&f, g, &at, px, &options) {
                sizes.push((image.width as usize, image.height as usize));
                let packed = sound(image, &[1, 3], &what);
                must!(sound(image.to_bitmap().image(), &[1, 3], &what) == packed, "{what}: as a bitmap, other pixels");
            }
            if let Ok(custom) = &custom {
                let through = RasterOptions::DEFAULT.with_layout(custom);
                let _ = r.glyph(&f, g, &at, px, &through).map(|im| sound(im, &[1, 3], &format!("{what}, filtered")));
            }
        }
        check_color_values(&input.choices);
        check_packing(&input.choices, &sizes);
    });
}

/// The input's text shaped every way and under the chosen options, its safe breaks held to their promise, laid out
/// and justified.
pub fn shape(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    let texts = input.texts();
    let features: Vec<Feature> = input.choices.features();
    each_face(input.font, |f| {
        let at = input.choices.location(&f);
        check_shaping(&f, &at, &texts[..1]);
        let options = input.choices.shape_options(&features);
        let (level, layout) = (input.choices.cluster_level(), input.choices.layout_options(&features));
        let across = if layout.writing_mode.is_vertical() { Orientation::Vertical } else { Orientation::Horizontal };
        for text in &texts {
            let run = f.shape(text, &at, &options);
            check_run_at(&run, text, "shape with chosen options", level);
            check_breaks(&f, &at, text, &options, &run, 2);
            for base in [BaseDirection::Auto, BaseDirection::RightToLeft] {
                for b in f.shape_bidi(text, &at, base, &options) {
                    let slice = text.get(b.bytes);
                    must!(slice.is_some(), "shape_bidi({text:?}): run {:?} is not a slice of the text", b.bytes);
                    check_run_at(&b.run, slice.unwrap_or_default(), "shape_bidi with chosen options", level);
                }
            }
            check_layout(&f.layout(text, &at, &layout), text);
            let justified = f.justify(text, &at, across, &input.choices.justify_options());
            must!(justified.width.is_finite(), "justify({text:?}): width {}", justified.width);
            let carets = f.caret_positions(text, &at, Orientation::Vertical);
            must!(carets.len() == text.len() + 1, "caret_positions: {} entries for {} bytes", carets.len(), text.len());
        }
    });
}

/// Subsets, closures and instances at the chosen location, each written font opened again and used.
pub fn derive(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    each_face(input.font, |f| {
        let at = input.choices.location(&f);
        let n = f.glyph_count();
        let mut glyphs = input.choices.glyphs(n);
        glyphs.extend(probes(n));
        let inst = check_instance(&f, &at);
        check_derived(&f, &glyphs, &at);
        let origin = Location::default();
        if let Ok(bytes) = &inst {
            let opened = Font::from_bytes(bytes);
            must!(opened.is_ok(), "an instance written opens with {:?}", opened.err());
            if let Ok(out) = opened {
                use_written(&out, &input.text, &origin);
            }
        }
        if let Ok(sub) = f.subset(&glyphs, &at) {
            let opened = Font::from_bytes(&sub.font);
            must!(opened.is_ok(), "a subset written opens with {:?}", opened.err());
            if let Ok(out) = opened {
                use_written(&out, &input.text, &origin);
            }
        }
        if let Ok(sub) = f.subset_text(&input.text, &at) {
            let opened = Font::from_bytes(&sub.font);
            must!(opened.is_ok(), "a text's subset written opens with {:?}", opened.err());
            if let Ok(out) = opened {
                use_written(&out, &input.text, &origin);
            }
        }
    });
}

// A font daegun wrote, described, drawn and shaped.
fn use_written(f: &Font, text: &str, at: &Location) {
    describe(f);
    for g in probes(f.glyph_count()) {
        let mut path = Path::default();
        let _ = f.outline(g, at, &mut path);
    }
    check_run(&f.shape(text, at, &Default::default()), text, "shape a written font");
}

/// Unicode alone: bidi, segmentation, line breaks, scripts, and the text read as a feature, tag and direction.
pub fn text(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    let whole = String::from_utf8_lossy(data);
    for text in [input.text.as_str(), &whole] {
        check_text(text);
    }
}

fn check_text(text: &str) {
    let len = text.len();
    for base in [BaseDirection::Auto, BaseDirection::LeftToRight, BaseDirection::RightToLeft] {
        let para = BidiParagraph::new(text, base);
        must!(para.levels.len() == len, "bidi: {} levels for {len} bytes", para.levels.len());
        must!(para.levels.iter().all(|&l| l <= 126), "bidi: a level past 126");
        let mut order = para.visual_order.clone();
        order.sort_unstable();
        let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        must!(order == starts, "bidi: the visual order isn't each character once");
        let mut at = 0;
        for p in &para.paragraphs {
            must!(p.start == at && p.end > p.start && p.end <= len, "bidi: paragraphs don't tile, {} at {at}", p.start);
            let mut seen: Vec<usize> = para.visual_runs(text, p.start..p.end).iter()
                .flat_map(|r| { must!(text.get(r.bytes).is_some(), "bidi: a run off the text"); r.bytes }).collect();
            seen.sort_unstable();
            must!(seen == (p.start..p.end).collect::<Vec<_>>(), "bidi: a paragraph's runs don't cover it once");
            at = p.end;
        }
        must!(at == len, "bidi: paragraphs end at {at} of {len}");
    }
    for (name, cuts) in [("graphemes", unicode::grapheme_boundaries(text)), ("words", unicode::word_boundaries(text))] {
        must!(cuts.first() == Some(&0) && cuts.last() == Some(&len), "{name}: {cuts:?} for {len} bytes");
        must!(cuts.windows(2).all(|w| w[0] < w[1]) || len == 0, "{name}: not rising");
        must!(cuts.iter().all(|&c| text.is_char_boundary(c)), "{name}: a cut inside a character");
    }
    let breaks = unicode::line_break_opportunities(text, None);
    must!(breaks.windows(2).all(|w| w[0].at < w[1].at), "line breaks: not rising");
    must!(breaks.iter().all(|b| text.is_char_boundary(b.at) && b.at > 0), "line breaks: one off a character");
    let ends = breaks.last().is_some_and(|b| b.at == len && b.mandatory);
    must!(len == 0 || ends, "line breaks: none mandatory at the end");
    let mut at = 0;
    for run in unicode::script_runs(text) {
        must!(run.bytes.start == at && run.bytes.end > at, "script runs don't tile at {at}");
        at = run.bytes.end;
        let s = run.script;
        if let Some(code) = s.iso15924() {
            must!(Script::from_iso15924(code) == Some(s), "script {} reads back from {code} as another", s.name());
        }
        let _ = (s.direction(), s.is_context_dependent(), s.opentype_tags().count());
    }
    let _ = Script::from_iso15924(text);
    must!(at == len, "script runs end at {at} of {len}");
    for c in text.chars().take(64) {
        let _ = (unicode::general_category(c), unicode::script(c), unicode::vertical_orientation(c));
        let _ = unicode::vertical_form(c);
    }
    if let Ok(feature) = text.parse::<Feature>() {
        must!(feature.to_string().parse::<Feature>().ok() == Some(feature), "{text:?} -> {feature} reads apart");
    }
    if let Ok(tag) = text.parse::<daegun::Tag>() {
        must!(tag.to_string().parse::<daegun::Tag>().ok() == Some(tag), "{text:?} -> {tag} reads otherwise");
    }
    let _ = text.parse::<daegun::shape::Direction>();
}

/// Every call and cross-check at once, the gate's random runner's work.
pub fn everything(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    exercise(input.font, &input.texts());
}

/// `everything`'s calls on a thread with `stack_size` bytes of stack, which real fonts fit, so a font that deepens
/// recursion past it overflows there, reported with its input.
pub fn stack(data: &[u8]) {
    let input = Input::parse(data);
    canary(&input);
    std::thread::scope(|s| {
        let thread = std::thread::Builder::new().stack_size(stack_size()).spawn_scoped(s, || {
            guard_thread();
            exercise(input.font, &input.texts());
        });
        // Joined by hand, so the thread's locals drop within the input too.
        if let Err(panic) = thread.expect("a thread to run on").join() {
            std::panic::resume_unwind(panic);
        }
    });
}

/// Fonts sharing one pool across four threads running chosen calls, each answer the one a lone font gives, and the
/// pool empty once the fonts drop.
#[cfg(feature = "threading")]
pub fn pool(data: &[u8]) {
    use daegun::{Cache, CachePool, CacheStats};
    let input = Input::parse(data);
    canary(&input);
    let open = || Font::from_vec(input.font.to_vec(), 0);
    let (Ok(alone), Ok(a), Ok(b)) = (open(), open(), open()) else { return };
    let caches = [Cache::Shape, Cache::Instance, Cache::Outline, Cache::Raster];
    let pool = CachePool::new(input.choices.budget().min(16 << 20));
    for f in [&a, &b] {
        for cache in caches {
            f.set_cache_pool(cache, Some(&pool));
        }
    }
    let at = input.choices.location(&alone);
    let glyphs = input.choices.glyphs(alone.glyph_count());
    let options = daegun::shape::ShapeOptions::default();
    let drawn = |f: &Font, g| {
        let mut p = Path::default();
        f.outline(g, &at, &mut p).map(|()| p).ok()
    };
    let shaped = |f: &Font| f.shape(&input.text, &at, &options).glyphs.clone();
    let expected = (glyphs.iter().map(|&g| drawn(&alone, g)).collect::<Vec<_>>(), shaped(&alone));
    std::thread::scope(|s| {
        let threads: Vec<_> = (0..4).map(|t| {
            let (fonts, glyphs, expected, input, at) = ([&a, &b], &glyphs, &expected, &input, &at);
            s.spawn(move || {
                guard_thread();
                let mut r = Rasterizer::new();
                for step in 0..12 {
                    let f = fonts[(t + step) % 2];
                    let k = step % glyphs.len();
                    match input.choices.op(t, step, 8) {
                        0 => must!(drawn(f, glyphs[k]) == expected.0[k], "thread {t}: {} drew otherwise", glyphs[k]),
                        1 => must!(shaped(f) == expected.1, "thread {t}: {:?} shaped otherwise", input.text),
                        2 => drop(r.glyph(f, glyphs[k], at, 16.0, &daegun::raster::RasterOptions::DEFAULT)),
                        3 => f.advances(glyphs, at, Orientation::Horizontal, &mut Vec::new()),
                        4 => f.set_cache_budget(caches[step % 4], input.choices.budget()),
                        5 => f.clear_cache(caches[(t + step) % 4]),
                        6 => drop(f.prewarm(glyphs, at)),
                        _ => drop(f.layout(&input.text, at, &Default::default())),
                    }
                }
                daegun::release_thread();
                let k = t % glyphs.len();
                let again = drawn(fonts[t % 2], glyphs[k]);
                must!(again == expected.0[k], "thread {t}: let go, {} drew otherwise", glyphs[k]);
            })
        }).collect();
        // Joined by hand: the scope's own join leaves the threads' locals still dropping after the input ends.
        for thread in threads {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
    });
    // Each worker let go of what its thread kept, and the pool of all it held: the fonts still draw as they did.
    pool.set_budget(0);
    must!(glyphs.iter().zip(&expected.0).all(|(&g, e)| drawn(&a, g) == *e), "the pool emptied, a glyph drew otherwise");
    pool.set_budget(input.choices.budget().min(16 << 20));
    drop((a, b));
    must!(pool.stats() == CacheStats::default(), "fonts dropped left the pool {:?}", pool.stats());
}
