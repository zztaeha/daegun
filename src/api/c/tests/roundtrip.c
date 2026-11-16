#include "daegun.h"

#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Rule 2's checks pass NULL where a pointer is required, as a caller unwarned may, and clang warns of each. */
#ifdef __clang__
#pragma clang diagnostic ignored "-Wnonnull"
#endif

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

/* A string literal as text and its byte count, as every call taking text reads it. */
#define TXT(s) (s), (sizeof(s) - 1)

#define INTER "assets/test-fonts/inter/InterVariable.ttf"
#define STIX "assets/test-fonts/stix-two-math/STIX2Math.otf"
#define COLR "assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf"

static const daegun_tag WGHT = DAEGUN_TAG('w', 'g', 'h', 't');

/* NULL where the compiler can't see it, for the calls that must refuse one with a length: GCC rightly warns of a
 * literal NULL passed with a nonzero length. */
static void *volatile hidden_null;
#define HIDDEN_NULL ((void *)hidden_null)

/* Paths are the engine root's: DAEGUN_ROOT names it for a runner starting elsewhere, as the iOS Simulator does. */
static uint8_t *slurp(const char *path, size_t *len)
{
    char rooted[4096];
    const char *root = getenv("DAEGUN_ROOT");
    if (root && path[0] != '/' && snprintf(rooted, sizeof rooted, "%s/%s", root, path) < (int)sizeof rooted) {
        path = rooted;
    }
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

static int str_is(daegun_str s, const char *want)
{
    return s.data && s.len == strlen(want) && memcmp(s.data, want, s.len) == 0 && s.data[s.len] == '\0';
}

static int text_is(const daegun_text *t, const char *want)
{
    daegun_str s = { NULL, 0 };
    return daegun_text_str(t, &s) == DAEGUN_OK && str_is(s, want);
}

static void abi_version_agrees(void)
{
    uint32_t got = daegun_abi_version();
    CHECK(got == DAEGUN_ABI_VERSION && DAEGUN_ABI_COMPATIBLE(got),
          "library reports ABI %u.%u.%u, this header is %u.%u.%u", DAEGUN_ABI_MAJOR(got), DAEGUN_ABI_MINOR(got),
          DAEGUN_ABI_PATCH(got), DAEGUN_ABI_MAJOR(DAEGUN_ABI_VERSION), DAEGUN_ABI_MINOR(DAEGUN_ABI_VERSION),
          DAEGUN_ABI_PATCH(DAEGUN_ABI_VERSION));
#if !DAEGUN_ABI_COMPATIBLE(DAEGUN_ABI_VERSION)
#error "a header is compatible with its own ABI"
#endif
    daegun_str release = daegun_version();
    CHECK(release.len > 0 && strlen(release.data) == release.len && release.data[0] >= '0' && release.data[0] <= '9',
          "the library's release reads %s", release.data);
}

/* Rule 2: NULL is an answer, never a crash. This is the test that a sanitizer cannot write for us:
 * it has to be attempted deliberately, because no correct program does it. */
static void null_is_refused_not_dereferenced(void)
{
    daegun_font *font = NULL;
    uint32_t out = 0;
    uint16_t upm = 0;

    CHECK(daegun_font_open(HIDDEN_NULL, 4, &font) == DAEGUN_NULL, "null data was not refused");
    CHECK(daegun_font_glyph_id(NULL, 'A', &out) == DAEGUN_NULL, "null font was not refused");
    CHECK(daegun_font_glyph_count(NULL, &out) == DAEGUN_NULL, "null font was not refused");
    CHECK(daegun_font_units_per_em(NULL, &upm) == DAEGUN_NULL, "null font was not refused");

    daegun_font_free(NULL);
}

static void bad_font_data_is_reported(void)
{
    uint8_t junk[64];
    memset(junk, 0xab, sizeof junk);
    daegun_font *font = (daegun_font *)junk;

    CHECK(daegun_font_open(junk, sizeof junk, &font) == DAEGUN_MALFORMED, "64 bytes of 0xab opened as a font");
    CHECK(font == NULL, "a failed open left its handle out set");

    daegun_str err = daegun_last_error();
    CHECK(err.len > 0, "a failed open left no reason");
    CHECK(err.data != NULL && strlen(err.data) == err.len,
          "the reason's length disagrees with strlen, so daegun_str's NUL promise is broken");
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

/* The font at path with its table `tag` set to data, or left out where data is NULL. */
static daegun_font *open_font_with(const char *path, daegun_tag tag, const uint8_t *data, size_t len)
{
    daegun_font *font = open_font(path);
    if (!font) {
        return NULL;
    }
    daegun_tag t[128];
    size_t n = 0;
    CHECK(daegun_font_table_tags(font, t, 128, &n) == DAEGUN_OK, "table_tags failed");
    daegun_table_map *map = daegun_table_map_new();
    for (size_t i = 0; i < n; i++) {
        daegun_bytes bytes = { NULL, 0 };
        if (daegun_font_table(font, t[i], &bytes) == DAEGUN_OK) {
            CHECK(daegun_table_map_set(map, t[i], bytes.data, bytes.len) == DAEGUN_OK, "table_map_set failed");
        }
    }
    if (data) {
        CHECK(daegun_table_map_set(map, tag, data, len) == DAEGUN_OK, "table_map_set failed");
    } else {
        CHECK(daegun_table_map_remove(map, tag) == DAEGUN_OK, "table_map_remove failed");
    }
    daegun_blob *built = NULL;
    CHECK(daegun_table_map_build(map, &built) == DAEGUN_OK, "building %s with a table changed failed", path);
    daegun_table_map_free(map);
    daegun_font_free(font);
    size_t built_len = 0;
    const uint8_t *built_data = daegun_blob_data(built, &built_len);
    daegun_font *changed = NULL;
    CHECK(built_data && daegun_font_open(built_data, built_len, &changed) == DAEGUN_OK,
          "%s with a table changed does not open", path);
    daegun_blob_free(built);
    return changed;
}

static daegun_location *location_at(const daegun_font *font, daegun_tag tag, double value)
{
    daegun_coord coord = { tag, value };
    daegun_location *loc = NULL;
    CHECK(daegun_font_location(font, &coord, 1, &loc) == DAEGUN_OK, "a location failed: %s", daegun_last_error().data);
    return loc;
}

static void a_real_font_answers(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    uint16_t upm = 0;
    uint32_t glyphs = 0, gid = 0;
    CHECK(daegun_font_units_per_em(font, &upm) == DAEGUN_OK && upm == 2048, "Inter's em is %u units", upm);
    CHECK(daegun_font_glyph_count(font, &glyphs) == DAEGUN_OK && glyphs > 1, "a face of %u glyphs", glyphs);

    CHECK(daegun_font_glyph_id(font, 'A', &gid) == DAEGUN_OK, "no glyph for 'A'");
    CHECK(gid != 0 && gid < glyphs, "'A' mapped to glyph %u of %u", gid, glyphs);

    uint32_t none = 0xffff;
    CHECK(daegun_font_glyph_id(font, 0x10FFFD, &none) == DAEGUN_ABSENT && none == 0xffff,
          "an unmapped character answered, or wrote its out");
    CHECK(daegun_font_glyph_id(font, 0xD800, &none) == DAEGUN_INVALID_ARGUMENT && none == 0xffff,
          "a surrogate was taken for a character");
    CHECK(daegun_font_glyph_id(font, 0x110000, &none) == DAEGUN_INVALID_ARGUMENT, "U+110000 was taken");

    /* The three cache budgets, and the instance cache filling as a location is used. */
    daegun_cache_stats stats = { 1, 1 };
    CHECK(daegun_font_set_cache_budget(font, DAEGUN_CACHE_OUTLINE, 64 * 1024) == DAEGUN_OK, "outline budget failed");
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_OUTLINE, &stats) == DAEGUN_OK && stats.entries == 0,
          "a fresh font's outline cache held %zu entries", stats.entries);
    CHECK(daegun_font_set_cache_budget(font, DAEGUN_CACHE_SHAPE, 32 * 1024) == DAEGUN_OK, "shape budget failed");
    CHECK(daegun_font_clear_cache(font, DAEGUN_CACHE_SHAPE) == DAEGUN_OK, "clearing shapes failed");
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_SHAPE, &stats) == DAEGUN_OK && stats.entries == 0
              && stats.bytes == 0,
          "the shape cache held %zu entries after clearing", stats.entries);
    CHECK(daegun_font_set_cache_budget(font, DAEGUN_CACHE_INSTANCE, 1024 * 1024) == DAEGUN_OK,
          "instance budget failed");
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_INSTANCE, &stats) == DAEGUN_OK && stats.entries == 0,
          "a fresh font held %zu locations", stats.entries);
    daegun_location *bold = location_at(font, WGHT, 700.0);
    daegun_shaped_run *at_bold = NULL, *empty = NULL;
    CHECK(daegun_font_shape(font, TXT("a"), bold, NULL, &at_bold) == DAEGUN_OK, "shaping at a weight failed");
    daegun_shaped_run_free(at_bold);
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_INSTANCE, &stats) == DAEGUN_OK && stats.entries == 1
              && stats.bytes > 0,
          "one weight held %zu locations of %zu bytes", stats.entries, stats.bytes);
    CHECK(daegun_font_shape(font, "", 0, NULL, NULL, &empty) == DAEGUN_OK, "shaping no text failed");
    daegun_shaped_run_free(empty);
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_SHAPE, &stats) == DAEGUN_OK && stats.entries == 2
              && stats.bytes > 0,
          "two runs held %zu entries of %zu bytes", stats.entries, stats.bytes);
    CHECK(daegun_font_set_cache_budget(font, DAEGUN_CACHE_SHAPE, 0) == DAEGUN_OK
              && daegun_font_cache_stats(font, DAEGUN_CACHE_SHAPE, &stats) == DAEGUN_OK && stats.entries == 0,
          "a budget of 0 kept %zu runs", stats.entries);
    CHECK(daegun_font_set_cache_budget(font, 4, 0) == DAEGUN_INVALID_ARGUMENT, "cache 4 was taken");
    CHECK(daegun_font_cache_stats(font, -1, &stats) == DAEGUN_INVALID_ARGUMENT, "cache -1 was taken");
    daegun_location_free(bold);

    size_t allowance = 0;
    CHECK(daegun_font_set_index_allowance(font, 4321) == DAEGUN_OK
              && daegun_font_index_allowance(font, &allowance) == DAEGUN_OK && allowance == 4321,
          "the index allowance read back as %zu", allowance);

    CHECK(daegun_font_set_cache_budget(NULL, DAEGUN_CACHE_SHAPE, 0) == DAEGUN_NULL, "a null font's budget was set");
    CHECK(daegun_font_set_index_allowance(NULL, 0) == DAEGUN_NULL, "a null font's allowance was set");
    CHECK(daegun_font_index_allowance(NULL, &allowance) == DAEGUN_NULL, "a null font's allowance was read");

    daegun_font_free(font);
}

/* A feature over a range of characters, a stated direction and the not-found glyphs reach the shaper: Inter with
 * zero over [1:2], right to left, and a not-found glyph of 5. */
static void shape_options_reach_the_shaper(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    const daegun_feature middle = { DAEGUN_TAG('z', 'e', 'r', 'o'), 1, 1, 2 };
    const daegun_feature every = { DAEGUN_TAG('z', 'e', 'r', 'o'), 1, 0, 0 };
    daegun_shape_options opts = { 0 };
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    daegun_shaped_run *ranged = NULL, *whole = NULL;
    opts.features = &middle;
    opts.features_len = 1;
    CHECK(daegun_font_shape(font, TXT("000"), NULL, &opts, &ranged) == DAEGUN_OK, "shaping with zero failed");
    opts.features = &every;
    CHECK(daegun_font_shape(font, TXT("000"), NULL, &opts, &whole) == DAEGUN_OK, "shaping with zero failed");
    size_t nr = 0, nw = 0;
    const uint32_t *r = daegun_shaped_run_glyphs(ranged, &nr), *w = daegun_shaped_run_glyphs(whole, &nw);
    CHECK(nr == 3 && r[0] == 1341 && r[1] == 1353 && r[2] == 1341, "zero over the second character gave %zu", nr);
    CHECK(nw == 3 && w[0] == 1353 && w[1] == 1353 && w[2] == 1353, "zero with an end of 0 gave %zu glyphs", nw);
    daegun_shaped_run_free(ranged);
    daegun_shaped_run_free(whole);

    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.direction = DAEGUN_DIRECTION_RTL;
    daegun_shaped_run *rtl = NULL;
    CHECK(daegun_font_shape(font, TXT("(1)"), NULL, &opts, &rtl) == DAEGUN_OK, "rtl shaping failed");
    size_t ng = 0, nc = 0;
    const uint32_t *g = daegun_shaped_run_glyphs(rtl, &ng);
    const uint32_t *c = daegun_shaped_run_clusters(rtl, &nc);
    CHECK(ng == 3 && nc == 3 && g[0] == 1446 && g[2] == 1447 && c[0] == 2 && c[2] == 0,
          "(1) right to left gave %zu glyphs", ng);
    daegun_shaped_run_free(rtl);

    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.not_found_glyph = 5;
    opts.has_not_found_variation_selector_glyph = true;
    opts.not_found_variation_selector_glyph = 6;
    daegun_shaped_run *missing = NULL, *selector = NULL;
    CHECK(daegun_font_shape(font, TXT("a\xF0\x9E\x9B\x80" "b"), NULL, &opts, &missing) == DAEGUN_OK
              && daegun_font_shape(font, TXT("a\xEF\xB8\x80" "b"), NULL, &opts, &selector) == DAEGUN_OK,
          "shaping with not-found glyphs failed");
    const uint32_t *m = daegun_shaped_run_glyphs(missing, &ng);
    CHECK(ng == 3 && m[1] == 5, "U+1E6C0 the font lacks gave %u", ng == 3 ? m[1] : 0);
    const uint32_t *v = daegun_shaped_run_glyphs(selector, &ng);
    CHECK(ng == 3 && v[1] == 6, "U+FE00 the font can't apply gave %u", ng == 3 ? v[1] : 0);
    daegun_shaped_run_free(missing);
    daegun_shaped_run_free(selector);

    /* A code naming none of its constants is refused, not guessed at. */
    daegun_shaped_run *refused = (daegun_shaped_run *)&opts;
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.direction = 5;
    CHECK(daegun_font_shape(font, TXT("a"), NULL, &opts, &refused) == DAEGUN_INVALID_ARGUMENT && refused == NULL,
          "direction 5 was taken, or its handle out left set");
    opts.direction = DAEGUN_DIRECTION_DEFAULT;
    opts.seed_script = DAEGUN_TAG('Q', 'q', 'q', 'q');
    CHECK(daegun_font_shape(font, TXT("a"), NULL, &opts, &refused) == DAEGUN_INVALID_ARGUMENT,
          "a seed script no ISO 15924 code names was taken");
    daegun_font_free(font);
}

/* Each options struct's _default: zero for those whose defaults are zero, the defaults otherwise, its size set,
 * and a size below 2.0's or past this library's refused, writing nothing. */
static void options_defaults_are_versioned(void)
{
    daegun_outline_options outline = { 0 }, zeroed = { 0 };
    memset(&outline, 0xcd, sizeof outline);
    memset(&zeroed, 0, sizeof zeroed);
    zeroed.size = sizeof zeroed;
    CHECK(daegun_outline_options_default(&outline, sizeof outline) == DAEGUN_OK
              && memcmp(&outline, &zeroed, sizeof outline) == 0,
          "outline options' defaults are not zero but for their size");

    daegun_shape_options shape = { 0 };
    daegun_shape_options shape_zero = { 0 };
    memset(&shape, 0xcd, sizeof shape);
    memset(&shape_zero, 0, sizeof shape_zero);
    shape_zero.size = sizeof shape_zero;
    CHECK(daegun_shape_options_default(&shape, sizeof shape) == DAEGUN_OK
              && memcmp(&shape, &shape_zero, sizeof shape) == 0,
          "shape options' defaults are not zero but for their size");

    daegun_justify_options justify = { 0 };
    CHECK(daegun_justify_options_default(&justify, sizeof justify) == DAEGUN_OK && justify.size == sizeof justify
              && justify.script == 0 && justify.target_width == 0.0,
          "justify options' defaults are not zero");

    daegun_layout_options layout = { 0 };
    CHECK(daegun_layout_options_default(&layout, sizeof layout) == DAEGUN_OK, "layout defaults failed");
    CHECK(isinf(layout.max_inline_size) && layout.max_inline_size > 0, "the default measure is %g, so it wraps",
          layout.max_inline_size);
    CHECK(layout.tab_size == 8.0 && layout.base_direction == DAEGUN_BASE_AUTO && layout.segmenter == NULL
              && layout.size == sizeof layout,
          "layout's defaults are not a tab of 8, an automatic direction and no segmenter");

    daegun_scene_options scene = { 0 };
    CHECK(daegun_scene_options_default(&scene, sizeof scene) == DAEGUN_OK && scene.foreground.a == 255
              && scene.foreground.r == 0 && scene.palette == 0,
          "the scene's default foreground is not opaque black");

    /* A struct from a newer header, larger than this library knows, and one cut below 2.0's. */
    unsigned char bigger[sizeof(daegun_shape_options) + 8];
    memset(bigger, 0xcd, sizeof bigger);
    CHECK(daegun_shape_options_default((daegun_shape_options *)(void *)bigger, sizeof bigger) == DAEGUN_UNSUPPORTED
              && bigger[0] == 0xcd,
          "a struct larger than this library knows was written");
    CHECK(daegun_shape_options_default(&shape, sizeof shape - 1) == DAEGUN_INVALID_ARGUMENT,
          "a struct below 2.0's size was taken");
    CHECK(daegun_outline_options_default(NULL, sizeof outline) == DAEGUN_NULL, "null options were not refused");

    /* The same rule where a call reads one: a NULL is the defaults, and a size past this library's or below 2.0's
     * refused. */
    daegun_font *font = open_font(INTER);
    if (!font) return;
    daegun_shaped_run *run = NULL;
    CHECK(daegun_font_shape(font, TXT("a"), NULL, NULL, &run) == DAEGUN_OK, "NULL options were not the defaults");
    daegun_shaped_run_free(run);
    run = NULL;
    shape.size = sizeof shape + 8;
    CHECK(daegun_font_shape(font, TXT("a"), NULL, &shape, &run) == DAEGUN_UNSUPPORTED && run == NULL,
          "options claiming more bytes than this library knows were read");
    CHECK(daegun_last_error().len > 0, "the refusal gave no reason");
    shape.size = 0;
    CHECK(daegun_font_shape(font, TXT("a"), NULL, &shape, &run) == DAEGUN_INVALID_ARGUMENT && run == NULL,
          "a zeroed struct's size was read as 2.0's");
    daegun_font_free(font);
}

static void a_font_file_is_one_face(const char *path)
{
    size_t len = 0;
    uint8_t *bytes = slurp(path, &len);
    if (!bytes) {
        CHECK(0, "could not read %s", path);
        return;
    }
    size_t n = 12345;
    CHECK(daegun_face_count(bytes, len, &n) == DAEGUN_OK && n == 1, "a plain .ttf reported %zu faces", n);
    free(bytes);
}

static void metrics_answer_and_free_cleanly(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    double asc = 0, desc = 0;
    CHECK(daegun_font_ascender(font, NULL, &asc) == DAEGUN_OK && daegun_font_descender(font, NULL, &desc) == DAEGUN_OK,
          "ascender or descender failed");
    CHECK(asc == 1984.0 && desc == -494.0, "Inter's ascender %g and descender %g in font units", asc, desc);

    daegun_line_metrics line = { 0 };
    double height = 0;
    CHECK(daegun_font_line_metrics(font, NULL, DAEGUN_HORIZONTAL, &line) == DAEGUN_OK
              && daegun_line_metrics_line_height(&line, &height) == DAEGUN_OK
              && height == line.ascent - line.descent + line.line_gap,
          "a line's height is not its ascent less its descent, with its gap");
    CHECK(daegun_font_line_metrics(font, NULL, 2, &line) == DAEGUN_INVALID_ARGUMENT, "orientation 2 was taken");

    daegun_text *family = NULL;
    CHECK(daegun_font_family_name(font, NULL, &family) == DAEGUN_OK && text_is(family, "Inter Variable"),
          "Inter's family is not Inter Variable");
    daegun_text_free(family);

    daegun_bounds box = { 0 };
    CHECK(daegun_font_bounds(font, &box) == DAEGUN_OK && box.x_min < box.x_max && box.y_min < box.y_max,
          "the font's box is inside out");

    daegun_names *names = NULL;
    CHECK(daegun_font_names(font, NULL, &names) == DAEGUN_OK, "names failed");
    size_t name_count = 0;
    CHECK(daegun_names_count(names, &name_count) == DAEGUN_OK && name_count > 10, "%zu names", name_count);
    uint16_t name_id = 0, last_id = 0;
    daegun_str name_text = { NULL, 0 };
    for (size_t i = 0; i < name_count; i++) {
        CHECK(daegun_names_at(names, i, &name_id, &name_text) == DAEGUN_OK && (i == 0 || name_id > last_id)
                  && name_text.data[name_text.len] == '\0',
              "name %zu, ID %u, out of order or without its NUL", i, name_id);
        last_id = name_id;
    }
    CHECK(daegun_names_at(names, 1, &name_id, &name_text) == DAEGUN_OK && name_id == 1
              && strcmp(name_text.data, "Inter Variable") == 0,
          "name 1 is %s", name_text.data);
    uint16_t kept_id = 77;
    daegun_str kept = { NULL, 0 };
    CHECK(daegun_names_at(names, name_count, &kept_id, &kept) == DAEGUN_INVALID_ARGUMENT && kept_id == 77
              && kept.data == NULL,
          "reading past the end was not DAEGUN_INVALID_ARGUMENT, or wrote its outs");
    daegun_names_free(names);

    int32_t style = -1;
    CHECK(daegun_font_style(font, NULL, &style) == DAEGUN_OK && style == DAEGUN_STYLE_NORMAL,
          "Inter's default instance is style %d", style);

    bool variable = false;
    CHECK(daegun_font_is_variable(font, &variable) == DAEGUN_OK && variable, "Inter is not variable");
    daegun_axis axis[4];
    size_t n = 0;
    CHECK(daegun_font_axes(font, axis, 4, &n) == DAEGUN_OK, "axes failed");
    CHECK(n == 2 && axis[0].tag == DAEGUN_TAG('o', 'p', 's', 'z') && axis[1].tag == WGHT,
          "Inter has %zu axes, not opsz and wght", n);
    for (size_t i = 0; i < n; i++) {
        CHECK(axis[i].min <= axis[i].default_value && axis[i].default_value <= axis[i].max,
              "axis %zu's default lies outside its range", i);
        CHECK(axis[i].name_id > 255, "axis %zu names its display name with ID %u", i, axis[i].name_id);
    }

    daegun_font_free(font);
}

static void math_constants_answer(void)
{
    daegun_font *font = open_font(STIX);
    if (!font) return;
    daegun_math_constants constants = { 0 };
    CHECK(daegun_font_math_constants(font, NULL, &constants) == DAEGUN_OK, "STIX has no MATH constants");
    CHECK(constants.axis_height == 258.0 && constants.script_percent_scale_down == 70.0,
          "STIX's axis height is %g and its script scale %g%%", constants.axis_height,
          constants.script_percent_scale_down);
    daegun_font_free(font);

    daegun_font *plain = open_font(INTER);
    if (!plain) return;
    constants.axis_height = -1.0;
    CHECK(daegun_font_math_constants(plain, NULL, &constants) == DAEGUN_ABSENT && constants.axis_height == -1.0,
          "a font without MATH gave constants, or wrote them");
    daegun_font_free(plain);
}

typedef struct { int moves, lines, quads, curves, closes; } pen_tally;
static void tally_move(void *u, float x, float y) { (void)x; (void)y; ((pen_tally *)u)->moves++; }
static void tally_line(void *u, float x, float y) { (void)x; (void)y; ((pen_tally *)u)->lines++; }
static void tally_quad(void *u, float a, float b, float c, float d)
{ (void)a; (void)b; (void)c; (void)d; ((pen_tally *)u)->quads++; }
static void tally_curve(void *u, float a, float b, float c, float d, float e, float f)
{ (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; ((pen_tally *)u)->curves++; }
static void tally_close(void *u) { ((pen_tally *)u)->closes++; }
static daegun_pen tally_pen(pen_tally *t)
{
    daegun_pen pen = { tally_move, tally_line, tally_quad, tally_curve, tally_close, t };
    return pen;
}

/* The tally first, so the tally callbacks count for it as for a bare pen_tally. */
struct pen_state {
    pen_tally tally;
    const daegun_font *font;
    uint32_t gid;
    const daegun_location *at;
    int reentered_ok;
};

/* Calls that lock what the drawing call might still hold: the outline cache, read and then written, the decoder's
 * scratch space through a glyph drawn from inside the callback, and the thread's slots, let go of mid-call. */
static void on_close(void *u)
{
    struct pen_state *st = u;
    st->tally.closes++;
    daegun_cache_stats stats = { 0 };
    daegun_pen inner = { 0 };
    daegun_thread_release();
    if (daegun_font_cache_stats(st->font, DAEGUN_CACHE_OUTLINE, &stats) == DAEGUN_OK
        && daegun_font_set_cache_budget(st->font, DAEGUN_CACHE_OUTLINE, 4u << 20) == DAEGUN_OK
        && daegun_font_outline(st->font, st->gid, st->at, &inner) == DAEGUN_OK) {
        st->reentered_ok++;
    }
}

static void the_pen_draws_and_reentry_is_safe(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    uint32_t gid = 0;
    if (daegun_font_glyph_id(font, 'B', &gid) != DAEGUN_OK) { daegun_font_free(font); return; }

    struct pen_state st;
    memset(&st, 0, sizeof st);
    st.font = font;
    st.gid = gid;

    daegun_pen pen = tally_pen(&st.tally);
    pen.close = on_close;

    CHECK(daegun_font_outline(font, gid, NULL, &pen) == DAEGUN_OK, "outline failed");
    const pen_tally *t = &st.tally;
    CHECK(t->moves > 0, "'B' produced no move_to");
    CHECK(t->closes >= t->moves, "%d contours opened but %d closed", t->moves, t->closes);
    CHECK(t->lines + t->quads + t->curves > 0, "'B' produced no segments at all");
    CHECK(st.reentered_ok == t->closes, "calling back into daegun from a pen callback did not work");

    /* The same from a prewarmed outline, which replays from the cache instead of decoding. */
    size_t added = 0;
    CHECK(daegun_font_prewarm(font, &gid, 1, NULL, &added) == DAEGUN_OK && added == 1, "prewarm took nothing");
    memset(&st, 0, sizeof st);
    st.font = font;
    st.gid = gid;
    CHECK(daegun_font_outline(font, gid, NULL, &pen) == DAEGUN_OK && t->closes > 0, "the prewarmed outline failed");
    CHECK(st.reentered_ok == t->closes, "calling back into daegun while replaying a prewarmed outline failed");

    daegun_pen empty = { 0 };
    CHECK(daegun_font_outline(font, gid, NULL, &empty) == DAEGUN_OK, "a pen with no callbacks was not accepted");

    daegun_font_free(font);
}

/* The same at a location, glyf's and CFF2's, each inner call drawing where the outer one does, which draws what a
 * pen that calls nothing does. */
static void reentry_at_a_location_is_safe(const char *path, uint32_t c)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    daegun_coord wght = { DAEGUN_TAG('w', 'g', 'h', 't'), 700.0 };
    daegun_location *bold = NULL;
    struct pen_state st;
    memset(&st, 0, sizeof st);
    st.font = font;
    if (daegun_font_location(font, &wght, 1, &bold) != DAEGUN_OK
        || daegun_font_glyph_id(font, c, &st.gid) != DAEGUN_OK) {
        CHECK(0, "%s: no bold, or no glyph for U+%04X", path, c);
        daegun_location_free(bold);
        daegun_font_free(font);
        return;
    }
    st.at = bold;
    pen_tally plain = { 0 };
    daegun_pen bare = tally_pen(&plain);
    daegun_pen pen = tally_pen(&st.tally);
    pen.close = on_close;
    CHECK(daegun_font_outline(font, st.gid, bold, &bare) == DAEGUN_OK
              && daegun_font_outline(font, st.gid, bold, &pen) == DAEGUN_OK,
          "%s: drawing at a location failed", path);
    CHECK(memcmp(&plain, &st.tally, sizeof plain) == 0, "%s: calling back into daegun changed what was drawn", path);
    CHECK(st.tally.closes > 0 && st.reentered_ok == st.tally.closes, "%s: calling back at a location failed", path);
    daegun_location_free(bold);
    daegun_font_free(font);
}

static void shaping_produces_positioned_glyphs(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    daegun_shaped_run *run = NULL;
    CHECK(daegun_font_shape(font, TXT("Waffle"), NULL, NULL, &run) == DAEGUN_OK, "shaping Waffle failed");
    size_t g = 0, a = 0, o = 0, c = 0, u = 0;
    const uint32_t *glyphs = daegun_shaped_run_glyphs(run, &g);
    const double *adv = daegun_shaped_run_advances(run, &a);
    daegun_shaped_run_offsets(run, &o);
    daegun_shaped_run_clusters(run, &c);
    daegun_shaped_run_unsafe_to_break(run, &u);
    CHECK(g > 0 && a == g && c == g && o == g && u == g, "%zu glyphs, %zu advances, %zu clusters, %zu offsets", g, a,
          c, o);
    double width = -1.0, sum = 0.0;
    for (size_t i = 0; i < a; i++) sum += adv[i];
    CHECK(daegun_shaped_run_width(run, &width) == DAEGUN_OK && width == sum && width > 0.0,
          "the run is %g wide, its advances sum to %g", width, sum);
    CHECK(daegun_shaped_run_width(NULL, &width) == DAEGUN_NULL, "a null run had a width");
    size_t none = 7;
    daegun_shaped_run_unsafe_to_concat(run, &none);
    CHECK(none == 0, "unsafe_to_concat holds %zu flags unasked", none);

    int all_notdef = 1;
    for (size_t i = 0; i < g; i++) {
        if (glyphs[i] != 0) all_notdef = 0;
    }
    CHECK(!all_notdef, "every glyph of 'Waffle' came back .notdef");

    bool complete = false, broken = true;
    CHECK(daegun_shaped_run_complete(run, &complete) == DAEGUN_OK && complete, "shaping plain Latin did not complete");
    CHECK(daegun_shaped_run_has_broken_syllable(run, &broken) == DAEGUN_OK && !broken, "Latin had a broken syllable");
    int32_t shaper = -1;
    daegun_str shaper_name = { NULL, 0 };
    CHECK(daegun_shaped_run_shaper(run, &shaper) == DAEGUN_OK && shaper == DAEGUN_SHAPER_DEFAULT
              && daegun_shaper_name(shaper, &shaper_name) == DAEGUN_OK && str_is(shaper_name, "default"),
          "Latin went through shaper %d, not the default", shaper);
    CHECK(daegun_shaper_name(-1, &shaper_name) == DAEGUN_INVALID_ARGUMENT, "shaper -1 had a name");
    daegun_shaped_run_free(run);

    /* Shaped into a new run, then into the same one again in place. */
    daegun_shaped_run *into = NULL;
    CHECK(daegun_font_shape_into(font, TXT("Waffle"), NULL, NULL, &into) == DAEGUN_OK && into,
          "shaping into a new run failed");
    daegun_shaped_run *same = into;
    size_t into_n = 0;
    CHECK(daegun_font_shape_into(font, TXT("ffl"), NULL, NULL, &into) == DAEGUN_OK && into == same,
          "a run of its own was not shaped in place");
    CHECK(daegun_shaped_run_glyphs(into, &into_n) && into_n > 0, "shaping into a run gave no glyphs");
    daegun_shaped_run_free(into);

    daegun_shaped_run *lig = NULL;
    CHECK(daegun_font_shape(font, TXT("ffl"), NULL, NULL, &lig) == DAEGUN_OK, "shaping ffl failed");
    size_t n = 0;
    daegun_shaped_run_glyphs(lig, &n);
    CHECK(n >= 1 && n <= 3, "'ffl' shaped to %zu glyphs", n);
    daegun_shaped_run_free(lig);

    /* Text is a count of bytes, so U+0000 is a character, and a slice goes in without a copy. */
    daegun_shaped_run *nul = NULL, *slice = NULL;
    CHECK(daegun_font_shape(font, "a\0b", 3, NULL, NULL, &nul) == DAEGUN_OK
              && daegun_shaped_run_glyphs(nul, &n) && n == 3,
          "text holding U+0000 shaped to %zu glyphs", n);
    CHECK(daegun_font_shape(font, "Waffle", 2, NULL, NULL, &slice) == DAEGUN_OK
              && daegun_shaped_run_glyphs(slice, &n) && n == 2,
          "two bytes of Waffle shaped to %zu glyphs", n);
    daegun_shaped_run_free(nul);
    daegun_shaped_run_free(slice);
    daegun_shaped_run *bad = NULL;
    CHECK(daegun_font_shape(font, "a\xff", 2, NULL, NULL, &bad) == DAEGUN_INVALID_ARGUMENT && bad == NULL,
          "text that isn't UTF-8 shaped");
    CHECK(strstr(daegun_last_error().data, "byte 1") != NULL, "the reason \"%s\" names no byte",
          daegun_last_error().data);
    /* Text is read as UTF-8 only where the shape cache misses, so options are read first; it's still the text's
       fault that's answered, through the cache, past it, and with options that are wrong too. */
    daegun_shape_options checked;
    CHECK(daegun_shape_options_default(&checked, sizeof checked) == DAEGUN_OK, "shape defaults failed");
    CHECK(daegun_font_shape(font, "a\xff", 2, NULL, &checked, &bad) == DAEGUN_INVALID_ARGUMENT && bad == NULL,
          "text that isn't UTF-8 shaped through the cache");
    checked.beginning_of_text = true;
    CHECK(daegun_font_shape(font, "a\xff", 2, NULL, &checked, &bad) == DAEGUN_INVALID_ARGUMENT && bad == NULL,
          "text that isn't UTF-8 shaped past the cache");
    checked.size = sizeof checked + 8;
    CHECK(daegun_font_shape(font, "a\xff", 2, NULL, &checked, &bad) == DAEGUN_INVALID_ARGUMENT
              && strstr(daegun_last_error().data, "byte 1") != NULL,
          "options wrong as well answered for text that isn't UTF-8: \"%s\"", daegun_last_error().data);

    daegun_font_free(font);
}

