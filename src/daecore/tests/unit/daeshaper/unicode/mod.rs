use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::{compose, decompose, general_category, horizontal_direction, mirrored, props, script, t};
use super::Script;
use crate::daecore::daeshaper::run::Direction;

// Every field of the ASCII copies is the trie's.
#[test]
fn asciis_records_are_the_tries() {
    let fields = |p: &t::Props| {
        (p.script, p.general_category, p.combining_class, p.flags, p.joining_type, p.grapheme_break, p.word_break)
    };
    let more = |p: &t::Props| (p.bidi_class, p.indic_conjunct_break, p.line_break, p.east_asian_width);
    for c in (0..0x80).filter_map(char::from_u32) {
        let (copy, trie) = (props(c), t::PROPS.get(u32::from(c)));
        assert!(fields(copy) == fields(trie) && more(copy) == more(trie), "U+{:04X}", u32::from(c));
        assert_eq!(copy.vertical_orientation, trie.vertical_orientation, "U+{:04X}", u32::from(c));
    }
}

// The flags that stand before a table's search are set exactly where the table holds the code point.
#[test]
fn a_characters_table_flags_say_what_the_tables_hold() {
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        let (cp, flags) = (u32::from(c), props(c).flags);
        let mirror = t::MIRRORING.binary_search_by_key(&cp, |&(k, _)| k).is_ok();
        let bracket = t::BIDI_BRACKETS.binary_search_by_key(&cp, |&(k, _)| k).is_ok();
        let extended = t::SCRIPT_EXTENSIONS.iter().any(|&(first, last, _)| (first..=last).contains(&cp));
        assert_eq!(flags & t::flag::HAS_MIRROR != 0, mirror, "U+{cp:04X}");
        assert_eq!(flags & t::flag::BRACKET != 0, bracket, "U+{cp:04X}");
        assert_eq!(flags & t::flag::SCRIPT_EXTENSIONS != 0, extended, "U+{cp:04X}");
    }
}

// The UCD's names for the categories, in `GeneralCategory`'s order.
const CATEGORIES: [&str; 30] = [
    "Cn", "Cc", "Cf", "Co", "Cs", "Ll", "Lm", "Lo", "Lt", "Lu", "Mc", "Me", "Mn", "Nd", "Nl", "No", "Pc", "Pd",
    "Pe", "Pf", "Pi", "Po", "Ps", "Sc", "Sk", "Sm", "So", "Zl", "Zp", "Zs",
];

// Each code point's category and major class, combining class, script, mirror and decomposition, its parts composing
// back, and each script's direction, as the fixture lists them from Unicode's files (`build_unicode_fixture.py`).
#[test]
fn unicode_properties_match_the_fixture() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-data/unicode-properties.tsv");
    let fixture = std::fs::read_to_string(path).expect("the fixture");
    let hex = |s: &str| u32::from_str_radix(s, 16).expect("hex");
    let at = |s: &str| char::from_u32(hex(s)).expect("a scalar value");
    let (mut mirrors, mut parts, mut directions) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut wrong = Vec::<String>::new();
    for line in fixture.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "p" => {
                for c in (hex(f[1])..=hex(f[2])).filter_map(char::from_u32) {
                    let iso = script(c).iso15924().unwrap_or("Zzzz");
                    let category = general_category(c);
                    let class = props(c).combining_class.to_string();
                    let major = (category.is_mark(), category.is_letter());
                    let named = (f[3].starts_with('M'), f[3].starts_with('L'));
                    if (CATEGORIES[category as usize], class.as_str(), iso) != (f[3], f[4], f[5]) || major != named {
                        wrong.push(line.to_string());
                        break;
                    }
                }
            }
            "m" => _ = mirrors.insert(at(f[1]), at(f[2])),
            "d" => _ = parts.insert(at(f[1]), (at(f[2]), (f[3] != "-").then(|| at(f[3])), f[4] == "1")),
            _ => _ = directions.insert(f[1], f[2]),
        }
    }
    for (i, iso) in t::SCRIPT_ISO_CODES.iter().enumerate() {
        let got = match horizontal_direction(Script(i as u16)) {
            Direction::RightToLeft => "R",
            _ => "L",
        };
        if directions.get(iso) != Some(&got) {
            wrong.push(format!("{iso} runs {got}"));
        }
    }
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        let want = parts.get(&c);
        let composes = |(a, b): (char, Option<char>)| b.is_some_and(|b| compose(a, b) == Some(c));
        if mirrored(c) != mirrors.get(&c).copied()
            || decompose(c) != want.map(|&(a, b, _)| (a, b))
            || want.is_some_and(|&(a, b, back)| composes((a, b)) != back)
        {
            wrong.push(format!("{:04X}", c as u32));
        }
    }
    assert!(wrong.is_empty(), "{} differ: {:?}", wrong.len(), &wrong[..wrong.len().min(20)]);
}
