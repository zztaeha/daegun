use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::io::{bin_search_header, read_u16_be, read_u24_be, read_u32_be, write_u16_be, write_u32_be};
use crate::daecore::daetype::table_bytes::Source;
use crate::daecore::daetype::{ByTag, TableBytes, Tables};
use crate::daecore::daetype::tables::tag_str;
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::Error;

fn checksum32(data: &[u8]) -> u32 {
    let (chunks, rem) = data.as_chunks::<4>();
    let mut sum = chunks.iter().fold(0u32, |acc, c| {
        // `from_be_bytes`, not a shift-or chain, is what lets LLVM see one byte-swapped load and
        // vectorize the fold: 116.7 µs against 569.5 µs over an 8.4 MB font.
        acc.wrapping_add(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
    });
    if !rem.is_empty() {
        let mut last: u32 = 0;
        for (i, &b) in rem.iter().enumerate() {
            last |= (b as u32) << ((3 - i) * 8);
        }
        sum = sum.wrapping_add(last);
    }
    sum
}

// maxp's numGlyphs at version 0.5 or 1.0, the ones the spec defines, a later 1.x read as 1.0 and its later fields
// read or not; any other version counts 0 glyphs. Each caller picks what no count counts as.
fn maxp_num_glyphs(maxp: &[u8]) -> Option<u16> {
    let count = read_u16_be(maxp, 4)?;
    let read = match read_u32_be(maxp, 0)? {
        0x0000_5000 => true,
        version => version >> 16 == 1,
    };
    Some(if read { count } else { 0 })
}

// MAXP's 24-bit count where it reads whole: 0.5 with its 7 bytes, 1.x with 33.
fn wide_num_glyphs(maxp: &[u8]) -> Option<u32> {
    let whole = match read_u32_be(maxp, 0)? {
        0x0000_5000 => true,
        version => version >> 16 == 1 && maxp.len() >= 33,
    };
    whole.then(|| read_u24_be(maxp, 4)).flatten()
}

// The one glyph count every path reads: MAXP's where it reads whole and the glyf pair read, or CFF2's CharStrings,
// hold that many glyphs, else maxp's, so a few bytes can't claim 16.7 million.
pub(crate) fn glyph_count<'t>(table: impl Fn(&str) -> Option<&'t [u8]>) -> Option<u32> {
    let narrow = || table("maxp").and_then(maxp_num_glyphs).map(u32::from);
    let Some(wide) = table("MAXP").and_then(wide_num_glyphs) else { return narrow() };
    let loca = || {
        let format = crate::daecore::daetype::format::glyf::loca_format(table("head")?)?;
        let tags = GlyfTags::of(|tag| table(tag).is_some_and(|t| !t.is_empty()));
        table(tags.glyf).filter(|t| !t.is_empty())?;
        Some(crate::daecore::daetype::format::glyf::glyf_glyph_count(table(tags.loca)?.len(), format, usize::MAX))
    };
    let charstrings = || {
        use crate::daecore::daetype::{format::cff::{read_dict, DictKind}, subsetter::cff_index_count};
        let cff2 = table("CFF2")?;
        let start = usize::from(*cff2.get(2)?);
        let top = cff2.get(start..start + usize::from(read_u16_be(cff2, 3)?))?;
        cff_index_count(cff2, read_dict(top, DictKind::Cff2).charstrings?, true).ok()
    };
    let held = loca().max(charstrings()).map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
    Some(wide).filter(|&wide| wide <= held).or_else(narrow)
}

pub(crate) fn num_glyphs(table_map: &impl ByTag) -> Option<u32> {
    glyph_count(|tag| table_map.table(tag))
}

// glyf and loca as every path picks them: GLYF and LOCA where both hold bytes, their composites then naming a glyph
// in 24 bits where a record says so; else glyf and loca.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct GlyfTags {
    pub(crate) glyf: &'static str,
    pub(crate) loca: &'static str,
    pub(crate) extended: bool,
}

