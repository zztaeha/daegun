use daegun::{BreakStrategy, Font, LayoutOptions};

fn font() -> Font {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).expect("Inter parses")
}

// A line with nothing to fit into must not break at every word. An infinite measure over a line's
// infinite stretch is NaN, which loses every comparison in the optimal search and so breaks at each
// opportunity; `ratio_between` answers 0.0 for that pair instead.
#[test]
fn an_unbounded_line_does_not_break_at_every_word() {
    let f = font();
    let text = "The optimal strategy searches for the least bad set of breaks in a paragraph.";
    let words = text.split_whitespace().count();

    for strategy in [BreakStrategy::Optimal, BreakStrategy::Greedy] {
        let opts = LayoutOptions { strategy, ..LayoutOptions::default() };
        let layout = f.layout(text, &[], &opts).expect("lays out");
        assert_eq!(
            layout.lines.len(),
            1,
            "{strategy:?} broke {} words into {} lines with no width to fit, which is what an \
             unguarded NaN ratio does",
            words,
            layout.lines.len(),
        );
    }
}
