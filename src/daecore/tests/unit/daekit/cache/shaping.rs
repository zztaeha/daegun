use super::{AxisKey, FontCache, KEPT_PLANS, PlanRef};
use crate::daecore::daekit::cache::RunContext;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::daecore::daeshaper::{face::Face, ot::tag::Tag, plan::FeatureSetting, run::Direction};

fn fixture(path: &str) -> FontCache {
    let bytes = std::fs::read(alloc::format!("{}/assets/test-fonts/{path}", env!("CARGO_MANIFEST_DIR")));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes.expect("a fixture"));
    FontCache::new(map.expect("parses")).expect("loads")
}

// Where a run of `text` alone, at the default, sits in a thread's front for `cache`.
#[cfg(feature = "threading")]
fn slot(cache: &FontCache, text: &str) -> usize {
    let head = (&AxisKey::default(), text.as_bytes(), false, None, "", "", None, None);
    super::front_slot(cache.runs_id, &head)
}

// `n` words whose runs cost the cache alike, under `threading` each on a slot of its own in the front, as a cache's
// number moves its slots and tests run in any order.
fn words(cache: &FontCache, n: usize) -> Vec<&'static str> {
    const ALL: [&str; 16] = [
        "mmmm", "nnnn", "aaaa", "hhhh", "oooo", "uuuu", "eeee", "ssss", "cccc", "rrrr", "vvvv", "xxxx", "zzzz", "wwww",
        "kkkk", "dddd",
    ];
    #[cfg(feature = "threading")]
    let mut seen = alloc::collections::BTreeSet::new();
    #[cfg(feature = "threading")]
    let all = ALL.into_iter().filter(|w| seen.insert(slot(cache, w)));
    #[cfg(not(feature = "threading"))]
    let all = { let _ = cache; ALL.into_iter() };
    let words: Vec<_> = all.take(n).collect();
    assert_eq!(words.len(), n, "too few words on slots of their own");
    words
}

fn kept(plan: &PlanRef<'_>) -> Option<*const crate::daecore::daeshaper::plan::Plan> {
    match plan {
        PlanRef::Kept(plan) => Some(*plan),
        PlanRef::Memo(_) => None,
    }
}

// The first plans asked for are kept and handed out as the same plan each time; past them, and for a morx font's
// ranged features, which build their ranges into the plan, a plan goes to the memo.
#[test]
fn plans_are_kept_up_to_the_slots_then_memoized() {
    let plans = |path: &str, start: u32| {
        let cache = fixture(path);
        let face = Face::new(&cache);
        let plan = |value: u32| {
            let features = [FeatureSetting { tag: Tag::new(b"ss01"), value, start, end: u32::MAX }];
            kept(&cache.shape_plan_cached(None, &face, Direction::LeftToRight, None, None, &features, &[]))
        };
        let first: alloc::vec::Vec<_> = (0..=KEPT_PLANS as u32).map(plan).collect();
        let again: alloc::vec::Vec<_> = (0..=KEPT_PLANS as u32).map(plan).collect();
        (first, again)
    };
    let (first, again) = plans("inter/InterVariable.ttf", 0);
    assert!(first[..KEPT_PLANS].iter().all(Option::is_some) && first[KEPT_PLANS].is_none());
    assert_eq!(first, again, "a kept plan was built again");
    // A ranged feature is keyed on being ranged alone, so OpenType keeps the plan.
    let (ranged, _) = plans("inter/InterVariable.ttf", 3);
    assert!(ranged[..KEPT_PLANS].iter().all(Option::is_some));
    let (morx, _) = plans("aat/TestMORXOne.ttf", 3);
    assert!(morx.iter().all(Option::is_none), "a morx plan for one call's ranges was kept");
}

