use alloc::vec::Vec;
use crate::daecore::daetype::outline::OutlinePen;
use crate::daecore::daetype::Tables;
use crate::daecore::daetype::format::sanitize::Reads;
use crate::Tag;

#[derive(Default)]
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv { Fnv(0xcbf2_9ce4_8422_2325) }
    fn u64(&mut self, v: u64) { self.0 = (self.0 ^ v).wrapping_mul(0x100_0000_01b3); }
    fn i64(&mut self, v: i64) { self.u64(v as u64); }
    fn f32(&mut self, v: f32) { self.u64(u64::from(v.to_bits())); }
    fn bytes(&mut self, b: &[u8]) { for &x in b { self.u64(u64::from(x)); } }
    fn done(self) -> u64 { self.0 }
}

impl OutlinePen for Fnv {
    fn move_to(&mut self, x: f32, y: f32) { self.u64(1); self.f32(x); self.f32(y); }
    fn line_to(&mut self, x: f32, y: f32) { self.u64(2); self.f32(x); self.f32(y); }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
        self.u64(3);
        for v in [a, b, x, y] { self.f32(v); }
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.u64(4);
        for v in [a, b, c, d, x, y] { self.f32(v); }
    }
    fn close(&mut self) { self.u64(5); }
}

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

type Map = Tables;

fn bytes_of(rel: &str) -> Vec<u8> {
    let path = format!("{FONTS}/{rel}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

fn tables(rel: &str) -> Map {
    crate::daecore::daetype::decoder::extract_ttf_tables(&bytes_of(rel))
        .unwrap_or_else(|e| panic!("{rel} did not parse: {e}"))
}

fn n_glyphs(map: &Map) -> u32 {
    map.get("maxp").and_then(|m| crate::daecore::daetype::decoder::read_u16_be(m, 4)).map_or(0, u32::from)
}

fn loca_of(map: &Map) -> Option<Vec<usize>> {
    let fmt = crate::daecore::daetype::decoder::read_i16_be(map.get("head")?, 50)?;
    crate::daecore::daetype::instancer::parse_loca(map, fmt, n_glyphs(map) as usize).ok()
}

fn check(what: &str, got: u64, want: u64) {
    assert_eq!(
        got, want,
        "\n{what} fingerprint moved.\n  recorded 0x{want:016x}\n  computed 0x{got:016x}\n\
         If the change was deliberate, update the constant in this file in the same commit.\n"
    );
}

const GLYF_FONTS: &[&str] = &[
    "eb-garamond/EBGaramond.ttf",
    "inter/InterVariable.ttf",
    "scheherazade-new/ScheherazadeNew-Regular.ttf",
    "noto-devanagari/NotoSansDevanagari.ttf",
    "noto-khmer/NotoSansKhmer.ttf",
    "noto-myanmar/NotoSansMyanmar.ttf",
    "bungee-tint/BungeeTint-Regular.ttf",
];

const CFF_FONTS: &[&str] = &["stix-two-math/STIX2Math.otf", "aat/TestKERNOne.otf"];

#[test]
fn glyf_outlines() {
    let mut f = Fnv::new();
    for rel in GLYF_FONTS {
        let map = tables(rel);
        let Some(loca) = loca_of(&map) else { continue };
        for gid in 0..n_glyphs(&map) {
            let _ = crate::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut f);
        }
    }
    check("glyf outlines", f.done(), 0x26a9_d38b_95ba_72a8);
}

#[test]
fn cff_outlines() {
    let mut f = Fnv::new();
    for rel in CFF_FONTS {
        let map = tables(rel);
        let Some(cff) = map.get("CFF ") else { continue };
        let Ok(o) = crate::daecore::daetype::outline::CffOutlines::parse(cff) else { continue };
        for gid in 0..n_glyphs(&map) {
            let _ = crate::daecore::daetype::outline::outline_cff_glyph_with(&o, cff, gid, &mut f);
        }
    }
    check("cff outlines", f.done(), 0x9dd2_39fd_886a_05cf);
}

#[test]
fn autohinted_outlines() {
    use crate::daecore::daetype::hinting::auto::{AutoHinter, CollectPen};
    let mut f = Fnv::new();
    for rel in GLYF_FONTS.iter().chain(CFF_FONTS.iter()) {
        let map = tables(rel);
        let n = n_glyphs(&map);
        let upm = crate::daecore::daetype::decoder::read_u16_be(map.get("head").unwrap(), 18).unwrap_or(1000);
        let cmap = map.get("cmap").cloned().unwrap_or_default();
        let loca = loca_of(&map);
        let cff = map.get("CFF ").cloned();
        let outlines = cff.as_ref().and_then(|c| crate::daecore::daetype::outline::CffOutlines::parse(c).ok());
        let collect = |gid: u32| {
            let mut pen = CollectPen::new();
            match (&outlines, &cff, &loca) {
                (Some(o), Some(c), _) => {
                    crate::daecore::daetype::outline::outline_cff_glyph_with(o, c, gid, &mut pen).ok()?;
                }
                (_, _, Some(l)) => {
                    crate::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, l, gid, &mut pen).ok()?;
                }
                _ => return None,
            }
            let p = pen.finish();
            (!p.is_empty()).then_some(p)
        };
        let lookup = crate::daecore::daetype::subsetter::CmapLookup::new(&cmap);
        let gid_of = |c: char| lookup.glyph_id(&cmap, c as u32);
        let Some(mut h) = AutoHinter::new(upm, &mut |c| gid_of(c), &mut |g| collect(g)) else { continue };
        for ppem in [8u16, 11, 16, 24, 40] {
            for gid in 0..n {
                let Some(p) = collect(gid) else { continue };
                let o = h.hint(&p, ppem.into());
                for v in o.y.iter().chain(o.x.iter()) { f.i64(i64::from(*v)); }
                for b in &o.flags { f.u64(u64::from(*b)); }
                for c in &o.contour_ends { f.u64(*c as u64); }
            }
        }
    }
    check("autohinted outlines", f.done(), 0x79d1_8451_f6bd_6432);
}

