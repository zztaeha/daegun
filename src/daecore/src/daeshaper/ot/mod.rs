pub(crate) mod apply;
pub(crate) mod gsub;
pub(crate) mod gpos;
pub(crate) mod digest;
pub(crate) mod map;
pub mod tag;
pub(crate) mod kern;
pub(crate) mod kerx;
pub(crate) mod morx;

use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be, search_records, window};
use crate::daecore::daetype::format::feature_variations::FeatureVariations;
use self::tag::Tag;

pub(crate) const COVERAGE_ABSENT: u16 = u16::MAX;

#[derive(Clone, Copy)]
pub(crate) struct Coverage<'a> {
    data: &'a [u8],
    index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
}

const MAX_INDEX_ENTRIES: usize = 65_536;

impl<'a> Coverage<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() < 4 { return None; }
        Some(Coverage { data, index: None })
    }

    pub(crate) fn with_index(
        data: &'a [u8],
        index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
    ) -> Option<Self> {
        Some(Coverage { index, ..Coverage::new(data)? })
    }

    pub(crate) fn index_entries(&self) -> Option<alloc::vec::Vec<(u32, u16)>> {
        let d = self.data;
        let mut out = alloc::vec::Vec::new();
        match read_u16_be(d, 0)? {
            1 => {
                let count = read_u16_be(d, 2)? as usize;
                for i in 0..count {
                    out.push((u32::from(read_u16_be(d, 4 + i * 2)?), u16::try_from(i).ok()?));
                }
            }
            2 => {
                let count = read_u16_be(d, 2)? as usize;
                for i in 0..count {
                    let rec = 4 + i * 6;
                    let r = window::<6>(d, rec)?;
                    let (start, end, first) = (
                        u16::from_be_bytes([r[0], r[1]]),
                        u16::from_be_bytes([r[2], r[3]]),
                        u16::from_be_bytes([r[4], r[5]]),
                    );
                    if start > end {
                        return None;
                    }
                    if out.len().saturating_add(usize::from(end - start) + 1) > MAX_INDEX_ENTRIES {
                        return None;
                    }
                    for g in start..=end {
                        out.push((u32::from(g), first.checked_add(g - start)?));
                    }
                }
            }
            _ => return None,
        }
        if out.windows(2).any(|w| w[0].0 >= w[1].0) {
            return None;
        }
        if out.iter().any(|&(_, v)| v == COVERAGE_ABSENT) {
            return None;
        }
        Some(out)
    }

    pub(crate) fn index_of(&self, glyph: u16) -> Option<u16> {
        if let Some(index) = self.index {
            let found = index.lookup(u32::from(glyph));
            return (found != COVERAGE_ABSENT).then_some(found);
        }
        crate::daecore::daetype::format::coverage::coverage_index(self.data, glyph)
    }

    pub(crate) fn contains(&self, glyph: u16) -> bool {
        self.index_of(glyph).is_some()
    }

    pub(crate) fn add_to_digest(&self, digest: &mut digest::Digest) {
        let d = self.data;
        let Some(format) = read_u16_be(d, 0) else {
            *digest = digest::Digest::full();
            return;
        };
        let Some(count) = read_u16_be(d, 2) else {
            *digest = digest::Digest::full();
            return;
        };

        match format {
            1 => {
                for i in 0..count as usize {
                    match read_u16_be(d, 4 + i * 2) {
                        Some(g) => digest.add(g),
                        None => {
                            *digest = digest::Digest::full();
                            return;
                        }
                    }
                }
            }
            2 => {
                for i in 0..count as usize {
                    let rec = 4 + i * 6;
                    match (read_u16_be(d, rec), read_u16_be(d, rec + 2)) {
                        (Some(first), Some(last)) => digest.add_range(first, last),
                        _ => {
                            *digest = digest::Digest::full();
                            return;
                        }
                    }
                }
            }
            _ => *digest = digest::Digest::full(),
        }
    }
}

pub(crate) struct SubtableIndex {
    pub(crate) coverage: Option<crate::daecore::daetype::format::index::SparseIndex>,
    pub(crate) class1: Option<crate::daecore::daetype::format::index::SparseIndex>,
    pub(crate) class2: Option<crate::daecore::daetype::format::index::SparseIndex>,
    pub(crate) digest: digest::Digest,
}

