#include "daegun.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures = 0;

#define CHECK(cond, ...)                                                                           \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            fprintf(stderr, "  FAIL %s:%d: ", __FILE__, __LINE__);                                 \
            fprintf(stderr, __VA_ARGS__);                                                          \
            fprintf(stderr, "\n");                                                                 \
            failures++;                                                                            \
        }                                                                                          \
    } while (0)

static uint8_t *slurp(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        return NULL;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n <= 0) {
        fclose(f);
        return NULL;
    }
    uint8_t *buf = malloc((size_t)n);
    if (!buf) {
        fclose(f);
        return NULL;
    }
    size_t got = fread(buf, 1, (size_t)n, f);
    fclose(f);
    if (got != (size_t)n) {
        free(buf);
        return NULL;
    }
    *len = got;
    return buf;
}

static void abi_version_agrees(void)
{
    uint32_t got = daegun_abi_version();
    CHECK(got == DAEGUN_ABI_VERSION,
          "library reports ABI %u.%u.%u, this header is %u.%u.%u", (got >> 16) & 0xffu,
          (got >> 8) & 0xffu, got & 0xffu, (DAEGUN_ABI_VERSION >> 16) & 0xffu,
          (DAEGUN_ABI_VERSION >> 8) & 0xffu, DAEGUN_ABI_VERSION & 0xffu);
}

/* Rule 2: NULL is an answer, never a crash. This is the test that a sanitizer cannot write for us –
 * it has to be attempted deliberately, because no correct program does it. */
static void null_is_refused_not_dereferenced(void)
{
    daegun_font *font = NULL;
    uint16_t out = 0;

    CHECK(daegun_font_open(NULL, 0, &font) == DAEGUN_NULL, "null data was not refused");
    CHECK(daegun_font_glyph_id(NULL, 'A', &out) == DAEGUN_NULL, "null font was not refused");
    CHECK(daegun_font_num_glyphs(NULL, &out) == DAEGUN_NULL, "null font was not refused");
    CHECK(daegun_font_upm(NULL, &out) == DAEGUN_NULL, "null font was not refused");

    daegun_font_free(NULL);
}

static void bad_font_data_is_reported(void)
{
    uint8_t junk[64];
    memset(junk, 0xab, sizeof junk);
    daegun_font *font = NULL;

    CHECK(daegun_font_open(junk, sizeof junk, &font) == DAEGUN_PARSE,
          "64 bytes of 0xab parsed as a font");
    CHECK(font == NULL, "a failed open still wrote a handle");

    daegun_str err = daegun_last_error();
    CHECK(err.len > 0, "a failed parse left no message");
    CHECK(err.data != NULL && strlen(err.data) == err.len,
          "the message length disagrees with strlen, so daegun_str's NUL promise is broken");
}

static daegun_font *open_font(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) {
        CHECK(0, "could not read %s", path);
        return NULL;
    }
    daegun_font *font = NULL;
    daegun_status st = daegun_font_open(bytes, len, &font);
    free(bytes);
    if (st != DAEGUN_OK) {
        CHECK(0, "opening %s returned %d: %s", path, st, daegun_last_error().data);
        return NULL;
    }
    return font;
}

static void a_real_font_answers(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) {
        CHECK(0, "could not read %s, so the only test that opens a real font did not run", path);
        return;
    }

    daegun_font *font = NULL;
    daegun_status st = daegun_font_open(bytes, len, &font);
    CHECK(st == DAEGUN_OK, "opening %s returned %d: %s", path, st, daegun_last_error().data);
    if (st != DAEGUN_OK) {
        free(bytes);
        return;
    }

    free(bytes);

    uint16_t upm = 0, glyphs = 0, gid = 0;
    CHECK(daegun_font_upm(font, &upm) == DAEGUN_OK, "upm failed");
    CHECK(upm > 0, "upm is zero");
    CHECK(daegun_font_num_glyphs(font, &glyphs) == DAEGUN_OK, "num_glyphs failed");
    CHECK(glyphs > 1, "a face with %u glyphs is not a face", glyphs);

    CHECK(daegun_font_glyph_id(font, 'A', &gid) == DAEGUN_OK, "no glyph for 'A'");
    CHECK(gid != 0, "'A' mapped to .notdef");
    CHECK(gid < glyphs, "glyph id %u is past the %u the face has", gid, glyphs);

    uint16_t none = 0xffff;
    daegun_status absent = daegun_font_glyph_id(font, 0x10FFFD, &none);
    CHECK(absent == DAEGUN_ABSENT || absent == DAEGUN_OK,
          "an unmapped codepoint returned %d", absent);
    if (absent == DAEGUN_ABSENT) {
        CHECK(none == 0xffff, "an ABSENT answer still wrote the out-parameter");
    }

    /* The four cache budgets. */
    size_t count = 0, cbytes = 0;
    CHECK(daegun_font_set_outline_cache_bytes(font, 64 * 1024) == DAEGUN_OK, "outline budget failed");
    CHECK(daegun_font_outline_cache_stats(font, &count, NULL) == DAEGUN_OK, "outline stats failed");
    CHECK(count == 0, "a fresh font's outline cache held %zu entries", count);

    CHECK(daegun_font_set_shape_cache_bytes(font, 32 * 1024) == DAEGUN_OK, "shape budget failed");
    CHECK(daegun_font_clear_shape_cache(font) == DAEGUN_OK, "clearing shapes failed");
    CHECK(daegun_font_shape_cache_stats(font, &count, &cbytes) == DAEGUN_OK, "shape stats failed");
    CHECK(count == 0 && cbytes == 0, "shape cache held %zu entries after clearing", count);

    size_t fonts = 1, tables = 1;
    CHECK(daegun_font_set_instance_cache_bytes(font, 1024 * 1024) == DAEGUN_OK, "instance budget failed");
    CHECK(daegun_font_instance_cache_stats(font, &fonts, &tables) == DAEGUN_OK, "instance stats failed");

    size_t allowance = 0;
    CHECK(daegun_font_set_cmap_index_allowance(font, 4321) == DAEGUN_OK, "index allowance failed");
    CHECK(daegun_font_cmap_index_allowance(font, &allowance) == DAEGUN_OK, "reading allowance failed");
    CHECK(allowance == 4321, "allowance read back as %zu", allowance);

    /* Every one of them has to refuse a null font rather than dereference it. */
    CHECK(daegun_font_set_outline_cache_bytes(NULL, 0) == DAEGUN_NULL, "null outline budget accepted");
    CHECK(daegun_font_set_shape_cache_bytes(NULL, 0) == DAEGUN_NULL, "null shape budget accepted");
    CHECK(daegun_font_set_instance_cache_bytes(NULL, 0) == DAEGUN_NULL, "null instance budget accepted");
    CHECK(daegun_font_set_cmap_index_allowance(NULL, 0) == DAEGUN_NULL, "null allowance accepted");
    CHECK(daegun_font_cmap_index_allowance(NULL, &allowance) == DAEGUN_NULL, "null allowance read accepted");

    daegun_font_free(font);
}

static void zeroed_options_are_the_defaults(void)
{
    daegun_outline_options zeroed;
    memset(&zeroed, 0, sizeof zeroed);

    daegun_outline_options given;
    memset(&given, 0xcd, sizeof given);
    CHECK(daegun_outline_options_default(&given) == DAEGUN_OK, "defaults failed");

    CHECK(memcmp(&zeroed, &given, sizeof zeroed) == 0,
          "memset(0) and daegun_outline_options_default disagree, so the header's promise is broken");
    CHECK(daegun_outline_options_default(NULL) == DAEGUN_NULL, "null options was not refused");
}

static void ttc_count_answers_for_a_plain_font(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) {
        CHECK(0, "could not read %s", path);
        return;
    }
    size_t n = 12345;
    CHECK(daegun_ttc_font_count(bytes, len, &n) == DAEGUN_OK, "ttc count failed");
    CHECK(n == 0, "a plain .ttf reported %zu faces", n);
    free(bytes);
}

static void metrics_answer_and_free_cleanly(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    int32_t asc = 0, desc = 0;
    CHECK(daegun_font_ascender(font, &asc) == DAEGUN_OK, "ascender failed");
    CHECK(daegun_font_descender(font, &desc) == DAEGUN_OK, "descender failed");
    CHECK(asc > desc, "ascender %d is not above descender %d", asc, desc);

    daegun_text *family = NULL;
    if (daegun_font_family_name(font, &family) == DAEGUN_OK) {
        daegun_str v = { NULL, 0 };
        CHECK(daegun_text_str(family, &v) == DAEGUN_OK, "text_str failed");
        CHECK(v.len > 0 && v.data != NULL, "family name came back empty");
        CHECK(strlen(v.data) == v.len, "family name's NUL disagrees with its length");
        daegun_text_free(family);
    }

    daegun_i32_list *bbox = NULL;
    CHECK(daegun_font_bbox(font, &bbox) == DAEGUN_OK, "bbox failed");
    size_t n = 0;
    const int32_t *box = daegun_i32_list_data(bbox, &n);
    CHECK(n == 4, "a bounding box of %zu numbers", n);
    if (n == 4) {
        CHECK(box[0] < box[2] && box[1] < box[3], "the bounding box is inside out");
    }
    daegun_i32_list_free(bbox);

    daegun_u16_list *ids = NULL;
    daegun_str_list *strings = NULL;
    CHECK(daegun_font_names(font, &ids, &strings) == DAEGUN_OK, "names failed");
    size_t id_count = 0, str_count = 0;
    daegun_u16_list_data(ids, &id_count);
    CHECK(daegun_str_list_count(strings, &str_count) == DAEGUN_OK, "str count failed");
    CHECK(id_count == str_count, "%zu name ids against %zu strings", id_count, str_count);
    if (str_count > 0) {
        daegun_str first = { NULL, 0 };
        CHECK(daegun_str_list_at(strings, 0, &first) == DAEGUN_OK, "str_list_at(0) failed");
        daegun_str past = { NULL, 0 };
        CHECK(daegun_str_list_at(strings, str_count, &past) == DAEGUN_RANGE,
              "reading past the end was not DAEGUN_RANGE");
    }
    daegun_u16_list_free(ids);
    daegun_str_list_free(strings);

    bool italic = false;
    daegun_status st = daegun_font_is_italic(font, &italic);
    CHECK(st == DAEGUN_OK || st == DAEGUN_ABSENT, "is_italic returned %d", st);

    bool variable = false;
    CHECK(daegun_font_is_variable(font, &variable) == DAEGUN_OK, "is_variable failed");
    daegun_str_list *tags = NULL;
    daegun_f64_list *ranges = NULL;
    CHECK(daegun_font_axes(font, &tags, &ranges) == DAEGUN_OK, "axes failed");
    size_t tag_count = 0, range_count = 0;
    CHECK(daegun_str_list_count(tags, &tag_count) == DAEGUN_OK, "axis tag count failed");
    daegun_f64_list_data(ranges, &range_count);
    CHECK(range_count == tag_count * 3, "%zu axes but %zu range numbers", tag_count, range_count);
    CHECK(!variable || tag_count > 0, "a variable face declared no axes");
    daegun_u16_list *flags = NULL, *axis_names = NULL;
    CHECK(daegun_font_axis_flags(font, &flags, &axis_names) == DAEGUN_OK, "axis flags failed");
    size_t flag_count = 0, name_count = 0;
    const uint16_t *flag_data = daegun_u16_list_data(flags, &flag_count);
    const uint16_t *name_data = daegun_u16_list_data(axis_names, &name_count);
    CHECK(flag_count == tag_count && name_count == tag_count, "%zu axes but %zu flags, %zu names",
          tag_count, flag_count, name_count);
    for (size_t i = 0; i < flag_count; i++) {
        CHECK((flag_data[i] & ~1u) == 0, "axis %zu has reserved flag bits 0x%04x", i, flag_data[i]);
        CHECK(name_data[i] > 255, "axis %zu names its display name with ID %u", i, name_data[i]);
    }
    daegun_u16_list_free(flags);
    daegun_u16_list_free(axis_names);
    daegun_str_list_free(tags);
    daegun_f64_list_free(ranges);

    daegun_font_free(font);
}

static void a_subset_is_a_font(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    uint16_t gid = 0;
    if (daegun_font_glyph_id(font, 'A', &gid) != DAEGUN_OK) { daegun_font_free(font); return; }

    uint16_t wanted[2] = { 0, gid };
    daegun_subset *sub = NULL;
    daegun_status st = daegun_font_subset(font, wanted, 2, NULL, 0, &sub);
    CHECK(st == DAEGUN_OK, "subset returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        size_t ttf_len = 0, map_len = 0;
        const uint8_t *ttf = daegun_subset_ttf(sub, &ttf_len);
        daegun_subset_gid_map(sub, &map_len);
        CHECK(ttf_len > 0 && ttf != NULL, "the subset is empty");

        daegun_font *reopened = NULL;
        CHECK(daegun_font_open(ttf, ttf_len, &reopened) == DAEGUN_OK,
              "the subset does not parse as a font: %s", daegun_last_error().data);
        daegun_font_free(reopened);
        daegun_subset_free(sub);
    }
    daegun_font_free(font);
}

static void math_constants_are_indexed(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read the math font at %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    int32_t count = daegun_math_constant_count();
    CHECK(count == 56, "the ABI knows %d math constants", count);

    double v = 0.0;
    daegun_status st = daegun_font_math_constant(font, DAEGUN_MATH_AXIS_HEIGHT, &v);
    CHECK(st == DAEGUN_OK, "axis height returned %d", st);
    CHECK(v != 0.0, "a math font reports an axis height of zero");

    CHECK(daegun_font_math_constant(font, count, &v) == DAEGUN_RANGE,
          "an out-of-range constant index was not refused");

    daegun_font_free(font);
}

struct pen_state {
    int moves, lines, quads, curves, closes;
    const daegun_font *font;
    uint16_t gid;
    int reentered_ok;
};

static void on_move(void *u, float x, float y)  { (void)x; (void)y; ((struct pen_state *)u)->moves++; }
static void on_line(void *u, float x, float y)  { (void)x; (void)y; ((struct pen_state *)u)->lines++; }
static void on_quad(void *u, float a, float b, float x, float y)
{ (void)a; (void)b; (void)x; (void)y; ((struct pen_state *)u)->quads++; }
static void on_curve(void *u, float a, float b, float c, float d, float x, float y)
{ (void)a; (void)b; (void)c; (void)d; (void)x; (void)y; ((struct pen_state *)u)->curves++; }

/* Calls that lock what the drawing call might still hold: the outline cache, read and then written,
 * and the decoder's scratch space through a glyph drawn from inside the callback. */
static void on_close(void *u)
{
    struct pen_state *st = u;
    st->closes++;
    size_t count = 0, bytes = 0;
    daegun_pen inner;
    memset(&inner, 0, sizeof inner);
    if (daegun_font_outline_cache_stats(st->font, &count, &bytes) == DAEGUN_OK
        && daegun_font_set_outline_cache_bytes(st->font, 4u << 20) == DAEGUN_OK
        && daegun_font_outline_glyph(st->font, st->gid, &inner) == DAEGUN_OK) {
        st->reentered_ok++;
    }
}

static void the_pen_draws_and_reentry_is_safe(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    uint16_t gid = 0;
    if (daegun_font_glyph_id(font, 'B', &gid) != DAEGUN_OK) { daegun_font_free(font); return; }

    struct pen_state st;
    memset(&st, 0, sizeof st);
    st.font = font;
    st.gid = gid;

    daegun_pen pen;
    pen.move_to = on_move;
    pen.line_to = on_line;
    pen.quad_to = on_quad;
    pen.curve_to = on_curve;
    pen.close = on_close;
    pen.user = &st;

    CHECK(daegun_font_outline_glyph(font, gid, &pen) == DAEGUN_OK, "outline failed");
    CHECK(st.moves > 0, "'B' produced no move_to");
    CHECK(st.closes >= st.moves, "%d contours opened but %d closed", st.moves, st.closes);
    CHECK(st.lines + st.quads + st.curves > 0, "'B' produced no segments at all");
    CHECK(st.reentered_ok == st.closes, "calling back into daegun from a pen callback did not work");

    /* The same from a prewarmed outline, which replays from the cache instead of decoding. */
    size_t added = 0;
    CHECK(daegun_font_prewarm(font, &gid, 1, NULL, 0, &added) == DAEGUN_OK && added == 1, "prewarm took nothing");
    memset(&st, 0, sizeof st);
    st.font = font;
    st.gid = gid;
    CHECK(daegun_font_outline_glyph(font, gid, &pen) == DAEGUN_OK && st.closes > 0, "the prewarmed outline failed");
    CHECK(st.reentered_ok == st.closes, "calling back into daegun while replaying a prewarmed outline failed");

    daegun_pen empty;
    memset(&empty, 0, sizeof empty);
    CHECK(daegun_font_outline_glyph(font, gid, &empty) == DAEGUN_OK,
          "a pen with no callbacks was not accepted");

    daegun_font_free(font);
}

static void shaping_produces_positioned_glyphs(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    daegun_run *run = NULL;
    daegun_status st = daegun_font_shape(font, "Waffle", NULL, 0, false, &run);
    CHECK(st == DAEGUN_OK, "shape returned %d", st);
    if (st == DAEGUN_OK) {
        size_t g = 0, a = 0, o = 0, c = 0;
        const uint16_t *glyphs = daegun_run_glyphs(run, &g);
        daegun_run_advances(run, &a);
        daegun_run_offsets(run, &o);
        daegun_run_clusters(run, &c);

        CHECK(g > 0, "shaping 'Waffle' produced no glyphs");
        CHECK(a == g, "%zu glyphs but %zu advances", g, a);
        CHECK(c == g, "%zu glyphs but %zu clusters", g, c);
        CHECK(o == g * 2, "%zu glyphs but %zu offset doubles", g, o);

        int all_notdef = 1;
        for (size_t i = 0; i < g; i++) { if (glyphs[i] != 0) all_notdef = 0; }
        CHECK(!all_notdef, "every glyph of 'Waffle' came back .notdef");

        bool complete = false;
        CHECK(daegun_run_complete(run, &complete) == DAEGUN_OK, "complete failed");
        CHECK(complete, "shaping plain Latin did not complete");

        daegun_str shaper = { NULL, 0 };
        CHECK(daegun_run_shaper(run, &shaper) == DAEGUN_OK, "shaper failed");
        CHECK(shaper.len > 0, "the run does not say which shaper ran");
        daegun_run_free(run);
    }

    daegun_run *lig = NULL;
    if (daegun_font_shape(font, "ffl", NULL, 0, false, &lig) == DAEGUN_OK) {
        size_t n = 0;
        daegun_run_glyphs(lig, &n);
        CHECK(n >= 1 && n <= 3, "'ffl' shaped to %zu glyphs", n);
        daegun_run_free(lig);
    }

    double w = 0.0;
    CHECK(daegun_font_measure_width(font, "Waffle", NULL, 0, 16.0, &w) == DAEGUN_OK,
          "measure_width failed");
    CHECK(w > 0.0, "'Waffle' measured %f wide", w);

    daegun_font_free(font);
}

/* Layout, and the borrowed-run rule: a run inside a layout must not be freed separately. */
static void layout_wraps_and_borrows(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    daegun_layout_options opts;
    CHECK(daegun_layout_options_default(&opts) == DAEGUN_OK, "layout defaults failed");
    /* The default must not wrap. If zeroing were the default this would be zero and wrap at every
     * glyph, which is exactly what the header warns about. */
    CHECK(opts.max_inline_size > 1.0e9, "the default max_inline_size is %f, so it would wrap",
          opts.max_inline_size);
    opts.max_inline_size = 6000.0;

    daegun_layout *layout = NULL;
    daegun_status st = daegun_font_layout(font, "the quick brown fox jumps over the lazy dog",
                                          NULL, 0, &opts, &layout);
    CHECK(st == DAEGUN_OK, "layout returned %d", st);
    if (st == DAEGUN_OK) {
        size_t lines = 0;
        double inline_size = 0.0;
        CHECK(daegun_layout_info(layout, &lines, &inline_size, NULL, NULL, NULL) == DAEGUN_OK,
              "layout info failed");
        CHECK(lines > 1, "a 43-character string at 6000 units wrapped to %zu line(s)", lines);
        /* NOT `inline_size <= max`: a single unbreakable run may exceed the measure, which is
         * documented behavior rather than a defect. The property worth asserting is that a wider
         * measure yields fewer lines. */
        CHECK(inline_size > 0.0, "the layout reports no width at all");

        daegun_layout_options wide = opts;
        wide.max_inline_size = 60000.0;
        daegun_layout *one = NULL;
        if (daegun_font_layout(font, "the quick brown fox jumps over the lazy dog", NULL, 0,
                               &wide, &one) == DAEGUN_OK) {
            size_t wide_lines = 0;
            daegun_layout_info(one, &wide_lines, NULL, NULL, NULL, NULL);
            CHECK(wide_lines < lines, "%zu lines at 6000 units but %zu at 60000",
                  lines, wide_lines);
            daegun_layout_free(one);
        }

        size_t runs = 0;
        CHECK(daegun_layout_line(layout, 0, &runs, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
                  == DAEGUN_OK, "layout line failed");
        CHECK(runs > 0, "the first line holds no runs");

        const daegun_run *borrowed = NULL;
        CHECK(daegun_layout_run(layout, 0, 0, &borrowed, NULL, NULL, NULL, NULL, NULL, NULL)
                  == DAEGUN_OK, "layout run failed");
        size_t g = 0;
        daegun_run_glyphs(borrowed, &g);
        CHECK(g > 0, "the first run of the first line holds no glyphs");
        /* Deliberately NOT daegun_run_free(borrowed) – it belongs to the layout, and freeing it
         * would be the double free the sanitizer exists to catch. */

        CHECK(daegun_layout_line(layout, lines, &runs, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
                  == DAEGUN_RANGE, "a line past the end was not DAEGUN_RANGE");
        daegun_layout_free(layout);
    }
    daegun_font_free(font);
}

static void text_analysis_needs_no_font(void)
{
    daegun_u32_list *graphemes = NULL;
    CHECK(daegun_text_grapheme_boundaries("héllo", &graphemes) == DAEGUN_OK, "graphemes failed");
    size_t n = 0;
    daegun_u32_list_data(graphemes, &n);
    CHECK(n > 0, "no grapheme boundaries in 'héllo'");
    daegun_u32_list_free(graphemes);

    daegun_u32_list *runs = NULL;
    CHECK(daegun_text_script_runs("hello", &runs) == DAEGUN_OK, "script runs failed");
    const uint32_t *r = daegun_u32_list_data(runs, &n);
    CHECK(n % 3 == 0, "script runs gave %zu numbers, not a multiple of three", n);
    if (n >= 3) {
        CHECK(r[0] < r[1], "a script run from %u to %u", r[0], r[1]);
        daegun_text *name = NULL;
        CHECK(daegun_script_name((uint16_t)r[2], &name) == DAEGUN_OK, "script name failed");
        daegun_str v = { NULL, 0 };
        daegun_text_str(name, &v);
        CHECK(v.len > 0, "the script has no name");
        daegun_text_free(name);
    }
    daegun_u32_list_free(runs);

    uint8_t base = 0;
    daegun_blob *levels = NULL;
    CHECK(daegun_text_resolve_bidi("hello", -1, &base, &levels, NULL) == DAEGUN_OK,
          "resolve_bidi failed");
    daegun_blob_data(levels, &n);
    CHECK(n > 0, "no bidi levels for 'hello'");
    daegun_blob_free(levels);
}

static void the_paint_graph_is_walkable(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) { CHECK(0, "could not read %s", path); return; }
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK) { free(bytes); CHECK(0, "open failed"); return; }
    free(bytes);

    uint16_t palettes = 0;
    CHECK(daegun_font_palette_count(font, &palettes) == DAEGUN_OK, "palette count failed");

    uint16_t glyphs = 0;
    daegun_font_num_glyphs(font, &glyphs);
    daegun_paint *paint = NULL;
    uint16_t found = 0;
    for (uint16_t g = 1; g < glyphs && g < 400; g++) {
        if (daegun_font_colr_v1_paint(font, g, NULL, 0, 0, &paint) == DAEGUN_OK) { found = g; break; }
    }
    if (!found) { daegun_font_free(font); return; }

    size_t n = 0, kids = 0;
    const daegun_paint_node *nodes = daegun_paint_nodes(paint, &n);
    const uint32_t *children = daegun_paint_children(paint, &kids);
    CHECK(n > 0, "glyph %u has a paint with no nodes", found);

    int visited = 0;
    for (size_t i = 0; i < n; i++) {
        const daegun_paint_node *node = &nodes[i];
        CHECK(node->kind >= 0 && node->kind <= DAEGUN_PAINT_COMPOSITE,
              "node %zu has kind %d", i, node->kind);
        CHECK((size_t)node->child_start + node->child_count <= kids,
              "node %zu names children %u..%u of %zu", i, node->child_start,
              node->child_start + node->child_count, kids);
        for (uint32_t c = 0; c < node->child_count; c++) {
            uint32_t idx = children[node->child_start + c];
            CHECK(idx < n, "node %zu has child index %u of %zu nodes", i, idx, n);
            CHECK(idx != i, "node %zu is its own child", i);
        }
        if (node->kind == DAEGUN_PAINT_COMPOSITE) {
            CHECK(node->child_count == 2, "a composite has %u children, not 2", node->child_count);
        }
        if (node->kind == DAEGUN_PAINT_SOLID) { visited++; }
    }

    size_t stops = 0;
    const double *offsets = NULL;
    const uint8_t *colors = NULL;
    CHECK(daegun_paint_stops(paint, &stops, &offsets, &colors) == DAEGUN_OK, "stops failed");
    for (size_t i = 0; i < n; i++) {
        CHECK((size_t)nodes[i].stops_start + nodes[i].stops_count <= stops,
              "node %zu names stops past the end", i);
    }
    (void)visited; (void)offsets; (void)colors;

    daegun_paint_free(paint);
    daegun_font_free(font);
}

