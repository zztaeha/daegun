use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::generic::schema::Schema;
use super::generic::Work;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be, write_u16_be, write_u32_be};

const EXTENSION_RECORD_LEN: usize = 8;

pub(crate) type SubtableSubsetter<'a> =
    &'a dyn Fn(u16, Option<&Schema>, &[u8], usize, &GlyphSet, &[u16], &mut Work) -> Option<Vec<u8>>;

// Subtables rebuilt in this pass by where they start and their type, kept once named again, so one
// that many lookups name is parsed at most twice, and the work that costs is spent across the table.
struct Pass {
    work: Work,
    rebuilt: alloc::collections::BTreeMap<(usize, u16), Option<Vec<u8>>>,
}
type SchemaForType<'a> = &'a dyn Fn(u16) -> Option<Schema>;

pub(crate) fn resolve_effective_type(buf: &[u8], ext_type: u16, lookup_type: u16, sub_off: usize) -> Option<(u16, usize)> {
    if lookup_type != ext_type { return Some((lookup_type, sub_off)); }
    let ext_format = read_u16_be(buf, sub_off)?;
    if ext_format != 1 { return None; }
    let real_type = read_u16_be(buf, sub_off + 2)?;
    if real_type == ext_type { return None; }
    let real_off = read_u32_be(buf, sub_off + 4)? as usize;
    Some((real_type, sub_off + real_off))
}

#[derive(Clone)]
struct RebuiltLookup {
    real_type: u16,
    flag: u16,
    mark_filtering_set: Option<u16>,
    subtables: Vec<Vec<u8>>,
}

impl RebuiltLookup {
    fn hollow(lookup_type: u16) -> Self {
        RebuiltLookup { real_type: lookup_type, flag: 0, mark_filtering_set: None, subtables: Vec::new() }
    }

    fn is_hollow(&self) -> bool { self.subtables.is_empty() }

    fn trailing(&self) -> usize { if self.flag & 0x0010 != 0 { 2 } else { 0 } }

    fn header_len(&self) -> usize { 6 + self.subtables.len() * 2 + self.trailing() }

    fn inline_len(&self) -> usize {
        if self.is_hollow() { return 6; }
        self.header_len() + self.subtables.iter().map(|t| t.len()).sum::<usize>()
    }

    fn promoted_header_len(&self) -> usize {
        if self.is_hollow() { return 6; }
        self.header_len() + self.subtables.len() * EXTENSION_RECORD_LEN
    }

    fn payload_len(&self) -> usize { self.subtables.iter().map(|t| t.len()).sum() }

