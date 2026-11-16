/* Layout: text laid out in lines, aligned, in either writing mode, and one line justified by JSTF. Rust's
 * layout module and Font::layout. */

#ifndef DAEGUN_LAYOUT_H
#define DAEGUN_LAYOUT_H

#include <daegun/base.h>
#include <daegun/shape.h>
#include <daegun/unicode.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* options */

#define DAEGUN_ALIGN_START    0
#define DAEGUN_ALIGN_END      1
#define DAEGUN_ALIGN_CENTER   2
#define DAEGUN_ALIGN_JUSTIFY  3

#define DAEGUN_WRITING_HORIZONTAL   0
#define DAEGUN_WRITING_VERTICAL_RL  1
#define DAEGUN_WRITING_VERTICAL_LR  2

/* How vertical text sets its characters: each as UAX #50 says, all upright, or all on their side. */
#define DAEGUN_TEXT_ORIENTATION_MIXED     0
#define DAEGUN_TEXT_ORIENTATION_UPRIGHT   1
#define DAEGUN_TEXT_ORIENTATION_SIDEWAYS  2

#define DAEGUN_BREAK_GREEDY   0
#define DAEGUN_BREAK_OPTIMAL  1

/** Whether a DAEGUN_WRITING_* sets lines top to bottom. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_writing_mode_is_vertical(int32_t mode, bool *out);

/** Everything a layout can be told, as Rust's LayoutOptions. Fill it with daegun_layout_options_default:
 * zeroing it is not the default, as a measure of 0 would wrap after every glyph. */
typedef struct {
    size_t      size;              /**< sizeof(daegun_layout_options) as your header has it */
    /** The measure, in font units. A word no line can hold is cut between graphemes, as CSS's overflow-wrap:
     * anywhere cuts it, so a line passes it only where one glyph cluster does. Infinity, the default,
     * doesn't wrap, and the lines align against the widest; NaN reads as infinity, and below 0 as 0. */
    double      max_inline_size;
    double      line_height;       /**< baseline to baseline, with has_line_height; else each line's own */
    /** The distance between tab stops, in advances of the space, as CSS's tab-size counts it: 8 by default.
     * A tab shapes as a space, reaches the next stop from the line's start edge and hangs at a line's end.
     * Justifying widens only the spaces after a line's last tab. */
    double      tab_size;
    const char *DAEGUN_NULLABLE language; /**< a BCP 47 tag, NUL-terminated; NULL for none */
    /** Where SA text breaks between words, called with segmenter_user. NULL takes the dictionaries
     * feature's word lists, which daegun_has_dictionaries reports, and without them SA text breaks only
     * where other text would. */
    daegun_word_segmenter DAEGUN_NULLABLE segmenter;
    void       *DAEGUN_NULLABLE segmenter_user;
    size_t      max_lines;         /**< with has_max_lines; daegun_layout_info says where the text was cut */
    bool        has_line_height;
    bool        has_max_lines;
    int32_t     align;             /**< DAEGUN_ALIGN_* */
    int32_t     writing_mode;      /**< DAEGUN_WRITING_* */
    int32_t     text_orientation;  /**< DAEGUN_TEXT_ORIENTATION_* */
    int32_t     base_direction;    /**< DAEGUN_BASE_*, AUTO by default */
    int32_t     strategy;          /**< DAEGUN_BREAK_* */
    /** OpenType features to turn on, off or set, each over its bytes of the text: small capitals, tabular
     * figures. */
    const daegun_feature *DAEGUN_NULLABLE features DAEGUN_COUNTED_BY(features_len);
    size_t      features_len;
} daegun_layout_options;

/** Sets the defaults in `size` bytes of *out and its size; pass sizeof *out. DAEGUN_INVALID_ARGUMENT for a
 * size below 2.0's, and DAEGUN_UNSUPPORTED for one larger than this library knows, each writing nothing. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_layout_options_default(daegun_layout_options *out, size_t size);

/* layout */

typedef struct daegun_layout daegun_layout;

/** Text laid out in lines at a location. A NULL opts means the defaults. A line that breaks at a soft hyphen
 * (U+00AD) shows the font's hyphen, U+2010 or else U+002D, and the optimal strategy charges such a break
 * demerits: 2,500 each, a penalty of 50 squared, and 10,000 more for two in a row. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_layout(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                   const daegun_location *DAEGUN_NULLABLE location, const daegun_layout_options *DAEGUN_NULLABLE opts,
                   daegun_layout *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_API void DAEGUN_CALL
daegun_layout_free(daegun_layout *DAEGUN_NULLABLE layout);

/** The whole: its lines, the widest line's extent, the extent across, first ascent to last descent, and,
 * with has_truncated, the byte where the text left out past max_lines begins. */
typedef struct {
    size_t line_count;
    double inline_size;
    double block_size;
    size_t truncated;
    bool   has_truncated;
} daegun_layout_info;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_layout_info_of(const daegun_layout *layout, daegun_layout_info *out);

/** One line: its runs, bytes start to end, its baseline, extent, ascent, descent (negative below the
 * baseline), and whether a hard break ends it. The baseline places it in the block: from its top edge, or
 * in vertical text from its left edge, with lines advancing rightward in vertical-lr and leftward in
 * vertical-rl. */
typedef struct {
    size_t run_count;
    size_t start;
    size_t end;
    double baseline;
    double inline_size;
    double ascent;
    double descent;
    bool   hard_break;
} daegun_layout_line;

