use daegun::{Cap, Font, HintMode, Join, OutlineOptions, Path, StrokeStyle};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

fn prepared(f: &Font, gid: u16, px: f32, opts: &OutlineOptions) -> (Path, daegun::PreparedGlyph) {
    let mut p = Path::default();
    let g = f.prepared_outline(gid, px, &[], opts, &mut p).expect("the glyph has an outline");
    (p, g)
}

fn width_height(p: &Path) -> (f64, f64) {
    let (x0, y0, x1, y1) = p.bounds().expect("ink");
    (x1 - x0, y1 - y0)
}

const FACES: [&str; 3] = ["inter/InterVariable.ttf", "eb-garamond/EBGaramond.ttf", "stix-two-math/STIX2Math.otf"];

#[test]
fn a_plain_prepared_outline_is_the_scaled_outline() {
    for rel in FACES {
        let f = font(rel);
        for gid in 1..f.num_glyphs().min(200) {
            let mut want = Path::default();
            let s = 48.0 / f64::from(f.upm());
            let mut scaled = daegun::TransformPen::new(&mut want, [s as f32 as f64, 0.0, 0.0, s as f32 as f64, 0.0, 0.0]);
            if f.outline_glyph_instanced(gid, &[], &mut scaled).is_none() {
                continue;
            }
            let (got, _) = prepared(&f, gid, 48.0, &OutlineOptions::default());
            assert_eq!(got, want, "{rel} gid {gid}");
        }
    }
}

#[test]
fn a_hinted_prepared_outline_is_the_hinted_outline() {
    let mut compared = 0usize;
    for rel in FACES {
        let f = font(rel);
        for gid in 1..f.num_glyphs().min(200) {
            let Some(h) = f.hinted_glyph(gid, 16.0, &[], HintMode::AutoForce) else { continue };
            let mut want = Path::default();
            daegun::draw_hinted(&h, &mut want);
            let (got, g) = prepared(&f, gid, 16.0, &OutlineOptions::default().with_hinting(HintMode::AutoForce));
            assert!(g.hinted, "{rel} gid {gid}: hinted_glyph hinted it, prepared_outline did not");
            assert_eq!(got, want, "{rel} gid {gid}");
            compared += 1;
        }
    }
    assert!(compared > 300, "only {compared} hinted outlines compared");
}

// A second route to the same outline, built in font units from public parts. Not pixels: an overlap
// union flips on a 1e-4 unit nudge, so two equal outlines can rasterize apart.
fn recipe(f: &Font, gid: u16, px: f32, o: &OutlineOptions) -> Path {
    let upm = f32::from(f.upm());
    let t = match (o.oblique, o.transform) {
        (Some(k), None) => Some([1.0, 0.0, k, 1.0, 0.0, 0.0]),
        (None, t) => t,
        (Some(_), Some(_)) => unreachable!("not a case below"),
    };
    let mut base = Path::default();
    match t {
        None => f.outline_glyph_instanced(gid, &[], &mut base),
        Some(m) => f.outline_glyph_instanced(gid, &[], &mut daegun::TransformPen::new(&mut base, m.map(f64::from))),
    }
    .expect("an outline");
    let tolerance = 0.25 * upm / px;
    let mut out = Path::default();
    match (o.stroke, o.embolden) {
        (Some(style), _) => daegun::stroke(&base, &style, tolerance, &mut out),
        (None, Some(units)) => {
            use daegun::OutlinePen;
            base.replay(None, &mut out);
            let mut ring = Path::default();
            daegun::stroke(&base, &StrokeStyle { width: units, join: Join::Round, cap: Cap::Round }, tolerance, &mut ring);
            let loops = polygons(&ring);
            // The ring winds as the fill does, turned whole when the two disagree.
            let turn = signed_area(&daegun::flatten(&base, 0.01).expect("within the cap")) * signed_area(&loops) < 0.0;
            for mut l in loops {
                if turn {
                    l.reverse();
                }
                out.move_to(l[0].0, l[0].1);
                l[1..].iter().for_each(|p| out.line_to(p.0, p.1));
                out.close();
            }
        }
        (None, None) => return base,
    }
    out
}