static void raw_tables_round_trip(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    const daegun_tag HEAD = DAEGUN_TAG('h', 'e', 'a', 'd'), ZZZZ = DAEGUN_TAG('Z', 'Z', 'Z', 'Z');

    char name[5];
    snprintf(name, sizeof name, "%c%c%c%c", DAEGUN_UNTAG(HEAD));
    CHECK(strcmp(name, "head") == 0, "DAEGUN_UNTAG spelled head as %s", name);

    bool has = false;
    CHECK(daegun_font_has_table(font, HEAD, &has) == DAEGUN_OK && has, "a font with no head table is not a font");
    CHECK(daegun_font_has_table(font, ZZZZ, &has) == DAEGUN_OK && !has, "the font claims a ZZZZ table");

    daegun_bytes head = { NULL, 0 };
    CHECK(daegun_font_table(font, HEAD, &head) == DAEGUN_OK && head.data && head.len >= 54,
          "head is %zu bytes, the spec says 54", head.len);
    uint32_t magic = 0;
    CHECK(daegun_read_u32_be(head.data, head.len, 12, &magic) == DAEGUN_OK && magic == 0x5f0f3cf5u,
          "head's magic is %08x", magic);

    daegun_bytes absent = { (const uint8_t *)1, 99 };
    CHECK(daegun_font_table(font, ZZZZ, &absent) == DAEGUN_ABSENT, "a missing table was not ABSENT");
    CHECK(absent.data == (const uint8_t *)1 && absent.len == 99, "an ABSENT table wrote its view");

    /* NULL and no room ask for the count alone; one short is refused and left as it was. */
    size_t n = 0;
    CHECK(daegun_font_table_tags(font, NULL, 0, &n) == DAEGUN_OK && n >= 5 && n <= 64, "%zu tables counted", n);
    daegun_tag t[64];
    memset(t, 0xAB, sizeof t);
    size_t again = 0;
    CHECK(daegun_font_table_tags(font, t, n - 1, &again) == DAEGUN_INVALID_ARGUMENT && again == n
              && t[0] == 0xABABABABu,
          "room for one tag fewer was not refused untouched with %zu needed", n);
    CHECK(daegun_font_table_tags(font, t, 64, &again) == DAEGUN_OK && again == n, "table_tags failed");
    int saw_head = 0;
    for (size_t i = 0; i < n; i++) {
        saw_head |= t[i] == HEAD;
        CHECK(i == 0 || t[i - 1] < t[i], "the tags are not in order at %zu", i);
    }
    CHECK(saw_head, "%zu tables, head not among them", n);

    daegun_location *bold = location_at(font, WGHT, 700.0);
    daegun_table_map *map = NULL;
    CHECK(daegun_font_instance_tables(font, bold, &map) == DAEGUN_OK, "instance_tables failed");
    size_t count = 0;
    CHECK(daegun_table_map_count(map, &count) == DAEGUN_OK && count >= 5, "the instanced map has %zu tables", count);

    daegun_bytes mapped = { NULL, 0 };
    CHECK(daegun_table_map_get(map, HEAD, &mapped) == DAEGUN_OK && mapped.len >= 54, "the map has no head");
    CHECK(daegun_table_map_get(map, ZZZZ, &mapped) == DAEGUN_ABSENT, "the map claims ZZZZ");

    daegun_tag first = 0;
    daegun_bytes first_bytes = { NULL, 0 };
    CHECK(daegun_table_map_tag_at(map, 0, &first) == DAEGUN_OK && first != 0, "tag_at 0 failed");
    CHECK(daegun_table_map_bytes_at(map, 0, &first_bytes) == DAEGUN_OK, "bytes_at 0 failed");
    CHECK(daegun_table_map_tag_at(map, count, &first) == DAEGUN_INVALID_ARGUMENT, "an index past the end was taken");
    CHECK(strstr(daegun_last_error().data, "is past the end") != NULL, "tag_at left the reason \"%s\"",
          daegun_last_error().data);
    CHECK(daegun_table_map_bytes_at(map, count, &first_bytes) == DAEGUN_INVALID_ARGUMENT,
          "bytes past the end were taken");

    daegun_blob *built = NULL;
    CHECK(daegun_table_map_build(map, &built) == DAEGUN_OK, "building from the map failed");
    size_t built_len = 0;
    const uint8_t *built_data = daegun_blob_data(built, &built_len);
    daegun_font *rebuilt = NULL;
    CHECK(built_len > 1000 && daegun_font_open(built_data, built_len, &rebuilt) == DAEGUN_OK,
          "the font this ABI built does not open: %s", daegun_last_error().data);
    uint32_t a = 0, b = 0;
    CHECK(daegun_font_glyph_count(font, &a) == DAEGUN_OK && daegun_font_glyph_count(rebuilt, &b) == DAEGUN_OK
              && a == b,
          "the rebuilt font has %u glyphs, the original %u", b, a);
    daegun_font_free(rebuilt);
    daegun_blob_free(built);

    CHECK(daegun_table_map_remove(map, HEAD) == DAEGUN_OK && daegun_table_map_remove(map, HEAD) == DAEGUN_OK,
          "removing head, or removing it again, failed");
    size_t after = 0;
    CHECK(daegun_table_map_count(map, &after) == DAEGUN_OK && after == count - 1, "removing one left %zu of %zu",
          after, count);
    daegun_table_map_free(map);

    daegun_blob *instance = NULL;
    CHECK(daegun_font_instance(font, bold, &instance) == DAEGUN_OK, "instancing at a weight failed");
    size_t instance_len = 0;
    const uint8_t *instance_data = daegun_blob_data(instance, &instance_len);
    daegun_font *pinned = NULL;
    bool variable = true;
    CHECK(daegun_font_open(instance_data, instance_len, &pinned) == DAEGUN_OK
              && daegun_font_is_variable(pinned, &variable) == DAEGUN_OK && !variable,
          "the instance does not open as a static font");
    daegun_font_free(pinned);
    daegun_blob_free(instance);
    daegun_location_free(bold);

    /* An empty map makes no font and builds no bytes, and any tag is stored, as Rust's map takes it. */
    daegun_table_map *empty = daegun_table_map_new();
    daegun_blob *nothing = NULL;
    n = 1;
    CHECK(daegun_table_map_build(empty, &nothing) == DAEGUN_OK && (daegun_blob_data(nothing, &n), n == 0),
          "an empty map built %zu bytes", n);
    daegun_blob_free(nothing);
    const uint8_t payload[8] = { 1, 2, 3, 4, 5, 6, 7, 8 };
    const daegun_tag test = DAEGUN_TAG('T', 'E', 'S', 'T');
    CHECK(daegun_table_map_set(empty, test, payload, sizeof payload) == DAEGUN_OK, "set failed");
    daegun_bytes back = { NULL, 0 };
    CHECK(daegun_table_map_get(empty, test, &back) == DAEGUN_OK && back.len == 8 && memcmp(back.data, payload, 8) == 0,
          "set and get disagree");
    CHECK(daegun_table_map_set(empty, test, NULL, 0) == DAEGUN_OK
              && daegun_table_map_get(empty, test, &back) == DAEGUN_OK && back.len == 0,
          "an empty table was not stored in place of the first");
    CHECK(daegun_table_map_set(empty, test, HIDDEN_NULL, 4) == DAEGUN_NULL, "NULL bytes with a count were stored");
    daegun_table_map_free(empty);

    daegun_font_free(font);
}

/* A location the instancer can't build answers the instancer's status and reason, from the instance and the
 * tables: here a variable font without hmtx, which it needs. */
static void a_location_that_cannot_be_instanced_says_why(const char *path)
{
    daegun_font *broken = open_font_with(path, DAEGUN_TAG('h', 'm', 't', 'x'), NULL, 0);
    if (!broken) return;
    daegun_location *heavy = location_at(broken, WGHT, 900.0);
    daegun_blob *inst = (daegun_blob *)&heavy;
    CHECK(daegun_font_instance(broken, heavy, &inst) == DAEGUN_MISSING,
          "an uninstanceable location gave an instance");
    CHECK(strstr(daegun_last_error().data, "hmtx") != NULL, "instance left the reason \"%s\"",
          daegun_last_error().data);
    daegun_table_map *tables = (daegun_table_map *)&heavy;
    CHECK(daegun_font_instance_tables(broken, heavy, &tables) == DAEGUN_MISSING,
          "an uninstanceable location gave its tables");
    CHECK(inst == NULL && tables == NULL, "a failed call left its handle out set");
    CHECK(daegun_font_instance(broken, heavy, NULL) == DAEGUN_NULL, "a NULL out instanced first");
    CHECK(daegun_font_instance_tables(broken, heavy, NULL) == DAEGUN_NULL, "a NULL out instanced its tables");
    daegun_location_free(heavy);
    daegun_font_free(broken);
}

static bool key_from_array(void *user, size_t index, uint32_t *out_key)
{
    const uint32_t *keys = (const uint32_t *)user;
    *out_key = keys[index];
    return true;
}

static bool key_that_refuses(void *user, size_t index, uint32_t *out_key)
{
    (void)index; (void)user; (void)out_key;
    return false;
}

static void the_readers_are_bounds_checked(void)
{
    const uint8_t data[4] = { 0x12, 0x34, 0x56, 0x78 };
    uint16_t u16 = 0;
    int16_t i16 = 0;
    uint32_t u32 = 0;
    size_t off = 0;

    CHECK(daegun_read_u16_be(data, sizeof data, 0, &u16) == DAEGUN_OK && u16 == 0x1234, "read_u16_be gave %04x", u16);
    CHECK(daegun_read_u32_be(data, sizeof data, 0, &u32) == DAEGUN_OK && u32 == 0x12345678u, "read_u32_be gave %08x",
          u32);
    CHECK(daegun_read_u24_be(data, sizeof data, 0, &u32) == DAEGUN_OK && u32 == 0x123456u, "read_u24_be gave %06x",
          u32);
    CHECK(daegun_read_i16_be(data, sizeof data, 2, &i16) == DAEGUN_OK && i16 == 0x5678, "read_i16_be gave %04x",
          (unsigned)i16);
    CHECK(daegun_read_offset24(data, sizeof data, 1, &off) == DAEGUN_OK && off == 0x345678, "read_offset24 gave %zx",
          off);

    CHECK(daegun_read_u16_be(data, sizeof data, 3, &u16) == DAEGUN_INVALID_ARGUMENT,
          "a two-byte read at offset 3 of a four-byte buffer was allowed");
    CHECK(daegun_read_u32_be(data, sizeof data, 1, &u32) == DAEGUN_INVALID_ARGUMENT,
          "a four-byte read at offset 1 of a four-byte buffer was allowed");
    CHECK(daegun_read_u16_be(data, sizeof data, (size_t)-1, &u16) == DAEGUN_INVALID_ARGUMENT,
          "an offset that overflows when two is added was allowed");

    uint8_t buf[8] = { 0 };
    CHECK(daegun_write_u16_be(buf, sizeof buf, 0, 0xbeef) == DAEGUN_OK
              && daegun_read_u16_be(buf, sizeof buf, 0, &u16) == DAEGUN_OK && u16 == 0xbeef,
          "wrote beef, read %04x", u16);
    CHECK(daegun_write_i16_be(buf, sizeof buf, 2, -2) == DAEGUN_OK
              && daegun_read_i16_be(buf, sizeof buf, 2, &i16) == DAEGUN_OK && i16 == -2,
          "wrote -2, read %d", (int)i16);
    CHECK(daegun_write_u32_be(buf, sizeof buf, 4, 0xdeadbeefu) == DAEGUN_OK
              && daegun_read_u32_be(buf, sizeof buf, 4, &u32) == DAEGUN_OK && u32 == 0xdeadbeefu,
          "wrote deadbeef, read %08x", u32);
    CHECK(daegun_write_offset24(buf, sizeof buf, 0, 0x010203) == DAEGUN_OK, "write_offset24 failed");
    uint8_t before[8];
    memcpy(before, buf, sizeof buf);
    CHECK(daegun_write_offset24(buf, sizeof buf, 0, 0x1000000) == DAEGUN_INVALID_ARGUMENT
              && memcmp(buf, before, 8) == 0,
          "a value past 24 bits was written");
    CHECK(daegun_write_u16_be(buf, sizeof buf, 7, 1) == DAEGUN_INVALID_ARGUMENT && memcmp(buf, before, 8) == 0,
          "a two-byte write at offset 7 of an eight-byte buffer was allowed");
    CHECK(daegun_write_u16_be(HIDDEN_NULL, 8, 0, 1) == DAEGUN_NULL, "a null write target was not refused");

    CHECK(daegun_records_fit(0, 4, 2, 8), "four two-byte records do fit in eight bytes");
    CHECK(!daegun_records_fit(1, 4, 2, 8), "they do not fit starting at one");
    CHECK(!daegun_records_fit(0, (size_t)-1, 2, 8), "an overflowing count was called fitting");

    CHECK(daegun_bytes_window(data, sizeof data, 0, 4) == data, "window did not borrow in place");
    CHECK(daegun_bytes_window(data, sizeof data, 1, 4) == NULL, "window past the end was allowed");
    CHECK(daegun_bytes_window(data, sizeof data, 2, 2) == data + 2, "window offset was wrong");
    CHECK(daegun_bytes_window(data, 0, 0, 0) == data, "an empty window is not at its buffer");
    CHECK(daegun_bytes_window(NULL, 0, 0, 0) == NULL, "an empty window of no buffer is not NULL");

    uint32_t keys[5] = { 10, 20, 30, 40, 50 };
    size_t index = 999;
    bool found = false;
    CHECK(daegun_search_records(5, 30, key_from_array, keys, &index, &found) == DAEGUN_OK && found && index == 2,
          "30 is at index 2, search said %zu found=%d", index, found);
    CHECK(daegun_search_records(5, 35, key_from_array, keys, &index, &found) == DAEGUN_OK && !found && index == 3,
          "35 inserts at 3, search said %zu found=%d", index, found);
    CHECK(daegun_search_records(5, 30, key_that_refuses, NULL, &index, &found) == DAEGUN_ABSENT,
          "a key reader that refused was not reported");
    CHECK(daegun_search_records(5, 30, NULL, NULL, &index, &found) == DAEGUN_NULL, "a null key reader was taken");

    CHECK(daegun_ot_round(0.5) == 1, "ot_round(0.5) is 1");
    CHECK(daegun_ot_round(-0.5) == 0, "ot_round(-0.5) is 0, not -1: it is floor(v + 0.5)");
    CHECK(daegun_ot_round(2.49) == 2, "ot_round(2.49) is 2");
}

static void paths_build_stroke_and_replay(void)
{
    daegun_path *path = daegun_path_new();
    CHECK(path != NULL, "a new path is null");

    bool empty = false;
    CHECK(daegun_path_is_empty(path, &empty) == DAEGUN_OK && empty, "a new path is not empty");
    daegun_bounds box = { 9, 9, -9, -9 };
    CHECK(daegun_path_bounds(path, &box) == DAEGUN_ABSENT && box.x_min == 9, "an empty path had a box");

    CHECK(daegun_path_move_to(path, 0.0f, 0.0f) == DAEGUN_OK, "move_to failed");
    CHECK(daegun_path_line_to(path, 100.0f, 0.0f) == DAEGUN_OK, "line_to failed");
    CHECK(daegun_path_quad_to(path, 150.0f, 50.0f, 100.0f, 100.0f) == DAEGUN_OK, "quad_to failed");
    CHECK(daegun_path_curve_to(path, 60.0f, 130.0f, 20.0f, 130.0f, 0.0f, 100.0f) == DAEGUN_OK, "curve_to failed");
    CHECK(daegun_path_close(path) == DAEGUN_OK, "close failed");
    CHECK(daegun_path_is_empty(path, &empty) == DAEGUN_OK && !empty, "a path with five verbs is empty");

    CHECK(daegun_path_bounds(path, &box) == DAEGUN_OK && box.x_min == 0.0 && box.y_min == 0.0,
          "bounds start at (%g, %g)", box.x_min, box.y_min);
    CHECK(box.x_max > 100.0 && box.x_max < 150.0 && box.y_max > 100.0 && box.y_max < 130.0,
          "the tight box ends at (%g, %g), not inside the controls", box.x_max, box.y_max);

    size_t cost = 0;
    CHECK(daegun_path_cost(path, &cost) == DAEGUN_OK && cost == 5 + 7 * 8, "five verbs and seven points cost %zu",
          cost);

    size_t verb_count = 0, point_count = 0;
    const uint8_t *verbs = daegun_path_verbs(path, &verb_count);
    const daegun_point *points = daegun_path_points(path, &point_count);
    CHECK(verb_count == 5 && verbs && verbs[0] == DAEGUN_VERB_MOVE && verbs[1] == DAEGUN_VERB_LINE
              && verbs[2] == DAEGUN_VERB_QUAD && verbs[3] == DAEGUN_VERB_CUBIC && verbs[4] == DAEGUN_VERB_CLOSE,
          "the verbs are not move, line, quad, cubic, close");
    const float want_x[7] = { 0, 100, 150, 100, 60, 20, 0 }, want_y[7] = { 0, 0, 50, 100, 130, 130, 100 };
    CHECK(point_count == 7, "1 + 1 + 2 + 3 points went in, %zu came out", point_count);
    for (size_t i = 0; points && i < point_count && i < 7; i++) {
        CHECK(points[i].x == want_x[i] && points[i].y == want_y[i], "point %zu is (%g, %g)", i, points[i].x,
              points[i].y);
    }

    pen_tally seen = { 0, 0, 0, 0, 0 };
    daegun_pen pen = tally_pen(&seen);
    CHECK(daegun_path_replay(path, NULL, &pen) == DAEGUN_OK, "replay failed");
    CHECK(seen.moves == 1 && seen.lines == 1 && seen.quads == 1 && seen.curves == 1 && seen.closes == 1,
          "replay gave %d/%d/%d/%d/%d rather than one of each", seen.moves, seen.lines, seen.quads, seen.curves,
          seen.closes);

    const double shift[6] = { 1.0, 0.0, 0.0, 1.0, 10.0, 20.0 };
    daegun_path *moved = daegun_path_new();
    daegun_pen into_moved = { 0 };
    CHECK(daegun_path_as_pen(moved, &into_moved) == DAEGUN_OK, "as_pen failed");
    CHECK(daegun_path_replay(path, shift, &into_moved) == DAEGUN_OK, "transformed replay failed");
    daegun_bounds moved_box = { 0 };
    CHECK(daegun_path_bounds(moved, &moved_box) == DAEGUN_OK && moved_box.x_min == 10.0 && moved_box.y_min == 20.0,
          "a (10, 20) shift put the corner at (%g, %g)", moved_box.x_min, moved_box.y_min);
    daegun_path_free(moved);

    daegun_stroke_style style = { 10.0f, DAEGUN_CAP_ROUND, DAEGUN_JOIN_ROUND, 4.0f };
    pen_tally stroked = { 0, 0, 0, 0, 0 };
    daegun_pen spen = tally_pen(&stroked);
    CHECK(daegun_path_stroke(path, &style, 0.25f, &spen) == DAEGUN_OK && stroked.moves > 0 && stroked.lines > 0,
          "stroking drew nothing");
    pen_tally simple = { 0, 0, 0, 0, 0 };
    daegun_pen sipen = tally_pen(&simple);
    CHECK(daegun_path_stroke_simplified(path, &style, 0.25f, &sipen) == DAEGUN_OK && simple.moves > 0,
          "simplified stroking drew nothing");
    CHECK(daegun_path_stroke(NULL, &style, 0.25f, &spen) == DAEGUN_NULL, "a null path was stroked");
    CHECK(daegun_path_stroke(path, NULL, 0.25f, &spen) == DAEGUN_NULL, "a null style was taken");
    daegun_stroke_style odd = style;
    odd.join = 3;
    CHECK(daegun_path_stroke(path, &odd, 0.25f, &spen) == DAEGUN_INVALID_ARGUMENT, "join 3 was taken");
    odd = style;
    odd.width = INFINITY;
    CHECK(daegun_path_stroke(path, &odd, 0.25f, &spen) == DAEGUN_INVALID_ARGUMENT, "an infinite width was stroked");

    /* Each turn of this zigzag, stroked wide and round, takes 256 points: 20,000 take five million, which a stroke
     * draws as it works them out, and its simplified form, held whole to be resolved, refuses. */
    daegun_path *zigzag = daegun_path_new();
    CHECK(daegun_path_move_to(zigzag, 0.0f, 0.0f) == DAEGUN_OK, "move_to failed");
    for (int i = 1; i <= 20000; i++) {
        (void)daegun_path_line_to(zigzag, (float)i * 0.5f, i % 2 ? 1000.0f : 0.0f);
    }
    daegun_stroke_style wide = { 200.0f, DAEGUN_CAP_BUTT, DAEGUN_JOIN_ROUND, 4.0f };
    pen_tally all = { 0, 0, 0, 0, 0 }, none = { 0, 0, 0, 0, 0 };
    daegun_pen apen = tally_pen(&all), npen = tally_pen(&none);
    CHECK(daegun_path_stroke(zigzag, &wide, 0.001f, &apen) == DAEGUN_OK, "a stroke past the cap failed");
    CHECK(all.lines > DAEGUN_MAX_FLATTEN_POINTS, "the stroke drew %d lines", all.lines);
    CHECK(daegun_path_stroke_simplified(zigzag, &wide, 0.001f, &npen) == DAEGUN_LIMIT_EXCEEDED,
          "a simplified stroke past the cap was drawn");
    CHECK(none.moves == 0 && none.lines == 0, "a refused stroke drew %d lines", none.lines);
    daegun_path_free(zigzag);

    /* Cleared, the path draws as a new one would. */
    CHECK(daegun_path_clear(path) == DAEGUN_OK && daegun_path_is_empty(path, &empty) == DAEGUN_OK && empty,
          "a cleared path is not empty");
    CHECK(daegun_path_bounds(path, &box) == DAEGUN_ABSENT, "a cleared path kept its box");
    CHECK(daegun_path_move_to(path, 5.0f, 6.0f) == DAEGUN_OK && daegun_path_line_to(path, 7.0f, 8.0f) == DAEGUN_OK
              && daegun_path_bounds(path, &box) == DAEGUN_OK && box.x_min == 5.0 && box.y_max == 8.0,
          "a cleared path drew again to (%g, %g)", box.x_min, box.y_max);
    CHECK(daegun_path_clear(NULL) == DAEGUN_NULL, "a NULL path was cleared");

    daegun_path_free(path);
    daegun_path_free(NULL);
}

static void glyph_quads_come_back_in_font_units(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;

    uint32_t gid = 0, space = 0;
    CHECK(daegun_font_glyph_id(font, 'B', &gid) == DAEGUN_OK, "no glyph for 'B'");
    daegun_quads *quads = NULL;
    CHECK(daegun_font_quads(font, gid, NULL, &quads) == DAEGUN_OK, "quads failed: %s", daegun_last_error().data);
    size_t n = 0;
    const daegun_quad *q = daegun_quads_data(quads, &n);
    CHECK(q != NULL && n > 0, "'B' came back as %zu curves", n);
    daegun_bounds box = { 0 };
    CHECK(daegun_font_glyph_bounds(font, gid, NULL, &box) == DAEGUN_OK, "'B' has no box");
    size_t outside = 0;
    for (size_t i = 0; q && i < n; i++) {
        const daegun_point *p[3] = { &q[i].start, &q[i].control, &q[i].end };
        for (int k = 0; k < 3; k++) {
            if (!(p[k]->x >= box.x_min - 1 && p[k]->x <= box.x_max + 1 && p[k]->y >= box.y_min - 1
                  && p[k]->y <= box.y_max + 1)) {
                outside++;
            }
        }
    }
    CHECK(outside == 0, "%zu points of 'B''s curves lie outside its box in font units", outside);
    daegun_quads_free(quads);
    quads = NULL;

    CHECK(daegun_font_glyph_id(font, ' ', &space) == DAEGUN_OK, "no glyph for the space");
    CHECK(daegun_font_quads(font, space, NULL, &quads) == DAEGUN_NO_OUTLINE && quads == NULL,
          "the space came back with curves");

    /* A square drawn counterclockwise in y-up, so rewinding has to reverse every curve. */
    daegun_path *square = daegun_path_new();
    (void)daegun_path_move_to(square, 0.0f, 0.0f);
    (void)daegun_path_line_to(square, 100.0f, 0.0f);
    (void)daegun_path_line_to(square, 100.0f, 100.0f);
    (void)daegun_path_line_to(square, 0.0f, 100.0f);
    (void)daegun_path_close(square);
    CHECK(daegun_path_quads(square, 0.25f, &quads) == DAEGUN_OK, "path_quads failed");
    q = daegun_quads_data(quads, &n);
    CHECK(n == 4 && q && q[0].start.x == 0.0f && q[0].control.x == 50.0f && q[0].end.x == 100.0f,
          "the first edge is not (0,0)-(100,0) with its control between");
    CHECK(daegun_quads_normalize_winding(quads) == DAEGUN_OK, "normalize_winding failed");
    q = daegun_quads_data(quads, &n);
    CHECK(q && q[0].start.x == 100.0f && q[0].end.x == 0.0f, "a counterclockwise square was not rewound");
    daegun_quads_free(quads);
    quads = NULL;

    daegun_path *empty = daegun_path_new();
    CHECK(daegun_path_quads(empty, 0.25f, &quads) == DAEGUN_NO_OUTLINE, "an empty path came back with curves");
    daegun_path_free(empty);

    CHECK(daegun_font_quads(NULL, gid, NULL, &quads) == DAEGUN_NULL, "a NULL font was taken");
    CHECK(daegun_font_quads(font, gid, NULL, NULL) == DAEGUN_NULL, "a NULL out was taken");
    CHECK(daegun_path_quads(NULL, 0.25f, &quads) == DAEGUN_NULL, "a NULL path was taken");
    CHECK(daegun_path_quads(square, 0.0f, &quads) == DAEGUN_INVALID_ARGUMENT, "a tolerance of zero was taken");
    CHECK(daegun_path_quads(square, NAN, &quads) == DAEGUN_INVALID_ARGUMENT, "a tolerance of NaN was taken");
    CHECK(daegun_quads_normalize_winding(NULL) == DAEGUN_NULL, "rewinding NULL was taken");
    CHECK(daegun_quads_data(NULL, &n) == NULL, "data from NULL quads");
    daegun_quads_free(NULL);

    bool overlaps = true;
    CHECK(daegun_font_has_overlap_flag(font, gid, &overlaps) == DAEGUN_OK && !overlaps, "Inter flagged an overlap");
    CHECK(daegun_font_has_overlap_flag(NULL, gid, &overlaps) == DAEGUN_NULL, "a NULL font was taken");
    CHECK(daegun_font_has_overlap_flag(font, gid, NULL) == DAEGUN_NULL, "a NULL out was taken");

    daegun_path_free(square);
    daegun_font_free(font);
}

static void text_analysis_needs_no_font(void)
{
    size_t bounds[16], n = 0;
    CHECK(daegun_grapheme_boundaries(TXT("he\xcc\x81llo"), bounds, 16, &n) == DAEGUN_OK && n == 6 && bounds[0] == 0
              && bounds[2] == 4 && bounds[5] == 7,
          "the graphemes of a combining e are %zu boundaries", n);
    size_t one = 9;
    CHECK(daegun_grapheme_boundaries(TXT("hello"), bounds, 2, &one) == DAEGUN_INVALID_ARGUMENT && one == 6,
          "a buffer too small was not refused with the count it needs");

    daegun_script_run runs[4];
    CHECK(daegun_script_runs(TXT("hello \xd7\xa9\xd7\x9c\xd7\x95\xd7\x9d"), runs, 4, &n) == DAEGUN_OK && n == 2,
          "Latin then Hebrew gave %zu runs", n);
    CHECK(n == 2 && runs[0].script == DAEGUN_TAG('L', 'a', 't', 'n') && runs[1].script == DAEGUN_TAG('H', 'e', 'b', 'r')
              && runs[0].end == runs[1].start && runs[1].end == 14,
          "the runs are not Latin then Hebrew to byte 14");
    daegun_str name = { NULL, 0 };
    CHECK(daegun_script_name(DAEGUN_TAG('H', 'e', 'b', 'r'), &name) == DAEGUN_OK && str_is(name, "Hebrew"),
          "Hebr's name is not Hebrew");
    CHECK(daegun_script_name(DAEGUN_TAG('Q', 'q', 'q', 'q'), &name) == DAEGUN_INVALID_ARGUMENT,
          "a script no ISO 15924 code names was named");

    daegun_bidi_paragraph *para = NULL;
    CHECK(daegun_bidi_paragraph_new(TXT("hello"), DAEGUN_BASE_AUTO, &para) == DAEGUN_OK, "the bidi paragraph failed");
    const uint8_t *levels = daegun_bidi_paragraph_levels(para, &n);
    CHECK(levels && n == 5 && levels[0] == 0, "'hello' resolved %zu levels", n);
    daegun_bidi_paragraph_free(para);
    CHECK(daegun_bidi_paragraph_new(TXT("hello"), 2, &para) == DAEGUN_INVALID_ARGUMENT && para == NULL,
          "base direction 2 was taken");
}

/* A paint's children start at the index after it, each next one at the end of the one before. */
static size_t children_of(const daegun_paint_node *nodes, size_t at)
{
    size_t count = 0;
    for (size_t c = at + 1; c < nodes[at].end; c = nodes[c].end) {
        count++;
    }
    return count;
}

struct visit_log {
    const daegun_paint_node *nodes;
    size_t entered, depth, deepest, mismatched;
};

static void visit_enter(void *user, const daegun_paint_node *node, const daegun_color_stop *stops,
                        size_t stops_count)
{
    struct visit_log *log = user;
    const daegun_paint_node *want = &log->nodes[log->entered++];
    if (node->kind != want->kind || node->glyph != want->glyph || node->stops_count != want->stops_count
        || stops_count != node->stops_count || node->end != 0 || node->stops_start != 0
        || (node->stops_count > 0) != (stops != NULL)) {
        log->mismatched++;
    }
    if (++log->depth > log->deepest) log->deepest = log->depth;
}

static void visit_leave(void *user) { ((struct visit_log *)user)->depth--; }

/* A visitor walking the same glyph again from inside each paint, on the same font, as CALLBACKS allows. */
struct reentry {
    daegun_font *font;
    uint32_t glyph;
    size_t calls, inner;
};

static void count_enter(void *user, const daegun_paint_node *node, const daegun_color_stop *stops,
                        size_t stops_count)
{
    (void)node;
    (void)stops;
    (void)stops_count;
    (*(size_t *)user)++;
}

static void reenter(void *user, const daegun_paint_node *node, const daegun_color_stop *stops, size_t stops_count)
{
    (void)node;
    (void)stops;
    (void)stops_count;
    struct reentry *r = user;
    size_t met = 0;
    daegun_paint_visitor inner = { count_enter, NULL, &met };
    if (daegun_font_visit_paint_graph(r->font, r->glyph, NULL, 0, &inner) == DAEGUN_OK) r->inner += met;
    r->calls++;
}

