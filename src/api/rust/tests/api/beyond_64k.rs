use std::collections::BTreeMap;

use daegun::color::PaintNode;
use daegun::outline::OutlinePen;
use daegun::shape::{Direction, ShapeOptions};
use daegun::{Font, GlyphClass, GlyphId, Location, Orientation, Tag};

use crate::located;

type Tables = BTreeMap<Tag, Vec<u8>>;

fn face(tables: &[(&str, &[u8])]) -> Font {
    let t: Tables = tables.iter().map(|&(tag, data)| (tag.parse().expect("a tag"), data.to_vec())).collect();
    Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the face parses")
}

fn tables_of(f: &Font) -> Tables {
    f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect()
}

fn inter() -> Tables {
    let f = Font::from_vec(std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("Inter"), 0);
    tables_of(&f.expect("it parses"))
}

// head at 1,000 units with long loca.
fn head() -> Vec<u8> {
    let mut h = vec![0; 54];
    h[..4].copy_from_slice(&[0, 1, 0, 0]);
    h[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    h[18..20].copy_from_slice(&1000u16.to_be_bytes());
    h[51] = 1;
    h
}

fn maxp(n: u16) -> Vec<u8> {
    [&[0, 0, 0x50, 0][..], &n.to_be_bytes()].concat()
}

// LOCA for `n` empty glyphs, beside a GLYF of a byte: MAXP counts only as far as glyph data holds glyphs.
fn empty_loca(n: usize) -> Vec<u8> {
    vec![0; 4 * (n + 1)]
}

// A simple glyph whose three on-curve points are the corners of its box.
fn triangle(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<u8> {
    let words = |w: &[i16]| w.iter().flat_map(|w| w.to_be_bytes()).collect::<Vec<u8>>();
    [words(&[1, x0, y0, x1, y1, 2, 0]), vec![1; 3], words(&[x0, x1 - x0, 0, y0, 0, y1 - y0])].concat()
}

// A subset's tables but the cmap and name daegun's subsets always carry, as display fonts.
fn subset_tables(f: &Font, gids: &[u32]) -> Tables {
    let gids: Vec<GlyphId> = gids.iter().copied().map(GlyphId).collect();
    let mut t = tables_of(&Font::from_vec(f.subset(&gids, &Location::default()).expect("a subset").font, 0)
        .expect("it parses"));
    assert!(t.remove(&Tag::new(b"cmap")).is_some() && t.remove(&Tag::new(b"name")).is_some());
    t
}

fn width(f: &Font, gid: u32) -> f64 {
    let b = f.glyph_bounds(GlyphId(gid), &Location::default()).expect("a box");
    b.x_max - b.x_min
}

// A path as one string of commands, so a test compares a whole outline at once.
#[derive(Default)]
struct Drawn(String);

impl OutlinePen for Drawn {
    fn move_to(&mut self, x: f32, y: f32) { self.0 += &format!("M{x},{y};"); }
    fn line_to(&mut self, x: f32, y: f32) { self.0 += &format!("L{x},{y};"); }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) { self.0 += &format!("Q{cx},{cy},{x},{y};"); }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.0 += &format!("C{ax},{ay},{bx},{by},{x},{y};");
    }
    fn close(&mut self) { self.0 += "Z;"; }
}

fn drawn(f: &Font, gid: u32) -> String {
    let mut pen = Drawn::default();
    let _ = f.outline(GlyphId(gid), &Location::default(), &mut pen);
    pen.0
}

// A cubic glyph's points: a point on the curve, then two cubic controls; in glyf, two quadratic ones.
const CUBIC_POINTS: &[u8] = b"\0\x02\0\0\x31\xB6\x96\x0A\x0A\x0A\x0A";

// maxp counts the glyphs unless a MAXP beside it reads whole and its glyphs are held. Each face has a head, which a
// font needs to open.
#[test]
fn maxp_counts_the_glyphs_unless_maxp_s_uppercase_twin_reads_and_is_held() {
    let (upper, maxp, head) = (b"\0\0\x50\0\x01\0\x01", maxp(5), head());
    let (held, short) = (empty_loca(0x10001), empty_loca(0x10000));
    let count = |t: &[(&str, &[u8])]| face(&[&[("head", &head[..]), ("maxp", &maxp), ("GLYF", &[0])][..], t].concat());
    assert_eq!(count(&[]).glyph_count(), 5);
    assert_eq!(count(&[("MAXP", upper), ("LOCA", &held)]).glyph_count(), 0x10001);
    assert_eq!(count(&[("MAXP", &upper[..6]), ("LOCA", &held)]).glyph_count(), 5, "a short MAXP");
    assert_eq!(count(&[("MAXP", upper), ("LOCA", &short)]).glyph_count(), 5, "a glyph not held");
    assert_eq!(count(&[("MAXP", upper)]).glyph_count(), 5, "no glyph held");
}

// GLYF is read only beside LOCA, and only GLYF's controls are cubic. Glyph 0 is a different triangle in each table,
// as daegun measures a box from the outline rather than the stored one; glyph 1 is the same in both.
#[test]
fn glyf_is_read_from_its_uppercase_twin_only_beside_loca_s() {
    let loca = [0u32, 29, 50].map(u32::to_be_bytes).concat();
    let cubic = [b"\0\x01\0\0\0\0\0\x14\0\x0A", CUBIC_POINTS].concat();
    let glyf_face = |upper_glyf: bool, upper_loca: bool| {
        let glyf = [triangle(1, 2, 4, 6), cubic.clone()].concat();
        let glyf_upper = [triangle(11, 12, 24, 26), cubic.clone()].concat();
        let (maxp, head) = (maxp(2), head());
        let mut t: Vec<(&str, &[u8])> = vec![("maxp", &maxp), ("loca", &loca), ("glyf", &glyf), ("head", &head)];
        if upper_glyf {
            t.push(("GLYF", &glyf_upper));
        }
        if upper_loca {
            t.push(("LOCA", &loca));
        }
        face(&t)
    };
    let cubics = |f: &Font| drawn(f, 1).matches('C').count();
    let (both, without_loca) = (glyf_face(true, true), glyf_face(true, false));
    assert_eq!((both.glyph_count(), width(&both, 0), cubics(&both)), (2, 13.0, 1));
    assert_eq!((width(&without_loca, 0), cubics(&without_loca)), (3.0, 0));
    assert_eq!(width(&glyf_face(false, true), 0), 3.0);
}

// From the uppercase tables alone: a contour of cubic controls alone starts midway between its last and first and keeps
// their pairing, and one of two controls closes at their midpoint.
#[test]
fn cubic_contours_keep_their_controls_paired() {
    let all_off = b"\0\x01\0\0\0\0\0\x64\0\x64\0\x03\0\0\xB0\xB2\xB4\xA2\x64\x64\x64".as_slice();
    let two_contours = b"\0\x02\0\0\0\0\x01\x2C\0\x64\0\x03\0\x07\0\0\xB0\xB2\xB4\xA2\x96\xB2\xB4\xA2\
        \x64\x64\xC8\x64\x64\x64\x64\x64";
    let two_controls = b"\0\x01\0\0\0\0\0\x64\0\x64\0\x01\0\0\xB0\xB6\x64\x64".as_slice();
    let circle = "M0,50;C0,0,100,0,100,50;C100,100,0,100,0,50;Z;";
    let cases = [
        (all_off, 4u16, 1u16, 100u16, circle.to_string()),
        (two_contours, 8, 2, 300, format!("{circle}M200,50;C200,0,300,0,300,50;C300,100,200,100,200,50;Z;")),
        (two_controls, 2, 1, 100, "M50,50;C0,0,100,100,50,50;Z;".to_string()),
    ];
    for (glyph, points, contours, width, want) in cases {
        let (mut maxp, mut head, mut hhea) = (vec![0; 33], vec![0; 54], vec![0; 38]);
        maxp[..4].copy_from_slice(&[0, 1, 0, 0]);
        maxp[6] = 1;
        maxp[7..9].copy_from_slice(&points.to_be_bytes());
        maxp[9..11].copy_from_slice(&contours.to_be_bytes());
        maxp[16] = 2;
        head[..4].copy_from_slice(&[0, 1, 0, 0]);
        head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[40..42].copy_from_slice(&width.to_be_bytes());
        head[43] = 100;
        head[51] = 1;
        hhea[..4].copy_from_slice(&[0, 1, 0, 0]);
        for (at, v) in [(4, 800), (6, -200), (10, 600), (14, 600 - width as i16), (16, width as i16), (18, 1)] {
            hhea[at..at + 2].copy_from_slice(&i16::to_be_bytes(v));
        }
        hhea[37] = 1;
        let loca = [&[0; 4][..], &(glyph.len() as u32).to_be_bytes()].concat();
        let hmtx = [2, 0x58, 0, 0];
        let tables: [(&str, &[u8]); 6] =
            [("MAXP", &maxp), ("head", &head), ("HHEA", &hhea), ("HMTX", &hmtx), ("LOCA", &loca), ("GLYF", glyph)];
        let f = face(&tables);
        assert_eq!(f.glyph_count(), 1);
        assert_eq!(drawn(&f, 0), want, "{contours} contours of {points} points");
    }
}

