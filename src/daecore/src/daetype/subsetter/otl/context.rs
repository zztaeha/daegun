use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::read_uint;
use super::{parse_coverage, build_coverage};
use super::super::remap_gid;
use super::generic::Work;
use crate::daecore::Error;

// `count` coverages at offsets of `width` bytes from `base`, each glyph held charged to `work`, as one coverage named
// many times is held at each naming.
pub(crate) fn parse_coverage_array(
    buf: &[u8], base: usize, array_off: usize, count: usize, width: usize, work: &mut Work,
) -> Result<Vec<Vec<u32>>, Error> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let rel = read_uint(buf, array_off + i * width, width)
            .ok_or(Error::malformed("Coverage array: offset truncated"))? as usize;
        let coverage = parse_coverage(buf, base + rel)?;
        work.spend(coverage.len())?;
        out.push(coverage);
    }
    Ok(out)
}

pub(crate) fn filter_coverage_group(group: &[Vec<u32>], active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<Vec<u32>>> {
    group.iter().map(|pos| {
        let surviving: Vec<u32> = pos.iter().filter_map(|&g| remap_gid(active, gid_map, g)).collect();
        if surviving.is_empty() { None } else { Some(surviving) }
    }).collect()
}

pub(crate) fn build_coverage_array_blobs(positions: &[Vec<u32>]) -> Vec<Vec<u8>> {
    positions.iter().map(|p| build_coverage(p)).collect()
}
