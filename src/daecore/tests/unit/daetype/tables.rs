use super::*;

// What only the tests change a font's tables with.
impl Tables {
    pub(crate) fn remove(&mut self, tag: &str) -> Option<TableBytes> {
        let i = self.find(tag_of(tag)?).ok()?;
        Some(self.0.remove(i).1)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&str, &mut TableBytes) -> bool) {
        self.0.retain_mut(|(tag, table)| keep(tag_str(tag), table));
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

// Through any run of inserts, removals and retains, the tables answer as a map of their tags does: the same table
// for each tag, the same one given back, and the same order, Debug included. A string of any other length is no tag.
#[test]
fn tables_answer_as_a_map_of_their_tags_does() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = |n: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % n
    };
    let tags = ["head", "HEAD", "cvt ", "CFF ", "CFF2", "OS/2", "glyf", "GLYF", "zzzz", "    ", "é12", "\u{7F}abc"];
    let table = |i: usize| TableBytes::from(alloc::vec![i as u8, (i >> 8) as u8]);
    let (mut tables, mut map) = (Tables::default(), BTreeMap::<String, TableBytes>::new());
    for step in 0..20_000 {
        let tag = tags[next(tags.len())];
        match next(10) {
            0..5 => assert_eq!(tables.insert(tag, table(step)), map.insert(tag.into(), table(step)), "{step}"),
            5..8 => assert_eq!(tables.remove(tag), map.remove(tag), "{step}"),
            8 => {
                let gone = tags[next(tags.len())];
                tables.retain(|t, _| t != gone);
                map.retain(|t, _| t != gone);
            }
            _ => {
                tables = map.iter().rev().map(|(t, v)| (t.clone(), v.clone())).collect();
                assert_eq!(tables.len(), map.len());
            }
        }
        for tag in tags {
            assert_eq!(tables.get(tag), map.get(tag), "{step}: {tag}");
            assert_eq!(tables.contains_key(tag), map.contains_key(tag));
        }
        assert!(tables.iter().eq(map.iter().map(|(t, v)| (t.as_str(), v))), "{step}");
        assert_eq!(format!("{tables:?}"), format!("{map:?}"));
    }
    for not_a_tag in ["", "hea", "heads", "cvt"] {
        assert_eq!(tables.get(not_a_tag), None);
    }
}

// A tag collected twice keeps its last table, as a map collects.
#[test]
fn a_repeated_tag_keeps_its_last_table() {
    let tables: Tables = [("glyf", TableBytes::from(alloc::vec![1])), ("glyf", TableBytes::from(alloc::vec![2]))]
        .into_iter()
        .collect();
    assert_eq!((tables.len(), tables["glyf"].as_slice()), (1, &[2u8][..]));
}
