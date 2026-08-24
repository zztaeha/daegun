/* daegun – a no_std, zero-dependency OpenType engine, from C.
 *
 * Everything the Rust API can do, C can do. This header is the contract.
 *
 * The C ABI is behind daegun's `capi` feature. Ask for the shape you want:
 *
 *     cargo rustc --release --features capi --crate-type cdylib     # libdaegun.so / .dylib / .dll
 *     cargo rustc --release --features capi --crate-type staticlib  # libdaegun.a / daegun.lib
 *
 * `capi` implies `threading`, which is what makes rule 5 below true.
 *
 * The static library needs the platform frameworks the GPU backends call into; the shared library
 * carries its own:
 *
 *     macOS / iOS   cc app.c libdaegun_c.a -framework Metal -framework Foundation \
 *                              -framework QuartzCore
 *     Linux         cc app.c libdaegun_c.a -lm -lpthread -ldl
 *     Windows       cl app.c daegun_c.lib ws2_32.lib userenv.lib ntdll.lib
 *
 * Vulkan and Direct3D are opened by name at run time, so neither adds anything here – a machine
 * without them answers DAEGUN_UNSUPPORTED instead of failing to load.
 *
 * THE FIVE RULES. Every function here obeys all of them, so there is nothing to remember per call.
 *
 *   1. A fallible call returns daegun_status. Results come back through out-parameters.
 *   2. Passing NULL where a pointer is required returns DAEGUN_NULL. It is never dereferenced.
*   3. daegun allocates and daegun frees. Never call free() on a pointer daegun gave you: this
 *      library has its own allocator, so that is undefined behavior rather than a matter of style.
 *   4. A borrowed view is a const pointer plus a count, valid until the handle it came from is
 *      freed. Copy it if you need it longer.
*   5. Handles are thread-safe: one handle may be used from several threads at once. Correct but
 *      contended, so a thread-per-font arrangement will beat a shared one under load.
 *
 * PANICS. The public path contains no unwrap, expect or panic!, and a gate step keeps it that way.
 * The library is built with panic = "abort": were one to occur it would end the process rather than
 * unwind into your stack frame, which across this boundary would be undefined behavior.
 */

#ifndef DAEGUN_H
#define DAEGUN_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* status */

/* What every fallible call returns.
 *
 * The values are frozen. A caller compares against these constants and a compiled consumer carries
 * the numbers, not the names, so renumbering would break every binary silently. New codes append. */
typedef int32_t daegun_status;

#define DAEGUN_OK           0   /* the call did what it says */
#define DAEGUN_NULL        -1   /* a required pointer was NULL – always the caller's bug */
#define DAEGUN_PARSE       -2   /* the font data did not parse; see daegun_last_error() */
#define DAEGUN_RANGE       -3   /* an argument was outside what the call accepts */
#define DAEGUN_ABSENT      -4   /* the font has no such glyph, table or axis – an answer, not a
                                   failure. Separate from DAEGUN_OK because C has no Option and
                                   zero is a real glyph id (.notdef). */
#define DAEGUN_UNSUPPORTED -5   /* the platform declined: no GPU, no driver, no such mode */

/* borrowed views */

/* A run of bytes daegun owns, lent to you. Valid until the handle it came from is freed. */
typedef struct {
    const uint8_t *data;
    size_t         len;
} daegun_bytes;

/* A UTF-8 string daegun owns, lent to you.
 *
 * NUL-terminated and length-carrying both. An interior NUL – which a font's name table may contain –
 * is replaced with U+FFFD, so strlen cannot mislead you and `len` still describes the whole string. */
typedef struct {
    const char *data;
    size_t      len;
} daegun_str;

/* handles */

/* Opaque. You never see the layout, so it is free to change without breaking you. */
typedef struct daegun_font daegun_font;

/* library */

/* The ABI this library was built with, as (major << 16) | (minor << 8) | patch.
 *
 * Refuse a library whose major does not match what you compiled against. A struct that grew a field
 * is not detectable any other way. */
uint32_t daegun_abi_version(void);

#define DAEGUN_ABI_VERSION ((1u << 16) | (1u << 8) | 7u)

/* What the last failing call ON THIS THREAD said, as UTF-8. Empty when nothing has failed.
 *
 * Per-thread, like errno, because handles are shared and a global slot would hand one thread
 * another's failure. Valid until the next failing call on this thread. */
daegun_str daegun_last_error(void);

/* font */

/* Opens a font. The bytes are COPIED, so you may free them the moment this returns. */
daegun_status daegun_font_open(const uint8_t *data, size_t len, daegun_font **out);

/* A buffer for a font, allocated by daegun so it can be handed back without a copy.
 *
 * daegun_font_open copies your bytes so you may free them the moment it returns. This is the way
 * around that copy – take a buffer, read the file into it, hand it back:
 *
 *     uint8_t *buf = daegun_font_buffer_new(len);
 *     fread(buf, 1, len, fp);
 *     daegun_font *font = NULL;
 *     daegun_status st = daegun_font_open_owned(buf, len, &font);
 *     // buf belongs to the font now, pass or fail. Do not free it, do not read it.
 *
 * NULL for a zero length. This is not an ordinary pointer: never free() it. */
uint8_t *daegun_font_buffer_new(size_t len);

/* Frees a buffer you took but never handed over. Null is a no-op. Do NOT call this after
 * daegun_font_open_owned – that took ownership, whether it succeeded or not. */
void daegun_font_buffer_free(uint8_t *data, size_t len);

/* Opens a font from a daegun buffer, taking ownership of it. Copies nothing.
 *
 * The buffer belongs to the font afterwards either way: freed with daegun_font_free, or immediately
 * if the bytes did not parse. `len` must be exactly what you passed to daegun_font_buffer_new. */
daegun_status daegun_font_open_owned(uint8_t *data, size_t len, daegun_font **out);

/* The same, for one face of a .ttc collection. */
daegun_status daegun_font_open_collection(const uint8_t *data, size_t len, size_t index,
                                          daegun_font **out);

/* Frees a font. NULL is a no-op, as free(NULL) is. */
void daegun_font_free(daegun_font *font);

/* The glyph a Unicode codepoint maps to. DAEGUN_ABSENT when the font has none. */
daegun_status daegun_font_glyph_id(const daegun_font *font, uint32_t codepoint, uint16_t *out);

/* How many glyphs the face has. */
daegun_status daegun_font_num_glyphs(const daegun_font *font, uint16_t *out);

/* Units per em: the denominator every em-space figure in this API is in. */
daegun_status daegun_font_upm(const daegun_font *font, uint16_t *out);

/* How many faces a .ttc holds. Zero for data that is not a collection. Takes bytes rather than a
 * font, because it answers what a caller asks before deciding which face to open. */
daegun_status daegun_ttc_font_count(const uint8_t *data, size_t len, size_t *out);

/* the cache */

/* Resizes the rasterized-glyph cache, in bytes. Zero turns caching off.
 *
 * Bounded by bytes rather than entries because one glyph at 4096px outweighs thousands at 12px, so
 * a count would let a single large render blow the budget. */
daegun_status daegun_font_set_glyph_cache_bytes(const daegun_font *font, size_t bytes);

/* Drops every cached glyph, keeping the byte bound. */
daegun_status daegun_font_clear_glyph_cache(const daegun_font *font);

/* How many glyphs the cache holds, and how many bytes that is. Either pointer may be NULL. */
daegun_status daegun_font_glyph_cache_stats(const daegun_font *font, size_t *out_count,
                                            size_t *out_bytes);

/* The remaining caches, each a ceiling grown into rather than memory reserved up front. Every one
 * of them is dead weight for some caller: a CPU-only program never fills the curve cache, a
 * Latin-only one never approaches the cmap index bound, a fixed-weight one never instances a
 * variable font. Defaults are 4 MB curve, 4 MB outline, 8 MB shape, 64 MB instance, 25 MB index. */

/* Built glyph curves for the GPU backends. Zero on a program that only rasterizes. */
daegun_status daegun_font_set_curve_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_clear_curve_cache(const daegun_font *font);
daegun_status daegun_font_curve_cache_stats(const daegun_font *font, size_t *out_count,
                                            size_t *out_bytes);

/* Decoded outlines, shared by the raster and stroke paths. Emptied by daegun_font_clear_prewarm,
 * which is the call that fills it. */
daegun_status daegun_font_set_outline_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_outline_cache_stats(const daegun_font *font, size_t *out_count,
                                              size_t *out_bytes);

/* Shaped runs, keyed by text. Worth lowering where the text never repeats, as in a log or a chat
 * transcript, since nothing cached there is ever read back. */
daegun_status daegun_font_set_shape_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_clear_shape_cache(const daegun_font *font);
daegun_status daegun_font_shape_cache_stats(const daegun_font *font, size_t *out_count,
                                            size_t *out_bytes);

/* Instanced variable fonts, the largest default by a wide margin. Only an animated axis fills it,
 * so a program that renders one fixed weight can set it close to zero. Reports the bytes held for
 * axis locations and for the instanced tables separately; either pointer may be NULL. */
daegun_status daegun_font_set_instance_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_instance_cache_stats(const daegun_font *font, size_t *out_locations,
                                               size_t *out_tables);

/* Unlike the others this is what is left to spend rather than a ceiling: building a cmap index
 * draws it down and never returns it, so setting it grants a fresh allowance. Sized for CJK. */
daegun_status daegun_font_set_cmap_index_allowance(const daegun_font *font, size_t bytes);
daegun_status daegun_font_cmap_index_allowance(const daegun_font *font, size_t *out_bytes);

/* options */

/* Where the display puts its color samples. Anything unrecognized is grayscale, so a caller on a
 * newer header than the library gets plain antialiasing rather than a failed render. */
#define DAEGUN_LAYOUT_GRAYSCALE          0
#define DAEGUN_LAYOUT_RGB_H              1
#define DAEGUN_LAYOUT_BGR_H              2
#define DAEGUN_LAYOUT_RGB_V              3
#define DAEGUN_LAYOUT_BGR_V              4
#define DAEGUN_LAYOUT_RGB_H_UNFILTERED   5
#define DAEGUN_LAYOUT_BGR_H_UNFILTERED   6
#define DAEGUN_LAYOUT_RGB_V_UNFILTERED   7
#define DAEGUN_LAYOUT_BGR_V_UNFILTERED   8

/* Whether to run the glyph's own TrueType bytecode, and under which interpretation. */
#define DAEGUN_HINT_NONE        0
#define DAEGUN_HINT_SUBPIXEL    1
#define DAEGUN_HINT_CLASSIC     2
#define DAEGUN_HINT_AUTO        3
#define DAEGUN_HINT_AUTO_FORCE  4

#define DAEGUN_JOIN_MITER  0
#define DAEGUN_JOIN_ROUND  1
#define DAEGUN_JOIN_BEVEL  2

#define DAEGUN_CAP_BUTT    0
#define DAEGUN_CAP_ROUND   1
#define DAEGUN_CAP_SQUARE  2

/* What you fill in and hand to anything that rasterizes.
 *
 * The has_ flags are how C says Option. Zeroing the struct gives the Rust default – grayscale, no
 * gamma, transform, hinting, stroke or synthesis – so memset(&o, 0, sizeof o) is a correct start;
 * daegun_raster_options_default() keeps working if a default ever stops being zero. */
typedef struct {
    int32_t layout;              /* DAEGUN_LAYOUT_* */
    int32_t hinting;             /* DAEGUN_HINT_* */
    int32_t has_gamma;
    float   gamma;               /* coverage^(1/gamma), applied as coverage becomes a byte */
    int32_t has_transform;
    float   transform[6];        /* [a, b, c, d, dx, dy] in font units, before rasterizing */
    int32_t has_stroke;
    float   stroke_width;        /* font units, so it scales with the size as the glyph does */
    int32_t stroke_join;         /* DAEGUN_JOIN_* */
    float   stroke_miter_limit;  /* read only when stroke_join is DAEGUN_JOIN_MITER */
    int32_t stroke_cap;          /* DAEGUN_CAP_* */
    int32_t has_embolden;
    float   embolden;            /* extra stem width in font units. THIS CHANGES THE ADVANCE. */
    int32_t has_oblique;
    float   oblique;             /* tangent of the angle from vertical; positive leans right */
} daegun_raster_options;

daegun_status daegun_raster_options_default(daegun_raster_options *out);

/* owned results */

/* Rule 3: daegun allocates and daegun frees. Anything below that hands back a collection hands back
 * one of these, and you return it with the matching _free. A borrowed view read out of one is valid
 * until that free. */
typedef struct daegun_u16_list daegun_u16_list;
typedef struct daegun_i32_list daegun_i32_list;
typedef struct daegun_f64_list daegun_f64_list;
typedef struct daegun_blob     daegun_blob;      /* a run of bytes: a font file, a bitmap */
typedef struct daegun_str_list daegun_str_list;
typedef struct daegun_text     daegun_text;      /* one owned string */
typedef struct daegun_usize_list daegun_usize_list;
typedef struct daegun_glyph_value_list daegun_glyph_value_list;

/* A glyph paired with what a table maps it to.
 *
 * One list of pairs rather than two parallel lists, which is the opposite of what the optional
 * results below do. The difference: there the halves answer different questions and a caller reads
 * one without the other, here neither half means anything alone. */
typedef struct {
    uint16_t glyph;
    uint16_t value;
} daegun_glyph_value;

/* The elements, borrowed. NULL if the list or the count pointer is NULL. */
const uint16_t *daegun_u16_list_data(const daegun_u16_list *list, size_t *out_count);
const int32_t  *daegun_i32_list_data(const daegun_i32_list *list, size_t *out_count);
const double   *daegun_f64_list_data(const daegun_f64_list *list, size_t *out_count);
const uint8_t  *daegun_blob_data(const daegun_blob *blob, size_t *out_count);
const size_t   *daegun_usize_list_data(const daegun_usize_list *list, size_t *out_count);
const daegun_glyph_value *daegun_glyph_value_list_data(const daegun_glyph_value_list *list,
                                                       size_t *out_count);

void daegun_u16_list_free(daegun_u16_list *list);
void daegun_i32_list_free(daegun_i32_list *list);
void daegun_f64_list_free(daegun_f64_list *list);
void daegun_blob_free(daegun_blob *blob);
void daegun_usize_list_free(daegun_usize_list *list);
void daegun_glyph_value_list_free(daegun_glyph_value_list *list);

daegun_status daegun_str_list_count(const daegun_str_list *list, size_t *out);
/* DAEGUN_RANGE past the end, rather than an empty string – a font may hold an empty name and you
 * must be able to tell the two apart. */
daegun_status daegun_str_list_at(const daegun_str_list *list, size_t index, daegun_str *out);
void daegun_str_list_free(daegun_str_list *list);

daegun_status daegun_text_str(const daegun_text *text, daegun_str *out);
void daegun_text_free(daegun_text *text);

