/* Fonts: opening one, what it is, its axes, named instances and locations, its names, its tables and
 * instances, and its caches. Rust's Font, Location, Axis, NamedInstance and Cache. */

#ifndef DAEGUN_FONT_H
#define DAEGUN_FONT_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* opening */

/** Opens a font. The bytes are COPIED, so you may free them the moment this returns. DAEGUN_MISSING for a
 * font whose head gives no unitsPerEm or whose maxp gives no glyph count: everything measured rests on them. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_font_open(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len,
                 daegun_font *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** Frees a buffer you took but never handed over; `len` must be the length daegun_font_buffer_new was
 * given, not how much you filled. NULL is a no-op. Do NOT call this after daegun_font_open_owned;
 * that took ownership, whether it succeeded or not. */
DAEGUN_API void DAEGUN_CALL
daegun_font_buffer_free(uint8_t *DAEGUN_NULLABLE data, size_t len);

/** Opens the face at `index` of a collection, or a font file's own at 0, from a daegun buffer without a copy.
 * The buffer belongs to the font afterwards either way: freed with daegun_font_free, or at once if the
 * bytes did not parse or out is NULL. `len` must be exactly what you passed to daegun_font_buffer_new.
 * A `len` of 0 is an empty font, DAEGUN_MALFORMED, and takes no buffer, as daegun_font_buffer_new gives
 * none that long. DAEGUN_INVALID_ARGUMENT for an index past the faces daegun_face_count counts. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_open_owned(uint8_t *DAEGUN_NULLABLE data, size_t len, size_t index,
                       daegun_font *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** A buffer for a font, allocated by daegun so it can be handed back without a copy.
 *
 * daegun_font_open copies your bytes so you may free them the moment it returns. This is the way
 * around that copy: take a buffer, read the file into it, hand it back:
 *
 *     uint8_t *buf = daegun_font_buffer_new(len);
 *     if (!buf || fread(buf, 1, len, fp) != len) { daegun_font_buffer_free(buf, len); return; }
 *     daegun_font *font = NULL;
 *     daegun_status st = daegun_font_open_owned(buf, len, 0, &font);
 *     // buf belongs to the font now, pass or fail. Do not free it, do not read it.
 *
 * NULL for a zero length, or when the allocation fails. This is not an ordinary pointer: never free()
 * it. */
DAEGUN_NODISCARD DAEGUN_API
DAEGUN_MALLOC(malloc(daegun_font_buffer_free, 1), malloc(daegun_font_open_owned, 1)) DAEGUN_ALLOC_SIZE(1, len)
uint8_t *DAEGUN_NULLABLE DAEGUN_CALL
daegun_font_buffer_new(size_t len);

/** The face at `index` of a collection, or a font file's own at 0, COPIED as daegun_font_open copies: the face's
 * tables from a collection, the whole file from a font file. DAEGUN_INVALID_ARGUMENT for an index past the faces
 * daegun_face_count counts. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_font_open_collection(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t index,
                            daegun_font *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** The face at `index` of a collection, or a font file's own at 0, read where it lies: NOTHING is copied. The bytes
 * must stay valid and unchanged for the rest of the program, as a static array or a file mapped and never unmapped
 * do, since the font, and every location, run or other handle made from it, reads them in place. C23's and C++26's
 * #embed builds a font file into such an array. Refused as daegun_font_open_collection refuses. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_font_open_static(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t index,
                        daegun_font *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** Frees a font. NULL is a no-op, as free(NULL) is. */
DAEGUN_API void DAEGUN_CALL
daegun_font_free(daegun_font *DAEGUN_NULLABLE font);

/** How many faces the bytes hold, each opened at its index by daegun_font_open_collection or
 * daegun_font_open_static: a collection's faces, 1 for a font file, 0 for bytes that are neither. Takes bytes
 * rather than a font, because it answers what a caller asks before deciding which face to open. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_face_count(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t *out);

/* what it is */

/** How many glyphs the face has: MAXP's 24-bit count where it reads whole and glyf and loca, or CFF2's
 * CharStrings, hold that many glyphs, else maxp's. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_glyph_count(const daegun_font *font, uint32_t *out);

/** Units per em: the grid every metric, advance and outline here is measured on. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_units_per_em(const daegun_font *font, uint16_t *out);

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_is_variable(const daegun_font *font, bool *out);

/* axes */

/** One axis of a variable font, as fvar declares it. hidden: the font asks for the axis to be kept out of
 * user interfaces. */
