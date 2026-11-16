use daegun::color::{PaintNode, SceneOptions};

use super::aat_subset::{carets_with, glyph_ids, u16s, u32s, F, F_I, I, L};
use daegun::{Bounds, GlyphId, Location};

// Records listing glyph 2 twice among 1 to 3: a search reading the lower middle record meets the first copy.

fn vorg() -> Vec<u8> {
    u16s(&[1, 0, 900, 4, 1, 100, 2, 200, 2, 300, 3, 400])
}

// One palette of one color.
fn cpal() -> Vec<u8> {
    let mut t = u16s(&[0, 1, 1, 1]);
    t.extend(u32s(&[14]));
    t.extend(u16s(&[0]));
    t.extend([0, 0, 0xFF, 0xFF]);
    t
}

// Base glyph records for glyphs 1, 2, 2 and 3, each over one layer: f, i, l and f_i.
fn colr_v0() -> Vec<u8> {
    let mut t = u16s(&[0, 4]);
    t.extend(u32s(&[14, 38]));
    t.extend(u16s(&[4, 1, 0, 1, 2, 1, 1, 2, 2, 1, 3, 3, 1]));
    t.extend(u16s(&[F, 0, I, 0, L, 0, F_I, 0]));
    t
}

// A version 1 COLR of `base` (glyph, paint) records, whose paints fill f or f_i, and `clips` (first, last, box).
fn colr_v1(base: &[(u16, bool)], clips: &[(u16, u16, [i16; 4])]) -> Vec<u8> {
    let list_at = 34;
    let paints_at = 4 + 6 * base.len();
    let mut list = u32s(&[base.len() as u32]);
    for &(glyph, wide) in base {
        list.extend(u16s(&[glyph]));
        list.extend(u32s(&[(paints_at + if wide { 11 } else { 0 }) as u32]));
    }
    for glyph in [F, F_I] {
        list.extend([10, 0, 0, 6]);
        list.extend(u16s(&[glyph]));
        list.extend([2, 0, 0, 0x40, 0]);
    }
    let clip_at = if clips.is_empty() { 0 } else { list_at + list.len() };
    let mut clip = vec![1];
    clip.extend(u32s(&[clips.len() as u32]));
    let boxes_at = 5 + 7 * clips.len();
    for (i, &(first, last, _)) in clips.iter().enumerate() {
        clip.extend(u16s(&[first, last]));
        clip.extend(&((boxes_at + 9 * i) as u32).to_be_bytes()[1..]);
    }
    for &(.., edges) in clips {
        clip.push(1);
        clip.extend(edges.iter().flat_map(|e| e.to_be_bytes()));
    }
    let mut t = u16s(&[1, 0]);
    t.extend(u32s(&[0, 0]));
    t.extend(u16s(&[0]));
    t.extend(u32s(&[list_at as u32, 0, clip_at as u32, 0, 0]));
    t.extend(list);
    if !clips.is_empty() {
        t.extend(clip);
    }
    t
}

#[test]
fn vorg_reads_the_first_copy_of_a_repeated_glyph() {
    let f = carets_with(&[("VORG", vorg())]);
    assert_eq!([1, 2, 3].map(|g| f.vertical_origin(GlyphId(g), &Location::default())), [100.0, 200.0, 400.0]);
}

// COLR's base records are searched as VORG's are, reading the lower middle record: glyph 2's first copy, in version 0
// and in version 1, where that copy fills f and the second f_i.
#[test]
fn colr_draws_the_first_copy_of_a_repeated_base_glyph() {
    let f = carets_with(&[("COLR", colr_v0()), ("CPAL", cpal())]);
    let layers = |g| f.colr_layers(GlyphId(g), 0).map(|layers| layers.iter().map(|l| l.glyph).collect::<Vec<_>>());
    assert_eq!([1, 2, 3].map(layers), [Some(glyph_ids(&[F])), Some(glyph_ids(&[I])), Some(glyph_ids(&[F_I]))]);

    let f = carets_with(&[("COLR", colr_v1(&[(1, true), (2, false), (2, true), (3, true)], &[])), ("CPAL", cpal())]);
    let filled = |g| match f.paint_graph(GlyphId(g), &Location::default(), 0)?.root().node() {
        PaintNode::Glyph { glyph } => Some(glyph),
        _ => None,
    };
    assert_eq!([1, 2, 3].map(filled), [F_I, F, F_I].map(|g| Some(GlyphId(g.into()))));
}

// A clip covers the glyphs from its first to its last, so where two overlap, the one the search meets holds.
#[test]
fn a_clip_box_covers_its_range_of_glyphs() {
    let (wide, narrow) = ([0, -100, 500, 800], [10, 0, 300, 700]);
    let clips = [(2, 5, wide), (3, 3, narrow)];
    let f = carets_with(&[("COLR", colr_v1(&[(3, false), (4, false)], &clips)), ("CPAL", cpal())]);
    let [x_min, y_min, x_max, y_max] = wide.map(f64::from);
    let edges = Bounds { x_min, y_min, x_max, y_max };
    let boxes = [2, 3, 4, 5, 6].map(|g| f.colr_clip_box(GlyphId(g), &Location::default()));
    assert_eq!(boxes, [Some(edges), Some(edges), Some(edges), Some(edges), None]);
}

// The spec: "If the COLR table is present in a font but no CPAL table exists, then the COLR table is ignored." A
// version 1 glyph then has no paint, scene or clip box, as a version 0 glyph has no layers.
#[test]
fn a_colr_without_cpal_is_ignored() {
    let colr = colr_v1(&[(3, false)], &[(3, 3, [10, 0, 300, 700])]);
    let drawn = |f: &daegun::Font| {
        let (glyph, at) = (GlyphId(3), Location::default());
        let scene = matches!(f.color_scene(glyph, &at, &SceneOptions::default()), Ok(Some(_)));
        (f.paint_graph(glyph, &at, 0).is_some(), scene, f.colr_clip_box(glyph, &at).is_some())
    };
    assert_eq!(drawn(&carets_with(&[("COLR", colr.clone()), ("CPAL", cpal())])), (true, true, true));
    assert_eq!(drawn(&carets_with(&[("COLR", colr)])), (false, false, false));
    assert_eq!(carets_with(&[("COLR", colr_v0())]).colr_layers(GlyphId(1), 0), None);
}
