/* daegun – a no_std, zero-dependency OpenType engine, from C.
 *
 * Everything the Rust API can do, C can do. This header is the contract.
 *
 * The C ABI is behind daegun's `capi` feature. Ask for the shape you want:
 *
 *     cargo rustc --release --features capi --crate-type cdylib     # libdaegun.so / .dylib, daegun.dll
 *     cargo rustc --release --features capi --crate-type staticlib  # libdaegun.a / daegun.lib
 *
 * `capi` implies `threading`, which is what makes rule 5 below true.
 *
 * The static library needs the system libraries Rust's standard library links against, as
 * `--print native-static-libs` reports them; the shared library carries its own:
 *
 *     macOS / iOS   cc app.c libdaegun.a
 *     Linux         cc app.c libdaegun.a -lgcc_s -lutil -lrt -lpthread -lm -ldl
 *     Windows       cl app.c daegun.lib kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib
 *
 * THE FIVE RULES. Every function here obeys all of them, so there is nothing to remember per call.
 *
 *   1. A fallible call returns daegun_status. Results come back through out-parameters.
 *   2. Passing NULL where a pointer is required returns DAEGUN_NULL. It is never dereferenced. A
 *      pointer that comes with a count above 0 is required, unless its call says it may be NULL.
 *   3. daegun allocates and daegun frees. Never call free() on a pointer daegun gave you: this
 *      library has its own allocator, so that is undefined behavior rather than a matter of style.
 *   4. A borrowed view is a const pointer plus a count, valid until the handle it came from is
 *      freed or changed by a call that takes it as non-const. Copy it if you need it longer.
 *   5. A handle a call takes as const may be used from several threads at once. A font locks what
 *      it caches, so sharing one is correct but contended, and a thread-per-font arrangement beats it
 *      under load. A handle a call takes as non-const (a path being built, quads being rewound, a
 *      packer, a table map), or one a pen draws into, needs that handle to itself until it returns.
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

/* borrowed views */

/* A run of bytes daegun owns, lent to you. Valid until the handle it came from is freed or changed. */
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

#define DAEGUN_ABI_VERSION ((1u << 16) | (2u << 8) | 0u)

/* The reason the last failure ON THIS THREAD gave, as UTF-8, valid until the next that gives one;
 * empty until one has. DAEGUN_PARSE always gives one, as does DAEGUN_RANGE from the outline, scene,
 * compositing, subpixel, loca and table map calls; others leave the slot as it was. Per-thread, like
 * errno, since handles are shared and a global slot would hand one thread another's failure. */
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
 * NULL for a zero length, or when the allocation fails. This is not an ordinary pointer: never free()
 * it. */
uint8_t *daegun_font_buffer_new(size_t len);

/* Frees a buffer you took but never handed over; `len` must be the length daegun_font_buffer_new was
 * given, not how much you filled. Null is a no-op. Do NOT call this after daegun_font_open_owned –
 * that took ownership, whether it succeeded or not. */
void daegun_font_buffer_free(uint8_t *data, size_t len);

/* Opens a font from a daegun buffer without a copy. The buffer belongs to the font afterwards either
 * way: freed with daegun_font_free, or at once if the bytes did not parse or out is NULL. `len` must
 * be exactly what you passed to daegun_font_buffer_new. */
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

/* Units per em: the grid the outlines are drawn on. Outlines, color scene paths, clip boxes and CFF
 * hints are in these units; advances and metrics are on the 1000-unit em instead. */
daegun_status daegun_font_upm(const daegun_font *font, uint16_t *out);

/* How many faces a .ttc holds. Zero for data that is not a collection. Takes bytes rather than a
 * font, because it answers what a caller asks before deciding which face to open. */
daegun_status daegun_ttc_font_count(const uint8_t *data, size_t len, size_t *out);

/* the cache */

/* The caches, each a ceiling grown into rather than memory reserved up front. Every one of them
 * is dead weight for some caller: a Latin-only program never approaches the cmap index bound, and
 * text that never repeats gains nothing from the shape cache. Defaults are 4 MB outline, 8 MB shape,
 * 64 MB instance, 25 MB index. */

/* Decoded outlines, which the outline calls and the autohinter replay instead of decoding again; the
 * bytecode and CFF hinters read the font. Filled by daegun_font_prewarm and emptied by
 * daegun_font_clear_prewarm. Either stats pointer may be NULL. */
daegun_status daegun_font_set_outline_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_outline_cache_stats(const daegun_font *font, size_t *out_count,
                                              size_t *out_bytes);

/* Shaped runs, keyed by text. Worth lowering where the text never repeats, as in a log or a chat
 * transcript, since nothing cached there is ever read back. Either stats pointer may be NULL. */
daegun_status daegun_font_set_shape_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_clear_shape_cache(const daegun_font *font);
daegun_status daegun_font_shape_cache_stats(const daegun_font *font, size_t *out_count,
                                            size_t *out_bytes);

/* Instanced fonts, the largest default by far. The newest instance is kept whatever the budget, so one
 * fixed weight runs on a budget of 0; a CFF2 font is instanced even at its default. Reports the bytes
 * held as whole instanced fonts (daegun_font_instance, subsetting) and as instanced tables for the
 * rest; either pointer may be NULL. */
daegun_status daegun_font_set_instance_cache_bytes(const daegun_font *font, size_t bytes);
daegun_status daegun_font_instance_cache_stats(const daegun_font *font, size_t *out_fonts,
                                               size_t *out_tables);

/* Unlike the others this is what is left to spend rather than a ceiling: building a cmap index
 * draws it down and never returns it, so setting it grants a fresh allowance. Sized for CJK. */
daegun_status daegun_font_set_cmap_index_allowance(const daegun_font *font, size_t bytes);
daegun_status daegun_font_cmap_index_allowance(const daegun_font *font, size_t *out_bytes);

/* options */

/* Where the display puts its color samples. Anything unrecognized is grayscale, so a caller on a
 * newer header than the library gets a grayscale layout rather than a refusal. */
#define DAEGUN_LAYOUT_GRAYSCALE          0
#define DAEGUN_LAYOUT_RGB_H              1
#define DAEGUN_LAYOUT_BGR_H              2
#define DAEGUN_LAYOUT_RGB_V              3
#define DAEGUN_LAYOUT_BGR_V              4
#define DAEGUN_LAYOUT_RGB_H_UNFILTERED   5
#define DAEGUN_LAYOUT_BGR_H_UNFILTERED   6
#define DAEGUN_LAYOUT_RGB_V_UNFILTERED   7
#define DAEGUN_LAYOUT_BGR_V_UNFILTERED   8

