use alloc::vec::Vec;

use super::{canonical_language, language_tags};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-data/language-tags.tsv");

fn tags_of(language: &str) -> Vec<u32> {
    canonical_language(language.as_bytes())
        .map_or_else(Vec::new, |l| language_tags(&l).as_slice().iter().map(|t| t.0).collect())
}

// Each language tag in the fixture gives the tags the fixture lists for it: every registered subtag, extlang,
// grandfathered and redundant tag, each rule's tag in several forms, and edges. `record_language_tags` writes it.
#[test]
fn language_tags_match_the_fixture() {
    let fixture = std::fs::read_to_string(FIXTURE).expect("the fixture");
    let mut wrong = Vec::new();
    for line in fixture.lines() {
        let (language, want) = line.split_once('\t').expect("a tab");
        let want: Vec<u32> = want.split_whitespace().map(|t| u32::from_str_radix(t, 16).expect("hex")).collect();
        let got = tags_of(language);
        if got != want {
            wrong.push((language, got, want));
        }
    }
    let shown = &wrong[..wrong.len().min(20)];
    assert!(wrong.is_empty(), "{} of {} differ: {shown:x?}", wrong.len(), fixture.lines().count());
}

// Rewrites the fixture's tags from the rules as they stand, for a change made on purpose.
#[test]
#[ignore = "rewrites assets/test-data/language-tags.tsv"]
fn record_language_tags() {
    let fixture = std::fs::read_to_string(FIXTURE).expect("the fixture");
    let mut out = alloc::string::String::new();
    for line in fixture.lines() {
        let language = line.split_once('\t').expect("a tab").0;
        let tags: Vec<_> = tags_of(language).iter().map(|t| alloc::format!("{t:08X}")).collect();
        out += &alloc::format!("{language}\t{}\n", tags.join(" "));
    }
    std::fs::write(FIXTURE, out).expect("written");
}
