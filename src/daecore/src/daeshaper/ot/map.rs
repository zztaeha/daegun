use alloc::vec::Vec;

use crate::daecore::daeshaper::buffer::glyph_flag;
use super::LayoutTable;
use super::tag::Tag;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TableIndex {
    Gsub = 0,
    Gpos = 1,
}

impl TableIndex {
    pub(crate) const ALL: [TableIndex; 2] = [TableIndex::Gsub, TableIndex::Gpos];

    pub(crate) fn idx(self) -> usize {
        self as usize
    }
}

pub(crate) mod feature_flags {
    pub(crate) const NONE: u32 = 0x0000;
    pub(crate) const GLOBAL: u32 = 0x0001;
    pub(crate) const HAS_FALLBACK: u32 = 0x0002;
    pub(crate) const MANUAL_ZWNJ: u32 = 0x0004;
    pub(crate) const MANUAL_ZWJ: u32 = 0x0008;
    pub(crate) const MANUAL_JOINERS: u32 = MANUAL_ZWNJ | MANUAL_ZWJ;
    pub(crate) const GLOBAL_MANUAL_JOINERS: u32 = GLOBAL | MANUAL_JOINERS;
    pub(crate) const GLOBAL_HAS_FALLBACK: u32 = GLOBAL | HAS_FALLBACK;
    pub(crate) const GLOBAL_SEARCH: u32 = 0x0010;
    pub(crate) const RANDOM: u32 = 0x0020;
    pub(crate) const PER_SYLLABLE: u32 = 0x0040;
}

pub(crate) const MAX_BITS: u32 = 8;
pub(crate) const MAX_VALUE: u32 = (1 << MAX_BITS) - 1;

const GLOBAL_BIT_SHIFT: u32 = 31;
const GLOBAL_BIT_MASK: u32 = 1 << GLOBAL_BIT_SHIFT;

fn first_free_bit() -> u32 {
    glyph_flag::DEFINED.count_ones() + 1
}

