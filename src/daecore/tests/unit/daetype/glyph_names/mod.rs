use super::*;
use alloc::string::String;

fn names(post: Option<&[u8]>, cff: Option<&[u8]>, n: u16) -> Vec<Option<String>> {
    glyph_names(&NameIndex::new(post, cff), post, cff, n.into()).map(|n| n.map(String::from)).collect()
}

fn cff_name(cff: &[u8], gid: u32) -> Option<String> {
    glyph_name(&NameIndex::new(None, Some(cff)), None, Some(cff), 0, gid).map(String::from)
}

// A post 2.0 of `count` glyphs by these indices, then these bytes for its strings.
fn post(count: u16, indices: &[u16], strings: &[u8]) -> Vec<u8> {
    let mut post = alloc::vec![0u8; 32];
    post[..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    post.extend(count.to_be_bytes());
    indices.iter().for_each(|i| post.extend(i.to_be_bytes()));
    [post, strings.to_vec()].concat()
}

// A CFF of `n` glyphs, each endchar, with these strings, an empty Private DICT and a charset: its
// bytes, last in the table, or a predefined one by number.
fn cff(n: usize, strings: &[&[u8]], charset: Result<&[u8], u8>) -> Vec<u8> {
    let index = |items: &[&[u8]]| {
        let mut b = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return b;
        }
        b.push(4);
        let mut at = 1u32;
        b.extend(at.to_be_bytes());
        for i in items {
            at += i.len() as u32;
            b.extend(at.to_be_bytes());
        }
        items.iter().for_each(|i| b.extend(*i));
        b
    };
    let int = |v: usize| [&[29u8][..], &(v as i32).to_be_bytes()].concat();
    let (name, strs, gsubrs) = (index(&[b"T"]), index(strings), index(&[]));
    let charstrings = index(&alloc::vec![&[14u8][..]; n]);
    let top_index = index(&[&[0u8; 23][..]]).len();
    let body = 4 + name.len() + top_index + strs.len() + gsubrs.len();
    let (charset_bytes, charset_value) = match charset {
        Ok(bytes) => (bytes.to_vec(), body + charstrings.len()),
        Err(k) => (Vec::new(), usize::from(k)),
    };
    let mut top = int(charset_value);
    top.push(15);
    top.extend(int(body));
    top.push(17);
    top.extend([int(0), int(1)].concat());
    top.push(18);
    [alloc::vec![1, 0, 4, 4], name, index(&[&top]), strs, gsubrs, charstrings, charset_bytes].concat()
}

// One 100,000-byte string named by all 1,999 glyphs would be 200 MB of names; past post's 63 bytes
// a name counts as none.
#[test]
fn a_name_past_63_bytes_is_absent() {
    let long = alloc::vec![b'a'; 100_000];
    let mut charset = alloc::vec![0u8];
    (1..2000).for_each(|_| charset.extend(391u16.to_be_bytes()));
    let names = names(None, Some(&cff(2000, &[&long], Ok(&charset))), 2000);
    assert_eq!(names.iter().flatten().map(String::len).sum::<usize>(), ".notdef".len());
}

// A predefined charset smaller than the font: the CharStrings count runs past it, which makes the charset absent,
// so no glyph is named from it.
#[test]
fn a_predefined_charset_past_its_size_names_nothing() {
    let expert = cff(200, &[], Err(1));
    assert_eq!((cff_name(&expert, 5), names(None, Some(&expert), 200)[5].clone()), (None, None));
    let iso = cff(300, &[], Err(0));
    assert_eq!(cff_name(&iso, 229), None);
    assert_eq!(cff_name(&cff(200, &[], Err(0)), 34).as_deref(), Some("A"));
}

// A format 0 charset of one SID for three glyphs: what it names before the cut stays named.
#[test]
fn a_charset_cut_short_keeps_what_it_read() {
    let font = cff(3, &[], Ok(&[0, 0, 34]));
    assert_eq!(names(None, Some(&font), 3), [Some(".notdef".into()), Some("A".into()), None]);
    assert_eq!(cff_name(&font, 1).as_deref(), Some("A"));
}

// post names every glyph its own count covers, past maxp's too, and its strings until one runs off the table: the
// third string claims 5 bytes of the 2 left, so it and every index past it name nothing.
#[test]
fn post_names_to_its_own_count_and_up_to_a_string_cut_short() {
    let post = post(6, &[0, 258, 259, 260, 261, 36], b"\x01a\x02bb\x05cc");
    let index = NameIndex::new(Some(&post), None);
    let name = |gid| glyph_name(&index, Some(&post), None, 2, gid);
    assert_eq!((0..7).map(name).collect::<Vec<_>>(),
               [Some(".notdef"), Some("a"), Some("bb"), None, None, Some("A"), None]);
    assert_eq!(names(Some(&post), None, 2), [Some(".notdef".into()), Some("a".into())]);
}

// A post claiming 65,535 names in 40 bytes reads the three indices it holds and keeps no string list.
#[test]
fn a_post_claiming_more_than_it_holds_keeps_nothing_for_it() {
    let post = post(65_535, &[36, 37, 258], &[]);
    let index = NameIndex::new(Some(&post), None);
    assert_eq!(index.heap_bytes(), 0);
    assert_eq!((0..4).map(|g| glyph_name(&index, Some(&post), None, 4, g)).collect::<Vec<_>>(),
               [Some("A"), Some("B"), None, None]);
}

// A post 2.0 naming only glyph 0: the rest come from the CFF, as glyph_name gives them.
#[test]
fn names_post_leaves_out_come_from_the_cff() {
    let font = cff(10, &[], Err(0));
    let mut post = alloc::vec![0u8; 32];
    post[..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    post.extend([0, 1, 0, 0]);
    let names = names(Some(&post), Some(&font), 10);
    assert_eq!(names[5], cff_name(&font, 5));
    assert!(names[5].is_some());
}

// Version 1.0 names the 258 standard glyphs by number, and 2.5 by an offset into that order.
#[test]
fn post_versions_1_and_2_5_name_glyphs() {
    let mut v1 = alloc::vec![0u8; 32];
    v1[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    assert_eq!(names(Some(&v1), None, 258)[36].as_deref(), Some("A"));
    assert_eq!(names(Some(&v1), None, 259)[36], None, "1.0 is only for the 258 standard glyphs");

    let mut v25 = alloc::vec![0u8; 32];
    v25[..4].copy_from_slice(&0x0002_5000u32.to_be_bytes());
    v25.extend(3u16.to_be_bytes());
    v25.extend([0, 35, 35]);
    assert_eq!(names(Some(&v25), None, 3), [Some(".notdef".into()), Some("A".into()), Some("B".into())]);
}
