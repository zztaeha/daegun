// For each font named on stdin, the text of those in the file given, each after its length in 4 bytes, whose letters
// it maps most of, as "<path>\t<index>", the index -1 where it maps none: the campaign seeds each font with a text
// in its own script.
use daegun::Font;

fn main() {
    let Some(file) = std::env::args().nth(1) else {
        eprintln!("usage: best-text <texts, each after its length> < font paths");
        std::process::exit(2);
    };
    let (data, mut texts) = (std::fs::read(&file).unwrap_or_default(), Vec::new());
    let mut rest = &data[..];
    while let [a, b, c, d, tail @ ..] = rest {
        let (text, more) = tail.split_at((u32::from_be_bytes([*a, *b, *c, *d]) as usize).min(tail.len()));
        texts.push(String::from_utf8_lossy(text));
        rest = more;
    }
    for path in std::io::stdin().lines().map_while(Result::ok) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(f) = Font::from_bytes(&bytes) else { continue };
        // The share of a text's letters the font maps, and how many, so a longer text wins a tie.
        let mapped = |text: &str| {
            let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
            let found = letters.iter().filter(|&&c| f.glyph_id(c).is_some()).count();
            (found * 1000 / letters.len().max(1), found)
        };
        let best = texts.iter().enumerate().map(|(i, t)| (mapped(t), i)).filter(|&((_, n), _)| n > 0).max();
        println!("{path}\t{}", best.map_or(-1, |(_, i)| i as i64));
    }
}
