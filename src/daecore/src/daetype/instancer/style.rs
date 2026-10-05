#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{parse_all_name_strings, parse_fvar_axes, read_fvar_instances_named, read_u16_be, write_u16_be};
use super::name_table::rewrite_name_table;
use super::stat_filter::filter_stat_to_instance;
use crate::daecore::daetype::TableBytes;

const FS_ITALIC: u16 = 0x0001;
const FS_BOLD: u16 = 0x0020;
const FS_REGULAR: u16 = 0x0040;
const FS_OBLIQUE: u16 = 0x0200;

const MAC_BOLD: u16 = 0x0001;
const MAC_ITALIC: u16 = 0x0002;

const BOLD_WEIGHT: f64 = 700.0;

const WIDTH_CLASSES: [(u16, f64); 9] = [
    (1, 50.0), (2, 62.5), (3, 75.0), (4, 87.5), (5, 100.0),
    (6, 112.5), (7, 125.0), (8, 150.0), (9, 200.0),
];

pub(crate) struct StyleTables {
    pub(crate) head: Option<Vec<u8>>,
    pub(crate) name: Option<Vec<u8>>,
    pub(crate) stat: Option<Vec<u8>>,
}

fn width_class_for(percentage: f64) -> u16 {
    WIDTH_CLASSES
        .iter()
        .min_by(|(_, a), (_, b)| {
            (a - percentage).abs().total_cmp(&(b - percentage).abs())
        })
        .map_or(5, |&(class, _)| class)
}

fn ribbi_name(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (true, true) => "Bold Italic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (false, false) => "Regular",
    }
}

