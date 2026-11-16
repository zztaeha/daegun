use daegun::{Font, GlyphId, Location};
use daegun::color::{PaintNode, PaintRef, SceneOptions};

use super::aat_subset::{carets_with, cut, one_face_collection, u16s, u32s, F, F_I};
use daegun::Tag;
use crate::located;

// Which readers take a COLR and its CPAL, each case breaking one rule on carets.ttf: every reader takes both by their
// headers, a part that leaves its table or names an unknown format absent, and drawing takes COLR beside a CPAL.

// Glyph 1 a version 0 glyph over f; glyph 2 a PaintGlyph of f_i over a PaintSolid, glyph 4 a PaintColrLayers of the
// LayerList's one entry, that PaintGlyph; one clip box over glyphs 2 to 4.
const BASE_LIST: usize = 44;
const LAYER_LIST: usize = 60;
const GLYPH: usize = 68;
const CLIP: usize = 85;

fn colr() -> Vec<u8> {
    let mut t = u16s(&[1, 1]);
    t.extend(u32s(&[34, 40]));
    t.extend(u16s(&[1]));
    t.extend(u32s(&[BASE_LIST as u32, LAYER_LIST as u32, CLIP as u32, 0, 0]));
    t.extend(u16s(&[1, 0, 1, F, 0]));
    t.extend([u32s(&[2]), u16s(&[2]), u32s(&[24]), u16s(&[4]), u32s(&[35])].concat());
    t.extend(u32s(&[1, 8]));
    t.extend([&[10, 0, 0, 6][..], &u16s(&[F_I]), &[2, 0, 0, 0x40, 0], &[1, 1, 0, 0, 0, 0]].concat());
    t.extend([&[1, 0, 0, 0, 1][..], &u16s(&[2, 4]), &[0, 0, 12, 1], &u16s(&[0, 0, 500, 700])].concat());
    t
}

fn cpal(version: u16, entries: u16, palettes: u16, records: u16, starts: &[u16], tail: &[u32]) -> Vec<u8> {
    let mut t = [u16s(&[version, entries, palettes, records]), u32s(&[0]), u16s(starts), u32s(tail)].concat();
    let records_at = t.len() as u32;
    t[8..12].copy_from_slice(&records_at.to_be_bytes());
    t.extend([0, 0, 0xFF, 0xFF].repeat(usize::from(records)));
    t
}

fn one_palette() -> Vec<u8> {
    cpal(0, 1, 1, 1, &[0], &[])
}

fn u32_at(b: &[u8], at: usize) -> usize {
    u32::from_be_bytes(b[at..at + 4].try_into().expect("4 bytes")) as usize
}

// A table's offset and length in a font file.
fn table_in(b: &[u8], tag: &[u8; 4]) -> (usize, usize) {
    let tables = usize::from(u16::from_be_bytes([b[4], b[5]]));
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &b[r..r + 4] == tag).expect("the table");
    (u32_at(b, record + 8), u32_at(b, record + 12))
}

fn fixture(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/{rel}", super::FONTS)).expect("the fixture")
}

// Each glyph's drawing on one line: its v0 layers, its v1 paint tree and its clip box.
fn drawn(f: &Font) -> Vec<String> {
    fn tree(p: PaintRef) -> String {
        let below = || p.children().map(tree).collect::<Vec<_>>().join(",");
        match p.node() {
            PaintNode::Layers => format!("L[{}]", below()),
            PaintNode::Solid { .. } => "S".into(),
            PaintNode::Glyph { glyph } => format!("G{glyph}({})", below()),
            other => format!("{other:?}"),
        }
    }
    (0..f.glyph_count())
        .filter_map(|g| {
            let mut line = String::new();
            if let Some(layers) = f.colr_layers(GlyphId(g), 0) {
                let each: Vec<String> =
                    layers.iter().map(|l| format!("{}:{}", l.glyph, if l.foreground { 'f' } else { 'c' })).collect();
                line += &format!(" v0 {}", each.join(" "));
            }
            if let Some(p) = f.paint_graph(GlyphId(g), &Location::default(), 0) {
                line += &format!(" v1 {}", tree(p.root()));
            }
            if let Some(clip) = f.colr_clip_box(GlyphId(g), &Location::default()) {
                line += &format!(" clip {} {} {} {}", clip.x_min, clip.y_min, clip.x_max, clip.y_max);
            }
            (!line.is_empty()).then(|| format!("g{g}{line}"))
        })
        .collect()
}

