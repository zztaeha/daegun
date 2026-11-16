use super::*;
use crate::daecore::daeshaper::plan::PlanRequest;
use crate::daecore::daekit::cache::FontCache;

fn noto_devanagari() -> FontCache {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/noto-devanagari/NotoSansDevanagari.ttf");
    let bytes = std::fs::read(path).expect("the fixture");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("its tables");
    FontCache::new(tables).expect("a font")
}

fn devanagari_plan(face: &Face) -> Plan {
    let tags = crate::daecore::daeshaper::ot::tag::script_tags(Script::DEVANAGARI);
    let direction = crate::daecore::daeshaper::run::Direction::LeftToRight;
    let request = PlanRequest {
        script: Some(Script::DEVANAGARI),
        direction,
        script_tags: tags.as_slice(),
        language_tags: &[],
        language: None,
        features: &[],
        switches: &[],
    };
    Plan::new(face, &request)
}

// Runs `check` on the model's view of a Devanagari plan for Noto Sans Devanagari, with the font's glyph count
// and its virama glyph.
fn on_a_devanagari_plan(check: impl FnOnce(&FontForms, u32, u32)) {
    let cache = noto_devanagari();
    let face = Face::new(&cache);
    let plan = devanagari_plan(&face);
    let p = FontForms::of(&plan, &face).expect("the model's data");
    let virama = p.data.virama.expect("a virama glyph");
    assert_eq!(p.data.positions.slots.len(), face.glyph_count() as usize);
    check(&p, face.glyph_count(), virama);
}

// Every glyph's position asked of the font, then read twice through the plan's cache: once filling it, once
// from it. Devanagari's below-base forms make some answers other than the base position.
#[test]
fn a_consonants_kept_position_is_the_one_its_font_gives() {
    on_a_devanagari_plan(|p, glyphs, virama| {
        let (run, mut below) = (&mut Run::default(), 0);
        for glyph in 0..glyphs {
            let asked = p.ask_position(run, glyph, virama);
            assert_eq!(p.position(run, glyph, virama), asked, "glyph {glyph}, filling the cache");
            assert_eq!(p.position(run, glyph, virama), asked, "glyph {glyph}, from the cache");
            below += usize::from(asked == at::BELOW);
        }
        assert!(below > 0, "no glyph took a below-base position");
    });
}

// A run with no edits left can't pay for an ask, so every consonant reads as a base and none of those answers is
// kept: a run that can pay gets the font's own.
#[test]
fn an_ask_the_run_cant_pay_for_isnt_kept() {
    on_a_devanagari_plan(|p, glyphs, virama| {
        let below = (0..glyphs).find(|&g| p.ask_position(&mut Run::default(), g, virama) == at::BELOW);
        let below = below.expect("a below-base consonant");
        let mut spent = Run::default();
        spent.begin();
        spent.take_ops(u32::MAX);
        assert_eq!(p.position(&mut spent, below, virama), at::BASE);
        assert_eq!(p.position(&mut Run::default(), below, virama), at::BELOW);
    });
}

// A plan's position table goes back to the font's allowance when the plan is dropped, while its location lives
// on, and a plan built again draws the same bytes.
#[test]
fn a_dropped_plans_positions_go_back_to_the_allowance() {
    let cache = noto_devanagari();
    let face = Face::new(&cache);
    let plan = devanagari_plan(&face);
    let left = cache.cmap_index_allowance();
    drop(plan);
    assert_eq!(cache.cmap_index_allowance(), left + face.glyph_count() as usize);
    let _again = devanagari_plan(&face);
    assert_eq!(cache.cmap_index_allowance(), left);
}

// Threads asking the same glyphs at once each get the font's answer, and the bytes they race to fill keep it.
#[cfg(feature = "threading")]
#[test]
fn threads_filling_the_cache_at_once_keep_the_fonts_answers() {
    use alloc::vec::Vec;
    on_a_devanagari_plan(|p, glyphs, virama| {
        let run = &mut Run::default();
        let asked: Vec<u8> = (0..glyphs).map(|glyph| p.ask_position(run, glyph, virama)).collect();
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    let run = &mut Run::default();
                    start.wait();
                    for glyph in 0..glyphs {
                        let position = p.position(run, glyph, virama);
                        assert_eq!(position, asked[glyph as usize], "glyph {glyph}");
                    }
                });
            }
        });
        let kept: Vec<u8> = p.data.positions.slots.iter().map(|slot| slot.load(Ordering::Relaxed)).collect();
        assert_eq!(kept, asked.iter().map(|position| position + 1).collect::<Vec<u8>>());
    });
}
