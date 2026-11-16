use super::*;

// One contextual subtable whose single entry names index 999, so 1,000 substitution offsets, all of
// them naming one lookup of glyphs 1 to 1,000.
fn morx() -> Vec<u8> {
    const N: u32 = 1_000;
    let mut body = [3u32, 20, 20, 20, 28].map(u32::to_be_bytes).concat();
    body.extend([0u16, 0, 999, NO_INDEX].map(u16::to_be_bytes).concat());
    (0..N).for_each(|_| body.extend((4 * N).to_be_bytes()));
    body.extend([8u16, 1, N as u16].map(u16::to_be_bytes).concat());
    body.extend((1..=N as u16).flat_map(|g| (g + 1_000).to_be_bytes()));
    let mut t = [2u16, 0].map(u16::to_be_bytes).concat();
    t.extend(1u32.to_be_bytes());
    t.extend([1u32, (16 + 12 + body.len()) as u32, 0, 1].map(u32::to_be_bytes).concat());
    t.extend([(12 + body.len()) as u32, 1, 1].map(u32::to_be_bytes).concat());
    t.extend(body);
    t
}

#[test]
fn a_shared_contextual_lookup_is_closed_over_once() {
    let mut active = GlyphSet::new();
    active.insert(1);
    let found = morx_closure(&morx(), &active, 3_000);
    assert_eq!(found, [1_001], "glyph 1's one substitute, named 1,000 times, found {} times", found.len());
}

#[test]
fn a_shared_contextual_lookup_is_written_once() {
    let t = morx();
    let mut active = GlyphSet::new();
    (0..=2_000).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..=2_000).collect();
    let out = subset_morx(&t, &active, &gid_map, 3_000);
    let len = out.as_ref().map_or(0, Vec::len);
    assert!(out.is_some() && len <= 2 * t.len(), "a {}-byte morx subset to {len} bytes", t.len());
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// A version 2 morx of one chain declaring `declared` subtables and holding these (type, body).
fn chain_of(declared: u32, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let body: Vec<u8> = subtables.iter()
        .flat_map(|(kind, b)| [[(12 + b.len()) as u32, *kind, 1].map(u32::to_be_bytes).concat(), b.clone()].concat())
        .collect();
    [u16s(&[2, 0]), [1u32, 1, (16 + body.len()) as u32, 0, declared].map(u32::to_be_bytes).concat(), body].concat()
}

// Format 2: one segment of glyphs `first..=last` with one value.
fn segment(first: u16, last: u16, value: u16) -> Vec<u8> {
    u16s(&[2, 6, 2, 12, 1, 0, last, first, value, 0xFFFF, 0xFFFF, 0])
}

// A ligature subtable: after glyph 1 (class 4) then 3 (class 5), an action pops 3 at offset 0 and 1 at offset 1,
// forming glyph 4.
fn one_then_three() -> Vec<u8> {
    let classes = u16s(&[8, 1, 3, 4, 1, 5]);
    let rows = u16s(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 2]);
    let entries = u16s(&[0, 0, 0, 2, SET_COMPONENT, 0, 0, SET_COMPONENT | PERFORM_ACTION, 0]);
    let actions = [0u32, LIG_LAST | 1].map(u32::to_be_bytes).concat();
    let (components, ligatures) = (u16s(&[0, 0, 0, 1]), u16s(&[0, 4]));
    let parts = [classes, rows, entries, actions, components, ligatures];
    let mut at = LIGATURE_HEADER as u32;
    let mut body = 6u32.to_be_bytes().to_vec();
    for part in &parts {
        body.extend(at.to_be_bytes());
        at += part.len() as u32;
    }
    body.extend(parts.concat());
    body
}

// Renumbered 3 to 2, 1's slot lands on 3's, so its range moves to the lowest free slot, 0; each offset places only
// the glyph it pops.
#[test]
fn a_colliding_range_takes_the_lowest_free_slots() {
    let mut active = GlyphSet::new();
    [0, 1, 3, 4].into_iter().for_each(|g| { active.insert(g); });
    let out = subset_morx(&chain_of(1, &[(2, one_then_three())]), &active, &[0, 1, 0, 2, 3], 5).expect("a subset morx");
    let body = &out[8 + 16 + 12..];
    let part = |i: usize| read_u32_be(body, 4 + 4 * i).map_or(0, |v| v as usize);
    assert_eq!(body.get(part(3)..part(4)), Some(&[0u32, LIG_LAST | LIG_OFFSET].map(u32::to_be_bytes).concat()[..]));
    assert_eq!(body.get(part(4)..part(5)), Some(&u16s(&[0, 0, 1])[..]));
}