#[test]
fn bytecode_hinted_outlines() {
    use crate::daecore::daetype::hinting::HintContext;
    let map = tables("test-fixtures/hinted.ttf");
    let upm = crate::daecore::daetype::decoder::read_u16_be(map.get("head").unwrap(), 18).unwrap();
    let loca = loca_of(&map).expect("loca");
    let glyf = map.get("glyf").expect("glyf").clone();
    let n = n_glyphs(&map);
    let mut f = Fnv::new();
    for ppem in [8u16, 11, 16, 24, 40] {
        let Some(mut ctx) = HintContext::new(&map, ppem.into(), upm) else { continue };
        for gid in 0..n {
            if let Some(o) = ctx.hint_glyph(&glyf, &loca, gid) {
                crate::daecore::daetype::hinting::draw_hinted(o, &mut f);
            }
        }
    }
    check("bytecode hinted outlines", f.done(), 0xe3ae_51d9_7db5_92f1);
}

#[test]
fn shaped_runs() {
    use crate::daecore::daekit::cache::FontCache;
    use crate::daecore::daeshaper::{face::Face, pipeline, run::Run};
    const RUNS: &[(&str, &str)] = &[
        ("inter/InterVariable.ttf", "The quick brown fox jumps over the lazy dog"),
        ("inter/InterVariable.ttf", "office affluent difficult"),
        ("eb-garamond/EBGaramond.ttf", "Typography, and the letters it sets."),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "العربية المتصلة"),
        ("noto-devanagari/NotoSansDevanagari.ttf", "हिन्दी लिपि"),
        ("noto-khmer/NotoSansKhmer.ttf", "ភាសាខ្មែរ"),
        ("noto-myanmar/NotoSansMyanmar.ttf", "မြန်မာဘာသာ"),
    ];
    let mut f = Fnv::new();
    for (rel, text) in RUNS {
        let fc = FontCache::new(tables(rel)).expect("the fixture loads");
        let face = Face::new(&fc);
        let mut probe = Run::default();
        probe.push_str(text);
        let direction = pipeline::settle_script(&mut probe);
        let tags = probe.script.map(crate::daecore::daeshaper::ot::tag::script_tags);
        let tags = tags.as_ref().map_or(&[][..], |t| t.as_slice());
        let plan = crate::tests::plain_plan(&face, probe.script, direction, tags);
        let mut buffer = Run::default();
        buffer.push_str(text);
        let dir = pipeline::settle_script(&mut buffer);
        pipeline::shape(&face, &plan, &mut buffer, dir, &[]);
        let glyphs = crate::tests::placed(&buffer);
        assert!(!glyphs.is_empty(), "{rel}: shaped nothing");
        for g in &glyphs {
            f.u64(u64::from(g.0.id));
            f.i64(i64::from(g.1.advance_x));
            f.i64(i64::from(g.1.advance_y));
            f.i64(i64::from(g.1.offset_x));
            f.i64(i64::from(g.1.offset_y));
            f.u64(u64::from(g.0.cluster));
        }
    }
    check("shaped runs", f.done(), 0xd2b9_e642_609c_d656);
}

