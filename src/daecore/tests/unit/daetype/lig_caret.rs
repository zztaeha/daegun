use super::*;
use super::super::subsetter::otl::gdef::var_store;

// GDEF's own store, as a font keeps it.
fn parsed_store(gdef: &[u8]) -> impl FnOnce() -> Option<Shared<ItemVariationStore>> + '_ {
    move || var_store(gdef).map(Shared::new)
}

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// GDEF 1.0 whose ligature, glyph 1, lists `count` carets at the offsets `offsets` from its LigGlyph,
// the caret tables `carets` after them.
fn gdef(count: u16, offsets: &[u16], carets: &[u8]) -> Vec<u8> {
    let lig_glyph = 2 + 2 * offsets.len() as u16 + carets.len() as u16;
    let mut t = words(&[1, 0, 0, 0, 12, 0, 6 + lig_glyph, 1, 6, count]);
    t.extend(words(offsets));
    t.extend(carets);
    t.extend(words(&[1, 1, 1]));
    t
}

// A point with no outline to read it from cannot be resolved; the caret after it keeps its place
// rather than taking the first's.
#[test]
fn a_caret_that_cannot_be_resolved_keeps_its_place() {
    let t = gdef(2, &[6, 10], &words(&[2, 1, 1, 600]));
    assert_eq!(ligature_carets(&t, 1, || None, &[], false, parsed_store(&t)), [None, Some(600.0)]);
}

// Four carets listed and the table ending after one offset and the caret it names: no carets, not
// the one read before the array runs out.
#[test]
fn a_caret_list_cut_short_gives_none() {
    let t = words(&[1, 0, 0, 0, 12, 0, 6, 1, 12, 1, 1, 1, 4, 4, 1, 600]);
    assert_eq!(ligature_carets(&t, 1, || None, &[], false, parsed_store(&t)), []);
}

// 8,192 carets naming one format 3 caret at 300 whose delta row spans 16,384 regions, each adding 1
// at the peak: worked out per caret, 134 million multiply-adds a call.
#[test]
fn a_shared_delta_is_worked_out_once() {
    const CARETS: u16 = 8_192;
    const REGIONS: u16 = 16_384;
    let caret_at = 2 + 2 * CARETS;
    let mut t = words(&[1, 3, 0, 0, 18, 0, 0]);
    let coverage = 18 + 6 + caret_at as usize + 12;
    let store = coverage + 6;
    t.extend((store as u32).to_be_bytes());
    t.extend(words(&[(coverage - 18) as u16, 1, 6, CARETS]));
    (0..CARETS).for_each(|_| t.extend(caret_at.to_be_bytes()));
    t.extend(words(&[3, 300, 6, 0, 0, 0x8000, 1, 1, 1]));
    t.extend(words(&[1, 0, 12, 1]));
    t.extend((16 + 6 * u32::from(REGIONS)).to_be_bytes());
    t.extend(words(&[1, REGIONS]));
    (0..REGIONS).for_each(|_| t.extend(words(&[0, 0x4000, 0x4000])));
    t.extend(words(&[1, 0, REGIONS]));
    t.extend((0..REGIONS).flat_map(u16::to_be_bytes));
    t.extend(core::iter::repeat_n(1u8, usize::from(REGIONS)));
    let started = std::time::Instant::now();
    let carets = ligature_carets(&t, 1, || None, &[1.0], false, parsed_store(&t));
    assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    assert_eq!(carets.len(), usize::from(CARETS));
    assert!(carets.iter().all(|&c| c == Some(300.0 + f64::from(REGIONS))), "{:?}", &carets[..1]);
}