// How many palettes read, and whether a subset of every glyph keeps COLR and CPAL, which goes with a rejected COLR
// once no paint names its colors.
fn read(f: &Font) -> (usize, bool, bool) {
    let all: Vec<GlyphId> = (0..f.glyph_count()).map(GlyphId).collect();
    let subset =
        Font::from_vec(f.subset(&all, &Location::default()).expect("a subset").font, 0).expect("the subset parses");
    (f.palettes().len(), subset.has_table(Tag::new(b"COLR")), subset.has_table(Tag::new(b"CPAL")))
}

// A case's name, its table, what drawing makes of it and what palettes and subsets read.
type Case<'a> = (&'a str, Vec<u8>, &'a [&'a str], (usize, bool, bool));

const ALL: [&str; 4] =
    ["g1 v0 1:c", "g2 v1 G5(S) clip 0 0 500 700", "g3 clip 0 0 500 700", "g4 v1 L[G5(S)] clip 0 0 500 700"];

#[test]
fn every_reader_takes_colr_by_its_header() {
    let edit = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut t = colr();
        f(&mut t);
        t
    };
    let put = |t: &mut Vec<u8>, at: usize, bytes: &[u8]| t[at..at + bytes.len()].copy_from_slice(bytes);
    let none: [&str; 0] = [];
    let cases: [Case; 12] = [
        ("valid", colr(), &ALL, (1, true, true)),
        ("v0 offset at the end", edit(&|t| put(t, 4, &106u32.to_be_bytes())), &ALL[1..], (1, true, true)),
        ("version 2", edit(&|t| put(t, 0, &[0, 2])), &none, (1, false, false)),
        ("base list's count over", edit(&|t| put(t, BASE_LIST, &3u32.to_be_bytes())), &ALL, (1, true, true)),
        ("layer list's count over", edit(&|t| put(t, LAYER_LIST, &2u32.to_be_bytes())), &ALL, (1, true, true)),
        ("clip list format 2", edit(&|t| t[CLIP] = 2), &[ALL[0], "g2 v1 G5(S)", "g4 v1 L[G5(S)]"], (1, true, true)),
        ("clip box format 0", edit(&|t| t[CLIP + 12] = 0), &[ALL[0], "g2 v1 G5(S)", "g4 v1 L[G5(S)]"], (1, true, true)),
        ("clip box format 3", edit(&|t| t[CLIP + 12] = 3), &[ALL[0], "g2 v1 G5(S)", "g4 v1 L[G5(S)]"], (1, true, true)),
        (
            "root in the records",
            edit(&|t| {
                put(t, BASE_LIST + 6, &10u32.to_be_bytes());
                put(t, BASE_LIST + 10, &512u16.to_be_bytes());
            }),
            &[ALL[0], "g2 v1 S clip 0 0 500 700", ALL[2], "g4 clip 0 0 500 700"],
            (1, true, true),
        ),
        (
            "glyph child null",
            edit(&|t| put(t, GLYPH + 1, &[0, 0, 0])),
            &[ALL[0], "g2 v1 G5(L[]) clip 0 0 500 700", ALL[2], "g4 v1 L[G5(L[])] clip 0 0 500 700"],
            (1, true, true),
        ),
        (
            "solid past the palette",
            edit(&|t| t[GLYPH + 8] = 1),
            &[ALL[0], "g2 v1 G5(L[]) clip 0 0 500 700", ALL[2], "g4 v1 L[G5(L[])] clip 0 0 500 700"],
            (1, true, true),
        ),
        (
            "layer run past the list",
            edit(&|t| put(t, LAYER_LIST, &0u32.to_be_bytes())),
            &[ALL[0], ALL[1], ALL[2], "g4 v1 L[L[]] clip 0 0 500 700"],
            (1, true, true),
        ),
    ];
    for (what, table, want, reads) in cases {
        let f = carets_with(&[("COLR", table), ("CPAL", one_palette())]);
        assert_eq!(drawn(&f), want, "{what}: drawn");
        assert_eq!(read(&f), reads, "{what}: read");
    }
}

#[test]
fn every_reader_takes_cpal_by_its_header() {
    let mut types_short = cpal(1, 1, 1, 1, &[0], &[0, 0, 0]);
    let types_at = types_short.len() as u32;
    types_short[14..18].copy_from_slice(&types_at.to_be_bytes());
    types_short.extend(u16s(&[0]));
    let none: [&str; 0] = [];
    let cases: [Case; 4] = [
        ("version 2", cpal(2, 1, 1, 1, &[0], &[0, 0, 0]), &none, (0, true, false)),
        ("more entries than colors", cpal(0, 2, 1, 1, &[0], &[]), &ALL, (1, true, true)),
        ("palette 1 past the colors", cpal(0, 1, 2, 1, &[0, 1], &[]), &ALL, (2, true, true)),
        ("16-bit room for 32-bit types", types_short, &ALL, (1, true, true)),
    ];
    for (what, table, want, reads) in cases {
        let f = carets_with(&[("COLR", colr()), ("CPAL", table)]);
        assert_eq!(drawn(&f), want, "{what}: drawn");
        assert_eq!(read(&f), reads, "{what}: read");
    }
}