fn polygons(lines: &Path) -> Vec<Vec<(f32, f32)>> {
    let (verbs, points) = lines.parts();
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    for (v, &p) in verbs.iter().filter(|v| !matches!(v, daegun::Verb::Close)).zip(points) {
        match v {
            daegun::Verb::Move => out.push(vec![p]),
            daegun::Verb::Line => out.last_mut().expect("a contour").push(p),
            _ => unreachable!("a stroke is drawn in lines"),
        }
    }
    out
}

fn signed_area(contours: &[Vec<(f32, f32)>]) -> f64 {
    let mut sum = 0.0;
    for c in contours {
        for (i, &(x0, y0)) in c.iter().enumerate() {
            let (x1, y1) = c[(i + 1) % c.len()];
            sum += f64::from(x0) * f64::from(y1) - f64::from(x1) * f64::from(y0);
        }
    }
    sum
}

#[test]
fn prepared_outlines_match_a_route_built_from_parts() {
    let skew = [0.8f32, 0.1, -0.2, 1.1, 30.0, -20.0];
    let cases = [
        OutlineOptions::default(),
        OutlineOptions::default().with_transform(skew),
        OutlineOptions::default().with_oblique(0.25),
        OutlineOptions::default().with_stroke(StrokeStyle { width: 40.0, join: Join::Round, cap: Cap::Round }),
        OutlineOptions::default().with_stroke(StrokeStyle { width: 40.0, join: Join::Miter { limit: 4.0 }, cap: Cap::Butt }),
        OutlineOptions::default().with_embolden(60.0),
    ];
    let (mut compared, mut worst) = (0usize, 0.0f32);
    for rel in FACES {
        let f = font(rel);
        for ch in "Hamgo4&8@".chars() {
            let Some(gid) = f.glyph_id(ch as u32) else { continue };
            for px in [16.0f32, 48.0] {
                let inv = f64::from(f32::from(f.upm()) / px);
                for (i, o) in cases.iter().enumerate() {
                    let (got, g) = prepared(&f, gid, px, o);
                    assert!(!g.hinted, "{rel} {ch} case {i}: hinted with no hinting asked for");
                    let mut back = Path::default();
                    got.replay(Some(&[inv, 0.0, 0.0, inv, 0.0, 0.0]), &mut back);
                    let want = recipe(&f, gid, px, o);
                    let ((bv, bp), (wv, wp)) = (back.parts(), want.parts());
                    assert_eq!(bv, wv, "{rel} {ch} at {px} px, case {i}: the verbs differ");
                    for (a, b) in bp.iter().zip(wp) {
                        worst = worst.max((a.0 - b.0).abs().max((a.1 - b.1).abs()));
                    }
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 280, "only {compared} outlines compared");
    assert!(worst <= BOUND, "a point strayed {worst} font units from the recipe");
}

// Measured at 1.2e-4: two float steps lost to the round trip through pixels.
const BOUND: f32 = 1e-3;

#[test]
fn hinting_keeps_the_stroke_width_the_slant_and_pixel_advances() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let (px, upm) = (64.0f32, f64::from(f.upm()));
    let hinted = OutlineOptions::default().with_hinting(HintMode::AutoForce);

    let (plain, g) = prepared(&f, gid, px, &hinted);
    assert!(g.hinted, "Inter's H was not hinted, so this proves nothing");
    let stroke = StrokeStyle { width: 40.0, join: Join::Round, cap: Cap::Butt };
    let (stroked, _) = prepared(&f, gid, px, &hinted.with_stroke(stroke));
    let grew = width_height(&stroked).0 - width_height(&plain).0;
    let want = 40.0 * f64::from(px) / upm;
    assert!((grew - want).abs() < 0.5, "a 40-unit stroke grew the hinted H by {grew} px, not {want}");
    let (bold, _) = prepared(&f, gid, px, &hinted.with_embolden(40.0));
    let grew = width_height(&bold).0 - width_height(&plain).0;
    assert!((grew - want).abs() < 0.5, "a 40-unit embolden grew the hinted H by {grew} px, not {want}");

    // A transform's offsets are in font units, hinted or not.
    let shift = 100.0 * f64::from(px) / upm;
    for opts in [OutlineOptions::default(), hinted] {
        let (at, _) = prepared(&f, gid, px, &opts);
        let (moved, _) = prepared(&f, gid, px, &opts.with_transform([1.0, 0.0, 0.0, 1.0, 100.0, 0.0]));
        let by = moved.bounds().expect("ink").0 - at.bounds().expect("ink").0;
        assert!((by - shift).abs() < 1e-3, "an offset of 100 units moved the H {by} px, not {shift}");
    }

    let (slanted, _) = prepared(&f, gid, px, &hinted.with_oblique(0.5));
    let (w0, h0) = width_height(&plain);
    let leaned = width_height(&slanted).0 - w0;
    assert!((leaned - 0.5 * h0).abs() < 1.0, "an oblique of 0.5 widened the hinted H by {leaned}, not {}", 0.5 * h0);

    // The advance is the font's, on the font's first hinted call as on any other.
    let want = f.shape("H", &[], false).expect("shapes").advances[0] / 1000.0 * f64::from(px);
    assert!((f64::from(g.advance_width) - want).abs() < 0.01, "first hinted advance {} against {want} px", g.advance_width);
    for opts in [OutlineOptions::default(), hinted] {
        let (_, g) = prepared(&f, gid, px, &opts);
        assert!((f64::from(g.advance_width) - want).abs() < 0.01, "advance {} against {want} px", g.advance_width);
    }
}

// Every mode but Classic leaves x where scaling put it, so x at 16.4 px is x at 16 px scaled up. The
// faces cover all three hinters: bytecode, CFF hints and the autohinter.
#[test]
fn a_fractional_size_is_hinted_at_that_size() {
    let mut compared = 0usize;
    for rel in ["test-fixtures/hinted.ttf", FACES[0], FACES[1], FACES[2]] {
        let f = font(rel);
        for mode in [HintMode::Subpixel, HintMode::Auto, HintMode::AutoForce] {
            for ch in "ABCDHamgo".chars() {
                let Some(gid) = f.glyph_id(ch as u32) else { continue };
                let Some(whole) = f.hinted_glyph(gid, 16.0, &[], mode) else { continue };
                let part = f.hinted_glyph(gid, 16.4, &[], mode).expect("hinted at 16 px but not at 16.4");
                assert_eq!(whole.x.len(), part.x.len(), "{rel} {ch} {mode:?}: the point count changed");
                for (&a, &b) in whole.x.iter().zip(&part.x) {
                    let want = f64::from(a) * 16.4 / 16.0;
                    assert!((f64::from(b) - want).abs() <= 2.0, "{rel} {ch} {mode:?}: x is {b}, not about {want}");
                }
                compared += 1;
            }
        }
    }
    assert!(compared > 60, "only {compared} hinted glyphs compared");

    // The fixture's B rounds its point 2 only below 30 ppem, and MPPEM reads 29.7 px as 30.
    let f = font("test-fixtures/hinted.ttf");
    let b = f.glyph_id('B' as u32).expect("B");
    let y2 = |px: f32| f.hinted_glyph(b, px, &[], HintMode::Subpixel).expect("B hints").y[2];
    assert_eq!(y2(29.3) % 64, 0, "29.3 px did not take the below-30 branch");
    assert_ne!(y2(29.7) % 64, 0, "29.7 px took the below-30 branch, so MPPEM did not round it to 30");

    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let (hinted, g) = prepared(&f, gid, 16.4, &OutlineOptions::default().with_hinting(HintMode::AutoForce));
    assert!(g.hinted, "Inter's H was not hinted at 16.4 px");
    let (plain, _) = prepared(&f, gid, 16.4, &OutlineOptions::default());
    let (got, want) = (width_height(&hinted).0, width_height(&plain).0);
    assert!((got - want).abs() < 1.0 / 32.0, "the hinted H is {got} px wide at 16.4 px, unhinted {want}");
}

// y is the half hinting moves, so it shows the size the hinter worked at. The autohinter snaps an H's
// top to the pixel nearest its height: at a size where that pixel is not the rounded size's, it must win.
#[test]
fn a_fractional_size_hints_heights_at_that_size() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let mut units = Path::default();
    f.outline_glyph_instanced(gid, &[], &mut units).expect("H");
    let top = units.bounds().expect("ink").3 / f64::from(f.upm());
    let px = (200..600)
        .map(|t| t as f32 / 10.0)
        .find(|&px| (top * f64::from(px)).round() != (top * f64::from(px.round())).round())
        .expect("a size that tells the two apart");
    let hinted = f.hinted_glyph(gid, px, &[], HintMode::AutoForce).expect("H hints");
    let hinted_top = f64::from(*hinted.y.iter().max().expect("points")) / 64.0;
    assert_eq!(hinted_top, (top * f64::from(px)).round(), "at {px} px the H's top snapped to the rounded size");

    // The CFF hinter fits stems at the size asked: worked at whole sizes, every size from 20 to 60 px
    // would hint STIX's H as the whole size it rounds to.
    let f = font("stix-two-math/STIX2Math.otf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let y = |px: f32| f.hinted_glyph(gid, px, &[], HintMode::Auto).expect("H hints").y;
    let sizes = (200..600).map(|t| t as f32 / 10.0).filter(|px| px.fract() != 0.0);
    assert!(sizes.into_iter().any(|px| y(px) != y(px.round())), "the CFF hinter worked at whole sizes from 20 to 60 px");
}

// Hinting reaches 65,535 ppem; past that a glyph has to come out at its size, unhinted.
#[test]
fn a_size_past_the_hinters_reach_comes_out_unhinted() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let hinted = OutlineOptions::default().with_hinting(HintMode::AutoForce);
    assert!(prepared(&f, gid, 65535.0, &hinted).1.hinted, "the largest ppem was not hinted");
    assert!(f.hinted_glyph(gid, 70000.0, &[], HintMode::AutoForce).is_none(), "70,000 ppem was hinted");
    let (got, g) = prepared(&f, gid, 70000.0, &hinted);
    assert!(!g.hinted, "70,000 ppem claims to be hinted");
    assert_eq!(got, prepared(&f, gid, 70000.0, &OutlineOptions::default()).0, "70,000 px drew another size");
}

#[test]
fn prepared_outlines_refuse_what_they_cannot_place() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let mut sink = Path::default();
    let mut try_with = |px: f32, o: OutlineOptions| f.prepared_outline(gid, px, &[], &o, &mut sink).is_some();
    assert!(!try_with(0.0, OutlineOptions::default()), "zero px was accepted");
    assert!(!try_with(f32::NAN, OutlineOptions::default()), "a NaN size was accepted");
    assert!(!try_with(16.0, OutlineOptions::default().with_oblique(f32::INFINITY)), "an infinite oblique was accepted");
    assert!(!try_with(16.0, OutlineOptions::default().with_transform([1.0, 0.0, 0.0, f32::NAN, 0.0, 0.0])),
        "a NaN transform was accepted");
    let style = StrokeStyle { width: f32::NAN, join: Join::Round, cap: Cap::Round };
    assert!(!try_with(16.0, OutlineOptions::default().with_stroke(style)), "a NaN stroke width was accepted");
    let style = StrokeStyle { width: f32::INFINITY, ..style };
    assert!(!try_with(16.0, OutlineOptions::default().with_stroke(style)), "an infinite stroke width was accepted");
    assert!(f.prepared_outline(f.num_glyphs(), 16.0, &[], &OutlineOptions::default(), &mut sink).is_none(),
        "a gid past the end was accepted");

    let space = f.glyph_id(' ' as u32).expect("space");
    let mut nothing = Path::default();
    let g = f.prepared_outline(space, 16.0, &[], &OutlineOptions::default(), &mut nothing).expect("a space has an advance");
    assert!(nothing.is_empty() && g.advance_width > 0.0, "the space drew ink or lost its advance");
}