/* One axis of a variable font, as you state it. The input six calls share.
 *
 * `tag` is a NUL-terminated four-character tag such as "wght". A tag that is not valid UTF-8 is
 * skipped rather than failing the call, because a location is a request and an axis the font does
 * not have is ignored anyway. */
typedef struct {
    const char *tag;
    double      value;
} daegun_axis;

/* what it says */

daegun_status daegun_font_is_variable(const daegun_font *font, bool *out);
daegun_status daegun_font_ascender(const daegun_font *font, int32_t *out);
daegun_status daegun_font_descender(const daegun_font *font, int32_t *out);
daegun_status daegun_font_cap_height(const daegun_font *font, int32_t *out);
daegun_status daegun_font_flags(const daegun_font *font, uint32_t *out);
daegun_status daegun_font_italic_angle(const daegun_font *font, double *out);

/* DAEGUN_ABSENT where the face states none. */
daegun_status daegun_font_family_name(const daegun_font *font, daegun_text **out);
daegun_status daegun_font_style(const daegun_font *font, daegun_text **out);
daegun_status daegun_font_name_string(const daegun_font *font, uint16_t name_id, daegun_text **out);

/* Every name the `name` table holds. Two lists rather than a map, because C has no map: the id at
 * index i in out_ids belongs to the string at index i in out_strings. Either may be NULL. */
daegun_status daegun_font_names(const daegun_font *font, daegun_u16_list **out_ids,
                                daegun_str_list **out_strings);

/* [xmin, ymin, xmax, ymax], in font units. */
daegun_status daegun_font_bbox(const daegun_font *font, daegun_i32_list **out);

/* Tracking at a point size, in font units. */
daegun_status daegun_font_tracking(const daegun_font *font, double ptem, bool horizontal,
                                   double *out);

typedef struct {
    double ascent;
    double descent;
    double line_gap;
} daegun_line_metrics;

daegun_status daegun_font_line_metrics(const daegun_font *font, bool vertical,
                                       daegun_line_metrics *out);

/* What OS/2 says. The has_ fields are Option again: a version below 4 carries no typographic
 * metrics, and a face may state no family class at all. */
typedef struct {
    uint16_t version;
    uint16_t has_family_class;
    uint16_t family_class;
    uint16_t has_selection;
    uint16_t selection;
    uint16_t has_win_metrics;
    int32_t  win_ascent;
    int32_t  win_descent;
    uint16_t has_typo_metrics;
    int32_t  typo_ascender;
    int32_t  typo_descender;
    int32_t  typo_line_gap;
} daegun_os2_info;

daegun_status daegun_font_os2_info(const daegun_font *font, daegun_os2_info *out);

/* The five selection predicates. All five are DAEGUN_ABSENT where the face states no OS/2 table. */
daegun_status daegun_font_is_italic(const daegun_font *font, bool *out);
daegun_status daegun_font_is_bold(const daegun_font *font, bool *out);
daegun_status daegun_font_is_regular(const daegun_font *font, bool *out);
daegun_status daegun_font_is_oblique(const daegun_font *font, bool *out);
daegun_status daegun_font_uses_typo_metrics(const daegun_font *font, bool *out);

typedef struct {
    int32_t x_height;
    int32_t underline_position;
    int32_t underline_thickness;
    int32_t strikeout_size;
    int32_t strikeout_position;
    int32_t subscript_x_size;
    int32_t subscript_y_size;
    int32_t subscript_x_offset;
    int32_t subscript_y_offset;
    int32_t superscript_x_size;
    int32_t superscript_y_size;
    int32_t superscript_x_offset;
    int32_t superscript_y_offset;
} daegun_typographic_metrics;

daegun_status daegun_font_typographic_metrics(const daegun_font *font, const daegun_axis *axes,
                                              size_t axes_len, daegun_typographic_metrics *out);

/* variations */

/* The declared axes. Three things per axis, so two lists: out_tags gets the tags, out_ranges gets
 * [min, default, max] per axis – axis i is at 3 * i. Either may be NULL. */
daegun_status daegun_font_axes(const daegun_font *font, daegun_str_list **out_tags,
                               daegun_f64_list **out_ranges);

/* One -1..=1 coordinate per fvar axis, in the font's own axis order. */
daegun_status daegun_font_normalized_axes(const daegun_font *font, const daegun_axis *axes,
                                          size_t axes_len, daegun_f64_list **out);

/* The face instanced at a location, as a complete font file. */
daegun_status daegun_font_instance(const daegun_font *font, const daegun_axis *axes,
                                   size_t axes_len, daegun_blob **out);

daegun_status daegun_font_named_instance_count(const daegun_font *font, size_t *out);

/* One named instance. Any out-parameter may be NULL. An instance that states no name writes a NULL
 * handle rather than failing, so one absent name does not hide the coordinates you asked for in the
 * same call. DAEGUN_RANGE past the end. */
daegun_status daegun_font_named_instance(const daegun_font *font, size_t index,
                                         daegun_text **out_name, daegun_text **out_postscript_name,
                                         daegun_str_list **out_coord_tags,
                                         daegun_f64_list **out_coord_values);

/* subsetting */

typedef struct daegun_subset daegun_subset;

const uint8_t  *daegun_subset_ttf(const daegun_subset *subset, size_t *out_len);
const uint16_t *daegun_subset_gid_map(const daegun_subset *subset, size_t *out_len);
void daegun_subset_free(daegun_subset *subset);

daegun_status daegun_font_subset(const daegun_font *font, const uint16_t *gids, size_t gids_len,
                                 const daegun_axis *axes, size_t axes_len, daegun_subset **out);
daegun_status daegun_font_subset_text(const daegun_font *font, const char *text,
                                      const daegun_axis *axes, size_t axes_len,
                                      daegun_subset **out);
daegun_status daegun_font_glyph_closure(const daegun_font *font, const uint16_t *gids,
                                        size_t gids_len, const daegun_axis *axes, size_t axes_len,
                                        daegun_u16_list **out);

/* MATH */

/* One MATH constant by index. The indices are generated from the Rust struct's field order, and
 * appending a constant stays backward compatible. */
#define DAEGUN_MATH_SCRIPT_PERCENT_SCALE_DOWN                    0
#define DAEGUN_MATH_SCRIPT_SCRIPT_PERCENT_SCALE_DOWN             1
#define DAEGUN_MATH_DELIMITED_SUB_FORMULA_MIN_HEIGHT             2
#define DAEGUN_MATH_DISPLAY_OPERATOR_MIN_HEIGHT                  3
#define DAEGUN_MATH_MATH_LEADING                                 4
#define DAEGUN_MATH_AXIS_HEIGHT                                  5
#define DAEGUN_MATH_ACCENT_BASE_HEIGHT                           6
#define DAEGUN_MATH_FLATTENED_ACCENT_BASE_HEIGHT                 7
#define DAEGUN_MATH_SUBSCRIPT_SHIFT_DOWN                         8
#define DAEGUN_MATH_SUBSCRIPT_TOP_MAX                            9
#define DAEGUN_MATH_SUBSCRIPT_BASELINE_DROP_MIN                  10
#define DAEGUN_MATH_SUPERSCRIPT_SHIFT_UP                         11
#define DAEGUN_MATH_SUPERSCRIPT_SHIFT_UP_CRAMPED                 12
#define DAEGUN_MATH_SUPERSCRIPT_BOTTOM_MIN                       13
#define DAEGUN_MATH_SUPERSCRIPT_BASELINE_DROP_MAX                14
#define DAEGUN_MATH_SUB_SUPERSCRIPT_GAP_MIN                      15
#define DAEGUN_MATH_SUPERSCRIPT_BOTTOM_MAX_WITH_SUBSCRIPT        16
#define DAEGUN_MATH_SPACE_AFTER_SCRIPT                           17
#define DAEGUN_MATH_UPPER_LIMIT_GAP_MIN                          18
#define DAEGUN_MATH_UPPER_LIMIT_BASELINE_RISE_MIN                19
#define DAEGUN_MATH_LOWER_LIMIT_GAP_MIN                          20
#define DAEGUN_MATH_LOWER_LIMIT_BASELINE_DROP_MIN                21
#define DAEGUN_MATH_STACK_TOP_SHIFT_UP                           22
#define DAEGUN_MATH_STACK_TOP_DISPLAY_STYLE_SHIFT_UP             23
#define DAEGUN_MATH_STACK_BOTTOM_SHIFT_DOWN                      24
#define DAEGUN_MATH_STACK_BOTTOM_DISPLAY_STYLE_SHIFT_DOWN        25
#define DAEGUN_MATH_STACK_GAP_MIN                                26
#define DAEGUN_MATH_STACK_DISPLAY_STYLE_GAP_MIN                  27
#define DAEGUN_MATH_STRETCH_STACK_TOP_SHIFT_UP                   28
#define DAEGUN_MATH_STRETCH_STACK_BOTTOM_SHIFT_DOWN              29
#define DAEGUN_MATH_STRETCH_STACK_GAP_ABOVE_MIN                  30
#define DAEGUN_MATH_STRETCH_STACK_GAP_BELOW_MIN                  31
#define DAEGUN_MATH_FRACTION_NUMERATOR_SHIFT_UP                  32
#define DAEGUN_MATH_FRACTION_NUMERATOR_DISPLAY_STYLE_SHIFT_UP    33
#define DAEGUN_MATH_FRACTION_DENOMINATOR_SHIFT_DOWN              34
#define DAEGUN_MATH_FRACTION_DENOMINATOR_DISPLAY_STYLE_SHIFT_DOWN 35
#define DAEGUN_MATH_FRACTION_NUMERATOR_GAP_MIN                   36
#define DAEGUN_MATH_FRACTION_NUM_DISPLAY_STYLE_GAP_MIN           37
#define DAEGUN_MATH_FRACTION_RULE_THICKNESS                      38
#define DAEGUN_MATH_FRACTION_DENOMINATOR_GAP_MIN                 39
#define DAEGUN_MATH_FRACTION_DENOM_DISPLAY_STYLE_GAP_MIN         40
#define DAEGUN_MATH_SKEWED_FRACTION_HORIZONTAL_GAP               41
#define DAEGUN_MATH_SKEWED_FRACTION_VERTICAL_GAP                 42
#define DAEGUN_MATH_OVERBAR_VERTICAL_GAP                         43
#define DAEGUN_MATH_OVERBAR_RULE_THICKNESS                       44
#define DAEGUN_MATH_OVERBAR_EXTRA_ASCENDER                       45
#define DAEGUN_MATH_UNDERBAR_VERTICAL_GAP                        46
#define DAEGUN_MATH_UNDERBAR_RULE_THICKNESS                      47
#define DAEGUN_MATH_UNDERBAR_EXTRA_DESCENDER                     48
#define DAEGUN_MATH_RADICAL_VERTICAL_GAP                         49
#define DAEGUN_MATH_RADICAL_DISPLAY_STYLE_VERTICAL_GAP           50
#define DAEGUN_MATH_RADICAL_RULE_THICKNESS                       51
#define DAEGUN_MATH_RADICAL_EXTRA_ASCENDER                       52
#define DAEGUN_MATH_RADICAL_KERN_BEFORE_DEGREE                   53
#define DAEGUN_MATH_RADICAL_KERN_AFTER_DEGREE                    54
#define DAEGUN_MATH_RADICAL_DEGREE_BOTTOM_RAISE_PERCENT          55

int32_t daegun_math_constant_count(void);
daegun_status daegun_font_math_constant(const daegun_font *font, int32_t which, double *out);

daegun_status daegun_font_math_italics_correction(const daegun_font *font, uint16_t gid, double *out);
daegun_status daegun_font_math_top_accent_attachment(const daegun_font *font, uint16_t gid,
                                                     double *out);
daegun_status daegun_font_math_is_extended_shape(const daegun_font *font, uint16_t gid, bool *out);
daegun_status daegun_font_math_min_connector_overlap(const daegun_font *font, double *out);

#define DAEGUN_MATH_KERN_TOP_RIGHT     0
#define DAEGUN_MATH_KERN_TOP_LEFT      1
#define DAEGUN_MATH_KERN_BOTTOM_RIGHT  2
#define DAEGUN_MATH_KERN_BOTTOM_LEFT   3

/* An unrecognized corner is DAEGUN_RANGE rather than a default: the four are not interchangeable, so
 * guessing one would be a wrong answer rather than a fallback. */
daegun_status daegun_font_math_kern(const daegun_font *font, uint16_t gid, int32_t corner,
                                    double height, double *out);

typedef struct daegun_math_construction daegun_math_construction;

daegun_status daegun_font_math_glyph_variants(const daegun_font *font, uint16_t gid, bool vertical,
                                              daegun_math_construction **out);

/* The discrete variants: their glyph ids, with advances at matching indices. */
daegun_status daegun_math_construction_variants(const daegun_math_construction *c, size_t *out_count,
                                                const uint16_t **out_gids,
                                                const double **out_advances);

/* The assembly, if there is one. out_part_values holds FOUR doubles per part – start connector, end
 * connector, full advance, and is_extender as 0 or 1 – so part i begins at 4 * i.
 * DAEGUN_ABSENT where the construction carries no assembly. */
daegun_status daegun_math_construction_assembly(const daegun_math_construction *c,
                                                double *out_italics_correction,
                                                size_t *out_part_count,
                                                const uint16_t **out_part_gids,
                                                const double **out_part_values);

void daegun_math_construction_free(daegun_math_construction *c);

/* BASE and STAT */

daegun_status daegun_font_base_is_glyph_free(const daegun_font *font, bool *out);

/* What BASE says for one script. Any out-parameter may be NULL; a script that names no default
 * baseline writes a NULL handle rather than failing. */
daegun_status daegun_font_base_info(const daegun_font *font, const char *script_tag, bool vertical,
                                    daegun_text **out_default_baseline,
                                    daegun_str_list **out_baseline_tags,
                                    daegun_f64_list **out_baseline_coords);

typedef struct daegun_stat daegun_stat;

daegun_status daegun_font_stat_info(const daegun_font *font, daegun_stat **out);
daegun_status daegun_stat_axes(const daegun_stat *stat, size_t *out_count,
                               daegun_str_list **out_tags, const uint16_t **out_orderings);
/* How many axis values STAT names. The values themselves are not exposed yet. */
daegun_status daegun_stat_value_count(const daegun_stat *stat, size_t *out);
daegun_status daegun_stat_elided_fallback_name(const daegun_stat *stat, daegun_text **out);
void daegun_stat_free(daegun_stat *stat);

/* the tag inventory */

daegun_status daegun_font_script_tags(const daegun_font *font, daegun_str_list **out);
daegun_status daegun_font_language_tags(const daegun_font *font, const char *script,
                                        daegun_str_list **out);
/* Either tag may be NULL, which is how C says None – the Rust signature takes Option for both. */
daegun_status daegun_font_feature_tags(const daegun_font *font, const char *script,
                                       const char *language, daegun_str_list **out);
