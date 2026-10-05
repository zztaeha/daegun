use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::decoder::{read_u16_be, read_u32_be, parse_all_name_strings, records_fit};
use crate::daecore::daetype::TableBytes;

#[derive(Debug, PartialEq)]
pub struct StatAxis { pub tag: String, pub name: Option<String>, pub ordering: u16 }

// `older_sibling` marks a value that describes other fonts of the family rather than this one.
#[derive(Debug, PartialEq)]
pub enum StatAxisValue {
    Single { axis_index: u16, name: Option<String>, value: f64, elidable: bool, older_sibling: bool },
    Range {
        axis_index: u16, name: Option<String>, nominal: f64, min: f64, max: f64, elidable: bool, older_sibling: bool,
    },
    Linked {
        axis_index: u16, name: Option<String>, value: f64, linked_value: f64, elidable: bool, older_sibling: bool,
    },
    Combo  { name: Option<String>, values: Vec<(u16, f64)>, elidable: bool, older_sibling: bool },
}

const OLDER_SIBLING_FONT_ATTRIBUTE: u16 = 0x0001;
const ELIDABLE_AXIS_VALUE_NAME: u16 = 0x0002;

fn read_fixed(data: &[u8], off: usize) -> Option<f64> {
    read_u32_be(data, off).map(|v| v as i32 as f64 / 65536.0)
}

pub type StatInfo = (Vec<StatAxis>, Vec<StatAxisValue>, Option<String>);