// Past the kept plans, plans are kept by bytes under the shape budget's part for them, least recently used out first:
// the first memoized stays through a hundred more read in between, counted with the runs, and one of 0 keeps none.
#[test]
fn plans_past_the_slots_are_kept_by_bytes() {
    let cache = fixture("inter/InterVariable.ttf");
    let face = Face::new(&cache);
    let plan = |value: u32| {
        let features = [FeatureSetting { tag: Tag::new(b"ss01"), value, start: 0, end: u32::MAX }];
        match cache.shape_plan_cached(None, &face, Direction::LeftToRight, None, None, &features, &[]) {
            PlanRef::Memo(plan) => Some(plan),
            PlanRef::Kept(_) => None,
        }
    };
    (0..KEPT_PLANS as u32).for_each(|v| assert!(plan(v).is_none()));
    let first = plan(KEPT_PLANS as u32).expect("a memoized plan");
    for v in 1..=100 {
        plan(KEPT_PLANS as u32 + v);
        let again = plan(KEPT_PLANS as u32).expect("memoized");
        assert!(crate::daecore::daekit::base::sync::Shared::ptr_eq(&first, &again), "built again after {v} more");
    }
    let (plans, bytes) = {
        let plans = crate::daecore::daekit::base::sync::read(cache.plans());
        (plans.len(), plans.bytes())
    };
    assert!(plans > 64 && cache.shape_cache_stats() == (plans, bytes), "{plans} plans, {bytes} B not counted");
    cache.set_shape_cache_bytes(0);
    assert_eq!(cache.shape_cache_stats(), (0, 0));
}

// A plan is keyed on the language's canonical form, so spellings of one tag share a plan and a longer tag that
// starts with another's doesn't.
#[test]
fn a_language_keys_its_plan_by_its_canonical_form() {
    let cache = fixture("inter/InterVariable.ttf");
    let face = Face::new(&cache);
    let ltr = Direction::LeftToRight;
    let plan = |language| kept(&cache.shape_plan_cached(None, &face, ltr, None, language, &[], &[]));
    assert_eq!(plan(Some("en-US")), plan(Some("en_us\0x")));
    assert_ne!(plan(Some("en")), plan(Some("en-us")));
    assert_ne!(plan(Some("en")), plan(None));
    assert_eq!(plan(Some("")), plan(None), "an empty tag is no language");
    let latin = Some(crate::daecore::daeshaper::unicode::script('a'));
    let given = [Tag::new(b"cyrl")];
    let with = |tags| kept(&cache.shape_plan_cached(latin, &face, ltr, tags, None, &[], &[]));
    assert_ne!(with(None), with(Some(&given)), "given script tags shared the script's own plan");
}

// The cache keeps owned keys and is asked with borrowed ones: each run, its text and context split as no other's, is
// found again as the run it kept, so the two order alike.
#[test]
fn a_run_is_found_again_by_its_borrowed_key() {
    let cache = fixture("inter/InterVariable.ttf");
    let (plain, bold) = (AxisKey::default(), cache.axis_key(&[("wght", 700.0)]));
    let ctx = |before, after| RunContext { before, after, ..RunContext::default() };
    let runs = [
        (&plain, "ab", ctx("c", "")),
        (&plain, "a", ctx("bc", "")),
        (&plain, "a", ctx("b", "c")),
        (&plain, "a", ctx("", "bc")),
        (&plain, "a", RunContext { language: Some(""), ..RunContext::default() }),
        (&plain, "a", RunContext { rtl: Some(false), ..RunContext::default() }),
        (&plain, "a", RunContext { rtl: Some(true), ..RunContext::default() }),
        (&plain, "a", RunContext::default()),
        (&bold, "a", RunContext::default()),
    ];
    let shape = || runs.iter().map(|(axes, text, ctx)| cache.shaped_run_in_context(axes, text, false, ctx));
    let (first, again): (Vec<_>, Vec<_>) = (shape().collect(), shape().collect());
    assert_eq!(cache.shape_cache_stats().0, runs.len());
    assert!(first.iter().zip(&again).all(|(a, b)| Arc::ptr_eq(a, b)), "a run was shaped again");
}

// Of three runs held to the byte, the next drops the second, the first being read again: under `threading` from the
// thread's front, marked in the cache at its next turn at the lock, kept though another font's run hits the slot.
#[test]
fn a_run_read_again_is_dropped_last() {
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let words = words(&cache, 4);
    let shape = |w: &str| cache.shaped_run(&axes, w, false);
    let first: Vec<_> = words[..3].iter().map(|w| shape(w)).collect();
    cache.set_shape_cache_bytes(cache.shape_cache_stats().1);
    shape(words[0]);
    #[cfg(feature = "threading")]
    {
        let other = fixture("inter/InterVariable.ttf");
        let at = slot(&cache, words[0]);
        let landing = (0..).map(|i| alloc::format!("{i}")).find(|w| slot(&other, w) == at).expect("a word");
        other.shaped_run(&axes, &landing, false);
    }
    shape(words[3]);
    assert_eq!(cache.shape_cache_stats().0, 3);
    assert!(Arc::ptr_eq(&shape(words[0]), &first[0]), "the run read again was dropped");
    assert!(!Arc::ptr_eq(&shape(words[1]), &first[1]), "the run not read again was kept");
}