static void the_paint_graph_is_walkable(void)
{
    daegun_font *font = open_font(COLR);
    if (!font) return;

    uint32_t glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &glyphs) == DAEGUN_OK, "glyph_count failed");
    daegun_paint_graph *graph = NULL;
    uint32_t found = 0;
    for (uint32_t g = 1; g < glyphs && g < 400 && !found; g++) {
        if (daegun_font_paint_graph(font, g, NULL, 0, &graph) == DAEGUN_OK) found = g;
    }
    CHECK(found, "no glyph of the COLR test font has a paint graph");
    if (!found) { daegun_font_free(font); return; }

    size_t n = 0, stops = 0;
    const daegun_paint_node *nodes = daegun_paint_graph_nodes(graph, &n);
    daegun_paint_graph_stops(graph, &stops);
    CHECK(n > 0 && nodes[0].end == n, "glyph %u's root does not span its %zu paints", found, n);
    for (size_t i = 0; i < n; i++) {
        CHECK(nodes[i].kind >= DAEGUN_PAINT_LAYERS && nodes[i].kind <= DAEGUN_PAINT_COMPOSITE,
              "paint %zu is kind %d", i, nodes[i].kind);
        CHECK(nodes[i].end > i && nodes[i].end <= n, "paint %zu ends at %zu of %zu", i, nodes[i].end, n);
        CHECK(nodes[i].stops_start + nodes[i].stops_count <= stops, "paint %zu's stops run past the end", i);
        if (nodes[i].kind == DAEGUN_PAINT_COMPOSITE) {
            CHECK(children_of(nodes, i) == 2, "a composite has %zu children, not 2", children_of(nodes, i));
        }
    }

    /* The visitor meets the same paints in the same order, with its stops lent for the call. */
    struct visit_log log = { nodes, 0, 0, 0, 0 };
    daegun_paint_visitor visitor = { visit_enter, visit_leave, &log };
    CHECK(daegun_font_visit_paint_graph(font, found, NULL, 0, &visitor) == DAEGUN_OK, "the visit failed");
    CHECK(log.entered == n && log.depth == 0 && log.mismatched == 0, "the visit met %zu of %zu paints, %zu differing",
          log.entered, n, log.mismatched);
    struct reentry again = { font, found, 0, 0 };
    daegun_paint_visitor nested = { reenter, NULL, &again };
    CHECK(daegun_font_visit_paint_graph(font, found, NULL, 0, &nested) == DAEGUN_OK && again.calls == n
              && again.inner == n * n,
          "a visit from inside a visit met %zu of %zu paints", again.inner, n * n);
    daegun_paint_visitor none = { NULL, NULL, NULL };
    CHECK(daegun_font_visit_paint_graph(font, found, NULL, 0, &none) == DAEGUN_OK, "a visitor of no callbacks failed");
    CHECK(daegun_font_visit_paint_graph(font, 0, NULL, 0, &visitor) == DAEGUN_ABSENT, ".notdef has a graph");
    daegun_paint_graph_free(graph);
    daegun_font_free(font);
}

static void a_color_scene_is_walkable(void)
{
    daegun_font *font = open_font(COLR);
    if (!font) return;
    uint16_t upm = 0;
    CHECK(daegun_font_units_per_em(font, &upm) == DAEGUN_OK && upm > 0, "the COLR font has no em");

    daegun_scene *scene = (daegun_scene *)font;
    CHECK(daegun_font_color_scene(font, 0, NULL, NULL, &scene) == DAEGUN_ABSENT && scene == NULL,
          ".notdef has a color scene, or the handle out was left set");

    /* sweep_coincident_stops_forward_blue_red_reflect: an ill-formed color line draws nothing, but the
     * glyph still has a color description. */
    size_t none = 1;
    CHECK(daegun_font_color_scene(font, 194, NULL, NULL, &scene) == DAEGUN_OK,
          "a color glyph drawing nothing was ABSENT");
    CHECK(daegun_scene_ops(scene, &none) != NULL && none == 0, "an ill-formed color line drew %zu ops", none);
    daegun_scene_free(scene);
    scene = NULL;

    /* The first glyph whose scene fills with a gradient, so every part of the struct gets read. */
    uint32_t glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &glyphs) == DAEGUN_OK, "glyph_count failed");
    int found = 0;
    for (uint32_t gid = 1; gid < glyphs && !found; gid++) {
        if (daegun_font_color_scene(font, gid, NULL, NULL, &scene) != DAEGUN_OK) continue;
        size_t nops = 0, nshapes = 0, ngrads = 0, nstops = 0, npaths = 0;
        const daegun_scene_op *ops = daegun_scene_ops(scene, &nops);
        const daegun_clip_shape *shapes = daegun_scene_shapes(scene, &nshapes);
        const daegun_gradient *grads = daegun_scene_gradients(scene, &ngrads);
        const daegun_stop *stops = daegun_scene_stops(scene, &nstops);
        CHECK(daegun_scene_path_count(scene, &npaths) == DAEGUN_OK, "path_count failed");
        CHECK(ops != NULL && nops > 0, "gid %u has a scene with no ops", gid);

        int depth = 0, gradient_fill = -1;
        for (size_t i = 0; ops && i < nops; i++) {
            const daegun_scene_op *op = &ops[i];
            if (op->kind == DAEGUN_OP_FILL) {
                daegun_path_view view = { 0 };
                CHECK(op->path < npaths && daegun_scene_path_at(scene, op->path, &view) == DAEGUN_OK
                          && view.verbs_len > 0,
                      "fill %zu names no path", i);
                CHECK(op->rule == DAEGUN_FILL_NONZERO || op->rule == DAEGUN_FILL_EVENODD, "rule %d", op->rule);
                if (op->has_gradient) {
                    CHECK(op->gradient < ngrads, "gradient %zu of %zu", op->gradient, ngrads);
                    if (gradient_fill < 0) gradient_fill = (int)i;
                }
            } else if (op->kind == DAEGUN_OP_PUSH_CLIP) {
                CHECK(op->shapes_start <= op->shapes_end && op->shapes_end <= nshapes,
                      "a clip's shapes run past the end");
                depth++;
            } else if (op->kind == DAEGUN_OP_PUSH_LAYER) {
                CHECK(op->blend >= DAEGUN_COMPOSITE_CLEAR && op->blend <= DAEGUN_COMPOSITE_HSL_LUMINOSITY,
                      "composite mode %d", op->blend);
                depth++;
            } else {
                depth--;
            }
            CHECK(depth >= 0, "a pop came before its push");
        }
        CHECK(depth == 0, "gid %u: pushes and pops do not balance", gid);
        for (size_t i = 0; shapes && i < nshapes; i++) {
            CHECK(shapes[i].path < npaths, "clip shape %zu names path %zu of %zu", i, shapes[i].path, npaths);
        }

        if (gradient_fill >= 0) {
            found = 1;
            size_t grad = ops[gradient_fill].gradient;
            const daegun_gradient *g = &grads[grad];
            CHECK(g->kind >= DAEGUN_GRADIENT_LINEAR && g->kind <= DAEGUN_GRADIENT_SWEEP, "kind %d", g->kind);
            CHECK(g->extend >= DAEGUN_EXTEND_PAD && g->extend <= DAEGUN_EXTEND_REFLECT, "extend %d", g->extend);
            CHECK(g->stops_start + g->stops_count <= nstops, "a stop run past the end");
            for (size_t k = 1; k < g->stops_count; k++) {
                float a = stops[g->stops_start + k - 1].offset, b = stops[g->stops_start + k].offset;
                CHECK(a <= b && a >= 0.0f && b <= 1.0f, "stops %f, %f are not sorted into 0..1", a, b);
            }

            /* A 64 px em, y flipped into device rows. */
            double to_device[6] = { 64.0 / upm, 0.0, 0.0, -64.0 / upm, 32.0, 64.0 };
            daegun_ramp *spec = NULL, *srgb = NULL, *bad = (daegun_ramp *)font;
            CHECK(daegun_scene_ramp(scene, grad, to_device, DAEGUN_INTERPOLATE_LINEAR_LIGHT, &spec) == DAEGUN_OK
                      && daegun_scene_ramp(scene, grad, to_device, DAEGUN_INTERPOLATE_SRGB, &srgb) == DAEGUN_OK,
                  "a ramp failed");
            int painted = 0, parted = 0;
            for (int y = 0; y < 64 && spec && srgb; y += 4) {
                for (int x = 0; x < 64; x += 4) {
                    daegun_rgba l = { 0, 0, 0, 0 }, r = { 0, 0, 0, 0 };
                    daegun_status st = daegun_ramp_at(spec, x, y, &l);
                    CHECK(st == DAEGUN_OK || st == DAEGUN_ABSENT, "sampling returned %d", st);
                    painted += st == DAEGUN_OK;
                    parted += st == DAEGUN_OK && daegun_ramp_at(srgb, x, y, &r) == DAEGUN_OK
                              && memcmp(&l, &r, sizeof l) != 0;
                }
            }
            CHECK(painted > 0, "the gradient painted nothing across a 64 px square");
            CHECK(parted > 0, "the sRGB blend matched the spec's at every sample");
            daegun_rgba c = { 0 };
            CHECK(daegun_ramp_at(NULL, 0, 0, &c) == DAEGUN_NULL, "sampling NULL was taken");
            CHECK(daegun_ramp_at(spec, 0, 0, NULL) == DAEGUN_NULL, "a NULL color was taken");
            CHECK(daegun_scene_ramp(scene, grad, to_device, 2, &bad) == DAEGUN_INVALID_ARGUMENT && bad == NULL,
                  "interpolation 2 was taken");
            CHECK(strstr(daegun_last_error().data, "interpolation") != NULL, "interpolation 2 left the reason \"%s\"",
                  daegun_last_error().data);
            CHECK(daegun_scene_ramp(scene, ngrads, to_device, DAEGUN_INTERPOLATE_SRGB, &bad) == DAEGUN_INVALID_ARGUMENT,
                  "a gradient past the end was taken");
            CHECK(daegun_scene_ramp(scene, grad, NULL, DAEGUN_INTERPOLATE_SRGB, &bad) == DAEGUN_NULL,
                  "a NULL transform was taken");
            daegun_ramp_free(spec);
            daegun_ramp_free(srgb);

            daegun_path_view view = { 0 };
            daegun_path *copy = (daegun_path *)font;
            CHECK(daegun_scene_path_at(scene, npaths, &view) == DAEGUN_INVALID_ARGUMENT,
                  "a path past the end was lent");
            CHECK(daegun_scene_path_copy(scene, npaths, &copy) == DAEGUN_INVALID_ARGUMENT && copy == NULL,
                  "a path past the end was copied");
            CHECK(daegun_scene_path_copy(scene, ops[gradient_fill].path, &copy) == DAEGUN_OK, "a copy failed");
            size_t copied = 0;
            CHECK(daegun_scene_path_at(scene, ops[gradient_fill].path, &view) == DAEGUN_OK
                      && daegun_path_verbs(copy, &copied) && copied == view.verbs_len,
                  "the copy holds %zu verbs of the scene's %zu", copied, view.verbs_len);
            daegun_path_free(copy);
        }
        daegun_scene_free(scene);
        scene = NULL;
    }
    CHECK(found, "no glyph of the COLR test font fills with a gradient");

    /* gid 154 has a layer that defers to the text color, so naming one has to reach a fill. */
    daegun_scene_options opts = { 0 };
    CHECK(daegun_scene_options_default(&opts, sizeof opts) == DAEGUN_OK, "scene defaults failed");
    opts.foreground.r = 255;
    int took = 0;
    if (daegun_font_color_scene(font, 154, NULL, &opts, &scene) == DAEGUN_OK) {
        size_t n = 0;
        const daegun_scene_op *ops = daegun_scene_ops(scene, &n);
        for (size_t i = 0; ops && i < n; i++) {
            took |= ops[i].kind == DAEGUN_OP_FILL && !ops[i].has_gradient && ops[i].color.r == 255
                    && ops[i].color.g == 0 && ops[i].color.b == 0;
        }
        daegun_scene_free(scene);
        scene = NULL;
    }
    CHECK(took, "the text color the options named reached no fill");

    daegun_bounds box = { 0 };
    CHECK(daegun_font_colr_clip_box(font, 0, NULL, &box) == DAEGUN_ABSENT, ".notdef has a clip box");

    size_t n = 0;
    CHECK(daegun_font_color_scene(NULL, 1, NULL, NULL, &scene) == DAEGUN_NULL, "a NULL font was taken");
    CHECK(daegun_font_color_scene(font, 1, NULL, NULL, NULL) == DAEGUN_NULL, "a NULL out was taken");
    CHECK(daegun_scene_ops(NULL, &n) == NULL, "ops from a NULL scene");
    daegun_scene_free(NULL);
    daegun_ramp_free(NULL);
    daegun_font_free(font);
}

/* Past either end of its color line a PAD gradient holds that end's stop, which a sample inside the
 * gradient never shows. Three lengths out, so pixel centers do not matter. */
static void a_padded_gradient_holds_its_end_colors(void)
{
    daegun_font *font = open_font(COLR);
    if (!font) return;
    uint32_t glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &glyphs) == DAEGUN_OK, "glyph_count failed");
    const double identity[6] = { 1.0, 0.0, 0.0, 1.0, 0.0, 0.0 };
    int found = 0;
    for (uint32_t gid = 1; gid < glyphs && !found; gid++) {
        daegun_scene *scene = NULL;
        if (daegun_font_color_scene(font, gid, NULL, NULL, &scene) != DAEGUN_OK) continue;
        size_t ngrads = 0, nstops = 0;
        const daegun_gradient *grads = daegun_scene_gradients(scene, &ngrads);
        const daegun_stop *stops = daegun_scene_stops(scene, &nstops);
        for (size_t i = 0; i < ngrads && !found; i++) {
            const daegun_gradient *g = &grads[i];
            if (g->kind != DAEGUN_GRADIENT_LINEAR || g->extend != DAEGUN_EXTEND_PAD || g->stops_count < 2) continue;
            found = 1;
            daegun_ramp *ramp = NULL;
            CHECK(daegun_scene_ramp(scene, i, identity, DAEGUN_INTERPOLATE_LINEAR_LIGHT, &ramp) == DAEGUN_OK,
                  "a ramp failed");
            const double ts[2] = { -3.0, 4.0 };
            const size_t ends[2] = { g->stops_start, g->stops_start + g->stops_count - 1 };
            for (int e = 0; e < 2 && ramp; e++) {
                const float *v = g->values;
                const double *m = g->transform;
                double gx = v[0] + ts[e] * (v[2] - v[0]), gy = v[1] + ts[e] * (v[3] - v[1]);
                double x = m[0] * gx + m[2] * gy + m[4], y = m[1] * gx + m[3] * gy + m[5];
                daegun_rgba c = { 0, 0, 0, 0 };
                CHECK(daegun_ramp_at(ramp, x - 0.5, y - 0.5, &c) == DAEGUN_OK, "glyph %u: nothing past the end", gid);
                const daegun_rgba want = stops[ends[e]].color;
                CHECK(abs(c.r - want.r) <= 1 && abs(c.g - want.g) <= 1 && abs(c.b - want.b) <= 1
                          && abs(c.a - want.a) <= 1,
                      "glyph %u, t = %g: (%u, %u, %u, %u) is not the end stop", gid, ts[e], c.r, c.g, c.b, c.a);
            }
            daegun_ramp_free(ramp);
        }
        daegun_scene_free(scene);
    }
    CHECK(found, "no glyph of the COLR test font fills with a padded linear gradient");
    daegun_font_free(font);
}

static int rgb_is(const float got[3], float r, float g, float b)
{
    return fabsf(got[0] - r) < 1e-6f && fabsf(got[1] - g) < 1e-6f && fabsf(got[2] - b) < 1e-6f;
}

/* Expected values worked from the Porter-Duff and blend formulas by hand, not read off the library. */
static void compositing_follows_colr(void)
{
    const float red[3] = { 1.0f, 0.0f, 0.0f }, blue[3] = { 0.0f, 0.0f, 1.0f };
    const float gray[3] = { 0.5f, 0.5f, 0.5f }, orange[3] = { 1.0f, 0.5f, 0.0f };
    float out[3], alpha = -1.0f;

    CHECK(daegun_composite(DAEGUN_COMPOSITE_SRC_OVER, red, 0.5f, blue, 1.0f, out, &alpha) == DAEGUN_OK
              && rgb_is(out, 0.5f, 0.0f, 0.5f) && alpha == 1.0f,
          "SRC_OVER of half red on blue gave (%g, %g, %g, %g)", out[0], out[1], out[2], alpha);
    CHECK(daegun_composite(DAEGUN_COMPOSITE_SRC_OVER, red, 0.5f, blue, 0.0f, out, &alpha) == DAEGUN_OK
              && rgb_is(out, 0.5f, 0.0f, 0.0f) && alpha == 0.5f,
          "SRC_OVER onto nothing is not premultiplied: (%g, %g, %g, %g)", out[0], out[1], out[2], alpha);
    CHECK(daegun_composite(DAEGUN_COMPOSITE_MULTIPLY, orange, 1.0f, gray, 1.0f, out, &alpha) == DAEGUN_OK
              && rgb_is(out, 0.5f, 0.25f, 0.0f) && alpha == 1.0f,
          "MULTIPLY gave (%g, %g, %g, %g)", out[0], out[1], out[2], alpha);
    CHECK(daegun_composite(DAEGUN_COMPOSITE_CLEAR, orange, 1.0f, gray, 1.0f, out, &alpha) == DAEGUN_OK
              && rgb_is(out, 0.0f, 0.0f, 0.0f) && alpha == 0.0f,
          "CLEAR left (%g, %g, %g, %g)", out[0], out[1], out[2], alpha);
    CHECK(daegun_composite(DAEGUN_COMPOSITE_XOR, orange, 1.0f, gray, 1.0f, out, &alpha) == DAEGUN_OK
              && rgb_is(out, 0.0f, 0.0f, 0.0f) && alpha == 0.0f,
          "XOR of two opaque pixels left (%g, %g, %g, %g)", out[0], out[1], out[2], alpha);
    /* The last mode, LUMINOSITY: gray takes orange's luminosity, 0.3 + 0.59 * 0.5. */
    CHECK(daegun_composite(DAEGUN_COMPOSITE_HSL_LUMINOSITY, orange, 1.0f, gray, 1.0f, out, &alpha) == DAEGUN_OK
              && rgb_is(out, 0.595f, 0.595f, 0.595f),
          "LUMINOSITY gave (%g, %g, %g)", out[0], out[1], out[2]);

    /* The blend function alone: a Porter-Duff mode answers the source, a blend mode B(Cb, Cs). */
    CHECK(daegun_blend(DAEGUN_COMPOSITE_MULTIPLY, gray, orange, out) == DAEGUN_OK && rgb_is(out, 0.5f, 0.25f, 0.0f),
          "MULTIPLY's blend gave (%g, %g, %g)", out[0], out[1], out[2]);
    CHECK(daegun_blend(DAEGUN_COMPOSITE_SRC_OVER, gray, orange, out) == DAEGUN_OK && rgb_is(out, 1.0f, 0.5f, 0.0f),
          "SRC_OVER's blend is not the source");

    CHECK(daegun_composite(28, orange, 1.0f, gray, 1.0f, out, &alpha) == DAEGUN_INVALID_ARGUMENT, "mode 28 was taken");
    CHECK(strstr(daegun_last_error().data, "DAEGUN_COMPOSITE_") != NULL, "mode 28 left the reason \"%s\"",
          daegun_last_error().data);
    CHECK(daegun_composite(-1, orange, 1.0f, gray, 1.0f, out, &alpha) == DAEGUN_INVALID_ARGUMENT, "mode -1 was taken");
    CHECK(daegun_blend(28, gray, orange, out) == DAEGUN_INVALID_ARGUMENT, "blend mode 28 was taken");
    CHECK(daegun_composite(3, orange, 1.0f, gray, 1.0f, NULL, &alpha) == DAEGUN_NULL, "a NULL out was taken");
    CHECK(daegun_composite(3, orange, 1.0f, gray, 1.0f, out, NULL) == DAEGUN_NULL, "a NULL alpha out was taken");
    CHECK(daegun_blend(3, NULL, orange, out) == DAEGUN_NULL, "a NULL backdrop was taken");
}

static void square_into(daegun_path *p, float x, float y, float side)
{
    (void)daegun_path_move_to(p, x, y);
    (void)daegun_path_line_to(p, x + side, y);
    (void)daegun_path_line_to(p, x + side, y + side);
    (void)daegun_path_line_to(p, x, y + side);
    (void)daegun_path_close(p);
}

/* A pen of the caller's own that adds to a path it holds a pointer to, shifted right by 100. */
struct forward { daegun_path *path; };
static void fwd_move(void *u, float x, float y)
{
    (void)daegun_path_move_to(((struct forward *)u)->path, x + 100.0f, y);
}
static void fwd_line(void *u, float x, float y)
{
    (void)daegun_path_line_to(((struct forward *)u)->path, x + 100.0f, y);
}
static void fwd_quad(void *u, float a, float b, float x, float y)
{ (void)daegun_path_quad_to(((struct forward *)u)->path, a + 100.0f, b, x + 100.0f, y); }
static void fwd_curve(void *u, float a, float b, float c, float d, float x, float y)
{ (void)daegun_path_curve_to(((struct forward *)u)->path, a + 100.0f, b, c + 100.0f, d, x + 100.0f, y); }
static void fwd_close(void *u) { (void)daegun_path_close(((struct forward *)u)->path); }

/* A pen from daegun_path_as_pen appends to its own path, so drawing a path into its own pen grows it
 * while it is read. Ten verbs is past the first allocation, so the growth moves the storage. */
static void a_path_can_draw_into_itself(void)
{
    daegun_path *p = daegun_path_new();
    square_into(p, 0.0f, 0.0f, 10.0f);
    square_into(p, 20.0f, 0.0f, 10.0f);
    size_t before = 0, after = 0;
    CHECK(daegun_path_verbs(p, &before) && before == 10, "two squares are %zu verbs", before);

    daegun_pen self = { 0 };
    CHECK(daegun_path_as_pen(p, &self) == DAEGUN_OK, "as_pen failed");
    const double shift[6] = { 1.0, 0.0, 0.0, 1.0, 100.0, 0.0 };
    CHECK(daegun_path_replay(p, shift, &self) == DAEGUN_OK, "replaying a path into itself failed");
    CHECK(daegun_path_verbs(p, &after) && after == 2 * before, "replaying %zu verbs into their own path gave %zu",
          before, after);
    daegun_bounds box = { 0 };
    CHECK(daegun_path_bounds(p, &box) == DAEGUN_OK && box.x_max == 130.0, "the shifted copy ends at %g, not 130",
          box.x_max);

    daegun_stroke_style style = { 2.0f, DAEGUN_CAP_BUTT, DAEGUN_JOIN_MITER, 4.0f };
    CHECK(daegun_path_stroke(p, &style, 0.25f, &self) == DAEGUN_OK, "stroking a path into itself failed");
    CHECK(daegun_path_stroke_simplified(p, &style, 0.25f, &self) == DAEGUN_OK,
          "simplified stroking of a path into itself failed");
    CHECK(daegun_path_verbs(p, &after) && after > 4 * before, "two strokes added only %zu verbs", after - 2 * before);

    /* The same path reached through a pen that is not daegun_path_as_pen's, which no pointer check sees. */
    daegun_path *q = daegun_path_new();
    for (int i = 0; i < 64; i++) square_into(q, (float)i * 20.0f, 0.0f, 10.0f);
    size_t q_before = 0, q_after = 0;
    daegun_path_verbs(q, &q_before);
    struct forward to_q = { q };
    daegun_pen fwd = { fwd_move, fwd_line, fwd_quad, fwd_curve, fwd_close, &to_q };
    CHECK(daegun_path_replay(q, NULL, &fwd) == DAEGUN_OK, "replaying a path into a pen that grows it failed");
    CHECK(daegun_path_verbs(q, &q_after) && q_after == 2 * q_before,
          "replaying %zu verbs into a pen that adds to the same path gave %zu", q_before, q_after);
    daegun_path_free(q);
    daegun_path_free(p);
}