/* Whether to run the glyph's own TrueType bytecode, and under which interpretation: SUBPIXEL and AUTO
 * as FreeType's v40 interpreter does, CLASSIC as its v35. AUTO autohints a glyph the bytecode does not
 * cover; AUTO_FORCE autohints every glyph. */
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

/* [xmin, ymin, xmax, ymax], on the 1000-unit em. */
daegun_status daegun_font_bbox(const daegun_font *font, daegun_i32_list **out);

/* Tracking at a point size, on the 1000-unit em. */
daegun_status daegun_font_tracking(const daegun_font *font, double ptem, bool horizontal,
                                   double *out);

typedef struct {
    double ascent;
    double descent;
    double line_gap;
} daegun_line_metrics;

daegun_status daegun_font_line_metrics(const daegun_font *font, bool vertical,
                                       daegun_line_metrics *out);

/* What OS/2 says. The has_ fields are Option again: a table too short to hold a field leaves it out,
 * so a face may state no family class or no typographic metrics at all. */
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

/* The five selection predicates. All five are DAEGUN_ABSENT where the face states no OS/2 table.
 * Oblique and typo metrics are bits an OS/2 table before version 4 reserves, so there they are false. */
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
/* Each axis's flags and the name ID of its display name, in the same order. Flag 0x0001 is
 * HIDDEN_AXIS: the font asks for the axis to be kept out of user interfaces. Either may be NULL. */
daegun_status daegun_font_axis_flags(const daegun_font *font, daegun_u16_list **out_flags,
                                     daegun_u16_list **out_name_ids);

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
 * guessing one would be a wrong answer rather than a fallback. So is a height that is not a number. */
daegun_status daegun_font_math_kern(const daegun_font *font, uint16_t gid, int32_t corner,
                                    double height, double *out);

typedef struct daegun_math_construction daegun_math_construction;

daegun_status daegun_font_math_glyph_variants(const daegun_font *font, uint16_t gid, bool vertical,
                                              daegun_math_construction **out);

/* The discrete variants: their glyph ids, with advances at matching indices. out_count is required;
 * either list may be NULL. */
daegun_status daegun_math_construction_variants(const daegun_math_construction *c, size_t *out_count,
                                                const uint16_t **out_gids,
                                                const double **out_advances);

/* The assembly, if there is one. out_part_values holds FOUR doubles per part – start connector, end
 * connector, full advance, and is_extender as 0 or 1 – so part i begins at 4 * i.
 * DAEGUN_ABSENT where the construction carries no assembly. Any out may be NULL. */
daegun_status daegun_math_construction_assembly(const daegun_math_construction *c,
                                                double *out_italics_correction,
                                                size_t *out_part_count,
                                                const uint16_t **out_part_gids,
                                                const double **out_part_values);

void daegun_math_construction_free(daegun_math_construction *c);

/* BASE and STAT */

daegun_status daegun_font_base_is_glyph_free(const daegun_font *font, bool *out);

/* A script's baselines on one axis at a location, in the 1000-unit em, DFLT's when the script has
 * none. Any out may be NULL; a script that names no default baseline writes a NULL handle. */
daegun_status daegun_font_base_info(const daegun_font *font, const char *script_tag, bool vertical,
                                    const daegun_axis *axes, size_t axes_len,
                                    daegun_text **out_default_baseline,
                                    daegun_str_list **out_baseline_tags,
                                    daegun_f64_list **out_baseline_coords);
/* A script's lowest and highest extent on one axis at a location; language and feature may be NULL.
 * A side the font leaves out gives false and 0. Any out may be NULL. */
daegun_status daegun_font_base_extents(const daegun_font *font, const char *script_tag,
                                       const char *language_tag, const char *feature_tag, bool vertical,
                                       const daegun_axis *axes, size_t axes_len,
                                       bool *out_has_min, double *out_min,
                                       bool *out_has_max, double *out_max);

typedef struct daegun_stat daegun_stat;

daegun_status daegun_font_stat_info(const daegun_font *font, daegun_stat **out);
/* Any out may be NULL. */
daegun_status daegun_stat_axes(const daegun_stat *stat, size_t *out_count,
                               daegun_str_list **out_tags, const uint16_t **out_orderings);
/* An axis's display name, BORROWED as daegun_stat_value_name's are. DAEGUN_ABSENT when unnamed. */
daegun_status daegun_stat_axis_name(const daegun_stat *stat, size_t index, daegun_str *out);
/* How many axis values STAT names; daegun_stat_value_at reads them. */
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

/* Every codepoint the cmap maps, with the glyph each reaches, at matching indices.
 * Any out may be NULL. */
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

/* A ligature's carets at a location, x or y by direction, one per caret the font lists. out_present
 * holds a byte each, 0 where a caret cannot be resolved (its value 0). Either out may be NULL. */
daegun_status daegun_font_ligature_carets(const daegun_font *font, uint16_t gid,
                                          const daegun_axis *axes, size_t axes_len, bool vertical,
                                          daegun_f64_list **out_values, daegun_blob **out_present);
daegun_status daegun_font_caret_positions(const daegun_font *font, const char *text,
                                          const daegun_axis *axes, size_t axes_len, bool vertical,
                                          daegun_f64_list **out);

#define DAEGUN_GLYPH_CLASS_BASE       0
#define DAEGUN_GLYPH_CLASS_LIGATURE   1
#define DAEGUN_GLYPH_CLASS_MARK       2
#define DAEGUN_GLYPH_CLASS_COMPONENT  3

daegun_status daegun_font_glyph_class(const daegun_font *font, uint16_t gid, int32_t *out);
daegun_status daegun_font_mark_attachment_class(const daegun_font *font, uint16_t gid, uint16_t *out);
/* A glyph's name from post, else from the CFF charset. DAEGUN_ABSENT where neither names it; a name
 * the font gives as empty, or longer than post's 63 bytes, counts as none. */
daegun_status daegun_font_glyph_name(const daegun_font *font, uint16_t gid, daegun_text **out);
/* Every glyph's name, as daegun_font_glyph_name gives it. Two lists again: out_present says which
 * glyphs have a name, the others' being empty strings. Any out may be NULL. */
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

/* The stored outline – the default instance, whatever location you are working at. DAEGUN_ABSENT
 * for a gid past the font; DAEGUN_PARSE, with the reason in daegun_last_error, for a glyph that does
 * not decode, by which time the pen may already hold part of it. */
daegun_status daegun_font_outline_glyph(const daegun_font *font, uint16_t gid,
                                        const daegun_pen *pen);
/* The same, resolving variation deltas at a location first. */
daegun_status daegun_font_outline_glyph_instanced(const daegun_font *font, uint16_t gid,
                                                  const daegun_axis *axes, size_t axes_len,
                                                  const daegun_pen *pen);

