use crate::Hint;
use daegun::{Bounds, Font, GlyphId, Location};
use daegun::outline::HintMode;

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

// Geometry, not pixels: the outline a rasterizer of your own would receive.
fn outline(f: &Font, gid: GlyphId, px: f32, mode: HintMode) -> Option<daegun::outline::Path> {
    let mut p = daegun::outline::Path::default();
    let options = daegun::outline::OutlineOptions::default().with_hinting(mode);
    f.prepared_outline(gid, &Location::default(), px, &options, &mut p).ok()?;
    Some(p)
}

fn hinted(f: &Font, gid: GlyphId, px: f32, mode: HintMode) -> bool {
    let mut sink = daegun::outline::Path::default();
    let options = daegun::outline::OutlineOptions::default().with_hinting(mode);
    f.prepared_outline(gid, &Location::default(), px, &options, &mut sink).is_ok_and(|g| g.hinted)
}

// How much of the line at height `y` the outline covers, by non-zero winding.
fn covered(p: &daegun::outline::Path, y: f32) -> f32 {
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for c in p.flatten(0.001).expect("within the cap") {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.y <= y) != (b.y <= y) {
                crossings.push((a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x), if b.y > a.y { 1 } else { -1 }));
            }
        }
    }
    crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut w, mut length) = (0, 0.0);
    for pair in crossings.windows(2) {
        w += pair[0].1;
        if w != 0 {
            length += pair[1].0 - pair[0].0;
        }
    }
    length
}

#[test]
fn auto_changes_an_unhinted_truetype_font_where_bytecode_cannot() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H').expect("Inter maps H");
    let plain = outline(&f, gid, 13.0, HintMode::None);
    assert_eq!(plain, outline(&f, gid, 13.0, HintMode::Font),
        "InterVariable ships no hinting, so Font must leave the outline alone – if this fails the fixture changed");
    assert!(!hinted(&f, gid, 13.0, HintMode::Font), "bytecode hinting ran on a font with no bytecode");
    assert!(hinted(&f, gid, 13.0, HintMode::Auto), "HintMode::Auto did not run the autohinter");
    assert_ne!(plain, outline(&f, gid, 13.0, HintMode::Auto), "the autohinter ran and moved nothing");
}

#[test]
fn auto_reaches_a_cff_font() {
    let f = font("source-serif/SourceSerif4Variable-Roman.otf");
    let gid = f.glyph_id('H').expect("Source Serif maps H");
    assert!(hinted(&f, gid, 13.0, HintMode::Auto), "a CFF font reached HintMode::Auto and nothing hinted it");
    assert_ne!(outline(&f, gid, 13.0, HintMode::None), outline(&f, gid, 13.0, HintMode::Auto),
        "a CFF outline came back from HintMode::Auto unchanged");
}

#[test]
fn auto_force_overrides_the_fonts_own_bytecode() {
    let f = font("test-fixtures/hinted.ttf");
    let bytecode = outline(&f, GlyphId(1), 16.0, HintMode::Font).expect("hinted.ttf gid 1");
    assert!(hinted(&f, GlyphId(1), 16.0, HintMode::Font), "the fixture's bytecode did not run");
    assert_eq!(
        Some(&bytecode),
        outline(&f, GlyphId(1), 16.0, HintMode::Auto).as_ref(),
        "Auto must defer to real bytecode"
    );
    assert_ne!(
        Some(&bytecode),
        outline(&f, GlyphId(1), 16.0, HintMode::AutoForce).as_ref(),
        "AutoForce kept the bytecode"
    );
}

// A font with its own programs is left to them under Auto: a glyph it gives none stays unhinted beside those it
// hints, which come out as Font's.
#[test]
fn auto_leaves_a_glyph_its_hinted_font_does_not_hint_unhinted() {
    let bytes = std::fs::read(format!("{}/Irianisadfstd/IrianisadfstdRegular.ttc", crate::FONTS)).expect("Irianis");
    let irianis = Font::from_collection(&bytes, 0).expect("Irianis opens");
    let hinted = font("test-fixtures/hinted.ttf");
    for (f, c) in [(&irianis, '«'), (&hinted, 'D')] {
        let gid = f.glyph_id(c).unwrap_or_else(|| panic!("{c} is mapped"));
        let hint = |mode| f.hinted(gid, &Location::default(), 16.0, mode);
        assert!(hint(HintMode::Font).is_none(), "{c} gained a glyph program; pick another glyph");
        assert!(hint(HintMode::AutoForce).is_some(), "the autohinter declined {c}");
        assert!(hint(HintMode::Auto).is_none(), "Auto autohinted {c}, which its font leaves unhinted");
    }
    let h = irianis.glyph_id('H').expect("Irianis maps H");
    let hint = |mode| irianis.hinted(h, &Location::default(), 16.0, mode);
    assert!(hint(HintMode::Font).is_some(), "Irianis's H lost its program");
    assert_eq!(hint(HintMode::Auto), hint(HintMode::Font), "Auto hinted H as the font's programs don't");
}