static int32_t key_from_array(size_t index, void *user, uint32_t *out_key)
{
    const uint32_t *keys = (const uint32_t *)user;
    *out_key = keys[index];
    return 1;
}

static int32_t key_that_refuses(size_t index, void *user, uint32_t *out_key)
{
    (void)index; (void)user; (void)out_key;
    return 0;
}

static void the_readers_are_bounds_checked(void)
{
    const uint8_t data[4] = { 0x12, 0x34, 0x56, 0x78 };
    uint16_t u16 = 0;
    int16_t  i16 = 0;
    uint32_t u32 = 0;
    size_t   off = 0;

    CHECK(daegun_read_u16_be(data, sizeof data, 0, &u16) == DAEGUN_OK, "read_u16_be failed");
    CHECK(u16 == 0x1234, "read_u16_be gave %04x, not 1234", u16);
    CHECK(daegun_read_u32_be(data, sizeof data, 0, &u32) == DAEGUN_OK, "read_u32_be failed");
    CHECK(u32 == 0x12345678u, "read_u32_be gave %08x", u32);
    CHECK(daegun_read_u24_be(data, sizeof data, 0, &u32) == DAEGUN_OK, "read_u24_be failed");
    CHECK(u32 == 0x123456u, "read_u24_be gave %06x", u32);
    CHECK(daegun_read_i16_be(data, sizeof data, 2, &i16) == DAEGUN_OK, "read_i16_be failed");
    CHECK(i16 == 0x5678, "read_i16_be gave %04x", (unsigned)i16);
    CHECK(daegun_read_offset24(data, sizeof data, 1, &off) == DAEGUN_OK, "read_offset24 failed");

    CHECK(daegun_read_u16_be(data, sizeof data, 3, &u16) == DAEGUN_RANGE,
          "a two-byte read at offset 3 of a four-byte buffer was allowed");
    CHECK(daegun_read_u32_be(data, sizeof data, 1, &u32) == DAEGUN_RANGE,
          "a four-byte read at offset 1 of a four-byte buffer was allowed");
    CHECK(daegun_read_u16_be(data, sizeof data, (size_t)-1, &u16) == DAEGUN_RANGE,
          "an offset that overflows when two is added was allowed");

    uint8_t buf[8] = { 0 };
    CHECK(daegun_write_u16_be(buf, sizeof buf, 0, 0xbeef) == DAEGUN_OK, "write_u16_be failed");
    CHECK(daegun_read_u16_be(buf, sizeof buf, 0, &u16) == DAEGUN_OK, "read back failed");
    CHECK(u16 == 0xbeef, "wrote beef, read %04x", u16);
    CHECK(daegun_write_i16_be(buf, sizeof buf, 2, -2) == DAEGUN_OK, "write_i16_be failed");
    CHECK(daegun_read_i16_be(buf, sizeof buf, 2, &i16) == DAEGUN_OK, "read back failed");
    CHECK(i16 == -2, "wrote -2, read %d", (int)i16);
    CHECK(daegun_write_u32_be(buf, sizeof buf, 4, 0xdeadbeefu) == DAEGUN_OK, "write_u32_be failed");
    CHECK(daegun_read_u32_be(buf, sizeof buf, 4, &u32) == DAEGUN_OK, "read back failed");
    CHECK(u32 == 0xdeadbeefu, "wrote deadbeef, read %08x", u32);
    CHECK(daegun_write_offset24(buf, sizeof buf, 0, 0x010203) == DAEGUN_OK, "write_offset24 failed");
    CHECK(daegun_write_u16_be(buf, sizeof buf, 7, 1) == DAEGUN_RANGE,
          "a two-byte write at offset 7 of an eight-byte buffer was allowed");
    CHECK(daegun_write_u16_be(NULL, 8, 0, 1) == DAEGUN_NULL, "a null write target was not refused");

    CHECK(daegun_records_fit(0, 4, 2, 8) == 1, "four two-byte records do fit in eight bytes");
    CHECK(daegun_records_fit(1, 4, 2, 8) == 0, "they do not fit starting at one");
    CHECK(daegun_records_fit(0, (size_t)-1, 2, 8) == 0, "an overflowing count was called fitting");

    CHECK(daegun_bytes_window(data, sizeof data, 0, 4) == data, "window did not borrow in place");
    CHECK(daegun_bytes_window(data, sizeof data, 1, 4) == NULL, "window past the end was allowed");
    CHECK(daegun_bytes_window(data, sizeof data, 2, 2) == data + 2, "window offset was wrong");

    uint32_t keys[5] = { 10, 20, 30, 40, 50 };
    size_t index = 999;
    int32_t found = -1;
    CHECK(daegun_search_records(5, 30, key_from_array, keys, &index, &found) == DAEGUN_OK,
          "search_records failed");
    CHECK(found != 0 && index == 2, "30 is at index 2, search said %zu found=%d", index, found);
    CHECK(daegun_search_records(5, 35, key_from_array, keys, &index, &found) == DAEGUN_OK,
          "search_records failed for a missing key");
    CHECK(found == 0 && index == 3, "35 inserts at 3, search said %zu found=%d", index, found);
    CHECK(daegun_search_records(5, 30, key_that_refuses, NULL, &index, &found) == DAEGUN_ABSENT,
          "a key reader that refused was not reported");
    CHECK(daegun_search_records(5, 30, NULL, NULL, &index, &found) == DAEGUN_NULL,
          "a null key reader was not refused");

    CHECK(daegun_ot_round(0.5) == 1, "ot_round(0.5) is 1");
    CHECK(daegun_ot_round(-0.5) == 0, "ot_round(-0.5) is 0, not -1: it is floor(v + 0.5)");
    CHECK(daegun_ot_round(2.49) == 2, "ot_round(2.49) is 2");
}

static void raw_tables_round_trip(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return;
    }

    int32_t has = 0;
    CHECK(daegun_font_has_table(font, "head", &has) == DAEGUN_OK, "has_table failed");
    CHECK(has, "a font with no head table is not a font");
    CHECK(daegun_font_has_table(font, "ZZZZ", &has) == DAEGUN_OK, "has_table failed");
    CHECK(!has, "the font claims a ZZZZ table");

    daegun_bytes head = { NULL, 0 };
    CHECK(daegun_font_table(font, "head", &head) == DAEGUN_OK, "no head table");
    CHECK(head.data != NULL && head.len >= 54, "head is %zu bytes, the spec says 54", head.len);

    uint32_t magic = 0;
    CHECK(daegun_read_u32_be(head.data, head.len, 12, &magic) == DAEGUN_OK, "reading magic failed");
    CHECK(magic == 0x5f0f3cf5u, "head magic is %08x, not 5f0f3cf5", magic);

    daegun_bytes absent = { (const uint8_t *)1, 99 };
    CHECK(daegun_font_table(font, "ZZZZ", &absent) == DAEGUN_ABSENT, "a missing table was not ABSENT");
    CHECK(absent.data == NULL && absent.len == 0, "an ABSENT table left the view untouched");

    daegun_str_list *tags = NULL;
    CHECK(daegun_font_table_tags(font, &tags) == DAEGUN_OK, "table_tags failed");
    size_t n = 0;
    CHECK(daegun_str_list_count(tags, &n) == DAEGUN_OK, "counting tags failed");
    CHECK(n >= 5, "a face with %zu tables is not a face", n);
    int saw_head = 0;
    for (size_t i = 0; i < n; i++) {
        daegun_str tag = { NULL, 0 };
        CHECK(daegun_str_list_at(tags, i, &tag) == DAEGUN_OK, "tag %zu failed", i);
        CHECK(tag.len == 4, "tag %zu is %zu characters, not four", i, tag.len);
        if (memcmp(tag.data, "head", 4) == 0) {
            saw_head = 1;
        }
    }
    CHECK(saw_head, "head is a table the font has and did not list");
    daegun_str_list_free(tags);

    daegun_axis axes[1] = { { "wght", 700.0 } };
    daegun_table_map *map = NULL;
    CHECK(daegun_font_instance_tables(font, axes, 1, &map) == DAEGUN_OK, "instance_tables failed");
    size_t count = 0;
    CHECK(daegun_table_map_count(map, &count) == DAEGUN_OK, "map count failed");
    CHECK(count >= 5, "the instanced map has %zu tables", count);

    daegun_bytes mapped = { NULL, 0 };
    CHECK(daegun_table_map_get(map, "head", &mapped) == DAEGUN_OK, "the map has no head");
    CHECK(mapped.len >= 54, "the mapped head is %zu bytes", mapped.len);
    CHECK(daegun_table_map_get(map, "ZZZZ", &mapped) == DAEGUN_ABSENT, "the map claims ZZZZ");

    daegun_str first = { NULL, 0 };
    CHECK(daegun_table_map_tag_at(map, 0, &first) == DAEGUN_OK, "tag_at 0 failed");
    CHECK(first.len == 4, "a tag is four characters");
    daegun_bytes first_bytes = { NULL, 0 };
    CHECK(daegun_table_map_bytes_at(map, 0, &first_bytes) == DAEGUN_OK, "bytes_at 0 failed");
    CHECK(daegun_table_map_tag_at(map, count, &first) == DAEGUN_RANGE,
          "an index past the end was allowed");
    CHECK(strstr(daegun_last_error().data, "no tag at") != NULL, "tag_at left the reason \"%s\"",
          daegun_last_error().data);
    CHECK(daegun_table_map_bytes_at(map, count, &first_bytes) == DAEGUN_RANGE,
          "bytes past the end were allowed");
    CHECK(strstr(daegun_last_error().data, "no table at") != NULL, "bytes_at left the reason \"%s\"",
          daegun_last_error().data);

    daegun_blob *built = NULL;
    CHECK(daegun_table_map_build(map, &built) == DAEGUN_OK, "building from the map failed");
    size_t built_len = 0;
    const uint8_t *built_data = daegun_blob_data(built, &built_len);
    CHECK(built_len > 1000, "the built font is %zu bytes", built_len);

    daegun_font *rebuilt = NULL;
    CHECK(daegun_font_open(built_data, built_len, &rebuilt) == DAEGUN_OK,
          "the font this ABI built does not open: %s", daegun_last_error().data);
    if (rebuilt) {
        uint16_t a = 0, b = 0;
        daegun_font_num_glyphs(font, &a);
        daegun_font_num_glyphs(rebuilt, &b);
        CHECK(a == b, "the rebuilt font has %u glyphs, the original %u", b, a);
        daegun_font_free(rebuilt);
    }
    daegun_blob_free(built);

    CHECK(daegun_table_map_remove(map, "head") == DAEGUN_OK, "removing head failed");
    CHECK(daegun_table_map_remove(map, "head") == DAEGUN_ABSENT, "removing it twice succeeded");
    size_t after = 0;
    daegun_table_map_count(map, &after);
    CHECK(after == count - 1, "removing one table left %zu of %zu", after, count);
    daegun_table_map_free(map);

    daegun_blob *one = NULL;
    CHECK(daegun_font_instance_table(font, axes, 1, "head", &one) == DAEGUN_OK,
          "instance_table failed");
    size_t one_len = 0;
    daegun_blob_data(one, &one_len);
    CHECK(one_len >= 54, "the instanced head is %zu bytes", one_len);
    daegun_blob_free(one);
    CHECK(daegun_font_instance_table(font, axes, 1, "ZZZZ", &one) == DAEGUN_ABSENT,
          "an absent instanced table was not ABSENT");

    daegun_table_map *empty = daegun_table_map_new();
    daegun_blob *nothing = NULL;
    CHECK(daegun_table_map_build(empty, &nothing) == DAEGUN_RANGE, "an empty map built a font");

    const uint8_t payload[8] = { 1, 2, 3, 4, 5, 6, 7, 8 };
    CHECK(daegun_table_map_set(empty, "TEST", payload, sizeof payload) == DAEGUN_OK, "set failed");
    daegun_bytes back = { NULL, 0 };
    CHECK(daegun_table_map_get(empty, "TEST", &back) == DAEGUN_OK, "get after set failed");
    CHECK(back.len == 8 && memcmp(back.data, payload, 8) == 0, "set and get disagree");
    daegun_table_map_free(empty);

    daegun_font_free(font);
}

typedef struct { int moves, lines, quads, curves, closes; } pen_tally;
static void tally_move(void *u, float x, float y) { (void)x; (void)y; ((pen_tally *)u)->moves++; }
static void tally_line(void *u, float x, float y) { (void)x; (void)y; ((pen_tally *)u)->lines++; }
static void tally_quad(void *u, float a, float b, float c, float d)
{ (void)a; (void)b; (void)c; (void)d; ((pen_tally *)u)->quads++; }
static void tally_curve(void *u, float a, float b, float c, float d, float e, float f)
{ (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; ((pen_tally *)u)->curves++; }
static void tally_close(void *u) { ((pen_tally *)u)->closes++; }

static void paths_build_stroke_and_replay(void)
{
    daegun_path *path = daegun_path_new();
    CHECK(path != NULL, "a new path is null");

    int32_t empty = 0;
    CHECK(daegun_path_is_empty(path, &empty) == DAEGUN_OK, "is_empty failed");
    CHECK(empty, "a new path is not empty");

    CHECK(daegun_path_move_to(path, 0.0f, 0.0f) == DAEGUN_OK, "move_to failed");
    CHECK(daegun_path_line_to(path, 100.0f, 0.0f) == DAEGUN_OK, "line_to failed");
    CHECK(daegun_path_quad_to(path, 150.0f, 50.0f, 100.0f, 100.0f) == DAEGUN_OK, "quad_to failed");
    CHECK(daegun_path_curve_to(path, 60.0f, 130.0f, 20.0f, 130.0f, 0.0f, 100.0f) == DAEGUN_OK,
          "curve_to failed");
    CHECK(daegun_path_close(path) == DAEGUN_OK, "close failed");

    CHECK(daegun_path_is_empty(path, &empty) == DAEGUN_OK, "is_empty failed");
    CHECK(!empty, "a path with five verbs is empty");

    double min_x = 9, min_y = 9, max_x = -9, max_y = -9;
    CHECK(daegun_path_bounds(path, &min_x, &min_y, &max_x, &max_y) == DAEGUN_OK, "bounds failed");
    CHECK(min_x == 0.0 && min_y == 0.0, "bounds start at (%g, %g)", min_x, min_y);
    CHECK(max_x >= 100.0 && max_y >= 100.0, "bounds end at (%g, %g)", max_x, max_y);

    size_t cost = 0;
    CHECK(daegun_path_cost(path, &cost) == DAEGUN_OK, "cost failed");
    CHECK(cost > 0, "a path with four segments costs nothing");

    size_t verb_count = 0, point_count = 0;
    CHECK(daegun_path_verbs(path, NULL, 0, &verb_count) == DAEGUN_OK, "verb count failed");
    CHECK(verb_count == 5, "five verbs went in, %zu came out", verb_count);
    CHECK(daegun_path_points(path, NULL, NULL, 0, &point_count) == DAEGUN_OK, "point count failed");
    CHECK(point_count == 7, "1 + 1 + 2 + 3 points went in, %zu came out", point_count);

    uint8_t verbs[8] = { 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff };
    CHECK(daegun_path_verbs(path, verbs, 8, &verb_count) == DAEGUN_OK, "verbs failed");
    CHECK(verbs[0] == DAEGUN_VERB_MOVE, "the first verb is not a move");
    CHECK(verbs[1] == DAEGUN_VERB_LINE, "the second verb is not a line");
    CHECK(verbs[2] == DAEGUN_VERB_QUAD, "the third verb is not a quad");
    CHECK(verbs[3] == DAEGUN_VERB_CUBIC, "the fourth verb is not a cubic");
    CHECK(verbs[4] == DAEGUN_VERB_CLOSE, "the fifth verb is not a close");
    CHECK(verbs[5] == 0xff, "verbs wrote past the count");

    uint8_t two[2] = { 0xff, 0xff };
    size_t whole = 0;
    CHECK(daegun_path_verbs(path, two, 2, &whole) == DAEGUN_OK, "a short verb buffer failed");
    CHECK(whole == 5, "a short buffer reported %zu rather than the whole count", whole);
    CHECK(two[0] == DAEGUN_VERB_MOVE && two[1] == DAEGUN_VERB_LINE, "a short buffer filled wrong");

    float xs[8] = { 0 }, ys[8] = { 0 };
    CHECK(daegun_path_points(path, xs, ys, 8, &point_count) == DAEGUN_OK, "points failed");
    CHECK(xs[0] == 0.0f && ys[0] == 0.0f, "the first point is (%g, %g)", xs[0], ys[0]);
    CHECK(xs[1] == 100.0f && ys[1] == 0.0f, "the second point is (%g, %g)", xs[1], ys[1]);

    pen_tally seen = { 0, 0, 0, 0, 0 };
    daegun_pen pen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &seen };
    CHECK(daegun_path_replay(path, NULL, &pen) == DAEGUN_OK, "replay failed");
    CHECK(seen.moves == 1 && seen.lines == 1 && seen.quads == 1 && seen.curves == 1
              && seen.closes == 1,
          "replay gave %d/%d/%d/%d/%d rather than one of each",
          seen.moves, seen.lines, seen.quads, seen.curves, seen.closes);

    const double shift[6] = { 1.0, 0.0, 0.0, 1.0, 10.0, 20.0 };
    daegun_path *moved = daegun_path_new();
    daegun_pen into_moved;
    CHECK(daegun_path_as_pen(moved, &into_moved) == DAEGUN_OK, "as_pen failed");
    CHECK(daegun_path_replay(path, shift, &into_moved) == DAEGUN_OK, "transformed replay failed");
    double mx = 9, my = 9, xx = 0, yy = 0;
    CHECK(daegun_path_bounds(moved, &mx, &my, &xx, &yy) == DAEGUN_OK, "bounds of the copy failed");
    CHECK(mx == 10.0 && my == 20.0, "a (10, 20) shift put the corner at (%g, %g)", mx, my);
    daegun_path_free(moved);

    daegun_stroke_style style = { 10.0f, DAEGUN_CAP_ROUND, DAEGUN_JOIN_ROUND, 4.0f };
    pen_tally stroked = { 0, 0, 0, 0, 0 };
    daegun_pen spen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &stroked };
    CHECK(daegun_path_stroke(path, &style, 0.25f, &spen) == DAEGUN_OK, "stroke failed");
    CHECK(stroked.moves > 0 && stroked.lines > 0, "stroking drew nothing");

    pen_tally simple = { 0, 0, 0, 0, 0 };
    daegun_pen sipen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &simple };
    CHECK(daegun_path_stroke_simplified(path, &style, 0.25f, &sipen) == DAEGUN_OK,
          "stroke_simplified failed");
    CHECK(simple.moves > 0, "simplified stroking drew nothing");

    CHECK(daegun_path_stroke(NULL, &style, 0.25f, &spen) == DAEGUN_NULL, "a null path was stroked");
    CHECK(daegun_path_stroke(path, NULL, 0.25f, &spen) == DAEGUN_NULL, "a null style was accepted");

    /* Each turn of this zigzag, stroked wide and round, takes 256 points: 20,000 take five million. */
    daegun_path *zigzag = daegun_path_new();
    daegun_path_move_to(zigzag, 0.0f, 0.0f);
    for (int i = 1; i <= 20000; i++) daegun_path_line_to(zigzag, (float)i * 0.5f, i % 2 ? 1000.0f : 0.0f);
    daegun_stroke_style wide = { 200.0f, DAEGUN_CAP_BUTT, DAEGUN_JOIN_ROUND, 4.0f };
    pen_tally none = { 0, 0, 0, 0, 0 };
    daegun_pen npen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &none };
    CHECK(daegun_path_stroke(zigzag, &wide, 0.001f, &npen) == DAEGUN_RANGE, "a stroke past the cap was drawn");
    CHECK(daegun_path_stroke_simplified(zigzag, &wide, 0.001f, &npen) == DAEGUN_RANGE,
          "a simplified stroke past the cap was drawn");
    CHECK(none.moves == 0 && none.lines == 0, "a refused stroke drew %d lines", none.lines);
    daegun_path_free(zigzag);

    daegun_path_free(path);
    daegun_path_free(NULL);
}

static void glyph_quads_come_back_as_floats(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    uint16_t gid = 0, space = 0;
    CHECK(daegun_font_glyph_id(font, 'B', &gid) == DAEGUN_OK, "no glyph for 'B'");
    daegun_quads *quads = NULL;
    daegun_status st = daegun_font_glyph_quads(font, gid, NULL, 0, &quads);
    CHECK(st == DAEGUN_OK, "glyph_quads returned %d: %s", st, daegun_last_error().data);
    if (quads) {
        size_t n = 0;
        const float *f = daegun_quads_data(quads, &n);
        CHECK(f != NULL && n > 0, "'B' came back as %zu curves", n);
        size_t bad = 0;
        for (size_t i = 0; f && i < n * 6; i++) {
            if (!isfinite(f[i]) || f[i] < -4.0f || f[i] > 4.0f) bad++;
        }
        CHECK(bad == 0, "%zu of the %zu floats are not finite em coordinates", bad, n * 6);
        daegun_quads_free(quads);
        quads = NULL;
    }

    CHECK(daegun_font_glyph_id(font, ' ', &space) == DAEGUN_OK, "no glyph for the space");
    CHECK(daegun_font_glyph_quads(font, space, NULL, 0, &quads) == DAEGUN_ABSENT,
          "the space glyph came back with curves");
    CHECK(quads == NULL, "an ABSENT answer still wrote the out-parameter");

    /* A square drawn counterclockwise in y-up, so rewinding has to reverse every curve. */
    daegun_path *square = daegun_path_new();
    daegun_path_move_to(square, 0.0f, 0.0f);
    daegun_path_line_to(square, 100.0f, 0.0f);
    daegun_path_line_to(square, 100.0f, 100.0f);
    daegun_path_line_to(square, 0.0f, 100.0f);
    daegun_path_close(square);
    CHECK(daegun_path_quads(square, 1000.0f, &quads) == DAEGUN_OK, "path_quads failed");
    if (quads) {
        size_t n = 0;
        const float *f = daegun_quads_data(quads, &n);
        CHECK(n == 4, "a closed square came back as %zu curves", n);
        CHECK(f && f[0] == 0.0f && f[2] == 0.05f && f[4] == 0.1f,
              "the first edge is not (0,0)-(0.1,0) in em");
        CHECK(daegun_quads_normalize_winding(quads) == DAEGUN_OK, "normalize_winding failed");
        f = daegun_quads_data(quads, &n);
        CHECK(f && f[0] == 0.1f && f[4] == 0.0f, "a counterclockwise square was not rewound");
        daegun_quads_free(quads);
        quads = NULL;
    }
    daegun_path *empty = daegun_path_new();
    CHECK(daegun_path_quads(empty, 1000.0f, &quads) == DAEGUN_ABSENT, "an empty path came back with curves");
    daegun_path_free(empty);

    size_t n = 0;
    CHECK(daegun_font_glyph_quads(NULL, gid, NULL, 0, &quads) == DAEGUN_NULL, "a NULL font was accepted");
    CHECK(daegun_font_glyph_quads(font, gid, NULL, 0, NULL) == DAEGUN_NULL, "a NULL out was accepted");
    CHECK(daegun_path_quads(NULL, 1000.0f, &quads) == DAEGUN_NULL, "a NULL path was accepted");
    CHECK(daegun_path_quads(square, 1000.0f, NULL) == DAEGUN_NULL, "a NULL out was accepted");
    CHECK(daegun_path_quads(square, 0.0f, &quads) == DAEGUN_RANGE, "zero units per em was accepted");
    CHECK(daegun_quads_normalize_winding(NULL) == DAEGUN_NULL, "rewinding NULL was accepted");
    CHECK(daegun_quads_data(NULL, &n) == NULL, "data from NULL quads");
    daegun_quads_free(NULL);

    daegun_path_free(square);
    daegun_font_free(font);
}

