use super::{run_carets, Edges, ShapedRun};
use crate::daecore::GlyphId;
use alloc::vec::Vec;

fn run(advances: &[f64], clusters: &[u32]) -> ShapedRun {
    let n = advances.len();
    ShapedRun {
        glyphs: vec![GlyphId(1); n],
        advances: advances.to_vec(),
        offsets: vec![Default::default(); n],
        unsafe_to_break: vec![false; n],
        unsafe_to_concat: vec![false; n],
        safe_to_insert_tatweel: vec![false; n],
        clusters: clusters.to_vec(),
        complete: true,
        has_broken_syllable: false,
        shaper: Default::default(),
    }
}

fn no_ligature(_: GlyphId) -> (Vec<Option<f64>>, f64) {
    unreachable!("only a one-glyph cluster asks for carets")
}

// Two graphemes shaped to two glyphs in one cluster split the cluster evenly, in either direction.
#[test]
fn graphemes_of_a_cluster_of_several_glyphs_split_it_evenly() {
    assert_eq!(run_carets(&run(&[300.0, 500.0], &[0, 0]), "ab", false, false, no_ligature), [0.0, 400.0, 800.0]);
    assert_eq!(run_carets(&run(&[300.0, 500.0], &[0, 0]), "ab", true, false, no_ligature), [800.0, 400.0, 0.0]);
}

// Lao MN's fi ligature puts its caret a unit left of the glyph, and carets may come out of order: each
// boundary stays inside the ligature and after the one before it.
#[test]
fn a_ligature_caret_outside_the_glyph_or_out_of_order_is_kept_in_place() {
    let one = |carets: &[Option<f64>]| {
        let carets = carets.to_vec();
        run_carets(&run(&[600.0], &[0]), "ffi", false, false, move |_| (carets.clone(), 0.0))
    };
    assert_eq!(one(&[Some(-1.0), Some(700.0)]), [0.0, 0.0, 600.0, 600.0]);
    assert_eq!(one(&[Some(400.0), Some(200.0)]), [0.0, 400.0, 400.0, 600.0]);
    let rtl = run_carets(&run(&[600.0], &[0]), "ffi", true, false, |_| (vec![Some(400.0), Some(200.0)], 0.0));
    assert_eq!(rtl, [600.0, 200.0, 200.0, 0.0]);
}

// A character with no glyph of its own, as a removed ignorable leaves, sits where the text starts, each of its three
// bytes.
#[test]
fn a_character_before_the_first_cluster_takes_its_edge() {
    let carets = run_carets(&run(&[300.0, 500.0], &[3, 4]), "\u{200B}ab", false, false, no_ligature);
    assert_eq!(carets, [0.0, 0.0, 0.0, 0.0, 300.0, 800.0]);
}

fn boundaries(edges: &Edges, carets: &[Option<f64>]) -> [f64; 2] {
    [1, 2].map(|k| edges.boundary(k, 3, carets))
}

const HORIZONTAL: Edges = Edges { left: 100.0, right: 1000.0, top: 0.0, rtl: false, vertical: false };

#[test]
fn left_to_right_components_end_at_the_carets_in_order() {
    assert_eq!(boundaries(&HORIZONTAL, &[Some(300.0), Some(600.0)]), [400.0, 700.0]);
}

// The first component of right-to-left text is the glyph's rightmost, so it ends at the last caret,
// measured from the left edge as every caret is. Read from the right edge, both would come out mirrored.
#[test]
fn right_to_left_components_end_at_the_carets_from_the_right() {
    let rtl = Edges { rtl: true, ..HORIZONTAL };
    assert_eq!(boundaries(&rtl, &[Some(300.0), Some(600.0)]), [700.0, 400.0]);
}

// Down a vertical line from the glyph's top at 880 above its origin, the first boundary is the
// highest caret.
#[test]
fn vertical_components_end_down_from_the_top() {
    let vertical = Edges { top: 880.0, vertical: true, ..HORIZONTAL };
    assert_eq!(boundaries(&vertical, &[Some(300.0), Some(600.0)]), [380.0, 680.0]);
}

// A caret that cannot be resolved falls evenly between the edges; the rest keep their places.
#[test]
fn an_unresolved_caret_falls_evenly_and_the_rest_stay() {
    assert_eq!(boundaries(&HORIZONTAL, &[None, Some(600.0)]), [400.0, 700.0]);
    assert_eq!(boundaries(&HORIZONTAL, &[Some(250.0), None]), [350.0, 700.0]);
}