#[test]
fn hinting_shifts_the_box_by_at_most_a_couple_of_pixels_at_any_size() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('o').expect("Inter maps o");
    for px in [12.0, 64.0, 500.0, 2000.0] {
        let Bounds { x_min: a0, y_min: b0, x_max: a1, y_max: b1 } =
            outline(&f, gid, px, HintMode::None).and_then(|p| p.bounds()).expect("o");
        let Bounds { x_min: c0, y_min: d0, x_max: c1, y_max: d1 } =
            outline(&f, gid, px, HintMode::Auto).and_then(|p| p.bounds()).expect("o");
        let (dw, dh) = (((c1 - c0) - (a1 - a0)).abs(), ((d1 - d0) - (b1 - b0)).abs());
        assert!(dw <= 2.0 && dh <= 2.0, "at {px} ppem hinting changed the box by {dw:.2} x {dh:.2} px");
    }
}

#[test]
fn a_non_latin_font_declines_rather_than_mis_hinting() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    assert!(!hinted(&f, GlyphId(5), 13.0, HintMode::AutoForce), "a font with no Latin coverage was hinted anyway");
    assert_eq!(outline(&f, GlyphId(5), 13.0, HintMode::None), outline(&f, GlyphId(5), 13.0, HintMode::AutoForce));
}

#[test]
fn a_sweep_of_glyphs_all_hint_under_auto() {
    let f = font("inter/InterVariable.ttf");
    let (mut checked, mut moved) = (0usize, 0usize);
    for gid in (1..400u32).step_by(7) {
        let Some(plain) = outline(&f, GlyphId(gid), 12.0, HintMode::None) else { continue };
        if plain.is_empty() {
            continue;
        }
        let auto = outline(&f, GlyphId(gid), 12.0, HintMode::Auto)
            .unwrap_or_else(|| panic!("gid {gid} has an outline but not under Auto"));
        assert!(hinted(&f, GlyphId(gid), 12.0, HintMode::Auto), "gid {gid} came back unhinted under Auto");
        checked += 1;
        moved += usize::from(plain != auto);
    }
    assert!(checked > 20, "swept only {checked} glyphs, too few to mean anything");
    assert!(moved * 4 >= checked, "only {moved} of {checked} glyphs moved under Auto");
}

#[test]
fn capitals_share_one_baseline_and_one_cap_height() {
    let f = font("inter/InterVariable.ttf");
    let rows: Vec<(char, f64, f64)> = "HEZLOCUST"
        .chars()
        .map(|c| {
            let gid = f.glyph_id(c).unwrap_or_else(|| panic!("Inter maps {c}"));
            let Bounds { x_min: _, y_min: y0, x_max: _, y_max: y1 } =
                outline(&f, gid, 13.0, HintMode::Auto).and_then(|p| p.bounds()).expect("ink");
            (c, y0.floor(), y1.ceil())
        })
        .collect();
    let (_, bottom, top) = rows[0];
    for &(c, b, t) in &rows {
        assert_eq!(b, bottom, "{c} sits on a different baseline row than H ({b} vs {bottom})");
        assert_eq!(t, top, "{c} reaches a different cap height row than H ({t} vs {top})");
    }
}

#[test]
fn the_crossbar_of_h_survives_small_sizes() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H').expect("Inter maps H");
    for px in [8.0f32, 10.0, 12.0, 14.0] {
        let p = outline(&f, gid, px, HintMode::Auto).expect("H");
        let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } = p.bounds().expect("ink");
        let width = (x1 - x0) as f32;
        let bar = (1..100).any(|i| covered(&p, (y0 + (y1 - y0) * f64::from(i) / 100.0) as f32) > width * 0.95);
        assert!(bar, "H at {px} px has no line covered stem to stem: the crossbar was lost");
    }
}

#[test]
fn cff_declared_hints_are_used_and_differ_from_the_autohinter() {
    let f = font("stix-two-math/STIX2Math.otf");
    let (mut checked, mut differ) = (0usize, 0usize);
    for c in "HEZLOCUSTABDFGMNPR".chars() {
        let gid = f.glyph_id(c).unwrap_or_else(|| panic!("STIX maps {c}"));
        let declared = outline(&f, gid, 13.0, HintMode::Auto);
        assert_ne!(outline(&f, gid, 13.0, HintMode::None), declared, "{c}: Auto matched unhinted");
        checked += 1;
        differ += usize::from(declared != outline(&f, gid, 13.0, HintMode::AutoForce));
    }
    assert!(checked >= 15 && differ * 2 >= checked, "only {differ} of {checked} differ from the autohinter");
}

