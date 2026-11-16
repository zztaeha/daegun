use std::sync::Arc;

use daegun::{Font, GlyphId, Location};
use daegun::shape::{ClusterLevel, Direction, ShapeOptions, ShapedRun, Shaper};
use daegun::unicode::script_runs;

use super::aat_subset::{carets_with, kern_ot, u16s, F, F_F_I, F_I, I, L};
use super::layout_build::{classes, coverage, layout, lookup, single, table, to, w};
use super::unicode_properties::mapping;

// abvs turning f into l under `tag`: the Universal Shaping Engine applies it and the default shaper doesn't.
fn abvs_under(tag: &[u8; 4]) -> Vec<(&'static str, Vec<u8>)> {
    vec![("GSUB", layout(tag, &[(b"abvs", &[0])], vec![lookup(1, 0, vec![to(single(F, L))])]))]
}

// "fi" seeded with the script of `sample`.
fn seeded(f: &Font, sample: &str) -> Arc<ShapedRun> {
    let script = script_runs(sample)[0].script;
    f.shape("fi", &Location::default(), &ShapeOptions::default().with_seed_script(script))
}

// Scripts of Unicode 17 and 18 go to the Universal Shaping Engine, so abvs applies, and Garay and Sidetic run right
// to left.
#[test]
fn new_scripts_take_their_shaper_and_direction() {
    let cases: [(&str, &[u8; 4], [u16; 2]); 8] = [
        ("\u{1E6C0}", b"tayo", [L, I]),
        ("\u{10940}", b"sidt", [I, L]),
        ("\u{16EA0}", b"berf", [L, I]),
        ("\u{11DB0}", b"tols", [L, I]),
        ("\u{18E00}", b"jurc", [L, I]),
        ("\u{125A8}", b"pcun", [L, I]),
        ("\u{3D000}", b"seal", [L, I]),
        ("\u{10D40}", b"gara", [I, L]),
    ];
    for (sample, tag, glyphs) in cases {
        let run = seeded(&carets_with(&abvs_under(tag)), sample);
        let want = glyphs.map(|g| GlyphId(g.into()));
        let got = (run.shaper, run.glyphs.as_slice());
        assert_eq!(got, (Shaper::Universal, &want[..]), "{}", String::from_utf8_lossy(tag));
    }
}

