use super::*;
use crate::daecore::daetype::subsetter::GlyphSet;
use core::cell::Cell;

// kern, liga and pref each name one SingleSubst of glyph 5, which the subset drops: kern and pref
// stay with no lookup, liga goes.
#[test]
fn features_that_steer_shaping_stay_without_lookups() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = [
        u16s(&[1, 0, 10, 34, 72, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 3, 0, 1, 2]),
        u16s(&[3]), b"kern".to_vec(), u16s(&[20]), b"liga".to_vec(), u16s(&[26]), b"pref".to_vec(), u16s(&[32]),
        u16s(&[0, 1, 0, 0, 1, 0, 0, 1, 0]),
        u16s(&[1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]),
    ].concat();
    let mut active = GlyphSet::new();
    [0, 1].into_iter().for_each(|g| { active.insert(g); });
    let out = super::super::gsub::subset_gsub(&gsub, &active, &[0, 1], &[], false).expect("a GSUB");
    let list = usize::from(read_u16_be(&out, 6).expect("FeatureList"));
    let count = usize::from(read_u16_be(&out, list).expect("count"));
    let tags: Vec<&[u8]> = (0..count).map(|i| &out[list + 2 + 6 * i..list + 6 + 6 * i]).collect();
    assert_eq!(tags, [&b"kern"[..], &b"pref"[..]]);
}

// A null lookup, feature and language system each hold nothing: lookup 0 stays hollow, the null liga goes for want
// of a lookup, and TRK keeps an empty language system.
#[test]
fn a_null_layout_record_is_empty() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = [
        u16s(&[1, 0, 10, 38, 58, 1]), b"DFLT".to_vec(), u16s(&[8, 10, 1]), b"TRK ".to_vec(), u16s(&[0]),
        u16s(&[0, 0xFFFF, 2, 0, 1]),
        u16s(&[2]), b"liga".to_vec(), u16s(&[0]), b"ccmp".to_vec(), u16s(&[14, 0, 1, 1]),
        u16s(&[2, 0, 6, 1, 0, 1, 8, 2, 8, 1, 2, 1, 1, 1]),
    ].concat();
    assert_eq!(lookups(&gsub).expect("a list").collect::<Vec<_>>(), [None, Some(64)]);

    let mut active = GlyphSet::new();
    [0, 1, 2].into_iter().for_each(|g| { active.insert(g); });
    let out = super::super::gsub::subset_gsub(&gsub, &active, &[0, 1, 2], &[], false).expect("a GSUB");
    let list = usize::from(read_u16_be(&out, 6).expect("FeatureList"));
    let count = usize::from(read_u16_be(&out, list).expect("count"));
    let tags: Vec<&[u8]> = (0..count).map(|i| &out[list + 2 + 6 * i..list + 6 + 6 * i]).collect();
    assert_eq!(tags, [&b"ccmp"[..]]);
    let scripts = usize::from(read_u16_be(&out, 4).expect("ScriptList"));
    let script = scripts + usize::from(read_u16_be(&out, scripts + 6).expect("DFLT"));
    let trk = script + usize::from(read_u16_be(&out, script + 8).expect("TRK"));
    assert_eq!(read_u16_be(&out, trk + 4), Some(0));
}

// DFLT's TRK names what its default does, so it goes, read through the default as the shaper reads a language the
// script lacks; XYZ names more and stays.
#[test]
fn a_language_system_alike_its_default_goes() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let gsub = [
        u16s(&[1, 0, 10, 60, 86, 1]), b"DFLT".to_vec(), u16s(&[8, 16, 2]), b"TRK ".to_vec(), u16s(&[24]),
        b"XYZ ".to_vec(), u16s(&[32, 0, 0xFFFF, 1, 0, 0, 0xFFFF, 1, 0, 0, 0xFFFF, 2, 0, 1, 2]), b"liga".to_vec(),
        u16s(&[14]), b"ccmp".to_vec(), u16s(&[20, 0, 1, 0, 0, 1, 0, 1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]),
    ].concat();
    let mut active = GlyphSet::new();
    (0..8).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..8).collect();
    let out = super::super::gsub::subset_gsub(&gsub, &active, &gid_map, &[], false).expect("a GSUB");
    let scripts = usize::from(read_u16_be(&out, 4).expect("ScriptList"));
    let script = scripts + usize::from(read_u16_be(&out, scripts + 6).expect("DFLT"));
    assert_eq!((read_u16_be(&out, script + 2), out.get(script + 4..script + 8)), (Some(1), Some(&b"XYZ "[..])));
}

