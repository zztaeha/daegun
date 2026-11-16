use super::*;
use crate::daecore::daetype::format::class_def::classes;
use alloc::{format, vec::Vec};

impl Coverage<'_> {
    // The list `each_entry` replaced, kept to check it against.
    pub(crate) fn index_entries(&self) -> Option<alloc::vec::Vec<(u32, u16)>> {
        let d = self.data;
        let mut out = alloc::vec::Vec::new();
        match read_u16_be(d, 0)? {
            1 => {
                let count = read_u16_be(d, 2)? as usize;
                for i in 0..count {
                    out.push((u32::from(read_u16_be(d, 4 + i * 2)?), u16::try_from(i).ok()?));
                }
            }
            2 => {
                let count = read_u16_be(d, 2)? as usize;
                for i in 0..count {
                    let r = window::<6>(d, 4 + i * 6)?;
                    let (start, end, first) = (
                        u16::from_be_bytes([r[0], r[1]]),
                        u16::from_be_bytes([r[2], r[3]]),
                        u16::from_be_bytes([r[4], r[5]]),
                    );
                    if start > end || out.len().saturating_add(usize::from(end - start) + 1) > 65_536 {
                        return None;
                    }
                    for g in start..=end {
                        out.push((u32::from(g), first.checked_add(g - start)?));
                    }
                }
            }
            _ => return None,
        }
        if out.windows(2).any(|w| w[0].0 >= w[1].0) || out.iter().any(|&(_, v)| v == COVERAGE_ABSENT) {
            return None;
        }
        Some(out)
    }
}

fn same(what: &str, each: &IndexEntries, absent: u16, old: Option<Vec<(u32, u16)>>) {
    let old = old.unwrap_or_default();
    let full = SparseIndex::from_sorted(&old, absent, usize::MAX);
    let budgets = [usize::MAX, full.as_ref().map_or(0, |i| i.bytes().saturating_sub(1)), 1_100];
    for budget in budgets {
        let want = SparseIndex::from_sorted(&old, absent, budget);
        assert!(SparseIndex::build(each, absent, budget) == want, "{what} at a budget of {budget}");
    }
}

fn old_pair_seconds(data: &[u8]) -> Vec<(u32, u16)> {
    let record_len = |at: usize| value_record_len(read_u16_be(data, at).unwrap_or(0));
    let stride = 2 + record_len(4) + record_len(6);
    let mut seconds: Vec<u32> = tables(data, 8)
        .flat_map(|set| {
            let count = usize::from(read_u16_be(set, 0).unwrap_or(0));
            (0..count).map_while(move |i| read_u16_be(set, 2 + i * stride))
        })
        .map(u32::from)
        .collect();
    seconds.sort_unstable();
    seconds.dedup();
    seconds.into_iter().map(|glyph| (glyph, 1)).collect()
}

fn old_classes(data: &[u8]) -> Option<Vec<(u32, u16)>> {
    classes(data)?.into_iter().map(|(g, c)| Some((g, u16::try_from(c).ok()?))).collect()
}

