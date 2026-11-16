// Darkening and gamma: what each does to the coverage it starts from.
use super::coverage::font;
use crate::color::Rgba;
use crate::daecore::daekit::draw::srgb::{coverage_table, from_linear, to_linear};
use crate::raster::{Bitmap, Gamma, RasterOptions, Rasterizer, StripeOrder, SubpixelLayout};
use crate::{Font, Location};
use alloc::vec::Vec;

fn draw(r: &mut Rasterizer, font: &Font, c: char, px: f32, options: &RasterOptions) -> Bitmap {
    let g = font.glyph_id(c).unwrap();
    r.glyph(font, g, &Location::default(), px, options).unwrap().to_bitmap()
}

fn ink(image: &Bitmap) -> u64 {
    image.pixels.iter().map(|&v| u64::from(v)).sum()
}

#[test]
fn gamma_maps_coverage_through_its_tables() {
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    let text = Rgba::opaque(200, 30, 90);
    for layout in [SubpixelLayout::grayscale(), SubpixelLayout::horizontal(StripeOrder::Bgr)] {
        for contrast in [0.0, 0.5] {
            let plain = RasterOptions::DEFAULT.with_layout(&layout);
            let exact = draw(&mut r, &font, 'g', 14.0, &plain);
            let corrected = draw(&mut r, &font, 'g', 14.0, &plain.with_gamma(Gamma::Srgb { text, contrast }));
            let tables = if layout.is_grayscale() {
                let y = 0.2126 * to_linear(200) + 0.7152 * to_linear(30) + 0.0722 * to_linear(90);
                vec![coverage_table(from_linear(y), contrast)]
            } else {
                [200, 30, 90].map(|v| coverage_table(v, contrast)).to_vec()
            };
            let channels = usize::from(exact.channels);
            for (i, (&v, &w)) in exact.pixels.iter().zip(&corrected.pixels).enumerate() {
                assert_eq!(tables[i % channels][usize::from(v)], w, "byte {i}, contrast {contrast}");
            }
        }
    }
}

#[test]
fn darkening_thickens_small_stems_and_leaves_large_ones() {
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    let (plain, dark) = (RasterOptions::DEFAULT, RasterOptions::DEFAULT.with_darken(true));
    for c in ['n', 'e', 'H'] {
        let (thin, thick) = (draw(&mut r, &font, c, 9.0, &plain), draw(&mut r, &font, c, 9.0, &dark));
        assert!(ink(&thick) > ink(&thin) * 11 / 10, "{c} at 9 px: {} against {}", ink(&thick), ink(&thin));
        let (thin, thick) = (draw(&mut r, &font, c, 64.0, &plain), draw(&mut r, &font, c, 64.0, &dark));
        assert_eq!(thin, thick, "{c} at 64 px, its stems past 2⅓ px");
    }
    // Inter's regular stem, measured across `l`: about 0.08 em.
    let stem = r.learned.last().and_then(|l| l.stem).unwrap();
    let em = f32::from(font.units_per_em());
    assert!((0.06 * em..0.1 * em).contains(&stem), "{stem} units");
}

// `l` maps to a glyph that doesn't decode and the font has no `I`: darkening measures the 0.075 em fallback and
// draws, rather than failing every glyph with `l`'s error.
#[test]
fn a_stem_glyph_that_wont_draw_is_passed_over() {
    let words = |w: &[i16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let broken = words(&[1, 0, 0, 10, 10]);
    let square = [words(&[1, 0, 0, 500, 500, 3, 0]), vec![1; 4], words(&[0, 500, 0, -500, 0, 0, 500, 0])].concat();
    let loca = [0, 0, broken.len(), broken.len() + square.len()].map(|o| (o as u32).to_be_bytes()).concat();
    let mut ascii = vec![0u8; 256];
    (ascii[usize::from(b'l')], ascii[usize::from(b'a')]) = (1, 2);
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> = [
        (*b"head", head),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 3]),
        (*b"loca", loca),
        (*b"glyf", [broken, square].concat()),
        (*b"cmap", [words(&[0, 1, 1, 0, 0, 12, 0, 262, 0]), ascii].concat()),
    ]
    .into_iter()
    .map(|(tag, t)| (crate::Tag::new(&tag), t))
    .collect();
    let font = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the font parses");
    assert_eq!(font.glyph_id('l'), Some(crate::GlyphId(1)));
    let mut r = Rasterizer::new();
    let at = |r: &mut Rasterizer, g: u32, o: &RasterOptions| {
        r.glyph(&font, crate::GlyphId(g), &Location::default(), 9.0, o).map(|_| ())
    };
    assert!(at(&mut r, 1, &RasterOptions::DEFAULT).is_err(), "l doesn't decode");
    assert!(at(&mut r, 2, &RasterOptions::DEFAULT.with_darken(true)).is_ok());
    assert_eq!(r.learned.last().and_then(|l| l.stem), Some(75.0));
}
