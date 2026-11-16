use daegun::Font;
use daegun::{Cache, CacheStats, Location, NameId};
use crate::located;
use daegun::GlyphId;
use daegun::shape::ShapeOptions;

fn font() -> Font {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_vec(bytes, 0).expect("parses")
}

fn fill_shapes(f: &Font, n: usize) {
    for i in 0..n {
        std::hint::black_box(f.shape(
            &format!("a budget worth measuring {i}"),
            &Location::default(),
            &ShapeOptions::default(),
        ));
    }
}

#[test]
fn the_shape_budget_bounds_the_shape_cache() {
    let f = font();
    f.set_cache_budget(Cache::Shape, 64 * 1024);
    fill_shapes(&f, 4000);
    let CacheStats { bytes, .. } = f.cache_stats(Cache::Shape);
    assert!(bytes <= 64 * 1024, "shape cache held {bytes} B against a 64 KB budget");
}

// Only `prewarm` writes to the outline cache, so only `prewarm` can fill it past the budget.
#[test]
fn the_outline_budget_bounds_the_outline_cache() {
    let f = font();
    f.prewarm((0..f.glyph_count().min(600)).map(GlyphId), &Location::default());
    let CacheStats { bytes: filled, .. } = f.cache_stats(Cache::Outline);
    assert!(filled > 64 * 1024, "prewarming 600 glyphs held only {filled} B, so the budget is untested");
    f.set_cache_budget(Cache::Outline, 32 * 1024);
    let CacheStats { bytes, .. } = f.cache_stats(Cache::Outline);
    assert!(bytes <= 32 * 1024, "outline cache held {bytes} B against a 32 KB budget");
    f.prewarm((0..f.glyph_count().min(600)).map(GlyphId), &Location::default());
    let CacheStats { bytes: again, .. } = f.cache_stats(Cache::Outline);
    assert!(again <= 32 * 1024, "prewarming past the budget held {again} B");
}

// One budget bounds everything each location builds, whole instances and drawing caches together.
// Unbudgeted, the same 40 weights hold about 37 MB, so the 2 MB bound has to evict to hold.
#[test]
fn the_instance_budget_bounds_instanced_fonts() {
    let f = font();
    let fill = |f: &Font| {
        for w in 0..40 {
            let axes: &[(&str, f64)] = &[("wght", 200.0 + f64::from(w) * 10.0)];
            std::hint::black_box(f.shape("weight", &located(f, axes), &ShapeOptions::default()));
            f.outline(GlyphId(40), &located(f, axes), &mut daegun::outline::Path::default()).expect("glyph 40 draws");
            std::hint::black_box(f.instance(&located(f, axes)).expect("instanced"));
        }
        f.cache_stats(Cache::Instance).bytes
    };
    let unbounded = fill(&f);
    assert!(unbounded > 8 * 1024 * 1024, "40 weights held only {unbounded} B, so the budget is untested");
    f.set_cache_budget(Cache::Instance, 2 * 1024 * 1024);
    let held = fill(&f);
    assert!(held <= 2 * 1024 * 1024, "instance caches held {held} B against a 2 MB budget");
}

// The allowance is spent down rather than capped, so what matters is that setting it grants more.
#[test]
fn the_cmap_allowance_is_readable_and_settable() {
    let f = font();
    f.set_index_allowance(1234);
    assert_eq!(f.index_allowance(), 1234);
}

// A budget of zero has to turn caching off without changing a single outline or a shaped run.
#[test]
fn caching_nothing_changes_the_output() {
    let f = font();
    let gid = f.glyph_id('g').expect("font has g");
    let draw = |f: &Font| {
        let mut p = daegun::outline::Path::default();
        f.prepared_outline(
            gid,
            &located(f, &[("wght", 650.0)]),
            28.0,
            &daegun::outline::OutlineOptions::default(),
            &mut p,
        )
        .expect("g draws");
        let run = f.shape("budget", &located(f, &[("wght", 650.0)]), &ShapeOptions::default()).glyphs.clone();
        (p, run, f.instance(&located(f, &[("wght", 650.0)])).expect("instanced"))
    };
    f.prewarm([gid], &located(&f, &[("wght", 650.0)]));
    let warm = draw(&f);
    f.set_cache_budget(Cache::Outline, 0);
    f.set_cache_budget(Cache::Shape, 0);
    f.set_cache_budget(Cache::Instance, 0);
    let cold = draw(&f);
    assert_eq!(f.cache_stats(Cache::Outline).entries, 0, "a zero outline budget still cached something");
    assert_eq!(f.cache_stats(Cache::Shape), CacheStats::default(), "a zero shape budget still cached a run");
    assert_eq!(warm, cold, "output changed when caching was turned off");
}

