use std::collections::BTreeMap;

use daegun::daecore::daetype::decoder::read_u16_be;
use daegun::daecore::daetype::subsetter::mark_glyph_set_count;
use daegun::Font;

use super::FONTS;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font")).expect("it parses")
}

// How many of a GSUB or GPOS's lookups filter marks by a mark glyph set.
fn filtering(table: &[u8]) -> usize {
    let list = usize::from(read_u16_be(table, 8).expect("a lookup list"));
    let count = read_u16_be(table, list).expect("a lookup count");
    (0..usize::from(count))
        .filter(|i| {
            let at = list + usize::from(read_u16_be(table, list + 2 + i * 2).expect("a lookup"));
            read_u16_be(table, at + 2).expect("a lookup flag") & 0x0010 != 0
        })
        .count()
}

// `base` with Noto Devanagari's GDEF and GSUB, which filter 13 lookups by its 10 mark glyph sets, and
// those sets made unreadable when `broken`; subset to every glyph, through the glyf or CFF path.
fn subset_with_devanagari_layout(base: &str, broken: bool) -> Font {
    let devanagari = font("noto-devanagari/NotoSansDevanagari.ttf");
    let source = font(base);
    let mut tables: BTreeMap<String, Vec<u8>> =
        source.table_tags().into_iter().map(|t| (t.to_string(), source.table(t).expect("listed").to_vec())).collect();
    let mut gdef = devanagari.table("GDEF").expect("a GDEF").to_vec();
    if broken {
        let sets = usize::from(read_u16_be(&gdef, 12).expect("MarkGlyphSets"));
        gdef[sets..sets + 2].copy_from_slice(&2u16.to_be_bytes());
    }
    tables.insert("GDEF".into(), gdef);
    tables.insert("GSUB".into(), devanagari.table("GSUB").expect("a GSUB").to_vec());
    let patched = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    let gids: Vec<u16> = (0..patched.num_glyphs()).collect();
    Font::from_vec(patched.subset(&gids, &[]).expect("a subset").ttf).expect("the subset parses")
}

// MarkGlyphSets of an undefined format cost only themselves: the glyph classes stay, and GSUB's
// lookups stop filtering by sets the subset's GDEF lacks.
#[test]
fn unreadable_mark_glyph_sets_cost_only_themselves() {
    for base in ["noto-devanagari/NotoSansDevanagari.ttf", "stix-two-math/STIX2Math.otf"] {
        let out = subset_with_devanagari_layout(base, true);
        let gdef = out.table("GDEF").unwrap_or_else(|| panic!("{base}: the subset lost its GDEF"));
        assert_ne!(read_u16_be(gdef, 4), Some(0), "{base}: the glyph classes went");
        assert_eq!(mark_glyph_set_count(gdef), 0, "{base}");
        assert_eq!(filtering(out.table("GSUB").expect("a GSUB")), 0, "{base}: lookups filter by sets GDEF lacks");
    }
}

#[test]
fn readable_mark_glyph_sets_keep_their_lookups_filtering() {
    for base in ["noto-devanagari/NotoSansDevanagari.ttf", "stix-two-math/STIX2Math.otf"] {
        let out = subset_with_devanagari_layout(base, false);
        assert_eq!(mark_glyph_set_count(out.table("GDEF").expect("a GDEF")), 10, "{base}");
        assert_eq!(filtering(out.table("GSUB").expect("a GSUB")), 13, "{base}");
    }
}
