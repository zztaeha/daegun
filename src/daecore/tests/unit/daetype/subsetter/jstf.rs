use super::*;

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// A priority naming one mod list from its eight lookup slots, the list `rel` bytes on.
fn priority(rel: u16) -> Vec<u8> {
    u16s(&[rel, rel, rel, rel, 0, rel, rel, rel, rel, 0])
}

fn list(n: u16) -> Vec<u8> {
    [u16s(&[n]), u16s(&(0..n).collect::<Vec<_>>())].concat()
}

fn everything() -> GlyphSet {
    let mut set = GlyphSet::new();
    set.insert(0);
    set
}

// One language system of 50 priorities named 51 times, each priority naming one 500-lookup list
// eight times: 1,442 bytes that would come to 20.5 MB. Each part is written once in each parent.
fn shared() -> Vec<u8> {
    let script_len = 6 + 50 * 6;
    let mut script = u16s(&[0, script_len as u16, 50]);
    (0..50).for_each(|_| script.extend([&b"dflt"[..], &(script_len as u16).to_be_bytes()].concat()));
    let lang_sys = [u16s(&[50]), u16s(&[102; 50])].concat();
    [u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[12]), script, lang_sys, priority(20), list(500)].concat()
}

#[test]
fn a_part_named_many_times_is_written_once() {
    let jstf = shared();
    assert_eq!(jstf.len(), 1_442);
    let out = subset_jstf(&jstf, &everything(), &[0]).expect("a subset JSTF");
    assert!(out.len() <= jstf.len(), "{} bytes of JSTF became {}", jstf.len(), out.len());
}

// Four language systems of three priorities, each priority naming one 2,000-lookup list eight
// times. Copied per naming that would pass 64 KB and leave the language systems NULL.
#[test]
fn every_language_system_stays_linked() {
    let script_at = 12usize;
    let lang_at = |k: usize| script_at + 24 + 8 * k;
    let prio_at = |j: usize| lang_at(4) + 20 * j;
    let list_at = prio_at(3);
    let rel = |from: usize, to: usize| (to - from) as u16;
    let mut script = u16s(&[0, rel(script_at, lang_at(0)), 3]);
    for k in 1..4 {
        script.extend([&b"LNG "[..], &rel(script_at, lang_at(k)).to_be_bytes()].concat());
    }
    let lang_systems: Vec<u8> = (0..4)
        .flat_map(|k| u16s(&[3, rel(lang_at(k), prio_at(0)), rel(lang_at(k), prio_at(1)), rel(lang_at(k), prio_at(2))]))
        .collect();
    let priorities: Vec<u8> = (0..3).flat_map(|j| priority(rel(prio_at(j), list_at))).collect();
    let jstf =
        [u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[script_at as u16]), script, lang_systems, priorities, list(2_000)]
            .concat();

    let out = subset_jstf(&jstf, &everything(), &[0]).expect("a subset JSTF");
    let script = usize::from(read_u16_be(&out, 10).expect("a script"));
    let mut lang_systems = alloc::vec![read_u16_be(&out, script + 2)];
    lang_systems.extend((0..3).map(|k| read_u16_be(&out, script + 6 + 6 * k + 4)));
    for ls in lang_systems {
        let ls = script + usize::from(ls.filter(|&o| o != 0).expect("a language system left NULL"));
        for p in 0..3 {
            let offset = read_u16_be(&out, ls + 2 + 2 * p).filter(|&o| o != 0).expect("a priority left NULL");
            let prio = ls + usize::from(offset);
            let list = prio + usize::from(read_u16_be(&out, prio).expect("a list"));
            assert_eq!(read_u16_be(&out, list), Some(2_000));
        }
    }
}