/* out_added, which may be NULL, counts the outlines the cache took: none past the outline budget, and
 * a full cache drops its oldest to take each new one. */
daegun_status daegun_font_prewarm(const daegun_font *font, const uint16_t *gids, size_t gids_len,
                                  const daegun_axis *axes, size_t axes_len, size_t *out_added);
daegun_status daegun_font_clear_prewarm(const daegun_font *font);

/* hinting */

typedef struct daegun_hinted_outline daegun_hinted_outline;

#define DAEGUN_FLAG_ON_CURVE 0x01
/* A cubic control point, in a hinted CFF outline; any other off-curve point is a quadratic control. */
#define DAEGUN_FLAG_CUBIC    0x02

/* DAEGUN_ABSENT where no hinter takes the glyph: DAEGUN_HINT_NONE, a px not finite and above zero or
 * past 65,535 ppem, a glyph too large to hint at that size, or one no hinter reads, including a font
 * whose bytecode fails or turns itself off at that size. Draw it unhinted then, as
 * daegun_font_prepared_outline does. Points are in 26.6 from the glyph's origin, flags hold
 * DAEGUN_FLAG_ON_CURVE and DAEGUN_FLAG_CUBIC only. */
daegun_status daegun_font_hinted_glyph(const daegun_font *font, uint16_t gid, float px,
                                       const daegun_axis *axes, size_t axes_len, int32_t hint_mode,
                                       daegun_hinted_outline **out);
/* Every pointer but out_count may be NULL. */
daegun_status daegun_hinted_outline_points(const daegun_hinted_outline *outline, size_t *out_count,
                                           const int32_t **out_x, const int32_t **out_y,
                                           const uint8_t **out_flags);
/* Where each contour ends, as the index of its last point: contour i runs from the point after
 * ends[i - 1] (point 0 for the first) through ends[i]. */
const size_t *daegun_hinted_outline_contours(const daegun_hinted_outline *outline,
                                             size_t *out_count);
void daegun_hinted_outline_free(daegun_hinted_outline *outline);

typedef struct daegun_cff_hints daegun_cff_hints;

daegun_status daegun_font_cff_hints(const daegun_font *font, uint16_t gid, daegun_cff_hints **out);
/* THREE doubles per stem – is_vertical as 0 or 1, then the edge positions min and max, so a stem's
 * width is max - min. An edge hint keeps the width of -20 or -21 its font gives it, so its max is below
 * its min. out_count receives the number of STEMS, not of doubles. */
const double *daegun_cff_hints_stems(const daegun_cff_hints *hints, size_t *out_count);
/* The hint masks the charstring sets as it draws, in order. Mask i takes effect from point *out_point of
 * the glyph, counted as daegun_font_hinted_glyph gives its points, and holds a bit per stem in
 * daegun_cff_hints_stems order, high bit first; the bits are BORROWED until the hints are freed.
 * DAEGUN_RANGE past the last mask. Any out may be NULL. */
daegun_status daegun_cff_hints_mask_count(const daegun_cff_hints *hints, size_t *out);
daegun_status daegun_cff_hints_mask_at(const daegun_cff_hints *hints, size_t index, size_t *out_point,
                                       const uint8_t **out_bits, size_t *out_len);
void daegun_cff_hints_free(daegun_cff_hints *hints);

/* shaping */

typedef struct daegun_run daegun_run;   /* one shaped run of text */

/* Borrowed views into a run, all valid until it is freed. */
const uint16_t *daegun_run_glyphs(const daegun_run *run, size_t *out_count);
const double   *daegun_run_advances(const daegun_run *run, size_t *out_count);
/* TWO doubles per glyph, x then y, so glyph i is at 2 * i. out_count is the number of DOUBLES. */
const double   *daegun_run_offsets(const daegun_run *run, size_t *out_count);
/* Which character of the input each glyph came from, as a character index rather than a byte offset.
 * Several glyphs may share a cluster, and one glyph may span several characters. */
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

/* Everything the shaper can be told. Zeroing it is the default, as with daegun_outline_options. */
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
    bool        has_seed_script;
    uint16_t    seed_script;     /* the run's script, an id as daegun_text_script_runs gives, kept in
                                    place of the one its text would be scanned for */
} daegun_shape_options;

/* The point size a run is tracked at when its options give none. */
#define DAEGUN_DEFAULT_POINT_SIZE 12

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
/* Each level's lookups to enable and disable, at most 64, highest priority first. lang_sys_tag may be
 * NULL; a language without data takes the default. JstfMax, which nothing applies, is not read. */
daegun_status daegun_font_justification_priorities(const daegun_font *font, const char *script_tag,
                                                   const char *lang_sys_tag,
                                                   daegun_jstf_priorities **out);
daegun_status daegun_jstf_priorities_count(const daegun_jstf_priorities *p, size_t *out);
/* One level, BORROWED – valid until the priorities are freed, and there is nothing to free. */
daegun_status daegun_jstf_priorities_at(const daegun_jstf_priorities *p, size_t index,
                                        const daegun_jstf_mods **out);
void daegun_jstf_priorities_free(daegun_jstf_priorities *p);

/* One of a level's eight lookup lists, BORROWED as the level is. DAEGUN_ABSENT where the level names no
 * such list, which is not a list naming no lookups; DAEGUN_RANGE for a `which` not below. Either out
 * may be NULL. */
#define DAEGUN_JSTF_SHRINKAGE_ENABLE_GSUB   0
#define DAEGUN_JSTF_SHRINKAGE_DISABLE_GSUB  1
#define DAEGUN_JSTF_SHRINKAGE_ENABLE_GPOS   2
#define DAEGUN_JSTF_SHRINKAGE_DISABLE_GPOS  3
#define DAEGUN_JSTF_EXTENSION_ENABLE_GSUB   4
#define DAEGUN_JSTF_EXTENSION_DISABLE_GSUB  5
#define DAEGUN_JSTF_EXTENSION_ENABLE_GPOS   6
#define DAEGUN_JSTF_EXTENSION_DISABLE_GPOS  7
daegun_status daegun_jstf_mods_lookups(const daegun_jstf_mods *mods, int32_t which,
                                       const uint16_t **out, size_t *out_count);

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
/* Any out may be NULL. */
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
/* The run is BORROWED, valid until the set is freed. Do not pass it to daegun_run_free.
 * Any out may be NULL. */
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
/* Any out may be NULL. */
daegun_status daegun_layout_info(const daegun_layout *layout, size_t *out_line_count,
                                 double *out_inline_size, double *out_block_size,
                                 bool *out_has_truncated, size_t *out_truncated);