// The newest instance is kept whatever the budget, or every call at one fixed weight would instance the
// font again. At a budget of 0 it is the only one kept.
#[test]
fn a_zero_instance_budget_keeps_the_newest_instance() {
    let draw = |f: &Font, w: f64| {
        f.outline(GlyphId(40), &located(f, &[("wght", w)]), &mut daegun::outline::Path::default())
            .expect("glyph 40 draws");
        std::hint::black_box(f.instance(&located(f, &[("wght", w)])).expect("instanced"));
    };
    let only = font();
    only.set_cache_budget(Cache::Instance, 0);
    draw(&only, 800.0);
    let CacheStats { entries, bytes, .. } = only.cache_stats(Cache::Instance);
    assert!(entries == 1 && bytes > 0, "a zero budget kept {entries} locations, so each call instances again");
    let both = font();
    both.set_cache_budget(Cache::Instance, 0);
    draw(&both, 700.0);
    draw(&both, 800.0);
    let kept = both.cache_stats(Cache::Instance);
    assert_eq!((kept.entries, kept.bytes), (entries, bytes), "a zero budget kept more than the newest");
}

// What prewarm reports added is what the cache took, so a budget of zero takes nothing.
#[test]
fn prewarm_counts_only_what_the_cache_took() {
    let f = font();
    f.set_cache_budget(Cache::Outline, 0);
    assert_eq!(
        f.prewarm((1..50).map(GlyphId), &Location::default()),
        0,
        "prewarm reported outlines a zero budget refused"
    );
    assert_eq!(f.cache_stats(Cache::Outline).entries, 0, "a zero budget held an outline");
}

// A lower budget evicts the least recently used runs until the rest fit, rather than every run.
#[test]
fn tightening_the_shape_budget_keeps_what_still_fits() {
    let f = font();
    fill_shapes(&f, 50);
    let CacheStats { entries: runs, bytes, .. } = f.cache_stats(Cache::Shape);
    f.set_cache_budget(Cache::Shape, bytes / 2);
    let CacheStats { entries: kept, bytes: held, .. } = f.cache_stats(Cache::Shape);
    assert!(kept > 0 && held <= bytes / 2, "tightening to {} B kept {kept} of {runs} runs, {held} B", bytes / 2);
}

// A run shaped again and again stays cached while runs used once overflow the budget around it.
#[test]
fn a_run_in_use_outlives_the_runs_around_it() {
    let f = font();
    f.set_cache_budget(Cache::Shape, 8 * 1024);
    let hot = f.shape("kept warm", &Location::default(), &ShapeOptions::default());
    for i in 0..2000 {
        let again = f.shape("kept warm", &Location::default(), &ShapeOptions::default());
        assert!(std::ptr::eq(&*hot, &*again), "the run in use was evicted and shaped again after {i} others");
        std::hint::black_box(f.shape(&format!("used once {i}"), &Location::default(), &ShapeOptions::default()));
    }
}

// `shape` hands out the cache's own run as an Arc in every build, `threading` or not, so code that names
// the type compiles either way, and a second call gets the same allocation.
#[test]
fn shape_hands_out_an_arc_in_every_build() {
    let f = font();
    let run: std::sync::Arc<daegun::shape::ShapedRun> =
        f.shape("Hamburg", &Location::default(), &ShapeOptions::default());
    assert!(
        std::sync::Arc::ptr_eq(&run, &f.shape("Hamburg", &Location::default(), &ShapeOptions::default())),
        "a cache hit shaped the run again"
    );
}

// Tightening keeps the most recently used locations that still fit, and the newest whatever the budget.
#[test]
fn tightening_the_instance_budget_keeps_what_still_fits() {
    let f = font();
    let sizes: Vec<usize> = [300.0, 500.0, 700.0]
        .into_iter()
        .map(|w| {
            let before = f.cache_stats(Cache::Instance).bytes;
            std::hint::black_box(f.instance(&located(&f, &[("wght", w)])).expect("instanced"));
            f.outline(GlyphId(40), &located(&f, &[("wght", w)]), &mut daegun::outline::Path::default())
                .expect("glyph 40 draws");
            f.cache_stats(Cache::Instance).bytes - before
        })
        .collect();
    f.set_cache_budget(Cache::Instance, sizes[1] + sizes[2]);
    let kept = f.cache_stats(Cache::Instance);
    assert_eq!((kept.entries, kept.bytes), (2, sizes[1] + sizes[2]), "tightening didn't keep the two newest");
    f.set_cache_budget(Cache::Instance, 0);
    let kept = f.cache_stats(Cache::Instance);
    assert_eq!((kept.entries, kept.bytes), (1, sizes[2]), "a zero budget didn't keep the newest alone");
}