#[derive(Clone, Copy, Debug)]
struct FeatureInfo {
    tag: Tag,
    seq: usize,
    max_value: u32,
    flags: u32,
    default_value: u32,
    stage: [usize; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FeatureMap {
    pub tag: Tag,
    pub(crate) index: [Option<u16>; 2],
    pub(crate) stage: [usize; 2],
    pub(crate) shift: u32,
    pub(crate) mask: u32,
    pub(crate) one_mask: u32,
    pub(crate) auto_zwnj: bool,
    pub(crate) auto_zwj: bool,
    pub(crate) random: bool,
    pub(crate) per_syllable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LookupMap {
    pub(crate) index: u16,
    pub(crate) mask: u32,
    pub(crate) auto_zwnj: bool,
    pub(crate) auto_zwj: bool,
    pub(crate) random: bool,
    pub(crate) per_syllable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookupOverride {
    pub(crate) table: TableIndex,
    pub(crate) index: u16,
    pub(crate) enable: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StageMap {
    pub(crate) last_lookup: usize,
    pub(crate) pause_index: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct Map {
    pub(crate) found_script: [bool; 2],
    pub(crate) chosen_script: [Option<Tag>; 2],
    pub(crate) global_mask: u32,
    features: Vec<FeatureMap>,
    lookups: [Vec<LookupMap>; 2],
    stages: [Vec<StageMap>; 2],
}

impl Map {
    pub(crate) fn mask(&self, tag: Tag) -> (u32, u32) {
        self.feature(tag).map_or((0, 0), |f| (f.mask, f.shift))
    }

    pub(crate) fn one_mask(&self, tag: Tag) -> u32 {
        self.feature(tag).map_or(0, |f| f.one_mask)
    }

    pub(crate) fn feature(&self, tag: Tag) -> Option<&FeatureMap> {
        self.features
            .binary_search_by(|f| f.tag.cmp(&tag))
            .ok()
            .map(|i| &self.features[i])
    }

    pub(crate) fn feature_index(&self, table: TableIndex, tag: Tag) -> Option<u16> {
        self.feature(tag).and_then(|f| f.index[table.idx()])
    }

    pub(crate) fn stages(&self, table: TableIndex) -> &[StageMap] {
        &self.stages[table.idx()]
    }

    pub(crate) fn stage_lookups(&self, table: TableIndex, stage: usize) -> &[LookupMap] {
        let stages = &self.stages[table.idx()];
        let lookups = &self.lookups[table.idx()];
        let start = stage.checked_sub(1).map_or(0, |p| stages[p].last_lookup);
        let end = stages.get(stage).map_or(lookups.len(), |s| s.last_lookup);
        &lookups[start..end]
    }
}

pub(crate) struct MapBuilder<'a> {
    gsub: Option<&'a LayoutTable<'a>>,
    gpos: Option<&'a LayoutTable<'a>>,
    found_script: [bool; 2],
    script_index: [Option<u16>; 2],
    chosen_script: [Option<Tag>; 2],
    lang_index: [Option<u16>; 2],
    current_stage: [usize; 2],
    feature_infos: Vec<FeatureInfo>,
    stages: [Vec<(usize, Option<usize>)>; 2],
    lookup_overrides: [Vec<(u16, bool)>; 2],
}

impl<'a> MapBuilder<'a> {
    pub(crate) fn chosen_script(&self, table: TableIndex) -> Option<Tag> {
        self.chosen_script[table.idx()]
    }

    pub fn new(
        gsub: Option<&'a LayoutTable<'a>>,
        gpos: Option<&'a LayoutTable<'a>>,
        script_tags: &[Tag],
        language_tags: &[Tag],
    ) -> Self {
        let mut b = MapBuilder {
            gsub,
            gpos,
            found_script: [false; 2],
            script_index: [None; 2],
            chosen_script: [None; 2],
            lang_index: [None; 2],
            current_stage: [0, 0],
            feature_infos: Vec::new(),
            stages: [Vec::new(), Vec::new()],
            lookup_overrides: [Vec::new(), Vec::new()],
        };
        for t in TableIndex::ALL {
            let Some(table) = b.table(t) else { continue };
            if let Some((found, idx, tag)) = table.select_script(script_tags) {
                b.found_script[t.idx()] = found;
                b.script_index[t.idx()] = Some(idx);
                b.chosen_script[t.idx()] = Some(tag);
                b.lang_index[t.idx()] = table.select_langsys(idx, language_tags);
            }
        }
        b
    }

    fn table(&self, t: TableIndex) -> Option<&'a LayoutTable<'a>> {
        match t {
            TableIndex::Gsub => self.gsub,
            TableIndex::Gpos => self.gpos,
        }
    }

    pub(crate) fn add_feature(&mut self, tag: Tag, flags: u32, value: u32) {
        let seq = self.feature_infos.len();
        self.feature_infos.push(FeatureInfo {
            tag,
            seq,
            max_value: value,
            flags,
            default_value: if flags & feature_flags::GLOBAL != 0 { value } else { 0 },
            stage: self.current_stage,
        });
    }

    pub(crate) fn enable_feature(&mut self, tag: Tag, flags: u32, value: u32) {
        self.add_feature(tag, flags | feature_flags::GLOBAL, value);
    }

    pub(crate) fn disable_feature(&mut self, tag: Tag) {
        self.add_feature(tag, feature_flags::GLOBAL, 0);
    }

    pub(crate) fn override_lookup(&mut self, table: TableIndex, index: u16, enable: bool) {
        self.lookup_overrides[table.idx()].push((index, enable));
    }

    pub(crate) fn add_pause(&mut self, table: TableIndex, pause: Option<usize>) {
        self.stages[table.idx()].push((self.current_stage[table.idx()], pause));
        self.current_stage[table.idx()] += 1;
    }

    pub(crate) fn add_gsub_pause(&mut self, pause: Option<usize>) {
        self.add_pause(TableIndex::Gsub, pause);
    }

    pub(crate) fn add_gpos_pause(&mut self, pause: Option<usize>) {
        self.add_pause(TableIndex::Gpos, pause);
    }

    fn dedup(&mut self) {
        if self.feature_infos.is_empty() {
            return;
        }
        self.feature_infos.sort_by(|a, b| a.tag.cmp(&b.tag).then(a.seq.cmp(&b.seq)));

        let mut j = 0;
        for i in 1..self.feature_infos.len() {
            if self.feature_infos[i].tag != self.feature_infos[j].tag {
                j += 1;
                self.feature_infos[j] = self.feature_infos[i];
                continue;
            }
            let cur = self.feature_infos[i];
            if cur.flags & feature_flags::GLOBAL != 0 {
                self.feature_infos[j].flags |= feature_flags::GLOBAL;
                self.feature_infos[j].max_value = cur.max_value;
                self.feature_infos[j].default_value = cur.default_value;
            } else {
                self.feature_infos[j].flags &= !feature_flags::GLOBAL;
                self.feature_infos[j].max_value = self.feature_infos[j].max_value.max(cur.max_value);
            }
            self.feature_infos[j].flags |= cur.flags & feature_flags::HAS_FALLBACK;
            self.feature_infos[j].stage[0] = self.feature_infos[j].stage[0].min(cur.stage[0]);
            self.feature_infos[j].stage[1] = self.feature_infos[j].stage[1].min(cur.stage[1]);
        }
        self.feature_infos.truncate(j + 1);
    }

    fn find_feature(&self, t: TableIndex, tag: Tag, global_search: bool) -> Option<u16> {
        let table = self.table(t)?;
        let script = self.script_index[t.idx()];
        let direct = script.and_then(|s| table.find_feature(s, self.lang_index[t.idx()], tag));
        match direct {
            Some(i) => Some(i),
            None if global_search => table.find_feature_globally(tag),
            None => None,
        }
    }

    fn compile_features(&mut self) -> (Vec<FeatureMap>, [usize; 2], u32) {
        let mut out: Vec<FeatureMap> = Vec::new();
        let mut required_stage = [0usize; 2];
        let mut global_mask = GLOBAL_BIT_MASK;
        let mut next_bit = first_free_bit();

        let required: [Option<u16>; 2] = TableIndex::ALL.map(|t| {
            self.table(t)
                .zip(self.script_index[t.idx()])
                .and_then(|(table, s)| table.required_feature(s, self.lang_index[t.idx()]))
        });

        self.dedup();

        for info in &self.feature_infos {
            let global_one = info.flags & feature_flags::GLOBAL != 0 && info.max_value == 1;
            let bits_needed = if global_one {
                0
            } else {
                MAX_BITS.min(u32::BITS - info.max_value.leading_zeros())
            };

            if info.max_value == 0 || next_bit + bits_needed >= GLOBAL_BIT_SHIFT {
                continue;
            }

            let mut index = [None; 2];
            let mut found = false;
            for t in TableIndex::ALL {
                if required[t.idx()].is_some_and(|r| self.feature_tag_is(t, r, info.tag)) {
                    required_stage[t.idx()] = info.stage[t.idx()];
                }
                let global_search = info.flags & feature_flags::GLOBAL_SEARCH != 0;
                if let Some(i) = self.find_feature(t, info.tag, global_search) {
                    index[t.idx()] = Some(i);
                    found = true;
                }
            }

            if !found && info.flags & feature_flags::HAS_FALLBACK == 0 {
                continue;
            }

            let (shift, mask) = if global_one {
                (GLOBAL_BIT_SHIFT, GLOBAL_BIT_MASK)
            } else {
                let shift = next_bit;
                let mask = (1u32 << (next_bit + bits_needed)) - (1u32 << next_bit);
                next_bit += bits_needed;
                global_mask |= (info.default_value << shift) & mask;
                (shift, mask)
            };

            out.push(FeatureMap {
                tag: info.tag,
                index,
                stage: info.stage,
                shift,
                mask,
                one_mask: (1u32 << shift) & mask,
                auto_zwnj: info.flags & feature_flags::MANUAL_ZWNJ == 0,
                auto_zwj: info.flags & feature_flags::MANUAL_ZWJ == 0,
                random: info.flags & feature_flags::RANDOM != 0,
                per_syllable: info.flags & feature_flags::PER_SYLLABLE != 0,
            });
        }

        out.sort_by_key(|f| f.tag);
        (out, required_stage, global_mask)
    }

    fn feature_tag_is(&self, t: TableIndex, feature: u16, tag: Tag) -> bool {
        self.table(t).and_then(|x| x.feature_tag(feature)) == Some(tag)
    }

    fn collect_lookups(
        &self,
        features: &[FeatureMap],
        required: [Option<u16>; 2],
        required_stage: [usize; 2],
        variations: [Option<u16>; 2],
    ) -> ([Vec<LookupMap>; 2], [Vec<StageMap>; 2]) {
        let mut lookups: [Vec<LookupMap>; 2] = [Vec::new(), Vec::new()];
        let mut stage_maps: [Vec<StageMap>; 2] = [Vec::new(), Vec::new()];

        for t in TableIndex::ALL {
            let i = t.idx();
            let table = self.table(t);
            let mut stage_index = 0usize;
            let mut last_lookup = 0usize;

            for stage in 0..self.current_stage[i] {
                if let Some(table) = table {
                    if let Some(f) = required[i]
                        && required_stage[i] == stage {
                            push_lookups(&mut lookups[i], table, f, GLOBAL_BIT_MASK, true, true, false, false, variations[i]);
                        }
                    for f in features {
                        if let Some(fi) = f.index[i]
                            && f.stage[i] == stage {
                                push_lookups(&mut lookups[i], table, fi, f.mask, f.auto_zwnj, f.auto_zwj, f.random, f.per_syllable, variations[i]);
                            }
                    }

                    let len = lookups[i].len();
                    if last_lookup + 1 < len {
                        lookups[i][last_lookup..].sort_by_key(|l| l.index);
                        let mut j = last_lookup;
                        for k in j + 1..len {
                            if lookups[i][k].index != lookups[i][j].index {
                                j += 1;
                                lookups[i][j] = lookups[i][k];
                            } else {
                                lookups[i][j].mask |= lookups[i][k].mask;
                                lookups[i][j].auto_zwnj &= lookups[i][k].auto_zwnj;
                                lookups[i][j].auto_zwj &= lookups[i][k].auto_zwj;
                            }
                        }
                        lookups[i].truncate(j + 1);
                    }
                }
                last_lookup = lookups[i].len();

                if let Some(&(at, pause)) = self.stages[i].get(stage_index)
                    && at == stage {
                        stage_maps[i].push(StageMap { last_lookup, pause_index: pause });
                        stage_index += 1;
                    }
            }

            if let Some(table) = table {
                self.apply_overrides(t, table.lookup_count(), &mut lookups[i], &mut stage_maps[i]);
            }
        }

        (lookups, stage_maps)
    }

    fn apply_overrides(
        &self,
        table: TableIndex,
        lookup_count: u16,
        lookups: &mut Vec<LookupMap>,
        stages: &mut [StageMap],
    ) {
        let overrides = &self.lookup_overrides[table.idx()];
        if overrides.is_empty() {
            return;
        }

        let mut wanted: alloc::collections::BTreeMap<u16, bool> = alloc::collections::BTreeMap::new();
        for &(index, enable) in overrides {
            wanted.entry(index).and_modify(|e| *e |= enable).or_insert(enable);
        }

        if wanted.values().any(|&enable| !enable) {
            let mut kept = 0usize;
            let mut at = 0usize;
            for stage in stages.iter_mut() {
                while at < stage.last_lookup {
                    if wanted.get(&lookups[at].index) != Some(&false) {
                        lookups[kept] = lookups[at];
                        kept += 1;
                    }
                    at += 1;
                }
                stage.last_lookup = kept;
            }
            debug_assert_eq!(at, lookups.len());
            lookups.truncate(kept);
        }

        let mut present: alloc::collections::BTreeSet<u16> = lookups.iter().map(|l| l.index).collect();
        let mut added = false;
        for (&index, &enable) in &wanted {
            if !enable || index >= lookup_count || !present.insert(index) {
                continue;
            }
            lookups.push(LookupMap {
                index,
                mask: GLOBAL_BIT_MASK,
                auto_zwnj: true,
                auto_zwj: true,
                random: false,
                per_syllable: false,
            });
            added = true;
        }
        if added
            && let Some(last) = stages.last_mut() {
                last.last_lookup = lookups.len();
            }
    }

    pub(crate) fn compile(mut self, variations: [Option<u16>; 2]) -> Map {
        let (features, required_stage, global_mask) = self.compile_features();

        self.add_gsub_pause(None);
        self.add_gpos_pause(None);

        let required: [Option<u16>; 2] = TableIndex::ALL.map(|t| {
            self.table(t)
                .zip(self.script_index[t.idx()])
                .and_then(|(table, s)| table.required_feature(s, self.lang_index[t.idx()]))
        });
        let (lookups, stages) = self.collect_lookups(&features, required, required_stage, variations);

        Map {
            found_script: self.found_script,
            chosen_script: self.chosen_script,
            global_mask,
            features,
            lookups,
            stages,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_lookups(
    out: &mut Vec<LookupMap>,
    table: &LayoutTable,
    feature: u16,
    mask: u32,
    auto_zwnj: bool,
    auto_zwj: bool,
    random: bool,
    per_syllable: bool,
    variation: Option<u16>,
) {
    let substitute = variation.and_then(|v| table.variation_substitute(v, feature));
    let count = table.lookup_count();
    for index in table.feature_lookups(feature, substitute) {
        if index < count {
            out.push(LookupMap { index, mask, auto_zwnj, auto_zwj, random, per_syllable });
        }
    }
}