// Ten lookups each naming one subtable: it is rebuilt at its first naming and kept at its
// second, and the rest reuse it, copied or written once.
#[test]
fn a_subtable_many_lookups_name_is_rebuilt_at_most_twice() {
    let n = 10u16;
    let lookup_at = |i: u16| 2 + 2 * n + 8 * i;
    let sub_at = lookup_at(n);
    let mut buf: Vec<u8> = [1u16, 0, 0, 0, 10, n].map(u16::to_be_bytes).concat();
    (0..n).for_each(|i| buf.extend(lookup_at(i).to_be_bytes()));
    for i in 0..n {
        buf.extend([1, 0, 1, sub_at - lookup_at(i)].iter().flat_map(|v: &u16| v.to_be_bytes()));
    }
    buf.extend([0; 4]);
    let calls = Cell::new(0);
    let subset: SubtableSubsetter = &|_, _, _, _, _| {
        calls.set(calls.get() + 1);
        Some(Parsed::Bytes(alloc::vec![0; 4]))
    };
    let (lookups, _) = read_lookups(&buf, 7, &[], subset, |_| None).expect("lookups");
    assert!(lookups.len() == 10 && lookups.iter().all(|l| l.subtables.len() == 1));
    assert!(calls.get() <= 2, "read {} times", calls.get());
    let (numbers, mut graph, writes) = ((0..n).collect::<Vec<u16>>(), Graph::default(), Cell::new(0));
    let shared = written(&lookups, &numbers, usize::MAX, |_| 0, |_| {
        writes.set(writes.get() + 1);
        Some(graph.add(alloc::vec![0; 4], Vec::new()))
    });
    let shared = shared.expect("lookups");
    assert!(shared.iter().all(|l| l.subtables == shared[0].subtables) && writes.get() <= 2);
    assert_eq!(graph.held(), 4, "the subtable is held more than once");
}

// `scripts` records naming one Script with no default, whose `langs` records name one system of `indices` entries,
// each naming the steering kern: each table is read once, but every naming's copy is held to the subset budget.
#[test]
fn script_and_language_records_naming_one_table_are_copied_within_the_budget() {
    let u16s = |w: &[usize]| w.iter().flat_map(|&v| (v as u16).to_be_bytes()).collect::<Vec<u8>>();
    let lists = |scripts: usize, langs: usize, indices: usize| {
        let (script, system) = (2 + 6 * scripts, 4 + 6 * langs);
        let features = 10 + script + system + 6 + 2 * indices;
        let mut buf = u16s(&[1, 0, 10, features, 0, scripts]);
        (0..scripts).for_each(|_| buf.extend([&b"DFLT"[..], &u16s(&[script])].concat()));
        buf.extend(u16s(&[0, langs]));
        (0..langs).for_each(|_| buf.extend([&b"ENG "[..], &u16s(&[system])].concat()));
        buf.extend(u16s(&[0, 0xFFFF, indices]));
        buf.extend(u16s(&alloc::vec![0; indices]));
        [buf, u16s(&[1]), b"kern".to_vec(), u16s(&[8, 0, 0])].concat()
    };
    let (_, scripts, _) = read_lists(&lists(64, 64, 64), (&[], &[], false), true).expect("within the budget");
    assert_eq!(scripts.len(), 64);
    assert!(scripts.iter().all(|s| s.langs.len() == 64 && s.langs.iter().all(|l| l.features == [0])));
    assert!(read_lists(&lists(256, 64, 64), (&[], &[], false), true).is_none());
}

// Lookup 0 keeps nothing and lookup 1 a glyph, so lookup 1 becomes 0; 'aaaa' names lookup 0 and 'bbbb' lookup 1.
// `scripts` ScriptRecords name one Script whose default and `langs` LangSysRecords all name one LangSys.
fn renumbered_gsub(scripts: usize, langs: usize) -> Vec<u8> {
    let u16s = |w: &[usize]| w.iter().flat_map(|&v| (v as u16).to_be_bytes()).collect::<Vec<u8>>();
    let (script, system) = (2 + 6 * scripts, 4 + 6 * langs);
    let script_list = [
        u16s(&[scripts]), [&b"latn"[..], &u16s(&[script])].concat().repeat(scripts),
        u16s(&[system, langs]), [&b"dflt"[..], &u16s(&[system])].concat().repeat(langs),
        u16s(&[0, 0xFFFF, 2, 0, 1]),
    ].concat();
    let features = [u16s(&[2]), b"aaaa".to_vec(), u16s(&[14]), b"bbbb".to_vec(), u16s(&[20, 0, 1, 0, 0, 1, 1])];
    let features = features.concat();
    let lookups = [u16s(&[2, 6, 26]), u16s(&[1, 0, 1, 8, 1, 6, 1, 1, 1, 5, 1, 0, 1, 8, 1, 6, 1, 1, 1, 1])].concat();
    let lists_at = [10, 10 + script_list.len(), 10 + script_list.len() + features.len()];
    [u16s(&[1, 0]), u16s(&lists_at), script_list, features, lookups].concat()
}