// One location is one key: a tag the font lacks, a value past an axis's range or at its default, and any axes on a
// static font name a location another key names, adding nothing to a cache; of a repeated tag the last counts.
#[test]
fn an_axis_key_holds_only_the_fonts_own_axes() {
    let entries = |f: &Font, first: &[(&str, f64)], then: &[(&str, f64)]| {
        f.clear_cache(Cache::Shape);
        let _ = f.shape("a", &located(f, first), &ShapeOptions::default());
        let _ = f.shape("a", &located(f, then), &ShapeOptions::default());
        f.cache_stats(Cache::Shape).entries
    };
    let f = font();
    assert_eq!(entries(&f, &[], &[("ZZZZ", 1.0)]), 1, "a tag Inter lacks");
    assert_eq!(entries(&f, &[], &[("wght", 400.0)]), 1, "Inter's default weight");
    assert_eq!(entries(&f, &[("wght", 900.0)], &[("wght", 2000.0)]), 1, "a weight past Inter's 900");
    assert_eq!(entries(&f, &[("wght", 700.0)], &[("wght", 300.0), ("ZZZZ", 1.0), ("wght", 700.0)]), 1);
    assert_eq!(entries(&f, &[], &[("wght", 900.0)]), 2, "a weight Inter has");
    assert_eq!(
        located(&f, &[("wght", 2000.0)]).normalized().to_vec(),
        located(&f, &[("wght", 900.0)]).normalized().to_vec()
    );

    let path = format!("{}/scheherazade-new/ScheherazadeNew-Regular.ttf", crate::FONTS);
    let fixed = Font::from_vec(std::fs::read(path).expect("fixture"), 0).expect("parses");
    let _ = fixed.shape("a", &Location::default(), &ShapeOptions::default());
    let bare = fixed.cache_stats(Cache::Instance);
    assert_eq!(entries(&fixed, &[], &[("wght", 700.0)]), 1, "a static font");
    assert_eq!(fixed.cache_stats(Cache::Instance), bare, "a static font instanced again at a weight it lacks");
}

// Shaping and subsetting text at every location read the font's own indexes, drawn once from its allowance, so a
// weight draws nothing more and evicting one gives nothing back. The cmap index is the font's, built once.
#[test]
fn every_location_draws_on_the_fonts_allowance() {
    let f = font();
    let full = f.index_allowance();
    let _ = f.shape("a", &Location::default(), &ShapeOptions::default());
    let font_only = f.index_allowance();
    assert!(font_only < full, "shaping at the default built no index from the font's allowance");
    let _ = f.shape("a", &located(&f, &[("wght", 700.0)]), &ShapeOptions::default());
    let _ = f.shape("a", &located(&f, &[("wght", 300.0)]), &ShapeOptions::default());
    assert_eq!(f.index_allowance(), font_only, "shaping at a weight built indexes of its own");
    f.subset_text("a", &located(&f, &[("wght", 700.0)])).expect("a subset");
    f.subset_text("a", &located(&f, &[("wght", 300.0)])).expect("a subset");
    assert_eq!(f.index_allowance(), font_only, "subsetting at a weight built indexes of its own");
    f.set_cache_budget(Cache::Instance, 0);
    assert_eq!(f.index_allowance(), font_only, "evicting the weights gave back what they never drew");

    let none = font();
    none.set_index_allowance(0);
    let _ = none.shape("a", &located(&none, &[("wght", 700.0)]), &ShapeOptions::default());
    assert_eq!(none.index_allowance(), 0);
}

// A location is charged for what it builds as it builds it: an origin measured there adds its page of glyphs to
// the budget's count. Its advances, worked out glyph by glyph in cells it holds from the start, add nothing.
#[test]
fn a_location_is_charged_for_what_it_builds() {
    let f = font();
    let bold = located(&f, &[("wght", 700.0)]);
    crate::advances(&f, &[GlyphId(40)], &bold);
    let read = f.cache_stats(Cache::Instance).bytes;
    crate::advances(&f, &(0..f.glyph_count()).map(GlyphId).collect::<Vec<_>>(), &bold);
    assert_eq!(f.cache_stats(Cache::Instance).bytes, read, "every glyph's advance grew the count");
    f.vertical_origin(GlyphId(40), &bold);
    let grown = f.cache_stats(Cache::Instance).bytes;
    assert!(grown >= read + 1024, "a measured origin grew the count by {} B", grown - read);
}

