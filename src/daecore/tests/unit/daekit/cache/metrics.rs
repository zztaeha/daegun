use super::{FontCache, MetricsTable};
use alloc::string::String;
use alloc::vec::Vec;

fn fonts(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a fixture directory").flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            _ if path.is_dir() => fonts(&path, out),
            Some("ttf" | "otf") => out.push(path),
            _ => {}
        }
    }
}

fn reads_alike(cache: &FontCache, what: &str) {
    for vertical in [false, true] {
        let tags = crate::daecore::daetype::format::metrics::shaped_tags(cache.tables(), vertical);
        let read = tags.header(cache.tables()).and_then(|_| tags.table(cache.tables()));
        let (kept, mtx) = (cache.metrics_table(vertical), tags.mtx);
        assert_eq!(kept.is_some(), read.is_some(), "{what} {mtx}");
        let (Some(kept), Some(read)) = (kept, read) else { continue };
        assert_eq!((kept.len(), kept.long()), (read.len(), read.long()), "{what} {mtx}");
        for gid in 0..kept.len() + 2 {
            let field = |t: &MetricsTable| (t.advance(gid), t.bearing(gid));
            assert_eq!(field(&kept), field(&read), "{what} {mtx} glyph {gid}");
        }
    }
}

// What construction keeps reads as `MetricsTable::read` reads the cache's own tables, glyph for glyph, in every
// fixture at its default and a variable one at its first axis's maximum, whose instance keeps the glyph count.
#[test]
fn kept_metrics_read_as_the_tables_do() {
    let mut paths = Vec::new();
    fonts(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts")), &mut paths);
    assert!(paths.len() > 20, "the fixtures are missing");
    for path in paths {
        let what = alloc::format!("{}", path.display());
        let bytes = std::fs::read(&path).expect("a fixture");
        let Ok(map) = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes) else { continue };
        let Ok(cache) = FontCache::new(map) else { continue };
        reads_alike(&cache, &what);
        let axes = crate::daecore::daetype::decoder::parse_fvar_axes(cache.tables()).unwrap_or_default();
        if let Some(axis) = axes.first() {
            let key = cache.axis_key(&[(axis.tag, axis.max)]);
            cache.described_at(&key, |at| reads_alike(at, &what));
        }
    }
}

// hinted.ttf's cache with glyph `gid` replaced, loca rewritten long.
fn hinted_with(gid: usize, record: &[u8]) -> FontCache {
    with_glyph("hinted.ttf", gid, record)
}

