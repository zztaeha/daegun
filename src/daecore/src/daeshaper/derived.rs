use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::normalize::hyphen_stand_in;
use super::script::{self, thai, Normalization};
use super::unicode::{self, SpaceFallback};

// Every code shaping `text` can look up in a font that `maps` codes: the text's own and each one the shaper of any
// script in it derives from them. A subset mapping all of these as its font does shapes `text` alike.
pub(crate) fn shaping_codes(text: &str, maps: impl Fn(u32) -> bool) -> BTreeSet<u32> {
    let chars: Vec<char> = text.chars().collect();
    // A run takes one script for all its codes, so a shaper that decomposes everything can take any code in the text.
    let all = decomposes_all(&chars);
    let mut codes = BTreeSet::new();
    let mut clusters = BTreeSet::new();
    let mut start = 0;
    while start < chars.len() {
        let end = start + 1 + chars[start + 1..].iter().take_while(|&&c| is_mark(c)).count();
        if clusters.insert(&chars[start..end]) {
            codes.extend(cluster(&chars[start..end], all, &maps));
        }
        start = end;
    }
    // Jamo the Hangul shaper joins into a syllable, two or three at a time.
    for (i, &a) in chars.iter().enumerate() {
        let Some(ab) = chars.get(i + 1).and_then(|&b| unicode::compose(a, b)) else { continue };
        codes.insert(ab);
        codes.extend(chars.get(i + 2).and_then(|&c| unicode::compose(ab, c)));
    }
    // A circle stands before a mark opening the text or a sign with no base; a decomposition's pieces follow their own
    // base, so only the text's codes count.
    if chars.iter().any(|&c| is_mark(c) || script::may_lack_its_base(c)) {
        codes.insert(unicode::DOTTED_CIRCLE);
    }
    codes.into_iter().map(u32::from).collect()
}

// A code and the marks after it, as the normalizer takes them: everything each derives, and whatever any shaper
// composes of one with another's mark, until nothing new comes.
fn cluster(chars: &[char], all: bool, maps: &impl Fn(u32) -> bool) -> BTreeSet<char> {
    // Alone, a code the font maps is decomposed only by a shaper that decomposes everything.
    let decomposes = |c: char| all || chars.len() > 1 || !maps(c as u32);
    let (mut pieces, mut found, mut seconds) = (BTreeSet::new(), Vec::new(), Vec::new());
    let mut queue = chars.to_vec();
    // Each round composes only the pairs a new piece makes, as the earlier ones gave all they give.
    let (mut old, mut old_seconds) = (0, 0);
    while !queue.is_empty() {
        while let Some(c) = queue.pop() {
            if pieces.insert(c) {
                found.push(c);
                derive(c, decomposes(c), maps, &mut queue);
            }
        }
        seconds.extend(found[old..].iter().copied().filter(|&c| is_mark(c) && composes_second(c)));
        for (i, &a) in found.iter().enumerate() {
            for &b in &seconds[if i < old { old_seconds } else { 0 }..] {
                #[cfg(test)]
                let _ = tests::PAIRS.try_with(|n| n.set(n.get() + 1));
                queue.extend(script::ALL.iter().filter_map(|model| model.compose(a, b, false)));
            }
        }
        (old, old_seconds) = (found.len(), seconds.len());
        queue.retain(|c| !pieces.contains(c));
    }
    pieces
}

// Whether any model composes a code with `c` after it: as canonical composition or a Hebrew presentation form does,
// or Bengali's nukta, which the Indic model composes onto YA.
fn composes_second(c: char) -> bool {
    unicode::composes_second(c) || c == '\u{09BC}'
}

// The codes shaping can turn `c` into or look up beside it, whether or not the font maps `c`.
fn derive(c: char, decomposes: bool, maps: &impl Fn(u32) -> bool, out: &mut Vec<char>) {
    for model in script::ALL {
        if let Some((a, b)) = model.decompose(c).filter(|_| decomposes) {
            // The normalizer gives up on a decomposition whose second piece the font lacks, before reading the first.
            if b.is_none_or(|b| maps(b as u32)) {
                out.push(a);
            }
            out.extend(b);
        }
    }
    out.extend(unicode::mirrored(c));
    out.extend(unicode::vertical_form(c));
    out.extend(thai::forms_of(c));
    // A missing space is drawn with the space glyph, a figure or punctuation space as wide as a digit or a period.
    let space = unicode::space_fallback(c);
    out.extend(space.map(SpaceFallback::width_sources).unwrap_or_default());
    if space.is_some() || unicode::is_default_ignorable(c) {
        out.push(' ');
    }
    out.extend(hyphen_stand_in(c));
}

// Whether any code's script takes a shaper that decomposes everything, each script's shaper selected once.
fn decomposes_all(chars: &[char]) -> bool {
    let mut seen = Vec::new();
    chars.iter().map(|&c| unicode::script(c)).any(|s| {
        if seen.contains(&s) { return false; }
        seen.push(s);
        script::select(Some(s), true, None, None).normalization() == Normalization::Recomposed
    })
}

fn is_mark(c: char) -> bool {
    unicode::general_category(c).is_mark()
}

#[cfg(test)]
#[path = "../../tests/unit/daeshaper/derived.rs"]
mod tests;
