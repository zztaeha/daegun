use super::FONTS;
use daegun::{Cache, Location};
use crate::located;
use daegun::GlyphId;

fn font(rel: &str) -> daegun::Font {
    let bytes = std::fs::read(format!("{FONTS}/{rel}")).expect("read font");
    daegun::Font::from_bytes(&bytes).expect("parse font")
}

#[test]
fn prewarm_counts_only_what_it_added() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let n = f.glyph_count();

    let inkable = (0..n)
        .filter(|&gid| {
            let mut p = daegun::outline::Path::default();
            f.outline(GlyphId(gid), &Location::default(), &mut p).is_ok() && !p.is_empty()
        })
        .count();

    let first = f.prewarm((0..n).map(GlyphId), &Location::default());
    assert_eq!(first, inkable, "prewarm cached {first} outlines where {inkable} are non-empty");
    assert!(first < n as usize, "every gid cached, so the empty-outline skip never fired");

    f.clear_cache(Cache::Outline);
    assert!(f.prewarm((0..500).map(GlyphId), &Location::default()) > 400);
    assert_eq!(
        f.prewarm((0..500).map(GlyphId), &Location::default()),
        0,
        "the second prewarm re-added already-cached outlines"
    );

    assert_eq!(f.prewarm((n..n.saturating_add(50)).map(GlyphId), &Location::default()), 0);
}

// A run's glyphs prewarm as the run holds them, by reference, and a repeat adds nothing.
#[test]
fn a_runs_glyphs_prewarm_by_reference() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let run = f.shape("Typeset", &Location::default(), &daegun::shape::ShapeOptions::default());
    assert_eq!(f.prewarm(&run.glyphs, &Location::default()), 6, "six letters, the second e adding none");
    assert_eq!(f.prewarm(run.glyphs.iter(), &Location::default()), 0);
}

fn drawn(f: &daegun::Font, gid: u32, axes: &[(&str, f64)]) -> Option<daegun::outline::Path> {
    let mut p = daegun::outline::Path::default();
    f.outline(GlyphId(gid), &located(f, axes), &mut p).ok()?;
    Some(p)
}

#[test]
fn a_prewarmed_outline_draws_what_a_decode_draws() {
    for (name, rel, glyphs) in [
        ("glyf", "eb-garamond/EBGaramond.ttf", 900u32),
        ("CFF", "stix-two-math/STIX2Math.otf", 900),
        ("CFF2 variable", "source-han-sans/SourceHanSansJP-VF.otf", 900),
    ] {
        let plain = font(rel);
        let warmed = font(rel);
        let n = plain.glyph_count().min(glyphs);
        let cached = warmed.prewarm((0..n).map(GlyphId), &Location::default());
        assert!(cached > 0, "{name}: prewarm cached nothing, so this test proves nothing");
        let mut compared = 0usize;
        for gid in 0..n {
            assert_eq!(drawn(&plain, gid, &[]), drawn(&warmed, gid, &[]), "{name}: gid {gid} replayed differently");
            compared += 1;
        }
        assert!(compared > 800, "{name}: only {compared} outlines compared");
    }
}

#[test]
fn a_prewarmed_outline_prepares_like_a_decode_under_a_transform() {
    let plain = font("eb-garamond/EBGaramond.ttf");
    let warmed = font("eb-garamond/EBGaramond.ttf");
    warmed.prewarm((0..400).map(GlyphId), &Location::default());
    let (s, c) = (0.4f32.sin(), 0.4f32.cos());
    for t in [[c, s, -s, c, 0.0, 0.0], [1.0, 0.0, 0.25, 1.0, 0.0, 0.0], [1.5, 0.0, 0.0, 0.75, 0.0, 0.0]] {
        let opts = daegun::outline::OutlineOptions::default().with_transform(t);
        for gid in 1..400u32 {
            let (mut a, mut b) = (daegun::outline::Path::default(), daegun::outline::Path::default());
            let ga = plain.prepared_outline(GlyphId(gid), &Location::default(), 24.0, &opts, &mut a);
            let gb = warmed.prepared_outline(GlyphId(gid), &Location::default(), 24.0, &opts, &mut b);
            assert_eq!((ga, a), (gb, b), "gid {gid} differed under transform {t:?}");
        }
    }
}

#[test]
fn prewarmed_outlines_are_keyed_by_axis_location() {
    let plain = font("inter/InterVariable.ttf");
    let warmed = font("inter/InterVariable.ttf");
    warmed.prewarm((1..400).map(GlyphId), &located(&warmed, &[("wght", 100.0)]));
    let mut differed_across_weights = 0usize;
    for gid in 1..400u32 {
        for w in [100.0f64, 900.0] {
            assert_eq!(drawn(&plain, gid, &[("wght", w)]), drawn(&warmed, gid, &[("wght", w)]),
                "gid {gid} at wght {w} was served the wrong prewarmed outline");
        }
        let (light, bold) = (drawn(&plain, gid, &[("wght", 100.0)]), drawn(&plain, gid, &[("wght", 900.0)]));
        if light.as_ref().is_some_and(|p| !p.is_empty()) && light != bold { differed_across_weights += 1 }
    }
    assert!(differed_across_weights > 100,
        "only {differed_across_weights} glyphs differ between wght 100 and 900, so the keying is untested");
}

#[test]
fn clearing_prewarm_restores_the_decode() {
    let f = font("eb-garamond/EBGaramond.ttf");
    let before: Vec<_> = (1..200u32).map(|g| drawn(&f, g, &[])).collect();
    assert!(f.prewarm((1..200).map(GlyphId), &Location::default()) > 0);
    let warmed: Vec<_> = (1..200u32).map(|g| drawn(&f, g, &[])).collect();
    f.clear_cache(Cache::Outline);
    assert_eq!(f.cache_stats(Cache::Outline).entries, 0, "clear_prewarm left outlines behind");
    let after: Vec<_> = (1..200u32).map(|g| drawn(&f, g, &[])).collect();
    assert_eq!(before, warmed, "prewarming changed an outline");
    assert_eq!(before, after, "clearing the prewarm changed an outline");
}
