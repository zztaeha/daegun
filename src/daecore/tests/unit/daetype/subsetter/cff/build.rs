use super::*;

#[test]
fn an_index_past_65535_objects_is_refused() {
    assert!(encode_cff_index(&vec![vec![1]; 70_000]).is_none());
    assert_eq!(encode_cff_index(&vec![vec![1]; 65_535]).map(|i| (i[0], i[1])), Some((0xFF, 0xFF)));
}

// One-byte returns and longer bodies, at each offset width: the INDEX the chunk writer lays out.
#[test]
fn an_index_written_in_one_pass_is_the_chunk_writers() {
    for big in [0, 300, 70_000] {
        let body = vec![7u8; big];
        let objects: Vec<&[u8]> = vec![&[11], &body, &[], &[11], &[1, 2, 3]];
        let ends: Vec<usize> = objects.iter().scan(0, |end, o| { *end += o.len(); Some(*end) }).collect();
        let mut chunks = Vec::new();
        append_cff_index_chunks(&mut chunks, &objects, &ends).expect("five objects");
        assert_eq!(encode_cff_index_refs(&objects), Some(chunks), "{big}");
    }
    assert_eq!(encode_cff_index_refs(&[]), Some(vec![0, 0]));
}