fn non_ribbi_remainder(subfamily: &str) -> String {
    subfamily
        .split_whitespace()
        .filter(|word| !matches!(*word, "Bold" | "Italic" | "Regular" | "Oblique"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn effective_location(
    table_map: &BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let Ok(axes) = parse_fvar_axes(table_map) {
        for axis in axes {
            let requested = axis_values.iter().find(|(tag, _)| *tag == axis.tag).map(|(_, v)| *v);
            let (lo, hi) = (axis.min.min(axis.max), axis.max.max(axis.min));
            out.insert(axis.tag, requested.unwrap_or(axis.default).clamp(lo, hi));
        }
    }
    out
}

fn matching_named_instance(
    table_map: &BTreeMap<String, TableBytes>,
    location: &BTreeMap<String, f64>,
    names: &BTreeMap<u16, String>,
) -> Option<(String, Option<String>)> {
    let instances = read_fvar_instances_named(table_map, names).ok()?;
    instances.into_iter().find_map(|instance| {
        let all_match = instance.coords.iter().all(|(tag, value)| {
            location.get(tag).is_some_and(|&coord| (coord - value).abs() < 1.0 / 65536.0)
        });
        match (all_match && !instance.coords.is_empty(), instance.name) {
            (true, Some(name)) => Some((name, instance.postscript_name)),
            _ => None,
        }
    })
}

pub(crate) fn apply_style_metadata(
    table_map: &BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
    os2_data: &mut [u8],
    post: &mut [u8],
) -> StyleTables {
    let location = effective_location(table_map, axis_values);
    if location.is_empty() {
        return StyleTables { head: None, name: None, stat: None };
    }

    // The location the font is drawn at, so a request past an axis's range names what it gets.
    if let (Some(&weight), true) = (location.get("wght"), os2_data.len() >= 6) {
        write_u16_be(os2_data, 4, weight.round().clamp(1.0, 1000.0) as u16);
    }
    if let (Some(&width), true) = (location.get("wdth"), os2_data.len() >= 8) {
        write_u16_be(os2_data, 6, width_class_for(width));
    }
    // MVAR cannot vary italicAngle, and the spec offers slnt for it.
    if let (Some(&slant), true) = (location.get("slnt"), post.len() >= 8) {
        let fixed = (slant.clamp(-90.0, 90.0) * 65536.0).round() as i32;
        post[4..8].copy_from_slice(&fixed.to_be_bytes());
    }

    let source_fs = if os2_data.len() >= 64 { read_u16_be(os2_data, 62).unwrap_or(0) } else { 0 };
    let oblique = match location.get("slnt") {
        Some(&s) => s != 0.0,
        None => source_fs & FS_OBLIQUE != 0,
    };
    let names = parse_all_name_strings(table_map);
    let (bold, italic, updates, removals) = match matching_named_instance(table_map, &location, &names) {
        Some((subfamily, postscript)) => {
            let heavy = match location.get("wght") {
                Some(&w) => w >= BOLD_WEIGHT,
                None => source_fs & FS_BOLD != 0,
            };
            let italic = match (location.get("ital"), location.get("slnt")) {
                (None, None) => source_fs & FS_ITALIC != 0,
                (ital, _) => ital.is_some_and(|&i| i >= 0.5) || oblique,
            };
            // A weight the subfamily names beside Bold moves into name 1, as the spec's Arial
            // Black example has it, so name 2 and the bits are Regular or Italic.
            let remainder = non_ribbi_remainder(&subfamily);
            let bold = heavy && remainder.is_empty();
            let postscript = postscript.unwrap_or_else(|| {
                format!("{}-{}", postscript_prefix(&names), alphanumeric(&subfamily))
            });
            let (updates, removals) = named_instance_names(&names, &subfamily, &remainder, postscript, bold, italic, os2_data);
            (bold, italic, updates, removals)
        }
        None => {
            // The names stay the variable font's, so the bits follow the subfamily they keep.
            let kept = names.get(&2).map_or("", String::as_str);
            let bold = kept.split_whitespace().any(|w| w == "Bold");
            let italic = kept.split_whitespace().any(|w| w == "Italic" || w == "Oblique");
            let mut updates = Vec::new();
            if let Some(postscript) = arbitrary_postscript(&postscript_prefix(&names), table_map, &location) {
                updates.extend(unique_id(&names, None, &postscript, os2_data).map(|id| (3, id)));
                updates.push((6, postscript));
            }
            (bold, italic, updates, alloc::vec![25])
        }
    };

    if os2_data.len() >= 64 {
        let version = read_u16_be(os2_data, 0).unwrap_or(0);
        let mut fs = source_fs;
        if bold { fs |= FS_BOLD; } else { fs &= !FS_BOLD; }
        if italic { fs |= FS_ITALIC; } else { fs &= !FS_ITALIC; }
        if version >= 4 {
            if oblique { fs |= FS_OBLIQUE; } else { fs &= !FS_OBLIQUE; }
        }
        if bold || italic { fs &= !FS_REGULAR; } else { fs |= FS_REGULAR; }
        write_u16_be(os2_data, 62, fs);
    }

    let head = table_map.get("head").filter(|h| h.len() >= 46).and_then(|h| {
        let mut out = h.to_owned_vec();
        let mut mac = read_u16_be(&out, 44)?;
        if bold { mac |= MAC_BOLD; } else { mac &= !MAC_BOLD; }
        if italic { mac |= MAC_ITALIC; } else { mac &= !MAC_ITALIC; }
        write_u16_be(&mut out, 44, mac);
        Some(out)
    });

    let name = table_map.get("name").and_then(|name| rewrite_name_table(name, &updates, &removals));
    let stat = table_map.get("STAT").map(|stat| {
        filter_stat_to_instance(stat, &location).unwrap_or_else(|| stat.to_owned_vec())
    });

    StyleTables { head, name, stat }
}

fn named_instance_names(
    names: &BTreeMap<u16, String>,
    subfamily: &str,
    remainder: &str,
    postscript: String,
    bold: bool,
    italic: bool,
    os2: &[u8],
) -> (Vec<(u16, String)>, Vec<u16>) {
    let family = names.get(&16).or_else(|| names.get(&1)).cloned().unwrap_or_default();
    let ribbi = ribbi_name(bold, italic);
    let mut updates: Vec<(u16, String)> = Vec::new();
    let mut removals = alloc::vec![25];
    if remainder.is_empty() {
        updates.push((1, family.clone()));
        removals.extend([16, 17]);
    } else {
        updates.push((1, format!("{family} {remainder}")));
        updates.push((16, family.clone()));
        updates.push((17, subfamily.to_string()));
    }
    updates.push((2, ribbi.to_string()));
    let full = format!("{family} {subfamily}");
    updates.extend(unique_id(names, Some(&full), &postscript, os2).map(|id| (3, id)));
    updates.push((4, full));
    updates.push((6, postscript));
    (updates, removals)
}

// Adobe TN #5902: name 25 if the font has one, else the family, keeping only ASCII letters and digits.
fn postscript_prefix(names: &BTreeMap<u16, String>) -> String {
    match names.get(&25) {
        Some(prefix) => prefix.clone(),
        None => alphanumeric(names.get(&16).or_else(|| names.get(&1)).map_or("", String::as_str)),
    }
}

fn alphanumeric(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).collect()
}

// TN #5902 for a location no named instance has: each axis off its default as "_", the value and
// the tag; past 127 characters, the prefix, "-", a hash and "...". The default location keeps the
// variable font's own name, which describes its default instance.
fn arbitrary_postscript(prefix: &str, table_map: &BTreeMap<String, TableBytes>, location: &BTreeMap<String, f64>) -> Option<String> {
    let axes = parse_fvar_axes(table_map).unwrap_or_default();
    let off_default: Vec<_> = axes.iter().filter(|a| location.get(&a.tag).is_some_and(|&v| v != a.default)).collect();
    if off_default.is_empty() {
        return None;
    }
    let mut name = String::from(prefix);
    for axis in off_default {
        let value = location.get(&axis.tag).copied().unwrap_or(axis.default);
        name.push('_');
        name.push_str(&shortest_fixed(value));
        name.push_str(axis.tag.trim_end_matches(' '));
    }
    if name.len() <= 127 {
        return Some(name);
    }
    let hash = name.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    Some(format!("{prefix}-{hash:08X}..."))
}

// The fewest decimals that read back as the same 16.16 value, with no trailing zeros.
fn shortest_fixed(value: f64) -> String {
    let fixed = (value * 65536.0).round();
    for places in 0..6 {
        let text = format!("{:.*}", places, fixed / 65536.0);
        if text.parse::<f64>().is_ok_and(|v| (v * 65536.0).round() == fixed) {
            let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
            return if text == "-0" { "0".into() } else { text.into() };
        }
    }
    format!("{}", fixed / 65536.0)
}

// As fontTools rewrites the unique ID: the old full or PostScript name inside it replaced, or else
// version, vendor and PostScript name.
fn unique_id(names: &BTreeMap<u16, String>, full: Option<&str>, postscript: &str, os2: &[u8]) -> Option<String> {
    let current = names.get(&3)?;
    let replacements = [(names.get(&4), full), (names.get(&6), Some(postscript))];
    for (old, new) in replacements {
        if let (Some(old), Some(new)) = (old, new)
            && !old.is_empty()
            && current.contains(old.as_str())
        {
            return Some(current.replace(old.as_str(), new));
        }
    }
    let version = names.get(&5).map_or("", |v| v.split(';').next().unwrap_or("").trim_start_matches("Version ").trim());
    let vendor: String = os2.get(58..62).map_or(String::new(), |v| v.iter().filter(|b| b.is_ascii_graphic()).map(|&b| char::from(b)).collect());
    Some(format!("{version};{vendor};{postscript}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn the_slant_sets_the_italic_angle() {
        let mut fvar = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
        fvar.extend(b"slnt");
        fvar.extend([0xFF, 0xF1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
        let map: BTreeMap<String, TableBytes> = [("fvar".to_string(), TableBytes::from_vec(fvar))].into_iter().collect();
        for (asked, angle) in [(-10.0, -10), (-20.0, -15)] {
            let mut post = vec![0u8; 32];
            apply_style_metadata(&map, &[("slnt".into(), asked)], &mut [], &mut post);
            assert_eq!(i32::from_be_bytes(post[4..8].try_into().unwrap()), angle << 16, "asked for {asked}");
        }
    }
}