/* Any out may be NULL. */
daegun_status daegun_layout_line(const daegun_layout *layout, size_t index, size_t *out_run_count,
                                 size_t *out_char_start, size_t *out_char_end, double *out_baseline,
                                 double *out_inline_size, double *out_ascent, double *out_descent,
                                 bool *out_hard_break);
/* The run is BORROWED, valid until the layout is freed. Any out may be NULL. */
daegun_status daegun_layout_run(const daegun_layout *layout, size_t line, size_t index,
                                const daegun_run **out_run, double *out_offset_x,
                                double *out_offset_y, uint8_t *out_level, size_t *out_char_start,
                                size_t *out_char_end, bool *out_upright);
void daegun_layout_free(daegun_layout *layout);

/* text analysis, which needs no font */

/* Character indices, not byte offsets, with the start and the end of the text included. */
daegun_status daegun_text_grapheme_boundaries(const char *text, daegun_u32_list **out);
daegun_status daegun_text_word_boundaries(const char *text, daegun_u32_list **out);
/* Two lists at matching indices: character indices, and one byte each, non-zero for a break the text
 * demands rather than merely allows, as the end of the text always does. Any out may be NULL. */
daegun_status daegun_text_line_break_opportunities(const char *text, daegun_u32_list **out_at,
                                                   daegun_blob **out_mandatory);
/* THREE numbers per run – start, end, and the script's id. Use daegun_script_name to name one; the
 * id is kept because a caller grouping runs compares ids rather than strings. */
daegun_status daegun_text_script_runs(const char *text, daegun_u32_list **out);
daegun_status daegun_script_name(uint16_t script, daegun_text **out);
/* DAEGUN_ABSENT only for the scripts written either way: Old Hungarian, Old Italic, Runic and
 * Tifinagh. Common, Inherited and unknown ids answer false. */
daegun_status daegun_script_is_rtl(uint16_t script, bool *out);

/* Any out may be NULL. */
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
/* Any out may be NULL. */
daegun_status daegun_visual_runs_at(const daegun_visual_runs *runs, size_t index,
                                    uint8_t *out_level, const size_t **out_chars,
                                    size_t *out_chars_count);
void daegun_visual_runs_free(daegun_visual_runs *runs);

/* color */

typedef struct daegun_colr_layers  daegun_colr_layers;
typedef struct daegun_palettes     daegun_palettes;
typedef struct daegun_glyph_bitmap daegun_glyph_bitmap;
typedef struct daegun_paint        daegun_paint;

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
/* Each palette entry's name ID, for labeling the colors a palette sets (CPAL version 1). Two lists:
 * out_name_ids one ID per entry, out_present one byte per entry, 0 where the font names none. Both
 * empty when it labels no entries. Either out may be NULL. */
daegun_status daegun_font_palette_entry_labels(const daegun_font *font, daegun_u16_list **out_name_ids,
                                               daegun_blob **out_present);
const daegun_palette_info *daegun_palettes_data(const daegun_palettes *p, size_t *out_count);
void daegun_palettes_free(daegun_palettes *p);

/* An embedded bitmap from the strike nearest target_ppem that holds the glyph: sbix, CBDT, then EBDT.
 * Color images come as PNG BYTES, as the face stores them, since daegun does not decode images;
 * monochrome and grayscale strikes come decoded, one byte of coverage a pixel. */
daegun_status daegun_font_glyph_bitmap(const daegun_font *font, uint16_t gid, uint16_t target_ppem,
                                       daegun_glyph_bitmap **out);
/* Where the image goes: its left and top edges in pixels at *out_ppem from the glyph origin, y up,
 * for every format alike. out_mirrored is true for an image to draw mirrored left to right, Apple's
 * sbix 'flip'. Any out may be NULL. */
daegun_status daegun_glyph_bitmap_placement(const daegun_glyph_bitmap *b, uint16_t *out_ppem,
                                            int16_t *out_left, int16_t *out_top, bool *out_mirrored);
/* The PNG bytes, BORROWED until b is freed. NULL for a coverage image, or when b or out_len is NULL. */
const uint8_t *daegun_glyph_bitmap_png(const daegun_glyph_bitmap *b, size_t *out_len);
/* The coverage, BORROWED until b is freed: *out_width bytes a row, *out_height rows, top to bottom,
 * 0 none to 255 full. NULL for a PNG, or when any argument is NULL. */
const uint8_t *daegun_glyph_bitmap_coverage(const daegun_glyph_bitmap *b, uint16_t *out_width,
                                            uint16_t *out_height);
void daegun_glyph_bitmap_free(daegun_glyph_bitmap *b);

/* COLR v1 */

/* COLR v1 is a TREE – fourteen variants, ten holding children – and it arrives flattened into an
 * array where children are indices. Node 0 is the root.
 *
 * EVERY variant uses child_start and child_count, so walking children needs no knowledge of which
 * variant you are looking at: Layers has as many as it has, Glyph, ColrGlyph and the six transforming
 * variants have one, Composite has TWO – source first, backdrop second – and the four leaves have none.
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
 * out_colors holds FOUR bytes per stop, so stop i is at 4 * i. Any out may be NULL. */
daegun_status daegun_paint_stops(const daegun_paint *p, size_t *out_count,
                                 const double **out_offsets, const uint8_t **out_colors);
/* A byte per stop at the same indices, 1 where the stop takes YOUR text color, its four color bytes
 * then meaningless, as a node's is_foreground says for a solid. */
const uint8_t *daegun_paint_stops_foreground(const daegun_paint *p, size_t *out_count);
void daegun_paint_free(daegun_paint *p);

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

/* How a path is stroked: the stroke fields of daegun_outline_options, standalone so a path can be
 * stroked on its own. `miter_limit` is read only when `join` is DAEGUN_JOIN_MITER. */
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
/* The bytes the path holds, one per verb and eight per point. The outline cache charges 176 more for
 * each outline it keeps. */
daegun_status daegun_path_cost(const daegun_path *path, size_t *out);
/* DAEGUN_ABSENT when the path has no points. */
daegun_status daegun_path_bounds(const daegun_path *path, double *out_min_x, double *out_min_y,
                                 double *out_max_x, double *out_max_y);

/* The verbs and points, COPIED into your buffers – the one place this ABI copies rather than
 * borrowing. Call with capacity 0 to learn the count, then again to fill; out_x or out_y may be NULL
 * to skip that coordinate. */
daegun_status daegun_path_verbs(const daegun_path *path, uint8_t *out, size_t capacity,
                                size_t *out_count);
daegun_status daegun_path_points(const daegun_path *path, float *out_x, float *out_y,
                                 size_t capacity, size_t *out_count);

/* Replays onto a pen, optionally through a 2x3 transform [a, b, c, d, e, f], or NULL for none. The
 * path is replayed as it was when the call began, so a pen may add to it. Every contour reaches the pen
 * closed, as a fill reads it; daegun_path_verbs gives the path as it was built. */