pub fn parse_stat(
    table_map: &BTreeMap<String, TableBytes>,
) -> Result<StatInfo, String> {
    let stat = table_map.get("STAT").ok_or("missing STAT")?;
    if stat.len() < 18 { return Err("STAT: header truncated".into()); }

    let minor_version               = read_u16_be(stat, 2).ok_or("STAT: header truncated")?;
    let design_axis_size            = read_u16_be(stat, 4).ok_or("STAT: header truncated")? as usize;
    let design_axis_count           = read_u16_be(stat, 6).ok_or("STAT: header truncated")? as usize;
    let design_axes_offset          = read_u32_be(stat, 8).ok_or("STAT: header truncated")? as usize;
    let axis_value_count            = read_u16_be(stat, 12).ok_or("STAT: header truncated")? as usize;
    let offset_to_axis_value_offsets = read_u32_be(stat, 14).ok_or("STAT: header truncated")? as usize;

    let names = parse_all_name_strings(table_map);
    // Each axis and value gets its name's own copy, so a name that every value names cannot multiply
    // past the bytes the two tables could hold between them.
    let mut name_budget = stat.len().saturating_mul(16).saturating_add(names.values().map(String::len).sum());
    let mut name_of = |id: u16| -> Option<String> {
        let name = names.get(&id)?;
        name_budget = name_budget.checked_sub(name.len())?;
        Some(name.clone())
    };

    const STAT_AXIS_RECORD_SIZE: usize = 8;
    if design_axis_count > 0 && design_axis_size < STAT_AXIS_RECORD_SIZE {
        return Err("STAT: designAxisSize is narrower than an AxisRecord".into());
    }
    if !records_fit(design_axes_offset, design_axis_count, design_axis_size, stat.len()) {
        return Err("STAT: design axis array does not fit the table".into());
    }
    let mut axes = Vec::with_capacity(design_axis_count);
    for i in 0..design_axis_count {
        let rec = design_axes_offset + i * design_axis_size;
        let tag_bytes = stat.get(rec..rec + 4).ok_or("STAT: design axis truncated")?;
        let tag      = String::from_utf8_lossy(tag_bytes).to_string();
        let name_id  = read_u16_be(stat, rec + 4).ok_or("STAT: design axis truncated")?;
        let ordering = read_u16_be(stat, rec + 6).ok_or("STAT: design axis truncated")?;
        axes.push(StatAxis { tag, name: name_of(name_id), ordering });
    }

    if !records_fit(offset_to_axis_value_offsets, axis_value_count, 2, stat.len()) {
        return Err("STAT: axis value offset array does not fit the table".into());
    }
    let mut combo_records_left = stat.len() / 6;

    let mut values = Vec::with_capacity(axis_value_count);
    for i in 0..axis_value_count {
        let off_rec = offset_to_axis_value_offsets + i * 2;
        let rel_off = read_u16_be(stat, off_rec).ok_or("STAT: axis value offset truncated")? as usize;
        let av = offset_to_axis_value_offsets + rel_off;

        let format = match read_u16_be(stat, av) { Some(f) => f, None => continue };
        // An axis index must be below designAxisCount; a value naming another axis is left out.
        let on_an_axis = |axis_index: u16| usize::from(axis_index) < design_axis_count;
        let flag = |flags: u16, bit: u16| flags & bit != 0;
        match format {
            1 => {
                let (Some(axis_index), Some(flags), Some(name_id), Some(value)) = (
                    read_u16_be(stat, av + 2), read_u16_be(stat, av + 4),
                    read_u16_be(stat, av + 6), read_fixed(stat, av + 8),
                ) else { continue };
                if !on_an_axis(axis_index) { continue; }
                values.push(StatAxisValue::Single {
                    axis_index, name: name_of(name_id), value,
                    elidable: flag(flags, ELIDABLE_AXIS_VALUE_NAME),
                    older_sibling: flag(flags, OLDER_SIBLING_FONT_ATTRIBUTE),
                });
            }
            2 => {
                let (Some(axis_index), Some(flags), Some(name_id), Some(nominal), Some(min), Some(max)) = (
                    read_u16_be(stat, av + 2), read_u16_be(stat, av + 4), read_u16_be(stat, av + 6),
                    read_fixed(stat, av + 8), read_fixed(stat, av + 12), read_fixed(stat, av + 16),
                ) else { continue };
                if !on_an_axis(axis_index) { continue; }
                values.push(StatAxisValue::Range {
                    axis_index, name: name_of(name_id), nominal, min, max,
                    elidable: flag(flags, ELIDABLE_AXIS_VALUE_NAME),
                    older_sibling: flag(flags, OLDER_SIBLING_FONT_ATTRIBUTE),
                });
            }
            3 => {
                let (Some(axis_index), Some(flags), Some(name_id), Some(value), Some(linked_value)) = (
                    read_u16_be(stat, av + 2), read_u16_be(stat, av + 4), read_u16_be(stat, av + 6),
                    read_fixed(stat, av + 8), read_fixed(stat, av + 12),
                ) else { continue };
                if !on_an_axis(axis_index) { continue; }
                values.push(StatAxisValue::Linked {
                    axis_index, name: name_of(name_id), value, linked_value,
                    elidable: flag(flags, ELIDABLE_AXIS_VALUE_NAME),
                    older_sibling: flag(flags, OLDER_SIBLING_FONT_ATTRIBUTE),
                });
            }
            4 => {
                let (Some(record_count), Some(flags), Some(name_id)) = (
                    read_u16_be(stat, av + 2), read_u16_be(stat, av + 4), read_u16_be(stat, av + 6),
                ) else { continue };
                let record_count = record_count as usize;
                if !records_fit(av.saturating_add(8), record_count, 6, stat.len()) { continue; }
                let Some(left) = combo_records_left.checked_sub(record_count) else { continue };
                combo_records_left = left;

                let mut combo_values = Vec::with_capacity(record_count);
                let mut ok = true;
                for j in 0..record_count {
                    let rec = av + 8 + j * 6;
                    match (read_u16_be(stat, rec), read_fixed(stat, rec + 2)) {
                        (Some(axis_index), Some(value)) if on_an_axis(axis_index) => {
                            combo_values.push((axis_index, value));
                        }
                        _ => { ok = false; break; }
                    }
                }
                if !ok { continue; }
                values.push(StatAxisValue::Combo {
                    name: name_of(name_id), values: combo_values,
                    elidable: flag(flags, ELIDABLE_AXIS_VALUE_NAME),
                    older_sibling: flag(flags, OLDER_SIBLING_FONT_ATTRIBUTE),
                });
            }
            _ => continue,
        }
    }

    let elided_fallback_name = if minor_version >= 1 && stat.len() >= 20 {
        read_u16_be(stat, 18).and_then(&mut name_of)
    } else {
        None
    };

    Ok((axes, values, elided_fallback_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    // Mac Roman names by ID.
    fn name_table(names: &[(u16, &[u8])]) -> Vec<u8> {
        let mut t = words(&[0, names.len() as u16, 6 + 12 * names.len() as u16]);
        let mut storage: Vec<u8> = Vec::new();
        for (id, s) in names {
            t.extend(words(&[1, 0, 0, *id, s.len() as u16, storage.len() as u16]));
            storage.extend(*s);
        }
        t.extend(storage);
        t
    }

    // STAT 1.2 over the axes `axes` (tag and name ID), its axis value offsets `offsets` counted from
    // their own start and the value tables `tables` after them.
    fn tables(
        names: Vec<u8>, axes: &[(&[u8; 4], u16)], offsets: &[u16], tables: &[u8],
    ) -> BTreeMap<String, TableBytes> {
        let values_at = 20 + 8 * axes.len() as u32;
        let mut stat = words(&[1, 2, 8, axes.len() as u16]);
        stat.extend(20u32.to_be_bytes());
        stat.extend(words(&[offsets.len() as u16]));
        stat.extend(values_at.to_be_bytes());
        stat.extend(words(&[0]));
        for (i, (tag, name)) in axes.iter().enumerate() {
            stat.extend(*tag);
            stat.extend(words(&[*name, i as u16]));
        }
        stat.extend(offsets.iter().flat_map(|o| o.to_be_bytes()));
        stat.extend(tables);
        [("name", names), ("STAT", stat)].into_iter().map(|(t, b)| (String::from(t), TableBytes::from(b))).collect()
    }

    // A format 1 value on `axis` with `flags`, named 257, at weight 700.
    fn single(axis: u16, flags: u16) -> Vec<u8> {
        words(&[1, axis, flags, 257, 700, 0])
    }

    // 1,024 values all naming one 65,535-byte name, 196,605 bytes once decoded: copied for each, they
    // would hold 201 MB, from a 2 KB STAT.
    #[test]
    fn a_long_name_named_by_every_value_is_not_copied_past_a_bound() {
        let long = alloc::vec![0xA0u8; 65_535];
        let value = words(&[1, 0, 0, 256, 700, 0]);
        let map = tables(name_table(&[(256, &long)]), &[(b"wght", 256)], &[2048; 1024], &value);
        let (axes, values, _) = parse_stat(&map).expect("a STAT");
        let value_name = |v: &StatAxisValue| match v { StatAxisValue::Single { name, .. } => name.clone(), _ => None };
        let held: usize = axes.iter().filter_map(|a| a.name.as_ref()).map(String::len).sum::<usize>()
            + values.iter().filter_map(value_name).map(|n| n.len()).sum::<usize>();
        let bound = 16 * map.get("STAT").map_or(0, |t| t.len()) + 196_605;
        assert!(held <= bound, "held {held} bytes of names, past {bound}");
        assert_eq!(values.len(), 1024);
    }

    // One design axis: a value on axis 7, or a combination naming it, is left out rather than handed
    // to a caller who would index the axes with it.
    #[test]
    fn a_value_on_no_axis_is_left_out() {
        let names = name_table(&[(256, b"Weight"), (257, b"Bold")]);
        let combo = words(&[4, 2, 0, 257, 0, 700, 0, 7, 1, 0]);
        let map = tables(names, &[(b"wght", 256)], &[6, 18, 30], &[single(0, 0), single(7, 0), combo].concat());
        let (_, values, _) = parse_stat(&map).expect("a STAT");
        assert_eq!(values.len(), 1);
        assert!(matches!(values[0], StatAxisValue::Single { axis_index: 0, .. }));
    }

    #[test]
    fn values_describing_older_siblings_say_so() {
        let names = name_table(&[(256, b"Weight"), (257, b"Bold")]);
        let map = tables(names, &[(b"wght", 256)], &[4, 16], &[single(0, 0x0001), single(0, 0x0002)].concat());
        let (_, values, _) = parse_stat(&map).expect("a STAT");
        let flags: Vec<_> = values
            .iter()
            .map(|v| match v {
                StatAxisValue::Single { elidable, older_sibling, .. } => (*elidable, *older_sibling),
                _ => (true, true),
            })
            .collect();
        assert_eq!(flags, [(false, true), (true, false)]);
    }
}
