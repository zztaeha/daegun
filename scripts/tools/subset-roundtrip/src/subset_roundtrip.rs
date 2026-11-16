use std::{env, fs, process};

use daegun::{Font, GlyphId, Location};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (input, output, text) = match args.as_slice() {
        [i, o] => (i, o, None),
        [i, o, t] => (i, o, Some(t)),
        _ => {
            eprintln!("usage: subset-roundtrip <in.ttf> <out.ttf> [text]");
            process::exit(2);
        }
    };

    let bytes = fs::read(input).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
    let font = Font::from_bytes(&bytes).unwrap_or_else(|e| fail(&format!("{input}: parse: {e}")));

    let cut = match text {
        Some(t) => font.subset_text(t, &Location::default()),
        None => {
            let n = font.glyph_count();
            let stride = if n > 21 { 7 } else if n > 3 { 2 } else { 1 };
            let gids: Vec<GlyphId> = (0..n).step_by(stride).map(GlyphId).collect();
            if gids.is_empty() {
                fail::<()>(&format!("{input}: the font has no glyphs"));
            }
            font.subset(&gids, &Location::default())
        }
    };
    let cut = cut.unwrap_or_else(|e| fail(&format!("{input}: subset: {e:?}")));

    if Font::from_bytes(&cut.font).is_err() {
        fail::<()>(&format!("{input}: the subset does not parse"));
    }

    eprintln!("{input}: {} -> {} bytes", bytes.len(), cut.font.len());
    fs::write(output, &cut.font).unwrap_or_else(|e| fail(&format!("{output}: {e}")));
}

fn fail<T>(message: &str) -> T {
    eprintln!("{message}");
    process::exit(1);
}