// Narrowing the budget drops what a new run would: of three runs, the one read again stays, under `threading` its
// mark paid first and carried as the runs move to one list, and the one dropped isn't read again from a front.
#[test]
fn a_narrower_budget_drops_a_run_not_read_again() {
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let words = words(&cache, 3);
    let shape = |w: &str| cache.shaped_run(&axes, w, false);
    let first: Vec<_> = words.iter().map(|w| shape(w)).collect();
    shape(words[0]);
    cache.set_shape_cache_bytes(cache.shape_cache_stats().1 / 3 * 2);
    assert_eq!(cache.shape_cache_stats().0, 2);
    assert!(Arc::ptr_eq(&shape(words[0]), &first[0]), "the run read again was dropped");
    assert!(!Arc::ptr_eq(&shape(words[1]), &first[1]), "a dropped run was read again");
}

// Runs read again are stamped in the order read: of the two, the one read first goes first.
#[cfg(not(feature = "threading"))]
#[test]
fn runs_read_again_are_stamped_in_the_order_read() {
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let words = words(&cache, 5);
    let shape = |w: &str| cache.shaped_run(&axes, w, false);
    let first: Vec<_> = words[..3].iter().map(|w| shape(w)).collect();
    cache.set_shape_cache_bytes(cache.shape_cache_stats().1);
    shape(words[1]);
    shape(words[0]);
    shape(words[3]);
    shape(words[4]);
    assert!(Arc::ptr_eq(&shape(words[0]), &first[0]), "the run read last was dropped");
    assert!(!Arc::ptr_eq(&shape(words[1]), &first[1]), "the run read first was kept");
}

// Under `threading` runs read again since they came in each go round once more, so a newer run never read again
// goes before them, and the next drops the oldest of the two.
#[cfg(feature = "threading")]
#[test]
fn runs_read_again_outlive_a_newer_run_not_read_again() {
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let words = words(&cache, 5);
    let shape = |w: &str| cache.shaped_run(&axes, w, false);
    let first: Vec<_> = words[..3].iter().map(|w| shape(w)).collect();
    cache.set_shape_cache_bytes(cache.shape_cache_stats().1);
    shape(words[1]);
    shape(words[0]);
    let newer = shape(words[3]);
    shape(words[4]);
    assert_eq!(cache.shape_cache_stats().0, 3);
    assert!(Arc::ptr_eq(&shape(words[0]), &first[0]), "a run read again was dropped");
    assert!(Arc::ptr_eq(&shape(words[1]), &first[1]), "a run read again was dropped");
    assert!(!Arc::ptr_eq(&shape(words[3]), &newer), "the newer run not read again was kept");
}

// Fonts given one text land on slots of their own, so one owing its cache a stamp there doesn't keep another out.
#[cfg(feature = "threading")]
#[test]
fn fonts_given_one_text_land_apart() {
    let text = (&AxisKey::default(), "apart".as_bytes(), false, None, "", "", None, None);
    let slots: alloc::collections::BTreeSet<_> = (0..8).map(|cache| super::front_slot(cache, &text)).collect();
    assert!(slots.len() > 4, "the caches' runs of one text shared {} slots of 8", slots.len());
}

// Two caches of one font hold their runs apart: one's run isn't read from a front as the other's.
#[test]
fn caches_keep_their_runs_apart() {
    let (one, two) = (fixture("inter/InterVariable.ttf"), fixture("inter/InterVariable.ttf"));
    let axes = AxisKey::default();
    let run = one.shaped_run(&axes, "apart", false);
    assert!(!Arc::ptr_eq(&run, &two.shaped_run(&axes, "apart", false)), "one cache's run read through another");
    assert_eq!(two.shape_cache_stats().0, 1);
}

// Bytes find the run their text was shaped to, where a hit reads no UTF-8; bytes that aren't UTF-8 are shaped to none.
#[cfg(feature = "capi")]
#[test]
fn bytes_find_the_run_of_their_text() {
    let cache = fixture("inter/InterVariable.ttf");
    let (axes, ctx) = (AxisKey::default(), RunContext::default());
    let run = cache.shaped_run(&axes, "bytes", false);
    let found = cache.shaped_bytes_in_context(&axes, b"bytes", false, &ctx);
    assert!(found.is_some_and(|found| Arc::ptr_eq(&found, &run)), "the text's run wasn't found by its bytes");
    let missed = cache.shaped_bytes_in_context(&axes, b"bytes, missed", false, &ctx);
    assert!(missed.is_some_and(|missed| !missed.glyphs.is_empty()), "bytes the cache missed weren't shaped");
    assert!(cache.shaped_bytes_in_context(&axes, b"a\xff", false, &ctx).is_none(), "bytes that aren't UTF-8 shaped");
    assert_eq!(cache.shape_cache_stats().0, 2, "bytes that aren't UTF-8 were kept");
}