daegun_status daegun_path_replay(const daegun_path *path, const double *transform,
                                 const daegun_pen *pen);

/* A pen that appends to this path – the other direction through daegun_pen, so a glyph outline can
 * be captured as a value. BORROWS the path: valid until the path is freed. A path may be replayed or
 * stroked into its own pen. */
daegun_status daegun_path_as_pen(daegun_path *path, daegun_pen *out);

/* `tolerance` is how far the flattened curves may sit from the true ones, in the path's units; one
 * not above zero, or NaN, means 0.1. A curve, round join or cap takes at most 256 segments, so a very
 * long one can sit further out. DAEGUN_RANGE, drawing nothing, for a stroke of more than
 * DAEGUN_MAX_FLATTEN_POINTS points or one whose width or points are not finite: a miter's tip
 * overflows long before its width. */
daegun_status daegun_path_stroke(const daegun_path *path, const daegun_stroke_style *style,
                                 float tolerance, const daegun_pen *pen);
/* The same, with the outline's self-intersections resolved into one boundary – what a filler that
 * does not do non-zero winding needs, since a stroke overlaps itself at every join. A stroke of more
 * than 16,384 edges, one that crosses itself into more than 131,072 pieces or past a fixed work
 * budget, or one whose resolved boundary fails its own check, comes back unresolved, as
 * daegun_path_stroke draws it. */
daegun_status daegun_path_stroke_simplified(const daegun_path *path,
                                            const daegun_stroke_style *style,
                                            float tolerance, const daegun_pen *pen);

/* Replays a hinted glyph onto a pen, converting F26Dot6 to pixels on the way. The other half
 * of daegun_font_hinted_glyph: that one grid-fits, this turns the result back into geometry. */
daegun_status daegun_hinted_outline_draw(const daegun_hinted_outline *outline,
                                         const daegun_pen *pen);

/* prepared outlines */

/* What daegun_font_prepared_outline does to an outline. All zeros is the default, the stored outline
 * unhinted; daegun_outline_options_default() stays correct if a default ever stops being zero. A stroke
 * or embolden overlaps itself, which a coverage-summing rasterizer counts twice: see
 * daegun_contours_resolve_overlaps. */
typedef struct {
    int32_t hinting;             /* DAEGUN_HINT_* */
    int32_t has_transform;
    float   transform[6];        /* [a, b, c, d, dx, dy], offsets in font units, applied after hinting
                                    and before a stroke or embolden; the advances ignore it */
    int32_t has_stroke;
    float   stroke_width;        /* font units, scaled with the size and not by the transform */
    int32_t stroke_join;         /* DAEGUN_JOIN_* */
    float   stroke_miter_limit;  /* read only when stroke_join is DAEGUN_JOIN_MITER */
    int32_t stroke_cap;          /* DAEGUN_CAP_* */
    int32_t has_embolden;
    float   embolden;            /* extra stem width in font units. THIS CHANGES THE ADVANCE. Ignored
                                    when has_stroke is set, since a stroke takes its place, and
                                    unless it is finite and above zero. */
    int32_t has_oblique;
    float   oblique;             /* tangent of the angle from vertical; positive leans right. Applied
                                    before the transform, so the glyph leans in its own frame. */
} daegun_outline_options;

daegun_status daegun_outline_options_default(daegun_outline_options *out);

/* Advances in pixels, the embolden widening both; advance_height is 0 for a font without vertical
 * metrics. hinted is 1 when a hinter ran. */
typedef struct {
    float   advance_width;
    float   advance_height;
    int32_t hinted;
} daegun_prepared_glyph;

/* A glyph's outline onto your pen: pixels, y up, origin at the glyph origin. NULL opts means the
 * defaults; out may be NULL. DAEGUN_RANGE, drawing nothing, unless px is finite and above zero; for a
 * transform, oblique or stroke width that is not finite, anything that overflows once scaled, or a
 * stroke or embolden past DAEGUN_MAX_FLATTEN_POINTS points. DAEGUN_ABSENT for no such glyph. */
daegun_status daegun_font_prepared_outline(const daegun_font *font, uint16_t gid, float px,
                                           const daegun_axis *axes, size_t axes_len,
                                           const daegun_outline_options *opts, const daegun_pen *pen,
                                           daegun_prepared_glyph *out);

/* quadratic curves */

/* An outline as quadratic curves in em units, y up, as a curve-evaluating GPU shader takes it: a line
 * becomes a curve with its control at the midpoint, a cubic splits to within about 1/4096 em.
 * DAEGUN_ABSENT for an empty outline, such as a space; DAEGUN_RANGE past DAEGUN_MAX_CURVES_PER_GLYPH
 * curves, or for a coordinate not finite or past FLT_MAX / 16; daegun_last_error() says which. */
typedef struct daegun_quads daegun_quads;

#define DAEGUN_MAX_CURVES_PER_GLYPH 16384

/* Wound so the total signed area is not positive: clockwise, the TrueType convention. A rewound curve
 * keeps its place, so the curves need not run end to start; each stands alone, as a shader takes it. */
daegun_status daegun_font_glyph_quads(const daegun_font *font, uint16_t gid, const daegun_axis *axes,
                                      size_t axes_len, daegun_quads **out);
/* Any path, divided by units_per_em, in the order it was drawn and not rewound. DAEGUN_RANGE too unless
 * units_per_em is finite and above zero, with a finite reciprocal. */
daegun_status daegun_path_quads(const daegun_path *path, float units_per_em, daegun_quads **out);
/* Rewinds in place to the orientation daegun_font_glyph_quads gives, swapping each curve's ends where
 * it keeps its place in the list. */
daegun_status daegun_quads_normalize_winding(daegun_quads *quads);
/* Six floats per curve – x0 y0 cx cy x1 y1 – BORROWED until the quads are freed. out_count is the
 * number of CURVES, not of floats. */
const float *daegun_quads_data(const daegun_quads *quads, size_t *out_count);
void daegun_quads_free(daegun_quads *quads);

/* flattening and overlapping contours */

/* A path as closed polygons. A curve halves until twice the area of its ends-and-midpoint triangle,
 * and of a cubic's quarter-point ones too, is within max_area; a NaN, infinite or non-positive one
 * halves all the way. DAEGUN_ABSENT if no contour of three points is left; DAEGUN_RANGE for a point
 * not finite or past FLT_MAX / 16, or past DAEGUN_MAX_FLATTEN_POINTS points kept or twice that read. */
#define DAEGUN_MAX_FLATTEN_POINTS 1048576
typedef struct daegun_contours daegun_contours;

