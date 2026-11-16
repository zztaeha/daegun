use super::*;

// Kept whole, 65,537 glyphs of changing advances stay long records in HMTX; hmtx holds 65,535 of them, the last
// two by their bearings alone.
#[test]
fn hmtx_keeps_65_535_long_records_and_its_uppercase_twin_all() {
    let n = 0x1_0001;
    let mtx: Vec<u8> = (0..n as u32).flat_map(|g| [(g % 3) as u16, 0]).flat_map(u16::to_be_bytes).collect();
    let all: Vec<u32> = (0..n as u32).collect();
    assert_eq!(rebuild_metrics(&mtx, n, n, &all, true), (mtx.clone(), n));
    let (lower, long) = rebuild_metrics(&mtx, n, n, &all, false);
    assert_eq!((long, lower.len()), (0xFFFF, 4 * 0xFFFF + 2 * 2));
}

// Format 0 pairs out of order, 2 1 before 1 2 twice: the font's search finds 1 2 at its first copy and never
// reaches 2 1, so the subset keeps 1 2, and 2 as a left glyph alone.
#[test]
fn kern_pairs_come_back_as_the_font_finds_them() {
    let kern: Vec<u8> = [0u16, 1, 0, 32, 1, 3, 12, 1, 6, 2, 1, 0xFFF6, 1, 2, 0xFFEC, 1, 2, 0xFFE2]
        .iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut active = GlyphSet::new();
    (0..3).for_each(|g| { active.insert(g); });
    let out = remap_kern(&kern, &[0, 1, 2], &active).expect("a subset kern");
    let pairs: Vec<(u16, u16, i16)> = (0..usize::from(read_u16_be(&out, 10).expect("a pair count")))
        .map(|i| {
            (
                read_u16_be(&out, 18 + 6 * i).unwrap(),
                read_u16_be(&out, 20 + 6 * i).unwrap(),
                read_i16_be(&out, 22 + 6 * i).unwrap(),
            )
        })
        .collect();
    assert_eq!(pairs, [(1, 2, -20), (2, 0xFFFF, 0)]);
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// Glyphs 2 and 3 in rows 0 and 1, 5 and 6 in columns 0 and 1, of a 2 by 2 array: a kern table of
// one format 2 subtable, OpenType or Apple. Offsets count from the subtable's start.
fn format2(apple: bool) -> Vec<u8> {
    let h: u16 = if apple { 8 } else { 6 };
    let array = h + 24;
    let body =
        [u16s(&[4, h + 8, h + 16, array, 2, 2, array, array + 4, 5, 2, 0, 2]), u16s(&[0xFFF6, 0xFFEC, 0xFFE2, 0xFFD8])]
            .concat();
    let length = usize::from(h) + body.len();
    let header = if apple { [u16s(&[1, 0, 0, 1]), (length as u32).to_be_bytes().to_vec(), u16s(&[2, 0])].concat() }
        else { u16s(&[0, 1, 0, length as u16, 0x0201]) };
    [header, body].concat()
}

// How shaping reads a format 2 pair: a glyph both class tables hold in any class but 1 sums its row and column
// offsets, and what lies past the array's start halves into an index.
fn kerning(kern: &[u8], apple: bool, left: u16, right: u16) -> i16 {
    let sub = if apple { 8 } else { 4 };
    let body = sub + if apple { 8 } else { 6 };
    let at = |k: usize| usize::from(read_u16_be(kern, k).unwrap_or(0));
    let class = |table: usize, g: u16| {
        let (first, n) = (at(sub + table) as u16, at(sub + table + 2) as u16);
        if g < first || g >= first + n { return None; }
        Some(at(sub + table + 4 + 2 * usize::from(g - first))).filter(|&k| k != 1)
    };
    let (Some(row), Some(column)) = (class(at(body + 2), left), class(at(body + 4), right)) else { return 0 };
    let array = at(body + 6);
    let Some(i) = (row + column).checked_sub(array) else { return 0 };
    read_i16_be(kern, sub + array + i / 2 * 2).unwrap_or(0)
}

#[test]
fn class_kerning_keeps_its_values_on_the_kept_glyphs() {
    let mut active = GlyphSet::new();
    [0, 3, 5, 6].into_iter().for_each(|g| { active.insert(g); });
    let gid_map = [0, 0, 0, 1, 0, 2, 3];
    for apple in [false, true] {
        let kern = format2(apple);
        let out = remap_kern(&kern, &gid_map, &active).expect("the class subtable is kept");
        for (left, right) in [(3, 5), (3, 6), (5, 3), (3, 3)] {
            let (new_left, new_right) = (gid_map[left] as u16, gid_map[right] as u16);
            assert_eq!(
                kerning(&out, apple, new_left, new_right),
                kerning(&kern, apple, left as u16, right as u16),
                "apple {apple}: {left} {right}",
            );
        }
        assert_eq!(kerning(&out, apple, 1, 2), -30);
    }
}

// Apple format 2 subtables of 22 bytes whose kept glyph 0 is in class 0x7FFE on both sides, its pair far past the
// subtable: rebuilt only as far as the subtable holds its array, where each would write 64 KB of zeros.
#[test]
fn a_class_sum_past_its_subtable_rebuilds_no_further() {
    let sub = [22u32.to_be_bytes().to_vec(), u16s(&[2, 0, 2, 16, 16, 16, 0, 1, 0x7FFE])].concat();
    let kern = |n: u32| [u16s(&[1, 0]), n.to_be_bytes().to_vec(), sub.repeat(n as usize)].concat();
    let active: GlyphSet = [0].into_iter().collect();
    let one = remap_kern(&kern(1), &[0], &active).expect("a subset kern");
    assert_eq!(kerning(&one, true, 0, 0), kerning(&kern(1), true, 0, 0));
    let out = remap_kern(&kern(10), &[0], &active).expect("a subset kern");
    assert!(out.len() < 2 * kern(10).len(), "{} bytes of kern became {}", kern(10).len(), out.len());
}

// Four 19-byte format 3 subtables of one glyph each, in a subset keeping 65,535: each writes a class for every kept
// glyph, 128 KB from 84 bytes of kern, which passes the budget and is left out.
#[test]
fn a_kern_rebuilt_past_its_budget_is_left_out() {
    let sub = [19u32.to_be_bytes().to_vec(), u16s(&[3, 0, 1]), alloc::vec![1, 1, 1, 0, 0, 0, 0, 0, 0]].concat();
    let kern = [u16s(&[1, 0]), 4u32.to_be_bytes().to_vec(), sub.repeat(4)].concat();
    let active: GlyphSet = (0..0xFFFF).collect();
    let gid_map: Vec<u32> = (0..0xFFFF).collect();
    assert_eq!(remap_kern(&kern, &gid_map, &active).map(|out| out.len()), None);
}

// The last subtable, 11,000 pairs, wraps its 16-bit length and is read to the table's end.
#[test]
fn the_last_kern_subtable_may_pass_64_kb() {
    let pairs = |n: u16| {
        [
            u16s(&[0, (14 + 6 * u32::from(n)) as u16, 0x0001, n, 0, 0, 0]),
            (0..n).flat_map(|i| u16s(&[1, i + 1, 5])).collect(),
        ]
        .concat()
    };
    let mut active = GlyphSet::new();
    (0..=11_000).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..=11_000).collect();
    let kern = [u16s(&[0, 2]), pairs(2), pairs(11_000)].concat();
    let out = remap_kern(&kern, &gid_map, &active).expect("a kern");
    let second = 4 + usize::from(read_u16_be(&out, 6).expect("length"));
    assert_eq!([read_u16_be(&out, 10), read_u16_be(&out, second + 6)], [Some(2), Some(11_000)]);
    assert_eq!(out.len(), kern.len());
}

