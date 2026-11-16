use alloc::vec::Vec;
use crate::daecore::daetype::decoder::write_u16_be;
use crate::daecore::daetype::format::cff::encode_cff_int;
use crate::daecore::Error;

// The bytes each offset of an INDEX takes, by the largest.
fn off_size(max_off: usize) -> usize {
    if max_off <= 0xFF { 1 } else if max_off <= 0xFFFF { 2 } else if max_off <= 0xFF_FFFF { 3 } else { 4 }
}

// A CFF INDEX counts its objects in 16 bits: past 65,535 there is none to write.
pub fn encode_cff_index(objects: &[Vec<u8>]) -> Option<Vec<u8>> {
    let refs: Vec<&[u8]> = objects.iter().map(|t| t.as_slice()).collect();
    encode_cff_index_refs(&refs)
}

pub(crate) fn encode_cff_index_refs(objects: &[&[u8]]) -> Option<Vec<u8>> {
    encode_cff_index_with(|| objects.iter().copied())
}

// An INDEX of the objects `objects` gives each time it's called, sized first and written in one allocation.
pub(crate) fn encode_cff_index_with<'o, I: Iterator<Item = &'o [u8]>>(objects: impl Fn() -> I) -> Option<Vec<u8>> {
    let (count, len) = objects().fold((0, 0), |(count, len), o| (count + 1, len + o.len()));
    if count == 0 {
        return Some(alloc::vec![0, 0]);
    }
    let width = off_size(len + 1);
    let mut out = Vec::with_capacity(3 + (count + 1) * width + len);
    out.extend(u16::try_from(count).ok()?.to_be_bytes());
    out.push(width as u8);
    let ends = core::iter::once(1).chain(objects().scan(1, |end, o| { *end += o.len(); Some(*end) }));
    match width {
        1 => put_offsets::<1>(&mut out, ends),
        2 => put_offsets::<2>(&mut out, ends),
        3 => put_offsets::<3>(&mut out, ends),
        _ => put_offsets::<4>(&mut out, ends),
    }
    // A Subrs INDEX is mostly one-byte returns, which a push writes faster than a copy.
    objects().for_each(|o| if let [byte] = o { out.push(*byte) } else { out.extend_from_slice(o) });
    Some(out)
}

// Offsets `N` bytes wide, so each is a fixed-size copy.
fn put_offsets<const N: usize>(out: &mut Vec<u8>, ends: impl Iterator<Item = usize>) {
    ends.for_each(|end| out.extend_from_slice(&(end as u32).to_be_bytes()[4 - N..]));
}

pub(crate) fn cff_index_flat_size(ends: &[usize]) -> usize {
    match ends.last() {
        None => 2,
        Some(&len) => 2 + 1 + (ends.len() + 1) * off_size(len + 1) + len,
    }
}

pub(crate) fn append_cff_index_chunks(out: &mut Vec<u8>, chunks: &[&[u8]], ends: &[usize]) -> Option<()> {
    let Some(&len) = ends.last() else {
        out.extend_from_slice(&[0, 0]);
        return Some(());
    };
    let count = u16::try_from(ends.len()).ok()?;
    let width = off_size(len + 1);
    let start = out.len();
    let header_size = 2 + 1 + (ends.len() + 1) * width;
    out.resize(start + header_size, 0);

    write_u16_be(out, start, count);
    out[start + 2] = width as u8;

    let write_off = |out: &mut Vec<u8>, i: usize, o: usize| {
        let pos = start + 3 + i * width;
        out[pos..pos + width].copy_from_slice(&(o as u32).to_be_bytes()[4 - width..]);
    };
    write_off(out, 0, 1);
    for (i, &end) in ends.iter().enumerate() {
        write_off(out, i + 1, end + 1);
    }

    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    debug_assert_eq!(out.len() - start, cff_index_flat_size(ends));
    Some(())
}

pub(crate) const TOO_MANY: Error = Error::limit("CFF: an INDEX of more than 65,535 objects");

// The fonts a CFF table's glyphs draw with: one Private DICT for them all, or CID-keyed, an FDSelect
// and each FD's DICT entries with the Private DICT it names.
pub(crate) enum Fonts<'a> {
    Named,
    Cid { fd_select: &'a [u8], fds: Vec<(&'a [u8], usize)> },
}

