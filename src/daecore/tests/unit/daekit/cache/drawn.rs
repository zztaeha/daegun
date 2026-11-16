use super::*;

fn planted() -> Path {
    let mut p = Path::default();
    p.move_to(1.0, 2.0);
    p.line_to(30.0, 4.0);
    p.close();
    p
}

// Down, a glyph's top is phantom point 3 as the location moves it: gvar-points's E at its yMax of 700 plus a top
// side bearing of 100, then raised 40.
#[test]
fn the_drawn_top_moves_with_phantom_point_3() {
    struct Raised;
    impl Vary for Raised {
        fn deltas(&self, _: u32, _: Shape<'_>, _: &mut usize) -> Result<Deltas, crate::daecore::Error> {
            Ok(Some(Shared::<[_]>::from(alloc::vec![(0, 0), (0, 0), (0, 40), (0, 0)]).into()))
        }
        fn known(&self, _: u32, _: &mut usize) -> Option<Result<Deltas, crate::daecore::Error>> {
            None
        }
    }
    let dir = env!("CARGO_MANIFEST_DIR");
    let path = alloc::format!("{dir}/assets/test-fonts/test-fixtures/gvar-points.ttf");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let mut map = map.expect("parses");
    let mut vhea = alloc::vec![0, 1, 0x10, 0];
    vhea.resize(34, 0);
    vhea.extend([0, 6]);
    let vmtx = [[0x03, 0xE8, 0, 0]; 6].concat();
    map.insert("vhea", vhea.into());
    map.insert("vmtx", [&vmtx[..22], &[0, 100]].concat().into());
    let cache = FontCache::new(map).expect("loads");
    let (glyf, loca) = (cache.glyf.as_ref().expect("a glyf"), cache.loca_offsets().expect("a loca"));
    let top = |vary: &dyn Vary| {
        cache.drawn_phantom::<true>(glyf, loca.into(), 5, vary, &mut GlyphCoords::default(), 0, &mut 1_000)
    };
    assert_eq!(top(&Stored), Ok(800));
    assert_eq!(top(&Raised), Ok(840));
}