// A set read again grown: the noncontextual subtable reads the glyph new since, the ligature subtable is walked
// again for the glyph it pushes that arrived, and the stackings are a fresh read's of the grown set.
#[test]
fn a_grown_set_reaches_what_a_fresh_read_does() {
    let t = chain_of(2, &[(4, segment(5, 5, 6)), (2, one_then_three())]);
    let set = |glyphs: &[u32]| {
        let mut set = GlyphSet::new();
        glyphs.iter().for_each(|&g| _ = set.insert(g));
        set
    };
    let (mut walked, mut left) = (Stackings::new(&t), usize::MAX);
    assert_eq!(walked.reach(&set(&[0, 1]), 7, &mut left), Some(Vec::new()));
    let grown = set(&[0, 1, 3, 5]);
    assert_eq!(walked.reach(&grown, 7, &mut left), Some(alloc::vec![4, 6]));
    let mut fresh = Stackings::new(&t);
    fresh.reach(&grown, 7, &mut left);
    assert!(walked.active == grown && walked.popped == fresh.popped, "the stackings aren't the grown set's");
    let popped = walked.popped.get(&(0, 1)).and_then(|p| match p {
        Popped::Each(each) => Some(each.values().cloned().collect::<Vec<_>>()),
        Popped::Any(_) => None,
    });
    assert_eq!(popped, Some(alloc::vec![[3].into(), [1].into()]), "the subset can't read the ligature's stacking");
}

fn everything(n: u16) -> (GlyphSet, Vec<u32>) {
    let mut active = GlyphSet::new();
    (0..n).for_each(|g| { active.insert(g.into()); });
    (active, (0..n).map(u32::from).collect())
}

// Noncontextual subtables each mapping glyphs 1 to 65,533 in one 18-byte segment. Rebuilt at four
// bytes a glyph, 100 of them would come to 26 MB; each stays one segment.
#[test]
fn a_segment_stays_a_segment() {
    let t = chain_of(20, &(0..20).map(|_| (4, segment(1, 65_533, 7))).collect::<Vec<_>>());
    let (active, gid_map) = everything(u16::MAX);
    let out = subset_morx(&t, &active, &gid_map, u16::MAX.into()).expect("a subset morx");
    assert!(out.len() <= t.len() + 400, "{} bytes of morx became {}", t.len(), out.len());
}

// A chain naming one subtable more than it holds keeps the ones it holds, as the shaper runs them.
#[test]
fn a_missing_last_subtable_keeps_the_ones_before() {
    let t = chain_of(2, &[(4, segment(1, 3, 2))]);
    let (active, gid_map) = everything(4);
    let out = subset_morx(&t, &active, &gid_map, 4).expect("the readable subtable stays");
    assert_eq!(read_u32_be(&out, 20), Some(1));
}

// 2,000 ligature actions at two offsets over 2,000 glyphs, each action its own range, would be 8 MB:
// actions share their offset's range, which an identity subset places over the source's.
#[test]
fn actions_sharing_an_offset_share_their_components() {
    const N: u16 = 2_000;
    let row: &[u16] = &[0, 0, 0, 0, 0];
    let classes = segment(1, N, 4);
    let class_at = 28;
    let state_at = class_at + classes.len();
    let entry_at = state_at + 2 * row.len() * 2;
    let action_at = entry_at + 6;
    let component_at = action_at + 4 * usize::from(N);
    let ligature_at = component_at + 2 * usize::from(N);
    let mut body = [
        5u32, class_at as u32, state_at as u32, entry_at as u32, action_at as u32, component_at as u32,
        ligature_at as u32,
    ]
    .map(u32::to_be_bytes)
    .concat();
    body.extend(classes);
    body.extend(u16s(row));
    body.extend(u16s(row));
    body.extend(u16s(&[0, 0, 0]));
    (0..N).for_each(|i| body.extend((0xBFFF_FFFFu32 - u32::from(i % 2)).to_be_bytes()));
    (0..N).for_each(|g| body.extend(g.to_be_bytes()));
    (0..N).for_each(|g| body.extend((g + 1).to_be_bytes()));
    let t = chain_of(1, &[(2, body)]);
    let (mut active, mut gid_map) = everything(N + 1);
    let out = subset_morx(&t, &active, &gid_map, (N + 1).into()).expect("a subset morx");
    assert_eq!(read_u32_be(&out, 20), Some(1), "the ligature subtable was dropped");
    assert!(out.len() <= t.len() + 64, "{} bytes of morx became {}", t.len(), out.len());

    // Dropping glyph 1,000 moves the second offset's values onto the first's, so it takes one new
    // range, not one per action.
    active = GlyphSet::new();
    (0..=N).filter(|&g| g != 1_000).for_each(|g| { active.insert(g.into()); });
    gid_map.iter_mut().skip(1_000).for_each(|g| *g = g.saturating_sub(1));
    let out = subset_morx(&t, &active, &gid_map, (N + 1).into()).expect("a subset morx");
    assert_eq!(read_u32_be(&out, 20), Some(1), "the ligature subtable was dropped");
    assert!(out.len() <= t.len() + 4 * usize::from(N) + 64, "{} bytes of morx became {}", t.len(), out.len());
}