// A component's id is 24-bit only in GLYF; glyf reads the same record with a 16-bit id, which names the composite
// itself, and draws nothing.
#[test]
fn a_24_bit_component_is_drawn_only_from_glyf_s_uppercase_twin() {
    let loca = [0, 0, 0, 0, 0, 0, 0, 0x11, 0, 0, 0, 0x26];
    let glyf = [
        b"\xFF\xFF\0\0\0\0\0\x14\0\x14\x20\x02\0\0\x01\0\0".as_slice(),
        b"\0\x01\0\0\0\0\0\x14\0\x14",
        CUBIC_POINTS,
    ]
    .concat();
    let (maxp, head) = (maxp(2), head());
    let lower: Vec<(&str, &[u8])> = vec![("maxp", &maxp), ("head", &head), ("glyf", &glyf), ("loca", &loca)];
    let upper = [lower.clone(), vec![("GLYF", &glyf), ("LOCA", &loca)]].concat();
    assert_eq!(drawn(&face(&upper), 0), "M0,0;C10,10,20,0,0,0;Z;");
    assert_eq!(drawn(&face(&lower), 0), "");
}

// The cubic glyph placed by its offset, then again by matching its point 0 to the composite's point 1.
#[test]
fn a_component_placed_by_its_points_keeps_its_cubic_controls() {
    let composite = b"\xFF\xFF\0\0\0\0\0\x1E\0\x14\x20\x22\0\0\x01\0\0\x20\0\0\0\x01\x01\0".as_slice();
    let glyf = [composite, b"\0\x01\0\0\0\0\0\x14\0\x0A", CUBIC_POINTS].concat();
    let loca = [0u32, 24, 45].map(u32::to_be_bytes).concat();
    let (maxp, head) = (maxp(2), head());
    let f = face(&[("maxp", &maxp), ("head", &head), ("GLYF", &glyf), ("LOCA", &loca)]);
    assert_eq!(drawn(&f, 0), "M0,0;C10,10,20,0,0,0;Z;M10,10;C20,20,30,10,10,10;Z;");
}

// Beside GLYF and LOCA, glyf and loca are left out of a subset without being asked, and the subset writes short
// offsets where they fit.
#[test]
fn a_subset_writes_glyf_s_uppercase_pair_alone_where_both_pairs_read() {
    let loca = [0, 0, 0, 0, 0, 0, 0, 1];
    let (maxp, head) = (maxp(1), head());
    let f = face(&[("maxp", &maxp), ("head", &head), ("glyf", b"\0\0"), ("loca", &loca), ("GLYF", b"\0\0"),
        ("LOCA", &loca)]);
    let sub = subset_tables(&f, &[0]);
    assert!(sub.get(&Tag::new(b"GLYF")).is_some_and(|t| !t.is_empty()));
    assert!(sub.get(&Tag::new(b"LOCA")).is_some_and(|t| !t.is_empty()));
    assert!(!sub.contains_key(&Tag::new(b"glyf")) && !sub.contains_key(&Tag::new(b"loca")));
    assert_eq!(sub[&Tag::new(b"head")][50..52], [0, 0]);
}