impl GlyfTags {
    pub(crate) fn of(held: impl Fn(&str) -> bool) -> GlyfTags {
        match held("GLYF") && held("LOCA") {
            true => GlyfTags { glyf: "GLYF", loca: "LOCA", extended: true },
            false => GlyfTags { glyf: "glyf", loca: "loca", extended: false },
        }
    }
}

pub(crate) fn glyf_tags(table_map: &impl ByTag) -> GlyfTags {
    GlyfTags::of(|tag| table_map.table(tag).is_some_and(|t| !t.is_empty()))
}

pub fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

// A table's tag as a map keys it: a `Tag`, or a string's first four bytes, zero-filled past a shorter one.
pub trait TableTag {
    fn table_tag(&self) -> [u8; 4];
}

impl TableTag for str {
    fn table_tag(&self) -> [u8; 4] {
        let mut tag = [0; 4];
        tag.iter_mut().zip(self.as_bytes()).for_each(|(to, &b)| *to = b);
        tag
    }
}

impl TableTag for String {
    fn table_tag(&self) -> [u8; 4] {
        self.as_str().table_tag()
    }
}

impl TableTag for crate::daecore::Tag {
    fn table_tag(&self) -> [u8; 4] {
        self.to_bytes()
    }
}

// Written in tag order, as the spec sorts the directory; of a tag given twice the last is kept, as a map keeps it.
// InvalidArgument for what a directory can't state: more than 65,535 tables, or a table's offset or length past u32.
pub fn build_ttf<'a, K, V>(table_map: impl IntoIterator<Item = (&'a K, &'a V)>) -> Result<Vec<u8>, Error>
where
    K: TableTag + ?Sized + 'a,
    V: AsRef<[u8]> + 'a,
{
    let mut tables: Vec<([u8; 4], &[u8])> =
        table_map.into_iter().map(|(tag, t)| (tag.table_tag(), t.as_ref())).collect();
    // Reversed, then sorted stably, so the last given of a tag comes first and is the one kept.
    tables.reverse();
    tables.sort_by_key(|&(tag, _)| tag);
    tables.dedup_by_key(|&mut (tag, _)| tag);
    let num_tables = tables.len();
    if num_tables == 0 { return Ok(Vec::new()); }
    if num_tables > usize::from(u16::MAX) {
        return Err(Error::invalid("font file: more than the 65,535 tables a directory counts"));
    }

    let sfnt_hdr  = 12;
    let dir_size  = num_tables * 16;
    let (entries, total) = directory_entries(sfnt_hdr + dir_size, tables.iter().map(|&(_, raw)| raw.len()))
        .ok_or(Error::invalid("font file: a table of 4 GiB or more, or one starting 4 GiB or more in"))?;

    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.resize(sfnt_hdr + dir_size, 0);
    let mut tbl_checksums: Vec<u32> = Vec::with_capacity(num_tables);
    for &(tag, raw) in &tables {
        // head's directory checksum reads checkSumAdjustment as zero: that field derives from it.
        tbl_checksums.push(if tag == *b"head" && raw.len() >= 12 {
            let mut zeroed = raw.to_vec();
            zeroed[8..12].fill(0);
            checksum32(&zeroed)
        } else {
            checksum32(raw)
        });
        out.extend_from_slice(raw);
        out.resize(pad4(out.len()), 0);
    }

    let cff = tables.iter().any(|&(tag, _)| tag == *b"CFF " || tag == *b"CFF2");
    let sfnt_version = if cff { 0x4F54_544F } else { 0x0001_0000 };
    write_u32_be(&mut out, 0, sfnt_version);
    for (k, v) in [num_tables].into_iter().chain(bin_search_header(num_tables, 16)).enumerate() {
        write_u16_be(&mut out, 4 + 2 * k, v as u16);
    }

    let mut dir_off = sfnt_hdr;
    for (i, &(tag, _)) in tables.iter().enumerate() {
        let cs = tbl_checksums[i];
        let [offset, length] = entries[i];
        out[dir_off..dir_off + 4].copy_from_slice(&tag);
        write_u32_be(&mut out, dir_off + 4,  cs);
        write_u32_be(&mut out, dir_off + 8,  offset);
        write_u32_be(&mut out, dir_off + 12, length);
        dir_off += 16;
    }

    if let Some(pos) = tables.iter().position(|&(tag, _)| tag == *b"head") {
        let head_off = entries[pos][0] as usize;
        let head_len = tables[pos].1.len();
        if head_len >= 12 && head_off + 12 <= out.len() {
            write_u32_be(&mut out, head_off + 8, 0);
            let header_cs = checksum32(&out[..sfnt_hdr + dir_size]);
            let file_cs = tbl_checksums.iter().fold(header_cs, |a, &c| a.wrapping_add(c));
            write_u32_be(&mut out, head_off + 8, 0xB1B0_AFBA_u32.wrapping_sub(file_cs));
        }
    }

    Ok(out)
}

