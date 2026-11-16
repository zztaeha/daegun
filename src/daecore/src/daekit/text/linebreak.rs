use alloc::vec::Vec;

// Where a line may end: at a break opportunity, or anywhere inside a segment no line can hold, as CSS's
// `overflow-wrap: anywhere` allows when no opportunity lies in the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    Opportunity,
    Anywhere,
}

// Glyphs at one end of a line shaped apart from the text past its break: those standing for `range`, out of
// `shaped`, which reaches out to the next break opportunity, and what they change the line's width by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reshaped {
    pub gain: f64,
    pub range: (usize, usize),
    pub shaped: (usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Breakpoint {
    pub at: usize,
    pub ink: f64,
    pub space: f64,
    pub stretch: f64,
    pub shrink: f64,
    pub forced: bool,
    // The text before it holds a tab, whose width depends on where its line starts.
    pub tab: bool,
    // A break at a soft hyphen, which a line ending there shows as a hyphen.
    pub hyphen: bool,
    // Its text has no advance and no tab, so a line starting either side of it is the same line.
    pub blank: bool,
    pub cut: Cut,
    // A line starting here, apart from the text before it, and one ending here, apart from the text after it
    // or with its hyphen.
    pub lead: Option<Reshaped>,
    pub tail: Option<Reshaped>,
    // What a line of only the segment ending here gains instead, as `lead` and `tail` may overlap in it.
    pub alone: Option<f64>,
}

impl Breakpoint {
    // What a line from here to `last`, one segment on if `alone`, gains by its ends shaped apart.
    pub fn gain_to(&self, last: &Breakpoint, alone: bool) -> f64 {
        match last.alone {
            Some(gain) if alone => gain,
            _ => self.lead.map_or(0.0, |r| r.gain) + last.tail.map_or(0.0, |r| r.gain),
        }
    }

    pub fn start() -> Breakpoint {
        Breakpoint {
            at: 0,
            ink: 0.0,
            space: 0.0,
            stretch: 0.0,
            shrink: 0.0,
            forced: false,
            tab: false,
            hyphen: false,
            blank: false,
            cut: Cut::Opportunity,
            lead: None,
            tail: None,
            alone: None,
        }
    }
}

// A line's natural width, and how far its spaces may stretch and shrink.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Extent {
    pub width: f64,
    pub stretch: f64,
    pub shrink: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fit {
    pub target: f64,
    pub line_end_stretch: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub from: usize,
    pub to: usize,
}

/// How lines are chosen.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BreakStrategy {
    /// Each line as full as it goes before the next starts.
    #[default]
    Greedy,
    /// The paragraph's breaks chosen together for even lines, Knuth and Plass's total fit.
    Optimal,
}

struct Metrics<'a, T> {
    bps: &'a [Breakpoint],
    cum: Vec<Cumulative>,
    tabbed: T,
}

#[derive(Clone, Copy, Default)]
struct Cumulative {
    ink: f64,
    space: f64,
    stretch: f64,
    shrink: f64,
    tabs: usize,
}

impl<'a, T: Fn(usize, usize) -> Extent> Metrics<'a, T> {
    fn new(bps: &'a [Breakpoint], tabbed: T) -> Self {
        let mut cum = Vec::with_capacity(bps.len() + 1);
        let mut run = Cumulative::default();
        cum.push(run);
        for bp in bps {
            run.ink += bp.ink;
            run.space += bp.space;
            run.stretch += bp.stretch;
            run.shrink += bp.shrink;
            run.tabs += usize::from(bp.tab);
            cum.push(run);
        }
        Metrics { bps, cum, tabbed }
    }

    // The line between two breaks: its ink and every space but the hanging last, with what its ends gain shaped apart.
    // Tab stops count from the line's start, so a tabbed line asks `tabbed`; a last line's spaces never shrink.
    #[inline]
    fn extent(&self, from: usize, to: usize) -> Extent {
        debug_assert!(to > from, "a line ends after the break it starts from");
        let (head, tail) = (&self.cum[from + 1], &self.cum[to]);
        let (first, last) = (&self.bps[from], &self.bps[to]);
        let ends = first.gain_to(last, to == from + 1);
        let line = if self.cum[to + 1].tabs != head.tabs {
            let line = self.tabbed_extent(from, to);
            Extent { width: line.width + ends, ..line }
        } else {
            Extent {
                width: (self.cum[to + 1].ink - head.ink) + (tail.space - head.space) + ends,
                stretch: tail.stretch - head.stretch,
                shrink: tail.shrink - head.shrink,
            }
        };
        if last.forced { Extent { shrink: 0.0, ..line } } else { line }
    }

