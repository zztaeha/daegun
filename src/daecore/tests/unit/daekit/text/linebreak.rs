use super::*;

// A justified paragraph on a measure of 100: (ink, hanging space, hyphen) per segment, the last forced.
fn breaks(segments: &[(f64, f64, Option<f64>)]) -> Vec<usize> {
    let mut bps = alloc::vec![Breakpoint::start()];
    for (i, &(ink, space, hyphen)) in segments.iter().enumerate() {
        let forced = i + 1 == segments.len();
        let (stretch, shrink) = (space / 2.0, space / 3.0);
        let tail = hyphen.map(|gain| Reshaped { gain, range: (i, i + 1), shaped: (i, i + 1) });
        let (at, hyphen) = (i + 1, hyphen.is_some());
        bps.push(Breakpoint { at, ink, space, stretch, shrink, forced, hyphen, tail, ..Breakpoint::start() });
    }
    let fit = Fit { target: 100.0, line_end_stretch: 0.0 };
    let lines = break_lines(&bps, &fit, BreakStrategy::Optimal, |_, _| unreachable!("no tabs"));
    lines.iter().map(|l| bps[l.to].at).collect()
}

const WORD: (f64, f64, Option<f64>) = (14.5, 2.0, None);

// Six words fill 97 with a ratio of 0.6, demerits about 1,000; a seventh piece ending at a hyphen fills
// 100 exactly, demerits 100 plus the hyphen's 2,500.
#[test]
fn a_hyphen_break_pays_its_penalty() {
    let words = |hyphen| breaks(&[WORD, WORD, WORD, WORD, WORD, WORD, (1.0, 0.0, hyphen), (20.0, 0.0, None)]);
    assert_eq!(words(Some(0.0)), [6, 8], "the hyphen won though its penalty outweighs the looser line");
    assert_eq!(words(None), [7, 8], "the same break without a hyphen should win, as it fits exactly");
}

// The second line chooses between a hyphen that fits exactly (100 plus 2,500) and a space at a ratio of
// 0.84 (about 4,800). It takes the hyphen unless the line before it ended at one too, which adds 10,000.
#[test]
fn two_hyphens_in_a_row_cost_more() {
    const NARROW: (f64, f64, Option<f64>) = (14.3, 2.0, None);
    let second = |first| {
        breaks(&[
            (100.0, 0.0, first),
            NARROW, NARROW, NARROW, NARROW, NARROW, NARROW,
            (2.2, 0.0, Some(0.0)),
            (20.0, 0.0, None),
        ])
    };
    assert_eq!(second(None), [1, 8, 9], "after a plain break the exact hyphen should win");
    assert_eq!(second(Some(0.0)), [1, 7, 9], "after a hyphen a second one should lose");
}

// Two graphemes of a cut word, each wider than the measure, take a line each. As one line with the text before
// them they'd overflow once, not twice, but a line that could still be broken never overflows.
#[test]
fn an_overfull_line_holds_one_segment() {
    let at = |at, ink, cut, forced| Breakpoint { at, ink, cut, forced, ..Breakpoint::start() };
    let bps = [
        Breakpoint::start(),
        at(1, 10.0, Cut::Anywhere, false),
        at(2, 10.0, Cut::Opportunity, false),
        at(3, 150.0, Cut::Anywhere, false),
        at(4, 150.0, Cut::Anywhere, false),
        at(5, 10.0, Cut::Opportunity, true),
    ];
    let fit = Fit { target: 100.0, line_end_stretch: 25.0 };
    let lines = break_lines(&bps, &fit, BreakStrategy::Optimal, |_, _| unreachable!("no tabs"));
    assert_eq!(lines.iter().map(|l| bps[l.to].at).collect::<Vec<_>>(), [2, 3, 4, 5]);
}

// The two words fill 101, which shrinking the space by a third of its 9 would bring to 98, but the last line
// of a paragraph is never justified, so it can't shrink.
#[test]
fn a_paragraph_s_last_line_never_shrinks() {
    assert_eq!(breaks(&[(50.0, 9.0, None), (42.0, 0.0, None)]), [1, 2]);
}

