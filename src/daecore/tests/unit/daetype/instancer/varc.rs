use super::*;
use crate::daecore::daetype::TableBytes;
use crate::daecore::daetype::instancer::{instance_tables_for_subset, instance_tables_from_map, AxisTag};
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be};
use crate::daecore::daetype::format::metrics::bearing_at;
use crate::daecore::daetype::format::sanitize::Reads;
use crate::daecore::daetype::varc::flatten;

fn fixture(name: &str) -> Tables {
    let path = alloc::format!("{}/assets/test-fonts/varc/{name}", env!("CARGO_MANIFEST_DIR"));
    crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture")).expect("parses")
}

#[test]
fn each_glyph_takes_its_drawn_record_and_a_glyph_past_loca_gets_one() {
    let drawn = BTreeMap::from([(1, vec![9, 9, 9])]);
    let (glyf, loca) = with_drawn(&[1, 2, 3, 4, 5], &[0, 3, 5], 3, &drawn).expect("within the budget");
    assert_eq!((glyf, loca), (vec![1, 2, 3, 0, 9, 9, 9, 0], vec![0, 4, 8, 8]));
    let past = BTreeMap::from([(2, vec![7, 7])]);
    let drawn = with_drawn(&[1, 2], &[0, 2], 3, &past).expect("within the budget");
    assert_eq!(drawn, (vec![1, 2, 7, 7], vec![0, 2, 2, 4]));
}

// The fixture's components vary, which the instancer won't pin: the instance holds each glyph VARC draws, its bearing
// at its xMin, inside head's box and maxp's counts, at the default too; one a subset reads leaves VARC to it.
#[test]
fn an_instance_draws_the_glyphs_of_a_varc_it_cannot_keep() {
    let map = fixture("varc-hangul.ttf");
    for at in [&[(AxisTag::new(b"wght"), 700.0)][..], &[]] {
        let varc = varc_instance(&map, at).expect("within the budget").expect("a VARC");
        assert!(varc.table.is_none() && !varc.drawn.is_empty());
        let instance = instance_tables_from_map(&map, &Reads::of_tables(&map), at).expect("an instance");
        assert!(!instance.contains_key("VARC"));
        let field = |tag: &str, at: usize| read_i16_be(&instance[tag], at).expect("a field");
        let loca = crate::daecore::daetype::subsetter::parse_loca(&instance["loca"], field("head", 50), 1 << 16);
        let long = usize::from(read_u16_be(&instance["hhea"], 34).expect("a count"));
        for (&gid, path) in &varc.drawn {
            let glyph = &instance["glyf"][loca[gid as usize]..loca[gid as usize + 1]];
            let written = flatten::glyf(path, false).expect("held");
            assert_eq!(glyph[..written.len()], written[..], "glyph {gid}");
            let [x_min, y_min, x_max, y_max] = [2, 4, 6, 8].map(|o| read_i16_be(&written, o).expect("a box"));
            assert_eq!(field("hmtx", bearing_at(long, gid as usize).expect("a bearing")), x_min);
            assert!(field("head", 36) <= x_min && field("head", 38) <= y_min);
            assert!(field("head", 40) >= x_max && field("head", 42) >= y_max);
            let points = read_u16_be(&written, 10 + 2 * (read_i16_be(&written, 0).expect("contours") as usize - 1));
            assert!(read_u16_be(&instance["maxp"], 6) > points, "glyph {gid}");
        }
        let kept = instance_tables_for_subset(&map, &Reads::of_tables(&map), at).expect("an instance");
        assert_eq!(kept.get("VARC").map(|v| &v[..]), map.get("VARC").map(|v| v.as_slice()));
    }
}

// A CFF2 with a store, which the instance pins: the VARC glyph's component names the store's axis, so the instance
// writes it as a charstring of its drawing, measured into head's box.
#[test]
fn a_cff2_instance_draws_the_glyphs_of_a_varc_it_cannot_keep() {
    let map = fixture("varc-cff2-store.otf");
    let instance = instance_tables_from_map(&map, &Reads::of_tables(&map), &[]).expect("an instance");
    assert!(!instance.contains_key("VARC"));
    let tables = instance.iter().map(|(tag, t)| (tag.clone(), TableBytes::from_vec(t.to_vec()))).collect();
    let font = crate::daecore::daekit::cache::FontCache::new(tables).expect("loads");
    let e = font.glyph_extents(2).expect("extents");
    assert_eq!([e.x_bearing, e.y_bearing, e.width, e.height], [200, 100, 150, -100]);
    assert_eq!([36, 38, 40, 42].map(|o| read_i16_be(&instance["head"], o)), [0, 0, 350, 100].map(Some));
}

// The fixture with every other glyph past its five naming all of glyf, at the default, where a whole instance copies
// the font's own loca: past the subset budget at 3,000 glyphs, though not at 200, the copy is refused.
#[test]
fn glyphs_sharing_their_bytes_past_the_budget_refuse_the_instance() {
    let instance = |glyphs: u16| {
        let mut map = fixture("varc-hangul.ttf");
        let (glyf, short) = (map["glyf"].len() as u32, map["loca"].to_vec());
        let own = short.as_chunks::<2>().0.iter().take(6).map(|&o| 2 * u32::from(u16::from_be_bytes(o)));
        let shared = (6..=u32::from(glyphs)).map(|i| if i % 2 == 0 { 0 } else { glyf });
        let loca = own.chain(shared).flat_map(u32::to_be_bytes).collect();
        let (mut head, mut maxp) = (map["head"].to_vec(), map["maxp"].to_vec());
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&glyphs.to_be_bytes());
        for (tag, table) in [("loca", loca), ("head", head), ("maxp", maxp)] {
            map.insert(tag, TableBytes::from_vec(table));
        }
        instance_tables_from_map(&map, &Reads::of_tables(&map), &[]).map(drop).map_err(|e| e.kind())
    };
    assert_eq!(instance(200), Ok(()));
    assert_eq!(instance(3_000), Err(crate::daecore::ErrorKind::LimitExceeded));
}