static void paths_flatten_and_resolve(void)
{
    CHECK(daegun_max_area_for(16.0f, 1000.0f) == 375.0f, "6 * 1000 / 16 came back as %f",
          (double)daegun_max_area_for(16.0f, 1000.0f));
    CHECK(daegun_max_area_for(0.0f, 1000.0f) == FLT_MAX, "a px of 0 did not ask for the coarsest");

    /* Two squares overlapping in a 49 by 40 corner, where no edge's midpoint lies: their union is one
     * boundary of 8 points. */
    daegun_path *two = daegun_path_new();
    square_into(two, 0.0f, 0.0f, 100.0f);
    square_into(two, 51.0f, 60.0f, 100.0f);
    daegun_contours *flat = NULL, *resolved = NULL;
    CHECK(daegun_path_flatten(two, 1.0f, &flat) == DAEGUN_OK, "flatten failed");
    size_t n = 0, points = 0;
    CHECK(daegun_contours_count(flat, &n) == DAEGUN_OK && n == 2, "two squares flattened to %zu contours", n);
    const daegun_point *p = daegun_contours_points(flat, 1, &points);
    CHECK(p && points == 4 && p[0].x == 51.0f && p[0].y == 60.0f, "the second square came back wrong");
    CHECK(daegun_contours_points(flat, 2, &points) == NULL, "a contour past the end");
    CHECK(daegun_contours_resolve_overlaps(flat, &resolved) == DAEGUN_OK, "the overlap was left alone");
    CHECK(daegun_contours_count(resolved, &n) == DAEGUN_OK && n == 1, "the union is %zu contours", n);
    daegun_contours_points(resolved, 0, &points);
    CHECK(points == 8, "the union's outline has %zu points, not 8", points);
    daegun_contours_free(resolved);
    resolved = NULL;

    /* Contours of the caller's own resolve as flattened ones do. */
    size_t lengths[2] = { 4, 4 };
    const daegun_point *first = daegun_contours_points(flat, 0, &points);
    daegun_point both[8];
    memcpy(both, first, 4 * sizeof *both);
    memcpy(both + 4, p, 4 * sizeof *both);
    daegun_contours *own = NULL;
    CHECK(daegun_contours_new(both, lengths, 2, &own) == DAEGUN_OK
              && daegun_contours_resolve_overlaps(own, &resolved) == DAEGUN_OK
              && daegun_contours_points(resolved, 0, &points) && points == 8,
          "contours of the caller's own did not resolve as the flattened ones");
    daegun_contours_free(resolved);
    daegun_contours_free(own);
    daegun_contours_free(flat);
    resolved = flat = NULL;
    size_t past[2] = { 1, (size_t)-1 };
    CHECK(daegun_contours_new(both, past, 2, &own) == DAEGUN_INVALID_ARGUMENT && own == NULL,
          "lengths that overflow were taken");
    CHECK(daegun_contours_new(NULL, lengths, 2, &own) == DAEGUN_NULL, "NULL points with lengths were taken");

    daegun_path *one = daegun_path_new();
    square_into(one, 0.0f, 0.0f, 100.0f);
    CHECK(daegun_path_flatten(one, 1.0f, &flat) == DAEGUN_OK, "flatten of one square failed");
    CHECK(daegun_contours_resolve_overlaps(flat, &resolved) == DAEGUN_ABSENT, "a lone square was resolved");
    daegun_contours_free(flat);
    flat = NULL;

    /* A quarter circle as one quadratic: a finer area has to give more points. */
    daegun_path *curve = daegun_path_new();
    (void)daegun_path_move_to(curve, 0.0f, 0.0f);
    (void)daegun_path_quad_to(curve, 100.0f, 0.0f, 100.0f, 100.0f);
    (void)daegun_path_close(curve);
    size_t coarse = 0, fine = 0;
    CHECK(daegun_path_flatten(curve, 100.0f, &flat) == DAEGUN_OK, "a coarse flatten failed");
    daegun_contours_points(flat, 0, &coarse);
    daegun_contours_free(flat);
    CHECK(daegun_path_flatten(curve, 1.0f, &flat) == DAEGUN_OK, "a fine flatten failed");
    daegun_contours_points(flat, 0, &fine);
    daegun_contours_free(flat);
    flat = NULL;
    CHECK(coarse >= 3 && fine > coarse, "a finer area gave %zu points against %zu", fine, coarse);

    daegun_path *empty = daegun_path_new();
    CHECK(daegun_path_flatten(empty, 1.0f, &flat) == DAEGUN_OK && daegun_contours_count(flat, &n) == DAEGUN_OK
              && n == 0,
          "an empty path flattened to %zu contours", n);
    daegun_contours_free(flat);
    flat = NULL;
    CHECK(daegun_path_flatten(NULL, 1.0f, &flat) == DAEGUN_NULL, "a NULL path was taken");
    CHECK(daegun_path_flatten(one, 1.0f, NULL) == DAEGUN_NULL, "a NULL out was taken");
    CHECK(daegun_contours_count(NULL, &n) == DAEGUN_NULL, "counting NULL contours");
    CHECK(daegun_contours_points(NULL, 0, &points) == NULL, "points from NULL contours");
    CHECK(daegun_contours_resolve_overlaps(NULL, &resolved) == DAEGUN_NULL, "resolving NULL contours");
    daegun_contours_free(NULL);

    /* A point that is not finite has no place in a polygon. At max_area 0 each curve is 4,097 points,
     * so 300 of them pass DAEGUN_MAX_FLATTEN_POINTS. */
    daegun_path *bad = daegun_path_new(), *wave = daegun_path_new();
    (void)daegun_path_move_to(bad, 0.0f, 0.0f);
    (void)daegun_path_line_to(bad, NAN, 10.0f);
    (void)daegun_path_line_to(bad, 10.0f, 10.0f);
    (void)daegun_path_close(bad);
    CHECK(daegun_path_flatten(bad, 1.0f, &flat) == DAEGUN_INVALID_ARGUMENT && !flat, "a NaN point was flattened");
    (void)daegun_path_move_to(wave, 0.0f, 0.0f);
    for (int i = 0; i < 300; i++) {
        float x = (float)i * 10.0f;
        (void)daegun_path_curve_to(wave, x + 3.0f, 50.0f, x + 6.0f, -50.0f, x + 10.0f, 0.0f);
    }
    (void)daegun_path_close(wave);
    CHECK(daegun_path_flatten(wave, 0.0f, &flat) == DAEGUN_LIMIT_EXCEEDED && !flat,
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
static void reach_quad(void *u, float cx, float cy, float x, float y)
{
    (void)cy;
    (void)y;
    reach_x(u, cx);
    reach_x(u, x);
}

/* The fixture's own hints and the autohinter place A apart, so the outlines tell the mode reached the
 * hinter, and 2, which names no mode, is refused. A contour's end is the index of its last point. */
static void a_hinted_glyph_is_readable_from_c(void)
{
    daegun_font *font = open_font("assets/test-fonts/test-fixtures/hinted.ttf");
    if (!font) return;
    uint32_t gid = 0;
    CHECK(daegun_font_glyph_id(font, 'A', &gid) == DAEGUN_OK, "the fixture has no A");
    daegun_hinted_outline *sub = NULL, *forced = NULL, *none = (daegun_hinted_outline *)font;
    CHECK(daegun_font_hinted_outline(font, gid, NULL, 16.0f, DAEGUN_HINT_FONT, &sub) == DAEGUN_OK,
          "the font's own hints failed");
    CHECK(daegun_font_hinted_outline(font, gid, NULL, 16.0f, DAEGUN_HINT_AUTO_FORCE, &forced) == DAEGUN_OK,
          "the autohinter failed");
    CHECK(daegun_font_hinted_outline(font, gid, NULL, 16.0f, DAEGUN_HINT_NONE, &none) == DAEGUN_ABSENT && none == NULL,
          "DAEGUN_HINT_NONE hinted");
    CHECK(daegun_font_hinted_outline(font, gid, NULL, 16.0f, 5, &none) == DAEGUN_INVALID_ARGUMENT, "mode 5 was taken");
    CHECK(daegun_font_hinted_outline(font, gid, NULL, 16.0f, 2, &none) == DAEGUN_INVALID_ARGUMENT, "mode 2 was taken");
    CHECK(daegun_font_hinted_outline(font, gid, NULL, 0.0f, DAEGUN_HINT_FONT, &none) == DAEGUN_INVALID_ARGUMENT
              && none == NULL && daegun_last_error().len > 0,
          "a px of 0 was not refused with a reason");
    size_t n = 0, m = 0, nk = 0, ends_n = 0;
    const daegun_point *ps = daegun_hinted_outline_points(sub, &n), *pf = daegun_hinted_outline_points(forced, &m);
    const uint8_t *kinds = daegun_hinted_outline_kinds(sub, &nk);
    CHECK(n > 0 && m > 0 && nk == n, "the modes hinted %zu and %zu points, of %zu kinds", n, m, nk);
    int differ = n != m, on = 0, cubic = 0;
    for (size_t i = 0; i < n && i < m; i++) {
        differ |= ps[i].x != pf[i].x || ps[i].y != pf[i].y;
        on += kinds[i] == DAEGUN_POINT_ON_CURVE;
        cubic += kinds[i] == DAEGUN_POINT_CUBIC;
    }
    CHECK(cubic == 0, "a TrueType A had %d cubic controls", cubic);
    CHECK(differ, "the font's hints and the autohinter placed A alike, so the mode did not reach the hinter");
    CHECK(on > 0, "no point of A lies on its curve");
    const size_t *ends = daegun_hinted_outline_contours(sub, &ends_n);
    CHECK(ends && ends_n > 0 && ends[ends_n - 1] == n - 1, "the last contour ends at %zu of %zu points",
          ends && ends_n ? ends[ends_n - 1] : 0, n);

    float lo = INFINITY, hi = -INFINITY;
    for (size_t i = 0; i < n; i++) {
        lo = ps[i].x < lo ? ps[i].x : lo;
        hi = ps[i].x > hi ? ps[i].x : hi;
    }
    struct reach r = { 0, INFINITY, -INFINITY };
    daegun_pen pen = { reach_move, reach_line, reach_quad, NULL, NULL, &r };
    CHECK(daegun_hinted_outline_draw(sub, &pen) == DAEGUN_OK, "hinted_outline_draw failed");
    CHECK((size_t)r.contours == ends_n && r.min_x == lo && r.max_x == hi,
          "the drawn A spans %g to %g px in %d contours, its points %g to %g in %zu", r.min_x, r.max_x, r.contours, lo,
          hi, ends_n);
    bool may = false;
    CHECK(daegun_hint_mode_may_autohint(DAEGUN_HINT_AUTO, &may) == DAEGUN_OK && may
              && daegun_hint_mode_may_autohint(DAEGUN_HINT_FONT, &may) == DAEGUN_OK && !may,
          "the modes that may autohint are not AUTO's");
    daegun_hinted_outline_free(sub);
    daegun_hinted_outline_free(forced);
    daegun_font_free(font);
}

/* Readers no other check reaches, each against a fixture's stored values. */
static void the_remaining_readers_answer(void)
{
    daegun_font *inter = open_font(INTER);
    if (inter) {
        uint32_t h = 0;
        CHECK(daegun_font_glyph_id(inter, 'H', &h) == DAEGUN_OK, "Inter has no H");
        daegun_path *plain = daegun_path_new(), *heavy = daegun_path_new();
        daegun_pen to_plain = { 0 }, to_heavy = { 0 };
        (void)daegun_path_as_pen(plain, &to_plain);
        (void)daegun_path_as_pen(heavy, &to_heavy);
        daegun_location *black = location_at(inter, WGHT, 900.0);
        CHECK(daegun_font_outline(inter, h, NULL, &to_plain) == DAEGUN_OK, "H at the default failed");
        CHECK(daegun_font_outline(inter, h, black, &to_heavy) == DAEGUN_OK, "H at wght 900 failed");
        daegun_bounds pb = { 0 }, hb = { 0 };
        CHECK(daegun_path_bounds(plain, &pb) == DAEGUN_OK && daegun_path_bounds(heavy, &hb) == DAEGUN_OK
                  && hb.x_max - hb.x_min > pb.x_max - pb.x_min,
              "H at wght 900 is no wider than the default");
        daegun_path_free(plain);
        daegun_path_free(heavy);
        daegun_location_free(black);

        size_t added = 0;
        daegun_cache_stats stats = { 9, 9 };
        CHECK(daegun_font_prewarm(inter, &h, 1, NULL, &added) == DAEGUN_OK && added == 1, "prewarm took %zu", added);
        CHECK(daegun_font_cache_stats(inter, DAEGUN_CACHE_OUTLINE, &stats) == DAEGUN_OK && stats.entries == 1
                  && stats.bytes > 0,
              "the warmed cache holds %zu outlines", stats.entries);
        CHECK(daegun_font_clear_cache(inter, DAEGUN_CACHE_OUTLINE) == DAEGUN_OK
                  && daegun_font_cache_stats(inter, DAEGUN_CACHE_OUTLINE, &stats) == DAEGUN_OK && stats.entries == 0
                  && stats.bytes == 0,
              "the cleared cache holds %zu outlines in %zu bytes", stats.entries, stats.bytes);
        daegun_font_free(inter);
    }

    daegun_font *stix = open_font(STIX);
    if (stix) {
        uint32_t h = 0;
        CHECK(daegun_font_glyph_id(stix, 'H', &h) == DAEGUN_OK, "STIX has no H");
        daegun_cff_hints *hints = NULL;
        CHECK(daegun_font_cff_hints(stix, h, NULL, &hints) == DAEGUN_OK, "STIX's H has no CFF hints");
        size_t n = 0;
        const daegun_cff_stem *stems = daegun_cff_hints_stems(hints, &n);
        CHECK(stems && n > 0, "STIX's H declares %zu stems", n);
        for (size_t i = 0; stems && i < n; i++) {
            CHECK(isfinite(stems[i].min) && isfinite(stems[i].max), "stem %zu's edges are not finite", i);
        }
        daegun_cff_hints_free(hints);

        /* E's nine stems take two bytes a mask, and its second mask switches them at point 20. */
        uint32_t e = 0;
        CHECK(daegun_font_glyph_id(stix, 'E', &e) == DAEGUN_OK, "STIX has no E");
        CHECK(daegun_font_cff_hints(stix, e, NULL, &hints) == DAEGUN_OK, "STIX's E has no CFF hints");
        daegun_cff_hints_stems(hints, &n);
        size_t masks = 0, last = 0;
        CHECK(daegun_cff_hints_mask_count(hints, &masks) == DAEGUN_OK && masks == 5, "STIX's E sets %zu masks", masks);
        for (size_t i = 0; i < masks; i++) {
            daegun_cff_mask mask = { 0, NULL, 0 };
            CHECK(daegun_cff_hints_mask_at(hints, i, &mask) == DAEGUN_OK && mask.bits && n == 9 && mask.bits_len == 2
                      && mask.point >= last,
                  "mask %zu holds %zu bytes for %zu stems from point %zu", i, mask.bits_len, n, mask.point);
            CHECK(i != 1 || (mask.point == 20 && mask.bits[0] == 115 && mask.bits[1] == 128),
                  "E's second mask is not 115 128 at 20");
            last = mask.point;
        }
        daegun_cff_mask past = { 0 };
        CHECK(daegun_cff_hints_mask_at(hints, masks, &past) == DAEGUN_INVALID_ARGUMENT, "a mask past the last");
        daegun_cff_hints_free(hints);
        daegun_cff_hints *ttf = (daegun_cff_hints *)stix;
        daegun_font *inter_again = open_font(INTER);
        CHECK(daegun_font_cff_hints(inter_again, 1, NULL, &ttf) == DAEGUN_ABSENT && ttf == NULL,
              "a TrueType glyph had CFF hints");
        daegun_font_free(inter_again);
        daegun_font_free(stix);
    }

    daegun_font *bungee = open_font("assets/test-fonts/bungee-tint/BungeeTint-Regular.ttf");
    if (bungee) {
        uint32_t a = 0;
        CHECK(daegun_font_glyph_id(bungee, 'A', &a) == DAEGUN_OK, "Bungee has no A");
        daegun_colr_layer l0[4], l1[4];
        size_t n0 = 0, n1 = 0;
        CHECK(daegun_font_colr_layers(bungee, a, 0, l0, 4, &n0) == DAEGUN_OK, "A in palette 0");
        CHECK(daegun_font_colr_layers(bungee, a, 1, l1, 4, &n1) == DAEGUN_OK, "A in palette 1");
        CHECK(n0 == 2 && n1 == 2, "A has %zu and %zu layers, not 2", n0, n1);
        if (n0 == 2 && n1 == 2) {
            CHECK(l0[0].color.r == 201 && l0[0].color.g == 9 && l0[0].color.b == 0,
                  "palette 0 colors A's first layer %u %u %u", l0[0].color.r, l0[0].color.g, l0[0].color.b);
            CHECK(l1[0].color.r == 255 && l1[0].color.g == 255 && l1[0].color.b == 255,
                  "palette 1 does not color it white");
            CHECK(l0[0].glyph == l1[0].glyph && !l0[0].foreground, "the palettes layer different glyphs");
        }
        daegun_font_free(bungee);
    }

    daegun_font *glyphs = open_font(COLR);
    if (glyphs) {
        daegun_palette p[4];
        size_t n = 0;
        CHECK(daegun_font_palettes(glyphs, p, 4, &n) == DAEGUN_OK, "palettes failed");
        CHECK(n == 3, "%zu palettes, not 3", n);
        if (n == 3) {
            CHECK(!p[0].light_safe && !p[0].dark_safe && p[1].dark_safe && !p[1].light_safe && p[2].light_safe
                      && !p[2].dark_safe && !p[0].has_name_id && p[2].index == 2,
                  "the palette flags are not CPAL's");
        }
        daegun_rgba colors[16] = {{0}};
        size_t nc = 0;
        CHECK(daegun_font_palette_colors(glyphs, 2, colors, 16, &nc) == DAEGUN_OK && nc == 14 && colors[0].r == 252
                  && colors[0].g == 113 && colors[0].b == 24 && colors[0].a == 255,
              "palette 2: %zu colors, the first %u,%u,%u", nc, colors[0].r, colors[0].g, colors[0].b);
        /* Glyph 148's gradient takes the text color at its middle stop. */
        daegun_paint_graph *graph = NULL;
        size_t stops = 0;
        CHECK(daegun_font_paint_graph(glyphs, 148, NULL, 0, &graph) == DAEGUN_OK, "glyph 148 has no paint graph");
        const daegun_color_stop *s = daegun_paint_graph_stops(graph, &stops);
        CHECK(stops == 3 && !s[0].foreground && s[1].foreground && !s[2].foreground,
              "glyph 148's stops are not plain, text, plain");
        daegun_paint_graph_free(graph);
        daegun_font_free(glyphs);
    }

    daegun_font *emoji = open_font("assets/test-fonts/noto-color-emoji/NotoColorEmoji.ttf");
    if (emoji) {
        uint32_t smile = 0;
        CHECK(daegun_font_glyph_id(emoji, 0x1F600, &smile) == DAEGUN_OK, "no glyph for U+1F600");
        daegun_bitmap *b = NULL;
        CHECK(daegun_font_bitmap(emoji, smile, 64, &b) == DAEGUN_OK, "no bitmap for U+1F600");
        daegun_bitmap_info info = { 0 };
        CHECK(daegun_bitmap_info_of(b, &info) == DAEGUN_OK, "no bitmap info");
        CHECK(info.format == DAEGUN_BITMAP_PNG && info.data && info.data_len > 8 && memcmp(info.data, "\x89PNG", 4) == 0
                  && info.ppem == 109,
              "not a PNG at 109 ppem");
        CHECK(info.top == 101 && info.width == 0 && !info.mirrored, "a CBDT PNG placed wrong");
        daegun_bitmap_free(b);
        daegun_font_free(emoji);
    }

    size_t len = 0;
    uint8_t *ttc = slurp("assets/test-fonts/test-fixtures/EBGaramond-InterVariable.ttc", &len);
    if (ttc) {
        daegun_font *second = NULL;
        uint32_t n = 0;
        size_t faces = 0;
        CHECK(daegun_face_count(ttc, len, &faces) == DAEGUN_OK && faces == 2, "the collection holds %zu faces",
              faces);
        CHECK(daegun_font_open_collection(ttc, len, 1, &second) == DAEGUN_OK, "the collection's second face");
        CHECK(daegun_font_glyph_count(second, &n) == DAEGUN_OK && n == 2937,
              "the collection's second face has %u glyphs, not Inter's 2,937", n);
        daegun_font_free(second);
        /* Read in place, so the bytes stay for the program's life, as the static open asks. */
        static uint8_t *kept;
        kept = ttc;
        daegun_font *placed = NULL;
        n = 0;
        CHECK(daegun_font_open_static(kept, len, 1, &placed) == DAEGUN_OK, "the second face read in place");
        CHECK(daegun_font_glyph_count(placed, &n) == DAEGUN_OK && n == 2937,
              "the second face read in place has %u glyphs, not 2,937", n);
        daegun_font_free(placed);
        placed = NULL;
        CHECK(daegun_font_open_static(kept, len, 2, &placed) == DAEGUN_INVALID_ARGUMENT && !placed,
              "a third face of two was read in place");
    }
}

/* A miter's tip overflows f32 long before the width does, which is an overflow; a NaN width is an argument. */
static void a_stroke_that_overflows_is_refused(void)
{
    daegun_path *p = daegun_path_new();
    (void)daegun_path_move_to(p, 0.0f, 0.0f);
    (void)daegun_path_line_to(p, 1000.0f, 0.0f);
    (void)daegun_path_line_to(p, 0.0f, 1.0f);
    daegun_path *sink = daegun_path_new();
    daegun_pen pen = { 0 };
    (void)daegun_path_as_pen(sink, &pen);
    const float widths[2] = { 1e33f, NAN };
    const daegun_status want[2] = { DAEGUN_OVERFLOW, DAEGUN_INVALID_ARGUMENT };
    for (int i = 0; i < 2; i++) {
        daegun_stroke_style style = { widths[i], DAEGUN_CAP_BUTT, DAEGUN_JOIN_MITER, 1e30f };
        CHECK(daegun_path_stroke(p, &style, 0.0f, &pen) == want[i], "stroke width %g answered otherwise",
              (double)widths[i]);
        CHECK(daegun_path_stroke_simplified(p, &style, 0.0f, &pen) == want[i],
              "simplified stroke width %g answered otherwise", (double)widths[i]);
    }
    size_t n = 1;
    daegun_path_verbs(sink, &n);
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
    uint16_t u16 = 0;
    uint32_t index = 0;
    CHECK(daegun_read_u16_be(HIDDEN_NULL, 4, 0, &u16) == DAEGUN_NULL, "a NULL buffer of 4 bytes was read");
    CHECK(daegun_read_u16_be(NULL, 0, 0, &u16) == DAEGUN_INVALID_ARGUMENT, "a read past an empty buffer was taken");
    CHECK(daegun_bytes_window(NULL, 4, 0, 2) == NULL, "a NULL buffer of 4 bytes gave a window");
    CHECK(daegun_coverage_index(HIDDEN_NULL, 10, 3, &index) == DAEGUN_NULL, "a NULL coverage table was searched");

    daegun_font *none = NULL;
    size_t faces = 9;
    CHECK(daegun_font_open(NULL, 0, &none) == DAEGUN_MALFORMED, "an empty font was not malformed");
    CHECK(daegun_font_open_owned(NULL, 0, 0, &none) == DAEGUN_MALFORMED, "an empty owned font was not malformed");
    CHECK(daegun_font_open_collection(NULL, 0, 0, &none) == DAEGUN_MALFORMED, "an empty collection opened");
    CHECK(daegun_font_open_static(NULL, 0, 0, &none) == DAEGUN_MALFORMED, "an empty font was read in place");
    CHECK(daegun_face_count(NULL, 0, &faces) == DAEGUN_OK && faces == 0, "an empty collection held %zu", faces);
    CHECK(daegun_write_u16_be(NULL, 0, 0, 1) == DAEGUN_INVALID_ARGUMENT, "a write past an empty buffer was taken");
    CHECK(daegun_write_u16_be(HIDDEN_NULL, 4, 0, 1) == DAEGUN_NULL, "a NULL buffer of 4 bytes was written");
    daegun_subpixel_layout *layout = NULL;
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 0, 1, 0, 0, NULL, 0, &layout) == DAEGUN_INVALID_ARGUMENT,
          "no taps and no weights was taken");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 1, 1, 0, 0, HIDDEN_NULL, 3, &layout) == DAEGUN_NULL,
          "NULL weights with a count were taken");
    CHECK(none == NULL && layout == NULL, "a refused call delivered a handle");

    daegun_path *p = daegun_path_new();
    (void)daegun_path_move_to(p, 0.0f, 0.0f);
    size_t n = 99;
    CHECK(daegun_path_verbs(p, NULL) == NULL, "verbs were lent with no count to say how many");
    CHECK(daegun_path_verbs(p, &n) && n == 1, "the path holds %zu verbs", n);
    daegun_path_free(p);

    daegun_shape_options opts = { 0 };
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.features_len = 2;
    daegun_shaped_run *run = NULL;
    CHECK(daegun_font_shape(font, TXT("ab"), NULL, &opts, &run) == DAEGUN_NULL && run == NULL,
          "NULL features with a count shaped");
    opts.features_len = 0;
    opts.before_len = 3;
    CHECK(daegun_font_shape(font, TXT("ab"), NULL, &opts, &run) == DAEGUN_NULL, "NULL context with a count shaped");
    CHECK(daegun_font_shape(font, HIDDEN_NULL, 2, NULL, NULL, &run) == DAEGUN_NULL, "NULL text with a count shaped");
    CHECK(daegun_font_shape(font, NULL, 0, NULL, NULL, &run) == DAEGUN_OK, "no text at all was refused");
    daegun_shaped_run_free(run);

    /* A NULL result is DAEGUN_NULL whatever the input, not ABSENT or INVALID_ARGUMENT when there was nothing to
     * give. */
    uint32_t glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &glyphs) == DAEGUN_OK, "glyph_count failed");
    daegun_path *empty = daegun_path_new();
    CHECK(daegun_font_hinted_outline(font, glyphs, NULL, 16.0f, DAEGUN_HINT_FONT, NULL) == DAEGUN_NULL,
          "a glyph past the end with a NULL out");
    CHECK(daegun_font_colr_layers(font, 5, 0, NULL, 0, NULL) == DAEGUN_NULL, "COLR layers with a NULL count");
    CHECK(daegun_general_category(0xD800, NULL) == DAEGUN_NULL, "a surrogate with a NULL out");
    CHECK(daegun_path_bounds(empty, NULL) == DAEGUN_NULL, "bounds of nothing into NULL");
    daegun_path_free(empty);

    static const uint8_t junk[4] = { 0xFF, 0xFF, 0xFF, 0xFF };
    size_t offsets[4];
    size_t held = 0;
    CHECK(daegun_font_bitmap(font, 5, 16, NULL) == DAEGUN_NULL, "a missing bitmap with a NULL out");
    CHECK(daegun_font_paint_graph(font, 5, NULL, 0, NULL) == DAEGUN_NULL, "a missing paint graph with a NULL out");
    CHECK(daegun_font_cff_hints(font, 5, NULL, NULL) == DAEGUN_NULL, "missing CFF hints with a NULL out");
    CHECK(daegun_font_name(font, 999, NULL, NULL) == DAEGUN_NULL, "a missing name with a NULL out");
    CHECK(daegun_font_index_allowance(font, NULL) == DAEGUN_NULL, "the index allowance into NULL");
    CHECK(daegun_parse_loca(junk, 4, 0, SIZE_MAX, offsets, 4, NULL) == DAEGUN_NULL, "loca with a NULL count");
    CHECK(daegun_parse_loca(junk, 4, 0, SIZE_MAX, HIDDEN_NULL, 4, &held) == DAEGUN_NULL && held == 0,
          "loca into NULL with room for 4, or its count set");
    CHECK(daegun_parse_loca(HIDDEN_NULL, 4, 0, SIZE_MAX, offsets, 4, &held) == DAEGUN_NULL,
          "a NULL loca of 4 bytes was read");
    daegun_path *drawn = daegun_path_new();
    daegun_pen into_drawn = { 0 };
    (void)daegun_path_as_pen(drawn, &into_drawn);
    CHECK(daegun_font_prepared_outline(font, 5, NULL, NAN, NULL, NULL, NULL) == DAEGUN_NULL,
          "a NULL pen was not DAEGUN_NULL before the px");
    daegun_path_free(drawn);
    CHECK(daegun_read_u16_be(junk, 4, 100, NULL) == DAEGUN_NULL, "a read past the end into NULL");
    CHECK(daegun_coverage_index(junk, 4, 3, NULL) == DAEGUN_NULL, "a junk coverage table with a NULL out");
    CHECK(daegun_coverage_glyphs(junk, 4, 0, NULL, 0, NULL) == DAEGUN_NULL, "junk coverage glyphs with a NULL count");
    CHECK(daegun_aat_state_table_parse(junk, 4, 0, 10, NULL) == DAEGUN_NULL, "a junk state table with a NULL out");
    CHECK(daegun_ankr_version(junk, 1, NULL) == DAEGUN_NULL, "a short ankr version with a NULL out");
    CHECK(daegun_ankr_control_point(junk, 4, 100, NULL, NULL) == DAEGUN_NULL, "a point past the end into NULL");
    CHECK(daegun_feature_variations_parse(junk, 4, NULL) == DAEGUN_NULL, "junk FeatureVariations with a NULL out");
    CHECK(daegun_ivs_parse(junk, 4, 0, NULL) == DAEGUN_NULL, "a junk variation store with a NULL out");
    CHECK(daegun_delta_set_index_map_parse(junk, 4, 0, NULL) == DAEGUN_NULL, "a junk index map with a NULL out");
    daegun_table_map *tables = daegun_table_map_new();
    CHECK(daegun_table_map_get(tables, DAEGUN_TAG('Z', 'Z', 'Z', 'Z'), NULL) == DAEGUN_NULL,
          "a missing mapped table with a NULL out");
    daegun_table_map_free(tables);
    daegun_font_free(font);
}

/* TEXT: bytes that aren't UTF-8 answer DAEGUN_INVALID_ARGUMENT with the byte, whether text, context or an
 * identifier. */
static void strings_are_utf8(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    daegun_shaped_run *run = NULL;
    daegun_shape_options opts = { 0 };
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.before = "\xc3";
    opts.before_len = 1;
    CHECK(daegun_font_shape(font, TXT("ab"), NULL, &opts, &run) == DAEGUN_INVALID_ARGUMENT,
          "context that isn't UTF-8 was taken");
    CHECK(strstr(daegun_last_error().data, "UTF-8") != NULL, "the reason was \"%s\"", daegun_last_error().data);
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.language = "\xff";
    CHECK(daegun_font_shape(font, TXT("ab"), NULL, &opts, &run) == DAEGUN_INVALID_ARGUMENT,
          "a language that isn't UTF-8 was taken");
    daegun_layout_options lopts = { 0 };
    CHECK(daegun_layout_options_default(&lopts, sizeof lopts) == DAEGUN_OK, "layout defaults failed");
    lopts.language = "\xff";
    daegun_layout *layout = NULL;
    CHECK(daegun_font_layout(font, TXT("ab"), NULL, &lopts, &layout) == DAEGUN_INVALID_ARGUMENT,
          "a layout language that isn't UTF-8 was taken");
    CHECK(run == NULL && layout == NULL, "a refused call delivered a result");

    /* A language that is UTF-8 but no BCP 47 tag the font knows is no error: it matches nothing. */
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.language = "zz-Qqqq";
    CHECK(daegun_font_shape(font, TXT("ab"), NULL, &opts, &run) == DAEGUN_OK, "an unknown language was refused");
    daegun_shaped_run_free(run);
    daegun_font_free(font);
}

/* A 60-degree corner miters at the default limit of 4, which a zeroed style asks for, and bevels at 1. */
static void a_zeroed_miter_limit_is_the_default(void)
{
    daegun_path *corner = daegun_path_new();
    (void)daegun_path_move_to(corner, 0.0f, 0.0f);
    (void)daegun_path_line_to(corner, 100.0f, 0.0f);
    (void)daegun_path_line_to(corner, 50.0f, 86.6f);
    const float limits[3] = { 0.0f, 4.0f, 1.0f };
    double reach[3] = { 0, 0, 0 };
    for (int i = 0; i < 3; i++) {
        daegun_stroke_style style = { 10.0f, DAEGUN_CAP_BUTT, DAEGUN_JOIN_MITER, limits[i] };
        daegun_path *stroked = daegun_path_new();
        daegun_pen into = { 0 };
        (void)daegun_path_as_pen(stroked, &into);
        daegun_bounds box = { 0 };
        CHECK(daegun_path_stroke(corner, &style, 0.25f, &into) == DAEGUN_OK
                  && daegun_path_bounds(stroked, &box) == DAEGUN_OK,
              "stroking at a limit of %g failed", (double)limits[i]);
        reach[i] = box.x_max;
        daegun_path_free(stroked);
    }
    CHECK(reach[0] == reach[1], "a limit of 0 reached %g, the default %g", reach[0], reach[1]);
    CHECK(reach[1] > 108.0 && reach[2] < 105.0, "the miter reached %g and the bevel %g", reach[1], reach[2]);
    daegun_path_free(corner);
}

/* A scene holds each glyph's outline once, and past the 2^20 points in all a small font's may hold it is
 * DAEGUN_LIMIT_EXCEEDED: P layers 17 composites of 64,000 points, R one composite eight times. */
static void a_scene_holds_each_outline_once_within_a_bound(void)
{
    daegun_font *font = open_font("assets/test-fonts/test-fixtures/colr-clip.ttf");
    if (!font) return;
    uint32_t past = 0, repeated = 0;
    CHECK(daegun_font_glyph_id(font, 'P', &past) == DAEGUN_OK, "the fixture has no P");
    CHECK(daegun_font_glyph_id(font, 'R', &repeated) == DAEGUN_OK, "the fixture has no R");
    daegun_scene *scene = (daegun_scene *)font;
    CHECK(daegun_font_color_scene(font, past, NULL, NULL, &scene) == DAEGUN_LIMIT_EXCEEDED && !scene,
          "a scene past 2^20 points was built");
    CHECK(daegun_font_color_scene(font, repeated, NULL, NULL, &scene) == DAEGUN_OK, "eight layers failed");
    size_t paths = 0, ops = 0;
    CHECK(daegun_scene_path_count(scene, &paths) == DAEGUN_OK && paths == 1, "eight layers of one glyph held %zu paths",
          paths);
    daegun_scene_ops(scene, &ops);
    CHECK(ops >= 8, "eight layers drew %zu ops", ops);
    daegun_scene_free(scene);
    daegun_font_free(font);
}

static void put_u16(uint8_t *p, uint16_t v) { p[0] = (uint8_t)(v >> 8); p[1] = (uint8_t)v; }
static void put_u32(uint8_t *p, uint32_t v) { put_u16(p, (uint16_t)(v >> 16)); put_u16(p + 2, (uint16_t)v); }

/* The fixture with D replaced by one contour of 10,000 on-curve points, climbing 1000 units and
 * falling back every 2 across. loca is rewritten long so the glyph can be any size. */
static daegun_font *fixture_with_zigzag(uint32_t *out_gid)
{
    daegun_font *base = open_font("assets/test-fonts/test-fixtures/hinted.ttf");
    if (!base) return NULL;
    daegun_table_map *map = NULL;
    uint32_t gid = 0, glyphs = 0;
    daegun_bytes glyf = { 0, 0 }, loca = { 0, 0 }, head = { 0, 0 };
    const daegun_tag GLYF = DAEGUN_TAG('g', 'l', 'y', 'f'), LOCA = DAEGUN_TAG('l', 'o', 'c', 'a');
    const daegun_tag HEAD = DAEGUN_TAG('h', 'e', 'a', 'd');
    CHECK(daegun_font_glyph_id(base, 'D', &gid) == DAEGUN_OK, "the fixture has no D");
    CHECK(daegun_font_glyph_count(base, &glyphs) == DAEGUN_OK, "no glyph count");
    CHECK(daegun_font_instance_tables(base, NULL, &map) == DAEGUN_OK, "no tables");
    CHECK(daegun_table_map_get(map, GLYF, &glyf) == DAEGUN_OK && daegun_table_map_get(map, LOCA, &loca) == DAEGUN_OK
              && daegun_table_map_get(map, HEAD, &head) == DAEGUN_OK,
          "the fixture lacks glyf, loca or head");

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
    for (uint32_t g = 0; g < glyphs; g++) {
        size_t from = 2u * ((size_t)loca.data[2 * g] << 8 | loca.data[2 * g + 1]);
        size_t to = 2u * ((size_t)loca.data[2 * g + 2] << 8 | loca.data[2 * g + 3]);
        put_u32(new_loca + 4 * g, (uint32_t)used);
        if (g == gid) {
            memcpy(new_glyf + used, zig, ZIG);
            used += ZIG;
        } else {
            memcpy(new_glyf + used, glyf.data + from, to - from);
            used += to - from;
        }
    }
    put_u32(new_loca + 4 * glyphs, (uint32_t)used);
    memcpy(new_head, head.data, head.len);
    put_u16(new_head + 50, 1);
    CHECK(daegun_table_map_set(map, GLYF, new_glyf, used) == DAEGUN_OK
              && daegun_table_map_set(map, LOCA, new_loca, 4u * (glyphs + 1u)) == DAEGUN_OK
              && daegun_table_map_set(map, HEAD, new_head, head.len) == DAEGUN_OK,
          "the patched tables were not stored");

    daegun_blob *blob = NULL;
    daegun_font *font = NULL;
    size_t len = 0;
    CHECK(daegun_table_map_build(map, &blob) == DAEGUN_OK, "the patched tables did not build");
    const uint8_t *bytes = daegun_blob_data(blob, &len);
    CHECK(daegun_font_open(bytes, len, &font) == DAEGUN_OK, "the patched font did not open");
    daegun_blob_free(blob);
    free(zig);
    free(new_glyf);
    free(new_loca);
    free(new_head);
    daegun_table_map_free(map);
    daegun_font_free(base);
    *out_gid = gid;
    return font;
}

/* Each turn of the zigzag, stroked this wide and round, takes 256 points: 10,000 turns take millions, each reaching
 * the pen as it's worked out, where past DAEGUN_MAX_FLATTEN_POINTS they were refused. */
static void a_prepared_stroke_past_the_cap_draws(void)
{
    uint32_t gid = 0;
    daegun_font *font = fixture_with_zigzag(&gid);
    if (!font) return;
    daegun_outline_options wide = { 0 }, bold = { 0 };
    CHECK(daegun_outline_options_default(&wide, sizeof wide) == DAEGUN_OK, "outline defaults failed");
    bold = wide;
    wide.has_stroke = true;
    wide.stroke.width = 40000.0f;
    wide.stroke.join = DAEGUN_JOIN_ROUND;
    wide.stroke.cap = DAEGUN_CAP_ROUND;
    bold.has_embolden = true;
    bold.embolden = 40000.0f;
    const daegun_outline_options *both[2] = { &wide, &bold };
    for (int k = 0; k < 2; k++) {
        pen_tally drawn = { 0, 0, 0, 0, 0 };
        daegun_pen pen = tally_pen(&drawn);
        CHECK(daegun_font_prepared_outline(font, gid, NULL, 1024.0f, both[k], &pen, NULL) == DAEGUN_OK,
              "%s past the cap was refused", k ? "an embolden" : "a stroke");
        CHECK(drawn.lines > DAEGUN_MAX_FLATTEN_POINTS, "%s drew %d lines", k ? "the embolden" : "the stroke",
              drawn.lines);
    }
    daegun_font_free(font);
}