// A run read again comes from the cache, or under `threading` the thread's front, until the cache is cleared, which
// lets go of it in both.
#[test]
fn a_cleared_cache_shapes_its_runs_again() {
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let first = cache.shaped_run(&axes, "clear", false);
    assert!(Arc::ptr_eq(&first, &cache.shaped_run(&axes, "clear", false)));
    cache.clear_shape_cache();
    assert_eq!(Arc::strong_count(&first), 1, "a cleared run is still held");
    assert!(!Arc::ptr_eq(&first, &cache.shaped_run(&axes, "clear", false)), "a cleared run was read again");
    assert_eq!(cache.shape_cache_stats().0, 1);
}

// Threads each reading their own run through one cache, mostly from their fronts, get it back each time once it's
// kept, a busy cache keeping it on a later miss, and leave the cache holding each run once.
#[cfg(feature = "threading")]
#[test]
fn threads_read_their_runs_through_one_cache() {
    let (cache, axes) = (&fixture("inter/InterVariable.ttf"), &AxisKey::default());
    std::thread::scope(|s| {
        for word in ["one", "two", "three", "four"] {
            s.spawn(move || {
                let mut kept = cache.shaped_run(axes, word, false);
                loop {
                    let next = cache.shaped_run(axes, word, false);
                    if Arc::ptr_eq(&kept, &next) {
                        break;
                    }
                    kept = next;
                }
                assert!((0..100).all(|_| Arc::ptr_eq(&kept, &cache.shaped_run(axes, word, false))));
            });
        }
    });
    assert_eq!(cache.shape_cache_stats().0, 4);
}

// A shape past a cache another thread holds neither waits for it nor is kept, and a later one is: a thread holding
// the lock here stands for threads queueing, which took 17× as long a shape at 8 threads.
#[cfg(feature = "threading")]
#[test]
fn a_shape_goes_on_past_a_busy_cache() {
    let (cache, axes) = (&fixture("inter/InterVariable.ttf"), &AxisKey::default());
    let alone = cache.shaped_run(axes, "typeface", false);
    cache.clear_shape_cache();
    let head = (axes, "typeface".as_bytes(), false, None, "", "", None, None);
    let (list, _) = cache.run_list(&head).expect("a budget keeps runs");
    let held = crate::daecore::daekit::base::sync::write_shared(list);
    let (tell, told) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(move || tell.send(cache.shaped_run(axes, "typeface", false)));
        let past = told.recv_timeout(std::time::Duration::from_secs(20));
        drop(held);
        let past = past.expect("the shape waited for the cache");
        assert_eq!(past.glyphs, alone.glyphs);
    });
    assert_eq!(cache.shape_cache_stats().0, 0);
    let kept = cache.shaped_run(axes, "typeface", false);
    assert_eq!(cache.shape_cache_stats().0, 1);
    assert!(Arc::ptr_eq(&kept, &cache.shaped_run(axes, "typeface", false)));
}

// A budget that changes how many lists hold the runs moves them, in the order of their use, each read as before.
#[cfg(feature = "threading")]
#[test]
fn runs_keep_their_order_as_a_budget_changes_how_many_lists_hold_them() {
    use super::{read_shared, Run, RUN_SHARDS, SHAPE_CACHE_BYTES};
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let words: Vec<_> = (0..40).map(|i| alloc::format!("word{i}")).collect();
    let shape = |w: &str| cache.shaped_run(&axes, w, false);
    let first: Vec<_> = words.iter().map(|w| shape(w)).collect();
    let (runs, bytes) = cache.shape_cache_stats();
    assert_eq!(runs, words.len());
    let lists = cache.run_shards();
    assert!(lists.lists.iter().filter(|list| read_shared(list).bytes() != 0).count() > 1, "one list holds every run");
    cache.set_shape_cache_bytes(bytes);
    assert_eq!(lists.in_use.get(), 1);
    assert_eq!(read_shared(&lists.lists[0]).len(), words.len(), "a run was left in a list no longer read");
    cache.set_shape_cache_bytes(SHAPE_CACHE_BYTES);
    assert_eq!(lists.in_use.get(), RUN_SHARDS);
    assert_eq!(cache.shape_cache_stats(), (runs, bytes), "moving back changed what's held");
    assert!(words.iter().zip(&first).all(|(w, run)| Arc::ptr_eq(&shape(w), run)), "a moved run was shaped again");
    cache.set_shape_cache_bytes(bytes / 2);
    let held = cache.shape_cache_stats().0;
    assert!(held < words.len());
    let newest = words.len() - held;
    let kept = |(w, run): (&alloc::string::String, &Arc<Run>)| Arc::ptr_eq(&shape(w), run);
    assert!(words[newest..].iter().zip(&first[newest..]).all(kept), "an older run outlived a newer");
}

