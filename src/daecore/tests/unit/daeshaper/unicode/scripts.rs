use alloc::format;
use alloc::string::ToString;

use super::{t, Script};

#[test]
fn every_script_reads_back_from_its_iso_code() {
    for i in 0..t::SCRIPT_ISO_CODES.len() {
        let s = Script(i as u16);
        assert_eq!(s.iso15924().and_then(Script::from_iso15924), Some(s), "{:?}", s.iso15924());
        let code = s.iso15924().unwrap_or_default();
        for cased in [code.to_ascii_uppercase(), code.to_ascii_lowercase()] {
            assert_eq!(Script::from_iso15924(&cased), Some(s), "{cased}");
        }
    }
    assert_eq!(Script(t::SCRIPT_ISO_CODES.len() as u16).iso15924(), None);
    for none in ["Qaaa", "", "Lat", "Latnn", "L\u{e1}t", "Latn\0"] {
        assert_eq!(Script::from_iso15924(none), None, "{none:?}");
    }
}

#[test]
fn every_script_parses_from_its_name_and_its_code_and_displays_its_name() {
    for i in 0..t::SCRIPT_NAMES.len() {
        let s = Script(i as u16);
        let name = s.to_string();
        assert_eq!(name, s.name());
        assert_eq!(format!("{s:?}"), format!("Script({name:?})"));
        let code = s.iso15924().unwrap_or_default();
        for text in [name.clone(), name.to_ascii_uppercase(), code.to_string(), code.to_ascii_lowercase()] {
            assert_eq!(text.parse::<Script>(), Ok(s), "{text}");
        }
    }
    for none in ["", "Qaaa", "Klingon", "Arabic ", "Arab\0"] {
        let kind = none.parse::<Script>().map_err(|e| e.kind());
        assert_eq!(kind, Err(crate::daecore::ErrorKind::InvalidArgument), "{none:?}");
    }
}

// Each constant is the script its name and doc name: the first, the last, Common and one of two words.
#[test]
fn the_constants_are_the_scripts_they_name() {
    let named = [
        (Script::ADLAM, "Adlam", "Adlm"),
        (Script::COMMON, "Common", "Zyyy"),
        (Script::OLD_HUNGARIAN, "Old_Hungarian", "Hung"),
        (Script::ZANABAZAR_SQUARE, "Zanabazar_Square", "Zanb"),
    ];
    for (s, name, code) in named {
        assert_eq!((s.name(), s.iso15924()), (name, Some(code)));
    }
}

// The registry's tags: a script's ISO code lowercased unless it names another, a second and third version before it
// for the Indic scripts (Myanmar a second only), DFLT for scripts the text around them decides.
#[test]
fn a_scripts_opentype_tags_are_the_registrys_newest_first() {
    let tags = |s: Script| s.opentype_tags().map(|t| t.to_bytes()).collect::<alloc::vec::Vec<_>>();
    assert_eq!(tags(Script::DEVANAGARI), [*b"dev3", *b"dev2", *b"deva"]);
    assert_eq!(tags(Script::MYANMAR), [*b"mym2", *b"mymr"]);
    assert_eq!(tags(Script::LATIN), [*b"latn"]);
    assert_eq!(tags(Script::HIRAGANA), [*b"kana"]);
    assert_eq!(tags(Script::YI), [*b"yi  "]);
    assert_eq!(tags(Script::NKO), [*b"nko "]);
    assert_eq!(tags(Script::COMMON), [*b"DFLT"]);
    assert_eq!(tags(Script::INHERITED), [*b"DFLT"]);
    assert_eq!(tags(Script(u16::MAX)), [*b"DFLT"]);
    assert_eq!(Script::TAMIL.opentype_tags().next_back().map(|t| t.to_bytes()), Some(*b"taml"));
}
