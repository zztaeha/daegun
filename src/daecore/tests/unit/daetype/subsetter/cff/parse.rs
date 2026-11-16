use super::*;
use super::super::build::encode_cff_index;

// An INDEX's end from its last offset is where reading every object ends, for each offSize.
#[test]
fn an_index_ends_where_its_objects_do() {
    for len in [0, 10, 300, 70_000] {
        let mut data = vec![7u8; 3];
        data.extend(encode_cff_index(&[vec![1; len], vec![2; 5]]).expect("an INDEX"));
        data.extend([9; 4]);
        let end = cff_index_spans(&data, 3, false).expect("it parses").1;
        assert_eq!(cff_index_end(&data, 3, false), Ok(end));
        assert!(cff_index_end(&data[..end - 1], 3, false).is_err());
    }
    assert_eq!(cff_index_end(&[0, 0, 0, 0], 0, true), Ok(4));
}

// The kept glyphs' SIDs, and the errors, are those of the whole walk, over each format cut short at every byte,
// a last range past the glyph count, a SID that wraps, and glyph counts of 0 to 12.
#[test]
fn kept_sids_read_as_the_whole_walk() {
    let whole = |cff: &[u8], n: usize, kept: &[u32]| {
        let mut sids = alloc::vec![0u16; n.max(1)];
        walk_charset(cff, 0, n, |g, sid| sids[usize::from(g)] = sid)
        .map(|_| kept.iter().map(|&g| sids[g as usize]).collect::<Vec<_>>())
    };
    let charsets: [&[u8]; 4] = [
        &[0, 0, 5, 0, 6, 0, 9, 1, 0, 0, 7, 0, 8, 0, 9, 0, 10, 0, 11, 0, 12, 0, 13],
        &[1, 0, 20, 2, 1, 0, 3, 0xFF, 0xFE, 4],
        &[2, 0, 20, 0, 2, 1, 0, 0, 3, 0xFF, 0xFF, 0, 30],
        &[7, 1, 2, 3],
    ];
    for full in charsets {
        for cut in 0..=full.len() {
            let cff = &full[..cut];
            for n in 0..=12usize {
                let all: Vec<u32> = (1..n as u32).collect();
                let odd: Vec<u32> = all.iter().copied().filter(|g| g % 2 == 1).collect();
                for kept in [&all[..], &odd[..], &[]] {
                    let (got, want) = (kept_sids(cff, 0, n, kept), whole(cff, n, kept));
                    assert_eq!(got, want, "{full:?} cut at {cut}, {n} glyphs");
                }
            }
        }
    }
}
