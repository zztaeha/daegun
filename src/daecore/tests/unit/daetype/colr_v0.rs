use super::*;
use alloc::string::String;
use crate::daecore::daetype::TableBytes;

// A CPAL of 2 entries a palette, palettes starting at `starts`, `records` colors and, after
// them, bytes that are not colors; version 1 with these entry labels when given.
fn cpal(starts: &[u16], records: u16, labels: Option<&[u16]>) -> Vec<u8> {
    let v1 = labels.is_some();
    let header = 12 + 2 * starts.len() + if v1 { 12 } else { 0 };
    let mut out = [u16::from(v1), 2, starts.len() as u16, records].map(u16::to_be_bytes).concat();
    out.extend((header as u32).to_be_bytes());
    starts.iter().for_each(|s| out.extend(s.to_be_bytes()));
    let labels_at = header + 4 * usize::from(records) + 4;
    if v1 {
        out.extend([0u32, 0, labels_at as u32].map(u32::to_be_bytes).concat());
    }
    for i in 0..records {
        out.extend([i as u8, 0, 0, 255]);
    }
    out.extend([0xAB; 4]);
    labels.unwrap_or_default().iter().for_each(|l| out.extend(l.to_be_bytes()));
    out
}

// Palette 1 starts at record 2 of 3, so its second entry would be the bytes after the records: it draws as
// palette 0. A lone palette that runs past them has its entries there refused.
#[test]
fn a_palette_past_the_color_records_draws_as_palette_0() {
    let table = cpal(&[0, 2], 3, None);
    let palette = CpalPalette::new(&table, 1).expect("palette 1");
    assert_eq!([0, 1].map(|i| palette.entry(&table, i)), [Some((0, 0, 0, 255)), Some((0, 0, 1, 255))]);
    let table = cpal(&[2], 3, None);
    let palette = CpalPalette::new(&table, 0).expect("palette 0");
    assert_eq!([0, 1].map(|i| palette.entry(&table, i)), [Some((0, 0, 2, 255)), None]);
}

// A base glyph naming two layers when the table holds one: its run's count leaves the layer records, so the run is
// absent, the second layer being the bytes after.
#[test]
fn a_layer_run_past_the_layer_records_is_absent() {
    let mut colr = [0u16, 1].map(u16::to_be_bytes).concat();
    colr.extend([14u32, 20].map(u32::to_be_bytes).concat());
    colr.extend(1u16.to_be_bytes());
    colr.extend([5u16, 0, 2].map(u16::to_be_bytes).concat());
    colr.extend([7u16, 0, 8, 0].map(u16::to_be_bytes).concat());
    let map: Tables =
        [("COLR", colr), ("CPAL", cpal(&[0], 2, None))]
            .into_iter()
            .map(|(t, b)| (String::from(t), TableBytes::from(b)))
            .collect();
    assert_eq!(colr_layers(&map, 5), None);
}

#[test]
fn palette_entries_have_their_labels() {
    let map: Tables =
        [(String::from("CPAL"), TableBytes::from(cpal(&[0], 2, Some(&[300, 0xFFFF]))))].into_iter().collect();
    assert_eq!(cpal_palette_entry_labels(&map), [Some(NameId(300)), None]);
    let map: Tables = [(String::from("CPAL"), TableBytes::from(cpal(&[0], 2, None)))].into_iter().collect();
    assert!(cpal_palette_entry_labels(&map).is_empty(), "a version 0 table labels nothing");
}

// An undefined later version still holds version 1's arrays, and a label array cut short keeps the
// labels it holds.
#[test]
fn a_later_version_has_its_labels() {
    let mut table = cpal(&[0], 2, Some(&[300, 0xFFFF]));
    table[1] = 2;
    let map: Tables = [(String::from("CPAL"), TableBytes::from(table.clone()))].into_iter().collect();
    assert_eq!(cpal_palette_entry_labels(&map), [Some(NameId(300)), None]);
    table.truncate(table.len() - 2);
    let map: Tables = [(String::from("CPAL"), TableBytes::from(table))].into_iter().collect();
    assert_eq!(cpal_palette_entry_labels(&map), [Some(NameId(300)), None]);
}