    // Out of line, so the common case above stays small enough to inline into the search.
    #[inline(never)]
    fn tabbed_extent(&self, from: usize, to: usize) -> Extent {
        (self.tabbed)(from, to)
    }

    // A forced break ends a paragraph, whose last line may fall short at no cost.
    fn end_stretch(&self, to: usize, fit: &Fit) -> f64 {
        if self.bps[to].forced { f64::INFINITY } else { fit.line_end_stretch }
    }

    // Each break as a start, keyed so that every later line costs the same from starts of one key: blank segments
    // apart, with one gain and hyphen, and no opportunity between that a later cut's floor could fall on. At an
    // infinite measure every line falls short alike, so only the hyphen and the floors tell starts apart.
    fn start_keys(&self, fit: &Fit) -> Vec<(usize, u64, bool)> {
        let finite = |x: f64| x.is_finite();
        let gains = self.bps.iter().flat_map(|b| [b.lead.map(|r| r.gain), b.tail.map(|r| r.gain), b.alone]);
        let infinite = fit.target == f64::INFINITY
            && self.cum.iter().all(|c| [c.ink, c.space, c.stretch, c.shrink].into_iter().all(finite))
            && gains.flatten().all(finite);
        // An opportunity a cut follows before the next one is a floor that cut's lines start from.
        let mut floors = alloc::vec![false; self.bps.len()];
        let mut cut_ahead = false;
        for (k, bp) in self.bps.iter().enumerate().rev() {
            match bp.cut {
                Cut::Anywhere => cut_ahead = true,
                Cut::Opportunity => (floors[k], cut_ahead) = (cut_ahead, false),
            }
        }
        let mut part = 0;
        let keys = self.bps.iter().zip(floors).enumerate().map(|(k, (bp, floor))| {
            part += usize::from(k > 0 && (floor || !(infinite || bp.blank)));
            let gain = if infinite { 0 } else { bp.lead.map_or(0.0, |r| r.gain).to_bits() };
            (part, gain, bp.hyphen)
        });
        keys.collect()
    }
}

fn ratio(line: Extent, end_stretch: f64, fit: &Fit) -> f64 {
    let slack = fit.target - line.width;
    let (stretch, shrink) = (line.stretch + end_stretch, line.shrink);
    if slack > 0.0 {
        // Infinite stretch says falling short costs nothing, and makes `slack` infinite, so the ratio is NaN, which
        // loses every comparison: no node stays active and optimal breaking silently gives one word per line.
        if stretch.is_infinite() { 0.0 }
        else if stretch <= 0.0 { f64::INFINITY }
        else { slack / stretch }
    } else if slack < 0.0 {
        if shrink.is_infinite() { 0.0 }
        else if shrink <= 0.0 { f64::NEG_INFINITY }
        else { slack / shrink }
    } else {
        0.0
    }
}

// A line at the limit of its stretch or shrink scores 100, cubed so slight stretching costs next to nothing and
// spacing past the limit soon outweighs the rest.
fn badness(ratio: f64) -> f64 {
    let a = ratio.abs();
    100.0 * a * a * a
}

// Each line costs (10 + badness)², so even evenly spaced lines cost 100 apiece and a paragraph never adds a line
// just to even its spacing.
const LINE_PENALTY: f64 = 10.0;
// A line two spacing classes from the one before, tight next to loose, costs what one of badness 90 does,
// (10 + 90)², so neighboring lines look alike.
const FITNESS_DEMERITS: f64 = 10_000.0;
// Past 10 times its stretch a line scores as at 10, its cost kept finite; the surcharge keeps each such line dearer
// than any within that.
const MAX_RATIO: f64 = 10.0;
const UNDERFULL_DEMERITS: f64 = 1.0e9;
// An overfull line, whose text spills past the measure, costs more than any underfull one, about 1.1e10 at most.
const OVERFULL_DEMERITS: f64 = 1.0e12;

