use daegun::Font;

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

const GARAMOND: &str = "eb-garamond/EBGaramond.ttf";
const STIX: &str = "stix-two-math/STIX2Math.otf";
const TTC: &str = "test-fixtures/EBGaramond-InterVariable.ttc";
const IRIANIS: &str = "Irianisadfstd/IrianisadfstdRegular.ttc";

#[test]
fn cmap_queries_agree_with_glyph_id() {
    let f = font(GARAMOND);
    for c in ['A', 'z', '0', 'ä'] {
        let cp = c as u32;
        assert_eq!(
            f.has_glyph(cp),
            f.glyph_id(cp).is_some(),
            "has_glyph and glyph_id disagree about {c:?}",
        );
    }
    assert!(f.has_glyph('A' as u32), "EBGaramond should carry A");
    assert!(!f.has_glyph(0x000F_FFFD), "an unmapped codepoint reported as present");
}

// Irianis carries a Mac Roman format 6 beside its Unicode tables. Byte 0x80 there is Ä, so a C1
// control must stay unmapped rather than draw as an accented letter.
#[test]
fn a_mac_table_does_not_map_c1_controls() {
    let path = format!("{}/{}", crate::FONTS, IRIANIS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    let f = Font::from_ttc(&bytes, 0).expect("Irianis opens");
    for cp in 0x80..0xA0 {
        assert_eq!(f.glyph_id(cp), None, "U+{cp:04X} mapped through the Mac table");
    }
    assert_eq!(f.glyph_id('Ä' as u32), Some(134));
    assert!(f.coverage().iter().all(|&(cp, _)| !(0x80..0xA0).contains(&cp)), "coverage lists a C1 control");
}

#[test]
fn glyph_names_match_glyph_name_per_gid() {
    let f = font(GARAMOND);
    let all = f.glyph_names();
    assert_eq!(all.len(), f.num_glyphs() as usize, "glyph_names is not one entry per glyph");

    let named = all.iter().filter(|n| n.is_some()).count();
    assert!(named > 0, "EBGaramond carries a post table but no name came back");

    for gid in [0u16, 1, 2, 40] {
        assert_eq!(
            f.glyph_name(gid),
            all[gid as usize],
            "glyph_name({gid}) disagrees with glyph_names()[{gid}]",
        );
    }
    assert_eq!(f.glyph_name(0).as_deref(), Some(".notdef"), "gid 0 should be .notdef");
}

#[test]
fn a_collection_reports_its_faces_and_opens_each() {
    let path = format!("{}/{}", crate::FONTS, TTC);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));

    let count = Font::ttc_font_count(&bytes);
    assert_eq!(count, 2, "the bundled collection holds EBGaramond and InterVariable");

    let names: Vec<u16> = (0..count)
        .map(|i| Font::from_ttc(&bytes, i).expect("face opens").num_glyphs())
        .collect();
    assert!(names[0] > 0 && names[1] > 0, "a face in the collection has no glyphs");
    assert_ne!(names[0], names[1], "both faces reported the same glyph count, so the index is ignored");

    assert!(Font::from_ttc(&bytes, count).is_err(), "an out-of-range face index was accepted");
    assert_eq!(Font::ttc_font_count(&[]), 0, "empty bytes reported as a collection");
    assert_eq!(Font::ttc_font_count(b"not a font at all"), 0, "junk reported as a collection");
}

#[test]
fn vertical_origins_are_reported_for_a_cff_face() {
    let f = font(STIX);
    let gid = f.glyph_id('A' as u32).expect("A");
    let per_glyph = f.vertical_origin(gid, &[]);
    let default = f.default_vertical_origin();

    match per_glyph {
        Some(v) => assert!(v.abs() < 10_000, "vertical origin {v} is outside any plausible em"),
        None => assert!(default.abs() < 10_000, "default vertical origin {default} is implausible"),
    }

    let g = font(GARAMOND);
    assert_eq!(g.default_vertical_origin(), 0, "a glyf font should report a zero default origin");
}