// Overlapping tables cost nothing read as windows, but a file written from them copies each: held to the subset
// budget of the bytes covered, or to 4x the file they are windows onto where more, as 1.x read and wrote that much.
pub(crate) fn copies_held(tables: &Tables) -> Result<(), Error> {
    let mut spans: Vec<(usize, usize)> = tables.iter().map(|(_, t)| (t.as_ptr() as usize, t.len())).collect();
    spans.sort_unstable();
    let (mut held, mut covered, mut end) = (0usize, 0usize, 0usize);
    for (start, len) in spans {
        held = held.saturating_add(len);
        covered += (start + len).saturating_sub(start.max(end));
        end = end.max(start + len);
    }
    let file = tables.iter().map(|(_, t)| t.source_len()).max().unwrap_or(0);
    let bound = crate::daecore::daetype::subsetter::subset_budget(covered).max(file.saturating_mul(4));
    match held <= bound {
        true => Ok(()),
        false => Err(Error::limit("font file: tables naming the same bytes too many times over to write")),
    }
}

// Each table's offset and length in a file whose tables, `lengths` long and each padded to four bytes, follow `start`
// bytes, then the file's length: None where an offset or a length passes u32.
fn directory_entries(start: usize, lengths: impl ExactSizeIterator<Item = usize>) -> Option<(Vec<[u32; 2]>, usize)> {
    let (mut entries, mut end) = (Vec::with_capacity(lengths.len()), start);
    for len in lengths {
        entries.push([u32::try_from(end).ok()?, u32::try_from(len).ok()?]);
        end = end.checked_add(len)?.checked_next_multiple_of(4)?;
    }
    Some((entries, end))
}

pub fn extract_ttf_tables(data: &[u8]) -> Result<Tables, Error> {
    extract_ttf_tables_owned_at(Shared::new(Source::Owned(data.to_vec())), 0)
}

// A font file, or face `index` of a collection, its tables windows onto `source` where they lie, as a font file's
// are: a collection's face reads the whole collection rather than a copy of its own tables.
pub(crate) fn extract_tables_in_place(source: Source, index: usize) -> Result<Tables, Error> {
    let dir_off = if is_collection(&source) {
        ttc_face_offset(&source, index)?
    } else if index == 0 {
        0
    } else {
        return Err(ONE_FACE);
    };
    extract_ttf_tables_owned_at(Shared::new(source), dir_off)
}

// A collection's list of face offsets and the faces it holds: at major 1 or 2 with a minor of 1 or more, the extended
// list after the first and, at 2, its three DSIG fields; else the first. The count it states is cut to the offsets
// the file holds.
fn ttc_faces(data: &[u8]) -> Option<(usize, usize)> {
    let mut count_at = 8;
    if matches!(read_u16_be(data, 4)?, 1 | 2) && read_u16_be(data, 6)? >= 1 {
        let skipped = (read_u32_be(data, 8)? as usize).checked_mul(4)?;
        count_at = skipped.checked_add(if read_u16_be(data, 4)? == 2 { 24 } else { 12 })?;
    }
    let stated = read_u32_be(data, count_at)? as usize;
    Some((count_at + 4, stated.min(data.len().saturating_sub(count_at + 4) / 4)))
}