static void a_color_scene_is_walkable(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    daegun_color_scene *scene = NULL;
    CHECK(daegun_font_colr_scene(font, 0, NULL, 0, 0, &scene) == DAEGUN_ABSENT, ".notdef has a color scene");
    CHECK(scene == NULL, "an ABSENT answer still wrote the out-parameter");

    /* The first glyph whose scene fills with a gradient, so every part of the struct gets read. */
    uint16_t glyphs = 0;
    daegun_font_num_glyphs(font, &glyphs);
    int found = 0;
    for (uint16_t gid = 1; gid < glyphs && !found; gid++) {
        if (daegun_font_colr_scene(font, gid, NULL, 0, 0, &scene) != DAEGUN_OK) continue;
        size_t nops = 0, nclips = 0, ngrads = 0, nstops = 0;
        const daegun_scene_op *ops = daegun_color_scene_ops(scene, &nops);
        const daegun_scene_clip *clips = daegun_color_scene_clips(scene, &nclips);
        const daegun_scene_gradient *grads = daegun_color_scene_gradients(scene, &ngrads);
        const double *offsets = NULL;
        const uint8_t *colors = NULL;
        CHECK(daegun_color_scene_stops(scene, &nstops, &offsets, &colors) == DAEGUN_OK, "stops failed");
        CHECK(ops != NULL && nops > 0, "gid %u has a scene with no ops", gid);

        int depth = 0, gradient_fill = -1;
        for (size_t i = 0; ops && i < nops; i++) {
            const daegun_scene_op *op = &ops[i];
            if (op->kind == DAEGUN_SCENE_FILL) {
                daegun_path *p = NULL;
                CHECK(daegun_color_scene_path(scene, op->path, &p) == DAEGUN_OK,
                      "fill %zu names no path", i);
                daegun_path_free(p);
                CHECK(op->rule == DAEGUN_FILL_NONZERO || op->rule == DAEGUN_FILL_EVENODD,
                      "rule %d", op->rule);
                if (op->paint == DAEGUN_SCENE_PAINT_GRADIENT) {
                    CHECK(op->gradient < ngrads, "gradient %u of %zu", op->gradient, ngrads);
                    if (gradient_fill < 0) gradient_fill = (int)i;
                }
            } else if (op->kind == DAEGUN_SCENE_PUSH_CLIP) {
                CHECK((size_t)op->clip_start + op->clip_count <= nclips, "clip run past the end");
                depth++;
            } else if (op->kind == DAEGUN_SCENE_PUSH_LAYER) {
                CHECK(op->blend >= 0 && op->blend <= 27, "composite mode %d", op->blend);
                depth++;
            } else {
                depth--;
            }
            CHECK(depth >= 0, "a pop came before its push");
        }
        CHECK(depth == 0, "gid %u: pushes and pops do not balance", gid);
        for (size_t i = 0; clips && i < nclips; i++) {
            daegun_path *p = NULL;
            CHECK(daegun_color_scene_path(scene, clips[i].path, &p) == DAEGUN_OK,
                  "clip %zu names no path", i);
            daegun_path_free(p);
        }

        if (gradient_fill >= 0) {
            found = 1;
            const daegun_scene_gradient *g = &grads[ops[gradient_fill].gradient];
            CHECK(g->kind >= DAEGUN_GRADIENT_LINEAR && g->kind <= DAEGUN_GRADIENT_SWEEP, "kind %d", g->kind);
            CHECK(g->extend >= DAEGUN_EXTEND_PAD && g->extend <= DAEGUN_EXTEND_REFLECT,
                  "extend %d", g->extend);
            CHECK((size_t)g->stops_start + g->stops_count <= nstops, "stop run past the end");
            for (uint32_t s = 1; s < g->stops_count; s++) {
                double a = offsets[g->stops_start + s - 1], b = offsets[g->stops_start + s];
                CHECK(a <= b && a >= 0.0 && b <= 1.0, "stops %f, %f are not sorted into 0..1", a, b);
            }

            /* A 64 px em, y flipped into device rows. */
            double to_device[6] = { 64.0 / 1000.0, 0.0, 0.0, -64.0 / 1000.0, 32.0, 64.0 };
            daegun_ramp *ramp = NULL;
            CHECK(daegun_color_scene_ramp(scene, ops[gradient_fill].gradient, to_device, &ramp) == DAEGUN_OK,
                  "ramp failed");
            int painted = 0;
            for (int y = 0; y < 64 && ramp; y += 4) {
                for (int x = 0; x < 64; x += 4) {
                    uint8_t c[4];
                    daegun_status st = daegun_ramp_sample(ramp, x, y, c);
                    CHECK(st == DAEGUN_OK || st == DAEGUN_ABSENT, "sample returned %d", st);
                    painted += st == DAEGUN_OK;
                }
            }
            CHECK(painted > 0, "the gradient painted nothing across a 64 px square");
            uint8_t c[4];
            CHECK(daegun_ramp_sample(NULL, 0, 0, c) == DAEGUN_NULL, "sampling NULL was accepted");
            CHECK(daegun_ramp_sample(ramp, 0, 0, NULL) == DAEGUN_NULL, "a NULL color was accepted");

            /* The default blends as the spec says, and the sRGB blend Chrome draws parts from it. */
            daegun_ramp *spec = NULL, *srgb = NULL, *bad = NULL;
            uint32_t grad = ops[gradient_fill].gradient;
            CHECK(daegun_color_scene_ramp_with(scene, grad, to_device, DAEGUN_INTERPOLATE_LINEAR_LIGHT, &spec)
                      == DAEGUN_OK
                      && daegun_color_scene_ramp_with(scene, grad, to_device, DAEGUN_INTERPOLATE_SRGB, &srgb)
                      == DAEGUN_OK,
                  "ramp_with failed");
            int same = 1, parted = 0;
            for (int y = 0; y < 64 && ramp && spec && srgb; y += 4) {
                for (int x = 0; x < 64; x += 4) {
                    uint8_t d[4] = { 0 }, l[4] = { 0 }, r[4] = { 0 };
                    daegun_status sd = daegun_ramp_sample(ramp, x, y, d);
                    same &= sd == daegun_ramp_sample(spec, x, y, l) && memcmp(d, l, 4) == 0;
                    parted += daegun_ramp_sample(srgb, x, y, r) == DAEGUN_OK && sd == DAEGUN_OK && memcmp(l, r, 4) != 0;
                }
            }
            CHECK(same, "the default ramp is not the spec's blend");
            CHECK(parted > 0, "the sRGB blend matched the spec's at every sample");
            CHECK(daegun_color_scene_ramp_with(scene, grad, to_device, 2, &bad) == DAEGUN_RANGE && bad == NULL,
                  "interpolation 2 was accepted");
            CHECK(strstr(daegun_last_error().data, "interpolation") != NULL, "interpolation 2 left the reason \"%s\"",
                  daegun_last_error().data);
            CHECK(daegun_color_scene_ramp_with(scene, grad, to_device, DAEGUN_INTERPOLATE_SRGB, NULL) == DAEGUN_NULL,
                  "a NULL out was accepted");
            daegun_ramp_free(spec);
            daegun_ramp_free(srgb);
            daegun_ramp_free(ramp);

            daegun_ramp *none = NULL;
            CHECK(daegun_color_scene_ramp(scene, (uint32_t)ngrads, to_device, &none) == DAEGUN_RANGE,
                  "a gradient past the end was accepted");
            daegun_path *p = NULL;
            CHECK(daegun_color_scene_path(scene, 0xffffffffu, &p) == DAEGUN_RANGE,
                  "an unknown path id was accepted");
        }
        daegun_color_scene_free(scene);
        scene = NULL;
    }
    CHECK(found, "no glyph in %s fills with a gradient", path);

    /* gid 154 has a layer that defers to the text color, so naming one has to reach a fill. */
    const uint8_t red[4] = { 255, 0, 0, 255 };
    int took = 0;
    if (daegun_font_colr_scene_with(font, 154, NULL, 0, 0, red, &scene) == DAEGUN_OK) {
        size_t n = 0;
        const daegun_scene_op *ops = daegun_color_scene_ops(scene, &n);
        for (size_t i = 0; ops && i < n; i++) {
            if (ops[i].kind == DAEGUN_SCENE_FILL && ops[i].paint == DAEGUN_SCENE_PAINT_SOLID
                && ops[i].rgba[0] == 255 && ops[i].rgba[1] == 0 && ops[i].rgba[2] == 0) took = 1;
        }
        daegun_color_scene_free(scene);
        scene = NULL;
    }
    CHECK(took, "the text color named in colr_scene_with reached no fill");

    size_t n = 0;
    CHECK(daegun_font_colr_scene(NULL, 1, NULL, 0, 0, &scene) == DAEGUN_NULL, "a NULL font was accepted");
    CHECK(daegun_font_colr_scene(font, 1, NULL, 0, 0, NULL) == DAEGUN_NULL, "a NULL out was accepted");
    CHECK(daegun_color_scene_ops(NULL, &n) == NULL, "ops from a NULL scene");
    daegun_color_scene_free(NULL);
    daegun_ramp_free(NULL);
    daegun_font_free(font);
}

/* Past either end of its color line a PAD gradient holds that end's stop, which a sample inside the
 * gradient never shows. Three lengths out, so pixel centers do not matter. */
static void a_padded_gradient_holds_its_end_colors(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    uint16_t glyphs = 0;
    daegun_font_num_glyphs(font, &glyphs);
    const double identity[6] = { 1.0, 0.0, 0.0, 1.0, 0.0, 0.0 };
    int found = 0;
    for (uint16_t gid = 1; gid < glyphs && !found; gid++) {
        daegun_color_scene *scene = NULL;
        if (daegun_font_colr_scene(font, gid, NULL, 0, 0, &scene) != DAEGUN_OK) continue;
        size_t ngrads = 0, nstops = 0;
        const daegun_scene_gradient *grads = daegun_color_scene_gradients(scene, &ngrads);
        const double *offsets = NULL;
        const uint8_t *colors = NULL;
        daegun_color_scene_stops(scene, &nstops, &offsets, &colors);
        for (size_t i = 0; i < ngrads && !found; i++) {
            const daegun_scene_gradient *g = &grads[i];
            if (g->kind != DAEGUN_GRADIENT_LINEAR || g->extend != DAEGUN_EXTEND_PAD || g->stops_count < 2) continue;
            found = 1;
            daegun_ramp *ramp = NULL;
            CHECK(daegun_color_scene_ramp(scene, (uint32_t)i, identity, &ramp) == DAEGUN_OK, "ramp failed");
            const double ts[2] = { -3.0, 4.0 };
            const uint32_t ends[2] = { g->stops_start, g->stops_start + g->stops_count - 1 };
            for (int e = 0; e < 2 && ramp; e++) {
                const double *n = g->numbers, *m = g->transform;
                double gx = n[0] + ts[e] * (n[2] - n[0]), gy = n[1] + ts[e] * (n[3] - n[1]);
                double x = m[0] * gx + m[2] * gy + m[4], y = m[1] * gx + m[3] * gy + m[5];
                uint8_t c[4] = { 0 };
                CHECK(daegun_ramp_sample(ramp, x - 0.5, y - 0.5, c) == DAEGUN_OK, "glyph %u: nothing past the end", gid);
                const uint8_t *want = &colors[4 * ends[e]];
                for (int k = 0; k < 4; k++) {
                    CHECK(abs((int)c[k] - (int)want[k]) <= 1, "glyph %u, t = %g: channel %d is %u, the end stop %u",
                          gid, ts[e], k, c[k], want[k]);
                }
            }
            daegun_ramp_free(ramp);
        }
        daegun_color_scene_free(scene);
    }
    CHECK(found, "no glyph in %s fills with a padded linear gradient", path);
    daegun_font_free(font);
}

static int rgba_is(const float got[4], float r, float g, float b, float a)
{
    return fabsf(got[0] - r) < 1e-6f && fabsf(got[1] - g) < 1e-6f && fabsf(got[2] - b) < 1e-6f
           && fabsf(got[3] - a) < 1e-6f;
}

/* Expected values worked from the Porter-Duff and blend formulas by hand, not read off the library. */
static void compositing_follows_colr(void)
{
    const float half_red[4] = { 1.0f, 0.0f, 0.0f, 0.5f }, blue[4] = { 0.0f, 0.0f, 1.0f, 1.0f };
    const float clear[4] = { 0.0f, 0.0f, 0.0f, 0.0f }, gray[4] = { 0.5f, 0.5f, 0.5f, 1.0f };
    const float orange[4] = { 1.0f, 0.5f, 0.0f, 1.0f };
    float out[4];

    CHECK(daegun_composite(3, half_red, blue, out) == DAEGUN_OK && rgba_is(out, 0.5f, 0.0f, 0.5f, 1.0f),
          "SRC_OVER of half red on blue gave (%g, %g, %g, %g)", out[0], out[1], out[2], out[3]);
    CHECK(daegun_composite(3, half_red, clear, out) == DAEGUN_OK && rgba_is(out, 0.5f, 0.0f, 0.0f, 0.5f),
          "SRC_OVER onto nothing is not premultiplied: (%g, %g, %g, %g)", out[0], out[1], out[2], out[3]);
    CHECK(daegun_composite(23, orange, gray, out) == DAEGUN_OK && rgba_is(out, 0.5f, 0.25f, 0.0f, 1.0f),
          "MULTIPLY gave (%g, %g, %g, %g)", out[0], out[1], out[2], out[3]);
    CHECK(daegun_composite(0, orange, gray, out) == DAEGUN_OK && rgba_is(out, 0.0f, 0.0f, 0.0f, 0.0f),
          "CLEAR left (%g, %g, %g, %g)", out[0], out[1], out[2], out[3]);
    CHECK(daegun_composite(11, orange, gray, out) == DAEGUN_OK && rgba_is(out, 0.0f, 0.0f, 0.0f, 0.0f),
          "XOR of two opaque pixels left (%g, %g, %g, %g)", out[0], out[1], out[2], out[3]);
    /* The last mode, LUMINOSITY: gray takes orange's luminosity, 0.3 + 0.59 * 0.5. */
    CHECK(daegun_composite(27, orange, gray, out) == DAEGUN_OK && rgba_is(out, 0.595f, 0.595f, 0.595f, 1.0f),
          "LUMINOSITY gave (%g, %g, %g, %g)", out[0], out[1], out[2], out[3]);

    const float bad[4] = { NAN, 0.0f, 0.0f, 1.0f };
    CHECK(daegun_composite(28, orange, gray, out) == DAEGUN_RANGE, "mode 28 was accepted");
    CHECK(strstr(daegun_last_error().data, "0 to 27") != NULL, "mode 28 left the reason \"%s\"",
          daegun_last_error().data);
    CHECK(daegun_composite(-1, orange, gray, out) == DAEGUN_RANGE, "mode -1 was accepted");
    CHECK(daegun_composite(3, bad, gray, out) == DAEGUN_RANGE, "a NaN source was accepted");
    /* Past 0..1 the HSL modes can answer NaN, as HUE does for this one. */
    const float huge[4] = { 1e30f, 0.5f, 1e30f, 0.5f }, over[4] = { 1e30f, 1e30f, 0.25f, 1.0f };
    const float bright[4] = { 1.5f, 0.0f, 0.0f, 1.0f };
    CHECK(daegun_composite(24, huge, over, out) == DAEGUN_RANGE, "a source of 1e30 was accepted");
    CHECK(daegun_composite(3, orange, bright, out) == DAEGUN_RANGE, "a backdrop of 1.5 was accepted");
    CHECK(daegun_composite(3, orange, gray, NULL) == DAEGUN_NULL, "a NULL out was accepted");
}

static void square_into(daegun_path *p, float x, float y, float side)
{
    daegun_path_move_to(p, x, y);
    daegun_path_line_to(p, x + side, y);
    daegun_path_line_to(p, x + side, y + side);
    daegun_path_line_to(p, x, y + side);
    daegun_path_close(p);
}

/* A pen of the caller's own that adds to a path it holds a pointer to, shifted right by 100. */
struct forward { daegun_path *path; };
static void fwd_move(void *u, float x, float y) { daegun_path_move_to(((struct forward *)u)->path, x + 100.0f, y); }
static void fwd_line(void *u, float x, float y) { daegun_path_line_to(((struct forward *)u)->path, x + 100.0f, y); }
static void fwd_quad(void *u, float a, float b, float x, float y)
{ daegun_path_quad_to(((struct forward *)u)->path, a + 100.0f, b, x + 100.0f, y); }
static void fwd_curve(void *u, float a, float b, float c, float d, float x, float y)
{ daegun_path_curve_to(((struct forward *)u)->path, a + 100.0f, b, c + 100.0f, d, x + 100.0f, y); }
static void fwd_close(void *u) { daegun_path_close(((struct forward *)u)->path); }

/* A pen from daegun_path_as_pen appends to its own path, so drawing a path into its own pen grows it
 * while it is read. Ten verbs is past the first allocation, so the growth moves the storage. */
static void a_path_can_draw_into_itself(void)
{
    daegun_path *p = daegun_path_new();
    square_into(p, 0.0f, 0.0f, 10.0f);
    square_into(p, 20.0f, 0.0f, 10.0f);
    size_t before = 0, after = 0;
    CHECK(daegun_path_verbs(p, NULL, 0, &before) == DAEGUN_OK && before == 10, "two squares are %zu verbs", before);

    daegun_pen self;
    CHECK(daegun_path_as_pen(p, &self) == DAEGUN_OK, "as_pen failed");
    const double shift[6] = { 1.0, 0.0, 0.0, 1.0, 100.0, 0.0 };
    CHECK(daegun_path_replay(p, shift, &self) == DAEGUN_OK, "replaying a path into itself failed");
    CHECK(daegun_path_verbs(p, NULL, 0, &after) == DAEGUN_OK && after == 2 * before,
          "replaying %zu verbs into their own path gave %zu", before, after);
    double x0 = 0, y0 = 0, x1 = 0, y1 = 0;
    CHECK(daegun_path_bounds(p, &x0, &y0, &x1, &y1) == DAEGUN_OK && x1 == 130.0,
          "the shifted copy ends at %g, not 130", x1);

    daegun_stroke_style style = { 2.0f, DAEGUN_CAP_BUTT, DAEGUN_JOIN_MITER, 4.0f };
    CHECK(daegun_path_stroke(p, &style, 0.25f, &self) == DAEGUN_OK, "stroking a path into itself failed");
    CHECK(daegun_path_stroke_simplified(p, &style, 0.25f, &self) == DAEGUN_OK,
          "simplified stroking of a path into itself failed");
    CHECK(daegun_path_verbs(p, NULL, 0, &after) == DAEGUN_OK && after > 4 * before,
          "two strokes added only %zu verbs", after - 2 * before);

    /* The same path reached through a pen that is not daegun_path_as_pen's, which no pointer check sees. */
    daegun_path *q = daegun_path_new();
    for (int i = 0; i < 64; i++) square_into(q, (float)i * 20.0f, 0.0f, 10.0f);
    size_t q_before = 0, q_after = 0;
    daegun_path_verbs(q, NULL, 0, &q_before);
    struct forward to_q = { q };
    daegun_pen fwd = { fwd_move, fwd_line, fwd_quad, fwd_curve, fwd_close, &to_q };
    CHECK(daegun_path_replay(q, NULL, &fwd) == DAEGUN_OK, "replaying a path into a pen that grows it failed");
    CHECK(daegun_path_verbs(q, NULL, 0, &q_after) == DAEGUN_OK && q_after == 2 * q_before,
          "replaying %zu verbs into a pen that adds to the same path gave %zu", q_before, q_after);
    daegun_path_free(q);
    daegun_path_free(p);
}

static void paths_flatten_and_resolve(void)
{
    float area = 0.0f;
    CHECK(daegun_flatten_max_area_for(16.0f, 1000.0f, &area) == DAEGUN_OK, "max_area_for failed");
    CHECK(area == 375.0f, "6 * 1000 / 16 came back as %f", area);
    CHECK(daegun_flatten_max_area_for(0.0f, 1000.0f, &area) == DAEGUN_RANGE, "zero px was accepted");

    /* Two squares overlapping in a 49 by 40 corner, where no edge's midpoint lies: their union is one
     * boundary of 8 points. */
    daegun_path *two = daegun_path_new();
    square_into(two, 0.0f, 0.0f, 100.0f);
    square_into(two, 51.0f, 60.0f, 100.0f);
    daegun_contours *flat = NULL, *resolved = NULL;
    CHECK(daegun_path_flatten(two, 1.0f, &flat) == DAEGUN_OK, "flatten failed");
    size_t n = 0, points = 0;
    if (flat) {
        CHECK(daegun_contours_count(flat, &n) == DAEGUN_OK && n == 2,
              "two squares flattened to %zu contours", n);
        const float *p = daegun_contours_points(flat, 1, &points);
        CHECK(p && points == 4 && p[0] == 51.0f && p[1] == 60.0f, "the second square came back wrong");
        CHECK(daegun_contours_points(flat, 2, &points) == NULL, "a contour past the end");
        CHECK(daegun_contours_resolve_overlaps(flat, &resolved) == DAEGUN_OK, "the overlap was left alone");
        if (resolved) {
            CHECK(daegun_contours_count(resolved, &n) == DAEGUN_OK && n == 1,
                  "the union is %zu contours", n);
            daegun_contours_points(resolved, 0, &points);
            CHECK(points == 8, "the union's outline has %zu points, not 8", points);
            daegun_contours_free(resolved);
            resolved = NULL;
        }
        daegun_contours_free(flat);
        flat = NULL;
    }

    daegun_path *one = daegun_path_new();
    square_into(one, 0.0f, 0.0f, 100.0f);
    CHECK(daegun_path_flatten(one, 1.0f, &flat) == DAEGUN_OK, "flatten of one square failed");
    CHECK(daegun_contours_resolve_overlaps(flat, &resolved) == DAEGUN_ABSENT, "a lone square was resolved");
    daegun_contours_free(flat);
    flat = NULL;

    /* A quarter circle as one quadratic: a finer tolerance has to give more points. */
    daegun_path *curve = daegun_path_new();
    daegun_path_move_to(curve, 0.0f, 0.0f);
    daegun_path_quad_to(curve, 100.0f, 0.0f, 100.0f, 100.0f);
    daegun_path_close(curve);
    size_t coarse = 0, fine = 0;
    if (daegun_path_flatten(curve, 100.0f, &flat) == DAEGUN_OK) {
        daegun_contours_points(flat, 0, &coarse);
        daegun_contours_free(flat);
    }
    if (daegun_path_flatten(curve, 1.0f, &flat) == DAEGUN_OK) {
        daegun_contours_points(flat, 0, &fine);
        daegun_contours_free(flat);
    }
    flat = NULL;
    CHECK(coarse >= 3 && fine > coarse, "a finer tolerance gave %zu points against %zu", fine, coarse);

    daegun_path *empty = daegun_path_new();
    CHECK(daegun_path_flatten(empty, 1.0f, &flat) == DAEGUN_ABSENT, "an empty path flattened to something");
    CHECK(daegun_path_flatten(NULL, 1.0f, &flat) == DAEGUN_NULL, "a NULL path was accepted");
    CHECK(daegun_path_flatten(one, 1.0f, NULL) == DAEGUN_NULL, "a NULL out was accepted");
    CHECK(daegun_contours_count(NULL, &n) == DAEGUN_NULL, "counting NULL contours");
    CHECK(daegun_contours_points(NULL, 0, &points) == NULL, "points from NULL contours");
    CHECK(daegun_contours_resolve_overlaps(NULL, &resolved) == DAEGUN_NULL, "resolving NULL contours");
    CHECK(daegun_flatten_max_area_for(16.0f, 1000.0f, NULL) == DAEGUN_NULL, "a NULL out was accepted");
    daegun_contours_free(NULL);

    /* A point that is not finite has no place in a polygon. At max_area 0 each curve is 4,097 points,
     * so 300 of them pass DAEGUN_MAX_FLATTEN_POINTS. */
    daegun_path *bad = daegun_path_new(), *wave = daegun_path_new();
    daegun_path_move_to(bad, 0.0f, 0.0f);
    daegun_path_line_to(bad, NAN, 10.0f);
    daegun_path_line_to(bad, 10.0f, 10.0f);
    daegun_path_close(bad);
    CHECK(daegun_path_flatten(bad, 1.0f, &flat) == DAEGUN_RANGE && !flat, "a NaN point was flattened");
    daegun_path_move_to(wave, 0.0f, 0.0f);
    for (int i = 0; i < 300; i++) {
        float x = (float)i * 10.0f;
        daegun_path_curve_to(wave, x + 3.0f, 50.0f, x + 6.0f, -50.0f, x + 10.0f, 0.0f);
    }
    daegun_path_close(wave);
    CHECK(daegun_path_flatten(wave, 0.0f, &flat) == DAEGUN_RANGE && !flat,
          "a path past DAEGUN_MAX_FLATTEN_POINTS was flattened");
    daegun_path_free(bad);
    daegun_path_free(wave);
    daegun_path_free(empty);
    daegun_path_free(curve);
    daegun_path_free(one);
    daegun_path_free(two);
}

