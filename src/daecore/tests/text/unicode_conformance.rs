use daegun::unicode::{BaseDirection, BidiParagraph, grapheme_boundaries, line_break_opportunities, word_boundaries};

fn read(name: &str) -> String {
    let path = format!("{}/scripts/data/unicode/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).expect("the vendored Unicode data")
}

fn char_of(hex: &str) -> char {
    char::from_u32(u32::from_str_radix(hex, 16).expect("hex")).expect("a scalar value")
}

// Every case of a break test file: its text, and the byte offsets where it may break.
fn break_cases(name: &str) -> Vec<(String, Vec<usize>)> {
    let mut cases = Vec::new();
    for line in read(name).lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let (mut text, mut breaks) = (String::new(), Vec::new());
        for token in line.split_whitespace() {
            match token {
                "÷" => breaks.push(text.len()),
                "×" => {}
                hex => text.push(char_of(hex)),
            }
        }
        cases.push((text, breaks));
    }
    cases
}

fn assert_breaks(name: &str, breaks: impl Fn(&str) -> Vec<usize>) {
    let cases = break_cases(name);
    let wrong: Vec<_> = cases.iter().filter(|(text, want)| breaks(text) != *want).take(10).collect();
    assert!(wrong.is_empty(), "{name}: {wrong:?}");
    assert!(!cases.is_empty());
}

#[test]
fn grapheme_boundaries_pass_the_conformance_test() {
    assert_breaks("GraphemeBreakTest.txt", grapheme_boundaries);
}

#[test]
fn word_boundaries_pass_the_conformance_test() {
    assert_breaks("WordBreakTest.txt", word_boundaries);
}

#[test]
fn line_breaks_pass_the_conformance_test() {
    assert_breaks("LineBreakTest.txt", |text| line_break_opportunities(text, None).iter().map(|b| b.at).collect());
}

// Whether `text` at `base` resolves to `levels`, x where a character is removed, and reorders its kept characters
// as `order`. The files count characters, so each byte answer is read at its character's first byte.
fn resolves(text: &str, base: BaseDirection, levels: &[Option<u8>], order: &[usize]) -> bool {
    let para = BidiParagraph::new(text, base);
    let starts: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    let kept = levels.iter().zip(&starts).all(|(want, &at)| want.is_none_or(|w| w == para.levels[at]));
    let char_of = |at: usize| starts.binary_search(&at).expect("a character's first byte");
    let visual: Vec<usize> = para.visual_order.iter().map(|&at| char_of(at)).filter(|&i| levels[i].is_some()).collect();
    kept && visual == order
}

fn levels_of(field: &str) -> Vec<Option<u8>> {
    field.split_whitespace().map(|l| l.parse().ok()).collect()
}

fn indices_of(field: &str) -> Vec<usize> {
    field.split_whitespace().map(|i| i.parse().expect("an index")).collect()
}

#[test]
fn bidi_passes_the_character_test() {
    let data = read("BidiCharacterTest.txt");
    let (mut cases, mut wrong) = (0, Vec::new());
    for line in data.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let fields: Vec<&str> = line.split(';').collect();
        let text: String = fields[0].split_whitespace().map(char_of).collect();
        let base = match fields[1] {
            "0" => BaseDirection::LeftToRight,
            "1" => BaseDirection::RightToLeft,
            _ => BaseDirection::Auto,
        };
        let para = BidiParagraph::new(&text, base).base_level;
        cases += 1;
        if para.to_string() != fields[2] || !resolves(&text, base, &levels_of(fields[3]), &indices_of(fields[4])) {
            wrong.push(line);
        }
    }
    assert!(wrong.is_empty() && cases > 0, "{} of {cases}: {:?}", wrong.len(), &wrong[..wrong.len().min(10)]);
}

// Each class is spelled by a character of it; the bits ask for auto, left-to-right and right-to-left paragraphs.
#[test]
fn bidi_passes_the_class_test() {
    let char_of_class = |class: &str| match class {
        "L" => 'a',
        "R" => '\u{05D0}',
        "AL" => '\u{0627}',
        "EN" => '1',
        "ES" => '+',
        "ET" => '#',
        "AN" => '\u{0660}',
        "CS" => ',',
        "NSM" => '\u{0300}',
        "BN" => '\u{00AD}',
        "B" => '\u{2029}',
        "S" => '\t',
        "WS" => ' ',
        "ON" => '!',
        "LRE" => '\u{202A}',
        "LRO" => '\u{202D}',
        "RLE" => '\u{202B}',
        "RLO" => '\u{202E}',
        "PDF" => '\u{202C}',
        "LRI" => '\u{2066}',
        "RLI" => '\u{2067}',
        "FSI" => '\u{2068}',
        "PDI" => '\u{2069}',
        other => panic!("class {other}"),
    };
    let (mut levels, mut order) = (Vec::new(), Vec::new());
    let data = read("BidiTest.txt");
    let (mut cases, mut wrong) = (0, Vec::new());
    for line in data.lines() {
        if let Some(field) = line.strip_prefix("@Levels:") {
            levels = levels_of(field);
        } else if let Some(field) = line.strip_prefix("@Reorder:") {
            order = indices_of(field);
        } else if let Some((classes, bits)) = line.split_once(';').filter(|_| !line.starts_with('#')) {
            let text: String = classes.split_whitespace().map(char_of_class).collect();
            let bits: u8 = bits.trim().parse().expect("paragraph bits");
            for (bit, base) in
                [(1, BaseDirection::Auto), (2, BaseDirection::LeftToRight), (4, BaseDirection::RightToLeft)]
            {
                if bits & bit != 0 {
                    cases += 1;
                    if !resolves(&text, base, &levels, &order) {
                        wrong.push((line, base));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty() && cases > 0, "{} of {cases}: {:?}", wrong.len(), &wrong[..wrong.len().min(10)]);
}