// A CFF or CFF2 face whose maxp says `maxp` glyphs and whose CharStrings hold `n`: base glyph 1 layers glyphs n - 2
// to n + 1, and glyphs n and n + 1 one layer each.
fn layered_past(rel: &str, n: u16, maxp: u16) -> Font {
    let mut count = Font::from_vec(fixture(rel), 0)
        .expect("the fixture").table(Tag::new(b"maxp")).expect("maxp").to_vec();
    count[4..6].copy_from_slice(&maxp.to_be_bytes());
    let (mut records, mut layers, mut first) = (Vec::new(), Vec::new(), 0);
    for (gid, run) in [(1, vec![n - 2, n - 1, n, n + 1]), (n, vec![1]), (n + 1, vec![1])] {
        records.extend(u16s(&[gid, first, run.len() as u16]));
        layers.extend(run.iter().flat_map(|&g| u16s(&[g, 0])));
        first += run.len() as u16;
    }
    let colr = [u16s(&[0, 3]), u32s(&[14, 32]), u16s(&[first]), records, layers].concat();
    super::aat_subset::base_with(rel, &[("maxp", count), ("COLR", colr), ("CPAL", one_palette())])
}

// Drawing counts a CFF face's glyphs by its CharStrings and a glyf face's by its loca, whatever maxp says (7 for 9 or
// 1,007): layer runs end at the first glyph past them, and bases past them have none.
#[test]
fn a_face_draws_layers_up_to_its_drawn_glyph_count() {
    let cases = [
        ("test-fixtures/half-edges.otf", 7, [5, 7, 9]),
        ("source-serif/SourceSerif4Variable-Roman.otf", 1464, [1462, 1464, 1466]),
        ("test-fixtures/carets.ttf", 7, [7, 9, 1007]),
    ];
    for (rel, n, counts) in cases {
        for maxp in counts {
            let f = layered_past(rel, n, maxp);
            let layers = |g| f.colr_layers(GlyphId(g), 0).map(|l| l.iter().map(|l| l.glyph.0).collect::<Vec<_>>());
            let n = u32::from(n);
            assert_eq!(layers(1), Some(vec![n - 2, n - 1]), "{rel} at maxp {maxp}");
            assert_eq!((layers(n), layers(n + 1)), (None, None), "{rel} at maxp {maxp}: bases past the CharStrings");
        }
    }
}

// test_glyphs_variable.ttf's COLR store moved past its stated end, in a font or a collection: the store is absent, so
// glyphs draw alike at any location, and the rest of the table is read and kept by subsets. Cut at the store, alike.
#[test]
fn a_colr_store_past_the_table_is_absent() {
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = std::collections::BTreeMap::new();
    let source = Font::from_vec(fixture("colr-v1-test-glyphs/test_glyphs_variable.ttf"), 0).expect("the fixture");
    for tag in source.table_tags() {
        tables.insert(tag, source.table(tag).expect("listed").to_vec());
    }
    let colr = tables.get_mut(&Tag::new(b"COLR")).expect("a COLR");
    let (end, store) = (colr.len() as u32, u32_at(colr, 30));
    let tail = colr[store..].to_vec();
    colr.extend(tail);
    colr[30..34].copy_from_slice(&end.to_be_bytes());
    let short = cut(daegun::raw::build_font(&tables).unwrap(), b"COLR", end + 1);
    let at = [("COL1", 1.0), ("ROTA", 90.0), ("GRX0", 200.0), ("SCSX", 1.5)];
    let scenes = |f: &Font, at: &[(&str, f64)]| {
        let at = located(f, at);
        let scene = |g| f.color_scene(GlyphId(g), &at, &SceneOptions::default());
        (0..f.glyph_count()).map(|g| format!("{:?}", scene(g))).collect::<Vec<_>>()
    };
    let at_store = cut(short.clone(), b"COLR", end);
    let fonts = [Font::from_vec(short.clone(), 0), Font::from_collection(&one_face_collection(&short), 0)];
    for f in fonts.into_iter().chain([Font::from_vec(at_store, 0)]) {
        let f = f.expect("it parses");
        let stored = scenes(&f, &[]);
        assert_eq!(scenes(&f, &at), stored, "the location moves nothing");
        assert!(stored.iter().any(|s| s.starts_with("Ok(Some")), "a glyph drawn");
        assert!(read(&f).1, "a subset keeps COLR");
    }
}

