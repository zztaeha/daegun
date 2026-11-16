use super::*;

// The search finds each character's form in the generated table, and none for any other character.
#[test]
fn every_character_finds_its_vertical_form_and_no_other() {
    let mut forms = t::VERTICAL_FORMS.iter().peekable();
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        let listed = forms.next_if(|&&(key, _)| key == c).map(|&(_, form)| form);
        assert_eq!(vertical_form(c), listed, "U+{:04X}", u32::from(c));
    }
    assert!(forms.next().is_none(), "the table is sorted");
}

// A character upright only in a vertical form stands upright where it has one.
#[test]
fn a_transformed_rotated_character_is_upright_in_its_form() {
    assert!(is_upright('日', false));
    assert!(!is_upright('a', true));
    let rotated_unless_formed = '\u{3008}';
    assert_eq!(vertical_orientation(rotated_unless_formed), VerticalOrientation::TransformedRotated);
    assert_eq!([is_upright(rotated_unless_formed, true), is_upright(rotated_unless_formed, false)], [true, false]);
}
