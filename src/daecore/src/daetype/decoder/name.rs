use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::io::{fitting_count, read_u16_be, Placed};
use crate::daecore::daetype::Tables;

pub fn read_font_family_name(table_map: &Tables) -> Option<String> {
    read_name_string(table_map, 16).or_else(|| read_name_string(table_map, 1))
}

pub fn read_name_string(table_map: &Tables, name_id: u16) -> Option<String> {
    let data = table_map.get("name")?;
    let (key, raw) = readable_records(data)
        .filter(|(key, _)| key.name_id == name_id)
        .reduce(|best, r| if score(r.0) > score(best.0) { r } else { best })?;
    let mut name = String::new();
    decode_name_into(raw, key, &mut name);
    let trimmed = name.trim_matches('\0').trim();
    // A byte to spare, so C's NUL takes it and the name moves into C without a copy.
    (!trimmed.is_empty()).then(|| {
        let mut owned = String::with_capacity(trimmed.len() + 1);
        owned.push_str(trimmed);
        owned
    })
}

// A name record's key, ordered as the spec sorts records.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct NameKey {
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    pub name_id: u16,
}

// Whether names are read and written here in a record's encoding: UTF-16 on Unicode and Windows' Symbol,
// BMP and full repertoire, and Mac Roman. Other Mac scripts and Windows' legacy CJK encodings are not.
fn codable(key: NameKey) -> bool {
    matches!((key.platform, key.encoding), (0, _) | (1, 0) | (3, 0 | 1 | 10))
}

// Whether a codable record holds the English name: Unicode's, Mac Roman's language 0 and Windows'
// English LCIDs. A language ID from 0x8000 names a format 1 language tag instead.
pub(crate) fn is_english(key: NameKey) -> bool {
    codable(key) && match key.platform {
        0 => key.language < 0x8000,
        1 => key.language == 0,
        _ => key.language < 0x8000 && key.language & 0x3FF == 0x009,
    }
}

// The record a name is picked from: English Windows, then Windows in any language, then Mac Roman in
// English, then any Mac Roman, then Unicode.
fn score(key: NameKey) -> u8 {
    match key.platform {
        3 => 4 + u8::from(is_english(key)),
        1 => 2 + u8::from(is_english(key)),
        _ => 1,
    }
}

// Each record and its string, in table order. A record or string lying past the table is skipped, so
// a count that overstates the table gives the records that fit.
pub(crate) fn name_records(data: &[u8]) -> impl Iterator<Item = (NameKey, &[u8])> {
    let count = read_u16_be(data, 2).map_or(0, usize::from);
    let storage = read_u16_be(data, 4).map_or(0, usize::from);
    (0..count).filter_map(move |i| {
        let field = |at: usize| read_u16_be(data, 6 + i * 12 + at);
        let key = NameKey { platform: field(0)?, encoding: field(2)?, language: field(4)?, name_id: field(6)? };
        Some((key, string(data, storage, field(8)?, field(10)?)?))
    })
}

// The records a name can be read from: those with a string in an encoding decoded here.
fn readable_records(data: &[u8]) -> impl Iterator<Item = (NameKey, &[u8])> {
    name_records(data).filter(|&(key, raw)| !raw.is_empty() && codable(key))
}

fn string(data: &[u8], storage: usize, length: u16, offset: u16) -> Option<&[u8]> {
    let at = storage + usize::from(offset);
    data.get(at..at + usize::from(length))
}

// Format 1's language tags whose records lie in the table. A tag whose string lies past it reads as
// empty, so it keeps the index records refer to it by.
pub(crate) fn lang_tags(data: &[u8]) -> Vec<&[u8]> {
    if read_u16_be(data, 0) != Some(1) {
        return Vec::new();
    }
    let storage = read_u16_be(data, 4).map_or(0, usize::from);
    let at = 6 + 12 * read_u16_be(data, 2).map_or(0, usize::from);
    (0..fitting_count(data, at, 4))
        .map(|i| {
            let field = |k: usize| read_u16_be(data, at + 2 + 4 * i + k);
            field(0)
                .zip(field(2))
                .and_then(|(length, offset)| string(data, storage, length, offset))
                .unwrap_or_default()
        })
        .collect()
}

