use daegun::OutlinePen;
use daegun::daecore::daetype::outline::{outline_cff_glyph_with, CffOutlines, Path};

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

fn draw(glyf: &[u8], loca: &[usize], gid: u16) -> Result<Vec<String>, String> {
    let mut pen = Record::default();
    daegun::outline_glyf_bytes(glyf, loca, gid, &mut pen).map(|_| pen.0)
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

// Four levels of sixteen copies over one leaf: guards against a cycle, a wide tree and a heavy leaf.
#[test]
fn the_glyf_budgets_hold() {
    let records = |gid: u16| composite(&vec![(0x0002u16, gid, [0i16, 0], &[][..]); 16]);
    let (glyf, loca) = font(&[records(0)]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("nesting too deep"));

    let (glyf, loca) = font(&[records(1), records(2), records(3), records(4), simple(&TRIANGLE, &[2])]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("work budget"));

    let many: Vec<(i16, i16)> = (0..20_000).map(|i| (i as i16, 0)).collect();
    let (glyf, loca) = font(&[records(1), records(2), simple(&many, &[19_999])]);
    assert!(draw(&glyf, &loca, 0).unwrap_err().contains("point budget"));

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
    [vec![1, 0, 4, 2], name, index(&[&top]), empty.clone(), empty, charset.to_vec(), charstrings, private, subrs].concat()
}

#[test]
fn a_seac_draws_its_base_and_accent() {
    let seac_after_hint = [139u8, 149, 1, 189, 247, 92, 204, 247, 85, 14];
    let seac_in_subroutine = [139u8, 149, 1, 189, 247, 92, 204, 247, 85, 32, 10];
    for program in [&seac_after_hint[..], &seac_in_subroutine[..]] {
        let cff = seac_cff(program);
        let outlines = CffOutlines::parse(&cff).expect("parses");
        let mut pen = Record::default();
        outline_cff_glyph_with(&outlines, &cff, 3, &mut pen).expect("draws");
        assert_eq!(pen.0, ["M0,0", "L100,0", "L100,100", "Z", "M50,200", "L60,200", "L60,210", "Z"]);
    }
}

// STIX Two Math's Ograve switches hints at the grave's second point. The mask must name it as the
// collected outline numbers it, 27, not as the interpreter counted moves and closing lines, 29.
#[test]
fn a_hint_mask_names_the_point_it_applies_from() {
    let stix = daegun::Font::from_bytes(&std::fs::read(format!("{}/stix-two-math/STIX2Math.otf", crate::FONTS)).unwrap()).unwrap();
    let at = |gid: u16| stix.cff_hints(gid).unwrap().masks.iter().map(|m| m.0).collect::<Vec<_>>();
    assert_eq!(at(145), [0, 27]);
    assert_eq!(at(146), [0, 24, 26]);
}
