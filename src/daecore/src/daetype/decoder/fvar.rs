use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::io::{read_u16_be, read_u32_be, records_fit};
use super::name::parse_all_name_strings;
use crate::daecore::daetype::TableBytes;

pub fn is_variable_font(table_map: &BTreeMap<String, TableBytes>) -> bool {
    table_map.get("fvar")
        .filter(|f| f.len() >= 16)
        .and_then(|f| read_u16_be(f, 8))
        .is_some_and(|count| count > 0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FvarAxis {
    pub tag: String,
    pub min: f64,
    pub default: f64,
    pub max: f64,
    pub flags: u16,
    pub name_id: u16,
}

impl FvarAxis {
    // HIDDEN_AXIS: the font asks for the axis to be kept out of user interfaces.
    pub fn is_hidden(&self) -> bool {
        self.flags & 0x0001 != 0
    }
}

pub fn parse_fvar_axes(table_map: &BTreeMap<String, TableBytes>) -> Result<Vec<FvarAxis>, String> {
    let fvar = table_map.get("fvar").ok_or("missing fvar")?;
    if fvar.len() < 16 { return Err("fvar: header truncated".into()); }

    let axes_array_offset = read_u16_be(fvar, 4).ok_or("fvar: header truncated")? as usize;
    let axis_count        = read_u16_be(fvar, 8).ok_or("fvar: header truncated")? as usize;
    let axis_size         = read_u16_be(fvar, 10).ok_or("fvar: header truncated")? as usize;

    const VARIATION_AXIS_RECORD_SIZE: usize = 20;
    if axis_count > 0 && axis_size < VARIATION_AXIS_RECORD_SIZE {
        return Err("fvar: axisSize is narrower than a VariationAxisRecord".into());
    }
    if !records_fit(axes_array_offset, axis_count, axis_size, fvar.len()) {
        return Err("fvar: axis array does not fit the table".into());
    }
    let mut axes = Vec::with_capacity(axis_count);
    for i in 0..axis_count {
        let ao = axes_array_offset + i * axis_size;
        let tag_bytes = fvar.get(ao..ao + 4).ok_or("fvar: axis record truncated")?;
        let tag     = String::from_utf8_lossy(tag_bytes).to_string();
        let min     = read_u32_be(fvar, ao + 4).ok_or("fvar: axis record truncated")?  as i32 as f64 / 65536.0;
        let default = read_u32_be(fvar, ao + 8).ok_or("fvar: axis record truncated")?  as i32 as f64 / 65536.0;
        let max     = read_u32_be(fvar, ao + 12).ok_or("fvar: axis record truncated")? as i32 as f64 / 65536.0;
        let flags   = read_u16_be(fvar, ao + 16).ok_or("fvar: axis record truncated")?;
        let name_id = read_u16_be(fvar, ao + 18).ok_or("fvar: axis record truncated")?;
        axes.push(FvarAxis { tag, min, default, max, flags, name_id });
    }
    Ok(axes)
}

#[derive(Debug)]
pub struct NamedInstance {
    pub name:            Option<String>,
    pub postscript_name: Option<String>,
    pub coords:          Vec<(String, f64)>,
}

pub fn read_fvar_instances(table_map: &BTreeMap<String, TableBytes>) -> Result<Vec<NamedInstance>, String> {
    read_fvar_instances_named(table_map, &parse_all_name_strings(table_map))
}

// The same, naming each instance from `names`, the name table a caller has already read.
pub(crate) fn read_fvar_instances_named(
    table_map: &BTreeMap<String, TableBytes>,
    names: &BTreeMap<u16, String>,
) -> Result<Vec<NamedInstance>, String> {
    let fvar = table_map.get("fvar").ok_or("missing fvar")?;
    if fvar.len() < 16 { return Err("fvar: header truncated".into()); }

    let axes_array_offset = read_u16_be(fvar, 4).ok_or("fvar: header truncated")? as usize;
    let axis_count        = read_u16_be(fvar, 8).ok_or("fvar: header truncated")? as usize;
    let axis_size         = read_u16_be(fvar, 10).ok_or("fvar: header truncated")? as usize;
    let instance_count    = read_u16_be(fvar, 12).ok_or("fvar: header truncated")? as usize;
    let instance_size     = read_u16_be(fvar, 14).ok_or("fvar: header truncated")? as usize;

    let tags: Vec<String> = parse_fvar_axes(table_map)?.into_iter().map(|a| a.tag).collect();

    // A later minor version may extend the record, so a longer one still carries the name.
    let has_postscript_name = instance_size >= 4 + axis_count * 4 + 2;
    let instance_array_offset = axis_count
        .checked_mul(axis_size)
        .and_then(|n| axes_array_offset.checked_add(n))
        .ok_or("fvar: axis array extent overflows")?;
    let min_instance_size = axis_count
        .checked_mul(4)
        .and_then(|n| n.checked_add(4))
        .ok_or("fvar: instance record extent overflows")?;
    if instance_count > 0 && instance_size < min_instance_size {
        return Err("fvar: instanceSize is narrower than an InstanceRecord".into());
    }
    if !records_fit(instance_array_offset, instance_count, instance_size, fvar.len()) {
        return Err("fvar: instance array does not fit the table".into());
    }

    let mut out = Vec::with_capacity(instance_count);
    for i in 0..instance_count {
        let rec = instance_array_offset + i * instance_size;
        let subfamily_name_id = read_u16_be(fvar, rec).ok_or("fvar: instance record truncated")?;

        let mut coords = Vec::with_capacity(axis_count);
        for (j, tag) in tags.iter().enumerate() {
            let v = read_u32_be(fvar, rec + 4 + j * 4).ok_or("fvar: instance coordinates truncated")?;
            coords.push((tag.clone(), v as i32 as f64 / 65536.0));
        }

        let postscript_name = if has_postscript_name {
            let ps_id = read_u16_be(fvar, rec + 4 + axis_count * 4).ok_or("fvar: instance record truncated")?;
            if ps_id == 0xFFFF { None } else { names.get(&ps_id).cloned() }
        } else {
            None
        };

        out.push(NamedInstance {
            name: names.get(&subfamily_name_id).cloned(),
            postscript_name,
            coords,
        });
    }

    Ok(out)
}