static void a_prepared_outline_reaches_the_pen(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    uint32_t gid = 0;
    uint16_t upm = 0;
    CHECK(daegun_font_glyph_id(font, 'H', &gid) == DAEGUN_OK && daegun_font_units_per_em(font, &upm) == DAEGUN_OK,
          "no glyph for 'H'");

    daegun_outline_options given = { 0 };
    CHECK(daegun_outline_options_default(&given, sizeof given) == DAEGUN_OK, "outline defaults failed");

    pen_tally plain_calls = { 0, 0, 0, 0, 0 }, bold_calls = { 0, 0, 0, 0, 0 };
    daegun_pen plain_pen = tally_pen(&plain_calls);
    daegun_pen bold_pen = tally_pen(&bold_calls);
    daegun_prepared_glyph plain = { 0 }, bold = { 0 };
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, NULL, &plain_pen, &plain) == DAEGUN_OK,
          "prepared_outline failed: %s", daegun_last_error().data);
    daegun_outline_options opts = given;
    opts.has_embolden = true;
    opts.embolden = 80.0f;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, &opts, &bold_pen, &bold) == DAEGUN_OK,
          "a bold prepared_outline failed");
    CHECK(plain_calls.moves > 0 && plain_calls.closes > 0, "the plain H drew nothing");
    CHECK(bold_calls.moves > plain_calls.moves, "the bold H added no stroke contours");
    CHECK(bold.advance_width > plain.advance_width, "bold advance %f is not wider than %f", (double)bold.advance_width,
          (double)plain.advance_width);
    /* The advance in font units, a different route to the same number. */
    double advance = 0;
    CHECK(daegun_font_advance(font, gid, NULL, DAEGUN_HORIZONTAL, &advance) == DAEGUN_OK
              && fabs(plain.advance_width - advance * 32.0 / upm) < 0.001,
          "a 32 px H advances %f px, not %f", (double)plain.advance_width, advance * 32.0 / upm);
    CHECK(!plain.hinted, "hinted with no hinting asked for");

    opts = given;
    opts.hinting = DAEGUN_HINT_AUTO_FORCE;
    daegun_prepared_glyph hinted = { 0 };
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 16.0f, &opts, &plain_pen, &hinted) == DAEGUN_OK
              && hinted.hinted,
          "AutoForce did not hint the H");

    CHECK(daegun_font_prepared_outline(font, gid, NULL, 0.0f, NULL, &plain_pen, NULL) == DAEGUN_INVALID_ARGUMENT,
          "zero px was taken");
    opts = given;
    opts.has_oblique = true;
    opts.oblique = INFINITY;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 16.0f, &opts, &plain_pen, NULL) == DAEGUN_INVALID_ARGUMENT,
          "an infinite oblique was taken");
    opts = given;
    opts.has_embolden = true;
    opts.embolden = 1e38f;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 65535.0f, &opts, &plain_pen, NULL) == DAEGUN_OVERFLOW,
          "an embolden that overflows once scaled was not DAEGUN_OVERFLOW");
    opts = given;
    opts.hinting = 9;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 16.0f, &opts, &plain_pen, NULL) == DAEGUN_INVALID_ARGUMENT,
          "hint mode 9 was taken");

    /* A half-scale transform halves the width; a stroke widens it; a NaN in either is refused. */
    daegun_path *full = daegun_path_new(), *half = daegun_path_new(), *stroked = daegun_path_new();
    daegun_pen into_full = { 0 }, into_half = { 0 }, into_stroked = { 0 };
    (void)daegun_path_as_pen(full, &into_full);
    (void)daegun_path_as_pen(half, &into_half);
    (void)daegun_path_as_pen(stroked, &into_stroked);
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, NULL, &into_full, NULL) == DAEGUN_OK, "plain H failed");
    opts = given;
    opts.has_transform = true;
    const float halving[6] = { 0.5f, 0.0f, 0.0f, 0.5f, 0.0f, 0.0f };
    memcpy(opts.transform, halving, sizeof halving);
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, &opts, &into_half, NULL) == DAEGUN_OK, "half H failed");
    opts.transform[3] = NAN;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, &opts, &plain_pen, NULL) == DAEGUN_INVALID_ARGUMENT,
          "a NaN transform was taken");
    opts = given;
    opts.has_stroke = true;
    opts.stroke.width = 40.0f;
    opts.stroke.join = DAEGUN_JOIN_ROUND;
    opts.stroke.cap = DAEGUN_CAP_ROUND;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, &opts, &into_stroked, NULL) == DAEGUN_OK,
          "stroked H failed");
    opts.stroke.width = NAN;
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 32.0f, &opts, &plain_pen, NULL) == DAEGUN_INVALID_ARGUMENT,
          "a NaN stroke width was taken");
    daegun_bounds f = { 0 }, h = { 0 }, s = { 0 };
    CHECK(daegun_path_bounds(full, &f) == DAEGUN_OK && daegun_path_bounds(half, &h) == DAEGUN_OK
              && daegun_path_bounds(stroked, &s) == DAEGUN_OK,
          "a prepared H drew nothing");
    CHECK(fabs((h.x_max - h.x_min) - (f.x_max - f.x_min) / 2.0) < 0.01, "a half-scale H is %g wide against %g",
          h.x_max - h.x_min, f.x_max - f.x_min);
    CHECK((s.x_max - s.x_min) > (f.x_max - f.x_min) + 0.5, "a 40-unit stroke widened the H from %g to %g",
          f.x_max - f.x_min, s.x_max - s.x_min);
    daegun_path_free(full);
    daegun_path_free(half);
    daegun_path_free(stroked);

    uint32_t glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &glyphs) == DAEGUN_OK, "glyph_count failed");
    daegun_status past = daegun_font_prepared_outline(font, glyphs, NULL, 16.0f, NULL, &plain_pen, NULL);
    CHECK(past != DAEGUN_OK && past != DAEGUN_ABSENT && daegun_last_error().len > 0,
          "a glyph past the end answered %d, not a failure with its reason", past);
    CHECK(daegun_font_prepared_outline(NULL, gid, NULL, 16.0f, NULL, &plain_pen, NULL) == DAEGUN_NULL,
          "a NULL font was taken");
    CHECK(daegun_font_prepared_outline(font, gid, NULL, 16.0f, NULL, NULL, NULL) == DAEGUN_NULL,
          "a NULL pen was taken");
    daegun_font_free(font);
}

static void the_subpixel_filter_is_readable(void)
{
    daegun_subpixel_layout *gray = NULL, *rgb = NULL, *vbgr = NULL;
    CHECK(daegun_subpixel_layout_grayscale(&gray) == DAEGUN_OK, "grayscale failed");
    CHECK(daegun_subpixel_layout_horizontal(DAEGUN_STRIPES_RGB, &rgb) == DAEGUN_OK, "horizontal RGB failed");
    CHECK(daegun_subpixel_layout_vertical(DAEGUN_STRIPES_BGR, &vbgr) == DAEGUN_OK, "vertical BGR failed");
    if (!gray || !rgb || !vbgr) return;

    uint8_t x = 0, y = 0, ch = 0;
    int8_t ox = 0, oy = 0;
    size_t px = 9, py = 9, n = 0;
    bool is_gray = false;
    CHECK(daegun_subpixel_layout_channels(gray, &ch) == DAEGUN_OK
              && daegun_subpixel_layout_is_grayscale(gray, &is_gray) == DAEGUN_OK,
          "grayscale's channels failed");
    const float *w = daegun_subpixel_layout_weights(gray, 0, &n);
    CHECK(ch == 1 && is_gray && w && n == 1 && w[0] == 1.0f, "grayscale is not one channel of weight 1");
    CHECK(daegun_subpixel_layout_weights(gray, 1, &n) == NULL, "grayscale has a second channel");
    CHECK(daegun_subpixel_layout_pad(gray, &px, &py) == DAEGUN_OK && px == 0 && py == 0,
          "grayscale asks for padding %zu, %zu", px, py);

    /* FIR5 over three stripes: 7 taps, 3x1 samples, starting 2 samples left, so one pixel of pad. */
    CHECK(daegun_subpixel_layout_oversample(rgb, &x, &y) == DAEGUN_OK && x == 3 && y == 1, "RGB oversamples %ux%u", x,
          y);
    CHECK(daegun_subpixel_layout_taps(rgb, &x, &y) == DAEGUN_OK && x == 7 && y == 1, "RGB has %ux%u taps", x, y);
    CHECK(daegun_subpixel_layout_origin(rgb, &ox, &oy) == DAEGUN_OK && ox == -2 && oy == 0, "RGB starts at %d, %d", ox,
          oy);
    CHECK(daegun_subpixel_layout_pad(rgb, &px, &py) == DAEGUN_OK && px == 1 && py == 0, "RGB pads %zu, %zu", px, py);
    for (size_t c = 0; c < 3; c++) {
        w = daegun_subpixel_layout_weights(rgb, c, &n);
        float sum = 0.0f;
        for (size_t i = 0; w && i < n; i++) sum += w[i];
        CHECK(w && n == 7 && fabsf(sum - 1.0f) < 1e-6f, "RGB channel %zu: %zu weights summing to %f", c, n,
              (double)sum);
        CHECK(w && w[c] == 8.0f / 256.0f && w[c + 2] == 86.0f / 256.0f, "RGB channel %zu is not FIR5 at %zu", c, c);
    }
    w = daegun_subpixel_layout_weights(vbgr, 0, &n);
    CHECK(daegun_subpixel_layout_pad(vbgr, &px, &py) == DAEGUN_OK && px == 0 && py == 1 && w && w[2] == 8.0f / 256.0f,
          "vertical BGR is not FIR5 in reverse order down the column");

    /* Unfiltered: three samples down the pixel, each channel weighing only its own stripe, blue's first. */
    daegun_subpixel_layout *bare = NULL;
    CHECK(daegun_subpixel_layout_unfiltered(DAEGUN_STRIPES_BGR, DAEGUN_VERTICAL, &bare) == DAEGUN_OK,
          "unfiltered vertical BGR failed");
    CHECK(daegun_subpixel_layout_oversample(bare, &x, &y) == DAEGUN_OK && x == 1 && y == 3,
          "unfiltered vertical BGR oversamples %ux%u", x, y);
    for (size_t c = 0; c < 3; c++) {
        w = daegun_subpixel_layout_weights(bare, c, &n);
        CHECK(w && n == 3 && w[2 - c] == 1.0f && w[0] + w[1] + w[2] == 1.0f,
              "unfiltered channel %zu does not weigh its own stripe alone", c);
    }

    /* Each layout's key is its own. */
    daegun_subpixel_layout *all[4] = { gray, rgb, vbgr, bare };
    uint64_t keys[4];
    for (int i = 0; i < 4; i++) {
        CHECK(daegun_subpixel_layout_key(all[i], &keys[i]) == DAEGUN_OK, "layout %d has no key", i);
        for (int j = 0; j < i; j++) {
            CHECK(keys[i] != keys[j], "layouts %d and %d share a key", i, j);
        }
    }
    daegun_subpixel_layout_free(bare);

    const float weights[9] = { 0.25f, 0.5f, 0.25f, 0.5f, 0.5f, 0.0f, 0.0f, 0.5f, 0.5f };
    daegun_subpixel_layout *custom = NULL, *none = NULL;
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 3, 1, -1, 0, weights, 9, &custom) == DAEGUN_OK,
          "a three-tap filter was refused");
    for (size_t c = 0; c < 3; c++) {
        w = daegun_subpixel_layout_weights(custom, c, &n);
        CHECK(w && n == 3 && w[0] == weights[c * 3] && w[2] == weights[c * 3 + 2],
              "channel %zu did not keep its weights", c);
    }
    CHECK(daegun_subpixel_layout_pad(custom, &px, &py) == DAEGUN_OK && px == 1 && py == 0,
          "a filter reaching one sample left does not pad one pixel");
    CHECK(daegun_subpixel_layout_from_weights(0, 1, 3, 1, -1, 0, weights, 9, &none) == DAEGUN_INVALID_ARGUMENT,
          "a zero oversample was taken");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 99, 1, 0, 0, weights, 9, &none) == DAEGUN_INVALID_ARGUMENT,
          "a tap count past the table was taken");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 3, 1, 0, 0, weights, 8, &none) == DAEGUN_INVALID_ARGUMENT,
          "eight weights were taken for three channels");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 2, 1, 0, 0, weights, 9, &none) == DAEGUN_INVALID_ARGUMENT,
          "three weights a channel were taken for two taps");

    /* The header's limits are literal copies of the library's, so the library has to agree. */
    static float wide[3 * DAEGUN_MAX_WEIGHTS];
    for (size_t i = 0; i < 3 * DAEGUN_MAX_WEIGHTS; i++) {
        wide[i] = 1.0f / DAEGUN_MAX_WEIGHTS;
    }
    daegun_subpixel_layout *widest = NULL;
    CHECK(daegun_subpixel_layout_from_weights(DAEGUN_MAX_OVERSAMPLE, DAEGUN_MAX_OVERSAMPLE, DAEGUN_MAX_TAPS,
                                              DAEGUN_MAX_TAPS, 0, 0, wide, 3 * DAEGUN_MAX_WEIGHTS, &widest)
              == DAEGUN_OK,
          "the widest filter the header allows was refused");
    w = daegun_subpixel_layout_weights(widest, 2, &n);
    CHECK(w && n == DAEGUN_MAX_WEIGHTS, "the widest filter holds %zu weights a channel", n);
    daegun_subpixel_layout_free(widest);
    CHECK(daegun_subpixel_layout_from_weights(DAEGUN_MAX_OVERSAMPLE + 1, 1, 1, 1, 0, 0, wide, 3, &none)
              == DAEGUN_INVALID_ARGUMENT,
          "an oversample past DAEGUN_MAX_OVERSAMPLE was taken");
    CHECK(daegun_subpixel_layout_from_weights(1, 1, DAEGUN_MAX_TAPS + 1, 1, 0, 0, wide, 3 * (DAEGUN_MAX_TAPS + 1),
                                              &none)
              == DAEGUN_INVALID_ARGUMENT,
          "a tap count past DAEGUN_MAX_TAPS was taken");
    const float unfinite[3] = { NAN, 1.0f, 1.0f };
    CHECK(daegun_subpixel_layout_from_weights(1, 1, 1, 1, 0, 0, unfinite, 3, &none) == DAEGUN_INVALID_ARGUMENT,
          "a NaN weight was taken");
    CHECK(strstr(daegun_last_error().data, "finite") != NULL, "a NaN weight left the reason \"%s\"",
          daegun_last_error().data);
    CHECK(none == NULL, "a refused layout was written");
    CHECK(daegun_subpixel_layout_horizontal(2, &none) == DAEGUN_INVALID_ARGUMENT, "stripe order 2 was taken");
    CHECK(daegun_subpixel_layout_unfiltered(DAEGUN_STRIPES_RGB, 2, &none) == DAEGUN_INVALID_ARGUMENT,
          "orientation 2 was taken");
    CHECK(daegun_subpixel_layout_horizontal(DAEGUN_STRIPES_RGB, NULL) == DAEGUN_NULL, "a NULL out was taken");
    CHECK(daegun_subpixel_layout_taps(NULL, &x, &y) == DAEGUN_NULL, "taps of NULL");
    CHECK(daegun_subpixel_layout_taps(rgb, &x, NULL) == DAEGUN_NULL, "taps into one NULL out");
    CHECK(daegun_subpixel_layout_weights(NULL, 0, &n) == NULL, "weights of NULL");
    daegun_subpixel_layout_free(NULL);
    daegun_subpixel_layout_free(custom);
    daegun_subpixel_layout_free(vbgr);
    daegun_subpixel_layout_free(rgb);
    daegun_subpixel_layout_free(gray);
}

/* The rasterizer lends each kind of image through daegun_image: coverage, a display's stripes, a color glyph's RGBA,
 * a path and both distance fields, each the size its channels say; with the raster cache on, a glyph drawn again
 * is lent from the cache, at the same address. */
static void the_rasterizer_lends_its_images(const char *path)
{
    daegun_font *font = open_font(path);
    daegun_font *colr = open_font(COLR);
    daegun_rasterizer *r = daegun_rasterizer_new();
    if (!font || !colr) goto done;
    uint32_t g = 0;
    CHECK(daegun_font_glyph_id(font, 'g', &g) == DAEGUN_OK && g != 0, "the font has no g");

    daegun_image image;
    memset(&image, 0, sizeof image);
    CHECK(daegun_rasterizer_glyph(r, font, g, NULL, 20.0f, NULL, &image) == DAEGUN_OK, "a glyph didn't draw: %s",
          daegun_last_error().data);
    size_t inked = 0;
    for (size_t i = 0; i < image.pixels_len; i++) inked += image.pixels[i] != 0;
    CHECK(image.channels == 1 && image.width > 0 && image.height > 0 && image.stride == image.width
              && image.pixels_len == image.stride * image.height && inked > 0,
          "a glyph's coverage is %ux%u, %u channels, stride %zu, %zu bytes, %zu inked", image.width, image.height,
          image.channels, image.stride, image.pixels_len, inked);

    daegun_subpixel_layout *rgb = NULL;
    CHECK(daegun_subpixel_layout_horizontal(DAEGUN_STRIPES_RGB, &rgb) == DAEGUN_OK, "no RGB layout");
    daegun_outline_options outline;
    CHECK(daegun_outline_options_default(&outline, sizeof outline) == DAEGUN_OK, "outline defaults failed");
    outline.hinting = DAEGUN_HINT_AUTO_FORCE;
    daegun_raster_options opts;
    CHECK(daegun_raster_options_default(&opts, sizeof opts) == DAEGUN_OK && opts.size == sizeof opts
              && opts.outline == NULL && opts.layout == NULL && opts.gamma == DAEGUN_GAMMA_LINEAR && !opts.darken,
          "raster option defaults are not zero");
    opts.layout = rgb;
    opts.outline = &outline;
    opts.gamma = DAEGUN_GAMMA_SRGB;
    opts.text = (daegun_rgba){20, 20, 20, 255};
    opts.contrast = 0.5f;
    opts.darken = true;
    opts.offset[0] = 0.25f;
    CHECK(daegun_rasterizer_glyph(r, font, g, NULL, 20.0f, &opts, &image) == DAEGUN_OK && image.channels == 3
              && image.stride == 3 * (size_t)image.width,
          "a striped glyph didn't draw three channels: %s", daegun_last_error().data);
    daegun_image kept = image;
    opts.gamma = 7;
    CHECK(daegun_rasterizer_glyph(r, font, g, NULL, 20.0f, &opts, &image) == DAEGUN_INVALID_ARGUMENT
              && image.pixels == kept.pixels && image.width == kept.width,
          "an unknown gamma was taken, or the image changed");

    CHECK(daegun_rasterizer_color_glyph(r, font, g, NULL, 20.0f, NULL, NULL, &image) == DAEGUN_ABSENT,
          "a glyph without a scene drew one");
    uint32_t glyphs = 0, painted = 0;
    CHECK(daegun_font_glyph_count(colr, &glyphs) == DAEGUN_OK, "glyph_count failed");
    for (uint32_t c = 1; c < glyphs && !painted; c++) {
        daegun_status st = daegun_rasterizer_color_glyph(r, colr, c, NULL, 32.0f, NULL, NULL, &image);
        if (st == DAEGUN_OK && image.channels == 4 && image.pixels_len == 4 * (size_t)image.width * image.height
            && image.width > 0) {
            painted = c;
        }
    }
    CHECK(painted != 0, "no color glyph drew RGBA");
    daegun_raster_options striped;
    CHECK(daegun_raster_options_default(&striped, sizeof striped) == DAEGUN_OK, "raster option defaults failed");
    striped.layout = rgb;
    CHECK(daegun_rasterizer_color_glyph(r, colr, painted, NULL, 32.0f, &striped, NULL, &image)
              == DAEGUN_INVALID_ARGUMENT,
          "a color glyph took a subpixel layout");

    daegun_path *square = daegun_path_new();
    CHECK(daegun_path_move_to(square, 0.0f, 0.0f) == DAEGUN_OK && daegun_path_line_to(square, 4.0f, 0.0f) == DAEGUN_OK
              && daegun_path_line_to(square, 4.0f, 4.0f) == DAEGUN_OK
              && daegun_path_line_to(square, 0.0f, 4.0f) == DAEGUN_OK && daegun_path_close(square) == DAEGUN_OK,
          "the square didn't build");
    const double moved[6] = {1.0, 0.0, 0.0, 1.0, 0.5, 0.0};
    CHECK(daegun_rasterizer_path(r, square, DAEGUN_FILL_NONZERO, NULL, &image) == DAEGUN_OK && image.width == 4
              && image.height == 4 && image.pixels[0] == 255,
          "a 4x4 square drew %ux%u", image.width, image.height);
    CHECK(daegun_rasterizer_path(r, square, DAEGUN_FILL_EVENODD, moved, &image) == DAEGUN_OK && image.width == 5
              && image.pixels[0] == 128,
          "a square moved half a pixel drew %ux%u, its first pixel %u", image.width, image.height,
          image.pixels_len ? image.pixels[0] : 0);
    CHECK(daegun_rasterizer_path(r, square, 2, NULL, &image) == DAEGUN_INVALID_ARGUMENT, "an unknown rule was taken");
    uint8_t into[6 * 6];
    memset(into, 7, sizeof into);
    daegun_target target = {-1, 5, 6, 6, 6, into, sizeof into};
    CHECK(daegun_rasterizer_path_into(r, square, DAEGUN_FILL_NONZERO, NULL, &target) == DAEGUN_OK && into[0] == 0
              && into[6 + 1] == 255 && into[4 * 6 + 4] == 255 && into[5 * 6 + 5] == 0,
          "a 4x4 square drew into a target as %u, %u, %u, %u", into[0], into[7], into[28], into[35]);
    target.pixels_len = sizeof into - 1;
    CHECK(daegun_rasterizer_path_into(r, square, DAEGUN_FILL_NONZERO, NULL, &target) == DAEGUN_INVALID_ARGUMENT,
          "a target its pixels don't hold was taken");
    daegun_path_free(square);
    /* A glyph and a color glyph drawn into targets their images' size match the images to a level. */
    for (int color = 0; color < 2; color++) {
        daegun_status st = color ? daegun_rasterizer_color_glyph(r, colr, painted, NULL, 32.0f, NULL, NULL, &image)
                                 : daegun_rasterizer_glyph(r, font, g, NULL, 20.0f, NULL, &image);
        daegun_image lent = image;
        uint8_t *copy = malloc(lent.pixels_len), *drawn = malloc(lent.pixels_len);
        if (st != DAEGUN_OK || !copy || !drawn) {
            CHECK(false, "the image to match didn't draw");
            free(copy);
            free(drawn);
            continue;
        }
        memcpy(copy, lent.pixels, lent.pixels_len);
        daegun_target t = {lent.left, lent.top, lent.width, lent.height, lent.stride, drawn, lent.pixels_len};
        st = color ? daegun_rasterizer_color_glyph_into(r, colr, painted, NULL, 32.0f, NULL, NULL, &t)
                   : daegun_rasterizer_glyph_into(r, font, g, NULL, 20.0f, NULL, &t);
        int worst = 0;
        for (size_t i = 0; i < lent.pixels_len; i++) {
            int gap = abs((int)copy[i] - (int)drawn[i]);
            worst = gap > worst ? gap : worst;
        }
        free(copy);
        free(drawn);
        CHECK(st == DAEGUN_OK && worst <= 1, "a %s drawn into a target is %d levels from its image",
              color ? "color glyph" : "glyph", worst);
    }
    uint8_t rgba[4] = {9, 9, 9, 9};
    daegun_target pixel = {0, 0, 1, 1, 4, rgba, sizeof rgba};
    CHECK(daegun_rasterizer_color_glyph_into(r, font, g, NULL, 20.0f, NULL, NULL, &pixel) == DAEGUN_ABSENT
              && rgba[0] == 9,
          "a glyph without a scene drew one into a target");

    CHECK(daegun_rasterizer_distance_field(r, font, g, NULL, 32.0f, DAEGUN_FIELD_SDF, 4.0f, &image) == DAEGUN_OK
              && image.channels == 1,
          "an SDF didn't draw one channel");
    CHECK(daegun_rasterizer_distance_field(r, font, g, NULL, 32.0f, DAEGUN_FIELD_MSDF, 4.0f, &image) == DAEGUN_OK
              && image.channels == 3,
          "an MSDF didn't draw three channels");
    CHECK(daegun_rasterizer_distance_field(r, font, g, NULL, 32.0f, 2, 4.0f, &image) == DAEGUN_INVALID_ARGUMENT
              && daegun_rasterizer_distance_field(r, font, g, NULL, 32.0f, DAEGUN_FIELD_SDF, 0.0f, &image)
                     == DAEGUN_INVALID_ARGUMENT,
          "an unknown field or a spread of 0 was taken");

    daegun_cache_stats stats = {9, 9};
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_RASTER, &stats) == DAEGUN_OK && stats.entries == 0,
          "the raster cache kept %zu images before it was on", stats.entries);
    CHECK(daegun_font_set_cache_budget(font, DAEGUN_CACHE_RASTER, 1 << 20) == DAEGUN_OK, "no raster budget");
    daegun_image first = { 0 }, again = { 0 };
    CHECK(daegun_rasterizer_glyph(r, font, g, NULL, 20.0f, NULL, &first) == DAEGUN_OK
              && daegun_rasterizer_glyph(r, font, g, NULL, 20.0f, NULL, &again) == DAEGUN_OK
              && first.pixels == again.pixels && first.pixels_len == again.pixels_len,
          "a glyph drawn again wasn't lent from the cache");
    CHECK(daegun_font_cache_stats(font, DAEGUN_CACHE_RASTER, &stats) == DAEGUN_OK && stats.entries == 1
              && stats.bytes > again.pixels_len,
          "the raster cache holds %zu images, %zu bytes", stats.entries, stats.bytes);
    CHECK(daegun_font_clear_cache(font, DAEGUN_CACHE_RASTER) == DAEGUN_OK
              && daegun_font_cache_stats(font, DAEGUN_CACHE_RASTER, &stats) == DAEGUN_OK && stats.entries == 0,
          "clearing left an image");
    daegun_subpixel_layout_free(rgb);
done:
    daegun_rasterizer_free(r);
    daegun_font_free(colr);
    daegun_font_free(font);
}

static void character_properties_need_no_font(void)
{
    int32_t gc = -1;
    CHECK(daegun_general_category('A', &gc) == DAEGUN_OK && gc == DAEGUN_GC_UPPERCASE_LETTER, "'A' is category %d", gc);
    CHECK(daegun_general_category('a', &gc) == DAEGUN_OK && gc == DAEGUN_GC_LOWERCASE_LETTER, "'a' is category %d", gc);
    CHECK(daegun_general_category(' ', &gc) == DAEGUN_OK && gc == DAEGUN_GC_SPACE_SEPARATOR, "space is category %d",
          gc);
    CHECK(daegun_general_category('5', &gc) == DAEGUN_OK && gc == DAEGUN_GC_DECIMAL_NUMBER, "'5' is category %d", gc);
    CHECK(daegun_general_category(0xE000, &gc) == DAEGUN_OK && gc == DAEGUN_GC_PRIVATE_USE, "U+E000 is category %d",
          gc);
    CHECK(daegun_general_category(0xD800, &gc) == DAEGUN_INVALID_ARGUMENT, "a surrogate was categorized");
    CHECK(daegun_general_category(0x110000, &gc) == DAEGUN_INVALID_ARGUMENT, "past U+10FFFF was taken");

    daegun_tag script = 0;
    CHECK(daegun_script(0x0628, &script) == DAEGUN_OK && script == DAEGUN_TAG('A', 'r', 'a', 'b'),
          "U+0628's script is %08X", (unsigned)script);
    CHECK(daegun_script('1', &script) == DAEGUN_OK && script == DAEGUN_TAG('Z', 'y', 'y', 'y'),
          "'1''s script is %08X", (unsigned)script);
    CHECK(daegun_script(0x0301, &script) == DAEGUN_OK && script == DAEGUN_TAG('Z', 'i', 'n', 'h'),
          "a combining acute's script is %08X", (unsigned)script);
    CHECK(daegun_script(0xD800, &script) == DAEGUN_INVALID_ARGUMENT, "a surrogate was given a script");

    uint32_t form = 0;
    CHECK(daegun_vertical_form(0x2014, &form) == DAEGUN_OK && form == 0xFE31, "the em dash's vertical form is U+%04X",
          form);
    CHECK(daegun_vertical_form(0x3001, &form) == DAEGUN_OK && form == 0xFE11,
          "the ideographic comma's form is U+%04X", form);
    CHECK(daegun_vertical_form('A', &form) == DAEGUN_ABSENT, "'A' has a vertical form");

    int32_t vo = -1;
    CHECK(daegun_vertical_orientation(0x4E00, &vo) == DAEGUN_OK && vo == DAEGUN_VO_UPRIGHT,
          "a CJK ideograph sets as %d in vertical text, not upright", vo);
    CHECK(daegun_vertical_orientation('A', &vo) == DAEGUN_OK && vo == DAEGUN_VO_ROTATED,
          "a Latin capital sets as %d in vertical text, not rotated", vo);
    CHECK(daegun_vertical_orientation(0x3002, &vo) == DAEGUN_OK && vo == DAEGUN_VO_TRANSFORMED_UPRIGHT,
          "an ideographic full stop sets as %d, not upright in its vertical form", vo);
}

static void the_format_walkers_read_a_real_table(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    uint32_t num_glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &num_glyphs) == DAEGUN_OK, "glyph_count failed");

    daegun_bytes hvar = { NULL, 0 };
    CHECK(daegun_font_table(font, DAEGUN_TAG('H', 'V', 'A', 'R'), &hvar) == DAEGUN_OK, "Inter has no HVAR");
    uint32_t ivs_off = 0;
    CHECK(daegun_read_u32_be(hvar.data, hvar.len, 4, &ivs_off) == DAEGUN_OK, "HVAR has no itemVariationStoreOffset");
    daegun_ivs *ivs = NULL;
    daegun_status st = daegun_ivs_parse(hvar.data, hvar.len, ivs_off, &ivs);
    CHECK(st == DAEGUN_OK, "parsing HVAR's store returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        size_t axes = 0, regions = 0, ivds = 0;
        CHECK(daegun_ivs_axis_count(ivs, &axes) == DAEGUN_OK && axes == 2, "Inter's store covers %zu axes", axes);
        CHECK(daegun_ivs_region_count(ivs, &regions) == DAEGUN_OK && regions > 0, "the store has no regions");
        CHECK(daegun_ivs_ivd_count(ivs, &ivds) == DAEGUN_OK && ivds > 0, "the store has no subtables");

        size_t region_axes = 0;
        const daegun_region_axis *ra = daegun_ivs_region(ivs, 0, &region_axes);
        CHECK(ra && region_axes == axes, "region 0 spans %zu axes of %zu", region_axes, axes);
        for (size_t a = 0; ra && a < region_axes; a++) {
            CHECK(ra[a].start <= ra[a].peak && ra[a].peak <= ra[a].end, "a region runs %g / %g / %g, out of order",
                  ra[a].start, ra[a].peak, ra[a].end);
        }
        CHECK(daegun_ivs_region(ivs, regions, &region_axes) == NULL, "a region past the end was lent");

        size_t rows = 0, region_count = 0;
        CHECK(daegun_ivs_ivd_rows(ivs, 0, &rows) == DAEGUN_OK, "ivd_rows failed");
        CHECK(daegun_ivs_ivd_rows(ivs, ivds, &rows) == DAEGUN_INVALID_ARGUMENT, "rows past the last subtable");
        const size_t *indices = daegun_ivs_ivd_region_indices(ivs, 0, &region_count);
        CHECK(indices != NULL || region_count == 0, "region indices came back null");
        if (rows > 0) {
            size_t row_len = 0;
            const int32_t *row = daegun_ivs_ivd_row(ivs, 0, 0, &row_len);
            CHECK(row != NULL && row_len == region_count, "a row has %zu deltas for %zu regions", row_len,
                  region_count);
        }
        CHECK(daegun_ivs_ivd_row(ivs, 0, rows, &rows) == NULL, "a row past the end was lent");

        double location[2] = { 0.0, 0.0 };
        double *scalars = calloc(regions, sizeof *scalars);
        size_t scalar_count = 0;
        CHECK(scalars && daegun_ivs_region_scalars(ivs, location, 2, scalars, regions, &scalar_count) == DAEGUN_OK
                  && scalar_count == regions,
              "%zu scalars for %zu regions", scalar_count, regions);
        double delta = 99.0;
        CHECK(daegun_ivs_delta(ivs, 0, 0, scalars, scalar_count, &delta) == DAEGUN_OK && delta == 0.0,
              "at the default location every delta is zero, got %g", delta);
        CHECK(daegun_ivs_region_scalars(ivs, location, 2, scalars, 0, &scalar_count) == DAEGUN_INVALID_ARGUMENT
                  && scalar_count == regions,
              "a buffer of 0 was not refused with the count it needs");
        free(scalars);
        daegun_ivs_free(ivs);
    }

    const uint8_t junk[8] = { 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff };
    daegun_ivs *bad = NULL;
    CHECK(daegun_ivs_parse(junk, sizeof junk, 0, &bad) != DAEGUN_OK && bad == NULL, "junk parsed as a store");
    daegun_feature_variations *fv = NULL;
    CHECK(daegun_feature_variations_parse(junk, sizeof junk, &fv) == DAEGUN_ABSENT, "junk carried FeatureVariations");
    daegun_ankr *ankr = NULL;
    CHECK(daegun_ankr_parse(junk, sizeof junk, num_glyphs, &ankr) == DAEGUN_MALFORMED, "junk parsed as an ankr table");
    CHECK(strstr(daegun_last_error().data, "ankr") != NULL, "junk ankr left the reason \"%s\"",
          daegun_last_error().data);

    daegun_font_free(font);
}

