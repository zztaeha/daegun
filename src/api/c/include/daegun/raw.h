/* Font data at the byte level, for tools that read or write tables daegun has no call for: bounds-checked
 * reads and writes, the structures several tables share, loca and glyf, and building a font from tables.
 * Rust's raw module. */

#ifndef DAEGUN_RAW_H
#define DAEGUN_RAW_H

#include <daegun/base.h>
#include <daegun/font.h>
#include <daegun/outline.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* building fonts */

/** A font file of the map's tables, in tag order, each padded to four bytes and checksummed, with head's
 * checkSumAdjustment set. An empty map, which makes no font, builds no bytes. DAEGUN_INVALID_ARGUMENT for what a
 * font's directory can't state: more than 65,535 tables, or a table of 4 GiB or more, or one starting 4 GiB or more
 * into the file. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_table_map_build(const daegun_table_map *map, daegun_blob *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** The offsets loca holds into glyf, one a glyph and one for the end, into your buffer (BUFFERS); `format` is
 * head's indexToLocFormat, 0 short and 1 long. Past 65,535 glyphs only as many as loca holds, and an entry
 * past its end reads 0; room for num_glyphs + 1 always suffices. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_WRITES(5, 6) daegun_status DAEGUN_CALL
daegun_parse_loca(DAEGUN_IN(loca_len) const uint8_t *DAEGUN_NULLABLE loca, size_t loca_len, int16_t format,
                  size_t num_glyphs, DAEGUN_OUT(out_len) size_t *DAEGUN_NULLABLE out, size_t out_len,
                  size_t *out_count);
/** Draws a glyph out of glyf bytes and the offsets daegun_parse_loca reads, composites resolved; `extended`
 * where the bytes are GLYF's, whose composites can name a glyph in 24 bits. A failure is the decoder's. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_READS(4, 5) daegun_status DAEGUN_CALL
daegun_outline_glyf_bytes(DAEGUN_IN(glyf_len) const uint8_t *DAEGUN_NULLABLE glyf, size_t glyf_len, bool extended,
                          DAEGUN_IN(loca_len) const size_t *DAEGUN_NULLABLE loca, size_t loca_len, uint32_t gid,
                          const daegun_pen *pen);

/* reads and writes */

/** Big-endian reads at `off`. DAEGUN_INVALID_ARGUMENT where they run past the end. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_read_u16_be(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, uint16_t *out);
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_read_i16_be(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, int16_t *out);
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_read_u24_be(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, uint32_t *out);
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_read_u32_be(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, uint32_t *out);
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_read_offset24(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, size_t *out);

/** Big-endian writes at `off`. DAEGUN_INVALID_ARGUMENT, writing nothing, where they run past the end, or for
 * an offset past 24 bits. */
DAEGUN_API DAEGUN_WRITES(1, 2) daegun_status DAEGUN_CALL
daegun_write_u16_be(DAEGUN_OUT(len) uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, uint16_t value);
DAEGUN_API DAEGUN_WRITES(1, 2) daegun_status DAEGUN_CALL
daegun_write_i16_be(DAEGUN_OUT(len) uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, int16_t value);
DAEGUN_API DAEGUN_WRITES(1, 2) daegun_status DAEGUN_CALL
daegun_write_u32_be(DAEGUN_OUT(len) uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, uint32_t value);
DAEGUN_API DAEGUN_WRITES(1, 2) daegun_status DAEGUN_CALL
daegun_write_offset24(DAEGUN_OUT(len) uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off, size_t value);

/** Whether `count` records of `stride` bytes from `start` fit within `len`, overflow included. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST bool DAEGUN_CALL
daegun_records_fit(size_t start, size_t count, size_t stride, size_t len);
/** `n` bytes at `off`, BORROWED from `data`, or NULL where they don't fit: one bounds check, and a pointer
 * good for n bytes. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST const uint8_t *DAEGUN_NULLABLE DAEGUN_CALL
daegun_bytes_window(const uint8_t *data, size_t len, size_t off, size_t n);

/** A binary search of `count` records sorted by key, `key_at` reading record `index`'s key into *out_key and
 * answering true, or false where it doesn't read. *out_found says whether `target` was there, and
 * *out_index where, or where it would go. It stops at the first midpoint that matches, which decides the
 * record where keys repeat. DAEGUN_ABSENT where a key doesn't read. `key_at` must return normally, as
 * CALLBACKS in base.h says. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_search_records(size_t count, uint32_t target,
                      bool (DAEGUN_CALLBACK *key_at)(void *DAEGUN_NULLABLE user, size_t index, uint32_t *out_key),
                      void *DAEGUN_NULLABLE user, size_t *out_index, bool *out_found);

/** The rounding the OpenType spec is written in: floor(value + 0.5). */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST int32_t DAEGUN_CALL
daegun_ot_round(double value);

/* Coverage */

