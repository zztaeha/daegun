use super::*;

// In f32, 0.49999997 + 0.5 is 1, and 8,388,609 + 0.5 lands on 8,388,610.
#[test]
fn an_f32_rounds_with_the_half_added_in_double() {
    assert_eq!([0.499_999_97, -0.5, 2.5, 8_388_609.0].map(ot_round_f32), [0, 0, 3, 8_388_609]);
}
