use daegun::{Font, HintMode};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

// Geometry, not pixels: the outline a rasterizer of your own would receive.
fn outline(f: &Font, gid: u16, px: f32, mode: HintMode) -> Option<daegun::Path> {
    let mut p = daegun::Path::default();
    f.prepared_outline(gid, px, &[], &daegun::OutlineOptions::default().with_hinting(mode), &mut p)?;
    Some(p)
}

fn hinted(f: &Font, gid: u16, px: f32, mode: HintMode) -> bool {
    let mut sink = daegun::Path::default();
    f.prepared_outline(gid, px, &[], &daegun::OutlineOptions::default().with_hinting(mode), &mut sink)
        .is_some_and(|g| g.hinted)
}

// How much of the line at height `y` the outline covers, by non-zero winding.
fn covered(p: &daegun::Path, y: f32) -> f32 {
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for c in daegun::flatten(p, 0.001).expect("within the cap") {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if (a.1 <= y) != (b.1 <= y) {
                crossings.push((a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0), if b.1 > a.1 { 1 } else { -1 }));
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
    let gid = f.glyph_id('H' as u32).expect("Inter maps H");
    let plain = outline(&f, gid, 13.0, HintMode::None);
    assert_eq!(plain, outline(&f, gid, 13.0, HintMode::Subpixel),
        "InterVariable ships no hinting, so Subpixel must leave the outline alone – if this fails the fixture changed");
    assert!(!hinted(&f, gid, 13.0, HintMode::Subpixel), "bytecode hinting ran on a font with no bytecode");
    assert!(hinted(&f, gid, 13.0, HintMode::Auto), "HintMode::Auto did not run the autohinter");
    assert_ne!(plain, outline(&f, gid, 13.0, HintMode::Auto), "the autohinter ran and moved nothing");
}

#[test]
fn auto_reaches_a_cff_font() {
    let f = font("source-serif/SourceSerif4Variable-Roman.otf");
    let gid = f.glyph_id('H' as u32).expect("Source Serif maps H");
    assert!(hinted(&f, gid, 13.0, HintMode::Auto), "a CFF font reached HintMode::Auto and nothing hinted it");
    assert_ne!(outline(&f, gid, 13.0, HintMode::None), outline(&f, gid, 13.0, HintMode::Auto),
        "a CFF outline came back from HintMode::Auto unchanged");
}

#[test]
fn auto_force_overrides_the_fonts_own_bytecode() {
    let f = font("test-fixtures/hinted.ttf");
    let bytecode = outline(&f, 1, 16.0, HintMode::Subpixel).expect("hinted.ttf gid 1");
    assert!(hinted(&f, 1, 16.0, HintMode::Subpixel), "the fixture's bytecode did not run");
    assert_eq!(Some(&bytecode), outline(&f, 1, 16.0, HintMode::Auto).as_ref(), "Auto must defer to real bytecode");
    assert_ne!(Some(&bytecode), outline(&f, 1, 16.0, HintMode::AutoForce).as_ref(), "AutoForce kept the bytecode");
}

#[test]
fn hinting_shifts_the_box_by_at_most_a_couple_of_pixels_at_any_size() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('o' as u32).expect("Inter maps o");
    for px in [12.0, 64.0, 500.0, 2000.0] {
        let (a0, b0, a1, b1) = outline(&f, gid, px, HintMode::None).and_then(|p| p.bounds()).expect("o");
        let (c0, d0, c1, d1) = outline(&f, gid, px, HintMode::Auto).and_then(|p| p.bounds()).expect("o");
        let (dw, dh) = (((c1 - c0) - (a1 - a0)).abs(), ((d1 - d0) - (b1 - b0)).abs());
        assert!(dw <= 2.0 && dh <= 2.0, "at {px} ppem hinting changed the box by {dw:.2} x {dh:.2} px");
    }
}

#[test]
fn a_non_latin_font_declines_rather_than_mis_hinting() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    assert!(!hinted(&f, 5, 13.0, HintMode::AutoForce), "a font with no Latin coverage was hinted anyway");
    assert_eq!(outline(&f, 5, 13.0, HintMode::None), outline(&f, 5, 13.0, HintMode::AutoForce));
}

#[test]
fn a_sweep_of_glyphs_all_hint_under_auto() {
    let f = font("inter/InterVariable.ttf");
    let (mut checked, mut moved) = (0usize, 0usize);
    for gid in (1..400u16).step_by(7) {
        let Some(plain) = outline(&f, gid, 12.0, HintMode::None) else { continue };
        if plain.is_empty() {
            continue;
        }
        let auto = outline(&f, gid, 12.0, HintMode::Auto).unwrap_or_else(|| panic!("gid {gid} has an outline but not under Auto"));
        assert!(hinted(&f, gid, 12.0, HintMode::Auto), "gid {gid} came back unhinted under Auto");
        checked += 1;
        moved += usize::from(plain != auto);
    }
    assert!(checked > 20, "swept only {checked} glyphs, too few to mean anything");
    assert!(moved * 4 >= checked, "only {moved} of {checked} glyphs moved under Auto");
}

#[test]
fn capitals_share_one_baseline_and_one_cap_height() {
    let f = font("inter/InterVariable.ttf");
    let rows: Vec<(char, f64, f64)> = "HEZLOCUST".chars().map(|c| {
        let gid = f.glyph_id(c as u32).unwrap_or_else(|| panic!("Inter maps {c}"));
        let (_, y0, _, y1) = outline(&f, gid, 13.0, HintMode::Auto).and_then(|p| p.bounds()).expect("ink");
        (c, y0.floor(), y1.ceil())
    }).collect();
    let (_, bottom, top) = rows[0];
    for &(c, b, t) in &rows {
        assert_eq!(b, bottom, "{c} sits on a different baseline row than H ({b} vs {bottom})");
        assert_eq!(t, top, "{c} reaches a different cap height row than H ({t} vs {top})");
    }
}