// A name table of `records`, sorted as the spec orders them, in format 1 when there are language tags.
// Each distinct string is stored once. None past the 64 KB a string offset can reach.
pub(crate) fn write_name_table<'a>(mut records: Vec<(NameKey, &'a [u8])>, tags: &[&'a [u8]]) -> Option<Vec<u8>> {
    records.sort_by_key(|&(key, _)| key);
    let header = 6 + 12 * records.len() + if tags.is_empty() { 0 } else { 2 + 4 * tags.len() };
    let mut out = Vec::with_capacity(header);
    for v in [u16::from(!tags.is_empty()), u16::try_from(records.len()).ok()?, u16::try_from(header).ok()?] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut strings = Placed::new(0);
    let mut place = |bytes: &'a [u8]| Some([u16::try_from(bytes.len()).ok()?, strings.place(bytes)?]);
    for &(key, bytes) in &records {
        let [length, offset] = place(bytes)?;
        for v in [key.platform, key.encoding, key.language, key.name_id, length, offset] {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    if !tags.is_empty() {
        out.extend_from_slice(&u16::try_from(tags.len()).ok()?.to_be_bytes());
        for &tag in tags {
            place(tag)?.iter().for_each(|v| out.extend_from_slice(&v.to_be_bytes()));
        }
    }
    out.extend(strings.bytes);
    Some(out)
}

// A string in a record's encoding, None where it has no encoder here or Mac Roman lacks a character.
pub(crate) fn encode_name(key: NameKey, s: &str) -> Option<Vec<u8>> {
    match key.platform {
        _ if !codable(key) => None,
        1 => s.chars().map(|c| mac_roman_byte(c as u32)).collect(),
        _ => Some(s.encode_utf16().flat_map(u16::to_be_bytes).collect()),
    }
}

fn decode_name_into(raw: &[u8], key: NameKey, out: &mut String) {
    out.clear();
    if key.platform == 1 {
        out.extend(raw.iter().map(|&b| mac_roman_char(b)));
    } else {
        out.extend(char::decode_utf16(raw.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes([b[0], b[1]])))
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)));
    }
}

// Each name ID's record `each_name_string` reads, those its decoding budget reaches, decoded when asked for: the
// style pass reads a handful, where decoding all took Inter's location 5 µs.
pub(crate) struct NameRecords<'a>(Vec<(NameKey, &'a [u8])>);