// A budget changing the count of lists between a miss picking its list and taking its lock keeps nothing there, either
// way, where no read would look; picked again, the run is kept.
#[cfg(feature = "threading")]
#[test]
fn a_run_picked_a_list_before_the_count_changed_is_not_kept_there() {
    use super::SHAPE_CACHE_BYTES;
    use crate::daecore::daekit::text::shape::{shape_run, ShapeOptions};
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let past_first = |w: &alloc::string::String| {
        let head = (&axes, w.as_bytes(), false, None, "", "", None, None);
        cache.run_list(&head).is_some_and(|(_, at)| at != 0)
    };
    let text = (0..).map(|i| alloc::format!("word{i}")).find(past_first).expect("a text past the first list");
    let parts = ((&axes, text.as_bytes(), false, None, "", "", None, None), &[][..]);
    let run = shape_run(&cache, &axes, &text, false, None, &ShapeOptions::default());
    for (from, to) in [(SHAPE_CACHE_BYTES, 1 << 20), (1 << 20, SHAPE_CACHE_BYTES)] {
        cache.set_shape_cache_bytes(from);
        let picked = cache.run_list(&parts.0).expect("a list");
        cache.set_shape_cache_bytes(to);
        cache.keep_run(&parts, &text, &run, picked);
        assert_eq!(cache.shape_cache_stats().0, 0, "a run was kept where no read looks, from {from} bytes to {to}");
    }
    cache.keep_run(&parts, &text, &run, cache.run_list(&parts.0).expect("a list"));
    assert_eq!(cache.shape_cache_stats().0, 1);
    assert!(Arc::ptr_eq(&run, &cache.shaped_run(&axes, &text, false)), "a run kept in its own list wasn't read");
}

// Under a budget of 0 a shape passes every list by, keeping nothing, from either count of lists, and a budget given
// back keeps runs again.
#[cfg(feature = "threading")]
#[test]
fn a_budget_of_0_takes_no_list() {
    use super::{RUN_SHARDS, SHAPE_CACHE_BYTES};
    let cache = fixture("inter/InterVariable.ttf");
    let axes = AxisKey::default();
    let head = (&axes, "typeface".as_bytes(), false, None, "", "", None, None);
    let shape = || cache.shaped_run(&axes, "typeface", false);
    let alone = shape();
    for budget in [SHAPE_CACHE_BYTES, 1 << 16] {
        cache.set_shape_cache_bytes(budget);
        let kept = shape();
        assert!(Arc::ptr_eq(&kept, &shape()), "a budget of {budget} kept no run");
        cache.set_shape_cache_bytes(0);
        assert_eq!(cache.shape_cache_stats(), (0, 0), "a budget of 0 held on to a run");
        assert!(cache.run_list(&head).is_none(), "a budget of 0 took a list");
        let past = shape();
        assert_eq!(past.glyphs, alone.glyphs);
        assert!(!Arc::ptr_eq(&past, &shape()), "a budget of 0 served a run again");
        assert_eq!(cache.shape_cache_stats(), (0, 0), "a budget of 0 kept a run");
    }
    cache.set_shape_cache_bytes(0);
    assert!(cache.run_list(&head).is_none(), "a budget of 0 set again took a list");
    cache.set_shape_cache_bytes(SHAPE_CACHE_BYTES);
    assert_eq!(cache.run_shards().in_use.get(), RUN_SHARDS);
    assert!(cache.run_list(&head).is_some());
}

