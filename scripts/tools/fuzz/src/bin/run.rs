// Inputs through a harness target in a plain build, each one's time: how triage tells a finding from the instrumented
// build's own slowness, and sees a panic's backtrace. Usage: run <target> <input>...
use daegun_fuzz_harness as harness;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(target), Some(inputs)) = (args.get(1), args.get(2..)) else {
        eprintln!("usage: run <target> <input>...");
        std::process::exit(2);
    };
    let run: fn(&[u8]) = match target.as_str() {
        "load" => harness::load,
        "outline" => harness::outline,
        "raster" => harness::raster,
        "shape" => harness::shape,
        "derive" => harness::derive,
        "text" => harness::text,
        "everything" => harness::everything,
        "stack" => harness::stack,
        #[cfg(feature = "threading")]
        "pool" => harness::pool,
        _ => {
            eprintln!("no target {target}");
            std::process::exit(2);
        }
    };
    for path in inputs {
        let Ok(data) = std::fs::read(path) else { continue };
        let start = std::time::Instant::now();
        harness::bounded(|| run(&data));
        println!("{path}: {:.3} s", start.elapsed().as_secs_f64());
    }
}