// A version 2 table, one the spec doesn't define, is read by no reader: nothing draws and a subset keeps none.
#[test]
fn a_version_2_colr_is_read_by_no_reader() {
    let mut table = colr();
    table[1] = 2;
    let f = carets_with(&[("COLR", table), ("CPAL", one_palette())]);
    let subset = Font::from_vec(
        f.subset([GlyphId(0), GlyphId(1), GlyphId(2), GlyphId(4), GlyphId(5)], &Location::default())
            .expect("a subset")
            .font,
        0,
    )
    .expect("it parses");
    assert!(subset.table(Tag::new(b"COLR")).is_none());
    assert!(f.color_scene(GlyphId(2), &Location::default(), &SceneOptions::default()).expect("reads").is_none());
}

// A root offset into the BaseGlyphList's own records reads the paint its bytes there make, a PaintSolid (format 2),
// as an offset anywhere in the table does: drawn, it's a clip, a fill and the clip's end.
#[test]
fn a_root_inside_the_records_reads_the_paint_there() {
    let mut table = colr();
    table[BASE_LIST + 6..BASE_LIST + 10].copy_from_slice(&10u32.to_be_bytes());
    table[BASE_LIST + 10..BASE_LIST + 12].copy_from_slice(&512u16.to_be_bytes());
    let f = carets_with(&[("COLR", table), ("CPAL", one_palette())]);
    let scene = f.color_scene(GlyphId(2), &Location::default(), &SceneOptions::default()).expect("reads");
    assert_eq!(scene.map(|s| s.ops().len()), Some(3));
}

// colr-clip.ttf: D nests 65 paints, and 255 layers share a 4,000-stop gradient. `skip` lifts D's root by that many
// paints, `kept` keeps that many layers.
pub(super) fn colr_clip(skip: usize, kept: usize, edit: impl FnOnce(&mut [u8], usize, usize)) -> Font {
    let mut b = fixture("test-fixtures/colr-clip.ttf");
    let (colr, len) = table_in(&b, b"COLR");
    let (list, layers) = (colr + u32_at(&b, colr + 14), colr + u32_at(&b, colr + 18));
    let record = |b: &[u8], gid: u16| {
        (0..u32_at(b, list)).map(|i| list + 4 + 6 * i).find(|&r| b[r..r + 2] == gid.to_be_bytes())
    };
    let deep = record(&b, 6).expect("D");
    let mut root = list + u32_at(&b, deep + 2);
    for _ in 0..skip {
        root += usize::from(b[root + 1]) << 16 | usize::from(b[root + 2]) << 8 | usize::from(b[root + 3]);
    }
    b[deep + 2..deep + 6].copy_from_slice(&((root - list) as u32).to_be_bytes());
    b[layers + 4 + 4 * kept..layers + 4 + 4 * 255].fill(0);
    let paint_4 = list + u32_at(&b, record(&b, 4).expect("glyph 4") + 2);
    edit(&mut b[colr..colr + len], paint_4 - colr, len);
    Font::from_vec(b, 0).expect("still a font")
}

fn colr_read(f: &Font) -> bool {
    read(f).1
}

// A COLR is read by its header, however deep its paints nest or many layers share a gradient, and its subset keeps it.
#[test]
fn a_subset_keeps_colr_whatever_its_depth_or_layers() {
    let plain = |_: &mut [u8], _: usize, _: usize| {};
    let verdicts = [(0, 0), (1, 0), (2, 0), (1, 68), (1, 69), (1, 255)].map(|(skip, kept)| {
        colr_read(&colr_clip(skip, kept, plain))
    });
    assert_eq!(verdicts, [true; 6]);
}

// A version 0 array of no records at an offset past the table or at its end, an unknown paint format, a child past
// the table: each part is absent or reads nothing, and the COLR is read and kept.
#[test]
fn a_subset_keeps_colr_with_parts_past_the_table() {
    let v0_at = |offset: fn(usize) -> usize| {
        move |t: &mut [u8], _: usize, len: usize| {
            t[2..4].fill(0);
            t[12..14].fill(0);
            t[4..8].copy_from_slice(&(offset(len) as u32).to_be_bytes());
        }
    };
    let cases: [(&str, Font, bool); 4] = [
        ("v0 past the end", colr_clip(1, 0, v0_at(|len| len + 1)), true),
        ("v0 at the end", colr_clip(1, 0, v0_at(|len| len)), true),
        ("unknown format", colr_clip(1, 0, |t, paint, _| t[paint] = 200), true),
        ("child past the end", colr_clip(1, 0, |t, paint, len| {
            t[paint + 1..paint + 4].copy_from_slice(&(len as u32).to_be_bytes()[1..])
        }), true),
    ];
    for (what, f, want) in cases {
        assert_eq!(colr_read(&f), want, "{what}");
    }
}