impl Default for SubtableIndex {
    fn default() -> Self {
        SubtableIndex {
            coverage: None,
            class1: None,
            class2: None,
            digest: digest::Digest::full(),
        }
    }
}

pub(crate) fn subtable_indexes(
    lookup: &Lookup,
    extension: u16,
    pair_pos: Option<u16>,
    context: u16,
    chain_context: u16,
    mut build: impl FnMut(&[(u32, u16)], u16) -> Option<crate::daecore::daetype::format::index::SparseIndex>,
) -> alloc::vec::Vec<SubtableIndex> {
    let mut out = alloc::vec::Vec::with_capacity(lookup.subtable_count as usize);
    for i in 0..lookup.subtable_count {
        let mut entry = SubtableIndex::default();
        if let Some((kind, data)) =
            lookup.subtable(i).and_then(|d| resolve_extension(d, lookup.kind, extension))
        {
            entry.digest = subtable_digest(kind, data, context, chain_context);

            if first_coverage_at(kind, data, context, chain_context) == Some(2) {
                entry.coverage = offset_coverage_at(data, 2)
                    .and_then(|c| c.index_entries())
                    .and_then(|e| build(&e, COVERAGE_ABSENT));
            }

            if Some(kind) == pair_pos && read_u16_be(data, 0) == Some(2) {
                let mut class_at = |at: usize| {
                    read_u16_be(data, at)
                        .filter(|&off| off != 0)
                        .and_then(|off| data.get(off as usize..))
                        .and_then(ClassDef::new)
                        .and_then(|c| c.index_entries())
                        .and_then(|e| build(&e, 0))
                };
                entry.class1 = class_at(8);
                entry.class2 = class_at(10);
            }
        }
        out.push(entry);
    }
    out
}

pub(crate) fn lookup_digest(
    lookup: &Lookup,
    extension: u16,
    context: u16,
    chain_context: u16,
) -> digest::Digest {
    use self::digest::Digest;
    let mut digest = Digest::new();

    for i in 0..lookup.subtable_count {
        let Some(data) = lookup.subtable(i) else { return Digest::full() };
        let Some((kind, data)) = resolve_extension(data, lookup.kind, extension) else {
            return Digest::full();
        };

        let sub = subtable_digest(kind, data, context, chain_context);
        digest.union(&sub);

        if digest == Digest::full() {
            return digest;
        }
    }

    digest
}

fn first_coverage_at(kind: u16, data: &[u8], context: u16, chain_context: u16) -> Option<usize> {
    if kind != context && kind != chain_context {
        return Some(2);
    }
    match read_u16_be(data, 0) {
        Some(1) | Some(2) => Some(2),
        Some(3) if kind == context => Some(6),
        Some(3) => read_u16_be(data, 2).map(usize::from).map(|n| 4 + n * 2 + 2),
        _ => None,
    }
}

fn subtable_digest(kind: u16, data: &[u8], context: u16, chain_context: u16) -> digest::Digest {
    let Some(at) = first_coverage_at(kind, data, context, chain_context) else {
        return digest::Digest::full();
    };
    let Some(coverage) = offset_coverage_at(data, at) else { return digest::Digest::full() };
    let mut d = digest::Digest::new();
    coverage.add_to_digest(&mut d);
    d
}

fn offset_coverage_at(subtable: &[u8], at: usize) -> Option<Coverage<'_>> {
    let off = read_u16_be(subtable, at)? as usize;
    Coverage::new(subtable.get(off..)?)
}

#[derive(Clone, Copy)]
pub(crate) struct ClassDef<'a> {
    data: &'a [u8],
    index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
}

