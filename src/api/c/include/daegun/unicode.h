/* Unicode text analysis, which needs no font: bidi, segmentation, line breaking, scripts and categories. Rust's
 * unicode module. Positions are byte offsets where a character starts, as everywhere here. */

#ifndef DAEGUN_UNICODE_H
#define DAEGUN_UNICODE_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* segmentation */

/** Where the text's grapheme clusters (UAX #29) begin, with its start and end, into `out`, room for
 * `out_len`; text_len + 1 always suffices. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_grapheme_boundaries(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                           DAEGUN_OUT(out_len) size_t *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);
/** The text's word boundaries (UAX #29), with its start and end, as above. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_word_boundaries(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                       DAEGUN_OUT(out_len) size_t *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

/* line breaking */

/** Your word segmentation for one run of text written without spaces between words, UAX #14's class SA:
 * Thai, Lao, Khmer, Myanmar and others. `text` is the run's `len` bytes of UTF-8. Write the byte offsets
 * from its start of the boundaries inside it to `out`, ascending, at most `cap` of them (one fewer than
 * the run's characters, room for every one), and return how many were written; one where no character
 * starts is ignored. It must return normally, as CALLBACKS in base.h says, and may call back into daegun, on
 * the font being laid out too: no lock is held while it runs. */
typedef size_t (DAEGUN_CALLBACK *daegun_word_segmenter)(void *DAEGUN_NULLABLE user, DAEGUN_IN(len) const char *text,
                                                        size_t len, DAEGUN_OUT(cap) size_t *out, size_t cap);

/** A place a line may break: the byte the next line starts at, and whether the text demands the break
 * rather than allows it, as after a newline and at the end. */
typedef struct {
    size_t at;
    bool   mandatory;
} daegun_line_break;

/** Where lines may break in the text, the end included, into `out`, room for `out_len`; text_len + 1 always
 * suffices. SA text breaks between words where `segmenter` finds them, called with `user`, or with none,
 * where the dictionaries feature's word lists do, and without them not at all. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_WRITES(5, 6) daegun_status DAEGUN_CALL
daegun_line_break_opportunities(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                                daegun_word_segmenter DAEGUN_NULLABLE segmenter, void *DAEGUN_NULLABLE user,
                                DAEGUN_OUT(out_len) daegun_line_break *DAEGUN_NULLABLE out, size_t out_len,
                                size_t *out_count);
/** Whether this build has the dictionaries feature's word lists, which break SA text with no segmenter. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST bool DAEGUN_CALL
daegun_has_dictionaries(void);

/* scripts */

/* A script is its ISO 15924 code packed as a tag, such as DAEGUN_TAG('A','r','a','b'), which names it in
 * every build; DAEGUN_TAG('Z','y','y','y') is Common and DAEGUN_TAG('Z','i','n','h') Inherited. Read in
 * any case, answered in title case. A tag naming no script this build knows answers DAEGUN_INVALID_ARGUMENT. */

/** One run of one script (UAX #24): bytes start to end, end not included. */
typedef struct {
    size_t     start;
    size_t     end;
    daegun_tag script;
} daegun_script_run;

/** The text's runs of one script, a Common or Inherited character joining the run around it, into `out`,
 * room for `out_len`; text_len always suffices. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_script_runs(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                   DAEGUN_OUT(out_len) daegun_script_run *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);
/** Its Unicode name, such as "Arabic", BORROWED: a static string. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_script_name(daegun_tag script, daegun_str *out);
/* Which way a script is written across a line: the direction most of its letters' Bidi classes give, so a
 * script once written either way, as Old Italic was, answers LEFT_TO_RIGHT. */
#define DAEGUN_SCRIPT_DIRECTION_LEFT_TO_RIGHT 0
#define DAEGUN_SCRIPT_DIRECTION_RIGHT_TO_LEFT 1

/** Which way it's written, a DAEGUN_SCRIPT_DIRECTION_*. Common and Inherited answer LEFT_TO_RIGHT. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_script_direction(daegun_tag script, int32_t *out);
/** The OpenType script tags it maps to, most specific first: Devanagari is 'dev3', 'dev2', 'deva', and a
 * shaper tries them in order. Into `out`, room for `out_len`; 3 always suffices. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(2, 3) daegun_status DAEGUN_CALL
daegun_script_opentype_tags(daegun_tag script, DAEGUN_OUT(out_len) daegun_tag *DAEGUN_NULLABLE out, size_t out_len,
                            size_t *out_count);
/** Whether it takes its identity from what surrounds it: true for Common and Inherited. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_script_is_context_dependent(daegun_tag script, bool *out);

/* characters */

