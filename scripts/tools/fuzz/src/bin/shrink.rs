// Fonts cut down to fuzzable seeds by daegun's own subsetter: each face of each font named on stdin, about 32 of its
// glyphs spread over its cmap, at its default and, where it varies, at its axes' two ends; written to the folder given.
use daegun::{Font, GlyphId, Tag};

fn main() {
    let Some(out) = std::env::args().nth(1) else {
        eprintln!("usage: shrink <folder> < font paths");
        std::process::exit(2);
    };
    let mut written = 0;
    for (n, path) in std::io::stdin().lines().map_while(Result::ok).enumerate() {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        for face in 0..Font::face_count(&bytes).min(4) {
            let Ok(f) = Font::from_collection(&bytes, face) else { continue };
            let mapped: Vec<GlyphId> = f.coverage().map(|(_, g)| g).collect();
            let step = (mapped.len() / 32).max(1);
            let mut glyphs: Vec<GlyphId> = mapped.iter().step_by(step).take(32).copied().collect();
            glyphs.push(GlyphId(0));
            let ends = |pick: fn(&daegun::Axis) -> f64| f.axes().iter().map(|a| (a.tag, pick(a))).collect::<Vec<_>>();
            let mut locations = vec![f.location(&[] as &[(Tag, f64)])];
            if f.is_variable() {
                locations.extend([f.location(&ends(|a| a.min)), f.location(&ends(|a| a.max))]);
            }
            for (k, at) in locations.iter().enumerate() {
                if let Ok(sub) = f.subset(&glyphs, at)
                    && sub.font.len() <= 1 << 20
                    && std::fs::write(format!("{out}/s{n}-{face}-{k}.ttf"), &sub.font).is_ok()
                {
                    written += 1;
                }
            }
        }
    }
    println!("{written} seeds");
}
