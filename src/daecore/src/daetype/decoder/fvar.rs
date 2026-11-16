use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::io::{read_fixed, read_u16_be, records_fit};
use super::name::parse_all_name_strings;
use crate::daecore::daetype::Tables;
use crate::daecore::{Error, NameId, Tag};

pub fn is_variable_font(table_map: &Tables) -> bool {
    table_map.get("fvar")
        .filter(|f| f.len() >= 16)
        .and_then(|f| read_u16_be(f, 8))
        .is_some_and(|count| count > 0)
}

/// One of a variable font's axes, as fvar lists it: user coordinates, as [`Font::location`](crate::Font::location)
/// takes them.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    /// What the axis varies: `wght`, `wdth`, `opsz`, or a font's own.
    pub tag: Tag,
    /// The lowest value it takes.
    pub min: f64,
    /// Its value at the default instance.
    pub default: f64,
    /// The highest value it takes.
    pub max: f64,
    /// The name table's ID for its name.
    pub name_id: NameId,
    /// Whether the font asks for the axis to be kept out of user interfaces (fvar's HIDDEN_AXIS flag).
    #[doc(alias("is_hidden"))]
    pub hidden: bool,
}

// fvar's HIDDEN_AXIS flag.
const AXIS_HIDDEN: u16 = 0x0001;

pub fn parse_fvar_axes(table_map: &Tables) -> Result<Vec<Axis>, Error> {
    let axis = |a: AxisRecord| Axis {
        tag: Tag::new(&a.tag),
        min: a.min,
        default: a.default,
        max: a.max,
        name_id: NameId(a.name_id),
        hidden: a.flags & AXIS_HIDDEN != 0,
    };
    Ok(axis_records(table_map.get("fvar").ok_or(Error::missing("missing fvar"))?)?.map(axis).collect())
}

// An axis record as fvar holds it, its tag as bytes.
pub(crate) struct AxisRecord {
    pub(crate) tag: [u8; 4],
    pub(crate) min: f64,
    pub(crate) default: f64,
    pub(crate) max: f64,
    pub(crate) flags: u16,
    pub(crate) name_id: u16,
}

pub(crate) fn axis_records(fvar: &[u8]) -> Result<impl ExactSizeIterator<Item = AxisRecord> + '_, Error> {
    if fvar.len() < 16 { return Err(Error::malformed("fvar: header truncated")); }

    let axes_array_offset = read_u16_be(fvar, 4).ok_or(Error::malformed("fvar: header truncated"))? as usize;
    let axis_count        = read_u16_be(fvar, 8).ok_or(Error::malformed("fvar: header truncated"))? as usize;
    let axis_size         = read_u16_be(fvar, 10).ok_or(Error::malformed("fvar: header truncated"))? as usize;

    const VARIATION_AXIS_RECORD_SIZE: usize = 20;
    if axis_count > 0 && axis_size < VARIATION_AXIS_RECORD_SIZE {
        return Err(Error::malformed("fvar: axisSize is narrower than a VariationAxisRecord"));
    }
    if !records_fit(axes_array_offset, axis_count, axis_size, fvar.len()) {
        return Err(Error::malformed("fvar: axis array does not fit the table"));
    }
    // Every record fits the table whole, so no field of one reads short.
    Ok((0..axis_count).map(move |i| {
        let ao = axes_array_offset + i * axis_size;
        let fixed = |at| read_fixed(fvar, ao + at).unwrap_or_default();
        let word = |at| read_u16_be(fvar, ao + at).unwrap_or(0);
        let tag = fvar.get(ao..ao + 4).and_then(|t| t.try_into().ok()).unwrap_or_default();
        AxisRecord { tag, min: fixed(4), default: fixed(8), max: fixed(12), flags: word(16), name_id: word(18) }
    }))
}