static void loca_and_glyf_are_walkable(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    daegun_bytes head = { NULL, 0 }, loca = { NULL, 0 }, glyf = { NULL, 0 };
    if (daegun_font_table(font, DAEGUN_TAG('l', 'o', 'c', 'a'), &loca) != DAEGUN_OK
        || daegun_font_table(font, DAEGUN_TAG('g', 'l', 'y', 'f'), &glyf) != DAEGUN_OK
        || daegun_font_table(font, DAEGUN_TAG('h', 'e', 'a', 'd'), &head) != DAEGUN_OK) {
        CHECK(0, "Inter lacks loca, glyf or head");
        daegun_font_free(font);
        return;
    }

    int16_t loca_format = 0;
    CHECK(daegun_read_i16_be(head.data, head.len, 50, &loca_format) == DAEGUN_OK, "head has no indexToLocFormat");
    uint32_t num_glyphs = 0;
    CHECK(daegun_font_glyph_count(font, &num_glyphs) == DAEGUN_OK, "glyph_count failed");

    size_t n = 0;
    size_t *offs = malloc(((size_t)num_glyphs + 1) * sizeof *offs);
    CHECK(offs && daegun_parse_loca(loca.data, loca.len, loca_format, num_glyphs, offs, (size_t)num_glyphs + 1, &n)
              == DAEGUN_OK,
          "parse_loca failed");
    CHECK(n == (size_t)num_glyphs + 1 && offs[0] <= offs[n - 1], "loca gave %zu offsets for %u glyphs", n, num_glyphs);

    /* Past 65,535 glyphs only as many as loca holds, so a count of SIZE_MAX reads loca and no further. */
    size_t held = 0;
    CHECK(daegun_parse_loca(loca.data, loca.len, loca_format, SIZE_MAX, NULL, 0, &held) == DAEGUN_OK && held <= 0x10000,
          "a glyph count of SIZE_MAX read %zu offsets", held);

    uint32_t gid = 0;
    CHECK(daegun_font_glyph_id(font, 'A', &gid) == DAEGUN_OK, "Inter has no A");
    pen_tally raw = { 0, 0, 0, 0, 0 };
    daegun_pen pen = tally_pen(&raw);
    daegun_status st = daegun_outline_glyf_bytes(glyf.data, glyf.len, false, offs, n, gid, &pen);
    CHECK(st == DAEGUN_OK && raw.moves > 0, "outlining glyph %u from raw bytes returned %d: %s", gid, st,
          daegun_last_error().data);

    pen_tally via_font = { 0, 0, 0, 0, 0 };
    daegun_pen fpen = tally_pen(&via_font);
    CHECK(daegun_font_outline(font, gid, NULL, &fpen) == DAEGUN_OK, "drawing via the font failed");
    CHECK(raw.moves == via_font.moves && raw.lines == via_font.lines && raw.quads == via_font.quads
              && raw.closes == via_font.closes,
          "the raw tier drew %d/%d/%d/%d and the font tier %d/%d/%d/%d for the same glyph", raw.moves, raw.lines,
          raw.quads, raw.closes, via_font.moves, via_font.lines, via_font.quads, via_font.closes);

    free(offs);
    daegun_font_free(font);
}

static void the_atlas_packer_packs(void)
{
    daegun_shelf_packer *p = daegun_shelf_packer_new(64, 64);
    CHECK(p != NULL, "a new packer is null");

    daegun_rect a = { 9, 9, 9, 9 }, b = { 9, 9, 9, 9 };
    CHECK(daegun_shelf_packer_insert(p, 16, 16, &a) == DAEGUN_OK && a.w == 16 && a.h == 16,
          "a 16x16 request came back %zux%zu", a.w, a.h);
    CHECK(daegun_shelf_packer_insert(p, 16, 16, &b) == DAEGUN_OK, "the second insert failed");
    /* Two rectangles in one atlas must not overlap, the one thing a packer is for. */
    int disjoint = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
    CHECK(disjoint, "the packer overlapped (%zu,%zu %zux%zu) with (%zu,%zu %zux%zu)", a.x, a.y, a.w, a.h, b.x, b.y,
          b.w, b.h);
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
    CHECK(daegun_shelf_packer_reset(NULL) == DAEGUN_NULL, "a null packer was reset");
    daegun_shelf_packer_free(p);
    daegun_shelf_packer_free(NULL);
}

static void the_rules_a_caller_would_get_wrong(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    daegun_line_metrics lm = { 0 };
    double h = 0.0;
    CHECK(daegun_font_line_metrics(font, NULL, DAEGUN_HORIZONTAL, &lm) == DAEGUN_OK
              && daegun_line_metrics_line_height(&lm, &h) == DAEGUN_OK,
          "line metrics failed");
    /* hhea gives 1984, -494 and 0: the difference, not the sum that looks as plausible. */
    CHECK(h == 2478.0, "Inter's line height is %g, not 1984 + 494", h);
    CHECK(daegun_line_metrics_line_height(NULL, &h) == DAEGUN_NULL, "the height of NULL");

    const struct { int32_t mode; bool may; } modes[] = {
        { DAEGUN_HINT_NONE, false },
        { DAEGUN_HINT_FONT, false },
        { DAEGUN_HINT_AUTO, true },
        { DAEGUN_HINT_AUTO_FORCE, true },
    };
    for (size_t i = 0; i < sizeof modes / sizeof modes[0]; i++) {
        bool may = !modes[i].may;
        CHECK(daegun_hint_mode_may_autohint(modes[i].mode, &may) == DAEGUN_OK && may == modes[i].may,
              "hint mode %d may autohint: %d", modes[i].mode, may);
    }
    bool may = false;
    CHECK(daegun_hint_mode_may_autohint(5, &may) == DAEGUN_INVALID_ARGUMENT, "hint mode 5 was taken");
    CHECK(daegun_hint_mode_may_autohint(2, &may) == DAEGUN_INVALID_ARGUMENT, "hint mode 2 was taken");

    const struct { int32_t level; bool graphemes, monotone; } levels[] = {
        { DAEGUN_CLUSTER_MONOTONE_GRAPHEMES, true, true },
        { DAEGUN_CLUSTER_MONOTONE_CHARACTERS, false, true },
        { DAEGUN_CLUSTER_CHARACTERS, false, false },
        { DAEGUN_CLUSTER_GRAPHEMES, true, false },
    };
    for (size_t i = 0; i < sizeof levels / sizeof levels[0]; i++) {
        bool g = !levels[i].graphemes, m = !levels[i].monotone;
        CHECK(daegun_cluster_level_is_graphemes(levels[i].level, &g) == DAEGUN_OK
                  && daegun_cluster_level_is_monotone(levels[i].level, &m) == DAEGUN_OK && g == levels[i].graphemes
                  && m == levels[i].monotone,
              "cluster level %d is graphemes=%d monotone=%d", levels[i].level, g, m);
    }
    CHECK(daegun_cluster_level_is_monotone(4, &may) == DAEGUN_INVALID_ARGUMENT, "cluster level 4 was taken");
    daegun_font_free(font);
}

static void scripts_answer_about_themselves(void)
{
    const daegun_tag latn = DAEGUN_TAG('L', 'a', 't', 'n'), arab = DAEGUN_TAG('A', 'r', 'a', 'b');
    const daegun_tag zyyy = DAEGUN_TAG('Z', 'y', 'y', 'y'), zinh = DAEGUN_TAG('Z', 'i', 'n', 'h');

    /* The space is Common, so it joins the Latin run before it. */
    daegun_script_run runs[5];
    size_t n = 0;
    CHECK(daegun_script_runs(TXT("Ab \xd8\xa7\xd9\x84"), runs, 5, &n) == DAEGUN_OK && n == 2, "%zu runs", n);
    CHECK(n == 2 && runs[0].start == 0 && runs[0].end == 3 && runs[0].script == latn && runs[1].start == 3
              && runs[1].end == 7 && runs[1].script == arab,
          "the runs are not Latin to 3 then Arabic to 7");
    CHECK(daegun_script_runs(TXT(","), runs, 5, &n) == DAEGUN_OK && n == 1 && runs[0].script == zyyy,
          "a lone comma is not Common");

    daegun_tag tags[3];
    CHECK(daegun_script_opentype_tags(latn, tags, 3, &n) == DAEGUN_OK && n == 1
              && tags[0] == DAEGUN_TAG('l', 'a', 't', 'n'),
          "Latin maps to %zu OpenType tags", n);
    CHECK(daegun_script_opentype_tags(DAEGUN_TAG('D', 'e', 'v', 'a'), tags, 3, &n) == DAEGUN_OK && n == 3
              && tags[0] == DAEGUN_TAG('d', 'e', 'v', '3') && tags[1] == DAEGUN_TAG('d', 'e', 'v', '2')
              && tags[2] == DAEGUN_TAG('d', 'e', 'v', 'a'),
          "Devanagari's tags are not dev3, dev2, deva");
    CHECK(daegun_script_opentype_tags(DAEGUN_TAG('D', 'e', 'v', 'a'), tags, 2, &n) == DAEGUN_INVALID_ARGUMENT && n == 3,
          "two slots for three tags were not refused with the count");
    CHECK(daegun_script_opentype_tags(latn, NULL, 0, NULL) == DAEGUN_NULL, "a NULL count was taken");

    int32_t direction = -1;
    bool ctx = true;
    CHECK(daegun_script_direction(latn, &direction) == DAEGUN_OK
              && direction == DAEGUN_SCRIPT_DIRECTION_LEFT_TO_RIGHT,
          "Latin runs as %d", direction);
    CHECK(daegun_script_direction(arab, &direction) == DAEGUN_OK
              && direction == DAEGUN_SCRIPT_DIRECTION_RIGHT_TO_LEFT,
          "Arabic runs as %d", direction);
    CHECK(daegun_script_direction(zyyy, &direction) == DAEGUN_OK
              && direction == DAEGUN_SCRIPT_DIRECTION_LEFT_TO_RIGHT,
          "Common runs as %d", direction);
    CHECK(daegun_script_direction(DAEGUN_TAG('I', 't', 'a', 'l'), &direction) == DAEGUN_OK
              && direction == DAEGUN_SCRIPT_DIRECTION_LEFT_TO_RIGHT,
          "Old Italic, its letters left to right by their Bidi class, runs as %d", direction);
    CHECK(daegun_script_is_context_dependent(latn, &ctx) == DAEGUN_OK && !ctx, "Latin depends on its context");
    CHECK(daegun_script_is_context_dependent(arab, &ctx) == DAEGUN_OK && !ctx, "Arabic depends on its context");
    CHECK(daegun_script_is_context_dependent(zyyy, &ctx) == DAEGUN_OK && ctx, "Common stands alone");
    CHECK(daegun_script_is_context_dependent(zinh, &ctx) == DAEGUN_OK && ctx, "Inherited stands alone");

    daegun_str name = { NULL, 0 };
    CHECK(daegun_script_name(arab, &name) == DAEGUN_OK && str_is(name, "Arabic"), "Arab's name is not Arabic");
    CHECK(daegun_script_name(DAEGUN_TAG('a', 'R', 'A', 'B'), &name) == DAEGUN_OK && str_is(name, "Arabic"),
          "ISO 15924's codes are read in any case, and aRAB is not Arabic");
    CHECK(daegun_script_direction(DAEGUN_TAG('Q', 'a', 'a', 'a'), &direction) == DAEGUN_INVALID_ARGUMENT,
          "a private-use code named a script");
}

static void subsetting_maps_glyph_ids(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    /* The cmap maps A to glyph 2 and B to glyph 65. */
    uint32_t a = 0, b = 0;
    CHECK(daegun_font_glyph_id(font, 'A', &a) == DAEGUN_OK && a == 2 && daegun_font_glyph_id(font, 'B', &b) == DAEGUN_OK
              && b == 65,
          "A and B are glyphs %u and %u", a, b);
    const uint32_t keep[3] = { 0, a, b };

    /* Room for the font's glyph count always suffices. */
    uint32_t glyph_count = 0;
    CHECK(daegun_font_glyph_count(font, &glyph_count) == DAEGUN_OK, "glyph_count failed");
    uint32_t *gids = malloc(glyph_count * sizeof *gids);
    size_t held = 0;
    CHECK(gids && daegun_font_glyph_closure(font, keep, 3, NULL, gids, glyph_count, &held) == DAEGUN_OK,
          "closure failed");
    CHECK(gids && held >= 3 && gids[0] == 0, "the closure holds %zu glyphs", held);
    for (size_t i = 1; gids && i < held; i++) {
        CHECK(gids[i - 1] < gids[i], "the closure is not sorted and unique at %zu", i);
    }

    daegun_subset *sub = NULL;
    daegun_status st = daegun_font_subset(font, keep, 3, NULL, &sub);
    CHECK(st == DAEGUN_OK, "subset returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        uint32_t na = 0, nb = 0, notdef = 9;
        CHECK(daegun_subset_renumbered(sub, 0, &notdef) == DAEGUN_OK && notdef == 0, ".notdef moved to %u", notdef);
        CHECK(daegun_subset_renumbered(sub, a, &na) == DAEGUN_OK && daegun_subset_renumbered(sub, b, &nb) == DAEGUN_OK
                  && na != nb && na < held && nb < held,
              "A and B map to %u and %u in a subset of %zu", na, nb, held);
        /* The first glyph the closure skipped is dropped from the subset. */
        uint32_t dropped = 0;
        for (size_t i = 0; gids && i < held && gids[i] == dropped; i++) {
            dropped++;
        }
        uint32_t nd = 9;
        CHECK(daegun_subset_renumbered(sub, dropped, &nd) == DAEGUN_ABSENT && nd == 9, "dropped glyph %u maps to %u",
              dropped, nd);

        size_t len = 0;
        const uint8_t *ttf = daegun_subset_bytes(sub, &len);
        daegun_font *reopened = NULL;
        uint32_t count = 0;
        CHECK(ttf && daegun_font_open(ttf, len, &reopened) == DAEGUN_OK
                  && daegun_font_glyph_count(reopened, &count) == DAEGUN_OK && count == held,
              "the subset holds %u glyphs, its closure %zu", count, held);
        daegun_font_free(reopened);
        daegun_subset_free(sub);
    }
    free(gids);

    /* By text, the cmap keeps the text's characters and no others. */
    st = daegun_font_subset_text(font, TXT("AB"), NULL, &sub);
    CHECK(st == DAEGUN_OK, "subset_text returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        size_t len = 0;
        const uint8_t *ttf = daegun_subset_bytes(sub, &len);
        daegun_font *reopened = NULL;
        uint32_t na = 0, got = 0, none = 9;
        CHECK(ttf && daegun_font_open(ttf, len, &reopened) == DAEGUN_OK, "the text subset does not open");
        CHECK(daegun_subset_renumbered(sub, a, &na) == DAEGUN_OK
                  && daegun_font_glyph_id(reopened, 'A', &got) == DAEGUN_OK && got == na,
              "the text subset maps A to %u, its glyph map to %u", got, na);
        CHECK(daegun_font_glyph_id(reopened, 'C', &none) == DAEGUN_ABSENT, "the text subset kept C");
        daegun_font_free(reopened);
        daegun_subset_free(sub);
    }
    CHECK(daegun_font_subset_text(font, TXT("\xff"), NULL, &sub) == DAEGUN_INVALID_ARGUMENT, "bad UTF-8 was subset");
    daegun_font_free(font);
}

static void stat_values_are_readable(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    daegun_stat *stat = NULL;
    CHECK(daegun_font_stat(font, NULL, &stat) == DAEGUN_OK, "Inter has no STAT");
    if (!stat) {
        daegun_font_free(font);
        return;
    }

    /* Every number here is a value Inter's STAT stores. */
    size_t axes_n = 0;
    const daegun_stat_axis *axes = daegun_stat_axes(stat, &axes_n);
    CHECK(axes && axes_n == 3 && axes[0].tag == DAEGUN_TAG('o', 'p', 's', 'z') && axes[1].tag == WGHT
              && axes[2].tag == DAEGUN_TAG('i', 't', 'a', 'l') && axes[2].ordering == 2 && axes[1].has_name,
          "STAT's axes are not opsz, wght, ital");
    daegun_str s = { NULL, 0 };
    CHECK(daegun_stat_axis_name(stat, 1, &s) == DAEGUN_OK && str_is(s, "Weight"), "axis 1 is not named Weight");
    CHECK(daegun_stat_axis_name(stat, 3, &s) == DAEGUN_INVALID_ARGUMENT, "an axis past the last was named");
    CHECK(daegun_stat_elided_fallback_name(stat, &s) == DAEGUN_OK && str_is(s, "Regular"),
          "the elided fallback is not Regular");

    size_t n = 0, combo_n = 9;
    CHECK(daegun_stat_value_count(stat, &n) == DAEGUN_OK && n == 12, "STAT names %zu values", n);
    const daegun_axis_value *combos = daegun_stat_combo_values(stat, &combo_n);
    CHECK(combo_n == 0 && combos != NULL, "Inter has %zu combo pairs", combo_n);

    daegun_stat_value v = { 0 };
    CHECK(daegun_stat_value_at(stat, 1, &v) == DAEGUN_OK && v.kind == DAEGUN_STAT_SINGLE && v.axis_index == 0
              && v.value == 32.0 && !v.elidable && v.has_name && v.combo_count == 0,
          "value 1 is not opsz 32");
    CHECK(daegun_stat_value_name(stat, 1, &s) == DAEGUN_OK && str_is(s, "Display"), "value 1 is not Display");
    CHECK(daegun_stat_value_at(stat, 5, &v) == DAEGUN_OK && v.kind == DAEGUN_STAT_LINKED && v.axis_index == 1
              && v.value == 400.0 && v.linked_value == 700.0 && v.elidable && !v.older_sibling,
          "value 5 is not Regular, linked to 700 and elidable");
    CHECK(daegun_stat_value_name(stat, 5, &s) == DAEGUN_OK && str_is(s, "Regular"), "value 5 is not Regular");
    CHECK(daegun_stat_value_at(stat, 11, &v) == DAEGUN_OK && v.kind == DAEGUN_STAT_LINKED && v.axis_index == 2
              && v.value == 0.0 && v.linked_value == 1.0,
          "value 11 is not ital 0 linked to 1");
    CHECK(daegun_stat_value_at(stat, n, &v) == DAEGUN_INVALID_ARGUMENT, "a value past the last was read");
    CHECK(daegun_stat_value_at(stat, 0, NULL) == DAEGUN_NULL, "a NULL out was taken");
    daegun_stat_free(stat);
    daegun_stat_free(NULL);

    daegun_location *bold = location_at(font, WGHT, 700.0);
    CHECK(daegun_font_stat(font, bold, &stat) == DAEGUN_OK, "no STAT at Bold");
    if (stat) {
        CHECK(daegun_stat_value_count(stat, &n) == DAEGUN_OK && n > 0, "Bold's STAT names no values");
        daegun_stat_free(stat);
    }
    daegun_location_free(bold);
    daegun_font_free(font);
}

static void an_owned_buffer_opens_without_a_copy(const char *path)
{
    /* Through a volatile, as GCC reads the allocator's size and rightly warns at a constant this large. */
    volatile size_t huge = SIZE_MAX;
    CHECK(daegun_font_buffer_new(huge) == NULL, "a SIZE_MAX buffer was handed out");
    CHECK(daegun_font_buffer_new(0) == NULL, "a zero-length buffer was allocated");
    size_t len = 0;
    uint8_t *file = slurp(path, &len);
    if (!file) {
        CHECK(0, "could not read %s", path);
        return;
    }

    uint8_t *buf = daegun_font_buffer_new(len);
    CHECK(buf != NULL, "buffer_new returned null for %zu bytes", len);
    if (buf) memcpy(buf, file, len);
    daegun_font *font = NULL;
    daegun_status st = daegun_font_open_owned(buf, len, 0, &font);
    CHECK(st == DAEGUN_OK, "open_owned returned %d: %s", st, daegun_last_error().data);
    if (st == DAEGUN_OK) {
        daegun_font *copied = NULL;
        CHECK(daegun_font_open(file, len, &copied) == DAEGUN_OK, "the copying open failed");
        uint16_t upm_a = 0, upm_b = 0;
        uint32_t ga = 0, gb = 0;
        CHECK(daegun_font_units_per_em(font, &upm_a) == DAEGUN_OK
                  && daegun_font_units_per_em(copied, &upm_b) == DAEGUN_OK
                  && daegun_font_glyph_count(font, &ga) == DAEGUN_OK
                  && daegun_font_glyph_count(copied, &gb) == DAEGUN_OK && upm_a == 2048 && upm_b == 2048 && ga == 2937
                  && gb == 2937,
          "owned open gave upm %u/%u glyphs %u/%u", upm_a, upm_b, ga, gb);
        daegun_bytes h1 = { NULL, 0 }, h2 = { NULL, 0 };
        CHECK(daegun_font_table(font, DAEGUN_TAG('h', 'e', 'a', 'd'), &h1) == DAEGUN_OK
                  && daegun_font_table(copied, DAEGUN_TAG('h', 'e', 'a', 'd'), &h2) == DAEGUN_OK && h1.len == h2.len
                  && h1.len == 54 && memcmp(h1.data, h2.data, h1.len) == 0,
              "the owned font's head differs from the copied one's");
        daegun_font_free(copied);
        daegun_font_free(font);
    }

    uint8_t *unused = daegun_font_buffer_new(4096);
    CHECK(unused != NULL, "buffer_new(4096) returned null");
    if (unused) {
        unused[0] = 1;
        unused[4095] = 2;
    }
    daegun_font_buffer_free(unused, 4096);
    daegun_font_buffer_free(NULL, 0);

    daegun_font *none = NULL;
    CHECK(daegun_font_open_owned(NULL, 10, 0, &none) == DAEGUN_NULL, "a null buffer was accepted");
    uint8_t *junk = daegun_font_buffer_new(64);
    if (junk) memset(junk, 0xff, 64);
    CHECK(daegun_font_open_owned(junk, 64, 0, &none) == DAEGUN_MALFORMED && none == NULL, "junk parsed as a font");
    /* Taken with no out to hand a font to: freed, not leaked, as LSan would say. */
    uint8_t *orphan = daegun_font_buffer_new(len);
    if (orphan) memcpy(orphan, file, len);
    CHECK(daegun_font_open_owned(orphan, len, 0, NULL) == DAEGUN_NULL, "a NULL out was taken");
    /* A font file holds one face, so index 1 is refused, and the buffer is still the call's to free. */
    uint8_t *second = daegun_font_buffer_new(len);
    if (second) memcpy(second, file, len);
    CHECK(daegun_font_open_owned(second, len, 1, &none) == DAEGUN_INVALID_ARGUMENT && none == NULL,
          "a font file's face 1 opened");
    free(file);
}

static int tags_are(const daegun_tag *got, size_t count, const daegun_tag *want, size_t n)
{
    return count == n && memcmp(got, want, n * sizeof *want) == 0;
}

/* What Inter says about itself, every number a value its tables store. */
static void a_face_describes_itself(void)
{
    daegun_font *font = open_font(INTER);
    if (!font) return;
    double cap = 0, angle = -1, tracking = -1;
    uint32_t flags = 0;
    CHECK(daegun_font_cap_height(font, NULL, &cap) == DAEGUN_OK && cap == 1490.0, "cap height %g, not 1490", cap);
    CHECK(daegun_font_pdf_flags(font, NULL, &flags) == DAEGUN_OK && flags == DAEGUN_PDF_SYMBOLIC,
          "descriptor flags %u, not Symbolic", flags);
    CHECK(daegun_font_italic_angle(font, NULL, &angle) == DAEGUN_OK && angle == 0.0, "italic angle %g", angle);
    CHECK(daegun_font_tracking(font, 12.0, DAEGUN_HORIZONTAL, &tracking) == DAEGUN_OK && tracking == 0.0,
          "tracking %g with no trak table", tracking);

    daegun_os2 os2 = { 0 };
    memset(&os2, 0xff, sizeof os2);
    CHECK(daegun_font_os2(font, NULL, &os2) == DAEGUN_OK, "os2 failed");
    CHECK(os2.version == 4 && os2.has_selection && os2.selection == 192, "OS/2 version %u, selection %u", os2.version,
          os2.selection);
    CHECK(os2.has_family_class && os2.family_class == 0, "family class %u", os2.family_class);
    CHECK(os2.has_weight_class && os2.weight_class == 400 && os2.has_width_class && os2.width_class == 5
              && os2.has_fs_type && os2.fs_type == 0,
          "weight class %u, width class %u, fsType %u", os2.weight_class, os2.width_class, os2.fs_type);
    CHECK(os2.has_win_metrics && os2.win_ascent == 1984.0 && os2.win_descent == 494.0, "win metrics %g and %g",
          os2.win_ascent, os2.win_descent);
    CHECK(os2.has_typo_metrics && os2.typo_ascender == 1984.0 && os2.typo_descender == -494.0
              && os2.typo_line_gap == 0.0,
          "typo metrics %g, %g and %g", os2.typo_ascender, os2.typo_descender, os2.typo_line_gap);
    bool bold = true, regular = false, oblique = true, italic = true, typo = false;
    CHECK(daegun_os2_is_bold(&os2, &bold) == DAEGUN_OK && !bold, "Inter's default is bold");
    CHECK(daegun_os2_is_regular(&os2, &regular) == DAEGUN_OK && regular, "Inter's default is not regular");
    CHECK(daegun_os2_is_oblique(&os2, &oblique) == DAEGUN_OK && !oblique, "Inter's default is oblique");
    CHECK(daegun_os2_is_italic(&os2, &italic) == DAEGUN_OK && !italic, "Inter's default is italic");
    CHECK(daegun_os2_uses_typo_metrics(&os2, &typo) == DAEGUN_OK && typo, "Inter ignores its typo metrics");
    CHECK(daegun_os2_is_bold(NULL, &bold) == DAEGUN_NULL, "the boldness of NULL");

    daegun_location *at_bold = location_at(font, WGHT, 700.0);
    CHECK(daegun_font_os2(font, at_bold, &os2) == DAEGUN_OK && os2.selection == 160 && os2.weight_class == 700
              && daegun_os2_is_bold(&os2, &bold) == DAEGUN_OK && bold,
          "Inter at wght 700 selects %u, weighing %u", os2.selection, os2.weight_class);
    daegun_text *subfamily = NULL, *at_default = NULL;
    CHECK(daegun_font_name(font, 2, at_bold, &subfamily) == DAEGUN_OK && text_is(subfamily, "Bold")
              && daegun_font_name(font, 2, NULL, &at_default) == DAEGUN_OK && text_is(at_default, "Regular"),
          "name 2 is not Bold at wght 700 and Regular at the default");
    daegun_text_free(subfamily);
    daegun_text_free(at_default);
    daegun_location_free(at_bold);

    daegun_typographic_metrics tm = { 0 }, heavy = { 0 };
    CHECK(daegun_font_typographic_metrics(font, NULL, &tm) == DAEGUN_OK, "typographic_metrics failed");
    CHECK(tm.has_x_height && tm.has_underline && tm.has_strikeout && tm.has_subscript && tm.has_superscript,
          "Inter leaves out a typographic metric");
    CHECK(tm.x_height == 1118 && tm.underline_position == -348 && tm.underline_thickness == 140
              && tm.strikeout_thickness == 140 && tm.strikeout_position == 671,
          "x-height %g, underline %g and %g, strikeout %g and %g", tm.x_height, tm.underline_position,
          tm.underline_thickness, tm.strikeout_thickness, tm.strikeout_position);
    CHECK(tm.subscript_x_size == 1331 && tm.subscript_y_size == 1229 && tm.subscript_x_offset == 0
              && tm.subscript_y_offset == 154 && tm.superscript_x_size == 1331 && tm.superscript_y_size == 1229
              && tm.superscript_x_offset == 0 && tm.superscript_y_offset == 717,
          "subscript %g and %g, superscript %g and %g", tm.subscript_x_size, tm.subscript_y_offset,
          tm.superscript_x_size, tm.superscript_y_offset);
    /* MVAR moves the underline and strikeout with the weight. */
    daegun_location *black = location_at(font, WGHT, 900.0);
    CHECK(daegun_font_typographic_metrics(font, black, &heavy) == DAEGUN_OK && heavy.underline_position == -302
              && heavy.underline_thickness == 232 && heavy.strikeout_thickness == 232 && heavy.x_height == 1118,
          "at 900 the underline is at %g, %g thick", heavy.underline_position, heavy.underline_thickness);
    size_t nn = 0;
    const double *norm = daegun_location_normalized(black, &nn);
    CHECK(norm && nn == 2 && norm[0] == 0.0 && norm[1] == 1.0, "wght 900 normalized to %zu coordinates", nn);
    daegun_location_free(black);

    /* Clamped to the axis, opsz at its default left out, and an axis Inter lacks ignored. */
    const daegun_coord asked[3] = { { WGHT, 2000.0 }, { DAEGUN_TAG('o', 'p', 's', 'z'), 14.0 },
                                    { DAEGUN_TAG('X', 'X', 'X', 'X'), 1.0 } };
    daegun_location *past = NULL, *plain = NULL;
    CHECK(daegun_font_location(font, asked, 3, &past) == DAEGUN_OK, "a location failed");
    const daegun_coord *resolved = daegun_location_coords(past, &nn);
    CHECK(resolved && nn == 1 && resolved[0].tag == WGHT && resolved[0].value == 900.0,
          "wght 2000 resolved to %zu coordinates", nn);
    CHECK(daegun_font_location(font, NULL, 0, &plain) == DAEGUN_OK && daegun_location_coords(plain, &nn) && nn == 0
              && daegun_location_normalized(plain, &nn) && nn == 0,
          "the default location holds %zu coordinates", nn);
    CHECK(daegun_location_coords(NULL, &nn) == NULL && daegun_location_coords(past, NULL) == NULL,
          "coordinates of NULL, or into a NULL count");
    daegun_location_free(past);
    daegun_location_free(plain);

    daegun_named_instances *named = NULL;
    size_t count = 0, cn = 0;
    daegun_str name = { NULL, 0 }, ps = { NULL, 0 };
    CHECK(daegun_font_named_instances(font, &named) == DAEGUN_OK
              && daegun_named_instances_count(named, &count) == DAEGUN_OK && count == 9,
          "%zu named instances, not 9", count);
    CHECK(daegun_named_instances_name(named, 0, &name) == DAEGUN_OK && str_is(name, "Thin")
              && daegun_named_instances_postscript_name(named, 0, &ps) == DAEGUN_OK
              && str_is(ps, "InterVariable-Thin"),
          "instance 0 is not Thin");
    const daegun_coord *coords = daegun_named_instances_coords(named, 0, &cn);
    CHECK(coords && cn == 2 && coords[0].tag == DAEGUN_TAG('o', 'p', 's', 'z') && coords[0].value == 14.0
              && coords[1].tag == WGHT && coords[1].value == 100.0,
          "Thin is not at opsz 14, wght 100");
    CHECK(daegun_named_instances_name(named, 9, &name) == DAEGUN_INVALID_ARGUMENT, "a tenth instance was named");
    CHECK(daegun_named_instances_coords(named, 9, &cn) == NULL, "a tenth instance has coordinates");
    daegun_named_instances_free(named);

    uint32_t f = 0, i = 0;
    CHECK(daegun_font_glyph_id(font, 'f', &f) == DAEGUN_OK && daegun_font_glyph_id(font, 'i', &i) == DAEGUN_OK,
          "Inter lacks f or i");
    const uint32_t fi[2] = { f, i };
    uint32_t cd[32];
    size_t cl = 0;
    CHECK(daegun_font_glyph_closure(font, fi, 2, NULL, cd, 32, &cl) == DAEGUN_OK, "glyph_closure failed");
    int kept = 0;
    for (size_t k = 0; k < cl; k++) kept += cd[k] == f || cd[k] == i;
    CHECK(cl == 15 && kept == 2, "f and i close over %zu glyphs, keeping %d of them", cl, kept);

    daegun_tag scripts[8], langs[8], features[64], rf[64];
    const daegun_tag want_scripts[4] = { DAEGUN_TAG('D', 'F', 'L', 'T'), DAEGUN_TAG('c', 'y', 'r', 'l'),
                                         DAEGUN_TAG('g', 'r', 'e', 'k'), DAEGUN_TAG('l', 'a', 't', 'n') };
    const daegun_tag want_langs[3] = { DAEGUN_TAG('C', 'A', 'T', ' '), DAEGUN_TAG('M', 'O', 'L', ' '),
                                       DAEGUN_TAG('R', 'O', 'M', ' ') };
    size_t ns = 0, nl = 0, nf = 0, nr = 0;
    CHECK(daegun_font_script_tags(font, scripts, 8, &ns) == DAEGUN_OK && tags_are(scripts, ns, want_scripts, 4),
          "the scripts are not DFLT, cyrl, grek and latn");
    CHECK(daegun_font_language_system_tags(font, DAEGUN_TAG('l', 'a', 't', 'n'), langs, 8, &nl) == DAEGUN_OK
              && tags_are(langs, nl, want_langs, 3),
          "latn's languages are not CAT, MOL and ROM");
    CHECK(daegun_font_feature_tags(font, 0, 0, features, 64, &nf) == DAEGUN_OK && nf == 42, "%zu features, not 42",
          nf);
    CHECK(daegun_font_feature_tags(font, DAEGUN_TAG('l', 'a', 't', 'n'), DAEGUN_TAG('R', 'O', 'M', ' '), rf, 64, &nr)
              == DAEGUN_OK
              && nr == 43,
          "ROM has %zu features, not DFLT's 42 and locl", nr);
    int locl = 0;
    for (size_t k = 0; k < nr; k++) {
        locl += rf[k] == DAEGUN_TAG('l', 'o', 'c', 'l');
        if (k > 0) CHECK(rf[k - 1] < rf[k], "ROM's features are not sorted and unique at %zu", k);
    }
    CHECK(locl == 1, "ROM lists locl %d times", locl);
    daegun_font_free(font);
}