/* The contours drawn and how far they reach in x, every point a callback is handed counted. */
struct reach { int contours; float min_x, max_x; };
static void reach_x(struct reach *r, float x)
{
    r->min_x = x < r->min_x ? x : r->min_x;
    r->max_x = x > r->max_x ? x : r->max_x;
}
static void reach_move(void *u, float x, float y) { (void)y; ((struct reach *)u)->contours++; reach_x(u, x); }
static void reach_line(void *u, float x, float y) { (void)y; reach_x(u, x); }
static void reach_quad(void *u, float cx, float cy, float x, float y) { (void)cy; (void)y; reach_x(u, cx); reach_x(u, x); }

/* The fixture's A rounds a stem horizontally, which Classic does and Subpixel declines, so x tells the
 * two modes apart. A contour's end is the index of its last point. */
static void a_hinted_glyph_is_readable_from_c(void)
{
    daegun_font *font = open_font("assets/test-fonts/test-fixtures/hinted.ttf");
    if (!font) return;
    uint16_t gid = 0;
    CHECK(daegun_font_glyph_id(font, 'A', &gid) == DAEGUN_OK, "the fixture has no A");
    daegun_hinted_outline *sub = NULL, *classic = NULL;
    CHECK(daegun_font_hinted_glyph(font, gid, 16.0f, NULL, 0, DAEGUN_HINT_SUBPIXEL, &sub) == DAEGUN_OK, "subpixel failed");
    CHECK(daegun_font_hinted_glyph(font, gid, 16.0f, NULL, 0, DAEGUN_HINT_CLASSIC, &classic) == DAEGUN_OK, "classic failed");
    size_t n = 0, m = 0, ends_n = 0;
    const int32_t *xs = NULL, *xc = NULL;
    daegun_hinted_outline_points(sub, &n, &xs, NULL, NULL);
    daegun_hinted_outline_points(classic, &m, &xc, NULL, NULL);
    CHECK(n > 0 && n == m, "the modes hinted %zu and %zu points", n, m);
    int differ = 0;
    for (size_t i = 0; i < n && i < m; i++) differ |= xs[i] != xc[i];
    CHECK(differ, "Subpixel and Classic hinted A's x the same, so the mode did not reach the hinter");
    const size_t *ends = daegun_hinted_outline_contours(sub, &ends_n);
    CHECK(ends && ends_n > 0 && ends[ends_n - 1] == n - 1, "the last contour ends at %zu of %zu points",
          ends && ends_n ? ends[ends_n - 1] : 0, n);

    int32_t lo = INT32_MAX, hi = INT32_MIN;
    for (size_t i = 0; i < n; i++) {
        lo = xs[i] < lo ? xs[i] : lo;
        hi = xs[i] > hi ? xs[i] : hi;
    }
    struct reach r = { 0, INFINITY, -INFINITY };
    daegun_pen pen = { reach_move, reach_line, reach_quad, NULL, NULL, &r };
    CHECK(daegun_hinted_outline_draw(sub, &pen) == DAEGUN_OK, "hinted_outline_draw failed");
    CHECK((size_t)r.contours == ends_n && r.min_x == lo / 64.0f && r.max_x == hi / 64.0f,
          "the drawn A spans %g to %g px in %d contours, its points %g to %g in %zu",
          r.min_x, r.max_x, r.contours, lo / 64.0f, hi / 64.0f, ends_n);
    daegun_hinted_outline_free(sub);
    daegun_hinted_outline_free(classic);
    daegun_font_free(font);
}

/* Readers no other check reaches, each against a fixture whose answer fontTools gives too. */
static void the_remaining_readers_answer(void)
{
    daegun_font *inter = open_font("assets/test-fonts/inter/InterVariable.ttf");
    if (inter) {
        uint16_t h = 0;
        daegun_font_glyph_id(inter, 'H', &h);
        daegun_path *plain = daegun_path_new(), *heavy = daegun_path_new();
        daegun_pen to_plain, to_heavy;
        daegun_path_as_pen(plain, &to_plain);
        daegun_path_as_pen(heavy, &to_heavy);
        const daegun_axis wght = {"wght", 900.0};
        CHECK(daegun_font_outline_glyph_instanced(inter, h, NULL, 0, &to_plain) == DAEGUN_OK, "H at no axes");
        CHECK(daegun_font_outline_glyph_instanced(inter, h, &wght, 1, &to_heavy) == DAEGUN_OK, "H at wght 900");
        double x0, y0, x1, y1, hx0, hy0, hx1, hy1;
        daegun_path_bounds(plain, &x0, &y0, &x1, &y1);
        daegun_path_bounds(heavy, &hx0, &hy0, &hx1, &hy1);
        CHECK(hx1 - hx0 > x1 - x0, "H at wght 900 is no wider than the default");
        daegun_path_free(plain);
        daegun_path_free(heavy);

        size_t added = 0, count = 9, bytes = 9;
        CHECK(daegun_font_prewarm(inter, &h, 1, NULL, 0, &added) == DAEGUN_OK && added == 1, "prewarm took %zu", added);
        daegun_font_outline_cache_stats(inter, &count, &bytes);
        CHECK(count == 1 && bytes > 0, "the warmed cache holds %zu outlines", count);
        CHECK(daegun_font_clear_prewarm(inter) == DAEGUN_OK, "clear_prewarm failed");
        daegun_font_outline_cache_stats(inter, &count, &bytes);
        CHECK(count == 0 && bytes == 0, "the cleared cache holds %zu outlines in %zu bytes", count, bytes);
        daegun_font_free(inter);
    }

    daegun_font *stix = open_font("assets/test-fonts/stix-two-math/STIX2Math.otf");
    if (stix) {
        uint16_t h = 0;
        daegun_font_glyph_id(stix, 'H', &h);
        daegun_cff_hints *hints = NULL;
        CHECK(daegun_font_cff_hints(stix, h, &hints) == DAEGUN_OK, "STIX's H has no CFF hints");
        size_t n = 0;
        const double *stems = daegun_cff_hints_stems(hints, &n);
        CHECK(stems && n > 0, "STIX's H declares %zu stems", n);
        for (size_t i = 0; stems && i < n; i++) {
            CHECK((stems[3 * i] == 0.0 || stems[3 * i] == 1.0) && isfinite(stems[3 * i + 1]) && isfinite(stems[3 * i + 2]),
                  "stem %zu is not a direction and two edges", i);
        }
        daegun_cff_hints_free(hints);

        /* E's nine stems take two bytes a mask, and its second mask switches them at point 20. */
        uint16_t e = 0;
        daegun_font_glyph_id(stix, 'E', &e);
        CHECK(daegun_font_cff_hints(stix, e, &hints) == DAEGUN_OK, "STIX's E has no CFF hints");
        daegun_cff_hints_stems(hints, &n);
        size_t masks = 0, last = 0;
        CHECK(daegun_cff_hints_mask_count(hints, &masks) == DAEGUN_OK && masks == 5, "STIX's E sets %zu masks", masks);
        for (size_t i = 0; i < masks; i++) {
            size_t point = 0, bytes = 0;
            const uint8_t *bits = NULL;
            CHECK(daegun_cff_hints_mask_at(hints, i, &point, &bits, &bytes) == DAEGUN_OK && bits
                      && n == 9 && bytes == 2 && point >= last,
                  "mask %zu holds %zu bytes for %zu stems from point %zu", i, bytes, n, point);
            CHECK(i != 1 || (point == 20 && bits[0] == 115 && bits[1] == 128), "E's second mask is not 115 128 at 20");
            last = point;
        }
        CHECK(daegun_cff_hints_mask_at(hints, masks, NULL, NULL, NULL) == DAEGUN_RANGE, "a mask past the last");
        daegun_cff_hints_free(hints);
        daegun_font_free(stix);
    }

    daegun_font *bungee = open_font("assets/test-fonts/bungee-tint/BungeeTint-Regular.ttf");
    if (bungee) {
        uint16_t a = 0;
        daegun_font_glyph_id(bungee, 'A', &a);
        daegun_colr_layers *first = NULL, *second = NULL;
        CHECK(daegun_font_colr_layers_for_palette(bungee, a, 0, &first) == DAEGUN_OK, "A in palette 0");
        CHECK(daegun_font_colr_layers_for_palette(bungee, a, 1, &second) == DAEGUN_OK, "A in palette 1");
        size_t n0 = 0, n1 = 0;
        const daegun_colr_layer *l0 = daegun_colr_layers_data(first, &n0), *l1 = daegun_colr_layers_data(second, &n1);
        CHECK(n0 == 2 && n1 == 2, "A has %zu and %zu layers, not 2", n0, n1);
        if (n0 == 2 && n1 == 2) {
            CHECK(l0[0].r == 201 && l0[0].g == 9 && l0[0].b == 0, "palette 0 colors A's first layer %u %u %u", l0[0].r, l0[0].g, l0[0].b);
            CHECK(l1[0].r == 255 && l1[0].g == 255 && l1[0].b == 255, "palette 1 does not color it white");
        }
        daegun_colr_layers_free(first);
        daegun_colr_layers_free(second);
        daegun_font_free(bungee);
    }

    daegun_font *glyphs = open_font("assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf");
    if (glyphs) {
        daegun_palettes *palettes = NULL;
        CHECK(daegun_font_palette_info(glyphs, &palettes) == DAEGUN_OK, "palette_info failed");
        size_t n = 0;
        const daegun_palette_info *p = daegun_palettes_data(palettes, &n);
        CHECK(n == 3, "%zu palettes, not 3", n);
        if (n == 3) {
            CHECK(!p[0].light_safe && !p[0].dark_safe && p[1].dark_safe && !p[1].light_safe && p[2].light_safe
                      && !p[2].dark_safe && !p[0].has_name_id, "the palette flags are not CPAL's");
        }
        daegun_palettes_free(palettes);
        /* Glyph 148's gradient takes the text color at its middle stop. */
        daegun_paint *paint = NULL;
        size_t stops = 0, nf = 0;
        CHECK(daegun_font_colr_v1_paint(glyphs, 148, NULL, 0, 0, &paint) == DAEGUN_OK
                  && daegun_paint_stops(paint, &stops, NULL, NULL) == DAEGUN_OK,
              "glyph 148 has no paint");
        const uint8_t *fg = daegun_paint_stops_foreground(paint, &nf);
        CHECK(stops == 3 && nf == 3 && !fg[0] && fg[1] && !fg[2], "glyph 148's stops are not plain, text, plain");
        daegun_paint_free(paint);
        daegun_font_free(glyphs);
    }

    daegun_font *emoji = open_font("assets/test-fonts/noto-color-emoji/NotoColorEmoji.ttf");
    if (emoji) {
        uint16_t smile = 0;
        daegun_font_glyph_id(emoji, 0x1F600, &smile);
        daegun_glyph_bitmap *b = NULL;
        CHECK(daegun_font_glyph_bitmap(emoji, smile, 64, &b) == DAEGUN_OK, "no bitmap for U+1F600");
        size_t len = 0;
        uint16_t ppem = 0, w = 0, h = 0;
        int16_t left = 0, top = 0;
        CHECK(daegun_glyph_bitmap_placement(b, &ppem, &left, &top, NULL) == DAEGUN_OK, "no placement");
        const uint8_t *png = daegun_glyph_bitmap_png(b, &len);
        CHECK(png && len > 8 && memcmp(png, "\x89PNG", 4) == 0 && ppem == 109, "not a PNG at 109 ppem");
        CHECK(top == 101 && !daegun_glyph_bitmap_coverage(b, &w, &h), "a CBDT PNG placed wrong, or given coverage");
        daegun_glyph_bitmap_free(b);
        daegun_font_free(emoji);
    }

    size_t len = 0;
    uint8_t *ttc = slurp("assets/test-fonts/test-fixtures/EBGaramond-InterVariable.ttc", &len);
    if (ttc) {
        daegun_font *second = NULL;
        uint16_t n = 0;
        CHECK(daegun_font_open_collection(ttc, len, 1, &second) == DAEGUN_OK, "the collection's second face");
        daegun_font_num_glyphs(second, &n);
        CHECK(n == 2937, "the collection's second face has %u glyphs, not Inter's 2,937", n);
        daegun_font_free(second);
        free(ttc);
    }
}

/* A miter's tip overflows f32 long before the width does; both, like a NaN width, are refused. */
static void a_stroke_that_overflows_is_refused(void)
{
    daegun_path *p = daegun_path_new();
    daegun_path_move_to(p, 0.0f, 0.0f);
    daegun_path_line_to(p, 1000.0f, 0.0f);
    daegun_path_line_to(p, 0.0f, 1.0f);
    daegun_path *sink = daegun_path_new();
    daegun_pen pen;
    daegun_path_as_pen(sink, &pen);
    const float widths[2] = {1e33f, NAN};
    for (int i = 0; i < 2; i++) {
        daegun_stroke_style style = {widths[i], DAEGUN_CAP_BUTT, DAEGUN_JOIN_MITER, 1e30f};
        CHECK(daegun_path_stroke(p, &style, 0.0f, &pen) == DAEGUN_RANGE, "stroke width %g was not RANGE", widths[i]);
        CHECK(daegun_path_stroke_simplified(p, &style, 0.0f, &pen) == DAEGUN_RANGE,
              "simplified stroke width %g was not RANGE", widths[i]);
    }
    size_t n = 1;
    daegun_path_verbs(sink, NULL, 0, &n);
    CHECK(n == 0, "a refused stroke drew %zu verbs", n);
    daegun_path_free(sink);
    daegun_path_free(p);
}

/* Rule 2: a count above 0 says the pointer beside it is required, so NULL with one is DAEGUN_NULL
 * rather than nothing. With a count of 0 these calls take NULL as empty. */
static void null_with_a_count_is_null(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    uint16_t u16 = 0, gid = 0;
    CHECK(daegun_read_u16_be(NULL, 4, 0, &u16) == DAEGUN_NULL, "a NULL buffer of 4 bytes was read");
    CHECK(daegun_read_u16_be(NULL, 0, 0, &u16) == DAEGUN_RANGE, "a read past an empty buffer was not RANGE");
    CHECK(daegun_bytes_window(NULL, 4, 0, 2) == NULL, "a NULL buffer of 4 bytes gave a window");
    CHECK(daegun_coverage_index(NULL, 10, 3, &gid) == DAEGUN_NULL, "a NULL coverage table was searched");

    daegun_table_map *map = daegun_table_map_new();
    CHECK(daegun_table_map_set(map, "abcd", NULL, 5) == DAEGUN_NULL, "a NULL table of 5 bytes was stored");
    CHECK(daegun_table_map_set(map, "abcd", NULL, 0) == DAEGUN_OK, "an empty table was refused");
    daegun_table_map_free(map);

    daegun_path *p = daegun_path_new();
    daegun_path_move_to(p, 0.0f, 0.0f);
    size_t n = 99;
    CHECK(daegun_path_verbs(p, NULL, 10, &n) == DAEGUN_NULL, "NULL verbs with a capacity of 10 were filled");
    CHECK(n == 99, "a refused call wrote its count: %zu", n);
    CHECK(daegun_path_verbs(p, NULL, 0, &n) == DAEGUN_OK && n == 1, "the verb count was not given");
    CHECK(daegun_path_points(p, NULL, NULL, 10, &n) == DAEGUN_OK && n == 1, "skipping both coordinates failed");
    daegun_path_free(p);

    daegun_run *run = NULL;
    CHECK(daegun_font_shape(font, "ab", NULL, 1, false, &run) == DAEGUN_NULL, "NULL axes with a count shaped");
    CHECK(daegun_font_shape_with_features(font, "ab", NULL, 0, false, NULL, NULL, 2, &run) == DAEGUN_NULL,
          "NULL features with a count shaped");
    CHECK(run == NULL, "a refused shape delivered a run");
    CHECK(daegun_font_shape(font, "ab", NULL, 0, false, &run) == DAEGUN_OK, "no axes at all was refused");
    daegun_run_free(run);

    /* A NULL result is DAEGUN_NULL whatever the input, not ABSENT or RANGE when there was nothing to give. */
    uint16_t glyphs = 0;
    daegun_font_num_glyphs(font, &glyphs);
    daegun_path *empty = daegun_path_new();
    CHECK(daegun_font_hinted_glyph(font, glyphs, 16.0f, NULL, 0, DAEGUN_HINT_SUBPIXEL, NULL) == DAEGUN_NULL,
          "a glyph past the end with a NULL out was not DAEGUN_NULL");
    CHECK(daegun_font_colr_layers(font, 5, NULL) == DAEGUN_NULL, "colr layers with a NULL out");
    CHECK(daegun_font_instance_table(font, NULL, 0, "ZZZZ", NULL) == DAEGUN_NULL, "a missing table with a NULL out");
    CHECK(daegun_char_general_category(0xD800, NULL) == DAEGUN_NULL, "a surrogate with a NULL out");
    CHECK(daegun_path_bounds(empty, NULL, NULL, NULL, NULL) == DAEGUN_NULL, "bounds of nothing into NULL");
    daegun_path_free(empty);

    static const uint8_t junk[4] = {0xFF, 0xFF, 0xFF, 0xFF};
    daegun_usize_list *glyph_offsets = NULL;
    CHECK(daegun_font_glyph_bitmap(font, 5, 16, NULL) == DAEGUN_NULL, "a missing bitmap with a NULL out");
    CHECK(daegun_font_colr_v1_paint(font, 5, NULL, 0, 0, NULL) == DAEGUN_NULL, "a missing paint with a NULL out");
    CHECK(daegun_font_cff_hints(font, 5, NULL) == DAEGUN_NULL, "missing CFF hints with a NULL out");
    CHECK(daegun_font_name_string(font, 999, NULL) == DAEGUN_NULL, "a missing name with a NULL out");
    CHECK(daegun_font_cmap_index_allowance(font, NULL) == DAEGUN_NULL, "the cmap allowance into NULL");
    CHECK(daegun_parse_loca(junk, 4, 0, SIZE_MAX, NULL) == DAEGUN_NULL, "too many glyphs with a NULL out");
    CHECK(daegun_parse_loca(NULL, 4, 0, SIZE_MAX, &glyph_offsets) == DAEGUN_NULL, "a NULL loca of 4 bytes was RANGE");
    daegun_path *drawn = daegun_path_new();
    daegun_pen into_drawn;
    daegun_path_as_pen(drawn, &into_drawn);
    CHECK(daegun_font_prepared_outline(font, 5, NAN, NULL, 2, NULL, &into_drawn, NULL) == DAEGUN_NULL,
          "NULL axes with a count were RANGE");
    daegun_path_free(drawn);
    CHECK(daegun_read_u16_be(junk, 4, 100, NULL) == DAEGUN_NULL, "a read past the end into NULL");
    CHECK(daegun_coverage_index(junk, 4, 3, NULL) == DAEGUN_NULL, "a junk coverage table with a NULL out");
    CHECK(daegun_coverage_glyphs(junk, 4, 0, NULL) == DAEGUN_NULL, "junk coverage glyphs with a NULL out");
    CHECK(daegun_aat_state_table_open(junk, 4, 0, 10, NULL) == DAEGUN_NULL, "a junk state table with a NULL out");
    CHECK(daegun_ankr_version(junk, 1, NULL) == DAEGUN_NULL, "a short ankr version with a NULL out");
    CHECK(daegun_ankr_control_point(junk, 4, 100, NULL, NULL) == DAEGUN_NULL, "a point past the end into NULL");
    CHECK(daegun_feature_variations_open(junk, 4, NULL) == DAEGUN_NULL, "junk feature variations with a NULL out");
    CHECK(daegun_ivs_parse(junk, 4, 0, NULL) == DAEGUN_NULL, "a junk variation store with a NULL out");
    CHECK(daegun_delta_set_index_map_parse(junk, 4, 0, NULL) == DAEGUN_NULL, "a junk index map with a NULL out");
    daegun_table_map *tables = daegun_table_map_new();
    CHECK(daegun_table_map_get(tables, "ZZZZ", NULL) == DAEGUN_NULL, "a missing mapped table with a NULL out");
    daegun_table_map_free(tables);
    daegun_font_free(font);
}

/* A scene holds each glyph's outline once, and past DAEGUN_MAX_FLATTEN_POINTS points in all it is
 * DAEGUN_RANGE: P layers 17 composites of 64,000 points, R one composite eight times. */
static void a_scene_holds_each_outline_once_within_a_bound(void)
{
    daegun_font *font = open_font("assets/test-fonts/test-fixtures/colr-clip.ttf");
    if (!font) return;
    uint16_t past = 0, repeated = 0;
    CHECK(daegun_font_glyph_id(font, 'P', &past) == DAEGUN_OK, "the fixture has no P");
    CHECK(daegun_font_glyph_id(font, 'R', &repeated) == DAEGUN_OK, "the fixture has no R");
    daegun_color_scene *scene = NULL;
    CHECK(daegun_font_colr_scene(font, past, NULL, 0, 0, &scene) == DAEGUN_RANGE && !scene,
          "a scene past DAEGUN_MAX_FLATTEN_POINTS points was built");
    CHECK(daegun_font_colr_scene(font, repeated, NULL, 0, 0, &scene) == DAEGUN_OK, "eight layers failed");
    daegun_path *p = NULL;
    CHECK(daegun_color_scene_path(scene, 0, &p) == DAEGUN_OK, "the one outline is missing");
    daegun_path_free(p);
    p = NULL;
    CHECK(daegun_color_scene_path(scene, 1, &p) == DAEGUN_RANGE, "eight layers of one glyph held two outlines");
    daegun_color_scene_free(scene);
    daegun_font_free(font);
}

static void put_u16(uint8_t *p, uint16_t v) { p[0] = (uint8_t)(v >> 8); p[1] = (uint8_t)v; }
static void put_u32(uint8_t *p, uint32_t v) { put_u16(p, (uint16_t)(v >> 16)); put_u16(p + 2, (uint16_t)v); }

/* The fixture with D replaced by one contour of 10,000 on-curve points, climbing 1000 units and
 * falling back every 2 across. loca is rewritten long so the glyph can be any size. */