// Glyph 1,000, new 1, reaching each action's component at its own offset, each a distinct value, so every placement
// searches past each slot placed before it: 8 million probes from an 8 KB array. They stop at the work budget.
#[test]
fn placements_searching_past_every_slot_stop_at_the_work_budget() {
    const K: usize = 4_000;
    let body: Vec<u8> = (1..=K as u16).flat_map(u16::to_be_bytes).collect();
    let layout = LigatureLayout {
        starts: Vec::new(), action: 0, component: 0, ligature: 0, n_actions: 0, n_components: K, n_ligatures: 0,
    };
    let mut work = crate::daecore::daetype::subsetter::subset_budget(body.len()) * 64;
    let mut components = Vec::new();
    let placed = (0..K)
        .take_while(|&i| place_components(&mut components, &body, &layout, &[(1_000, 1)], i as i64 - 1_000, &mut work)
            .is_some())
        .count();
    assert!((1_000..K).contains(&placed), "{placed} placed");
}

// Four noncontextual subtables each taking glyphs 0 to 65,534 to glyph 1, in a font of 2 glyphs: a pass finds glyph 1
// once, not once a pair, and its 262,144 reads, each lookup's unit and glyphs, spend the closure's budget, so a budget
// short of them refuses it.
#[test]
fn noncontextual_substitutes_are_found_once_and_spend_the_budget() {
    let t = chain_of(4, &alloc::vec![(4, segment(0, 0xFFFE, 1)); 4]);
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let found = morx_closure(&t, &active, 2);
    assert!(found == [1], "glyph 1 found {} times", found.len());
    let mut left = 262_144;
    assert!(Stackings::new(&t).reach(&active, 2, &mut left).is_some() && left == 0, "{left} left");
    let mut left = 262_143;
    assert_eq!(Stackings::new(&t).reach(&active, 2, &mut left), None);
}

// A ligature subtable of 30,000 classes whose class lookup gives glyphs 1 to 29,996 one each: one pass over the
// glyphs gathers every class's, where a pass for each class would read 900 million.
#[test]
fn a_ligature_subtables_classes_gather_their_glyphs_in_one_pass() {
    const CLASSES: u16 = 30_000;
    let mut classes = u16s(&[8, 1, CLASSES - 4]);
    classes.extend((4..CLASSES).flat_map(u16::to_be_bytes));
    let rows = alloc::vec![0u8; 2 * usize::from(CLASSES)];
    let parts = [classes, rows, u16s(&[0, 0, 0]), LIG_LAST.to_be_bytes().to_vec(), u16s(&[0]), u16s(&[0])];
    let mut at = LIGATURE_HEADER as u32;
    let mut body = u32::from(CLASSES).to_be_bytes().to_vec();
    for part in &parts {
        body.extend(at.to_be_bytes());
        at += part.len() as u32;
    }
    body.extend(parts.concat());
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let started = std::time::Instant::now();
    morx_closure(&chain_of(1, &[(2, body)]), &active, 2);
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
}

