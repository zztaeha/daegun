/* Glyphs: mapping characters to glyphs, a glyph's box, advances, vertical origin, ligature carets, classes
 * and names. Rust's glyph calls on Font. */

#ifndef DAEGUN_GLYPHS_H
#define DAEGUN_GLYPHS_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* characters to glyphs */

/* A code point is a uint32_t: C has no type meaning "a scalar value", and wchar_t is 16 bits on Windows.
 * A surrogate or a value past U+10FFFF answers DAEGUN_INVALID_ARGUMENT rather than a guess. */

/** The glyph a character maps to. DAEGUN_ABSENT when the font has none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_glyph_id(const daegun_font *font, uint32_t codepoint, uint32_t *out);

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_has_glyph(const daegun_font *font, uint32_t codepoint, bool *out);

/** The glyph each character of the text maps to, into out_gids, and into out_present a byte each, 1 where
 * the font has one: 0 is .notdef and every other id is a real glyph in some face, so no id can say "none".
 * Both have room for `out_len`; text_len always suffices. out_present may be NULL. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) DAEGUN_WRITES(4, 6) daegun_status DAEGUN_CALL
daegun_font_glyph_ids(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                      DAEGUN_OUT(out_len) uint32_t *DAEGUN_NULLABLE out_gids,
                      DAEGUN_OUT_OR_NULL(out_len) uint8_t *DAEGUN_NULLABLE out_present, size_t out_len,
                      size_t *out_count);

/** Every character the cmap maps, with the glyph each reaches, at matching indices of your buffers
 * (BUFFERS), either NULL to leave it out. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_coverage(const daegun_font *font, DAEGUN_OUT_OR_NULL(out_len) uint32_t *DAEGUN_NULLABLE out_codepoints,
                     DAEGUN_OUT_OR_NULL(out_len) uint32_t *DAEGUN_NULLABLE out_gids, size_t out_len,
                     size_t *out_count);

/** The glyph a character takes before a variation selector, from cmap format 14. DAEGUN_ABSENT where the
 * pair maps to none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_variation_glyph_id(const daegun_font *font, uint32_t codepoint, uint32_t selector, uint32_t *out);

/* boxes and advances */

/** A glyph's tight ink box at a location. DAEGUN_ABSENT for a glyph that draws nothing; a space has no ink,
 * which differs from a box of zero size. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_glyph_bounds(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                         daegun_bounds *out);

/** Each glyph's advance at a location, along DAEGUN_HORIZONTAL or DAEGUN_VERTICAL, into `out`, which has room
 * for gids_len. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) DAEGUN_WRITES(6, 3) daegun_status DAEGUN_CALL
daegun_font_advances(const daegun_font *font, DAEGUN_IN(gids_len) const uint32_t *DAEGUN_NULLABLE gids, size_t gids_len,
                     const daegun_location *DAEGUN_NULLABLE location, int32_t orientation,
                     DAEGUN_OUT(gids_len) double *DAEGUN_NULLABLE out);
/** One glyph's. Down a line, a font without vmtx advances each glyph by the height between its ascender
 * and descender. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_advance(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                    int32_t orientation, double *out);

/** Where a glyph's vertical origin lies at a location, up from the baseline: VORG's, else the top of its box
 * as vmtx places it, else the font's line centered on the em. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_vertical_origin(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                            double *out);
/** VORG's default vertical origin, up from the baseline; 0 for a TrueType font or one without VORG. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_default_vertical_origin(const daegun_font *font, double *out);

/** A ligature's carets at a location, along an orientation, one per caret the font lists, into your
 * buffers (BUFFERS), either NULL to leave it out. out_present holds a byte each, 0 where a caret cannot be
 * resolved (its value 0). */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_ligature_carets(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                            int32_t orientation, DAEGUN_OUT_OR_NULL(out_len) double *DAEGUN_NULLABLE out_values,
                            DAEGUN_OUT_OR_NULL(out_len) uint8_t *DAEGUN_NULLABLE out_present, size_t out_len,
                            size_t *out_count);

/* classes and names */

#define DAEGUN_GLYPH_CLASS_BASE       0
#define DAEGUN_GLYPH_CLASS_LIGATURE   1
#define DAEGUN_GLYPH_CLASS_MARK       2
#define DAEGUN_GLYPH_CLASS_COMPONENT  3

/** GDEF's class, a DAEGUN_GLYPH_CLASS_*; DAEGUN_ABSENT where GDEF gives none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_glyph_class(const daegun_font *font, uint32_t gid, int32_t *out);
/** GDEF's mark attachment class, 24 bits wide, as ClassDef format 3 is. DAEGUN_ABSENT where GDEF gives none,
 * as a class of 0 is. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_mark_attachment_class(const daegun_font *font, uint32_t gid, uint32_t *out);

/* The longest glyph name, in bytes: post's limit, which CFF's names are held to as well. */
#define DAEGUN_MAX_GLYPH_NAME 63

/** A glyph's name from post, else from the CFF charset, into `out` with a NUL after it, room for `out_len`
 * bytes; DAEGUN_MAX_GLYPH_NAME + 1 always suffices. *out_count is the bytes it takes, the NUL among them.
 * Nothing is allocated. DAEGUN_ABSENT where neither names it; a name the font gives as empty, longer than
 * DAEGUN_MAX_GLYPH_NAME or holding a NUL, which a C string can't, counts as none. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_font_glyph_name(const daegun_font *font, uint32_t gid, DAEGUN_OUT(out_len) char *DAEGUN_NULLABLE out,
                       size_t out_len, size_t *out_count);
/** Every glyph's name, as daegun_font_glyph_name gives it, in one list that holds them in one buffer: an empty
 * string where it gives none, as no name is empty. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_glyph_names(const daegun_font *font, daegun_str_list *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_GLYPHS_H */
