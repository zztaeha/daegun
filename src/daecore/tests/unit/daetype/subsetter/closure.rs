use super::*;

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// One lookup of `k` subtables, all one SingleSubst taking each pair's first glyph to its second.
fn singles(k: u16, pairs: &[(u16, u16)]) -> Vec<u8> {
    let n = pairs.len() as u16;
    [
        u16s(&[1, 0, 10, 30, 44]),
        u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
        u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0]),
        u16s(&[1, 4, 1, 0, k]), u16s(&vec![6 + 2 * k; k.into()]),
        u16s(&[2, 6 + 2 * n, n]), pairs.iter().flat_map(|p| p.1.to_be_bytes()).collect(),
        u16s(&[1, n]), pairs.iter().flat_map(|p| p.0.to_be_bytes()).collect(),
    ].concat()
}

// An sbix strike in which each of glyphs 1 to n duplicates the next, which a pass follows one step of.
fn dupes(n: u32) -> Vec<u8> {
    let data = 4 + 4 * (n + 3);
    let offsets = (0..=n + 2).flat_map(|g| (data + 10 * g.saturating_sub(1).min(n)).to_be_bytes());
    let records = (2..=n + 1).flat_map(|g| [u16s(&[0, 0]), b"dupe".to_vec(), u16s(&[g as u16])].concat());
    [u16s(&[1, 0, 0, 1, 0, 12, 16, 72]), offsets.collect(), records.collect()].concat()
}

// Glyph 1 closed over `tables`, holding glyph 7 as a glyf component.
fn close(tables: &[(&str, &[u8])]) -> Result<Vec<u32>, crate::daecore::Error> {
    close_in(u32::from(u16::MAX), &[1], tables)
}

// `requested` closed over `tables` in a font of `glyphs` glyphs, as `close`.
fn close_in(glyphs: u32, requested: &[u32], tables: &[(&str, &[u8])]) -> Result<Vec<u32>, crate::daecore::Error> {
    let table = |tag: &str| tables.iter().find(|t| t.0 == tag).map(|t| t.1);
    let reach = |gids: &[u32], set: &mut GlyphSet| {
        gids.iter().filter(|&&g| g < glyphs).for_each(|&g| {
            set.insert(g);
            if g == 1 { set.insert(7); }
        });
        Ok(())
    };
    let closed = closure_loop(table, &FaceReads::of(table), glyphs, requested, reach);
    closed.map(|closed| closed.active.iter().collect())
}

// 2,000 morx chains of 16 bytes that each count every subtable there is, so each reads the rest of the table: the
// closure doesn't read a morx shaping doesn't, rather than walking the 1,999,000 subtables they yield each pass.
#[test]
fn a_morx_whose_chains_read_each_other_is_not_walked() {
    let u32s = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let morx = [u16s(&[2, 0]), u32s(&[2_000]), u32s(&[16, 16, 0, u32::MAX]).repeat(2_000)].concat();
    let table = |tag: &str| (tag == "morx").then_some(&morx[..]);
    let reach = |gids: &[u32], set: &mut GlyphSet| {
        gids.iter().for_each(|&g| _ = set.insert(g));
        Ok(())
    };
    let closed = closure_loop(table, &FaceReads::of(table), 3, &[1], reach).expect("a closure");
    assert!(closed.stackings.is_none(), "the closure walked morx");
}

// Each table once, in rendering order: GSUB substitutes only what text maps to, so MATH's variant 3, COLR's layer 5
// and component 7 never become 4, 6 and 8, while MATH reads the variants GSUB gives. A chain of 100 closes whole.
#[test]
fn each_table_closes_once_in_rendering_order() {
    let gsub = singles(1, &[(3, 4), (5, 6), (7, 8)]);
    let colr = u16s(&[0, 1, 0, 14, 0, 20, 1, 1, 0, 1, 5, 0]);
    let math = u16s(&[1, 0, 0, 0, 10, 0, 12, 0, 1, 0, 18, 1, 1, 1, 0, 1, 3, 0]);
    assert_eq!(close(&[("GSUB", &gsub), ("COLR", &colr), ("MATH", &math)]), Ok(vec![0, 1, 3, 5, 7]));
    let variant_of_3 = u16s(&[1, 0, 0, 0, 10, 0, 12, 0, 1, 0, 18, 1, 1, 3, 0, 1, 9, 0]);
    let gsub = singles(1, &[(1, 3)]);
    assert_eq!(close(&[("GSUB", &gsub), ("MATH", &variant_of_3)]), Ok(vec![0, 1, 3, 7, 9]), "MATH reads GSUB's 3");
    let chain: Vec<(u16, u16)> = (1..=100).map(|g| (g, g + 1)).collect();
    assert_eq!(close(&[("GSUB", &singles(1, &chain))]), Ok((0..=101).collect()));
}