// A format 3 subtable of 6 glyphs in a 9-glyph font, so 6 to 8 read class 0: 5 values, 3 left classes and 3
// right ones. Glyph 4's left class is past the count and a cell names a value past the values, both reading 0.
fn format3(apple: bool, coverage: u16) -> Vec<u8> {
    let values = u16s(&[0, 0xFFF6, 0xFFEC, 20, 0xFFD8]);
    let (lefts, rights) = ([0u8, 1, 2, 1, 3, 2], [2u8, 0, 1, 2, 1, 0]);
    let index = [0u8, 1, 4, 2, 9, 3, 1, 1, 0];
    let body = [u16s(&[6]), vec![5, 3, 3, 0], values, lefts.to_vec(), rights.to_vec(), index.to_vec()].concat();
    let header = if apple { 8 } else { 6 };
    let length = header + body.len();
    let header = if apple {
        [u16s(&[1, 0, 0, 1]), (length as u32).to_be_bytes().to_vec(), u16s(&[coverage | 3, 0])].concat()
    } else {
        u16s(&[0, 1, 0, length as u16, 0x0301])
    };
    [header, body].concat()
}

// Every kept pair's value in each format 3 subtable, read as shaping reads it, source and subset alike.
fn kept_pairs_read_alike(kern: &[u8], out: &[u8], active: &GlyphSet, gid_map: &[u32], past: u32) {
    use crate::daecore::daeshaper::ot::kern::testing::format3_values;
    for left in active.iter().filter(|&g| g < past) {
        for right in active.iter().filter(|&g| g < past) {
            let (l, r) = (gid_map[left as usize], gid_map[right as usize]);
            assert_eq!(format3_values(out, l, r), format3_values(kern, left, right), "{left} {right}");
        }
    }
}