// A fixture's cache with glyph `gid` replaced, loca rewritten long.
fn with_glyph(fixture: &str, gid: usize, record: &[u8]) -> FontCache {
    use crate::daecore::daetype::TableBytes;
    let path = alloc::format!("{}/assets/test-fonts/test-fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(path).expect("a fixture");
    let mut map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let (glyf, loca) = (map["glyf"].to_vec(), map["loca"].to_vec());
    let at = |g: usize| 2 * usize::from(u16::from_be_bytes([loca[2 * g], loca[2 * g + 1]]));
    let (mut new_glyf, mut new_loca) = (Vec::new(), Vec::new());
    for g in 0..loca.len() / 2 - 1 {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        new_glyf.extend_from_slice(if g == gid { record } else { &glyf[at(g)..at(g + 1)] });
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut head = map["head"].to_vec();
    head[50..52].copy_from_slice(&1i16.to_be_bytes());
    map.insert(String::from("head"), TableBytes::from(head));
    map.insert(String::from("glyf"), TableBytes::from(new_glyf));
    map.insert(String::from("loca"), TableBytes::from(new_loca));
    FontCache::new(map).expect("loads")
}

// A glyph's (x bearing, y bearing, width, height) as extents give it.
fn extents(cache: &FontCache, gid: u32) -> Option<(i32, i32, i32, i32)> {
    cache.glyph_extents(gid).map(|e| (e.x_bearing, e.y_bearing, e.width, e.height))
}

// hinted.ttf's glyph 1, A: drawn 37 to 337 across and 0 to 703 up, its left side bearing 37.
fn hinted_a() -> (FontCache, Vec<u8>) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/hinted.ttf");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let plain = FontCache::new(map.expect("parses")).expect("loads");
    let (start, end) = plain.loca_offsets().map(|l| (l.get(1), l.get(2))).expect("a loca");
    let record = plain.glyf.as_ref().expect("glyf")[start.expect("glyph 1")..end.expect("glyph 2")].to_vec();
    (plain, record)
}

// A box is the drawn outline's, x from the origin its header's xMin less its left side bearing places: an empty record
// or 0 contours has none, one under 10 bytes fails to draw, and A's backward box puts the origin 263 right of its ink.
#[test]
fn a_glyph_s_box_is_its_drawn_outlines_from_its_origin() {
    let (plain, record) = hinted_a();
    assert_eq!(extents(&plain, 1), Some((37, 703, 300, -703)));
    let header = [0, 0, 0, 0x25, 0, 0, 1, 0x51, 2, 0xbf];
    let empty: [&[u8]; 3] = [&[], &header, &[&header[..], &[0, 0]].concat()];
    for record in empty {
        assert_eq!(extents(&hinted_with(1, record), 1), Some((0, 0, 0, 0)), "{record:?}");
    }
    let cut: [&[u8]; 5] = [&[0], &[0, 1], &[0, 1, 0, 0x25], &[0xff, 0xff, 0, 0x25], &[0, 1, 0, 0x25, 0, 0, 1, 0x51, 2]];
    for record in cut {
        assert_eq!(extents(&hinted_with(1, record), 1), None, "{record:?}");
    }
    let mut backwards = record;
    let (x_min, x_max) = ([backwards[2], backwards[3]], [backwards[6], backwards[7]]);
    backwards[2..4].copy_from_slice(&x_max);
    backwards[6..8].copy_from_slice(&x_min);
    assert_eq!(extents(&hinted_with(1, &backwards), 1), Some((-263, 703, 300, -703)), "a box stored backwards");
}

// At the default a glyph is boxed by its drawn outline as anywhere else, not by a header box gone stale: A's header
// claiming -50 to 803 up and 37 to 437 across still boxes it as drawn, 0 to 703 up and 37 to 337 across.
#[test]
fn a_glyph_whose_header_box_is_stale_is_boxed_by_its_drawn_outline() {
    let (_, mut stale) = hinted_a();
    for (at, v) in [(4, -50i16), (6, 437), (8, 803)] {
        stale[at..at + 2].copy_from_slice(&v.to_be_bytes());
    }
    let cache = hinted_with(1, &stale);
    let drawn = cache.drawn_bounds(1, &mut 0).flatten().map(|[x0, y0, x1, y1]| [x0, y0, x1, y1].map(|v| v as i32));
    assert_eq!(drawn, Some([37, 0, 337, 703]));
    assert_eq!(extents(&cache, 1), Some((37, 703, 300, -703)));
}

fn fixture(path: &str) -> FontCache {
    let bytes = std::fs::read(alloc::format!("{}/assets/test-fonts/{path}", env!("CARGO_MANIFEST_DIR")));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes.expect("a fixture"));
    FontCache::new(map.expect("parses")).expect("loads")
}

fn extents_of(cache: &FontCache, gids: &[u32]) -> Vec<Option<(i32, i32, i32, i32)>> {
    gids.iter().map(|&g| cache.glyph_extents(g).map(|e| (e.x_bearing, e.y_bearing, e.width, e.height))).collect()
}

// Each edge rounds half away from 0 before the size is taken (half-edges.otf), a CFF outline's and a color glyph's
// painted box alike: 'low', -2.5 to 7.5 across, spans -3 to 8, 11 wide.
#[test]
fn a_box_on_halves_rounds_each_edge_before_its_size() {
    let cache = fixture("test-fixtures/half-edges.otf");
    let cff = [(-3, 11, 11, -8), (1, -1, 9, -10), (-8, 5, 6, -9)].map(Some);
    let colr = [(11, 351, 150, -350), (-159, 2, 150, -351)].map(Some);
    assert_eq!(extents_of(&cache, &[1, 2, 3]), cff);
    assert_eq!(extents_of(&cache, &[5, 6]), colr);
}

