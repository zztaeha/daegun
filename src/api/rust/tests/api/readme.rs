// The README's Rust examples, run as written and held to what their comments say. Each block below appears in
// README.md line for line, but for two names: its "Inter.ttf" is INTER here and its "subset.ttf" is `out`.
use std::error::Error;

const INTER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
const COLR_CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/colr-clip.ttf");
const README: &str = include_str!("../../../../../README.md");
const SELF: &str = include_str!("readme.rs");
const README_C: &str = include_str!("../../../c/tests/readme.c");

// Every Rust block in the README, its lines trimmed and the two names swapped, is a run of lines in this file.
#[test]
fn each_readme_block_is_one_this_file_runs() {
    let ours: Vec<&str> = SELF.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut blocks = 0;
    for block in README.split("```rust\n").skip(1) {
        let block = block.split("```").next().unwrap_or_default();
        let block = block.replace("\"Inter.ttf\"", "INTER").replace("\"subset.ttf\"", "&out");
        let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let found = ours.windows(lines.len()).any(|w| w == lines);
        assert!(found, "a README block isn't in readme.rs as written:\n{block}");
        blocks += 1;
    }
    assert_eq!(blocks, 8, "the README holds {blocks} Rust blocks, not the 8 run here");
}

// Every C block in the README is a run of lines in readme.c, which the gate's C step builds and runs.
#[test]
fn each_readme_c_block_is_one_readme_c_runs() {
    let ours: Vec<&str> = README_C.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut blocks = 0;
    for block in README.split("```c\n").skip(1) {
        let block = block.split("```").next().unwrap_or_default();
        let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let found = ours.windows(lines.len()).any(|w| w == lines);
        assert!(found, "a README block isn't in readme.c as written:\n{block}");
        blocks += 1;
    }
    assert_eq!(blocks, 6, "the README holds {blocks} C blocks, not the 6 readme.c runs");
}

