/* Shaping: text to positioned glyphs, one run or bidi runs, and the carets between characters. Rust's
 * shape module and Font::shape. */

#ifndef DAEGUN_SHAPE_H
#define DAEGUN_SHAPE_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* runs */

typedef struct daegun_shaped_run daegun_shaped_run;

/** One glyph's offset from where its advance puts it, in font units. */
typedef struct {
    double x;
    double y;
} daegun_offset;

/** Borrowed views into a run, all valid until it is freed, each one entry a glyph. */
DAEGUN_API const uint32_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_glyphs(const daegun_shaped_run *run, size_t *out_count);
/** Each glyph's advance along the run's direction. */
DAEGUN_API const double *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_advances(const daegun_shaped_run *run, size_t *out_count);
DAEGUN_API const daegun_offset *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_offsets(const daegun_shaped_run *run, size_t *out_count);
/** Where each glyph's cluster starts, a byte offset into the text. Several glyphs may share a cluster, and
 * one glyph may span several characters. */
DAEGUN_API const uint32_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_clusters(const daegun_shaped_run *run, size_t *out_count);
/** A byte each, 1 where breaking the line before the glyph would change how it shapes. */
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_unsafe_to_break(const daegun_shaped_run *run, size_t *out_count);
/** Empty unless the options asked for them: report_unsafe_to_concat, report_tatweel_positions. */
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_unsafe_to_concat(const daegun_shaped_run *run, size_t *out_count);
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_shaped_run_safe_to_insert_tatweel(const daegun_shaped_run *run, size_t *out_count);

/** The sum of its advances: its width, or its height in vertical text. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shaped_run_width(const daegun_shaped_run *run, double *out);
/** False where shaping stopped short, past its limits on a hostile font. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shaped_run_complete(const daegun_shaped_run *run, bool *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shaped_run_has_broken_syllable(const daegun_shaped_run *run, bool *out);
/* The shaping model a run went through: its script's own, the general one, or DUMBER, which leaves to a font's
 * morx the shaping of a script that has a model of its own. */
#define DAEGUN_SHAPER_DEFAULT          0  /* the general model, for a script with none of its own */
#define DAEGUN_SHAPER_DUMBER           1
#define DAEGUN_SHAPER_ARABIC           2  /* Arabic's and Syriac's joining */
#define DAEGUN_SHAPER_HANGUL           3
#define DAEGUN_SHAPER_HEBREW           4
#define DAEGUN_SHAPER_INDIC            5  /* the Indic scripts' syllables, Devanagari's and its kin's */
#define DAEGUN_SHAPER_KHMER            6
#define DAEGUN_SHAPER_MYANMAR          7
#define DAEGUN_SHAPER_MYANMAR_ZAWGYI   8  /* Myanmar in Zawgyi, asked for with the language tag qaag */
#define DAEGUN_SHAPER_THAI             9  /* Thai's and Lao's marks */
#define DAEGUN_SHAPER_UNIVERSAL       10  /* the Universal Shaping Engine's syllables */

/** Which model, a DAEGUN_SHAPER_*; a newer library may answer one this header doesn't name. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shaped_run_shaper(const daegun_shaped_run *run, int32_t *out);
/** A DAEGUN_SHAPER_*'s name in snake case, such as "myanmar_zawgyi", BORROWED: a static string.
 * DAEGUN_INVALID_ARGUMENT for a code this library doesn't know. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shaper_name(int32_t shaper, daegun_str *out);
/** A run of your own holding a copy of what `run` holds, such as one BORROWED from a set of bidi runs or a layout
 * that must outlive it; free it with daegun_shaped_run_free. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shaped_run_copy(const daegun_shaped_run *run, daegun_shaped_run *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_API void DAEGUN_CALL
daegun_shaped_run_free(daegun_shaped_run *DAEGUN_NULLABLE run);

/* options */

