use super::aat_subset::{base_with, carets_with, font as fixture, u16s, u32s};
use daegun::{GlyphId, Location, Tag};
use crate::located;

// Axes AAAA and BBBB from -1 to 1.
fn fvar() -> Vec<u8> {
    let fixed = |v: i32| u32s(&[(v << 16) as u32]);
    let mut fvar = u16s(&[1, 0, 16, 2, 2, 20, 0, 12]);
    for tag in [b"AAAA", b"BBBB"] {
        fvar.extend(tag);
        fvar.extend([fixed(-1), fixed(0), fixed(1)].concat());
        fvar.extend(u16s(&[0, 256]));
    }
    fvar
}

fn words(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A simple glyph of one rectangle from (x0, y0) to (x1, y1), its corners on the curve and stored as words.
fn rectangle(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<u8> {
    let mut g = words(&[1, x0, y0, x1, y1, 3, 0]);
    g.extend([1; 4]);
    g.extend(words(&[x0, x1 - x0, 0, x0 - x1, y0, 0, y1 - y0, 0]));
    g
}

// A composite of glyph `gid` at (dx, dy), its offset in words, `flags` besides, and `bounds` its stored box.
fn composite(gid: u16, dx: i16, dy: i16, flags: u16, bounds: [i16; 4]) -> Vec<u8> {
    let mut g = words(&[-1]);
    g.extend(words(&bounds));
    g.extend(u16s(&[flags | 0x0003, gid]));
    g.extend(words(&[dx, dy]));
    g
}

type Tuple = (i16, Vec<i16>, Vec<i16>);

// A gvar of each glyph's tuples in turn: an embedded peak on AAAA, then x and y deltas for every point and the four
// phantom points, as words.
fn gvar(glyphs: &[&[Tuple]]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offsets = vec![0u32];
    for tuples in glyphs {
        if !tuples.is_empty() {
            let headers = 4 + 8 * tuples.len();
            let mut serialized = Vec::new();
            let mut glyph = u16s(&[tuples.len() as u16, headers as u16]);
            for (peak, xs, ys) in tuples.iter() {
                let mut packed = Vec::new();
                for deltas in [xs, ys] {
                    packed.push(0x40 | (deltas.len() as u8 - 1));
                    packed.extend(words(deltas));
                }
                glyph.extend(u16s(&[packed.len() as u16, 0x8000]));
                glyph.extend(words(&[*peak, 0]));
                serialized.extend(packed);
            }
            glyph.extend(serialized);
            data.extend(glyph);
        }
        offsets.push(data.len() as u32);
    }
    let offsets_at = 20;
    let data_at = offsets_at + 4 * offsets.len();
    let mut t = u16s(&[1, 0, 2, 0]);
    t.extend(u32s(&[data_at as u32]));
    t.extend(u16s(&[glyphs.len() as u16, 1]));
    t.extend(u32s(&[data_at as u32]));
    t.extend(u32s(&offsets));
    t.extend(data);
    t
}

const HALF: i16 = 0x3FFF;

// glyf and its short loca for `glyphs`, each padded to 4 bytes.
fn glyf_loca(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in glyphs {
        loca.extend(u16s(&[(glyf.len() / 2) as u16]));
        glyf.extend(g);
        glyf.resize(glyf.len().next_multiple_of(4), 0);
    }
    loca.extend(u16s(&[(glyf.len() / 2) as u16]));
    (glyf, loca)
}

// carets.ttf's glyphs: 0 empty, advancing 30 more at AAAA 1; 1 and 2 rectangles whose right edge and top move 1 under
// a peak of 16383/16384, 2 drawn by 4; 3 advancing 50 more, drawn by 5 under USE_MY_METRICS.
fn font() -> daegun::Font {
    let glyphs = [
        Vec::new(),
        rectangle(100, 0, 1100, 700),
        rectangle(50, 1, 350, 400),
        rectangle(50, 0, 350, 700),
        composite(2, 0, 100, 0, [50, 101, 350, 500]),
        composite(3, 0, 0, 0x0200, [50, 0, 350, 700]),
        rectangle(50, 0, 350, 700),
    ];
    let (glyf, loca) = glyf_loca(&glyphs);
    let mut hmtx = words(&[400, 0, 400, 100, 400, 50, 400, 50, 900, 50]);
    hmtx.extend(words(&[50, 50]));
    let none = || vec![0; 8];
    let gvar = gvar(&[
        &[(0x4000, vec![0, 30, 0, 0], vec![0; 4])],
        &[(HALF, vec![0, 1, 1, 0, 0, 0, 0, 0], none())],
        &[(HALF, none(), vec![0, 0, 1, 1, 0, 0, 0, 0])],
        &[(0x4000, vec![0, 0, 0, 0, 0, 50, 0, 0], none())],
        &[(HALF, vec![0; 5], vec![1, 0, 0, 0, 0])],
        &[],
        &[],
    ]);
    carets_with(&[("fvar", fvar()), ("glyf", glyf), ("loca", loca), ("hmtx", hmtx), ("gvar", gvar)])
}

// Glyph 1's right edge lands on 1100.5 in single precision, which would round up, where the sum drawn in 16.16
// is just under it.
#[test]
fn a_varied_point_rounds_from_its_16_16_sum() {
    let f = font();
    assert_eq!(f.glyph_bounds(GlyphId(1), &located(&f, &[("AAAA", 8191.0 / 16384.0)])).map(|b| b.x_max), Some(1100.0));
}

// Composite 4 places its component's top, 400.49997, 100.49997 up, each moved by its delta rounded once as drawing
// moves them: 400 and 100, so its top is 500. Its box sets its vertical origin, centered from its top.
#[test]
fn a_composites_box_is_taken_from_its_points_each_moved_whole() {
    let f = font();
    assert_eq!(f.vertical_origin(GlyphId(4), &located(&f, &[("AAAA", 8191.0 / 16384.0)])), 800.0);
}

// Composite 5 has no variation data of its own but advances as glyph 3 does under USE_MY_METRICS, and the empty
// glyph 0 advances by its phantom point.
#[test]
fn a_composite_advances_as_the_component_whose_metrics_it_uses() {
    let f = font();
    assert_eq!(crate::advances(&f, &[0, 3, 5].map(GlyphId), &located(&f, &[("AAAA", 1.0)])), [430.0, 450.0, 450.0]);
}

// A cvt of 500 and 32,767, and a cvar adding 1,100 and 1 to them at BBBB 1, then 1 to the first under a peak of
// 16383/16384 on AAAA.
fn hinted() -> daegun::Font {
    let mut cvar = u16s(&[1, 0, 2, 24, 5, 0x8000, 0, 0x4000, 5, 0x8000, HALF as u16, 0]);
    for deltas in [[1100, 1], [1, 0]] {
        cvar.push(0x41);
        cvar.extend(words(&deltas));
    }
    carets_with(&[("fvar", fvar()), ("cvt ", words(&[500, 32767])), ("cvar", cvar)])
}

fn hinted_cvt() -> Vec<u8> {
    let f = hinted();
    let at = located(&f, &[("AAAA", 8191.0 / 16384.0), ("BBBB", 1.0)]);
    let instance = daegun::Font::from_bytes(&f.instance(&at).expect("an instance")).expect("the instance parses");
    instance.table(Tag::new(b"cvt ")).expect("a cvt").to_vec()
}

// The first entry's deltas sum in 16.16 to just under 1100.5, so it moves by 1100.
#[test]
fn a_cvt_entrys_deltas_are_summed_in_16_16_and_rounded_once() {
    assert_eq!(hinted_cvt()[..2], words(&[1600]));
}

#[test]
fn a_cvt_entry_wraps_past_its_16_bits() {
    assert_eq!(hinted_cvt()[2..], words(&[-32768]));
}


// font() with head's field at `at`, indexToLocFormat or glyphDataFormat, set to 2.
fn of_unknown_format(at: usize) -> daegun::Font {
    let f = font();
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.get_mut(&Tag::new(b"head")).expect("a head")[at..at + 2].copy_from_slice(&[0, 2]);
    daegun::Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses")
}

// glyf of an unknown format, glyphDataFormat or indexToLocFormat 2, holds no glyph, drawn as for shaping: hmtx
// advances, the ascender as origin, no box or subset glyf.
#[test]
fn a_glyf_of_an_unknown_format_holds_no_glyph() {
    for at in [50, 52] {
        let f = of_unknown_format(at);
        for axes in [&[][..], &[("AAAA", 1.0)]] {
            assert_eq!(
                crate::advances(&f, &[0, 1, 3, 5].map(GlyphId), &located(&f, axes)),
                [400.0, 400.0, 400.0, 900.0],
                "{at} {axes:?}"
            );
            assert_eq!(f.vertical_origin(GlyphId(1), &located(&f, axes)), 800.0, "{at} {axes:?}");
        }
        assert_eq!(f.glyph_bounds(GlyphId(1), &Location::default()), None, "{at}");
        let sub = daegun::Font::from_vec(f.subset_text("fi", &Location::default()).expect("a subset").font, 0)
            .expect("the subset parses");
        assert_eq!((sub.table(Tag::new(b"glyf")), sub.table(Tag::new(b"loca"))), (None, None), "{at}");
    }
}

// One glyph's variation data: `count` as written, each tuple's header (its data size as written, index and, if
// embedded, its peak), then each tuple's data.
fn var_data(count: u16, tuples: &[(u16, u16, [i16; 2], Vec<u8>)]) -> Vec<u8> {
    let headers: usize = tuples.iter().map(|(_, index, ..)| if index & 0x8000 != 0 { 8 } else { 4 }).sum();
    let mut g = u16s(&[count, 4 + headers as u16]);
    for (size, index, peak, _) in tuples {
        g.extend(u16s(&[*size, *index]));
        if index & 0x8000 != 0 {
            g.extend(words(peak));
        }
    }
    tuples.iter().for_each(|(.., data)| g.extend(data));
    g
}

// A gvar of each glyph's variation data, then `padding` bytes no glyph's data reaches.
fn gvar_of(glyphs: &[Vec<u8>], padding: usize) -> Vec<u8> {
    let (mut data, mut offsets) = (Vec::new(), vec![0u32]);
    for g in glyphs {
        data.extend(g);
        data.resize(data.len().next_multiple_of(2), 0);
        offsets.push(data.len() as u32);
    }
    let data_at = (20 + 4 * offsets.len()) as u32;
    let mut t = u16s(&[1, 0, 2, 0]);
    t.extend(u32s(&[data_at]));
    t.extend(u16s(&[glyphs.len() as u16, 1]));
    t.extend(u32s(&[data_at]));
    t.extend(u32s(&offsets));
    t.extend(data);
    t.resize(t.len() + padding, 0);
    t
}

// A tuple at AAAA 1 over a glyph of `n` outline points, all its points' deltas given: x then y, each a run of zeros
// but for one delta at `at` by `by`.
fn moving(n: u16, at: (u16, i8), by: (u16, i8)) -> (u16, u16, [i16; 2], Vec<u8>) {
    let run = |(i, v): (u16, i8)| -> Vec<u8> {
        let mut r = Vec::new();
        if i > 0 {
            r.push(0x80 | (i - 1) as u8);
        }
        r.extend([0, v as u8]);
        if i + 1 < n + 4 {
            r.push(0x80 | (n + 2 - i) as u8);
        }
        r
    };
    let data = [run(at), run(by)].concat();
    (data.len() as u16, 0x8000, [0x4000, 0], data)
}

// carets.ttf's glyphs replaced by `glyphs`, each advancing 400 from a bearing of 100 in its five long metrics and
// two bearings, varied by `var`.
fn varied(glyphs: &[Vec<u8>], var: &[Vec<u8>], padding: usize, extra: &[(&str, Vec<u8>)]) -> daegun::Font {
    let (glyf, loca) = glyf_loca(glyphs);
    let hmtx = words(&[[400, 100].repeat(5), vec![100, 100]].concat());
    let gvar = gvar_of(var, padding);
    let mut tables = vec![("fvar", fvar()), ("glyf", glyf), ("loca", loca), ("hmtx", hmtx), ("gvar", gvar)];
    tables.extend(extra.iter().cloned());
    carets_with(&tables)
}

// Rectangles 1 to 6 read past: data past the glyph's, deltas past their tuple, a second header past the data, 300 in
// 32-bit deltas, and moves of 10 after a missing shared peak and after shared points past their count.
fn read_past() -> daegun::Font {
    let rectangle = || rectangle(100, 0, 1100, 700);
    let glyphs = [vec![], rectangle(), rectangle(), rectangle(), rectangle(), rectangle(), rectangle()];
    let advance = moving(4, (5, 10), (0, 0));
    let (_, index, peak, data) = advance.clone();
    let third = var_data(2, &[moving(4, (5, -10), (0, 0))]);
    let mut longs = vec![0x84, 0xC0, 0, 0, 1, 0x2C, 0x81, 0x87];
    let mut sixth = u16s(&[0x8001, 12, data.len() as u16, 0x8000]);
    sixth.extend(words(&peak));
    sixth.extend([1, 1, 5, 6]);
    sixth.extend(&data);
    let var = [
        vec![],
        var_data(1, &[(20, index, peak, vec![0x87, 0x87])]),
        var_data(1, &[(9, index, peak, [vec![0x47], words(&[1, 2, 3, 4])].concat())]),
        third,
        var_data(1, &[(longs.len() as u16, index, peak, core::mem::take(&mut longs))]),
        var_data(2, &[(data.len() as u16, 5, peak, data.clone()), advance]),
        sixth,
    ];
    varied(&glyphs, &var, 0, &[])
}

// A simple glyph of `n` points on one contour, the first at (100, 0) and the rest on it.
fn many_points(n: usize) -> Vec<u8> {
    let mut g = words(&[1, 100, 0, 100, 0, (n - 1) as i16, 0]);
    g.push(1);
    let mut left = n - 1;
    while left > 0 {
        let run = left.min(256);
        g.extend([0x39, (run - 1) as u8]);
        left -= run;
    }
    g.extend(words(&[100, 0]));
    g
}

// A composite of `count` copies of glyph `gid` at the origin, the last under USE_MY_METRICS.
fn copies(gid: u16, count: usize) -> Vec<u8> {
    let mut g = words(&[-1, 0, 0, 0, 0]);
    for i in 0..count {
        let more = if i + 1 < count { 0x0020 } else { 0x0200 };
        g.extend(u16s(&[0x0002 | more, gid]));
        g.extend([0, 0]);
    }
    g
}

// Glyphs at AAAA 1: 1 of 4,095 tuples, 33.6 million of work; 2 with coordinates cut short, 3 drawn by 2 and 5 past
// 200,000 points, which fail to measure; 4, 16,384 copies of glyph 0. The gvar is padded so the font may do the work.
fn failing() -> daegun::Font {
    let mut cut = rectangle(100, 0, 1100, 700);
    cut.truncate(cut.len() - 8);
    let glyphs = [vec![], many_points(4094), cut, copies(2, 1), copies(0, 16_384), copies(6, 4), many_points(50_001)];
    let mut heavy = vec![moving_point(4094 + 1, 10)];
    heavy.extend((1..4095).map(|_| moving_point(4094 + 1, 0)));
    let zero = moving(0, (1, 30), (2, 30));
    let mut var = vec![var_data(1, &[zero]), var_data(4095, &heavy), var_data(1, &[moving(4, (5, 10), (0, 0))])];
    var.resize(7, vec![]);
    let mut vhea = u16s(&[1, 0x1000, 800, 0xFF38, 0]);
    vhea.resize(34, 0);
    vhea.extend(u16s(&[7]));
    let vmtx = words(&[1000, 800].repeat(7));
    varied(&glyphs, &var, 300_000, &[("vhea", vhea), ("vmtx", vmtx)])
}

// A tuple at AAAA 1 naming only point `at`, moving it `by` across.
fn moving_point(at: u16, by: i8) -> (u16, u16, [i16; 2], Vec<u8>) {
    let mut data = vec![1, 0x80];
    data.extend(u16s(&[at]));
    data.extend([0, by as u8, 0x80]);
    (data.len() as u16, 0xA000, [0x4000, 0], data)
}

// How many contours a glyph draws.
struct Moves(usize);

impl daegun::outline::OutlinePen for Moves {
    fn move_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

// Tuples read within their glyph's data: 1's and 2's running past it and 6's points past their count are passed over;
// 3's header and 5's shared tuple past it fail the glyph's data. All five stay as stored at 400; 4 moves 300 (32-bit).
#[test]
fn a_glyphs_tuples_read_within_its_own_data() {
    let f = read_past();
    let at = located(&f, &[("AAAA", 1.0)]);
    assert_eq!(crate::advances(&f, &[1, 2, 3, 4, 5, 6].map(GlyphId), &at), [400.0, 400.0, 400.0, 700.0, 400.0, 400.0]);
    let stored = Some(bounds(100.0, 0.0, 1100.0, 700.0));
    assert_eq!([1, 2, 3, 5, 6].map(|g| f.glyph_bounds(GlyphId(g), &at)), [stored; 5]);
}

// 2, 3 and 5 fail to measure, so their advances and vertical origins are their phantom points'. They fail to draw
// too: 5's 200,004 points run past an outline's 65,535 once its first component is drawn. 1, as spec-valid, draws.
#[test]
fn a_glyph_that_fails_to_draw_takes_its_metrics_from_its_phantom_points() {
    let f = failing();
    let at = located(&f, &[("AAAA", 1.0)]);
    let widths = crate::advances(&f, &[0, 1, 2, 3, 4, 5, 6].map(GlyphId), &at);
    assert_eq!(widths, [430.0, 410.0, 410.0, 410.0, 430.0, 400.0, 400.0]);
    assert_eq!(
        (0..7).map(|g| f.vertical_origin(GlyphId(g), &at)).collect::<Vec<_>>(),
        [830.0, 800.0, 1500.0, 1500.0, 830.0, 800.0, 800.0]
    );
    assert!(f.outline(GlyphId(1), &at, &mut Moves(0)).is_ok() && f.glyph_bounds(GlyphId(1), &at).is_some());
    for g in [2, 3, 5] {
        let mut pen = Moves(0);
        assert_eq!(
            (f.glyph_bounds(GlyphId(g), &at), f.outline(GlyphId(g), &at, &mut pen).is_err()),
            (None, true),
            "{g}"
        );
        assert_eq!(pen.0, usize::from(g == 5), "{g}");
    }
    assert!(f.glyph_bounds(GlyphId(6), &at).is_some());
}

// Odd glyphs 1 to 31, a point each, share one variation data over 16,382 axes: 4,094 tuples peaking on BBBB, which
// don't apply at AAAA 1 but are charged each axis, and one on AAAA of `size` bytes moving the point 10 across. Glyph
// 32 draws them all: it spends 48 and each one's tuples, 4,095 times 16,383 + size + 10, past GLYPH_WORK from a size
// of 20,467. Glyph 2 draws glyph 1 a thousand times. Each point's glyph holds a one-instruction program, so it hints.
fn shared_tuples(size: u16) -> daegun::Font {
    let axes = 16_382;
    let fixed = |v: i32| u32s(&[(v << 16) as u32]);
    let mut fvar = u16s(&[1, 0, 16, 2, axes, 20, 0, 12]);
    for i in 0..axes {
        fvar.extend(match i {
            0 => *b"AAAA",
            1 => *b"BBBB",
            _ => format!("{i:04x}").into_bytes().try_into().expect("four digits"),
        });
        fvar.extend([fixed(-1), fixed(0), fixed(1)].concat());
        fvar.extend(u16s(&[0, 256]));
    }
    let mut shared = vec![0u8; 4 * usize::from(axes)];
    shared[2..4].copy_from_slice(&u16s(&[0x4000]));
    shared[2 * usize::from(axes)..][..2].copy_from_slice(&u16s(&[0x4000]));
    let mut data = u16s(&[4_095, 4 + 4 * 4_095]);
    (0..4_094).for_each(|_| data.extend(u16s(&[0, 0])));
    data.extend(u16s(&[size, 0x2001]));
    data.extend([1, 0, 0, 0, 10, 0x80]);
    data.resize(data.len() + usize::from(size) - 6, 0);
    let offsets: Vec<u32> = (0..34).map(|g| if g % 2 == 0 && g > 0 { data.len() as u32 } else { 0 }).collect();
    let shared_at = 20 + 4 * offsets.len() as u32;
    let mut gvar = u16s(&[1, 0, axes, 2]);
    gvar.extend(u32s(&[shared_at]));
    gvar.extend(u16s(&[33, 1]));
    gvar.extend(u32s(&[shared_at + shared.len() as u32]));
    gvar.extend(u32s(&offsets));
    gvar.extend([shared, data].concat());
    let mut all = composite(1, 0, 0, 0x0020, [100, 0, 100, 0]);
    for g in (3..32).step_by(2) {
        all.extend(&composite(g, 0, 0, if g < 31 { 0x0020 } else { 0 }, [0; 4])[10..]);
    }
    let mut point = many_points(1);
    point.splice(12..14, [0, 1, 0x00]);
    let mut glyphs: Vec<Vec<u8>> = (0..32).map(|g| if g % 2 == 1 { point.clone() } else { vec![] }).collect();
    glyphs[2] = copies(1, 1_000);
    glyphs.push(all);
    let (glyf, loca) = glyf_loca(&glyphs);
    let hmtx = words(&[[400, 100].repeat(5), vec![100; 28]].concat());
    let mut maxp = fixture("test-fixtures/carets.ttf").table(Tag::new(b"maxp")).expect("maxp").to_vec();
    maxp[4..6].copy_from_slice(&u16s(&[33]));
    let tables = [("fvar", fvar), ("glyf", glyf), ("loca", loca), ("hmtx", hmtx), ("gvar", gvar), ("maxp", maxp)];
    carets_with(&tables)
}

// Glyph 32 drawn first, or after the glyphs it draws were drawn and their deltas kept, spends the same and draws or
// fails alike, with one error: just under GLYPH_WORK at a size of 20,466, just past it at 20,467.
#[test]
fn a_composite_draws_alike_whatever_was_drawn_before_it() {
    let draw = |f: &daegun::Font, g| f.outline(GlyphId(g), &located(f, &[("AAAA", 1.0)]), &mut Moves(0));
    for (size, draws) in [(20_466, true), (20_467, false)] {
        let (cold, warm) = (shared_tuples(size), shared_tuples(size));
        assert!((1..32).step_by(2).all(|g| draw(&warm, g).is_ok()), "{size}");
        let (cold, warm) = (draw(&cold, 32), draw(&warm, 32));
        assert_eq!((cold.is_ok(), cold), (draws, warm), "{size}");
    }
}

// Glyph 2 draws glyph 1, of 67 million units of deltas, a thousand times: drawn, measured, hinted and instanced, the
// deltas are worked out once a draw rather than past GLYPH_WORK 62 times over.
#[test]
fn a_glyph_a_composite_draws_many_times_works_its_deltas_out_once() {
    let f = shared_tuples(6);
    let at = located(&f, &[("AAAA", 1.0)]);
    assert!(f.outline(GlyphId(2), &at, &mut Moves(0)).is_ok());
    assert_eq!(f.glyph_bounds(GlyphId(2), &at), Some(bounds(110.0, 0.0, 110.0, 0.0)));
    assert!(matches!(f.hinted_outline(GlyphId(2), &at, 16.0, daegun::outline::HintMode::Font), Ok(Some(_))));
    let instance = daegun::Font::from_bytes(&f.instance(&at).expect("an instance")).expect("the instance parses");
    assert_eq!(instance.glyph_bounds(GlyphId(2), &Location::default()), Some(bounds(110.0, 0.0, 110.0, 0.0)));
}

// The largest glyph the spec allows, 65,531 points (65,535 with phantoms) under 4,095 tuples each moving point 0 and so
// its contour 1 across: its 537 million units of deltas draw, measure, hint and instance, every point at x 4,195.
#[test]
#[ignore = "works out 537 million units of deltas: run it with --release"]
fn the_largest_glyph_the_spec_allows_draws() {
    let mut glyph = many_points(65_531);
    glyph.splice(12..14, [0, 1, 0x00]);
    let mut var = vec![vec![], var_data(4_095, &vec![moving_point(0, 1); 4_095])];
    var.resize(7, vec![]);
    let f = varied(&[vec![], glyph, vec![], vec![], vec![], vec![], vec![]], &var, 0, &[]);
    let at = located(&f, &[("AAAA", 1.0)]);
    let moved = Some(bounds(4_195.0, 0.0, 4_195.0, 0.0));
    let mut pen = Moves(0);
    assert!(f.outline(GlyphId(1), &at, &mut pen).is_ok() && pen.0 == 1);
    assert_eq!(f.glyph_bounds(GlyphId(1), &at), moved);
    assert!(matches!(f.hinted_outline(GlyphId(1), &at, 16.0, daegun::outline::HintMode::Font), Ok(Some(_))));
    let instance = daegun::Font::from_bytes(&f.instance(&at).expect("an instance")).expect("the instance parses");
    assert_eq!(instance.glyph_bounds(GlyphId(1), &Location::default()), moved);
}

// A font without gvar whose glyph 1 is a line from (100, 300) to (1100, 300), with no area, and glyph 2 a rectangle
// whose stored box is wrong, 0 to 2000 both ways.
fn measured() -> daegun::Font {
    let mut line = words(&[1, 100, 300, 1100, 300, 1, 0]);
    line.extend([1, 1]);
    line.extend(words(&[100, 1000, 300, 0]));
    let mut stale = rectangle(100, 0, 1100, 700);
    stale[2..10].copy_from_slice(&words(&[0, 0, 2000, 2000]));
    let (glyf, loca) = glyf_loca(&[vec![], line, stale, vec![], vec![], vec![], vec![]]);
    let hmtx = words(&[[400, 100].repeat(5), vec![100, 100]].concat());
    carets_with(&[("fvar", fvar()), ("glyf", glyf), ("loca", loca), ("hmtx", hmtx)])
}

// A glyph is measured by its drawn points at every location, gvar or not: the line's points box it as its stored box
// does, so its vertical origin holds, and the rectangle's stale stored box gives way to its points', 850 at each.
#[test]
fn a_glyph_is_measured_by_its_points_at_every_location() {
    let f = measured();
    assert_eq!([1, 2].map(|g| f.vertical_origin(GlyphId(g), &Location::default())), [800.0, 850.0]);
    assert_eq!([1, 2].map(|g| f.vertical_origin(GlyphId(g), &located(&f, &[("AAAA", 1.0)]))), [800.0, 850.0]);
}

fn bounds(x_min: f64, y_min: f64, x_max: f64, y_max: f64) -> daegun::Bounds {
    daegun::Bounds { x_min, y_min, x_max, y_max }
}

// glyf's overlap flags: a simple glyph's on its first point, a composite's on any component or on any glyph it's
// built from. A composite inside itself stops at the nesting limit, unflagged.
#[test]
fn overlap_flags_come_from_a_glyph_and_what_it_is_built_from() {
    let mut flagged = rectangle(0, 0, 100, 100);
    flagged[14] |= 0x40;
    let mut second = composite(2, 0, 0, 0x0020, [0, 0, 100, 100]);
    second.extend(u16s(&[0x0400 | 0x0003, 2]));
    second.extend(words(&[0, 0]));
    let glyphs = [
        Vec::new(),
        flagged,
        rectangle(0, 0, 100, 100),
        composite(2, 0, 0, 0x0400, [0, 0, 100, 100]),
        composite(1, 0, 0, 0, [0, 0, 100, 100]),
        second,
        composite(6, 0, 0, 0, [0, 0, 100, 100]),
    ];
    let (glyf, loca) = glyf_loca(&glyphs);
    let f = carets_with(&[("glyf", glyf), ("loca", loca)]);
    let got: Vec<bool> = (0..=7).map(|g| f.has_overlap_flag(GlyphId(g))).collect();
    assert_eq!(got, [false, true, false, true, true, true, false, false]);
}

// With every simple glyph flagged, a glyph VARC draws still carries none, as VARC has no such flag; one whose record
// fails is drawn from glyf, and carries glyf's.
#[test]
fn a_glyph_varc_draws_carries_no_overlap_flag() {
    let source = fixture("varc/varc-hostile.ttf");
    let (mut glyf, loca) = (source.table(Tag::new(b"glyf")).expect("glyf").to_vec(), source.table(Tag::new(b"loca")));
    let word = |i: usize| loca.and_then(|l| l.get(2 * i..2 * i + 2)).map(|w| u16::from_be_bytes([w[0], w[1]]));
    let at = |i: usize| word(i).map(|w| usize::from(w) * 2);
    for gid in 0..source.glyph_count() as usize {
        let (start, end) = (at(gid).expect("loca"), at(gid + 1).expect("loca"));
        let n = glyf.get(start..end).filter(|g| g.len() >= 10).map_or(0, |g| i16::from_be_bytes([g[0], g[1]]));
        if n > 0 {
            let flags = start + 12 + 2 * n as usize;
            let len = u16::from_be_bytes([glyf[flags - 2], glyf[flags - 1]]);
            glyf[flags + usize::from(len)] |= 0x40;
        }
    }
    let f = base_with("varc/varc-hostile.ttf", &[("glyf", glyf)]);
    let flagged = |gid| f.has_overlap_flag(GlyphId(gid));
    assert!(!source.has_overlap_flag(GlyphId(1)), "the fixture flags its square");
    assert!(flagged(1), "a square glyf draws");
    assert!(!flagged(7), "a glyph VARC draws, its own glyf outline flagged");
    assert!(!flagged(31), "a glyph VARC draws past 65,535 points");
    assert!(flagged(42), "a glyph whose record doesn't read");
}

// fvar lists AAAA twice, so it has no one range and any value stands: -0.0 and 0.0 are one location, and hash alike.
#[test]
fn an_axis_listed_twice_hashes_minus_0_as_0() {
    use core::hash::{Hash, Hasher};
    let mut twice = fvar();
    twice[36..40].copy_from_slice(b"AAAA");
    let f = carets_with(&[("fvar", twice)]);
    let hash = |l: &Location| {
        let mut h = std::hash::DefaultHasher::new();
        l.hash(&mut h);
        h.finish()
    };
    let (minus, plus) = (located(&f, &[("AAAA", -0.0)]), located(&f, &[("AAAA", 0.0)]));
    assert_eq!(minus, plus);
    assert_eq!(hash(&minus), hash(&plus));
}