daegun_status daegun_font_justification_glyphs(const daegun_font *font, const char *script_tag,
                                               daegun_u16_list **out);

/* glyphs */

typedef struct daegun_u32_list daegun_u32_list;
const uint32_t *daegun_u32_list_data(const daegun_u32_list *list, size_t *out_count);
void daegun_u32_list_free(daegun_u32_list *list);

daegun_status daegun_font_has_glyph(const daegun_font *font, uint32_t codepoint, bool *out);

/* The glyph each character of a string maps to.
 *
 * TWO lists, because entries may be absent: out_gids holds one id per character, out_present one
 * byte per character, non-zero where the font actually has a glyph. A sentinel would not work – 0 is
 * .notdef and every other uint16_t is a real glyph id in some face. Either may be NULL. */
daegun_status daegun_font_glyph_ids(const daegun_font *font, const char *text,
                                    daegun_u16_list **out_gids, daegun_blob **out_present);

/* Every codepoint the cmap maps, with the glyph each reaches, at matching indices. */
daegun_status daegun_font_coverage(const daegun_font *font, daegun_u32_list **out_codepoints,
                                   daegun_u16_list **out_gids);
daegun_status daegun_font_codepoints(const daegun_font *font, daegun_u32_list **out);

/* A glyph's tight ink box at a location: out receives FOUR doubles, [xmin, ymin, xmax, ymax].
 * DAEGUN_ABSENT for a glyph that draws nothing – a space has no ink, which differs from a box of
 * zero size. */
daegun_status daegun_font_glyph_bounds(const daegun_font *font, uint16_t gid,
                                       const daegun_axis *axes, size_t axes_len, double *out);

daegun_status daegun_font_variation_glyph_id(const daegun_font *font, uint32_t base,
                                             uint32_t selector, uint16_t *out);

daegun_status daegun_font_advance_widths(const daegun_font *font, const uint16_t *gids,
                                         size_t gids_len, const daegun_axis *axes, size_t axes_len,
                                         daegun_f64_list **out);
daegun_status daegun_font_vertical_advance(const daegun_font *font, uint16_t gid,
                                           const daegun_axis *axes, size_t axes_len, uint32_t *out);
daegun_status daegun_font_vertical_origin(const daegun_font *font, uint16_t gid,
                                          const daegun_axis *axes, size_t axes_len, int32_t *out);
daegun_status daegun_font_default_vertical_origin(const daegun_font *font, int32_t *out);

daegun_status daegun_font_ligature_carets(const daegun_font *font, uint16_t gid,
                                          const daegun_axis *axes, size_t axes_len,
                                          daegun_f64_list **out);
daegun_status daegun_font_caret_positions(const daegun_font *font, const char *text,
                                          const daegun_axis *axes, size_t axes_len, bool vertical,
                                          daegun_f64_list **out);

#define DAEGUN_GLYPH_CLASS_BASE       0
#define DAEGUN_GLYPH_CLASS_LIGATURE   1
#define DAEGUN_GLYPH_CLASS_MARK       2
#define DAEGUN_GLYPH_CLASS_COMPONENT  3

daegun_status daegun_font_glyph_class(const daegun_font *font, uint16_t gid, int32_t *out);
daegun_status daegun_font_mark_attachment_class(const daegun_font *font, uint16_t gid, uint16_t *out);
daegun_status daegun_font_glyph_name(const daegun_font *font, uint16_t gid, daegun_text **out);
/* Two lists again: an empty string is a name a font can genuinely state, so out_present says which
 * entries mean anything. */
daegun_status daegun_font_glyph_names(const daegun_font *font, daegun_str_list **out_names,
                                      daegun_blob **out_present);

/* outlines */

/* What you hand daegun to receive an outline. The first thing that crosses INTO the library.
 *
 * `user` is passed back untouched to every callback; daegun never reads it. Any callback may be
 * NULL, and that event is then skipped – so a caller wanting only the on-curve points supplies three
 * of the five.
 *
 * Your callbacks MUST NOT unwind: a C++ exception or a longjmp through Rust frames is undefined
 * behavior and nothing here can prevent it. You MAY call back into daegun from inside them,
 * including on the same font – no lock is held while your callback runs. */
typedef struct {
    void (*move_to)(void *user, float x, float y);
    void (*line_to)(void *user, float x, float y);
    void (*quad_to)(void *user, float cx, float cy, float x, float y);
    void (*curve_to)(void *user, float c1x, float c1y, float c2x, float c2y, float x, float y);
    void (*close)(void *user);
    void *user;
} daegun_pen;

/* The stored outline – the default instance, whatever location you are working at. */
daegun_status daegun_font_outline_glyph(const daegun_font *font, uint16_t gid,
                                        const daegun_pen *pen);
/* The same, resolving variation deltas at a location first. */
daegun_status daegun_font_outline_glyph_instanced(const daegun_font *font, uint16_t gid,
                                                  const daegun_axis *axes, size_t axes_len,
                                                  const daegun_pen *pen);

daegun_status daegun_font_prewarm(const daegun_font *font, const uint16_t *gids, size_t gids_len,
                                  const daegun_axis *axes, size_t axes_len, size_t *out_added);
daegun_status daegun_font_clear_prewarm(const daegun_font *font);

/* rasterizing */

typedef struct daegun_bitmap daegun_bitmap;

/* width and height are the bitmap's, in pixels. xmin and ymin place its bottom-left corner relative
 * to the pen. The bounds_ fields are the em-space box the pixels came from, which a caller
 * compositing at sub-pixel offsets needs and cannot recover from the integers. */
typedef struct {
    int32_t xmin;
    int32_t ymin;
    size_t  width;
    size_t  height;
    float   advance_width;
    float   advance_height;
    float   bounds_xmin;
    float   bounds_ymin;
    float   bounds_width;
    float   bounds_height;
} daegun_metrics;

daegun_status daegun_bitmap_metrics(const daegun_bitmap *bitmap, daegun_metrics *out);
/* The coverage, borrowed. One byte per pixel for grayscale, three for a subpixel layout – the LENGTH
 * is what says which, so divide by width * height rather than tracking what you asked for. */
const uint8_t *daegun_bitmap_pixels(const daegun_bitmap *bitmap, size_t *out_len);
void daegun_bitmap_free(daegun_bitmap *bitmap);

/* DAEGUN_ABSENT for a glyph that draws nothing. A space rasterizes to no pixels, which is an answer
 * rather than a failure. */
daegun_status daegun_font_rasterize_glyph(const daegun_font *font, uint16_t gid, float px,
                                          const daegun_axis *axes, size_t axes_len,
                                          daegun_bitmap **out);
/* A NULL opts means the defaults, so you need not build the struct to get them. */
daegun_status daegun_font_rasterize_glyph_with(const daegun_font *font, uint16_t gid, float px,
                                               const daegun_axis *axes, size_t axes_len,
                                               const daegun_raster_options *opts,
                                               daegun_bitmap **out);

/* hinting */

typedef struct daegun_hinted_outline daegun_hinted_outline;

#define DAEGUN_FLAG_ON_CURVE 0x01

daegun_status daegun_font_hinted_glyph(const daegun_font *font, uint16_t gid, float px,
                                       const daegun_axis *axes, size_t axes_len, int32_t hint_mode,
                                       daegun_hinted_outline **out);
/* Every pointer but out_count may be NULL. */
daegun_status daegun_hinted_outline_points(const daegun_hinted_outline *outline, size_t *out_count,
                                           const int32_t **out_x, const int32_t **out_y,
                                           const uint8_t **out_flags);
/* Where each contour ends, as an index one past its last point. */
const size_t *daegun_hinted_outline_contours(const daegun_hinted_outline *outline,
                                             size_t *out_count);
void daegun_hinted_outline_free(daegun_hinted_outline *outline);

typedef struct daegun_cff_hints daegun_cff_hints;

daegun_status daegun_font_cff_hints(const daegun_font *font, uint16_t gid, daegun_cff_hints **out);
/* THREE doubles per stem – is_vertical as 0 or 1, then the edge positions min and max, so a stem's
 * width is max - min. out_count receives the number of STEMS, not of doubles. */
const double *daegun_cff_hints_stems(const daegun_cff_hints *hints, size_t *out_count);
void daegun_cff_hints_free(daegun_cff_hints *hints);

/* shaping */

typedef struct daegun_run daegun_run;   /* one shaped run of text */

/* Borrowed views into a run, all valid until it is freed. */
const uint16_t *daegun_run_glyphs(const daegun_run *run, size_t *out_count);
const double   *daegun_run_advances(const daegun_run *run, size_t *out_count);
/* TWO doubles per glyph, x then y, so glyph i is at 2 * i. out_count is the number of DOUBLES. */
const double   *daegun_run_offsets(const daegun_run *run, size_t *out_count);
/* Which byte of the input each glyph came from. Several glyphs may share a cluster, and one glyph
 * may span several characters. */
const uint32_t *daegun_run_clusters(const daegun_run *run, size_t *out_count);
const uint8_t  *daegun_run_unsafe_to_break(const daegun_run *run, size_t *out_count);
const uint8_t  *daegun_run_unsafe_to_concat(const daegun_run *run, size_t *out_count);
const uint8_t  *daegun_run_safe_to_insert_tatweel(const daegun_run *run, size_t *out_count);

daegun_status daegun_run_complete(const daegun_run *run, bool *out);
daegun_status daegun_run_has_broken_syllable(const daegun_run *run, bool *out);
/* Which shaping model the run went through – the script's own, or the general one. */
daegun_status daegun_run_shaper(const daegun_run *run, daegun_str *out);
void daegun_run_free(daegun_run *run);

/* One OpenType feature you are turning on, off, or selecting an alternate of. */
typedef struct {
    const char *tag;
    uint32_t    value;
} daegun_feature;

daegun_status daegun_font_shape(const daegun_font *font, const char *text, const daegun_axis *axes,
                                size_t axes_len, bool vertical, daegun_run **out);
daegun_status daegun_font_shape_with_language(const daegun_font *font, const char *text,
                                              const daegun_axis *axes, size_t axes_len,
                                              bool vertical, const char *language,
                                              daegun_run **out);
/* script may be NULL, which is how C says None. */
daegun_status daegun_font_shape_with_features(const daegun_font *font, const char *text,
                                              const daegun_axis *axes, size_t axes_len,
                                              bool vertical, const char *script,
                                              const daegun_feature *features, size_t features_len,
                                              daegun_run **out);

#define DAEGUN_CLUSTER_MONOTONE_GRAPHEMES   0
#define DAEGUN_CLUSTER_MONOTONE_CHARACTERS  1
#define DAEGUN_CLUSTER_CHARACTERS           2
#define DAEGUN_CLUSTER_GRAPHEMES            3

#define DAEGUN_IGNORABLES_HIDE      0
#define DAEGUN_IGNORABLES_REMOVE    1
#define DAEGUN_IGNORABLES_PRESERVE  2

/* Everything the shaper can be told. Zeroing it is the default, as with daegun_raster_options. */
typedef struct {
    int32_t     cluster_level;   /* DAEGUN_CLUSTER_* */
    int32_t     ignorables;      /* DAEGUN_IGNORABLES_* */
    const char *before;          /* text preceding this run, for context. May be NULL. */
    const char *after;           /* text following it. May be NULL. */
    bool        beginning_of_text;
    bool        has_point_size;
    double      point_size;
    const daegun_feature *features;
    size_t      features_len;
    const char *script;          /* NULL to work it out */
    const char *language;        /* may be NULL */
    bool        report_unsafe_to_concat;
    bool        report_tatweel_positions;
    bool        suppress_dotted_circle;
    bool        has_invisible_glyph;
    uint16_t    invisible_glyph;
} daegun_shape_options;

daegun_status daegun_shape_options_default(daegun_shape_options *out);
/* A NULL opts means the defaults. */
daegun_status daegun_font_shape_with_options(const daegun_font *font, const char *text,
                                             const daegun_axis *axes, size_t axes_len,
                                             bool vertical, const daegun_shape_options *opts,
                                             daegun_run **out);

daegun_status daegun_font_measure_width(const daegun_font *font, const char *text,
                                        const daegun_axis *axes, size_t axes_len, double font_size,
                                        double *out);

/* justification */

typedef struct daegun_jstf_priorities daegun_jstf_priorities;
typedef struct daegun_jstf_mods       daegun_jstf_mods;
typedef struct daegun_justified       daegun_justified;

daegun_status daegun_font_justification_extenders(const daegun_font *font, const char *script_tag,
                                                  daegun_u16_list **out);
/* lang_sys_tag may be NULL. */
daegun_status daegun_font_justification_priorities(const daegun_font *font, const char *script_tag,
                                                   const char *lang_sys_tag,
                                                   daegun_jstf_priorities **out);
daegun_status daegun_jstf_priorities_count(const daegun_jstf_priorities *p, size_t *out);
/* One level, BORROWED – valid until the priorities are freed, and there is nothing to free. */
daegun_status daegun_jstf_priorities_at(const daegun_jstf_priorities *p, size_t index,
                                        const daegun_jstf_mods **out);
void daegun_jstf_priorities_free(daegun_jstf_priorities *p);

daegun_status daegun_font_shape_justified(const daegun_font *font, const char *text,
                                          const daegun_axis *axes, size_t axes_len, bool vertical,
                                          const daegun_jstf_mods *mods, bool shrink,
                                          daegun_run **out);

daegun_status daegun_font_justify(const daegun_font *font, const char *text,
                                  const daegun_axis *axes, size_t axes_len, bool vertical,
                                  const char *script_tag, const char *lang_sys_tag,
                                  double target_width, double tolerance, daegun_justified **out);
/* BORROWED. Do not pass it to daegun_run_free. */
const daegun_run *daegun_justified_run(const daegun_justified *j);
daegun_status daegun_justified_info(const daegun_justified *j, bool *out_has_level,
                                    size_t *out_level, bool *out_shrink, double *out_width,
                                    bool *out_best_effort);
void daegun_justified_free(daegun_justified *j);

/* bidi */

typedef struct daegun_bidi_runs daegun_bidi_runs;

/* base: 0 left-to-right, 1 right-to-left, -1 to let the first strong character decide. That is how
 * C says Option<bool>, and it recurs everywhere a direction may be unstated. */
daegun_status daegun_font_shape_bidi(const daegun_font *font, const char *text,
                                     const daegun_axis *axes, size_t axes_len, int32_t base,
                                     daegun_bidi_runs **out);
daegun_status daegun_font_shape_bidi_with(const daegun_font *font, const char *text,
                                          const daegun_axis *axes, size_t axes_len, int32_t base,
                                          const daegun_shape_options *opts,
                                          daegun_bidi_runs **out);