static daegun_font *fixture_with_zigzag(uint16_t *out_gid)
{
    daegun_font *base = open_font("assets/test-fonts/test-fixtures/hinted.ttf");
    if (!base) return NULL;
    daegun_table_map *map = NULL;
    uint16_t gid = 0, glyphs = 0;
    daegun_bytes glyf = { 0 }, loca = { 0 }, head = { 0 };
    CHECK(daegun_font_glyph_id(base, 'D', &gid) == DAEGUN_OK, "the fixture has no D");
    CHECK(daegun_font_num_glyphs(base, &glyphs) == DAEGUN_OK, "no glyph count");
    CHECK(daegun_font_instance_tables(base, NULL, 0, &map) == DAEGUN_OK, "no tables");
    daegun_table_map_get(map, "glyf", &glyf);
    daegun_table_map_get(map, "loca", &loca);
    daegun_table_map_get(map, "head", &head);

    enum { POINTS = 10000, ZIG = 10 + 2 + 2 + POINTS * 5 };
    uint8_t *zig = calloc(1, ZIG), *new_glyf = malloc(glyf.len + ZIG), *new_loca = malloc(4u * (glyphs + 1u));
    uint8_t *new_head = malloc(head.len);
    int16_t box[] = { 1, 0, 0, 2 * POINTS, 1000, POINTS - 1, 0 };
    for (int i = 0; i < 7; i++) put_u16(zig + 2 * i, (uint16_t)box[i]);
    for (int i = 0; i < POINTS; i++) {
        zig[14 + i] = 1;
        put_u16(zig + 14 + POINTS + 2 * i, i ? 2 : 0);
        put_u16(zig + 14 + 3 * POINTS + 2 * i, (uint16_t)(i == 0 ? 0 : i % 2 ? 1000 : -1000));
    }
    size_t used = 0;
    for (uint16_t g = 0; g < glyphs; g++) {
        size_t from = 2u * ((size_t)loca.data[2 * g] << 8 | loca.data[2 * g + 1]);
        size_t to = 2u * ((size_t)loca.data[2 * g + 2] << 8 | loca.data[2 * g + 3]);
        put_u32(new_loca + 4 * g, (uint32_t)used);
        if (g == gid) { memcpy(new_glyf + used, zig, ZIG); used += ZIG; }
        else { memcpy(new_glyf + used, glyf.data + from, to - from); used += to - from; }
    }
    put_u32(new_loca + 4 * glyphs, (uint32_t)used);
    memcpy(new_head, head.data, head.len);
    put_u16(new_head + 50, 1);
    daegun_table_map_set(map, "glyf", new_glyf, used);
    daegun_table_map_set(map, "loca", new_loca, 4u * (glyphs + 1u));
    daegun_table_map_set(map, "head", new_head, head.len);

    daegun_blob *blob = NULL;
    daegun_font *font = NULL;
    size_t len = 0;
    CHECK(daegun_table_map_build(map, &blob) == DAEGUN_OK, "the patched tables did not build");
    const uint8_t *bytes = daegun_blob_data(blob, &len);
    CHECK(daegun_font_open(bytes, len, &font) == DAEGUN_OK, "the patched font did not open");
    daegun_blob_free(blob);
    free(zig); free(new_glyf); free(new_loca); free(new_head);
    daegun_table_map_free(map);
    daegun_font_free(base);
    *out_gid = gid;
    return font;
}

/* A stroke or embolden that would take more than DAEGUN_MAX_FLATTEN_POINTS points is DAEGUN_RANGE,
 * with nothing drawn, where a narrow one draws. */
static void a_prepared_stroke_past_the_cap_is_range(void)
{
    uint16_t gid = 0;
    daegun_font *font = fixture_with_zigzag(&gid);
    if (!font) return;
    daegun_outline_options narrow, wide, bold;
    daegun_outline_options_default(&narrow);
    narrow.has_stroke = 1;
    narrow.stroke_width = 10.0f;
    narrow.stroke_join = DAEGUN_JOIN_ROUND;
    narrow.stroke_cap = DAEGUN_CAP_ROUND;
    wide = narrow;
    wide.stroke_width = 40000.0f;
    daegun_outline_options_default(&bold);
    bold.has_embolden = 1;
    bold.embolden = 40000.0f;

    pen_tally drawn = { 0, 0, 0, 0, 0 }, none = { 0, 0, 0, 0, 0 };
    daegun_pen dpen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &drawn };
    daegun_pen npen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &none };
    CHECK(daegun_font_prepared_outline(font, gid, 1024.0f, NULL, 0, &narrow, &dpen, NULL) == DAEGUN_OK,
          "a narrow stroke of the zigzag failed");
    CHECK(drawn.lines > 20000, "the zigzag drew %d lines", drawn.lines);
    CHECK(daegun_font_prepared_outline(font, gid, 1024.0f, NULL, 0, &wide, &npen, NULL) == DAEGUN_RANGE,
          "a stroke past the cap was not DAEGUN_RANGE");
    CHECK(daegun_font_prepared_outline(font, gid, 1024.0f, NULL, 0, &bold, &npen, NULL) == DAEGUN_RANGE,
          "an embolden past the cap was not DAEGUN_RANGE");
    CHECK(none.moves == 0 && none.lines == 0, "a refused outline drew %d lines", none.lines);
    daegun_font_free(font);
}

static void a_prepared_outline_reaches_the_pen(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    uint16_t gid = 0;
    CHECK(daegun_font_glyph_id(font, 'H', &gid) == DAEGUN_OK, "no glyph for 'H'");

    daegun_outline_options given;
    CHECK(daegun_outline_options_default(&given) == DAEGUN_OK, "outline defaults failed");

    pen_tally plain_calls = {0}, bold_calls = {0};
    daegun_pen plain_pen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &plain_calls };
    daegun_pen bold_pen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &bold_calls };
    daegun_prepared_glyph plain, bold;
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, NULL, &plain_pen, &plain) == DAEGUN_OK,
          "prepared_outline failed: %s", daegun_last_error().data);
    daegun_outline_options opts = given;
    opts.has_embolden = 1;
    opts.embolden = 80.0f;
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, &opts, &bold_pen, &bold) == DAEGUN_OK,
          "a bold prepared_outline failed");
    CHECK(plain_calls.moves > 0 && plain_calls.closes > 0, "the plain H drew nothing");
    CHECK(bold_calls.moves > plain_calls.moves, "the bold H added no stroke contours");
    CHECK(bold.advance_width > plain.advance_width, "bold advance %f is not wider than %f",
          bold.advance_width, plain.advance_width);
    /* advance_widths answers per 1000 em, a different route to the same number. */
    daegun_f64_list *widths = NULL;
    if (daegun_font_advance_widths(font, &gid, 1, NULL, 0, &widths) == DAEGUN_OK) {
        size_t n = 0;
        const double *w = daegun_f64_list_data(widths, &n);
        double want = n == 1 ? w[0] / 1000.0 * 32.0 : -1.0;
        CHECK(fabs(plain.advance_width - want) < 0.05,
              "a 32 px H advances %f px, not %f", plain.advance_width, want);
        daegun_f64_list_free(widths);
    }
    CHECK(plain.hinted == 0, "hinted with no hinting asked for");

    opts = given;
    opts.hinting = DAEGUN_HINT_AUTO_FORCE;
    daegun_prepared_glyph hinted;
    CHECK(daegun_font_prepared_outline(font, gid, 16.0f, NULL, 0, &opts, &plain_pen, &hinted) == DAEGUN_OK
              && hinted.hinted == 1, "AutoForce did not hint the H");

    uint16_t glyphs = 0;
    daegun_font_num_glyphs(font, &glyphs);
    CHECK(daegun_font_prepared_outline(font, glyphs, 16.0f, NULL, 0, NULL, &plain_pen, NULL)
              == DAEGUN_ABSENT,
          "a gid past the end was drawn");
    CHECK(daegun_font_prepared_outline(font, gid, 0.0f, NULL, 0, NULL, &plain_pen, NULL) == DAEGUN_RANGE,
          "zero px was accepted");
    opts = given;
    opts.has_oblique = 1;
    opts.oblique = 1.0f / 0.0f;
    CHECK(daegun_font_prepared_outline(font, gid, 16.0f, NULL, 0, &opts, &plain_pen, NULL) == DAEGUN_RANGE,
          "an infinite oblique was accepted");
    opts = given;
    opts.has_embolden = 1;
    opts.embolden = 1e38f;
    CHECK(daegun_font_prepared_outline(font, gid, 65535.0f, NULL, 0, &opts, &plain_pen, NULL) == DAEGUN_RANGE,
          "an embolden that overflows once scaled was not RANGE");

    /* A half-scale transform halves the width; a stroke widens it; a NaN in either is refused. */
    daegun_path *full = daegun_path_new(), *half = daegun_path_new(), *stroked = daegun_path_new();
    daegun_pen into_full, into_half, into_stroked;
    daegun_path_as_pen(full, &into_full);
    daegun_path_as_pen(half, &into_half);
    daegun_path_as_pen(stroked, &into_stroked);
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, NULL, &into_full, NULL) == DAEGUN_OK, "plain H failed");
    opts = given;
    opts.has_transform = 1;
    const float halving[6] = { 0.5f, 0.0f, 0.0f, 0.5f, 0.0f, 0.0f };
    memcpy(opts.transform, halving, sizeof halving);
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, &opts, &into_half, NULL) == DAEGUN_OK, "half H failed");
    opts.transform[3] = NAN;
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, &opts, &plain_pen, NULL) == DAEGUN_RANGE,
          "a NaN transform was accepted");
    opts = given;
    opts.has_stroke = 1;
    opts.stroke_width = 40.0f;
    opts.stroke_join = DAEGUN_JOIN_ROUND;
    opts.stroke_cap = DAEGUN_CAP_ROUND;
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, &opts, &into_stroked, NULL) == DAEGUN_OK,
          "stroked H failed");
    opts.stroke_width = NAN;
    CHECK(daegun_font_prepared_outline(font, gid, 32.0f, NULL, 0, &opts, &plain_pen, NULL) == DAEGUN_RANGE,
          "a NaN stroke width was accepted");
    double f0, g0, f1, g1, h0, k0, h1, k1, s0, t0, s1, t1;
    daegun_path_bounds(full, &f0, &g0, &f1, &g1);
    daegun_path_bounds(half, &h0, &k0, &h1, &k1);
    daegun_path_bounds(stroked, &s0, &t0, &s1, &t1);
    CHECK(fabs((h1 - h0) - (f1 - f0) / 2.0) < 0.01, "a half-scale H is %g wide against %g", h1 - h0, f1 - f0);
    CHECK((s1 - s0) > (f1 - f0) + 0.5, "a 40-unit stroke widened the H from %g to %g", f1 - f0, s1 - s0);
    daegun_path_free(full);
    daegun_path_free(half);
    daegun_path_free(stroked);
    CHECK(daegun_font_prepared_outline(NULL, gid, 16.0f, NULL, 0, NULL, &plain_pen, NULL) == DAEGUN_NULL,
          "a NULL font was accepted");
    CHECK(daegun_font_prepared_outline(font, gid, 16.0f, NULL, 0, NULL, NULL, NULL) == DAEGUN_NULL,
          "a NULL pen was accepted");
    daegun_font_free(font);
}

static void the_subpixel_filter_is_readable(void)
{
    daegun_subpixel_layout *gray = NULL, *rgb = NULL, *vbgr = NULL, *odd = NULL;
    CHECK(daegun_subpixel_layout_new(DAEGUN_LAYOUT_GRAYSCALE, &gray) == DAEGUN_OK, "grayscale failed");
    CHECK(daegun_subpixel_layout_new(DAEGUN_LAYOUT_RGB_H, &rgb) == DAEGUN_OK, "RGB_H failed");
    CHECK(daegun_subpixel_layout_new(DAEGUN_LAYOUT_BGR_V, &vbgr) == DAEGUN_OK, "BGR_V failed");
    CHECK(daegun_subpixel_layout_new(99, &odd) == DAEGUN_OK, "an unknown id failed");
    if (!gray || !rgb || !vbgr || !odd) return;

    uint8_t x = 0, y = 0, ch = 0;
    int8_t ox = 0, oy = 0;
    size_t px = 9, py = 9, n = 0;
    int32_t is_gray = -1;
    daegun_subpixel_layout_channels(gray, &ch);
    daegun_subpixel_layout_is_grayscale(gray, &is_gray);
    const float *w = daegun_subpixel_layout_weights(gray, 0, &n);
    CHECK(ch == 1 && is_gray == 1 && w && n == 1 && w[0] == 1.0f,
          "grayscale is not one channel of weight 1");
    CHECK(daegun_subpixel_layout_weights(gray, 1, &n) == NULL, "grayscale has a second channel");
    daegun_subpixel_layout_pad(gray, &px, &py);
    CHECK(px == 0 && py == 0, "grayscale asks for padding %zu, %zu", px, py);

    /* FIR5 over three stripes: 7 taps, 3x1 samples, starting 2 samples left, so one pixel of pad. */
    daegun_subpixel_layout_oversample(rgb, &x, &y);
    CHECK(x == 3 && y == 1, "RGB_H oversamples %ux%u", x, y);
    daegun_subpixel_layout_taps(rgb, &x, &y);
    CHECK(x == 7 && y == 1, "RGB_H has %ux%u taps", x, y);
    daegun_subpixel_layout_origin(rgb, &ox, &oy);
    CHECK(ox == -2 && oy == 0, "RGB_H starts at %d, %d", ox, oy);
    daegun_subpixel_layout_pad(rgb, &px, &py);
    CHECK(px == 1 && py == 0, "RGB_H pads %zu, %zu", px, py);
    for (size_t c = 0; c < 3; c++) {
        w = daegun_subpixel_layout_weights(rgb, c, &n);
        float sum = 0.0f;
        for (size_t i = 0; w && i < n; i++) sum += w[i];
        CHECK(w && n == 7 && fabsf(sum - 1.0f) < 1e-6f,
              "RGB_H channel %zu: %zu weights summing to %f", c, n, sum);
        CHECK(w && w[c] == 8.0f / 256.0f && w[c + 2] == 86.0f / 256.0f,
              "RGB_H channel %zu is not FIR5 at %zu", c, c);
    }
    daegun_subpixel_layout_pad(vbgr, &px, &py);
    w = daegun_subpixel_layout_weights(vbgr, 0, &n);
    CHECK(px == 0 && py == 1 && w && w[2] == 8.0f / 256.0f,
          "BGR_V is not FIR5 in reverse order down the column");

    uint64_t k1 = 0, k2 = 0, k3 = 0;
    CHECK(daegun_subpixel_layout_cache_key(rgb, &k1) == DAEGUN_OK, "cache_key failed");
    daegun_subpixel_layout_key(DAEGUN_LAYOUT_RGB_H, &k2);
    daegun_subpixel_layout_cache_key(odd, &k3);
    CHECK(k1 == k2, "the handle and the id disagree about RGB_H's identity");
    CHECK(k1 != k3, "RGB_H and grayscale share a cache key");
    daegun_subpixel_layout_key(DAEGUN_LAYOUT_GRAYSCALE, &k2);
    CHECK(k3 == k2, "an unknown id is not grayscale");

    const float weights[9] = { 0.25f, 0.5f, 0.25f, 0.5f, 0.5f, 0.0f, 0.0f, 0.5f, 0.5f };
    daegun_subpixel_layout *custom = NULL, *none = NULL;
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 3, 1, -1, 0, weights, &custom) == DAEGUN_OK,
          "a three-tap filter was refused");
    for (size_t c = 0; c < 3; c++) {
        w = daegun_subpixel_layout_weights(custom, c, &n);
        CHECK(w && n == 3 && w[0] == weights[c * 3] && w[2] == weights[c * 3 + 2],
              "channel %zu did not keep its own weights", c);
    }
    daegun_subpixel_layout_pad(custom, &px, &py);
    CHECK(px == 1 && py == 0, "a filter reaching one sample left does not pad one pixel");
    CHECK(daegun_subpixel_layout_from_weights(0, 1, 3, 1, -1, 0, weights, &none) == DAEGUN_RANGE,
          "a zero oversample was accepted");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 99, 1, 0, 0, weights, &none) == DAEGUN_RANGE,
          "a tap count past the table was accepted");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 0, 1, 0, 0, weights, &none) == DAEGUN_RANGE,
          "zero taps were accepted");

    /* The header's limits are literal copies of the library's, so the library has to agree. */
    static float wide[3 * DAEGUN_MAX_SUBPIXEL_WEIGHTS];
    for (size_t i = 0; i < 3 * DAEGUN_MAX_SUBPIXEL_WEIGHTS; i++) {
        wide[i] = 1.0f / DAEGUN_MAX_SUBPIXEL_WEIGHTS;
    }
    daegun_subpixel_layout *widest = NULL;
    CHECK(daegun_subpixel_layout_from_weights(DAEGUN_MAX_OVERSAMPLE, DAEGUN_MAX_OVERSAMPLE,
                                              DAEGUN_MAX_SUBPIXEL_TAPS, DAEGUN_MAX_SUBPIXEL_TAPS, 0, 0,
                                              wide, &widest) == DAEGUN_OK,
          "the widest filter the header allows was refused");
    w = daegun_subpixel_layout_weights(widest, 2, &n);
    CHECK(w && n == DAEGUN_MAX_SUBPIXEL_WEIGHTS, "the widest filter holds %zu weights a channel", n);
    daegun_subpixel_layout_free(widest);
    CHECK(daegun_subpixel_layout_from_weights(DAEGUN_MAX_OVERSAMPLE + 1, 1, 1, 1, 0, 0, wide,
                                              &none) == DAEGUN_RANGE,
          "an oversample past DAEGUN_MAX_OVERSAMPLE was accepted");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, DAEGUN_MAX_SUBPIXEL_TAPS + 1, 1, 0, 0, wide,
                                              &none) == DAEGUN_RANGE,
          "a tap count past DAEGUN_MAX_SUBPIXEL_TAPS was accepted");
    CHECK(daegun_subpixel_layout_from_weights(1, DAEGUN_MAX_OVERSAMPLE + 1, 1, 1, 0, 0, wide,
                                              &none) == DAEGUN_RANGE,
          "a y oversample past DAEGUN_MAX_OVERSAMPLE was accepted");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 1, DAEGUN_MAX_SUBPIXEL_TAPS + 1, 0, 0, wide,
                                              &none) == DAEGUN_RANGE,
          "a y tap count past DAEGUN_MAX_SUBPIXEL_TAPS was accepted");
    const float unfinite[3] = {NAN, 1.0f, 1.0f};
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 1, 1, 0, 0, unfinite, &none) == DAEGUN_RANGE,
          "a NaN weight was accepted");
    CHECK(none == NULL, "a refused layout was written");
    CHECK(daegun_subpixel_layout_new(DAEGUN_LAYOUT_RGB_H, NULL) == DAEGUN_NULL, "a NULL out was accepted");
    CHECK(daegun_subpixel_layout_taps(NULL, &x, &y) == DAEGUN_NULL, "taps of NULL");
    CHECK(daegun_subpixel_layout_weights(NULL, 0, &n) == NULL, "weights of NULL");
    daegun_subpixel_layout_free(NULL);
    daegun_subpixel_layout_free(custom);
    daegun_subpixel_layout_free(odd);
    daegun_subpixel_layout_free(vbgr);
    daegun_subpixel_layout_free(rgb);
    daegun_subpixel_layout_free(gray);
}

static void character_properties_need_no_font(void)
{
    int32_t gc = -1;
    CHECK(daegun_char_general_category('A', &gc) == DAEGUN_OK, "general_category failed");
    CHECK(gc == DAEGUN_GC_UPPERCASE_LETTER, "'A' is category %d, not uppercase letter", gc);
    CHECK(daegun_char_general_category('a', &gc) == DAEGUN_OK, "general_category failed");
    CHECK(gc == DAEGUN_GC_LOWERCASE_LETTER, "'a' is category %d, not lowercase letter", gc);
    CHECK(daegun_char_general_category(' ', &gc) == DAEGUN_OK, "general_category failed");
    CHECK(gc == DAEGUN_GC_SPACE_SEPARATOR, "space is category %d", gc);
    CHECK(daegun_char_general_category('5', &gc) == DAEGUN_OK, "general_category failed");
    CHECK(gc == DAEGUN_GC_DECIMAL_NUMBER, "'5' is category %d", gc);

    CHECK(daegun_char_general_category(0xD800, &gc) == DAEGUN_RANGE, "a surrogate was categorized");
    CHECK(daegun_char_general_category(0x110000, &gc) == DAEGUN_RANGE, "past U+10FFFF was allowed");

    uint32_t form = 0;
    CHECK(daegun_char_vertical_form(0x2014, &form) == DAEGUN_OK, "an em dash has a vertical form");
    CHECK(form == 0xFE31, "the em dash's vertical form is U+%04X, not FE31", form);
    CHECK(daegun_char_vertical_form(0x3001, &form) == DAEGUN_OK, "an ideographic comma has one");
    CHECK(form == 0xFE11, "the ideographic comma's form is U+%04X", form);
    CHECK(daegun_char_vertical_form('A', &form) == DAEGUN_ABSENT, "'A' has a vertical form");

    int32_t upright = -1;
    CHECK(daegun_char_is_upright(0x4E00, 0, &upright) == DAEGUN_OK, "is_upright failed");
    CHECK(upright, "a CJK ideograph does not stand upright in vertical text");
    CHECK(daegun_char_is_upright('A', 0, &upright) == DAEGUN_OK, "is_upright failed");
    CHECK(!upright, "a Latin capital stands upright in vertical text");
}

static void the_format_walkers_read_a_real_table(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return;
    }
    uint16_t num_glyphs = 0;
    daegun_font_num_glyphs(font, &num_glyphs);

    daegun_bytes gsub = { NULL, 0 };
    if (daegun_font_table(font, "GSUB", &gsub) == DAEGUN_OK) {
        uint16_t idx = 0;
        daegun_status st = daegun_coverage_index(gsub.data, gsub.len, 1, &idx);
        CHECK(st == DAEGUN_OK || st == DAEGUN_ABSENT, "coverage_index returned %d", st);
    }

    daegun_bytes hvar = { NULL, 0 };
    if (daegun_font_table(font, "HVAR", &hvar) == DAEGUN_OK) {
        uint32_t ivs_off = 0;
        CHECK(daegun_read_u32_be(hvar.data, hvar.len, 4, &ivs_off) == DAEGUN_OK,
              "HVAR has no itemVariationStoreOffset");
        daegun_ivs *ivs = NULL;
        daegun_status st = daegun_ivs_parse(hvar.data, hvar.len, ivs_off, &ivs);
        CHECK(st == DAEGUN_OK, "parsing HVAR's store returned %d: %s", st, daegun_last_error().data);
        if (st == DAEGUN_OK) {
            size_t axes = 0, regions = 0, ivds = 0;
            CHECK(daegun_ivs_axis_count(ivs, &axes) == DAEGUN_OK, "axis_count failed");
            CHECK(daegun_ivs_region_count(ivs, &regions) == DAEGUN_OK, "region_count failed");
            CHECK(daegun_ivs_ivd_count(ivs, &ivds) == DAEGUN_OK, "ivd_count failed");
            CHECK(axes > 0, "a variable font's store covers no axes");
            CHECK(regions > 0, "a variable font's store has no regions");
            CHECK(ivds > 0, "a variable font's store has no subtables");

            daegun_region_axis ra = { 9, 9, 9 };
            CHECK(daegun_ivs_region_axis(ivs, 0, 0, &ra) == DAEGUN_OK, "region_axis failed");
            CHECK(ra.start <= ra.peak && ra.peak <= ra.end,
                  "a region runs %g / %g / %g, out of order", ra.start, ra.peak, ra.end);
            CHECK(daegun_ivs_region_axis(ivs, regions, 0, &ra) == DAEGUN_RANGE,
                  "a region past the end was allowed");

            size_t rows = 0;
            CHECK(daegun_ivs_ivd_rows(ivs, 0, &rows) == DAEGUN_OK, "ivd_rows failed");
            size_t region_count = 0;
            const size_t *indices = daegun_ivs_ivd_region_indices(ivs, 0, &region_count);
            CHECK(indices != NULL || region_count == 0, "region indices came back null");
            if (rows > 0) {
                size_t row_len = 0;
                const int32_t *row = daegun_ivs_ivd_row(ivs, 0, 0, &row_len);
                CHECK(row != NULL, "the first delta row is null");
                CHECK(row_len == region_count,
                      "a row has %zu deltas for %zu regions", row_len, region_count);
            }
            CHECK(daegun_ivs_ivd_row(ivs, 0, rows, &rows) == NULL, "a row past the end was given");

            double location[4] = { 0.0, 0.0, 0.0, 0.0 };
            daegun_f64_list *scalars = NULL;
            CHECK(daegun_ivs_region_scalars(ivs, location, axes, &scalars) == DAEGUN_OK,
                  "region_scalars failed");
            size_t scalar_count = 0;
            const double *sd = daegun_f64_list_data(scalars, &scalar_count);
            CHECK(scalar_count == regions, "%zu scalars for %zu regions", scalar_count, regions);
            double delta = 99.0;
            CHECK(daegun_ivs_delta(ivs, 0, 0, sd, scalar_count, &delta) == DAEGUN_OK,
                  "ivs_delta failed");
            CHECK(delta == 0.0, "at the default location every delta is zero, got %g", delta);
            daegun_f64_list_free(scalars);
            daegun_ivs_free(ivs);
        }
    }

    daegun_bytes morx = { NULL, 0 };
    if (daegun_font_table(font, "morx", &morx) == DAEGUN_OK) {
        daegun_aat_lookup *lookup = NULL;
        daegun_status st = daegun_aat_lookup_open(morx.data, morx.len, num_glyphs, &lookup);
        CHECK(st == DAEGUN_OK || st == DAEGUN_PARSE, "aat_lookup_open returned %d", st);
        if (st == DAEGUN_OK) {
            daegun_aat_lookup_free(lookup);
        }
    }

    const uint8_t junk[8] = { 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff };
    daegun_ivs *bad = NULL;
    CHECK(daegun_ivs_parse(junk, sizeof junk, 0, &bad) == DAEGUN_PARSE, "junk parsed as a store");
    daegun_feature_variations *fv = NULL;
    CHECK(daegun_feature_variations_open(junk, sizeof junk, &fv) == DAEGUN_ABSENT,
          "junk carried feature variations");
    daegun_ankr *ankr = NULL;
    CHECK(daegun_ankr_open(junk, sizeof junk, num_glyphs, &ankr) == DAEGUN_PARSE,
          "junk parsed as an ankr table");

    daegun_font_free(font);
}

