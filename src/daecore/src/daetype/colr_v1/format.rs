use super::super::decoder::{read_i16_be, read_u16_be, read_u32_be, records_fit};

pub(crate) struct PaintLayout {
    pub(crate) inline_len: usize,
    pub(crate) children: &'static [usize],
    // The glyph a paint names: where it sits and its width, 24 bits in format 33.
    pub(crate) glyph: Option<(usize, usize)>,
}

const NONE: &[usize] = &[];
const CHILD_AT_1: &[usize] = &[1];

pub(crate) fn paint_layout(format: u8) -> Option<PaintLayout> {
    let (inline_len, children, glyph) = match format {
        1 => (6, NONE, None),
        2 => (5, NONE, None),
        3 => (9, NONE, None),
        4 => (16, NONE, None),
        5 => (20, NONE, None),
        6 => (16, NONE, None),
        7 => (20, NONE, None),
        8 => (12, NONE, None),
        9 => (16, NONE, None),
        10 => (6, CHILD_AT_1, Some((4, 2))),
        11 => (3, NONE, Some((1, 2))),
        12 => (7, CHILD_AT_1, None),
        13 => (7, CHILD_AT_1, None),
        14 => (8, CHILD_AT_1, None),
        15 => (12, CHILD_AT_1, None),
        16 => (8, CHILD_AT_1, None),
        17 => (12, CHILD_AT_1, None),
        18 => (12, CHILD_AT_1, None),
        19 => (16, CHILD_AT_1, None),
        20 => (6, CHILD_AT_1, None),
        21 => (10, CHILD_AT_1, None),
        22 => (10, CHILD_AT_1, None),
        23 => (14, CHILD_AT_1, None),
        24 => (6, CHILD_AT_1, None),
        25 => (10, CHILD_AT_1, None),
        26 => (10, CHILD_AT_1, None),
        27 => (14, CHILD_AT_1, None),
        28 => (8, CHILD_AT_1, None),
        29 => (12, CHILD_AT_1, None),
        30 => (12, CHILD_AT_1, None),
        31 => (16, CHILD_AT_1, None),
        32 => (8, &[1usize, 5][..], None),
        33 => (7, CHILD_AT_1, Some((4, 3))),
        _ => return None,
    };
    Some(PaintLayout { inline_len, children, glyph })
}

// A ClipBox's edges, x_min, y_min, x_max and y_max, and the variable format's varIndexBase: formats 1 and 2, as the
// spec defines them, any other unknown and absent.
pub(crate) fn clip_box_at(colr: &[u8], at: usize) -> Option<([i16; 4], Option<u32>)> {
    let edges = [1, 3, 5, 7].map(|k| at.checked_add(k).and_then(|o| read_i16_be(colr, o)));
    let [Some(x0), Some(y0), Some(x1), Some(y1)] = edges else { return None };
    let var = match *colr.get(at)? {
        1 => None,
        2 => Some(read_u32_be(colr, at.checked_add(9)?)?),
        _ => return None,
    };
    Some(([x0, y0, x1, y1], var))
}

// A ColorLine's stop count and the bytes each stop takes; None where its stops' count leaves the table.
pub(crate) fn color_line_stops(colr: &[u8], at: usize, is_var: bool) -> Option<(usize, usize)> {
    let count = usize::from(read_u16_be(colr, at.checked_add(1)?)?);
    let stride = if is_var { 10 } else { 6 };
    records_fit(at + 3, count, stride, colr.len()).then_some((count, stride))
}