// A chain of 60,000 composites, each naming the next by USE_MY_METRICS, over a simple glyph: the origin is found at
// its foot, and a chain leading back into itself is refused.
#[test]
fn a_drawn_origin_follows_metrics_as_deep_as_composites_go() {
    use crate::daecore::daetype::format::glyf::USE_MY_METRICS;
    let origin = |last: Option<u16>| {
        let (mut glyf, mut loca) = (Vec::new(), alloc::vec![0u32]);
        for next in (1..=60_000u16).map(|k| if k == 60_000 { last.unwrap_or(k) } else { k }) {
            glyf.extend([0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
            glyf.extend([USE_MY_METRICS.to_be_bytes(), next.to_be_bytes(), [0, 0]].concat());
            loca.push(glyf.len() as u32);
        }
        glyf.extend([0, 1, 0, 50, 0, 0, 0, 50, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        loca.push(glyf.len() as u32);
        let dir = env!("CARGO_MANIFEST_DIR");
        let path = alloc::format!("{dir}/assets/test-fonts/test-fixtures/gvar-points.ttf");
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
        let mut map = map.expect("parses");
        let mut head = map.get("head").expect("head").to_vec();
        head[50..52].copy_from_slice(&[0, 1]);
        let mut maxp = map.get("maxp").expect("maxp").to_vec();
        maxp[4..6].copy_from_slice(&60_001u16.to_be_bytes());
        map.insert("head", head.into());
        map.insert("maxp", maxp.into());
        map.insert("glyf", glyf.into());
        map.insert("loca", loca.iter().flat_map(|o| o.to_be_bytes()).collect::<Vec<u8>>().into());
        map.remove("gvar");
        let cache = FontCache::new(map).expect("loads");
        let (glyf, loca) = (cache.glyf.as_ref().expect("a glyf"), cache.loca_offsets().expect("a loca"));
        let scratch = &mut GlyphCoords::default();
        cache.drawn_phantom::<false>(glyf, loca.into(), 0, &Stored, scratch, 0, &mut (1 << 24)).map_err(|e| e.kind())
    };
    assert!(origin(None).is_ok());
    assert_eq!(origin(Some(0)), Err(crate::daecore::ErrorKind::Malformed));
}

// An outline no decode could draw, so a draw that returns it read the kept one: at the default from the font's
// store, at a location from its own.
#[test]
fn a_cff2_glyph_drawn_again_replays_its_kept_outline() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let path = alloc::format!("{dir}/assets/test-fonts/source-serif/SourceSerif4Variable-Roman.otf");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let cache = FontCache::new(map.expect("parses")).expect("loads");
    let draw = |at: &FontCache, gid| {
        let mut p = Path::default();
        at.draw_cff2(gid, None, &mut 0, &mut p).expect("CFF2").expect("draws");
        p
    };
    draw(&cache, 5);
    cache.font.cff2_kept.get().expect("the default's store").keep(6, planted());
    assert_eq!(draw(&cache, 6), planted());
    let bold = cache.axis_key(&[("wght", 700.0)]);
    cache.at_location(&bold, |at| {
        assert_ne!(draw(at, 6), planted(), "a location read the default's store");
        at.cff2_at.as_ref().expect("a location").kept.keep(7, planted());
        assert_eq!(draw(at, 7), planted());
    });
    assert_ne!(draw(&cache, 7), planted(), "the default read a location's store");
}

#[test]
fn outlines_stop_being_kept_past_their_budget() {
    let kept = KeptOutlines::new(130);
    kept.keep(0, planted());
    assert_eq!(kept.get(0), Some(&planted()));
    kept.keep(0, Path::default());
    assert_eq!(kept.get(0), Some(&planted()), "a kept outline was replaced");
    kept.keep(200, planted());
    assert_eq!(kept.get(200), None, "a glyph past the count was kept");
    kept.bytes.set(KEPT_OUTLINE_BYTES);
    kept.keep(129, planted());
    assert_eq!(kept.get(129), None, "an outline was kept past the budget");
}

// gvar-points, whose glyph 1 has one contour of 4 points and one tuple of 19 bytes at wght 900.
fn gvar_points() -> FontCache {
    let dir = env!("CARGO_MANIFEST_DIR");
    let path = alloc::format!("{dir}/assets/test-fonts/test-fixtures/gvar-points.ttf");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    FontCache::new(map.expect("parses")).expect("loads")
}

// The work drawing glyph 1's points by `vary` takes.
fn drawn_work(cache: &FontCache, vary: &dyn Vary) -> usize {
    use crate::daecore::daetype::outline::{glyph_points_varied, Points};
    let (glyf, loca) = (cache.glyf.as_ref().expect("a glyf"), cache.loca_offsets().expect("a loca"));
    let (mut scratch, mut work) = (GlyphCoords::default(), 0);
    let points = Points::default();
    glyph_points_varied(glyf, false, loca.into(), 1, &mut scratch, vary, points, &mut work).expect("it draws");
    work
}

// gvar-points's glyph 1 drawn at wght 900 takes the tuple's unit and axis and its bytes and 8 points, phantoms among
// them, from the draw's work, beside the visit, contour and points the default takes.
#[test]
fn a_glyph_drawn_at_a_location_spends_its_tuples_from_the_draw() {
    let cache = gvar_points();
    let key = cache.axis_key(&[("wght", 900.0)]);
    let varied = cache.at_location(&key, |at| drawn_work(&cache, at.drawn.as_deref().expect("glyphs moved there")));
    assert_eq!((drawn_work(&cache, &Stored), varied), (1 + 1 + 4, 1 + 1 + 4 + 2 + 19 + 8));
}

// Glyph 1's origin at wght 900 takes 5 to decode it and 29 for its tuple, whether or not a draw kept its deltas first:
// 34 holds it and 33 fails it both ways.
#[test]
fn a_glyph_s_origin_spends_alike_whether_its_deltas_were_kept() {
    let origin = |kept: bool, mut left: usize| {
        let cache = gvar_points();
        let (glyf, loca) = (cache.glyf.as_ref().expect("a glyf"), cache.loca_offsets().expect("a loca"));
        cache.at_location(&cache.axis_key(&[("wght", 900.0)]), |at| {
            let vary: &dyn Vary = at.drawn.as_deref().expect("glyphs moved there");
            if kept {
                drawn_work(&cache, vary);
            }
            let scratch = &mut GlyphCoords::default();
            cache.drawn_phantom::<false>(glyf, loca.into(), 1, vary, scratch, 0, &mut left).is_ok()
        })
    };
    assert_eq!([origin(false, 34), origin(false, 33), origin(true, 34), origin(true, 33)], [true, false, true, false]);
}

// Glyph 1's whole-unit deltas at wght 900, and the 26.6 ones hinting takes, each spend the 29 working them out took,
// worked out or kept.
#[test]
fn kept_deltas_spend_what_working_them_out_took() {
    let cache = gvar_points();
    let (glyf, loca) = (cache.glyf.as_ref().expect("a glyf"), cache.loca_offsets().expect("a loca"));
    let mut points = GlyphCoords::default();
    assert!(extract_coords_into(glyph_bytes(glyf, loca.into(), 1).expect("glyph 1"), 1, &mut points));
    let spent = cache.at_location(&cache.axis_key(&[("wght", 900.0)]), |at| {
        let vary: &dyn Vary = at.drawn.as_deref().expect("glyphs moved there");
        let spent = |fine: bool| {
            let (shape, mut left) = (Shape::simple(&points), 1_000);
            match fine {
                true => vary.deltas_f26dot6(1, shape, &mut left).map(drop),
                false => vary.deltas(1, shape, &mut left).map(drop),
            }
            .expect("deltas");
            1_000 - left
        };
        [spent(false), spent(false), spent(true), spent(true)]
    });
    assert_eq!(spent, [29; 4]);
}