impl<'a> NameRecords<'a> {
    pub(crate) fn new(table_map: &'a Tables) -> NameRecords<'a> {
        NameRecords::upto(table_map, u16::MAX)
    }

    // The IDs up to `last` alone, each read as among them all, as the budget is spent in order of ID.
    pub(crate) fn upto(table_map: &'a Tables, last: u16) -> NameRecords<'a> {
        let Some(data) = table_map.get("name") else { return NameRecords(Vec::new()) };
        let mut records = best_records(data, last);
        let mut budget = data.len().saturating_mul(4);
        let cut = records.iter().position(|(_, raw)| match budget.checked_sub(raw.len()) {
            Some(left) => {
                budget = left;
                false
            }
            None => true,
        });
        if let Some(cut) = cut {
            records.truncate(cut);
        }
        NameRecords(records)
    }

    // The name, none where it is empty once trimmed.
    pub(crate) fn get(&self, name_id: u16) -> Option<String> {
        let (key, raw) = self.0[self.0.binary_search_by_key(&name_id, |(key, _)| key.name_id).ok()?];
        let mut name = String::new();
        decode_name_into(raw, key, &mut name);
        let trimmed = name.trim_matches('\0').trim();
        (!trimmed.is_empty()).then(|| trimmed.into())
    }
}

// Keyed by the ID as the caller reads it: the core by number, the API by `NameId`.
pub fn parse_all_name_strings<K: From<u16> + Ord>(table_map: &Tables) -> BTreeMap<K, String> {
    let mut out = BTreeMap::new();
    each_name_string(table_map, &mut out);
    out
}

// Where each name goes as it is read: a map of owned strings for Rust, one buffer of them all for C.
pub(crate) trait NameSink {
    // How many names follow, and the bytes they're stored in as far as decoding reads them, before the first.
    fn reserve(&mut self, _count: usize, _stored: usize) {}
    fn put(&mut self, name_id: u16, name: &str);
}

impl<K: From<u16> + Ord> NameSink for BTreeMap<K, String> {
    fn put(&mut self, name_id: u16, name: &str) {
        self.insert(K::from(name_id), name.to_string());
    }
}

// Every name in one buffer, in order of ID, for a reader that looks a few up rather than keeping a string each.
#[derive(Default)]
pub(crate) struct NameText {
    ids: Vec<u16>,
    ends: Vec<usize>,
    text: String,
}

impl NameSink for NameText {
    fn reserve(&mut self, count: usize, stored: usize) {
        self.ids.reserve(count);
        self.ends.reserve(count);
        self.text.reserve(stored);
    }

    fn put(&mut self, name_id: u16, name: &str) {
        self.ids.push(name_id);
        self.text.push_str(name);
        self.ends.push(self.text.len());
    }
}

impl NameText {
    pub(crate) fn get(&self, name_id: u16) -> Option<&str> {
        let i = self.ids.binary_search(&name_id).ok()?;
        Some(&self.text[i.checked_sub(1).map_or(0, |before| self.ends[before])..self.ends[i]])
    }

    // The bytes every name takes.
    pub(crate) fn bytes(&self) -> usize {
        self.text.len()
    }
}

// Each name ID's best record decoded, in order of ID, lent to `out`; one empty once trimmed is left out.
pub(crate) fn each_name_string(table_map: &Tables, out: &mut impl NameSink) {
    let Some(data) = table_map.get("name") else { return };
    let records = best_records(data, u16::MAX);
    // Records may all share one long string, so the bytes decoded, and those reserved, are capped by the table's size.
    let mut budget = data.len().saturating_mul(4);
    let stored = records.iter().fold(0usize, |n, (_, raw)| n.saturating_add(raw.len()));
    out.reserve(records.len(), stored.min(budget));
    let mut scratch = String::new();
    for (key, raw) in records {
        let Some(left) = budget.checked_sub(raw.len()) else { break };
        budget = left;
        decode_name_into(raw, key, &mut scratch);
        let trimmed = scratch.trim_matches('\0').trim();
        if !trimmed.is_empty() {
            out.put(key.name_id, trimmed);
        }
    }
}

// Each name ID's record a reader takes, up to `last`, in order of ID: the first of the best score among those it can
// read, in table order, which the stable sort keeps within an ID.
fn best_records(data: &[u8], last: u16) -> Vec<(NameKey, &[u8])> {
    let mut best: Vec<_> = readable_records(data).filter(|r| r.0.name_id <= last).collect();
    best.sort_by_key(|r| r.0.name_id);
    best.dedup_by(|later, kept| {
        let same = later.0.name_id == kept.0.name_id;
        if same && score(later.0) > score(kept.0) {
            *kept = *later;
        }
        same
    });
    best
}

// `data` cut to the records its readers take, each name ID's one, so every name reads as from the whole table. None
// where it wouldn't be smaller, or where its decoding budget, a multiple of its length, could run out sooner.
pub(crate) fn read_records_only(data: &[u8]) -> Option<Vec<u8>> {
    let best = best_records(data, u16::MAX);
    let decoded: usize = best.iter().map(|(_, raw)| raw.len()).sum();
    let lean = write_name_table(best, &[])?;
    (lean.len() < data.len() && decoded <= lean.len().saturating_mul(4)).then_some(lean)
}

pub(crate) const MAC_ROMAN_HIGH: [char; 128] = [
    'Ä','Å','Ç','É','Ñ','Ö','Ü','á','à','â','ä','ã','å','ç','é','è',
    'ê','ë','í','ì','î','ï','ñ','ó','ò','ô','ö','õ','ú','ù','û','ü',
    '†','°','¢','£','§','•','¶','ß','®','©','™','´','¨','≠','Æ','Ø',
    '∞','±','≤','≥','¥','µ','∂','∑','∏','π','∫','ª','º','Ω','æ','ø',
    '¿','¡','¬','√','ƒ','≈','∆','«','»','…','\u{A0}','À','Ã','Õ','Œ','œ',
    '–','—','“','”','‘','’','÷','◊','ÿ','Ÿ','⁄','€','‹','›','ﬁ','ﬂ',
    '‡','·','‚','„','‰','Â','Ê','Á','Ë','È','Í','Î','Ï','Ì','Ó','Ô',
    '\u{F8FF}','Ò','Ú','Û','Ù','ı','ˆ','˜','¯','˘','˙','˚','¸','˝','˛','ˇ',
];

pub(crate) fn mac_roman_char(b: u8) -> char {
    if b < 0x80 {
        return b as char;
    }
    MAC_ROMAN_HIGH[usize::from(b) - 0x80]
}

// MacOS Turkish is MacRoman with seven positions reassigned, per Apple's TURKISH.TXT. A cmap
// subtable on platform 1 encoding 0 names it with language 18.
const MAC_TURKISH_OVERRIDES: [(u8, char); 7] = [
    (0xDA, 'Ğ'), (0xDB, 'ğ'), (0xDC, 'İ'), (0xDD, 'ı'),
    (0xDE, 'Ş'), (0xDF, 'ş'), (0xF5, '\u{F8A0}'),
];

pub(crate) fn mac_turkish_char(b: u8) -> char {
    match MAC_TURKISH_OVERRIDES.iter().find(|&&(k, _)| k == b) {
        Some(&(_, c)) => c,
        None => mac_roman_char(b),
    }
}

pub(crate) fn mac_turkish_byte(codepoint: u32) -> Option<u8> {
    let wanted = char::from_u32(codepoint)?;
    if let Some(&(b, _)) = MAC_TURKISH_OVERRIDES.iter().find(|&&(_, c)| c == wanted) {
        return Some(b);
    }
    let byte = mac_roman_byte(codepoint)?;
    // A byte Turkish reassigned does not stand for whatever MacRoman put there.
    (!MAC_TURKISH_OVERRIDES.iter().any(|&(k, _)| k == byte)).then_some(byte)
}

// MAC_ROMAN_HIGH in character order with each byte, for a binary search back from a character.
const MAC_ROMAN_BY_CHAR: [(char, u8); 128] = {
    let mut table = [('\0', 0u8); 128];
    let mut i = 0;
    while i < 128 {
        table[i] = (MAC_ROMAN_HIGH[i], 0x80 + i as u8);
        let mut j = i;
        while j > 0 && table[j - 1].0 as u32 > table[j].0 as u32 {
            let swap = table[j - 1];
            table[j - 1] = table[j];
            table[j] = swap;
            j -= 1;
        }
        i += 1;
    }
    table
};

pub(crate) fn mac_roman_byte(codepoint: u32) -> Option<u8> {
    if codepoint < 0x80 {
        return Some(codepoint as u8);
    }
    let at = MAC_ROMAN_BY_CHAR.binary_search_by_key(&codepoint, |&(c, _)| c as u32).ok()?;
    Some(MAC_ROMAN_BY_CHAR[at].1)
}

/// A name in one of the languages the name table records it in.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct LocalizedName {
    /// The language as a BCP 47 tag, such as `ja` or `zh-TW`; `und` where the record names none daegun reads.
    pub language: String,
    /// The name.
    pub name: String,
}

pub fn read_localized_names(table_map: &Tables, name_id: u16) -> Vec<LocalizedName> {
    let mut out = Vec::new();
    each_localized_name(table_map, name_id, |language, name| {
        out.push(LocalizedName { language: language.to_string(), name: name.to_string() });
    });
    out
}

// Each language a name is recorded in and the name, sorted by tag, lent to `put`: the record `read_name_string`
// would pick where two give one language, as a Windows record and a Mac one do for English.
pub(crate) fn each_localized_name(table_map: &Tables, name_id: u16, mut put: impl FnMut(&str, &str)) {
    let Some(data) = table_map.get("name") else { return };
    // Records may share one long string, so the bytes decoded, format 1's tags once each and then the names, are
    // capped by the table's size, as for every name; a tag past the cap goes undecoded, its records read as `und`.
    let mut budget = data.len().saturating_mul(4);
    let tags: Vec<String> = lang_tags(data)
        .into_iter()
        .map_while(|raw| {
            budget = budget.checked_sub(raw.len())?;
            let units = raw.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b));
            Some(char::decode_utf16(units).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)).collect())
        })
        .collect();
    let mut best: BTreeMap<&str, (NameKey, &[u8])> = BTreeMap::new();
    for (key, raw) in readable_records(data).filter(|(key, _)| key.name_id == name_id) {
        let language = language_tag(key, &tags);
        if best.get(language).is_none_or(|&(held, _)| score(key) > score(held)) {
            best.insert(language, (key, raw));
        }
    }
    let mut name = String::new();
    for (language, (key, raw)) in best {
        let Some(left) = budget.checked_sub(raw.len()) else { break };
        budget = left;
        decode_name_into(raw, key, &mut name);
        let trimmed = name.trim_matches('\0').trim();
        if !trimmed.is_empty() {
            put(language, trimmed);
        }
    }
}