// A glyph whose FD is absent, its range naming one past the FDArray or an FDSelect of no known format, measures in FD
// 0; a sentinel past the glyphs or a first range past glyph 0 leaves its range as it was. Nothing refuses the table.
#[test]
fn a_glyph_whose_fd_is_absent_measures_in_fd_0() {
    use crate::daecore::daetype::{format::cff::{read_dict, DictKind}, subsetter::cff::parse::read_header};
    for (path, tag, gid) in
        [("test-fixtures/cff-cid.otf", "CFF ", 1), ("source-serif/SourceSerif4Variable-Roman.otf", "CFF2", 2)]
    {
        let bytes = std::fs::read(alloc::format!("{}/assets/test-fonts/{path}", env!("CARGO_MANIFEST_DIR")));
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes.expect("a fixture")).expect("parses");
        let table = map[tag].to_vec();
        let top = match tag {
            "CFF2" => {
                let top_end = 5 + usize::from(u16::from_be_bytes([table[3], table[4]]));
                read_dict(&table[5..top_end], DictKind::Cff2)
            }
            _ => read_header(&table).expect("a header").top,
        };
        let at = top.fd_select.expect("an FDSelect");
        let sentinel = at + 3 + 3 * usize::from(u16::from_be_bytes([table[at + 1], table[at + 2]]));
        let with = |at: usize, bytes: &[u8]| {
            let (mut map, mut table) = (map.clone(), table.clone());
            table[at..at + bytes.len()].copy_from_slice(bytes);
            map.insert(String::from(tag), crate::daecore::daetype::TableBytes::from(table));
            FontCache::new(map).expect("loads")
        };
        // Each fixture's glyph `gid` starts range `gid`.
        let fd = at + 5 + 3 * gid as usize;
        let (in_fd_0, as_read) = (extents_of(&with(fd, &[0]), &[gid]), extents_of(&with(fd, &[table[fd]]), &[gid]));
        assert!(in_fd_0[0].is_some() && as_read[0].is_some(), "{path}");
        let past = (u16::from_be_bytes([table[sentinel], table[sentinel + 1]]) + 1).to_be_bytes();
        let cases = [(fd, &[200][..], &in_fd_0), (at, &[9], &in_fd_0), (sentinel, &past, &as_read)];
        for (at, bytes, want) in cases.into_iter().chain([(at + 3, &[0, 1][..], &as_read)]) {
            assert_eq!(&extents_of(&with(at, bytes), &[gid]), want, "{path} at {at}");
        }
    }
}

// A color glyph's extents are worked out once a cache, however often a run asks: colr-clip's past (25), whose scene
// is refused once its points are spent, fills its slot at the first ask, and the second gives the same answer.
// hinted.ttf's glyf glyphs are measured, which keeps their own, so no slot is made for them.
#[test]
fn a_color_glyphs_extents_are_kept_once_worked_out() {
    let cache = fixture("test-fixtures/colr-clip.ttf");
    let before = cache.held_bytes();
    let first = cache.glyph_extents(25);
    let kept = cache.kept_extents.get().expect("kept");
    let page = kept.0[0].get().expect("a page");
    assert_eq!(page[25].get(), Some(&first));
    assert!(cache.held_bytes() >= before + size_of::<super::ExtentsPage>());
    assert_eq!(cache.glyph_extents(25), first);
    let plain = fixture("test-fixtures/hinted.ttf");
    assert!(plain.glyph_extents(1).is_some());
    assert!(plain.kept_extents.get().is_none());
}