static void loca_and_glyf_are_walkable(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return;
    }
    daegun_bytes head = { NULL, 0 }, loca = { NULL, 0 }, glyf = { NULL, 0 };
    if (daegun_font_table(font, "loca", &loca) != DAEGUN_OK
        || daegun_font_table(font, "glyf", &glyf) != DAEGUN_OK
        || daegun_font_table(font, "head", &head) != DAEGUN_OK) {
        daegun_font_free(font);
        return;
    }

    int16_t loca_format = 0;
    CHECK(daegun_read_i16_be(head.data, head.len, 50, &loca_format) == DAEGUN_OK,
          "head has no indexToLocFormat");

    uint16_t num_glyphs = 0;
    daegun_font_num_glyphs(font, &num_glyphs);

    daegun_usize_list *offsets = NULL;
    CHECK(daegun_parse_loca(loca.data, loca.len, loca_format, num_glyphs, &offsets) == DAEGUN_OK,
          "parse_loca failed");
    size_t n = 0;
    const size_t *offs = daegun_usize_list_data(offsets, &n);
    CHECK(n == (size_t)num_glyphs + 1, "loca gave %zu offsets for %u glyphs", n, num_glyphs);
    CHECK(offs[0] <= offs[n - 1], "loca runs backwards");
    daegun_usize_list *huge = NULL;
    CHECK(daegun_parse_loca(loca.data, loca.len, loca_format, SIZE_MAX, &huge) == DAEGUN_RANGE,
          "a glyph count of SIZE_MAX was accepted");

    uint16_t gid = 0;
    daegun_font_glyph_id(font, 'A', &gid);
    pen_tally raw = { 0, 0, 0, 0, 0 };
    daegun_pen pen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &raw };
    daegun_status st = daegun_outline_glyf_bytes(glyf.data, glyf.len, offs, n, gid, &pen);
    CHECK(st == DAEGUN_OK, "outlining glyph %u from raw bytes returned %d: %s",
          gid, st, daegun_last_error().data);
    CHECK(raw.moves > 0, "outlining 'A' from raw glyf bytes drew nothing");

    pen_tally via_font = { 0, 0, 0, 0, 0 };
    daegun_pen fpen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, &via_font };
    CHECK(daegun_font_outline_glyph(font, gid, &fpen) == DAEGUN_OK, "drawing via the font failed");
    CHECK(raw.moves == via_font.moves && raw.lines == via_font.lines
              && raw.quads == via_font.quads && raw.closes == via_font.closes,
          "the raw tier drew %d/%d/%d/%d and the font tier %d/%d/%d/%d for the same glyph",
          raw.moves, raw.lines, raw.quads, raw.closes,
          via_font.moves, via_font.lines, via_font.quads, via_font.closes);

    daegun_usize_list_free(offsets);
    daegun_font_free(font);
}

static void the_atlas_packer_packs(void)
{
    daegun_shelf_packer *p = daegun_shelf_packer_new(64, 64);
    CHECK(p != NULL, "a new packer is null");

    daegun_rect a = { 9, 9, 9, 9 }, b = { 9, 9, 9, 9 };
    CHECK(daegun_shelf_packer_insert(p, 16, 16, &a) == DAEGUN_OK, "the first insert failed");
    CHECK(a.w == 16 && a.h == 16, "a 16x16 request came back %zux%zu", a.w, a.h);
    CHECK(daegun_shelf_packer_insert(p, 16, 16, &b) == DAEGUN_OK, "the second insert failed");
    /* Two rectangles in one atlas must not overlap – the one thing a packer is for. */
    int disjoint = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
    CHECK(disjoint, "the packer overlapped (%zu,%zu %zux%zu) with (%zu,%zu %zux%zu)",
          a.x, a.y, a.w, a.h, b.x, b.y, b.w, b.h);
    CHECK(a.x + a.w <= 64 && a.y + a.h <= 64, "a rectangle ran outside the atlas");

    daegun_rect huge = { 0, 0, 0, 0 };
    CHECK(daegun_shelf_packer_insert(p, 65, 65, &huge) == DAEGUN_ABSENT,
          "a rectangle bigger than the atlas was placed");

    CHECK(daegun_shelf_packer_reset(p) == DAEGUN_OK, "reset failed");
    daegun_rect again = { 9, 9, 9, 9 };
    CHECK(daegun_shelf_packer_insert(p, 64, 64, &again) == DAEGUN_OK,
          "a full-size rectangle did not fit after a reset");
    CHECK(daegun_shelf_packer_insert(p, 1, 1, &again) == DAEGUN_ABSENT,
          "a 64x64 atlas holding a 64x64 rectangle still had room");

    CHECK(daegun_shelf_packer_insert(NULL, 1, 1, &again) == DAEGUN_NULL, "a null packer was used");
    daegun_shelf_packer_free(p);
    daegun_shelf_packer_free(NULL);
}

static void the_rules_a_caller_would_get_wrong(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return;
    }
    daegun_line_metrics lm;
    CHECK(daegun_font_line_metrics(font, false, &lm) == DAEGUN_OK, "line_metrics failed");
    double h = 0.0;
    CHECK(daegun_line_metrics_height(&lm, &h) == DAEGUN_OK, "line height failed");
    CHECK(h > 0.0, "line height is %g", h);
    CHECK(h == lm.ascent - lm.descent + lm.line_gap, "line height is not the documented formula");
    /* And it is NOT the sum, which is the mistake this call exists to prevent. */
    CHECK(h != lm.ascent + lm.descent + lm.line_gap || lm.descent == 0.0,
          "with a negative descent the sum and the difference must differ");

    int32_t may = -1;
    CHECK(daegun_hint_mode_may_autohint(DAEGUN_HINT_AUTO, &may) == DAEGUN_OK, "may_autohint failed");
    CHECK(may, "DAEGUN_HINT_AUTO may not autohint");
    CHECK(daegun_hint_mode_may_autohint(DAEGUN_HINT_AUTO_FORCE, &may) == DAEGUN_OK, "failed");
    CHECK(may, "DAEGUN_HINT_AUTO_FORCE may not autohint");
    CHECK(daegun_hint_mode_may_autohint(DAEGUN_HINT_NONE, &may) == DAEGUN_OK, "failed");
    CHECK(!may, "DAEGUN_HINT_NONE may autohint");
    CHECK(daegun_hint_mode_may_autohint(DAEGUN_HINT_CLASSIC, &may) == DAEGUN_OK, "failed");
    CHECK(!may, "DAEGUN_HINT_CLASSIC may autohint");

    int32_t g = -1, m = -1;
    CHECK(daegun_cluster_level_is_graphemes(DAEGUN_CLUSTER_MONOTONE_GRAPHEMES, &g) == DAEGUN_OK, "failed");
    CHECK(daegun_cluster_level_is_monotone(DAEGUN_CLUSTER_MONOTONE_GRAPHEMES, &m) == DAEGUN_OK, "failed");
    CHECK(g && m, "MONOTONE_GRAPHEMES is graphemes=%d monotone=%d", g, m);
    CHECK(daegun_cluster_level_is_graphemes(DAEGUN_CLUSTER_GRAPHEMES, &g) == DAEGUN_OK, "failed");
    CHECK(daegun_cluster_level_is_monotone(DAEGUN_CLUSTER_GRAPHEMES, &m) == DAEGUN_OK, "failed");
    CHECK(g && !m, "GRAPHEMES is graphemes=%d monotone=%d", g, m);
    CHECK(daegun_cluster_level_is_graphemes(DAEGUN_CLUSTER_CHARACTERS, &g) == DAEGUN_OK, "failed");
    CHECK(!g, "CHARACTERS groups by grapheme");

    daegun_font_free(font);
}

static void scripts_answer_about_themselves(void)
{
    uint16_t latin = 0xffff, arabic = 0xffff;
    daegun_u32_list *runs = NULL;
    CHECK(daegun_text_script_runs("Ab", &runs) == DAEGUN_OK, "script_runs failed for Latin");
    size_t rn = 0;
    const uint32_t *rd = daegun_u32_list_data(runs, &rn);
    CHECK(rn >= 3, "a Latin run came back as %zu numbers", rn);
    if (rn >= 3) {
        latin = (uint16_t)rd[2];
    }
    daegun_u32_list_free(runs);

    runs = NULL;
    CHECK(daegun_text_script_runs("\xD8\xA7\xD9\x84", &runs) == DAEGUN_OK,
          "script_runs failed for Arabic");
    rd = daegun_u32_list_data(runs, &rn);
    CHECK(rn >= 3, "an Arabic run came back as %zu numbers", rn);
    if (rn >= 3) {
        arabic = (uint16_t)rd[2];
    }
    daegun_u32_list_free(runs);
    CHECK(latin != arabic, "Latin and Arabic came back as the same script id %u", latin);

    daegun_str_list *tags = NULL;
    CHECK(daegun_script_opentype_tags(latin, &tags) == DAEGUN_OK, "opentype_tags failed");
    size_t n = 0;
    CHECK(daegun_str_list_count(tags, &n) == DAEGUN_OK, "counting tags failed");
    CHECK(n >= 1, "Latin maps to %zu OpenType tags", n);
    if (n >= 1) {
        daegun_str t = { NULL, 0 };
        CHECK(daegun_str_list_at(tags, 0, &t) == DAEGUN_OK, "tag 0 failed");
        CHECK(t.len == 4, "an OpenType script tag is four characters, got %zu", t.len);
    }
    daegun_str_list_free(tags);

    bool rtl = true;
    CHECK(daegun_script_is_rtl(latin, &rtl) == DAEGUN_OK && !rtl, "Latin runs right to left");
    CHECK(daegun_script_is_rtl(arabic, &rtl) == DAEGUN_OK && rtl, "Arabic runs left to right");

    int32_t ctx = -1;
    CHECK(daegun_script_is_context_dependent(latin, &ctx) == DAEGUN_OK, "failed");
    CHECK(!ctx, "Latin takes its identity from its neighbors");
    CHECK(daegun_script_is_context_dependent(arabic, &ctx) == DAEGUN_OK, "failed");
    CHECK(!ctx, "Arabic takes its identity from its neighbors");

    runs = NULL;
    CHECK(daegun_text_script_runs(",", &runs) == DAEGUN_OK, "script_runs failed for a comma");
    rd = daegun_u32_list_data(runs, &rn);
    if (rn >= 3) {
        CHECK(daegun_script_is_context_dependent((uint16_t)rd[2], &ctx) == DAEGUN_OK, "failed");
        CHECK(ctx, "a lone comma is not context dependent, so nothing is");
    }
    daegun_u32_list_free(runs);

    runs = NULL;
    CHECK(daegun_text_script_runs("\xF0\x90\x8C\x80", &runs) == DAEGUN_OK, "script_runs failed for Old Italic");
    rd = daegun_u32_list_data(runs, &rn);
    CHECK(rn >= 3 && daegun_script_is_rtl((uint16_t)rd[2], &rtl) == DAEGUN_ABSENT,
          "Old Italic, written either way, has a direction");
    daegun_u32_list_free(runs);
}

static void subsetting_maps_glyph_ids(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return;
    }
    uint16_t a = 0, b = 0;
    daegun_font_glyph_id(font, 'A', &a);
    daegun_font_glyph_id(font, 'B', &b);
    const uint16_t keep[3] = { 0, a, b };

    daegun_subset *sub = NULL;
    daegun_status st = daegun_font_subset(font, keep, 3, NULL, 0, &sub);
    CHECK(st == DAEGUN_OK, "subset returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        uint16_t na = 0xffff, nb = 0xffff, notdef = 0xffff;
        CHECK(daegun_subset_new_gid(sub, 0, &notdef) == DAEGUN_OK, ".notdef was dropped");
        CHECK(notdef == 0, ".notdef moved to %u", notdef);
        CHECK(daegun_subset_new_gid(sub, a, &na) == DAEGUN_OK, "'A' was dropped from its own subset");
        CHECK(daegun_subset_new_gid(sub, b, &nb) == DAEGUN_OK, "'B' was dropped from its own subset");
        CHECK(na != nb, "'A' and 'B' both mapped to %u", na);

        uint16_t dropped_from = 0;
        for (uint16_t g = 1; g < 200; g++) {
            if (g != a && g != b) { dropped_from = g; break; }
        }
        uint16_t nd = 0xffff;
        st = daegun_subset_new_gid(sub, dropped_from, &nd);
        CHECK(st == DAEGUN_ABSENT || (st == DAEGUN_OK && nd != 0),
              "glyph %u was dropped but mapped to %u with status %d", dropped_from, nd, st);

        size_t len = 0;
        const uint8_t *ttf = daegun_subset_ttf(sub, &len);
        CHECK(ttf != NULL && len > 0, "the subset has no bytes");
        daegun_font *reopened = NULL;
        CHECK(daegun_font_open(ttf, len, &reopened) == DAEGUN_OK, "the subset does not open");
        daegun_font_free(reopened);
        daegun_subset_free(sub);
    }
    daegun_font_free(font);
}

static void stat_values_are_readable(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return;
    }
    daegun_stat *stat = NULL;
    if (daegun_font_stat_info(font, &stat) != DAEGUN_OK) {
        daegun_font_free(font);
        return;
    }
    size_t n = 0;
    CHECK(daegun_stat_value_count(stat, &n) == DAEGUN_OK, "value_count failed");
    CHECK(n > 0, "a face with a STAT table names no axis values");

    size_t combo_n = 0;
    const daegun_axis_value *combos = daegun_stat_combo_values(stat, &combo_n);
    CHECK(combos != NULL || combo_n == 0, "the combo list is null with a non-zero count");

    size_t named = 0;
    for (size_t i = 0; i < n; i++) {
        daegun_stat_value v;
        memset(&v, 0xff, sizeof v);
        CHECK(daegun_stat_value_at(stat, i, &v) == DAEGUN_OK, "value %zu failed", i);
        CHECK(v.kind >= DAEGUN_STAT_SINGLE && v.kind <= DAEGUN_STAT_COMBO,
              "value %zu has kind %d", i, v.kind);
        CHECK(v.elidable == 0 || v.elidable == 1, "elidable is %u", v.elidable);
        CHECK(v.older_sibling == 0 || v.older_sibling == 1, "older_sibling is %u", v.older_sibling);
        if (v.kind == DAEGUN_STAT_RANGE) {
            CHECK(v.min <= v.value && v.value <= v.max,
                  "range %zu is %g in [%g, %g]", i, v.value, v.min, v.max);
        }
        if (v.kind == DAEGUN_STAT_COMBO) {
            CHECK((size_t)v.combo_start + v.combo_count <= combo_n,
                  "combo %zu indexes %u..%u of %zu pairs",
                  i, v.combo_start, v.combo_start + v.combo_count, combo_n);
        } else {
            CHECK(v.combo_count == 0, "a non-combo value carries %u pairs", v.combo_count);
        }
        daegun_str name = { NULL, 0 };
        daegun_status ns = daegun_stat_value_name(stat, i, &name);
        CHECK(ns == DAEGUN_OK || ns == DAEGUN_ABSENT, "name %zu returned %d", i, ns);
        CHECK((ns == DAEGUN_OK) == (v.has_name != 0),
              "value %zu says has_name=%u and the name call returned %d", i, v.has_name, ns);
        if (ns == DAEGUN_OK) {
            CHECK(name.data != NULL && strlen(name.data) == name.len, "name %zu is malformed", i);
            named++;
        }
    }
    CHECK(named > 0, "a STAT table with %zu values names none of them", n);
    CHECK(daegun_stat_value_at(stat, n, NULL) == DAEGUN_NULL, "a null out-parameter was accepted");
    daegun_stat_free(stat);
    daegun_font_free(font);
}

static void an_owned_buffer_opens_without_a_copy(const char *path)
{
    CHECK(daegun_font_buffer_new(SIZE_MAX) == NULL, "a SIZE_MAX buffer was handed out");
    size_t len = 0;
    uint8_t *file = slurp(path, &len);
    if (!file) { CHECK(0, "could not read %s", path); return; }

    uint8_t *buf = daegun_font_buffer_new(len);
    CHECK(buf != NULL, "buffer_new returned null for %zu bytes", len);
    memcpy(buf, file, len);

    daegun_font *font = NULL;
    daegun_status st = daegun_font_open_owned(buf, len, &font);
    CHECK(st == DAEGUN_OK, "open_owned returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        daegun_font *copied = NULL;
        CHECK(daegun_font_open(file, len, &copied) == DAEGUN_OK, "the copying open failed");
        uint16_t a = 0, b = 0, ga = 0, gb = 0;
        daegun_font_upm(font, &a);          daegun_font_upm(copied, &b);
        daegun_font_num_glyphs(font, &ga);  daegun_font_num_glyphs(copied, &gb);
        CHECK(a == b && ga == gb, "owned open gave upm %u/%u glyphs %u/%u", a, b, ga, gb);

        daegun_bytes h1 = { NULL, 0 }, h2 = { NULL, 0 };
        daegun_font_table(font, "head", &h1);
        daegun_font_table(copied, "head", &h2);
        CHECK(h1.len == h2.len && h1.len > 0 && memcmp(h1.data, h2.data, h1.len) == 0,
              "the owned font's head differs from the copied one's");
        daegun_font_free(copied);
        daegun_font_free(font);   /* takes the buffer with it */
    }

    uint8_t *unused = daegun_font_buffer_new(4096);
    CHECK(unused != NULL, "buffer_new(4096) returned null");
    unused[0] = 1; unused[4095] = 2;   /* writable to its full length */
    daegun_font_buffer_free(unused, 4096);
    daegun_font_buffer_free(NULL, 0);

    CHECK(daegun_font_buffer_new(0) == NULL, "a zero-length buffer was allocated");
    daegun_font *none = NULL;
    CHECK(daegun_font_open_owned(NULL, 10, &none) == DAEGUN_NULL, "a null buffer was accepted");

    uint8_t *junk = daegun_font_buffer_new(64);
    memset(junk, 0xff, 64);
    CHECK(daegun_font_open_owned(junk, 64, &none) == DAEGUN_PARSE, "junk parsed as a font");

    free(file);
}

static int str_is(daegun_str s, const char *want)
{
    return s.data != NULL && s.len == strlen(want) && memcmp(s.data, want, s.len) == 0;
}

static int text_is(const daegun_text *t, const char *want)
{
    daegun_str s = { NULL, 0 };
    return t != NULL && daegun_text_str(t, &s) == DAEGUN_OK && str_is(s, want);
}

static int strings_are(const daegun_str_list *list, const char *const *want, size_t n)
{
    size_t count = 0;
    if (daegun_str_list_count(list, &count) != DAEGUN_OK || count != n) {
        return 0;
    }
    for (size_t i = 0; i < n; i++) {
        daegun_str s = { NULL, 0 };
        if (daegun_str_list_at(list, i, &s) != DAEGUN_OK || !str_is(s, want[i])) {
            return 0;
        }
    }
    return 1;
}

#define INTER "assets/test-fonts/inter/InterVariable.ttf"