/* The max_area 1.1.7's CPU rasterizer flattened with, for a path in font units drawn at px pixels
 * per em. DAEGUN_RANGE unless both are finite and above zero. */
daegun_status daegun_flatten_max_area_for(float px, float units_per_em, float *out);
daegun_status daegun_path_flatten(const daegun_path *path, float max_area, daegun_contours **out);
daegun_status daegun_contours_count(const daegun_contours *contours, size_t *out);
/* Two floats per point, x then y, BORROWED until the contours are freed. out_count is the number of
 * POINTS. NULL for an index past the last contour. */
const float *daegun_contours_points(const daegun_contours *contours, size_t index, size_t *out_count);

/* One boundary in place of contours that overlap, for a rasterizer that sums coverage. Points match to
 * 1e-4, so contours belong in font units of an em up to about 2048; at em scale or far larger, more come
 * back DAEGUN_ABSENT, which means keep what you have: no overlap, a union that did not verify, more than
 * DAEGUN_MAX_RESOLVE_EDGES points, or crossings into more than 16 times that many pieces. */
#define DAEGUN_MAX_RESOLVE_EDGES 512
/* The boundary comes back counterclockwise, filled side on the left, whatever the input's winding. */
daegun_status daegun_contours_resolve_overlaps(const daegun_contours *contours, daegun_contours **out);
void daegun_contours_free(daegun_contours *contours);

/* subpixel filters */

/* How a display's color samples are filtered from a glyph's coverage, for a rasterizer doing subpixel
 * antialiasing itself. Coverage is sampled oversample_x by oversample_y times a pixel, and a channel is
 * its weights over a taps_x by taps_y window of samples, starting origin samples from the pixel's
 * first. Grow a glyph's box by pad pixels a side, which covers the window before and past the pixel. */
typedef struct daegun_subpixel_layout daegun_subpixel_layout;

#define DAEGUN_MAX_OVERSAMPLE        4
#define DAEGUN_MAX_SUBPIXEL_TAPS     8
#define DAEGUN_MAX_SUBPIXEL_WEIGHTS 64   /* DAEGUN_MAX_SUBPIXEL_TAPS squared, per channel */

/* A named layout, DAEGUN_LAYOUT_*. Anything unrecognized is grayscale. */
daegun_status daegun_subpixel_layout_new(int32_t layout, daegun_subpixel_layout **out);
/* A filter of your own: `weights` is taps_x * taps_y floats for each of three channels, one channel
 * after another, each row by row: tap (x, y) is at y * taps_x + x. DAEGUN_RANGE for an oversample of
 * zero or past DAEGUN_MAX_OVERSAMPLE, taps of zero or past DAEGUN_MAX_SUBPIXEL_TAPS, or a weight that is
 * not finite. */
daegun_status daegun_subpixel_layout_from_weights(uint8_t oversample_x, uint8_t oversample_y,
                                                  uint8_t taps_x, uint8_t taps_y,
                                                  int8_t origin_x, int8_t origin_y,
                                                  const float *weights, daegun_subpixel_layout **out);
daegun_status daegun_subpixel_layout_oversample(const daegun_subpixel_layout *layout,
                                                uint8_t *out_x, uint8_t *out_y);
daegun_status daegun_subpixel_layout_taps(const daegun_subpixel_layout *layout, uint8_t *out_x,
                                          uint8_t *out_y);
daegun_status daegun_subpixel_layout_origin(const daegun_subpixel_layout *layout, int8_t *out_x,
                                            int8_t *out_y);
daegun_status daegun_subpixel_layout_pad(const daegun_subpixel_layout *layout, size_t *out_x,
                                         size_t *out_y);
/* 1 for grayscale, 3 for a color layout. */
daegun_status daegun_subpixel_layout_channels(const daegun_subpixel_layout *layout, uint8_t *out);
daegun_status daegun_subpixel_layout_is_grayscale(const daegun_subpixel_layout *layout, int32_t *out);
/* taps_x * taps_y weights, BORROWED until the layout is freed. NULL past the last channel. */
const float *daegun_subpixel_layout_weights(const daegun_subpixel_layout *layout, size_t channel,
                                            size_t *out_count);
/* A layout's identity, for use as a cache key: a 64-bit hash of the parameters as given, so two filters
 * share one only by a collision, and one written another way (an extra zero weight, a -0.0) gets a key
 * of its own; the named layouts are written one way each. The first form takes a DAEGUN_LAYOUT_*. */
daegun_status daegun_subpixel_layout_key(int32_t layout, uint64_t *out);
daegun_status daegun_subpixel_layout_cache_key(const daegun_subpixel_layout *layout, uint64_t *out);
void daegun_subpixel_layout_free(daegun_subpixel_layout *layout);

/* color scenes */

/* A color glyph as the ops that draw it, COLR v0 and v1 alike: fill a path, push or pop a clip, push or
 * pop a layer, at most 128 deep. Paint them in order, back to front. Paths are in font units, y up, each
 * placed by its op's transform and held once however many ops name it. A v1 glyph comes clipped to its
 * clip box, and one the spec says not to draw (unbounded, with no box) gives its v0 layers if it has
 * any. DAEGUN_ABSENT for no color description; DAEGUN_RANGE past DAEGUN_MAX_FLATTEN_POINTS points. */
typedef struct daegun_color_scene daegun_color_scene;

#define DAEGUN_SCENE_FILL        0
#define DAEGUN_SCENE_PUSH_CLIP   1
#define DAEGUN_SCENE_POP_CLIP    2
#define DAEGUN_SCENE_PUSH_LAYER  3
#define DAEGUN_SCENE_POP_LAYER   4

#define DAEGUN_SCENE_PAINT_SOLID     0
#define DAEGUN_SCENE_PAINT_GRADIENT  1

#define DAEGUN_FILL_NONZERO 0
#define DAEGUN_FILL_EVENODD 1

/* One op, with the variants flattened into one shape as daegun_paint_node is. A field means something
 * only for the kinds named beside it. `blend` is COLR's composite mode, 0 clear to 27 HSL luminosity,
 * the numbering daegun_paint_node.composite_mode uses. */
typedef struct {
    double   transform[6];  /* FILL: [a, b, c, d, e, f] for x' = a*x + c*y + e */
    int32_t  kind;          /* DAEGUN_SCENE_* */
    int32_t  rule;          /* FILL: DAEGUN_FILL_NONZERO or DAEGUN_FILL_EVENODD */
    int32_t  paint;         /* FILL: DAEGUN_SCENE_PAINT_* */
    int32_t  blend;         /* PUSH_LAYER */
    uint32_t path;          /* FILL: for daegun_color_scene_path */
    uint32_t gradient;      /* FILL with a gradient: an index into daegun_color_scene_gradients */
    uint32_t clip_start;    /* PUSH_CLIP: the union of the clip_count shapes at clip_start */
    uint32_t clip_count;
    float    opacity;       /* PUSH_LAYER */
    uint8_t  rgba[4];       /* FILL with a solid paint, straight alpha */
} daegun_scene_op;

