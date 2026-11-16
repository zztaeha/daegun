use daegun::layout::Adjustment;
use daegun::{Font, GlyphId, Location, Orientation, Tag};
use daegun::shape::ShapeOptions;

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

const GARAMOND: &str = "eb-garamond/EBGaramond.ttf";
const STIX: &str = "stix-two-math/STIX2Math.otf";
const TTC: &str = "test-fixtures/EBGaramond-InterVariable.ttc";
const IRIANIS: &str = "Irianisadfstd/IrianisadfstdRegular.ttc";
const INTER: &str = "inter/InterVariable.ttf";

#[test]
fn cmap_queries_agree_with_glyph_id() {
    let f = font(GARAMOND);
    for c in ['A', 'z', '0', 'ä'] {
        assert_eq!(
            f.has_glyph(c),
            f.glyph_id(c).is_some(),
            "has_glyph and glyph_id disagree about {c:?}",
        );
    }
    assert!(f.has_glyph('A'), "EBGaramond should carry A");
    assert!(!f.has_glyph('\u{F_FFFD}'), "an unmapped codepoint reported as present");
}

// Irianis carries a Mac Roman format 6 beside its Unicode tables. Byte 0x80 there is Ä, so a C1
// control must stay unmapped rather than draw as an accented letter.
#[test]
fn a_mac_table_does_not_map_c1_controls() {
    let path = format!("{}/{}", crate::FONTS, IRIANIS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    let f = Font::from_collection(&bytes, 0).expect("Irianis opens");
    for c in '\u{80}'..'\u{A0}' {
        assert_eq!(f.glyph_id(c), None, "U+{:04X} mapped through the Mac table", u32::from(c));
    }
    assert_eq!(f.glyph_id('Ä'), Some(GlyphId(134)));
    assert!(f.coverage().all(|(c, _)| !('\u{80}'..'\u{A0}').contains(&c)), "coverage lists a C1 control");
}

#[test]
fn glyph_names_match_glyph_name_per_gid() {
    let f = font(GARAMOND);
    let all: Vec<_> = f.glyph_names().collect();
    assert_eq!(all.len(), f.glyph_count() as usize, "glyph_names is not one entry per glyph");

    let named = all.iter().filter(|n| n.is_some()).count();
    assert!(named > 0, "EBGaramond carries a post table but no name came back");

    for gid in [0u32, 1, 2, 40] {
        assert_eq!(
            f.glyph_name(GlyphId(gid)),
            all[gid as usize],
            "glyph_name({gid}) disagrees with glyph_names()[{gid}]",
        );
    }
    assert_eq!(f.glyph_name(GlyphId(0)), Some(".notdef"), "gid 0 should be .notdef");
}

#[test]
fn a_collection_reports_its_faces_and_opens_each() {
    let path = format!("{}/{}", crate::FONTS, TTC);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));

    let count = Font::face_count(&bytes);
    assert_eq!(count, 2, "the bundled collection holds EBGaramond and InterVariable");

    let names: Vec<u32> = (0..count)
        .map(|i| Font::from_collection(&bytes, i).expect("face opens").glyph_count())
        .collect();
    assert!(names[0] > 0 && names[1] > 0, "a face in the collection has no glyphs");
    assert_ne!(names[0], names[1], "both faces reported the same glyph count, so the index is ignored");

    assert!(Font::from_collection(&bytes, count).is_err(), "an out-of-range face index was accepted");
    assert_eq!(Font::face_count(&[]), 0, "empty bytes reported as a font");
    assert_eq!(Font::face_count(b"not a font at all"), 0, "junk reported as a font");
}

// A font file is one face, opened at index 0 by from_collection as from_bytes opens it, and refused past it.
#[test]
fn a_font_file_is_one_face() {
    let path = format!("{}/{}", crate::FONTS, INTER);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    assert_eq!(Font::face_count(&bytes), 1, "a font file isn't one face");
    let (face, file) = (Font::from_collection(&bytes, 0).expect("face 0"), Font::from_bytes(&bytes).expect("opens"));
    assert!(face.table_tags().eq(file.table_tags()), "face 0 holds other tables");
    assert!(face.table_tags().all(|t| face.table(t) == file.table(t)), "face 0's tables differ");
    let past = Font::from_collection(&bytes, 1).map(|_| ());
    assert!(past.is_err_and(|e| e.kind() == daegun::ErrorKind::InvalidArgument), "index 1 of a font file opened");
}