// The subset's features by tag, each with its lookups.
fn features_of(gsub: &[u8]) -> Vec<(Vec<u8>, Vec<u16>)> {
    let list = usize::from(read_u16_be(gsub, 6).expect("FeatureList"));
    let count = usize::from(read_u16_be(gsub, list).expect("count"));
    (0..count).map(|i| {
        let rec = list + 2 + 6 * i;
        let at = list + usize::from(read_u16_be(gsub, rec + 4).expect("a feature"));
        let n = usize::from(read_u16_be(gsub, at + 2).expect("its lookup count"));
        (gsub[rec..rec + 4].to_vec(), (0..n).filter_map(|k| read_u16_be(gsub, at + 4 + 2 * k)).collect())
    }).collect()
}

// 200 scripts sharing one Script of 200 systems sharing one LangSys read each table once, within the budget, so the
// renumbered 'bbbb' names lookup 0 and 'aaaa', naming nothing kept, goes, as with one script of one system.
#[test]
fn shared_scripts_and_systems_are_read_once_under_renumbered_lookups() {
    let active: GlyphSet = (0..3).collect();
    for (scripts, langs) in [(1, 1), (200, 200)] {
        let out = super::super::gsub::subset_gsub(&renumbered_gsub(scripts, langs), &active, &[0, 1, 2], &[], false)
            .expect("a subset GSUB");
        assert_eq!(features_of(&out), [(b"bbbb".to_vec(), alloc::vec![0])], "{scripts} x {langs}");
    }
}

// A LangSysRecord naming a LangSys past the table's end: the lists can't be rebuilt, and the source's would name
// lookup 1, which is now 0, so the table goes rather than apply the wrong lookups.
#[test]
fn lists_that_cannot_be_rebuilt_under_renumbered_lookups_refuse_the_table() {
    let mut gsub = renumbered_gsub(1, 1);
    let record = 10 + 2 + 6 + 4 + 4;
    gsub[record..record + 2].copy_from_slice(&0xFFF0u16.to_be_bytes());
    let active: GlyphSet = (0..3).collect();
    assert_eq!(super::super::gsub::subset_gsub(&gsub, &active, &[0, 1, 2], &[], false), None);
}

// 2,000 entries naming one lookup whose subtable rebuilds to 4 KB: copied per entry, 8 MB from 8 KB of table, past
// the budget, so the copying writer refuses it; the graph holds the subtable once.
#[test]
fn copies_past_the_budget_are_refused() {
    let n = 2_000u16;
    let mut buf: Vec<u8> = [1u16, 0, 0, 0, 10, n].map(u16::to_be_bytes).concat();
    (0..n).for_each(|_| buf.extend((2 + 2 * n).to_be_bytes()));
    buf.extend([1u16, 0, 1, 8, 0].map(u16::to_be_bytes).concat());
    let subset: SubtableSubsetter = &|_, _, _, _, _| Some(Parsed::Bytes(alloc::vec![1; 4096]));
    let (lookups, _) = read_lookups(&buf, 7, &[], subset, |_| None).expect("lookups");
    let numbers: Vec<u16> = (0..n).collect();
    let budget = crate::daecore::daetype::subsetter::subset_budget(buf.len());
    let bytes = |parsed: &Parsed| match parsed {
        Parsed::Bytes(bytes) => Some(bytes.clone()),
        Parsed::Value(_) => None,
    };
    assert!(written(&lookups, &numbers, budget, Vec::len, bytes).is_none());
    let mut graph = Graph::default();
    assert!(written(&lookups, &numbers, budget, |_| 0, |p| Some(graph.add(bytes(p)?, Vec::new()))).is_some());
    assert_eq!(graph.held(), 4096);
}