// In a font of 20 glyphs, GSUB takes 500 to 5: neither a requested 500 nor MATH's variant 500 of glyph 1 reaches
// it, as a glyph past the font closes over nothing.
#[test]
fn glyphs_past_the_font_close_over_nothing() {
    let gsub = singles(1, &[(500, 5)]);
    let math = u16s(&[1, 0, 0, 0, 10, 0, 12, 0, 1, 0, 18, 1, 1, 1, 0, 1, 500, 0]);
    let tables: [(&str, &[u8]); 2] = [("GSUB", &gsub), ("MATH", &math)];
    assert_eq!(close_in(20, &[500], &tables), Ok(vec![0]));
    assert_eq!(close_in(20, &[1], &tables), Ok(vec![0, 1, 7]));
}

// 1,200 subtables sharing a chain of 16: its 17 passes spend a set's 1,025 words on each, past what a 2.5 KB
// GSUB gives, so the subset fails rather than spending it. 400 close.
#[test]
fn a_gsub_closure_past_its_budget_fails_the_subset() {
    let chain: Vec<(u16, u16)> = (1..=16).map(|g| (g, g + 1)).collect();
    assert_eq!(close(&[("GSUB", &singles(400, &chain))]), Ok((0..=17).collect()));
    let err = close(&[("GSUB", &singles(1_200, &chain))]).expect_err("past the budget");
    assert!(err.message().contains("GSUB closure past its work budget"), "{err}");
}

// A chain of 100 closes, past 64 passes; one of 30,000 in a 420 KB sbix stops at the work budget, about 100
// passes in, rather than walking it 30,000 times.
#[test]
fn a_closure_runs_until_its_work_budget() {
    assert!(close(&[("sbix", &dupes(100))]).is_ok_and(|s| s.contains(&101)));
    let err = close(&[("sbix", &dupes(30_000))]).expect_err("the chain is past the budget");
    assert!(err.message().contains("work budget"), "{err}");
}

// A VARC of 200 glyphs whose records start at these offsets, over `data` zero bytes: each 3 a component of glyph 0.
fn varc_records(offsets: impl Iterator<Item = u32>, data: usize) -> Vec<u8> {
    let offsets: Vec<u8> = offsets.flat_map(u32::to_be_bytes).collect();
    let head = [0x0001_0000u32, 24, 0, 0, 0, 34].map(u32::to_be_bytes).concat();
    [head, u16s(&[2, 1, 0, 199, 0]), 200u32.to_be_bytes().to_vec(), alloc::vec![4], offsets, alloc::vec![0; data]]
        .concat()
}

// 100 glyphs sharing one record of 2,000 bytes read it once; 100 records starting a byte apart over 2,100 bytes are
// 200 KB of components from 3 KB, past the budget, so the subset fails rather than reading them all.
#[test]
fn a_varc_closure_past_its_budget_fails_the_subset() {
    let glyphs: Vec<u32> = (0..200).collect();
    let shared = varc_records((0..=200).map(|i| if i % 2 == 0 && i < 200 { 1 } else { 2_001 }), 2_000);
    assert_eq!(close_in(201, &glyphs, &[("VARC", &shared)]), Ok(glyphs.clone()));
    let apart = varc_records((0..=200).map(|i| i / 2 + 1 + if i % 2 == 0 && i < 200 { 0 } else { 2_000 }), 2_100);
    let err = close_in(201, &glyphs, &[("VARC", &apart)]).expect_err("past the budget");
    assert!(err.message().contains("VARC closure past its work budget"), "{err}");
}

// sbix's glyph 1 duplicates glyph 2, which COLR draws with layer 5: the pass that adds 2 closes COLR over it, so 5
// comes too and COLR keeps 2's record.
#[test]
fn a_glyph_a_pass_adds_is_drawn_as_the_request_is() {
    let (sbix, colr) = (dupes(1), u16s(&[0, 1, 0, 14, 0, 20, 1, 2, 0, 1, 5, 0]));
    let tables: [(&str, &[u8]); 2] = [("sbix", &sbix), ("COLR", &colr)];
    let table = |tag: &str| tables.iter().find(|t| t.0 == tag).map(|t| t.1);
    let reach = |gids: &[u32], set: &mut GlyphSet| {
        set.extend(gids.iter().copied());
        Ok(())
    };
    let closed = closure_loop(table, &FaceReads::of(table), 20, &[1], reach).expect("a closure");
    assert_eq!(closed.active.iter().collect::<Vec<_>>(), [0, 1, 2, 5]);
    assert_eq!(closed.colr.glyphs.iter().collect::<Vec<_>>(), [0, 1, 2, 5]);
}