/* How Inter maps text to glyphs and what its runs report, with ids as its cmap holds them. */
static void glyphs_and_runs_answer(void)
{
    daegun_font *font = open_font(INTER);
    if (!font) return;
    bool has = false;
    CHECK(daegun_font_has_glyph(font, 'A', &has) == DAEGUN_OK && has, "Inter has no A");
    CHECK(daegun_font_has_glyph(font, 0x10FFFF, &has) == DAEGUN_OK && !has, "Inter maps U+10FFFF");

    uint32_t ids[6];
    uint8_t present[6];
    size_t n = 0;
    CHECK(daegun_font_glyph_ids(font, TXT("A\xF4\x8F\xBF\xBF" "b"), ids, present, 6, &n) == DAEGUN_OK && n == 3
              && ids[0] == 2 && ids[1] == 0 && ids[2] == 578 && present[0] == 1 && present[1] == 0 && present[2] == 1,
          "glyph_ids gave %zu ids", n);
    CHECK(daegun_font_glyph_ids(font, TXT("Ab"), ids, NULL, 1, &n) == DAEGUN_INVALID_ARGUMENT && n == 2,
          "room for one id was not refused with the two needed");

    size_t ncp = 0;
    CHECK(daegun_font_coverage(font, NULL, NULL, 0, &ncp) == DAEGUN_OK && ncp == 2852, "%zu covered, not 2852", ncp);
    uint32_t *cp = malloc(ncp * sizeof *cp), *cg = malloc(ncp * sizeof *cg);
    CHECK(cp && cg && daegun_font_coverage(font, cp, cg, ncp, &ncp) == DAEGUN_OK, "coverage failed");
    int a_maps = 0;
    for (size_t k = 0; cp && cg && k < ncp; k++) a_maps |= cp[k] == 'A' && cg[k] == 2;
    CHECK(a_maps, "coverage maps no A to 2");
    CHECK(cg && daegun_font_coverage(font, NULL, cg, ncp, &ncp) == DAEGUN_OK, "coverage refused a NULL half");
    free(cp);
    free(cg);

    daegun_bounds box = { 0, 0, 0, 0 };
    CHECK(daegun_font_glyph_bounds(font, 2, NULL, &box) == DAEGUN_OK && box.x_min == 52.0 && box.y_min == 0.0
              && box.x_max == 1361.0 && box.y_max == 1490.0,
          "A's box is %g, %g, %g, %g", box.x_min, box.y_min, box.x_max, box.y_max);
    CHECK(daegun_font_glyph_bounds(font, 1777, NULL, &box) == DAEGUN_ABSENT, "a space has ink");
    uint32_t uvs = 0;
    CHECK(daegun_font_variation_glyph_id(font, 'A', 0xFE00, &uvs) == DAEGUN_ABSENT, "Inter maps A with VS1");

    /* No vmtx or VORG: a glyph advances down by the line, 1984 + 494, and A's box is centered on it. */
    double vadv = 0, vorig = 0, dvorig = 1;
    CHECK(daegun_font_advance(font, 2, NULL, DAEGUN_VERTICAL, &vadv) == DAEGUN_OK && vadv == 2478.0,
          "A's vertical advance is %g, not 2478", vadv);
    CHECK(daegun_font_vertical_origin(font, 2, NULL, &vorig) == DAEGUN_OK && vorig == 1984.0,
          "A's vertical origin is %g, not 1490 + (2478 - 1490) / 2", vorig);
    CHECK(daegun_font_vertical_origin(font, 65535, NULL, &vorig) == DAEGUN_OK && vorig == 1984.0,
          "glyph 65535, past the count, has vertical origin %g, not the ascender", vorig);
    CHECK(daegun_font_default_vertical_origin(font, &dvorig) == DAEGUN_OK && dvorig == 0.0,
          "the default vertical origin is %g", dvorig);
    const uint32_t ab[2] = { 507, 578 };
    double advances[2] = { 0, 0 };
    CHECK(daegun_font_advances(font, ab, 2, NULL, DAEGUN_HORIZONTAL, advances) == DAEGUN_OK && advances[0] == 1150.0
              && advances[1] == 1254.0,
          "a and b advance %g and %g", advances[0], advances[1]);
    double carets[3] = { -1, -1, -1 };
    CHECK(daegun_font_caret_positions(font, TXT("ab"), NULL, DAEGUN_HORIZONTAL, carets, 3, &n) == DAEGUN_OK && n == 3
              && carets[0] == 0.0 && carets[1] == 1150.0 && carets[2] == 2404.0,
          "ab has %zu caret positions", n);

    int32_t klass = -1;
    uint32_t mark_class = 9;
    CHECK(daegun_font_glyph_class(font, 2, &klass) == DAEGUN_OK && klass == DAEGUN_GLYPH_CLASS_BASE, "A is class %d",
          klass);
    CHECK(daegun_font_glyph_class(font, 1770, &klass) == DAEGUN_OK && klass == DAEGUN_GLYPH_CLASS_MARK,
          "the combining acute is class %d", klass);
    CHECK(daegun_font_mark_attachment_class(font, 1770, &mark_class) == DAEGUN_ABSENT && mark_class == 9,
          "the acute has mark attachment class %u", mark_class);
    char gname[DAEGUN_MAX_GLYPH_NAME + 1];
    size_t gname_n = 0;
    CHECK(daegun_font_glyph_name(font, 2, gname, sizeof gname, &gname_n) == DAEGUN_OK && gname_n == 2
              && strcmp(gname, "A") == 0,
          "glyph 2 is not A");
    CHECK(daegun_font_glyph_name(font, 2, gname, 1, &gname_n) == DAEGUN_INVALID_ARGUMENT && gname_n == 2,
          "a name was written into a byte");
    daegun_str_list *names = NULL;
    CHECK(daegun_font_glyph_names(font, &names) == DAEGUN_OK, "glyph_names failed");
    size_t nnames = 0;
    daegun_str n2 = { NULL, 0 };
    CHECK(daegun_str_list_count(names, &nnames) == DAEGUN_OK && nnames == 2937
              && daegun_str_list_at(names, 2, &n2) == DAEGUN_OK && str_is(n2, "A"),
          "%zu names, without A at 2", nnames);
    daegun_str_list_free(names);

    /* Romanian's locl turns s-cedilla into s-comma. */
    daegun_shape_options opts = { 0 };
    CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
    opts.language = "ro";
    daegun_shaped_run *ro = NULL, *plain = NULL;
    size_t ng = 0;
    CHECK(daegun_font_shape(font, TXT("\xC5\x9F"), NULL, &opts, &ro) == DAEGUN_OK
              && daegun_font_shape(font, TXT("\xC5\x9F"), NULL, NULL, &plain) == DAEGUN_OK,
          "shaping s-cedilla failed");
    const uint32_t *rg = daegun_shaped_run_glyphs(ro, &ng);
    CHECK(ng == 1 && rg[0] == 878, "s-cedilla in Romanian is glyph %u, not 878", ng ? rg[0] : 0);
    const uint32_t *pg = daegun_shaped_run_glyphs(plain, &ng);
    CHECK(ng == 1 && pg[0] == 876, "s-cedilla is glyph %u, not 876", ng ? pg[0] : 0);
    daegun_shaped_run_free(ro);
    daegun_shaped_run_free(plain);

    opts.language = NULL;
    opts.report_unsafe_to_concat = true;
    opts.report_tatweel_positions = true;
    daegun_shaped_run *reported = NULL;
    CHECK(daegun_font_shape(font, TXT("Hi"), NULL, &opts, &reported) == DAEGUN_OK, "shaping with reports failed");
    size_t nb = 0, nc = 0, nt = 0;
    const uint8_t *utb = daegun_shaped_run_unsafe_to_break(reported, &nb);
    const uint8_t *utc = daegun_shaped_run_unsafe_to_concat(reported, &nc);
    CHECK(daegun_shaped_run_safe_to_insert_tatweel(reported, &nt) != NULL || nt == 0, "tatweel flags came back NULL");
    CHECK(nb == 2 && !utb[0] && !utb[1], "Hi is unsafe to break: %zu flags", nb);
    CHECK(nc == 2 && utc[0] && utc[1] && nt == 2, "%zu concat and %zu tatweel flags, asked for", nc, nt);
    daegun_shaped_run_free(reported);

    /* Hi is 1522 + 496 wide; at its own width it keeps its natural shaping. */
    daegun_justify_options jo = { 0 };
    CHECK(daegun_justify_options_default(&jo, sizeof jo) == DAEGUN_OK, "justify defaults failed");
    jo.script = DAEGUN_TAG('l', 'a', 't', 'n');
    jo.target_width = 2018.0;
    jo.tolerance = 0.5;
    daegun_shaped_run *jrun = NULL;
    daegun_justification info = { 0 };
    memset(&info, 0xff, sizeof info);
    CHECK(daegun_font_justify(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, &jo, &jrun, &info) == DAEGUN_OK,
          "justify at the natural width failed");
    CHECK(!info.has_level && info.adjustment == DAEGUN_ADJUST_EXTEND && !info.best_effort && info.width == 2018.0,
          "Hi at its own width came back %g wide", info.width);
    CHECK(daegun_shaped_run_glyphs(jrun, &ng) && ng == 2, "the justified run has %zu glyphs", ng);
    daegun_shaped_run_free(jrun);
    jo.target_width = 4096.0;
    jo.tolerance = 1.0;
    jrun = NULL;
    CHECK(daegun_font_justify(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, &jo, &jrun, &info) == DAEGUN_OK
              && !info.has_level && info.adjustment == DAEGUN_ADJUST_EXTEND && info.best_effort,
          "with no JSTF, a stretch to 4096 was not a best effort");
    daegun_shaped_run_free(jrun);
    jo.target_width = 1000.0;
    jrun = NULL;
    CHECK(daegun_font_justify(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, &jo, &jrun, &info) == DAEGUN_OK
              && info.adjustment == DAEGUN_ADJUST_SHRINK && info.best_effort && info.width == 2018.0,
          "Hi squeezed to 1000 came back %g wide, adjustment %d", info.width, (int)info.adjustment);
    daegun_shaped_run_free(jrun);
    CHECK(daegun_font_justify(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, &jo, &jrun, NULL) == DAEGUN_OK,
          "a NULL info was refused");
    daegun_shaped_run_free(jrun);
    daegun_font_free(font);

    daegun_font *with_carets = open_font("assets/test-fonts/test-fixtures/carets.ttf");
    if (with_carets) {
        double c[4];
        uint8_t p[4];
        size_t nl = 0;
        CHECK(daegun_font_ligature_carets(with_carets, 6, NULL, DAEGUN_HORIZONTAL, c, p, 4, &nl) == DAEGUN_OK,
              "ligature_carets failed");
        CHECK(nl == 3 && c[0] == 250.0 && c[1] == 500.0 && c[2] == 750.0 && p[0] && p[1] && p[2],
              "f_f_l has %zu carets", nl);
        CHECK(daegun_font_ligature_carets(with_carets, 5, NULL, DAEGUN_VERTICAL, c, NULL, 4, &nl) == DAEGUN_OK,
              "ligature_carets failed");
        CHECK(nl == 1 && c[0] == -850.0, "f_i's caret, point 1 at (420, 0), is not 850 below its vertical origin");
        CHECK(daegun_font_ligature_carets(with_carets, 1, NULL, DAEGUN_HORIZONTAL, c, NULL, 4, &nl) == DAEGUN_OK
                  && nl == 0,
              "f, no ligature, has %zu carets", nl);
        daegun_font_free(with_carets);
    }

    daegun_font *deva = open_font("assets/test-fonts/noto-devanagari/NotoSansDevanagari.ttf");
    if (deva) {
        daegun_shaped_run *run = NULL;
        bool broken = false;
        CHECK(daegun_font_shape(deva, TXT("\xE0\xA4\xBF"), NULL, NULL, &run) == DAEGUN_OK
                  && daegun_shaped_run_has_broken_syllable(run, &broken) == DAEGUN_OK && broken,
              "a vowel sign with no consonant is not a broken syllable");
        daegun_shaped_run_free(run);
        daegun_font_free(deva);
    }
}

/* STIX's MATH, every number a value the table stores. */
static void math_glyph_info_answers(void)
{
    daegun_font *font = open_font(STIX);
    if (!font) return;
    uint32_t paren = 0, f = 0;
    CHECK(daegun_font_glyph_id(font, '(', &paren) == DAEGUN_OK && daegun_font_glyph_id(font, 0x1D453, &f) == DAEGUN_OK,
          "STIX lacks ( or the italic f");
    double v = -1.0;
    bool extended = false;
    CHECK(daegun_font_math_italics_correction(font, f, NULL, &v) == DAEGUN_OK && v == 20.0,
          "the italic f's correction is %g", v);
    CHECK(daegun_font_math_italics_correction(font, paren, NULL, &v) == DAEGUN_ABSENT, "( has an italic correction");
    CHECK(daegun_font_math_top_accent_attachment(font, f, NULL, &v) == DAEGUN_OK && v == 470.0,
          "its accent sits at %g", v);
    CHECK(daegun_font_math_is_extended_shape(font, paren, &extended) == DAEGUN_OK && extended, "( is not extended");
    CHECK(daegun_font_math_min_connector_overlap(font, &v) == DAEGUN_OK && v == 100.0, "the connector overlap is %g",
          v);
    CHECK(daegun_font_math_kern(font, 7, NULL, DAEGUN_MATH_KERN_TOP_RIGHT, 250.0, &v) == DAEGUN_OK && v == 32.0,
          "glyph 7's top right kern at 250 is %g", v);
    CHECK(daegun_font_math_kern(font, 22, NULL, DAEGUN_MATH_KERN_BOTTOM_LEFT, 250.0, &v) == DAEGUN_OK && v == -90.0,
          "glyph 22's bottom left kern at 250 is %g", v);
    CHECK(daegun_font_math_kern(font, 7, NULL, 4, 250.0, &v) == DAEGUN_INVALID_ARGUMENT, "corner 4 was taken");
    CHECK(daegun_font_math_kern(font, 7, NULL, DAEGUN_MATH_KERN_TOP_RIGHT, NAN, &v) == DAEGUN_INVALID_ARGUMENT,
          "a NaN height was taken");

    daegun_math_construction *c = NULL;
    CHECK(daegun_font_math_glyph_variants(font, paren, NULL, DAEGUN_VERTICAL, &c) == DAEGUN_OK,
          "( has no vertical variants");
    size_t nv = 0;
    const daegun_math_variant *vs = daegun_math_construction_variants(c, &nv);
    CHECK(vs && nv == 13 && vs[0].glyph == paren && vs[0].advance == 933.0 && vs[12].glyph == 1312
              && vs[12].advance == 3821.0,
          "( has %zu variants", nv);
    daegun_glyph_assembly as = { 0 };
    CHECK(daegun_math_construction_assembly(c, &as) == DAEGUN_OK && as.italics_correction == 0.0 && as.parts_len == 3,
          "( assembles from %zu parts", as.parts_len);
    if (as.parts_len == 3) {
        const daegun_glyph_part *p = as.parts;
        CHECK(p[0].glyph == 4862 && p[0].start_connector_length == 0 && p[0].end_connector_length == 250
                  && p[0].full_advance == 1273 && !p[0].extender && p[1].glyph == 4861
                  && p[1].start_connector_length == 1000 && p[1].end_connector_length == 1000
                  && p[1].full_advance == 1252 && p[1].extender && p[2].glyph == 4860
                  && p[2].start_connector_length == 250 && p[2].end_connector_length == 0 && p[2].full_advance == 1273
                  && !p[2].extender,
              "the parts of ( are not 4862, 4861 and 4860 as the table stores them");
    }
    daegun_math_construction_free(c);
    c = NULL;
    CHECK(daegun_font_math_glyph_variants(font, paren, NULL, DAEGUN_HORIZONTAL, &c) == DAEGUN_ABSENT && c == NULL,
          "( has horizontal variants");
    CHECK(daegun_font_math_glyph_variants(font, paren, NULL, 2, &c) == DAEGUN_INVALID_ARGUMENT,
          "orientation 2 was taken");
    daegun_font_free(font);

    daegun_font *plain = open_font(INTER);
    if (plain) {
        daegun_math_constants constants = { 0 };
        CHECK(daegun_font_math_constants(plain, NULL, &constants) == DAEGUN_ABSENT, "Inter has MATH constants");
        CHECK(daegun_font_math_min_connector_overlap(plain, &v) == DAEGUN_ABSENT, "Inter has a connector overlap");
        daegun_font_free(plain);
    }

    /* The fixture's values move with half each delta at wght 650. */
    daegun_font *varied = open_font("assets/test-fonts/test-fixtures/math-var.ttf");
    if (varied) {
        daegun_location *at = location_at(varied, WGHT, 650.0);
        daegun_math_constants constants = { 0 };
        CHECK(daegun_font_math_constants(varied, at, &constants) == DAEGUN_OK && constants.math_leading == 120.0
                  && constants.axis_height == 265.0,
              "MathLeading %g and AxisHeight %g at wght 650", constants.math_leading, constants.axis_height);
        uint32_t x = 0, left = 0;
        CHECK(daegun_font_glyph_id(varied, 'x', &x) == DAEGUN_OK
                  && daegun_font_glyph_id(varied, '(', &left) == DAEGUN_OK,
              "the fixture lacks x or (");
        CHECK(daegun_font_math_italics_correction(varied, x, at, &v) == DAEGUN_OK && v == 20.0,
              "x's correction is %g", v);
        CHECK(daegun_font_math_top_accent_attachment(varied, x, at, &v) == DAEGUN_OK && v == 135.0,
              "x's accent is %g", v);
        CHECK(daegun_font_math_kern(varied, x, at, DAEGUN_MATH_KERN_TOP_RIGHT, 549.0, &v) == DAEGUN_OK && v == 55.0,
              "x's kern below 550 is %g", v);
        CHECK(daegun_font_math_kern(varied, x, at, DAEGUN_MATH_KERN_TOP_RIGHT, 550.0, &v) == DAEGUN_OK && v == 0.0,
              "x's kern from 550 is %g", v);
        daegun_math_construction *built = NULL;
        daegun_glyph_assembly assembly = { 0 };
        CHECK(daegun_font_math_glyph_variants(varied, left, at, DAEGUN_VERTICAL, &built) == DAEGUN_OK
                  && daegun_math_construction_assembly(built, &assembly) == DAEGUN_OK
                  && assembly.italics_correction == 48.0,
              "('s assembly's correction is %g", assembly.italics_correction);
        daegun_math_construction_free(built);
        daegun_location_free(at);
        daegun_font_free(varied);
    }
}

/* A layout's features reach its runs, as shaping with them does: Inter's tnum widens its 1s. NULL with a length is
 * refused. */
static void layout_features_answer(void)
{
    daegun_font *font = open_font(INTER);
    if (!font) return;
    daegun_feature tnum = { DAEGUN_TAG('t', 'n', 'u', 'm'), 1, 0, 0 };
    daegun_shape_options shaping = { 0 };
    daegun_layout_options opts = { 0 };
    CHECK(daegun_shape_options_default(&shaping, sizeof shaping) == DAEGUN_OK
              && daegun_layout_options_default(&opts, sizeof opts) == DAEGUN_OK,
          "option defaults failed");
    shaping.features = &tnum;
    shaping.features_len = 1;
    opts.features = &tnum;
    opts.features_len = 1;
    daegun_shaped_run *plain = NULL, *tabular = NULL;
    daegun_layout *laid = NULL;
    daegun_layout_info info = { 0 };
    double plain_width = 0, tabular_width = 0;
    CHECK(daegun_font_shape(font, TXT("111"), NULL, NULL, &plain) == DAEGUN_OK
              && daegun_font_shape(font, TXT("111"), NULL, &shaping, &tabular) == DAEGUN_OK
              && daegun_shaped_run_width(plain, &plain_width) == DAEGUN_OK
              && daegun_shaped_run_width(tabular, &tabular_width) == DAEGUN_OK && plain_width != tabular_width,
          "tnum moved nothing: %g and %g", plain_width, tabular_width);
    CHECK(daegun_font_layout(font, TXT("111"), NULL, &opts, &laid) == DAEGUN_OK
              && daegun_layout_info_of(laid, &info) == DAEGUN_OK && info.inline_size == tabular_width,
          "a layout with tnum is %g wide, its run %g", info.inline_size, tabular_width);
    daegun_layout_free(laid);
    laid = NULL;
    opts.features = NULL;
    CHECK(daegun_font_layout(font, TXT("111"), NULL, &opts, &laid) == DAEGUN_NULL && laid == NULL,
          "NULL features with a length were taken");
    daegun_shaped_run_free(plain);
    daegun_shaped_run_free(tabular);
    daegun_font_free(font);
}

