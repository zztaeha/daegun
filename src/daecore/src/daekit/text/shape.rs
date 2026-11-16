use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::daecore::daekit::cache::{AxisKey, FontCache, RunContext};
use crate::daecore::daetype::jstf::JstfModLists;
use crate::daecore::daeshaper::plan::{FeatureSetting, LookupSwitch, Table};
use crate::daecore::daeshaper::run::{ClusterLevel, Direction};
use crate::daecore::daeshaper::script::Shaper;
use crate::daecore::daeshaper::unicode::Script;
use crate::daecore::{GlyphId, Tag};

// Features a shape reads in place; past them it collects them.
const FEW_FEATURES: usize = 16;

/// Where a glyph or a run sits from where it would otherwise, in font units, y up.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offset {
    /// Across.
    pub x: f64,
    /// Up.
    pub y: f64,
}

impl From<(f64, f64)> for Offset {
    fn from((x, y): (f64, f64)) -> Offset {
        Offset { x, y }
    }
}

impl From<Offset> for (f64, f64) {
    fn from(o: Offset) -> (f64, f64) {
        (o.x, o.y)
    }
}

/// A shaped run: its glyphs in visual order, each with its advance, offset and cluster, in font units.
#[non_exhaustive]
#[derive(Debug, PartialEq, Default)]
pub struct ShapedRun {
    /// The glyphs, in the order they are drawn.
    pub glyphs: Vec<GlyphId>,
    /// Each glyph's advance: across a line, or down one in vertical text.
    pub advances: Vec<f64>,
    /// Each glyph's offset from where its advance puts it.
    pub offsets: Vec<Offset>,
    /// Whether breaking the text before each glyph and shaping the two sides apart could change them.
    pub unsafe_to_break: Vec<bool>,
    /// Whether shaping the text on either side of each glyph apart and joining the runs could change them;
    /// empty unless [`ShapeOptions::report_unsafe_to_concat`] asked for it.
    pub unsafe_to_concat: Vec<bool>,
    /// Whether a tatweel may go before each glyph; empty unless [`ShapeOptions::report_tatweel_positions`]
    /// asked for it.
    pub safe_to_insert_tatweel: Vec<bool>,
    /// Where each glyph's cluster starts, a byte offset into the text.
    pub clusters: Vec<u32>,
    /// False where shaping stopped short, past its limits on a hostile font.
    pub complete: bool,
    /// Whether a syllable was malformed, so a dotted circle may stand in for its base.
    pub has_broken_syllable: bool,
    /// The shaping model the run went through.
    pub shaper: Shaper,
}

// By hand for `clone_from`, which keeps each vector's room, so a run copied into a kept one allocates only where it
// outgrows it; both name every field, so a field added can't be left out.
impl Clone for ShapedRun {
    fn clone(&self) -> ShapedRun {
        let ShapedRun {
            glyphs, advances, offsets, unsafe_to_break, unsafe_to_concat, safe_to_insert_tatweel, clusters, complete,
            has_broken_syllable, shaper,
        } = self;
        ShapedRun {
            glyphs: glyphs.clone(),
            advances: advances.clone(),
            offsets: offsets.clone(),
            unsafe_to_break: unsafe_to_break.clone(),
            unsafe_to_concat: unsafe_to_concat.clone(),
            safe_to_insert_tatweel: safe_to_insert_tatweel.clone(),
            clusters: clusters.clone(),
            complete: *complete,
            has_broken_syllable: *has_broken_syllable,
            shaper: *shaper,
        }
    }

    fn clone_from(&mut self, source: &ShapedRun) {
        let ShapedRun {
            glyphs, advances, offsets, unsafe_to_break, unsafe_to_concat, safe_to_insert_tatweel, clusters, complete,
            has_broken_syllable, shaper,
        } = source;
        self.glyphs.clone_from(glyphs);
        self.advances.clone_from(advances);
        self.offsets.clone_from(offsets);
        self.unsafe_to_break.clone_from(unsafe_to_break);
        self.unsafe_to_concat.clone_from(unsafe_to_concat);
        self.safe_to_insert_tatweel.clone_from(safe_to_insert_tatweel);
        self.clusters.clone_from(clusters);
        (self.complete, self.has_broken_syllable, self.shaper) = (*complete, *has_broken_syllable, *shaper);
    }
}