// A GSUB 1.2 of one script, one feature and one SingleSubst of glyph 1, its lists at the 32-bit fields or the
// 16-bit ones.
fn gsub_1_2(long: bool) -> Vec<u8> {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let mut head = [u16s(&[1, 2, 0, 0, 0]), alloc::vec![0; 16]].concat();
    for (k, list) in [26u32, 46, 60].into_iter().enumerate() {
        match long {
            true => head[14 + 4 * k..18 + 4 * k].copy_from_slice(&list.to_be_bytes()),
            false => head[4 + 2 * k..6 + 2 * k].copy_from_slice(&(list as u16).to_be_bytes()),
        }
    }
    let lookups = match long {
        true => [u16s(&[1, 0, 6]), u16s(&[1, 0, 1, 8, 1, 6, 10, 1, 1, 1])].concat(),
        false => u16s(&[1, 4, 1, 0, 1, 8, 1, 6, 10, 1, 1, 1]),
    };
    [head, u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]), u16s(&[1]), b"ccmp".to_vec(),
        u16s(&[8, 0, 1, 0]), lookups].concat()
}

// 1.2 is written through its 32-bit fields alone, the 16-bit ones 0, from either; the shaper reads the
// lookup back. With no lookup left the rebuilt lists stay 1.2.
#[test]
fn a_version_1_2_table_is_written_through_its_32_bit_lists() {
    use crate::daecore::daeshaper::ot::LayoutTable;
    let kept = |glyphs: &mut dyn Iterator<Item = u32>| {
        let mut active = GlyphSet::new();
        glyphs.for_each(|g| { active.insert(g); });
        active
    };
    let (active, gid_map) = (kept(&mut (0..12)), (0..12).collect::<Vec<u32>>());
    let out = super::super::gsub::subset_gsub(&gsub_1_2(false), &active, &gid_map, &[], false).expect("a GSUB");
    assert_eq!(super::super::gsub::subset_gsub(&gsub_1_2(true), &active, &gid_map, &[], false).as_ref(), Some(&out));
    assert_eq!((read_u32_be(&out, 0), &out[4..10]), (Some(0x0001_0002), &[0; 6][..]));
    let (lists, width) = LayoutTable::parse(&out).expect("a layout").lists();
    assert!(lists.iter().all(|&list| list != 0) && width == 4, "{lists:?} {width}");
    let lookup = LayoutTable::parse(&out).and_then(|t| t.lookup(0)).expect("lookup 0");
    assert_eq!((lookup.kind, lookup.subtable_count), (1, 1));

    let active = kept(&mut (0..12).filter(|&g| g != 1));
    let out = super::super::gsub::subset_gsub(&gsub_1_2(true), &active, &gid_map, &[], false).expect("a GSUB");
    assert_eq!((read_u32_be(&out, 0), &out[4..10]), (Some(0x0001_0002), &[0; 6][..]));
    let (lists, width) = LayoutTable::parse(&out).expect("a layout").lists();
    assert!(lists.iter().all(|&list| list >= 26) && width == 4, "{lists:?} {width}");
}

// 1.2's lists may lie past 64 KB: one whose lists start 70,000 bytes in keeps them, every feature kept, without the
// bytes before them. A header cut short of its version's length is no table.
#[test]
fn a_version_1_2_table_s_lists_may_lie_past_16_bits() {
    use crate::daecore::daeshaper::ot::LayoutTable;
    let near = gsub_1_2(true);
    let mut far = near[..26].to_vec();
    for at in [14, 18, 22] {
        let list = read_u32_be(&far, at).expect("a list") + 70_000;
        far[at..at + 4].copy_from_slice(&list.to_be_bytes());
    }
    far.extend(alloc::vec![0; 70_000]);
    far.extend(&near[26..]);
    let mut active = GlyphSet::new();
    (0..12).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..12).collect();
    let out = super::super::gsub::subset_gsub(&far, &active, &gid_map, &[], false).expect("a GSUB");
    let lookup = LayoutTable::parse(&out).and_then(|t| t.lookup(0)).expect("lookup 0");
    let features = LayoutTable::parse(&out).and_then(|t| read_u16_be(&out, t.lists().0[1]));
    assert_eq!((lookup.kind, features), (1, Some(1)));
    assert!(out.len() < 1_000, "{} bytes", out.len());

    let cut = [0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(super::super::gsub::subset_gsub(&cut, &active, &gid_map, &[], false), None);
}