// A store of an unknown format, a region index past its list or an fvar of another major version leave glyph 19 drawn,
// varied or not, and the COLR read and kept.
#[test]
fn variation_data_never_takes_the_colr_down() {
    let edit = |store_format: bool, region_past: bool, fvar_2: bool| {
        let mut b = fixture("colr-v1-test-glyphs/test_glyphs_variable.ttf");
        let (colr, fvar) = (table_in(&b, b"COLR").0, table_in(&b, b"fvar").0);
        let store = colr + u32_at(&b, colr + 30);
        let (regions, first_set) = (store + u32_at(&b, store + 2), store + u32_at(&b, store + 8));
        if store_format {
            b[store..store + 2].copy_from_slice(&2u16.to_be_bytes());
        }
        if region_past {
            let count = [b[regions + 2], b[regions + 3]];
            b[first_set + 6..first_set + 8].copy_from_slice(&count);
        }
        if fvar_2 {
            b[fvar..fvar + 4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
        }
        Font::from_vec(b, 0).expect("still a font")
    };
    let verdict = |f: Font| (f.paint_graph(GlyphId(19), &Location::default(), 0).is_some(), colr_read(&f));
    let (yes, no) = (true, false);
    for (store, region, fvar) in [(no, no, no), (yes, no, no), (no, yes, no), (no, no, yes), (yes, no, yes)] {
        assert_eq!(verdict(edit(store, region, fvar)), (true, true), "{store} {region} {fvar}");
    }
}

// A layer whose paint lies past the table is absent and the rest of its run kept: spread's 255 layers stay 255, the one
// edited drawing nothing.
#[test]
fn a_layer_past_the_table_is_absent_and_its_run_kept() {
    let drawn_layers = |bad: usize| {
        let f = colr_clip(1, 255, |t, _, len| {
            let at = u32_at(t, 18) + 4 + 4 * bad;
            t[at..at + 4].copy_from_slice(&(len as u32).to_be_bytes());
        });
        let graph = f.paint_graph(GlyphId(28), &Location::default(), 0).expect("spread");
        assert_eq!(graph.root().node(), PaintNode::Layers);
        graph.root().children().count()
    };
    assert_eq!([0, 1, 200].map(drawn_layers), [255; 3]);
}

// A subset keeps the names CPAL 1's labels give, its one palette's 300 and its kept color's 301, beside the font's
// own name 1; 302, which nothing names, goes.
#[test]
fn a_subset_keeps_the_names_cpal_s_labels_give() {
    let mut labeled = cpal(1, 1, 1, 1, &[0], &[0, 0, 0]);
    let palette_labels = labeled.len() as u32;
    labeled[18..22].copy_from_slice(&palette_labels.to_be_bytes());
    labeled.extend(u16s(&[300]));
    let entry_labels = labeled.len() as u32;
    labeled[22..26].copy_from_slice(&entry_labels.to_be_bytes());
    labeled.extend(u16s(&[301]));
    let records = [(1, "Carets"), (300, "Palette"), (301, "Ink"), (302, "Other")];
    let mut name = u16s(&[0, records.len() as u16, 6 + 12 * records.len() as u16]);
    let mut strings = Vec::new();
    for (id, text) in records {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        name.extend(u16s(&[3, 1, 0x409, id, utf16.len() as u16, strings.len() as u16]));
        strings.extend(utf16);
    }
    name.extend(strings);
    let f = carets_with(&[("COLR", colr()), ("CPAL", labeled), ("name", name)]);
    let all: Vec<GlyphId> = (0..f.glyph_count()).map(GlyphId).collect();
    let sub = Font::from_vec(f.subset(&all, &Location::default()).expect("a subset").font, 0)
        .expect("the subset parses");
    let name = sub.table(Tag::new(b"name")).expect("a name table");
    let count = usize::from(u16::from_be_bytes([name[2], name[3]]));
    let ids: Vec<u16> = (0..count).map(|i| u16::from_be_bytes([name[12 + 12 * i], name[13 + 12 * i]])).collect();
    assert_eq!(ids, [1, 300, 301]);
}
