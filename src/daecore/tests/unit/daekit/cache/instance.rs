use super::*;

// A location another read dropped from the budget is built and kept again when read here next, though this
// thread read it last: what the thread kept of it can't stand in for the font's own.
#[test]
fn a_location_dropped_from_the_budget_is_kept_again_when_read() {
    let path = alloc::format!("{}/assets/test-fonts/inter/InterVariable.ttf", env!("CARGO_MANIFEST_DIR"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let fc = FontCache::new(map.expect("parses")).expect("loads");
    fc.set_instance_cache_bytes(0);
    let (bold, light) = (fc.axis_key(&[("wght", 700.0)]), fc.axis_key(&[("wght", 300.0)]));
    fc.described_at(&bold, |_| ());
    fc.at_location(&light, |_| ());
    fc.described_at(&bold, |_| ());
    assert!(write(&fc.locations().list).get(&bold).is_some(), "the weight read last wasn't kept");
}

// A read at a new location goes on past a list another thread holds, on a cache of its own that it doesn't keep, and
// its next read at that location keeps one.
#[cfg(feature = "threading")]
#[test]
fn a_read_goes_on_past_a_busy_list_of_locations() {
    let path = alloc::format!("{}/assets/test-fonts/inter/InterVariable.ttf", env!("CARGO_MANIFEST_DIR"));
    let open = || {
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(&path).expect("a fixture"));
        FontCache::new(map.expect("parses")).expect("loads")
    };
    let (alone, fc) = (&open(), &open());
    let bold = &fc.axis_key(&[("wght", 700.0)]);
    let advance = |at: &FontCache| i64::from(at.advances(false).of(36));
    let expected = alone.described_at(bold, advance);
    assert_ne!(expected, advance(alone), "a weight that moves nothing proves nothing");
    let held = write(&fc.locations().list);
    let (tell, told) = std::sync::mpsc::channel();
    let (go, gone) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|s| {
        s.spawn(move || {
            let _ = tell.send(fc.described_at(bold, advance));
            if gone.recv().is_ok() {
                let _ = tell.send(fc.described_at(bold, advance));
            }
        });
        // Dropped with this closure, so a failure here doesn't leave the thread waiting.
        let go = go;
        let past = told.recv_timeout(std::time::Duration::from_secs(20));
        drop(held);
        assert_eq!(past.expect("the read waited for the list"), expected);
        assert_eq!(fc.instance_cache_stats().0, 0, "a cache made past a busy list was kept");
        go.send(()).expect("the thread is waiting");
        assert_eq!(told.recv().expect("the thread reads again"), expected);
    });
    assert_eq!(fc.instance_cache_stats().0, 1, "the next read didn't keep the location");
}

// A location's cache kept after the location left the budget charges nothing as it grows, and one in it charges its
// growth alone: the budget's count stays what its locations hold.
#[test]
fn a_cache_kept_past_its_location_charges_nothing() {
    let path = alloc::format!("{}/assets/test-fonts/inter/InterVariable.ttf", env!("CARGO_MANIFEST_DIR"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let fc = FontCache::new(map.expect("parses")).expect("loads");
    fc.set_instance_cache_bytes(0);
    let draw = |at: &FontCache, glyphs: core::ops::Range<u32>| {
        let loca = at.loca_offsets().expect("loca");
        for gid in glyphs {
            let path = &mut crate::daecore::daetype::outline::Path::default();
            let _ = at.draw_glyf_reusing(loca.into(), gid, &mut 0, path);
        }
    };
    let (bold, light) = (fc.axis_key(&[("wght", 700.0)]), fc.axis_key(&[("wght", 300.0)]));
    let kept = fc.location_cache(&bold, Slot::Drawn).0.expect("built");
    let (at_light, _) = fc.location_cache(&light, Slot::Drawn);
    let at_light = at_light.expect("built");
    assert!(write(&fc.locations().list).get(&bold).is_none(), "a budget of 0 kept the older weight");
    let held = fc.instance_cache_stats().1;
    fc.charged(&kept, |at| draw(at, 0..80));
    assert_eq!(fc.instance_cache_stats().1, held, "a cache out of the budget was charged");
    fc.charged(&at_light, |at| draw(at, 0..80));
    let grown = fc.instance_cache_stats().1;
    assert!(grown > held);
    assert_eq!(grown, located_cost(&light, &write(&fc.locations().list).get(&light).cloned().expect("kept")));
}

// Without `threading` a font keeps the locations it resolved last, as a thread does with it: one read again and again
// stays through a hundred others, and the list holds no more than eight.
#[cfg(not(feature = "threading"))]
#[test]
fn a_resolved_location_read_again_stays() {
    let path = alloc::format!("{}/assets/test-fonts/inter/InterVariable.ttf", env!("CARGO_MANIFEST_DIR"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let fc = FontCache::new(map.expect("parses")).expect("loads");
    let bold = fc.axis_key(&[("wght", 700.0)]);
    let first = fc.resolve_axes(&bold);
    for w in 0..100 {
        fc.resolve_axes(&fc.axis_key(&[("wght", 200.0 + f64::from(w))]));
        assert!(Shared::ptr_eq(&first, &fc.resolve_axes(&bold)), "resolved again after {w} others");
    }
    assert_eq!(read(&fc.location_by_axis).len(), KEPT_AXES);
}