impl<'a> ClassDef<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() < 4 { return None; }
        Some(ClassDef { data, index: None })
    }

    pub(crate) fn with_index(
        data: &'a [u8],
        index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
    ) -> Option<Self> {
        Some(ClassDef { index, ..ClassDef::new(data)? })
    }

    pub(crate) fn all_class_zero() -> Self {
        ClassDef { data: &[], index: None }
    }

    pub(crate) fn indexed<'b>(
        &self,
        index: Option<&'b crate::daecore::daetype::format::index::SparseIndex>,
    ) -> ClassDef<'b>
    where
        'a: 'b,
    {
        ClassDef { data: self.data, index }
    }

    pub(crate) fn index_entries(&self) -> Option<alloc::vec::Vec<(u32, u16)>> {
        let d = self.data;
        let mut out = alloc::vec::Vec::new();
        match read_u16_be(d, 0)? {
            1 => {
                let h = window::<4>(d, 2)?;
                let start = u16::from_be_bytes([h[0], h[1]]);
                let count = u16::from_be_bytes([h[2], h[3]]);
                for i in 0..count {
                    let class = read_u16_be(d, 6 + i as usize * 2)?;
                    if class != 0 {
                        out.push((u32::from(start.checked_add(i)?), class));
                    }
                }
            }
            2 => {
                let count = read_u16_be(d, 2)? as usize;
                for i in 0..count {
                    let rec = 4 + i * 6;
                    let r = window::<6>(d, rec)?;
                    let (start, end, class) = (
                        u16::from_be_bytes([r[0], r[1]]),
                        u16::from_be_bytes([r[2], r[3]]),
                        u16::from_be_bytes([r[4], r[5]]),
                    );
                    if start > end {
                        return None;
                    }
                    if class != 0 {
                        if out.len().saturating_add(usize::from(end - start) + 1) > MAX_INDEX_ENTRIES {
                            return None;
                        }
                        out.extend((start..=end).map(|g| (u32::from(g), class)));
                    }
                }
            }
            _ => return None,
        }
        if out.windows(2).any(|w| w[0].0 >= w[1].0) {
            return None;
        }
        Some(out)
    }

    pub(crate) fn class_of(&self, glyph: u16) -> u16 {
        if let Some(index) = self.index {
            return index.lookup(u32::from(glyph));
        }
        self.class_of_checked(glyph).unwrap_or(0)
    }

    fn class_of_checked(&self, glyph: u16) -> Option<u16> {
        let d = self.data;
        match read_u16_be(d, 0)? {
            1 => {
                let h = window::<4>(d, 2)?;
                let start = u16::from_be_bytes([h[0], h[1]]);
                let count = u16::from_be_bytes([h[2], h[3]]);
                if glyph < start { return None; }
                let idx = glyph - start;
                if idx >= count { return None; }
                read_u16_be(d, 6 + idx as usize * 2)
            }
            2 => {
                let count = read_u16_be(d, 2)? as usize;
                let cand = match search_records(count, glyph as u32, |i| {
                    read_u16_be(d, 4 + i * 6).map(u32::from)
                })? {
                    Ok(i) => i,
                    Err(0) => return None,
                    Err(i) => i - 1,
                };
                let rec = window::<6>(d, 4 + cand * 6)?;
                let start = u16::from_be_bytes([rec[0], rec[1]]);
                let end = u16::from_be_bytes([rec[2], rec[3]]);
                if glyph < start || glyph > end { return None; }
                Some(u16::from_be_bytes([rec[4], rec[5]]))
            }
            _ => None,
        }
    }
}

pub(crate) struct Gdef<'a> {
    data: &'a [u8],
    pub(crate) glyph_classes: Option<ClassDef<'a>>,
    pub(crate) mark_attach_classes: Option<ClassDef<'a>>,
    mark_glyph_sets_off: usize,
    pub(crate) item_var_store_off: usize,
}