daegun_status daegun_bidi_runs_count(const daegun_bidi_runs *runs, size_t *out);
/* The run is BORROWED, valid until the set is freed. Do not pass it to daegun_run_free. */
daegun_status daegun_bidi_runs_at(const daegun_bidi_runs *runs, size_t index,
                                  const daegun_run **out_run, uint8_t *out_level,
                                  const size_t **out_chars, size_t *out_chars_count);
void daegun_bidi_runs_free(daegun_bidi_runs *runs);

/* layout */

#define DAEGUN_ALIGN_START    0
#define DAEGUN_ALIGN_END      1
#define DAEGUN_ALIGN_CENTER   2
#define DAEGUN_ALIGN_JUSTIFY  3

#define DAEGUN_WRITING_HORIZONTAL   0
#define DAEGUN_WRITING_VERTICAL_RL  1
#define DAEGUN_WRITING_VERTICAL_LR  2

#define DAEGUN_ORIENTATION_MIXED     0
#define DAEGUN_ORIENTATION_UPRIGHT   1
#define DAEGUN_ORIENTATION_SIDEWAYS  2

#define DAEGUN_BREAK_GREEDY   0
#define DAEGUN_BREAK_OPTIMAL  1

bool daegun_writing_mode_is_vertical(int32_t mode);

/* NOTE: zeroing this is NOT the default. max_inline_size of zero would wrap after every glyph;
 * daegun_layout_options_default sets it to infinity, which means "do not wrap". */
typedef struct {
    /* The measure, in the SAME 1000-upm units daegun_run_advances reports – not pixels. A value
     * smaller than one glyph does not wrap harder, it simply cannot be met.
     *
     * A line MAY exceed it when a single unbreakable run does; daegun_layout_info's inline_size is
     * the widest line, not a promise about this number. */
    double      max_inline_size;
    int32_t     align;             /* DAEGUN_ALIGN_* */
    int32_t     writing_mode;      /* DAEGUN_WRITING_* */
    int32_t     text_orientation;  /* DAEGUN_ORIENTATION_* */
    int32_t     base_direction;    /* 0 ltr, 1 rtl, -1 decide */
    const char *language;          /* may be NULL */
    bool        has_line_height;
    double      line_height;
    int32_t     strategy;          /* DAEGUN_BREAK_* */
    bool        has_max_lines;
    size_t      max_lines;
} daegun_layout_options;

typedef struct daegun_layout daegun_layout;

daegun_status daegun_layout_options_default(daegun_layout_options *out);
daegun_status daegun_font_layout(const daegun_font *font, const char *text,
                                 const daegun_axis *axes, size_t axes_len,
                                 const daegun_layout_options *opts, daegun_layout **out);
daegun_status daegun_layout_info(const daegun_layout *layout, size_t *out_line_count,
                                 double *out_inline_size, double *out_block_size,
                                 bool *out_has_truncated, size_t *out_truncated);
daegun_status daegun_layout_line(const daegun_layout *layout, size_t index, size_t *out_run_count,
                                 size_t *out_char_start, size_t *out_char_end, double *out_baseline,
                                 double *out_inline_size, double *out_ascent, double *out_descent,
                                 bool *out_hard_break);
/* The run is BORROWED, valid until the layout is freed. */
daegun_status daegun_layout_run(const daegun_layout *layout, size_t line, size_t index,
                                const daegun_run **out_run, double *out_offset_x,
                                double *out_offset_y, uint8_t *out_level, size_t *out_char_start,
                                size_t *out_char_end, bool *out_upright);
void daegun_layout_free(daegun_layout *layout);

/* text analysis, which needs no font */

daegun_status daegun_text_grapheme_boundaries(const char *text, daegun_u32_list **out);
daegun_status daegun_text_word_boundaries(const char *text, daegun_u32_list **out);
/* Two lists at matching indices: byte offsets, and one byte each, non-zero for a break the text
 * demands rather than merely allows. */
daegun_status daegun_text_line_break_opportunities(const char *text, daegun_u32_list **out_at,
                                                   daegun_blob **out_mandatory);
/* THREE numbers per run – start, end, and the script's id. Use daegun_script_name to name one; the
 * id is kept because a caller grouping runs compares ids rather than strings. */
daegun_status daegun_text_script_runs(const char *text, daegun_u32_list **out);
daegun_status daegun_script_name(uint16_t script, daegun_text **out);
/* DAEGUN_ABSENT where the script has no inherent direction. */
daegun_status daegun_script_is_rtl(uint16_t script, bool *out);

daegun_status daegun_text_resolve_bidi(const char *text, int32_t base, uint8_t *out_base_level,
                                       daegun_blob **out_levels, daegun_u32_list **out_visual_order);

/* A resolved paragraph, kept so one line at a time can be asked about – rebuilding it per line would
 * redo the whole resolution each time. */
typedef struct daegun_bidi_paragraph daegun_bidi_paragraph;
typedef struct daegun_visual_runs    daegun_visual_runs;

daegun_status daegun_text_bidi_paragraph(const char *text, int32_t base,
                                         daegun_bidi_paragraph **out);
daegun_status daegun_bidi_paragraph_base_level(const daegun_bidi_paragraph *p, uint8_t *out);
void daegun_bidi_paragraph_free(daegun_bidi_paragraph *p);

/* start and end are CHARACTER indices into the paragraph, not bytes. */
daegun_status daegun_text_line_visual_runs(const daegun_bidi_paragraph *p, size_t start, size_t end,
                                           daegun_visual_runs **out);
daegun_status daegun_visual_runs_count(const daegun_visual_runs *runs, size_t *out);
daegun_status daegun_visual_runs_at(const daegun_visual_runs *runs, size_t index,
                                    uint8_t *out_level, const size_t **out_chars,
                                    size_t *out_chars_count);
void daegun_visual_runs_free(daegun_visual_runs *runs);

/* color */

typedef struct daegun_colr_layers  daegun_colr_layers;
typedef struct daegun_palettes     daegun_palettes;
typedef struct daegun_glyph_bitmap daegun_glyph_bitmap;
typedef struct daegun_paint        daegun_paint;
typedef struct daegun_scene        daegun_scene;

/* One COLR v0 layer. is_foreground means the layer takes YOUR text color rather than one from the
 * palette, and the four channels are then meaningless. */
typedef struct {
    uint16_t gid;
    uint8_t  r, g, b, a;
    bool     is_foreground;
} daegun_colr_layer;

daegun_status daegun_font_colr_layers(const daegun_font *font, uint16_t gid,
                                      daegun_colr_layers **out);
daegun_status daegun_font_colr_layers_for_palette(const daegun_font *font, uint16_t gid,
                                                  uint16_t palette_index,
                                                  daegun_colr_layers **out);
const daegun_colr_layer *daegun_colr_layers_data(const daegun_colr_layers *layers,
                                                 size_t *out_count);
void daegun_colr_layers_free(daegun_colr_layers *layers);

/* light_safe and dark_safe are both false on a COLR v0 face, which states no flags – an absence of
 * information rather than a claim that the palette suits neither. */
typedef struct {
    uint16_t index;
    bool     light_safe;
    bool     dark_safe;
    bool     has_name_id;
    uint16_t name_id;
} daegun_palette_info;

daegun_status daegun_font_palette_count(const daegun_font *font, uint16_t *out);
daegun_status daegun_font_palette_info(const daegun_font *font, daegun_palettes **out);
const daegun_palette_info *daegun_palettes_data(const daegun_palettes *p, size_t *out_count);
void daegun_palettes_free(daegun_palettes *p);

/* An embedded bitmap, as PNG BYTES rather than pixels – the face stores them that way and daegun
 * does not decode images. You already have a PNG decoder or do not want one. */
daegun_status daegun_font_glyph_bitmap(const daegun_font *font, uint16_t gid, uint16_t target_ppem,
                                       daegun_glyph_bitmap **out);
const uint8_t *daegun_glyph_bitmap_png(const daegun_glyph_bitmap *b, size_t *out_len,
                                       uint16_t *out_ppem, int16_t *out_origin_x,
                                       int16_t *out_origin_y);
void daegun_glyph_bitmap_free(daegun_glyph_bitmap *b);

/* COLR v1 */

/* COLR v1 is a TREE – fifteen variants, nine holding a child – and it arrives flattened into an
 * array where children are indices. Node 0 is the root.
 *
 * EVERY variant uses child_start and child_count, so walking children needs no knowledge of which
 * variant you are looking at: Layers has as many as it has, the nine transforming variants have one,
 * Composite has TWO – source first, backdrop second – and the leaves have none.
 *
 * `numbers` means whatever `kind` says:
 *
 *   LINEAR_GRADIENT   [0..6) = x0, y0, x1, y1, x2, y2
 *   RADIAL_GRADIENT   [0..6) = x0, y0, r0, x1, y1, r1
 *   SWEEP_GRADIENT    [0..4) = cx, cy, start_angle, end_angle
 *   TRANSFORM         [0..6) = the matrix
 *   TRANSLATE         [0..2) = dx, dy
 *   SCALE             [0..2) = sx, sy          and [4..6) = center when has_center
 *   SCALE_UNIFORM     [0..1) = s               and [4..6) = center when has_center
 *   ROTATE            [0..1) = angle           and [4..6) = center when has_center
 *   SKEW              [0..2) = x_angle, y_angle and [4..6) = center when has_center
 *
 * Fixed at eight so there is no variant-dependent layout to get wrong. */
#define DAEGUN_PAINT_LAYERS           0
#define DAEGUN_PAINT_GLYPH            1
#define DAEGUN_PAINT_COLR_GLYPH       2
#define DAEGUN_PAINT_SOLID            3
#define DAEGUN_PAINT_LINEAR_GRADIENT  4
#define DAEGUN_PAINT_RADIAL_GRADIENT  5
#define DAEGUN_PAINT_SWEEP_GRADIENT   6
#define DAEGUN_PAINT_TRANSFORM        7
#define DAEGUN_PAINT_TRANSLATE        8
#define DAEGUN_PAINT_SCALE            9
#define DAEGUN_PAINT_SCALE_UNIFORM   10
#define DAEGUN_PAINT_ROTATE          11
#define DAEGUN_PAINT_SKEW            12
#define DAEGUN_PAINT_COMPOSITE       13

typedef struct {
    int32_t  kind;          /* DAEGUN_PAINT_* */
    uint32_t child_start;
    uint32_t child_count;
    uint32_t stops_start;
    uint32_t stops_count;
    uint16_t glyph_id;
    uint8_t  is_foreground;
    uint8_t  r, g, b, alpha;
    uint8_t  extend;
    uint8_t  composite_mode;
    uint8_t  has_center;
    double   numbers[8];
} daegun_paint_node;

daegun_status daegun_font_colr_v1_paint(const daegun_font *font, uint16_t gid,
                                        const daegun_axis *axes, size_t axes_len,
                                        uint16_t palette_index, daegun_paint **out);
const daegun_paint_node *daegun_paint_nodes(const daegun_paint *p, size_t *out_count);
/* A node's children are the child_count entries at child_start. */
const uint32_t *daegun_paint_children(const daegun_paint *p, size_t *out_count);
/* Every gradient's stops in one run: a node's are the stops_count entries at stops_start, and
 * out_colors holds FOUR bytes per stop, so stop i is at 4 * i. */
daegun_status daegun_paint_stops(const daegun_paint *p, size_t *out_count,
                                 const double **out_offsets, const uint8_t **out_colors);
void daegun_paint_free(daegun_paint *p);

/* out_skipped_ops is how many paint operations the renderer could not carry out. Non-zero means the
 * image is incomplete rather than wrong, and anything showing it should know. */
daegun_status daegun_font_render_colr_glyph(const daegun_font *font, uint16_t gid, float px,
                                            const daegun_axis *axes, size_t axes_len,
                                            uint16_t palette_index, daegun_scene **out);

/* The same, with the text color a COLR layer takes when it defers to the caller instead of naming a
 * palette entry. foreground is four bytes, RGBA; NULL means opaque black, as the call above uses. */
daegun_status daegun_font_render_colr_glyph_with(const daegun_font *font, uint16_t gid, float px,
                                                 const daegun_axis *axes, size_t axes_len,
                                                 uint16_t palette_index,
                                                 const uint8_t foreground[4],
                                                 daegun_scene **out);
const uint8_t *daegun_scene_rgba(const daegun_scene *s, size_t *out_len, size_t *out_width,
                                 size_t *out_height, int32_t *out_left, int32_t *out_top,
                                 size_t *out_skipped_ops);
void daegun_scene_free(daegun_scene *s);

/* the GPU path */

typedef struct daegun_batch daegun_batch;
typedef struct daegun_drawn daegun_drawn;
typedef struct daegun_color_slots daegun_color_slots;

/* Where a glyph's geometry landed in a batch. */
typedef struct {
    uint32_t band_base;
    uint32_t h_bands;
    uint32_t v_bands;
    uint32_t hull_base;
    float    box_min[2];
    float    box_max[2];
} daegun_glyph_slot;

/* What the batch's four buffers are made of. NONE of these were declared until round 9, so the
 * five calls below handed back `const void *` with a count and nothing to cast it to – the GPU data
 * path, which is the entire reason the batch is public, was unreadable from C. */

/* One point of a quadratic, in em space. */
typedef struct {
    float x;
    float y;
} daegun_curve_point;

/* One horizontal or vertical slice of a glyph, naming a run in the band-curve index. */
typedef struct {
    uint32_t first_curve;
    uint32_t curve_count;
} daegun_band;

/* One vertex of the polygon actually drawn, with the per-corner dilation the shader needs. */
typedef struct {
    float pos[2];
    float dilate[4];
} daegun_hull_vertex;

/* One flat-colored shape of a color glyph: the curves, and the shape's own tint in the 0..1
 * straight-alpha form daegun_glyph_instance.tint takes. Draw them in the order they came back –
 * they paint back to front and the shader does no depth testing. */
typedef struct {
    daegun_glyph_slot slot;
    float             tint[4];
} daegun_color_slot;

daegun_status daegun_batch_new(daegun_batch **out);
daegun_status daegun_batch_clear(daegun_batch *batch);
daegun_status daegun_batch_append(daegun_batch *batch, const float *quads, size_t count,
                                 daegun_glyph_slot *out);
/* Bumped whenever the buffers change, so you know when to re-upload. */
daegun_status daegun_batch_revision(const daegun_batch *batch, uint64_t *out);

/* The four buffers, borrowed. Valid until the batch is CHANGED or freed – appending a glyph may
 * reallocate, so holding one of these across a draw is holding a dangling pointer.
 * daegun_batch_revision is how to tell. */
const daegun_curve_point *daegun_batch_curves(const daegun_batch *batch, size_t *out_count);
const uint32_t *daegun_batch_band_curves(const daegun_batch *batch, size_t *out_count);
const daegun_band *daegun_batch_bands(const daegun_batch *batch, size_t *out_count);
const daegun_hull_vertex *daegun_batch_hulls(const daegun_batch *batch, size_t *out_count);
void daegun_batch_free(daegun_batch *batch);

