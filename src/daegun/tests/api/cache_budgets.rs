use daegun::Font;

fn font() -> Font {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_vec(bytes).expect("parses")
}

fn fill_shapes(f: &Font, n: usize) {
    for i in 0..n {
        std::hint::black_box(f.shape(&format!("a budget worth measuring {i}"), &[], false));
    }
}

#[test]
fn the_shape_budget_bounds_the_shape_cache() {
    let f = font();
    f.set_shape_cache_bytes(64 * 1024);
    fill_shapes(&f, 4000);
    let (_, bytes) = f.shape_cache_stats();
    assert!(bytes <= 64 * 1024, "shape cache held {bytes} B against a 64 KB budget");
}

// Only `prewarm` writes to the outline cache, so only `prewarm` can fill it past the budget.
#[test]
fn the_outline_budget_bounds_the_outline_cache() {
    let f = font();
    f.prewarm(0..f.num_glyphs().min(600), &[]);
    let (_, filled) = f.outline_cache_stats();
    assert!(filled > 64 * 1024, "prewarming 600 glyphs held only {filled} B, so the budget is untested");
    f.set_outline_cache_bytes(32 * 1024);
    let (_, bytes) = f.outline_cache_stats();
    assert!(bytes <= 32 * 1024, "outline cache held {bytes} B against a 32 KB budget");
    f.prewarm(0..f.num_glyphs().min(600), &[]);
    let (_, again) = f.outline_cache_stats();
    assert!(again <= 32 * 1024, "prewarming past the budget held {again} B");
}

// Unbudgeted, the same 40 weights hold about 16 MB, so the 2 MB bound has to evict to hold.
#[test]
fn the_instance_budget_bounds_instanced_fonts() {
    let f = font();
    let fill = |f: &Font| {
        for w in 0..40 {
            let axes: &[(&str, f64)] = &[("wght", 200.0 + f64::from(w) * 10.0)];
            std::hint::black_box(f.shape("weight", axes, false));
            std::hint::black_box(f.outline_glyph_instanced(40, axes, &mut daegun::Path::default()));
        }
        let (locations, tables) = f.instance_cache_stats();
        locations + tables
    };
    let unbounded = fill(&f);
    assert!(unbounded > 4 * 1024 * 1024, "40 weights held only {unbounded} B, so the budget is untested");
    f.set_instance_cache_bytes(2 * 1024 * 1024);
    let held = fill(&f);
    assert!(held <= 2 * 1024 * 1024, "instance caches held {held} B against a 2 MB budget");
}

// The allowance is spent down rather than capped, so what matters is that setting it grants more.
#[test]
fn the_cmap_allowance_is_readable_and_settable() {
    let f = font();
    f.set_cmap_index_allowance(1234);
    assert_eq!(f.cmap_index_allowance(), 1234);
}

// A budget of zero has to turn caching off without changing a single outline or a shaped run.
#[test]
fn caching_nothing_changes_the_output() {
    let f = font();
    let gid = f.glyph_id('g' as u32).expect("font has g");
    let draw = |f: &Font| {
        let mut p = daegun::Path::default();
        f.prepared_outline(gid, 28.0, &[("wght", 650.0)], &daegun::OutlineOptions::default(), &mut p);
        let run = f.shape("budget", &[("wght", 650.0)], false).expect("shapes").glyphs.clone();
        (p, run, f.instance(&[("wght", 650.0)]))
    };
    f.prewarm([gid], &[("wght", 650.0)]);
    let warm = draw(&f);
    f.set_outline_cache_bytes(0);
    f.set_shape_cache_bytes(0);
    f.set_instance_cache_bytes(0);
    let cold = draw(&f);
    assert_eq!(f.outline_cache_stats().0, 0, "a zero outline budget still cached something");
    assert_eq!(f.shape_cache_stats(), (0, 0), "a zero shape budget still cached a run");
    assert_eq!(warm, cold, "output changed when caching was turned off");
}

// The newest instance is kept whatever the budget, or every call at one fixed weight would instance the
// font again. At a budget of 0 it is the only one kept.
#[test]
fn a_zero_instance_budget_keeps_the_newest_instance() {
    let draw = |f: &Font, w: f64| {
        f.outline_glyph_instanced(40, &[("wght", w)], &mut daegun::Path::default());
        std::hint::black_box(f.instance(&[("wght", w)]));
    };
    let only = font();
    only.set_instance_cache_bytes(0);
    draw(&only, 800.0);
    let (fonts, tables) = only.instance_cache_stats();
    assert!(fonts > 0 && tables > 0, "a zero budget kept nothing, so each call instances again");
    let both = font();
    both.set_instance_cache_bytes(0);
    draw(&both, 700.0);
    draw(&both, 800.0);
    assert_eq!(both.instance_cache_stats(), (fonts, tables), "a zero budget kept more than the newest");
}

// What prewarm reports added is what the cache took, so a budget of zero takes nothing.
#[test]
fn prewarm_counts_only_what_the_cache_took() {
    let f = font();
    f.set_outline_cache_bytes(0);
    assert_eq!(f.prewarm(1..50, &[]), 0, "prewarm reported outlines a zero budget refused");
    assert_eq!(f.outline_cache_stats().0, 0, "a zero budget held an outline");
}
