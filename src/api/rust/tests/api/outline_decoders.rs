use daegun::outline::{OutlinePen, Path};
use daegun::{Font, GlyphId, Location, Tag};

use crate::located;

#[derive(Default)]
struct Record(Vec<String>);

impl OutlinePen for Record {
    fn move_to(&mut self, x: f32, y: f32) { self.0.push(format!("M{x},{y}")); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.push(format!("L{x},{y}")); }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("Q{x},{y}")); }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("C{x},{y}")); }
    fn close(&mut self) { self.0.push("Z".into()); }
}

fn be(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

// A simple glyph of on-curve points with full-width coordinates.
fn simple(points: &[(i16, i16)], ends: &[u16]) -> Vec<u8> {
    let mut g = be(&[ends.len() as i16, 0, 0, 0, 0]);
    ends.iter().for_each(|&e| g.extend(e.to_be_bytes()));
    g.extend([0, 0]);
    g.extend(std::iter::repeat_n(0x01u8, points.len()));
    let (mut px, mut py) = (0, 0);
    for &(x, _) in points { g.extend((x - px).to_be_bytes()); px = x; }
    for &(_, y) in points { g.extend((y - py).to_be_bytes()); py = y; }
    g
}

// A composite of (flags, glyph, args, scale words) records; MORE_COMPONENTS is set on all but the last.
fn composite(records: &[(u16, u16, [i16; 2], &[i16])]) -> Vec<u8> {
    let mut g = be(&[-1, 0, 0, 0, 0]);
    for (i, (flags, gid, args, scale)) in records.iter().enumerate() {
        let more = if i + 1 < records.len() { 0x20 } else { 0 };
        g.extend((flags | more | 0x0001).to_be_bytes());
        g.extend(gid.to_be_bytes());
        g.extend(be(args));
        g.extend(be(scale));
    }
    g
}

fn font(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<usize>) {
    let (mut glyf, mut loca) = (Vec::new(), vec![0]);
    for g in glyphs {
        glyf.extend(g);
        loca.push(glyf.len());
    }
    (glyf, loca)
}

fn draw(glyf: &[u8], loca: &[usize], gid: u32) -> Result<Vec<String>, String> {
    let mut pen = Record::default();
    daegun::raw::outline_glyf_bytes(glyf, false, loca, gid, &mut pen).map(|_| pen.0).map_err(|e| e.to_string())
}

const TRIANGLE: [(i16, i16); 3] = [(0, 0), (100, 0), (0, 100)];

#[test]
fn a_malformed_simple_glyph_is_refused() {
    let whole = simple(&TRIANGLE, &[2]);
    let (glyf, loca) = font(&[whole[..whole.len() - 2].to_vec()]);
    assert!(draw(&glyf, &loca, 0).is_err(), "a glyph missing its last y");
    let (glyf, loca) = font(&[simple(&[(0, 0), (1, 1), (2, 2), (3, 3)], &[2, 1])]);
    assert!(draw(&glyf, &loca, 0).is_err(), "end points 2, 1");
}

#[test]
fn a_glyph_is_read_only_from_its_own_bytes() {
    let (glyf, mut loca) = font(&[simple(&TRIANGLE, &[2]), simple(&TRIANGLE, &[2])]);
    loca[1] -= 4;
    assert!(draw(&glyf, &loca, 0).is_err(), "glyph 0 drew from bytes loca gives glyph 1");
}

#[test]
fn a_component_is_placed_by_matching_points() {
    let (glyf, loca) = font(&[
        simple(&TRIANGLE, &[2]),
        composite(&[(0x0002, 0, [10, 10], &[]), (0x0000, 0, [1, 2], &[])]),
    ]);
    assert_eq!(
        draw(&glyf, &loca, 1).expect("draws"),
        ["M10,10", "L110,10", "L10,110", "Z", "M110,-90", "L210,-90", "L110,10", "Z"],
        "the second triangle's point 2 lands on the first's point 1 at (110, 10)",
    );
}

#[test]
fn a_scale_is_read_before_a_matrix() {
    let (glyf, loca) = font(&[
        simple(&TRIANGLE, &[2]),
        composite(&[(0x0002 | 0x0008 | 0x0080, 0, [0, 0], &[0x2000]), (0x0002, 0, [300, 0], &[])]),
    ]);
    assert_eq!(
        draw(&glyf, &loca, 1).expect("draws"),
        ["M0,0", "L50,0", "L0,50", "Z", "M300,0", "L400,0", "L300,100", "Z"],
    );
}

// Four levels of sixteen copies over one leaf: guards against a cycle, a wide tree and a heavy leaf. Leaves of points
// fill a drawn outline's 65,535 first, and empty ones spend the glyph's work.
#[test]
fn the_glyf_budgets_hold() {
    let records = |gid: u16| composite(&vec![(0x0002u16, gid, [0i16, 0], &[][..]); 16]);
    let (glyf, loca) = font(&[records(0)]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("inside itself"));

    let (glyf, loca) = font(&[records(1), records(2), records(3), records(4), simple(&TRIANGLE, &[2])]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("than an outline can"));
    let (glyf, loca) = font(&[records(1), records(2), records(3), records(4), Vec::new()]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("work budget"));

    let many: Vec<(i16, i16)> = (0..20_000).map(|i| (i as i16, 0)).collect();
    let (glyf, loca) = font(&[records(1), records(2), simple(&many, &[19_999])]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("than an outline can"));

    // 32,767 end points and nothing after them: read in full each visit unless charged as read.
    let mut leaf = be(&[32_767, 0, 0, 0, 0]);
    (0..32_767u16).for_each(|e| leaf.extend(e.to_be_bytes()));
    let (glyf, loca) = font(&[records(1), records(2), records(3), records(4), leaf]);
    let started = std::time::Instant::now();
    assert!(draw(&glyf, &loca, 0).is_err());
    assert!(started.elapsed().as_secs_f64() < 2.0, "{:?}", started.elapsed());
}

#[test]
fn a_replayed_path_closes_every_contour() {
    let mut path = Path::default();
    path.move_to(0.0, 0.0);
    path.line_to(10.0, 0.0);
    path.move_to(0.0, 10.0);
    path.line_to(10.0, 10.0);
    let mut out = Record::default();
    path.replay(None, &mut out);
    assert_eq!(out.0, ["M0,0", "L10,0", "Z", "M0,10", "L10,10", "Z"]);
}

// A CFF of four glyphs: .notdef, A (SID 34), grave (SID 124), and Agrave drawn as a seac of the two
// with its accent at (50, 200), reached after a stem hint and through a local subroutine.
fn seac_cff(seac: &[u8]) -> Vec<u8> {
    let index = |items: &[&[u8]]| {
        let mut b = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() { return b; }
        b.push(2);
        let mut at = 1u16;
        b.extend(at.to_be_bytes());
        for i in items { at += i.len() as u16; b.extend(at.to_be_bytes()); }
        items.iter().for_each(|i| b.extend(*i));
        b
    };
    let int = |v: usize| { let mut b = vec![29]; b.extend((v as i32).to_be_bytes()); b };
    let a = [139u8, 139, 21, 239, 139, 5, 139, 239, 5, 14];
    let grave = [139u8, 139, 21, 149, 139, 5, 139, 149, 5, 14];
    let charstrings = index(&[&[14], &a, &grave, seac]);
    let charset = [0u8, 0, 34, 0, 124, 0, 175];
    let subrs = index(&[&[14]]);
    let name = index(&[b"T"]);
    let empty = index(&[]);
    let top_len = 6 + 6 + 11;
    let top_index_len = index(&[&vec![0u8; top_len]]).len();
    let charset_at = 4 + name.len() + top_index_len + 2 * empty.len();
    let charstrings_at = charset_at + charset.len();
    let private_at = charstrings_at + charstrings.len();
    let mut private = int(6);
    private.push(19);
    let mut top = int(charset_at);
    top.push(15);
    top.extend(int(charstrings_at));
    top.push(17);
    top.extend(int(private.len()));
    top.extend(int(private_at));
    top.push(18);
    [vec![1, 0, 4, 2], name, index(&[&top]), empty.clone(), empty, charset.to_vec(), charstrings, private, subrs]
        .concat()
}

#[test]
fn a_seac_draws_its_base_and_accent() {
    let seac_after_hint = [139u8, 149, 1, 189, 247, 92, 204, 247, 85, 14];
    let seac_in_subroutine = [139u8, 149, 1, 189, 247, 92, 204, 247, 85, 32, 10];
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    for program in [&seac_after_hint[..], &seac_in_subroutine[..]] {
        let tables = std::collections::BTreeMap::from([
            (Tag::new(b"CFF "), seac_cff(program)),
            (Tag::new(b"head"), head.clone()),
            (Tag::new(b"maxp"), vec![0, 0, 0x50, 0, 0, 4]),
        ]);
        let f = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the face parses");
        let mut pen = Record::default();
        f.outline(GlyphId(3), &Location::default(), &mut pen).expect("draws");
        assert_eq!(pen.0, ["M0,0", "L100,0", "L100,100", "Z", "M50,200", "L60,200", "L60,210", "Z"]);
    }
}

// STIX Two Math's Ograve switches hints at the grave's second point. The mask must name it as the
// collected outline numbers it, 27, not as the interpreter counted moves and closing lines, 29.
#[test]
fn a_hint_mask_names_the_point_it_applies_from() {
    let stix =
        daegun::Font::from_bytes(&std::fs::read(format!("{}/stix-two-math/STIX2Math.otf", crate::FONTS)).unwrap())
            .unwrap();
    let at = |gid: u32| {
        let hints = stix.cff_hints(GlyphId(gid), &daegun::Location::default()).unwrap();
        hints.masks.iter().map(|m| m.point).collect::<Vec<_>>()
    };
    assert_eq!(at(145), [0, 27]);
    assert_eq!(at(146), [0, 24, 26]);
}

// Every point a pen is given, a curve's three each.
#[derive(Default)]
struct Points(Vec<(f32, f32)>);

impl OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) { self.0.push((x, y)); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.push((x, y)); }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) { self.0.extend([(cx, cy), (x, y)]); }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.0.extend([(ax, ay), (bx, by), (x, y)]);
    }
    fn close(&mut self) {}
}