// A class definition or coverage that rules in several lookups name is built once, under its table's lookup slots,
// and each lookup's index holds that one copy.
#[test]
fn lookups_naming_one_rule_table_share_its_index() {
    use crate::daecore::daeshaper::{plan::{Plan, PlanRequest}, unicode::script};
    let cache = fixture("noto-devanagari/NotoSansDevanagari.ttf");
    let face = Face::new(&cache);
    let deva = script('क');
    let tags = crate::daecore::daeshaper::ot::tag::script_tags(deva);
    let request = PlanRequest {
        script: Some(deva),
        direction: Direction::LeftToRight,
        script_tags: tags.as_slice(),
        language_tags: &[],
        language: None,
        features: &[],
        switches: &[],
    };
    let plan = Plan::new(&face, &request);
    let mut holders = alloc::collections::BTreeMap::<*const _, alloc::collections::BTreeSet<usize>>::new();
    for (lookup, index) in [&plan.gsub.indexes, &plan.gpos.indexes].into_iter().flatten().enumerate() {
        for rule in index.iter().flat_map(|i| i.subtables.iter().flat_map(|s| s.rules.iter().flatten())) {
            holders.entry(crate::daecore::daekit::base::sync::Shared::as_ptr(rule)).or_default().insert(lookup);
        }
    }
    assert!(holders.values().any(|lookups| lookups.len() > 1), "no lookups share a rule table's index");
}

// Without the index, a code looked up twice answers as the subtables do both times, and so does the code sharing its
// slot, read right after; a code past Unicode leaves the slots as they were.
#[test]
fn recent_glyphs_answer_as_the_subtables() {
    let codes = (0..0x3_0000).chain(0x10_FF00..0x11_0000);
    let fonts = [
        "inter/InterVariable.ttf",
        "source-han-sans/SourceHanSansJP-VF.otf",
        "noto-myanmar/NotoSansMyanmar.ttf",
    ];
    for path in fonts {
        let cache = fixture(path);
        let want = |code: u32| {
            let (map, tables) = cache.char_map();
            map.glyph_id(tables, code)
        };
        for code in codes.clone() {
            assert_eq!([cache.glyph_id(code), cache.glyph_id(code)], [want(code); 2], "{path}: U+{code:04X}");
            assert_eq!(cache.glyph_id(code ^ 0x100), want(code ^ 0x100), "{path}: U+{:04X} after", code ^ 0x100);
        }
        assert!(cache.font.cmap.index.get().is_none(), "{path}: the index was built");
        use core::sync::atomic::Ordering::Relaxed;
        let slots = |c: &FontCache| c.font.cmap.recent.0.each_ref().map(|s| s.load(Relaxed));
        let before = slots(&cache);
        assert_eq!([0x11_0000, u32::MAX].map(|c| cache.glyph_id(c)), [None; 2], "{path}: past Unicode");
        assert_eq!(slots(&cache), before, "{path}: a code past Unicode was held");
    }
}

// A glyph past 16 bits isn't held, so its code answers from the subtable each time.
#[test]
fn a_glyph_past_16_bits_is_not_held() {
    let mut head = alloc::vec![0u8; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let maxp = [0, 0, 0x50, 0, 0, 10];
    // One format 12 subtable (3, 10): U+0041 to glyph 70,000 and U+0042 to glyph 5.
    let mut format12 = alloc::vec![0, 12, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 2];
    [[0x41, 0x41, 70_000u32], [0x42, 0x42, 5]].iter().flatten().for_each(|v| format12.extend(v.to_be_bytes()));
    let mut cmap = alloc::vec![0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12];
    cmap.extend(format12);
    let tables: alloc::collections::BTreeMap<alloc::string::String, Vec<u8>> =
        [("head", head), ("maxp", maxp.to_vec()), ("cmap", cmap)].map(|(t, b)| (t.into(), b)).into();
    let font = crate::daecore::daetype::decoder::build_ttf(&tables).unwrap();
    let cache = FontCache::new(crate::daecore::daetype::decoder::extract_ttf_tables(&font).expect("parses"))
        .expect("loads");
    assert_eq!([0x41, 0x41, 0x42, 0x42].map(|c| cache.glyph_id(c)), [Some(70_000), Some(70_000), Some(5), Some(5)]);
    let slot = |code: usize| cache.font.cmap.recent.0[code].load(core::sync::atomic::Ordering::Relaxed);
    assert_eq!([slot(0x41), slot(0x42)], [u32::MAX, 5], "U+0041 was held, or U+0042 wasn't");
}