impl ShapedRun {
    /// The sum of its advances: its width, or its height in vertical text.
    #[doc(alias("measure_width"))]
    pub fn width(&self) -> f64 {
        self.advances.iter().sum()
    }
}

/// What becomes of default ignorables, the invisible characters such as joiners and variation selectors.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Ignorables {
    /// Drawn as an invisible glyph of no advance.
    #[default]
    Hide,
    /// Left out of the run.
    Remove,
    /// Drawn as the font maps them.
    Preserve,
}

/// An OpenType feature and its value, over `bytes` of the text.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Feature {
    /// The feature's tag, such as `liga`.
    pub tag: Tag,
    /// 0 turns it off, 1 on, and a higher value picks an alternate.
    pub value: u32,
    /// The bytes of the text it holds over, an end past the text's running to its end.
    pub bytes: core::range::Range<usize>,
}

// By its fields in order, for a kept run's key; `Range` has no order of its own.
impl Ord for Feature {
    fn cmp(&self, other: &Feature) -> core::cmp::Ordering {
        let fields = |f: &Feature| (f.tag, f.value, f.bytes.start, f.bytes.end);
        fields(self).cmp(&fields(other))
    }
}

impl PartialOrd for Feature {
    fn partial_cmp(&self, other: &Feature) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Feature {
    /// A feature over all the text.
    #[must_use]
    pub const fn new(tag: Tag, value: u32) -> Self {
        Feature { tag, value, bytes: core::range::Range { start: 0, end: usize::MAX } }
    }
}

/// A feature written as shaping tools and CSS write one: `kern`, `-liga` (off), `+smcp` (on), `aalt=2` or `aalt 2`,
/// `on` and `off` as values, the tag quoted or not, and `[start:end]` for the bytes it covers, either end left out
/// for the text's own and `[i]` for the one at `i`: `kern[3:5]=0`. Anything else is an
/// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) error.
///
/// ```
/// use daegun::shape::Feature;
/// use daegun::Tag;
///
/// let off: Feature = "kern[3:5]=0".parse()?;
/// assert_eq!(off, Feature { bytes: (3..5).into(), ..Feature::new(Tag::new(b"kern"), 0) });
/// assert_eq!("-liga".parse(), Ok(Feature::new(Tag::new(b"liga"), 0)));
/// # Ok::<(), daegun::Error>(())
/// ```
impl core::str::FromStr for Feature {
    type Err = crate::daecore::Error;

    fn from_str(text: &str) -> Result<Feature, Self::Err> {
        let bad = || crate::daecore::Error::invalid("feature: not a tag with an optional sign, range and value");
        let rest = text.trim();
        let (mut value, rest) = match rest.strip_prefix(['+', '-']) {
            Some(after) => (u32::from(rest.starts_with('+')), after.trim_start()),
            None => (1, rest),
        };
        let (tag, rest) = match rest.strip_prefix(['"', '\'']) {
            Some(quoted) => quoted.split_once(&rest[..1]).ok_or_else(bad)?,
            None => rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len())),
        };
        if tag.is_empty() {
            return Err(bad());
        }
        let tag: Tag = tag.parse()?;
        let mut rest = rest.trim_start();
        let (mut start, mut end) = (0, usize::MAX);
        if let Some(range) = rest.strip_prefix('[') {
            let (range, after) = range.split_once(']').ok_or_else(bad)?;
            let at = |n: &str| n.trim().parse::<usize>().map_err(|_| bad());
            match range.split_once([':', ';']) {
                Some((from, to)) => {
                    if !from.trim().is_empty() { start = at(from)?; }
                    if !to.trim().is_empty() { end = at(to)?; }
                }
                None if range.trim().is_empty() => {}
                None => (start, end) = (at(range)?, at(range)?.checked_add(1).ok_or_else(bad)?),
            }
            rest = after.trim_start();
        }
        let given = rest.strip_prefix('=');
        match given.unwrap_or(rest).trim() {
            "" if given.is_some() => return Err(bad()),
            "" => {}
            "on" => value = 1,
            "off" => value = 0,
            number => value = number.parse().map_err(|_| bad())?,
        }
        Ok(Feature { tag, value, bytes: core::range::Range { start, end } })
    }
}