#[test]
#[expect(unused_variables, reason = "the README's loop names each op for its reader")]
fn the_readme_examples_run_and_say_what_they_do() -> Result<(), Box<dyn Error>> {
    use daegun::outline::{OutlineOptions, Path};
    use daegun::shape::ShapeOptions;
    use daegun::{Font, Location};

    let bytes = std::fs::read(INTER)?;
    let font = Font::from_bytes(&bytes)?;

    let run = font.shape("Wave", &Location::default(), &ShapeOptions::DEFAULT);
    // glyphs [459, 507, 980, 614], advances [1914.0, 1120.0, 1111.0, 1194.0]

    let mut outline = Path::default();
    font.prepared_outline(run.glyphs[0], &Location::default(), 32.0, &OutlineOptions::default(), &mut outline)?;
    // one contour, 15 lines and 16 quadratics, about 30 by 23 pixels, y up from the baseline

    use daegun::outline::Verb;
    let glyphs: Vec<u32> = run.glyphs.iter().map(|g| g.0).collect();
    assert_eq!((glyphs, run.advances.clone()), (vec![459, 507, 980, 614], vec![1914.0, 1120.0, 1111.0, 1194.0]));
    let count = |v: Verb| outline.parts().0.iter().filter(|&&w| w == v).count();
    assert_eq!((count(Verb::Move), count(Verb::Line), count(Verb::Quad)), (1, 15, 16), "W's verbs at 32 px");
    let b = outline.bounds().expect("W has a box");
    let (w, h) = (b.x_max - b.x_min, b.y_max - b.y_min);
    assert!((w - 30.0).abs() < 1.0 && (h - 23.0).abs() < 1.0, "W at 32 px is {w} by {h}");
    assert_eq!(bytes.len(), 879_708, "Inter's size, which the subset example quotes");

    let gid = font.glyph_id('W').expect("Inter maps W");
    {
        use daegun::Tag;

        let bold = font.location(&[(Tag::new(b"wght"), 700.0), (Tag::new(b"opsz"), 28.0)]);
        let run = font.shape("Wave", &bold, &ShapeOptions::DEFAULT);
        font.prepared_outline(gid, &bold, 32.0, &OutlineOptions::default(), &mut outline)?;

        assert_ne!(run.advances[0], 1914.0, "wght 700 shaped as the default");
    }
    {
        use daegun::outline::{HintMode, OutlineOptions};

        let opts = OutlineOptions::default().with_hinting(HintMode::Auto).with_embolden(40.0);
        let glyph = font.prepared_outline(gid, &Location::default(), 32.0, &opts, &mut outline)?;
        // glyph.advance_width includes the embolden, and glyph.hinted says a hinter ran

        let plain = font.prepared_outline(gid, &Location::default(), 32.0, &OutlineOptions::default(), &mut outline)?;
        assert!(glyph.advance_width > plain.advance_width && glyph.hinted, "{glyph:?} against {plain:?}");
    }
    {
        use daegun::outline::{max_area_for, resolve_overlaps};

        let curves = font.quads(gid, &Location::default())?;
        // 32 curves, each a Quad { start, control, end } in font units, wound clockwise

        let mut units = Path::default();
        font.outline(gid, &Location::default(), &mut units)?;
        let polygons = units.flatten(max_area_for(32.0, f32::from(font.units_per_em())))
            .expect("a real glyph stays under MAX_FLATTEN_POINTS");
        let merged = resolve_overlaps(&polygons);
        // Some for this W, whose one contour crosses itself where its strokes meet; None for an H

        assert_eq!(curves.len(), 32, "W's quadratics");
        assert!(merged.is_some(), "W's crossing contour was left as it was");
        let mut h = Path::default();
        font.outline(font.glyph_id('H').expect("Inter maps H"), &Location::default(), &mut h)?;
        let polygons = h.flatten(max_area_for(32.0, f32::from(font.units_per_em()))).expect("H flattens");
        assert!(resolve_overlaps(&polygons).is_none(), "H's contours were merged");
    }
    let text = "Type is the voice of the page, and a paragraph is its breath.";
    {
        use daegun::layout::{Align, BreakStrategy, LayoutOptions};

        let scale = 16.0 / f64::from(font.units_per_em());
        let options = LayoutOptions::default()
            .with_max_inline_size(240.0 / scale)
            .with_line_height(20.0 / scale)
            .with_strategy(BreakStrategy::Optimal)
            .with_align(Align::Start);
        let layout = font.layout(text, &Location::default(), &options);

        for line in &layout.lines {
            for run in &line.runs {
                // run.run.glyphs, run.run.advances, run.offset
            }
        }

        assert!(layout.lines.len() > 1, "a paragraph past 240 px stayed on one line");
    }
    {
        let out = std::env::temp_dir().join(format!("daegun-readme-{}.ttf", std::process::id()));
        let subset = font.subset_text("Type is the voice of the page.", &Location::default())?;
        std::fs::write(&out, &subset.font)?;
        // 879,708 bytes to 9,824

        assert_eq!(std::fs::metadata(&out)?.len(), 9_824, "the subset's size");
        std::fs::remove_file(&out)?;
    }
    {
        let font = Font::from_bytes(&std::fs::read(COLR_CLIP)?)?;
        let gid = font.glyph_id('N').expect("the fixture maps N");
        use daegun::color::SceneOptions;

        let scene = font.color_scene(gid, &Location::default(), &SceneOptions::default())?
            .expect("is a color glyph");
        for op in scene.ops() {
            // color::Op::Fill { path, paint, rule, transform }, and clips and layers pushed and popped;
            // scene.path(path) is the outline a fill draws, in font units
        }

        assert!(!scene.ops().is_empty(), "N's scene is empty");
    }
    {
        use daegun::raster::{RasterOptions, Rasterizer};

        let (px, baseline) = (32.0, 30);
        let scale = f64::from(px) / f64::from(font.units_per_em());
        let run = font.shape("Wave", &Location::default(), &ShapeOptions::DEFAULT);
        let (width, height) = (96, 40);
        let mut canvas = vec![0u8; width * height];

        let mut rasterizer = Rasterizer::new();
        let mut pen = 2.0;
        for ((&gid, advance), offset) in run.glyphs.iter().zip(&run.advances).zip(&run.offsets) {
            // whole pixels place the image, and the fraction moves the glyph within it
            let (x, y) = (pen + offset.x * scale, offset.y * scale);
            let options = RasterOptions::DEFAULT.with_offset(x.fract() as f32, y.fract() as f32);
            let image = rasterizer.glyph(&font, gid, &Location::default(), px, &options)?;
            let (left, top) = (x as i32 + image.left, baseline - y as i32 - image.top);
            for row in 0..image.height {
                let start = (top + row as i32) as usize * width + left as usize;
                for (dst, &src) in canvas[start..].iter_mut().zip(image.row(row)) {
                    *dst = dst.saturating_add(src);
                }
            }
            pen += advance * scale;
        }

        // "Wave" inked across 82 by 25 pixels, each byte the share of its pixel the glyphs cover

        let inked: Vec<usize> = (0..canvas.len()).filter(|&i| canvas[i] > 0).collect();
        let (xs, ys) = (inked.iter().map(|i| i % width), inked.iter().map(|i| i / width));
        let ink = (xs.clone().min(), xs.max(), ys.clone().min(), ys.max());
        assert_eq!(ink, (Some(2), Some(83), Some(6), Some(30)), "Wave's ink in the canvas");
    }
    Ok(())
}
