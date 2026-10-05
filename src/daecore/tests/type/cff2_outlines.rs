use daegun::daecore::daetype::outline::OutlinePen;
use daegun::Font;

const SOURCE_SERIF: &str = "source-serif/SourceSerif4Variable-Roman.otf";
const WGHT_500: &[(&str, f64)] = &[("wght", 500.0)];

fn with_font<R>(rel: &str, body: impl FnOnce(&Font) -> R) -> R {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).expect("font opens");
    let font = Font::from_bytes(&bytes).expect("font parses");
    body(&font)
}

#[derive(Default)]
struct LineClosings {
    start: (f32, f32),
    last_line: Option<(f32, f32)>,
    hits: usize,
}

impl OutlinePen for LineClosings {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = (x, y);
        self.last_line = None;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.last_line = Some((x, y));
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.last_line = None;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.last_line = None;
    }
    fn close(&mut self) {
        if self.last_line == Some(self.start) {
            self.hits += 1;
        }
    }
}

struct FirstPoint(Option<(f32, f32)>);

impl OutlinePen for FirstPoint {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.0.is_none() {
            self.0 = Some((x, y));
        }
    }
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

// These charstrings walk back to where the contour began, and `close` draws that segment anyway.
// Emitting both renders correctly but serializes a path no other engine produces.
#[test]
fn contours_do_not_restate_the_closing_segment() {
    with_font(SOURCE_SERIF, |font| {
        let mut offenders = Vec::new();
        for gid in 0..font.num_glyphs() {
            let mut pen = LineClosings::default();
            if font.outline_glyph_instanced(gid, WGHT_500, &mut pen).is_none() {
                continue;
            }
            if pen.hits > 0 {
                offenders.push(gid);
            }
        }
        assert!(
            offenders.is_empty(),
            "{} glyphs restate the closing segment, first few {:?}",
            offenders.len(),
            &offenders[..offenders.len().min(8)]
        );
    });
}

// Charstring coordinates are relative, so rounding each blended delta to a whole unit accumulates
// error along the outline: at wght 500 this point would land on 6.0 instead of 5.82489.
#[test]
fn blend_keeps_sub_unit_precision() {
    with_font(SOURCE_SERIF, |font| {
        let gid = font.glyph_id('A' as u32).expect("font has A");
        let mut pen = FirstPoint(None);
        font.outline_glyph_instanced(gid, WGHT_500, &mut pen)
            .expect("A outlines");
        let (x, y) = pen.0.expect("A has a contour");
        assert!((x - 5.824_89).abs() < 0.001, "expected x 5.82489, got {x}");
        assert_eq!(y, 0.0);
    });
}
