use super::*;
use alloc::string::String;
use crate::daecore::daetype::TableBytes;
use alloc::vec::Vec;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A horizontal BASE of the one baseline tag 'romn', each script record naming the BaseScript `body`.
fn base(scripts: &[&[u8; 4]], body: &[u8]) -> Tables {
    let mut t = words(&[1, 0, 8, 0, 4, 10, 1, 0x726F, 0x6D6E]);
    t.extend(words(&[scripts.len() as u16]));
    for tag in scripts {
        t.extend(*tag);
        t.extend(words(&[2 + 6 * scripts.len() as u16]));
    }
    t.extend(body);
    map(t)
}

fn map(base: Vec<u8>) -> Tables {
    [(String::from("BASE"), TableBytes::from(base))].into_iter().collect()
}

// A BaseScript: baseline -120 in coordinate `format` (2 names glyph 5); default MinMax -200 to 800,
// 'kern' -250 to 850, 'liga' -300 to 900; an 'ENG ' MinMax with only a minimum, -210.
fn script(format: u16) -> Vec<u8> {
    let mut s = words(&[12, 26, 1, 0x454E, 0x4720, 72]);
    s.extend(words(&[0, 1, 6, format, (-120i16) as u16, 5, 0]));
    s.extend(words(&[22, 26, 2, 0x6B65, 0x726E, 30, 34, 0x6C69, 0x6761, 38, 42]));
    for v in [-200i16, 800, -250, 850, -300, 900] {
        s.extend(words(&[1, v as u16]));
    }
    s.extend(words(&[6, 0, 0, 1, (-210i16) as u16]));
    s
}

fn table(map: &Tables) -> &[u8] {
    map.get("BASE").expect("a BASE")
}

// FeatMinMax records are eight bytes; read six apart, the second would come from the first's bytes
// and a glyph-free table read as naming glyphs.
#[test]
fn feature_extents_are_read_eight_bytes_apart() {
    let map = base(&[b"latn"], &script(1));
    assert!(base_is_glyph_free(table(&map)));
    let extents = |lang, feat| base_min_max(&map, Some(*b"latn"), lang, feat, false, &[]);
    assert_eq!(extents(None, None), Some((Some(-200.0), Some(800.0))));
    assert_eq!(extents(None, Some(*b"kern")), Some((Some(-250.0), Some(850.0))));
    assert_eq!(extents(None, Some(*b"liga")), Some((Some(-300.0), Some(900.0))));
    assert_eq!(extents(None, Some(*b"smcp")), Some((Some(-200.0), Some(800.0))));
    assert_eq!(extents(Some(*b"ENG "), None), Some((Some(-210.0), None)));
    assert_eq!(extents(Some(*b"TRK "), None), Some((Some(-200.0), Some(800.0))));
}

// Format 2 gives its coordinate, and names a glyph by ID.
#[test]
fn a_coordinate_naming_a_glyph_is_read_and_found() {
    let map = base(&[b"latn"], &script(2));
    assert!(!base_is_glyph_free(table(&map)));
    let info = base_script_info(&map, Some(*b"latn"), false, &[]).expect("latn's baselines");
    assert_eq!(info.baselines.get(&Tag::new(b"romn")), Some(&-120.0));
}

#[test]
fn dflt_stands_in_for_a_script_without_baselines() {
    let map = base(&[b"DFLT"], &script(1));
    assert_eq!(base_script_info(&map, Some(*b"latn"), false, &[]).map(|i| i.baselines.len()), Some(1));
    assert!(base_min_max(&map, Some(*b"latn"), None, None, false, &[]).is_some());
}

// The empty tag is four spaces, which a record of other bytes never matches, and no tag finds no script:
// without a DFLT, neither has baselines.
#[test]
fn an_empty_tag_names_no_script() {
    let map = base(&[&[0xFF; 4]], &script(1));
    assert_eq!(base_script_info(&map, Some(*b"    "), false, &[]), None);
    assert_eq!(base_script_info(&map, None, false, &[]), None);
}

// A NULL BaseScript offset names no script, so it hides no glyph from a subset.
#[test]
fn a_null_script_record_names_no_script() {
    let mut bytes = table(&base(&[b"latn", b"grek"], &script(1))).to_vec();
    bytes[30..32].copy_from_slice(&[0, 0]);
    let map = map(bytes);
    assert!(base_is_glyph_free(table(&map)));
    assert!(base_script_info(&map, Some(*b"latn"), false, &[]).is_some());
    assert_eq!(base_script_info(&map, Some(*b"grek"), false, &[]), None);
}

// A count past the table's end holds the records that fit: this tag list states two tags and
// holds one, and ENG's MinMax states a feature record and holds none.
#[test]
fn a_count_past_the_tables_end_holds_the_records_that_fit() {
    let tags = map(words(&[
        1, 0, 8, 0, 28, 4, 1, 0x6C61, 0x746E, 8, 6, 0, 0, 0, 1, 6, 1, (-120i16) as u16, 2, 0x726F, 0x6D6E,
    ]));
    let info = base_script_info(&tags, Some(*b"latn"), false, &[]).expect("latn's baselines");
    assert_eq!(info.default_baseline, Some(Tag::new(b"romn")));
    assert_eq!(info.baselines.get(&Tag::new(b"romn")), Some(&-120.0));

    let mut body = script(1);
    body[76..78].copy_from_slice(&[0, 1]);
    let features = base(&[b"latn"], &body);
    assert!(base_is_glyph_free(table(&features)));
    let english = base_min_max(&features, Some(*b"latn"), Some(*b"ENG "), None, false, &[]);
    assert_eq!(english, Some((Some(-210.0), None)));
}

// 32 script records naming one BaseScript, its 32 language records naming one MinMax of 65,535
// feature records: walked per naming, 67 million records, walked once, 65,535.
#[test]
fn a_shared_script_and_min_max_are_walked_once() {
    let mut body = words(&[0, 6 + 6 * 32, 32]);
    (0..32).for_each(|_| body.extend(words(&[0x4142, 0x4344, 6 + 6 * 32])));
    body.extend(words(&[0, 0, u16::MAX]));
    body.extend(alloc::vec![0u8; 8 * usize::from(u16::MAX)]);
    let map = base(&[b"latn"; 32], &body);
    let started = std::time::Instant::now();
    assert!(base_is_glyph_free(table(&map)));
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
}

// Scripts naming BaseValues two bytes apart in one run of 0xFFFF words, so each reads a coordinate past the table for
// every word after it: past the subset budget at 20 scripts, though not at 2, where the table may name any glyph.
#[test]
fn values_that_overlap_past_the_budget_are_not_glyph_free() {
    let glyph_free = |scripts: usize| {
        let (first, run) = (14 + 6 * scripts, 14 + 12 * scripts);
        let mut t = words(&[1, 0, 8, 0, 0, 4, scripts as u16]);
        for i in 0..scripts {
            t.extend(b"latn");
            t.extend(words(&[(first + 6 * i - 12) as u16]));
        }
        (0..scripts).for_each(|i| t.extend(words(&[(run + 2 * i - first - 6 * i) as u16, 0, 0])));
        t.extend(words(&[0xFFFF; 20_000]));
        base_is_glyph_free(&t)
    };
    assert!(glyph_free(2));
    assert!(!glyph_free(20));
}