// A hyphen costs 50², what a line of badness 40 costs, (10 + 40)², and a second in a row 10,000 more, a line of
// badness 90's cost: a paragraph hyphenates only where its spacing gains more than that.
const HYPHEN_PENALTY: f64 = 50.0;
const DOUBLE_HYPHEN_DEMERITS: f64 = 10_000.0;

// Spacing classes by how much of its room a line uses: over half its shrink, within half either way, up to all its
// stretch, and past it.
fn fitness_class(ratio: f64) -> u8 {
    if ratio < -0.5 {
        0
    } else if ratio <= 0.5 {
        1
    } else if ratio <= 1.0 {
        2
    } else {
        3
    }
}

#[derive(Clone, Copy)]
struct LineCost {
    plain: f64,
    jumped: f64,
    class: u8,
    open: bool,
}

fn line_cost(line: Extent, end_stretch: f64, fit: &Fit) -> LineCost {
    let ratio = ratio(line, end_stretch, fit);
    let (effective, surcharge) = if ratio < -1.0 {
        (-1.0, OVERFULL_DEMERITS)
    } else if ratio > MAX_RATIO {
        (MAX_RATIO, UNDERFULL_DEMERITS)
    } else {
        (ratio, 0.0)
    };
    let base = line_demerits(effective);
    LineCost {
        plain: base + surcharge,
        jumped: (base + FITNESS_DEMERITS) + surcharge,
        class: fitness_class(effective),
        open: ratio >= -1.0,
    }
}

impl LineCost {
    fn demerits_after(&self, prev_class: u8) -> f64 {
        if (self.class as i16 - prev_class as i16).abs() > 1 { self.jumped } else { self.plain }
    }
}

fn line_demerits(ratio: f64) -> f64 {
    let base = LINE_PENALTY + badness(ratio);
    base * base
}

fn break_greedy(bps: &[Breakpoint], fit: &Fit, tabbed: impl Fn(usize, usize) -> Extent) -> Vec<Line> {
    let mut lines = Vec::new();
    if bps.len() < 2 {
        return lines;
    }

    let m = Metrics::new(bps, tabbed);
    let mut from = 0;
    let (mut last_fit, mut last_cut) = (None, None);
    let mut j = 1;
    while j < bps.len() {
        if m.extent(from, j).width <= fit.target {
            if bps[j].forced {
                lines.push(Line { from, to: j });
                from = j;
                (last_fit, last_cut) = (None, None);
            } else if bps[j].cut == Cut::Opportunity {
                last_fit = Some(j);
            } else {
                last_cut = Some(j);
            }
            j += 1;
            continue;
        }
        // A forced break is not exempt from falling back to the last opportunity that fit: the text before it must
        // still fit, or a paragraph ending in a long sentence becomes one enormous line.
        let brk = last_fit.or(last_cut).unwrap_or(j);
        lines.push(Line { from, to: brk });
        from = brk;
        (last_fit, last_cut) = (None, None);
        j = brk + 1;
    }
    if from + 1 < bps.len() {
        lines.push(Line { from, to: bps.len() - 1 });
    }
    lines
}

#[derive(Clone, Copy)]
struct Node {
    position: usize,
    fitness: u8,
    demerits: f64,
    previous: usize,
}

// The least demerits kept by fitness at each start key of one part, NaN for none.
type Least = Vec<(u64, bool, [f64; 4])>;

