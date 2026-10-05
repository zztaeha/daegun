mod tables;

use alloc::string::String;
use alloc::vec::Vec;

use self::tables::{CFF_STANDARD_STRINGS, MACINTOSH_NAMES};

const N_STD_STRINGS: u16 = 391;

// The post table's limit on a name; a longer one, from post or CFF, is taken as absent. A font can
// give every glyph the same long string at no cost to itself, and each name handed out is a copy.
const MAX_NAME: usize = 63;

// ISOAdobe, Expert and Expert Subset cover 229, 166 and 87 glyphs. A font with more refers to names
// the charset does not have, and FreeType refuses it, so none of its names are read.
const ISO_ADOBE_GLYPHS: usize = 229;

// A post name where post gives one, else the CFF charset's, as `glyph_names` resolves each glyph.
pub fn glyph_name(post: Option<&[u8]>, cff: Option<&[u8]>, n_glyphs: u16, gid: u16) -> Option<String> {
    post.and_then(|p| post_glyph_name(p, n_glyphs, gid)).or_else(|| cff.and_then(|c| cff_glyph_name(c, gid)))
}

pub fn glyph_names(post: Option<&[u8]>, cff: Option<&[u8]>, n_glyphs: u16) -> Vec<Option<String>> {
    let mut names = match post {
        Some(p) if read_u32(p, 0) == Some(0x0002_0000) => post_glyph_names(p, n_glyphs),
        Some(p) => (0..n_glyphs).map(|g| post_glyph_name(p, n_glyphs, g)).collect(),
        None => alloc::vec![None; usize::from(n_glyphs)],
    };
    if names.iter().any(Option::is_none)
        && let Some(cff_names) = cff.and_then(|c| cff_glyph_names(c, n_glyphs))
    {
        for (name, fallback) in names.iter_mut().zip(cff_names) {
            if name.is_none() {
                *name = fallback;
            }
        }
    }
    names
}

// Versions 1.0 (the 258 standard Macintosh names, for a font of exactly those 258 glyphs), 2.0, and
// 2.5 (each glyph's offset into the standard order).
pub(crate) fn post_glyph_name(post: &[u8], n_glyphs: u16, gid: u16) -> Option<String> {
    let standard = |i: usize| MACINTOSH_NAMES.get(i).map(|n| String::from(*n));
    match read_u32(post, 0)? {
        0x0001_0000 if n_glyphs == 258 => standard(usize::from(gid)),
        0x0002_5000 => {
            let count = read_u16(post, 32)?;
            if gid >= count || count > 258 {
                return None;
            }
            let offset = i32::from(*post.get(34 + usize::from(gid))? as i8);
            standard(usize::try_from(i32::from(gid) + offset).ok()?)
        }
        0x0002_0000 => {
            let count = read_u16(post, 32)?;
            if gid >= count {
                return None;
            }
            let index = read_u16(post, 34 + usize::from(gid) * 2)?;
            if let Some(name) = MACINTOSH_NAMES.get(usize::from(index)) {
                return Some(String::from(*name));
            }
            let mut at = 34 + usize::from(count) * 2;
            let mut remaining = usize::from(index) - MACINTOSH_NAMES.len();
            loop {
                let len = usize::from(*post.get(at)?);
                let text = post.get(at + 1..at + 1 + len)?;
                if remaining == 0 {
                    return name_of(text);
                }
                remaining -= 1;
                at += 1 + len;
            }
        }
        _ => None,
    }
}

fn post_glyph_names(post: &[u8], n_glyphs: u16) -> Vec<Option<String>> {
    let none = || alloc::vec![None; usize::from(n_glyphs)];
    let Some(count) = read_u16(post, 32) else { return none() };
    let table_end = 34 + usize::from(count) * 2;

    // Only the strings some glyph names: spans cost 8 bytes each and a Pascal string can be 1, so
    // walking to the table's end would let an N-byte post ask for 8N.
    let indices: Vec<u16> = (0..n_glyphs.min(count))
        .filter_map(|g| read_u16(post, 34 + usize::from(g) * 2))
        .collect();
    let wanted = indices
        .iter()
        .filter_map(|&i| usize::from(i).checked_sub(MACINTOSH_NAMES.len()))
        .max();
    let Some(wanted) = wanted else {
        return (0..n_glyphs).map(|g| post_glyph_name(post, n_glyphs, g)).collect();
    };

    let mut spans: Vec<(u32, u32)> = Vec::new();
    let mut at = table_end;
    for _ in 0..=wanted {
        let Some(&len) = post.get(at) else { break };
        let len = usize::from(len);
        if post.get(at + 1..at + 1 + len).is_none() {
            break;
        }
        spans.push(((at + 1) as u32, (at + 1 + len) as u32));
        at += 1 + len;
    }

    (0..n_glyphs)
        .map(|g| {
            let index = usize::from(*indices.get(usize::from(g))?);
            if let Some(name) = MACINTOSH_NAMES.get(index) {
                return Some(String::from(*name));
            }
            let (s, e) = *spans.get(index - MACINTOSH_NAMES.len())?;
            name_of(post.get(s as usize..e as usize)?)
        })
        .collect()
}

