use super::io::{read_u16_be, read_i16_be};
use crate::daecore::daetype::Tables;

// fsSelection's bits. USE_TYPO_METRICS and OBLIQUE are read in any version: the spec reserves both before
// version 4, but fonts set them there.
pub const FS_ITALIC: u16 = 0x0001;
pub const FS_BOLD: u16 = 0x0020;
pub const FS_REGULAR: u16 = 0x0040;
pub const FS_USE_TYPO_METRICS: u16 = 0x0080;
pub const FS_OBLIQUE: u16 = 0x0200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Os2Metrics {
    pub subscript: [i16; 4],
    pub superscript: [i16; 4],
    pub y_strikeout_size: i16,
    pub y_strikeout_position: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Os2LineMetrics {
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Os2WinMetrics {
    pub ascent: u16,
    pub descent: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Os2Fields {
    pub version: u16,
    pub us_weight_class: Option<u16>,
    pub us_width_class: Option<u16>,
    pub fs_type: Option<u16>,
    pub metrics: Option<Os2Metrics>,
    pub s_family_class: Option<u16>,
    pub fs_selection: Option<u16>,
    pub sx_height: Option<i16>,
    pub s_cap_height: Option<i16>,
    pub line_metrics: Option<Os2LineMetrics>,
    pub win_metrics: Option<Os2WinMetrics>,
}

impl Os2Fields {
    pub fn use_typo_metrics(&self) -> bool {
        self.fs_selection.is_some_and(|v| v & FS_USE_TYPO_METRICS != 0)
    }
}

fn parse_metrics(os2: &[u8]) -> Option<Os2Metrics> {
    if os2.len() < 30 { return None; }
    Some(Os2Metrics {
        subscript: [
            read_i16_be(os2, 10)?, read_i16_be(os2, 12)?,
            read_i16_be(os2, 14)?, read_i16_be(os2, 16)?,
        ],
        superscript: [
            read_i16_be(os2, 18)?, read_i16_be(os2, 20)?,
            read_i16_be(os2, 22)?, read_i16_be(os2, 24)?,
        ],
        y_strikeout_size: read_i16_be(os2, 26)?,
        y_strikeout_position: read_i16_be(os2, 28)?,
    })
}

pub fn parse_os2(table_map: &Tables) -> Option<Os2Fields> {
    let os2 = table_map.get("OS/2")?;
    let version = read_u16_be(os2, 0)?;

    let metrics = parse_metrics(os2);

    let s_family_class = if os2.len() >= 32 { read_u16_be(os2, 30) } else { None };
    let fs_selection   = if os2.len() >= 64 { read_u16_be(os2, 62) } else { None };

    let (sx_height, s_cap_height) = if os2.len() >= 90 && version >= 2 {
        (read_i16_be(os2, 86), read_i16_be(os2, 88))
    } else {
        (None, None)
    };

    let line_metrics = if os2.len() >= 74 {
        Some(Os2LineMetrics {
            ascender: read_i16_be(os2, 68)?,
            descender: read_i16_be(os2, 70)?,
            line_gap: read_i16_be(os2, 72)?,
        })
    } else {
        None
    };
    let win_metrics = if os2.len() >= 78 {
        Some(Os2WinMetrics { ascent: read_u16_be(os2, 74)?, descent: read_u16_be(os2, 76)? })
    } else {
        None
    };

    Some(Os2Fields {
        version,
        us_weight_class: read_u16_be(os2, 4),
        us_width_class: read_u16_be(os2, 6),
        fs_type: read_u16_be(os2, 8),
        metrics,
        s_family_class,
        fs_selection,
        sx_height,
        s_cap_height,
        line_metrics,
        win_metrics,
    })
}