impl<'a> Gdef<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() < 12 { return None; }
        let minor = read_u16_be(data, 2)?;
        let sub = |off: usize| -> Option<ClassDef<'a>> {
            let rel = read_u16_be(data, off)? as usize;
            if rel == 0 { return None; }
            ClassDef::new(data.get(rel..)?)
        };
        let mark_glyph_sets_off = if minor >= 2 && data.len() >= 14 {
            read_u16_be(data, 12).unwrap_or(0) as usize
        } else {
            0
        };
        let item_var_store_off = if minor >= 3 && data.len() >= 18 {
            read_u32_be(data, 14).unwrap_or(0) as usize
        } else {
            0
        };
        Some(Gdef {
            data,
            glyph_classes: sub(4),
            mark_attach_classes: sub(10),
            mark_glyph_sets_off,
            item_var_store_off,
        })
    }

    pub(crate) fn has_glyph_classes(&self) -> bool {
        self.glyph_classes.is_some()
    }

    pub(crate) fn is_mark_glyph(&self, glyph: u16, set_index: u16) -> bool {
        let Some(base) = Some(self.mark_glyph_sets_off).filter(|&o| o != 0) else { return false };
        let d = self.data;
        let Some(count) = read_u16_be(d, base + 2) else { return false };
        if set_index >= count { return false; }
        let rec = base + 4 + set_index as usize * 4;
        let Some(hi) = read_u16_be(d, rec) else { return false };
        let Some(lo) = read_u16_be(d, rec + 2) else { return false };
        let rel = ((hi as usize) << 16) | lo as usize;
        d.get(base + rel..)
            .and_then(Coverage::new)
            .is_some_and(|c| c.contains(glyph))
    }
}

pub(crate) fn glyph_props_from_gdef_class(class: u16, mark_attach: u16) -> u16 {
    use super::buffer::glyph_props;
    match class {
        1 => glyph_props::BASE_GLYPH,
        2 => glyph_props::LIGATURE,
        3 => glyph_props::MARK | (mark_attach << 8),
        _ => 0,
    }
}

pub(crate) mod lookup_flags {
    pub(crate) const RIGHT_TO_LEFT: u16 = 0x0001;
    pub(crate) const IGNORE_BASE_GLYPHS: u16 = 0x0002;
    pub(crate) const IGNORE_LIGATURES: u16 = 0x0004;
    pub(crate) const IGNORE_MARKS: u16 = 0x0008;
    pub(crate) const IGNORE_FLAGS: u16 = IGNORE_BASE_GLYPHS | IGNORE_LIGATURES | IGNORE_MARKS;
    pub(crate) const USE_MARK_FILTERING_SET: u16 = 0x0010;
    pub(crate) const MARK_ATTACHMENT_TYPE_MASK: u16 = 0xFF00;
}

#[derive(Clone, Copy)]
pub(crate) struct Lookup<'a> {
    table: &'a [u8],
    at: usize,
    pub(crate) kind: u16,
    pub(crate) flags: u16,
    pub(crate) subtable_count: u16,
}

impl<'a> Lookup<'a> {
    pub(crate) fn subtable(&self, i: u16) -> Option<&'a [u8]> {
        if i >= self.subtable_count {
            return None;
        }
        let rel = read_u16_be(self.table, self.at + 6 + i as usize * 2)? as usize;
        self.table.get(self.at + rel..)
    }

    pub(crate) fn mark_filtering_set(&self) -> Option<u16> {
        if self.flags & lookup_flags::USE_MARK_FILTERING_SET == 0 {
            return None;
        }
        read_u16_be(self.table, self.at + 6 + self.subtable_count as usize * 2)
    }

    pub(crate) fn props(&self) -> u32 {
        let mut props = self.flags as u32;
        if let Some(set) = self.mark_filtering_set() {
            props |= (set as u32) << 16;
        }
        props
    }
}

pub mod offered {
    use super::LayoutTable;
    use alloc::vec::Vec;

    pub fn script_tags(data: &[u8]) -> Vec<[u8; 4]> {
        let Some(t) = LayoutTable::parse(data) else { return Vec::new() };
        t.list_tags(t.script_list)
    }

    pub fn language_tags(data: &[u8], script: &[u8; 4]) -> Vec<[u8; 4]> {
        let Some(t) = LayoutTable::parse(data) else { return Vec::new() };
        let Some(i) = t.tag_index(t.script_list, script) else { return Vec::new() };
        let Some(s) = t.record_offset(t.script_list, i, 6, 4) else { return Vec::new() };
        t.list_tags(s + 2)
    }