// A font read where it lies, or kept as given, answers as its copy does, a font file at index 0 and each face of a
// collection; an index past a collection's faces, or past a font file's one, is refused.
#[test]
fn a_font_read_in_place_answers_as_its_copy() {
    use daegun::ErrorKind;
    let read = |rel: &str| {
        let path = format!("{}/{}", crate::FONTS, rel);
        std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
    };
    let same = |copy: &Font, place: Font, what: &str| {
        assert!(copy.table_tags().eq(place.table_tags()), "{what}: the tables differ");
        for tag in copy.table_tags() {
            assert_eq!(copy.table(tag), place.table(tag), "{what}: {tag} differs");
        }
        let shape = |f: &Font| {
            let run = f.shape("Hamburgefonstiv ffi", &Location::default(), &ShapeOptions::default());
            (run.glyphs.clone(), run.advances.clone())
        };
        assert_eq!(shape(copy), shape(&place), "{what}: shaping differs");
    };
    let ttc: &'static [u8] = read(TTC).leak();
    for i in 0..2 {
        let copy = Font::from_collection(ttc, i).expect("face opens");
        same(&copy, Font::from_static(ttc, i).expect("face opens in place"), &format!("face {i}"));
        same(&copy, Font::from_vec(ttc.to_vec(), i).expect("face opens kept"), &format!("face {i}, kept"));
    }
    let file: &'static [u8] = read(GARAMOND).leak();
    same(&Font::from_bytes(file).expect("opens"), Font::from_static(file, 0).expect("opens in place"), "a file");
    same(&Font::from_bytes(file).expect("opens"), Font::from_vec(file.to_vec(), 0).expect("opens kept"), "a file kept");

    let refused = |opened: daegun::Result<Font>| opened.err().map(|e| e.kind());
    assert_eq!(refused(Font::from_static(ttc, 2)), Some(ErrorKind::InvalidArgument), "a third face of two opened");
    assert_eq!(refused(Font::from_static(file, 1)), Some(ErrorKind::InvalidArgument), "a font file's second face");
    assert_eq!(refused(Font::from_vec(ttc.to_vec(), 2)), Some(ErrorKind::InvalidArgument), "a third face kept");
    assert_eq!(refused(Font::from_vec(file.to_vec(), 1)), Some(ErrorKind::InvalidArgument), "a file's second, kept");
    assert!(Font::from_static(b"not a font", 0).is_err(), "junk opened");
}

// A collection stating two faces that ends after the first's offset: face_count counts one, so every open refuses
// index 1 as past the faces rather than as a malformed file.
#[test]
fn a_collection_index_past_the_offsets_it_holds_is_an_invalid_argument() {
    use daegun::ErrorKind;
    let ttc: &'static [u8] = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\x0c";
    assert_eq!(Font::face_count(ttc), 1);
    let refused = |opened: daegun::Result<Font>| opened.err().map(|e| e.kind());
    assert_eq!(refused(Font::from_collection(ttc, 1)), Some(ErrorKind::InvalidArgument), "copied");
    assert_eq!(refused(Font::from_static(ttc, 1)), Some(ErrorKind::InvalidArgument), "read in place");
    assert_eq!(refused(Font::from_vec(ttc.to_vec(), 1)), Some(ErrorKind::InvalidArgument), "kept");
}

#[test]
fn vertical_origins_are_reported_for_a_cff_face() {
    let f = font(STIX);
    let gid = f.glyph_id('A').expect("A");
    let (per_glyph, default) = (f.vertical_origin(gid, &Location::default()), f.default_vertical_origin());
    assert!(per_glyph.abs() < 10_000.0, "vertical origin {per_glyph} is outside any plausible em");
    assert!(default.abs() < 10_000.0, "default vertical origin {default} is implausible");

    let g = font(GARAMOND);
    assert_eq!(g.default_vertical_origin(), 0.0, "a glyf font should report a zero default origin");
}

#[test]
fn math_tables_answer_for_a_math_font() {
    let f = font(STIX);

    let overlap = f.math_min_connector_overlap().expect("STIX2Math carries MATH");
    assert!(overlap >= 0.0, "a negative minimum connector overlap: {overlap}");

    let gid = f.glyph_id('\u{222B}').expect("integral sign");
    let construction = f.math_glyph_variants(gid, &Location::default(), Orientation::Vertical);
    assert!(
        construction.is_some(),
        "the integral sign carries no vertical construction, so MATH variants are not being read",
    );

    let corners = [
        daegun::math::MathKernCorner::TopRight,
        daegun::math::MathKernCorner::TopLeft,
        daegun::math::MathKernCorner::BottomRight,
        daegun::math::MathKernCorner::BottomLeft,
    ];
    for corner in corners {
        let k = f.math_kern(gid, &Location::default(), corner, 500.0);
        assert!(k.is_ok_and(f64::is_finite), "math_kern returned no finite value for {corner:?}");
    }
}