/* One shape of a PUSH_CLIP's union: its own path, placed by its own transform as a FILL's is. */
typedef struct {
    double   transform[6];  /* [a, b, c, d, e, f] for x' = a*x + c*y + e */
    uint32_t path;          /* for daegun_color_scene_path */
    int32_t  rule;          /* DAEGUN_FILL_NONZERO or DAEGUN_FILL_EVENODD */
} daegun_scene_clip;

#define DAEGUN_GRADIENT_LINEAR 0
#define DAEGUN_GRADIENT_RADIAL 1
#define DAEGUN_GRADIENT_SWEEP  2

/* COLR's numbering, the values daegun_paint_node.extend holds. */
#define DAEGUN_EXTEND_PAD     0
#define DAEGUN_EXTEND_REPEAT  1
#define DAEGUN_EXTEND_REFLECT 2

/* `numbers` means whatever `kind` says: LINEAR [0..4) = x0, y0, x1, y1; RADIAL [0..6) = x0, y0, r0,
 * x1, y1, r1; SWEEP [0..4) = cx, cy, start_angle, end_angle in degrees. `transform` takes the gradient's
 * own space into the scene's. Stops come sorted within 0..1, the geometry moved to a color line's first
 * and last stop, so a radius can fall below 0, painting nothing; one stop is a solid, none nothing. */
typedef struct {
    double   transform[6];
    double   numbers[6];
    int32_t  kind;          /* DAEGUN_GRADIENT_* */
    int32_t  extend;        /* DAEGUN_EXTEND_* */
    uint32_t stops_start;
    uint32_t stops_count;
} daegun_scene_gradient;

daegun_status daegun_font_colr_scene(const daegun_font *font, uint16_t gid, const daegun_axis *axes,
                                     size_t axes_len, uint16_t palette_index,
                                     daegun_color_scene **out);
/* The same, with the four-byte RGBA a layer takes when it defers to your text color. NULL means
 * opaque black, which is what daegun_font_colr_scene uses. */
daegun_status daegun_font_colr_scene_with(const daegun_font *font, uint16_t gid,
                                          const daegun_axis *axes, size_t axes_len,
                                          uint16_t palette_index, const uint8_t *foreground,
                                          daegun_color_scene **out);
/* A COLR v1 glyph's clip box at the location, out[4] = x_min, y_min, x_max, y_max in font units:
 * all it draws stays inside, which sizes a surface without walking the scene. DAEGUN_ABSENT for none. */
daegun_status daegun_font_colr_clip_box(const daegun_font *font, uint16_t gid, const daegun_axis *axes,
                                        size_t axes_len, int32_t *out);
/* All BORROWED until the scene is freed. */
const daegun_scene_op *daegun_color_scene_ops(const daegun_color_scene *scene, size_t *out_count);
const daegun_scene_clip *daegun_color_scene_clips(const daegun_color_scene *scene, size_t *out_count);
const daegun_scene_gradient *daegun_color_scene_gradients(const daegun_color_scene *scene,
                                                          size_t *out_count);
/* Every gradient's stops in one run: a gradient's are the stops_count entries at stops_start, and
 * out_colors holds FOUR bytes per stop, so stop i is at 4 * i. */
daegun_status daegun_color_scene_stops(const daegun_color_scene *scene, size_t *out_count,
                                       const double **out_offsets, const uint8_t **out_colors);
/* A COPY of one path, freed with daegun_path_free. DAEGUN_RANGE for an unknown id. */
daegun_status daegun_color_scene_path(const daegun_color_scene *scene, uint32_t path_id,
                                      daegun_path **out);
void daegun_color_scene_free(daegun_color_scene *scene);

/* A gradient made ready to sample, so that linear, radial and sweep colors and the three extend modes
 * come out as COLR defines them, blended as the OpenType spec says: in linear light, with alpha
 * premultiplied. `to_device` takes the scene's space to your pixels; build one ramp per gradient and
 * transform, not one per pixel. DAEGUN_RANGE for an unknown gradient or a transform not finite. */
typedef struct daegun_ramp daegun_ramp;

daegun_status daegun_color_scene_ramp(const daegun_color_scene *scene, uint32_t gradient,
                                      const double to_device[6], daegun_ramp **out);

/* The same, blended as `interpolation` says. DAEGUN_INTERPOLATE_SRGB blends the stored sRGB values
 * with alpha straight, as Chrome and Cairo draw COLR gradients. DAEGUN_RANGE for any other value. */
#define DAEGUN_INTERPOLATE_LINEAR_LIGHT 0
#define DAEGUN_INTERPOLATE_SRGB         1
daegun_status daegun_color_scene_ramp_with(const daegun_color_scene *scene, uint32_t gradient,
                                           const double to_device[6], int32_t interpolation,
                                           daegun_ramp **out);
/* The color at the CENTER of device pixel (x, y), four bytes RGBA, straight alpha. DAEGUN_ABSENT where
 * the gradient paints nothing, as a radial one does outside its cone. */
daegun_status daegun_ramp_sample(const daegun_ramp *ramp, double x, double y, uint8_t out_rgba[4]);
void daegun_ramp_free(daegun_ramp *ramp);

/* COLR's composite of one source pixel over one backdrop pixel, so a rasterizer drawing a scene's
 * layers needs no blend math of its own: `mode` is a layer's blend, 0 to 27. src and backdrop are
 * straight-alpha RGBA in 0..1; out comes back PREMULTIPLIED, its color already scaled by out[3].
 * DAEGUN_RANGE for a mode outside 0 to 27 or a value outside 0..1, where some modes answer NaN. */
daegun_status daegun_composite(int32_t mode, const float src[4], const float backdrop[4], float out[4]);

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

/* Every table of the font pinned to `axes`; a font whose variation tables do not parse comes back as
 * stored, as a static font does.
 *
 * The bytes are COPIED out of the font. The Rust call borrows wherever a table passes through
 * untouched, and that borrow cannot cross into C – nothing would stop the font being freed while
 * the map still pointed into it. Use daegun_font_instance_table when only one table is wanted; it
 * keeps the saving. */
daegun_status daegun_font_instance_tables(const daegun_font *font, const daegun_axis *axes,
                                          size_t axis_count, daegun_table_map **out);

