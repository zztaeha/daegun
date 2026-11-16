use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::num_glyphs;
use crate::daecore::daetype::format::sanitize::varc_passes;
use crate::daecore::daetype::outline::Path;
use crate::daecore::daetype::subsetter::subset_budget;
use crate::daecore::daetype::varc::subset::{covered_glyphs, subset, Subset};
use crate::daecore::daetype::Tables;
use crate::daecore::Error;
use super::AxisTag;

// VARC in a whole instance: the table written with every axis pinned, or None where it keeps no glyph, fails the
// sanitizer or is refused; where refused, each glyph drawn for the instance's outlines.
pub(crate) struct VarcInstance {
    pub(crate) table: Option<Vec<u8>>,
    pub(crate) drawn: BTreeMap<u32, Path>,
}

impl VarcInstance {
    // The instance's tables with VARC as kept, or without it.
    pub(crate) fn apply(self, out_map: &mut BTreeMap<String, Cow<'_, [u8]>>) {
        match self.table {
            Some(table) => out_map.insert("VARC".into(), Cow::Owned(table)),
            None => out_map.remove("VARC"),
        };
    }
}

// None for a font without VARC; an error where the glyphs drawn run past their budget.
pub(crate) fn varc_instance(
    table_map: &Tables, axis_values: &[(AxisTag, f64)],
) -> Result<Option<VarcInstance>, Error> {
    let Some(varc) = table_map.get("VARC") else { return Ok(None) };
    let n = num_glyphs(table_map).unwrap_or(0);
    let (table, drawn) = match varc_passes(varc).then(|| subset(varc, &(0..n).collect(), Some, true, true)) {
        Some(Subset::Written(table)) => (Some(table), BTreeMap::new()),
        Some(Subset::Refused) => {
            let gids: Vec<u32> = covered_glyphs(varc).into_iter().filter(|&g| g < n).collect();
            (None, crate::daecore::daekit::cache::varc_drawn_from(table_map, axis_values, &gids)?)
        }
        Some(Subset::Unkept) | None => (None, BTreeMap::new()),
    };
    Ok(Some(VarcInstance { table, drawn }))
}

// `glyf`'s glyphs, as `loca` places them, with each of `drawn` in its glyph's place: every glyph through `count`,
// each padded to an even length. Glyphs can share their bytes, so those copied from `glyf` are held to the subset
// budget, as a subset's copy is.
pub(crate) fn with_drawn(
    glyf: &[u8], loca: &[usize], count: usize, drawn: &BTreeMap<u32, Vec<u8>>,
) -> Result<(Vec<u8>, Vec<usize>), Error> {
    let (mut out, mut ends) = (Vec::with_capacity(glyf.len()), Vec::with_capacity(count + 1));
    let mut left = subset_budget(glyf.len());
    ends.push(0);
    for gid in 0..count {
        let glyph = match drawn.get(&(gid as u32)) {
            Some(glyph) => glyph,
            None => {
                let own = loca.get(gid..gid + 2).and_then(|r| glyf.get(r[0]..r[1])).unwrap_or_default();
                let past = Error::limit("instance: glyf ranges past the subset budget");
                left = left.checked_sub(own.len()).ok_or(past)?;
                own
            }
        };
        out.extend_from_slice(glyph);
        out.resize(out.len().next_multiple_of(2), 0);
        ends.push(out.len());
    }
    Ok((out, ends))
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/varc.rs"]
mod tests;
