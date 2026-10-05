use super::*;

type Dir = BTreeMap<String, ttf_dir::TtfEntry>;
pub(crate) type Outlines<'a> = Option<(&'a [u8], Vec<usize>)>;

// The tables a closure pass reads, so its work is counted in their bytes.
const PASS_TABLES: [&str; 13] = [
    "GSUB", "COLR", "MATH", "morx", "sbix", "CBLC", "CBDT", "EBLC", "EBDT", "bloc", "bdat", "bsln", "fmtx",
];

// Past the 64 passes any table gets, a closure may spend this much more reading them.
const CLOSURE_FLOOR: usize = 1 << 24;

// Every glyph the requested ones reach, .notdef among them, in passes bounded by the bytes they read,
// as a chain moves one step a pass. `reach` inserts: the glyf walk drops gids past the font, CFF keeps them.
pub(crate) fn closure_loop<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]>,
    num_glyphs: u16,
    requested: &[u16],
    mut reach: impl FnMut(&[u16], &mut GlyphSet),
) -> Result<GlyphSet, String> {
    let mut active = GlyphSet::new();
    let seed: Vec<u16> = core::iter::once(0).chain(requested.iter().copied()).collect();
    reach(&seed, &mut active);

    let gsub = match table("GSUB") {
        Some(t) => Some(otl::gsub::GsubEdges::new(t).ok_or("subset: GSUB substitutions past the subset budget")?),
        None => None,
    };
    let pass_bytes = PASS_TABLES.iter().filter_map(|&tag| table(tag)).map(<[u8]>::len).sum::<usize>().max(1);
    let mut budget = pass_bytes.saturating_mul(64).saturating_add(CLOSURE_FLOOR);
    loop {
        budget = budget.checked_sub(pass_bytes).ok_or("subset: glyph closure past its work budget")?;
        let mut new_gids: Vec<u16> = Vec::new();
        if let Some(colr) = table("COLR") {
            new_gids.extend(colr::colr_closure(colr, &active));
        }
        if let Some(gsub) = &gsub {
            new_gids.extend(gsub.reach(&active));
        }
        if let Some(math) = table("MATH") {
            new_gids.extend(math::math_closure(math, &active));
        }
        if let Some(morx) = table("morx") {
            new_gids.extend(aat::morx::morx_closure(morx, &active, num_glyphs));
        }
        new_gids.extend(bitmap::bitmap_closure(&table, usize::from(num_glyphs), &active));
        new_gids.extend(aat::simple::metrics_glyphs(&table));
        // Done when a pass adds nothing: a gid found again is in the set or, past the font, refused by `reach`.
        new_gids.retain(|g| !active.contains(g));
        let before = active.len();
        reach(&new_gids, &mut active);
        if active.len() == before { break; }
    }
    Ok(active)
}

// A TrueType font's glyph count and outlines, when it has them.
pub(crate) fn glyf_outlines<'a>(ttf: &'a [u8], dir: &Dir) -> Result<(usize, Outlines<'a>), String> {
    let head = slice_table(ttf, dir, "head").ok_or("subset: missing head")?;
    let maxp = slice_table(ttf, dir, "maxp").ok_or("subset: missing maxp")?;
    let loca_fmt = read_i16_be(head, 50).ok_or("subset: head table truncated")?;
    let num_glyphs = read_u16_be(maxp, 4).ok_or("subset: maxp table truncated")? as usize;
    if num_glyphs == 0 {
        return Err("subset: maxp reports zero glyphs".into());
    }
    let outlines = match (slice_table(ttf, dir, "glyf"), slice_table(ttf, dir, "loca")) {
        (Some(glyf), Some(loca)) => Some((glyf, parse_loca(loca, loca_fmt, num_glyphs))),
        (None, None) => None,
        (Some(_), None) => return Err("subset: font has glyf but no loca".into()),
        (None, Some(_)) => return Err("subset: font has loca but no glyf".into()),
    };
    Ok((num_glyphs, outlines))
}

pub(crate) fn glyf_closure_of(
    ttf: &[u8], dir: &Dir, num_glyphs: usize, outlines: &Outlines, requested: &[u16],
) -> Result<GlyphSet, String> {
    closure_loop(|tag| slice_table(ttf, dir, tag), num_glyphs as u16, requested, |gids, set| match outlines {
        Some((glyf, loca)) => active_gids_into(gids, glyf, loca, num_glyphs, set),
        None => set.extend(gids.iter().copied().filter(|&g| usize::from(g) < num_glyphs)),
    })
}

pub fn glyf_closure(ttf: &[u8], requested: &[u16]) -> Result<GlyphSet, String> {
    let dir = parse_ttf_dir(ttf);
    let (num_glyphs, outlines) = glyf_outlines(ttf, &dir)?;
    glyf_closure_of(ttf, &dir, num_glyphs, &outlines, requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // One SingleSubst taking each of glyphs 1 to n to the next, which a pass follows one step of.
    fn chain(n: u16) -> Vec<u8> {
        [
            u16s(&[1, 0, 10, 30, 44]),
            u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
            u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0]),
            u16s(&[1, 4, 1, 0, 1, 8]),
            u16s(&[2, 6 + 2 * n, n]), (2..=n + 1).flat_map(u16::to_be_bytes).collect(),
            u16s(&[1, n]), (1..=n).flat_map(u16::to_be_bytes).collect(),
        ].concat()
    }

    fn close(gsub: &[u8]) -> Result<GlyphSet, String> {
        closure_loop(|tag| (tag == "GSUB").then_some(gsub), u16::MAX, &[1], |gids, set| gids.iter().for_each(|&g| { set.insert(g); }))
    }

    // A chain of 100 closes, past 64 passes; one of 30,000 in a 120 KB GSUB stops at the work budget,
    // about 200 passes in, rather than walking it 30,000 times.
    #[test]
    fn a_closure_runs_until_its_work_budget() {
        assert!(close(&chain(100)).is_ok_and(|s| s.contains(&101)));
        let err = close(&chain(30_000)).err().expect("the chain is past the budget");
        assert!(err.contains("work budget"), "{err}");
    }
}
