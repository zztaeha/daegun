use super::*;
use alloc::string::String;

// 256 copies of a 20,000-point leaf pass the 65,535 points an outline holds, which fails the glyph.
#[test]
fn a_glyph_past_the_points_an_outline_holds_fails() {
    let mut leaf = alloc::vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0x4E, 0x1F, 0, 0];
    leaf.extend(core::iter::repeat_n(0x31, 20_000));
    let copies = |gid: u8| {
        let mut g = alloc::vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        (0..16).for_each(|i| g.extend([0, if i < 15 { 0x22 } else { 0x02 }, 0, gid, 0, 0]));
        g
    };
    let glyphs = [copies(1), copies(2), leaf];
    let mut loca = alloc::vec![0];
    glyphs.iter().for_each(|g| loca.push(loca.last().unwrap() + g.len()));
    let glyf = glyphs.concat();
    let mut scratch = GlyphCoords::default();
    let mut pen = super::super::PointBounds::default();
    let drawn = outline_glyf_glyph_varied(&glyf, false, (&loca).into(), 0, &mut scratch, &Stored, &mut 0, &mut pen);
    assert!(drawn.unwrap_err().message().contains("more points"));
}

// The instancer boxes a glyph of 65,536 points, each x step -32,768, so its last point lies on i32::MIN; a delta of
// -1 there holds the box at the rail.
#[test]
fn a_point_moved_past_the_rail_holds_the_box_at_it() {
    let mut glyph = alloc::vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0];
    glyph.extend(core::iter::repeat_n(0x21, 65_536));
    glyph.extend([0x80, 0].repeat(65_536));
    let mut coords = GlyphCoords::default();
    assert!(extract_coords_into(&glyph, 1, &mut coords), "the glyph decodes");
    let mut moves = alloc::vec![(0, 0); 65_536];
    moves[65_535] = (-1, 0);
    assert_eq!(points_box(&coords, Some(&moves)), Some([i32::MIN, 0, -32_768, 0]));
}

// A point as x, y and its flag.
type Flagged = (i16, i16, u8);

// A one-contour GLYF glyph of `points`, coordinates in words.
fn simple(points: &[Flagged]) -> Vec<u8> {
    let words = |w: &mut Vec<u8>, v: i16| w.extend(v.to_be_bytes());
    let mut g = Vec::new();
    [1, 0, 0, 0, 0, points.len() as i16 - 1, 0].into_iter().for_each(|v| words(&mut g, v));
    g.extend(points.iter().map(|p| p.2));
    let (mut x, mut y) = (0, 0);
    points.iter().for_each(|p| { words(&mut g, p.0 - x); x = p.0 });
    points.iter().for_each(|p| { words(&mut g, p.1 - y); y = p.1 });
    g
}

#[derive(Default)]
struct Drawn(String);

impl OutlinePen for Drawn {
    fn move_to(&mut self, x: f32, y: f32) { self.0 += &alloc::format!("M{x},{y} ") }
    fn line_to(&mut self, x: f32, y: f32) { self.0 += &alloc::format!("L{x},{y} ") }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) { self.0 += &alloc::format!("Q{a},{b},{x},{y} ") }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.0 += &alloc::format!("C{a},{b},{c},{d},{x},{y} ")
    }
    fn close(&mut self) { self.0 += "Z" }
}