const TTCF: u32 = 0x7474_6366;
pub(crate) const ONE_FACE: Error = Error::invalid("a font file holds one face, index 0");

pub fn is_collection(data: &[u8]) -> bool {
    read_u32_be(data, 0) == Some(TTCF)
}

// The faces whose offsets a collection holds, whatever count its header states; 1 for bytes a font file's
// directory starts, else 0. The collection first, as a match on the signature took 12 more instructions.
pub fn face_count(data: &[u8]) -> usize {
    if is_collection(data) {
        return ttc_faces(data).map_or(0, |(_, faces)| faces);
    }
    usize::from(data.len() >= 12 && matches!(read_u32_be(data, 0), Some(0x0001_0000 | 0x4F54_544F | 0x7472_7565)))
}

pub fn extract_ttc_tables(data: &[u8], index: usize) -> Result<Tables, Error> {
    if !is_collection(data) {
        return Err(Error::malformed("TTC: not a collection"));
    }
    let dir_off = ttc_face_offset(data, index)?;
    // A face holds its own tables, copied into one buffer, rather than the whole collection: the bytes they cover,
    // each copied once however the entries overlap, so each table is a window as a font file's is.
    let dir = directory(data, dir_off)?;
    // Each span the tables cover, overlapping ones joined, with where it lands in the copy.
    let mut covered: Vec<(usize, usize, usize)> = dir.tables().map(|e| (e.off, e.off + e.len, 0)).collect();
    covered.sort_unstable();
    covered.dedup_by(|next, kept| {
        let joins = next.0 <= kept.1;
        if joins {
            kept.1 = kept.1.max(next.1);
        }
        joins
    });
    // A face whose tables total more than 4x the bytes they cover keeps the whole collection, as a write may copy 4x
    // the file its tables are windows onto, which only covered bytes would cut short.
    let covered_len: usize = covered.iter().map(|&(start, end, _)| end - start).sum();
    let held = dir.tables().fold(0usize, |sum, e| sum.saturating_add(e.len));
    let (buf, starts): (_, Vec<usize>) = if held > covered_len.saturating_mul(4) {
        (Shared::new(Source::Owned(data.to_vec())), dir.tables().map(|e| e.off).collect())
    } else {
        let mut compact = Vec::with_capacity(covered_len);
        for span in &mut covered {
            span.2 = compact.len();
            compact.extend_from_slice(&data[span.0..span.1]);
        }
        let starts = dir.tables().map(|e| {
            let (start, _, at) = covered[covered.partition_point(|&(start, ..)| start <= e.off) - 1];
            at + e.off - start
        });
        (Shared::new(Source::Owned(compact)), starts.collect())
    };
    let tables = dir.tables().zip(starts)
        .map(|(&Entry { tag, len, .. }, start)| {
            let table = TableBytes::slice(&buf, start, len).ok_or(Error::malformed("TTC: a table out of bounds"))?;
            Ok((tag, table))
        })
        .collect::<Result<_, Error>>()?;
    Ok(Tables::from_sorted(tables))
}

// Where face `index` of the collection in `data` starts its directory.
fn ttc_face_offset(data: &[u8], index: usize) -> Result<usize, Error> {
    let (list, faces) = ttc_faces(data).ok_or(Error::malformed("TTC: header truncated"))?;
    if index >= faces {
        return Err(Error::owned(
            crate::daecore::ErrorKind::InvalidArgument,
            format!("TTC: font index {} out of range ({} fonts)", index, faces),
        ));
    }
    let entry = index
        .checked_mul(4)
        .and_then(|i| i.checked_add(list))
        .ok_or(Error::malformed("TTC: offset table truncated"))?;
    read_u32_be(data, entry).map(|off| off as usize).ok_or(Error::malformed("TTC: offset table truncated"))
}