// At a location a color glyph is measured from the variable COLR: glyph 105 skewed at SKXA 84.732, its angle's delta
// rounded once into 2.14 (daegun's recorded output), and clip deltas of 62.5 floored at minimums, ceiled at maximums.
#[test]
fn a_color_glyph_at_a_location_is_measured_from_the_variable_table() {
    let cache = fixture("colr-v1-test-glyphs/test_glyphs_variable.ttf");
    let at = |axes: &[(&str, f64)], gids: &[u32]| {
        let axes: Vec<(String, f64)> = axes.iter().map(|&(tag, v)| (String::from(tag), v)).collect();
        cache.described_at(&cache.axis_key(&axes), |at| extents_of(at, gids))
    };
    assert_eq!(at(&[("SKXA", 84.732)], &[105]), [Some((-7893, 951, 8643, -701))]);
    let ties = [("CLXI", 62.5), ("CLYI", -62.5), ("CLXA", -62.5), ("CLYA", 62.5), ("CLIO", -62.5)];
    let boxes = [(62, 1063, 376, -626), (62, 563, 376, -626), (562, 563, 376, -626), (562, 1063, 376, -626),
        (312, 813, 376, -626), (37, 963, 926, -926)];
    assert_eq!(at(&ties, &[156, 157, 158, 159, 160, 166]), boxes.map(Some));
}

// A described location's advances both ways and VORG origins match the whole instance's glyph for glyph and past
// the last, in every variable fixture at extremes, halfway and every axis at maximum.
#[test]
fn a_location_varies_each_glyph_as_its_instance_does() {
    use super::AxisKey;
    use crate::daecore::daetype::{instancer, TableBytes};
    let whole = |cache: &FontCache, key: &AxisKey| {
        let tables = instancer::instance_tables_from_map(cache.tables(), cache.instance_reads(), key).ok()?;
        let map = tables.into_iter().map(|(tag, data)| (tag, TableBytes::from(data.into_owned()))).collect();
        let mut at = FontCache::new(map).ok()?;
        let location = cache.resolve_axes(key);
        let views = instancer::advance_views(cache.tables());
        let past = |v: bool| {
            let view = views[usize::from(v)].as_ref();
            let spare = Default::default();
            let advances = instancer::VariedAdvances::new(cache.tables(), v, &location.coords, None, view, &spare);
            advances.map_or(0, |a| a.past())
        };
        at.advances_past = [past(false), past(true)];
        Some(at)
    };
    let read = |at: &FontCache, gids: &[u32]| {
        let advance = |vertical: bool, gid: u32| at.advances(vertical).of(gid);
        let each = |f: &dyn Fn(u32) -> i64| gids.iter().map(|&gid| f(gid)).collect::<Vec<_>>();
        [
            each(&|gid| advance(false, gid).into()),
            each(&|gid| advance(true, gid).into()),
            each(&|gid| at.vorg_origins().map_or(i64::MIN, |o| o.of(gid).into())),
        ]
    };
    let mut paths = Vec::new();
    fonts(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts")), &mut paths);
    let (mut advances, mut origins) = (0, 0);
    for path in paths {
        let what = alloc::format!("{}", path.display());
        let bytes = std::fs::read(&path).expect("a fixture");
        let Ok(map) = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes) else { continue };
        let Ok(cache) = FontCache::new(map) else { continue };
        let Ok(axes) = crate::daecore::daetype::decoder::parse_fvar_axes(cache.tables()) else { continue };
        let gids: Vec<u32> = (0..=cache.num_glyphs.saturating_add(2)).rev().chain([u32::from(u16::MAX)]).collect();
        let mut keys: Vec<Vec<(crate::daecore::Tag, f64)>> = axes
            .iter()
            .flat_map(|a| [a.min, (a.min + a.default) / 2.0, (a.default + a.max) / 2.0, a.max].map(|v| (a, v)))
            .map(|(a, v)| alloc::vec![(a.tag, v)])
            .collect();
        keys.push(axes.iter().map(|a| (a.tag, a.max)).collect());
        for key in keys {
            let key: AxisKey = cache.axis_key(&key);
            let described = cache.described_at(&key, |at| {
                advances += at.varied_advances.iter().flatten().count();
                origins += usize::from(at.varied_origins.is_some());
                (read(at, &gids), read(at, &gids))
            });
            assert_eq!(described.0, described.1, "{what} {key:?} read again");
            let Some(instance) = whole(&cache, &key) else { continue };
            assert_eq!(described.0, read(&instance, &gids), "{what} {key:?}");
        }
    }
    assert!(advances > 0 && origins > 0, "{advances} sides and {origins} VORGs varied glyph by glyph");
}

