use alloc::string::String;

use crate::daecore::daeshaper::generated::language_tags::{ANY_SPAN, LANGUAGE_TAGS, TAG_RULES};
use crate::daecore::daeshaper::unicode::Script;

pub use crate::daecore::daekit::base::tag::Tag;

// From the OpenType script tag registry: the scripts whose tag isn't their ISO 15924 code lowercased.
const OWN_TAGS: [(&str, &[u8; 4]); 5] =
    [("Hira", b"kana"), ("Laoo", b"lao "), ("Nkoo", b"nko "), ("Vaii", b"vai "), ("Yiii", b"yi  ")];

// The scripts the registry gives a second tag for their second shaping spec, by the stem its digit follows. All but
// Myanmar answer to a third too, under which a font asks for the Universal Shaping Engine's model (`script::select`).
const INDIC_STEMS: [(&str, &[u8; 3]); 10] = [
    ("Beng", b"bng"), ("Deva", b"dev"), ("Gujr", b"gjr"), ("Guru", b"gur"), ("Knda", b"knd"),
    ("Mlym", b"mlm"), ("Mymr", b"mym"), ("Orya", b"ory"), ("Taml", b"tml"), ("Telu", b"tel"),
];

// A script's OpenType tags, newest first; DFLT for one that takes its script from the text around it.
pub fn script_tags(script: Script) -> Tags {
    let mut tags = Tags::default();
    let Some(iso) = script.iso15924().filter(|_| !script.is_context_dependent()) else {
        tags.push(Tag::DEFAULT_SCRIPT);
        return tags;
    };
    if let Some(&(_, &[a, b, c])) = INDIC_STEMS.iter().find(|(code, _)| *code == iso) {
        let versions: &[u8] = if iso == "Mymr" { b"2" } else { b"32" };
        versions.iter().for_each(|&digit| tags.push(Tag::new(&[a, b, c, digit])));
    }
    let own = OWN_TAGS.iter().find(|(code, _)| *code == iso).map(|&(_, tag)| *tag);
    tags.push(Tag::new(&own.unwrap_or_else(|| lowercased(iso))));
    tags
}

fn lowercased(iso: &str) -> [u8; 4] {
    let mut tag = [b' '; 4];
    tag.iter_mut().zip(iso.bytes()).for_each(|(to, from)| *to = from.to_ascii_lowercase());
    tag
}

// At most three tags, kept without allocating: a script's versions, or a language's systems.
#[derive(Debug)]
pub struct Tags {
    tags: [Tag; 3],
    len: usize,
}

impl Default for Tags {
    fn default() -> Tags {
        Tags { tags: [Tag(0); 3], len: 0 }
    }
}