#[test]
fn instanced_fonts() {
    const AXES: &[(&str, &[(&str, f64)])] = &[
        ("inter/InterVariable.ttf", &[("wght", 400.0)]),
        ("inter/InterVariable.ttf", &[("wght", 700.0)]),
        ("eb-garamond/EBGaramond.ttf", &[("wght", 600.0)]),
        ("source-serif/SourceSerif4Variable-Roman.otf", &[("wght", 500.0)]),
        (
            "colr-v1-test-glyphs/test_glyphs_variable.ttf",
            &[("ROTA", 270.0), ("SCSX", 1.5), ("GRR0", -500.0), ("COL1", 1.0), ("CLXI", 100.0)],
        ),
    ];
    let mut f = Fnv::new();
    for (rel, axes) in AXES {
        let map = tables(rel);
        use crate::daecore::daetype::instancer::AxisTag;
        let owned: Vec<(AxisTag, f64)> = axes.iter().map(|(t, v)| (t.parse::<AxisTag>().expect("a tag"), *v)).collect();
        match crate::daecore::daetype::instancer::instance_font_from_map(&map, &Reads::of_tables(&map), &owned) {
            Ok(out) => { f.u64(out.len() as u64); f.bytes(&out); }
            Err(e) => f.bytes(e.message().as_bytes()),
        }
    }
    check("instanced fonts", f.done(), 0xc01e_ed2a_5409_fa45);
}

#[test]
fn subset_fonts() {
    let mut f = Fnv::new();
    for rel in GLYF_FONTS.iter().take(4) {
        let raw = bytes_of(rel);
        let n = n_glyphs(&tables(rel));
        let req: Vec<u32> = (0..n).step_by(7).take(300).collect();
        match crate::daecore::daetype::subsetter::subset_ttf(&raw, &req) {
            Ok(r) => { f.u64(r.font.len() as u64); f.bytes(&r.font); f.u64(r.gid_map.len() as u64); }
            Err(e) => f.bytes(e.message().as_bytes()),
        }
    }
    for rel in CFF_FONTS {
        let map = tables(rel);
        let Some(cff) = map.get("CFF ") else { continue };
        let req: Vec<u32> = (0..n_glyphs(&map)).step_by(5).take(200).collect();
        match crate::daecore::daetype::subsetter::subset_cff_compacting(cff, &req) {
            Ok(r) => { f.u64(r.font.len() as u64); f.bytes(&r.font); }
            Err(e) => f.bytes(e.message().as_bytes()),
        }
    }
    check("subset fonts", f.done(), 0x5e6a_0f2a_be60_73a9);
}

