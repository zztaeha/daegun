use super::{drawn_fits, trimmed, Components};

// A simple glyph of 4 points and one instruction is cut after its coordinates, empty where its instruction length
// ends it or its flags miscount its points; a composite, a glyph of no contours or a short one stays as is.
#[test]
fn a_subset_copies_a_simple_glyph_cut_after_its_coordinates() {
    let head = [0u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3];
    let glyph = [&head[..], &[0, 1, 0xB0], &[0x37, 0x39, 1, 0x01], &[5, 0, 7, 6, 0, 8], &[0, 0, 0]].concat();
    assert_eq!(trimmed(&glyph), &glyph[..25]);
    assert!(trimmed(&[&head[..], &[0, 0]].concat()).is_empty());
    let mut over = glyph.clone();
    over[17] = 3;
    assert!(trimmed(&over).is_empty());
    assert!(trimmed(&glyph[..17]).is_empty() && trimmed(&glyph[..18]).is_empty());
    let composite = [0xFFu8, 0xFF, 1, 2, 3, 4, 5, 6, 7, 8, 9, 9];
    for kept in [&composite[..], &[0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[0, 1, 0]] {
        assert_eq!(trimmed(kept), kept);
    }
}

// In GLYF a component flagged GID_IS_24BIT names its glyph in 3 bytes, its arguments after; glyf's reader ignores
// the flag and reads 16 bits, as only GLYF defines it.
#[test]
fn a_24_bit_component_id_is_read_only_in_glyf_s_uppercase_twin() {
    let header = [0xFFu8, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
    let glyph = [&header[..], &[0x20, 0x22, 0x01, 0x23, 0x45, 5, 6], &[0, 3, 0, 7, 0xFF, 0xFF, 0, 2]].concat();
    let read = |extended| {
        let records = Components::new(&glyph, extended).map_while(Result::ok);
        records.map(|c| (c.gid, c.args, c.gid_len())).collect::<alloc::vec::Vec<_>>()
    };
    assert_eq!(read(true), [(0x01_2345, (5, 6), 3), (7, (-1, 2), 2)]);
    assert_eq!(read(false)[0], (0x0123, (0x45, 5), 2));
}

// An outline holds 65,535 points, 4 of them kept for the phantoms, and 65,535 contours.
#[test]
fn a_drawn_outline_holds_65535_points_and_contours() {
    assert!(drawn_fits((0, 0), 65_531, 1) && !drawn_fits((0, 0), 65_532, 1));
    assert!(drawn_fits((60_000, 0), 5_531, 1) && !drawn_fits((60_000, 0), 5_532, 1));
    assert!(drawn_fits((0, 0), 10, 65_535) && !drawn_fits((0, 0), 10, 65_536));
    assert!(drawn_fits((0, 60_000), 10, 5_535) && !drawn_fits((0, 60_001), 10, 5_535));
}

// Glyphs 1 to 101 a chain, each of 20,000 components naming the next: each glyph met spends a visit, and once they're
// spent no component is read, so the walk is bounded by MAX_VISITS rather than taking 20,000 reads a visit.
#[test]
fn an_overlap_walk_stops_reading_once_its_visits_are_spent() {
    use super::{overlap_flagged, LocaRef, MAX_VISITS, MORE_COMPONENTS};
    let composite = |next: u16, count: usize| {
        let mut g = alloc::vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        for i in 0..count {
            let flags = if i + 1 < count { MORE_COMPONENTS } else { 0 };
            g.extend([flags.to_be_bytes(), next.to_be_bytes(), [0, 0]].concat());
        }
        g
    };
    let (mut glyf, mut loca) = (composite(1, 1), alloc::vec![0, 0]);
    loca[1] = glyf.len();
    for g in 1..=101u16 {
        glyf.extend(composite(g + 1, 20_000));
        loca.push(glyf.len());
    }
    let mut visits = MAX_VISITS;
    assert!(!overlap_flagged(&glyf, LocaRef::Offsets(&loca), 0, false, &mut visits));
    assert_eq!(visits, 0);
}

// A chain of 60,000 composites over a glyph flagged as overlapping: the flag is read at the bottom.
#[test]
fn an_overlap_flag_is_read_as_deep_as_composites_go() {
    use super::{overlap_flagged, LocaRef, MAX_VISITS, OVERLAP_SIMPLE};
    let (mut glyf, mut loca) = (alloc::vec::Vec::new(), alloc::vec![0]);
    for next in 1..=60_000u16 {
        glyf.extend([0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
        glyf.extend(next.to_be_bytes());
        glyf.extend([0, 0]);
        loca.push(glyf.len());
    }
    glyf.extend([0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1 | OVERLAP_SIMPLE, 0, 0, 0, 0]);
    loca.push(glyf.len());
    let mut visits = MAX_VISITS;
    assert!(overlap_flagged(&glyf, LocaRef::Offsets(&loca), 0, false, &mut visits));
}