daegun_status daegun_font_gpu_glyph(const daegun_font *font, daegun_batch *batch, uint16_t gid,
                                    const daegun_axis *axes, size_t axes_len,
                                    daegun_glyph_slot *out);
daegun_status daegun_font_gpu_color_glyph(const daegun_font *font, daegun_batch *batch,
                                          uint16_t gid, const daegun_axis *axes, size_t axes_len,
                                          uint16_t palette_index, daegun_color_slots **out);

/* The same, with the text color for layers that defer to it. NULL foreground means opaque black. */
daegun_status daegun_font_gpu_color_glyph_with(const daegun_font *font, daegun_batch *batch,
                                               uint16_t gid, const daegun_axis *axes,
                                               size_t axes_len, uint16_t palette_index,
                                               const uint8_t foreground[4],
                                               daegun_color_slots **out);
const daegun_color_slot *daegun_color_slots_data(const daegun_color_slots *slots,
                                                 size_t *out_count);
void daegun_color_slots_free(daegun_color_slots *slots);

#define DAEGUN_PREFER_AUTO       0
#define DAEGUN_PREFER_CPU        1
#define DAEGUN_PREFER_GPU        2
#define DAEGUN_PREFER_REFERENCE  3

/* Zeroing this is the default. */
typedef struct {
    int32_t prefer;              /* DAEGUN_PREFER_* */
    bool    strict;
    bool    has_cpu_below_ppem;
    float   cpu_below_ppem;
    bool    avoid_software_gpu;
} daegun_policy;

daegun_status daegun_policy_default(daegun_policy *out);

#define DAEGUN_DRAWN_NOTHING     0
#define DAEGUN_DRAWN_CPU         1
#define DAEGUN_DRAWN_GPU         2
#define DAEGUN_DRAWN_GPU_COLOR   3
#define DAEGUN_DRAWN_SCENE       4
#define DAEGUN_DRAWN_REFERENCE   5
#define DAEGUN_DRAWN_BATCH_FULL  6
#define DAEGUN_DRAWN_REFUSED     7

/* Draws one glyph wherever the policy and the device say it belongs.
 *
 * THERE IS NO DRAW TARGET HANDLE, and that is deliberate. DrawTarget borrows the batch mutably, and
 * a handle would let you outlive the borrow – a use-after-free C cannot see. Building it inside the
 * call makes that unrepresentable: a NULL device is the CPU-only arrangement, a non-NULL one is the
 * device-aware one, and the policy is a parameter rather than something you configure and keep.
 *
 * palette is the color palette index, or -1 for none. opts, policy and device may all be NULL. */
daegun_status daegun_font_draw_glyph(const daegun_font *font, daegun_batch *batch,
                                     const void *device, const daegun_policy *policy, uint16_t gid,
                                     float px, const daegun_axis *axes, size_t axes_len,
                                     const daegun_raster_options *opts, int32_t palette,
                                     daegun_drawn **out);

/* The same, with the text color for layers that defer to it. NULL foreground means opaque black. */
daegun_status daegun_font_draw_glyph_with(const daegun_font *font, daegun_batch *batch,
                                          const void *device, const daegun_policy *policy,
                                          uint16_t gid, float px, const daegun_axis *axes,
                                          size_t axes_len, const daegun_raster_options *opts,
                                          int32_t palette, const uint8_t foreground[4],
                                          daegun_drawn **out);

daegun_status daegun_drawn_kind(const daegun_drawn *d, int32_t *out);
daegun_status daegun_drawn_is_ok(const daegun_drawn *d, bool *out);
/* BORROWED. Do not free these separately – they belong to the draw result. */
daegun_status daegun_drawn_bitmap(const daegun_drawn *d, const daegun_bitmap **out);
daegun_status daegun_drawn_slot(const daegun_drawn *d, daegun_glyph_slot *out);
const daegun_color_slot *daegun_drawn_color_slots(const daegun_drawn *d, size_t *out_count);
daegun_status daegun_drawn_scene(const daegun_drawn *d, const daegun_scene **out);
void daegun_drawn_free(daegun_drawn *d);

/* paths and stroking */

/* A path: geometry as a value rather than a stream of pen calls.
 *
 * Built by driving it – daegun_path_move_to and friends are the same five calls a daegun_pen
 * carries, because on the Rust side a path *is* a pen. There is no _finish: a path is readable and
 * strokeable at any point, and may be extended afterwards. */
typedef struct daegun_path daegun_path;

/* What each verb takes from the point array, in order. */
#define DAEGUN_VERB_MOVE  0  /* one point */
#define DAEGUN_VERB_LINE  1  /* one point */
#define DAEGUN_VERB_QUAD  2  /* two points: control, end */
#define DAEGUN_VERB_CUBIC 3  /* three points: two controls, end */
#define DAEGUN_VERB_CLOSE 4  /* no points */

/* How a path is stroked. The same fields daegun_raster_options carries, standalone so a path can be
 * stroked without a rasterizer. `miter_limit` is read only when `join` is DAEGUN_JOIN_MITER. */
typedef struct {
    float   width;
    int32_t cap;   /* DAEGUN_CAP_* */
    int32_t join;  /* DAEGUN_JOIN_* */
    float   miter_limit;
} daegun_stroke_style;

daegun_path *daegun_path_new(void);
void daegun_path_free(daegun_path *path);

daegun_status daegun_path_move_to(daegun_path *path, float x, float y);
daegun_status daegun_path_line_to(daegun_path *path, float x, float y);
daegun_status daegun_path_quad_to(daegun_path *path, float cx, float cy, float x, float y);
daegun_status daegun_path_curve_to(daegun_path *path, float c1x, float c1y,
                                   float c2x, float c2y, float x, float y);
daegun_status daegun_path_close(daegun_path *path);

daegun_status daegun_path_is_empty(const daegun_path *path, int32_t *out);
/* What the path costs to fill, in the engine's own units – the number daegun_policy compares
 * against to decide whether a glyph is worth a GPU round trip. */
daegun_status daegun_path_cost(const daegun_path *path, size_t *out);
/* DAEGUN_ABSENT when the path has no points. */
daegun_status daegun_path_bounds(const daegun_path *path, double *out_min_x, double *out_min_y,
                                 double *out_max_x, double *out_max_y);

/* The verbs and points, COPIED into your buffers – the one place this ABI copies rather than
 * borrowing. Call with capacity 0 to learn the count, then again to fill. */
daegun_status daegun_path_verbs(const daegun_path *path, uint8_t *out, size_t capacity,
                                size_t *out_count);
daegun_status daegun_path_points(const daegun_path *path, float *out_x, float *out_y,
                                 size_t capacity, size_t *out_count);

/* Replays onto a pen, optionally through a 2x3 transform [a, b, c, d, e, f], or NULL for none. */
daegun_status daegun_path_replay(const daegun_path *path, const double *transform,
                                 const daegun_pen *pen);

/* A pen that appends to this path – the other direction through daegun_pen, so a glyph outline can
 * be captured as a value. BORROWS the path: valid until the path is freed. */
daegun_status daegun_path_as_pen(daegun_path *path, daegun_pen *out);

/* `tolerance` is how far the flattened curves may sit from the true ones, in the path's units. */
daegun_status daegun_path_stroke(const daegun_path *path, const daegun_stroke_style *style,
                                 float tolerance, const daegun_pen *pen);
/* The same, with the outline's self-intersections resolved into one boundary – what a filler that
 * does not do non-zero winding needs, since a stroke overlaps itself at every join. */
daegun_status daegun_path_stroke_simplified(const daegun_path *path,
                                            const daegun_stroke_style *style,
                                            float tolerance, const daegun_pen *pen);

/* Building a scene of your own, rather than receiving one from a color glyph.

 * A builder holds paths and the fills over them. Push each path once, keep the id it hands back,
 * and fill it as many times as you like at different transforms – which is how one glyph outline
 * serves every place it appears. Render turns the whole thing into a daegun_scene you read with
 * daegun_scene_rgba above.
 *
 * The scene is authored y-up. Render applies `[s, 0, 0, -s, 0, 0]` with `s = px / upem`, so that
 * scale is yours to choose: px = 2, upem = 1 draws a layout expressed in CSS pixels at 2x. */
typedef struct daegun_scene_builder daegun_scene_builder;

daegun_scene_builder *daegun_scene_builder_new(void);
void daegun_scene_builder_free(daegun_scene_builder *b);

/* Copies the path in, so you may free or reuse yours immediately. */
daegun_status daegun_scene_builder_push_path(daegun_scene_builder *b, const daegun_path *path,
                                             size_t *out_id);

/* `rgba` is four bytes, `transform` six doubles [a, b, c, d, e, f] for x' = a*x + c*y + e.
 * `rule` is DAEGUN_FILL_NONZERO or DAEGUN_FILL_EVENODD. DAEGUN_RANGE for an unknown path id, an
 * unknown rule, or a transform that is not finite. */
#define DAEGUN_FILL_NONZERO 0
#define DAEGUN_FILL_EVENODD 1
daegun_status daegun_scene_builder_fill(daegun_scene_builder *b, size_t path_id,
                                        const uint8_t rgba[4], int32_t rule,
                                        const double transform[6]);

/* The canvas is fitted to the ink, so fill a full-bleed background first if you want a fixed size.
 * Free the result with daegun_scene_free. */
daegun_status daegun_scene_builder_render(const daegun_scene_builder *b, float px, float upem,
                                          daegun_scene **out);

/* Reading a builder back. `op_count` is how many fills it holds; `path` hands back a copy of one
 * you pushed, which you free with daegun_path_free. */
daegun_status daegun_scene_builder_is_empty(const daegun_scene_builder *b, bool *out);
daegun_status daegun_scene_builder_op_count(const daegun_scene_builder *b, size_t *out);
daegun_status daegun_scene_builder_path(const daegun_scene_builder *b, size_t path_id,
                                        daegun_path **out);

/* Replays a hinted glyph onto a pen, converting F26Dot6 to whole pixels on the way. The other half
 * of daegun_font_hinted_glyph: that one grid-fits, this turns the result back into geometry. */
daegun_status daegun_hinted_outline_draw(const daegun_hinted_outline *outline,
                                         const daegun_pen *pen);

/* raw tables and font building */

/* One table's bytes, exactly as the file stores them. BORROWED: valid until the font is freed.
 *
 * `tag` is the four-character name – "GSUB", "cmap", "OS/2" – including any trailing space, since
 * "cvt " and "CFF " really are spelled that way. DAEGUN_ABSENT when the font has no such table,
 * which is not the same as a table that is empty.
 *
 * These are the font's bytes, not an instance's: a variable font's `glyf` here is the stored
 * default shape. daegun_font_instance_table is what resolves a location. */
daegun_status daegun_font_table(const daegun_font *font, const char *tag, daegun_bytes *out);

/* Every table the font carries, in sorted order. Free with daegun_str_list_free. */
daegun_status daegun_font_table_tags(const daegun_font *font, daegun_str_list **out);

daegun_status daegun_font_has_table(const daegun_font *font, const char *tag, int32_t *out);

/* A tag-to-bytes map daegun owns: what instancing produces, and what building an sfnt takes.
 *
 * Writable, because the Rust API's build_font takes any map – so a caller can instance a font, drop
 * a table, patch another, and build the result. A read-only view would translate the type and lose
 * the point of it. */
typedef struct daegun_table_map daegun_table_map;

/* Every table of the font pinned to `axes`. DAEGUN_ABSENT only when a variable font's own variation
 * tables do not parse.
 *
 * The bytes are COPIED out of the font. The Rust call borrows wherever a table passes through
 * untouched, and that borrow cannot cross into C – nothing would stop the font being freed while
 * the map still pointed into it. Use daegun_font_instance_table when only one table is wanted; it
 * keeps the saving. */
daegun_status daegun_font_instance_tables(const daegun_font *font, const daegun_axis *axes,
                                          size_t axis_count, daegun_table_map **out);

/* One table of the font pinned to `axes`, copied. Free with daegun_blob_free. */
daegun_status daegun_font_instance_table(const daegun_font *font, const daegun_axis *axes,
                                         size_t axis_count, const char *tag, daegun_blob **out);

daegun_table_map *daegun_table_map_new(void);
void daegun_table_map_free(daegun_table_map *map);

daegun_status daegun_table_map_count(const daegun_table_map *map, size_t *out);
/* BORROWED, and invalidated by _set and _remove as well as by _free. */
daegun_status daegun_table_map_tag_at(const daegun_table_map *map, size_t index, daegun_str *out);
daegun_status daegun_table_map_bytes_at(const daegun_table_map *map, size_t index,
                                        daegun_bytes *out);
daegun_status daegun_table_map_get(const daegun_table_map *map, const char *tag, daegun_bytes *out);
/* Copies the bytes. */
daegun_status daegun_table_map_set(daegun_table_map *map, const char *tag,
                                   const uint8_t *data, size_t len);
daegun_status daegun_table_map_remove(daegun_table_map *map, const char *tag);
/* Assembles the map into an sfnt: the directory, the offsets, and the checksums. An empty map is
 * DAEGUN_RANGE rather than a header describing nothing. Free with daegun_blob_free. */
daegun_status daegun_table_map_build(const daegun_table_map *map, daegun_blob **out);

/* The offsets `loca` stores, one per glyph plus a terminator. `format` is head's indexToLocFormat:
 * 0 for the short form, 1 for the long one. Free with daegun_usize_list_free. */
daegun_status daegun_parse_loca(const uint8_t *loca, size_t len, int16_t format,
                                size_t num_glyphs, daegun_usize_list **out);

/* Draws one glyph straight out of `glyf` bytes, composites resolved – the escape hatch for a table
 * that did not come from a font this ABI opened. */
daegun_status daegun_outline_glyf_bytes(const uint8_t *glyf, size_t glyf_len,
                                        const size_t *loca, size_t loca_len,
                                        uint16_t glyph, const daegun_pen *pen);

/* reading a table by hand */

/* The engine's own bounds-checked readers, for a private table, a vendor extension, or a field
 * daegun has no opinion about. Every one answers DAEGUN_RANGE rather than reading past the end. */
daegun_status daegun_read_u16_be(const uint8_t *data, size_t len, size_t off, uint16_t *out);
daegun_status daegun_read_i16_be(const uint8_t *data, size_t len, size_t off, int16_t *out);
daegun_status daegun_read_u24_be(const uint8_t *data, size_t len, size_t off, uint32_t *out);
daegun_status daegun_read_u32_be(const uint8_t *data, size_t len, size_t off, uint32_t *out);
daegun_status daegun_read_offset24(const uint8_t *data, size_t len, size_t off, size_t *out);

/* The writers. The Rust ones return nothing and no-op out of range; these say so instead, because a
 * caller writing past the end of its own buffer wants to be told. */