#[test]
fn color_glyphs() {
    const COLR: &[&str] = &[
        "colr-v1-test-glyphs/test_glyphs.ttf",
        "colr-v1-test-glyphs/test_glyphs_variable.ttf",
        "bungee-tint/BungeeTint-Regular.ttf",
    ];
    let mut f = Fnv::new();
    for rel in COLR {
        let map = tables(rel);
        if !map.contains_key("COLR") {
            continue;
        }
        let n = n_glyphs(&map);
        let palettes = crate::daecore::daetype::colr_v0::cpal_palette_count(&map).clamp(1, 2);
        for gid in 0..n {
            for pal in 0..palettes {
                if let Some(layers) = crate::daecore::daetype::colr_v0::colr_layers_for_palette(&map, gid, pal) {
                    for l in layers {
                        let c = l.color;
                        for v in [l.glyph.0.into(), c.r.into(), c.g.into(), c.b.into(), c.a.into()] {
                            f.u64(v);
                        }
                        f.u64(u64::from(l.foreground));
                    }
                }
            }
        }
        // Drawn at the first axes' normalized values, each set in design units on its side of the default.
        let cache = crate::daecore::daekit::cache::FontCache::new(map.clone()).expect("the font loads");
        let axes = crate::daecore::daetype::decoder::parse_fvar_axes(&map).unwrap_or_default();
        for loc in [&[][..], &[0.5, -0.25, 1.0][..], &[1.0, 1.0, 1.0][..]] {
            let side = |a: &crate::daecore::daetype::decoder::Axis, n: f64| {
                a.default + n * if n < 0.0 { a.default - a.min } else { a.max - a.default }
            };
            let design: Vec<(Tag, f64)> = axes.iter().zip(loc).map(|(a, &n)| (a.tag, side(a, n))).collect();
            let key = cache.axis_key(&design);
            for gid in 0..n {
                for pal in 0..palettes {
                    f.bytes(format!("{:?}", cache.colr_v1_paint(gid, &key, pal)).as_bytes());
                }
            }
        }
    }
    check("color glyphs", f.done(), 0x0f89_2bbf_18c1_a5bf);
}

#[test]
fn metadata_and_cmap() {
    let mut f = Fnv::new();
    for rel in GLYF_FONTS.iter().chain(CFF_FONTS.iter()) {
        let map = tables(rel);
        if let Some(cmap) = map.get("cmap") {
            let lookup = crate::daecore::daetype::subsetter::CmapLookup::new(cmap);
            for cp in (0x20u32..0x2500).step_by(3) {
                f.u64(u64::from(lookup.glyph_id(cmap, cp).unwrap_or(0)));
            }
        }
        for (id, s) in crate::daecore::daetype::decoder::parse_all_name_strings::<u16>(&map) {
            f.u64(u64::from(id));
            f.bytes(s.as_bytes());
        }
        if let Some(o) = crate::daecore::daetype::decoder::parse_os2(&map) {
            f.u64(u64::from(o.version));
            f.u64(u64::from(o.fs_selection.unwrap_or(0)));
        }
        if let Ok(axes) = crate::daecore::daetype::decoder::parse_fvar_axes(&map) {
            for a in axes {
                f.bytes(&a.tag.to_bytes());
                for v in [a.min, a.default, a.max] { f.i64(v.to_bits() as i64); }
            }
        }
        let (post, cff) = (map.get("post").map(|v| v.as_slice()), map.get("CFF ").map(|v| v.as_slice()));
        let index = crate::daecore::daetype::glyph_names::NameIndex::new(post, cff);
        let names = crate::daecore::daetype::glyph_names::glyph_names(&index, post, cff, n_glyphs(&map).min(1500));
        for (gid, name) in names.into_iter().enumerate() {
            f.u64(gid as u64);
            if let Some(n) = name { f.bytes(n.as_bytes()); }
        }
    }
    check("metadata and cmap", f.done(), 0x4f29_c0ac_fd65_0ea6);
}