// A non-CID CFF's charset and where its strings are, read for one glyph or all of them: the INDEXes
// are read by their headers and the one entry wanted, not materialized.
struct CffNames<'a> {
    cff: &'a [u8],
    charset: Option<usize>,
    predefined: u8,
    covered: usize,
    strings: Option<usize>,
}

impl<'a> CffNames<'a> {
    fn read(cff: &'a [u8]) -> Option<CffNames<'a>> {
        let hdr = usize::from(*cff.get(2)?);
        let after_name = super::subsetter::cff_index_end(cff, hdr, false).ok()?;
        let after_top = super::subsetter::cff_index_end(cff, after_name, false).ok()?;
        let fields = super::subsetter::parse_top_dict(super::subsetter::cff_index_entry(cff, after_name, 0)?).ok()?;
        if fields.ros.is_some() {
            return None;
        }
        super::subsetter::cff_index_end(cff, fields.charstrings_off, false).ok()?;
        let covered = super::subsetter::cff_index_count(cff, fields.charstrings_off).ok()?;
        let strings = super::subsetter::cff_index_end(cff, after_top, false).ok().map(|_| after_top);
        Some(CffNames { cff, charset: fields.charset_off, predefined: fields.charset_predefined, covered, strings })
    }

    // The predefined charset's SID for a glyph, None past the charset or for a font larger than it.
    fn predefined_sid(&self, gid: u16) -> Option<u16> {
        let table: &[u16] = match self.predefined {
            1 => &super::subsetter::cff::expert_charsets::EXPERT_CHARSET,
            2 => &super::subsetter::cff::expert_charsets::EXPERT_SUBSET_CHARSET,
            _ => return (self.covered <= ISO_ADOBE_GLYPHS).then_some(gid),
        };
        if self.covered > table.len() {
            return None;
        }
        table.get(usize::from(gid)).copied()
    }

    fn name(&self, sid: u16) -> Option<String> {
        if let Some(name) = CFF_STANDARD_STRINGS.get(usize::from(sid)) {
            return Some(String::from(*name));
        }
        name_of(super::subsetter::cff_index_entry(self.cff, self.strings?, usize::from(sid - N_STD_STRINGS))?)
    }
}

pub(crate) fn cff_glyph_name(cff: &[u8], gid: u16) -> Option<String> {
    let names = CffNames::read(cff)?;
    if usize::from(gid) >= names.covered {
        return None;
    }
    let sid = match names.charset {
        Some(off) => charset_sid(cff, off, names.covered, gid)?,
        None => names.predefined_sid(gid)?,
    };
    names.name(sid)
}

// A format 0 charset is an array, read where the glyph is; ranges are walked to it.
fn charset_sid(cff: &[u8], off: usize, n_glyphs: usize, gid: u16) -> Option<u16> {
    if gid == 0 {
        return Some(0);
    }
    if *cff.get(off)? == 0 {
        return read_u16(cff, off + 1 + 2 * (usize::from(gid) - 1));
    }
    let mut found = None;
    let _ = super::subsetter::walk_charset(cff, off, n_glyphs, |g, sid| {
        if g == gid {
            found = Some(sid);
            super::subsetter::CharsetFlow::Stop
        } else {
            super::subsetter::CharsetFlow::Continue
        }
    });
    found
}

// Every glyph's name as `cff_glyph_name` gives it. A charset cut short names the glyphs read before
// the cut, as one glyph at a time would.
fn cff_glyph_names(cff: &[u8], n_glyphs: u16) -> Option<Vec<Option<String>>> {
    let names = CffNames::read(cff)?;
    let n = usize::from(n_glyphs);
    let mut sids: Vec<Option<u16>> = alloc::vec![None; n];
    match names.charset {
        Some(off) => {
            if let Some(slot) = sids.first_mut() {
                *slot = Some(0);
            }
            let _ = super::subsetter::walk_charset(cff, off, names.covered, |g, sid| {
                if let Some(slot) = sids.get_mut(usize::from(g)) {
                    *slot = Some(sid);
                }
                super::subsetter::CharsetFlow::Continue
            });
        }
        None => {
            for (g, slot) in sids.iter_mut().enumerate() {
                *slot = names.predefined_sid(u16::try_from(g).ok()?);
            }
        }
    }
    Some(
        sids.iter()
            .enumerate()
            .map(|(g, sid)| if g < names.covered { names.name((*sid)?) } else { None })
            .collect(),
    )
}

// A name as the font spells it: UTF-8, not empty, no longer than MAX_NAME.
fn name_of(text: &[u8]) -> Option<String> {
    let name = core::str::from_utf8(text).ok()?;
    (!name.is_empty() && name.len() <= MAX_NAME).then(|| String::from(name))
}

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    super::decoder::read_u16_be(data, off)
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    super::decoder::read_u32_be(data, off)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A CFF of `n` glyphs, each endchar, with these strings, an empty Private DICT and a charset: its
    // bytes, last in the table, or a predefined one by number.
    fn cff(n: usize, strings: &[&[u8]], charset: Result<&[u8], u8>) -> Vec<u8> {
        let index = |items: &[&[u8]]| {
            let mut b = (items.len() as u16).to_be_bytes().to_vec();
            if items.is_empty() {
                return b;
            }
            b.push(4);
            let mut at = 1u32;
            b.extend(at.to_be_bytes());
            for i in items {
                at += i.len() as u32;
                b.extend(at.to_be_bytes());
            }
            items.iter().for_each(|i| b.extend(*i));
            b
        };
        let int = |v: usize| [&[29u8][..], &(v as i32).to_be_bytes()].concat();
        let (name, strs, gsubrs) = (index(&[b"T"]), index(strings), index(&[]));
        let charstrings = index(&alloc::vec![&[14u8][..]; n]);
        let top_index = index(&[&[0u8; 23][..]]).len();
        let body = 4 + name.len() + top_index + strs.len() + gsubrs.len();
        let (charset_bytes, charset_value) = match charset {
            Ok(bytes) => (bytes.to_vec(), body + charstrings.len()),
            Err(k) => (Vec::new(), usize::from(k)),
        };
        let mut top = int(charset_value);
        top.push(15);
        top.extend(int(body));
        top.push(17);
        top.extend([int(0), int(1)].concat());
        top.push(18);
        [alloc::vec![1, 0, 4, 4], name, index(&[&top]), strs, gsubrs, charstrings, charset_bytes].concat()
    }