// Drops each open break before `j` that no later line can end more cheaply from than one kept before it: at a start
// of its key, no costlier, of its fitness or one jumping at fewer classes (1 for 0, 2 for 3), so it wins ties too.
fn drop_dominated(active: &mut Vec<usize>, nodes: &[Node], j: usize, keys: &[(usize, u64, bool)], least: &mut Least) {
    let mut part = usize::MAX;
    active.retain(|&a| {
        let node = &nodes[a];
        if node.position == j {
            return true;
        }
        let (at, gain, hyphen) = keys[node.position];
        if at != part {
            least.clear();
            part = at;
        }
        let k = match least.iter().position(|l| (l.0, l.1) == (gain, hyphen)) {
            Some(k) => k,
            None => {
                least.push((gain, hyphen, [f64::NAN; 4]));
                least.len() - 1
            }
        };
        let (best, f) = (&mut least[k].2, usize::from(node.fitness));
        let rival = match f {
            0 => best[0].min(best[1]),
            3 => best[3].min(best[2]),
            _ => best[f],
        };
        let dominated = rival <= node.demerits;
        if !dominated {
            best[f] = node.demerits;
        }
        !dominated
    });
}

fn break_optimal(bps: &[Breakpoint], fit: &Fit, tabbed: impl Fn(usize, usize) -> Extent) -> Vec<Line> {
    if bps.len() < 2 {
        return Vec::new();
    }

    let mut nodes: Vec<Node> = alloc::vec![Node {
        position: 0,
        fitness: 1,
        demerits: 0.0,
        previous: usize::MAX,
    }];
    let mut active: Vec<usize> = alloc::vec![0];
    let mut still_active: Vec<usize> = Vec::new();
    let m = Metrics::new(bps, tabbed);
    let (keys, mut least) = (m.start_keys(fit), Least::new());
    // Checked once the open breaks double, so a paragraph whose breaks are all of use pays next to nothing.
    let mut check_at = 64;
    let mut last_opportunity = 0;

    for (j, bp) in bps.iter().enumerate().skip(1) {
        let (forced, end_stretch) = (bp.forced, m.end_stretch(j, fit));
        // A line ending at a cut starts at or after the last opportunity, so it holds none.
        let floor = if bp.cut == Cut::Anywhere { last_opportunity } else { 0 };
        if bp.cut == Cut::Opportunity {
            last_opportunity = j;
        }
        let mut best: [Option<(f64, usize)>; 4] = [None; 4];
        still_active.clear();
        let mut memo: Option<(usize, LineCost)> = None;

        for &a in &active {
            let node = nodes[a];
            if node.position < floor {
                still_active.push(a);
                continue;
            }
            let cost = match memo {
                Some((p, c)) if p == node.position => c,
                _ => {
                    let c = line_cost(m.extent(node.position, j), end_stretch, fit);
                    memo = Some((node.position, c));
                    c
                }
            };

            // A line that could still be broken never overflows, or one overflowing anyway would take in more.
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

            if !forced && cost.open {
                still_active.push(a);
            }
        }

        for (class, entry) in best.iter().enumerate() {
            let Some((d, prev)) = *entry else { continue };
            nodes.push(Node {
                position: j,
                fitness: class as u8,
                demerits: d,
                previous: prev,
            });
            still_active.push(nodes.len() - 1);
        }

        if still_active.len() >= check_at {
            drop_dominated(&mut still_active, &nodes, j, &keys, &mut least);
            check_at = 2 * still_active.len().max(32);
        }

        core::mem::swap(&mut active, &mut still_active);
    }

    let end = bps.len() - 1;
    let best = (0..nodes.len())
        .filter(|&i| nodes[i].position == end)
        .min_by(|&a, &b| {
            nodes[a].demerits.partial_cmp(&nodes[b].demerits).unwrap_or(core::cmp::Ordering::Equal)
        });
    let Some(best) = best else { return Vec::new() };

    let mut out = Vec::new();
    let mut cur = best;
    while nodes[cur].previous != usize::MAX {
        out.push(Line { from: nodes[nodes[cur].previous].position, to: nodes[cur].position });
        cur = nodes[cur].previous;
    }
    out.reverse();
    out
}

// `tabbed` gives the extent of a line between two breaks that holds a tab.
pub(crate) fn break_lines(
    bps: &[Breakpoint],
    fit: &Fit,
    strategy: BreakStrategy,
    tabbed: impl Fn(usize, usize) -> Extent,
) -> Vec<Line> {
    match strategy {
        BreakStrategy::Greedy => break_greedy(bps, fit, tabbed),
        BreakStrategy::Optimal => break_optimal(bps, fit, tabbed),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/text/linebreak.rs"]
mod tests;