// What a CFF table is written from, each part in the order it is laid out.
pub(crate) struct CffParts<'a> {
    pub header: &'a [u8],
    pub names: &'a [u8],
    // The Top DICT's entries, but for the offsets written here.
    pub top: &'a [u8],
    pub strings: &'a [u8],
    pub gsubrs: &'a [u8],
    // A charset's bytes, or none to name the predefined one.
    pub charset: &'a [u8],
    pub predefined: u8,
    pub charstrings: (&'a [&'a [u8]], &'a [usize]),
    pub fonts: Fonts<'a>,
    // Each Private DICT once, then the local Subrs it names at its own length.
    pub privates: Vec<(&'a [u8], &'a [u8])>,
}

#[derive(Default)]
struct Offsets {
    charset: usize,
    fd_select: usize,
    charstrings: usize,
    fd_array: usize,
    privates: Vec<usize>,
}

// A size or offset as a five-byte int, refused past what one holds.
fn checked_int(v: usize) -> Result<Vec<u8>, Error> {
    i32::try_from(v).map(encode_cff_int).map_err(|_| Error::limit("CFF: an offset past 2 GB"))
}

impl CffParts<'_> {
    fn top_dict(&self, at: &Offsets) -> Result<Vec<u8>, Error> {
        let mut dict = self.top.to_vec();
        dict.extend(if self.charset.is_empty() {
            encode_cff_int(i32::from(self.predefined))
        } else {
            checked_int(at.charset)?
        });
        dict.push(15);
        dict.extend(checked_int(at.charstrings)?);
        dict.push(17);
        match self.fonts {
            Fonts::Named => {
                let (private, _) = self.privates.first().ok_or(Error::malformed("CFF: no Private DICT"))?;
                dict.extend(checked_int(private.len())?);
                dict.extend(checked_int(at.privates[0])?);
                dict.push(18);
            }
            Fonts::Cid { .. } => {
                dict.extend(checked_int(at.fd_array)?);
                dict.extend([12, 36]);
                dict.extend(checked_int(at.fd_select)?);
                dict.extend([12, 37]);
            }
        }
        Ok(dict)
    }

    fn fd_array(&self, at: &Offsets) -> Result<Vec<u8>, Error> {
        let Fonts::Cid { fds, .. } = &self.fonts else { return Ok(Vec::new()) };
        let dicts = fds.iter()
            .map(|&(entries, private)| {
                let (dict, _) =
                    self.privates.get(private).ok_or(Error::malformed("CFF: an FD naming no Private DICT"))?;
                Ok([entries, &checked_int(dict.len())?, &checked_int(at.privates[private])?, &[18]].concat())
            })
            .collect::<Result<Vec<Vec<u8>>, Error>>()?;
        encode_cff_index(&dicts).ok_or(TOO_MANY)
    }
}

// Every offset a DICT here names is a five-byte int, so a DICT written with each one 0 is as long as
// the one written last, and the layout follows from it.
pub(crate) fn write_cff(parts: &CffParts<'_>) -> Result<Vec<u8>, Error> {
    let fd_select = match parts.fonts {
        Fonts::Cid { fd_select, .. } => fd_select,
        Fonts::Named => &[],
    };
    let mut at = Offsets { privates: vec![0; parts.privates.len()], ..Offsets::default() };
    let top_len = parts.top_dict(&at)?.len();
    let fd_array_len = parts.fd_array(&at)?.len();
    at.charset = parts.header.len()
        + parts.names.len()
        + cff_index_flat_size(&[top_len])
        + parts.strings.len()
        + parts.gsubrs.len();
    at.fd_select = at.charset + parts.charset.len();
    at.charstrings = at.fd_select + fd_select.len();
    at.fd_array = at.charstrings + cff_index_flat_size(parts.charstrings.1);
    let mut end = at.fd_array + fd_array_len;
    for (slot, (dict, subrs)) in at.privates.iter_mut().zip(&parts.privates) {
        *slot = end;
        end += dict.len() + subrs.len();
    }

    let mut out = Vec::with_capacity(end);
    out.extend_from_slice(parts.header);
    out.extend_from_slice(parts.names);
    out.extend(encode_cff_index(&[parts.top_dict(&at)?]).ok_or(TOO_MANY)?);
    out.extend_from_slice(parts.strings);
    out.extend_from_slice(parts.gsubrs);
    out.extend_from_slice(parts.charset);
    out.extend_from_slice(fd_select);
    append_cff_index_chunks(&mut out, parts.charstrings.0, parts.charstrings.1).ok_or(TOO_MANY)?;
    out.extend(parts.fd_array(&at)?);
    for (dict, subrs) in &parts.privates {
        out.extend_from_slice(dict);
        out.extend_from_slice(subrs);
    }
    debug_assert_eq!(out.len(), end);
    Ok(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/cff/build.rs"]
mod tests;
