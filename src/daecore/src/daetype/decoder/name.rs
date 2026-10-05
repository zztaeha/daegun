use alloc::string::String;
use alloc::string::ToString;
use alloc::collections::BTreeMap;
use super::io::read_u16_be;
use crate::daecore::daetype::TableBytes;

pub fn read_font_family_name(table_map: &BTreeMap<String, TableBytes>) -> Option<String> {
    read_name_string(table_map, 16).or_else(|| read_name_string(table_map, 1))
}

pub fn read_name_string(table_map: &BTreeMap<String, TableBytes>, name_id: u16) -> Option<String> {
    let data = table_map.get("name")?;
    let (_, platform, raw) = records(data)
        .filter(|r| r.0 == name_id)
        .reduce(|best, r| if score(r.1) > score(best.1) { r } else { best })?;
    let mut name = String::new();
    decode_name_into(raw, platform.0, &mut name);
    let trimmed = name.trim_matches('\0').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

// As FreeType picks a name: English Windows, then Windows in any language, then Mac Roman in English,
// then any Mac Roman, then Unicode. Mac scripts other than Roman have no decoder here.
fn score((platform, encoding, language): (u16, u16, u16)) -> u8 {
    match (platform, encoding) {
        (3, 0 | 1 | 10) if language & 0x3FF == 0x009 => 5,
        (3, 0 | 1 | 10) => 4,
        (1, 0) if language == 0 => 3,
        (1, 0) => 2,
        (0, _) => 1,
        _ => 0,
    }
}

type Record<'a> = (u16, (u16, u16, u16), &'a [u8]);

// Each record a name can be read from: (name ID, (platform, encoding, language), its bytes). Records
// that are empty, past the table or in an encoding with no decoder are skipped, as FreeType does.
fn records(data: &[u8]) -> impl Iterator<Item = Record<'_>> {
    let count = read_u16_be(data, 2).map_or(0, usize::from);
    let storage = read_u16_be(data, 4).map_or(0, usize::from);
    (0..count).filter_map(move |i| {
        let rec = 6 + i * 12;
        let field = |at: usize| read_u16_be(data, rec + at);
        let key = (field(0)?, field(2)?, field(4)?);
        let (length, offset) = (usize::from(field(8)?), usize::from(field(10)?));
        let raw = data.get(storage + offset..storage + offset + length)?;
        (length > 0 && score(key) > 0).then_some((field(6)?, key, raw))
    })
}

fn decode_name_into(raw: &[u8], platform: u16, out: &mut String) {
    out.clear();
    if platform == 1 {
        out.extend(raw.iter().map(|&b| mac_roman_char(b)));
    } else {
        out.extend(char::decode_utf16(raw.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes([b[0], b[1]])))
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)));
    }
}

pub fn parse_all_name_strings(table_map: &BTreeMap<String, TableBytes>) -> BTreeMap<u16, String> {
    let mut out = BTreeMap::new();
    let Some(data) = table_map.get("name") else { return out };
    let mut best: BTreeMap<u16, Record> = BTreeMap::new();
    for record in records(data) {
        let entry = best.entry(record.0).or_insert(record);
        if score(record.1) > score(entry.1) {
            *entry = record;
        }
    }

    // Records may all share one long string, so the bytes decoded are capped by the table's size.
    let mut budget = data.len().saturating_mul(4);
    let mut scratch = String::new();
    for (name_id, (_, key, raw)) in best {
        let Some(left) = budget.checked_sub(raw.len()) else { break };
        budget = left;
        decode_name_into(raw, key.0, &mut scratch);
        let trimmed = scratch.trim_matches('\0').trim();
        if !trimmed.is_empty() {
            out.insert(name_id, trimmed.to_string());
        }
    }
    out
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

pub fn read_font_style(table_map: &BTreeMap<String, TableBytes>) -> &'static str {
    let italic_os2 = super::os2::parse_os2(table_map)
        .and_then(|o| o.fs_selection)
        .is_some_and(|v| v & 0x0001 != 0);

    let italic_head = table_map.get("head")
        .filter(|d| d.len() >= 46)
        .and_then(|d| read_u16_be(d, 44))
        .is_some_and(|v| v & 0x0002 != 0);

    if italic_os2 || italic_head { "italic" } else { "normal" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_bytes_and_characters_round_trip() {
        for b in 0..=0xFFu8 {
            assert_eq!(mac_roman_byte(mac_roman_char(b) as u32), Some(b), "MacRoman 0x{b:02X}");
            assert_eq!(mac_turkish_byte(mac_turkish_char(b) as u32), Some(b), "MacOS Turkish 0x{b:02X}");
        }
        assert_eq!(mac_roman_byte(0x0100), None);
        assert_eq!(mac_roman_byte('Ğ' as u32), None);
    }
}
