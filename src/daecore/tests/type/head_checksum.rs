use daegun::Font;

// The whole-file rule alone passes even with every directory entry wrong, since the file sum
// subtracts the stale adjustment back out: both rules are checked, or the two errors cancel.
fn checksum32(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 4 <= data.len() {
        sum = sum.wrapping_add(u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]));
        i += 4;
    }
    if i < data.len() {
        let mut tail = [0u8; 4];
        tail[..data.len() - i].copy_from_slice(&data[i..]);
        sum = sum.wrapping_add(u32::from_be_bytes(tail));
    }
    sum
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

struct Checked {
    directory_entry: bool,
    whole_file:      bool,
}

fn check(ttf: &[u8]) -> Option<Checked> {
    let num_tables = u16::from_be_bytes([ttf[4], ttf[5]]) as usize;
    let dir_size = num_tables * 16;
    let mut head: Option<(usize, usize, u32)> = None;
    for i in 0..num_tables {
        let e = 12 + i * 16;
        if &ttf[e..e + 4] == b"head" {
            head = Some((be32(ttf, e + 8) as usize, be32(ttf, e + 12) as usize, be32(ttf, e + 4)));
        }
    }
    let (off, len, stored) = head?;
    if len < 12 { return None }

    let mut zeroed = ttf[off..off + len].to_vec();
    zeroed[8..12].fill(0);

    let mut file = ttf.to_vec();
    file[off + 8..off + 12].fill(0);
    let mut sum = checksum32(&file[..12 + dir_size]);
    for i in 0..num_tables {
        let e = 12 + i * 16;
        let (o, l) = (be32(ttf, e + 8) as usize, be32(ttf, e + 12) as usize);
        sum = sum.wrapping_add(checksum32(&file[o..o + l]));
    }

    Some(Checked {
        directory_entry: stored == checksum32(&zeroed),
        whole_file:      be32(ttf, off + 8) == 0xB1B0_AFBA_u32.wrapping_sub(sum),
    })
}

#[test]
fn head_checksums_hold_for_every_test_font() {
    let mut checked = 0;
    let mut bad_entry = Vec::new();
    let mut bad_file = Vec::new();

    let mut stack = vec![std::path::PathBuf::from(crate::FONTS)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("font dir reads").flatten() {
            let path = entry.path();
            if path.is_dir() { stack.push(path); continue }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "ttf" && ext != "otf" { continue }

            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(font) = Font::from_bytes(&bytes) else { continue };
            let Ok(sub) = font.subset_text("Hamburgefonstiv 123", &[]) else { continue };
            let Some(r) = check(&sub.ttf) else { continue };

            checked += 1;
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !r.directory_entry { bad_entry.push(name.clone()) }
            if !r.whole_file { bad_file.push(name) }
        }
    }

    assert!(checked > 0, "no fonts were checked");
    assert!(bad_entry.is_empty(), "{}/{checked} wrong head directory entry: {bad_entry:?}", bad_entry.len());
    assert!(bad_file.is_empty(), "{}/{checked} wrong checkSumAdjustment: {bad_file:?}", bad_file.len());
}
