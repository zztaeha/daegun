use super::*;

std::thread_local! {
    // The pairs `cluster` has handed the models to compose, on this thread.
    pub(super) static PAIRS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

#[test]
fn a_script_whose_shaper_decomposes_everything_is_found_after_others_repeat() {
    assert!(!decomposes_all(&['a', 'b', ' ', 'a', '1']));
    assert!(decomposes_all(&['a', 'b', ' ', 'a', '\u{0915}']), "Devanagari takes the Indic shaper");
}

// Every pair any model composes has a second `composes_second` takes: each code's decompositions by every model, and
// each Hebrew letter or presentation form before each Hebrew mark, as Hebrew composes some no decomposition names.
#[test]
fn every_second_a_model_composes_is_one_a_cluster_composes_onto() {
    let composes = |a: char, b: char| script::ALL.iter().any(|m| m.compose(a, b, false).is_some());
    let decomposed = (0..=0x10FFFF).filter_map(char::from_u32).flat_map(|c| {
        script::ALL.iter().filter_map(move |m| m.decompose(c)).filter_map(|(a, b)| Some((a, b?)))
    });
    let hebrew = ('\u{0590}'..='\u{05FF}').chain('\u{FB1D}'..='\u{FB4F}');
    let hebrew = hebrew.flat_map(|a| ('\u{0590}'..='\u{05FF}').map(move |b| (a, b)));
    for (a, b) in decomposed.chain(hebrew) {
        assert!(!composes(a, b) || composes_second(b), "U+{:04X} U+{:04X}", u32::from(a), u32::from(b));
    }
}

// A base and 400 distinct marks, a few of which compose onto it: each pair goes to the models once, and only with a
// mark some model composes onto. Each mark tried with each, every round, would make some 500,000.
#[test]
fn a_cluster_composes_each_pair_once_and_only_onto_a_second() {
    let marks = (0..=0x10FFFF).filter_map(char::from_u32).filter(|&c| is_mark(c));
    let chars: Vec<char> = core::iter::once('a').chain(marks.take(400)).collect();
    let pairs = || PAIRS.try_with(core::cell::Cell::take).expect("the thread's counter");
    pairs();
    let pieces = cluster(&chars, false, &|_| true);
    let seconds = pieces.iter().filter(|&&c| is_mark(c) && composes_second(c)).count();
    assert!(pieces.contains(&'\u{E0}') && pieces.contains(&'\u{301}'));
    let pairs = pairs();
    assert!(pairs <= pieces.len() * seconds, "{pairs} pairs");
}
