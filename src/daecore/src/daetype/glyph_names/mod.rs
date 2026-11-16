mod tables;

use alloc::vec::Vec;

use self::tables::{CFF_STANDARD_STRINGS, MACINTOSH_NAMES};
use super::format::cff::Charset;
use super::subsetter::cff::expert_charsets::predefined_charset_sid;
use super::subsetter::cff::parse::read_header;

// A CFF SID below this names a standard string, and from it on a string of the font's own.
pub(crate) const N_STD_STRINGS: u16 = CFF_STANDARD_STRINGS.len() as u16;

// The post chapter's limit, at version 2.0: "Names must be no longer than 63 characters". A longer one, from post or
// CFF, is taken as absent.
pub const MAX_GLYPH_NAME: usize = 63;

// Where a face's names lie, found once so a glyph's name is read where it is rather than walked to: where each of post
// 2.0's own strings starts, and a non-CID CFF's String INDEX with each glyph's SID.
#[derive(Default)]
pub struct NameIndex {
    strings: Vec<u32>,
    cff: Option<CffSids>,
}

struct CffSids {
    strings: usize,
    sids: Vec<u16>,
}

impl NameIndex {
    pub fn new(post: Option<&[u8]>, cff: Option<&[u8]>) -> NameIndex {
        NameIndex { strings: post.map_or_else(Vec::new, post_strings), cff: cff.and_then(cff_sids) }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.strings) + self.cff.as_ref().map_or(0, |c| heap_of(&c.sids))
    }
}

// A post name where post gives one, else the CFF charset's. `index` is the face's, built from these tables.
pub fn glyph_name<'a>(
    index: &NameIndex, post: Option<&'a [u8]>, cff: Option<&'a [u8]>, n_glyphs: u32, gid: u32,
) -> Option<&'a str> {
    post.and_then(|p| post_glyph_name(p, &index.strings, n_glyphs, gid))
        .or_else(|| cff_glyph_name(cff?, index.cff.as_ref()?, gid))
}

pub fn glyph_names<'a>(
    index: &NameIndex, post: Option<&'a [u8]>, cff: Option<&'a [u8]>, n_glyphs: u32,
) -> impl ExactSizeIterator<Item = Option<&'a str>> + DoubleEndedIterator + core::iter::FusedIterator + Clone {
    (0..n_glyphs).map(move |g| glyph_name(index, post, cff, n_glyphs, g))
}

// Versions 1.0 (the 258 standard Macintosh names, for a font of exactly those 258 glyphs), 2.0, and
// 2.5 (each glyph's offset into the standard order).
fn post_glyph_name<'a>(post: &'a [u8], strings: &[u32], n_glyphs: u32, gid: u32) -> Option<&'a str> {
    let standard = |i: usize| MACINTOSH_NAMES.get(i).copied();
    match read_u32(post, 0)? {
        0x0001_0000 if n_glyphs == 258 => standard(gid as usize),
        0x0002_5000 => {
            let count = read_u16(post, 32)?;
            if gid >= u32::from(count) || count > 258 {
                return None;
            }
            let offset = i32::from(*post.get(34 + gid as usize)? as i8);
            standard(usize::try_from(gid as i32 + offset).ok()?)
        }
        0x0002_0000 => {
            let count = read_u16(post, 32)?;
            if gid >= u32::from(count) {
                return None;
            }
            let index = usize::from(read_u16(post, 34 + gid as usize * 2)?);
            if let Some(name) = standard(index) {
                return Some(name);
            }
            let at = *strings.get(index - MACINTOSH_NAMES.len())? as usize;
            name_of(post.get(at + 1..at + 1 + usize::from(*post.get(at)?))?)
        }
        _ => None,
    }
}

// post 2.0's own strings by where each length byte is, as far as any of post's entries names, past maxp's count too,
// short of the first that runs off the table. Not to the end: one-byte strings would let an N-byte post ask for 4N.
fn post_strings(post: &[u8]) -> Vec<u32> {
    let Some(count) = read_u16(post, 32).filter(|_| read_u32(post, 0) == Some(0x0002_0000)) else { return Vec::new() };
    let wanted = (0..usize::from(count))
        .map_while(|g| read_u16(post, 34 + g * 2))
        .filter_map(|i| usize::from(i).checked_sub(MACINTOSH_NAMES.len()))
        .max();
    let (mut starts, mut at) = (Vec::new(), 34 + usize::from(count) * 2);
    for _ in 0..wanted.map_or(0, |w| w + 1) {
        let Some(&len) = post.get(at) else { break };
        let (Ok(start), Some(_)) = (u32::try_from(at), post.get(at + 1..at + 1 + usize::from(len))) else { break };
        starts.push(start);
        at += 1 + usize::from(len);
    }
    starts
}

// A non-CID CFF's String INDEX and each glyph's SID from glyph 0 on, as far as the charset reads: one cut short
// names the glyphs read before the cut. The SIDs are no more than CharStrings counts, which its bytes hold.
fn cff_sids(cff: &[u8]) -> Option<CffSids> {
    let header = read_header(cff).ok()?;
    if header.top.ros {
        return None;
    }
    super::subsetter::cff_index_end(cff, header.charstrings, false).ok()?;
    let covered = super::subsetter::cff_index_count(cff, header.charstrings, false).ok()?;
    let sids = match header.top.charset() {
        Charset::At(off) => {
            let mut sids = Vec::with_capacity(covered);
            sids.extend(&[0][..covered.min(1)]);
            let _ = super::subsetter::walk_charset(cff, off, covered, |_, sid| sids.push(sid));
            sids
        }
        Charset::Predefined(id) => {
            (0..covered).map_while(|g| predefined_charset_sid(id, u32::try_from(g).ok()?, covered)).collect()
        }
    };
    Some(CffSids { strings: header.strings, sids })
}

fn cff_glyph_name<'a>(cff: &'a [u8], kept: &CffSids, gid: u32) -> Option<&'a str> {
    let sid = *kept.sids.get(gid as usize)?;
    if let Some(&name) = CFF_STANDARD_STRINGS.get(usize::from(sid)) {
        return Some(name);
    }
    name_of(super::subsetter::cff_index_entry(cff, kept.strings, usize::from(sid - N_STD_STRINGS))?)
}

// A name as the font spells it: UTF-8, not empty, no longer than MAX_GLYPH_NAME.
fn name_of(text: &[u8]) -> Option<&str> {
    let name = core::str::from_utf8(text).ok()?;
    (!name.is_empty() && name.len() <= MAX_GLYPH_NAME).then_some(name)
}

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    super::decoder::read_u16_be(data, off)
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    super::decoder::read_u32_be(data, off)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/glyph_names/mod.rs"]
mod tests;
