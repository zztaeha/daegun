use super::*;

// Paragraphs of every class but R, AL and the explicit ones, left to right or of no set direction, resolve as the whole
// algorithm resolves them; those holding an Arabic digit (AN) take the whole algorithm.
#[test]
fn a_left_to_right_paragraph_resolves_as_the_whole_algorithm_does() {
    // L, EN, AN, ES, ET, CS, NSM, BN, S, WS, ON and B.
    let pool = ['a', 'Z', '7', '\u{0663}', '+', '-', '$', '%', ',', '.', ':', '\u{0301}', '\u{200B}', '\u{00AD}', '\t',
        ' ', '\u{2003}', '!', '(', ')', '"', '\u{2029}'];
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let mut fast = 0;
    for round in 0..4000 {
        let chars: Vec<char> = (0..1 + next(24)).map(|_| pool[next(pool.len())]).collect();
        let classes: Vec<Class> = chars.iter().map(|&c| class_of(c)).collect();
        let base = [None, Some(false)][round % 2];
        let (mut quick, mut whole) = (vec![9; chars.len()], vec![9; chars.len()]);
        let level = resolve_paragraph(&chars, &classes, base, &mut quick);
        assert_eq!((level, &quick), (resolve_fully(&chars, &classes, base, &mut whole), &whole), "{chars:?} {base:?}");
        fast += usize::from(!classes.iter().any(|&c| matches!(c, Class::AN)));
    }
    assert!(fast > 1000, "only {fast} paragraphs took the fast path");
}

// A line at its paragraph's level throughout, trailing spaces and all, reorders into the one run it's given as.
#[test]
fn a_line_at_its_paragraphs_level_is_one_run() {
    for text in ["plain words, 12.5% ", "\u{05D0}\u{05D1} \u{05D2}  ", "a\tb\u{2003}", "\u{0301}x"] {
        for base_level in [0, 1, 2] {
            let levels = vec![base_level; text.len()];
            let ends = |&(a, b): &(usize, usize)| text.is_char_boundary(a) && text.is_char_boundary(b);
            for (start, end) in [(0, text.len()), (1, text.len() - 1)].into_iter().filter(ends) {
                let whole = reordered(base_level, &levels, &text[start..end], start, VisualRuns::default());
                let quick = line_visual_runs(base_level, &levels, text, start, end);
                assert_eq!(quick.0, whole.0, "{text:?} at level {base_level}, bytes {start} to {end}");
            }
        }
    }
}
