use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::plan::FeatureSetting;
use crate::daecore::daetype::decoder::{midpoint_search, read_u16_be, read_u32_be};
use crate::daecore::daetype::format::aat::{morx_chains, MorxChain};
use crate::daecore::daetype::format::sanitize::{feat_passes, ltag_passes};
use super::tag::{canonical_language, Tag};

const LETTER_CASE: u16 = 3;
const SMALL_CAPS: u16 = 3;
const CHARACTER_ALTERNATIVES: u16 = 17;
const LOWER_CASE: u16 = 37;
const LOWER_CASE_SMALL_CAPS: u16 = 1;
const LANGUAGE_TAG: u16 = 39;
const EXCLUSIVE: u16 = 0x8000;

// Apple's mapping of OpenType features to an AAT feature type and the selectors that turn it on and off, by tag.
const MAPPING: [(&[u8; 4], u16, u16, u16); 78] = [
    (b"afrc", 11, 1, 0), (b"c2pc", 38, 2, 0), (b"c2sc", 38, 1, 0), (b"calt", 36, 0, 1), (b"case", 33, 0, 1),
    (b"clig", 1, 18, 19), (b"cpsp", 33, 2, 3), (b"cswh", 36, 4, 5), (b"dlig", 1, 4, 5), (b"expt", 20, 10, 16),
    (b"frac", 11, 2, 0), (b"fwid", 22, 1, 7), (b"halt", 22, 6, 7), (b"hist", 40, 0, 1), (b"hkna", 34, 0, 1),
    (b"hlig", 1, 20, 21), (b"hngl", 23, 1, 0), (b"hojo", 20, 12, 16), (b"hwid", 22, 2, 7), (b"ital", 32, 2, 3),
    (b"jp04", 20, 11, 16), (b"jp78", 20, 2, 16), (b"jp83", 20, 3, 16), (b"jp90", 20, 4, 16), (b"liga", 1, 2, 3),
    (b"lnum", 21, 1, 2), (b"mgrk", 15, 10, 11), (b"nlck", 20, 13, 16), (b"onum", 21, 0, 2), (b"ordn", 10, 3, 0),
    (b"palt", 22, 5, 7), (b"pcap", 37, 2, 0), (b"pkna", 22, 0, 7), (b"pnum", 6, 1, 4), (b"pwid", 22, 0, 7),
    (b"qwid", 22, 4, 7), (b"rlig", 1, 0, 1), (b"ruby", 28, 2, 3), (b"sinf", 10, 4, 0), (b"smcp", 37, 1, 0),
    (b"smpl", 20, 1, 16), (b"ss01", 35, 2, 3), (b"ss02", 35, 4, 5), (b"ss03", 35, 6, 7), (b"ss04", 35, 8, 9),
    (b"ss05", 35, 10, 11), (b"ss06", 35, 12, 13), (b"ss07", 35, 14, 15), (b"ss08", 35, 16, 17),
    (b"ss09", 35, 18, 19), (b"ss10", 35, 20, 21), (b"ss11", 35, 22, 23), (b"ss12", 35, 24, 25),
    (b"ss13", 35, 26, 27), (b"ss14", 35, 28, 29), (b"ss15", 35, 30, 31), (b"ss16", 35, 32, 33),
    (b"ss17", 35, 34, 35), (b"ss18", 35, 36, 37), (b"ss19", 35, 38, 39), (b"ss20", 35, 40, 41),
    (b"subs", 10, 2, 0), (b"sups", 10, 1, 0), (b"swsh", 36, 2, 3), (b"titl", 19, 4, 0), (b"tnam", 20, 14, 16),
    (b"tnum", 6, 0, 4), (b"trad", 20, 0, 16), (b"twid", 22, 3, 7), (b"unic", 3, 14, 15), (b"valt", 22, 5, 7),
    (b"vert", 4, 0, 1), (b"vhal", 22, 6, 7), (b"vkna", 34, 2, 3), (b"vpal", 22, 5, 7), (b"vrt2", 4, 0, 1),
    (b"vrtr", 4, 2, 3), (b"zero", 14, 4, 5),
];

// A feature type and setting asked for, in the order asked.
#[derive(Clone, Copy)]
struct Setting {
    kind: u16,
    setting: u32,
    exclusive: bool,
    order: usize,
}

// The flags a chain runs with over clusters `first..=last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RangeFlags {
    pub(crate) flags: u32,
    pub(crate) first: u32,
    pub(crate) last: u32,
}

// Each chain's flags over every range the features' starts and ends cut the text into: its defaults, then those of
// every chain feature a setting asked for there or the language names.
pub(crate) fn chain_flags(
    face: &Face,
    morx: &[u8],
    features: &[FeatureSetting],
    language: Option<&str>,
) -> Vec<Vec<RangeFlags>> {
    let ltag = face.table("ltag").filter(|t| ltag_passes(t));
    let chains: Vec<_> = morx_chains(morx).collect();
    let mut out = alloc::vec![Vec::new(); chains.len()];
    for (settings, first, last) in ranges(face.table("feat").filter(|t| feat_passes(t)), features) {
        for (ranges, chain) in out.iter_mut().zip(&chains) {
            let flags = compile(chain, &settings, ltag, language);
            ranges.push(RangeFlags { flags, first, last });
        }
    }
    out
}