    fn write_header_into(&self, out: &mut [u8], at: usize, declared_type: u16) {
        write_u16_be(out, at, declared_type);
        write_u16_be(out, at + 2, self.flag);
        write_u16_be(out, at + 4, self.subtables.len() as u16);
        if self.trailing() > 0 {
            write_u16_be(out, at + 6 + self.subtables.len() * 2, self.mark_filtering_set.unwrap_or(0));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild_one_lookup(
    buf: &[u8],
    lookup_start: usize,
    ext_type: u16,
    active: &GlyphSet,
    gid_map: &[u16],
    mark_sets: u16,
    subset_subtable: SubtableSubsetter,
    schema_for_type: SchemaForType,
    pass: &mut Pass,
) -> RebuiltLookup {
    let Some(lookup_type) = read_u16_be(buf, lookup_start) else { return RebuiltLookup::hollow(1) };
    let Some(orig_flag) = read_u16_be(buf, lookup_start + 2) else { return RebuiltLookup::hollow(lookup_type) };
    let Some(sub_count) = read_u16_be(buf, lookup_start + 4) else { return RebuiltLookup::hollow(lookup_type) };

    // A mark filtering set the subset's GDEF does not hold is dropped with its flag: OTS, which
    // browsers run on web fonts, refuses a lookup naming one.
    let mark_filtering_set = (orig_flag & 0x0010 != 0)
        .then(|| read_u16_be(buf, lookup_start + 6 + sub_count as usize * 2))
        .flatten()
        .filter(|&set| set < mark_sets);
    let flag = if mark_filtering_set.is_some() { orig_flag } else { orig_flag & !0x0010 };

    // A subtable named twice in one lookup applies where its first naming does, so later ones go.
    let mut effective_type: Option<u16> = None;
    let mut plan: Option<Option<Schema>> = None;
    let mut subtables: Vec<Vec<u8>> = Vec::new();
    let mut seen = alloc::collections::BTreeSet::new();
    for i in 0..sub_count as usize {
        let Some(rel) = read_u16_be(buf, lookup_start + 6 + i * 2) else { break };
        let Some((this_type, this_off)) = resolve_effective_type(buf, ext_type, lookup_type, lookup_start + rel as usize) else { continue };
        if !seen.insert(this_off) { continue; }
        if *effective_type.get_or_insert(this_type) != this_type { continue; }
        if plan.is_none() { plan = Some(schema_for_type(this_type)); }
        let schema = plan.as_ref().and_then(Option::as_ref);
        let rebuilt = match pass.rebuilt.get(&(this_off, this_type)) {
            Some(done) => done.clone(),
            None if pass.work.seen(this_off) => {
                let done = subset_subtable(this_type, schema, buf, this_off, active, gid_map, &mut pass.work);
                pass.rebuilt.insert((this_off, this_type), done.clone());
                done
            }
            None => subset_subtable(this_type, schema, buf, this_off, active, gid_map, &mut pass.work),
        };
        subtables.extend(rebuilt);
    }
    let (Some(real_type), false) = (effective_type, subtables.is_empty()) else {
        return RebuiltLookup::hollow(lookup_type);
    };

    RebuiltLookup { real_type, flag, mark_filtering_set, subtables }
}

#[allow(clippy::too_many_arguments)]
fn rebuild_lookups(
    buf: &[u8],
    lookup_list_off: usize,
    ext_type: u16,
    active: &GlyphSet,
    gid_map: &[u16],
    mark_sets: u16,
    subset_subtable: SubtableSubsetter,
    schema_for_type: SchemaForType,
) -> Option<Vec<RebuiltLookup>> {
    // A lookup named by many entries is rebuilt once and copied, and the copies stop at twice the table.
    let count = read_u16_be(buf, lookup_list_off)? as usize;
    let budget = crate::daecore::daetype::subsetter::subset_budget(buf.len());
    let (mut out, mut held): (Vec<RebuiltLookup>, usize) = (Vec::with_capacity(count), 0);
    let mut first_of = alloc::collections::BTreeMap::new();
    let mut pass = Pass { work: Work::for_table(buf.len()), rebuilt: alloc::collections::BTreeMap::new() };
    for i in 0..count {
        let rel = read_u16_be(buf, lookup_list_off + 2 + i * 2)?;
        let lookup = match first_of.get(&rel) {
            Some(&j) => RebuiltLookup::clone(&out[j]),
            None => {
                first_of.insert(rel, i);
                rebuild_one_lookup(
                    buf, lookup_list_off + rel as usize, ext_type, active, gid_map,
                    mark_sets, subset_subtable, schema_for_type, &mut pass,
                )
            }
        };
        held = held.saturating_add(lookup.inline_len());
        if held > budget { return None; }
        out.push(lookup);
    }
    Some(out)
}

fn assemble_inline(lookups: &[RebuiltLookup]) -> Option<Vec<u8>> {
    let header_len = 2 + lookups.len() * 2;
    let mut out = vec![0u8; header_len];
    write_u16_be(&mut out, 0, u16::try_from(lookups.len()).ok()?);

    let mut pos = header_len;
    for (i, lk) in lookups.iter().enumerate() {
        write_u16_be(&mut out, 2 + i * 2, u16::try_from(pos).ok()?);
        pos = pos.checked_add(lk.inline_len())?;
    }

    for lk in lookups {
        let at = out.len();
        out.resize(at + lk.header_len(), 0);
        if lk.is_hollow() {
            write_u16_be(&mut out, at, lk.real_type);
            continue;
        }
        lk.write_header_into(&mut out, at, lk.real_type);
        let mut sub_pos = lk.header_len();
        for (i, st) in lk.subtables.iter().enumerate() {
            write_u16_be(&mut out, at + 6 + i * 2, u16::try_from(sub_pos).ok()?);
            sub_pos = sub_pos.checked_add(st.len())?;
        }
        for st in &lk.subtables { out.extend_from_slice(st); }
    }
    Some(out)
}

fn assemble_promoted(lookups: &[RebuiltLookup], ext_type: u16) -> Option<Vec<u8>> {
    let list_header = 2 + lookups.len() * 2;

    let mut lookup_at = Vec::with_capacity(lookups.len());
    let mut pos = list_header;
    for lk in lookups {
        lookup_at.push(pos);
        pos = pos.checked_add(lk.promoted_header_len())?;
    }
    let payload_base = pos;

    let mut out = vec![0u8; payload_base];
    write_u16_be(&mut out, 0, u16::try_from(lookups.len()).ok()?);
    for (i, &at) in lookup_at.iter().enumerate() {
        write_u16_be(&mut out, 2 + i * 2, u16::try_from(at).ok()?);
    }

    let mut payload_pos = payload_base;
    for (lk, &at) in lookups.iter().zip(&lookup_at) {
        if lk.is_hollow() {
            write_u16_be(&mut out, at, lk.real_type);
            continue;
        }
        lk.write_header_into(&mut out, at, ext_type);
        let records_at = at + lk.header_len();
        for (i, st) in lk.subtables.iter().enumerate() {
            let record = records_at + i * EXTENSION_RECORD_LEN;
            write_u16_be(&mut out, at + 6 + i * 2, u16::try_from(record - at).ok()?);
            write_u16_be(&mut out, record, 1);
            write_u16_be(&mut out, record + 2, lk.real_type);
            write_u32_be(&mut out, record + 4, u32::try_from(payload_pos - record).ok()?);
            payload_pos = payload_pos.checked_add(st.len())?;
        }
    }

    out.reserve(lookups.iter().map(RebuiltLookup::payload_len).sum());
    for lk in lookups {
        for st in &lk.subtables { out.extend_from_slice(st); }
    }
    Some(out)
}

// The subset lookups after the ScriptList and FeatureList, trimmed or rebuilt, then FeatureVariations
// moved whole; a table not understood that far keeps all its source ahead of the lookups.
#[allow(clippy::too_many_arguments)]
pub(crate) fn subset_lookup_table(
    buf: &[u8],
    ext_type: u16,
    active: &GlyphSet,
    gid_map: &[u16],
    mark_sets: u16,
    subset_subtable: SubtableSubsetter,
    schema_for_type: SchemaForType,
) -> Option<Vec<u8>> {
    if buf.len() < 10 { return None; }
    let lookup_off = read_u16_be(buf, 8)? as usize;
    let lookups = rebuild_lookups(
        buf, lookup_off, ext_type, active, gid_map, mark_sets, subset_subtable, schema_for_type,
    )?;

    let variations = super::feature_variations(buf);
    let new_lookup_off = match variations {
        Some(_) => super::layout_live_prefix_len(buf).unwrap_or(buf.len()),
        None => buf.len(),
    };
    if new_lookup_off > 0xFFFF { return None; }

    let fits = |list: &Vec<u8>| new_lookup_off + list.len() <= 0xFFFF;
    let list = match assemble_inline(&lookups).filter(fits) {
        Some(inline) => inline,
        None => assemble_promoted(&lookups, ext_type)?,
    };

    // FeatureVariations names features by index, so with one the features keep theirs.
    let live: Vec<bool> = lookups.iter().map(|l| !l.subtables.is_empty()).collect();
    let rebuilt = match variations {
        Some(None) => rebuild_prefix(buf, &live).filter(|p| p.len() <= new_lookup_off),
        _ => None,
    };
    let (mut out, moved) = match (rebuilt, variations.flatten()) {
        (Some(prefix), _) => (prefix, None),
        (None, Some(fv)) if fv.end <= new_lookup_off => (buf.get(..new_lookup_off)?.to_vec(), None),
        (None, fv) => (buf.get(..new_lookup_off)?.to_vec(), fv),
    };
    let lookup_at = u16::try_from(out.len()).ok()?;
    write_u16_be(&mut out, 8, lookup_at);
    out.extend_from_slice(&list);
    if let Some(fv) = moved {
        let at = u32::try_from(out.len()).ok()?;
        write_u32_be(&mut out, 10, at);
        out.extend_from_slice(buf.get(fv)?);
    }
    Some(out)
}

struct LangSys {
    tag: u32,
    features: Vec<u16>,
    required: Option<u16>,
}

struct Script {
    tag: u32,
    default_features: Option<Vec<u16>>,
    default_required: Option<u16>,
    langs: Vec<LangSys>,
}

// Features the shaper asks after by presence alone, kept with no lookup: a GPOS kern feature turns
// the legacy kern table off, and Indic base finding reads pref's mask, as HarfBuzz never drops pref.
const STEERING: [[u8; 4]; 2] = [*b"kern", *b"pref"];

// A version 1.0 header, ScriptList and FeatureList for a table with no FeatureVariations, the features
// with no live lookup left out but those that steer shaping; None when every feature stays.
fn rebuild_prefix(buf: &[u8], live: &[bool]) -> Option<Vec<u8>> {
    super::layout_header_len(buf)?;
    let script_off = read_u16_be(buf, 4)? as usize;
    let feature_off = read_u16_be(buf, 6)? as usize;
    let count = |off: usize| if off == 0 { Some(0) } else { read_u16_be(buf, off).map(usize::from) };

    let feature_count = count(feature_off)?;
    let mut kept: Vec<(u32, Vec<u16>, Option<Vec<u8>>)> = Vec::new();
    let mut remap: Vec<Option<u16>> = alloc::vec![None; feature_count];
    for (i, slot) in remap.iter_mut().enumerate() {
        let rec = feature_off + 2 + i * 6;
        let tag = read_u32_be(buf, rec)?;
        let f = feature_off + read_u16_be(buf, rec + 4)? as usize;
        let params_rel = read_u16_be(buf, f)? as usize;
        let n_idx = read_u16_be(buf, f + 2)? as usize;

        let mut indices: Vec<u16> = Vec::new();
        for k in 0..n_idx {
            let idx = read_u16_be(buf, f + 4 + k * 2)?;
            if live.get(idx as usize).copied().unwrap_or(false) {
                indices.push(idx);
            }
        }
        let params = if params_rel == 0 {
            None
        } else {
            let at = f + params_rel;
            let end = super::feature_params_extent_at(buf, rec, at)?;
            Some(buf.get(at..end)?.to_vec())
        };
        if indices.is_empty() && params.is_none() && !STEERING.contains(&tag.to_be_bytes()) { continue; }
        *slot = Some(u16::try_from(kept.len()).ok()?);
        kept.push((tag, indices, params));
    }
    if kept.len() == feature_count { return None; }

    let script_count = count(script_off)?;
    let mut scripts: Vec<Script> = Vec::new();
    for i in 0..script_count {
        let rec = script_off + 2 + i * 6;
        let tag = read_u32_be(buf, rec)?;
        let s = script_off + read_u16_be(buf, rec + 4)? as usize;
        let default_rel = read_u16_be(buf, s)? as usize;
        let lang_count = read_u16_be(buf, s + 2)? as usize;

        let read_langsys = |at: usize| -> Option<(Vec<u16>, Option<u16>)> {
            let required = read_u16_be(buf, at + 2)?;
            let n = read_u16_be(buf, at + 4)? as usize;
            let mut out = Vec::new();
            for k in 0..n {
                let old = read_u16_be(buf, at + 6 + k * 2)? as usize;
                if let Some(Some(new)) = remap.get(old) { out.push(*new); }
            }
            let new_required = match required {
                0xFFFF => None,
                r => remap.get(r as usize).copied().flatten(),
            };
            Some((out, new_required))
        };

        let (default_features, default_required) = if default_rel == 0 {
            (None, None)
        } else {
            let (f, r) = read_langsys(s + default_rel)?;
            (Some(f), r)
        };
        let mut langs = Vec::new();
        for j in 0..lang_count {
            let lrec = s + 4 + j * 6;
            let ltag = read_u32_be(buf, lrec)?;
            let at = s + read_u16_be(buf, lrec + 4)? as usize;
            let (f, r) = read_langsys(at)?;
            langs.push(LangSys { tag: ltag, features: f, required: r });
        }
        scripts.push(Script { tag, default_features, default_required, langs });
    }

    let mut script_list: Vec<u8> = Vec::new();
    script_list.extend_from_slice(&u16::try_from(scripts.len()).ok()?.to_be_bytes());
    let mut script_bodies: Vec<u8> = Vec::new();
    let script_records_len = 2 + scripts.len() * 6;
    for Script { tag, default_features, default_required, langs } in &scripts {
        let body_at = script_records_len + script_bodies.len();
        script_list.extend_from_slice(&tag.to_be_bytes());
        script_list.extend_from_slice(&u16::try_from(body_at).ok()?.to_be_bytes());

        let header_len = 4 + langs.len() * 6;
        let mut body: Vec<u8> = Vec::new();
        let mut tail: Vec<u8> = Vec::new();
        let push_langsys = |tail: &mut Vec<u8>, features: &[u16], required: Option<u16>| -> usize {
            let at = header_len + tail.len();
            tail.extend_from_slice(&0u16.to_be_bytes());
            tail.extend_from_slice(&required.unwrap_or(0xFFFF).to_be_bytes());
            tail.extend_from_slice(&(features.len() as u16).to_be_bytes());
            for f in features { tail.extend_from_slice(&f.to_be_bytes()); }
            at
        };
        let default_at = default_features.as_ref()
            .map(|f| push_langsys(&mut tail, f, *default_required));
        body.extend_from_slice(&u16::try_from(default_at.unwrap_or(0)).ok()?.to_be_bytes());
        body.extend_from_slice(&u16::try_from(langs.len()).ok()?.to_be_bytes());
        for LangSys { tag: ltag, features, required } in langs {
            let at = push_langsys(&mut tail, features, *required);
            body.extend_from_slice(&ltag.to_be_bytes());
            body.extend_from_slice(&u16::try_from(at).ok()?.to_be_bytes());
        }
        body.extend_from_slice(&tail);
        script_bodies.extend_from_slice(&body);
    }
    script_list.extend_from_slice(&script_bodies);

    let mut feature_list: Vec<u8> = Vec::new();
    feature_list.extend_from_slice(&u16::try_from(kept.len()).ok()?.to_be_bytes());
    let feature_records_len = 2 + kept.len() * 6;
    let mut feature_bodies: Vec<u8> = Vec::new();
    for (tag, indices, params) in &kept {
        let body_at = feature_records_len + feature_bodies.len();
        feature_list.extend_from_slice(&tag.to_be_bytes());
        feature_list.extend_from_slice(&u16::try_from(body_at).ok()?.to_be_bytes());

        let own_len = 4 + indices.len() * 2;
        feature_bodies.extend_from_slice(&u16::try_from(params.as_ref().map_or(0, |_| own_len)).ok()?.to_be_bytes());
        feature_bodies.extend_from_slice(&u16::try_from(indices.len()).ok()?.to_be_bytes());
        for idx in indices { feature_bodies.extend_from_slice(&idx.to_be_bytes()); }
        if let Some(p) = params { feature_bodies.extend_from_slice(p); }
    }
    feature_list.extend_from_slice(&feature_bodies);

    let mut out: Vec<u8> = alloc::vec![0u8; 10];
    write_u16_be(&mut out, 0, 1);
    let script_at = 10usize;
    let feature_at = script_at + script_list.len();
    write_u16_be(&mut out, 4, u16::try_from(script_at).ok()?);
    write_u16_be(&mut out, 6, u16::try_from(feature_at).ok()?);
    out.extend_from_slice(&script_list);
    out.extend_from_slice(&feature_list);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    // kern, liga and pref each name one SingleSubst of glyph 5, which the subset drops: kern and pref
    // stay with no lookup, liga goes.
    #[test]
    fn features_that_steer_shaping_stay_without_lookups() {
        let u16s = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let gsub = [
            u16s(&[1, 0, 10, 34, 72, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 3, 0, 1, 2]),
            u16s(&[3]), b"kern".to_vec(), u16s(&[20]), b"liga".to_vec(), u16s(&[26]), b"pref".to_vec(), u16s(&[32]),
            u16s(&[0, 1, 0, 0, 1, 0, 0, 1, 0]),
            u16s(&[1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]),
        ].concat();
        let mut active = GlyphSet::new();
        [0, 1].into_iter().for_each(|g| { active.insert(g); });
        let out = super::super::gsub::subset_gsub(&gsub, &active, &[0, 1], 0).expect("a GSUB");
        let list = usize::from(read_u16_be(&out, 6).expect("FeatureList"));
        let tags: Vec<&[u8]> = (0..usize::from(read_u16_be(&out, list).expect("count"))).map(|i| &out[list + 2 + 6 * i..list + 6 + 6 * i]).collect();
        assert_eq!(tags, [&b"kern"[..], &b"pref"[..]]);
    }

    // Ten lookups each naming one subtable: it is rebuilt at its first naming and kept at its
    // second, and the rest reuse it.
    #[test]
    fn a_subtable_many_lookups_name_is_rebuilt_at_most_twice() {
        let n = 10u16;
        let lookup_at = |i: u16| 2 + 2 * n + 8 * i;
        let sub_at = lookup_at(n);
        let mut buf: Vec<u8> = n.to_be_bytes().to_vec();
        (0..n).for_each(|i| buf.extend(lookup_at(i).to_be_bytes()));
        for i in 0..n {
            buf.extend([1, 0, 1, sub_at - lookup_at(i)].iter().flat_map(|v: &u16| v.to_be_bytes()));
        }
        buf.extend([0; 4]);
        let calls = Cell::new(0);
        let subset = |_: u16, _: Option<&Schema>, _: &[u8], _: usize, _: &GlyphSet, _: &[u16], _: &mut Work| {
            calls.set(calls.get() + 1);
            Some(alloc::vec![0; 4])
        };
        let lookups = rebuild_lookups(&buf, 0, 7, &GlyphSet::new(), &[], 0, &subset, &|_| None).expect("lookups");
        assert!(lookups.len() == 10 && lookups.iter().all(|l| l.subtables.len() == 1));
        assert!(calls.get() <= 2, "rebuilt {} times", calls.get());
    }
}