#[test]
fn serifs_survive_on_both_outline_formats() {
    for (label, rel) in
        [("STIX2Math (CFF)", "stix-two-math/STIX2Math.otf"), ("EBGaramond (TrueType)", "eb-garamond/EBGaramond.ttf")]
    {
        let f = font(rel);
        let gid = f.glyph_id('H').unwrap_or_else(|| panic!("{label} maps H"));
        let p = outline(&f, gid, 13.0, HintMode::AutoForce).expect("H");
        let Bounds { x_min: _, y_min: y0, x_max: _, y_max: y1 } = p.bounds().expect("ink");
        // Stems at 30%: mid-height is the crossbar, which covers as much as a serif.
        let at = |t: f64| covered(&p, (y0 + (y1 - y0) * t) as f32);
        let stems = at(0.3);
        assert!(at(0.95) > stems, "{label}: the top serif is not wider than the stems ({} vs {stems})", at(0.95));
        assert!(at(0.05) > stems, "{label}: the bottom serif is not wider than the stems ({} vs {stems})", at(0.05));
    }
}

// STIX's O is eight cubics, which hinted stay cubics: two quadratics through each one's controls'
// midpoint would sit 7 px off the curve at 200 px.
#[test]
fn a_cff_curve_stays_cubic_once_hinted() {
    let f = font("stix-two-math/STIX2Math.otf");
    let gid = f.glyph_id('O').expect("STIX maps O");
    let cubics = |mode| {
        let p = outline(&f, gid, 200.0, mode).expect("O");
        p.parts().0.iter().filter(|v| matches!(v, daegun::outline::Verb::Cubic)).count()
    };
    for mode in [HintMode::Auto, HintMode::AutoForce] {
        assert!(hinted(&f, gid, 200.0, mode), "{mode:?} did not hint O");
        assert_eq!(cubics(mode), cubics(HintMode::None), "{mode:?} lost O's cubics");
    }
}

// A zone catches an edge within half a pixel and is off past a 3/4 px overshoot, so no extreme moves
// past a pixel (plus the outline's 1/64). Caught in font units, N would grow 38 px at 2000.
#[test]
fn no_extreme_moves_past_a_pixel_at_display_sizes() {
    for rel in ["inter/InterVariable.ttf", "eb-garamond/EBGaramond.ttf", "stix-two-math/STIX2Math.otf"] {
        let f = font(rel);
        for px in [72.0, 200.0, 2000.0] {
            for ch in ('a'..='z').chain('A'..='Z') {
                let gid = f.glyph_id(ch).unwrap_or_else(|| panic!("{rel} maps {ch}"));
                let bounds = |mode| outline(&f, gid, px, mode).and_then(|p| p.bounds()).expect("ink");
                let (a, b) = (bounds(HintMode::None), bounds(HintMode::AutoForce));
                let moved = [a.x_min - b.x_min, a.y_min - b.y_min, a.x_max - b.x_max, a.y_max - b.y_max]
                    .into_iter()
                    .map(f64::abs)
                    .fold(0.0, f64::max);
                assert!(moved <= 1.0 + 1.0 / 64.0, "{rel} {ch} at {px} px: an extreme moved {moved:.3} px");
            }
        }
    }
}

// 30,000 rightward runs at six heights, none facing another: linking each to every other would be 10^9
// steps a call. Two copies pass the 65,535 points a hinted glyph may hold and are drawn unhinted.
#[test]
fn a_huge_glyph_autohints_in_time_or_not_at_all() {
    use crate::bytecode::build;
    let points: Vec<(i16, i16)> = (0..30_000).flat_map(|k: i16| [(0, k % 6 * 50), (900, k % 6 * 50)]).collect();
    let case = build::Case {
        points: Box::leak(points.into_boxed_slice()),
        font_tables: false,
        composite: Some((0, 0, 0, &[])),
        copies: 2,
        ..build::BOX
    };
    let base = std::fs::read(format!("{}/test-fixtures/hinted.ttf", crate::FONTS)).expect("hinted.ttf");
    let f = Font::from_vec(build::font(&base, &case), 0).expect("builds");
    let started = std::time::Instant::now();
    assert!(hinted(&f, GlyphId(1), 16.0, HintMode::AutoForce), "the 60,000-point glyph was not hinted");
    assert!(started.elapsed().as_secs_f64() < 2.0, "hinting took {:?}", started.elapsed());
    assert!(!hinted(&f, GlyphId(2), 16.0, HintMode::AutoForce), "120,000 points were hinted");
}