// The shear comes first and the caller's transform after it, so a quarter turn turns the slanted
// glyph. The other order slants the turned one, and only the matrix tells them apart.
#[test]
fn an_oblique_is_applied_before_the_transform() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let (px, k) = (48.0f32, 0.25f32);
    let turn = [0.0f32, 1.0, -1.0, 0.0, 0.0, 0.0];
    let (got, _) = prepared(&f, gid, px, &OutlineOptions::default().with_transform(turn).with_oblique(k));
    let s = f64::from(px / f32::from(f.upm()));
    let mut want = Path::default();
    let shear_then_turn = [0.0, s, -s, f64::from(k) * s, 0.0, 0.0];
    f.outline_glyph_instanced(gid, &[], &mut daegun::TransformPen::new(&mut want, shear_then_turn)).expect("H");
    let ((gv, gp), (wv, wp)) = (got.parts(), want.parts());
    assert_eq!(gv, wv, "the verbs differ");
    let worst = gp.iter().zip(wp).map(|(a, b)| (a.0 - b.0).abs().max((a.1 - b.1).abs())).fold(0.0f32, f32::max);
    assert!(worst < 1e-2, "a point is {worst} px from the shear-then-turn outline");
}

// A stroke and an embolden do not combine: the stroke is drawn, and the embolden must not widen an
// advance it did not draw.
#[test]
fn a_stroke_wins_over_an_embolden() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let stroke = OutlineOptions::default().with_stroke(StrokeStyle { width: 40.0, join: Join::Round, cap: Cap::Round });
    let (alone, g_alone) = prepared(&f, gid, 48.0, &stroke);
    let (both, g_both) = prepared(&f, gid, 48.0, &stroke.with_embolden(120.0));
    assert_eq!(both, alone, "the embolden changed what the stroke drew");
    assert_eq!(g_both.advance_width, g_alone.advance_width, "an embolden that was not drawn widened the advance");
}