daegun_status daegun_write_u16_be(uint8_t *data, size_t len, size_t off, uint16_t value);
daegun_status daegun_write_i16_be(uint8_t *data, size_t len, size_t off, int16_t value);
daegun_status daegun_write_u32_be(uint8_t *data, size_t len, size_t off, uint32_t value);
daegun_status daegun_write_offset24(uint8_t *data, size_t len, size_t off, size_t value);

/* Whether `count` records of `stride` bytes starting at `start` fit within `len`. Reports the
 * answer directly: an overflow in that arithmetic is one of the things it exists to catch. */
int32_t daegun_records_fit(size_t start, size_t count, size_t stride, size_t len);

/* `n` bytes at `off`, or NULL when they do not fit. BORROWS `data`. The Rust form is
 * const-generic – window<4> – and C has no such thing, so the width is an argument; what survives
 * is the point of it: one bounds check, and a pointer either good for n bytes or NULL. */
const uint8_t *daegun_bytes_window(const uint8_t *data, size_t len, size_t off, size_t n);

/* Binary-searches `count` records whose keys you read out.
 *
 * `key_at` is called with an index and must write the key through out_key and return non-zero;
 * returning zero means the record could not be read, and the search answers DAEGUN_ABSENT. It must
 * not longjmp, throw, or free anything this call holds.
 *
 * On DAEGUN_OK, *out_found is non-zero when `target` was present and *out_index is where; zero when
 * it was not, and *out_index is where it would be inserted. */
daegun_status daegun_search_records(size_t count, uint32_t target,
                                    int32_t (*key_at)(size_t index, void *user, uint32_t *out_key),
                                    void *user, size_t *out_index, int32_t *out_found);

/* The font-unit rounding the whole spec is written in: floor(v + 0.5). */
int32_t daegun_ot_round(double value);

/* A glyph's index within a coverage table, or DAEGUN_ABSENT when it is not covered. */
daegun_status daegun_coverage_index(const uint8_t *data, size_t len, uint16_t glyph, uint16_t *out);
/* Every glyph a coverage table covers, in table order. Free with daegun_u16_list_free. */
daegun_status daegun_coverage_glyphs(const uint8_t *buf, size_t len, size_t off,
                                     daegun_u16_list **out);

/* Apple Advanced Typography */

/* Every handle below OWNS a copy of the table you pass it, so the buffer is yours again the moment
 * the call returns. The Rust views borrow, and a handle holding both bytes and a view into them is
 * self-referential – the copy is what makes the handle independent of your memory rather than
 * making you keep an ordering C has no way to check. */

/* An AAT lookup. All six published formats sit behind one _value, so there is one handle. */
typedef struct daegun_aat_lookup daegun_aat_lookup;

daegun_status daegun_aat_lookup_open(const uint8_t *data, size_t len, uint16_t num_glyphs,
                                     daegun_aat_lookup **out);
/* DAEGUN_ABSENT when the lookup maps nothing to this glyph. */
daegun_status daegun_aat_lookup_value(const daegun_aat_lookup *lookup, uint16_t glyph,
                                      uint16_t *out);
/* Every mapping. Free with daegun_glyph_value_list_free. */
daegun_status daegun_aat_lookup_entries(const daegun_aat_lookup *lookup,
                                        daegun_glyph_value_list **out);
void daegun_aat_lookup_free(daegun_aat_lookup *lookup);

/* One cell of a state table. */
typedef struct {
    uint16_t new_state;
    uint16_t flags;
    /* The two type-specific words. Rearrangement uses neither, ligature only the first. */
    uint16_t word1;
    uint16_t word2;
} daegun_aat_entry;

/* The classes and state every morx machine starts from. */
#define DAEGUN_AAT_CLASS_END_OF_TEXT   0
#define DAEGUN_AAT_CLASS_DELETED_GLYPH 2
#define DAEGUN_AAT_STATE_START_OF_TEXT 0

/* A morx state machine. The 32-bit form, whose newState is a state index. */
typedef struct daegun_aat_state_table daegun_aat_state_table;

/* `extra_words` is how many type-specific words each entry carries: none for rearrangement, one for
 * ligature and contextual substitution. */
daegun_status daegun_aat_state_table_open(const uint8_t *data, size_t len, size_t extra_words,
                                          uint16_t num_glyphs, daegun_aat_state_table **out);
/* Out-of-bounds glyphs get the table's own out-of-bounds class. */
daegun_status daegun_aat_state_table_class(const daegun_aat_state_table *table, uint16_t glyph,
                                           uint16_t *out);
/* DAEGUN_RANGE when the table has no such cell. */
daegun_status daegun_aat_state_table_entry(const daegun_aat_state_table *table, uint16_t state,
                                           uint16_t class_, daegun_aat_entry *out);
void daegun_aat_state_table_free(daegun_aat_state_table *table);

/* The ankr table: anchor points a morx machine attaches marks to. */
typedef struct daegun_ankr daegun_ankr;

daegun_status daegun_ankr_version(const uint8_t *data, size_t len, uint16_t *out);
/* One control point read straight out of a buffer, without an ankr around it. */
daegun_status daegun_ankr_control_point(const uint8_t *data, size_t len, size_t at,
                                        int16_t *out_x, int16_t *out_y);
daegun_status daegun_ankr_open(const uint8_t *data, size_t len, uint16_t num_glyphs,
                               daegun_ankr **out);
daegun_status daegun_ankr_point_count(const daegun_ankr *ankr, uint16_t glyph, uint32_t *out);
/* DAEGUN_ABSENT when the glyph has no anchor at that index. */
daegun_status daegun_ankr_anchor_point(const daegun_ankr *ankr, uint16_t glyph, uint16_t index,
                                       int16_t *out_x, int16_t *out_y);
void daegun_ankr_free(daegun_ankr *ankr);

/* variation data */

/* The FeatureVariations record a GSUB or GPOS table may point at: which features change at which
 * points in the design space. */
typedef struct daegun_feature_variations daegun_feature_variations;

/* DAEGUN_ABSENT when the layout table carries none, which most do. */
daegun_status daegun_feature_variations_open(const uint8_t *layout, size_t len,
                                             daegun_feature_variations **out);
/* The same, at an offset you already know. Always succeeds: the Rust `at` is infallible and lets
 * the accessors bounds-check, so reporting a failure here would mean inventing one. */
daegun_status daegun_feature_variations_at(const uint8_t *layout, size_t len, size_t at,
                                           daegun_feature_variations **out);
/* Which variation record applies at `coords` – normalized axis coordinates in 2.14 fixed point, as
 * daegun_font_normalized_axes produces. DAEGUN_ABSENT when none does. */
daegun_status daegun_feature_variations_find(const daegun_feature_variations *vars,
                                             const int32_t *coords, size_t coord_count,
                                             uint16_t *out);
/* The alternate feature table a variation substitutes for `feature`, as an offset. */
daegun_status daegun_feature_variations_substitute(const daegun_feature_variations *vars,
                                                   uint16_t variation, uint16_t feature,
                                                   size_t *out);
void daegun_feature_variations_free(daegun_feature_variations *vars);

/* One axis of one variation region: where it starts, peaks and ends. */
typedef struct {
    double start;
    double peak;
    double end;
} daegun_region_axis;

/* An item variation store, out of a GDEF, HVAR, MVAR or VVAR table.
 *
 * The one handle here that copies nothing: the Rust store is built rather than a view over the
 * table, so this owns it outright. */
typedef struct daegun_ivs daegun_ivs;

daegun_status daegun_ivs_parse(const uint8_t *buf, size_t len, size_t base, daegun_ivs **out);
daegun_status daegun_ivs_axis_count(const daegun_ivs *ivs, size_t *out);
daegun_status daegun_ivs_region_count(const daegun_ivs *ivs, size_t *out);
daegun_status daegun_ivs_region_axis(const daegun_ivs *ivs, size_t region, size_t axis,
                                     daegun_region_axis *out);
/* How many ItemVariationData subtables, and how many delta rows in one. */
daegun_status daegun_ivs_ivd_count(const daegun_ivs *ivs, size_t *out);
daegun_status daegun_ivs_ivd_rows(const daegun_ivs *ivs, size_t ivd, size_t *out);
/* BORROWED. Valid until the store is freed. NULL on a bad index. */
const size_t *daegun_ivs_ivd_region_indices(const daegun_ivs *ivs, size_t ivd, size_t *out_count);
const int32_t *daegun_ivs_ivd_row(const daegun_ivs *ivs, size_t ivd, size_t inner,
                                  size_t *out_count);

/* Each region's scalar at `location`, computed once for a whole run of deltas – the scalars depend
 * only on the location, so a caller resolving a thousand glyphs at one location computes them once.
 * Free with daegun_f64_list_free. */
daegun_status daegun_ivs_region_scalars(const daegun_ivs *ivs, const double *location,
                                        size_t axis_count, daegun_f64_list **out);
/* One delta, interpolated from those scalars. */
daegun_status daegun_ivs_delta(const daegun_ivs *ivs, size_t outer, size_t inner,
                               const double *scalars, size_t scalar_count, double *out);
void daegun_ivs_free(daegun_ivs *ivs);

/* A DeltaSetIndexMap: the indirection from an item to a store's (outer, inner) pair. */
typedef struct daegun_delta_set_index_map daegun_delta_set_index_map;

daegun_status daegun_delta_set_index_map_parse(const uint8_t *buf, size_t len, size_t base,
                                               daegun_delta_set_index_map **out);
daegun_status daegun_delta_set_index_map_count(const daegun_delta_set_index_map *map, size_t *out);
/* An index past the end is not an error: the map clamps to its last entry, which is what the spec
 * says a map shorter than the item count means. */
daegun_status daegun_delta_set_index_map_lookup(const daegun_delta_set_index_map *map, size_t index,
                                                size_t *out_outer, size_t *out_inner);
void daegun_delta_set_index_map_free(daegun_delta_set_index_map *map);

/* character properties, no font */

/* A character's Unicode general category. A uint32_t code point rather than a char, because C has
 * no type meaning "a scalar value" and wchar_t is 16 bits on Windows; a surrogate or a value past
 * U+10FFFF is DAEGUN_RANGE rather than a guess. */
#define DAEGUN_GC_UNASSIGNED           0
#define DAEGUN_GC_CONTROL              1
#define DAEGUN_GC_FORMAT               2
#define DAEGUN_GC_PRIVATE_USE          3
#define DAEGUN_GC_SURROGATE            4
#define DAEGUN_GC_LOWERCASE_LETTER     5
#define DAEGUN_GC_MODIFIER_LETTER      6
#define DAEGUN_GC_OTHER_LETTER         7
#define DAEGUN_GC_TITLECASE_LETTER     8
#define DAEGUN_GC_UPPERCASE_LETTER     9
#define DAEGUN_GC_SPACING_MARK        10
#define DAEGUN_GC_ENCLOSING_MARK      11
#define DAEGUN_GC_NONSPACING_MARK     12
#define DAEGUN_GC_DECIMAL_NUMBER      13
#define DAEGUN_GC_LETTER_NUMBER       14
#define DAEGUN_GC_OTHER_NUMBER        15
#define DAEGUN_GC_CONNECT_PUNCTUATION 16
#define DAEGUN_GC_DASH_PUNCTUATION    17
#define DAEGUN_GC_CLOSE_PUNCTUATION   18
#define DAEGUN_GC_FINAL_PUNCTUATION   19
#define DAEGUN_GC_INITIAL_PUNCTUATION 20
#define DAEGUN_GC_OTHER_PUNCTUATION   21
#define DAEGUN_GC_OPEN_PUNCTUATION    22
#define DAEGUN_GC_CURRENCY_SYMBOL     23
#define DAEGUN_GC_MODIFIER_SYMBOL     24
#define DAEGUN_GC_MATH_SYMBOL         25
#define DAEGUN_GC_OTHER_SYMBOL        26
#define DAEGUN_GC_LINE_SEPARATOR      27
#define DAEGUN_GC_PARAGRAPH_SEPARATOR 28
#define DAEGUN_GC_SPACE_SEPARATOR     29

daegun_status daegun_char_general_category(uint32_t codepoint, int32_t *out);

/* Whether a character stands upright in vertical text. `has_vertical_form` is what
 * daegun_char_vertical_form answers for the same character: the two characters whose orientation is
 * "rotated unless a vertical form exists" need it, and asking keeps this from looking the
 * substitution up twice. */
daegun_status daegun_char_is_upright(uint32_t codepoint, int32_t has_vertical_form, int32_t *out);

/* The vertical presentation form, or DAEGUN_ABSENT when there is none – an em dash becomes a
 * vertical one, an ideographic comma moves to the corner of its box. */
daegun_status daegun_char_vertical_form(uint32_t codepoint, uint32_t *out);

/* the GPU backends and routing */

/* THE ONE CARVE-OUT FROM RULE 5. Everything above is thread-safe; nothing in this section is.
 * A renderer, a target and a geometry each hold raw device handles, command queues and function
 * tables, and none of the three is safe to drive from two threads at once – which is true of
 * Metal, Vulkan and Direct3D themselves, not something this ABI added. One thread per renderer.
 *
 * Rules 1 to 4 hold here unchanged. In particular a target or a geometry keeps its renderer alive:
 * freeing the renderer first is allowed, and the device goes when the last of them does. */

/* The shader source, so an application that already owns a device can compile the pipeline itself
 * and draw daegun's output through it. */
#define DAEGUN_SHADER_GLSL 0  /* OpenGL 4.3+ / ES 3.1+. Prepend your own #version line. */
#define DAEGUN_SHADER_HLSL 1  /* D3D11 and D3D12 at SM 5.0, and Vulkan through DXC. */
#define DAEGUN_SHADER_MSL  2  /* Metal Shading Language 2.2 and up. */

#define DAEGUN_STAGE_VERTEX            0
#define DAEGUN_STAGE_FRAGMENT          1
#define DAEGUN_STAGE_SUBPIXEL_FRAGMENT 2

/* Free with daegun_text_free. An owned string rather than a borrowed view of the static source,
 * because every shader compiler wants either a NUL or a length and daegun_text carries both. */
daegun_status daegun_shader_source(int32_t language, int32_t stage, daegun_text **out);

/* One glyph's instance data, uploaded straight to the GPU. Fields are public and this struct IS the
 * wire format: the size assertion at the bottom of this header is what keeps the two in step. */
typedef struct {
    float    glyph_box[4];
    float    tint[4];
    float    offset[2];
    float    em_pixels[2];
    float    scale;
    uint32_t band_base;
    uint32_t bands_per_axis;
    uint32_t hull_base;
    float    inv_scale;
    float    _pad[3];
} daegun_glyph_instance;

#define DAEGUN_MAX_SUBPIXEL_WEIGHTS 64
#define DAEGUN_MAX_SUBPIXEL_TAPS     8
#define DAEGUN_MAX_SUPERSAMPLE       4

