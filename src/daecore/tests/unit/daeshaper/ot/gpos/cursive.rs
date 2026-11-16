use super::reroot;
use crate::daecore::daeshaper::run::{AttachKind, Placement};

fn hanging(links: [(i16, i32); 4]) -> [Placement; 4] {
    links.map(|(to, offset_y)| {
        let mut p = Placement { offset_y, ..Default::default() };
        (p.attach.to, p.attach.kind) = (to, AttachKind::Cursive);
        p
    })
}

// A glyph in a ring of 0 to 1 to 2 and back turns the ring's links around as far as the one that would close it, which
// is dropped: 0 becomes the root, 1 hangs from 0 and 2 from 1, each offset across negated from its old child's.
#[test]
fn a_cyclic_chain_turns_around_up_to_the_link_closing_it() {
    let mut places = hanging([(1, 10), (1, 20), (-2, 40), (0, 0)]);
    reroot(&mut places, 0, 3, true);
    assert_eq!(places.map(|p| (p.attach.to, p.offset_y)), [(0, 10), (-1, -10), (-1, -20), (0, 0)]);
}