// With one unit to the em, every coordinate at 65,535 px scales past what the hinters can add in 26.6,
// where the scale is pinned and the glyph would collapse: it is drawn unhinted instead.
#[test]
fn hinting_a_tiny_em_at_the_largest_size_falls_back_to_the_outline() {
    for (rel, mode, text) in [
        ("inter/InterVariable.ttf", HintMode::AutoForce, "OoH"),
        ("stix-two-math/STIX2Math.otf", HintMode::Auto, "HEO"),
        ("test-fixtures/hinted.ttf", HintMode::Subpixel, "ABE"),
    ] {
        let f = font(rel);
        let tables: std::collections::BTreeMap<String, std::borrow::Cow<[u8]>> = f
            .table_tags()
            .into_iter()
            .map(|tag| {
                let mut bytes = f.table(tag).expect("listed").to_vec();
                if tag == "head" {
                    bytes[18..20].copy_from_slice(&1u16.to_be_bytes());
                }
                (tag.to_string(), std::borrow::Cow::Owned(bytes))
            })
            .collect();
        let tiny = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
        assert_eq!(tiny.upm(), 1, "{rel}: the patch did not take");
        for ch in text.chars() {
            let gid = tiny.glyph_id(ch as u32).expect("mapped");
            assert!(tiny.hinted_glyph(gid, 65_535.0, &[], mode).is_none(), "{rel} {ch}: hinted past the scale's limit");
            let (got, g) = prepared(&tiny, gid, 65_535.0, &OutlineOptions::default().with_hinting(mode));
            assert!(!g.hinted, "{rel} {ch}: flagged hinted");
            assert_eq!(got, prepared(&tiny, gid, 65_535.0, &OutlineOptions::default()).0, "{rel} {ch}");
        }
    }
}