// Where each table the sfnt at the start of `data` names lies, as (offset, length), by the rules its
// tables are read by.
pub(crate) fn ttf_table_ranges(data: &[u8]) -> Result<BTreeMap<String, (usize, usize)>, Error> {
    Ok(directory(data, 0)?.tables().map(|e| (tag_str(&e.tag).into(), (e.off, e.len))).collect())
}

fn extract_ttf_tables_owned_at(buf: Shared<Source>, dir_off: usize) -> Result<Tables, Error> {
    in_order(&buf, dir_off).map_or_else(|| by_directory(&buf, dir_off), Ok)
}

fn by_directory(buf: &Shared<Source>, dir_off: usize) -> Result<Tables, Error> {
    let dir = directory(buf, dir_off)?;
    let mut tables = Vec::with_capacity(dir.tables().count());
    for &Entry { tag, off, len, .. } in dir.tables() {
        tables.push((tag, TableBytes::slice(buf, off, len).ok_or(Error::malformed("TTF/OTF: a table out of bounds"))?));
    }
    Ok(Tables::from_sorted(tables))
}

// A directory as the spec lays it out, read straight into the tables: tags rising and printable, entries inside the
// file with bytes (glyf may be empty), one outline format. Else None, and `directory` reads it.
fn in_order(buf: &Shared<Source>, dir_off: usize) -> Option<Tables> {
    let data: &[u8] = buf;
    if !matches!(read_u32_be(data, dir_off)?, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
        return None;
    }
    let count = usize::from(read_u16_be(data, dir_off + 4)?);
    let raw = data.get(dir_off.checked_add(12)?..)?.as_chunks::<16>().0.get(..count)?;
    let word = |e: &[u8; 16], at: usize| u32::from_be_bytes([e[at], e[at + 1], e[at + 2], e[at + 3]]);
    let mut tables = Vec::with_capacity(count);
    let (mut last, mut outlines) = (0, [false; 2]);
    for e in raw {
        let (tag, off, len) = (word(e, 0), word(e, 8) as usize, word(e, 12) as usize);
        // Each byte 0x20 to 0x7E: no top bit set, adding 0x60 sets it, and adding 1 doesn't.
        let printable = (tag | tag.wrapping_add(0x0101_0101) | !tag.wrapping_add(0x6060_6060)) & 0x8080_8080 == 0;
        if tag <= last || !printable || off > data.len() || len > data.len() - off || (len == 0 && tag != GLYF) {
            return None;
        }
        last = tag;
        match tag {
            GLYF | GLYF_WIDE => outlines[0] = true,
            CFF | CFF2 => outlines[1] = true,
            _ => {}
        }
        tables.push((tag.to_be_bytes(), TableBytes::slice(buf, off, len)?));
    }
    if outlines == [true; 2] {
        return None;
    }
    Some(Tables::from_sorted(tables))
}

// The tags the one-pass read tells apart, as the words a directory's order compares.
const fn tag_word(tag: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*tag)
}
const GLYF: u32 = tag_word(b"glyf");
const GLYF_WIDE: u32 = tag_word(b"GLYF");
const CFF: u32 = tag_word(b"CFF ");
const CFF2: u32 = tag_word(b"CFF2");

// Each tag's entry as `directory` reads it, the first that fits the file: its place in the file, whether it reads as
// a table, where it starts and its length clipped to the file.
struct Entry {
    tag: [u8; 4],
    place: u16,
    read: bool,
    off: usize,
    len: usize,
}

// A directory's entries in tag order, one for each tag.
struct Directory(Vec<Entry>);