// cff-operators.otf's outlines unhinted and unscaled, floored, None where a glyph fails; what it floors is 16.16:
// 200/3, sqrt(2), and `random`'s draws from seed -5, its low 32 bits, 42,754 and 4,539 in 65,536ths times 300.
#[test]
fn cff_operators_draw_their_outlines_in_16_16() {
    let bytes = std::fs::read(format!("{}/test-fixtures/cff-operators.otf", crate::FONTS)).unwrap();
    let font = daegun::Font::from_bytes(&bytes).unwrap();
    let want: [Option<&[(i32, i32)]>; 21] = [
        Some(&[(0, 0), (66, 0), (266, 300), (166, 300)]),
        Some(&[(0, 0), (141, 0), (141, 173), (41, 173)]),
        Some(&[(0, 0), (33, 0), (33, 300)]),
        Some(&[(0, 0), (250, 0), (250, 250), (125, 125)]),
        Some(&[(0, 0), (400, 0), (400, 300)]),
        Some(&[(0, 0), (300, 0), (300, 300)]),
        Some(&[(0, 0), (250, 0), (250, 125)]),
        Some(&[(0, 0), (10, 80), (410, 380)]),
        Some(&[(0, 0), (150, 400), (150, 700)]),
        None,
        Some(&[(0, 0), (100, 200), (100, 500)]),
        Some(&[(0, 0), (100, 200), (100, 500)]),
        Some(&[(0, 0), (195, 0), (195, 20)]),
        Some(&[(0, 0), (300, 0), (300, 300)]),
        Some(&[(0, 0), (300, 0), (300, 300), (0, 300)]),
        None,
        Some(&[(0, 0), (300, 0), (300, 300)]),
        Some(&[]),
        // A late mask drops its two operands and takes 300's first byte, 247, so its second, 192, reads as 53.
        Some(&[(0, 0), (53, 0), (53, 300)]),
        None,
        Some(&[(0, 0), (327, 0), (327, 327)]),
    ];
    let mut drawn = Vec::new();
    for (gid, want) in (1..).zip(want) {
        let mut pen = Points::default();
        let got = font.outline(GlyphId(gid), &Location::default(), &mut pen).ok().map(|_| pen.0.clone());
        let floor = |p: &[(f32, f32)]| p.iter().map(|&(x, y)| (x.floor() as i32, y.floor() as i32)).collect::<Vec<_>>();
        let floored = got.as_deref().map(floor);
        assert_eq!(floored.as_deref(), want, "glyph {gid}: not the expected outline");
        drawn.push(got.unwrap_or_default());
    }
    let fixed = |v: i32| (f64::from(v) / 65536.0) as f32;
    assert_eq!(drawn[0][1].0, fixed(4_369_067));
    assert_eq!(drawn[1][1].0, fixed(9_268_200));
    assert_eq!(drawn[12][2].1, fixed(4_539 * 300));
}

