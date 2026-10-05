pub(crate) const ARG_1_AND_2_ARE_WORDS:     u16 = 0x0001;
pub(crate) const ARGS_ARE_XY_VALUES:        u16 = 0x0002;
pub(crate) const WE_HAVE_A_SCALE:           u16 = 0x0008;
pub(crate) const MORE_COMPONENTS:           u16 = 0x0020;
pub(crate) const WE_HAVE_AN_X_AND_Y_SCALE:  u16 = 0x0040;
pub(crate) const WE_HAVE_A_TWO_BY_TWO:      u16 = 0x0080;
pub(crate) const SCALED_COMPONENT_OFFSET:   u16 = 0x0800;
pub(crate) const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;

// The bytes a component's scale takes, its flags read in the spec's order, WE_HAVE_A_SCALE first,
// as FreeType, HarfBuzz and fontTools read them.
pub(crate) fn scale_len(flags: u16) -> usize {
    if flags & WE_HAVE_A_SCALE != 0 {
        2
    } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
        4
    } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
        8
    } else {
        0
    }
}
