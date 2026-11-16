// daegun's rule: x stays as drawn unless the font declares native ClearType (INSTCTRL selector 3, from Microsoft's
// ClearType page), so advances and subpixel positions match the unhinted outline.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct ClearType {
    pub native: bool,
    // Whether IUP has run on each axis in this glyph program.
    iup_x: bool,
    iup_y: bool,
}

impl ClearType {
    pub(crate) fn new(native: bool) -> ClearType {
        ClearType { native, iup_x: false, iup_y: false }
    }

    pub(crate) fn moves_x(self) -> bool {
        self.native
    }

    // daegun's rule: post-IUP deltas fix bi-level pixel patterns that distort smooth text, so in backward
    // compatibility nothing moves or flips once IUP has run on both axes.
    pub(crate) fn settled(self) -> bool {
        !self.native && self.iup_x && self.iup_y
    }

    pub(crate) fn moves_y(self) -> bool {
        !self.settled()
    }

    pub(crate) fn ran_iup(&mut self, x_axis: bool) {
        if x_axis { self.iup_x = true } else { self.iup_y = true }
    }

    // DELTAP and SHPIX in backward compatibility, per Microsoft's ClearType page: kept on a point touched in y, which
    // a delta moves with its whole stroke rather than denting the outline, and in a composite's own program.
    pub(crate) fn keeps_delta(self, touched_y: bool, in_composite: bool) -> bool {
        self.native || (!self.settled() && (touched_y || in_composite))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/hinting/cleartype.rs"]
mod tests;