/// One of the instances fvar names, a point in the design space with a name of its own.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct NamedInstance {
    /// Its subfamily name ("Bold Condensed"), none where the name table lacks it.
    pub name: Option<String>,
    /// Its PostScript name, none where fvar or the name table gives none.
    pub postscript_name: Option<String>,
    /// Its coordinate on each axis, in fvar's order, for [`Font::location`](crate::Font::location).
    pub coords: Vec<(Tag, f64)>,
}

pub fn read_fvar_instances(table_map: &Tables) -> Result<Vec<NamedInstance>, Error> {
    let names: BTreeMap<u16, String> = parse_all_name_strings(table_map);
    let name = |id: u16| names.get(&id).cloned();
    let named = |r: InstanceRecord| NamedInstance {
        name: name(r.subfamily),
        postscript_name: r.postscript.and_then(name),
        coords: r.coords,
    };
    Ok(instance_records(table_map)?.into_iter().map(named).collect())
}

// An instance as fvar holds it: its subfamily's and PostScript name's IDs, and its coordinates.
pub(crate) struct InstanceRecord {
    pub(crate) subfamily: u16,
    pub(crate) postscript: Option<u16>,
    pub(crate) coords: Vec<(Tag, f64)>,
}

pub(crate) fn instance_records(table_map: &Tables) -> Result<Vec<InstanceRecord>, Error> {
    let fvar = table_map.get("fvar").ok_or(Error::missing("missing fvar"))?;
    if fvar.len() < 16 { return Err(Error::malformed("fvar: header truncated")); }

    let axes_array_offset = read_u16_be(fvar, 4).ok_or(Error::malformed("fvar: header truncated"))? as usize;
    let axis_count        = read_u16_be(fvar, 8).ok_or(Error::malformed("fvar: header truncated"))? as usize;
    let axis_size         = read_u16_be(fvar, 10).ok_or(Error::malformed("fvar: header truncated"))? as usize;
    let instance_count    = read_u16_be(fvar, 12).ok_or(Error::malformed("fvar: header truncated"))? as usize;
    let instance_size     = read_u16_be(fvar, 14).ok_or(Error::malformed("fvar: header truncated"))? as usize;

    let tags: Vec<Tag> = parse_fvar_axes(table_map)?.into_iter().map(|a| a.tag).collect();

    // A later minor version may extend the record, so a longer one still carries the name.
    let has_postscript_name = instance_size >= 4 + axis_count * 4 + 2;
    let instance_array_offset = axis_count
        .checked_mul(axis_size)
        .and_then(|n| axes_array_offset.checked_add(n))
        .ok_or(Error::malformed("fvar: axis array extent overflows"))?;
    let min_instance_size = axis_count
        .checked_mul(4)
        .and_then(|n| n.checked_add(4))
        .ok_or(Error::malformed("fvar: instance record extent overflows"))?;
    if instance_count > 0 && instance_size < min_instance_size {
        return Err(Error::malformed("fvar: instanceSize is narrower than an InstanceRecord"));
    }
    if !records_fit(instance_array_offset, instance_count, instance_size, fvar.len()) {
        return Err(Error::malformed("fvar: instance array does not fit the table"));
    }

    let mut out = Vec::with_capacity(instance_count);
    for i in 0..instance_count {
        let rec = instance_array_offset + i * instance_size;
        let subfamily_name_id = read_u16_be(fvar, rec).ok_or(Error::malformed("fvar: instance record truncated"))?;

        let mut coords = Vec::with_capacity(axis_count);
        for (j, tag) in tags.iter().enumerate() {
            let v = read_fixed(fvar, rec + 4 + j * 4).ok_or(Error::malformed("fvar: instance coordinates truncated"))?;
            coords.push((*tag, v));
        }

        let postscript = if has_postscript_name {
            let ps_id = read_u16_be(fvar, rec + 4 + axis_count * 4)
                .ok_or(Error::malformed("fvar: instance record truncated"))?;
            (ps_id != 0xFFFF).then_some(ps_id)
        } else {
            None
        };

        out.push(InstanceRecord { subfamily: subfamily_name_id, postscript, coords });
    }

    Ok(out)
}