// A location takes the tables by glyph a location gone gave back, emptied: one weight after another with one location
// kept, each draws, measures and shapes as on a font that has been at no other.
#[test]
fn a_location_after_another_answers_as_alone() {
    let f = font();
    f.set_cache_budget(Cache::Instance, 0);
    let glyphs: Vec<GlyphId> = (0..300).map(GlyphId).collect();
    let text = "Typography is the craft of endowing human language with a durable visual form.";
    let drawn = |f: &Font, at: &Location| -> Vec<daegun::outline::Path> {
        glyphs.iter().map(|&g| {
            let mut path = daegun::outline::Path::default();
            f.outline(g, at, &mut path).map(|()| path).unwrap_or_default()
        }).collect()
    };
    for w in [300.0, 800.0, 450.0, 300.0] {
        let alone = font();
        let (at, there) = (located(&f, &[("wght", w)]), located(&alone, &[("wght", w)]));
        assert_eq!(drawn(&f, &at), drawn(&alone, &there), "glyphs drawn at wght {w}");
        assert_eq!(crate::advances(&f, &glyphs, &at), crate::advances(&alone, &glyphs, &there), "wght {w}");
        let runs = [&f, &alone].map(|f| f.shape(text, &located(f, &[("wght", w)]), &ShapeOptions::default()));
        assert_eq!(runs[0].glyphs, runs[1].glyphs, "shaped at wght {w}");
    }
}

// A location holds no name until one is read there: its names are built then, read alike every time after and
// charged once, and they name the instance as the whole instance's table does.
#[test]
fn a_locations_names_are_built_on_the_first_read() {
    let f = font();
    let bold: &[(&str, f64)] = &[("wght", 900.0)];
    let _ = f.shape("Hamburgefonstiv", &located(&f, bold), &ShapeOptions::default());
    let shaped = f.cache_stats(Cache::Instance).bytes;
    let names = f.names(&located(&f, bold));
    let read = f.cache_stats(Cache::Instance).bytes;
    assert!(read > shaped, "reading names at the location built nothing");
    let at = located(&f, bold);
    let subfamily = NameId::SUBFAMILY_NAME;
    assert_eq!((f.names(&at), f.name(subfamily, &at)), (names.clone(), names.get(&subfamily).cloned()));
    f.family_name(&located(&f, bold));
    assert_eq!(f.cache_stats(Cache::Instance).bytes, read, "names read again grew the count");
    let whole = Font::from_bytes(&f.instance(&located(&f, bold)).expect("instances")).expect("parses");
    assert_eq!(names, whole.names(&Location::default()), "the whole instance names it otherwise");
    assert_ne!(names, f.names(&Location::default()), "the location names itself as the font");
}

// Every run costs its place in the cache, so a budget of 0 keeps none, empty text included, and empty text at
// one weight after another stays inside a small budget rather than growing past it.
#[test]
fn no_run_is_kept_for_free() {
    let f = font();
    f.set_cache_budget(Cache::Shape, 0);
    let _ = f.shape("", &Location::default(), &ShapeOptions::default());
    let _ = f.shape("a", &Location::default(), &ShapeOptions::default());
    assert_eq!(f.cache_stats(Cache::Shape), CacheStats::default(), "a budget of 0 kept a run");

    f.set_cache_budget(Cache::Shape, 4096);
    for w in 0..100 {
        let _ = f.shape("", &located(&f, &[("wght", 300.0 + f64::from(w))]), &ShapeOptions::default());
    }
    let CacheStats { entries, bytes, .. } = f.cache_stats(Cache::Shape);
    assert!(entries < 100 && bytes <= 4096, "{entries} empty runs held {bytes} B against a 4 KB budget");
}

// Off until a budget is set: drawing leaves nothing behind.
#[test]
fn the_raster_cache_starts_off() {
    let f = font();
    let mut r = daegun::raster::Rasterizer::new();
    for c in ['a', 'g'] {
        let g = f.glyph_id(c).expect("the font has it");
        r.glyph(&f, g, &Location::default(), 16.0, &daegun::raster::RasterOptions::DEFAULT).expect("draws");
    }
    assert_eq!(f.cache_stats(Cache::Raster), CacheStats::default(), "the raster cache kept an image while off");
}