// An embolden widens both advances, as FreeType's does. A font with no vertical metrics has no height
// to widen, and says so with 0.
#[test]
fn an_embolden_widens_the_advance_height_too() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let gid = f.glyph_id(0x65E5).expect("日");
    let (_, plain) = prepared(&f, gid, 48.0, &OutlineOptions::default());
    let (_, bold) = prepared(&f, gid, 48.0, &OutlineOptions::default().with_embolden(60.0));
    assert!(plain.advance_height > 0.0, "Source Han Sans has vertical metrics but no advance height");
    let want = 60.0 * 48.0 / f32::from(f.upm());
    assert!((bold.advance_height - plain.advance_height - want).abs() < 0.01,
        "the advance height grew {}, not {want}", bold.advance_height - plain.advance_height);

    let inter = font("inter/InterVariable.ttf");
    let h = inter.glyph_id('H' as u32).expect("H");
    let (_, g) = prepared(&inter, h, 48.0, &OutlineOptions::default().with_embolden(60.0));
    assert_eq!(g.advance_height, 0.0, "a font without vertical metrics gave an advance height");

    // A vmtx with no vhea to count its entries is no vertical metrics either.
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = inter.table_tags().into_iter()
        .map(|t| (t.to_string(), inter.table(t).expect("listed").to_vec()))
        .collect();
    tables.insert("vmtx".into(), vec![0x04, 0x00, 0x00, 0x00]);
    let half = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    let (_, g) = prepared(&half, h, 48.0, &OutlineOptions::default().with_embolden(60.0));
    assert_eq!(g.advance_height, 0.0, "a vmtx without vhea gave an advance height");
}

