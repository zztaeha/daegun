use std::time::{Duration, Instant};
use alloc::vec::Vec;

use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daeshaper::{face::Face, pipeline, run::Run};

fn cache_of(rel: &str) -> FontCache {
    let path = format!("{}/{}", super::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes)
        .unwrap_or_else(|e| panic!("{path} did not parse: {e}"));
    FontCache::new(map).expect("the fixture loads")
}

// Shapes are timed eight at a time, a sample their mean: one shape under a microsecond lands on whole ticks of the
// 41.67 ns clock. Eight divides every row's count and `pmu-fast.sh`'s counter batches.
const GROUP: usize = 8;

fn report(name: &str, detail: &str, samples: &mut [Duration]) {
    samples.sort();
    let n = samples.len();
    let sum: Duration = samples.iter().sum();
    eprintln!("{name}");
    eprintln!("  {detail}");
    eprintln!(
        "  n {n}  min {:?}  mean {:?}  median {:?}  p95 {:?}  max {:?}",
        samples[0], sum / n as u32, samples[n / 2], samples[(n as f64 * 0.95) as usize], samples[n - 1],
    );
}

// Instructions and cycles a shape over each batch run wholly on performance cores, as `pmu-fast.sh` reads them.
fn report_counts(name: &str, batch: usize, total: usize, kept: &mut [(f64, f64)]) {
    if kept.is_empty() {
        eprintln!("counts {name} batch {batch} kept 0/{total}");
        return;
    }
    let median = |k: &mut [(f64, f64)], key: fn(&(f64, f64)) -> f64| {
        k.sort_by(|a, b| key(a).total_cmp(&key(b)));
        key(&k[k.len() / 2])
    };
    let insns = median(kept, |k| k.0);
    let cycles = median(kept, |k| k.1);
    eprintln!("counts {name} batch {batch} kept {}/{total} insn {insns:.1} cyc {cycles:.1}", kept.len());
}

fn bench(name: &str, rel: &str, text: &str, iters: usize, detail: &str) {
    let env = |key: &str| std::env::var(key).ok().and_then(|s| s.parse::<usize>().ok());
    let batch = env("DAEGUN_BENCH_BATCH").unwrap_or(0);
    let iters = match env("DAEGUN_BENCH_BATCHES") {
        Some(batches) if batch > 0 => batch * batches,
        _ => iters * env("DAEGUN_BENCH_SCALE").unwrap_or(1),
    };
    let fc = cache_of(rel);
    let face = Face::new(&fc);

    let mut probe = Run::default();
    probe.push_str(text);
    let direction = pipeline::settle_script(&mut probe);
    let tags = probe.script.map(crate::daecore::daeshaper::ot::tag::script_tags);
    let tags = tags.as_ref().map_or(&[][..], |t| t.as_slice());
    let plan = crate::tests::plain_plan(&face, probe.script, direction, tags);

    let mut warm = Run::default();
    warm.push_str(text);
    let warm_dir = pipeline::settle_script(&mut warm);
    pipeline::shape(&face, &plan, &mut warm, warm_dir, &[]);
    let first = crate::tests::placed(&warm);
    let expect = first.len();
    assert!(expect > 0, "{name}: shaped nothing at all");
    assert!(
        first.iter().all(|g| g.0.id != 0),
        "{name}: a .notdef means the fixture stopped mapping this text and the bench times a notdef pass",
    );
    assert!(
        first.iter().map(|g| g.1.advance_x as i64).sum::<i64>() != 0,
        "{name}: every advance is zero, so positioning produced nothing to measure",
    );

    assert!(
        iters % GROUP == 0 && batch % GROUP == 0,
        "{name}: {iters} shapes in batches of {batch} must fill whole groups of {GROUP}",
    );
    let mut samples = Vec::with_capacity(iters / GROUP);
    let mut proof = 0usize;
    let mut kept = Vec::new();
    // The first fifth of the batches warms caches and clocks and isn't kept.
    let warm = iters.checked_div(batch).unwrap_or(0) / 5;
    let mut last = super::core_counts();
    let mut group = Vec::with_capacity(GROUP);
    for i in (GROUP..=iters).step_by(GROUP) {
        group.extend((0..GROUP).map(|_| {
            let mut buffer = Run::default();
            buffer.push_str(text);
            let target = pipeline::settle_script(&mut buffer);
            (buffer, target)
        }));
        let t = Instant::now();
        for (buffer, target) in &mut group {
            pipeline::shape(&face, &plan, buffer, *target, &[]);
        }
        samples.push(t.elapsed() / GROUP as u32);
        for (buffer, _) in group.drain(..) {
            let r = crate::tests::placed(&buffer);
            proof += r.iter().filter(|g| g.0.id != 0).count();
            core::hint::black_box(&r);
        }
        if batch > 0 && i % batch == 0 {
            let now = super::core_counts();
            if let (Some([p0, e0]), Some([p1, e1])) = (last, now)
                && e1 == e0
                && i / batch > warm
            {
                kept.push(((p1[0] - p0[0]) as f64 / batch as f64, (p1[1] - p0[1]) as f64 / batch as f64));
            }
            last = now;
        }
    }
    assert_eq!(
        proof,
        iters * expect,
        "{name}: every iteration must yield {expect} non-notdef glyphs; a short count means a run bailed \
         out early and the median is timing a failure path",
    );
    report(name, detail, &mut samples);
    if batch > 0 {
        report_counts(name, batch, iters / batch - warm, &mut kept);
    }
}