/** DAEGUN_INVALID_ARGUMENT past the last line. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_layout_line_at(const daegun_layout *layout, size_t line, daegun_layout_line *out);

/** One run of a line, in visual order: the run as shaped, BORROWED until the layout is freed (never pass it
 * to daegun_shaped_run_free; daegun_shaped_run_copy keeps it past the layout), its clusters counting from
 * start, where it starts from the line's origin, its bidi level, its bytes start to end, and whether it
 * stands upright, as horizontal text always does, rather than on its side in a vertical line. */
typedef struct {
    const daegun_shaped_run *DAEGUN_NULLABLE run;
    daegun_offset offset;
    size_t        start;
    size_t        end;
    uint8_t       level;
    bool          upright;
} daegun_layout_run;

/** DAEGUN_INVALID_ARGUMENT past the line's last run, or the last line. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_layout_run_at(const daegun_layout *layout, size_t line, size_t run, daegun_layout_run *out);

/* justification */

/** What to justify a line to, as Rust's JustifyOptions: the width to reach in font units, how far from it a
 * width may be and still count, and whose JSTF priorities apply: an OpenType script, whose levels apply only where
 * JSTF lists it (0 names none, so the line keeps its natural shaping), and a language system, 0 for the script's
 * default. Fill it with daegun_justify_options_default, whose defaults are all 0. */
typedef struct {
    size_t     size;              /**< sizeof(daegun_justify_options) as your header has it */
    daegun_tag script;
    daegun_tag language_system;
    double     target_width;
    double     tolerance;
} daegun_justify_options;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_justify_options_default(daegun_justify_options *out, size_t size);

/** How a line was justified: its width, the JSTF priority level it was shaped at with has_level (none
 * where its natural width was kept), which way it had to go, DAEGUN_ADJUST_SHRINK where its natural width is past
 * the target by more than the tolerance, else DAEGUN_ADJUST_EXTEND, and whether no level came within the
 * tolerance, so it is only the nearest. */
typedef struct {
    double  width;
    size_t  level;
    int32_t adjustment;
    bool    has_level;
    bool    best_effort;
} daegun_justification;

/** Justifies one line at a location, along an orientation: its natural shaping where that is within the
 * tolerance, else the first JSTF priority level that comes within it, else the nearest. out_info may be
 * NULL. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_justify(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                    const daegun_location *DAEGUN_NULLABLE location, int32_t orientation,
                    const daegun_justify_options *DAEGUN_NULLABLE opts,
                    daegun_shaped_run *DAEGUN_NULLABLE *DAEGUN_NONNULL out_run,
                    daegun_justification *DAEGUN_NULLABLE out_info);

/** The glyphs JSTF lists for a script as extenders, which a justifier may repeat to widen a line, such as
 * the tatweel, into your buffer (BUFFERS). DAEGUN_ABSENT where JSTF lists none. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_font_justification_glyphs(const daegun_font *font, daegun_tag script,
                                 DAEGUN_OUT(out_len) uint32_t *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

/** A script's JSTF priority levels for a language system (0 for its default, as is one without data),
 * highest priority first. DAEGUN_ABSENT where JSTF has no data for the script. JstfMax, which nothing
 * applies, is not read. */
typedef struct daegun_jstf_priorities daegun_jstf_priorities;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_justification_priorities(const daegun_font *font, daegun_tag script, daegun_tag language_system,
                                     daegun_jstf_priorities *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_jstf_priorities_count(const daegun_jstf_priorities *priorities, size_t *out);
DAEGUN_API void DAEGUN_CALL
daegun_jstf_priorities_free(daegun_jstf_priorities *DAEGUN_NULLABLE priorities);

/* Which of a level's eight lookup lists. */
#define DAEGUN_JSTF_SHRINKAGE_ENABLE_GSUB   0
#define DAEGUN_JSTF_SHRINKAGE_DISABLE_GSUB  1
#define DAEGUN_JSTF_SHRINKAGE_ENABLE_GPOS   2
#define DAEGUN_JSTF_SHRINKAGE_DISABLE_GPOS  3
#define DAEGUN_JSTF_EXTENSION_ENABLE_GSUB   4
#define DAEGUN_JSTF_EXTENSION_DISABLE_GSUB  5
#define DAEGUN_JSTF_EXTENSION_ENABLE_GPOS   6
#define DAEGUN_JSTF_EXTENSION_DISABLE_GPOS  7

/** One of a level's lookup lists, BORROWED until the priorities are freed. DAEGUN_ABSENT where the level
 * names no such list, which is not a list naming no lookups; DAEGUN_INVALID_ARGUMENT past the last level. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_jstf_priorities_lookups(const daegun_jstf_priorities *priorities, size_t level, int32_t which,
                               const uint16_t *DAEGUN_NULLABLE *DAEGUN_NONNULL out, size_t *out_count);

/* Which way a JSTF level adjusts a line. */
#define DAEGUN_ADJUST_SHRINK  0
#define DAEGUN_ADJUST_EXTEND  1

/** Shapes text at a location with one level's lookups applied one way. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_shape_justified(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text,
                            size_t text_len, const daegun_location *DAEGUN_NULLABLE location, int32_t orientation,
                            const daegun_jstf_priorities *priorities, size_t level, int32_t adjustment,
                            daegun_shaped_run *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_layout_options) == 104, "layout options end without padding");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_layout_options, max_inline_size) == 8, "layout options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_layout_options, max_lines) == 56, "layout options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_layout_options, align) == 68, "layout options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_layout_options, strategy) == 84, "layout options layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_justify_options) == 32, "justify options are a size, two tags, two doubles");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_layout_info) == 40, "layout info layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_layout_line) == 64, "layout line layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_layout_run) == 48, "layout run layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_justification) == 24, "justification layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_justification, adjustment) == 16, "justification layout");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_LAYOUT_H */
