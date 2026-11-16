/* Tables: subsetting with its glyph closure, BASE baselines and extents, and the tags GSUB and GPOS offer.
 * Rust's tables module and Font's calls for them. */

#ifndef DAEGUN_TABLES_H
#define DAEGUN_TABLES_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* subsetting */

typedef struct daegun_subset daegun_subset;

/** A subset of the font, pinned to a location, holding these glyphs and every glyph they reach: components,
 * substitutions and color layers among them. A failure says why the subsetter couldn't build it. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_subset(const daegun_font *font, DAEGUN_IN(gids_len) const uint32_t *DAEGUN_NULLABLE gids, size_t gids_len,
                   const daegun_location *DAEGUN_NULLABLE location, daegun_subset *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** A subset holding what the text maps to and every glyph that reaches, its cmap keeping only the text's
 * characters. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_subset_text(const daegun_font *font, DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len,
                        const daegun_location *DAEGUN_NULLABLE location,
                        daegun_subset *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The subset's font file, BORROWED until it is freed. */
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_subset_bytes(const daegun_subset *subset, size_t *out_count);
/** The id glyph `old` of the source has in the subset. DAEGUN_ABSENT where the subset dropped it. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subset_renumbered(const daegun_subset *subset, uint32_t old, uint32_t *out);
DAEGUN_API void DAEGUN_CALL
daegun_subset_free(daegun_subset *DAEGUN_NULLABLE subset);

/** The glyphs a subset of these would hold: them and every glyph they reach, sorted, each once, into your
 * buffer (BUFFERS); room for the font's glyph count always suffices. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) DAEGUN_WRITES(5, 6) daegun_status DAEGUN_CALL
daegun_font_glyph_closure(const daegun_font *font, DAEGUN_IN(gids_len) const uint32_t *DAEGUN_NULLABLE gids,
                          size_t gids_len, const daegun_location *DAEGUN_NULLABLE location,
                          DAEGUN_OUT(out_len) uint32_t *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

/* BASE */

/** One baseline of a script: its tag, such as DAEGUN_TAG('r','o','m','n'), and its position in font units, y
 * for horizontal text and x for vertical. */
typedef struct {
    daegun_tag tag;
    double     position;
} daegun_baseline;

typedef struct daegun_base_script daegun_base_script;

/** A script's baselines at a location along an orientation, DFLT's where BASE gives the script none.
 * DAEGUN_ABSENT for a font without BASE, or whose BASE has neither. `script` is an OpenType script tag. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_base_script(const daegun_font *font, daegun_tag script, const daegun_location *DAEGUN_NULLABLE location,
                        int32_t orientation, daegun_base_script *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The baseline the script's glyphs sit on. DAEGUN_ABSENT where BASE names none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_base_script_default_baseline(const daegun_base_script *script, daegun_tag *out);
/** Its baselines in tag order, BORROWED until it is freed. */
DAEGUN_API const daegun_baseline *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_base_script_baselines(const daegun_base_script *script, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_base_script_free(daegun_base_script *DAEGUN_NULLABLE script);

/** How far a script's glyphs reach across a line, in font units: below and above it, or left and right of a
 * vertical one, each with its has_ where the font gives it. */
typedef struct {
    double min;
    double max;
    bool   has_min;
    bool   has_max;
} daegun_extents;

/** A script's extents at a location along an orientation, for an OpenType language system and a feature where
 * given (0 for none): the language system's, else the script's default, narrowed to the feature's where it has its
 * own. DFLT's where BASE gives the script none; DAEGUN_ABSENT for a font without BASE, or whose BASE has neither. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_base_extents(const daegun_font *font, daegun_tag script, const daegun_location *DAEGUN_NULLABLE location,
                         int32_t orientation, daegun_tag language_system, daegun_tag feature, daegun_extents *out);
/** Whether the font has BASE and none of its coordinates names a glyph, so renumbering glyphs leaves it
 * valid. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_has_glyph_free_base(const daegun_font *font, bool *out);

/* layout tags */

/** The OpenType scripts GSUB and GPOS offer, sorted, each once, into your buffer (BUFFERS), as are the two
 * lists after it. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(2, 3) daegun_status DAEGUN_CALL
daegun_font_script_tags(const daegun_font *font, DAEGUN_OUT(out_len) daegun_tag *DAEGUN_NULLABLE out, size_t out_len,
                        size_t *out_count);
/** The OpenType language systems GSUB and GPOS offer for a script, sorted, each once. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_font_language_system_tags(const daegun_font *font, daegun_tag script,
                                 DAEGUN_OUT(out_len) daegun_tag *DAEGUN_NULLABLE out, size_t out_len,
                                 size_t *out_count);
/** The features GSUB and GPOS offer a script and OpenType language system, sorted, each once: for a script of 0,
 * the one shaping falls back to (DFLT, dflt, then latn), and for a language system of 0 or one the script lacks,
 * its default. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(4, 5) daegun_status DAEGUN_CALL
daegun_font_feature_tags(const daegun_font *font, daegun_tag script, daegun_tag language_system,
                         DAEGUN_OUT(out_len) daegun_tag *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_baseline) == 16, "a baseline is a tag and a double");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_extents) == 24, "extents are two doubles and two flags");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_TABLES_H */