#[test]
fn math_tables_answer_for_a_math_font() {
    let f = font(STIX);

    let overlap = f.math_min_connector_overlap().expect("STIX2Math carries MATH");
    assert!(overlap >= 0.0, "a negative minimum connector overlap: {overlap}");

    let gid = f.glyph_id(0x222B).expect("integral sign");
    let construction = f.math_glyph_variants(gid, true);
    assert!(
        construction.is_some(),
        "the integral sign carries no vertical construction, so MATH variants are not being read",
    );

    let corners = [
        daegun::MathKernCorner::TopRight,
        daegun::MathKernCorner::TopLeft,
        daegun::MathKernCorner::BottomRight,
        daegun::MathKernCorner::BottomLeft,
    ];
    for corner in corners {
        let k = f.math_kern(gid, corner, 500.0);
        assert!(k.is_some_and(f64::is_finite), "math_kern returned no finite value for {corner:?}");
    }
}

#[test]
fn metadata_tables_parse_or_declare_themselves_absent() {
    let f = font(GARAMOND);

    if let Some(info) = f.base_info("latn", false, &[]) {
        assert!(
            info.default_baseline_tag.is_none_or(|t| t.len() == 4)
                && info.baseline_coords.iter().all(|(tag, _)| tag.len() == 4),
            "a BASE baseline tag is not four bytes",
        );
    }
    if let Some(stat) = f.stat_info() {
        assert!(
            stat.axes.iter().all(|a| a.tag.len() == 4),
            "a STAT axis tag is not four bytes",
        );
    }
}

#[test]
fn justification_reports_what_the_font_offers() {
    let f = font(GARAMOND);

    assert!(f.justification_glyphs("latn").is_none_or(|g| g.iter().all(|&i| i < f.num_glyphs())));
    assert!(f.justification_priorities("latn", None).is_none_or(|p| !p.is_empty()));
    assert!(
        f.justification_extenders("latn").iter().all(|&g| g < f.num_glyphs()),
        "an extender glyph id is out of range for this font",
    );

    let opts = daegun::JustifyOptions { script_tag: "latn", lang_sys_tag: None, target_width: 6000.0, tolerance: 1.0 };
    let justified = f.justify("hello world", &[], false, &opts).expect("a Latin line justifies");
    let natural: f64 = f.shape("hello world", &[], false).expect("shapes").advances.iter().sum();
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

#[test]
fn shaping_with_options_matches_plain_shaping_at_defaults() {
    let f = font(GARAMOND);
    let plain = f.shape("Waffle", &[], false).expect("shapes");
    let with_opts = f
        .shape_with_options("Waffle", &[], false, &daegun::ShapeOptions::default())
        .expect("shapes with default options");
    assert_eq!(plain.glyphs, with_opts.glyphs, "default ShapeOptions changed the glyphs");
    assert_eq!(plain.advances, with_opts.advances, "default ShapeOptions changed the advances");
}

#[test]
fn layout_wraps_at_the_measure_it_is_given() {
    let f = font(GARAMOND);
    let text = "the quick brown fox jumps over the lazy dog";

    let wide = daegun::LayoutOptions { max_inline_size: 100_000.0, ..Default::default() };
    let narrow = daegun::LayoutOptions { max_inline_size: 4_000.0, ..Default::default() };

    let one = f.layout(text, &[], &wide).expect("lays out");
    let many = f.layout(text, &[], &narrow).expect("lays out");
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
    let plain = f.shape(text, &[], false).expect("shapes");

    let empty = daegun::JstfModLists {
        shrinkage_enable_gsub: None,
        shrinkage_disable_gsub: None,
        shrinkage_enable_gpos: None,
        shrinkage_disable_gpos: None,
        extension_enable_gsub: None,
        extension_disable_gsub: None,
        extension_enable_gpos: None,
        extension_disable_gpos: None,
    };

    for shrink in [true, false] {
        let got = f.shape_justified(text, &[], false, &empty, shrink).expect("shapes justified");
        assert_eq!(
            got.glyphs, plain.glyphs,
            "an empty mod list changed the glyphs at shrink = {shrink}",
        );
        assert_eq!(
            got.advances.len(), got.glyphs.len(),
            "the justified run's arrays disagree in length at shrink = {shrink}",
        );
    }

    let no_gpos = daegun::JstfModLists {
        shrinkage_disable_gpos: Some((0..64).collect()),
        shrinkage_enable_gsub: None,
        shrinkage_disable_gsub: None,
        shrinkage_enable_gpos: None,
        extension_enable_gsub: None,
        extension_disable_gsub: None,
        extension_enable_gpos: None,
        extension_disable_gpos: Some((0..64).collect()),
    };
    let unkerned = f.shape_justified(text, &[], false, &no_gpos, true).expect("shapes");
    assert_eq!(unkerned.glyphs, plain.glyphs, "disabling GPOS changed which glyphs were selected");
}