/* The subpixel filter. Build it with daegun_subpixel_params_from_layout rather than by hand: most
 * of it is a kernel the engine derives from the layout, and `taps` past DAEGUN_MAX_SUBPIXEL_TAPS
 * indexes past the shader's own table. */
typedef struct {
    float    weights[DAEGUN_MAX_SUBPIXEL_WEIGHTS * 3];
    uint32_t oversample[2];
    uint32_t taps[2];
    int32_t  origin[2];
    uint32_t channels;
    uint32_t supersample;
} daegun_subpixel_params;

/* Builds a glyph's instance data from where its geometry landed in the batch.
 *
 * Use this rather than filling daegun_glyph_instance in yourself. `inv_scale` must be zero and not
 * infinity at a zero scale, or the coordinate the fragment shader rebuilds stops being finite; and
 * `bands_per_axis` is one number because both axes are sliced by the same n, which is not something
 * the struct can say. `offset` and `em_pixels` are two floats, `tint` is four, RGBA. */
daegun_status daegun_glyph_slot_instance(const daegun_glyph_slot *slot, const float *offset,
                                         float scale, const float *em_pixels, const float *tint,
                                         daegun_glyph_instance *out);

/* `layout` is one of the DAEGUN_LAYOUT_* constants; DAEGUN_LAYOUT_GRAYSCALE is the default. */
daegun_status daegun_subpixel_params_from_layout(int32_t layout, daegun_subpixel_params *out);

#define DAEGUN_MODE_GRAYSCALE 0
#define DAEGUN_MODE_SUBPIXEL  1

/* What a device is, and what it calls itself.
 *
 * Every backend's _renderer_profile produces one, and daegun_device_profile_new builds one without
 * a device at all – for a caller that opened its own, or that is deciding before opening any. */
typedef struct daegun_device_profile daegun_device_profile;

#define DAEGUN_DEVICE_UNKNOWN    0
#define DAEGUN_DEVICE_DISCRETE   1
#define DAEGUN_DEVICE_INTEGRATED 2
#define DAEGUN_DEVICE_VIRTUAL    3
#define DAEGUN_DEVICE_SOFTWARE   4

daegun_status daegun_device_profile_new(int32_t kind, const char *name,
                                        daegun_device_profile **out);
/* From a VkPhysicalDeviceType, for a caller that already called vkGetPhysicalDeviceProperties. */
daegun_status daegun_device_profile_from_vulkan(int32_t device_type, const char *name,
                                                daegun_device_profile **out);
daegun_status daegun_device_profile_kind(const daegun_device_profile *profile, int32_t *out);
/* Free with daegun_text_free. */
daegun_status daegun_device_profile_name(const daegun_device_profile *profile, daegun_text **out);
void daegun_device_profile_free(daegun_device_profile *profile);

/* What the GPU extraction said about a glyph, which is one input to the routing decision. */
#define DAEGUN_GPU_OK             0
#define DAEGUN_GPU_NO_OUTLINE     1
#define DAEGUN_GPU_TOO_COMPLEX    2
#define DAEGUN_GPU_NON_FINITE     3
#define DAEGUN_GPU_BATCH_FULL     4
#define DAEGUN_GPU_NOT_FLAT_COLOR 5

/* Where the glyph belongs. Rendered::Refused carries a reason in Rust, so it is two codes here –
 * a C caller switches once and both arms are distinct outcomes. */
#define DAEGUN_ROUTED_NOTHING                   0
#define DAEGUN_ROUTED_CPU                       1
#define DAEGUN_ROUTED_GPU                       2
#define DAEGUN_ROUTED_REFERENCE                 3
#define DAEGUN_ROUTED_SCENE                     4
#define DAEGUN_ROUTED_FLUSH_AND_RETRY           5
#define DAEGUN_ROUTED_REFUSED_NON_FINITE        6
#define DAEGUN_ROUTED_REFUSED_PREFERENCE_UNMET  7

/* What a caller is asking for. The flags are int32_t rather than bool for the same reason every
 * other flag in this header is. */
typedef struct {
    float   ppem;
    int32_t hinted;
    int32_t stroked;
    int32_t gamma;
    int32_t emboldened;
    int32_t obliqued;
} daegun_request;

/* The routing decision on its own, without drawing anything – for a caller with its own drawing to
 * do that wants only the policy answered. `device` may be NULL, meaning there is no GPU. */
daegun_status daegun_route(int32_t attempt, const daegun_request *request,
                           const daegun_device_profile *device, const daegun_policy *policy,
                           int32_t *out);

/* Why a backend declined, when one did. The detail is in daegun_last_error; this is the part every
 * backend agrees on, since a VkResult means nothing to a caller drawing through Metal. */
#define DAEGUN_REFUSAL_NO_DEVICE   0
#define DAEGUN_REFUSAL_BAD_TARGET  1
#define DAEGUN_REFUSAL_UNSUPPORTED 2
#define DAEGUN_REFUSAL_FAILED      3

/* Declares one backend's surface; the four are identical by construction.
 *
 * Availability: metal on Apple platforms, d3d11 and d3d12 on Windows, vulkan everywhere. Vulkan is
 * opened by name at run time, so a machine without it answers DAEGUN_UNSUPPORTED from _renderer_new
 * instead of failing to load this library. */
#define DAEGUN_DECLARE_BACKEND(b)                                                                  \
    typedef struct daegun_##b##_renderer daegun_##b##_renderer;                                    \
    typedef struct daegun_##b##_target   daegun_##b##_target;                                      \
    typedef struct daegun_##b##_geometry daegun_##b##_geometry;                                    \
                                                                                                   \
    /* DAEGUN_UNSUPPORTED when there is no such device, which is an answer and not a failure. */    \
    daegun_status daegun_##b##_renderer_new(daegun_##b##_renderer **out);                           \
    /* Safe to call while a target or geometry is still alive; the device goes with the last. */    \
    void daegun_##b##_renderer_free(daegun_##b##_renderer *renderer);                               \
    /* Free with daegun_text_free. */                                                               \
    daegun_status daegun_##b##_renderer_device_name(const daegun_##b##_renderer *renderer,          \
                                                    daegun_text **out);                             \
    /* Free with daegun_device_profile_free. */                                                     \
    daegun_status daegun_##b##_renderer_profile(const daegun_##b##_renderer *renderer,              \
                                                daegun_device_profile **out);                       \
    daegun_status daegun_##b##_renderer_supports_subpixel(const daegun_##b##_renderer *renderer,     \
                                                          int32_t *out);                            \
    /* The default projection, column-major. Needs no device: it depends on this API's clip-space   \
     * convention and nothing else, so it can be built before anything is opened. */                \
    daegun_status daegun_##b##_ortho(uint32_t width, uint32_t height, float *out);                  \
                                                                                                   \
    daegun_status daegun_##b##_target_new(const daegun_##b##_renderer *renderer,                        \
                                      uint32_t width, uint32_t height,                              \
                                      daegun_##b##_target **out);                                   \
    daegun_status daegun_##b##_target_width(const daegun_##b##_target *target, uint32_t *out);      \
    daegun_status daegun_##b##_target_height(const daegun_##b##_target *target, uint32_t *out);     \
    /* BORROWED, BGRA. What the last _read_pixels left behind, without the round trip. Valid until  \
     * the target is drawn into again or freed. */                                                  \
    const uint8_t *daegun_##b##_target_pixels(const daegun_##b##_target *target,                    \
                                              size_t *out_count);                                   \
    /* One pixel as BGRA into four bytes. DAEGUN_RANGE outside the target. */                       \
    daegun_status daegun_##b##_target_pixel(const daegun_##b##_target *target,                      \
                                            uint32_t x, uint32_t y, uint8_t *out);                  \
    void daegun_##b##_target_free(daegun_##b##_target *target);                                     \
    /* An offscreen target in the caller's byte order. format is a DAEGUN_SURFACE_* value. */        \
    daegun_status daegun_##b##_target_with_format(const daegun_##b##_renderer *renderer,             \
                                                  uint32_t width, uint32_t height, int32_t format,   \
                                                  daegun_##b##_target **out);                        \
    /* What the target clears to before each draw, four bytes RGBA. NULL keeps what it already       \
     * holds, which is how a second geometry draws over the first rather than erasing it. */         \
    daegun_status daegun_##b##_target_set_clear(daegun_##b##_target *target,                         \
                                                const uint8_t clear[4]);                             \
                                                                                                   \
    daegun_status daegun_##b##_geometry_new(const daegun_##b##_renderer *renderer,                      \
                                        const daegun_batch *batch,                                  \
                                        daegun_##b##_geometry **out);                               \
    /* Compare against daegun_batch_revision to know whether this upload is stale. */               \
    daegun_status daegun_##b##_geometry_revision(const daegun_##b##_geometry *geometry,              \
                                                 uint64_t *out);                                    \
    void daegun_##b##_geometry_free(daegun_##b##_geometry *geometry);                                \
                                                                                                   \
    daegun_status daegun_##b##_draw(const daegun_##b##_renderer *renderer,                          \
                                    daegun_##b##_target *target,                                    \
                                    const daegun_##b##_geometry *geometry,                          \
                                    const daegun_glyph_instance *instances, size_t instance_count,  \
                                    const daegun_subpixel_params *subpixel, int32_t mode);          \
    /* Under a projection of your own – column-major, NULL for the default. */                      \
    daegun_status daegun_##b##_draw_with(const daegun_##b##_renderer *renderer,                     \
                                         daegun_##b##_target *target,                               \
                                         const daegun_##b##_geometry *geometry,                     \
                                         const daegun_glyph_instance *instances,                    \
                                         size_t instance_count,                                     \
                                         const daegun_subpixel_params *subpixel, int32_t mode,      \
                                         const float *projection);                                  \
    daegun_status daegun_##b##_wait(const daegun_##b##_renderer *renderer,                          \
                                    daegun_##b##_target *target);                                   \
    /* Waits, copies off the device, and hands back BGRA. BORROWED, valid until the target is drawn \
     * into again or freed. NULL on failure, with the reason in daegun_last_error.                  \
     *                                                                                              \
     * THIS IS THE EXPENSIVE CALL. On a discrete GPU the readback dominates: 489 us of a 513 us     \
     * frame at 1024x1024, measured. A caller that can consume the target on the device should not  \
     * call it at all. */                                                                           \
    const uint8_t *daegun_##b##_read_pixels(const daegun_##b##_renderer *renderer,                  \
                                            daegun_##b##_target *target, size_t *out_count)

/* The byte order of a surface. CAMetalLayer and most swapchains are BGRA; daegun's own offscreen
 * targets are RGBA unless asked otherwise. */
#define DAEGUN_SURFACE_RGBA8 0
#define DAEGUN_SURFACE_BGRA8 1

#if defined(__APPLE__)
DAEGUN_DECLARE_BACKEND(metal);

/* Metal only: re-uploads into an existing geometry if the batch changed, and does nothing if it did
 * not – so it may be called every frame. The plain _geometry call allocates a new upload each time.
 * The other three backends have no equivalent to translate. */
daegun_status daegun_metal_geometry_sync(daegun_metal_geometry *geometry,
                                         const daegun_metal_renderer *renderer,
                                         const daegun_batch *batch);

/* Adopts a device the caller already made, which is what lets daegun draw into that device's
 * surfaces: a drawable's texture belongs to the device its CAMetalLayer was created on. daegun
 * retains the device and releases only its own reference, so it survives the renderer even if you
 * release yours first. */
daegun_status daegun_metal_renderer_from_device(void *device, daegun_metal_renderer **out);

/* A target over an MTLTexture daegun did not create. No format argument, unlike the other three
 * backends: the texture carries its own, and anything but RGBA8 or BGRA8 answers DAEGUN_RANGE. */
daegun_status daegun_metal_target_from_texture(const daegun_metal_renderer *renderer, void *texture,
                                               uint32_t width, uint32_t height,
                                               daegun_metal_target **out);

/* The same over a CAMetalDrawable. daegun presents it on the command buffer carrying the draw, so
 * the queue orders the two – do NOT present it yourself as well. Hold the drawable until the draw
 * is submitted; daegun retains it and releases it with the target. */
daegun_status daegun_metal_target_from_drawable(const daegun_metal_renderer *renderer,
                                                void *drawable, uint32_t width, uint32_t height,
                                                daegun_metal_target **out);
#endif

DAEGUN_DECLARE_BACKEND(vulkan);

/* What the target clears to before each draw, four bytes RGBA. NULL keeps what the target already
 * holds, which is how a second geometry draws over the first rather than erasing it. */


/* A target over a VkImage daegun did not create, such as one acquired from a swapchain. daegun
 * builds a view and framebuffer over it and destroys only those. It leaves the image in
 * VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, so presenting it is still yours to arrange. */
daegun_status daegun_vulkan_target_from_image(const daegun_vulkan_renderer *renderer,
                                              uint64_t image, uint32_t width, uint32_t height,
                                              int32_t format, daegun_vulkan_target **out);

/* Adopts a device the caller already made, which is what lets daegun draw into that device's
 * swapchain. daegun destroys neither the device nor the instance, so both must outlive it.
 *
 * dual_src_blend is what you ENABLED at device creation, not what the hardware supports: daegun
 * cannot tell them apart, and without it there is no subpixel pipeline. */
/* The handles behind the renderer, for building a swapchain on the device daegun made. They belong
 * to the renderer and die with it, so nothing built on them may outlive it. Vulkan's dispatchable
 * handles are pointers, so each arrives as a plain void *. Any out-pointer may be NULL. */
daegun_status daegun_vulkan_renderer_handles(const daegun_vulkan_renderer *renderer,
                                             void **out_instance, void **out_physical_device,
                                             void **out_device, uint32_t *out_queue_family);

daegun_status daegun_vulkan_renderer_from_device(void *instance, void *physical_device,
                                                 void *device, uint32_t queue_family,
                                                 int32_t dual_src_blend,
                                                 daegun_vulkan_renderer **out);

#if defined(_WIN32)
DAEGUN_DECLARE_BACKEND(d3d11);
DAEGUN_DECLARE_BACKEND(d3d12);
/* Direct3D only: the feature level as a string like "11_1", and whether the adapter is WARP –
 * which daegun_policy's avoid_software_gpu exists to steer away from. */
daegun_status daegun_d3d11_feature_level(const daegun_d3d11_renderer *renderer, daegun_text **out);
daegun_status daegun_d3d11_is_software(const daegun_d3d11_renderer *renderer, int32_t *out);
/* The device and its immediate context, for drawing into a swapchain the caller owns. Both belong
 * to the renderer and die with it. Either out-pointer may be NULL. */
daegun_status daegun_d3d11_renderer_handles(const daegun_d3d11_renderer *renderer,
                                            void **out_device, void **out_second);