#[test]
fn the_crossbar_of_h_survives_small_sizes() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("Inter maps H");
    for px in [8.0f32, 10.0, 12.0, 14.0] {
        let p = outline(&f, gid, px, HintMode::Auto).expect("H");
        let (x0, y0, x1, y1) = p.bounds().expect("ink");
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
        let gid = f.glyph_id(c as u32).unwrap_or_else(|| panic!("STIX maps {c}"));
        let declared = outline(&f, gid, 13.0, HintMode::Auto);
        assert_ne!(outline(&f, gid, 13.0, HintMode::None), declared, "{c}: Auto matched unhinted");
        checked += 1;
        differ += usize::from(declared != outline(&f, gid, 13.0, HintMode::AutoForce));
    }
    assert!(checked >= 15 && differ * 2 >= checked, "only {differ} of {checked} differ from the autohinter");
}

#[test]
fn serifs_survive_on_both_outline_formats() {
    for (label, rel) in [("STIX2Math (CFF)", "stix-two-math/STIX2Math.otf"), ("EBGaramond (TrueType)", "eb-garamond/EBGaramond.ttf")] {
        let f = font(rel);
        let gid = f.glyph_id('H' as u32).unwrap_or_else(|| panic!("{label} maps H"));
        let p = outline(&f, gid, 13.0, HintMode::AutoForce).expect("H");
        let (_, y0, _, y1) = p.bounds().expect("ink");
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
    let gid = f.glyph_id('O' as u32).expect("STIX maps O");
    let cubics = |mode| {
        let p = outline(&f, gid, 200.0, mode).expect("O");
        p.parts().0.iter().filter(|v| matches!(v, daegun::Verb::Cubic)).count()
    };
    for mode in [HintMode::Auto, HintMode::AutoForce] {
        assert!(hinted(&f, gid, 200.0, mode), "{mode:?} did not hint O");
        assert_eq!(cubics(mode), cubics(HintMode::None), "{mode:?} lost O's cubics");
    }
}

// As FreeType: a zone catches an edge within half a pixel and is off past a 3/4 px overshoot, so no
// extreme moves past a pixel (plus the outline's 1/64). Caught in font units, N would grow 38 px at 2000.
#[test]
fn no_extreme_moves_past_a_pixel_at_display_sizes() {
    for rel in ["inter/InterVariable.ttf", "eb-garamond/EBGaramond.ttf", "stix-two-math/STIX2Math.otf"] {
        let f = font(rel);
        for px in [72.0, 200.0, 2000.0] {
            for ch in ('a'..='z').chain('A'..='Z') {
                let gid = f.glyph_id(ch as u32).unwrap_or_else(|| panic!("{rel} maps {ch}"));
                let bounds = |mode| outline(&f, gid, px, mode).and_then(|p| p.bounds()).expect("ink");
                let (a, b) = (bounds(HintMode::None), bounds(HintMode::AutoForce));
                let moved = [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3].into_iter().map(f64::abs).fold(0.0, f64::max);
                assert!(moved <= 1.0 + 1.0 / 64.0, "{rel} {ch} at {px} px: an extreme moved {moved:.3} px");
            }
        }
    }
}

// FreeType 2.14.3's zones for these faces, from its aflatin trace: flats told from rounds by the
// level run around each extremum, and STIX's inverted ascender zone (706 over 673) averaged.
#[test]
fn blue_zones_match_freetype() {
    use daegun::daecore::daetype::hinting::auto::{AutoHinter, CollectPen};
    let cases: [(&str, [(f32, f32); 6]); 3] = [
        ("eb-garamond/EBGaramond.ttf", [(653.0, 664.0), (-5.0, -14.0), (705.0, 705.0), (405.0, 414.0), (-3.0, -14.0), (-285.0, -287.0)]),
        ("stix-two-math/STIX2Math.otf", [(657.0, 669.0), (0.0, -12.0), (689.0, 689.0), (473.0, 485.0), (0.0, -10.0), (-220.0, -235.0)]),
        ("inter/InterVariable.ttf", [(1490.0, 1510.0), (0.0, -20.0), (1490.0, 1539.0), (1118.0, 1132.0), (0.0, -24.0), (-418.0, -426.0)]),
    ];
    for (rel, want) in cases {
        let f = font(rel);
        let mut resolve = |c: char| f.glyph_id(c as u32);
        let mut outline_of = |g: u16| {
            let mut pen = CollectPen::new();
            f.outline_glyph(g, &mut pen)?;
            Some(pen.finish())
        };
        let zones = AutoHinter::compute_zones(f.upm(), &mut resolve, &mut outline_of);
        let got: Vec<(f32, f32)> = zones.zones.iter().map(|z| (z.reference, z.overshoot)).collect();
        assert_eq!(got, want, "{rel}");
    }
}

// 30,000 rightward runs at six heights, none facing another: linking each to every other would be 10^9
// steps a call. Two copies pass FreeType's 65,535 points and are drawn unhinted.
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
    let f = Font::from_vec(build::font(&base, &case)).expect("builds");
    let started = std::time::Instant::now();
    assert!(hinted(&f, 1, 16.0, HintMode::AutoForce), "the 60,000-point glyph was not hinted");
    assert!(started.elapsed().as_secs_f64() < 2.0, "hinting took {:?}", started.elapsed());
    assert!(!hinted(&f, 2, 16.0, HintMode::AutoForce), "120,000 points were hinted");
}