// GLYF's cubic rule: cubic pairs make cubics, a point on the curve implied between pairs, a run through point 0 whole;
// mixed or odd runs draw as quadratic, a cubic bit on the curve means nothing, and each closes by `close` alone.
#[test]
fn cubic_contours_draw_by_the_drafts_rule() {
    const ON: u8 = 1;
    const C: u8 = FLAG_CUBIC;
    let cases: [(&[Flagged], &str); 15] = [
        (&[(0, 0, C), (100, 0, C), (100, 100, C), (0, 100, C)], "M0,50 C0,0,100,0,100,50 C100,100,0,100,0,50 Z"),
        (&[(0, 0, C), (100, 100, C)], "M50,50 C0,0,100,100,50,50 Z"),
        (&[(0, 0, ON), (50, 100, C), (100, 0, ON)], "M0,0 Q50,100,100,0 Z"),
        (&[(0, 0, ON), (0, 100, C), (100, 100, C), (200, 100, 0), (200, 0, ON)],
         "M0,0 Q0,100,50,100 Q100,100,150,100 Q200,100,200,0 Z"),
        (&[(0, 0, ON), (0, 100, C), (100, 100, C), (200, 100, C), (200, 0, ON)],
         "M0,0 Q0,100,50,100 Q100,100,150,100 Q200,100,200,0 Z"),
        (&[(0, 0, ON), (0, 100, C), (100, 150, C), (200, 150, C), (300, 100, C), (300, 0, ON)],
         "M0,0 C0,100,100,150,150,150 C200,150,300,100,300,0 Z"),
        (&[(0, 100, C), (100, 100, C), (100, 0, ON), (0, 0, ON)], "M0,0 C0,100,100,100,100,0 Z"),
        (&[(0, 100, C), (100, 100, C), (100, 0, ON), (0, 0, 0)], "M0,50 Q0,100,50,100 Q100,100,100,0 Q0,0,0,50 Z"),
        (&[(0, 0, 0), (100, 0, ON), (100, 100, C), (0, 100, C)], "M0,50 Q0,0,100,0 Q100,100,50,100 Q0,100,0,50 Z"),
        (&[(0, 0, ON | C), (50, 100, C), (100, 100, C), (100, 0, ON | C)], "M0,0 C50,100,100,100,100,0 Z"),
        (&[(0, 0, C), (100, 0, C), (50, 100, C)], "M25,50 Q0,0,50,0 Q100,0,75,50 Q50,100,25,50 Z"),
        (&[(50, 50, C)], "M50,50 Q50,50,50,50 Z"),
        (&[(0, 100, C), (100, 100, C), (100, 0, ON), (0, 0, ON | C)], "M0,0 C0,100,100,100,100,0 Z"),
        (&[(0, 0, ON), (0, 100, C), (100, 100, C), (100, 0, ON), (200, -100, 0), (300, 0, ON)],
         "M0,0 C0,100,100,100,100,0 Q200,-100,300,0 Z"),
        (&[(0, 100, C), (0, 0, ON), (100, 0, C), (200, 0, C), (200, 100, C)],
         "M0,0 C100,0,200,0,200,50 C200,100,0,100,0,0 Z"),
    ];
    for (points, want) in cases {
        let glyf = simple(points);
        let mut drawn = Drawn::default();
        outline_glyf_bytes(&glyf, true, &[0, glyf.len()], 0, &mut drawn).expect("the glyph draws");
        assert_eq!(drawn.0, want);
    }
    let mixed = simple(&[(0, 0, 0), (100, 0, C), (100, 100, 0), (0, 100, C)]);
    for (extended, want) in [
        (true, "M0,50 Q0,0,50,0 Q100,0,100,50 Q100,100,50,100 Q0,100,0,50 Z"),
        (false, "M0,50 Q0,0,50,0 Q100,0,100,50 Q100,100,50,100 Q0,100,0,50 Z"),
    ] {
        let mut drawn = Drawn::default();
        outline_glyf_bytes(&mixed, extended, &[0, mixed.len()], 0, &mut drawn).expect("the glyph draws");
        assert_eq!(drawn.0, want, "GLYF {extended}");
    }
}

// A location whose deltas move nothing but take `cost` of the draw's work for each glyph, kept as a cache keeps them,
// each use spending it again.
struct Costly(usize, core::cell::RefCell<BTreeSet<u32>>);

impl Vary for Costly {
    fn deltas(&self, gid: u32, _: Shape<'_>, left: &mut usize) -> Result<Deltas, Error> {
        *left = left.checked_sub(self.0).ok_or(Error::limit("deltas past the draw's work"))?;
        self.1.borrow_mut().insert(gid);
        Ok(None)
    }

    fn known(&self, gid: u32, left: &mut usize) -> Option<Result<Deltas, Error>> {
        let kept = self.1.borrow().contains(&gid);
        kept.then(|| self.deltas(gid, Shape::Empty, left))
    }
}

// A composite of two components naming a 3-point glyph: its 2 glyphs' deltas take 1,000 each of the draw's work, the
// leaf's worked out once though named twice, beside the 3 visits, 2 contours and 6 points; deltas past it fail it.
#[test]
fn a_glyph_s_deltas_are_spent_from_its_draw_s_work() {
    let leaf = simple(&[(0, 0, 1), (100, 0, 1), (0, 100, 1)]);
    let header = [0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
    let composite = [&header[..], &[0, 0x22, 0, 1, 0, 0], &[0, 0x02, 0, 1, 0, 0]].concat();
    let glyf = [composite.as_slice(), &leaf].concat();
    let loca = [0, composite.len(), glyf.len()];
    let drawn = |cost: usize| {
        let (mut scratch, mut work, costly) = (GlyphCoords::default(), 0, Costly(cost, Default::default()));
        let paying = Paying::new(&costly, 0);
        let points = glyph_points_varied(
            &glyf, false, (&loca[..]).into(), 0, &mut scratch, &paying, Points::default(), &mut work,
        );
        (points.map(drop).map_err(|e| e.kind()), work)
    };
    assert_eq!(drawn(1_000), (Ok(()), 3 + 2 + 6 + 2_000));
    assert_eq!(drawn(GLYPH_WORK / 2).0, Err(crate::daecore::ErrorKind::LimitExceeded));
}