    pub fn feature_tags(
        data: &[u8],
        script: Option<&[u8; 4]>,
        language: Option<&[u8; 4]>,
    ) -> Vec<[u8; 4]> {
        let Some(t) = LayoutTable::parse(data) else { return Vec::new() };
        let si = match script {
            Some(tag) => match t.tag_index(t.script_list, tag) {
                Some(i) => i,
                None => return Vec::new(),
            },
            None => match t.select_script(&[]) {
                Some((_, i, _)) => i,
                None => return Vec::new(),
            },
        };
        let li = language.and_then(|tag| {
            let s = t.record_offset(t.script_list, si, 6, 4)?;
            t.tag_index(s + 2, tag)
        });
        let mut out: Vec<[u8; 4]> = Vec::new();
        let mut push = |f: u16| {
            if let Some(tag) = t.feature_tag(f) {
                out.push(tag.to_bytes());
            }
        };
        if let Some(req) = t.required_feature(si, li) { push(req) }
        for f in t.feature_indices(si, li) { push(f) }
        out.sort_unstable();
        out.dedup();
        out
    }
}

pub(crate) struct LayoutTable<'a> {
    data: &'a [u8],
    script_list: usize,
    feature_list: usize,
    lookup_list: usize,
    feature_variations: Option<usize>,
}

