use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::TableBytes;

// A font's tables in tag order. A font names a few dozen at most, so a sorted list searched by the tag's four bytes
// as one word beats a map keyed by strings, which allocates every key and compares each through memcmp.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Tables(Vec<([u8; 4], TableBytes)>);

const SCANNED: usize = 32;

// A table's tag is four bytes, so a string of any other length names no table.
fn tag_of(tag: &str) -> Option<[u8; 4]> {
    tag.as_bytes().try_into().ok()
}

// A tag's bytes as text: a directory keeps only printable ones and the rest came from a `str`, so all are UTF-8.
pub(crate) fn tag_str(tag: &[u8; 4]) -> &str {
    core::str::from_utf8(tag).unwrap_or_default()
}

impl Tables {
    fn find(&self, tag: [u8; 4]) -> Result<usize, usize> {
        let word = u32::from_be_bytes(tag);
        let key = |e: &([u8; 4], TableBytes)| u32::from_be_bytes(e.0);
        // A font's few dozen tables are scanned: that mispredicts once, a search at half its steps.
        if self.0.len() <= SCANNED {
            let at = self.0.iter().position(|e| key(e) >= word).unwrap_or(self.0.len());
            return if self.0.get(at).is_some_and(|e| key(e) == word) { Ok(at) } else { Err(at) };
        }
        self.0.binary_search_by_key(&word, key)
    }

    pub fn get(&self, tag: &str) -> Option<&TableBytes> {
        let i = self.find(tag_of(tag)?).ok()?;
        Some(&self.0[i].1)
    }

    pub(crate) fn get_tagged(&self, tag: crate::daecore::Tag) -> Option<&TableBytes> {
        let i = self.find(tag.to_bytes()).ok()?;
        Some(&self.0[i].1)
    }

    // Each table with its tag, in tag order.
    pub(crate) fn tagged(
        &self,
    ) -> impl ExactSizeIterator<Item = (crate::daecore::Tag, &TableBytes)> + DoubleEndedIterator
           + core::iter::FusedIterator + Clone {
        self.0.iter().map(|(tag, table)| (crate::daecore::Tag::new(tag), table))
    }

    pub fn contains_key(&self, tag: &str) -> bool {
        self.get(tag).is_some()
    }

    // The table it replaces, as a map gives it back.
    pub fn insert(&mut self, tag: impl AsRef<str>, table: TableBytes) -> Option<TableBytes> {
        let tag = tag_of(tag.as_ref());
        debug_assert!(tag.is_some(), "a table tag is four bytes");
        let tag = tag?;
        match self.find(tag) {
            Ok(i) => Some(core::mem::replace(&mut self.0[i].1, table)),
            Err(i) => {
                self.0.insert(i, (tag, table));
                None
            }
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&str, &TableBytes)> + ExactSizeIterator {
        self.into_iter()
    }

    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.0.iter().map(|(tag, _)| tag_str(tag))
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        crate::daecore::daekit::base::sync::heap_of(&self.0)
    }

    // The tables whose tags `keep` takes, in tag order, in room for them all, as a filter's collect grows by doubling.
    pub(crate) fn filtered(&self, keep: impl Fn(&str) -> bool) -> Tables {
        let mut kept = Vec::with_capacity(self.0.len());
        kept.extend(self.0.iter().filter(|(tag, _)| keep(tag_str(tag))).cloned());
        Tables(kept)
    }

    // Tables already in tag order, each tag once, as a font's directory gives them.
    pub(crate) fn from_sorted(tables: Vec<([u8; 4], TableBytes)>) -> Tables {
        debug_assert!(tables.is_sorted_by(|a, b| u32::from_be_bytes(a.0) < u32::from_be_bytes(b.0)));
        Tables(tables)
    }
}

// A font's tables found by tag, wherever they are held: the font's own, or a map an instance or subset is built in.
pub trait ByTag {
    fn table(&self, tag: &str) -> Option<&[u8]>;
}

impl ByTag for Tables {
    fn table(&self, tag: &str) -> Option<&[u8]> {
        self.get(tag).map(|t| &**t)
    }
}

impl<V: AsRef<[u8]>> ByTag for BTreeMap<String, V> {
    fn table(&self, tag: &str) -> Option<&[u8]> {
        self.get(tag).map(V::as_ref)
    }
}

// As a map collects, a repeated tag keeps its last table.
impl<K: AsRef<str>> FromIterator<(K, TableBytes)> for Tables {
    fn from_iter<I: IntoIterator<Item = (K, TableBytes)>>(iter: I) -> Tables {
        let mut tables = Tables::default();
        tables.extend(iter);
        tables
    }
}

impl<K: AsRef<str>, const N: usize> From<[(K, TableBytes); N]> for Tables {
    fn from(tables: [(K, TableBytes); N]) -> Tables {
        tables.into_iter().collect()
    }
}

impl<K: AsRef<str>> Extend<(K, TableBytes)> for Tables {
    fn extend<I: IntoIterator<Item = (K, TableBytes)>>(&mut self, iter: I) {
        for (tag, table) in iter {
            self.insert(tag, table);
        }
    }
}

impl<'a> IntoIterator for &'a Tables {
    type Item = (&'a str, &'a TableBytes);
    type IntoIter = core::iter::Map<
        core::slice::Iter<'a, ([u8; 4], TableBytes)>,
        fn(&'a ([u8; 4], TableBytes)) -> (&'a str, &'a TableBytes),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(tag, table)| (tag_str(tag), table))
    }
}

impl core::ops::Index<&str> for Tables {
    type Output = TableBytes;

    #[track_caller]
    fn index(&self, tag: &str) -> &TableBytes {
        match self.get(tag) {
            Some(table) => table,
            None => panic!("no {tag:?} table"),
        }
    }
}

impl core::fmt::Debug for Tables {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/tables.rs"]
mod tests;