/* What Inter says about itself, each against the Rust API's answer for the same call. */
static void a_face_describes_itself(void)
{
    daegun_font *font = open_font(INTER);
    if (!font) {
        return;
    }
    int32_t cap = 0;
    uint32_t flags = 0;
    double angle = -1.0, tracking = -1.0;
    CHECK(daegun_font_cap_height(font, &cap) == DAEGUN_OK && cap == 728, "cap height %d, not 728", cap);
    CHECK(daegun_font_flags(font, &flags) == DAEGUN_OK && flags == 32, "head flags %u, not 32", flags);
    CHECK(daegun_font_italic_angle(font, &angle) == DAEGUN_OK && angle == 0.0, "italic angle %g", angle);
    CHECK(daegun_font_tracking(font, 12.0, true, &tracking) == DAEGUN_OK && tracking == 0.0,
          "tracking %g with no trak table", tracking);
    daegun_text *style = NULL;
    CHECK(daegun_font_style(font, &style) == DAEGUN_OK && text_is(style, "normal"), "the style is not normal");
    daegun_text_free(style);

    daegun_os2_info os2;
    memset(&os2, 0xff, sizeof os2);
    CHECK(daegun_font_os2_info(font, &os2) == DAEGUN_OK, "os2_info failed");
    CHECK(os2.version == 4 && os2.has_selection && os2.selection == 192,
          "OS/2 version %u, selection %u", os2.version, os2.selection);
    CHECK(os2.has_family_class && os2.family_class == 0, "family class %u", os2.family_class);
    CHECK(os2.has_win_metrics && os2.win_ascent == 969 && os2.win_descent == 241,
          "win metrics %d and %d", os2.win_ascent, os2.win_descent);
    CHECK(os2.has_typo_metrics && os2.typo_ascender == 969 && os2.typo_descender == -241
              && os2.typo_line_gap == 0,
          "typo metrics %d, %d and %d", os2.typo_ascender, os2.typo_descender, os2.typo_line_gap);
    bool bold = true, regular = false, oblique = true, typo = false;
    CHECK(daegun_font_is_bold(font, &bold) == DAEGUN_OK && !bold, "Inter's default is bold");
    CHECK(daegun_font_is_regular(font, &regular) == DAEGUN_OK && regular, "Inter's default is not regular");
    CHECK(daegun_font_is_oblique(font, &oblique) == DAEGUN_OK && !oblique, "Inter's default is oblique");
    CHECK(daegun_font_uses_typo_metrics(font, &typo) == DAEGUN_OK && typo, "Inter ignores its typo metrics");

    daegun_axis black[1] = { { "wght", 900.0 } };
    daegun_typographic_metrics tm, heavy;
    memset(&tm, 0, sizeof tm);
    memset(&heavy, 0, sizeof heavy);
    CHECK(daegun_font_typographic_metrics(font, NULL, 0, &tm) == DAEGUN_OK, "typographic_metrics failed");
    CHECK(tm.x_height == 546 && tm.underline_position == -170 && tm.underline_thickness == 68
              && tm.strikeout_size == 68 && tm.strikeout_position == 328,
          "x-height %d, underline %d and %d, strikeout %d and %d", tm.x_height, tm.underline_position,
          tm.underline_thickness, tm.strikeout_size, tm.strikeout_position);
    CHECK(tm.subscript_x_size == 650 && tm.subscript_y_offset == 75 && tm.superscript_y_offset == 350,
          "subscript %d and %d, superscript %d", tm.subscript_x_size, tm.subscript_y_offset,
          tm.superscript_y_offset);
    CHECK(daegun_font_typographic_metrics(font, black, 1, &heavy) == DAEGUN_OK
              && heavy.underline_position == -147 && heavy.underline_thickness == 113,
          "at 900 the underline is at %d, %d thick", heavy.underline_position, heavy.underline_thickness);

    daegun_f64_list *norm = NULL;
    CHECK(daegun_font_normalized_axes(font, black, 1, &norm) == DAEGUN_OK, "normalized_axes failed");
    size_t nn = 0;
    const double *nd = daegun_f64_list_data(norm, &nn);
    CHECK(nn == 2 && nd[0] == 0.0 && nd[1] == 1.0, "wght 900 normalized to %zu coordinates", nn);
    daegun_f64_list_free(norm);

    daegun_axis bolder[1] = { { "wght", 700.0 } };
    daegun_blob *inst = NULL;
    CHECK(daegun_font_instance(font, bolder, 1, &inst) == DAEGUN_OK, "instance failed");
    size_t inst_len = 0;
    const uint8_t *inst_data = daegun_blob_data(inst, &inst_len);
    daegun_font *pinned = NULL;
    bool variable = true;
    CHECK(inst_data && daegun_font_open(inst_data, inst_len, &pinned) == DAEGUN_OK
              && daegun_font_is_variable(pinned, &variable) == DAEGUN_OK && !variable,
          "the instance does not open as a static font");
    daegun_font_free(pinned);
    daegun_blob_free(inst);

    size_t named = 0;
    CHECK(daegun_font_named_instance_count(font, &named) == DAEGUN_OK && named == 9,
          "%zu named instances, not 9", named);
    daegun_text *name = NULL, *ps = NULL;
    daegun_str_list *coord_tags = NULL;
    daegun_f64_list *coord_values = NULL;
    static const char *const opsz_wght[2] = { "opsz", "wght" };
    CHECK(daegun_font_named_instance(font, 0, &name, &ps, &coord_tags, &coord_values) == DAEGUN_OK,
          "named_instance 0 failed");
    CHECK(text_is(name, "Thin") && text_is(ps, "InterVariable-Thin"), "instance 0 is not Thin");
    size_t cn = 0;
    const double *cv = daegun_f64_list_data(coord_values, &cn);
    CHECK(strings_are(coord_tags, opsz_wght, 2) && cn == 2 && cv[0] == 14.0 && cv[1] == 100.0,
          "Thin is not at opsz 14, wght 100");
    daegun_text_free(name);
    daegun_text_free(ps);
    daegun_str_list_free(coord_tags);
    daegun_f64_list_free(coord_values);
    CHECK(daegun_font_named_instance(font, 9, NULL, NULL, NULL, NULL) == DAEGUN_RANGE, "a tenth instance");

    uint16_t f = 0, i = 0, h = 0;
    daegun_font_glyph_id(font, 'f', &f);
    daegun_font_glyph_id(font, 'i', &i);
    daegun_font_glyph_id(font, 'H', &h);
    const uint16_t fi[2] = { f, i };
    daegun_u16_list *closure = NULL;
    CHECK(daegun_font_glyph_closure(font, fi, 2, NULL, 0, &closure) == DAEGUN_OK, "glyph_closure failed");
    size_t cl = 0;
    const uint16_t *cd = daegun_u16_list_data(closure, &cl);
    int kept = 0;
    for (size_t k = 0; k < cl; k++) {
        kept += cd[k] == f || cd[k] == i;
    }
    CHECK(cl == 20 && kept == 2, "f and i close over %zu glyphs, keeping %d of them", cl, kept);
    daegun_u16_list_free(closure);
    daegun_subset *sub = NULL;
    uint16_t nh = 0;
    CHECK(daegun_font_subset_text(font, "Hi", NULL, 0, &sub) == DAEGUN_OK
              && daegun_subset_new_gid(sub, h, &nh) == DAEGUN_OK && nh != 0,
          "subset_text left H out");
    daegun_subset_free(sub);

    daegun_str_list *scripts = NULL, *langs = NULL, *features = NULL, *latn = NULL;
    static const char *const want_scripts[4] = { "DFLT", "cyrl", "grek", "latn" };
    static const char *const want_langs[3] = { "CAT ", "MOL ", "ROM " };
    size_t nf = 0, nl = 0;
    CHECK(daegun_font_script_tags(font, &scripts) == DAEGUN_OK && strings_are(scripts, want_scripts, 4),
          "the scripts are not DFLT, cyrl, grek and latn");
    CHECK(daegun_font_language_tags(font, "latn", &langs) == DAEGUN_OK && strings_are(langs, want_langs, 3),
          "latn's languages are not CAT, MOL and ROM");
    CHECK(daegun_font_feature_tags(font, NULL, NULL, &features) == DAEGUN_OK
              && daegun_str_list_count(features, &nf) == DAEGUN_OK && nf == 42,
          "%zu features, not 42", nf);
    CHECK(daegun_font_feature_tags(font, "latn", "ROM ", &latn) == DAEGUN_OK
              && daegun_str_list_count(latn, &nl) == DAEGUN_OK && nl == 43,
          "ROM has %zu features, not DFLT's 42 and locl", nl);
    int locl = 0;
    for (size_t k = 0; k < nl; k++) {
        daegun_str t = { NULL, 0 };
        locl += daegun_str_list_at(latn, k, &t) == DAEGUN_OK && str_is(t, "locl");
    }
    CHECK(locl == 1, "ROM lists locl %d times", locl);
    daegun_str_list_free(scripts);
    daegun_str_list_free(langs);
    daegun_str_list_free(features);
    daegun_str_list_free(latn);

    daegun_stat *stat = NULL;
    CHECK(daegun_font_stat_info(font, &stat) == DAEGUN_OK, "stat_info failed");
    size_t axes = 0;
    daegun_str_list *axis_tags = NULL;
    const uint16_t *orderings = NULL;
    static const char *const stat_tags[3] = { "opsz", "wght", "ital" };
    CHECK(daegun_stat_axes(stat, &axes, &axis_tags, &orderings) == DAEGUN_OK && axes == 3
              && strings_are(axis_tags, stat_tags, 3) && orderings[0] == 0 && orderings[1] == 1
              && orderings[2] == 2,
          "STAT's %zu axes are not opsz, wght and ital in order", axes);
    daegun_str_list_free(axis_tags);
    CHECK(daegun_stat_axes(stat, NULL, NULL, NULL) == DAEGUN_OK, "stat_axes refused NULL outs");
    static const char *const axis_names[3] = { "Optical Size", "Weight", "Italic" };
    for (size_t i = 0; i < 3; i++) {
        daegun_str name = { NULL, 0 };
        CHECK(daegun_stat_axis_name(stat, i, &name) == DAEGUN_OK && name.len == strlen(axis_names[i])
                  && memcmp(name.data, axis_names[i], name.len) == 0,
              "STAT axis %zu is not named %s", i, axis_names[i]);
    }
    daegun_str past = { NULL, 0 };
    CHECK(daegun_stat_axis_name(stat, 3, &past) == DAEGUN_RANGE, "a fourth axis has a name");
    daegun_text *elided = NULL;
    CHECK(daegun_stat_elided_fallback_name(stat, &elided) == DAEGUN_OK && text_is(elided, "Regular"),
          "the elided fallback name is not Regular");
    daegun_text_free(elided);
    daegun_stat_free(stat);
    daegun_font_free(font);
}

/* How Inter maps text to glyphs and what its runs report. */
static void glyphs_and_runs_answer(void)
{
    daegun_font *font = open_font(INTER);
    if (!font) {
        return;
    }
    bool has = false;
    CHECK(daegun_font_has_glyph(font, 'A', &has) == DAEGUN_OK && has, "Inter has no A");
    CHECK(daegun_font_has_glyph(font, 0x10FFFF, &has) == DAEGUN_OK && !has, "Inter maps U+10FFFF");
    daegun_u16_list *ids = NULL;
    daegun_blob *present = NULL;
    CHECK(daegun_font_glyph_ids(font, "A\xF4\x8F\xBF\xBF" "b", &ids, &present) == DAEGUN_OK, "glyph_ids failed");
    size_t ni = 0, np = 0;
    const uint16_t *id = daegun_u16_list_data(ids, &ni);
    const uint8_t *pr = daegun_blob_data(present, &np);
    CHECK(ni == 3 && np == 3 && id[0] == 2 && id[2] == 578 && pr[0] && !pr[1] && pr[2],
          "glyph_ids gave %zu ids and %zu flags", ni, np);
    daegun_u16_list_free(ids);
    daegun_blob_free(present);

    daegun_u32_list *cps = NULL, *all = NULL;
    daegun_u16_list *cov = NULL;
    CHECK(daegun_font_coverage(font, &cps, &cov) == DAEGUN_OK, "coverage failed");
    CHECK(daegun_font_codepoints(font, &all) == DAEGUN_OK, "codepoints failed");
    size_t ncp = 0, ncg = 0, nall = 0;
    const uint32_t *cp = daegun_u32_list_data(cps, &ncp);
    const uint16_t *cg = daegun_u16_list_data(cov, &ncg);
    daegun_u32_list_data(all, &nall);
    int a_maps = 0;
    for (size_t k = 0; k < ncp && k < ncg; k++) {
        a_maps |= cp[k] == 'A' && cg[k] == 2;
    }
    CHECK(ncp == 2852 && ncg == 2852 && nall == 2852 && a_maps,
          "coverage gave %zu and %zu, codepoints %zu, not 2852 with A at 2", ncp, ncg, nall);
    daegun_u32_list_free(cps);
    daegun_u16_list_free(cov);
    daegun_u32_list_free(all);

    uint16_t space = 0, acute = 0, uvs = 0;
    daegun_font_glyph_id(font, ' ', &space);
    daegun_font_glyph_id(font, 0x301, &acute);
    double box[4] = { 0.0, 0.0, 0.0, 0.0 };
    CHECK(daegun_font_glyph_bounds(font, 2, NULL, 0, box) == DAEGUN_OK && box[0] == 25.390625
              && box[1] == 0.0 && box[2] == 664.55078125 && box[3] == 727.5390625,
          "A's box is %g, %g, %g, %g", box[0], box[1], box[2], box[3]);
    CHECK(daegun_font_glyph_bounds(font, space, NULL, 0, box) == DAEGUN_ABSENT, "a space has ink");
    CHECK(daegun_font_variation_glyph_id(font, 'A', 0xFE00, &uvs) == DAEGUN_ABSENT, "Inter maps A with VS1");
    uint32_t vadv = 0;
    int32_t vorig = 0, dvorig = 1;
    CHECK(daegun_font_vertical_advance(font, 2, NULL, 0, &vadv) == DAEGUN_OK && vadv == 1210,
          "A's vertical advance is %u, not 1210", vadv);
    CHECK(daegun_font_vertical_origin(font, 2, NULL, 0, &vorig) == DAEGUN_ABSENT, "Inter has a VORG");
    CHECK(daegun_font_default_vertical_origin(font, &dvorig) == DAEGUN_OK && dvorig == 0,
          "the default vertical origin is %d", dvorig);
    daegun_f64_list *positions = NULL;
    CHECK(daegun_font_caret_positions(font, "ab", NULL, 0, false, &positions) == DAEGUN_OK, "caret_positions failed");
    size_t npos = 0;
    const double *pos = daegun_f64_list_data(positions, &npos);
    CHECK(npos == 3 && pos[0] == 0.0 && pos[1] == 561.5234375 && pos[2] == 1173.828125,
          "ab has %zu caret positions", npos);
    daegun_f64_list_free(positions);

    int32_t klass = -1;
    uint16_t mark_class = 9;
    CHECK(daegun_font_glyph_class(font, 2, &klass) == DAEGUN_OK && klass == DAEGUN_GLYPH_CLASS_BASE,
          "A is class %d", klass);
    CHECK(daegun_font_glyph_class(font, acute, &klass) == DAEGUN_OK && klass == DAEGUN_GLYPH_CLASS_MARK,
          "the combining acute is class %d", klass);
    CHECK(daegun_font_mark_attachment_class(font, acute, &mark_class) == DAEGUN_OK && mark_class == 0,
          "the acute's mark attachment class is %u", mark_class);
    daegun_text *gname = NULL;
    CHECK(daegun_font_glyph_name(font, 2, &gname) == DAEGUN_OK && text_is(gname, "A"), "glyph 2 is not A");
    daegun_text_free(gname);
    daegun_str_list *names = NULL;
    daegun_blob *named = NULL;
    CHECK(daegun_font_glyph_names(font, &names, &named) == DAEGUN_OK, "glyph_names failed");
    size_t nnames = 0, nnamed = 0;
    daegun_str n2 = { NULL, 0 };
    daegun_str_list_count(names, &nnames);
    const uint8_t *has_name = daegun_blob_data(named, &nnamed);
    CHECK(nnames == 2937 && nnamed == 2937 && has_name[2] && daegun_str_list_at(names, 2, &n2) == DAEGUN_OK
              && str_is(n2, "A"),
          "%zu names and %zu flags, without A at 2", nnames, nnamed);
    daegun_str_list_free(names);
    daegun_blob_free(named);

    daegun_run *lang = NULL;
    CHECK(daegun_font_shape_with_language(font, "Hi", NULL, 0, false, "TRK", &lang) == DAEGUN_OK,
          "shape_with_language failed");
    size_t ng = 0;
    const uint16_t *g = daegun_run_glyphs(lang, &ng);
    CHECK(ng == 2 && g[0] == 161 && g[1] == 689, "Hi in Turkish shaped to %zu glyphs", ng);
    daegun_run_free(lang);

    daegun_shape_options opts;
    memset(&opts, 0xff, sizeof opts);
    CHECK(daegun_shape_options_default(&opts) == DAEGUN_OK && opts.cluster_level == DAEGUN_CLUSTER_MONOTONE_GRAPHEMES
              && opts.ignorables == DAEGUN_IGNORABLES_HIDE && opts.features == NULL && opts.script == NULL
              && !opts.report_unsafe_to_concat && !opts.has_invisible_glyph && !opts.has_seed_script
              && opts.seed_script == 0,
          "the default options are not the defaults");
    opts.report_unsafe_to_concat = true;
    opts.report_tatweel_positions = true;
    daegun_run *reported = NULL, *plain = NULL;
    CHECK(daegun_font_shape_with_options(font, "Hi", NULL, 0, false, &opts, &reported) == DAEGUN_OK,
          "shape_with_options failed");
    size_t nb = 0, nc = 0, nt = 0;
    const uint8_t *utb = daegun_run_unsafe_to_break(reported, &nb);
    const uint8_t *utc = daegun_run_unsafe_to_concat(reported, &nc);
    daegun_run_safe_to_insert_tatweel(reported, &nt);
    CHECK(nb == 2 && !utb[0] && !utb[1], "Hi is unsafe to break: %zu flags", nb);
    CHECK(nc == 2 && utc[0] && utc[1] && nt == 2, "%zu concat and %zu tatweel flags, asked for", nc, nt);
    bool broken = true;
    CHECK(daegun_run_has_broken_syllable(reported, &broken) == DAEGUN_OK && !broken, "Hi has a broken syllable");
    daegun_run_free(reported);
    CHECK(daegun_font_shape_with_options(font, "Hi", NULL, 0, false, NULL, &plain) == DAEGUN_OK,
          "NULL options were refused");
    daegun_run_unsafe_to_concat(plain, &nc);
    CHECK(nc == 0, "unasked, unsafe_to_concat came back for %zu glyphs", nc);
    daegun_run_free(plain);

    daegun_justified *j = NULL;
    bool has_level = true, shrink = true, best = true;
    size_t level = 9;
    double width = 0.0;
    CHECK(daegun_font_justify(font, "Hi", NULL, 0, false, "latn", NULL, 985.3515625, 0.5, &j) == DAEGUN_OK
              && daegun_justified_info(j, &has_level, &level, &shrink, &width, &best) == DAEGUN_OK,
          "justify at the natural width failed");
    CHECK(!has_level && !shrink && !best && width == 985.3515625, "Hi at its own width came back %g wide", width);
    daegun_run_glyphs(daegun_justified_run(j), &ng);
    CHECK(ng == 2, "the justified run has %zu glyphs", ng);
    daegun_justified_free(j);
    j = NULL;
    CHECK(daegun_font_justify(font, "Hi", NULL, 0, false, "latn", NULL, 2000.0, 1.0, &j) == DAEGUN_OK
              && daegun_justified_info(j, &has_level, NULL, &shrink, NULL, &best) == DAEGUN_OK,
          "justify to 2000 failed");
    CHECK(!has_level && !shrink && best, "with no JSTF, a stretch to 2000 was not a best effort");
    daegun_justified_free(j);
    daegun_font_free(font);

    daegun_font *carets = open_font("assets/test-fonts/test-fixtures/carets.ttf");
    if (carets) {
        daegun_f64_list *list = NULL;
        daegun_blob *present = NULL;
        CHECK(daegun_font_ligature_carets(carets, 6, NULL, 0, false, &list, &present) == DAEGUN_OK,
              "ligature_carets failed");
        size_t n = 0, np = 0;
        const double *c = daegun_f64_list_data(list, &n);
        const uint8_t *p = daegun_blob_data(present, &np);
        CHECK(n == 3 && np == 3 && c[0] == 250.0 && c[1] == 500.0 && c[2] == 750.0 && p[0] && p[1] && p[2],
              "f_f_l has %zu carets", n);
        daegun_f64_list_free(list);
        daegun_blob_free(present);
        list = NULL;
        CHECK(daegun_font_ligature_carets(carets, 5, NULL, 0, true, &list, NULL) == DAEGUN_OK,
              "ligature_carets failed");
        c = daegun_f64_list_data(list, &n);
        CHECK(n == 1 && c[0] == 0.0, "f_i's caret, point 1 at (420, 0), is not 0 down a vertical line");
        daegun_f64_list_free(list);
        list = NULL;
        CHECK(daegun_font_ligature_carets(carets, 1, NULL, 0, false, &list, NULL) == DAEGUN_OK,
              "ligature_carets failed");
        daegun_f64_list_data(list, &n);
        CHECK(n == 0, "f, no ligature, has %zu carets", n);
        daegun_f64_list_free(list);
        daegun_font_free(carets);
    }

    daegun_font *deva = open_font("assets/test-fonts/noto-devanagari/NotoSansDevanagari.ttf");
    if (deva) {
        daegun_run *run = NULL;
        CHECK(daegun_font_shape(deva, "\xE0\xA4\xBF", NULL, 0, false, &run) == DAEGUN_OK
                  && daegun_run_has_broken_syllable(run, &broken) == DAEGUN_OK && broken,
              "a vowel sign with no consonant is not a broken syllable");
        daegun_run_free(run);
        daegun_font_free(deva);
    }
}

static void math_glyph_info_answers(void)
{
    daegun_font *font = open_font("assets/test-fonts/stix-two-math/STIX2Math.otf");
    if (!font) {
        return;
    }
    uint16_t paren = 0, f = 0;
    daegun_font_glyph_id(font, '(', &paren);
    daegun_font_glyph_id(font, 0x1D453, &f);
    double v = -1.0;
    bool extended = false;
    CHECK(daegun_font_math_italics_correction(font, f, &v) == DAEGUN_OK && v == 20.0, "the italic f's correction is %g", v);
    CHECK(daegun_font_math_top_accent_attachment(font, f, &v) == DAEGUN_OK && v == 470.0, "its accent sits at %g", v);
    CHECK(daegun_font_math_is_extended_shape(font, paren, &extended) == DAEGUN_OK && extended, "( is not extended");
    CHECK(daegun_font_math_min_connector_overlap(font, &v) == DAEGUN_OK && v == 100.0, "the connector overlap is %g", v);
    CHECK(daegun_font_math_kern(font, 7, DAEGUN_MATH_KERN_TOP_RIGHT, 250.0, &v) == DAEGUN_OK && v == 32.0,
          "glyph 7's top right kern at 250 is %g", v);
    CHECK(daegun_font_math_kern(font, 22, DAEGUN_MATH_KERN_BOTTOM_LEFT, 250.0, &v) == DAEGUN_OK && v == -90.0,
          "glyph 22's bottom left kern at 250 is %g", v);
    CHECK(daegun_font_math_kern(font, 7, 4, 250.0, &v) == DAEGUN_RANGE, "corner 4 was accepted");

    daegun_math_construction *c = NULL;
    CHECK(daegun_font_math_glyph_variants(font, paren, true, &c) == DAEGUN_OK, "( has no vertical variants");
    size_t nv = 0, parts = 0;
    const uint16_t *vg = NULL, *pg = NULL;
    const double *va = NULL, *pv = NULL;
    double ic = -1.0;
    CHECK(daegun_math_construction_variants(c, &nv, &vg, &va) == DAEGUN_OK && nv == 13 && vg[0] == paren
              && va[0] == 933.0 && vg[12] == 1312 && va[12] == 3821.0,
          "( has %zu variants", nv);
    CHECK(daegun_math_construction_variants(c, NULL, &vg, &va) == DAEGUN_NULL, "a NULL count was accepted");
    CHECK(daegun_math_construction_assembly(c, &ic, &parts, &pg, &pv) == DAEGUN_OK && ic == 0.0 && parts == 3,
          "( assembles from %zu parts", parts);
    if (parts == 3) {
        static const double want[12] = { 0, 250, 1273, 0, 1000, 1000, 1252, 1, 250, 0, 1273, 0 };
        int same = pg[0] == 4862 && pg[1] == 4861 && pg[2] == 4860;
        for (int k = 0; k < 12; k++) {
            same &= pv[k] == want[k];
        }
        CHECK(same, "the parts of ( are not 4862, 4861 and 4860 as fontTools reads them");
    }
    daegun_math_construction_free(c);
    c = NULL;
    CHECK(daegun_font_math_glyph_variants(font, paren, false, &c) == DAEGUN_ABSENT && c == NULL,
          "( has horizontal variants");
    daegun_font_free(font);
}

/* fontTools' values: pairs first, last and deep in their lists, a default sequence, and two misses. */
static void variation_sequences_answer(void)
{
    daegun_font *font = open_font("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    if (!font) {
        return;
    }
    static const struct {
        uint32_t base, selector;
        uint16_t want;
    } found[4] = {
        { 0x4FAE, 0xFE00, 15200 }, { 0x2A61A, 0xE0101, 17423 }, { 0x9089, 0xE010E, 17243 }, { 0x2E6EA, 0xE0100, 16144 },
    };
    for (size_t i = 0; i < 4; i++) {
        uint16_t gid = 0;
        CHECK(daegun_font_variation_glyph_id(font, found[i].base, found[i].selector, &gid) == DAEGUN_OK
                  && gid == found[i].want,
              "U+%04X U+%04X gave %u, not %u", found[i].base, found[i].selector, gid, found[i].want);
    }
    uint16_t gid = 0;
    CHECK(daegun_font_variation_glyph_id(font, 0x4FAE, 0xE0104, &gid) == DAEGUN_ABSENT, "VS21 resolved U+4FAE");
    CHECK(daegun_font_variation_glyph_id(font, 0x4FAE, 0xFE0F, &gid) == DAEGUN_ABSENT, "VS16 resolved U+4FAE");
    daegun_font_free(font);
}