// Sixteen ligature subtables naming one machine, fifteen of them just a header: each pass pays to read its arrays, and
// each walk holds 65 stacks 32,000 ligatures wide from one budget, past which a walk gives every one.
#[test]
fn ligature_walks_share_one_budget() {
    const S: u32 = 16;
    let rows = u16s(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    let entries = u16s(&[0, 0, 0, 0, SET_COMPONENT, 0, 0, PERFORM_ACTION, 0]);
    let actions: Vec<u8> = (0..64u32).flat_map(|i| if i == 63 { LIG_LAST } else { 0 }.to_be_bytes()).collect();
    let ligatures: Vec<u8> = (2..32_002u16).flat_map(u16::to_be_bytes).collect();
    let parts = [u16s(&[8, 1, 1, 4]), rows, entries, actions, u16s(&[0]), ligatures];
    let mut starts = alloc::vec![LIGATURE_HEADER as u32];
    parts.iter().for_each(|part| starts.push(starts[starts.len() - 1] + part.len() as u32));
    let header = |shift: u32| {
        let offsets = starts[..6].iter().map(|&at| at + shift);
        [5].into_iter().chain(offsets).flat_map(u32::to_be_bytes).collect::<Vec<u8>>()
    };
    let mut subtables: Vec<(u32, Vec<u8>)> = (1..S).rev().map(|k| (2, header(40 * k))).collect();
    subtables.push((2, [header(0), parts.concat()].concat()));
    let t = chain_of(S, &subtables);
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let (mut walked, mut left) = (Stackings::new(&t), usize::MAX);
    assert_eq!(walked.reach(&active, 2, &mut left).map(|found| found.len()), Some(0));
    assert!(usize::MAX - left >= 16 * 32_000, "the pass spent {}", usize::MAX - left);
    let (mut walked, mut left) = (Stackings::new(&t), usize::MAX);
    walked.walks = 2_000_000;
    assert_eq!(walked.reach(&active, 2, &mut left).map(|found| found.len()), Some(32_000), "the walks spent apart");
    assert_eq!(walked.walks, 0);
}

// A contextual subtable whose entry names index 7, so 8 substitution offsets, naming in turn 4 lookups that each take
// glyphs 0 to 65,534 to glyph 1: the writer expands each once, spending their reads of `work`, and one short refuses.
#[test]
fn contextual_lookups_the_writer_expands_spend_its_work() {
    const N: u32 = 8;
    let mut body = [3u32, 20, 20, 20, 28].map(u32::to_be_bytes).concat();
    body.extend([0, 0, N as u16 - 1, NO_INDEX].map(u16::to_be_bytes).concat());
    (0..N).for_each(|i| body.extend((4 * N + 24 * (i % 4)).to_be_bytes()));
    (0..4).for_each(|_| body.extend(segment(0, 0xFFFE, 1)));
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let mut work = usize::MAX;
    assert!(subset_contextual(&body, body.len(), &active, &[0, 1], 2, &mut work).is_some());
    let spent = usize::MAX - work;
    assert!((4 * 65_536..5 * 65_536).contains(&spent), "{spent} spent");
    let mut work = spent;
    let kept = subset_contextual(&body, body.len(), &active, &[0, 1], 2, &mut work);
    assert!(kept.is_some() && work == 0, "{work} left");
    let mut work = spent - 1;
    assert!(subset_contextual(&body, body.len(), &active, &[0, 1], 2, &mut work).is_none(), "one short, it's kept");
}

// `s` subtables, contextual and insertion in turn, sharing the last one's machine: `classes`, its class lookup, then
// two states of four classes and one entry, which substitutes and inserts nothing.
fn sharing(s: u32, classes: Vec<u8>) -> Vec<u8> {
    let n = classes.len() as u32;
    let parts = [classes, alloc::vec![0; 16], u16s(&[0, 0, NO_INDEX, NO_INDEX])];
    let header = |shift: u32| {
        let offsets = [20, 20 + n, 36 + n, 44 + n].map(|at| at + shift);
        [4].into_iter().chain(offsets).flat_map(u32::to_be_bytes).collect::<Vec<u8>>()
    };
    let kind = |k: u32| if k.is_multiple_of(2) { 1 } else { 5 };
    let mut subtables: Vec<(u32, Vec<u8>)> = (1..s).rev().map(|k| (kind(k), header(32 * k))).collect();
    subtables.push((kind(0), [header(0), parts.concat()].concat()));
    chain_of(s, &subtables)
}

// 400 such subtables whose class lookup is one segment of glyphs 0 to 65,534: a pass reads it once for each, 26 million
// glyphs, so each spends the pass's budget, which refuses the pass short of them.
#[test]
fn subtables_sharing_a_machine_spend_the_closures_pass_budget() {
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let mut left = 1 << 18;
    let reached = Stackings::new(&sharing(400, segment(0, 0xFFFE, 1))).reach(&active, 2, &mut left);
    assert_eq!(reached, None, "the pass read 400 subtables within its budget");
    let mut left = usize::MAX;
    assert!(Stackings::new(&sharing(2, segment(0, 0xFFFE, 1))).reach(&active, 2, &mut left).is_some());
    assert!(usize::MAX - left > 2 * 65_535, "two subtables spent {}", usize::MAX - left);
}

// The writer reads them as often, and more: each subtable spends what its reading may, and once that's spent every
// later one is left out unread.
#[test]
fn subtables_sharing_a_machine_spend_the_writers_work() {
    let t = sharing(400, segment(0, 0xFFFE, 1));
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let mut work = Work { reading: 1 << 18, placing: usize::MAX };
    let kept = morx_chains(&t).flat_map(|chain| chain.subtables(&t))
        .filter(|s| subset_subtable(s.body, s.own, s.coverage, &active, &[0, 1], 0xFFFF, None, &mut work).is_some())
        .count();
    assert!((1..400).contains(&kept) && work.reading == 0, "{kept} of 400 kept, {} left", work.reading);
}
