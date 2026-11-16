use super::*;

// A postcompensation lookup that names glyph 1 twice, as format 6 allows.
#[test]
fn a_glyph_named_twice_by_postcompensation_is_kept_once() {
    let words: [u16; 34] = [
        1, 0, 0, 10, 0,
        0, 32, 36,
        6, 4, 1, 4, 0, 0, 1, 0,
        0, 0,
        6, 4, 2, 8, 1, 0, 1, 20, 1, 20,
        0, 1, 0, 3, 0, 8,
    ];
    let just: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut active = GlyphSet::new();
    active.insert(0);
    active.insert(1);
    let out = subset_just(&just, &active, &[0, 1], 2).expect("a subset just");
    let pc = usize::from(read_u16_be(&out, 14).expect("a pc offset"));
    let lookup = Lookup::parse(out.get(pc..).expect("the pc lookup"), 2).expect("the pc lookup parses");
    let (_, at) = lookup.pairs()[0];
    assert_eq!(read_u32_be(&out, pc + usize::from(at)), Some(1), "glyph 1's actions are not where its offset says");
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// A just table of (category table, width deltas, postcompensation, lookup) per direction, laid out
// after the headers. Parts with the same bytes are written once, so directions can share one.
type Dir = (Option<Vec<u8>>, Vec<u8>, Option<Vec<u8>>, Vec<u8>);

fn just_of(dirs: [Option<Dir>; 2]) -> Vec<u8> {
    let mut out = u16s(&[1, 0, 0, 0, 0]);
    let mut at = 10 + dirs.iter().flatten().map(|d| 6 + d.3.len()).sum::<usize>();
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut heads = Vec::new();
    for (i, d) in dirs.iter().enumerate() {
        let Some((class, wdc, pc, lookup)) = d else { continue };
        out[6 + 2 * i..8 + 2 * i].copy_from_slice(&((10 + heads.len()) as u16).to_be_bytes());
        let mut head = Vec::new();
        for part in [class.as_ref(), Some(wdc), pc.as_ref()] {
            let Some(part) = part else { head.extend([0, 0]); continue };
            let found = parts.iter().scan(10 + dirs.iter().flatten().map(|d| 6 + d.3.len()).sum::<usize>(), |o, p| {
                let here = *o;
                *o += p.len();
                Some((here, p))
            }).find(|(_, p)| *p == part).map(|(o, _)| o);
            let o = found.unwrap_or_else(|| { let o = at; at += part.len(); parts.push(part.clone()); o });
            head.extend((o as u16).to_be_bytes());
        }
        head.extend(lookup);
        heads.extend(head);
    }
    [out, heads, parts.concat()].concat()
}

fn wide(glyphs: &[u16]) -> Vec<u32> {
    glyphs.iter().map(|&g| g.into()).collect()
}

fn active(glyphs: &[u16]) -> GlyphSet {
    let mut set = GlyphSet::new();
    glyphs.iter().for_each(|&g| { set.insert(g.into()); });
    set
}

// The category table is a mort-style subtable: rebuilt in place, its class array follows the
// kept glyphs. Read as an extended state table, it would be dropped from every subset.
#[test]
fn the_category_table_is_kept_with_its_classes_renumbered() {
    let states = [0u8; 12];
    let st = [u16s(&[6, 8, 17, 29]), u16s(&[1, 5]), alloc::vec![4, 5, 4, 5, 4], states.to_vec(), u16s(&[0, 0, 0, 0])]
        .concat();
    let category = [u16s(&[(8 + st.len()) as u16, 0, 0, 1]), st].concat();
    let lookup = build_aat_lookup(&[(2, 0)]).expect("a lookup");
    let just = just_of([Some((Some(category), u16s(&[0, 0]), None, lookup)), None]);
    let out = subset_just(&just, &active(&[0, 2, 4]), &[0, 0, 1, 0, 2], 6).expect("a subset just");
    let direction = usize::from(read_u16_be(&out, 6).expect("a direction"));
    let at = usize::from(read_u16_be(&out, direction).expect("a category offset"));
    assert_ne!(at, 0, "the category table was dropped");
    assert_eq!(out[at + 16..at + 22], [0, 1, 0, 2, 5, 5]);
}

// Two directions each with its own 40,000-byte width delta table cannot both sit within 16-bit
// offsets: the table is refused, not written with an offset wrapped past 64 KB. One they share fits.
#[test]
fn offsets_past_64_kb_refuse_the_table_but_a_shared_part_fits() {
    let lookup = build_aat_lookup(&[(1, 0)]).expect("a lookup");
    let (a, b) = (alloc::vec![1u8; 40_000], alloc::vec![2u8; 40_000]);
    let everything = active(&[0, 1]);
    let apart = just_of([Some((None, a.clone(), None, lookup.clone())), Some((None, b, None, lookup.clone()))]);
    assert!(subset_just(&apart, &everything, &[0, 1], 2).is_none(), "an offset past 64 KB was written");

    let shared = just_of([Some((None, a.clone(), None, lookup.clone())), Some((None, a, None, lookup))]);
    let out = subset_just(&shared, &everything, &[0, 1], 2).expect("a shared table fits");
    let wdc = |dir: usize| read_u16_be(&out, usize::from(read_u16_be(&out, dir).expect("a direction")) + 2);
    assert_eq!(wdc(6), wdc(8));
    assert!(out.len() < 41_000, "the shared table was written twice");
}

// 1,100 glyphs whose postcompensation names one action list: copied per glyph it would pass 64 KB
// and drop the whole postcompensation table.
#[test]
fn an_action_list_many_glyphs_share_is_written_once() {
    let n = 1_100u16;
    let list = [
        0u32.to_be_bytes().to_vec(),
        1u32.to_be_bytes().to_vec(),
        u16s(&[0, 3]),
        56u32.to_be_bytes().to_vec(),
        alloc::vec![0; 48],
    ]
    .concat();
    let lookup_len = 12 + 4 * (usize::from(n) + 1);
    let pc_lookup = {
        let mut t = u16s(&[6, 4, n, 4096, 10, 4 * n - 4096]);
        (1..=n).for_each(|g| t.extend(u16s(&[g, lookup_len as u16])));
        t.extend(u16s(&[0xFFFF, 0]));
        t
    };
    let list = list[4..].to_vec();
    let pc = [pc_lookup, list].concat();
    let lookup = build_aat_lookup(&[(1, 0)]).expect("a lookup");
    let just = just_of([Some((None, u16s(&[0, 0]), Some(pc), lookup)), None]);
    let glyphs: Vec<u16> = (0..=n).collect();
    let out = subset_just(&just, &active(&glyphs), &wide(&glyphs), (n + 1).into()).expect("a subset just");
    let head = usize::from(read_u16_be(&out, 6).expect("a direction"));
    let pc = usize::from(read_u16_be(&out, head + 4).expect("a pc offset"));
    assert_ne!(pc, 0, "the postcompensation table was dropped");
    let lookup = Lookup::parse(&out[pc..], (n + 1).into()).expect("the pc lookup");
    let at: alloc::collections::BTreeSet<u16> = lookup.pairs().into_iter().map(|(_, v)| v).collect();
    assert_eq!(at.len(), 1, "the list was written more than once");
    assert_eq!(read_u32_be(&out, pc + usize::from(*at.first().expect("one"))), Some(1));
}

// A postcompensation value of 0 means no action. Glyphs 1 and 3 have one, and read as an offset it
// parses the lookup's own bytes into a stretch action they never had.
#[test]
fn a_postcompensation_value_of_0_is_no_action() {
    let list = [1u32.to_be_bytes().to_vec(), u16s(&[0, 3]), 8u32.to_be_bytes().to_vec()].concat();
    let pc = [u16s(&[0, 1, 0, 3, 0, 8, 14]), list].concat();
    let lookup = build_aat_lookup(&[(1, 0)]).expect("a lookup");
    let just = just_of([Some((None, u16s(&[0, 0]), Some(pc), lookup)), None]);
    let glyphs: Vec<u16> = (0..6).collect();
    let out = subset_just(&just, &active(&glyphs), &wide(&glyphs), 6).expect("a subset just");
    let head = usize::from(read_u16_be(&out, 6).expect("a direction"));
    let pc = usize::from(read_u16_be(&out, head + 4).expect("a pc offset"));
    let lookup = Lookup::parse(&out[pc..], 6).expect("the pc lookup");
    assert_eq!(lookup.pairs().into_iter().map(|(g, _)| g).collect::<Vec<_>>(), [5]);
}

// Postcompensation naming `glyphs` lists, each starting at one of `records` stretch records' last word, so reading
// on through every record after it: past the subset budget the table is refused, within it the lists go.
#[test]
fn action_lists_read_past_the_budget_refuse_the_table() {
    let just = |glyphs: usize, records: usize| {
        let base = 6 + 2 * glyphs;
        let head = u16s(&[1, 0, 0, 10, 0, 0, 22, 24, 8, 1, 0, 0, 8, 1, glyphs as u16]);
        let values: Vec<u16> = (0..glyphs).map(|i| (base + 12 * i + 8) as u16).collect();
        [head, u16s(&values), u16s(&[0, 3, 0, 12, 0xFFFF, 0xFFFF]).repeat(records)].concat()
    };
    let kept: Vec<u16> = (0..=64).collect();
    assert!(subset_just(&just(64, 2_000), &active(&kept), &wide(&kept), 65).is_none());
    let within = subset_just(&just(64, 20), &active(&kept), &wide(&kept), 65).expect("within the budget");
    assert_eq!(read_u16_be(&within, 14), Some(0));
}