// A quadratic glyph past 65,535 is measured, and its subset writes GLYF, LOCA, MAXP and head alone.
#[test]
fn a_glyph_past_65_535_is_measured_and_subset() {
    let glyph = triangle(1, 2, 4, 6);
    let n = 0x10002;
    let mut loca = vec![0; (n + 1) * 4];
    loca[n * 4..].copy_from_slice(&(glyph.len() as u32).to_be_bytes());
    let head = head();
    let f = face(&[("MAXP", b"\0\0\x50\0\x01\0\x02"), ("head", &head), ("GLYF", &glyph), ("LOCA", &loca)]);
    assert_eq!((f.glyph_count(), width(&f, 0x10001)), (n as u32, 3.0));

    let sub = Font::from_vec(f.subset([GlyphId(0x10001)], &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert_eq!((sub.glyph_count(), width(&sub, 1)), (2, 3.0));
    let sub = subset_tables(&f, &[0x10001]);
    assert_eq!(sub.keys().map(Tag::to_string).collect::<Vec<_>>(), ["GLYF", "LOCA", "MAXP", "head"]);
    assert_eq!(sub[&Tag::new(b"GLYF")], [glyph, vec![0]].concat());
    assert_eq!(sub[&Tag::new(b"LOCA")], [0, 0, 0, 0, 0, 15]);
    assert_eq!(sub[&Tag::new(b"MAXP")], b"\0\0\x50\0\0\0\x02");
    assert_eq!(sub[&Tag::new(b"head")][50..52], [0, 0]);
    let empty = subset_tables(&f, &[5]);
    assert_eq!(
        (&empty[&Tag::new(b"GLYF")][..], &empty[&Tag::new(b"LOCA")][..]),
        (&[0][..], &[0; 6][..]),
        "one byte where no glyph has any"
    );
}

// Each of the four metric pairs, lowercase and uppercase, is subset from its own source; the subset advances by HMTX.
#[test]
fn a_hybrid_s_metric_pairs_are_each_subset_from_their_own_source() {
    let header = |len: usize| {
        let mut h = vec![0; len];
        (h[1], h[len - 1]) = (1, 3);
        h
    };
    let (lower, upper, head) = (header(36), header(38), head());
    let f = face(&[
        ("head", &head), ("maxp", &maxp(3)), ("MAXP", b"\0\0\x50\0\0\0\x03"),
        ("hhea", &lower), ("HHEA", &upper), ("vhea", &lower), ("VHEA", &upper),
        ("hmtx", &[0, 10, 0, 1, 0, 20, 0, 2, 0, 30, 0, 3]), ("HMTX", &[0, 100, 0, 4, 0, 200, 0, 5, 1, 44, 0, 6]),
        ("vmtx", &[0, 40, 0, 7, 0, 50, 0, 8, 0, 60, 0, 9]), ("VMTX", &[1, 144, 0, 10, 1, 244, 0, 11, 2, 88, 0, 12]),
    ]);
    let sub = Font::from_vec(f.subset([GlyphId(0), GlyphId(2)], &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert_eq!(crate::advances(&sub, &[GlyphId(0), GlyphId(1)], &Location::default()), [100.0, 300.0]);

    let (mut lower, mut upper) = (lower, upper);
    (lower[35], upper[37]) = (2, 2);
    let want: Tables = [
        ("maxp", maxp(2)), ("MAXP", b"\0\0\x50\0\0\0\x02".to_vec()),
        ("hhea", lower.clone()), ("HHEA", upper.clone()), ("vhea", lower), ("VHEA", upper),
        ("head", head), ("hmtx", vec![0, 10, 0, 1, 0, 30, 0, 3]), ("HMTX", vec![0, 100, 0, 4, 1, 44, 0, 6]),
        ("vmtx", vec![0, 40, 0, 7, 0, 60, 0, 9]), ("VMTX", vec![1, 144, 0, 10, 2, 88, 0, 12]),
    ]
    .into_iter()
    .map(|(tag, data)| (tag.parse().expect("a tag"), data))
    .collect();
    let mut got = subset_tables(&f, &[0, 2]);
    got.get_mut(&Tag::new(b"head")).expect("head")[8..12].fill(0);
    assert_eq!(got, want);
}

// Inter without HVAR: an instance's HMTX advances come from the instanced outlines' phantom points, as hmtx's do.
#[test]
fn an_instance_measures_hmtx_s_uppercase_twin_from_its_outlines() {
    let mut lower = inter();
    lower.remove(&Tag::new(b"HVAR"));
    let mut upper = lower.clone();
    let hhea = upper.remove(&Tag::new(b"hhea")).expect("hhea");
    upper.insert(Tag::new(b"HHEA"), [&hhea[..34], &[0, 0], &hhea[34..]].concat());
    let hmtx = upper.remove(&Tag::new(b"hmtx")).expect("hmtx");
    upper.insert(Tag::new(b"HMTX"), hmtx);

    let instanced = |t: &Tables| {
        let f = Font::from_vec(daegun::raw::build_font(t).unwrap(), 0).expect("the font parses");
        Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("an instance")).expect("it parses")
    };
    let (want, got) = (instanced(&lower), instanced(&upper));
    assert!(got.has_table(Tag::new(b"HMTX")) && !got.has_table(Tag::new(b"hmtx")));
    let gids: Vec<GlyphId> = (0..want.glyph_count()).map(GlyphId).collect();
    let advances = crate::advances(&want, &gids, &Location::default());
    let lower = Font::from_vec(daegun::raw::build_font(&lower).unwrap(), 0).expect("parses");
    assert_ne!(advances, crate::advances(&lower, &gids, &Location::default()));
    assert_eq!(crate::advances(&got, &gids, &Location::default()), advances);
}

// A pinned font keeps no GVAR. Only a variable subset would keep one, and daegun doesn't write those.
#[test]
fn a_pinned_font_keeps_no_gvar_uppercase_twin() {
    let mut t = inter();
    t.insert(Tag::new(b"GVAR"), vec![0; 2]);
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the font parses");
    let instance = Font::from_bytes(&f.instance(&Location::default()).expect("an instance")).expect("it parses");
    let subset = Font::from_vec(f.subset([GlyphId(0), GlyphId(1)], &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert!(!instance.has_table(Tag::new(b"GVAR")) && !subset.has_table(Tag::new(b"GVAR")));
}

// An instance's maxp counts its glyphs by MAXP's count where MAXP reads, capped at 65,535 to fit maxp's 16 bits.
// Inter's long loca gains empty glyphs up to the count, which MAXP counts only where they're held.
#[test]
fn an_instance_s_maxp_counts_its_glyphs_as_maxp_s_uppercase_twin_does() {
    for (count, want) in [(3_000u32, 3_000u16), (0x1_0001, 0xFFFF)] {
        let mut t = inter();
        t.insert(Tag::new(b"MAXP"), [&[0, 0, 0x50, 0][..], &count.to_be_bytes()[1..]].concat());
        let loca = t.get_mut(&Tag::new(b"loca")).expect("loca");
        let end = loca[loca.len() - 4..].to_vec();
        loca.extend(end.repeat(count as usize + 1 - loca.len() / 4));
        let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the font parses");
        let instance =
            Font::from_bytes(&f.instance(&located(&f, &[("wght", 700.0)])).expect("an instance")).expect("it parses");
        assert_eq!(instance.table(Tag::new(b"maxp")).expect("maxp")[4..6], want.to_be_bytes());
        assert_eq!(instance.glyph_count(), count);
    }
}

// A one-axis font of `glyphs` whose gvar moves no point, in GLYF, LOCA and GVAR where `upper`, each glyph's bearing
// as `lsb` gives it, instanced at wght 900. Each of `twins` stands beside them unread, a copy of glyf or of loca.
fn instanced(glyphs: &[Vec<u8>], lsb: impl Fn(usize) -> i16, upper: bool, twins: &[&str]) -> Font {
    let n = glyphs.len();
    let mut loca = vec![0u32];
    glyphs.iter().for_each(|g| loca.push(loca[loca.len() - 1] + g.len() as u32));
    let axis = [&b"wght"[..], &[0, 100, 0, 0, 1, 0x90, 0, 0, 3, 0x84, 0, 0, 0, 0, 1, 0]].concat();
    let fvar = [&[0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8][..], &axis].concat();
    let (count, data_at) = (n as u32, 20 + u32::from(upper) + 4 * (n as u32 + 1));
    let count = if upper { &count.to_be_bytes()[1..] } else { &count.to_be_bytes()[2..] };
    let gvar = [&[0, 1, 0, 0, 0, 1, 0, 0][..], &data_at.to_be_bytes(), count, &[0, 1], &data_at.to_be_bytes()].concat();
    let gvar = [gvar, vec![0; 4 * (n + 1)]].concat();
    let hmtx: Vec<u8> = (0..n).flat_map(|g| [600u16.to_be_bytes(), lsb(g).to_be_bytes()].concat()).collect();
    let (maxp, head, mut hhea) = (maxp(n as u16), head(), vec![0; 36]);
    hhea[..4].copy_from_slice(&[0, 1, 0, 0]);
    hhea[34..].copy_from_slice(&(n as u16).to_be_bytes());
    let tags = if upper { ["GLYF", "LOCA", "GVAR"] } else { ["glyf", "loca", "gvar"] };
    let (glyf, loca) = (glyphs.concat(), loca.into_iter().flat_map(u32::to_be_bytes).collect::<Vec<u8>>());
    let mut t: Vec<(&str, &[u8])> = vec![("maxp", &maxp), ("head", &head), ("hhea", &hhea), ("hmtx", &hmtx),
        ("fvar", &fvar), (tags[0], &glyf), (tags[1], &loca), (tags[2], &gvar)];
    t.extend(twins.iter().map(|&twin| (twin, if twin.eq_ignore_ascii_case("glyf") { &glyf[..] } else { &loca[..] })));
    let f = face(&t);
    Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("an instance")).expect("it parses")
}

// Off the default every simple glyph is rewritten: the cubic bit stays in GLYF and leaves glyf, which ignores it,
// whether gvar moves no point (glyph 0) or the bearing shifts it (glyph 1). The twin it doesn't read is left out.
#[test]
fn an_instance_keeps_the_cubic_bit_only_in_glyf_s_uppercase_twin() {
    let glyph = [b"\0\x01\0\0\0\0\0\x14\0\x0A".as_slice(), CUBIC_POINTS].concat();
    let cases = [
        (false, "glyf", &["GLYF"][..], [0x31, 0x36, 0x16, 0x33, 0x36, 0x16]),
        (false, "glyf", &["LOCA"][..], [0x31, 0x36, 0x16, 0x33, 0x36, 0x16]),
        (true, "GLYF", &["glyf", "loca"][..], [0x31, 0xB6, 0x96, 0x33, 0xB6, 0x96]),
    ];
    for (upper, tag, twins, want) in cases {
        let instance = instanced(&[glyph.clone(), glyph.clone()], |g| 5 * g as i16, upper, twins);
        let written = instance.table(tag.parse().expect("a tag")).expect("the glyphs");
        assert_eq!([&written[14..17], &written[22 + 14..22 + 17]].concat(), want, "{tag}");
        for twin in twins {
            assert!(instance.table(twin.parse().expect("a tag")).is_none(), "{twin}");
        }
    }
}

// Each glyph pads to 2 bytes where short offsets, which store half the offset, reach the last, as they do to 131,070
// bytes; past them none pads: 5,957 glyphs of 21 bytes take 131,054 padded and 5,958 take 131,076.
#[test]
fn an_instance_pads_its_glyphs_only_under_short_offsets() {
    let glyph = [b"\0\x01\0\0\0\0\0\x14\0\x0A".as_slice(), CUBIC_POINTS].concat();
    for (n, short) in [(5_957, true), (5_958, false)] {
        let instance = instanced(&vec![glyph.clone(); n], |_| 0, false, &[]);
        let loca = instance.table(Tag::new(b"loca")).expect("loca");
        let end = match short {
            true => usize::from(u16::from_be_bytes([loca[2 * n], loca[2 * n + 1]])) * 2,
            false => u32::from_be_bytes(loca[4 * n..4 * n + 4].try_into().unwrap()) as usize,
        };
        let long = instance.table(Tag::new(b"head")).expect("head")[51];
        assert_eq!((long, end), (u8::from(!short), n * if short { 22 } else { 21 }));
    }
}

// DMAP answers before cmap, a default taking the nominal glyph; a text subset keeps what it alone answers (4 zero
// bytes if nothing) and drops one the sanitizer refuses, keeping the rest of the font.
#[test]
fn dmap_answers_before_cmap_and_is_subset() {
    let cmap: &[u8] = &[
        0, 0, 0, 2, 0, 3, 0, 10, 0, 0, 0, 20, 0, 0, 0, 5, 0, 0, 0, 48,
        0, 12, 0, 0, 0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'A', 0, 0, 0, b'B', 0, 0, 0, 1,
        0, 14, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0xFE, 0x0F, 0, 0, 0, 0, 0, 0, 0, 21,
        0, 0, 0, 3, 0, 0, b'A', 0, 3, 0, 0, b'B', 0, 3, 0, 0, b'C', 0, 3,
    ];
    let dmap: &[u8] = &[
        0, 0, 0, 2, 0, 3, 0, 10, 0, 0, 0, 20, 0, 0, 0, 5, 0, 0, 0, 60,
        0, 12, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, b'A', 0, 0, 0, b'A', 0, 0, 0, 4, 0, 0, 0, b'C', 0, 0, 0, b'C', 0, 0, 0, 5,
        0, 14, 0, 0, 0, 38, 0, 0, 0, 1, 0, 0xFE, 0x0F, 0, 0, 0, 21, 0, 0, 0, 29,
        0, 0, 0, 1, 0, 0, b'B', 0, 0, 0, 0, 1, 0, 0, b'A', 0, 6,
    ];
    let head = head();
    let f = face(&[("maxp", &maxp(7)), ("head", &head), ("cmap", cmap), ("DMAP", dmap)]);
    let abc = |f: &Font, at: &dyn Fn(&Font, char) -> Option<GlyphId>| ['A', 'B', 'C'].map(|c| at(f, c).map(|g| g.0));
    let nominal = |f: &Font, c| f.glyph_id(c);
    let variant = |f: &Font, c| f.variation_glyph_id(c, '\u{FE0F}');
    assert_eq!(abc(&f, &nominal), [Some(4), Some(2), Some(5)]);
    assert_eq!(abc(&f, &variant), [Some(6), Some(2), Some(3)]);
    assert_eq!(f.coverage().collect::<Vec<_>>(), [('A', GlyphId(4)), ('B', GlyphId(2)), ('C', GlyphId(5))]);

    let sub = Font::from_vec(f.subset_text("ABC\u{FE0F}", &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert!(sub.table(Tag::new(b"DMAP")).is_some_and(|t| !t.is_empty()));
    assert_eq!(abc(&sub, &nominal), [Some(3), Some(1), Some(4)]);
    assert_eq!(abc(&sub, &variant), [Some(5), Some(1), Some(2)]);
    let own = face(&[("maxp", &maxp(7)), ("head", &head), ("cmap", sub.table(Tag::new(b"DMAP")).expect("kept"))]);
    assert_eq!(
        (own.coverage().collect::<Vec<_>>(), abc(&own, &variant)),
        (vec![('A', GlyphId(3)), ('C', GlyphId(4))], [Some(5), None, None])
    );
    let none = Font::from_vec(f.subset_text("B", &Location::default()).expect("a subset").font, 0).expect("it parses");
    assert_eq!(none.table(Tag::new(b"DMAP")), Some(&[0u8; 4][..]));

    let alone = |record: [u8; 4], body: &[u8]| [&[0, 0, 0, 1][..], &record, &[0, 0, 0, 12], body].concat();
    for (dmap, nominals, variants) in [
        (alone([0, 3, 0, 10], &dmap[20..60]), [Some(4), Some(2), Some(5)], [Some(3), Some(3), Some(3)]),
        (alone([0, 0, 0, 5], &dmap[60..]), [Some(1), Some(2), None], [Some(6), Some(2), Some(3)]),
    ] {
        let f = face(&[("maxp", &maxp(7)), ("head", &head), ("cmap", cmap), ("DMAP", &dmap)]);
        assert_eq!((abc(&f, &nominal), abc(&f, &variant)), (nominals, variants));
    }

    let mut refused = dmap.to_vec();
    refused[1] = 1;
    let f = face(&[("maxp", &maxp(7)), ("head", &head), ("cmap", cmap), ("DMAP", &refused)]);
    assert_eq!((abc(&f, &nominal), abc(&f, &variant)), ([Some(1), Some(2), None], [Some(3), Some(3), Some(3)]));
    let sub = Font::from_vec(f.subset_text("ABC\u{FE0F}", &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert_eq!(sub.table(Tag::new(b"DMAP")), None);
    assert_eq!((abc(&sub, &nominal), abc(&sub, &variant)), ([Some(1), Some(2), None], [Some(3), Some(3), Some(3)]));
}

// Format 15's 24-bit variant answers before 14's. A subset writes the answers into one variation subtable, so 14's
// glyph is dropped and the variant is glyph 1.
#[test]
fn format_15_answers_before_format_14_and_is_subset() {
    let cmap: &[u8] = &[
        0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 28, 0, 0, 0, 5, 0, 0, 0, 56, 0, 0, 0, 5, 0, 0, 0, 86,
        0, 12, 0, 0, 0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'A', 0, 0, 0, b'A', 0, 1, 0, 1,
        0, 14, 0, 0, 0, 30, 0, 0, 0, 1, 0, 0xFE, 0x0F, 0, 0, 0, 0, 0, 0, 0, 21, 0, 0, 0, 1, 0, 0, b'A', 0, 2,
        0, 15, 0, 0, 0, 31, 0, 0, 0, 1, 0, 0xFE, 0x0F, 0, 0, 0, 0, 0, 0, 0, 21, 0, 0, 0, 1, 0, 0, b'A', 1, 0, 1,
    ];
    let (head, loca, upper) = (head(), empty_loca(0x10002), b"\0\0\x50\0\x01\0\x02");
    let f = face(&[("MAXP", upper), ("head", &head), ("GLYF", &[0]), ("LOCA", &loca), ("cmap", cmap)]);
    assert_eq!(
        (f.glyph_id('A'), f.variation_glyph_id('A', '\u{FE0F}')),
        (Some(GlyphId(0x1_0001)), Some(GlyphId(0x1_0001)))
    );

    let out = f.subset_text("A\u{FE0F}", &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font, 0).expect("it parses");
    assert_eq!(
        (sub.glyph_count(), sub.glyph_id('A'), sub.variation_glyph_id('A', '\u{FE0F}')),
        (2, Some(GlyphId(1)), Some(GlyphId(1)))
    );
}

// A 16-bit font whose format 12 maps a code past its glyphs: the lookup and shaping answer that glyph, and a
// subset leaves out both the code and the glyph.
#[test]
fn a_code_mapped_past_the_font_answers_and_is_left_out_of_a_subset() {
    let cmap: Vec<u8> = [&[0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12, 0, 12, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 2][..],
        &[0x41, 0x41, 70_000, 0x42, 0x42, 1].map(u32::to_be_bytes).concat()].concat();
    let head = head();
    let f = face(&[("maxp", &maxp(5)), ("head", &head), ("cmap", &cmap)]);
    assert_eq!((f.glyph_id('A'), f.glyph_id('B')), (Some(GlyphId(70_000)), Some(GlyphId(1))));
    assert_eq!(f.shape("AB", &Location::default(), &ShapeOptions::default()).glyphs, [GlyphId(70_000), GlyphId(1)]);
    let sub = Font::from_vec(f.subset_text("AB", &Location::default()).expect("a subset").font, 0).expect("it parses");
    assert_eq!((sub.glyph_count(), sub.glyph_id('A'), sub.glyph_id('B')), (2, None, Some(GlyphId(1))));
}

// Formats 14 and 15: a default range holds at most 256 codes, so a subset of 1 to 768 of three adjacent ranges' codes
// keeps each, the last answering its own glyph.
#[test]
fn default_ranges_hold_at_most_256_codes() {
    let head = head();
    for format in [14, 15] {
        let mut cmap: Vec<u8> = vec![
            0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0, 48, 0, 3, 0, 10, 0, 0, 0, 20,
            0, 12, 0, 0, 0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0x10, 0, 0, 0, 0x12, 0xFF, 0, 0, 0, 1,
            0, 14, 0, 0, 0, 37, 0, 0, 0, 1, 0, 0xFE, 0, 0, 0, 0, 21, 0, 0, 0, 0,
            0, 0, 0, 3, 0, 0x10, 0, 0xFF, 0, 0x11, 0, 0xFF, 0, 0x12, 0, 0xFF,
        ];
        cmap[49] = format;
        let f = face(&[("maxp", &[0, 0, 0x50, 0, 3, 1]), ("head", &head), ("cmap", &cmap)]);
        for count in [1, 255, 256, 257, 512, 513, 768] {
            let text: String = (0x1000..0x1000 + count).chain([0xFE00]).filter_map(char::from_u32).collect();
            let sub = Font::from_vec(f.subset_text(&text, &Location::default()).expect("a subset").font, 0)
                .expect("it parses");
            let variant = |c: u32| sub.variation_glyph_id(char::from_u32(c).expect("a scalar"), '\u{FE00}');
            let kept = (0x1000..0x1300).filter(|&c| variant(c).is_some()).count();
            let last = variant(0x1000 + count - 1);
            assert_eq!((kept, last), (count as usize, Some(GlyphId(count))), "format {format}, {count} codes");
        }
    }
}

// A right-to-left run looks a selector up on its base's mirror, a vertical one on its vertical form, so a text subset
// keeps both: "א(\u{FE0F}" draws ')''s variant [5|1], "（\u{FE0F}" U+FE35's [5].
#[test]
fn a_text_subset_keeps_a_mirrored_or_vertical_base_s_sequence() {
    let group = |c: u32, g: u32| [c, c, g].map(u32::to_be_bytes).concat();
    let f12 = [&[0, 12, 0, 0, 0, 0, 0, 52, 0, 0, 0, 0, 0, 0, 0, 3][..], &group(0x28, 2), &group(0x29, 3)];
    let f12 = [&f12.concat()[..], &group(0x5D0, 1)];
    let f14: &[u8] =
        &[0, 14, 0, 0, 0, 30, 0, 0, 0, 1, 0, 0xFE, 0x0F, 0, 0, 0, 0, 0, 0, 0, 21, 0, 0, 0, 1, 0, 0, 0x29, 0, 5];
    let cmap = [&[0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0, 20, 0, 3, 0, 10, 0, 0, 0, 50][..], f14, &f12.concat()].concat();
    let head = head();
    let f = face(&[("maxp", &maxp(6)), ("head", &head), ("cmap", &cmap)]);
    let text = "\u{5D0}(\u{FE0F}";
    assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs, [GlyphId(5), GlyphId(1)]);
    let out = f.subset_text(text, &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font.clone(), 0).expect("it parses");
    assert_eq!(
        sub.shape(text, &Location::default(), &ShapeOptions::default()).glyphs,
        [out.renumbered(GlyphId(5)).expect("kept"), out.renumbered(GlyphId(1)).expect("kept")]
    );

    let f12 = [&[0, 12, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 2][..], &group(0xFE35, 3), &group(0xFF08, 2)].concat();
    let f14: &[u8] =
        &[0, 14, 0, 0, 0, 30, 0, 0, 0, 1, 0, 0xFE, 0x0F, 0, 0, 0, 0, 0, 0, 0, 21, 0, 0, 0, 1, 0, 0xFE, 0x35, 0, 5];
    let cmap = [&[0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0, 20, 0, 3, 0, 10, 0, 0, 0, 50][..], f14, &f12].concat();
    let f = face(&[("maxp", &maxp(6)), ("head", &head), ("cmap", &cmap)]);
    let text = "\u{FF08}\u{FE0F}";
    assert_eq!(
        f.shape(text, &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom)).glyphs,
        [GlyphId(5)]
    );
    let out = f.subset_text(text, &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font.clone(), 0).expect("it parses");
    assert_eq!(
        sub.shape(text, &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom)).glyphs,
        [out.renumbered(GlyphId(5)).expect("kept")]
    );
}

fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn u24s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()[1..].to_vec()).collect()
}

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn coverage3(glyphs: &[u32]) -> Vec<u8> {
    [u16s(&[3]), u24s(&[glyphs.len() as u32]), u24s(glyphs)].concat()
}

// GSUB or GPOS of one script and one feature naming lookup 0, which reaches the rest through its records, each
// lookup one subtable. For `list2` the lists lie at 1.2's 32-bit offsets, and the lookups at 32-bit offsets too.
fn layout(lookups: &[(u16, &[u8])], version: u32, list2: bool) -> Vec<u8> {
    let width = if list2 { 4 } else { 2 };
    let offset = |at: usize| (at as u32).to_be_bytes()[4 - width..].to_vec();
    let mut head = [u32s(&[version]), u16s(&[0, 0, 0])].concat();
    head.extend(match version {
        0x0001_0001 => u32s(&[0]),
        0x0001_0002.. => u32s(&[0; 4]),
        _ => Vec::new(),
    });
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let features = [u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0])].concat();
    let bodies: Vec<Vec<u8>> = lookups.iter().map(|&(kind, sub)| [&u16s(&[kind, 0, 1, 8])[..], sub].concat()).collect();
    let mut list = u16s(&[lookups.len() as u16]);
    let mut at = 2 + width * bodies.len();
    for body in &bodies {
        list.extend(offset(at));
        at += body.len();
    }
    let lists = [0, scripts.len(), scripts.len() + features.len()].map(|at| head.len() + at);
    for (i, list_at) in lists.into_iter().enumerate() {
        let at = if list2 { 14 + 4 * i } else { 4 + 2 * i };
        head[at..at + width].copy_from_slice(&offset(list_at));
    }
    [head, scripts, features, list, bodies.concat()].concat()
}

// A face of `glyphs` glyphs beside `tables`, a to e mapped to glyphs 1 to 5.
fn layout_face(glyphs: u32, tables: &[(&str, &[u8])]) -> Font {
    let count = match u16::try_from(glyphs) {
        Ok(n) => ("maxp", maxp(n)),
        Err(_) => ("MAXP", [u32s(&[0x5000]), u24s(&[glyphs])].concat()),
    };
    let cmap = [u16s(&[0, 1, 3, 10]), u32s(&[12]), u16s(&[12, 0]), u32s(&[28, 0, 1, 0x61, 0x65, 1])].concat();
    let (head, loca) = (head(), empty_loca(glyphs as usize));
    let held = [("GLYF", &[0][..]), ("LOCA", &loca[..])];
    face(&[&[(count.0, &count.1[..]), ("cmap", &cmap[..]), ("head", &head[..])][..], &held, tables].concat())
}

// `text` shaped: each glyph's id, x and y offset and advance, rounded.
fn positions(f: &Font, text: &str) -> Vec<(u32, i32, i32, i32)> {
    let run = f.shape(text, &Location::default(), &ShapeOptions::default());
    let r = |v: f64| v.round() as i32;
    run.glyphs.iter().zip(&run.offsets).zip(&run.advances).map(|((&g, o), &a)| (g.0, r(o.x), r(o.y), r(a))).collect()
}

// A 24-bit mark attachment past its format: mark b, base a, one class, b's anchor at the origin, a's array at 50.
fn marks() -> Vec<u8> {
    let head = [u32s(&[20, 28]), u16s(&[1]), u32s(&[36, 50]), coverage3(&[2]), coverage3(&[1])].concat();
    [head, u24s(&[1]), u16s(&[0]), u24s(&[8]), u16s(&[1, 0, 0])].concat()
}

// MarkMarkPos 2 attaching b to a 300 across and 400 up, where GDEF classes both as marks.
fn mark_mark_pos() -> Vec<u8> {
    let sub = [u16s(&[2]), marks(), u16s(&[1]), u24s(&[5]), u16s(&[1, 300, 400])].concat();
    layout(&[(6, &sub)], 0x0001_0000, false)
}

// Faces of each 24-bit format, and forms no source font holds (reverse chains,
// CursivePos, ContextPos), each shaping "ab" the same in the font and in its text and all-glyph subsets.
#[test]
fn each_24_bit_format_shapes_and_subsets_alike() {
    type Want<'a> = &'a [(u32, i32, i32, i32)];
    let check = |name: &str, glyphs: u32, tables: &[(&str, &[u8])], want: Want| {
        let f = layout_face(glyphs, tables);
        assert_eq!(positions(&f, "ab"), want, "{name}");
        for subset in [
            f.subset_text("ab", &Location::default()),
            f.subset((0..glyphs).map(GlyphId).collect::<Vec<_>>(), &Location::default()),
        ] {
            let subset = subset.expect("a subset");
            let old: BTreeMap<u32, u32> =
                (0..glyphs).filter_map(|g| Some((subset.renumbered(GlyphId(g))?.0, g))).collect();
            let shaped = positions(&Font::from_vec(subset.font, 0).expect("it parses"), "ab");
            let back: Vec<_> = shaped.into_iter().map(|(g, x, y, a)| (old[&g], x, y, a)).collect();
            assert_eq!(back, want, "{name}, its subset");
        }
    };
    let one = |kind: u16, parts: &[Vec<u8>]| layout(&[(kind, &parts.concat())], 0x0001_0000, false);
    // A 24-bit format, its coverage of a at 12 and one set at 20.
    let one_set = |format: u16| [u16s(&[format]), u32s(&[12]), u24s(&[1, 20]), coverage3(&[1])].concat();
    let anchor = |x: u16, y: u16| u16s(&[1, x, y]);
    let ab: Want = &[(1, 0, 0, 500), (2, 0, 0, 500)];
    let ss3 = [u16s(&[3]), u32s(&[9]), u24s(&[10]), coverage3(&[1])].concat();
    let sp3 = [u16s(&[3]), u32s(&[10]), u16s(&[4, 100]), coverage3(&[1])].concat();

    let substituted: Want = &[(11, 0, 0, 500), (2, 0, 0, 500)];
    let to_0: Want = &[(0, 0, 0, 500), (2, 0, 0, 500)];
    check("SingleSubst 3", 20, &[("GSUB", &layout(&[(1, &ss3)], 0x0001_0000, false))], substituted);
    let wrapped = one(1, &[u16s(&[3]), u32s(&[9]), u24s(&[0xFF_FFFF]), coverage3(&[1])]);
    check("SingleSubst 3 of a delta wrapping at 24 bits", 20, &[("GSUB", &wrapped)], to_0);
    let wrapped = one(1, &[u16s(&[1, 6, 0xFFFF]), coverage3(&[1])]);
    check("SingleSubst 1 over a coverage of format 3", 20, &[("GSUB", &wrapped)], to_0);
    let ranges = one(1, &[u16s(&[4]), u32s(&[15]), u24s(&[2, 15, 16]), u16s(&[4]), u24s(&[1, 1, 2, 0])]);
    check("SingleSubst 4 over a coverage of format 4", 20, &[("GSUB", &ranges)], &[(15, 0, 0, 500), (16, 0, 0, 500)]);
    let multiple = one(2, &[one_set(2), u16s(&[2]), u24s(&[70_000, 70_001])]);
    let want: Want = &[(70_000, 0, 0, 500), (70_001, 0, 0, 500), (2, 0, 0, 500)];
    check("MultipleSubst 2, past 65,535", 80_000, &[("GSUB", &multiple)], want);
    let ligature = one(4, &[one_set(2), u16s(&[1]), u24s(&[5, 70_002]), u16s(&[2]), u24s(&[2])]);
    check("LigatureSubst 2, past 65,535", 80_000, &[("GSUB", &ligature)], &[(70_002, 0, 0, 500)]);
    let reverse = one(8, &[u16s(&[2]), u32s(&[16]), u16s(&[0, 0]), u24s(&[1, 9]), coverage3(&[1])]);
    check("ReverseChainSingleSubst 2", 20, &[("GSUB", &reverse)], &[(9, 0, 0, 500), (2, 0, 0, 500)]);
    let lists = [("GSUB 1.2 of 32-bit lists", 0x0001_0002, true), ("GSUB 1.2 of 16-bit lists", 0x0001_0002, false)];
    for (name, version, list2) in lists {
        check(name, 20, &[("GSUB", &layout(&[(1, &ss3)], version, list2))], substituted);
    }
    check("GSUB 2.0", 20, &[("GSUB", &layout(&[(1, &ss3)], 0x0002_0000, false))], ab);

    let moved: Want = &[(1, 0, 0, 600), (2, 0, 0, 500)];
    check("SinglePos 3", 20, &[("GPOS", &layout(&[(1, &sp3)], 0x0001_0000, false))], moved);
    let single = one(1, &[u16s(&[4]), u32s(&[13]), u16s(&[4]), u24s(&[1]), u16s(&[120]), coverage3(&[1])]);
    check("SinglePos 4", 20, &[("GPOS", &single)], &[(1, 0, 0, 620), (2, 0, 0, 500)]);
    let pairs = [u16s(&[3]), u32s(&[16]), u16s(&[4, 0]), u24s(&[1, 24]), coverage3(&[1])].concat();
    let pairs = one(2, &[pairs, u24s(&[1, 2]), u16s(&[0xFFCE])]);
    check("PairPos 3", 20, &[("GPOS", &pairs)], &[(1, 0, 0, 450), (2, 0, 0, 500)]);
    let classes = [u16s(&[3]), u24s(&[1, 1, 1]), u16s(&[3]), u24s(&[2, 1, 1])].concat();
    let pairs = [u16s(&[4]), u32s(&[30]), u16s(&[4, 0]), u32s(&[38, 49]), u16s(&[2, 2, 0, 0, 0, 0xFFBA])].concat();
    let pairs = one(2, &[pairs, coverage3(&[1]), classes]);
    check("PairPos 4 of ClassDef 3s", 20, &[("GPOS", &pairs)], &[(1, 0, 0, 430), (2, 0, 0, 500)]);
    let cursive = [u16s(&[2]), u32s(&[21]), u24s(&[2, 0, 38, 32, 0]), coverage3(&[1, 2])].concat();
    let cursive = one(3, &[cursive, anchor(0, 0), anchor(500, 100)]);
    check("CursivePos 2", 20, &[("GPOS", &cursive)], &[(1, 0, 0, 500), (2, 0, 100, 500)]);
    let gdef = |classes: [u16; 2]| [u32s(&[0x0001_0000]), u16s(&[12, 0, 0, 0, 1, 1, 2]), u16s(&classes)].concat();
    let base = one(4, &[u16s(&[2]), marks(), u24s(&[1, 6]), anchor(300, 400)]);
    check("MarkBasePos 2", 20, &[("GPOS", &base)], &[(1, 0, 0, 500), (2, -200, 400, 500)]);
    let ligature = one(5, &[u16s(&[2]), marks(), u24s(&[1, 6]), u16s(&[1]), u24s(&[5]), anchor(300, 400)]);
    let want: Want = &[(1, 0, 0, 500), (2, -200, 400, 0)];
    check("MarkLigPos 2", 20, &[("GPOS", &ligature), ("GDEF", &gdef([2, 3]))], want);
    let want: Want = &[(1, 0, 0, 0), (2, 300, 400, 0)];
    check("MarkMarkPos 2", 20, &[("GPOS", &mark_mark_pos()), ("GDEF", &gdef([3, 3]))], want);

    // Each context, and whether its rule reaches lookup 1 at the second glyph: Context 4's rules lie at 16-bit
    // offsets, ChainContext 4's at 24-bit ones, and ChainContext 5 counts its rule sets in 16 bits.
    let rule = [u16s(&[2, 1]), u24s(&[2]), u16s(&[0, 1])].concat();
    let class_rule = u16s(&[2, 1, 2, 0, 1]);
    let chain_rule = [u16s(&[0, 2]), u24s(&[2]), u16s(&[0, 1, 0, 1])].concat();
    let chain_class_rule = u16s(&[0, 2, 2, 0, 1, 0, 1]);
    let classes = [u16s(&[4]), u24s(&[2, 1, 1]), u16s(&[1]), u24s(&[2, 2]), u16s(&[2])].concat();
    let sets = [one_set(4), u16s(&[1])].concat();
    let class_sets = [u16s(&[5]), u32s(&[19, 27]), u24s(&[2, 0, 48]), coverage3(&[1]), classes.clone()].concat();
    let coverages = [u16s(&[6, 2, 1]), u24s(&[16, 24]), u16s(&[0, 1]), coverage3(&[1]), coverage3(&[2])].concat();
    let contexts = [
        ("Context 4, rules at 16-bit offsets", [sets.clone(), u16s(&[4]), rule.clone()].concat(), true),
        ("Context 4, rules at 24-bit offsets", [sets.clone(), u24s(&[5]), rule].concat(), false),
        ("Context 5, ClassDef 4", [class_sets, u16s(&[1]), u24s(&[5]), class_rule].concat(), true),
        ("Context 6", coverages, true),
    ];
    let chain_sets = |count: Vec<u8>| {
        let at = 24 + count.len() as u32;
        let head = [u16s(&[5]), u32s(&[at, 0, at + 8, 0]), count, u24s(&[0, at + 29])];
        [head.concat(), coverage3(&[1]), classes.clone(), u16s(&[1]), u24s(&[5]), chain_class_rule.clone()].concat()
    };
    let chains = [
        ("ChainContext 4, rules at 24-bit offsets", [sets.clone(), u24s(&[5]), chain_rule.clone()].concat(), true),
        ("ChainContext 4, rules at 16-bit offsets", [sets, u16s(&[4]), chain_rule].concat(), false),
        ("ChainContext 5, sets counted in 16 bits", chain_sets(u16s(&[2])), true),
        ("ChainContext 5, sets counted in 24 bits", chain_sets(u24s(&[2])), false),
    ];
    let nest = |tag: &str, kind: u16, single: &[u8], applied: Want, cases: &[(&str, Vec<u8>, bool)]| {
        for (name, sub, applies) in cases {
            let table = layout(&[(kind, sub), (1, single)], 0x0001_0000, false);
            check(&format!("{tag} {name}"), 20, &[(tag, &table)], if *applies { applied } else { ab });
        }
    };
    nest("GSUB", 5, &ss3, substituted, &contexts);
    nest("GSUB", 6, &ss3, substituted, &chains);
    nest("GPOS", 7, &sp3, moved, &contexts);
    nest("GPOS", 8, &sp3, moved, &chains);
}

// GDEF 1.4's 32-bit classes make a and b marks where its 16-bit ones make them bases, with b in mark class 5 and glyph
// 3's caret at 100: read from 1.4's, so MarkMarkPos 2 attaches b to a; from 1.3, none.
#[test]
fn a_gdef_of_1_4_is_read_from_its_32_bit_tables() {
    let classes = [u16s(&[3]), u24s(&[1, 2, 3, 3]), u16s(&[1, 1, 2, 1, 1])].concat();
    let attachment = [u16s(&[4]), u24s(&[1, 2, 2]), u16s(&[5])].concat();
    let carets = [u32s(&[10]), u24s(&[1, 18]), coverage3(&[3]), u16s(&[1, 4, 1, 100])].concat();
    let mut gdef = [u32s(&[0x0001_0004]), u16s(&[52, 0, 0, 0, 0]), u32s(&[0, 38, 0, 75, 62, 0])].concat();
    gdef.extend([classes, attachment, carets].concat());
    let gpos = mark_mark_pos();
    let read: [(u8, _, _, Vec<Option<f64>>, &[_]); 2] = [
        (4, GlyphClass::Mark, Some(5), vec![Some(100.0)], &[(1, 0, 0, 0), (2, 300, 400, 0)]),
        (3, GlyphClass::Base, None, vec![], &[(1, 0, 0, 500), (2, 0, 0, 500)]),
    ];
    for (minor, class, attachment, carets, want) in read {
        gdef[3] = minor;
        let f = layout_face(20, &[("GPOS", &gpos), ("GDEF", &gdef)]);
        assert_eq!(
            [f.glyph_class(GlyphId(1)), f.glyph_class(GlyphId(2)), f.glyph_class(GlyphId(3))],
            [Some(class), Some(class), None]
        );
        assert_eq!(
            (
                f.mark_attachment_class(GlyphId(2)).map(|c| c.get()),
                f.ligature_carets(GlyphId(3), &Location::default(), Orientation::Horizontal)
            ),
            (attachment, carets)
        );
        assert_eq!(positions(&f, "ab"), want, "GDEF 1.{minor}");
    }
}

// What only longer rules reach (a ligature's third part, chain and reverse-chain context, a mark and base past index 0,
// an unindexed format 4 coverage range, a pair past its set's first record).
#[test]
fn longer_24_bit_rules_shape_past_their_first_entries() {
    let one = |kind: u16, parts: &[Vec<u8>]| layout(&[(kind, &parts.concat())], 0x0001_0000, false);
    let set = |format: u16, glyph: u32| [u16s(&[format]), u32s(&[12]), u24s(&[1, 20]), coverage3(&[glyph])].concat();
    let ligature = one(4, &[set(2, 1), u16s(&[1]), u24s(&[5, 70_003]), u16s(&[3]), u24s(&[2, 3])]);
    assert_eq!(positions(&layout_face(80_000, &[("GSUB", &ligature)]), "abc"), [(70_003, 0, 0, 500)]);

    let middle = |b: u32| [(1, 0, 0, 500), (b, 0, 0, 500), (3, 0, 0, 500)];
    let rule = [u16s(&[1]), u24s(&[1]), u16s(&[1, 1]), u24s(&[3]), u16s(&[1, 0, 1])].concat();
    let single = [u16s(&[3]), u32s(&[9]), u24s(&[10]), coverage3(&[2])].concat();
    let chain = layout(&[(6, &[set(4, 2), u16s(&[1]), u24s(&[5]), rule].concat()), (1, &single)], 0x0001_0000, false);
    assert_eq!(positions(&layout_face(20, &[("GSUB", &chain)]), "abc"), middle(12));
    let context = [u16s(&[1]), u24s(&[30]), u16s(&[1]), u24s(&[38, 1, 9])].concat();
    let reverse = one(8, &[u16s(&[2]), u32s(&[22]), context, coverage3(&[2]), coverage3(&[1]), coverage3(&[3])]);
    assert_eq!(positions(&layout_face(20, &[("GSUB", &reverse)]), "abc"), middle(9));

    let coverage4 = [u16s(&[4]), u24s(&[1, 0, 1, 0])].concat();
    let marks = [u24s(&[2]), u16s(&[0]), u24s(&[13]), u16s(&[0]), u24s(&[19]), u16s(&[1, 0, 0, 1, 50, 60])].concat();
    let bases = [u24s(&[2, 9, 15]), u16s(&[1, 100, 200, 1, 300, 400])].concat();
    let head = [u16s(&[2]), u32s(&[20, 31]), u16s(&[1]), u32s(&[45, 70]), coverage3(&[0, 2])].concat();
    let base = one(4, &[head, coverage4, marks, bases]);
    assert_eq!(positions(&layout_face(20, &[("GPOS", &base)]), "ab"), [(1, 0, 0, 500), (2, -250, 340, 500)]);

    let pairs = [u16s(&[3]), u32s(&[16]), u16s(&[4, 0]), u24s(&[1, 24]), coverage3(&[1]), u24s(&[2, 0])].concat();
    let pairs = one(2, &[pairs, u16s(&[10]), u24s(&[2]), u16s(&[0xFFCE])]);
    assert_eq!(positions(&layout_face(20, &[("GPOS", &pairs)]), "ab"), [(1, 0, 0, 450), (2, 0, 0, 500)]);
}

// VORG 1 names glyph 1's origin, VORG 2 glyph 65,537's in 24 bits, others taking the default; a subset of 65,537
// keeps VORG 2 with the glyph's new id.
#[test]
fn vorg_2_names_origins_past_65_535() {
    let (head, upper, loca) = (head(), b"\0\0\x50\0\x01\0\x02", empty_loca(0x10002));
    let held = |t: &[(&str, &[u8])]| {
        face(&[&[("head", &head[..]), ("MAXP", upper), ("GLYF", &[0]), ("LOCA", &loca)], t].concat())
    };
    let v1 = held(&[("VORG", &[0, 1, 0, 0, 0, 123, 0, 1, 0, 1, 1, 65])]);
    assert_eq!(v1.vertical_origin(GlyphId(1), &Location::default()), 321.0);
    let vorg = [0, 2, 0, 0, 0, 123, 0, 0, 1, 1, 0, 1, 1, 65];
    let v2 = held(&[("VORG", &vorg)]);
    assert_eq!(
        (
            v2.vertical_origin(GlyphId(0x1_0001), &Location::default()),
            v2.vertical_origin(GlyphId(1), &Location::default())
        ),
        (321.0, 123.0)
    );
    let sub = Font::from_vec(v2.subset([GlyphId(0x1_0001)], &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert_eq!(sub.vertical_origin(GlyphId(1), &Location::default()), 321.0);
    assert_eq!(sub.table(Tag::new(b"VORG")), Some(&[0, 2, 0, 0, 0, 123, 0, 0, 1, 0, 0, 1, 1, 65][..]));
}

// Glyph 0 paints PaintGlyph 33 naming glyph 65,537 in 24 bits, a subset renaming it 1. daegun paints COLR only
// beside a CPAL, so one is added.
#[test]
fn paint_glyph_format_33_names_a_glyph_in_24_bits() {
    let colr = [
        &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 34][..], &[0; 16], &[0, 0, 0, 1, 0, 0, 0, 0, 0, 10],
        &[33, 0, 0, 7, 1, 0, 1, 2, 0xFF, 0xFF, 0x40, 0],
    ]
    .concat();
    let glyph = |f: &Font| {
        let graph = f.paint_graph(GlyphId(0), &Location::default(), 0)?;
        let PaintNode::Glyph { glyph } = graph.root().node() else { return None };
        let child = graph.root().children().next().map(|c| c.node());
        Some((glyph, matches!(child, Some(PaintNode::Solid { foreground: true, .. }))))
    };
    let cpal = [0, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 255];
    let (head, loca) = (head(), empty_loca(0x10002));
    let held = [("head", &head[..]), ("MAXP", b"\0\0\x50\0\x01\0\x02"), ("GLYF", &[0]), ("LOCA", &loca)];
    let f = face(&[&held[..], &[("COLR", &colr), ("CPAL", &cpal)]].concat());
    assert_eq!(glyph(&f), Some((GlyphId(0x1_0001), true)));
    // The text color alone needs no palette, so the subset drops CPAL and isn't drawn there.
    let sub = Font::from_vec(f.subset([GlyphId(0)], &Location::default()).expect("a subset").font, 0)
        .expect("it parses");
    assert!(!sub.has_table(Tag::new(b"CPAL")));
    let colr = sub.table(Tag::new(b"COLR")).expect("COLR");
    assert!(colr.windows(7).any(|w| w == [33, 0, 0, 7, 0, 0, 1]), "{colr:?}");
}


// 'latn' names 'romn' at a BaseCoord of format 4, its glyph in 24 bits, read as 123. A subset leaves BASE out, as it
// does where a format 2 coordinate names a glyph.
#[test]
fn base_coordinate_format_4_reads_its_coordinate() {
    let base = [
        &[0, 1, 0, 0, 0, 8, 0, 0, 0, 4, 0, 10, 0, 1, b'r', b'o', b'm', b'n'][..],
        &[0, 1, b'l', b'a', b't', b'n', 0, 8, 0, 6, 0, 0, 0, 0, 0, 0, 0, 1, 0, 6],
        &[0, 4, 0, 123, 0, 0, 0, 0, 0],
    ]
    .concat();
    let f = face(&[("head", &head()), ("maxp", &maxp(2)), ("BASE", &base)]);
    let info = f.base_script(Tag::new(b"latn"), &Location::default(), Orientation::Horizontal).expect("a baseline");
    assert_eq!(info.baselines.get(&Tag::new(b"romn")), Some(&123.0));
    assert!(
        !Font::from_vec(f.subset([GlyphId(1)], &Location::default()).expect("a subset").font, 0)
            .expect("it parses")
            .has_table(Tag::new(b"BASE"))
    );
}

// A JSTF 1.1 whose scriptList2 extends glyphs 7 and 65,537 in 24 bits, default and 'ENG ' sharing a system enabling
// GSUB lookup 3: read through it, and a subset keeping both writes 1.0 once their new ids (1, 2) fit 16 bits.
#[test]
fn jstf_1_1_reads_and_subsets_script_list_2() {
    let jstf = [
        &[0, 1, 0, 1, 0, 1, b'l', b'a', b't', b'n', 0, 22][..],
        &[0, 1, b'l', b'a', b't', b'n', 0, 0, 0, 32],
        &[0, 6, 0, 0, 0, 0, 0, 1, 0, 5],
        &[0, 0, 0, 16, 0, 0, 0, 24, 0, 1, b'E', b'N', b'G', b' ', 0, 24],
        &[0, 2, 0, 0, 7, 1, 0, 1, 0, 1, 0, 4],
        &[0, 20],
        &[0; 18],
        &[0, 1, 0, 3],
    ]
    .concat();
    let (head, loca, upper) = (head(), empty_loca(0x10002), b"\0\0\x50\0\x01\0\x02");
    let f = face(&[("head", &head), ("MAXP", upper), ("GLYF", &[0]), ("LOCA", &loca), ("JSTF", &jstf)]);
    let levels = |f: &Font| {
        [None, Some(Tag::new(b"ENG "))].map(|l| f.justification_priorities(Tag::new(b"latn"), l).map(|l| l.len()))
    };
    assert_eq!(
        (f.justification_glyphs(Tag::new(b"latn")), levels(&f)),
        (Some(vec![GlyphId(7), GlyphId(0x1_0001)]), [Some(1); 2])
    );
    let kept = f.subset([GlyphId(7), GlyphId(0x1_0001)], &Location::default()).expect("a subset");
    let sub = Font::from_vec(kept.font, 0).expect("it parses");
    assert_eq!(
        (sub.justification_glyphs(Tag::new(b"latn")), levels(&sub)),
        (Some(vec![GlyphId(1), GlyphId(2)]), [Some(1); 2])
    );
    assert_eq!(sub.table(Tag::new(b"JSTF")).map(|t| &t[..4]), Some(&[0, 1, 0, 0][..]));
}

// A collection at `version` whose first list names `old` and whose extended list, after the first and at major 2 its
// three DSIG fields, names `new`; each face's table offsets moved to where it sits.
fn extended_collection(version: [u8; 4], old: &[u8], new: &[u8]) -> Vec<u8> {
    let dsig = if version[1] == 2 { 12 } else { 0 };
    let moved = |sfnt: &[u8], by: usize| {
        let mut face = sfnt.to_vec();
        for i in 0..usize::from(u16::from_be_bytes([face[4], face[5]])) {
            let at = 20 + 16 * i;
            let offset = u32::from_be_bytes(face[at..at + 4].try_into().expect("four")) + by as u32;
            face[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        }
        face
    };
    let (first, second) = (24 + dsig, 24 + dsig + old.len());
    let header = [&b"ttcf"[..], &version, &1u32.to_be_bytes(), &(first as u32).to_be_bytes(), &vec![0; dsig]].concat();
    [header, 1u32.to_be_bytes().to_vec(), (second as u32).to_be_bytes().to_vec(), moved(old, first), moved(new, second)]
        .concat()
}

// The extended TTC header (1.1 and 2.1): its faces are its second list's; at minor 0, or at a major other than 1 or
// 2, the first list's, as in a plain header.
#[test]
fn an_extended_collection_names_its_faces_in_its_second_list() {
    let sfnt = |n: u16| {
        let tables = Tables::from([(Tag::new(b"head"), head()), (Tag::new(b"maxp"), maxp(n))]);
        daegun::raw::build_font(&tables).unwrap()
    };
    let glyphs = |ttc: &[u8]| Font::from_collection(ttc, 0).map(|f| f.glyph_count()).ok();
    for (version, want) in [([0, 1, 0, 1], 9), ([0, 2, 0, 1], 9), ([0, 1, 0, 0], 3), ([0, 3, 0, 1], 3)] {
        let ttc = extended_collection(version, &sfnt(3), &sfnt(9));
        assert_eq!((glyphs(&ttc), Font::face_count(&ttc)), (Some(want), 1), "{version:?}");
    }
    let mut cut = extended_collection([0, 1, 0, 1], &sfnt(3), &sfnt(9));
    cut[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(Font::face_count(&cut), (cut.len() - 20) / 4, "a count cut to the words after its own");
}