// Every glyph's advance both ways, past the last too, is read where hmtx and vmtx lie and builds nothing; a font
// without them keeps one list a direction, half an em across and nothing down where it has no strikes.
#[test]
fn advances_are_read_where_they_lie() {
    use super::Advances;
    use crate::daecore::daetype::decoder::read_u16_be;
    let inter = fixture("inter/InterVariable.ttf");
    let held = inter.held.built.get();
    for (vertical, gid) in [false, true].into_iter().flat_map(|v| (0..inter.num_glyphs + 3).map(move |g| (v, g))) {
        inter.advances(vertical).of(gid);
    }
    assert_eq!(inter.held.built.get(), held, "reading advances built something");
    assert!(matches!(inter.advances(false), Advances::Table(..)));
    let hmtx = &inter.tables()["hmtx"];
    let last = inter.num_glyphs - 1;
    assert_eq!(inter.advances(false).of(last), u32::from(read_u16_be(hmtx, 4 * last as usize).expect("a record")));
    assert_eq!(inter.advances(false).of(last + 1), 0, "past the glyphs at the default");

    let mut map = inter.tables().clone();
    map.retain(|tag, _| !matches!(tag, "hmtx" | "hhea" | "vmtx" | "vhea"));
    let bare = FontCache::new(map).expect("loads");
    assert!(matches!(bare.advances(false), Advances::Listed(_)));
    assert_eq!([bare.advances(false).of(5), bare.advances(true).of(5)], [u32::from(bare.upm) / 2, 0]);
    let built = bare.held.built.get();
    bare.advances(false).of(6);
    assert_eq!(bare.held.built.get(), built, "the list is built once");
}

// The lean read shaping makes gives `MetricsTable::advance` on each glyph it covers and `past` beyond, for tables
// whole, ending on half a field, and holding more records than glyphs; long records cut short leave both with none.
#[test]
fn table_advances_read_as_the_table_does() {
    use crate::daecore::daetype::format::metrics::MetricsTable;
    let data: Vec<u8> = (0..23u8).map(|b| b.wrapping_mul(37)).collect();
    for (len, stated, count) in [(23, 3, 9), (10, 3, 6), (11, 1, 5), (12, 1, 5), (16, 4, 2), (6, 9, 9)] {
        let Some(mtx) = MetricsTable::read(&data[..len], stated, count) else {
            assert!(super::AdvanceShape::of(&data[..len], stated, count).is_none(), "len {len}, stated {stated}");
            continue;
        };
        let table = super::AdvanceShape::of(&data[..len], stated, count).expect("has data").over(&data[..len], 7);
        for gid in 0..12u32 {
            let want = if (gid as usize) < mtx.len() { u32::from(mtx.advance(gid as usize)) } else { 7 };
            assert_eq!(table.of(gid), want, "len {len}, stated {stated}, count {count}, glyph {gid}");
        }
    }
}