#[test]
fn metadata_tables_parse_or_declare_themselves_absent() {
    let f = font(GARAMOND);

    let printable = |t: &Tag| t.to_bytes().iter().all(|b| b.is_ascii_graphic() || *b == b' ');
    if let Some(info) = f.base_script(Tag::new(b"latn"), &Location::default(), Orientation::Horizontal) {
        assert!(
            info.default_baseline.as_ref().is_none_or(printable) && info.baselines.keys().all(printable),
            "a BASE baseline tag is not printable",
        );
    }
    if let Some(stat) = f.stat(&Location::default()) {
        assert!(stat.axes.iter().all(|a| printable(&a.tag)), "a STAT axis tag is not printable");
    }
}

#[test]
fn justification_reports_what_the_font_offers() {
    let f = font(GARAMOND);

    assert!(
        f.justification_glyphs(Tag::new(b"latn")).is_none_or(|g| g.iter().all(|&i| i.0 < f.glyph_count())),
        "a justification glyph id is out of range for this font",
    );
    assert!(f.justification_priorities(Tag::new(b"latn"), None).is_none_or(|p| !p.is_empty()));

    let opts = daegun::layout::JustifyOptions::new(Tag::new(b"latn"), 6000.0).with_tolerance(1.0);
    let justified = f.justify("hello world", &Location::default(), Orientation::Horizontal, &opts);
    let natural: f64 = f.shape("hello world", &Location::default(), &ShapeOptions::default()).advances.iter().sum();
    assert!(
        justified.run.advances.len() == justified.run.glyphs.len(),
        "the justified run's arrays disagree in length",
    );
    let total: f64 = justified.run.advances.iter().sum();
    assert!(
        total >= natural - 1.0,
        "justifying to a wider measure made the line narrower: {total} against {natural}",
    );
}

// 30,000 priority levels naming one table of no lookups: every one is given, past 64 too, and a line justified past
// its reach is shaped once for them all, as they shape alike.
#[test]
fn every_jstf_level_is_given_and_alike_ones_shape_once() {
    use super::aat_subset::{carets_with, u16s};
    let levels = 30_000u16;
    let jstf = [
        u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[12, 0, 6, 0, levels]), u16s(&vec![2 + 2 * levels; levels.into()]),
        vec![0; 20],
    ]
    .concat();
    let f = carets_with(&[("JSTF", jstf)]);
    let given = f.justification_priorities(Tag::new(b"latn"), None).expect("its levels");
    assert_eq!(given.len(), 30_000);
    let opts = daegun::layout::JustifyOptions::new(Tag::new(b"latn"), 100_000.0);
    let line = f.justify("fil", &Location::default(), Orientation::Horizontal, &opts);
    assert_eq!((line.level, line.best_effort), (None, true));
}

#[test]
fn layout_wraps_at_the_measure_it_is_given() {
    let f = font(GARAMOND);
    let text = "the quick brown fox jumps over the lazy dog";

    let wide = daegun::layout::LayoutOptions::default().with_max_inline_size(100_000.0);
    let narrow = daegun::layout::LayoutOptions::default().with_max_inline_size(4_000.0);

    let one = f.layout(text, &Location::default(), &wide);
    let many = f.layout(text, &Location::default(), &narrow);
    assert_eq!(one.lines.len(), 1, "a very wide measure should not wrap");
    assert!(
        many.lines.len() > one.lines.len(),
        "a narrow measure produced {} lines, no more than the wide one's {}",
        many.lines.len(),
        one.lines.len(),
    );
}

#[test]
fn shape_justified_with_empty_lists_matches_plain_shaping() {
    let f = font(GARAMOND);
    let text = "waffle iron";
    let plain = f.shape(text, &Location::default(), &ShapeOptions::default());

    let empty = daegun::layout::JstfModLists::default();

    for shrink in [Adjustment::Shrink, Adjustment::Extend] {
        let got = f.shape_justified(text, &Location::default(), Orientation::Horizontal, &empty, shrink);
        assert_eq!(
            got.glyphs, plain.glyphs,
            "an empty mod list changed the glyphs at {shrink:?}",
        );
        assert_eq!(
            got.advances.len(), got.glyphs.len(),
            "the justified run's arrays disagree in length at {shrink:?}",
        );
    }

    let mut no_gpos = daegun::layout::JstfModLists::default();
    no_gpos.shrinkage_disable_gpos = Some((0..64).collect());
    no_gpos.extension_disable_gpos = Some((0..64).collect());
    let unkerned = f.shape_justified(text, &Location::default(), Orientation::Horizontal, &no_gpos, Adjustment::Shrink);
    assert_eq!(unkerned.glyphs, plain.glyphs, "disabling GPOS changed which glyphs were selected");
}

