use super::*;
use crate::daecore::GlyphId;

fn outlines_and_reads(source: &Tables, cff: &[u8]) -> (Option<CffOutlines>, FaceReads) {
    let font = |tag: &str| source.get(tag).map(|t| t.as_slice());
    (CffOutlines::parse(cff).ok(), FaceReads::of(font))
}

// A gid past the CFF, as a cmap or GSUB entry can name, survives the closure but not the compacting
// CFF subset; maxp and the metrics must count only the glyphs the new CFF holds.
#[test]
fn a_compacted_cff_subset_counts_only_the_glyphs_it_keeps() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/stix-two-math/STIX2Math.otf");
    let bytes = std::fs::read(path).expect("STIX is a fixture");
    let source = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("STIX parses");
    let cff = source.get("CFF ").expect("STIX is CFF").to_owned_vec();
    let (outlines, reads) = outlines_and_reads(&source, &cff);
    let text = TextCmap { mappings: &[(65, 36)], sequences: &[] };
    let entries = || Shared::new(CmapEntries::default());
    let cff_source = CffSource::Cff(&cff, outlines.as_ref());
    let none = |_: &[u32]| Ok(BTreeMap::new());
    let out = subset_cff_flavored(&source, &reads, cff_source, &[36, 6_000], Some(text), &entries, &none)
        .expect("a subset");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&out.font).expect("the subset parses");
    let kept = crate::daecore::daetype::outline::CffOutlines::parse(tables.get("CFF ").expect("CFF"))
        .expect("the subset CFF parses").num_glyphs();
    let maxp = read_u16_be(tables.get("maxp").expect("maxp"), 4).expect("numGlyphs");
    assert_eq!(usize::from(maxp), kept, "maxp counts {maxp} glyphs, the CFF holds {kept}");
    let metrics = read_u16_be(tables.get("hhea").expect("hhea"), 34).expect("numberOfHMetrics");
    assert_eq!(usize::from(metrics), kept, "hhea counts {metrics} metrics, the CFF holds {kept} glyphs");
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// STIX plus a kern and a VORG naming glyph 6,000, past its glyphs, beside 36 and 37; a DSIG, a MERG and an EBSC of
// an empty strike, which go; and a vhea too short to count metrics, kept with its vmtx as the source has them.
#[test]
fn a_compacted_cff_subset_keeps_or_drops_each_glyph_table_as_it_should() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/stix-two-math/STIX2Math.otf");
    let bytes = std::fs::read(path).expect("STIX is a fixture");
    let mut source = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("STIX parses");
    let mut strike = vec![0u8; 48];
    strike[44..46].copy_from_slice(&[12, 12]);
    let mut scale = vec![0u8; 28];
    scale[24..28].copy_from_slice(&[24, 24, 12, 12]);
    for (tag, table) in [
        ("kern", u16s(&[0, 1, 0, 26, 0x0001, 2, 12, 1, 0, 36, 37, 0xFFCE, 36, 6_000, 0xFFC4])),
        ("VORG", u16s(&[1, 0, 880, 2, 36, 900, 6_000, 910])),
        ("DSIG", u16s(&[0, 1, 0, 0])),
        ("MERG", u16s(&[0, 0, 0, 0])),
        ("EBLC", [u16s(&[2, 0, 0, 1]), strike].concat()),
        ("EBDT", u16s(&[2, 0])),
        ("EBSC", [u16s(&[2, 0, 0, 1]), scale].concat()),
        ("vhea", u16s(&[1, 0, 880])),
        ("vmtx", u16s(&[1000, 0, 1000, 0])),
    ] {
        source.insert(tag, table.into());
    }
    let cff = source.get("CFF ").expect("STIX is CFF").to_owned_vec();
    let (outlines, reads) = outlines_and_reads(&source, &cff);
    let text = TextCmap { mappings: &[(65, 36)], sequences: &[] };
    let gids = [36, 37, 6_000];
    let entries = || Shared::new(CmapEntries::default());
    let cff_source = CffSource::Cff(&cff, outlines.as_ref());
    let out = subset_cff_flavored(&source, &reads, cff_source, &gids, Some(text), &entries, &|_| Ok(BTreeMap::new()))
        .expect("a subset");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&out.font).expect("the subset parses");
    let new = |g| out.renumbered(GlyphId(g)).expect("kept").0 as u16;
    assert_eq!(
        tables.get("kern").map(|t| t.to_owned_vec()),
        Some(u16s(&[0, 1, 0, 20, 0x0001, 1, 6, 0, 0, new(36), new(37), 0xFFCE]))
    );
    assert_eq!(tables.get("VORG").map(|t| t.to_owned_vec()), Some(u16s(&[1, 0, 880, 1, new(36), 900])));
    for tag in ["DSIG", "MERG", "EBLC", "EBSC"] {
        assert!(!tables.contains_key(tag), "{tag} was kept");
    }
    for tag in ["vhea", "vmtx"] {
        assert_eq!(tables.get(tag).map(|t| t.to_owned_vec()), source.get(tag).map(|t| t.to_owned_vec()), "{tag}");
    }
}