/* One table of the font pinned to `axes`, copied, or as stored as above. DAEGUN_ABSENT for a tag the
 * font lacks. Free with daegun_blob_free. */
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
 * 0 for the short form, 1 for the long one. DAEGUN_RANGE for more than 65,535 glyphs, which no font
 * has. Free with daegun_usize_list_free. */
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
 * ligature, two for contextual substitution. */
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
/* Which variation record applies at `coords`, one 2.14 integer per axis: each -1..=1 coordinate
 * daegun_font_normalized_axes gives, times 16384 and rounded. A record with a condition on an axis
 * past `coord_count` is passed over, as the spec says. DAEGUN_ABSENT when none applies. */
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
 * says a map shorter than the item count means. An empty map is no map: index i is (0, i). */
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

/* the atlas packer, and rules */

/* Everything in this section is a rule the engine owns and a caller would otherwise re-derive. */

/* Where a glyph landed in an atlas. */
typedef struct {
    size_t x;
    size_t y;
    size_t w;
    size_t h;
} daegun_rect;

/* A shelf packer for an atlas of a given size. Your rasterizer gives you pixels and a size; putting a
 * thousand of them into one texture is a packing problem, and this is the solved version of it. */
typedef struct daegun_shelf_packer daegun_shelf_packer;

daegun_shelf_packer *daegun_shelf_packer_new(size_t width, size_t height);
void daegun_shelf_packer_free(daegun_shelf_packer *packer);
/* DAEGUN_ABSENT when the atlas is full – an answer, not a bad argument. Flush and start a new one. */
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

/* The OpenType tags a script maps to, most specific first – Devanagari is dev3, dev2, deva, and a
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
    uint8_t  older_sibling; /* describes other fonts of the family, not this one */
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

/* layouts */

/* Repeated on the Rust side as `const _: () = assert!(size_of::<…>() == …)`. Both must agree or
 * neither builds: a layout believed rather than measured is a layout that is wrong eventually. */
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(daegun_status) == 4, "status must be an int32");
_Static_assert(sizeof(daegun_bytes) == 2 * sizeof(size_t), "bytes view must be two words");
_Static_assert(sizeof(daegun_str) == 2 * sizeof(size_t), "string view must be two words");
_Static_assert(sizeof(daegun_line_metrics) == 24, "line metrics must be three doubles");
_Static_assert(sizeof(daegun_typographic_metrics) == 52, "typographic metrics must be thirteen ints");
_Static_assert(sizeof(daegun_axis) == sizeof(void *) + 8, "an axis is a pointer and a double");
_Static_assert(sizeof(daegun_pen) == 6 * sizeof(void *), "a pen is six pointers");
_Static_assert(sizeof(daegun_colr_layer) == 8, "a COLR v0 layer is eight bytes");
_Static_assert(sizeof(daegun_glyph_value) == 4, "a glyph-value pair is two uint16");
_Static_assert(sizeof(daegun_aat_entry) == 8, "a state-table entry is four uint16");
_Static_assert(sizeof(daegun_region_axis) == 24, "a region axis is three doubles");
_Static_assert(sizeof(daegun_stroke_style) == 16, "a stroke style is four words");
_Static_assert(sizeof(daegun_rect) == 4 * sizeof(size_t), "a rect is four size_t");
_Static_assert(sizeof(daegun_stat_value) == 56, "a STAT value is its kind and flags, four doubles and two uint32");
_Static_assert(sizeof(daegun_axis_value) == 16, "an axis value is a uint16 and a double");
_Static_assert(sizeof(daegun_feature) == 16, "a feature is a tag, a value and a range");
_Static_assert(sizeof(daegun_layout_options) == 64, "layout options are sixteen words");
_Static_assert(sizeof(daegun_os2_info) == 36, "OS/2 info is nine words");
_Static_assert(sizeof(daegun_paint_node) == 96, "a paint node is eight doubles and its tags");
_Static_assert(sizeof(daegun_palette_info) == 8, "palette info is two words");
_Static_assert(sizeof(daegun_shape_options) == 88, "shape options are twenty-two words");
_Static_assert(sizeof(daegun_outline_options) == 68, "outline options must be seventeen words");
_Static_assert(sizeof(daegun_prepared_glyph) == 12, "a prepared glyph is three words");
_Static_assert(sizeof(daegun_scene_op) == 88, "a scene op is six doubles and ten words");
_Static_assert(sizeof(daegun_scene_clip) == 56, "a scene clip is six doubles and two words");
_Static_assert(sizeof(daegun_scene_gradient) == 112, "a scene gradient is twelve doubles, four words");

/* Offsets, not only sizes: a reordering passes every sizeof check and hands a caller the wrong
 * number. Mirrored by offset_of! on the Rust side. */
_Static_assert(offsetof(daegun_outline_options, hinting) == 0, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, has_transform) == 4, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, transform) == 8, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, has_stroke) == 32, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, stroke_width) == 36, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, stroke_join) == 40, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, stroke_miter_limit) == 44, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, stroke_cap) == 48, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, has_embolden) == 52, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, embolden) == 56, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, has_oblique) == 60, "outline options layout");
_Static_assert(offsetof(daegun_outline_options, oblique) == 64, "outline options layout");

_Static_assert(offsetof(daegun_prepared_glyph, advance_width) == 0, "prepared glyph layout");
_Static_assert(offsetof(daegun_prepared_glyph, advance_height) == 4, "prepared glyph layout");
_Static_assert(offsetof(daegun_prepared_glyph, hinted) == 8, "prepared glyph layout");

_Static_assert(offsetof(daegun_scene_op, transform) == 0, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, kind) == 48, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, rule) == 52, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, paint) == 56, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, blend) == 60, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, path) == 64, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, gradient) == 68, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, clip_start) == 72, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, clip_count) == 76, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, opacity) == 80, "scene op layout");
_Static_assert(offsetof(daegun_scene_op, rgba) == 84, "scene op layout");

_Static_assert(offsetof(daegun_scene_clip, transform) == 0, "scene clip layout");
_Static_assert(offsetof(daegun_scene_clip, path) == 48, "scene clip layout");
_Static_assert(offsetof(daegun_scene_clip, rule) == 52, "scene clip layout");

_Static_assert(offsetof(daegun_scene_gradient, transform) == 0, "scene gradient layout");
_Static_assert(offsetof(daegun_scene_gradient, numbers) == 48, "scene gradient layout");
_Static_assert(offsetof(daegun_scene_gradient, kind) == 96, "scene gradient layout");
_Static_assert(offsetof(daegun_scene_gradient, extend) == 100, "scene gradient layout");
_Static_assert(offsetof(daegun_scene_gradient, stops_start) == 104, "scene gradient layout");
_Static_assert(offsetof(daegun_scene_gradient, stops_count) == 108, "scene gradient layout");
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_H */