/** A glyph's index in a Coverage table. DAEGUN_ABSENT where it isn't covered; a range's index can pass
 * 65,535. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_coverage_index(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, uint32_t glyph, uint32_t *out);
/** Every glyph the Coverage table at `off` covers, in table order, into your buffer (BUFFERS).
 * DAEGUN_MALFORMED where it doesn't parse, and DAEGUN_UNSUPPORTED for a format other than 1 to 4. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) DAEGUN_WRITES(4, 5) daegun_status DAEGUN_CALL
daegun_coverage_glyphs(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t off,
                       DAEGUN_OUT(out_len) uint32_t *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

/* AAT lookups, state tables and ankr */

/* Each handle here keeps a copy of the bytes it was given, so they are yours again once the call returns. A
 * table that doesn't parse answers DAEGUN_MALFORMED where it is cut short or points past its end,
 * DAEGUN_UNSUPPORTED for a lookup format other than 0, 2, 4, 6, 8 and 10, and DAEGUN_INVALID_ARGUMENT for
 * more extra words than a state table's entry can hold. */

/** An AAT lookup table, of any of its six formats. */
typedef struct daegun_aat_lookup daegun_aat_lookup;

DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_aat_lookup_parse(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, uint32_t num_glyphs,
                        daegun_aat_lookup *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** DAEGUN_ABSENT where the lookup maps nothing to the glyph. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_aat_lookup_value(const daegun_aat_lookup *lookup, uint32_t glyph, uint16_t *out);
/** Every glyph it maps, with its value, into your buffer (BUFFERS). */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(2, 3) daegun_status DAEGUN_CALL
daegun_aat_lookup_entries(const daegun_aat_lookup *lookup, DAEGUN_OUT(out_len) daegun_glyph_value *DAEGUN_NULLABLE out,
                          size_t out_len, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_aat_lookup_free(daegun_aat_lookup *DAEGUN_NULLABLE lookup);

/* The classes and the state every AAT state table predefines: the class a machine is handed once past the
 * last glyph, the class of glyph 0xFFFF, the mark a deleted glyph leaves, and the state it starts in. */
#define DAEGUN_AAT_CLASS_END_OF_TEXT    0
#define DAEGUN_AAT_CLASS_DELETED_GLYPH  2
#define DAEGUN_AAT_STATE_START_OF_TEXT  0

/** One entry of an extended state table: the state to go to, its flags, which each table defines, and its
 * extra words, 0 for a table with fewer. */
typedef struct {
    uint16_t new_state;
    uint16_t flags;
    uint16_t word1;
    uint16_t word2;
} daegun_aat_entry;

/** An extended AAT state table, as morx's subtables hold them. */
typedef struct daegun_aat_state_table daegun_aat_state_table;

/** `extra_words` is how many words each entry carries past its state and flags: none for rearrangement, one
 * for ligature, two for contextual substitution. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_aat_state_table_parse(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t extra_words,
                             uint32_t num_glyphs, daegun_aat_state_table *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** A glyph's class, the table's out-of-bounds class for one it doesn't class. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_aat_state_table_class(const daegun_aat_state_table *table, uint32_t glyph, uint16_t *out);
/** DAEGUN_ABSENT where the table has no such cell. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_aat_state_table_entry(const daegun_aat_state_table *table, uint16_t state, uint16_t class_id,
                             daegun_aat_entry *out);
DAEGUN_API void DAEGUN_CALL
daegun_aat_state_table_free(daegun_aat_state_table *DAEGUN_NULLABLE table);

/** The ankr table: the anchor points a morx machine attaches marks to. */
typedef struct daegun_ankr daegun_ankr;

/** Its version. DAEGUN_INVALID_ARGUMENT for data under two bytes. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_ankr_version(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, uint16_t *out);
/** One control point at `at`, read with no ankr around it. DAEGUN_INVALID_ARGUMENT past the end. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_ankr_control_point(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t at, int16_t *out_x,
                          int16_t *out_y);
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_ankr_parse(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, uint32_t num_glyphs,
                  daegun_ankr *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ankr_point_count(const daegun_ankr *ankr, uint32_t glyph, uint32_t *out);
/** DAEGUN_ABSENT where the glyph has no anchor at that index. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ankr_anchor_point(const daegun_ankr *ankr, uint32_t glyph, uint16_t index, int16_t *out_x, int16_t *out_y);
DAEGUN_API void DAEGUN_CALL
daegun_ankr_free(daegun_ankr *DAEGUN_NULLABLE ankr);

/* item variation stores */

/** An ItemVariationStore, from a GDEF, HVAR, MVAR or VVAR table. */
typedef struct daegun_ivs daegun_ivs;

/** One axis of a region, in normalized coordinates: its influence rises from `start` to 1 at `peak` and falls
 * back to 0 at `end`. */
typedef struct {
    double start;
    double peak;
    double end;
} daegun_region_axis;

/** The store at `base`. A failure says why it doesn't parse. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_ivs_parse(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t base,
                 daegun_ivs *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ivs_axis_count(const daegun_ivs *ivs, size_t *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ivs_region_count(const daegun_ivs *ivs, size_t *out);
/** One region's axes, BORROWED until the store is freed; NULL past the last region. */
DAEGUN_API const daegun_region_axis *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_ivs_region(const daegun_ivs *ivs, size_t region, size_t *out_count);
/** How many ItemVariationData subtables it holds, and how many rows one holds. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ivs_ivd_count(const daegun_ivs *ivs, size_t *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ivs_ivd_rows(const daegun_ivs *ivs, size_t ivd, size_t *out);
/** The regions a subtable names, and one row's deltas, a delta a region named, BORROWED until the store is
 * freed; NULL past the last subtable or row. */
DAEGUN_API const size_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_ivs_ivd_region_indices(const daegun_ivs *ivs, size_t ivd, size_t *out_count);
DAEGUN_API const int32_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_ivs_ivd_row(const daegun_ivs *ivs, size_t ivd, size_t row, size_t *out_count);
/** Each region's scalar at a location, one normalized coordinate an axis, an axis past them at 0, into `out`,
 * room for `out_len`; the region count always suffices. Computed once for a location, they serve every delta
 * read there. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) DAEGUN_WRITES(4, 5) daegun_status DAEGUN_CALL
daegun_ivs_region_scalars(const daegun_ivs *ivs, DAEGUN_IN(location_len) const double *DAEGUN_NULLABLE location,
                          size_t location_len, DAEGUN_OUT(out_len) double *DAEGUN_NULLABLE out, size_t out_len,
                          size_t *out_count);
/** An item's delta: its row's deltas weighted by those scalars and summed; 0 for an index past the store's. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(4, 5) daegun_status DAEGUN_CALL
daegun_ivs_delta(const daegun_ivs *ivs, size_t outer, size_t inner,
                 DAEGUN_IN(scalars_len) const double *DAEGUN_NULLABLE scalars, size_t scalars_len, double *out);
DAEGUN_API void DAEGUN_CALL
daegun_ivs_free(daegun_ivs *DAEGUN_NULLABLE ivs);

/** A DeltaSetIndexMap: the indirection from an item to a store's outer and inner indices. */
typedef struct daegun_delta_set_index_map daegun_delta_set_index_map;

/** The map at `base`. A failure says why it doesn't parse. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_delta_set_index_map_parse(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE data, size_t len, size_t base,
                                 daegun_delta_set_index_map *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_delta_set_index_map_count(const daegun_delta_set_index_map *map, size_t *out);
/** An item's outer and inner indices: an index past the map's takes its last entry, and an empty map maps
 * item i to (0, i). */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_delta_set_index_map_lookup(const daegun_delta_set_index_map *map, size_t index, size_t *out_outer,
                                  size_t *out_inner);
DAEGUN_API void DAEGUN_CALL
daegun_delta_set_index_map_free(daegun_delta_set_index_map *DAEGUN_NULLABLE map);

/* FeatureVariations */

/** The FeatureVariations a GSUB or GPOS table may point at: which features change where in the design
 * space. Each handle keeps a copy of the layout table it was given. */
typedef struct daegun_feature_variations daegun_feature_variations;

/** The FeatureVariations a GSUB or GPOS of version 1.1 or later names. DAEGUN_ABSENT for an earlier version
 * or a null offset. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_feature_variations_parse(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE layout, size_t len,
                                daegun_feature_variations *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The FeatureVariations at an offset you know, its reads bounds-checked as they go. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_feature_variations_at(DAEGUN_IN(len) const uint8_t *DAEGUN_NULLABLE layout, size_t len, size_t at,
                             daegun_feature_variations *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The first variation record whose conditions hold at `coords`, one F2DOT14 an axis, an axis past them at
 * 0. A value condition adds `store`'s delta, GDEF's, and nothing for a NULL store. DAEGUN_ABSENT where none
 * holds. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_feature_variations_find(const daegun_feature_variations *variations,
                               DAEGUN_IN(coords_len) const int32_t *DAEGUN_NULLABLE coords, size_t coords_len,
                               const daegun_ivs *DAEGUN_NULLABLE store, uint32_t *out);
/** Where in the layout table the feature that record `variation` substitutes for feature index `feature`
 * lies, 0 for the empty feature a null offset names. DAEGUN_ABSENT where it substitutes none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_feature_variations_substitute(const daegun_feature_variations *variations, uint32_t variation,
                                     uint16_t feature, size_t *out);
DAEGUN_API void DAEGUN_CALL
daegun_feature_variations_free(daegun_feature_variations *DAEGUN_NULLABLE variations);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_aat_entry) == 8, "an entry is four words");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_region_axis) == 24, "a region axis is three doubles");

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_RAW_H */