// The entries a thread's last directory held, lent to its next, as an open would otherwise allocate and free a list.
#[cfg(feature = "std")]
std::thread_local! {
    static SPARE: core::cell::Cell<Vec<Entry>> = const { core::cell::Cell::new(Vec::new()) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_BYTES: usize = 64 << 10;

// A directory of more than `SPARE_BYTES` drops its own list, as does one past its thread's teardown.
impl Drop for Directory {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if crate::daecore::daekit::base::sync::heap_of(&self.0) <= SPARE_BYTES {
            let mut entries = core::mem::take(&mut self.0);
            entries.clear();
            let _ = SPARE.try_with(|spare| spare.set(entries));
        }
    }
}

impl Directory {
    fn get(&self, tag: &str) -> Option<(usize, usize)> {
        let tag = u32::from_be_bytes(tag.as_bytes().try_into().ok()?);
        let e = &self.0[self.0.binary_search_by_key(&tag, |e| u32::from_be_bytes(e.tag)).ok()?];
        e.read.then_some((e.off, e.len))
    }

    fn tables(&self) -> impl Iterator<Item = &Entry> {
        self.0.iter().filter(|e| e.read)
    }
}

const METRICS: [&[u8; 4]; 4] = [b"hmtx", b"HMTX", b"vmtx", b"VMTX"];

// Of each tag the first entry that fits the file counts, a metrics table clipped to it. Zero length reads as missing
// but for glyf, where it is a font whose glyphs are all empty, and a tag not printable is skipped.
fn directory(data: &[u8], dir_off: usize) -> Result<Directory, Error> {
    if dir_off.checked_add(12).is_none_or(|end| data.len() < end) {
        return Err(Error::malformed("TTF/OTF: file too short"));
    }
    let sfversion = read_u32_be(data, dir_off).ok_or(Error::malformed("TTF/OTF: header truncated"))?;
    if sfversion != 0x0001_0000 && sfversion != 0x4F54_544F && sfversion != 0x7472_7565 {
        return Err(Error::owned(
            crate::daecore::ErrorKind::Malformed,
            format!("TTF/OTF: not a font file (signature: 0x{:08X})", sfversion),
        ));
    }
    let num_tables = read_u16_be(data, dir_off + 4).ok_or(Error::malformed("TTF/OTF: header truncated"))?;
    let Some(raw) = data.get(dir_off + 12..).and_then(|d| d.as_chunks::<16>().0.get(..usize::from(num_tables))) else {
        return Err(Error::malformed("TTF/OTF: table directory truncated"));
    };
    let field = |e: &[u8; 16], at: usize| u32::from_be_bytes([e[at], e[at + 1], e[at + 2], e[at + 3]]) as usize;
    #[cfg(feature = "std")]
    let mut entries = SPARE.try_with(core::cell::Cell::take).unwrap_or_default();
    #[cfg(not(feature = "std"))]
    let mut entries = Vec::new();
    entries.extend((0..num_tables).zip(raw).map(|(place, e)| {
        let tag = [e[0], e[1], e[2], e[3]];
        Entry { tag, place, read: false, off: field(e, 8), len: field(e, 12) }
    }));
    entries.sort_unstable_by_key(|e| (u32::from_be_bytes(e.tag), e.place));
    let fits = |e: &Entry| e.off <= data.len() && (e.len <= data.len() - e.off || METRICS.contains(&&e.tag));
    let mut last = None;
    entries.retain(|e| {
        let first = fits(e) && last != Some(e.tag);
        if first {
            last = Some(e.tag);
        }
        first
    });
    for e in &mut entries {
        e.len = e.len.min(data.len() - e.off);
        e.read = e.tag.iter().all(|b| (0x20..=0x7E).contains(b)) && (e.len > 0 || &e.tag == b"glyf");
    }

    let mut dir = Directory(entries);
    // A font carrying both outline formats is malformed, and sfntVersion breaks the tie:
    // 0x00010000 and 'true' mean TrueType, 'OTTO' means CFF.
    let has = |tag| dir.get(tag).is_some();
    if (has("glyf") || has("GLYF")) && (has("CFF ") || has("CFF2")) {
        let dropped: &[&[u8; 4]] = match sfversion {
            0x4F54_544F => &[b"glyf", b"loca", b"GLYF", b"LOCA"],
            _ => &[b"CFF ", b"CFF2"],
        };
        dir.0.iter_mut().filter(|e| dropped.contains(&&e.tag)).for_each(|e| e.read = false);
    }
    Ok(dir)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/decoder/ttf.rs"]
mod tests;