/* A character's general category, a DAEGUN_GC_* in Unicode's order of the two-letter codes. */
#define DAEGUN_GC_UNASSIGNED            0   /* Cn */
#define DAEGUN_GC_CONTROL               1   /* Cc */
#define DAEGUN_GC_FORMAT                2   /* Cf */
#define DAEGUN_GC_PRIVATE_USE           3   /* Co */
#define DAEGUN_GC_SURROGATE             4   /* Cs */
#define DAEGUN_GC_LOWERCASE_LETTER      5   /* Ll */
#define DAEGUN_GC_MODIFIER_LETTER       6   /* Lm */
#define DAEGUN_GC_OTHER_LETTER          7   /* Lo */
#define DAEGUN_GC_TITLECASE_LETTER      8   /* Lt */
#define DAEGUN_GC_UPPERCASE_LETTER      9   /* Lu */
#define DAEGUN_GC_SPACING_MARK         10   /* Mc */
#define DAEGUN_GC_ENCLOSING_MARK       11   /* Me */
#define DAEGUN_GC_NONSPACING_MARK      12   /* Mn */
#define DAEGUN_GC_DECIMAL_NUMBER       13   /* Nd */
#define DAEGUN_GC_LETTER_NUMBER        14   /* Nl */
#define DAEGUN_GC_OTHER_NUMBER         15   /* No */
#define DAEGUN_GC_CONNECT_PUNCTUATION  16   /* Pc */
#define DAEGUN_GC_DASH_PUNCTUATION     17   /* Pd */
#define DAEGUN_GC_CLOSE_PUNCTUATION    18   /* Pe */
#define DAEGUN_GC_FINAL_PUNCTUATION    19   /* Pf */
#define DAEGUN_GC_INITIAL_PUNCTUATION  20   /* Pi */
#define DAEGUN_GC_OTHER_PUNCTUATION    21   /* Po */
#define DAEGUN_GC_OPEN_PUNCTUATION     22   /* Ps */
#define DAEGUN_GC_CURRENCY_SYMBOL      23   /* Sc */
#define DAEGUN_GC_MODIFIER_SYMBOL      24   /* Sk */
#define DAEGUN_GC_MATH_SYMBOL          25   /* Sm */
#define DAEGUN_GC_OTHER_SYMBOL         26   /* So */
#define DAEGUN_GC_LINE_SEPARATOR       27   /* Zl */
#define DAEGUN_GC_PARAGRAPH_SEPARATOR  28   /* Zp */
#define DAEGUN_GC_SPACE_SEPARATOR      29   /* Zs */

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_general_category(uint32_t codepoint, int32_t *out);
/** A character's Script property as its ISO 15924 code, packed as a tag: 'Zyyy' for one many scripts share, such
 * as a digit or a space, and 'Zinh' for a mark that takes its base's. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_script(uint32_t codepoint, daegun_tag *out);
/** The presentation form a character takes in vertical text, from the Vertical Forms and CJK Compatibility
 * Forms blocks, such as U+FE12 for U+3002. DAEGUN_ABSENT for one with none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_vertical_form(uint32_t codepoint, uint32_t *out);
/* How a character sets in vertical text (UAX #50). Upright where it's UPRIGHT or TRANSFORMED_UPRIGHT, and
 * where it's TRANSFORMED_ROTATED and the font draws a vertical form of it. */
#define DAEGUN_VO_ROTATED             0  /* R: sideways, turned a quarter clockwise, as Latin letters set */
#define DAEGUN_VO_TRANSFORMED_ROTATED 1  /* Tr: upright in a vertical form where one is drawn, else sideways */
#define DAEGUN_VO_TRANSFORMED_UPRIGHT 2  /* Tu: upright, in a vertical form where one is drawn */
#define DAEGUN_VO_UPRIGHT             3  /* U: upright, as CJK ideographs set */

/** A DAEGUN_VO_*; a newer library may answer a code this header doesn't name. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_vertical_orientation(uint32_t codepoint, int32_t *out);

/* bidi */

/** Text resolved for bidi (UAX #9), each paragraph's direction set by a DAEGUN_BASE_*: each byte's level,
 * odd for right to left, and the visual order, each paragraph reordered as a line of its own. */
typedef struct daegun_bidi_paragraph daegun_bidi_paragraph;

/** A paragraph: bytes start to end, ending with the separator that ends it, and its level. */
typedef struct {
    size_t  start;
    size_t  end;
    uint8_t level;
} daegun_paragraph_run;

DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_bidi_paragraph_new(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len, int32_t base,
                          daegun_bidi_paragraph *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The first paragraph's level: 0 left to right, 1 right to left. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_bidi_paragraph_base_level(const daegun_bidi_paragraph *paragraph, uint8_t *out);
/** BORROWED, as each view of the handle: a level for each byte, every byte of a character at its level, so
 * any position indexes it. */
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_bidi_paragraph_levels(const daegun_bidi_paragraph *paragraph, size_t *out_count);
/** Each character's start, in visual order, left to right. */
DAEGUN_API const size_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_bidi_paragraph_visual_order(const daegun_bidi_paragraph *paragraph, size_t *out_count);
/** The paragraphs, in order. */
DAEGUN_API const daegun_paragraph_run *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_bidi_paragraph_paragraphs(const daegun_bidi_paragraph *paragraph, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_bidi_paragraph_free(daegun_bidi_paragraph *DAEGUN_NULLABLE paragraph);

/** One run of one level on a line, in visual order: bytes start to end, which an odd level draws last to
 * first. */
typedef struct {
    size_t  start;
    size_t  end;
    uint8_t level;
} daegun_visual_run;

/** A line's runs left to right: bytes start to end of the text `paragraph` resolved, which `text` must be,
 * reordered by the levels of the paragraph holding `start`, into `out`, room for `out_len`; end - start
 * always suffices. None for an empty or reversed range, or one where start or end is inside a character. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) DAEGUN_WRITES(6, 7) daegun_status DAEGUN_CALL
daegun_bidi_paragraph_visual_runs(const daegun_bidi_paragraph *paragraph,
                                  DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len, size_t start,
                                  size_t end, DAEGUN_OUT(out_len) daegun_visual_run *DAEGUN_NULLABLE out,
                                  size_t out_len, size_t *out_count);

#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_line_break) == 16, "a line break is a size_t and a flag");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_script_run) == 24, "a script run is two size_t and a tag");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_paragraph_run) == 24, "a paragraph run is two size_t and a level");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_visual_run) == 24, "a visual run is two size_t and a level");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_UNICODE_H */