// Every option that changes the pixels keys an image of its own, and a hit is the image a cold draw gives, lent from
// the cache itself, so two rasterizers lend the same bytes. Color glyphs and distance fields too.
#[test]
fn a_raster_hit_is_the_image_drawn() {
    use daegun::color::{Rgba, SceneOptions};
    use daegun::outline::{Cap, HintMode, Join, OutlineOptions, StrokeStyle};
    use daegun::raster::{Field, Gamma, RasterOptions, Rasterizer, StripeOrder, SubpixelLayout};

    #[derive(Clone, Copy)]
    enum Kind<'o> {
        Glyph(&'o RasterOptions<'o>),
        Color(SceneOptions),
        Field(Field),
    }
    let draw = |r: &mut Rasterizer, f: &Font, g: GlyphId, at: &Location, px: f32, kind: Kind| -> (usize, Vec<u8>) {
        let image = match kind {
            Kind::Glyph(o) => r.glyph(f, g, at, px, o).expect("draws"),
            Kind::Color(s) => r.color_glyph(f, g, at, px, &RasterOptions::DEFAULT, &s).expect("draws").expect("color"),
            Kind::Field(field) => r.distance_field(f, g, at, px, field).expect("draws"),
        };
        (image.pixels.as_ptr() as usize, image.to_bitmap().pixels)
    };
    let check = |f: &Font, cold: &Font, cases: &[(GlyphId, Location, f32, Kind)]| {
        f.set_cache_budget(Cache::Raster, 16 << 20);
        let (mut r, mut s, mut c) = (Rasterizer::new(), Rasterizer::new(), Rasterizer::new());
        for (i, &(g, ref at, px, kind)) in cases.iter().enumerate() {
            let (_, truth) = draw(&mut c, cold, g, at, px, kind);
            let (_, first) = draw(&mut r, f, g, at, px, kind);
            let (lent, hit) = draw(&mut r, f, g, at, px, kind);
            let (again, other) = draw(&mut s, f, g, at, px, kind);
            assert!(first == truth && hit == truth && other == truth, "case {i}: a cached image differs");
            assert!(truth.is_empty() || lent == again, "case {i}: a hit was drawn again, not lent from the cache");
        }
        assert_eq!(f.cache_stats(Cache::Raster).entries, cases.len(), "two cases shared a key");
        assert_eq!(cold.cache_stats(Cache::Raster), CacheStats::default());
    };

    let f = font();
    let (a, g) = (f.glyph_id('a').expect("a"), f.glyph_id('g').expect("g"));
    let (at, bold) = (Location::default(), located(&f, &[("wght", 700.0)]));
    let o = RasterOptions::DEFAULT;
    let (rgb, bgr) = [StripeOrder::Rgb, StripeOrder::Bgr].map(SubpixelLayout::horizontal).into();
    let outline = |x: OutlineOptions| o.with_outline(x);
    let stroke = |join, cap| {
        outline(OutlineOptions::DEFAULT.with_stroke(StrokeStyle::new(40.0).with_join(join).with_cap(cap)))
    };
    let srgb = |text, contrast| o.with_gamma(Gamma::Srgb { text, contrast });
    let options = [
        o,
        o.with_offset(0.25, 0.0),
        o.with_offset(0.0, 0.25),
        o.with_layout(&rgb),
        o.with_layout(&bgr),
        o.with_darken(true),
        srgb(Rgba::opaque(0, 0, 0), 0.5),
        srgb(Rgba::opaque(255, 255, 255), 0.5),
        srgb(Rgba::opaque(0, 0, 0), 1.0),
        outline(OutlineOptions::DEFAULT.with_hinting(HintMode::AutoForce)),
        outline(OutlineOptions::DEFAULT.with_transform([1.0, 0.0, 0.2, 1.0, 0.0, 0.0])),
        outline(OutlineOptions::DEFAULT.with_embolden(30.0)),
        outline(OutlineOptions::DEFAULT.with_oblique(0.2)),
        stroke(Join::Miter { limit: 4.0 }, Cap::Butt),
        stroke(Join::Miter { limit: 1.5 }, Cap::Butt),
        stroke(Join::Round, Cap::Butt),
        stroke(Join::Bevel, Cap::Butt),
        stroke(Join::Bevel, Cap::Round),
    ];
    let fields = [Field::Sdf { spread: 2.0 }, Field::Sdf { spread: 4.0 }, Field::Msdf { spread: 2.0 }];
    let kinds: Vec<Kind> = options.iter().map(Kind::Glyph).chain(fields.map(Kind::Field)).collect();
    let mut cases: Vec<_> = kinds.iter().map(|&k| (a, at.clone(), 24.0, k)).collect();
    cases.extend([(g, at.clone(), 24.0, kinds[0]), (a, bold, 24.0, kinds[0]), (a, at.clone(), 25.0, kinds[0])]);
    check(&f, &font(), &cases);

    let path = format!("{}/colr-v1-test-glyphs/test_glyphs.ttf", crate::FONTS);
    let color = || Font::from_vec(std::fs::read(&path).expect("the fixture"), 0).expect("parses");
    let f = color();
    let glyph = (1..f.glyph_count())
        .map(GlyphId)
        .find(|&g| f.color_scene(g, &Location::default(), &SceneOptions::DEFAULT).is_ok_and(|s| s.is_some()))
        .expect("a color glyph");
    let red = SceneOptions::DEFAULT.with_foreground(Rgba::opaque(200, 0, 0));
    let scenes = [SceneOptions::DEFAULT, red, SceneOptions::DEFAULT.with_palette(1)];
    let cases: Vec<_> = scenes.iter().map(|&s| (glyph, Location::default(), 32.0, Kind::Color(s))).collect();
    check(&f, &color(), &cases);
}

