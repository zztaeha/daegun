use super::aat_shaping::{gdef, kern, kern_machine, kerx};
use super::aat_subset::{carets_with, glyph_ids, lookup, morx, shapes_alike, stx, u16s, u32s, F, I, L};
use daegun::Location;
use daegun::shape::ShapeOptions;

// Classes 4, 5 and 6 for f, i and l, after the four every state table has.
const CLASSES: [(u16, u16); 3] = [(F, 4), (I, 5), (L, 6)];

// A GSUB whose ccmp, the default script's one feature, substitutes `from` with `to`.
fn gsub_single(from: u16, to: u16) -> Vec<u8> {
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let features = [u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0])].concat();
    let lookups = u16s(&[1, 4, 1, 0, 1, 8, 2, 8, 1, to, 1, 1, from]);
    let (at_scripts, at_features) = (10, 10 + scripts.len() as u16);
    let at_lookups = at_features + features.len() as u16;
    [u16s(&[1, 0, at_scripts, at_features, at_lookups]), scripts, features, lookups].concat()
}

// A ligature subtable that forms nothing, its ligature list at offset 0.
fn ligatures_at_zero() -> Vec<u8> {
    let start: &[u16] = &[0; 7];
    let tails = [u32s(&[0x8000_0000]), u16s(&[0]), u16s(&[F])];
    let mut body = stx(7, &CLASSES, &[start, start], &[&[0, 0, 0]], &tails);
    body[24..28].fill(0);
    body
}

// An Apple `kern` pair, fi by -40.
fn kern_pair() -> Vec<u8> {
    kern(&[(0, 0, [u16s(&[1, 0, 0, 0]), u16s(&[F, I, (-40i16) as u16])].concat())])
}

// morx turns f into l beside a ligature subtable whose ligatures lie at offset 0, the subtable's own start: that
// subtable forms nothing, the morx is read whole, and GSUB's ccmp, which would turn f into i, gives way to it.
#[test]
fn a_morx_with_an_odd_subtable_is_still_read() {
    let good = (4, lookup(&[(F, L)]));
    let gsub = ("GSUB", gsub_single(F, I));
    let read = carets_with(&[("morx", morx(2, std::slice::from_ref(&good))), gsub.clone()]);
    assert_eq!(read.shape("f", &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(&[L]));
    let odd = carets_with(&[("morx", morx(2, &[good, (2, ligatures_at_zero())])), gsub]);
    assert_eq!(odd.shape("f", &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(&[L]));
    shapes_alike(&odd, "fil");
}

// A morx is read by its own bytes, whatever the lengths of the tables beside it: 19,892 bytes beside a GSUB of 2,794
// and a GDEF of 340 or 341, it turns f into l either way.
#[test]
fn a_morx_is_read_whatever_the_lengths_beside_it() {
    let padded = |mut table: Vec<u8>, len: usize| {
        table.resize(len, 0);
        table
    };
    let with_gdef = |gdef_len: usize| {
        carets_with(&[
            ("morx", padded(morx(2, &[(4, lookup(&[(F, L)]))]), 19_892)),
            ("GSUB", padded(gsub_single(F, I), 2_794)),
            ("GDEF", padded(gdef(&[]), gdef_len)),
        ])
    };
    for gdef_len in [340, 341] {
        let f = with_gdef(gdef_len);
        assert_eq!(f.shape("f", &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(&[L]));
        shapes_alike(&f, "fil");
    }
}

// f pushes and l kerns it by -40, but i's entry goes to the row before the state array, which is absent: the machine
// stops there, so fil keeps its advances where fl kerns.
#[test]
fn a_kern_machine_stops_at_a_state_before_its_array() {
    let mut machine = kern_machine(0, &[-40, 61]);
    machine[46..48].copy_from_slice(&10u16.to_be_bytes());
    let f = carets_with(&[("kern", kern(&[(0, 1, machine)]))]);
    for (text, advances) in [("fl", vec![360.0, 400.0]), ("fil", vec![400.0; 3])] {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, advances, "{text}");
        shapes_alike(&f, text);
    }
}

// kerx format 2 classes f and i as bytes 4 and 2, so fi reads its array's fourth value, -60. With the array past the
// table the value is absent, kerning nothing, and kerx still stands in place of `kern`'s -40.
#[test]
fn a_kerx_whose_array_lies_past_it_kerns_nothing() {
    let (left, right) = (lookup(&[(F, 4)]), lookup(&[(I, 2)]));
    let end = 28 + left.len() + right.len();
    let format2 = |array: usize, values: &[i16]| {
        let offsets = u32s(&[4, 28, (28 + left.len()) as u32, array as u32]);
        let values = u16s(&values.iter().map(|&v| v as u16).collect::<Vec<_>>());
        kerx(2, [offsets, left.clone(), right.clone(), values].concat())
    };
    let sound = carets_with(&[("kerx", format2(end, &[0, 0, 0, -60])), ("kern", kern_pair())]);
    assert_eq!(sound.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [340.0, 400.0]);
    let past = carets_with(&[("kerx", format2(end + 2, &[])), ("kern", kern_pair())]);
    assert_eq!(past.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0; 2]);
    for f in [&sound, &past] {
        shapes_alike(f, "fi");
    }
}

// kerx is read at versions 2 to 4, the ones Apple's manual defines; from 3 its subtables' glyph coverage follows
// them, which where it's missing is absent and the subtables still read.
#[test]
fn kerx_is_read_at_versions_2_to_4() {
    let pair = [u32s(&[1, 0, 0, 0]), u16s(&[F, I, (-40i16) as u16])].concat();
    let (kerned, plain) = ([360.0, 400.0], [400.0; 2]);
    let cases = [
        (1, false, plain),
        (2, false, kerned),
        (3, false, kerned),
        (3, true, kerned),
        (4, true, kerned),
        (5, true, plain),
    ];
    for (version, coverage, advances) in cases {
        let mut table = kerx(0, pair.clone());
        table[..2].copy_from_slice(&u16::to_be_bytes(version));
        if coverage {
            table.extend(u32s(&[0]));
        }
        let f = carets_with(&[("kerx", table)]);
        let run = f.shape("fi", &Location::default(), &ShapeOptions::default());
        assert_eq!(run.advances, advances, "version {version}, coverage {coverage}");
        shapes_alike(&f, "fi");
    }
}

// A kerx whose one subtable is of an unknown format still stands in for `kern`, which it extends and beside which
// pairs would kern twice: fi keeps its advances.
#[test]
fn a_kerx_of_subtables_none_runs_still_takes_kern_down() {
    let f = carets_with(&[("kerx", kerx(3, Vec::new())), ("kern", kern_pair())]);
    assert_eq!(f.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0; 2]);
    shapes_alike(&f, "fi");
}