typedef struct {
    daegun_tag tag;
    uint16_t   name_id;
    bool       hidden;
    double     min;
    double     default_value;
    double     max;
} daegun_axis;

/** The declared axes, in fvar's order, into your buffer (BUFFERS); none for a static face. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(2, 3) daegun_status DAEGUN_CALL
daegun_font_axes(const daegun_font *font, DAEGUN_OUT(out_len) daegun_axis *DAEGUN_NULLABLE out, size_t out_len,
                 size_t *out_count);

/* named instances */

typedef struct daegun_named_instances daegun_named_instances;

/** The instances fvar names, in its order. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_named_instances(const daegun_font *font, daegun_named_instances *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_named_instances_count(const daegun_named_instances *list, size_t *out);
/** An instance's subfamily name and PostScript name, BORROWED until the list is freed. DAEGUN_ABSENT where
 * it states none; DAEGUN_INVALID_ARGUMENT past the end. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_named_instances_name(const daegun_named_instances *list, size_t index, daegun_str *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_named_instances_postscript_name(const daegun_named_instances *list, size_t index, daegun_str *out);
/** Its coordinates, BORROWED until the list is freed, to make a location of with daegun_font_location.
 * NULL past the end. */
DAEGUN_API const daegun_coord *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_named_instances_coords(const daegun_named_instances *list, size_t index, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_named_instances_free(daegun_named_instances *DAEGUN_NULLABLE list);

/* locations */

/** A place in a variable font's design space, resolved once and passed to every call that depends on one:
 * metrics, advances, outlines, shaping, layout, color and instances. A value past its axis's range is
 * clamped to it, of a repeated tag the last counts, and a tag the face has no axis for is ignored. A
 * NULL location is the default instance wherever a call takes one, as is a location of no coords.
 *
 * A location keeps the coords it was given: passed to another font, it is resolved again from them, so a
 * value the first font clamped or left out at its default means to the second what it was given as. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_location(const daegun_font *font, DAEGUN_IN(coords_len) const daegun_coord *DAEGUN_NULLABLE coords,
                     size_t coords_len, daegun_location *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_API void DAEGUN_CALL
daegun_location_free(daegun_location *DAEGUN_NULLABLE location);
/** The coordinates it resolved to, in fvar's order, clamped, axes at their default left out; BORROWED until
 * the location is freed. NULL when location or out_count is NULL. */
DAEGUN_API const daegun_coord *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_location_coords(const daegun_location *location, size_t *out_count);
/** One coordinate from -1 to 1 per fvar axis, in fvar's order, avar applied; BORROWED as above. Empty at the
 * default instance, where every one is 0. */
DAEGUN_API const double *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_location_normalized(const daegun_location *location, size_t *out_count);

/* names */

/** The family name at a location, read from the name table an instance there would have: the typographic
 * family, name ID 16, else the family, ID 1. DAEGUN_ABSENT where the face states none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_family_name(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                        daegun_text *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/* Whether a font's glyphs stand upright or lean, as its OS/2 and head tables say. */
#define DAEGUN_STYLE_NORMAL  0
#define DAEGUN_STYLE_ITALIC  1
#define DAEGUN_STYLE_OBLIQUE 2  /* a slanted upright, as fsSelection may mark beside italic, which it comes before */

/** A DAEGUN_STYLE_* code; the name table's style is daegun_font_name's, name ID 2. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_style(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, int32_t *out);

/** One name by its ID, at a location, as daegun_font_family_name reads it: of its records, an English one
 * for Windows, else another for Windows, else an English one for the Mac in Mac Roman, else another in Mac
 * Roman, else a Unicode one, the first of each in the table's order, trimmed of NULs and spaces.
 * DAEGUN_ABSENT where the face states none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_name(const daegun_font *font, uint16_t name_id, const daegun_location *DAEGUN_NULLABLE location,
                 daegun_text *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

typedef struct daegun_names daegun_names;

/** Every name the `name` table holds at a location, each with its name ID, in ascending order of ID, each
 * read as daegun_font_name reads it. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_names(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                  daegun_names *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_names_count(const daegun_names *names, size_t *out);
/** The name at `index`: its ID and its string, BORROWED until the names are freed. DAEGUN_INVALID_ARGUMENT
 * past the end. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_names_at(const daegun_names *names, size_t index, uint16_t *out_id, daegun_str *out);
DAEGUN_API void DAEGUN_CALL
daegun_names_free(daegun_names *DAEGUN_NULLABLE names);

typedef struct daegun_localized_names daegun_localized_names;

/** One name by its ID in each language the `name` table at a location records it in, sorted by language; none
 * where the table has no record for the ID. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_localized_names(const daegun_font *font, uint16_t name_id, const daegun_location *DAEGUN_NULLABLE location,
                            daegun_localized_names *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_localized_names_count(const daegun_localized_names *names, size_t *out);
/** The name at `index`: its language as a BCP 47 tag, such as "ja" or "zh-TW", "und" where its record names none
 * daegun reads, and its string, both BORROWED until the names are freed. DAEGUN_INVALID_ARGUMENT past the end. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_localized_names_at(const daegun_localized_names *names, size_t index, daegun_str *out_language,
                          daegun_str *out);
DAEGUN_API void DAEGUN_CALL
daegun_localized_names_free(daegun_localized_names *DAEGUN_NULLABLE names);

/* tables and instances */

/** One table's bytes, exactly as the file stores them. BORROWED: valid until the font is freed.
 * DAEGUN_ABSENT when the font has no such table, which is not the same as a table that is empty.
 *
 * These are the font's bytes, not an instance's: a variable font's `glyf` here is the stored
 * default shape. daegun_font_instance_tables is what resolves a location. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_table(const daegun_font *font, daegun_tag tag, daegun_bytes *out);

/** Every table the font carries, in sorted order, as tags, into your buffer (BUFFERS). */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(2, 3) daegun_status DAEGUN_CALL
daegun_font_table_tags(const daegun_font *font, DAEGUN_OUT(out_len) daegun_tag *DAEGUN_NULLABLE out, size_t out_len,
                       size_t *out_count);

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_has_table(const daegun_font *font, daegun_tag tag, bool *out);

/** The face instanced at a location, as a complete font file; a static face is its own instance. The bytes are
 * the font's instance cache's, shared rather than copied, so a second call at the location builds nothing. A
 * location the instancer can't build answers its failure's status, with the reason in daegun_last_error(). */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_instance(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                     daegun_blob *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** A tag-to-bytes map daegun owns: what instancing produces, and what building an sfnt takes
 * (daegun_table_map_build, in raw.h).
 *
 * Writable, because the Rust API's build_font takes any map, so a caller can instance a font, drop
 * a table, patch another, and build the result. A read-only view would translate the type and lose
 * the point of it. */
typedef struct daegun_table_map daegun_table_map;

/** Every table of the font pinned to a location. A static font comes back as stored; a location the
 * instancer can't build answers as daegun_font_instance does.
 *
 * The bytes are COPIED out of the font. The Rust call borrows wherever a table passes through
 * untouched, and that borrow cannot cross into C: nothing would stop the font being freed while
 * the map still pointed into it. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_instance_tables(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                            daegun_table_map *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

DAEGUN_API void DAEGUN_CALL
daegun_table_map_free(daegun_table_map *DAEGUN_NULLABLE map);
DAEGUN_NODISCARD DAEGUN_API DAEGUN_MALLOC(malloc(daegun_table_map_free, 1), returns_nonnull)
daegun_table_map * DAEGUN_CALL
daegun_table_map_new(void);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_table_map_count(const daegun_table_map *map, size_t *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_table_map_tag_at(const daegun_table_map *map, size_t index, daegun_tag *out);
/** BORROWED, and invalidated by _set and _remove as well as by _free. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_table_map_bytes_at(const daegun_table_map *map, size_t index, daegun_bytes *out);
/** DAEGUN_ABSENT for a tag the map lacks. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_table_map_get(const daegun_table_map *map, daegun_tag tag, daegun_bytes *out);
/** Copies the bytes in, in place of any the tag held. */
DAEGUN_API DAEGUN_READS(3, 4) daegun_status DAEGUN_CALL
daegun_table_map_set(daegun_table_map *map, daegun_tag tag, DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data,
                     size_t len);
/** Removing a tag the map lacks does nothing. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_table_map_remove(daegun_table_map *map, daegun_tag tag);

/* caches */

/* The caches, each a ceiling grown into rather than memory reserved up front. Every one of them
 * is dead weight for some caller: text that never repeats gains nothing from the shape cache.
 * Defaults are 8 MB shape, 64 MB instance, 4 MB outline, and 0 raster, which is off until set.
 *
 * SHAPE: shaped runs, keyed by text. Each run is charged for its entry as well as its text and
 * glyphs, so a budget of 0 keeps none.
 * INSTANCE: every location other than the default, under one budget: the tables each changes, what
 * drawing, metrics and shaping build there, and its whole instance. The newest location is kept
 * whatever the budget, so one fixed weight runs on a budget of 0; a CFF2 font is instanced even at its
 * default. A thread keeps the location it last read past the budget's drop, until it reads another
 * there or ends.
 * OUTLINE: decoded outlines, which the outline calls and the autohinter replay instead of decoding
 * again; daegun_font_prewarm fills it.
 * RASTER: images a rasterizer drew of the font's glyphs, lent again instead of drawn.
 * Each may also draw from a pool other fonts' caches share: see daegun_font_set_cache_pool. */
#define DAEGUN_CACHE_SHAPE    0
#define DAEGUN_CACHE_INSTANCE 1
#define DAEGUN_CACHE_OUTLINE  2
#define DAEGUN_CACHE_RASTER   3

typedef struct {
    size_t entries;
    size_t bytes;     /**< as the budget counts them */
} daegun_cache_stats;

/** A lower budget applies at once, keeping what was used last; the shape cache keeps runs read again since they
 * came in ahead of those that weren't, rather than strictly in the order last used. A shape budget of 4 MB or more
 * is split over lists, each keeping its own, and one of 0 sets them aside, so threads shaping meet at no lock.
 * DAEGUN_INVALID_ARGUMENT for a cache code not above. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_set_cache_budget(const daegun_font *font, int32_t cache, size_t bytes);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_cache_stats(const daegun_font *font, int32_t cache, daegun_cache_stats *out);
DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_clear_cache(const daegun_font *font, int32_t cache);

/** A budget the caches of several fonts share, each joining it with daegun_font_set_cache_pool: past it, the entry
 * least recently used across all of them goes first, whichever font's. Caches of several kinds may share one. It may
 * be shared across threads as a font may, and freed while fonts are in it: they hold it until they leave or go. */
typedef struct daegun_cache_pool daegun_cache_pool;

DAEGUN_API void DAEGUN_CALL
daegun_cache_pool_free(daegun_cache_pool *DAEGUN_NULLABLE pool);
/** A pool of `bytes`, a ceiling its caches grow into together, never memory reserved. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_MALLOC(malloc(daegun_cache_pool_free, 1), returns_nonnull)
daegun_cache_pool * DAEGUN_CALL
daegun_cache_pool_new(size_t bytes);
/** A lower budget applies at once, keeping what was used last across its caches. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_cache_pool_set_budget(const daegun_cache_pool *pool, size_t bytes);
/** What the caches in the pool hold now, together. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_cache_pool_stats(const daegun_cache_pool *pool, daegun_cache_stats *out);
/** Draws a cache's bytes from a pool, or with a NULL pool from the font's budget alone again. It keeps to both
 * budgets: past its own it lets its oldest go, and past the pool's the oldest across every cache in the pool goes,
 * whichever font's; a budget of SIZE_MAX leaves the pool alone in charge. What it holds moves with it, oldest first,
 * and a font freed gives the pool its bytes back. In a pool, every cache keeps entries read again since they came in
 * ahead of those that weren't, and each font's newest location stays past the pool's budget as past its own.
 * DAEGUN_INVALID_ARGUMENT for a cache code not above. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_set_cache_pool(const daegun_font *font, int32_t cache, const daegun_cache_pool *DAEGUN_NULLABLE pool);

/** Draws glyphs at a location ahead of use, into the outline cache. out_added, which may be NULL, counts
 * the outlines the cache took: none past the outline budget, and a full cache drops its oldest to take
 * each new one. */
DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_font_prewarm(const daegun_font *font, DAEGUN_IN(gids_len) const uint32_t *DAEGUN_NULLABLE gids, size_t gids_len,
                    const daegun_location *DAEGUN_NULLABLE location, size_t *DAEGUN_NULLABLE out_added);

/** Unlike the caches, this is what is left to spend rather than a ceiling, shared by every index the font
 * and its locations build: building one draws it down, and a location gives its indexes' bytes back when
 * it is dropped. 4 MiB plus eight times the bytes of the font's tables by default, more than any real font's
 * indexes take. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_set_index_allowance(const daegun_font *font, size_t bytes);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_index_allowance(const daegun_font *font, size_t *out);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_axis) == 32, "an axis is a tag, a name ID and a flag, then three doubles");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_axis, name_id) == 4, "axis layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_axis, hidden) == 6, "axis layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_axis, min) == 8, "axis layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_cache_stats) == 2 * sizeof(size_t), "cache stats are two size_t");

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_FONT_H */