// FontMatrices unhinted, floored: a sheared Top DICT its FDs compose with (half, double, offset) or an odd matrix
// read as identity; a 3 em name-keyed offset; FD em 1 at 1,000×, its seac 11 failing as a CID-keyed font's does.
#[test]
fn font_matrices_place_outlines() {
    type Glyphs = Vec<Vec<(i32, i32)>>;
    let cid: Glyphs = (1..=8)
        .map(|k| {
            let at = 10_000 * k;
            match k % 2 {
                1 => vec![(at, at), (at, at + 300_000), (at + 200_000, at + 300_000)],
                _ => vec![(at, at), (at + 300_000, at), (at + 400_000, at)],
            }
        })
        .chain([
            vec![(100_000, 0), (300_000, 0), (300_000, 200_000)],
            vec![(100_000, 0), (200_000, 0), (200_000, 100_000)],
        ])
        .collect();
    let cases: [(&str, Glyphs); 3] = [
        ("cff-matrix", vec![
            vec![(110, 0), (400, 0), (450, 300), (160, 300)],
            vec![(73, 20), (213, 20), (238, 170), (98, 170)],
            vec![(260, 0), (800, 0), (900, 600), (360, 600)],
            vec![(140, 0), (400, 0), (450, 300), (190, 300)],
        ]),
        ("cff-matrix-name", vec![vec![(6110, 8000), (6400, 8000), (6400, 8300), (6110, 8300)]]),
        ("cff-cid", cid),
    ];
    for (name, want) in cases {
        let bytes = std::fs::read(format!("{}/test-fixtures/{name}.otf", crate::FONTS)).unwrap();
        let font = daegun::Font::from_bytes(&bytes).unwrap();
        for (gid, want) in (1..).zip(want) {
            let mut pen = Points::default();
            font.outline(GlyphId(gid), &Location::default(), &mut pen).expect("draws");
            let floored: Vec<_> = pen.0.iter().map(|&(x, y)| (x.floor() as i32, y.floor() as i32)).collect();
            assert_eq!(floored, want, "{name}: glyph {gid} isn't where its matrices place it");
        }
    }
    let cid = daegun::Font::from_bytes(&std::fs::read(format!("{}/test-fixtures/cff-cid.otf", crate::FONTS)).unwrap());
    assert!(cid.unwrap().outline(GlyphId(11), &Location::default(), &mut Points::default()).is_err());
}