impl<'a> LayoutTable<'a> {
    pub(crate) fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 10 {
            return None;
        }
        let h = window::<8>(data, 2)?;
        let w = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let minor = w(0);
        let (script_list, feature_list, lookup_list) = (w(2) as usize, w(4) as usize, w(6) as usize);
        let feature_variations = if minor >= 1 && data.len() >= 14 {
            match read_u32_be(data, 10)? {
                0 => None,
                v => Some(v as usize),
            }
        } else {
            None
        };
        Some(LayoutTable { data, script_list, feature_list, lookup_list, feature_variations })
    }

    pub(crate) fn lookup_count(&self) -> u16 {
        read_u16_be(self.data, self.lookup_list).unwrap_or(0)
    }

    pub(crate) fn lookup(&self, index: u16) -> Option<Lookup<'a>> {
        if index >= self.lookup_count() {
            return None;
        }
        let rel = read_u16_be(self.data, self.lookup_list + 2 + index as usize * 2)? as usize;
        let at = self.lookup_list + rel;
        let h = window::<6>(self.data, at)?;
        Some(Lookup {
            table: self.data,
            at,
            kind: u16::from_be_bytes([h[0], h[1]]),
            flags: u16::from_be_bytes([h[2], h[3]]),
            subtable_count: u16::from_be_bytes([h[4], h[5]]),
        })
    }

    fn record_offset(&self, list: usize, index: u16, stride: usize, tag_at: usize) -> Option<usize> {
        let rel = read_u16_be(self.data, list + 2 + index as usize * stride + tag_at)? as usize;
        Some(list + rel)
    }

    fn list_tags(&self, list: usize) -> alloc::vec::Vec<[u8; 4]> {
        let count = read_u16_be(self.data, list).unwrap_or(0);
        (0..count as usize)
            .filter_map(|i| {
                let at = list + 2 + i * 6;
                let b = self.data.get(at..at + 4)?;
                Some([b[0], b[1], b[2], b[3]])
            })
            .collect()
    }

    fn tag_index(&self, list: usize, tag: &[u8; 4]) -> Option<u16> {
        let count = read_u16_be(self.data, list)?;
        (0..count).find(|&i| {
            let at = list + 2 + i as usize * 6;
            self.data.get(at..at + 4) == Some(&tag[..])
        })
    }

    fn find_tag(&self, list: usize, tag: Tag) -> Option<u16> {
        let count = read_u16_be(self.data, list)?;
        for i in 0..count {
            let at = list + 2 + i as usize * 6;
            let bytes = self.data.get(at..at + 4)?;
            if bytes == tag.to_bytes() {
                return Some(i);
            }
        }
        None
    }

    pub(crate) fn select_script(&self, tags: &[Tag]) -> Option<(bool, u16, Tag)> {
        for &tag in tags {
            if let Some(i) = self.find_tag(self.script_list, tag) {
                return Some((true, i, tag));
            }
        }
        for tag in [
            Tag::DEFAULT_SCRIPT,
            Tag::from_bytes(b"dflt"),
            Tag::from_bytes(b"latn"),
        ] {
            if let Some(i) = self.find_tag(self.script_list, tag) {
                return Some((false, i, tag));
            }
        }
        None
    }

    fn script_table(&self, script: u16) -> Option<usize> {
        self.record_offset(self.script_list, script, 6, 4)
    }

    pub(crate) fn select_langsys(&self, script: u16, tags: &[Tag]) -> Option<u16> {
        let s = self.script_table(script)?;
        let count = read_u16_be(self.data, s + 2)?;
        for &tag in tags {
            for i in 0..count {
                let at = s + 4 + i as usize * 6;
                if self.data.get(at..at + 4)? == tag.to_bytes() {
                    return Some(i);
                }
            }
        }
        None
    }

    fn langsys_table(&self, script: u16, lang: Option<u16>) -> Option<usize> {
        let s = self.script_table(script)?;
        match lang {
            Some(i) => {
                let rel = read_u16_be(self.data, s + 4 + i as usize * 6 + 4)? as usize;
                Some(s + rel)
            }
            None => match read_u16_be(self.data, s)? {
                0 => None,
                rel => Some(s + rel as usize),
            },
        }
    }

    pub(crate) fn required_feature(&self, script: u16, lang: Option<u16>) -> Option<u16> {
        let ls = self.langsys_table(script, lang)?;
        match read_u16_be(self.data, ls + 2)? {
            0xFFFF => None,
            i => Some(i),
        }
    }

    pub(crate) fn feature_indices(&self, script: u16, lang: Option<u16>)
        -> impl Iterator<Item = u16> + '_
    {
        let ls = self.langsys_table(script, lang);
        let count = ls.and_then(|ls| read_u16_be(self.data, ls + 4)).unwrap_or(0);
        (0..count).filter_map(move |i| {
            read_u16_be(self.data, ls.unwrap_or(0) + 6 + i as usize * 2)
        })
    }

    pub(crate) fn feature_tag(&self, feature: u16) -> Option<Tag> {
        let at = self.feature_list + 2 + feature as usize * 6;
        let b = self.data.get(at..at + 4)?;
        Some(Tag::from_bytes(&[b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn find_feature(&self, script: u16, lang: Option<u16>, tag: Tag) -> Option<u16> {
        self.feature_indices(script, lang)
            .find(|&i| self.feature_tag(i) == Some(tag))
    }

    pub(crate) fn find_feature_globally(&self, tag: Tag) -> Option<u16> {
        let count = read_u16_be(self.data, self.feature_list)?;
        (0..count).find(|&i| self.feature_tag(i) == Some(tag))
    }

    pub(crate) fn feature_lookups(&self, feature: u16, substitute: Option<usize>)
        -> impl Iterator<Item = u16> + '_
    {
        let at = match substitute {
            Some(off) => Some(off),
            None => read_u16_be(self.data, self.feature_list + 2 + feature as usize * 6 + 4)
                .map(|rel| self.feature_list + rel as usize),
        };
        let count = at.and_then(|at| read_u16_be(self.data, at + 2)).unwrap_or(0);
        (0..count).filter_map(move |i| read_u16_be(self.data, at.unwrap_or(0) + 4 + i as usize * 2))
    }

    pub(crate) fn find_variation_index(&self, coords: &[i32], axis_count: usize) -> Option<u16> {
        FeatureVariations::at(self.data, self.feature_variations?).with_axis_count(axis_count).find(coords)
    }

    pub(crate) fn variation_substitute(&self, variation: u16, feature: u16) -> Option<usize> {
        FeatureVariations::at(self.data, self.feature_variations?).substitute(variation, feature)
    }
}

pub(crate) fn resolve_extension(
    subtable: &[u8],
    lookup_kind: u16,
    extension_kind: u16,
) -> Option<(u16, &[u8])> {
    if lookup_kind != extension_kind {
        return Some((lookup_kind, subtable));
    }
    if read_u16_be(subtable, 0)? != 1 {
        return None;
    }
    let real = read_u16_be(subtable, 2)?;
    if real == extension_kind {
        return None;
    }
    let off = read_u32_be(subtable, 4)? as usize;
    Some((real, subtable.get(off..)?))
}

pub(crate) fn value_record_len(format: u16) -> usize {
    (format & 0x00FF).count_ones() as usize * 2
}