// With glyph 3, left class 1 meets right class 1, whose cell is past the values; without it, left class 1 and
// values 2 and 3 go. Glyph 4's left class, past the count, takes an added row of zeros in both.
#[test]
fn a_format3_subset_keeps_each_kept_pairs_value_and_only_what_they_read() {
    let cases: [(&[u32], [u8; 3]); 2] = [(&[0, 2, 3, 4, 5, 7], [5, 4, 3]), (&[0, 2, 4, 5, 7], [3, 3, 3])];
    for (glyphs, counts) in cases {
        let active: GlyphSet = glyphs.iter().copied().collect();
        let mut gid_map = vec![0; 8];
        glyphs.iter().enumerate().for_each(|(new, &old)| gid_map[old as usize] = new as u32);
        for (apple, coverage) in [(true, 0x2000), (true, 0), (false, 0)] {
            let kern = format3(apple, coverage);
            let out = remap_kern(&kern, &gid_map, &active).expect("a subset kern");
            kept_pairs_read_alike(&kern, &out, &active, &gid_map, 9);
            let body = if apple { 16 } else { 10 };
            assert_eq!(read_u16_be(&out, body), Some(glyphs.len() as u16), "glyphCount is the subset's");
            assert_eq!(out[body + 2..body + 5], counts, "values, left and right classes of {glyphs:?}");
        }
    }
}

// Skia's five format 3 subtables, four of them variations, where the font is installed.
#[test]
fn skias_format3_subtables_keep_each_kept_pairs_value() {
    let Ok(ttf) = std::fs::read("/System/Library/Fonts/Supplemental/Skia.ttf") else { return };
    let dir = ttf_table_ranges(&ttf).expect("Skia reads");
    let kern = slice_table(&ttf, &dir, "kern").expect("Skia has a kern");
    let requested: Vec<u32> = (0..591).step_by(3).collect();
    let subset = super::super::subset_ttf(&ttf, &requested).expect("a subset");
    let out_dir = ttf_table_ranges(&subset.font).expect("the subset reads");
    let out = slice_table(&subset.font, &out_dir, "kern").expect("the subset keeps kern");
    let active: GlyphSet = requested.iter().copied().filter(|&g| subset.renumbered(GlyphId(g)).is_some()).collect();
    kept_pairs_read_alike(kern, out, &active, &subset.gid_map, 591);
    assert!(out.len() < kern.len(), "{} of {} bytes", out.len(), kern.len());
}

// A record whose glyph the subset holds no place for is left out; the rest stay, in the source's version at
// minor 0 and version 2's 24-bit ids.
#[test]
fn a_vorg_record_without_a_place_is_left_out() {
    let vorg = u16s(&[1, 5, 880, 2, 36, 900, 6_000, 910]);
    let mut active = GlyphSet::new();
    [0, 36, 6_000].into_iter().for_each(|g| { active.insert(g); });
    let mut gid_map = vec![0; 37];
    gid_map[36] = 1;
    assert_eq!(remap_vorg(&vorg, &gid_map, &active), Some(u16s(&[1, 0, 880, 1, 1, 900])));
    let mut active = GlyphSet::new();
    [0, 36, 0x1_0001].into_iter().for_each(|g| { active.insert(g); });
    let mut gid_map = vec![0; 0x1_0002];
    (gid_map[36], gid_map[0x1_0001]) = (1, 2);
    let v2 = [0, 2, 0, 0, 3, 112, 0, 0, 3, 0, 0, 36, 3, 132, 0, 0x17, 0x70, 3, 142, 1, 0, 1, 0, 9];
    let want = [0, 2, 0, 0, 3, 112, 0, 0, 2, 0, 0, 1, 3, 132, 0, 0, 2, 0, 9];
    assert_eq!(remap_vorg(&v2, &gid_map, &active), Some(want.to_vec()));
    // A caller's own map can renumber a version 1 glyph past 16 bits: no table, not a cut id.
    gid_map[36] = 0x1_0000;
    assert_eq!(remap_vorg(&vorg, &gid_map, &active), None);
}