// Code points to glyphs, plus other tables.
fn with(groups: &[(u32, u32, u16)], tables: Vec<(&'static str, Vec<u8>)>) -> Vec<(&'static str, Vec<u8>)> {
    [mapping(groups), tables].concat()
}

// The run in one line, glyph=cluster@x,y+advance#flags, down the page as +0,-advance.
fn printed(tables: &[(&str, Vec<u8>)], text: &str, vertical: bool, options: &ShapeOptions) -> String {
    let options = if vertical { options.with_direction(Direction::TopToBottom) } else { *options };
    let run = carets_with(tables).shape(text, &Location::default(), &options);
    let glyphs: Vec<String> = (0..run.glyphs.len())
        .map(|i| {
            let (x, y) = run.offsets[i].into();
            let at = if (x, y) == (0.0, 0.0) { String::new() } else { format!("@{x},{y}") };
            let advance = match run.advances[i] {
                a if vertical && a != 0.0 => format!("0,{}", -a),
                a => a.to_string(),
            };
            let flag = |set: &[bool], bit: u8| if set.get(i) == Some(&true) { bit } else { 0 };
            let bits =
                flag(&run.unsafe_to_break, 1) | flag(&run.unsafe_to_concat, 2) | flag(&run.safe_to_insert_tatweel, 4);
            let flags = if bits == 0 { String::new() } else { format!("#{bits}") };
            format!("{}={}{at}+{advance}{flags}", run.glyphs[i], run.clusters[i])
        })
        .collect();
    format!("[{}]", glyphs.join("|"))
}

fn plain(tables: &[(&str, Vec<u8>)], text: &str) -> String {
    printed(tables, text, false, &ShapeOptions::default())
}

// calt turns beh into l before yeh.
fn beh_before_yeh() -> Vec<(&'static str, Vec<u8>)> {
    let chain = table(vec![w(&[3, 0, 1]), to(coverage(&[F])), w(&[1]), to(coverage(&[I])), w(&[1, 0, 1])]);
    let lookups = vec![lookup(6, 0, vec![to(chain)]), lookup(1, 0, vec![to(single(F, L))])];
    with(&[(0x0628, 0x0628, F), (0x064A, 0x064A, I)], vec![("GSUB", layout(b"arab", &[(b"calt", &[0])], lookups))])
}

// rclt turns beh into i, and calt i into l.
fn rclt_then_calt() -> Vec<(&'static str, Vec<u8>)> {
    let lookups = vec![lookup(1, 0, vec![to(single(F, I))]), lookup(1, 0, vec![to(single(I, L))])];
    with(&[(0x0628, 0x0628, F)], vec![("GSUB", layout(b"arab", &[(b"calt", &[1]), (b"rclt", &[0])], lookups))])
}

// A ZWNJ is skipped in an Arabic context, so beh still comes before yeh, and calt runs in rclt's stage, after it.
#[test]
fn arabic_contexts_skip_zwnj_and_calt_runs_with_rclt() {
    assert_eq!(plain(&beh_before_yeh(), "\u{0628}\u{200C}\u{064A}"), "[2=2+400#1|3=0+400]");
    assert_eq!(plain(&rclt_then_calt(), "\u{0628}"), "[3=0+400]");
}

// Beh, alef, Phags-pa's left-joining superfixed ra, and a space to stand for what is hidden. A GPOS, empty, keeps
// fallback mark positioning from flagging a hidden character with the letter before it.
fn joining_letters() -> Vec<(&'static str, Vec<u8>)> {
    let groups = [(0x20, 0x20, L), (0x0627, 0x0627, I), (0x0628, 0x0628, F), (0xA872, 0xA872, F_F_I)];
    with(&groups, vec![("GPOS", layout(b"arab", &[], vec![]))])
}

// A letter after the run makes its end unsafe to break, or safe for a tatweel, and a run's first or last letter is
// unsafe to concatenate beside a joining neighbor.
#[test]
fn joining_flags_reach_into_the_context() {
    let shape = |text: &str, before: &str, after: &str, concat: bool, tatweel: bool| {
        let options = ShapeOptions::default()
            .with_report_unsafe_to_concat(concat)
            .with_report_tatweel_positions(tatweel)
            .with_context(before, after);
        printed(&joining_letters(), text, false, &options)
    };
    assert_eq!(shape("\u{0628}\u{200E}", "", "\u{0628}", false, false), "[3=2+0#1|1=0+400]");
    assert_eq!(shape("\u{0628}\u{200E}", "", "\u{0628}", false, true), "[3=2+0#5|1=0+400]");
    assert_eq!(shape("\u{0627}", "\u{0628}", "", true, false), "[2=0+400#2]");
    assert_eq!(shape("\u{A872}", "", "\u{00A0}", true, false), "[4=0+900#2]");
}

// stch turns tatweel into f, l and f, l a repeating tile 2 units wide, between behs 2,000 wide.
fn tatweel_tiles() -> Vec<(&'static str, Vec<u8>)> {
    let hmtx = u16s(&[400, 20, 400, 20, 400, 20, 2, 20, 2000, 20, 20, 20]);
    let tiles = table(vec![w(&[1]), to(coverage(&[I])), w(&[1]), to(u16s(&[3, F, L, F]))]);
    let gsub = layout(b"arab", &[(b"stch", &[0])], vec![lookup(2, 0, vec![to(tiles)])]);
    with(&[(0x0628, 0x0628, F_F_I), (0x0640, 0x0640, I)], vec![("hmtx", hmtx), ("GSUB", gsub)])
}

// A stretch makes at most 256 glyphs: of the 600 tiles that would fill the word's 2,000 units with the fixed ones,
// 254 are drawn, placed as all 600 would be.
#[test]
fn a_stretch_makes_at_most_256_glyphs() {
    let tiles: Vec<String> = (-154..100)
        .map(|k| if k == 0 { "3=2+0#1".to_string() } else { format!("3=2@{},0+0#1", 2 * k) })
        .collect();
    let want = format!("[4=4+2000#1|1=2@-708,0+0#1|{}|1=2@200,0+0#1|4=0+2000]", tiles.join("|"));
    assert_eq!(plain(&tatweel_tiles(), "\u{0628}\u{0640}\u{0628}"), want);
}

// rlig joins two Old Hungarian letters, with an acute to place on the second.
fn old_hungarian_ligature() -> Vec<(&'static str, Vec<u8>)> {
    let set = table(vec![w(&[1]), to(u16s(&[F_I, 2, I]))]);
    let rlig = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let gsub = layout(b"hung", &[(b"rlig", &[0])], vec![lookup(4, 0, vec![to(rlig)])]);
    with(&[(0x0301, 0x0301, L), (0x10C80, 0x10C80, F), (0x10C81, 0x10C81, I)], vec![("GSUB", gsub)])
}

// Most of Old Hungarian's letters are of Bidi class R, so down the page its ligature splits as a right-to-left one
// does, and the acute over the second letter sits on the ligature's left part.
#[test]
fn a_right_to_left_ligature_splits_right_to_left_down_the_page() {
    let run = printed(&old_hungarian_ligature(), "\u{10C80}\u{10C81}\u{0301}", true, &ShapeOptions::default());
    assert_eq!(run, "[5=0@-450,-850+0,-1000|3=0@105,956+0]");
}

// rphf makes Modi ra and virama a repha.
fn modi_repha() -> Vec<(&'static str, Vec<u8>)> {
    let set = table(vec![w(&[1]), to(u16s(&[F_I, 2, I]))]);
    let rphf = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let gsub = layout(b"modi", &[(b"rphf", &[0])], vec![lookup(4, 0, vec![to(rphf)])]);
    let groups = [(0x1160E, 0x1160E, L), (0x11628, 0x11628, F), (0x11630, 0x11630, F_F_I), (0x1163F, 0x1163F, I)];
    with(&groups, vec![("GSUB", gsub)])
}

// The Universal Shaping Engine puts a repha before a post-base vowel sign, moves a pre-base vowel sign before a
// symbol, splits Chakma's vowel sign o though the font has it, and keeps Kaithi's dddha whole where the font has it.
#[test]
fn universal_clusters_reorder_and_decompose_as_their_data_says() {
    assert_eq!(plain(&modi_repha(), "\u{11628}\u{1163F}\u{1160E}\u{11630}"), "[3=0+400|5=0+900|4=0+900]");
    let buginese = mapping(&[(0x1A19, 0x1A19, I), (0x1A1F, 0x1A1F, F)]);
    assert_eq!(plain(&buginese, "\u{1A1F}\u{1A19}"), "[2=0+400|1=0+400]");
    let kaithi = mapping(&[(0x11099, 0x11099, I), (0x1109A, 0x1109A, F), (0x110BA, 0x110BA, L)]);
    assert_eq!(plain(&kaithi, "\u{1109A}"), "[1=0+400]");
    let chakma = [(0x11107, 0x11107, L), (0x11127, 0x11127, F_I), (0x1112E, 0x1112E, F), (0x11131, 0x11131, I)];
    let chakma = mapping(&chakma);
    assert_eq!(plain(&chakma, "\u{11107}\u{1112E}"), "[3=0+400|2=0@-400,0+0|5=0@-900,0+0]");
}

// Without GPOS marks, Hebrew composes to a presentation form its decomposition names: Yiddish double yod and patah
// make one, yod and patah don't.
#[test]
fn hebrew_composes_only_to_its_presentation_forms() {
    let f = mapping(&[(0x05B7, 0x05B7, I), (0x05D9, 0x05D9, F), (0x05F2, 0x05F2, L), (0xFB1F, 0xFB1F, F_I)]);
    assert_eq!(plain(&f, "\u{05F2}\u{05B7}"), "[5=0+900]");
    assert_eq!(plain(&f, "\u{05D9}\u{05B7}"), "[2=0@80,-762+0|1=0+400]");
}

// kern pulls the second syllable 300 units toward the first.
fn hangul_kerned() -> Vec<(&'static str, Vec<u8>)> {
    let pairs = [u16s(&[1, 6, 0, 0]), u16s(&[F, I, (-300i16) as u16])].concat();
    with(&[(0x0301, 0x0301, L), (0xAC00, 0xAC00, F), (0xAC01, 0xAC01, I)], vec![("kern", kern_ot(&[(1, 0, pairs)]))])
}

// With fallback positioning, kern applies to Hangul and an acute is placed; a trailing jamo a syllable can't take is
// unsafe to break from it, and a tone mark joins its syllable's cluster.
#[test]
fn hangul_takes_fallback_positions_and_keeps_its_clusters() {
    assert_eq!(plain(&hangul_kerned(), "\u{AC00}\u{AC01}"), "[1=0+100|2=3+400#1]");
    assert_eq!(plain(&hangul_kerned(), "\u{AC00}\u{0301}"), "[1=0+400|3=0@-320,762+0]");
    assert_eq!(plain(&mapping(&[(0x11C3, 0x11C3, I), (0xAC00, 0xAC00, F)]), "\u{AC00}\u{11C3}"), "[1=0+400|2=3+400#1]");
    let tone = mapping(&[(0x302E, 0x302E, L), (0xAC00, 0xAC00, F), (0xAC01, 0xAC01, I)]);
    let characters = ShapeOptions::default().with_cluster_level(ClusterLevel::MonotoneCharacters);
    assert_eq!(printed(&tone, "\u{AC00}\u{302E}\u{AC01}", false, &characters), "[3=0+400|1=0+400|2=6+400]");
}

// Without a space glyph, spaces take the invisible glyph at a quarter em, not its own width, and a narrow space half
// that. An invisible glyph of 0 names none, so a hidden ZWJ takes the font's space.
#[test]
fn spaces_take_the_invisible_glyph_at_their_own_widths() {
    let invisible = |glyph| ShapeOptions::default().with_invisible_glyph(glyph);
    let no_space = mapping(&[(0x41, 0x41, F), (0x42, 0x42, I)]);
    let run = printed(&no_space, "A B\u{2002}B\u{202F}A", false, &invisible(GlyphId(3)));
    assert_eq!(run, "[1=0+400|3=1+250|2=2+400|3=3+500|2=6+400|3=7+125|1=10+400]");
    let space = mapping(&[(0x20, 0x20, L), (0x41, 0x41, F)]);
    assert_eq!(printed(&space, "A\u{200D}A", false, &invisible(GlyphId(0))), "[1=0+400|3=0+0|1=4+400]");
}

// Devanagari ra, virama and ka: rphf's one context names `first` for ra and changes nothing, and half joins ra and
// virama.
fn reph_context(kind: u16, context: Vec<u8>) -> Vec<(&'static str, Vec<u8>)> {
    let set = table(vec![w(&[1]), to(u16s(&[F_I, 2, I]))]);
    let half = table(vec![w(&[1]), to(coverage(&[F])), w(&[1]), to(set)]);
    let lookups = vec![lookup(kind, 0, vec![to(context)]), lookup(4, 0, vec![to(half)])];
    let gsub = layout(b"dev2", &[(b"half", &[1]), (b"rphf", &[0])], lookups);
    with(&[(0x0915, 0x0915, L), (0x0930, 0x0930, F), (0x094D, 0x094D, I)], vec![("GSUB", gsub)])
}

// A context of format 2 or 3 tests its first glyph by its coverage, which lacks ra, so ra and virama make a half
// form, whether the coverage's glyph shares ra's digest bits, 4,096 past it, or not, 64 past.
#[test]
fn a_reph_context_tests_its_first_glyph_by_its_coverage() {
    for (first, want) in [(F + 4096, "[5=0+900|3=6+400#1]"), (F + 64, "[5=0+900|3=6+400#1]")] {
        let rules = table(vec![w(&[1]), to(u16s(&[2, 0, 2]))]);
        let class_def = classes(&[(F, F, 1), (I, I, 2)]);
        let contexts = [
            (5, table(vec![w(&[3, 2, 0]), to(coverage(&[first])), to(coverage(&[I]))])),
            (5, table(vec![w(&[2]), to(coverage(&[first])), to(class_def), w(&[2, 0]), to(rules)])),
            (6, table(vec![w(&[3, 0, 2]), to(coverage(&[first])), to(coverage(&[I])), w(&[0, 0])])),
        ];
        for (kind, context) in contexts {
            let label = format!("{first} {kind} {}", context[1]);
            assert_eq!(plain(&reph_context(kind, context), "\u{0930}\u{094D}\u{0915}"), want, "{label}");
        }
    }
}