// Every glyph of Source Serif 4, drawn again once all are drawn, draws as it drew first, at its default and at a
// location; and a location draws its own outlines, as a font that never drew at its default draws them.
#[test]
fn a_cff2_glyph_drawn_again_draws_as_it_drew_first() {
    let bytes = std::fs::read(format!("{}/source-serif/SourceSerif4Variable-Roman.otf", crate::FONTS)).unwrap();
    let (font, fresh) = (daegun::Font::from_bytes(&bytes).unwrap(), daegun::Font::from_bytes(&bytes).unwrap());
    let all = |f: &daegun::Font, axes: &[(&str, f64)]| -> Vec<Option<Path>> {
        let at = located(f, axes);
        let draw = |gid| {
            let mut path = Path::default();
            f.outline(GlyphId(gid), &at, &mut path).ok().map(|()| path)
        };
        (0..f.glyph_count()).map(draw).collect()
    };
    let bold = [("wght", 700.0)];
    for axes in [&[][..], &bold] {
        let first = all(&font, axes);
        assert!(first.iter().flatten().any(|p| !p.is_empty()), "at {axes:?}, nothing drew");
        assert!(all(&font, axes) == first, "at {axes:?}, a glyph drawn again drew otherwise");
    }
    assert!(all(&font, &bold) == all(&fresh, &bold), "a location drew outlines not its own");
}

