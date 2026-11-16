use super::*;
use crate::daecore::daetype::TableBytes;
use alloc::vec;

// An fvar with one axis, slnt, from -15 to 0.
fn slanted() -> Tables {
    let mut fvar = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
    fvar.extend(b"slnt");
    fvar.extend([0xFF, 0xF1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
    [("fvar".to_string(), TableBytes::from_vec(fvar))].into_iter().collect()
}

#[test]
fn the_slant_sets_the_italic_angle() {
    for (asked, angle) in [(-10.0, -10), (-20.0, -15)] {
        let mut post = vec![0u8; 32];
        apply_style_metadata(&slanted(), &[(AxisTag::new(b"slnt"), asked)], &mut [], &mut post, true);
        assert_eq!(i32::from_be_bytes(post[4..8].try_into().unwrap()), angle << 16, "asked for {asked}");
    }
}

#[test]
fn oblique_is_cleared_in_any_version_and_set_only_from_version_4() {
    for (version, stored, asked, want) in [
        (3, FS_OBLIQUE, 0.0, 0),
        (3, FS_OBLIQUE, -10.0, FS_OBLIQUE),
        (3, 0, -10.0, 0),
        (4, FS_OBLIQUE, 0.0, 0),
        (4, 0, -10.0, FS_OBLIQUE),
    ] {
        let mut os2 = vec![0u8; 96];
        os2[..2].copy_from_slice(&u16::to_be_bytes(version));
        os2[62..64].copy_from_slice(&stored.to_be_bytes());
        apply_style_metadata(&slanted(), &[(AxisTag::new(b"slnt"), asked)], &mut os2, &mut [0u8; 32], true);
        let oblique = read_u16_be(&os2, 62).unwrap() & FS_OBLIQUE;
        assert_eq!(oblique, want, "version {version}, stored {stored:#x}, slnt {asked}");
    }
}

// A unique ID of 1,000 "a"s whose full name is "a": the new full name of 2,001 bytes put in a thousand times would pass
// the 65,535 bytes a record holds, so it's passed over for the PostScript name, which fits.
#[test]
fn a_unique_id_replaced_past_a_record_s_length_takes_the_short_form() {
    let k = 1_000;
    let long: Vec<u8> = core::iter::repeat_n([0u8, b'a'], k).flatten().collect();
    let records: [(u16, u16); 5] = [(1, 2 * k as u16), (2, 2 * k as u16), (3, 2 * k as u16), (4, 2), (6, 2)];
    let mut t = [&[0u8, 0, 0, 5][..], &((6 + 12 * records.len()) as u16).to_be_bytes()].concat();
    for (id, len) in records {
        [3u16, 1, 0x409, id, len, 0].iter().for_each(|v| t.extend(v.to_be_bytes()));
    }
    t.extend(&long);
    let map: Tables = [("name".to_string(), TableBytes::from_vec(t))].into_iter().collect();
    let names = NameRecords::new(&map);
    let full = alloc::format!("{} {}", names.get(1).expect("a family"), names.get(2).expect("a subfamily"));
    assert_eq!(unique_id(&names, Some(&full), "PS", &[]), Some("PS".repeat(k)));
}