/// A feature as `parse` reads it back: `-` before a feature off, `=` and the value past 1, and its bytes in brackets
/// unless it covers the whole text, `[i]` for the one at `i`. A tag of anything but letters and digits is quoted, in
/// single quotes if it holds a double one.
///
/// ```
/// use daegun::shape::Feature;
/// use daegun::Tag;
///
/// let feature = Feature { bytes: (3..5).into(), ..Feature::new(Tag::new(b"aalt"), 2) };
/// assert_eq!(feature.to_string(), "aalt[3:5]=2");
/// assert_eq!(feature.to_string().parse(), Ok(feature));
/// ```
impl core::fmt::Display for Feature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.value == 0 {
            f.write_str("-")?;
        }
        let bytes = self.tag.to_bytes();
        match (bytes.iter().all(u8::is_ascii_alphanumeric), bytes.contains(&b'"')) {
            (true, _) => write!(f, "{}", self.tag)?,
            (false, false) => write!(f, "\"{}\"", self.tag)?,
            (false, true) => write!(f, "'{}'", self.tag)?,
        }
        match (self.bytes.start, self.bytes.end) {
            (0, usize::MAX) => {}
            (start, end) if start.checked_add(1) == Some(end) => write!(f, "[{start}]")?,
            (start, end) => {
                f.write_str("[")?;
                if start != 0 { write!(f, "{start}")? }
                f.write_str(":")?;
                if end != usize::MAX { write!(f, "{end}")? }
                f.write_str("]")?;
            }
        }
        if self.value > 1 { write!(f, "={}", self.value)? }
        Ok(())
    }
}

/// Everything the shaper can be told. `ShapeOptions::default()` shapes text as it reads, as its script and the font
/// say, with no context.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShapeOptions<'a> {
    /// The direction to shape in; none to take left to right or right to left from the text.
    pub direction: Option<Direction>,
    /// The OpenType script whose lookups apply, in place of the one the text's script picks.
    pub script: Option<Tag>,
    /// The language whose lookups apply, as a BCP 47 tag such as `tr` or `sr-Latn`.
    pub language: Option<&'a str>,
    /// Features to turn on, off or set, each over its range.
    pub features: &'a [Feature],
    /// How clusters are formed.
    pub cluster_level: ClusterLevel,
    /// The text before the run, which joining and contextual lookups read.
    pub before: &'a str,
    /// The text after the run.
    pub after: &'a str,
    /// Whether the run starts the text, so nothing precedes it.
    pub beginning_of_text: bool,
    /// The point size trak is read at, [`DEFAULT_POINT_SIZE`](crate::metrics::DEFAULT_POINT_SIZE) when none or one
    /// not finite and above zero.
    pub point_size: Option<f64>,
    /// Whether to fill [`ShapedRun::unsafe_to_concat`].
    pub report_unsafe_to_concat: bool,
    /// Whether to fill [`ShapedRun::safe_to_insert_tatweel`].
    pub report_tatweel_positions: bool,
    /// What becomes of default ignorables.
    pub ignorables: Ignorables,
    /// Whether to leave out the dotted circle a broken syllable gets.
    pub suppress_dotted_circle: bool,
    /// The glyph an invisible character is drawn with, none for the font's space.
    pub invisible_glyph: Option<GlyphId>,
    /// The script to shape the run as, in place of the one its text would be scanned for.
    pub seed_script: Option<Script>,
    /// What a character the font has no glyph for is drawn with.
    pub not_found_glyph: GlyphId,
    /// What a variation selector the font can't apply is drawn with, where it would otherwise go as a default
    /// ignorable.
    pub not_found_variation_selector_glyph: Option<GlyphId>,
}

impl Default for ShapeOptions<'_> {
    fn default() -> Self {
        ShapeOptions::DEFAULT
    }
}

