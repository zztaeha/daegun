// ISOAdobe, Expert and Expert Subset cover 229, 166 and 87 glyphs. A font with more refers to names
// its charset does not have, which makes it invalid, so none of its glyphs is named from it.
const ISO_ADOBE_GLYPHS: usize = 229;

// A predefined charset's SID for a glyph: 0 is ISOAdobe, where a glyph's SID is its index, and 1 and 2
// the Expert tables below.
pub(crate) fn predefined_charset_sid(id: u8, gid: u32, n_glyphs: usize) -> Option<u16> {
    let table: &[u16] = match id {
        1 => &EXPERT_CHARSET,
        2 => &EXPERT_SUBSET_CHARSET,
        _ => return (n_glyphs <= ISO_ADOBE_GLYPHS && (gid as usize) < ISO_ADOBE_GLYPHS).then_some(gid as u16),
    };
    if n_glyphs > table.len() {
        return None;
    }
    table.get(gid as usize).copied()
}

const EXPERT_CHARSET: [u16; 166] = [
    0, 1, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238,
    13, 14, 15, 99, 239, 240, 241, 242, 243, 244, 245, 246,
    247, 248, 27, 28, 249, 250, 251, 252, 253, 254, 255, 256,
    257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 109, 110,
    267, 268, 269, 270, 271, 272, 273, 274, 275, 276, 277, 278,
    279, 280, 281, 282, 283, 284, 285, 286, 287, 288, 289, 290,
    291, 292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302,
    303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314,
    315, 316, 317, 318, 158, 155, 163, 319, 320, 321, 322, 323,
    324, 325, 326, 150, 164, 169, 327, 328, 329, 330, 331, 332,
    333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344,
    345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356,
    357, 358, 359, 360, 361, 362, 363, 364, 365, 366, 367, 368,
    369, 370, 371, 372, 373, 374, 375, 376, 377, 378,
];

const EXPERT_SUBSET_CHARSET: [u16; 87] = [
    0, 1, 231, 232, 235, 236, 237, 238, 13, 14, 15, 99,
    239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 27, 28,
    249, 250, 251, 253, 254, 255, 256, 257, 258, 259, 260, 261,
    262, 263, 264, 265, 266, 109, 110, 267, 268, 269, 270, 272,
    300, 301, 302, 305, 314, 315, 158, 155, 163, 320, 321, 322,
    323, 324, 325, 326, 150, 164, 169, 327, 328, 329, 330, 331,
    332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343,
    344, 345, 346,
];