// Every coverage, class definition, pair set, GDEF table and cmap behind these fonts' shaping indexes, built from
// the font, against the index its old list built, at budgets that page it or refuse it.
#[test]
fn indexes_built_from_the_font_match_those_built_from_its_lists() {
    let fonts = [
        "inter/InterVariable.ttf", "scheherazade-new/ScheherazadeNew-Regular.ttf",
        "noto-devanagari/NotoSansDevanagari.ttf", "noto-khmer/NotoSansKhmer.ttf",
        "noto-myanmar/NotoSansMyanmar.ttf", "eb-garamond/EBGaramond.ttf",
        "source-han-sans/SourceHanSansJP-VF.otf", "stix-two-math/STIX2Math.otf",
    ];
    let mut checked = 0;
    for rel in fonts {
        let bytes = std::fs::read(format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).unwrap();
        for (tag, kinds) in [("GSUB", crate::daecore::daeshaper::ot::gsub::KINDS),
                             ("GPOS", crate::daecore::daeshaper::ot::gpos::KINDS)] {
            let Some(table) = map.get(tag).and_then(|t| LayoutTable::parse(t)) else { continue };
            for index in 0..table.lookup_count() {
                let Some(lookup) = table.lookup(index) else { continue };
                for i in 0..lookup.subtable_count {
                    let resolved = resolve_extension(lookup.subtable(i), lookup.kind, kinds.extension);
                    let Some((kind, data)) = resolved else { continue };
                    let what = format!("{rel} {tag} lookup {index} subtable {i}");
                    if let Some(at) = first_coverage_at(kind, data, kinds.pair_pos, kinds.context,
                                                        kinds.chain_context) {
                        let c = Coverage::at(data, at);
                        same(&what, &|e| c.each_entry(e), COVERAGE_ABSENT, c.index_entries());
                    }
                    if kinds.pair_pos == Some(kind) && read_u16_be(data, 0) == Some(1) {
                        let seconds = pair_seconds(data, &mut { usize::MAX }).expect("within the work");
                        let each = |e: &mut Entries| {
                            seconds.runs().for_each(|(a, b)| e.put_run(a, b, 1, 0));
                            Some(())
                        };
                        same(&what, &each, 0, Some(old_pair_seconds(data)));
                    }
                    let (ats, class) = rule_tables(kind, data, &kinds);
                    let pair_classes = if kinds.pair_pos == Some(kind) && read_u16_be(data, 0) == Some(2) {
                        &[8, 10][..]
                    } else {
                        &[]
                    };
                    let pairs = pair_classes.iter().map(|&at| (at, true));
                    for (at, class) in ats.into_iter().map(|at| (at, class)).chain(pairs) {
                        let t = table_at(data, at);
                        if class {
                            same(&what, &|e| ClassDef::new(t).each_entry(e), 0, old_classes(t));
                        } else {
                            let c = Coverage::new(t);
                            same(&what, &|e| c.each_entry(e), COVERAGE_ABSENT, c.index_entries());
                        }
                    }
                    checked += 1;
                }
            }
        }
        if let Some(gdef) = map.get("GDEF").and_then(|t| Gdef::new(t)) {
            for c in [&gdef.glyph_classes, &gdef.mark_attach_classes].into_iter().flatten() {
                same(&format!("{rel} GDEF"), &|e| c.each_entry(e), 0, old_classes(c.data));
            }
            for set in (0..gdef.mark_glyph_set_count()).filter_map(|i| gdef.mark_set(i)) {
                same(&format!("{rel} GDEF set"), &|e| set.each_entry(e), COVERAGE_ABSENT, set.index_entries());
            }
        }
        let cmap = map.get("cmap").unwrap();
        let lookup = crate::daecore::daetype::subsetter::CmapLookup::new(cmap);
        let entries = lookup.entries(cmap).into_iter().map(|(c, g)| (c, g as u16)).collect();
        same(&format!("{rel} cmap"), &|e| { lookup.each_entry(cmap, e); Some(()) }, 0, Some(entries));
    }
    assert!(checked > 1_700, "{checked} subtables");
}

// Coverages the old list refused or kept, and class definitions in and out of order, built the same way.
#[test]
fn edge_case_coverages_and_classes_build_as_their_lists_did() {
    let be = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let coverages = [
        be(&[1, 3, 4, 9, 20]), be(&[1, 3, 4, 20, 9]), be(&[1, 3, 4, 4, 9]), be(&[1, 0]), be(&[1, 5, 1]),
        be(&[2, 2, 1, 5, 0, 9, 12, 5]), be(&[2, 2, 1, 5, 0, 5, 12, 5]), be(&[2, 2, 9, 12, 0, 1, 5, 4]),
        be(&[2, 1, 7, 3, 0]), be(&[2, 1, 10, 20, 0xFFF0]), be(&[2, 1, 10, 20, 0xFFF4]), be(&[2, 1, 10, 20, 0xFFF5]),
        be(&[2, 1, 0, 0xFFFF, 0]), be(&[3, 1, 2, 3]), be(&[2]),
    ];
    for (k, t) in coverages.iter().enumerate() {
        let c = Coverage::new(t);
        same(&format!("coverage {k}"), &|e| c.each_entry(e), COVERAGE_ABSENT, c.index_entries());
    }
    let classes = [
        be(&[1, 4, 3, 1, 0, 2]), be(&[1, 0xFFFE, 3, 1, 2, 3]), be(&[2, 2, 1, 5, 1, 7, 9, 2]),
        be(&[2, 3, 30, 40, 1, 5, 8, 1, 10, 20, 3]), be(&[2, 2, 1, 5, 1, 9, 3, 2]), be(&[2, 2, 1, 5, 0, 6, 9, 3]),
        be(&[2, 9, 1, 2, 4]), be(&[1]),
    ];
    for (k, t) in classes.iter().enumerate() {
        same(&format!("classes {k}"), &|e| ClassDef::new(t).each_entry(e), 0, old_classes(t));
    }
}