static void base_and_vertical_metrics_answer(void)
{
    daegun_font *font = open_font("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    if (!font) {
        return;
    }
    bool glyph_free = false;
    CHECK(daegun_font_base_is_glyph_free(font, &glyph_free) == DAEGUN_OK && glyph_free, "BASE needs glyphs");
    /* fontTools' values, icfb and icft moved by BASE's store at wght 900. */
    static const char *const tags[4] = { "icfb", "icft", "ideo", "romn" };
    static const double want[2][2][4] = {
        { { -67, 827, -120, 0 }, { -94, 854, -120, 0 } },
        { { 53, 947, 0, 120 }, { 26, 974, 0, 120 } },
    };
    const daegun_axis heavy = { "wght", 900 };
    for (int vertical = 0; vertical < 2; vertical++) {
        for (int at = 0; at < 2; at++) {
            daegun_text *def = NULL;
            daegun_str_list *names = NULL;
            daegun_f64_list *coords = NULL;
            CHECK(daegun_font_base_info(font, "hani", vertical, at ? &heavy : NULL, (size_t)at, &def, &names,
                                        &coords) == DAEGUN_OK,
                  "base_info failed");
            size_t n = 0;
            const double *c = daegun_f64_list_data(coords, &n);
            bool same = text_is(def, "ideo") && strings_are(names, tags, 4) && n == 4;
            for (size_t k = 0; same && k < 4; k++) {
                same = fabs(c[k] - want[vertical][at][k]) < 1e-9;
            }
            CHECK(same, "hani's %s baselines at %s are not where fontTools puts them",
                  vertical ? "vertical" : "horizontal", at ? "wght 900" : "the default");
            daegun_text_free(def);
            daegun_str_list_free(names);
            daegun_f64_list_free(coords);
        }
    }
    CHECK(daegun_font_base_info(font, "hani", false, NULL, 0, NULL, NULL, NULL) == DAEGUN_OK,
          "base_info refused NULL outs");
    bool has_min = true, has_max = true;
    CHECK(daegun_font_base_extents(font, "hani", NULL, NULL, false, NULL, 0, &has_min, NULL, &has_max, NULL)
              == DAEGUN_ABSENT,
          "Source Han's BASE has no MinMax, yet base_extents answered");
    uint16_t kan = 0;
    daegun_font_glyph_id(font, 0x6F22, &kan);
    uint32_t vadv = 0;
    int32_t vorig = 0, dvorig = 0;
    CHECK(daegun_font_vertical_advance(font, kan, NULL, 0, &vadv) == DAEGUN_OK && vadv == 1000,
          "U+6F22's vertical advance is %u", vadv);
    CHECK(daegun_font_vertical_origin(font, kan, NULL, 0, &vorig) == DAEGUN_OK && vorig == 880,
          "U+6F22's vertical origin is %d", vorig);
    CHECK(daegun_font_default_vertical_origin(font, &dvorig) == DAEGUN_OK && dvorig == 880,
          "VORG's default is %d", dvorig);
    daegun_font_free(font);
}

/* Scheherazade's JSTF names extenders and no priorities, so a priority level is added to Inter by
 * hand: one level whose extension enables GSUB lookup 0. */
static void justification_answers(void)
{
    daegun_font *arabic = open_font("assets/test-fonts/scheherazade-new/ScheherazadeNew-Regular.ttf");
    if (arabic) {
        daegun_u16_list *glyphs = NULL, *extenders = NULL;
        size_t ng = 0, ne = 0;
        CHECK(daegun_font_justification_glyphs(arabic, "arab", &glyphs) == DAEGUN_OK, "justification_glyphs failed");
        CHECK(daegun_font_justification_extenders(arabic, "arab", &extenders) == DAEGUN_OK, "extenders failed");
        const uint16_t *g = daegun_u16_list_data(glyphs, &ng);
        const uint16_t *e = daegun_u16_list_data(extenders, &ne);
        CHECK(ng == 2 && g[0] == 1313 && g[1] == 1315 && ne == 2 && e[0] == 1313 && e[1] == 1315,
              "arab's extenders are not 1313 and 1315");
        daegun_u16_list_free(glyphs);
        daegun_u16_list_free(extenders);
        daegun_jstf_priorities *none = NULL;
        CHECK(daegun_font_justification_priorities(arabic, "arab", NULL, &none) == DAEGUN_ABSENT && none == NULL,
              "priorities came back from a JSTF that names none");

        daegun_shape_options opts;
        daegun_shape_options_default(&opts);
        opts.report_tatweel_positions = true;
        daegun_run *run = NULL;
        size_t nt = 0;
        CHECK(daegun_font_shape_with_options(arabic, "\xD8\xA8\xD8\xA8", NULL, 0, false, &opts, &run) == DAEGUN_OK,
              "shaping two behs failed");
        const uint8_t *t = daegun_run_safe_to_insert_tatweel(run, &nt);
        CHECK(nt == 2 && t[0] && !t[1], "a tatweel fits between two behs at %zu places", nt);
        daegun_run_free(run);

        /* "(" alone is Common and shapes as default; seeded Arabic, as between Arabic words, it mirrors. */
        daegun_u32_list *runs = NULL;
        size_t rn = 0;
        CHECK(daegun_text_script_runs("\xD8\xB3\xD9\x84\xD8\xA7\xD9\x85", &runs) == DAEGUN_OK, "script runs failed");
        const uint32_t *rd = daegun_u32_list_data(runs, &rn);
        daegun_shape_options_default(&opts);
        opts.has_seed_script = rn >= 3;
        opts.seed_script = rn >= 3 ? (uint16_t)rd[2] : 0;
        daegun_u32_list_free(runs);
        daegun_run *seeded = NULL, *plain = NULL;
        size_t ns = 0, np = 0;
        daegun_str shaper = { NULL, 0 };
        CHECK(daegun_font_shape_with_options(arabic, "(", NULL, 0, false, &opts, &seeded) == DAEGUN_OK
                  && daegun_font_shape_with_options(arabic, "(", NULL, 0, false, NULL, &plain) == DAEGUN_OK,
              "shaping ( failed");
        const uint16_t *sg = daegun_run_glyphs(seeded, &ns), *pg = daegun_run_glyphs(plain, &np);
        CHECK(ns == 1 && np == 1 && sg[0] == 10 && pg[0] == 9, "( seeded Arabic is not the mirrored glyph");
        CHECK(daegun_run_shaper(seeded, &shaper) == DAEGUN_OK && shaper.len == 6 && memcmp(shaper.data, "arabic", 6) == 0,
              "( seeded Arabic did not reach the Arabic shaper");
        daegun_run_free(seeded);
        daegun_run_free(plain);
        daegun_font_free(arabic);
    }

    daegun_font *inter = open_font(INTER);
    if (!inter) {
        return;
    }
    static const uint8_t jstf[46] = {
        0, 1, 0, 0, 0, 1, 'l', 'a', 't', 'n', 0, 12,
        0, 0, 0, 6, 0, 0,
        0, 1, 0, 4,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1, 0, 0,
    };
    daegun_table_map *map = NULL;
    daegun_blob *built = NULL;
    daegun_font *font = NULL;
    size_t len = 0;
    CHECK(daegun_font_instance_tables(inter, NULL, 0, &map) == DAEGUN_OK
              && daegun_table_map_set(map, "JSTF", jstf, sizeof jstf) == DAEGUN_OK
              && daegun_table_map_build(map, &built) == DAEGUN_OK,
          "Inter with a JSTF table did not build");
    const uint8_t *bytes = daegun_blob_data(built, &len);
    CHECK(bytes && daegun_font_open(bytes, len, &font) == DAEGUN_OK, "Inter with a JSTF table does not open");
    daegun_blob_free(built);
    daegun_table_map_free(map);
    daegun_font_free(inter);
    if (!font) {
        return;
    }
    daegun_jstf_priorities *levels = NULL;
    size_t count = 0;
    const daegun_jstf_mods *mods = NULL;
    CHECK(daegun_font_justification_priorities(font, "latn", NULL, &levels) == DAEGUN_OK
              && daegun_jstf_priorities_count(levels, &count) == DAEGUN_OK && count == 1,
          "the JSTF table's %zu priority levels are not one", count);
    CHECK(daegun_jstf_priorities_at(levels, 0, &mods) == DAEGUN_OK && mods != NULL, "level 0 is missing");
    const daegun_jstf_mods *past = NULL;
    CHECK(daegun_jstf_priorities_at(levels, 1, &past) == DAEGUN_RANGE, "a second level was found");
    for (int32_t which = 0; which < 8; which++) {
        const uint16_t *lookups = NULL;
        size_t nl = 0;
        daegun_status st = daegun_jstf_mods_lookups(mods, which, &lookups, &nl);
        if (which == DAEGUN_JSTF_EXTENSION_ENABLE_GSUB) {
            CHECK(st == DAEGUN_OK && nl == 1 && lookups[0] == 0, "the extension does not enable GSUB lookup 0");
        } else {
            CHECK(st == DAEGUN_ABSENT, "list %d came back as %d from a level that names none", which, st);
        }
    }
    CHECK(daegun_jstf_mods_lookups(mods, 8, NULL, NULL) == DAEGUN_RANGE, "a ninth lookup list");
    for (int shrink = 0; shrink < 2; shrink++) {
        daegun_run *run = NULL;
        size_t ng = 0;
        CHECK(daegun_font_shape_justified(font, "Hi", NULL, 0, false, mods, shrink, &run) == DAEGUN_OK
                  && daegun_run_glyphs(run, &ng) != NULL && ng == 2,
              "shape_justified with shrink %d gave %zu glyphs", shrink, ng);
        daegun_run_free(run);
    }
    daegun_jstf_priorities_free(levels);
    daegun_font_free(font);
}

static void bidi_and_line_analysis_answer(void)
{
    const char *mixed = "abc \xD7\x90\xD7\x91\xD7\x92";
    daegun_font *font = open_font(INTER);
    if (font) {
        daegun_bidi_runs *runs = NULL;
        size_t n = 0, nc = 0, ng = 0;
        const daegun_run *run = NULL;
        uint8_t level = 9;
        const size_t *chars = NULL;
        CHECK(daegun_font_shape_bidi(font, mixed, NULL, 0, -1, &runs) == DAEGUN_OK
                  && daegun_bidi_runs_count(runs, &n) == DAEGUN_OK && n == 2,
              "abc and Hebrew shaped to %zu runs", n);
        CHECK(daegun_bidi_runs_at(runs, 0, &run, &level, &chars, &nc) == DAEGUN_OK && level == 0 && nc == 4
                  && chars[0] == 0 && chars[3] == 3,
              "the first run is at level %u over %zu characters", level, nc);
        const uint16_t *g = daegun_run_glyphs(run, &ng);
        CHECK(ng == 4 && g[0] == 507 && g[3] == 1777, "abc and the space shaped to %zu glyphs", ng);
        CHECK(daegun_bidi_runs_at(runs, 1, NULL, &level, &chars, &nc) == DAEGUN_OK && level == 1 && nc == 3
                  && chars[0] == 4 && chars[2] == 6,
              "the Hebrew run is at level %u over %zu characters", level, nc);
        CHECK(daegun_bidi_runs_at(runs, 2, NULL, NULL, NULL, NULL) == DAEGUN_RANGE, "a third run was found");
        daegun_bidi_runs_free(runs);

        daegun_shape_options opts;
        daegun_shape_options_default(&opts);
        runs = NULL;
        CHECK(daegun_font_shape_bidi_with(font, mixed, NULL, 0, 1, &opts, &runs) == DAEGUN_OK
                  && daegun_bidi_runs_count(runs, &n) == DAEGUN_OK && n == 2
                  && daegun_bidi_runs_at(runs, 1, NULL, &level, &chars, &nc) == DAEGUN_OK && level == 2
                  && nc == 3 && chars[0] == 0,
              "in a right-to-left paragraph abc is not the second run, at level 2");
        daegun_bidi_runs_free(runs);
        daegun_font_free(font);
    }

    daegun_bidi_paragraph *p = NULL;
    daegun_visual_runs *vr = NULL;
    uint8_t base = 9, level = 9;
    size_t n = 0, nc = 0;
    const size_t *chars = NULL;
    CHECK(daegun_text_bidi_paragraph(mixed, -1, &p) == DAEGUN_OK
              && daegun_bidi_paragraph_base_level(p, &base) == DAEGUN_OK && base == 0,
          "the paragraph's base level is %u", base);
    CHECK(daegun_text_line_visual_runs(p, 0, 7, &vr) == DAEGUN_OK && daegun_visual_runs_count(vr, &n) == DAEGUN_OK
              && n == 2,
          "the line has %zu visual runs", n);
    CHECK(daegun_visual_runs_at(vr, 1, &level, &chars, &nc) == DAEGUN_OK && level == 1 && nc == 3
              && chars[0] == 6 && chars[2] == 4,
          "the Hebrew run is not reversed");
    CHECK(daegun_visual_runs_at(vr, 2, NULL, NULL, NULL) == DAEGUN_RANGE, "a third visual run was found");
    daegun_visual_runs_free(vr);
    daegun_bidi_paragraph_free(p);
    p = NULL;
    CHECK(daegun_text_bidi_paragraph(mixed, 1, &p) == DAEGUN_OK
              && daegun_bidi_paragraph_base_level(p, &base) == DAEGUN_OK && base == 1,
          "a right-to-left paragraph has base level %u", base);
    daegun_bidi_paragraph_free(p);

    daegun_u32_list *words = NULL, *at = NULL;
    daegun_blob *mandatory = NULL;
    size_t nw = 0, na = 0, nm = 0;
    CHECK(daegun_text_word_boundaries("hi there", &words) == DAEGUN_OK, "word_boundaries failed");
    const uint32_t *w = daegun_u32_list_data(words, &nw);
    CHECK(nw == 4 && w[0] == 0 && w[1] == 2 && w[2] == 3 && w[3] == 8, "hi there has %zu word boundaries", nw);
    CHECK(daegun_text_line_break_opportunities("hi there\nyo", &at, &mandatory) == DAEGUN_OK, "line breaks failed");
    const uint32_t *a = daegun_u32_list_data(at, &na);
    const uint8_t *m = daegun_blob_data(mandatory, &nm);
    CHECK(na == 3 && nm == 3 && a[0] == 3 && a[1] == 9 && a[2] == 11 && !m[0] && m[1] && m[2],
          "the line breaks are not 3, then 9 and 11 demanded");
    daegun_u32_list_free(words);
    daegun_u32_list_free(at);
    daegun_blob_free(mandatory);

    CHECK(daegun_writing_mode_is_vertical(DAEGUN_WRITING_VERTICAL_RL)
              && daegun_writing_mode_is_vertical(DAEGUN_WRITING_VERTICAL_LR)
              && !daegun_writing_mode_is_vertical(DAEGUN_WRITING_HORIZONTAL),
          "the writing modes disagree on which are vertical");
}

/* Tables too small for a fixture, built here: a format 6 lookup, a morx state machine, an ankr and a
 * FeatureVariations record. The index map is Inter's own, against the Rust reader's answers. */
static void the_walkers_read_tables_built_by_hand(void)
{
    static const uint8_t single[20] = { 0, 6, 0, 4, 0, 2, 0, 8, 0, 1, 0, 0, 0, 3, 0, 30, 0, 7, 0, 70 };
    daegun_aat_lookup *lookup = NULL;
    uint16_t v = 0;
    CHECK(daegun_aat_lookup_open(single, sizeof single, 10, &lookup) == DAEGUN_OK, "the lookup did not open");
    CHECK(daegun_aat_lookup_value(lookup, 7, &v) == DAEGUN_OK && v == 70, "glyph 7 maps to %u", v);
    CHECK(daegun_aat_lookup_value(lookup, 5, &v) == DAEGUN_ABSENT, "glyph 5 maps to something");
    daegun_glyph_value_list *entries = NULL;
    size_t ne = 0;
    CHECK(daegun_aat_lookup_entries(lookup, &entries) == DAEGUN_OK, "lookup entries failed");
    const daegun_glyph_value *e = daegun_glyph_value_list_data(entries, &ne);
    CHECK(ne == 2 && e[0].glyph == 3 && e[0].value == 30 && e[1].glyph == 7 && e[1].value == 70,
          "the lookup lists %zu mappings", ne);
    daegun_glyph_value_list_free(entries);
    daegun_aat_lookup_free(lookup);

    /* Five classes over three glyphs. From the start state, class 4 moves to state 1 with flags
     * 0x8000 and its one extra word, 0x1234. */
    static const uint8_t machine[56] = {
        0, 0, 0, 5, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0, 44,
        0, 0, 0, 4, 0, 4, 0, 3,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
        0, 1, 0x80, 0, 0x12, 0x34,
    };
    daegun_aat_state_table *table = NULL;
    uint16_t k = 0;
    daegun_aat_entry cell;
    memset(&cell, 0xff, sizeof cell);
    CHECK(daegun_aat_state_table_open(machine, sizeof machine, 1, 3, &table) == DAEGUN_OK, "the state table did not open");
    CHECK(daegun_aat_state_table_class(table, 1, &k) == DAEGUN_OK && k == 4, "glyph 1 is class %u", k);
    CHECK(daegun_aat_state_table_class(table, 3, &k) == DAEGUN_OK && k == 1, "a glyph past the font is class %u", k);
    CHECK(daegun_aat_state_table_class(table, 0xFFFF, &k) == DAEGUN_OK && k == DAEGUN_AAT_CLASS_DELETED_GLYPH,
          "the deleted glyph is class %u", k);
    CHECK(daegun_aat_state_table_entry(table, DAEGUN_AAT_STATE_START_OF_TEXT, 4, &cell) == DAEGUN_OK
              && cell.new_state == 1 && cell.flags == 0x8000 && cell.word1 == 0x1234 && cell.word2 == 0,
          "the cell reads state %u, flags %x, words %x and %x", cell.new_state, cell.flags, cell.word1, cell.word2);
    CHECK(daegun_aat_state_table_entry(table, 0, 5, &cell) == DAEGUN_RANGE, "a sixth class was read");
    CHECK(daegun_aat_state_table_entry(table, 9, 0, &cell) == DAEGUN_RANGE, "a tenth state was read");
    daegun_aat_state_table_free(table);

    /* Glyph 1 has two anchors, (10, 20) and (-30, 40); glyphs 0 and 2 have none. */
    static const uint8_t anchors[36] = {
        0, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 20,
        0, 0, 0, 12, 0, 0, 0, 12,
        0, 0, 0, 2, 0, 10, 0, 20, 0xFF, 0xE2, 0, 40,
        0, 0, 0, 0,
    };
    daegun_ankr *ankr = NULL;
    uint32_t points = 9;
    int16_t x = 0, y = 0;
    CHECK(daegun_ankr_open(anchors, sizeof anchors, 3, &ankr) == DAEGUN_OK, "the ankr did not open");
    CHECK(daegun_ankr_point_count(ankr, 1, &points) == DAEGUN_OK && points == 2, "glyph 1 has %u anchors", points);
    CHECK(daegun_ankr_point_count(ankr, 0, &points) == DAEGUN_OK && points == 0, "glyph 0 has %u anchors", points);
    CHECK(daegun_ankr_anchor_point(ankr, 1, 1, &x, &y) == DAEGUN_OK && x == -30 && y == 40,
          "glyph 1's second anchor is (%d, %d)", x, y);
    CHECK(daegun_ankr_anchor_point(ankr, 1, 2, &x, &y) == DAEGUN_ABSENT, "glyph 1 has a third anchor");
    daegun_ankr_free(ankr);

    /* One record: axis 0 from 0.5 to 1.0, 8192 to 16384 in F2Dot14, swaps feature 3 for the feature
     * table at byte 42. */
    static const uint8_t variations[46] = {
        0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0, 30,
        0, 1, 0, 0, 0, 6,
        0, 1, 0, 0, 0x20, 0, 0x40, 0,
        0, 1, 0, 0, 0, 1, 0, 3, 0, 0, 0, 12,
        0, 0, 0, 0,
    };
    daegun_feature_variations *fv = NULL;
    uint16_t record = 9;
    size_t alternate = 0;
    const int32_t inside[1] = { 12000 }, outside[1] = { 4000 };
    CHECK(daegun_feature_variations_at(variations, sizeof variations, 0, &fv) == DAEGUN_OK, "feature_variations_at failed");
    CHECK(daegun_feature_variations_find(fv, inside, 1, &record) == DAEGUN_OK && record == 0,
          "0.73 matched record %u", record);
    CHECK(daegun_feature_variations_find(fv, outside, 1, &record) == DAEGUN_ABSENT, "0.24 matched a record");
    CHECK(daegun_feature_variations_substitute(fv, 0, 3, &alternate) == DAEGUN_OK && alternate == 42,
          "feature 3's alternate is at %zu", alternate);
    CHECK(daegun_feature_variations_substitute(fv, 0, 2, &alternate) == DAEGUN_ABSENT, "feature 2 has an alternate");
    daegun_feature_variations_free(fv);

    daegun_font *font = open_font(INTER);
    if (!font) {
        return;
    }
    daegun_bytes hvar = { NULL, 0 };
    uint32_t map_at = 0;
    daegun_delta_set_index_map *map = NULL;
    size_t count = 0, outer = 0, inner = 0;
    CHECK(daegun_font_table(font, "HVAR", &hvar) == DAEGUN_OK && daegun_read_u32_be(hvar.data, hvar.len, 8, &map_at) == DAEGUN_OK
              && daegun_delta_set_index_map_parse(hvar.data, hvar.len, map_at, &map) == DAEGUN_OK,
          "HVAR's advance map did not parse");
    CHECK(daegun_delta_set_index_map_count(map, &count) == DAEGUN_OK && count == 2937, "the map has %zu entries", count);
    CHECK(daegun_delta_set_index_map_lookup(map, 2, &outer, &inner) == DAEGUN_OK && outer == 3 && inner == 46,
          "A maps to (%zu, %zu)", outer, inner);
    CHECK(daegun_delta_set_index_map_lookup(map, 5000, &outer, &inner) == DAEGUN_OK && outer == 1 && inner == 302,
          "past the end maps to (%zu, %zu), not the last entry", outer, inner);
    daegun_delta_set_index_map_free(map);
    daegun_font_free(font);
}

/* Runs after main returns, when the thread's storage may already be gone: a reason is read and set
 * there, and neither may abort the process. */
static void last_error_at_exit(void)
{
    (void)daegun_last_error();
    daegun_font *junk = NULL;
    static const uint8_t bytes[16] = {0xAB};
    (void)daegun_font_open(bytes, sizeof bytes, &junk);
    (void)daegun_last_error();
}

int main(int argc, char **argv)
{
    atexit(last_error_at_exit);
    const char *font = argc > 1 ? argv[1] : "assets/test-fonts/inter/InterVariable.ttf";

    printf("daegun C round trip\n");
    abi_version_agrees();
    null_is_refused_not_dereferenced();
    bad_font_data_is_reported();
    zeroed_options_are_the_defaults();
    ttc_count_answers_for_a_plain_font(font);
    a_real_font_answers(font);
    an_owned_buffer_opens_without_a_copy(font);
    metrics_answer_and_free_cleanly(font);
    a_subset_is_a_font(font);
    the_pen_draws_and_reentry_is_safe(font);
    shaping_produces_positioned_glyphs(font);
    layout_wraps_and_borrows(font);
    text_analysis_needs_no_font();
    the_paint_graph_is_walkable("assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf");
    math_constants_are_indexed("assets/test-fonts/stix-two-math/STIX2Math.otf");
    the_readers_are_bounds_checked();
    raw_tables_round_trip(font);
    paths_build_stroke_and_replay();
    glyph_quads_come_back_as_floats(font);
    the_subpixel_filter_is_readable();
    a_prepared_outline_reaches_the_pen(font);
    a_hinted_glyph_is_readable_from_c();
    a_prepared_stroke_past_the_cap_is_range();
    null_with_a_count_is_null(font);
    a_stroke_that_overflows_is_refused();
    the_remaining_readers_answer();
    a_scene_holds_each_outline_once_within_a_bound();
    paths_flatten_and_resolve();
    a_path_can_draw_into_itself();
    compositing_follows_colr();
    a_color_scene_is_walkable("assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf");
    a_padded_gradient_holds_its_end_colors("assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf");
    character_properties_need_no_font();
    the_format_walkers_read_a_real_table(font);
    loca_and_glyf_are_walkable(font);
    the_atlas_packer_packs();
    the_rules_a_caller_would_get_wrong(font);
    scripts_answer_about_themselves();
    subsetting_maps_glyph_ids(font);
    stat_values_are_readable(font);
    a_face_describes_itself();
    glyphs_and_runs_answer();
    math_glyph_info_answers();
    base_and_vertical_metrics_answer();
    variation_sequences_answer();
    justification_answers();
    bidi_and_line_analysis_answer();
    the_walkers_read_tables_built_by_hand();

    if (failures == 0) {
        printf("  ok\n");
        return 0;
    }
    printf("  %d failed\n", failures);
    return 1;
}