// One script extending glyphs 7 and 9. Kept as 1 and 2, the subset is 1.0; with 9 renumbered past 65,535 it is
// 1.1, its scriptList empty and its script in scriptList2 at 32-bit offsets with 24-bit extenders, read back.
#[test]
fn extenders_past_16_bits_write_version_1_1() {
    let header = [0, 1, 0, 0, 0, 1, b'l', b'a', b't', b'n', 0, 12, 0, 6, 0, 0, 0, 0];
    let jstf = [&header[..], &u16s(&[2, 7, 9])].concat();
    let mut active = GlyphSet::new();
    [0, 7, 9].iter().for_each(|&g| { active.insert(g); });
    let mut gid_map = alloc::vec![0; 10];
    (gid_map[7], gid_map[9]) = (1, 2);
    let narrow = subset_jstf(&jstf, &active, &gid_map).expect("a subset JSTF");
    assert_eq!(narrow, [&jstf[..14], &[0, 0, 0, 0, 0, 2, 0, 1, 0, 2]].concat());

    gid_map[9] = 0x1_0001;
    let wide = subset_jstf(&jstf, &active, &gid_map).expect("a subset JSTF");
    assert_eq!(wide, [
        &[0, 1, 0, 1, 0, 0, 0, 1, b'l', b'a', b't', b'n', 0, 0, 0, 16][..],
        &[0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 1, 1, 0, 1],
    ].concat());
    let map = [(alloc::string::String::from("JSTF"), crate::daecore::daetype::TableBytes::from(wide))].into();
    assert_eq!(super::super::super::jstf::jstf_extender_glyphs(&map, *b"latn"), Some(alloc::vec![1, 0x1_0001]));
}

// Three language systems share one priority and its 40,000-byte list. In the subset each takes a
// copy, the third lands past the script's 16-bit offsets, and the table is refused, not left NULL.
#[test]
fn a_script_past_64_kb_is_refused() {
    const N: usize = 3;
    let lang_at = |k: usize| 12 + 6 + 6 * N + 4 * k;
    let prio_at = lang_at(N);
    let mut script = u16s(&[0, 0, N as u16]);
    (0..N).for_each(|k| script.extend([&b"LNG "[..], &((lang_at(k) - 12) as u16).to_be_bytes()].concat()));
    let lang_systems: Vec<u8> = (0..N).flat_map(|k| u16s(&[1, (prio_at - lang_at(k)) as u16])).collect();
    let jstf =
        [u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[12]), script, lang_systems, priority(20), list(20_000)].concat();
    assert!(subset_jstf(&jstf, &everything(), &[0]).is_none(), "a language system was left unlinked");
}

// latn extends glyph 1, kept; `scripts` more records name scripts whose 20,000 extenders, from a run of one word, the
// subset lacks: with `apart` each its own script, each list two bytes on from the last, else all one script.
fn extended(scripts: usize, apart: bool) -> Vec<u8> {
    const RUN: u16 = 20_000;
    let tables = if apart { scripts } else { 1 };
    let first = 6 + 6 * (scripts + 1);
    let at = |i: usize| first + 10 + 6 * if apart { i } else { 0 };
    let run = first + 10 + 6 * tables;
    let records: Vec<u8> = (0..scripts).flat_map(|i| [&b"xxxx"[..], &u16s(&[at(i) as u16])].concat()).collect();
    let own: Vec<u8> = (0..tables).flat_map(|i| u16s(&[(run + 2 * i - at(i)) as u16, 0, 0])).collect();
    [u16s(&[1, 0, scripts as u16 + 1]), b"latn".to_vec(), u16s(&[first as u16]), records, u16s(&[6, 0, 0, 1, 1]),
     own, u16s(&[RUN; RUN as usize + 200])].concat()
}

// 200 scripts whose extender lists start two bytes apart read 8 MB from 43 KB: past the budget the table is refused.
#[test]
fn extenders_read_past_the_budget_refuse_the_table() {
    let (active, gid_map): (GlyphSet, _) = ((0..2).collect(), [0, 1]);
    assert!(subset_jstf(&extended(1, true), &active, &gid_map).is_some());
    assert!(subset_jstf(&extended(200, true), &active, &gid_map).is_none());
}

// 5,000 records naming one script read its extenders once, where reading them for each would take 100 million steps.
#[test]
fn records_naming_one_script_read_its_extenders_once() {
    let (active, gid_map): (GlyphSet, _) = ((0..2).collect(), [0, 1]);
    let jstf = extended(5_000, false);
    let started = std::time::Instant::now();
    let out = subset_jstf(&jstf, &active, &gid_map).expect("a subset JSTF");
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    assert_eq!(read_u16_be(&out, 4), Some(1), "only latn stays");
}
