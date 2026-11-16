use super::*;

fn private(bytes: &[u8]) -> Private {
    Private::read(bytes).expect("a Private DICT")
}

// An operand in 32.32 and the bytes it takes.
fn fixed(dict: &[u8]) -> Option<(i64, usize)> {
    operand(dict, 0).map(|(d, len)| (d.fixed(), len))
}

// TN 5176's two examples, -2.25 and 0.140541E-3, and BlueScale's default, each to the nearest 2^-32.
#[test]
fn a_real_reads_to_the_nearest_32_32() {
    assert_eq!(fixed(&[30, 0xe2, 0xa2, 0x5f]), Some((-9_663_676_416, 4)));
    assert_eq!(fixed(&[30, 0x0a, 0x14, 0x05, 0x41, 0xc3, 0xff]), Some((603_619, 7)));
    assert_eq!(fixed(&[30, 0x0a, 0x03, 0x96, 0x25, 0xff]), Some((BLUE_SCALE, 6)));
    assert_eq!(fixed(&[30, 0x1f]), Some((WHOLE, 2)));
    // 1E40 is past i64's 32.32 and 1E-40 under its least step.
    assert_eq!(fixed(&[30, 0x1b, 0x40, 0xff]), Some((i64::MAX, 4)));
    assert_eq!(fixed(&[30, 0x1c, 0x40, 0xff]), Some((0, 4)));
}

// A real's exponent is taken whole, however long its digits run: 1 then 1,021 zeros, E-1021, is 1, and E-1022 a tenth;
// an exponent of 30 nines is past any value.
#[test]
fn a_real_s_exponent_is_exact_however_long_its_digits() {
    let real = |exponent: &[u8]| [&[30, 0x10][..], &[0x00; 510], exponent].concat();
    assert_eq!(fixed(&real(&[0xc1, 0x02, 0x1f])), Some((WHOLE, 515)));
    assert_eq!(fixed(&real(&[0xc1, 0x02, 0x2f])), Some(((WHOLE + 5) / 10, 515)));
    let past = [&[30, 0x1b][..], &[0x99; 15], &[0xff]].concat();
    assert_eq!(fixed(&past), Some((i64::MAX, 18)));
}

// A reserved nibble, a minus after a digit, a second point, or a real the DICT cuts short doesn't read.
#[test]
fn a_malformed_real_doesnt_read() {
    for real in [&[30, 0x1d, 0xff][..], &[30, 0x1e, 0xff], &[30, 0x1a, 0x2a, 0x3f], &[30, 0x12]] {
        assert_eq!(fixed(real), None, "{real:x?}");
    }
}

// 3 over 0.0005 is 6,000 exactly, as a matrix's offset over its y scale must be; 1 over 3 rounds.
#[test]
fn a_decimal_divides_exactly() {
    let of = |bytes: &[u8]| operand(bytes, 0).expect("a number").0;
    let (three, five_ten_thousandths) = (of(&[142]), of(&[30, 0x5c, 0x4f]));
    assert_eq!(three.over(five_ten_thousandths), 6000 << 32);
    assert_eq!(Decimal::ONE.over(three), 1_431_655_765);
    assert_eq!(of(&[30, 0xe1, 0xff]).over(three), -1_431_655_765);
}

// BlueValues -15 0 500 sum their deltas to -15 -15 485, the lone last left out; BlueScale 0.039625 as a real reads as
// the default; BlueShift 6.
#[test]
fn delta_arrays_sum_and_pair() {
    let p = private(&[30, 0x0a, 0x03, 0x96, 0x25, 0xff, 12, 9, 124, 139, 248, 136, 6, 145, 12, 10]);
    assert_eq!(p.blue_values.get(), &[(-15 << 16, -15 << 16)]);
    assert_eq!((p.blue_scale, p.blue_shift), (Private::default().blue_scale, 6 << 16));
}

// A negative BlueShift or BlueFuzz is no distance; a large one, and a zone past 32,767 units, read as given.
#[test]
fn hint_distances_read_as_given_but_never_negative() {
    let p = private(&[28, 0x07, 0xd1, 12, 11, 135, 12, 10, 29, 0, 0, 0x80, 0, 139, 6]);
    assert_eq!((p.blue_fuzz, p.blue_shift), (2001 << 16, 0));
    assert_eq!(p.blue_values.get(), &[(0x8000 << 16, 0x8000 << 16)]);
}

// TN 5176 allows 48 operands before an operator; a reserved byte leaves the layout unknown.
#[test]
fn a_dictionary_past_the_operand_limit_or_with_a_reserved_byte_doesnt_read() {
    let mut many = alloc::vec![139; 48];
    many.push(6);
    assert!(Private::read(&many).is_some());
    many.insert(0, 139);
    assert!(Private::read(&many).is_none());
    assert!(Private::read(&[139, 22]).is_none());
    assert!(Private::read(&[139, 255, 0, 0, 0, 0]).is_none());
}

// BlueScale with no operand keeps its default; initialRandomSeed takes its integer's low 32 bits.
#[test]
fn a_number_field_without_its_operand_keeps_its_default() {
    let p = private(&[12, 9, 12, 10, 29, 0xff, 0xff, 0xff, 0xfb, 12, 19]);
    assert_eq!((p.blue_scale, p.blue_shift, p.random_seed), (BLUE_SCALE, 7 << 16, 0xffff_fffb));
}

// LanguageGroup reads as the integer it is, 0 where it's absent; 1 marks an ideographic FD (Type 1, 5.11).
#[test]
fn language_group_reads_as_an_integer_defaulting_to_0() {
    assert_eq!(private(&[140, 12, 17]).language_group, 1);
    assert_eq!(private(&[]).language_group, 0);
}

// BlueValues [-10 + 0.5 × 4, 0 + 0.5 × 2] blended at vsindex 0, then summed as a delta array.
#[test]
fn a_cff2_blend_weighs_each_default_by_its_regions() {
    let dict = [129, 139, 143, 141, 141, 23, 6];
    let read = PrivateDict::read(&dict, true, &mut |v| (v == 0).then_some(2)).expect("read");
    let p = read.at(Some(&[0x10000, 0x8000])).expect("weighed");
    assert_eq!(p.blue_values.get(), &[(-8 << 16, -7 << 16)]);
    assert!(read.at(None).is_none(), "a blend weighed without weights");
    assert!(PrivateDict::read(&dict, true, &mut |_| None).is_none(), "a vsindex the store lacks read");
}

// A blend whose operands include another's result, or a vsindex after a blend, is outside the CFF2 chapter.
#[test]
fn nested_blends_and_late_vsindex_dont_read() {
    let mut weights = |_| Some(2);
    assert!(PrivateDict::read(&[139, 140, 140, 23, 140, 140, 23, 6], true, &mut weights).is_none());
    assert!(PrivateDict::read(&[139, 140, 140, 23, 6, 139, 22], true, &mut weights).is_none());
    assert!(PrivateDict::read(&[140, 22, 139, 140, 140, 23, 6], true, &mut weights).is_some_and(|d| d.vsindex == 1));
}