// Locations are equal, and hash alike, when they were asked for the same values, whichever font made them, so a map
// can key on them; a location at Inter's default weight isn't the default, as another font may vary it.
#[test]
fn locations_key_a_map_by_what_they_were_asked() {
    use std::collections::HashSet;
    use std::hash::{BuildHasher, RandomState};
    let (a, b) = (font(INTER), font(INTER));
    let wght = |f: &Font, v: f64| f.location(&[(Tag::new(b"wght"), v)]);
    let hash = RandomState::new();
    assert_eq!(wght(&a, 700.0), wght(&b, 700.0), "two fonts' wght 700 differ");
    assert_eq!(hash.hash_one(wght(&a, 700.0)), hash.hash_one(wght(&b, 700.0)), "equal locations hashed apart");
    assert_ne!(wght(&a, 400.0), Location::default(), "a weight asked for is the default location");
    assert_eq!(a.location(&[]), Location::default(), "no values asked for is not the default");
    let all = [wght(&a, 700.0), wght(&b, 700.0), wght(&a, 300.0), Location::default(), wght(&a, 400.0)];
    let keys: HashSet<Location> = all.into_iter().collect();
    assert_eq!(keys.len(), 4, "a set of locations kept {} keys, not 4", keys.len());
}

// The common traits answer as std's do: PDF flags combine, test empty and print as their number; a tag is its four
// bytes; a glyph id narrows to 16 bits only where it fits.
#[test]
fn the_common_traits_answer_as_std_does() {
    use daegun::metrics::PdfFlags;
    let both = PdfFlags::SERIF | PdfFlags::ITALIC;
    assert_eq!(both & PdfFlags::ITALIC, PdfFlags::ITALIC, "& kept the wrong flags");
    assert!((both & PdfFlags::SYMBOLIC).is_empty() && PdfFlags::empty().is_empty() && !both.is_empty());
    assert_eq!((format!("{both:x}"), format!("{both:#b}")), ("42".into(), "0b1000010".into()), "the flags printed");
    assert_eq!(Tag::from(*b"wght"), Tag::new(b"wght"), "a tag from its bytes differs");
    assert_eq!(u16::try_from(GlyphId(65_535)), Ok(65_535), "the last 16-bit glyph didn't narrow");
    assert!(u16::try_from(GlyphId(65_536)).is_err(), "a glyph past 16 bits narrowed");
}

// A pen or visitor lent by `&mut`, or as a trait object, draws and walks as the one it lends.
#[test]
fn pens_and_visitors_lend_as_std_writers_do() {
    use daegun::color::{PaintGraph, PaintVisitor};
    use daegun::outline::{OutlinePen, Path};

    fn draw_with(f: &Font, glyph: GlyphId, mut pen: impl OutlinePen) {
        f.outline(glyph, &Location::default(), &mut pen).expect("the glyph draws");
    }
    let f = font(INTER);
    let glyph = f.glyph_id('g').expect("Inter maps g");
    let (mut lent, mut own) = (Path::default(), Path::default());
    draw_with(&f, glyph, &mut lent);
    f.outline(glyph, &Location::default(), &mut own).expect("the glyph draws");
    assert_eq!(lent, own, "a lent pen drew another outline");

    let f = font("test-fixtures/colr-clip.ttf");
    let glyph = f.glyph_id('N').expect("the fixture maps N");
    let (mut by_dyn, mut by_ref) = (PaintGraph::new(), PaintGraph::new());
    let visitor: &mut dyn PaintVisitor = &mut by_dyn;
    assert!(f.visit_paint_graph(glyph, &Location::default(), 0, visitor), "N has no paint graph");
    assert!(f.visit_paint_graph(glyph, &Location::default(), 0, &mut &mut by_ref), "N has no paint graph");
    let built = f.paint_graph(glyph, &Location::default(), 0).expect("N has a paint graph");
    assert!(by_dyn == built && by_ref == built, "a lent visitor walked another graph");
}