// Knuth and Plass's search with every open break kept: the oracle for the search that drops the dominated ones.
fn every_break_kept(bps: &[Breakpoint], fit: &Fit) -> Vec<(usize, usize)> {
    let m = Metrics::new(bps, |_, _| unreachable!("no tabs"));
    let mut nodes = alloc::vec![Node { position: 0, fitness: 1, demerits: 0.0, previous: usize::MAX }];
    let (mut active, mut last_opportunity) = (alloc::vec![0], 0);
    for (j, bp) in bps.iter().enumerate().skip(1) {
        let floor = if bp.cut == Cut::Anywhere { last_opportunity } else { 0 };
        if bp.cut == Cut::Opportunity {
            last_opportunity = j;
        }
        let (mut best, mut kept): ([Option<(f64, usize)>; 4], Vec<usize>) = ([None; 4], Vec::new());
        for &a in &active {
            let node = nodes[a];
            if node.position < floor {
                kept.push(a);
                continue;
            }
            let cost = line_cost(m.extent(node.position, j), m.end_stretch(j, fit), fit);
            if !cost.open && j > node.position + 1 {
                continue;
            }
            let mut total = node.demerits + cost.demerits_after(node.fitness);
            if bp.hyphen {
                total += HYPHEN_PENALTY * HYPHEN_PENALTY;
                if bps[node.position].hyphen {
                    total += DOUBLE_HYPHEN_DEMERITS;
                }
            }
            let slot = &mut best[cost.class as usize];
            if slot.is_none_or(|(bd, _)| total < bd) {
                *slot = Some((total, a));
            }
            if !bp.forced && cost.open {
                kept.push(a);
            }
        }
        for (class, entry) in best.iter().enumerate() {
            let Some((demerits, previous)) = *entry else { continue };
            nodes.push(Node { position: j, fitness: class as u8, demerits, previous });
            kept.push(nodes.len() - 1);
        }
        active = kept;
    }
    let end = bps.len() - 1;
    let last = (0..nodes.len()).filter(|&i| nodes[i].position == end);
    let Some(mut at) = last.min_by(|&a, &b| nodes[a].demerits.total_cmp(&nodes[b].demerits)) else { return Vec::new() };
    let mut lines = Vec::new();
    while nodes[at].previous != usize::MAX {
        lines.push((nodes[nodes[at].previous].position, nodes[at].position));
        at = nodes[at].previous;
    }
    lines.reverse();
    lines
}

// A paragraph of `n` random segments from `seed`: words, hanging spaces, blank segments `blank` in 100, hyphens with
// gains, a cut `cuts` in 100, gains at some starts, and a forced break about every 2,000.
fn paragraph(seed: u64, n: usize, blank: u64, cuts: u64) -> Vec<Breakpoint> {
    let mut state = seed;
    let mut below = |n: u64| {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % n
    };
    let mut bps = alloc::vec![Breakpoint::start()];
    for i in 0..n {
        let is_blank = below(100) < blank;
        let ink = if is_blank { 0.0 } else { below(300) as f64 / 10.0 };
        let space = if is_blank || below(4) == 0 { 0.0 } else { below(60) as f64 / 10.0 };
        let hyphen = !is_blank && below(15) == 0;
        let cut = if below(100) < cuts { Cut::Anywhere } else { Cut::Opportunity };
        let reshaped = |gain| Reshaped { gain, range: (i, i + 1), shaped: (i, i + 1) };
        let tail = hyphen.then(|| reshaped(below(50) as f64 / 10.0));
        let lead = (below(20) == 0).then(|| reshaped(below(20) as f64 / 10.0 - 1.0));
        let forced = i + 1 == n || below(2000) == 0;
        let (stretch, shrink) = (space / 2.0, space / 3.0);
        let blank = is_blank;
        bps.push(Breakpoint {
            at: i + 1, ink, space, stretch, shrink, forced, hyphen, blank, cut, tail, lead, ..Breakpoint::start()
        });
    }
    bps
}

fn optimal(bps: &[Breakpoint], fit: &Fit) -> Vec<(usize, usize)> {
    let lines = break_lines(bps, fit, BreakStrategy::Optimal, |_, _| unreachable!("no tabs"));
    lines.iter().map(|l| (l.from, l.to)).collect()
}

// Lines of 400 segments hold over 1,000 open breaks, and the cheapest 1,000 alone miss the paragraph's best breaks;
// dropping only breaks another always beats finds them, as do paragraphs with blank runs, cuts and gains.
#[test]
fn only_dominated_breaks_are_dropped() {
    let wide = paragraph(4, 3_000, 5, 0);
    let fit = Fit { target: 6_000.0, line_end_stretch: 1_500.0 };
    assert_eq!(optimal(&wide, &fit), every_break_kept(&wide, &fit));
    for seed in 0..200 {
        let bps = paragraph(seed, 150, 60, 20);
        let fits = [(80.0, 0.0), (80.0, 20.0), (f64::INFINITY, f64::INFINITY), (f64::INFINITY, 0.0)];
        for (target, line_end_stretch) in fits {
            let fit = Fit { target, line_end_stretch };
            assert_eq!(optimal(&bps, &fit), every_break_kept(&bps, &fit), "seed {seed} at {target}");
        }
    }
}

// 200,000 blank segments keep a few open breaks, not one each, and so does a paragraph of 200,000 words at an
// infinite measure, whose every line falls short alike: each breaks as one line.
#[test]
fn equal_starts_keep_one_open_break() {
    let blanks = paragraph(1, 200_000, 100, 0);
    let fit = Fit { target: 100.0, line_end_stretch: 25.0 };
    assert_eq!(optimal(&blanks, &fit).len(), blanks.iter().filter(|b| b.forced).count());
    let words = paragraph(1, 200_000, 0, 0);
    let fit = Fit { target: f64::INFINITY, line_end_stretch: f64::INFINITY };
    assert_eq!(optimal(&words, &fit).len(), words.iter().filter(|b| b.forced).count());
}