/* The table's stored values: pairs first, last and deep in their lists, and two misses. */
static void variation_sequences_answer(void)
{
    daegun_font *font = open_font("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    if (!font) return;
    static const struct {
        uint32_t base, selector, want;
    } found[4] = {
        { 0x4FAE, 0xFE00, 15200 }, { 0x2A61A, 0xE0101, 17423 }, { 0x9089, 0xE010E, 17243 }, { 0x2E6EA, 0xE0100, 16144 },
    };
    for (size_t i = 0; i < 4; i++) {
        uint32_t gid = 0;
        CHECK(daegun_font_variation_glyph_id(font, found[i].base, found[i].selector, &gid) == DAEGUN_OK
                  && gid == found[i].want,
              "U+%04X U+%04X gave %u, not %u", found[i].base, found[i].selector, gid, found[i].want);
    }
    uint32_t gid = 0;
    CHECK(daegun_font_variation_glyph_id(font, 0x4FAE, 0xE0104, &gid) == DAEGUN_ABSENT, "VS21 resolved U+4FAE");
    CHECK(daegun_font_variation_glyph_id(font, 0x4FAE, 0xFE0F, &gid) == DAEGUN_ABSENT, "VS16 resolved U+4FAE");
    CHECK(daegun_font_variation_glyph_id(font, 0x4FAE, 0xD800, &gid) == DAEGUN_INVALID_ARGUMENT,
          "a surrogate selector was taken");
    daegun_font_free(font);
}

/* The table's records: the family name in English and in Japanese, sorted by language. */
static void names_read_in_each_language(void)
{
    daegun_font *font = open_font("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    if (!font) return;
    daegun_localized_names *names = NULL;
    size_t n = 0;
    CHECK(daegun_font_localized_names(font, 1, NULL, &names) == DAEGUN_OK
              && daegun_localized_names_count(names, &n) == DAEGUN_OK && n == 2,
          "%zu localized family names, not 2", n);
    daegun_str language = { NULL, 0 }, name = { NULL, 0 };
    CHECK(daegun_localized_names_at(names, 0, &language, &name) == DAEGUN_OK && strcmp(language.data, "en") == 0
              && strcmp(name.data, "Source Han Sans JP VF") == 0,
          "the first is %s, %s", language.data, name.data);
    CHECK(daegun_localized_names_at(names, 1, &language, &name) == DAEGUN_OK && strcmp(language.data, "ja") == 0
              && strcmp(name.data, "\xE6\xBA\x90\xE3\x83\x8E\xE8\xA7\x92\xE3\x82\xB4\xE3\x82\xB7\xE3\x83\x83"
                                   "\xE3\x82\xAF JP VF") == 0,
          "the second is %s, %s", language.data, name.data);
    daegun_str kept = { NULL, 0 };
    CHECK(daegun_localized_names_at(names, 2, &language, &kept) == DAEGUN_INVALID_ARGUMENT && kept.data == NULL,
          "reading past the end was not DAEGUN_INVALID_ARGUMENT, or wrote its out");
    daegun_localized_names_free(names);
    CHECK(daegun_font_localized_names(font, 300, NULL, &names) == DAEGUN_OK
              && daegun_localized_names_count(names, &n) == DAEGUN_OK && n == 0,
          "%zu names for an ID with no record", n);
    daegun_localized_names_free(names);
    daegun_font_free(font);
}

/* The table's stored values, and icfb and icft as BASE's store moves them at wght 900. */
static void base_and_vertical_metrics_answer(void)
{
    daegun_font *font = open_font("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    if (!font) return;
    bool glyph_free = false;
    CHECK(daegun_font_has_glyph_free_base(font, &glyph_free) == DAEGUN_OK && glyph_free, "BASE needs glyphs");
    const daegun_tag tags[4] = { DAEGUN_TAG('i', 'c', 'f', 'b'), DAEGUN_TAG('i', 'c', 'f', 't'),
                                 DAEGUN_TAG('i', 'd', 'e', 'o'), DAEGUN_TAG('r', 'o', 'm', 'n') };
    static const double want[2][2][4] = {
        { { -67, 827, -120, 0 }, { -94, 854, -120, 0 } },
        { { 53, 947, 0, 120 }, { 26, 974, 0, 120 } },
    };
    const daegun_tag hani = DAEGUN_TAG('h', 'a', 'n', 'i');
    daegun_location *heavy = location_at(font, WGHT, 900.0);
    for (int32_t orientation = 0; orientation < 2; orientation++) {
        for (int at = 0; at < 2; at++) {
            daegun_base_script *script = NULL;
            daegun_tag def = 0;
            size_t n = 0;
            CHECK(daegun_font_base_script(font, hani, at ? heavy : NULL, orientation, &script) == DAEGUN_OK,
                  "base_script failed");
            const daegun_baseline *b = daegun_base_script_baselines(script, &n);
            bool same = daegun_base_script_default_baseline(script, &def) == DAEGUN_OK
                        && def == DAEGUN_TAG('i', 'd', 'e', 'o') && b && n == 4;
            for (size_t k = 0; same && k < 4; k++) {
                same = b[k].tag == tags[k] && b[k].position == want[orientation][at][k];
            }
            CHECK(same, "hani's %s baselines at %s are not where the table puts them",
                  orientation ? "vertical" : "horizontal", at ? "wght 900" : "the default");
            daegun_base_script_free(script);
        }
    }
    daegun_location_free(heavy);
    daegun_extents extents = { 0 };
    CHECK(daegun_font_base_extents(font, hani, NULL, DAEGUN_HORIZONTAL, 0, 0, &extents) == DAEGUN_ABSENT,
          "Source Han's BASE has no MinMax, yet base_extents answered");

    uint32_t kan = 0;
    double vadv = 0, vorig = 0, dvorig = 0;
    CHECK(daegun_font_glyph_id(font, 0x6F22, &kan) == DAEGUN_OK, "Source Han lacks U+6F22");
    CHECK(daegun_font_advance(font, kan, NULL, DAEGUN_VERTICAL, &vadv) == DAEGUN_OK && vadv == 1000.0,
          "U+6F22's vertical advance is %g", vadv);
    CHECK(daegun_font_vertical_origin(font, kan, NULL, &vorig) == DAEGUN_OK && vorig == 880.0,
          "U+6F22's vertical origin is %g", vorig);
    CHECK(daegun_font_default_vertical_origin(font, &dvorig) == DAEGUN_OK && dvorig == 880.0, "VORG's default is %g",
          dvorig);
    daegun_font_free(font);

    daegun_font *inter = open_font(INTER);
    if (inter) {
        daegun_base_script *none = NULL;
        CHECK(daegun_font_base_script(inter, hani, NULL, DAEGUN_HORIZONTAL, &none) == DAEGUN_ABSENT && none == NULL,
              "Inter, with no BASE, has baselines");
        CHECK(daegun_font_has_glyph_free_base(inter, &glyph_free) == DAEGUN_OK && !glyph_free,
              "Inter, with no BASE, has a glyph-free one");
        daegun_font_free(inter);
    }
}

/* A word segmenter that finds one boundary, after the run's first character, and counts its calls. */
static size_t cut_after_first(void *user, const char *text, size_t len, size_t *out, size_t cap)
{
    ++*(int *)user;
    if (cap == 0) return 0;
    size_t first = 1;
    while (first < len && ((unsigned char)text[first] & 0xC0) == 0x80) ++first;
    out[0] = first;
    return 1;
}

static size_t cut_inside_first(void *user, const char *text, size_t len, size_t *out, size_t cap)
{
    (void)text;
    (void)len;
    ++*(int *)user;
    if (cap == 0) return 0;
    out[0] = 1;
    return 1;
}

static void bidi_and_line_analysis_answer(void)
{
    static const char mixed[] = "abc \xD7\x90\xD7\x91\xD7\x92";
    daegun_font *font = open_font(INTER);
    if (font) {
        daegun_bidi_runs *runs = NULL;
        const daegun_shaped_run *run = NULL;
        size_t n = 0, start = 9, end = 9, ng = 0;
        uint8_t level = 9;
        CHECK(daegun_font_shape_bidi(font, TXT(mixed), NULL, DAEGUN_BASE_AUTO, NULL, &runs) == DAEGUN_OK
                  && daegun_bidi_runs_count(runs, &n) == DAEGUN_OK && n == 2,
              "abc and Hebrew shaped to %zu runs", n);
        CHECK(daegun_bidi_runs_at(runs, 0, &run, &level, &start, &end) == DAEGUN_OK && level == 0 && start == 0
                  && end == 4,
              "the first run is at level %u over %zu to %zu", level, start, end);
        const uint32_t *g = daegun_shaped_run_glyphs(run, &ng);
        CHECK(ng == 4 && g[0] == 507 && g[3] == 1777, "abc and the space shaped to %zu glyphs", ng);
        CHECK(daegun_bidi_runs_at(runs, 1, NULL, &level, &start, &end) == DAEGUN_OK && level == 1 && start == 4
                  && end == 10,
              "the Hebrew run is at level %u over %zu to %zu", level, start, end);
        CHECK(daegun_bidi_runs_at(runs, 2, NULL, NULL, NULL, NULL) == DAEGUN_INVALID_ARGUMENT, "a third run was found");
        /* A copy of a borrowed run outlives the set, and shaping into it leaves the run it was copied from alone. */
        daegun_shaped_run *kept = NULL, *again = NULL;
        CHECK(daegun_shaped_run_copy(run, &kept) == DAEGUN_OK && kept, "a borrowed run did not copy");
        CHECK(daegun_shaped_run_copy(kept, &again) == DAEGUN_OK && again, "a copy did not copy");
        daegun_bidi_runs_free(runs);
        g = daegun_shaped_run_glyphs(kept, &ng);
        CHECK(ng == 4 && g[0] == 507 && g[3] == 1777, "the copy kept %zu glyphs past the set", ng);
        CHECK(daegun_font_shape_into(font, TXT("ffl"), NULL, NULL, &again) == DAEGUN_OK, "shaping into a copy failed");
        g = daegun_shaped_run_glyphs(kept, &ng);
        CHECK(ng == 4 && g[0] == 507, "shaping into one copy changed the other to %zu glyphs", ng);
        daegun_shaped_run_free(again);
        daegun_shaped_run_free(kept);
        CHECK(daegun_shaped_run_copy(NULL, &kept) == DAEGUN_NULL && kept == NULL, "a NULL run copied");

        /* Right to left, the space takes the paragraph's level and abc sits a level above it, last. */
        runs = NULL;
        CHECK(daegun_font_shape_bidi(font, TXT(mixed), NULL, DAEGUN_BASE_RTL, NULL, &runs) == DAEGUN_OK
                  && daegun_bidi_runs_count(runs, &n) == DAEGUN_OK && n == 3,
              "a right-to-left paragraph shaped to %zu runs", n);
        CHECK(daegun_bidi_runs_at(runs, 1, NULL, &level, &start, &end) == DAEGUN_OK && level == 1 && start == 3
                  && end == 4,
              "the space, Latin at the Hebrew's level, is not the second run");
        CHECK(daegun_bidi_runs_at(runs, 2, NULL, &level, &start, &end) == DAEGUN_OK && level == 2 && start == 0
                  && end == 3,
              "in a right-to-left paragraph abc is not the third run, at level 2");
        daegun_bidi_runs_free(runs);

        runs = NULL;
        CHECK(daegun_font_shape_bidi(font, TXT("Ab \xCE\x91\xCE\xB2 \xD0\x92\xD0\xB2"), NULL, DAEGUN_BASE_AUTO, NULL,
                                     &runs) == DAEGUN_OK
                  && daegun_bidi_runs_count(runs, &n) == DAEGUN_OK && n == 3
                  && daegun_bidi_runs_at(runs, 1, NULL, &level, &start, &end) == DAEGUN_OK && level == 0
                  && start == 3 && end == 8,
              "Latin, Greek and Cyrillic at one level are not three runs");
        daegun_bidi_runs_free(runs);
        CHECK(daegun_font_shape_bidi(font, TXT(mixed), NULL, 2, NULL, &runs) == DAEGUN_INVALID_ARGUMENT,
              "base direction 2 was taken");
        daegun_font_free(font);
    }

    daegun_bidi_paragraph *p = NULL;
    uint8_t base = 9;
    size_t n = 0;
    CHECK(daegun_bidi_paragraph_new(TXT(mixed), DAEGUN_BASE_AUTO, &p) == DAEGUN_OK
              && daegun_bidi_paragraph_base_level(p, &base) == DAEGUN_OK && base == 0,
          "the paragraph's base level is %u", base);
    const size_t *order = daegun_bidi_paragraph_visual_order(p, &n);
    CHECK(order && n == 7 && order[3] == 3 && order[4] == 8 && order[6] == 4, "the Hebrew is not reversed in order");
    daegun_visual_run vr[8];
    CHECK(daegun_bidi_paragraph_visual_runs(p, TXT(mixed), 0, 10, vr, 8, &n) == DAEGUN_OK && n == 2 && vr[0].level == 0
              && vr[1].start == 4 && vr[1].end == 10 && vr[1].level == 1,
          "the line has %zu visual runs", n);
    CHECK(daegun_bidi_paragraph_visual_runs(p, TXT(mixed), 6, 2, vr, 8, &n) == DAEGUN_OK && n == 0,
          "a reversed range has runs");
    CHECK(daegun_bidi_paragraph_visual_runs(p, TXT(mixed), 0, 5, vr, 8, &n) == DAEGUN_OK && n == 0,
          "a range ending inside a character has runs");
    daegun_bidi_paragraph_free(p);
    p = NULL;
    CHECK(daegun_bidi_paragraph_new(TXT(mixed), DAEGUN_BASE_RTL, &p) == DAEGUN_OK
              && daegun_bidi_paragraph_base_level(p, &base) == DAEGUN_OK && base == 1,
          "a right-to-left paragraph has base level %u", base);
    daegun_bidi_paragraph_free(p);
    p = NULL;
    CHECK(daegun_bidi_paragraph_new(TXT("\xD7\x90\nab"), DAEGUN_BASE_AUTO, &p) == DAEGUN_OK,
          "the bidi paragraph failed");
    const daegun_paragraph_run *paras = daegun_bidi_paragraph_paragraphs(p, &n);
    CHECK(paras && n == 2 && paras[0].start == 0 && paras[0].end == 3 && paras[0].level == 1 && paras[1].start == 3
              && paras[1].end == 5 && paras[1].level == 0,
          "a right-to-left paragraph and a left-to-right one gave %zu", n);
    daegun_bidi_paragraph_free(p);

    size_t words[16];
    CHECK(daegun_word_boundaries(TXT("hi there"), words, 16, &n) == DAEGUN_OK && n == 4 && words[0] == 0
              && words[1] == 2 && words[2] == 3 && words[3] == 8,
          "hi there has %zu word boundaries", n);
    daegun_line_break breaks[16];
    CHECK(daegun_line_break_opportunities(TXT("hi there\nyo"), NULL, NULL, breaks, 16, &n) == DAEGUN_OK && n == 3
              && breaks[0].at == 3 && !breaks[0].mandatory && breaks[1].at == 9 && breaks[1].mandatory
              && breaks[2].at == 11 && breaks[2].mandatory,
          "the line breaks are not 3, then 9 and 11 demanded");

    /* The Thai run of "\u0E01\u0E02\u0E03 ab" goes to the segmenter alone. */
    int calls = 0;
    static const char thai[] = "\xE0\xB8\x81\xE0\xB8\x82\xE0\xB8\x83 ab";
    CHECK(daegun_line_break_opportunities(TXT(thai), cut_after_first, &calls, breaks, 16, &n) == DAEGUN_OK && n == 3
              && breaks[0].at == 3 && breaks[1].at == 10 && breaks[2].at == 12 && calls == 1,
          "%zu breaks, the first at %zu, with the segmenter asked %d times", n, n ? breaks[0].at : 0, calls);
    CHECK(daegun_line_break_opportunities(TXT(thai), cut_inside_first, &calls, breaks, 16, &n) == DAEGUN_OK && n == 2
              && breaks[0].at == 10 && breaks[1].at == 12 && calls == 2,
          "a boundary inside a character was taken: %zu breaks, the first at %zu", n, n ? breaks[0].at : 0);
    CHECK(daegun_line_break_opportunities(TXT(thai), NULL, NULL, breaks, 16, &n) == DAEGUN_OK && n == 2
              && breaks[0].at == 10 && breaks[1].at == 12,
          "%zu breaks without a segmenter", n);

    /* "\u0E2A\u0E27\u0E31\u0E2A\u0E14\u0E35\u0E04\u0E23\u0E31\u0E1A", two words, splits only with the word lists. */
    CHECK(daegun_line_break_opportunities(TXT("\xE0\xB8\xAA\xE0\xB8\xA7\xE0\xB8\xB1\xE0\xB8\xAA\xE0\xB8\x94"
                                              "\xE0\xB8\xB5\xE0\xB8\x84\xE0\xB8\xA3\xE0\xB8\xB1\xE0\xB8\x9A"),
                                          NULL, NULL, breaks, 16, &n) == DAEGUN_OK,
          "line breaks in Thai failed");
    bool split = n == 2 && breaks[0].at == 18 && breaks[1].at == 30;
    CHECK(split == daegun_has_dictionaries() && (split || (n == 1 && breaks[0].at == 30)),
          "%zu breaks in two Thai words, with daegun_has_dictionaries %d", n, daegun_has_dictionaries());

    bool vertical = false;
    CHECK(daegun_writing_mode_is_vertical(DAEGUN_WRITING_VERTICAL_RL, &vertical) == DAEGUN_OK && vertical
              && daegun_writing_mode_is_vertical(DAEGUN_WRITING_VERTICAL_LR, &vertical) == DAEGUN_OK && vertical
              && daegun_writing_mode_is_vertical(DAEGUN_WRITING_HORIZONTAL, &vertical) == DAEGUN_OK && !vertical,
          "the writing modes disagree on which are vertical");
    CHECK(daegun_writing_mode_is_vertical(3, &vertical) == DAEGUN_INVALID_ARGUMENT, "writing mode 3 was taken");
}

static void layout_wraps_and_borrows(const char *path)
{
    daegun_font *font = open_font(path);
    if (!font) return;
    daegun_layout_options opts = { 0 };
    CHECK(daegun_layout_options_default(&opts, sizeof opts) == DAEGUN_OK, "layout defaults failed");

    /* Thai text breaks where the segmenter says, before the cut three characters in. */
    daegun_shaped_run *three = NULL;
    size_t k = 0;
    const double *adv = NULL;
    CHECK(daegun_font_shape(font, TXT("\xE0\xB8\x81\xE0\xB8\x82\xE0\xB8\x84"), NULL, NULL, &three) == DAEGUN_OK
              && (adv = daegun_shaped_run_advances(three, &k)) != NULL && k == 3,
          "shaping three Thai letters failed");
    if (k == 3) {
        int calls = 0;
        daegun_layout_options thai = opts;
        thai.max_inline_size = adv[0] + adv[1] + adv[2] + 1.0;
        thai.segmenter = cut_after_first;
        thai.segmenter_user = &calls;
        daegun_layout *cut = NULL;
        daegun_layout_info info = { 0 };
        daegun_layout_line second = { 0 };
        CHECK(daegun_font_layout(font, TXT("\xE0\xB8\x81\xE0\xB8\x82\xE0\xB8\x84\xE0\xB8\x87"), NULL, &thai, &cut)
                      == DAEGUN_OK
                  && daegun_layout_info_of(cut, &info) == DAEGUN_OK
                  && daegun_layout_line_at(cut, 1, &second) == DAEGUN_OK,
              "laying out Thai with a segmenter failed");
        CHECK(info.line_count == 2 && second.start == 3 && calls == 1,
              "%zu lines, the second from %zu, with the segmenter asked %d times", info.line_count, second.start,
              calls);
        daegun_layout_free(cut);
    }
    daegun_shaped_run_free(three);

    /* A tab shapes as the space, 576 units wide, and ends on the first stop, eight spaces from the line's start. */
    daegun_layout *tabbed = NULL;
    daegun_layout_run lr = { 0 };
    size_t n = 0, m = 0;
    CHECK(daegun_font_layout(font, TXT("a\tb"), NULL, &opts, &tabbed) == DAEGUN_OK
              && daegun_layout_run_at(tabbed, 0, 0, &lr) == DAEGUN_OK,
          "laying out a tab failed");
    const uint32_t *glyphs = daegun_shaped_run_glyphs(lr.run, &n);
    const double *advances = daegun_shaped_run_advances(lr.run, &m);
    CHECK(n == 3 && m == 3 && glyphs[1] == 1777 && advances[0] + advances[1] == 8.0 * 576.0,
          "the tab is glyph %u, ending at %g, not the space ending on 8 x 576", n == 3 ? glyphs[1] : 0,
          m == 3 ? advances[0] + advances[1] : 0.0);
    daegun_layout_free(tabbed);

    static const char fox[] = "the quick brown fox jumps over the lazy dog";
    daegun_layout *layout = NULL, *one = NULL;
    daegun_layout_info info = { 0 }, wide_info = { 0 };
    opts.max_inline_size = 12288.0;
    CHECK(daegun_font_layout(font, TXT(fox), NULL, &opts, &layout) == DAEGUN_OK
              && daegun_layout_info_of(layout, &info) == DAEGUN_OK && info.line_count > 1 && info.inline_size > 0.0
              && info.inline_size <= 12288.0 && !info.has_truncated,
          "a 43-character string at 6 em wrapped to %zu line(s)", info.line_count);
    opts.max_inline_size = INFINITY;
    CHECK(daegun_font_layout(font, TXT(fox), NULL, &opts, &one) == DAEGUN_OK
              && daegun_layout_info_of(one, &wide_info) == DAEGUN_OK && wide_info.line_count == 1,
          "an unbounded measure wrapped to %zu lines", wide_info.line_count);
    daegun_layout_free(one);

    daegun_layout_line line = { 0 };
    CHECK(daegun_layout_line_at(layout, 0, &line) == DAEGUN_OK && line.run_count > 0 && line.start == 0
              && line.ascent == 1984.0 && line.descent == -494.0 && !line.hard_break,
          "the first line holds %zu runs", line.run_count);
    CHECK(daegun_layout_run_at(layout, 0, 0, &lr) == DAEGUN_OK && daegun_shaped_run_glyphs(lr.run, &n) && n > 0,
          "the first run of the first line holds no glyphs");
    /* Deliberately not freed: the run belongs to the layout, and freeing it is the double free ASan catches. A copy
     * is a run of the caller's own, which outlives the layout. */
    daegun_shaped_run *kept = NULL;
    CHECK(daegun_shaped_run_copy(lr.run, &kept) == DAEGUN_OK, "a layout's run does not copy");
    CHECK(daegun_layout_line_at(layout, info.line_count, &line) == DAEGUN_INVALID_ARGUMENT, "a line past the end");
    CHECK(daegun_layout_run_at(layout, 0, line.run_count + 99, &lr) == DAEGUN_INVALID_ARGUMENT, "a run past the end");
    daegun_layout_free(layout);
    CHECK(daegun_shaped_run_glyphs(kept, &m) && m == n, "a run copied from a layout lost its glyphs with the layout");
    daegun_shaped_run_free(kept);

    opts.max_inline_size = 12288.0;
    opts.has_max_lines = true;
    opts.max_lines = 1;
    CHECK(daegun_font_layout(font, TXT(fox), NULL, &opts, &layout) == DAEGUN_OK
              && daegun_layout_info_of(layout, &info) == DAEGUN_OK && info.line_count == 1 && info.has_truncated
              && info.truncated > 0 && info.truncated < 43,
          "one line of the fox was not cut, at %zu", info.truncated);
    daegun_layout_free(layout);
    daegun_font_free(font);
}

/* Scheherazade's JSTF names extenders and no priorities, so a priority level is added to Inter by hand: one
 * level whose extension enables GSUB lookup 0. */
static void justification_answers(void)
{
    daegun_font *arabic = open_font("assets/test-fonts/scheherazade-new/ScheherazadeNew-Regular.ttf");
    const daegun_tag arab = DAEGUN_TAG('a', 'r', 'a', 'b');
    if (arabic) {
        uint32_t g[4];
        size_t ng = 0;
        CHECK(daegun_font_justification_glyphs(arabic, arab, g, 4, &ng) == DAEGUN_OK, "justification_glyphs failed");
        CHECK(ng == 2 && g[0] == 1313 && g[1] == 1315, "arab's extenders are not 1313 and 1315");
        daegun_jstf_priorities *none = NULL;
        CHECK(daegun_font_justification_priorities(arabic, arab, 0, &none) == DAEGUN_ABSENT && none == NULL,
              "priorities came back from a JSTF that names none");

        daegun_shape_options opts = { 0 };
        CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
        opts.report_tatweel_positions = true;
        daegun_shaped_run *run = NULL;
        size_t nt = 0;
        CHECK(daegun_font_shape(arabic, TXT("\xD8\xA8\xD8\xA8"), NULL, &opts, &run) == DAEGUN_OK,
              "shaping two behs failed");
        const uint8_t *t = daegun_shaped_run_safe_to_insert_tatweel(run, &nt);
        CHECK(nt == 2 && t[0] && !t[1], "a tatweel fits between two behs at %zu places", nt);
        daegun_shaped_run_free(run);

        /* "(" alone is Common and shapes as default; seeded Arabic, as between Arabic words, it mirrors. */
        CHECK(daegun_shape_options_default(&opts, sizeof opts) == DAEGUN_OK, "shape defaults failed");
        opts.seed_script = DAEGUN_TAG('A', 'r', 'a', 'b');
        daegun_shaped_run *seeded = NULL, *plain = NULL;
        size_t ns = 0, np = 0;
        int32_t shaper = -1;
        CHECK(daegun_font_shape(arabic, TXT("("), NULL, &opts, &seeded) == DAEGUN_OK
                  && daegun_font_shape(arabic, TXT("("), NULL, NULL, &plain) == DAEGUN_OK,
              "shaping ( failed");
        const uint32_t *sg = daegun_shaped_run_glyphs(seeded, &ns), *pg = daegun_shaped_run_glyphs(plain, &np);
        CHECK(ns == 1 && np == 1 && sg[0] == 10 && pg[0] == 9, "( seeded Arabic is not the mirrored glyph");
        CHECK(daegun_shaped_run_shaper(seeded, &shaper) == DAEGUN_OK && shaper == DAEGUN_SHAPER_ARABIC,
              "( seeded Arabic did not reach the Arabic shaper");
        daegun_shaped_run_free(seeded);
        daegun_shaped_run_free(plain);
        daegun_font_free(arabic);
    }

    static const uint8_t jstf[46] = {
        0, 1, 0, 0, 0, 1, 'l', 'a', 't', 'n', 0, 12,
        0, 0, 0, 6, 0, 0,
        0, 1, 0, 4,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1, 0, 0,
    };
    daegun_font *font = open_font_with(INTER, DAEGUN_TAG('J', 'S', 'T', 'F'), jstf, sizeof jstf);
    if (!font) return;
    daegun_jstf_priorities *levels = NULL;
    size_t count = 0;
    CHECK(daegun_font_justification_priorities(font, DAEGUN_TAG('l', 'a', 't', 'n'), 0, &levels) == DAEGUN_OK
              && daegun_jstf_priorities_count(levels, &count) == DAEGUN_OK && count == 1,
          "the JSTF table's %zu priority levels are not one", count);
    for (int32_t which = 0; which < 8; which++) {
        const uint16_t *lookups = NULL;
        size_t nl = 0;
        daegun_status st = daegun_jstf_priorities_lookups(levels, 0, which, &lookups, &nl);
        if (which == DAEGUN_JSTF_EXTENSION_ENABLE_GSUB) {
            CHECK(st == DAEGUN_OK && nl == 1 && lookups[0] == 0, "the extension does not enable GSUB lookup 0");
        } else {
            CHECK(st == DAEGUN_ABSENT, "list %d came back as %d from a level that names none", which, st);
        }
    }
    const uint16_t *lookups = NULL;
    size_t nl = 0;
    CHECK(daegun_jstf_priorities_lookups(levels, 0, 8, &lookups, &nl) == DAEGUN_INVALID_ARGUMENT, "a ninth list");
    CHECK(daegun_jstf_priorities_lookups(levels, 1, 0, &lookups, &nl) == DAEGUN_INVALID_ARGUMENT, "a second level");
    for (int32_t adjust = DAEGUN_ADJUST_SHRINK; adjust <= DAEGUN_ADJUST_EXTEND; adjust++) {
        daegun_shaped_run *run = NULL;
        size_t ng = 0;
        CHECK(daegun_font_shape_justified(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, levels, 0, adjust, &run)
                      == DAEGUN_OK
                  && daegun_shaped_run_glyphs(run, &ng) != NULL && ng == 2,
              "shape_justified adjusting %d gave %zu glyphs", adjust, ng);
        daegun_shaped_run_free(run);
    }
    daegun_shaped_run *run = NULL;
    CHECK(daegun_font_shape_justified(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, levels, 1, DAEGUN_ADJUST_EXTEND, &run)
              == DAEGUN_INVALID_ARGUMENT,
          "a level past the last was applied");
    CHECK(daegun_font_shape_justified(font, TXT("Hi"), NULL, DAEGUN_HORIZONTAL, levels, 0, 2, &run)
              == DAEGUN_INVALID_ARGUMENT,
          "adjustment 2 was taken");
    daegun_jstf_priorities_free(levels);
    daegun_font_free(font);
}

/* Tables too small for a fixture, built here: two coverage tables, a format 6 lookup, a morx state machine, an
 * ankr and a FeatureVariations record. The index map is Inter's own, against the Rust reader's answers. */
static void the_walkers_read_tables_built_by_hand(void)
{
    /* Glyphs 3, 7 and 9; then glyphs 10 to 20 from index 65,530, so glyph 20's is past 65,535. */
    static const uint8_t listed[10] = { 0, 1, 0, 3, 0, 3, 0, 7, 0, 9 };
    static const uint8_t ranged[10] = { 0, 2, 0, 1, 0, 10, 0, 20, 0xFF, 0xFA };
    uint32_t at = 0;
    CHECK(daegun_coverage_index(listed, sizeof listed, 7, &at) == DAEGUN_OK && at == 1, "glyph 7 is at %u", at);
    CHECK(daegun_coverage_index(listed, sizeof listed, 8, &at) == DAEGUN_ABSENT, "glyph 8 is covered");
    CHECK(daegun_coverage_index(ranged, sizeof ranged, 20, &at) == DAEGUN_OK && at == 65540, "glyph 20 is at %u", at);
    CHECK(daegun_coverage_index(ranged, sizeof ranged, 21, &at) == DAEGUN_ABSENT, "glyph 21 is covered");
    uint32_t cg[4];
    size_t nc = 0;
    CHECK(daegun_coverage_glyphs(listed, sizeof listed, 0, cg, 4, &nc) == DAEGUN_OK && nc == 3 && cg[0] == 3
              && cg[2] == 9,
          "the listed coverage holds %zu glyphs", nc);

    static const uint8_t single[20] = { 0, 6, 0, 4, 0, 2, 0, 8, 0, 1, 0, 0, 0, 3, 0, 30, 0, 7, 0, 70 };
    daegun_aat_lookup *lookup = NULL;
    uint16_t v = 0;
    CHECK(daegun_aat_lookup_parse(single, sizeof single, 10, &lookup) == DAEGUN_OK, "the lookup did not parse");
    CHECK(daegun_aat_lookup_value(lookup, 7, &v) == DAEGUN_OK && v == 70, "glyph 7 maps to %u", v);
    CHECK(daegun_aat_lookup_value(lookup, 5, &v) == DAEGUN_ABSENT, "glyph 5 maps to something");
    daegun_glyph_value e[4];
    size_t ne = 0;
    CHECK(daegun_aat_lookup_entries(lookup, e, 4, &ne) == DAEGUN_OK, "lookup entries failed");
    CHECK(ne == 2 && e[0].glyph == 3 && e[0].value == 30 && e[1].glyph == 7 && e[1].value == 70,
          "the lookup lists %zu mappings", ne);
    daegun_aat_lookup_free(lookup);

    /* Five classes over three glyphs. From the start state, class 4 moves to state 1 with flags 0x8000 and its
     * one extra word, 0x1234. */
    static const uint8_t machine[56] = {
        0, 0, 0, 5, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0, 44,
        0, 0, 0, 4, 0, 4, 0, 3,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
        0, 1, 0x80, 0, 0x12, 0x34,
    };
    daegun_aat_state_table *table = NULL;
    uint16_t klass = 0;
    daegun_aat_entry cell = { 0 };
    memset(&cell, 0xff, sizeof cell);
    CHECK(daegun_aat_state_table_parse(machine, sizeof machine, 1, 3, &table) == DAEGUN_OK,
          "the state table did not parse");
    CHECK(daegun_aat_state_table_class(table, 1, &klass) == DAEGUN_OK && klass == 4, "glyph 1 is class %u", klass);
    CHECK(daegun_aat_state_table_class(table, 3, &klass) == DAEGUN_OK && klass == 1,
          "a glyph past the font is class %u", klass);
    CHECK(daegun_aat_state_table_class(table, 0xFFFF, &klass) == DAEGUN_OK && klass == DAEGUN_AAT_CLASS_DELETED_GLYPH,
          "the deleted glyph is class %u", klass);
    CHECK(daegun_aat_state_table_entry(table, DAEGUN_AAT_STATE_START_OF_TEXT, 4, &cell) == DAEGUN_OK
              && cell.new_state == 1 && cell.flags == 0x8000 && cell.word1 == 0x1234 && cell.word2 == 0,
          "the cell reads state %u, flags %x, words %x and %x", cell.new_state, cell.flags, cell.word1, cell.word2);
    CHECK(daegun_aat_state_table_entry(table, 0, 5, &cell) == DAEGUN_ABSENT, "a sixth class was read");
    CHECK(daegun_aat_state_table_entry(table, 9, 0, &cell) == DAEGUN_ABSENT, "a tenth state was read");
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
    uint16_t version = 9;
    int16_t x = 0, y = 0;
    CHECK(daegun_ankr_version(anchors, sizeof anchors, &version) == DAEGUN_OK && version == 0, "ankr version %u",
          version);
    CHECK(daegun_ankr_control_point(anchors, sizeof anchors, 24, &x, &y) == DAEGUN_OK && x == 10 && y == 20,
          "the point at 24 is (%d, %d)", x, y);
    CHECK(daegun_ankr_parse(anchors, sizeof anchors, 3, &ankr) == DAEGUN_OK, "the ankr did not parse");
    CHECK(daegun_ankr_point_count(ankr, 1, &points) == DAEGUN_OK && points == 2, "glyph 1 has %u anchors", points);
    CHECK(daegun_ankr_point_count(ankr, 0, &points) == DAEGUN_OK && points == 0, "glyph 0 has %u anchors", points);
    CHECK(daegun_ankr_anchor_point(ankr, 1, 1, &x, &y) == DAEGUN_OK && x == -30 && y == 40,
          "glyph 1's second anchor is (%d, %d)", x, y);
    CHECK(daegun_ankr_anchor_point(ankr, 1, 2, &x, &y) == DAEGUN_ABSENT, "glyph 1 has a third anchor");
    daegun_ankr_free(ankr);

    /* One record: axis 0 from 0.5 to 1.0, 8192 to 16384 in F2Dot14, swaps feature 3 for the feature table at
     * byte 42. */
    static const uint8_t variations[46] = {
        0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0, 30,
        0, 1, 0, 0, 0, 6,
        0, 1, 0, 0, 0x20, 0, 0x40, 0,
        0, 1, 0, 0, 0, 1, 0, 3, 0, 0, 0, 12,
        0, 0, 0, 0,
    };
    daegun_feature_variations *fv = NULL;
    uint32_t record = 9;
    size_t alternate = 0;
    const int32_t inside[1] = { 12000 }, outside[1] = { 4000 };
    CHECK(daegun_feature_variations_at(variations, sizeof variations, 0, &fv) == DAEGUN_OK,
          "feature_variations_at failed");
    CHECK(daegun_feature_variations_find(fv, inside, 1, NULL, &record) == DAEGUN_OK && record == 0,
          "0.73 matched record %u", record);
    CHECK(daegun_feature_variations_find(fv, outside, 1, NULL, &record) == DAEGUN_ABSENT, "0.24 matched a record");
    CHECK(daegun_feature_variations_substitute(fv, 0, 3, &alternate) == DAEGUN_OK && alternate == 42,
          "feature 3's alternate is at %zu", alternate);
    CHECK(daegun_feature_variations_substitute(fv, 0, 2, &alternate) == DAEGUN_ABSENT, "feature 2 has an alternate");
    daegun_feature_variations_free(fv);

    /* The same record behind a GSUB 1.1 header, found from its offset there: the alternate moves with it. */
    uint8_t gsub[14 + sizeof variations] = { 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 14 };
    memcpy(gsub + 14, variations, sizeof variations);
    fv = NULL;
    CHECK(daegun_feature_variations_parse(gsub, sizeof gsub, &fv) == DAEGUN_OK, "GSUB's variations did not parse");
    CHECK(daegun_feature_variations_find(fv, inside, 1, NULL, &record) == DAEGUN_OK && record == 0,
          "0.73 matched record %u in GSUB", record);
    CHECK(daegun_feature_variations_substitute(fv, 0, 3, &alternate) == DAEGUN_OK && alternate == 56,
          "feature 3's alternate is at %zu in GSUB", alternate);
    daegun_feature_variations_free(fv);

    daegun_font *font = open_font(INTER);
    if (!font) return;
    daegun_bytes hvar = { NULL, 0 };
    uint32_t map_at = 0;
    daegun_delta_set_index_map *map = NULL;
    size_t count = 0, outer = 0, inner = 0;
    CHECK(daegun_font_table(font, DAEGUN_TAG('H', 'V', 'A', 'R'), &hvar) == DAEGUN_OK
              && daegun_read_u32_be(hvar.data, hvar.len, 8, &map_at) == DAEGUN_OK
              && daegun_delta_set_index_map_parse(hvar.data, hvar.len, map_at, &map) == DAEGUN_OK,
          "HVAR's advance map did not parse");
    CHECK(daegun_delta_set_index_map_count(map, &count) == DAEGUN_OK && count == 2937, "the map has %zu entries",
          count);
    CHECK(daegun_delta_set_index_map_lookup(map, 2, &outer, &inner) == DAEGUN_OK && outer == 3 && inner == 46,
          "A maps to (%zu, %zu)", outer, inner);
    CHECK(daegun_delta_set_index_map_lookup(map, 5000, &outer, &inner) == DAEGUN_OK && outer == 1 && inner == 302,
          "past the end maps to (%zu, %zu), not the last entry", outer, inner);
    daegun_delta_set_index_map_free(map);
    daegun_font_free(font);
}

/* The conformance glyphs' clip boxes, as the Rust tests read them, and one varied, each delta rounded. */
static void a_clip_box_bounds_its_glyph(void)
{
    daegun_font *glyphs = open_font(COLR);
    daegun_font *variable = open_font("assets/test-fonts/colr-v1-test-glyphs/test_glyphs_variable.ttf");
    if (glyphs && variable) {
        daegun_bounds box = { 9, 9, 9, 9 }, none = { 9, 9, 9, 9 };
        CHECK(daegun_font_colr_clip_box(glyphs, 156, NULL, &box) == DAEGUN_OK && box.x_min == 0 && box.y_min == 500
                  && box.x_max == 500 && box.y_max == 1000,
              "glyph 156's clip box is %g, %g, %g, %g", box.x_min, box.y_min, box.x_max, box.y_max);
        CHECK(daegun_font_colr_clip_box(glyphs, 1, NULL, &none) == DAEGUN_ABSENT && none.x_min == 9 && none.y_max == 9,
              "a glyph with no clip box answered or wrote one");
        /* 4095/16384 of each axis leaves every edge 0.03 past a whole unit: rounded outward, as the COLR spec asks,
         * each grows by 1. */
        const double q = 4095.0 / 16384.0 * 500.0;
        const daegun_coord at[5] = {
            { DAEGUN_TAG('C', 'L', 'X', 'I'), q }, { DAEGUN_TAG('C', 'L', 'Y', 'I'), q },
            { DAEGUN_TAG('C', 'L', 'X', 'A'), -q }, { DAEGUN_TAG('C', 'L', 'Y', 'A'), -q },
            { DAEGUN_TAG('C', 'L', 'I', 'O'), q },
        };
        daegun_location *loc = NULL;
        CHECK(daegun_font_location(variable, at, 5, &loc) == DAEGUN_OK
                  && daegun_font_colr_clip_box(variable, 160, loc, &box) == DAEGUN_OK && box.x_min == 374
                  && box.y_min == 374 && box.x_max == 626 && box.y_max == 626,
              "glyph 160's varied clip box is %g, %g, %g, %g", box.x_min, box.y_min, box.x_max, box.y_max);
        daegun_location_free(loc);
        CHECK(daegun_font_colr_clip_box(glyphs, 156, NULL, NULL) == DAEGUN_NULL, "a NULL box was written");
    }
    daegun_font_free(glyphs);
    daegun_font_free(variable);
}

/* A CPAL version 1 whose first entry is labeled by name 300 and second by none, and one labeling nothing. */
static void palette_entries_have_their_labels(const char *path)
{
    static const uint8_t cpal[38] = {
        0, 1, 0, 2, 0, 1, 0, 2, 0, 0, 0, 26, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 34,
        0, 0, 255, 255, 255, 0, 0, 255,
        1, 44, 255, 255,
    };
    daegun_font *labeled = open_font_with(path, DAEGUN_TAG('C', 'P', 'A', 'L'), cpal, sizeof cpal);
    daegun_font *unlabeled = open_font(COLR);
    if (labeled && unlabeled) {
        uint16_t id[4];
        uint8_t p[4];
        size_t ni = 0;
        CHECK(daegun_font_palette_entry_labels(labeled, id, p, 4, &ni) == DAEGUN_OK, "entry labels failed");
        CHECK(ni == 2 && id[0] == 300 && id[1] == 0 && p[0] == 1 && p[1] == 0, "the labels are not 300 and none");
        CHECK(daegun_font_palette_entry_labels(unlabeled, id, p, 4, &ni) == DAEGUN_OK && ni == 0,
              "a palette with no entry labels gave %zu", ni);
        CHECK(daegun_font_palette_entry_labels(labeled, NULL, NULL, 0, &ni) == DAEGUN_OK && ni == 2,
              "the count alone was refused, or isn't 2");

        daegun_palette pal[4];
        size_t n = 0;
        CHECK(daegun_font_palettes(labeled, pal, 4, &n) == DAEGUN_OK && n == 1 && pal[0].index == 0
                  && !pal[0].has_name_id && !pal[0].light_safe && !pal[0].dark_safe,
              "the hand-built CPAL has %zu palettes", n);
    }
    daegun_font_free(labeled);
    daegun_font_free(unlabeled);
}

/* Runs after main returns, when the thread's storage may already be gone: a reason is read and set there, and
 * neither may abort the process. */
static void last_error_at_exit(void)
{
    (void)daegun_last_error();
    daegun_font *junk = NULL;
    static const uint8_t bytes[16] = { 0xAB };
    (void)daegun_font_open(bytes, sizeof bytes, &junk);
    (void)daegun_last_error();
}

int main(int argc, char **argv)
{
    atexit(last_error_at_exit);
    const char *font = argc > 1 ? argv[1] : INTER;

    printf("daegun C round trip\n");
    abi_version_agrees();
    null_is_refused_not_dereferenced();
    bad_font_data_is_reported();
    a_real_font_answers(font);
    shape_options_reach_the_shaper(font);
    options_defaults_are_versioned();
    a_font_file_is_one_face(font);
    metrics_answer_and_free_cleanly(font);
    math_constants_answer();
    the_pen_draws_and_reentry_is_safe(font);
    reentry_at_a_location_is_safe(font, 'B');
    reentry_at_a_location_is_safe("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf", 0x6C38);
    shaping_produces_positioned_glyphs(font);
    raw_tables_round_trip(font);
    a_location_that_cannot_be_instanced_says_why(font);
    the_readers_are_bounds_checked();
    paths_build_stroke_and_replay();
    glyph_quads_come_back_in_font_units(font);
    text_analysis_needs_no_font();
    the_paint_graph_is_walkable();
    a_color_scene_is_walkable();
    a_padded_gradient_holds_its_end_colors();
    compositing_follows_colr();
    a_path_can_draw_into_itself();
    paths_flatten_and_resolve();
    a_hinted_glyph_is_readable_from_c();
    the_remaining_readers_answer();
    a_stroke_that_overflows_is_refused();
    null_with_a_count_is_null(font);
    strings_are_utf8(font);
    a_zeroed_miter_limit_is_the_default();
    a_scene_holds_each_outline_once_within_a_bound();
    a_prepared_stroke_past_the_cap_draws();
    a_prepared_outline_reaches_the_pen(font);
    the_subpixel_filter_is_readable();
    the_rasterizer_lends_its_images(font);
    character_properties_need_no_font();
    the_format_walkers_read_a_real_table(font);
    loca_and_glyf_are_walkable(font);
    the_atlas_packer_packs();
    the_rules_a_caller_would_get_wrong(font);
    scripts_answer_about_themselves();
    subsetting_maps_glyph_ids(font);
    stat_values_are_readable(font);
    an_owned_buffer_opens_without_a_copy(font);
    a_face_describes_itself();
    glyphs_and_runs_answer();
    math_glyph_info_answers();
    variation_sequences_answer();
    layout_features_answer();
    names_read_in_each_language();
    base_and_vertical_metrics_answer();
    bidi_and_line_analysis_answer();
    layout_wraps_and_borrows(font);
    justification_answers();
    the_walkers_read_tables_built_by_hand();
    a_clip_box_bounds_its_glyph();
    palette_entries_have_their_labels(font);

    if (failures == 0) {
        printf("  ok\n");
        return 0;
    }
    printf("  %d failed\n", failures);
    return 1;
}