// At a location a point-matched composite is boxed by its metrics as its drawn outline is, its component scaled to
// land between whole units: gvar-points.ttf's E made of A, then B scaled by 2/3 with its point 0 on A's point 2.
#[test]
fn a_point_matched_composites_box_is_its_drawn_outlines() {
    use crate::daecore::daetype::outline::PointBounds;
    let words = |v: &[i16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let composite = words(&[-1, 0, 0, 0, 0, 0x0023, 1, 0, 0, 0x0009, 2, 2, 0, 0x2AAB]);
    let cache = with_glyph("gvar-points.ttf", 5, &composite);
    let key = cache.axis_key(&[("wght", 650.0)]);
    let drawn = cache.at_location(&key, |at| {
        let mut pen = PointBounds::default();
        at.draw_glyf(at.loca_offsets()?.into(), 5, &mut 0, &mut pen).ok()?;
        pen.finish().map(|(x0, y0, x1, y1)| [x0, y0, x1, y1].map(f64::from))
    });
    let boxed = |e: super::GlyphExtents| (e.x_bearing, e.y_bearing, e.width, e.height);
    let measured = cache.described_at(&key, |at| at.glyph_extents(5)).map(boxed);
    let drawn = drawn.map(|b| boxed(super::GlyphExtents::from_bounds(b, |v| v.round())));
    assert!(drawn.is_some_and(|(x, _, w, _)| x + w > 700));
    assert_eq!(measured, drawn);
}

// Glyph 1 names glyph 2 a thousand times by USE_MY_METRICS, and glyph 2's thousand components name glyph 3 plainly,
// read on every visit though none is followed: given 100,000 units of work, the walk spends them on those reads.
#[test]
fn a_top_walk_spends_work_on_components_it_does_not_follow() {
    use crate::daecore::daetype::format::glyf::{MORE_COMPONENTS, USE_MY_METRICS};
    let composite = |gid: u16, flags: u16| {
        let mut g = alloc::vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for i in 0..1_000 {
            let more = if i + 1 < 1_000 { MORE_COMPONENTS } else { 0 };
            g.extend([(flags | more).to_be_bytes(), gid.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    let (one, two) = (composite(2, USE_MY_METRICS), composite(3, 0));
    let glyf = [one.as_slice(), &two].concat();
    let (loca, vmtx) = ([0, 0, one.len(), glyf.len(), glyf.len()], MetricsTable::new(&[], 0));
    let mut walk = super::TopWalk { glyf: &glyf, extended: false, loca: (&loca[..]).into(), vmtx: &vmtx, edges: 0,
        work: 100_000 };
    assert_eq!(walk.top(1, &mut 0), None);
}

// A chain of 60,000 composites, each naming the next by USE_MY_METRICS, over a glyph whose top is 500: its top is the
// chain's.
#[test]
fn a_top_walk_goes_as_deep_as_composites_go() {
    use crate::daecore::daetype::format::glyf::USE_MY_METRICS;
    let (mut glyf, mut loca) = (Vec::new(), alloc::vec![0]);
    for next in 1..=60_000u16 {
        glyf.extend([0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
        glyf.extend([USE_MY_METRICS.to_be_bytes(), next.to_be_bytes(), [0, 0]].concat());
        loca.push(glyf.len());
    }
    glyf.extend([0, 1, 0, 0, 0, 0, 0, 0, 0x01, 0xF4, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
    loca.push(glyf.len());
    let vmtx = MetricsTable::new(&[], 0);
    let mut walk = super::TopWalk { glyf: &glyf, extended: false, loca: (&loca[..]).into(), vmtx: &vmtx, edges: 0,
        work: 1 << 24 };
    assert_eq!(walk.top(0, &mut 0), Some(500));
}

// A font of glyph 1, `points` points from (0, 0) to (100, 200); 64 composites of it, glyphs 2 to 65; and COLR
// version 0 base glyphs 66, its 64 layers all glyph 1, and 67, its layers the 64 composites.
fn layered(points: u16) -> FontCache {
    use crate::daecore::daetype::TableBytes;
    let words = |w: &[i16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let mut heavy = words(&[1, 0, 0, 100, 200, points as i16 - 1, 0]);
    heavy.extend([0x31, 0x37]);
    let mut left = usize::from(points) - 2;
    while left > 0 {
        let n = left.min(256);
        heavy.extend([0x39, (n - 1) as u8]);
        left -= n;
    }
    heavy.extend([100, 200]);
    let composite = words(&[-1, 0, 0, 100, 200, 3, 1, 0, 0]);
    let glyphs = [vec![Vec::new(), heavy], vec![composite; 64], vec![Vec::new(); 2]].concat();
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in &glyphs {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(g);
        glyf.resize(glyf.len().next_multiple_of(2), 0);
    }
    loca.extend((glyf.len() as u32).to_be_bytes());
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let maxp = [&[0u8, 0, 0x50, 0][..], &(glyphs.len() as u16).to_be_bytes()].concat();
    let mut colr = words(&[0, 2, 0, 14, 0, 26, 128]);
    colr.extend(words(&[66, 0, 64, 67, 64, 64]));
    colr.extend((0..128).flat_map(|i| words(&[if i < 64 { 1 } else { i - 62 }, 0])));
    let cpal = [&[0u16, 1, 1, 1, 0, 14, 0].map(u16::to_be_bytes).concat()[..], &[0, 0, 0, 255]].concat();
    let tables = [("head", head), ("maxp", maxp), ("loca", loca), ("glyf", glyf), ("COLR", colr), ("CPAL", cpal)];
    FontCache::new(tables.into_iter().map(|(t, b)| (String::from(t), TableBytes::from(b))).collect()).expect("loads")
}

// A color glyph's layers measure each glyph once, within the points drawn and decoded its scene may hold: 64 layers
// of one 16,400-point glyph measure, 64 composites of it pass the bound, and of 8,000 points don't.
#[test]
fn a_color_glyph_s_layers_measure_each_glyph_once_within_a_scene_s_points() {
    let cache = layered(16_400);
    assert_eq!(extents(&cache, 1), Some((0, 200, 100, -200)));
    assert_eq!(extents_of(&cache, &[66, 67]), [Some((0, 200, 100, -200)), Some((0, 0, 0, 0))]);
    let cache = layered(8_000);
    assert_eq!(extents(&cache, 67), Some((0, 200, 100, -200)), "64 composites within the bound");
}

// Glyph 2 holds 1,000 components of the empty glyph 1, and each of glyphs 3 on, `layers` of them, 64 of glyph 2: read
// whole, 64,065 glyphs, but drawing no point. The last glyph is a triangle whose COLR version 0 layers are those.
fn hollow(layers: u8) -> FontCache {
    use crate::daecore::daetype::TableBytes;
    let words = |w: &[i16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let composite = |gid: u8, n: usize| {
        let mut g = words(&[-1, 0, 0, 0, 0]);
        (0..n).for_each(|i| g.extend([0, if i + 1 < n { 0x22 } else { 0x02 }, 0, gid, 0, 0]));
        g
    };
    let triangle = [&words(&[1, 0, 0, 10, 10, 2, 0])[..], &[0x31, 0x33, 0x27, 10, 10, 10]].concat();
    let base = 3 + i16::from(layers);
    let glyphs = [vec![Vec::new(), Vec::new(), composite(1, 1000)], vec![composite(2, 64); layers.into()]].concat();
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in glyphs.iter().chain([&triangle]) {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(g);
    }
    loca.extend((glyf.len() as u32).to_be_bytes());
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let maxp = [&[0u8, 0, 0x50, 0][..], &(base as u16 + 1).to_be_bytes()].concat();
    let mut colr = words(&[0, 1, 0, 14, 0, 20, layers.into(), base, 0, layers.into()]);
    colr.extend((3..base).flat_map(|g| words(&[g, 0])));
    let cpal = [&[0u16, 1, 1, 1, 0, 14, 0].map(u16::to_be_bytes).concat()[..], &[0, 0, 0, 255]].concat();
    let tables = [("head", head), ("maxp", maxp), ("loca", loca), ("glyf", glyf), ("COLR", colr), ("CPAL", cpal)];
    FontCache::new(tables.into_iter().map(|(t, b)| (String::from(t), TableBytes::from(b))).collect()).expect("loads")
}

// Glyphs read whole cost a color glyph's extents what reading them took though they draw no point: one layer of
// 64,065 glyphs read measures, as a layer drawing nothing, and 17 pass a small font's 2^20, leaving the triangle's own.
#[test]
fn layers_drawing_no_point_cost_a_color_glyph_s_extents_what_reading_them_took() {
    assert_eq!(extents(&hollow(1), 4), Some((0, 0, 0, 0)));
    assert_eq!(extents(&hollow(17), 20), Some((0, 10, 10, -10)));
}