    // One 100,000-byte string named by all 1,999 glyphs would be 200 MB of names; past post's 63 bytes
    // a name counts as none.
    #[test]
    fn a_name_past_63_bytes_is_absent() {
        let long = alloc::vec![b'a'; 100_000];
        let mut charset = alloc::vec![0u8];
        (1..2000).for_each(|_| charset.extend(391u16.to_be_bytes()));
        let names = glyph_names(None, Some(&cff(2000, &[&long], Ok(&charset))), 2000);
        assert_eq!(names.iter().flatten().map(String::len).sum::<usize>(), ".notdef".len());
    }

    // FreeType refuses a predefined charset smaller than the font, so no glyph is named from it.
    #[test]
    fn a_predefined_charset_past_its_size_names_nothing() {
        let expert = cff(200, &[], Err(1));
        assert_eq!((cff_glyph_name(&expert, 5), glyph_names(None, Some(&expert), 200)[5].clone()), (None, None));
        let iso = cff(300, &[], Err(0));
        assert_eq!(cff_glyph_name(&iso, 229), None);
        assert_eq!(cff_glyph_name(&cff(200, &[], Err(0)), 34).as_deref(), Some("A"));
    }

    // A format 0 charset of one SID for three glyphs: what it names before the cut stays named.
    #[test]
    fn a_charset_cut_short_keeps_what_it_read() {
        let font = cff(3, &[], Ok(&[0, 0, 34]));
        assert_eq!(glyph_names(None, Some(&font), 3), [Some(".notdef".into()), Some("A".into()), None]);
        assert_eq!(cff_glyph_name(&font, 1).as_deref(), Some("A"));
    }

    // A post 2.0 naming only glyph 0: the rest come from the CFF, as glyph_name gives them.
    #[test]
    fn names_post_leaves_out_come_from_the_cff() {
        let font = cff(10, &[], Err(0));
        let mut post = alloc::vec![0u8; 32];
        post[..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
        post.extend([0, 1, 0, 0]);
        let names = glyph_names(Some(&post), Some(&font), 10);
        assert_eq!(names[5], cff_glyph_name(&font, 5));
        assert!(names[5].is_some());
    }

    // Version 1.0 names the 258 standard glyphs by number, and 2.5 by an offset into that order.
    #[test]
    fn post_versions_1_and_2_5_name_glyphs() {
        let mut v1 = alloc::vec![0u8; 32];
        v1[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        assert_eq!(glyph_names(Some(&v1), None, 258)[36].as_deref(), Some("A"));
        assert_eq!(glyph_names(Some(&v1), None, 259)[36], None, "1.0 is only for the 258 standard glyphs");

        let mut v25 = alloc::vec![0u8; 32];
        v25[..4].copy_from_slice(&0x0002_5000u32.to_be_bytes());
        v25.extend(3u16.to_be_bytes());
        v25.extend([0, 35, 35]);
        assert_eq!(glyph_names(Some(&v25), None, 3), [Some(".notdef".into()), Some("A".into()), Some("B".into())]);
    }
}