// With none promoted the third of a lookup's three 40,000-byte subtables is out of reach, so every lookup goes
// behind Extension records, the small one too.
#[test]
fn a_list_that_overflows_promotes_every_lookup() {
    let mut graph = Graph::default();
    let small = graph.add(alloc::vec![1; 10], Vec::new());
    let large: Vec<usize> = (0..3).map(|i| graph.add(alloc::vec![i; 40_000], Vec::new())).collect();
    let lookup = |subtables: Vec<usize>| {
        RebuiltLookup { real_type: 1, flag: 0, mark_filtering_set: None, subtables }
    };
    let list = shared_list(&[lookup(alloc::vec![small]), lookup(large)], graph, 7, false, |_| true).expect("a list");
    let kind = |i: usize| read_u16_be(&list, usize::from(read_u16_be(&list, 2 + 2 * i).expect("a lookup")));
    assert_eq!([kind(0), kind(1)], [Some(7), Some(7)]);
}

// From 1.2 only a lookup whose own subtables pass 16-bit reach is promoted to Extension, its subtables found.
#[test]
fn a_long_lookup_list_promotes_only_the_lookups_that_need_it() {
    let lookup = |first: usize| RebuiltLookup {
        real_type: 1,
        flag: 0,
        mark_filtering_set: None,
        subtables: alloc::vec![alloc::vec![0; first], alloc::vec![9; 4]],
    };
    let list = assemble_long(&[lookup(70_000), lookup(4)], 7).expect("a list");
    let at = |i: usize| read_u32_be(&list, 2 + 4 * i).expect("a lookup") as usize;
    assert_eq!([read_u16_be(&list, at(0)), read_u16_be(&list, at(1))], [Some(7), Some(1)]);
    let record = at(0) + usize::from(read_u16_be(&list, at(0) + 8).expect("a second subtable"));
    let second = record + read_u32_be(&list, record + 4).expect("its offset") as usize;
    assert_eq!((read_u16_be(&list, record + 2), &list[second..second + 4]), (Some(1), &[9; 4][..]));
}

// Four lookups starting two bytes apart over one run of 0xFFFF words name 65,535 subtables each, four
// times what lookups laid apart could. 16,000 in 195 KB held a subset for 3 s. Each walker stops at the budget.
#[test]
fn lookups_crafted_to_overlap_stop_at_the_offset_budget() {
    let n = 4usize;
    let (list, first) = (10usize, 12 + 2 * n);
    let mut t = alloc::vec![0u8; first];
    t[0..2].copy_from_slice(&1u16.to_be_bytes());
    t[8..10].copy_from_slice(&(list as u16).to_be_bytes());
    t[list..list + 2].copy_from_slice(&(n as u16).to_be_bytes());
    for k in 0..n {
        t[list + 2 + 2 * k..list + 4 + 2 * k].copy_from_slice(&((first - list + 2 * k) as u16).to_be_bytes());
    }
    t.extend(alloc::vec![0xFF; 2 * (n + 65_536)]);
    let mut active = GlyphSet::new();
    active.insert(0);
    assert!(super::super::gsub::subset_gsub(&t, &active, &[0], &[], false).is_none(), "the subset kept the GSUB");
    assert!(super::super::gpos::instance_gpos(&t, &[], &|_, _| 0).is_none(), "the instance kept walking");
}

// 200 FeatureRecords naming one Feature of 65,535 lookup indices read it once, within the budget its 131 KB give;
// eight naming Features that start two bytes apart over 0xFFFF words read 524,280 indices, past it.
#[test]
fn a_feature_many_records_name_is_numbered_once_within_the_budget() {
    let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let table = |rels: &[u16], features: Vec<u8>| {
        let records = rels.iter().flat_map(|&rel| [b"test".to_vec(), u16s(&[rel])].concat());
        [u16s(&[1, 0, 0, 10, 0, rels.len() as u16]), records.collect(), features].concat()
    };
    let n = 200;
    let shared = table(&alloc::vec![2 + 6 * n; n.into()], [u16s(&[0, 0xFFFF]), alloc::vec![0; 2 * 0xFFFF]].concat());
    assert_eq!(numbering(&shared, &[], false), Some(Vec::new()));
    let starts: Vec<u16> = (0..8).map(|k| 2 + 6 * 8 + 2 * k).collect();
    let overlapping = table(&starts, alloc::vec![0xFF; 2 * (0xFFFF + 8) + 4]);
    assert_eq!(numbering(&overlapping, &[], false), None);
}