#[test]
#[ignore]
fn shape_latin_sentence_inter() {
    bench(
        "shape_latin_sentence_inter",
        "inter/InterVariable.ttf",
        "Typography is the craft of endowing human language with a durable visual form.",
        20_000,
        "InterVariable.ttf, default instance, Latin LTR, 78-character English sentence",
    );
}

#[test]
#[ignore]
fn shape_latin_ligatures_eb_garamond() {
    bench(
        "shape_latin_ligatures_eb_garamond",
        "eb-garamond/EBGaramond.ttf",
        "The office staff shuffled a fistful of waffles.",
        20_000,
        "EBGaramond.ttf, Latin LTR, a sentence chosen so liga fires repeatedly (ffi, ffl, fi, fl)",
    );
}

#[test]
#[ignore]
fn shape_arabic_joined_run_scheherazade() {
    bench(
        "shape_arabic_joined_run_scheherazade",
        "scheherazade-new/ScheherazadeNew-Regular.ttf",
        "السلام عليكم ورحمة الله وبركاته",
        15_000,
        "ScheherazadeNew-Regular.ttf, Arabic RTL, one fully joined run: init/medi/fina plus marks",
    );
}

#[test]
#[ignore]
fn shape_devanagari_conjuncts_noto() {
    bench(
        "shape_devanagari_conjuncts_noto",
        "noto-devanagari/NotoSansDevanagari.ttf",
        "हिन्दी भाषा में स्वागत है",
        8_000,
        "NotoSansDevanagari.ttf, Devanagari LTR, conjuncts and matra reordering",
    );
}

#[test]
#[ignore]
fn shape_cjk_sentence_source_han() {
    bench(
        "shape_cjk_sentence_source_han",
        "source-han-sans/SourceHanSansJP-VF.otf",
        "日本語の組版では、文字の間隔と行の組み方が読みやすさを決める。",
        120_000,
        "SourceHanSansJP-VF.otf, Japanese LTR, 30 characters, no GSUB substitution to speak of",
    );
}

#[test]
#[ignore]
fn shape_myanmar_syllables_noto() {
    bench(
        "shape_myanmar_syllables_noto",
        "noto-myanmar/NotoSansMyanmar.ttf",
        "မြန်မာဘာသာစကားသည် မြန်မာနိုင်ငံ၏ ရုံးသုံးဘာသာစကား ဖြစ်သည်။",
        15_000,
        "NotoSansMyanmar.ttf, Myanmar LTR, medials, stacked consonants and reordered vowels",
    );
}
