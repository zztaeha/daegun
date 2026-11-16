use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use daegun_fuzz_harness::{check_syntax, exercise, texts_for, Rng};

fn mutate(bytes: &mut Vec<u8>, rng: &mut Rng) {
    let num_tables = |b: &[u8]| -> usize {
        if b.len() < 28 { return 0 }
        let n = u16::from_be_bytes([b[4], b[5]]) as usize;
        if 12 + n * 16 <= b.len() { n } else { 0 }
    };
    let window = |b: &[u8], rng: &mut Rng, w: usize| -> Option<usize> {
        b.len().checked_sub(w).map(|slack| rng.below(slack + 1))
    };

    if bytes.is_empty() { return; }
    match rng.below(9) {
        0 => { let at = rng.below(bytes.len()); bytes.truncate(at); }
        1 => { let at = rng.below(bytes.len()); bytes[at] ^= 1 << rng.below(8); }
        2 => if let Some(at) = window(bytes, rng, 2) {
            bytes[at] = 0xFF; bytes[at + 1] = 0xFF;
        },
        3 => if let Some(at) = window(bytes, rng, 4) {
            for b in &mut bytes[at..at + 4] { *b = 0xFF; }
        },
        4 => if let Some(at) = window(bytes, rng, 4) {
            for b in &mut bytes[at..at + 4] { *b = 0; }
        },
        5 => {
            let n = num_tables(bytes);
            if n >= 2 {
                let (a, b) = (rng.below(n), rng.below(n));
                for k in 0..16 {
                    bytes.swap(12 + a * 16 + k, 12 + b * 16 + k);
                }
            }
        }
        6 => {
            let n = num_tables(bytes);
            if n >= 1 {
                let e = 12 + rng.below(n) * 16 + 8;
                let off = (rng.next() % bytes.len() as u64) as u32;
                bytes[e..e + 4].copy_from_slice(&off.to_be_bytes());
            }
        }
        // Near a table's head, where its offsets and counts lie, values that only overflow a 32-bit usize: a run of
        // them, as two counts side by side overflow only together.
        7 => {
            let n = num_tables(bytes);
            if n >= 1 {
                let e = 12 + rng.below(n) * 16 + 8;
                let table = u32::from_be_bytes([bytes[e], bytes[e + 1], bytes[e + 2], bytes[e + 3]]) as usize;
                let at = table.saturating_add(2 * rng.below(32));
                const FAR: [u32; 6] = [0xFFFF_FFFF, 0xFFFF_FFFC, 0xFFFF_FFF0, 0xC000_0000, 0x8000_0000, 0x4000_0000];
                let far = FAR[rng.below(FAR.len())].to_be_bytes();
                for at in (0..1 + rng.below(4)).map(|i| at.saturating_add(4 * i)) {
                    if let Some(field) = at.checked_add(4).and_then(|end| bytes.get_mut(at..end)) {
                        field.copy_from_slice(&far);
                    }
                }
            }
        }
        _ => {
            let at = rng.below(bytes.len());
            let len = rng.below(64).min(bytes.len() - at);
            let fill = (rng.next() & 0xFF) as u8;
            for b in &mut bytes[at..at + len] { *b = fill; }
        }
    }
}

fn fixtures(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); continue }
            if matches!(p.extension().and_then(|x| x.to_str()), Some("ttf" | "otf" | "ttc")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str, default: u64| -> u64 {
        args.iter().position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let count = flag("--count", 2000);
    let seed = flag("--seed", 0);
    let only = args.iter().any(|a| a == "--seed");
    let dump: Option<String> = args.iter().position(|a| a == "--dump")
        .and_then(|i| args.get(i + 1)).cloned();

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/test-fonts");
    let corpus = fixtures(&root);
    // Each `--extra dir` seeds with that folder's fonts too, such as the beyond-64K proof's. One missing adds none.
    let extra: Vec<PathBuf> =
        args.windows(2).filter(|pair| pair[0] == "--extra").flat_map(|pair| fixtures(Path::new(&pair[1]))).collect();
    if corpus.is_empty() {
        eprintln!("no fixtures under {}", root.display());
        std::process::exit(2);
    }

    let hook = std::panic::take_hook();
    // Quiet over thousands of inputs; `--seed` asks about one, where it fell.
    std::panic::set_hook(Box::new(move |info| {
        if only {
            eprintln!("{info}\n{}", std::backtrace::Backtrace::force_capture());
        }
    }));

    let mut failures = Vec::new();
    let range: Box<dyn Iterator<Item = u64>> =
        if only { Box::new(seed..=seed) } else { Box::new(0..count) };

    for s in range {
        let mut rng = Rng(s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        // One input in two from the repo's own, which a folder of thousands would drown.
        let pool = if extra.is_empty() || rng.below(2) == 0 { &corpus } else { &extra };
        let pick = &pool[rng.below(pool.len())];
        let Ok(bytes) = std::fs::read(pick) else { continue };
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let mut bytes = bytes;
            for _ in 0..=rng.below(4) {
                mutate(&mut bytes, &mut rng);
            }
            if let Some(path) = &dump {
                let _ = std::fs::write(path, &bytes);
            }
            check_syntax(&mut rng);
            let texts = texts_for(&mut rng);
            exercise(&bytes, &texts);
        }));
        if let Err(payload) = caught {
            let why = payload.downcast_ref::<String>().map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("(no message)")
                .to_string();
            failures.push((s, pick.file_name().unwrap_or_default().to_string_lossy().to_string(), why));
        }
    }

    std::panic::set_hook(hook);

    if failures.is_empty() {
        println!("{} inputs from {} fonts, no panics", if only { 1 } else { count }, corpus.len() + extra.len());
        return;
    }
    println!("{} panics", failures.len());
    for (s, f, why) in failures.iter().take(20) {
        println!("  seed {s} ({f}): {why}");
    }
    std::process::exit(1);
}