// The budget bounds what the raster cache holds, a lower one applies at once, and clearing empties it.
#[test]
fn the_raster_budget_bounds_the_raster_cache() {
    let f = font();
    let mut r = daegun::raster::Rasterizer::new();
    let fill = |f: &Font, r: &mut daegun::raster::Rasterizer| {
        for g in 0..f.glyph_count().min(600) {
            let _ = r.glyph(f, GlyphId(g), &Location::default(), 48.0, &daegun::raster::RasterOptions::DEFAULT);
        }
    };
    f.set_cache_budget(Cache::Raster, 64 * 1024);
    fill(&f, &mut r);
    let CacheStats { entries, bytes, .. } = f.cache_stats(Cache::Raster);
    assert!(entries > 10 && bytes <= 64 * 1024, "{entries} images, {bytes} B against a 64 KB budget");
    f.set_cache_budget(Cache::Raster, 16 * 1024);
    let CacheStats { entries: kept, bytes, .. } = f.cache_stats(Cache::Raster);
    assert!(kept > 0 && kept < entries && bytes <= 16 * 1024, "tightening kept {kept} images, {bytes} B");
    f.clear_cache(Cache::Raster);
    assert_eq!(f.cache_stats(Cache::Raster), CacheStats::default(), "clearing left an image");
}

fn fixture(path: &str) -> (Vec<u8>, Font) {
    let bytes = std::fs::read(format!("{}/{path}", crate::FONTS)).expect("a fixture");
    (bytes.clone(), Font::from_vec(bytes, 0).expect("parses"))
}

fn table_bytes(f: &Font) -> usize {
    f.table_tags().map(|t| f.table(t).map_or(0, <[u8]>::len)).sum()
}

const INDEX_FLOOR: usize = 4 << 20;

// A font's indexes may take a 4 MiB floor and eight times its tables' bytes: Source Han Sans's 8.4 MB of tables give
// them 71.6 MB.
#[test]
fn the_allowance_grows_with_the_font() {
    for path in ["inter/InterVariable.ttf", "source-han-sans/SourceHanSansJP-VF.otf"] {
        let (_, f) = fixture(path);
        assert_eq!(f.index_allowance(), INDEX_FLOOR + 8 * table_bytes(&f), "{path}");
    }
}

// A table record naming bytes another names adds none, so naming a table again grows no allowance.
#[test]
fn bytes_two_tables_name_count_once() {
    let (mut bytes, whole) = fixture("inter/InterVariable.ttf");
    let tables = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let record = |bytes: &[u8], tag: &[u8; 4]| (0..tables).map(|i| 12 + 16 * i).find(|&e| &bytes[e..e + 4] == tag);
    let (glyf, name) = (record(&bytes, b"glyf").expect("glyf"), record(&bytes, b"name").expect("name"));
    let name_len = u32::from_be_bytes(bytes[name + 12..name + 16].try_into().expect("four bytes")) as usize;
    bytes.copy_within(glyf + 8..glyf + 16, name + 8);
    let named_twice = Font::from_vec(bytes, 0).expect("parses");
    assert_eq!(named_twice.index_allowance(), whole.index_allowance() - 8 * name_len);
}