impl Tags {
    pub(crate) fn push(&mut self, tag: Tag) {
        if let Some(slot) = self.tags.get_mut(self.len) {
            *slot = tag;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[Tag] {
        &self.tags[..self.len]
    }
}

impl IntoIterator for Tags {
    type Item = Tag;
    type IntoIter = core::iter::Take<core::array::IntoIter<Tag, 3>>;

    fn into_iter(self) -> Self::IntoIter {
        self.tags.into_iter().take(self.len)
    }
}

// A language tag as the shaper compares it: up to its first NUL, lowercase, `_` read as `-`, and cut at the first
// byte that can't stand in a tag. None for an empty one.
pub(crate) fn canonical_language(tag: &[u8]) -> Option<String> {
    canonical_language_bytes(tag).map(|bytes| bytes.map(char::from).collect())
}

// `canonical_language`'s bytes, read without building it.
pub(crate) fn canonical_language_bytes(tag: &[u8]) -> Option<impl Iterator<Item = u8> + '_> {
    let tag = tag.split(|&b| b == 0).next().filter(|t| !t.is_empty())?;
    Some(tag.iter().map_while(|&b| match b {
        b'a'..=b'z' | b'0'..=b'9' | b'-' => Some(b),
        b'A'..=b'Z' => Some(b.to_ascii_lowercase()),
        b'_' => Some(b'-'),
        _ => None,
    }))
}

// Where a rule wants the language tag's language.
pub(crate) enum Head {
    // Anywhere: the rule's subtags alone decide.
    Any,
    // The whole tag, as written: a grandfathered one, which no other rule reads.
    Whole(&'static str),
    // At the start, with its hyphen.
    Language(&'static str),
    // At the start with its script, and region if any, each subtag whole and before the limit.
    Prefix(&'static str),
}

// Tags for a language tag that matches the head and holds every one of the subtags.
pub(crate) struct Rule {
    pub(crate) head: Head,
    pub(crate) subtags: &'static [&'static str],
    pub(crate) tags: &'static [[u8; 4]],
}

impl Rule {
    fn matches(&self, language: &str, limit: usize, any: bool) -> bool {
        let head = match self.head {
            Head::Any => any,
            Head::Whole(_) => false,
            Head::Language(start) => language.starts_with(start),
            Head::Prefix(start) => {
                limit >= start.len()
                    && language.starts_with(start)
                    && matches!(language.as_bytes().get(start.len()), None | Some(b'-'))
            }
        };
        head && self.subtags.iter().all(|s| has_subtag(language, limit, s))
    }
}

// Whether `subtag`, with its leading hyphen, stands whole in `language` before `limit`.
fn has_subtag(language: &str, limit: usize, subtag: &str) -> bool {
    let whole = |at: usize| !language.as_bytes().get(at + subtag.len()).is_some_and(u8::is_ascii_alphanumeric);
    language.match_indices(subtag).map(|(at, _)| at).take_while(|&at| at < limit).any(whole)
}

// The OpenType language systems for a canonical language tag, most specific first. Matching stops at its first
// singleton, where a private-use or extension section starts.
pub(crate) fn language_tags(language: &str) -> Tags {
    let mut tags = Tags::default();
    let bytes = language.as_bytes();
    if bytes.starts_with(b"x-") {
        return tags;
    }
    let singleton = (1..bytes.len().saturating_sub(1)).find(|&i| bytes[i - 1] == b'-' && bytes[i + 1] == b'-');
    let mut limit = singleton.map_or(bytes.len(), |i| i - 1);
    // A grandfathered tag is irregular, so it's matched as written. Any other is read from an extended language
    // subtag, three letters after the first, on: that's the language, as RFC 5646's canonical form writes it.
    let mut rule = TAG_RULES.iter().find(|r| matches!(r.head, Head::Whole(tag) if tag == language));
    let mut language = language;
    if let Some((first, rest)) = language.split_once('-')
        && limit >= 6
        && rest.split('-').next().is_some_and(|e| e.len() == 3 && e.bytes().all(|b| b.is_ascii_lowercase()))
    {
        limit -= first.len() + 1;
        language = rest;
    }
    let hyphen = language.find('-');
    let any = hyphen.is_some_and(|at| limit >= ANY_SPAN + 2 && at < limit && limit - at >= ANY_SPAN);
    rule = rule.or_else(|| TAG_RULES.iter().find(|r| r.matches(language, limit, any)));
    if let Some(rule) = rule {
        rule.tags.iter().for_each(|t| tags.push(Tag::new(t)));
        return tags;
    }

    let subtag = language.split('-').next().unwrap_or_default().as_bytes();
    if !(2..=3).contains(&subtag.len()) || !subtag.iter().all(u8::is_ascii_lowercase) {
        return tags;
    }
    let mut key = [0; 3];
    key[..subtag.len()].copy_from_slice(subtag);
    let rows = &LANGUAGE_TAGS[LANGUAGE_TAGS.partition_point(|&(k, _)| k < key)..];
    let mut found = rows.iter().take_while(|&&(k, _)| k == key).peekable();
    if found.peek().is_none() {
        // An unlisted three-letter subtag is taken as an ISO 639-3 code, which is its tag in capitals.
        if subtag.len() == 3 {
            let [a, b, c] = key.map(|k| k.to_ascii_uppercase());
            tags.push(Tag::new(&[a, b, c, b' ']));
        }
        return tags;
    }
    found.map_while(|(_, t)| (*t != [0; 4]).then(|| Tag::new(t))).for_each(|t| tags.push(t));
    tags
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/tag.rs"]
mod tests;