// A record's language as a BCP 47 tag: format 1's own from 0x8000, as `tags` decoded them, else Windows' or Mac's by
// ID; `und` for none.
fn language_tag(key: NameKey, tags: &[String]) -> &str {
    use super::name_languages::{MAC, WINDOWS};
    let by_id = |table: &[(u16, &'static str)]| {
        table.binary_search_by_key(&key.language, |&(id, _)| id).ok().map(|at| table[at].1)
    };
    let tag = match key.language {
        from @ 0x8000.. => tags.get(usize::from(from - 0x8000)).map(String::as_str),
        _ if key.platform == 3 => by_id(WINDOWS),
        _ if key.platform == 1 => by_id(MAC),
        _ => None,
    };
    tag.filter(|t| !t.is_empty()).unwrap_or("und")
}

/// Whether a font's glyphs stand upright or lean, as its OS/2 and head tables say.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FontStyle {
    /// Upright.
    Normal,
    /// Italic: OS/2's fsSelection or head's macStyle says so.
    Italic,
    /// Oblique, a slanted upright: fsSelection says so, as it may beside italic, which oblique comes before, as
    /// CSS's font matching reads the two.
    Oblique,
}

pub fn read_font_style(table_map: &Tables) -> FontStyle {
    let selection = super::os2::parse_os2(table_map).and_then(|o| o.fs_selection).unwrap_or(0);
    if selection & super::os2::FS_OBLIQUE != 0 {
        return FontStyle::Oblique;
    }
    let italic_os2 = selection & super::os2::FS_ITALIC != 0;

    let italic_head = table_map.get("head")
        .filter(|d| d.len() >= 46)
        .and_then(|d| read_u16_be(d, 44))
        .is_some_and(|v| v & 0x0002 != 0);

    if italic_os2 || italic_head { FontStyle::Italic } else { FontStyle::Normal }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/decoder/name.rs"]
mod tests;