// A 40-byte cmap mapping all of Unicode to one glyph, as LastResort's does, keeps its 2.2 MB index within the floor.
#[test]
fn a_cmap_of_all_unicode_keeps_its_index() {
    let words = |w: &[u32], wide: bool| -> Vec<u8> {
        let bytes = |v: u32| if wide { v.to_be_bytes().to_vec() } else { (v as u16).to_be_bytes().to_vec() };
        w.iter().flat_map(|&v| bytes(v)).collect()
    };
    let cmap = [words(&[0, 1, 3, 10], false), words(&[12], true), words(&[13, 0], false), words(&[28, 0, 1], true)]
        .concat();
    let f = super::aat_subset::carets_with(&[("cmap", [cmap, words(&[0, 0x10_FFFF, 1], true)].concat())]);
    let full = f.index_allowance();
    let got = f.shape("a\u{20AC}\u{1F600}", &Location::default(), &ShapeOptions::default());
    assert_eq!(got.glyphs, [GlyphId(1); 3]);
    let drawn = full - f.index_allowance();
    assert!(drawn > 2_000_000, "the cmap's index went unbuilt: the indexes drew {drawn} bytes");
}

// 100 lookups, each a coverage of every glyph id in 10 bytes, would index as 12.8 MB: the indexes stay within the
// floor and eight times the font's bytes, and the lookups left unindexed read their tables to the same glyphs.
#[test]
fn wide_coverages_grow_no_index_past_the_allowance() {
    use super::aat_subset::{carets_with, glyph_ids, u16s, F, I, L};
    use super::layout_build::{layout, lookup, to};
    let single = |last: u16| [u16s(&[1, 6, 0]), u16s(&[2, 1, 0, last, 0])].concat();
    let lookups = (0..100).map(|i| lookup(1, 0, vec![to(single(u16::MAX - i))])).collect();
    let all: Vec<u16> = (0..100).collect();
    let f = carets_with(&[("GSUB", layout(b"DFLT", &[(b"ccmp", &all)], lookups))]);
    let full = f.index_allowance();
    let got = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(got.glyphs, glyph_ids(&[F, I, L]));
    let drawn = full - f.index_allowance();
    let most = INDEX_FLOOR + 8 * table_bytes(&f);
    assert!(drawn <= most, "the indexes drew {drawn} bytes, past {most}");
}

// A run past 64 KB is kept under a budget it is a small part of.
#[test]
fn a_long_run_is_kept_under_a_budget_it_fits() {
    let f = font();
    let text = "Typography is the craft of endowing human language with a durable visual form. ".repeat(40);
    f.set_cache_budget(Cache::Shape, 64 << 20);
    let first = f.shape(&text, &Location::default(), &ShapeOptions::default());
    let CacheStats { bytes, .. } = f.cache_stats(Cache::Shape);
    assert!(bytes > 64 << 10, "a run of {bytes} B proves nothing past 64 KB");
    let again = f.shape(&text, &Location::default(), &ShapeOptions::default());
    assert!(std::sync::Arc::ptr_eq(&first, &again), "a run of {bytes} B under a 64 MB budget was shaped again");
}

fn fill_outlines(f: &Font) {
    f.prewarm((0..f.glyph_count().min(300)).map(GlyphId), &Location::default());
}