// A lookup whose 32,000 subtables share one coverage of 32,767 entries would read a billion entries to index. The
// first subtables index within the table's work; those past it pass every glyph, so shaping reads them itself.
#[test]
fn subtables_past_a_tables_index_work_pass_every_glyph() {
    let (subtables, entries) = (32_000u16, 32_767u16);
    let subtable_at = 6 + 2 * subtables;
    let mut words = alloc::vec![1, 0, 0, 0, 10, 1, 4, 1, 0, subtables];
    words.extend((0..subtables).map(|_| subtable_at));
    words.extend([1, 6, 1, 1, entries]);
    words.extend((0..entries).map(|_| 5));
    let gsub: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let t = LayoutTable::parse(&gsub).expect("a GSUB");
    let kinds = crate::daecore::daeshaper::ot::gsub::KINDS;
    let mut work = index_work(gsub.len());
    let lookup = t.lookup(0).expect("lookup 0");
    let index = lookup_index(&lookup, &kinds, false, |_, _| None, |_, _| None, |_| true, &mut work);
    assert_eq!(work, 0);
    assert_eq!(index.subtables.len(), usize::from(subtables));
    assert!(index.subtables[0].digest.may_have(5) && !index.subtables[0].digest.may_have(6));
    assert_eq!(index.subtables.last().map(|s| s.digest), Some(digest::Digest::full()));
}

// A chained format 1 subtable of three sets: one of an input-first rule, a lookahead-first rule and a null rule, an
// empty one, and one stating five rules in bytes that hold two offsets. The plan keeps each rule's first read as the
// font gives it, a set's as far as its bytes hold offsets; without the bytes for them, the digests alone.
#[test]
fn a_rule_sets_first_reads_are_its_rules_own() {
    let words = [1, 0, 3, 12, 42, 44, 3, 8, 18, 0, 0, 2, 4, 0, 0, 1, 7, 1, 1, 9, 0, 0, 5, 4u16];
    let data: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let from_font = |set: usize, rules: usize| -> Vec<u32> {
        let set = tables(&data, 4).nth(set).expect("the set");
        tables(set, 0).take(rules).map(|rule| apply::FirstRead::pack(apply::first_read::<2>(rule, true))).collect()
    };
    let sets = rule_sets(&data, true, &mut |_| true, &mut { usize::MAX }).expect("its rule sets");
    assert_eq!(sets.firsts(0), [4, 1 << 16 | 9, 1 << 17]);
    assert_eq!([0, 1, 2].map(|set| sets.firsts(set).to_vec()), [from_font(0, 3), from_font(1, 0), from_font(2, 2)]);
    assert_eq!((sets.firsts(3), sets.digest(3)), (&[][..], u64::MAX));
    assert_eq!([sets.digest(0), sets.digest(1)], [u64::MAX; 2]);

    let mut asked = 0;
    let mut digests_only = |_: usize| {
        asked += 1;
        asked == 1
    };
    let bare = rule_sets(&data, true, &mut digests_only, &mut { usize::MAX }).expect("its digests");
    let kept = [0, 1, 2].map(|set| (bare.firsts(set).len(), bare.digest(set)));
    assert_eq!(kept, [0, 1, 2].map(|set| (0, sets.digest(set))));
}

// A chained format 1 subtable of two sets of 600 rules, all naming one, with work for the first set alone: it keeps
// no index and draws nothing from the face's allowance.
#[test]
fn rule_sets_past_their_work_draw_nothing() {
    let (rules, set_len) = (600, 1_202);
    let set = |at: u16| [vec![rules], vec![at; usize::from(rules)]].concat();
    let words = [&[1, 0, 2, 10, 10 + set_len][..], &set(2 * set_len), &set(set_len), &[0, 1, 0, 0]].concat();
    let data: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut drawn = 0;
    let mut draw = |bytes| {
        drawn += bytes;
        true
    };
    assert!(rule_sets(&data, true, &mut draw, &mut { 1_000 }).is_none());
    assert_eq!(drawn, 0);
}

// A chained format 1 subtable whose 16,385 set offsets all name one set of 65,535 rules: 4.3 GB of first reads, more
// than a 32-bit size counts, so the allowance is never asked for less and only the digests are kept.
#[test]
#[ignore = "reads a billion rules, and tells only on a 32-bit target: run it there with --release"]
fn rule_sets_past_a_32_bit_size_ask_no_less_of_the_allowance() {
    let sets = 16_385u16;
    let set = [&[u16::MAX][..], &alloc::vec![0; usize::from(u16::MAX)]].concat();
    let words = [&[1, 0, sets][..], &alloc::vec![6 + 2 * sets; usize::from(sets)], &set].concat();
    let data: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let firsts = (u64::from(sets) * (u64::from(u16::MAX) + 1) + 1) * 4;
    let mut asked = Vec::new();
    let mut reserve = |bytes: usize| {
        asked.push(bytes);
        bytes <= 25_000_000
    };
    let built = rule_sets(&data, true, &mut reserve, &mut { usize::MAX }).expect("its digests");
    assert!(built.firsts(0).is_empty(), "first reads kept");
    assert!(asked[1..].iter().all(|&bytes| bytes as u64 == firsts), "asked for {asked:?} of {firsts}");
}
