use super::*;

// In backward compatibility nothing moves along x, y moves until IUP has run on both axes, and a delta is kept only on
// a point touched in y or in a composite's own program; a font declaring native ClearType is held to none of it.
#[test]
fn backward_compatibility_keeps_x_and_settles_after_both_iups() {
    let mut rules = ClearType::new(false);
    assert!(!rules.moves_x() && rules.moves_y());
    let kept = [(false, false), (true, false), (false, true)].map(|(touched, composite)| {
        rules.keeps_delta(touched, composite)
    });
    assert_eq!(kept, [false, true, true]);
    rules.ran_iup(true);
    assert!(!rules.settled(), "settled after IUP on x alone");
    rules.ran_iup(false);
    assert!(rules.settled() && !rules.moves_y());
    assert!(!rules.keeps_delta(true, true), "a post-IUP delta was kept");

    let mut native = ClearType::new(true);
    native.ran_iup(true);
    native.ran_iup(false);
    assert!(native.moves_x() && native.moves_y() && !native.settled() && native.keeps_delta(false, false));
}