fn compile(chain: &MorxChain, settings: &[Setting], ltag: Option<&[u8]>, language: Option<&str>) -> u32 {
    let asked = |kind: u16, setting: u16| settings.iter().any(|s| s.kind == kind && s.setting == u32::from(setting));
    chain.features().fold(chain.default_flags, |flags, (kind, setting, enable, disable)| {
        let spoken = kind == LANGUAGE_TAG && setting != 0
            && matches(ltag.and_then(|t| tag(t, setting - 1)), language);
        // The deprecated small caps setting stands in for lower case small caps.
        let named = asked(kind, setting)
            || (kind, setting) == (LETTER_CASE, SMALL_CAPS) && asked(LOWER_CASE, LOWER_CASE_SMALL_CAPS)
            || spoken;
        if named { flags & disable | enable } else { flags }
    })
}

// The settings in force over each range of clusters, with its first and last, swept from the features' starts and
// ends in order, ends first where they meet. An end takes off the setting its start put on, and a range ending at or
// before its start holds nowhere, as under GSUB.
fn ranges(feat: Option<&[u8]>, features: &[FeatureSetting]) -> Vec<(Vec<Setting>, u32, u32)> {
    let asked: Vec<(Setting, u32, u32)> = feat.map_or_else(Vec::new, |feat| {
        features.iter().enumerate().filter_map(|(order, f)| Some((setting(feat, f, order)?, f.start, f.end))).collect()
    });
    if asked.is_empty() {
        return alloc::vec![(Vec::new(), 0, u32::MAX)];
    }
    let mut events: Vec<(u32, bool, Setting)> =
        asked.iter().filter(|a| a.1 < a.2).flat_map(|&(s, start, end)| [(start, true, s), (end, false, s)]).collect();
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(priority(&a.2, &b.2)));
    events.push((u32::MAX, false, Setting { kind: 0, setting: 0, exclusive: false, order: usize::MAX }));
    let (mut out, mut active, mut last) = (Vec::new(), Vec::<Setting>::new(), 0);
    for (index, start, s) in events {
        if index != last {
            out.push((merged(active.clone()), last, index - 1));
            last = index;
        }
        if start {
            active.push(s);
        } else if let Some(k) = active.iter().position(|a| a.order == s.order) {
            active.remove(k);
        }
    }
    if let Some(tail) = out.last_mut() {
        tail.2 = u32::MAX;
    }
    out
}

// The setting a feature asks for where feat names its type, small caps falling back to the deprecated letter case type.
fn setting(feat: &[u8], f: &FeatureSetting, order: usize) -> Option<Setting> {
    if f.tag == Tag::new(b"aalt") {
        let setting = Setting { kind: CHARACTER_ALTERNATIVES, setting: f.value, exclusive: true, order };
        return (named(feat, CHARACTER_ALTERNATIVES).0 != 0).then_some(setting);
    }
    let at = MAPPING.binary_search_by_key(&f.tag.0, |m| Tag::new(m.0).0).ok()?;
    let (_, kind, on, off) = MAPPING[at];
    let (mut count, mut flags) = named(feat, kind);
    if count == 0 && (kind, on) == (LOWER_CASE, LOWER_CASE_SMALL_CAPS) {
        (count, flags) = named(feat, LETTER_CASE);
    }
    let setting = u32::from(if f.value != 0 { on } else { off });
    (count != 0).then_some(Setting { kind, setting, exclusive: flags & EXCLUSIVE != 0, order })
}

// By type, then by value where a type isn't exclusive and the two aren't one setting's on and off, then the last
// asked first.
fn priority(a: &Setting, b: &Setting) -> core::cmp::Ordering {
    let by_setting = !a.exclusive && a.setting & !1 != b.setting & !1;
    a.kind.cmp(&b.kind).then(if by_setting { a.setting.cmp(&b.setting) } else { b.order.cmp(&a.order) })
}

// Sorted, keeping the last asked of one exclusive type or of one setting's on and off, as an OpenType feature asked
// twice takes its last value.
fn merged(mut settings: Vec<Setting>) -> Vec<Setting> {
    settings.sort_by(priority);
    settings.dedup_by(|later, kept| {
        later.kind == kept.kind && (later.exclusive || later.setting & !1 == kept.setting & !1)
    });
    settings
}

// feat's settings count and flags for a feature type, its name found as a midpoint search finds it; none gives 0.
fn named(feat: &[u8], kind: u16) -> (u16, u16) {
    let count = read_u16_be(feat, 4).map_or(0, usize::from);
    let kind_at = |i: usize| read_u16_be(feat, 12 + 12 * i);
    midpoint_search(count, |i| kind_at(i).map(|k| kind.cmp(&k)))
        .and_then(|i| Some((read_u16_be(feat, 12 + 12 * i + 2)?, read_u16_be(feat, 12 + 12 * i + 8)?)))
        .unwrap_or((0, 0))
}

// ltag's `index`th language tag, in its canonical form; none past its count or where it's empty.
fn tag(ltag: &[u8], index: u16) -> Option<String> {
    let range = 12 + 4 * usize::from(index);
    if u32::from(index) >= read_u32_be(ltag, 8)? {
        return None;
    }
    let (at, len) = (usize::from(read_u16_be(ltag, range)?), usize::from(read_u16_be(ltag, range + 2)?));
    canonical_language(ltag.get(at..at + len.min(63))?)
}

// Whether `specific` is `language`, or more specific: `language` followed by a hyphen and more. No language matches
// only no language.
fn matches(language: Option<String>, specific: Option<&str>) -> bool {
    match (language.as_deref(), specific) {
        (Some(l), Some(s)) => s.strip_prefix(l).is_some_and(|rest| rest.is_empty() || rest.starts_with('-')),
        (l, s) => l == s,
    }
}
