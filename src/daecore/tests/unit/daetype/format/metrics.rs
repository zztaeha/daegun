use super::*;
use alloc::string::String;

// Past 65,535 long records a glyph keeps its bearing alone, and the count is cut back where the advances end the
// same; 65,535 or fewer, or a table too short for its count, stay as they are.
#[test]
fn metrics_for_a_16_bit_count_keep_65_535_long_records() {
    let long = 0x1_0001;
    let mtx: Vec<u8> = (0..long).flat_map(|g: u32| [(g % 3) as u16, g as u16]).chain([7]).flat_map(u16::to_be_bytes)
        .collect();
    let (out, kept) = narrowed(mtx.clone(), long as usize);
    assert_eq!((kept, out.len()), (0xFFFF, 4 * 0xFFFF + 2 * 2 + 2));
    assert_eq!((&out[..4 * 0xFFFF], &out[4 * 0xFFFF..]), (&mtx[..4 * 0xFFFF], &[0xFF, 0xFF, 0, 0, 0, 7][..]));
    let same: Vec<u8> = (0..long).flat_map(|_| [500u16, 0]).flat_map(u16::to_be_bytes).collect();
    assert_eq!(narrowed(same, long as usize).1, 1);
    assert_eq!(narrowed(mtx[..4 * 0xFFFF].to_vec(), 0xFFFF).1, 0xFFFF);
    assert_eq!(narrowed(mtx[..8].to_vec(), long as usize), (mtx[..8].to_vec(), 0xFFFF));
}

// A header at `major` with its count of long records, 32 bits where `wide`, and three glyphs' metrics.
fn font(tables: &[(&str, Vec<u8>)]) -> Tables {
    let maxp = ("maxp", alloc::vec![0, 0, 0x50, 0, 0, 3]);
    tables.iter().chain([&maxp]).map(|(tag, bytes)| (String::from(*tag), TableBytes::from(bytes.clone()))).collect()
}

fn hea(major: u16, wide: bool, long: u32) -> Vec<u8> {
    let mut hea = alloc::vec![0; LONG_METRICS];
    hea[..2].copy_from_slice(&major.to_be_bytes());
    let count = long.to_be_bytes();
    hea.extend_from_slice(if wide { &count[..] } else { &count[2..] });
    hea
}

// Shaping reads a header at major 1 and long enough for its count, and HMTX where HHEA's 32-bit count gives a glyph
// a metric, else hmtx; drawing takes HHEA alone beside GLYF, and hhea first without it.
#[test]
fn each_path_picks_its_metrics_pair() {
    let mtx = |advance: u16| [advance, 0].repeat(3).iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let both = |upper: Vec<u8>| {
        font(&[("hhea", hea(1, false, 3)), ("hmtx", mtx(500)), ("HHEA", upper), ("HMTX", mtx(700))])
    };
    let shaped = |map: &Tables| shaped_tags(map, false).table(map).map(|t| t.advance(1));
    assert_eq!(shaped(&both(hea(1, true, 3))), Some(700));
    for upper in [hea(1, true, 0), hea(2, true, 3), hea(1, false, 3)] {
        assert_eq!(shaped(&both(upper)), Some(500));
    }
    let lone = font(&[("hhea", hea(2, false, 3)), ("hmtx", mtx(500))]);
    assert_eq!((shaped_header(&lone, false), shaped(&lone).filter(|&a| a > 0)), (None, None));
    assert_eq!(shaped_header(&both(hea(1, true, 3)), false).map(<[u8]>::len), Some(38));

    let drawn =
        |map: &Tables| drawn_metrics(map, false).map(|(hea, _, long)| (hea.len(), long));
    let mut glyf = both(hea(1, true, 2));
    assert_eq!(drawn(&glyf), Some((36, Some(3))));
    let tables = [("GLYF", alloc::vec![0]), ("LOCA", alloc::vec![0])];
    glyf.extend(tables.map(|(t, b)| (String::from(t), TableBytes::from(b))));
    assert_eq!(drawn(&glyf), Some((38, Some(2))));
    glyf.remove("HHEA");
    assert_eq!(drawn(&glyf), None);
}

// Long records whose count leaves the table are absent and take every metric with them: three stated in 10 bytes.
// Two that fit keep theirs; past them a glyph takes the last advance, its bearing 0 where the bearing array leaves
// the table, and with no glyph count the bearings run on to the table's end.
#[test]
fn an_array_past_the_table_is_absent() {
    let data = [1, 244, 0, 10, 2, 88, 0, 20, 0, 30];
    assert_eq!([0, 1, 2].map(|g| MetricsTable::new(&data, 3).pair(g)), [(0, 0); 3]);
    assert!(MetricsTable::read(&data, 3, 4).is_none());
    let read = MetricsTable::read(&data, 2, 4).expect("a table");
    assert_eq!([0, 1, 2, 3, 4].map(|g| read.pair(g)), [(500, 10), (600, 20), (600, 0), (600, 0), (0, 0)]);
    assert_eq!(MetricsTable::read(&data, 2, 3).expect("a table").pair(2), (600, 30), "a bearing array that fits");
    let counted_by_none = MetricsTable::new(&data, 2);
    assert_eq!([2, 3, 9].map(|g| counted_by_none.pair(g)), [(600, 30), (600, 0), (600, 0)]);
}

// Four long records, the last two of the second's advance, and a bearing after: the third and fourth keep their
// bearings alone. A table shorter than its records stays as it is, as does one whose last advance changes.
#[test]
fn long_records_of_the_last_advance_become_bearings() {
    let data = vec![1, 244, 0, 10, 2, 88, 0, 20, 2, 88, 0, 30, 2, 88, 0, 40, 0, 50];
    assert_eq!(compacted(data.clone(), 4), (vec![1, 244, 0, 10, 2, 88, 0, 20, 0, 30, 0, 40, 0, 50], 2));
    assert_eq!(compacted(data[..14].to_vec(), 4), (data[..14].to_vec(), 4));
    assert_eq!(compacted(data[..8].to_vec(), 2), (data[..8].to_vec(), 2));
    assert_eq!(compacted(Vec::new(), 0), (Vec::new(), 0));
}