// One pool bounds every font's caches in it, of each kind it holds, and counts what each holds, no more.
#[test]
fn a_pool_bounds_the_caches_of_every_font_in_it() {
    let pool = daegun::CachePool::new(256 << 10);
    let fonts: Vec<Font> = (0..4).map(|_| font()).collect();
    for f in &fonts {
        for cache in [Cache::Shape, Cache::Outline, Cache::Raster] {
            f.set_cache_pool(cache, Some(&pool));
        }
        f.set_cache_budget(Cache::Raster, usize::MAX);
    }
    let mut r = daegun::raster::Rasterizer::new();
    for (i, f) in fonts.iter().enumerate() {
        fill_shapes(f, 300 + i);
        fill_outlines(f);
        for g in 0..100 {
            let _ = r.glyph(f, GlyphId(g), &Location::default(), 24.0, &daegun::raster::RasterOptions::DEFAULT);
        }
    }
    let CacheStats { entries, bytes, .. } = pool.stats();
    assert!(entries > 100 && bytes <= 256 << 10, "the pool held {entries} entries, {bytes} B against 256 KB");
    let each = |cache| fonts.iter().map(|f| f.cache_stats(cache)).fold((0, 0), |n, s| (n.0 + s.entries, n.1 + s.bytes));
    let held = [Cache::Shape, Cache::Outline, Cache::Raster].map(each);
    assert_eq!(held.iter().fold((0, 0), |n, s| (n.0 + s.0, n.1 + s.1)), (entries, bytes), "the pool miscounted");
    let alone = font();
    alone.set_cache_budget(Cache::Raster, usize::MAX);
    fill_shapes(&alone, 300);
    fill_outlines(&alone);
    for g in 0..100 {
        let _ = r.glyph(&alone, GlyphId(g), &Location::default(), 24.0, &daegun::raster::RasterOptions::DEFAULT);
    }
    let one: usize = [Cache::Shape, Cache::Outline, Cache::Raster].map(|c| alone.cache_stats(c).bytes).iter().sum();
    assert!(4 * one > 2 * (256 << 10), "a font out of the pool held only {one} B, so the pool's bound is untested");
}

// Past the pool's budget the run least recently used goes, whichever font's: a run one font shapes again and again
// outlives the runs another shapes once around it.
#[test]
fn a_pool_evicts_the_least_recently_used_across_fonts() {
    let pool = daegun::CachePool::new(16 << 10);
    let (hot, cold) = (font(), font());
    for f in [&hot, &cold] {
        f.set_cache_pool(Cache::Shape, Some(&pool));
    }
    let kept = hot.shape("kept warm", &Location::default(), &ShapeOptions::default());
    for i in 0..500 {
        let again = hot.shape("kept warm", &Location::default(), &ShapeOptions::default());
        assert!(std::sync::Arc::ptr_eq(&kept, &again), "the run in use was evicted after {i} of another font's");
        std::hint::black_box(cold.shape(&format!("used once {i}"), &Location::default(), &ShapeOptions::default()));
    }
    assert!(pool.stats().bytes <= 16 << 10);
}

// Each font keeps to its own budget in a pool, and leaving keeps what it holds, giving the pool its bytes back, as a
// font dropped does.
#[test]
fn a_font_keeps_its_own_budget_and_gives_its_bytes_back() {
    let pool = daegun::CachePool::new(64 << 20);
    let (small, other) = (font(), font());
    small.set_cache_budget(Cache::Shape, 8 << 10);
    for f in [&small, &other] {
        f.set_cache_pool(Cache::Shape, Some(&pool));
        fill_shapes(f, 200);
    }
    let (mine, theirs) = (small.cache_stats(Cache::Shape), other.cache_stats(Cache::Shape));
    assert!(mine.bytes <= 8 << 10 && theirs.bytes > 8 << 10, "{mine:?} against its own 8 KB, {theirs:?}");
    let both = (mine.entries + theirs.entries, mine.bytes + theirs.bytes);
    assert_eq!((pool.stats().entries, pool.stats().bytes), both, "the pool counted otherwise than its fonts hold");
    other.set_cache_pool(Cache::Shape, None);
    assert_eq!((pool.stats(), other.cache_stats(Cache::Shape)), (mine, theirs), "leaving moved what other held");
    drop(small);
    assert_eq!(pool.stats(), CacheStats::default(), "a dropped font's runs stayed counted");
    other.set_cache_pool(Cache::Shape, Some(&pool));
    pool.set_budget(4 << 10);
    assert!(pool.stats().bytes <= 4 << 10 && other.cache_stats(Cache::Shape).bytes <= 4 << 10);
}

// Each font keeps its newest location past the pool's budget, as past its own, so a font read at one weight keeps it in
// a pool of 0; the older go whichever font read them.
#[test]
fn a_pool_leaves_each_font_its_newest_location() {
    let pool = daegun::CachePool::new(0);
    let fonts = [font(), font()];
    for f in &fonts {
        f.set_cache_pool(Cache::Instance, Some(&pool));
        for w in [300.0, 700.0] {
            f.outline(GlyphId(40), &located(f, &[("wght", w)]), &mut daegun::outline::Path::default())
                .expect("glyph 40 draws");
        }
    }
    let held: Vec<CacheStats> = fonts.iter().map(|f| f.cache_stats(Cache::Instance)).collect();
    assert!(held.iter().all(|s| s.entries == 1), "a pool of 0 kept {held:?}");
    assert_eq!(pool.stats().entries, 2);
}