impl<'a> ShapeOptions<'a> {
    /// The defaults, as `default()` gives them. `&ShapeOptions::DEFAULT` is built once, where `&default()` is built
    /// on the stack each call, and options of your own can be a constant too:
    ///
    /// ```
    /// use daegun::shape::{Direction, ShapeOptions};
    ///
    /// const RTL: ShapeOptions = ShapeOptions::DEFAULT.with_direction(Direction::RightToLeft);
    /// # assert_eq!(RTL.direction, Some(Direction::RightToLeft));
    /// ```
    pub const DEFAULT: ShapeOptions<'static> = ShapeOptions {
        direction: None,
        script: None,
        language: None,
        features: &[],
        cluster_level: ClusterLevel::MonotoneGraphemes,
        before: "",
        after: "",
        beginning_of_text: false,
        point_size: None,
        report_unsafe_to_concat: false,
        report_tatweel_positions: false,
        ignorables: Ignorables::Hide,
        suppress_dotted_circle: false,
        invisible_glyph: None,
        seed_script: None,
        not_found_glyph: GlyphId::NOTDEF,
        not_found_variation_selector_glyph: None,
    };

    /// These options with a direction.
    #[must_use]
    pub const fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    /// These options with an OpenType script.
    #[must_use]
    pub const fn with_script(mut self, script: Tag) -> Self {
        self.script = Some(script);
        self
    }

    /// These options with a language.
    #[must_use]
    pub const fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    /// These options with features.
    #[must_use]
    pub const fn with_features(mut self, features: &'a [Feature]) -> Self {
        self.features = features;
        self
    }

    /// These options with a cluster level.
    #[must_use]
    pub const fn with_cluster_level(mut self, cluster_level: ClusterLevel) -> Self {
        self.cluster_level = cluster_level;
        self
    }

    /// These options with the text around the run.
    #[must_use]
    pub const fn with_context(mut self, before: &'a str, after: &'a str) -> Self {
        self.before = before;
        self.after = after;
        self
    }

    /// These options with the run at the start of the text, or not.
    #[must_use]
    pub const fn with_beginning_of_text(mut self, beginning: bool) -> Self {
        self.beginning_of_text = beginning;
        self
    }

    /// These options with a point size for trak.
    #[must_use]
    pub const fn with_point_size(mut self, point_size: f64) -> Self {
        self.point_size = Some(point_size);
        self
    }

    /// These options filling [`ShapedRun::unsafe_to_concat`], or not.
    #[must_use]
    pub const fn with_report_unsafe_to_concat(mut self, report: bool) -> Self {
        self.report_unsafe_to_concat = report;
        self
    }

    /// These options filling [`ShapedRun::safe_to_insert_tatweel`], or not.
    #[must_use]
    pub const fn with_report_tatweel_positions(mut self, report: bool) -> Self {
        self.report_tatweel_positions = report;
        self
    }

    /// These options with default ignorables treated so.
    #[must_use]
    pub const fn with_ignorables(mut self, ignorables: Ignorables) -> Self {
        self.ignorables = ignorables;
        self
    }

    /// These options leaving out the dotted circle, or not.
    #[must_use]
    pub const fn with_suppress_dotted_circle(mut self, suppress: bool) -> Self {
        self.suppress_dotted_circle = suppress;
        self
    }

    /// These options drawing invisible characters with a glyph.
    #[must_use]
    pub const fn with_invisible_glyph(mut self, glyph: GlyphId) -> Self {
        self.invisible_glyph = Some(glyph);
        self
    }

    /// These options shaping the run as a script.
    #[must_use]
    pub const fn with_seed_script(mut self, script: Script) -> Self {
        self.seed_script = Some(script);
        self
    }

    /// These options drawing a character the font lacks with a glyph.
    #[must_use]
    pub const fn with_not_found_glyph(mut self, glyph: GlyphId) -> Self {
        self.not_found_glyph = glyph;
        self
    }

    /// These options drawing a variation selector the font can't apply with a glyph.
    #[must_use]
    pub const fn with_not_found_variation_selector_glyph(mut self, glyph: GlyphId) -> Self {
        self.not_found_variation_selector_glyph = Some(glyph);
        self
    }

    // What the run cache keys on, where these options ask for nothing else: whether the run is vertical and its
    // context. None for options it doesn't hold.
    pub(crate) fn cached(&self) -> Option<(bool, RunContext<'a>)> {
        let ShapeOptions {
            direction, script, language, features, cluster_level, before, after, beginning_of_text, point_size,
            report_unsafe_to_concat, report_tatweel_positions, ignorables, suppress_dotted_circle, invisible_glyph,
            seed_script, not_found_glyph, not_found_variation_selector_glyph,
        } = *self;
        let plain = script.is_none()
            && features.is_empty()
            && cluster_level == ClusterLevel::default()
            && !beginning_of_text
            && point_size.is_none()
            && !report_unsafe_to_concat
            && !report_tatweel_positions
            && ignorables == Ignorables::default()
            && !suppress_dotted_circle
            && invisible_glyph.is_none()
            && not_found_glyph == GlyphId::NOTDEF
            && not_found_variation_selector_glyph.is_none();
        let (vertical, rtl) = match direction {
            None => (false, None),
            Some(Direction::LeftToRight) => (false, Some(false)),
            Some(Direction::RightToLeft) => (false, Some(true)),
            Some(Direction::TopToBottom) => (true, None),
            Some(Direction::BottomToTop) => return None,
        };
        plain.then_some((vertical, RunContext { rtl, before, after, language, seed_script, features: &[] }))
    }
}