// hinted.ttf with one glyph replaced. loca is rewritten long, so the new glyph can be any size.
pub(crate) fn hinted_with(gid: u16, glyph: &[u8]) -> Font {
    patched("test-fixtures/hinted.ttf", &[(gid, glyph.to_vec())])
}

// A glyf font with some glyphs replaced. loca is rewritten long, so a new glyph can be any size.
fn patched(rel: &str, glyphs: &[(u16, Vec<u8>)]) -> Font {
    let f = font(rel);
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    let (loca, glyf) = (f.table("loca").expect("loca"), f.table("glyf").expect("glyf"));
    let at = |g: usize| 2 * usize::from(u16::from_be_bytes([loca[2 * g], loca[2 * g + 1]]));
    let (mut new_glyf, mut new_loca) = (Vec::new(), Vec::new());
    for g in 0..usize::from(f.num_glyphs()) {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        let new = glyphs.iter().find(|(gid, _)| usize::from(*gid) == g).map(|(_, data)| data.as_slice());
        new_glyf.extend_from_slice(new.unwrap_or(&glyf[at(g)..at(g + 1)]));
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    tables.get_mut("head").expect("head")[50..52].copy_from_slice(&1i16.to_be_bytes());
    tables.insert("glyf".into(), new_glyf);
    tables.insert("loca".into(), new_loca);
    Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses")
}

// One contour of on-curve points that climb 1000 units and fall back every 2 units across.
fn zigzag_glyph(n: u16) -> Vec<u8> {
    let mut g = Vec::new();
    for v in [1, 0, 0, 2 * n as i16, 1000, n as i16 - 1, 0] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g.extend(std::iter::repeat_n(1u8, n.into()));
    for i in 0..n {
        g.extend_from_slice(&(if i == 0 { 0i16 } else { 2 }).to_be_bytes());
    }
    for i in 0..n {
        g.extend_from_slice(&(if i == 0 { 0i16 } else if i % 2 == 1 { 1000 } else { -1000 }).to_be_bytes());
    }
    g
}

// Each turn of the zigzag, stroked this wide and round, takes 256 points: 10,000 turns take millions.
// Refused whole, with nothing drawn and no glyph to report, where a narrow stroke still draws.
#[test]
fn a_stroke_past_the_point_cap_is_refused() {
    let probe = font("test-fixtures/hinted.ttf");
    let gid = probe.glyph_id('D' as u32).expect("D");
    let f = hinted_with(gid, &zigzag_glyph(10_000));
    let round = |width| StrokeStyle { width, join: Join::Round, cap: Cap::Round };
    let (narrow, _) = prepared(&f, gid, 1024.0, &OutlineOptions::default().with_stroke(round(10.0)));
    assert!(narrow.parts().1.len() > 20_000, "the zigzag glyph did not draw: {} points", narrow.parts().1.len());
    for o in [OutlineOptions::default().with_stroke(round(40_000.0)), OutlineOptions::default().with_embolden(40_000.0)] {
        let mut p = Path::default();
        assert!(f.prepared_outline(gid, 1024.0, &[], &o, &mut p).is_none(), "{o:?} was drawn");
        assert!(p.is_empty(), "{o:?} drew {} points before it was refused", p.parts().1.len());
    }
}

// A square from (100, 0) to (400, 600) running `program` as its glyph program.
fn square_glyph(program: &[u8]) -> Vec<u8> {
    let mut g = Vec::new();
    for v in [1, 100, 0, 400, 600, 3, program.len() as i16] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g.extend_from_slice(program);
    g.extend([1u8; 4]);
    for v in [100i16, 300, 0, -300, 0, 0, 600, 0] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g
}

// The hinting context, with its scaled CVT, is kept for the size it was built at. Kept for the rounded
// size, 16.4 px after 16 px would move a point by CVT 1 as scaled for 16, which this program does.
#[test]
fn the_hinting_context_is_kept_per_exact_size() {
    let gid = font("test-fixtures/hinted.ttf").glyph_id('D' as u32).expect("D");
    // SVTCA[y], PUSHB[0, 1], RCVT, SHPIX: point 0 rises by CVT 1, 100 units.
    let program = [0x00, 0xB1, 0x00, 0x01, 0x45, 0x38];
    let classic = OutlineOptions::default().with_hinting(HintMode::Classic);
    let draw = |f: &Font, px| prepared(f, gid, px, &classic).0;
    let fresh = draw(&hinted_with(gid, &square_glyph(&program)), 16.4);
    assert_ne!(fresh, draw(&hinted_with(gid, &square_glyph(&[])), 16.4), "the program moved nothing");
    let f = hinted_with(gid, &square_glyph(&program));
    draw(&f, 16.0);
    assert_eq!(draw(&f, 16.4), fresh, "16.4 px after 16 px was hinted with 16 px's context");
}

// Finite options can still overflow once scaled to 65,535 px or composed with each other. Each is
// refused with nothing drawn, rather than drawing an empty stroke, an infinite advance or infinities.
#[test]
fn finite_options_that_overflow_are_refused() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let hinted = OutlineOptions::default().with_hinting(HintMode::AutoForce);
    let wide = StrokeStyle { width: 1e38, join: Join::Round, cap: Cap::Round };
    for (what, o) in [
        ("a hinted stroke of 1e38", hinted.with_stroke(wide)),
        ("a stroke of 3e37", OutlineOptions::default().with_stroke(StrokeStyle { width: 3e37, ..wide })),
        ("an embolden of 1e38", OutlineOptions::default().with_embolden(1e38)),
        ("an offset of 3e38", OutlineOptions::default().with_transform([1.0, 0.0, 0.0, 1.0, 3e38, 0.0])),
        ("an oblique of 1e30 on a scale of 1e10", OutlineOptions::default()
            .with_transform([1e10, 0.0, 0.0, 1.0, 0.0, 0.0]).with_oblique(1e30)),
    ] {
        let mut p = Path::default();
        let g = f.prepared_outline(gid, 65535.0, &[], &o, &mut p);
        assert!(g.is_none() && p.is_empty(), "{what}: {g:?} with {} points", p.parts().1.len());
    }

    // With no option at all, a glyph with a point past one em (a control point will do) and an advance
    // short of it overflows at f32::MAX px on the way to the pen.
    let upm = f32::from(f.upm());
    let tall = (0..f.num_glyphs())
        .find(|&g| {
            let mut p = Path::default();
            f.outline_glyph(g, &mut p).is_some()
                && f.advance_widths(&[g], &[])[0] < 1000.0
                && p.parts().1.iter().any(|q| q.0.abs() > upm || q.1.abs() > upm)
        })
        .expect("Inter has a glyph with a point past one em and an advance short of it");
    let mut p = Path::default();
    let g = f.prepared_outline(tall, f32::MAX, &[], &OutlineOptions::default(), &mut p);
    assert!(g.is_none() && p.is_empty(), "gid {tall} at f32::MAX px: {g:?} with {} points", p.parts().1.len());
}