fn varc_font(name: &str, drop: &str) -> FontCache {
    let path = alloc::format!("{}/assets/test-fonts/varc/{name}", env!("CARGO_MANIFEST_DIR"));
    let mut map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    map.as_mut().expect("parses").remove(drop);
    FontCache::new(map.expect("parses")).expect("loads")
}

fn subset_tables(font: &FontCache, key: &AxisKey, gids: &[u32]) -> (Tables, Vec<u32>) {
    let out = font.subset_font(key, gids).expect("a subset");
    (crate::daecore::daetype::decoder::extract_ttf_tables(&out.font).expect("parses"), out.gid_map)
}

// Glyph 34's components name axes, which a subset pins: it holds the glyph as VARC draws it where the subset stands,
// its side bearing at its xMin.
#[test]
fn a_glyf_subset_draws_each_glyph_of_a_varc_it_cannot_keep() {
    use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be};
    use crate::daecore::daetype::outline::Path;
    let font = varc_font("varc-hostile.ttf", "");
    for at in [&[][..], &[("wght", 900.0)]] {
        let key = font.axis_key(at);
        let (tables, gid_map) = subset_tables(&font, &key, &[34]);
        assert!(!tables.contains_key("VARC"));
        let mut path = Path::default();
        font.at_location(&key, |at| at.draw_varc(34, &mut 0, &mut path)).expect("drawn");
        let drawn = flatten::glyf(&path, false).expect("held");
        let new = gid_map[34] as usize;
        let format = read_i16_be(&tables["head"], 50).expect("a format");
        let loca = subsetter::parse_loca(&tables["loca"], format, 1 << 16);
        assert_eq!(tables["glyf"][loca[new]..loca[new] + drawn.len()], drawn[..], "at {at:?}");
        let long = usize::from(read_u16_be(&tables["hhea"], 34).expect("a count"));
        let at = crate::daecore::daetype::format::metrics::bearing_at(long, new).expect("a bearing");
        assert_eq!(read_i16_be(&tables["hmtx"], at), read_i16_be(&drawn, 2));
    }
}

// Without fvar nothing is pinned: VARC is kept, and gvar beside it, which its components vary their glyphs by.
#[test]
fn a_subset_of_a_font_without_fvar_keeps_varc_and_gvar() {
    let (tables, _) = subset_tables(&varc_font("varc-hostile.ttf", "fvar"), &AxisKey::default(), &[34]);
    assert!(tables.contains_key("VARC") && tables.contains_key("gvar"));
}

// The CFF2 subset resolves the store's blends, which the VARC glyph's component varies: it's written as drawn.
#[test]
fn a_cff2_subset_draws_the_glyphs_of_a_varc_whose_blends_it_resolves() {
    let (tables, gid_map) = subset_tables(&varc_font("varc-cff2-store.otf", ""), &AxisKey::default(), &[2]);
    assert!(!tables.contains_key("VARC"));
    let new = if gid_map.is_empty() { 2 } else { gid_map[2] };
    let e = FontCache::new(tables).expect("loads").glyph_extents(new).expect("extents");
    assert_eq!([e.x_bearing, e.y_bearing, e.width, e.height], [200, 100, 150, -100]);
}