// A `stated_rtl` of None takes the direction the text guesses.
pub fn shape_run(
    fc: &FontCache, axes: &AxisKey, text: &str, vertical: bool,
    stated_rtl: Option<bool>, opts: &ShapeOptions,
) -> Arc<ShapedRun> {
    shared(|run| shape_with_overrides(fc, axes, text, vertical, stated_rtl, opts, &[], run))
}

// A run filled where it is kept, as filling one beside it and moving it in copies it whole and stalls on the copy.
fn shared(fill: impl FnOnce(&mut ShapedRun)) -> Arc<ShapedRun> {
    let mut run = Arc::new(ShapedRun::default());
    if let Some(fresh) = Arc::get_mut(&mut run) {
        fill(fresh);
    }
    run
}

// `shape_run` into a run whose vectors are reused.
pub(crate) fn shape_run_into(
    fc: &FontCache, axes: &AxisKey, text: &str, opts: &ShapeOptions, run: &mut ShapedRun,
) {
    shape_with_overrides(fc, axes, text, false, None, opts, &[], run);
}

pub(crate) fn shape_run_justified(
    fc: &FontCache, axes: &AxisKey, text: &str, vertical: bool,
    mods: &JstfModLists, shrink: bool,
) -> Arc<ShapedRun> {
    let overrides = jstf_mods_to_overrides(mods, shrink);
    shared(|run| shape_with_overrides(fc, axes, text, vertical, None, &ShapeOptions::default(), &overrides, run))
}

fn jstf_mods_to_overrides(mods: &JstfModLists, shrink: bool) -> Vec<LookupSwitch> {
    let (enable_gsub, disable_gsub, enable_gpos, disable_gpos) = if shrink {
        (
            &mods.shrinkage_enable_gsub, &mods.shrinkage_disable_gsub,
            &mods.shrinkage_enable_gpos, &mods.shrinkage_disable_gpos,
        )
    } else {
        (
            &mods.extension_enable_gsub, &mods.extension_disable_gsub,
            &mods.extension_enable_gpos, &mods.extension_disable_gpos,
        )
    };

    let mut out = Vec::new();
    let mut push = |list: &Option<Vec<u16>>, table: Table, on: bool| {
        for &lookup in list.iter().flatten() {
            out.push(LookupSwitch { table, lookup, on });
        }
    };
    push(disable_gsub, Table::Gsub, false);
    push(disable_gpos, Table::Gpos, false);
    push(enable_gsub, Table::Gsub, true);
    push(enable_gpos, Table::Gpos, true);
    out
}

#[expect(clippy::too_many_arguments, reason = "a run's request in six parts, and the run it fills")]
fn shape_with_overrides(
    fc: &FontCache, axes: &AxisKey, text: &str, vertical: bool,
    stated_rtl: Option<bool>, opts: &ShapeOptions, lookup_overrides: &[LookupSwitch], run: &mut ShapedRun,
) {
    fc.described_at(axes, |at| shape_at(fc, at, text, vertical, stated_rtl, opts, lookup_overrides, run));
}