/** One OpenType feature you are turning on, off, or selecting an alternate of, over bytes start to end of
 * the text, end not included. An end of 0 or UINT32_MAX runs to the text's end, so a feature
 * zeroed but for its tag and value covers all of it. */
typedef struct {
    daegun_tag tag;
    uint32_t   value;
    uint32_t   start;
    uint32_t   end;
} daegun_feature;

/** A feature from its text, as shaping tools and CSS write one and Rust's Feature reads it: `kern`, `-liga`, `aalt=2`,
 * `kern[3:5]=0`. DAEGUN_INVALID_ARGUMENT for anything else. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_feature_from_string(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len, daegun_feature *out);

#define DAEGUN_DIRECTION_DEFAULT  0  /* as the text reads, left to right or right to left */
#define DAEGUN_DIRECTION_LTR      1
#define DAEGUN_DIRECTION_RTL      2
#define DAEGUN_DIRECTION_TTB      3  /* top to bottom: vertical text */
#define DAEGUN_DIRECTION_BTT      4

#define DAEGUN_CLUSTER_MONOTONE_GRAPHEMES   0
#define DAEGUN_CLUSTER_MONOTONE_CHARACTERS  1
#define DAEGUN_CLUSTER_CHARACTERS           2
#define DAEGUN_CLUSTER_GRAPHEMES            3

#define DAEGUN_IGNORABLES_HIDE      0
#define DAEGUN_IGNORABLES_REMOVE    1
#define DAEGUN_IGNORABLES_PRESERVE  2

/** Everything the shaper can be told, as Rust's ShapeOptions. Fill it with daegun_shape_options_default,
 * which gives zero to all but its size, as every field's default is. */
typedef struct {
    size_t      size;            /**< sizeof(daegun_shape_options) as your header has it */
    int32_t     direction;       /**< DAEGUN_DIRECTION_* */
    daegun_tag  script;          /**< an OpenType script tag whose lookups apply, 0 for the text's own */
    const char *DAEGUN_NULLABLE language; /**< a BCP 47 tag, NUL-terminated; NULL for none */
    const daegun_feature *DAEGUN_NULLABLE features DAEGUN_COUNTED_BY(features_len);
    size_t      features_len;
    /** Text before the run and after it, which joining and contextual lookups read. */
    const char *DAEGUN_NULLABLE before DAEGUN_COUNTED_BY(before_len);
    size_t      before_len;
    const char *DAEGUN_NULLABLE after DAEGUN_COUNTED_BY(after_len);
    size_t      after_len;
    int32_t     cluster_level;   /**< DAEGUN_CLUSTER_* */
    int32_t     ignorables;      /**< DAEGUN_IGNORABLES_* */
    uint32_t    invisible_glyph; /**< with has_invisible_glyph; else the font's space */
    uint32_t    not_found_glyph; /**< for a character the font lacks; 0 is .notdef */
    uint32_t    not_found_variation_selector_glyph; /**< with its has_; else such a selector goes as a
                                                       default ignorable */
    daegun_tag  seed_script;     /**< the script to shape the run as, an ISO 15924 code as a tag, such as
                                    DAEGUN_TAG('A','r','a','b'); 0 to scan the text for one */
    bool        beginning_of_text; /**< nothing precedes the run */
    bool        has_point_size;
    bool        report_unsafe_to_concat;
    bool        report_tatweel_positions;
    bool        suppress_dotted_circle;
    bool        has_invisible_glyph;
    bool        has_not_found_variation_selector_glyph;
    double      point_size;      /**< for trak, with has_point_size; else DAEGUN_DEFAULT_POINT_SIZE */
} daegun_shape_options;

/** Zeroes `size` bytes of *out and sets its size; pass sizeof *out. DAEGUN_INVALID_ARGUMENT for a size
 * below 2.0's, and DAEGUN_UNSUPPORTED for one larger than this library knows, each writing nothing. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shape_options_default(daegun_shape_options *out, size_t size);

/** Two of the four cluster levels group by grapheme, and two are monotone, so neither question is a
 * comparison against one constant. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_cluster_level_is_graphemes(int32_t level, bool *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_cluster_level_is_monotone(int32_t level, bool *out);

/* shaping */