// Ten composites, each of the next at a scale of 1.99994 on both axes, over 4,000 points that climb
// 32,767 units apiece: a point 1.4e14 units out. Its scale is far under any bound a glyph could set.
#[test]
fn a_plain_outline_that_overflows_once_scaled_is_refused() {
    let mut glyphs: Vec<(u16, Vec<u8>)> = (1..=10u16)
        .map(|g| {
            let mut c: Vec<u8> = Vec::new();
            for v in [-1i16, 0, 0, 0, 0, 0x0083, (g + 1) as i16, 0, 0, 0x7FFF, 0x7FFF, 0x7FFF, 0x7FFF] {
                c.extend(v.to_be_bytes());
            }
            (g, c)
        })
        .collect();
    let mut simple: Vec<u8> = Vec::new();
    for v in [1i16, 0, 0, 0, 0, 3999, 0] {
        simple.extend(v.to_be_bytes());
    }
    simple.extend([1u8; 4000]);
    for _ in 0..2 {
        (0..4000).for_each(|_| simple.extend(32767i16.to_be_bytes()));
    }
    glyphs.push((11, simple));
    let f = patched("structure/TestShapeLana.ttf", &glyphs);
    let mut whole = Path::default();
    f.outline_glyph(1, &mut whole).expect("the chain draws");
    let reach = whole.parts().1.iter().fold(0.0f32, |m, p| m.max(p.0.abs()).max(p.1.abs()));
    assert!(reach > 1e14 && reach.is_finite(), "the chain reached only {reach}");

    let px = 5e24 * f32::from(f.upm());
    let mut p = Path::default();
    let g = f.prepared_outline(1, px, &[], &OutlineOptions::default(), &mut p);
    assert!(g.is_none() && p.is_empty(), "{g:?} with {} points", p.parts().1.len());
}