/* Adopts a device the caller already made, which is what lets daegun draw into that device's
 * swapchain: a backbuffer belongs to the device its swapchain was created on. daegun takes a COM
 * reference on each handle and releases only those.
 *
 * D3D11 takes the device and its immediate context. D3D12 takes the device and a direct command
 * queue – using the caller's queue is what orders the draw against their Present without a fence. */
daegun_status daegun_d3d11_renderer_from_device(void *device, void *context,
                                                daegun_d3d11_renderer **out);
daegun_status daegun_d3d12_renderer_from_device(void *device, void *queue,
                                                daegun_d3d12_renderer **out);

/* A target over a texture daegun did not create, such as a swapchain backbuffer. format is a
 * DAEGUN_SURFACE_* value. D3D12 leaves the resource in D3D12_RESOURCE_STATE_RENDER_TARGET, so
 * transitioning it to PRESENT is yours, as presentation itself is. */
daegun_status daegun_d3d11_target_from_texture(const daegun_d3d11_renderer *renderer, void *texture,
                                               uint32_t width, uint32_t height, int32_t format,
                                               daegun_d3d11_target **out);
daegun_status daegun_d3d12_target_from_texture(const daegun_d3d12_renderer *renderer, void *texture,
                                               uint32_t width, uint32_t height, int32_t format,
                                               daegun_d3d12_target **out);

daegun_status daegun_d3d12_feature_level(const daegun_d3d12_renderer *renderer, daegun_text **out);
daegun_status daegun_d3d12_is_software(const daegun_d3d12_renderer *renderer, int32_t *out);
/* The device and its command queue – the second handle is the queue on D3D12 where it is the
 * immediate context on D3D11. Either out-pointer may be NULL. */
daegun_status daegun_d3d12_renderer_handles(const daegun_d3d12_renderer *renderer,
                                            void **out_device, void **out_second);
#endif

/* the atlas packer, and rules */

/* Everything in this section is a rule the engine owns and a caller would otherwise re-derive.
 * They exist because the verification round went looking for `pub fn` on re-exported *types* – the
 * one shape no version of the parity check could see – and found a whole type with no C form. */

/* Where a glyph landed in an atlas. */
typedef struct {
    size_t x;
    size_t y;
    size_t w;
    size_t h;
} daegun_rect;

/* A shelf packer for an atlas of a given size: the same one daegun's own glyph cache uses.
 *
 * Rasterizing gives you pixels and a size; putting a thousand of them into one texture is a packing
 * problem, and this is the solved version of it. */
typedef struct daegun_shelf_packer daegun_shelf_packer;

daegun_shelf_packer *daegun_shelf_packer_new(size_t width, size_t height);
void daegun_shelf_packer_free(daegun_shelf_packer *packer);
/* DAEGUN_ABSENT when the atlas is full – an answer, not a bad argument. Flush and start a new one,
 * which is what daegun's own cache does. */
daegun_status daegun_shelf_packer_insert(daegun_shelf_packer *packer, size_t width, size_t height,
                                         daegun_rect *out);
/* Empties the atlas, keeping its size. */
daegun_status daegun_shelf_packer_reset(daegun_shelf_packer *packer);

/* Baseline to baseline: ascent - descent + line_gap. A call rather than a subtraction you write,
 * because `descent` is negative and the obvious `ascent + descent + line_gap` is off by twice it
 * while looking plausible on every font. */
daegun_status daegun_line_metrics_height(const daegun_line_metrics *metrics, double *out);

/* What a glyph id became after subsetting, DAEGUN_ABSENT if the subset dropped it.
 *
 * NOT the same as indexing daegun_subset_gid_map yourself, and that is the point: an empty map means
 * every glyph kept its old id, and a zero for any id but zero means dropped rather than mapped to
 * .notdef. Get either wrong and glyphs render as the wrong shape, silently. */
daegun_status daegun_subset_new_gid(const daegun_subset *subset, uint16_t old_gid, uint16_t *out);

/* Whether a hint mode may run the autohinter. Two of the five may; which two is daegun's rule, and
 * a caller that wrote the comparison out is the one that breaks when a third joins them. */
daegun_status daegun_hint_mode_may_autohint(int32_t mode, int32_t *out);

/* Two of the four cluster levels group by grapheme, and two are monotone – so neither question is a
 * comparison against one constant. */
daegun_status daegun_cluster_level_is_graphemes(int32_t level, int32_t *out);
daegun_status daegun_cluster_level_is_monotone(int32_t level, int32_t *out);

/* The OpenType tags a script maps to, most specific first – Devanagari is dev2 then deva, and a
 * shaper tries them in order. Free with daegun_str_list_free. */
daegun_status daegun_script_opentype_tags(uint16_t script, daegun_str_list **out);
/* Whether a script takes its identity from what surrounds it rather than standing alone: true for
 * Common (punctuation, digits, spaces), Inherited (combining marks), and unknown ids. A comma
 * between two Arabic words belongs to that run; between two Latin words, to that one.
 *
 * NOT about contextual shaping – Arabic joins and Devanagari reorders, and both are scripts in
 * their own right. */
daegun_status daegun_script_is_context_dependent(uint16_t script, int32_t *out);

/* STAT axis values */

/* Which fields of daegun_stat_value mean anything. */
#define DAEGUN_STAT_SINGLE 0
#define DAEGUN_STAT_RANGE  1
#define DAEGUN_STAT_LINKED 2
#define DAEGUN_STAT_COMBO  3

/* One STAT axis value, with the four variants flattened into one shape – the same choice the COLR
 * paint graph made, since C has no sum type and four parallel lists would have to be correlated by
 * index. */
typedef struct {
    int32_t  kind;          /* DAEGUN_STAT_* */
    uint16_t axis_index;    /* meaningless for COMBO, which spans axes */
    uint8_t  elidable;
    uint8_t  has_name;      /* whether daegun_stat_value_name will answer */
    double   value;         /* SINGLE's value, RANGE's nominal, LINKED's value; 0 for COMBO */
    double   min;           /* RANGE only */
    double   max;           /* RANGE only */
    double   linked_value;  /* LINKED only */
    uint32_t combo_start;   /* COMBO only: where its pairs start in daegun_stat_combo_values */
    uint32_t combo_count;
} daegun_stat_value;

/* One (axis, value) pair of a COMBO. */
typedef struct {
    uint16_t axis_index;
    double   value;
} daegun_axis_value;

daegun_status daegun_stat_value_at(const daegun_stat *stat, size_t index, daegun_stat_value *out);
/* BORROWED, valid until the STAT handle is freed – unlike most strings here, which come back as a
 * daegun_text you free. These are already owned by the handle. DAEGUN_ABSENT when unnamed. */
daegun_status daegun_stat_value_name(const daegun_stat *stat, size_t index, daegun_str *out);
/* BORROWED. Index from a value's combo_start for combo_count entries. */
const daegun_axis_value *daegun_stat_combo_values(const daegun_stat *stat, size_t *out_count);

/* more of the GPU wire format */

/* A device profile from what Direct3D or Metal reports. `uma` is a tri-state – negative unknown,
 * zero discrete, positive unified – because the Rust argument is Option<bool> and the third state is
 * the one that matters: whether a readback crosses a bus is not something to guess at. */
daegun_status daegun_device_profile_from_d3d(int32_t software, int32_t uma, const char *name,
                                             daegun_device_profile **out);
daegun_status daegun_device_profile_from_metal(int32_t uma, const char *name,
                                               daegun_device_profile **out);
/* Whether this is a software rasterizer pretending to be a GPU. Not kind == DAEGUN_DEVICE_SOFTWARE,
 * which is what you would write and what breaks the day a second kind counts. */
daegun_status daegun_device_profile_is_software(const daegun_device_profile *profile, int32_t *out);

/* The affine variant of daegun_glyph_slot_instance: `transform` is the 2x2 part [a, b, c, d], and
 * the em-space extent each axis needs is the length of its column. For a rotation or a skew. */
daegun_status daegun_glyph_slot_instance_affine(const daegun_glyph_slot *slot, const float *offset,
                                                float scale, const float *transform,
                                                const float *tint, daegun_glyph_instance *out);

/* How far past its box a glyph's coverage reaches, per axis. Two of these, and they are different:
 * _dilation is fractional pixels, _pad is the whole pixels an atlas slot must grow by, rounding up
 * on the negative origin only. */
daegun_status daegun_subpixel_params_dilation(const daegun_subpixel_params *params, float *out);
daegun_status daegun_subpixel_params_pad(const daegun_subpixel_params *params, size_t *out);

/* The same filter, supersampled n times. */
daegun_status daegun_subpixel_params_with_supersampling(const daegun_subpixel_params *params,
                                                        uint32_t n, daegun_subpixel_params *out);

/* A filter of your own: three arrays of taps_x * taps_y weights, one per channel, laid end to end.
 * DAEGUN_RANGE past DAEGUN_MAX_SUBPIXEL_TAPS or DAEGUN_MAX_SUPERSAMPLE. */
daegun_status daegun_subpixel_params_from_weights(uint8_t oversample_x, uint8_t oversample_y,
                                                  uint8_t taps_x, uint8_t taps_y,
                                                  int8_t origin_x, int8_t origin_y,
                                                  const float *weights,
                                                  daegun_subpixel_params *out);

/* A named layout's identity, for use as a cache key: two layouts that filter identically share one,
 * so a glyph cached under either is valid under the other. */
daegun_status daegun_subpixel_layout_key(int32_t layout, uint64_t *out);

/* layouts */

/* Repeated on the Rust side as `const _: () = assert!(size_of::<…>() == …)`.
 *
 * Both must agree or neither builds. This is the discipline that caught a four-byte
 * VkPhysicalDeviceProperties and a four-byte D3D11_SHADER_RESOURCE_VIEW_DESC: a layout believed
 * rather than measured is a layout that is wrong eventually. */
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(daegun_status) == 4, "status must be an int32");
_Static_assert(sizeof(daegun_bytes) == 2 * sizeof(size_t), "bytes view must be two words");
_Static_assert(sizeof(daegun_str) == 2 * sizeof(size_t), "string view must be two words");
_Static_assert(sizeof(daegun_raster_options) == 80, "raster options must be twenty words");
_Static_assert(sizeof(daegun_line_metrics) == 24, "line metrics must be three doubles");
_Static_assert(sizeof(daegun_typographic_metrics) == 52, "typographic metrics must be thirteen ints");
_Static_assert(sizeof(daegun_axis) == sizeof(void *) + 8, "an axis is a pointer and a double");
_Static_assert(sizeof(daegun_pen) == 6 * sizeof(void *), "a pen is six pointers");
_Static_assert(sizeof(daegun_colr_layer) == 8, "a COLR v0 layer is eight bytes");
_Static_assert(sizeof(daegun_glyph_value) == 4, "a glyph-value pair is two uint16");
_Static_assert(sizeof(daegun_aat_entry) == 8, "a state-table entry is four uint16");
_Static_assert(sizeof(daegun_region_axis) == 24, "a region axis is three doubles");
_Static_assert(sizeof(daegun_stroke_style) == 16, "a stroke style is four words");
_Static_assert(sizeof(daegun_glyph_instance) == 80, "a glyph instance is twenty words");
_Static_assert(sizeof(daegun_subpixel_params) == 800, "subpixel params are two hundred words");
_Static_assert(sizeof(daegun_request) == 24, "a request is a float and five ints");
_Static_assert(sizeof(daegun_rect) == 4 * sizeof(size_t), "a rect is four size_t");
_Static_assert(sizeof(daegun_stat_value) == 48, "a STAT value is six words and four doubles");
_Static_assert(sizeof(daegun_axis_value) == 16, "an axis value is a uint16 and a double");
_Static_assert(sizeof(daegun_feature) == 16, "a feature is a tag, a value and a range");
_Static_assert(sizeof(daegun_glyph_slot) == 32, "a glyph slot is four uints and two float pairs");
_Static_assert(sizeof(daegun_layout_options) == 64, "layout options are sixteen words");
_Static_assert(sizeof(daegun_metrics) == 48, "glyph metrics are twelve words");
_Static_assert(sizeof(daegun_os2_info) == 36, "OS/2 info is nine words");
_Static_assert(sizeof(daegun_paint_node) == 96, "a paint node is eight doubles and its tags");
_Static_assert(sizeof(daegun_palette_info) == 8, "palette info is two words");
_Static_assert(sizeof(daegun_policy) == 16, "a policy is four words");
_Static_assert(sizeof(daegun_shape_options) == 80, "shape options are twenty words");

/* Offsets, not only sizes: daegun_glyph_slot is 32 bytes however its six fields are arranged, so a
 * reordering passes every sizeof check and hands a caller the wrong number. These types live in
 * daecore and daerizer, so they can move without anyone editing this file. Mirrored by offset_of!
 * on the Rust side. */
_Static_assert(offsetof(daegun_glyph_slot, band_base) == 0, "glyph slot layout");
_Static_assert(offsetof(daegun_glyph_slot, h_bands) == 4, "glyph slot layout");
_Static_assert(offsetof(daegun_glyph_slot, v_bands) == 8, "glyph slot layout");
_Static_assert(offsetof(daegun_glyph_slot, hull_base) == 12, "glyph slot layout");
_Static_assert(offsetof(daegun_glyph_slot, box_min) == 16, "glyph slot layout");
_Static_assert(offsetof(daegun_glyph_slot, box_max) == 24, "glyph slot layout");

_Static_assert(offsetof(daegun_glyph_instance, glyph_box) == 0, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, tint) == 16, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, offset) == 32, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, em_pixels) == 40, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, scale) == 48, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, band_base) == 52, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, bands_per_axis) == 56, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, hull_base) == 60, "glyph instance layout");
_Static_assert(offsetof(daegun_glyph_instance, inv_scale) == 64, "glyph instance layout");

_Static_assert(offsetof(daegun_subpixel_params, weights) == 0, "subpixel params layout");
_Static_assert(offsetof(daegun_subpixel_params, oversample) == 768, "subpixel params layout");
_Static_assert(offsetof(daegun_subpixel_params, taps) == 776, "subpixel params layout");
_Static_assert(offsetof(daegun_subpixel_params, origin) == 784, "subpixel params layout");
_Static_assert(offsetof(daegun_subpixel_params, channels) == 792, "subpixel params layout");
_Static_assert(offsetof(daegun_subpixel_params, supersample) == 796, "subpixel params layout");

_Static_assert(sizeof(daegun_curve_point) == 8, "a curve point is two floats");
_Static_assert(sizeof(daegun_band) == 8, "a band is two uint32");
_Static_assert(offsetof(daegun_band, curve_count) == 4, "band layout");
_Static_assert(sizeof(daegun_hull_vertex) == 24, "a hull vertex is six floats");
_Static_assert(offsetof(daegun_hull_vertex, dilate) == 8, "hull vertex layout");
_Static_assert(sizeof(daegun_color_slot) == 48, "a color slot is a glyph slot and a tint");
_Static_assert(offsetof(daegun_color_slot, tint) == 32, "color slot layout");
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_H */
