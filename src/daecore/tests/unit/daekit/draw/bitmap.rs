use super::*;

// Rows read through the stride, empty past the last however long the slice; the owned copy packs them and
// lends them back as they were.
#[test]
fn rows_are_read_through_the_stride_and_packed_when_owned() {
    let pixels: Vec<u8> = (0..30).collect();
    let image = Image { left: -2, top: 7, width: 3, height: 3, stride: 10, channels: 2, pixels: &pixels };
    assert_eq!(image.row(0), [0, 1, 2, 3, 4, 5]);
    assert_eq!(image.row(2), [20, 21, 22, 23, 24, 25]);
    assert!(image.row(3).is_empty() && image.row(u32::MAX).is_empty());
    let shorter = Image { height: 2, ..image };
    assert!(shorter.row(2).is_empty(), "a row past the image reads its slice's spare bytes");
    let owned = image.to_bitmap();
    assert_eq!(owned.pixels, [0, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15, 20, 21, 22, 23, 24, 25]);
    assert_eq!((owned.left, owned.top, owned.width, owned.height, owned.channels), (-2, 7, 3, 3, 2));
    let lent = owned.image();
    assert_eq!(lent.stride, 6);
    assert!((0..3).all(|y| lent.row(y) == image.row(y)));
    assert_eq!(lent.to_bitmap(), owned);
}

// A lent slice too short for its rows reads them as empty rather than past its end.
#[test]
fn a_short_slice_reads_no_row_past_it() {
    let pixels = [1u8, 2, 3, 4];
    let image = Image { left: 0, top: 0, width: 2, height: 3, stride: 2, channels: 1, pixels: &pixels };
    assert_eq!((image.row(0), image.row(1), image.row(2)), (&[1, 2][..], &[3, 4][..], &[][..]));
    assert_eq!(image.to_bitmap().pixels, [1, 2, 3, 4]);
}

// Fields set past what a usize holds read as empty rows and copy what rows there are, on 64-bit and 32-bit alike.
#[test]
fn rows_past_what_a_usize_holds_read_as_empty() {
    let pixels = [7u8; 8];
    let huge = Image {
        left: 0,
        top: 0,
        width: u32::MAX,
        height: u32::MAX,
        stride: usize::MAX / 2,
        channels: 255,
        pixels: &pixels,
    };
    assert!(huge.row(0).is_empty() && huge.row(3).is_empty() && huge.row(u32::MAX).is_empty());
    let narrow = Image { width: 2, channels: 1, height: 3, ..huge };
    assert_eq!((narrow.row(0), narrow.row(1), narrow.row(2)), (&[7, 7][..], &[][..], &[][..]));
    let mut owned = narrow.to_bitmap();
    assert_eq!(owned.pixels, [7, 7]);
    owned.width = u32::MAX;
    owned.channels = 255;
    assert!(owned.image().row(0).is_empty());
}