// The transform reaches the outline only: under a half scale, a 40-unit stroke or embolden still widens
// the glyph by 40 units, and the advances stay the font's.
#[test]
fn the_transform_scales_neither_a_stroke_an_embolden_nor_the_advances() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H' as u32).expect("H");
    let (px, upm) = (64.0f32, f64::from(f.upm()));
    let half = OutlineOptions::default().with_transform([0.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
    let (plain, g_plain) = prepared(&f, gid, px, &half);
    let want = 40.0 * f64::from(px) / upm;
    let stroke = StrokeStyle { width: 40.0, join: Join::Round, cap: Cap::Butt };
    for (what, opts) in [("stroke", half.with_stroke(stroke)), ("embolden", half.with_embolden(40.0))] {
        let (wide, _) = prepared(&f, gid, px, &opts);
        let grew = width_height(&wide).0 - width_height(&plain).0;
        assert!((grew - want).abs() < 0.5, "a 40-unit {what} under a half scale grew the H by {grew} px, not {want}");
    }
    let (_, g_whole) = prepared(&f, gid, px, &OutlineOptions::default());
    assert_eq!(g_plain.advance_width, g_whole.advance_width, "the half scale halved the advance");
    let (_, g_bold) = prepared(&f, gid, px, &half.with_embolden(40.0));
    assert!((f64::from(g_bold.advance_width - g_whole.advance_width) - want).abs() < 0.01, "the emboldened advance");
}
