use super::*;

// A message held as a static and one built at run time are the same error when their kind and text are.
#[test]
fn errors_are_equal_and_hash_alike_by_kind_and_text() {
    use core::hash::BuildHasher;
    let fixed = Error::malformed("glyf: truncated");
    let built = Error::owned(ErrorKind::Malformed, "glyf: truncated".into());
    assert_eq!(fixed, built);
    assert_ne!(fixed, Error::missing("glyf: truncated"));
    assert_ne!(fixed, Error::malformed("glyf: too short"));
    let state = std::hash::RandomState::new();
    assert_eq!(state.hash_one(&fixed), state.hash_one(&built));
}

#[cfg(feature = "std")]
#[test]
fn an_error_becomes_an_io_error_it_can_be_taken_back_from() {
    let io = std::io::Error::from(Error::invalid("tag: longer than four bytes"));
    assert_eq!(io.kind(), std::io::ErrorKind::InvalidInput);
    let back = io.into_inner().and_then(|e| e.downcast::<Error>().ok()).expect("the error as its source");
    assert_eq!(back.kind(), ErrorKind::InvalidArgument);
    assert_eq!(std::io::Error::from(Error::malformed("x")).kind(), std::io::ErrorKind::InvalidData);
}
