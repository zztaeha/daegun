use crate::outline::OutlinePen;
use crate::{Font, GlyphId, Location};
use alloc::vec::Vec;

#[derive(Default)]
struct Starts(Vec<(f32, f32)>);

impl OutlinePen for Starts {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

fn starts(rel: &str, gid: u32) -> Vec<(f32, f32)> {
    let path = format!("{}/{}", super::FONTS, rel);
    let bytes = std::fs::read(&path).expect("font opens");
    let font = Font::from_bytes(&bytes).expect("font parses");
    let mut pen = Starts::default();
    font.outline(GlyphId(gid), &Location::default(), &mut pen).expect("glyph has an outline");
    pen.0
}

const MORX_ONE: &str = "aat/TestMORXOne.ttf";

// A closed contour fills identically whichever vertex it starts from, so no rasterized comparison
// can catch a wrong start. Only the path itself shows it, hence these.
#[test]
fn off_curve_first_point_starts_at_the_last_point() {
    // gid 9 is a single contour: point 0 is off-curve at (378, -89), the last point is on-curve at
    // (500, -89), and the first on-curve point is index 3 at (54, 357).
    assert_eq!(starts(MORX_ONE, 9), vec![(500.0, -89.0)]);
}

#[test]
fn on_curve_first_point_starts_there() {
    assert_eq!(starts(MORX_ONE, 0)[0], (62.0, 700.0));
}