// On the font's own cache, which the face, the buffer and the plan come from, its metrics and glyphs' measures read
// from `at`, the cache describing the location.
#[expect(clippy::too_many_arguments, reason = "a run's request in six parts, and the run it fills")]
fn shape_at(
    font: &FontCache, at: &FontCache, text: &str, vertical: bool,
    stated_rtl: Option<bool>, opts: &ShapeOptions, lookup_overrides: &[LookupSwitch], run: &mut ShapedRun,
) {
    use crate::daecore::daeshaper::face::Face;
    use crate::daecore::daeshaper::pipeline::{settle_script, shape};
    use crate::daecore::daeshaper::run::{flag, Fact, Mask, Settings};

    let face = Face::new(font).with_point_size(opts.point_size).located(at);
    // The buffer's clusters are byte offsets, as a range is; one over all the text is global.
    let index = |c: usize| u32::try_from(c).unwrap_or(u32::MAX);
    let parsed = opts.features.iter().map(|f| FeatureSetting {
        tag: f.tag,
        value: f.value,
        start: index(f.bytes.start),
        end: index(f.bytes.end),
    });
    // A few features are read in place, so a call that gives them allocates nothing for them.
    let blank = FeatureSetting { tag: Tag(0), value: 0, start: 0, end: 0 };
    let (mut few, mut many) = ([blank; FEW_FEATURES], Vec::new());
    let features: &[FeatureSetting] = if opts.features.len() <= FEW_FEATURES {
        let mut n = 0;
        for f in parsed {
            few[n] = f;
            n += 1;
        }
        &few[..n]
    } else {
        many.extend(parsed);
        &many
    };

    let mut buf = font.take_buffer();
    buf.push_str(text);
    // A seeded script is kept; only an unset one is read from the text.
    buf.script = opts.seed_script;
    let asked = |on: bool, bits: Mask| if on { bits } else { 0 };
    buf.settings = Settings {
        cluster_level: opts.cluster_level,
        reported: flag::UNSAFE_TO_BREAK
            | asked(opts.report_unsafe_to_concat, flag::UNSAFE_TO_CONCAT)
            | asked(opts.report_tatweel_positions, flag::SAFE_TO_INSERT_TATWEEL),
        ignorables: opts.ignorables,
        starts_text: opts.beginning_of_text,
        dotted_circles: !opts.suppress_dotted_circle,
        // Glyph 0 is .notdef, so it names no invisible glyph.
        invisible: opts.invisible_glyph.filter(|&g| g != GlyphId::NOTDEF).map(u32::from),
        missing: opts.not_found_glyph.0,
        missing_selector: opts.not_found_variation_selector_glyph.map(u32::from),
    };
    buf.set_context(opts.before, opts.after);

    let guessed = settle_script(&mut buf);
    let direction = match (opts.direction, stated_rtl) {
        (Some(direction), _) => direction,
        _ if vertical => Direction::TopToBottom,
        (None, Some(true)) => Direction::RightToLeft,
        (None, Some(false)) => Direction::LeftToRight,
        (None, None) => guessed,
    };
    let vertical = direction.is_vertical();
    buf.direction = direction;

    let forced = opts.script.map(|t| [t]);
    let script = forced.as_ref().map(|one| &one[..]);

    let plan = font.shape_plan_cached(buf.script, &face, direction, script, opts.language, features, lookup_overrides);
    shape(&face, &plan, &mut buf, direction, features);

    // A pass a vector, each filled to its exact size without the pushes' capacity checks: in the room it has, or
    // collected afresh, as growing an empty vector costs more than allocating one.
    let (info, pos) = (&buf.glyphs[..buf.len], &buf.places[..buf.len]);
    fn fill<T>(out: &mut Vec<T>, items: impl ExactSizeIterator<Item = T>) {
        if out.capacity() < items.len() {
            *out = items.collect();
        } else {
            out.clear();
            out.extend(items);
        }
    }
    let flagged = |out: &mut Vec<bool>, report: bool, flag| match report {
        true => fill(out, info.iter().map(|i| i.mask & flag != 0)),
        false => out.clear(),
    };
    fill(&mut run.glyphs, info.iter().map(|i| GlyphId(i.id)));
    let advances = pos.iter().map(|p| if vertical { p.advance_y.saturating_neg() } else { p.advance_x });
    fill(&mut run.advances, advances.map(f64::from));
    fill(&mut run.offsets, pos.iter().map(|p| Offset { x: f64::from(p.offset_x), y: f64::from(p.offset_y) }));
    flagged(&mut run.unsafe_to_break, true, flag::UNSAFE_TO_BREAK);
    flagged(&mut run.unsafe_to_concat, opts.report_unsafe_to_concat, flag::UNSAFE_TO_CONCAT);
    flagged(&mut run.safe_to_insert_tatweel, opts.report_tatweel_positions, flag::SAFE_TO_INSERT_TATWEEL);
    fill(&mut run.clusters, info.iter().map(|i| i.cluster));
    run.complete = !buf.failed;
    run.has_broken_syllable = buf.facts.has(Fact::BrokenSyllable);
    run.shaper = plan.model.kind();
    font.give_buffer(buf);
}