// A pen that lets its thread go and draws again from inside its own call, at a location, glyf's and CFF2's: it draws
// what a pen calling nothing does, and so does each draw inside it.
#[test]
fn a_pen_drawing_again_from_inside_its_call_draws_the_same() {
    struct Again<'f> {
        font: &'f Font,
        glyph: GlyphId,
        at: &'f Location,
        drawn: Record,
        inside: Vec<Vec<String>>,
    }
    impl OutlinePen for Again<'_> {
        fn move_to(&mut self, x: f32, y: f32) { self.drawn.move_to(x, y) }
        fn line_to(&mut self, x: f32, y: f32) { self.drawn.line_to(x, y) }
        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) { self.drawn.quad_to(cx, cy, x, y) }
        fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
            self.drawn.curve_to(a, b, c, d, x, y)
        }
        fn close(&mut self) {
            self.drawn.close();
            daegun::release_thread();
            let mut inside = Record::default();
            self.font.outline(self.glyph, self.at, &mut inside).expect("a draw from inside");
            self.inside.push(inside.0);
        }
    }
    for (path, c) in [("inter/InterVariable.ttf", 'B'), ("source-han-sans/SourceHanSansJP-VF.otf", '永')] {
        let font = Font::from_vec(std::fs::read(format!("{}/{path}", crate::FONTS)).expect(path), 0).expect(path);
        let bold = located(&font, &[("wght", 700.0)]);
        let glyph = font.glyph_id(c).expect("a glyph");
        let mut plain = Record::default();
        font.outline(glyph, &bold, &mut plain).expect(path);
        let mut again = Again { font: &font, glyph, at: &bold, drawn: Record::default(), inside: Vec::new() };
        font.outline(glyph, &bold, &mut again).expect(path);
        assert_eq!(again.drawn.0, plain.0, "{path}");
        assert!(!again.inside.is_empty() && again.inside.iter().all(|i| *i == plain.0), "{path}");
    }
}

// A chain of 60,000 composites, each shifting the next one unit, over a triangle: drawn whole, its shifts summed.
#[test]
fn composites_nest_as_deep_as_their_glyphs_go() {
    let mut glyphs: Vec<Vec<u8>> = (1..=60_000u16).map(|next| composite(&[(0x0002, next, [1, 0], &[])])).collect();
    glyphs.push(simple(&TRIANGLE, &[2]));
    let (glyf, loca) = font(&glyphs);
    assert_eq!(draw(&glyf, &loca, 0).expect("draws"), ["M60000,0", "L60100,0", "L60000,100", "Z"]);
}