/** One run of text at a location. A NULL opts means the defaults. A run asked for with no more than a
 * direction, context, a language and a seed script comes from the font's run cache where it holds one. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_shape(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                  const daegun_location *DAEGUN_NULLABLE location, const daegun_shape_options *DAEGUN_NULLABLE opts,
                  daegun_shaped_run *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** Shapes text as daegun_font_shape does, into *run, whose lists keep their room: shaping run after run into one
 * allocates only where a run outgrows the last. *run is a run of yours from a shaping call, or NULL for a new one;
 * on DAEGUN_OK it is the shaped run, the same handle unless the shape cache still held it, and any other status
 * leaves it as it was. It neither reads nor fills the shape cache. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_shape_into(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                       const daegun_location *DAEGUN_NULLABLE location,
                       const daegun_shape_options *DAEGUN_NULLABLE opts,
                       daegun_shaped_run *DAEGUN_NULLABLE *DAEGUN_NONNULL run);

typedef struct daegun_bidi_runs daegun_bidi_runs;

/* A paragraph's base direction, wherever a call takes one: AUTO lets its first strong character decide. */
#define DAEGUN_BASE_AUTO -1
#define DAEGUN_BASE_LTR   0
#define DAEGUN_BASE_RTL   1

/** Text of any direction, as bidi runs in visual order, each cut where its script changes, as
 * daegun_font_layout cuts its lines, and each shaped as its script. Options that name a script keep each
 * bidi run whole; a seed script stands in for a run with no script of its own. A NULL opts means the
 * defaults, and their direction is the one bidi resolves for each run. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_shape_bidi(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                       const daegun_location *DAEGUN_NULLABLE location, int32_t base,
                       const daegun_shape_options *DAEGUN_NULLABLE opts,
                       daegun_bidi_runs *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_bidi_runs_count(const daegun_bidi_runs *runs, size_t *out);
/** One run: the shaped run, BORROWED until the set is freed (never pass it to daegun_shaped_run_free), its
 * embedding level, and the bytes it covers, start to end, end not included. Any out may be NULL;
 * DAEGUN_INVALID_ARGUMENT past the end. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_bidi_runs_at(const daegun_bidi_runs *runs, size_t index,
                    const daegun_shaped_run *DAEGUN_NULLABLE *DAEGUN_NULLABLE out_run,
                    uint8_t *DAEGUN_NULLABLE out_level, size_t *DAEGUN_NULLABLE out_start,
                    size_t *DAEGUN_NULLABLE out_end);
DAEGUN_API void DAEGUN_CALL
daegun_bidi_runs_free(daegun_bidi_runs *DAEGUN_NULLABLE runs);

/** The caret before each byte, every byte of a character at its leading edge, then the text's trailing
 * edge, from its left (or top) edge, into `out`, room for `out_len`; text_len + 1 always suffices.
 * Horizontal text takes its runs as daegun_font_shape_bidi cuts them, so the carets sit where those runs
 * draw; vertical text is one run, top to bottom. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) DAEGUN_WRITES(6, 7) daegun_status DAEGUN_CALL
daegun_font_caret_positions(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text,
                            size_t text_len, const daegun_location *DAEGUN_NULLABLE location, int32_t orientation,
                            DAEGUN_OUT(out_len) double *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_feature) == 16, "a feature is a tag, a value and a range");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_offset) == 16, "an offset is two doubles");
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_shape_options) == 112, "shape options end without padding");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_shape_options, direction) == 8, "shape options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_shape_options, before) == 40, "shape options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_shape_options, cluster_level) == 72, "shape options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_shape_options, seed_script) == 92, "shape options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_shape_options, beginning_of_text) == 96, "shape options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_shape_options, point_size) == 104, "shape options layout");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_SHAPE_H */
