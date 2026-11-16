/* libFuzzer over every C call: an input is choices, then a font. An options struct lies in a buffer of its exact
 * size, so ASan sees a read past it, and one of a size this library doesn't know must never succeed. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "daegun.h"

/* Each stage reads its own, so no stage runs on the zeros another left it. run.sh seeds this many. */
#define STAGE 512
#define STAGES 9
#define CHOICES (STAGE * STAGES)

typedef struct {
    const uint8_t *at;
    size_t left;
} choices;

static uint8_t byte(choices *c)
{
    if (!c->left) {
        return 0;
    }
    c->left--;
    return *c->at++;
}

static uint32_t word(choices *c)
{
    uint32_t w = 0;
    for (int i = 0; i < 4; i++) {
        w = w << 8 | byte(c);
    }
    return w;
}

/* A number as its bits, so NaN, the infinities and the huge come up as often as the plain. */
static double number(choices *c)
{
    uint64_t bits = (uint64_t)word(c) << 32 | word(c);
    double d;
    memcpy(&d, &bits, sizeof d);
    return d;
}

static float number32(choices *c)
{
    uint32_t bits = word(c);
    float f;
    memcpy(&f, &bits, sizeof f);
    return f;
}

static int32_t code(choices *c) { return (int32_t)(byte(c) % 8) - 1; }

/* The usual value seven times in eight, as a call refused at its options reaches nothing behind them. */
static int32_t usual(choices *c, int32_t value) { return byte(c) & 7 ? value : code(c); }
static uint32_t usual_word(choices *c, uint32_t value) { return byte(c) & 7 ? value : word(c); }
static double usual_number(choices *c, double value) { return byte(c) & 7 ? value : number(c); }
static float usual_number32(choices *c, float value) { return byte(c) & 7 ? value : number32(c); }

static void fail(void) { abort(); }

/* `filled` in a buffer of its own size three times in four, else one the choices pick, the bytes past it what a
 * newer header's fields might hold. */
static void *sized(choices *c, const void *filled, size_t known)
{
    size_t size = byte(c) & 3 ? known : sizeof(size_t) + byte(c) % (known + 17 - sizeof(size_t));
    uint8_t *out = malloc(size);
    if (!out) {
        fail();
    }
    memcpy(out, filled, size < known ? size : known);
    for (size_t i = known; i < size; i++) {
        out[i] = byte(c);
    }
    memcpy(out, &size, sizeof size);
    return out;
}

/* Whether a call given options of a size this library doesn't know answered as it must: never OK. */
static void held(const void *opts, size_t known, daegun_status status)
{
    size_t size;
    memcpy(&size, opts, sizeof size);
    if (size != known && status == DAEGUN_OK) {
        fail();
    }
}

static volatile uint8_t seen;

/* Reads what a call lent to its last byte, so ASan sees a count past the memory. */
static void touch(const void *view, size_t bytes)
{
    const uint8_t *b = view;
    for (size_t i = 0; b && i < bytes; i++) {
        seen ^= b[i];
    }
}

#define LENT(call, ...)                                         \
    do {                                                        \
        size_t n_ = 0;                                          \
        __typeof__(call(__VA_ARGS__, &n_)) v_ = call(__VA_ARGS__, &n_); \
        touch(v_, n_ * sizeof *v_);                             \
    } while (0)

static void see_str(daegun_str s) { touch(s.data, s.len); }

static void see_text(daegun_text *t)
{
    daegun_str s = { 0 };
    if (t && daegun_text_str(t, &s) == DAEGUN_OK) {
        see_str(s);
    }
    daegun_text_free(t);
}

static void entered(void *user, const daegun_paint_node *node, const daegun_color_stop *stops, size_t count)
{
    (void)user;
    touch(node, sizeof *node);
    touch(stops, count * sizeof *stops);
}
static void left(void *user) { (void)user; }

static bool key_of(void *user, size_t index, uint32_t *out)
{
    const daegun_bytes *b = user;
    return daegun_read_u32_be(b->data, b->len, index, out) == DAEGUN_OK;
}

static size_t every_third(void *user, const char *t, size_t len, size_t *out, size_t cap)
{
    (void)user, (void)t;
    size_t n = 0;
    for (size_t at = 3; at < len && n < cap; at += 3) {
        out[n++] = at;
    }
    return n;
}

static void see_run(const daegun_shaped_run *run)
{
    bool b = false;
    int32_t shaper = 0;
    daegun_str name = { 0 };
    LENT(daegun_shaped_run_glyphs, run);
    LENT(daegun_shaped_run_advances, run);
    LENT(daegun_shaped_run_offsets, run);
    LENT(daegun_shaped_run_clusters, run);
    LENT(daegun_shaped_run_unsafe_to_break, run);
    LENT(daegun_shaped_run_unsafe_to_concat, run);
    LENT(daegun_shaped_run_safe_to_insert_tatweel, run);
    (void)daegun_shaped_run_complete(run, &b);
    (void)daegun_shaped_run_has_broken_syllable(run, &b);
    if (daegun_shaped_run_shaper(run, &shaper) == DAEGUN_OK && daegun_shaper_name(shaper, &name) == DAEGUN_OK) {
        see_str(name);
    }
}

static void see_scene(choices *c, const daegun_scene *scene)
{
    size_t paths = 0;
    LENT(daegun_scene_ops, scene);
    LENT(daegun_scene_shapes, scene);
    LENT(daegun_scene_gradients, scene);
    LENT(daegun_scene_stops, scene);
    (void)daegun_scene_path_count(scene, &paths);
    for (size_t i = 0; i <= paths && i < 8; i++) {
        daegun_path_view view;
        if (daegun_scene_path_at(scene, i, &view) == DAEGUN_OK) {
            touch(view.verbs, view.verbs_len);
            touch(view.points, view.points_len * sizeof *view.points);
        }
    }
    daegun_path *copy = NULL;
    (void)daegun_scene_path_copy(scene, byte(c) % 8, &copy);
    daegun_path_free(copy);
    double to_device[6];
    for (int i = 0; i < 6; i++) {
        to_device[i] = usual_number(c, i == 0 || i == 3);
    }
    daegun_ramp *ramp = NULL;
    int32_t interpolation = usual(c, DAEGUN_INTERPOLATE_LINEAR_LIGHT);
    if (daegun_scene_ramp(scene, byte(c) % 4, to_device, interpolation, &ramp) == DAEGUN_OK) {
        daegun_rgba rgba;
        (void)daegun_ramp_at(ramp, number(c), number(c), &rgba);
    }
    daegun_ramp_free(ramp);
}

static const char *const TEXTS[] = {
    "Typography is the craft of endowing human language.",
    "\xd8\xa7\xd9\x84\xd8\xb3\xd9\x84\xd8\xa7\xd9\x85 abc",
    "\xe0\xa4\x95\xe0\xa4\xbf\xe0\xa4\xb8\xe0\xa5\x80 \xe0\xa4\xad\xe0\xa5\x80",
    "\xe4\xb8\x80\xe3\x80\x82\xf0\x9f\x98\x80",
    "a\tb\nc\xc2\xad\x64 \xe2\x80\x8b e",
    "\xe0\xb8\xaa\xe0\xb8\xa7\xe0\xb8\xb1\xe0\xb8\xaa\xe0\xb8\x94\xe0\xb8\xb5 abc",
};

static const char *text(choices *c, size_t *len, char *room)
{
    uint8_t pick = byte(c);
    if (pick < 200) {
        const char *t = TEXTS[pick % (sizeof TEXTS / sizeof *TEXTS)];
        *len = strlen(t);
        return t;
    }
    *len = byte(c) % 32;
    for (size_t i = 0; i < *len; i++) {
        room[i] = (char)byte(c);
    }
    return room;
}

static void to(void *user, float x, float y) { (void)user, (void)x, (void)y; }
static void quad(void *user, float a, float b, float x, float y) { (void)user, (void)a, (void)b, (void)x, (void)y; }
static void curve(void *user, float a, float b, float c, float d, float x, float y)
{
    (void)user, (void)a, (void)b, (void)c, (void)d, (void)x, (void)y;
}
static void close_path(void *user) { (void)user; }

/* Mostly a glyph or a script the font has, as one it hasn't only says so. */
static uint32_t a_glyph(choices *c, const daegun_font *font)
{
    uint32_t count = 0, w = word(c);
    (void)daegun_font_glyph_count(font, &count);
    return count && byte(c) & 7 ? w % count : w;
}

static daegun_tag a_script(choices *c, const daegun_font *font)
{
    daegun_tag tags[16];
    size_t count = 0;
    if (byte(c) & 7 && daegun_font_script_tags(font, tags, 16, &count) == DAEGUN_OK && count) {
        return tags[byte(c) % (count < 16 ? count : 16)];
    }
    return byte(c) & 1 ? DAEGUN_TAG('a', 'r', 'a', 'b') : word(c);
}

static void shaping(choices *c, const daegun_font *font, const daegun_location *at, const char *t, size_t len)
{
    daegun_shape_options o;
    if (daegun_shape_options_default(&o, sizeof o) != DAEGUN_OK) {
        fail();
    }
    o.direction = usual(c, o.direction);
    o.script = usual_word(c, o.script);
    o.cluster_level = usual(c, o.cluster_level);
    o.ignorables = usual(c, o.ignorables);
    o.invisible_glyph = word(c);
    o.not_found_glyph = word(c);
    o.seed_script = usual_word(c, 0);
    o.beginning_of_text = byte(c) & 1;
    o.has_point_size = !(byte(c) & 7);
    o.has_invisible_glyph = byte(c) & 1;
    o.point_size = number(c);
    daegun_feature feature = { usual_word(c, DAEGUN_TAG('l', 'i', 'g', 'a')), word(c), word(c) % 64, word(c) % 64 };
    o.features = &feature;
    o.features_len = byte(c) & 1;
    o.before = t;
    o.before_len = byte(c) & 3 ? 0 : byte(c) % (len + 1);
    o.language = byte(c) & 1 ? "ar" : NULL;
    daegun_shape_options *opts = sized(c, &o, sizeof o);

    daegun_shaped_run *run = NULL;
    daegun_status st = daegun_font_shape(font, t, len, at, opts, &run);
    held(opts, sizeof o, st);
    if (st == DAEGUN_OK) {
        double width = 0;
        see_run(run);
        (void)daegun_shaped_run_width(run, &width);
        daegun_shaped_run *copy = NULL;
        (void)daegun_shaped_run_copy(run, &copy);
        daegun_shaped_run_free(copy);
        held(opts, sizeof o, daegun_font_shape_into(font, t, len, at, opts, &run));
    }
    daegun_shaped_run_free(run);
    daegun_bidi_runs *runs = NULL;
    st = daegun_font_shape_bidi(font, t, len, at, usual(c, DAEGUN_BASE_AUTO), opts, &runs);
    held(opts, sizeof o, st);
    size_t count = 0;
    if (st == DAEGUN_OK && daegun_bidi_runs_count(runs, &count) == DAEGUN_OK) {
        for (size_t i = 0; i <= count && i < 8; i++) {
            const daegun_shaped_run *bidi = NULL;
            uint8_t level = 0;
            size_t start = 0, end = 0;
            if (daegun_bidi_runs_at(runs, i, &bidi, &level, &start, &end) == DAEGUN_OK) {
                see_run(bidi);
            }
        }
    }
    daegun_bidi_runs_free(runs);
    free(opts);
}

static void laying_out(choices *c, const daegun_font *font, const daegun_location *at, const char *t, size_t len)
{
    daegun_layout_options o;
    if (daegun_layout_options_default(&o, sizeof o) != DAEGUN_OK) {
        fail();
    }
    o.max_inline_size = usual_number(c, (double)(word(c) % 40000));
    o.line_height = usual_number(c, 1500.0);
    o.tab_size = usual_number(c, 8.0);
    o.max_lines = byte(c) % 8;
    o.has_line_height = byte(c) & 1;
    o.has_max_lines = byte(c) & 1;
    o.align = usual(c, byte(c) % 4);
    o.writing_mode = usual(c, byte(c) % 3);
    o.text_orientation = usual(c, byte(c) % 3);
    o.base_direction = usual(c, byte(c) % 3);
    o.strategy = usual(c, byte(c) % 2);
    daegun_feature feature = { usual_word(c, DAEGUN_TAG('l', 'i', 'g', 'a')), word(c), word(c) % 64, word(c) % 64 };
    o.features = &feature;
    o.features_len = byte(c) & 1;
    daegun_layout_options *opts = sized(c, &o, sizeof o);
    daegun_layout *layout = NULL;
    daegun_status st = daegun_font_layout(font, t, len, at, opts, &layout);
    held(opts, sizeof o, st);
    daegun_layout_info info;
    if (st == DAEGUN_OK && daegun_layout_info_of(layout, &info) == DAEGUN_OK) {
        for (size_t i = 0; i < info.line_count; i++) {
            daegun_layout_line line;
            if (daegun_layout_line_at(layout, i, &line) != DAEGUN_OK) {
                continue;
            }
            for (size_t r = 0; r <= line.run_count && r < 8; r++) {
                daegun_layout_run run;
                if (daegun_layout_run_at(layout, i, r, &run) == DAEGUN_OK) {
                    see_run(run.run);
                }
            }
        }
    }
    daegun_layout_free(layout);
    free(opts);

    daegun_justify_options j;
    if (daegun_justify_options_default(&j, sizeof j) != DAEGUN_OK) {
        fail();
    }
    j.script = a_script(c, font);
    j.language_system = usual_word(c, 0);
    j.target_width = usual_number(c, (double)(word(c) % 40000));
    j.tolerance = usual_number(c, 1.0);
    daegun_justify_options *jopts = sized(c, &j, sizeof j);
    daegun_shaped_run *run = NULL;
    daegun_justification justified;
    held(jopts, sizeof j, daegun_font_justify(font, t, len, at, usual(c, DAEGUN_HORIZONTAL), jopts, &run, &justified));
    daegun_shaped_run_free(run);
    free(jopts);
}

/* An image a rasterizer lent: its rows within its pixels, each read to its last byte. */
static void see_image(const daegun_image *image)
{
    size_t row = (size_t)image->width * image->channels;
    if (image->height && (image->stride < row || image->pixels_len < image->stride * (image->height - 1) + row)) {
        fail();
    }
    touch(image->pixels, image->pixels_len);
}

static void drawing(choices *c, const daegun_font *font, const daegun_location *at, uint32_t gid)
{
    daegun_pen pen = { to, to, quad, curve, close_path, NULL };
    (void)daegun_font_outline(font, gid, at, &pen);

    daegun_outline_options o;
    if (daegun_outline_options_default(&o, sizeof o) != DAEGUN_OK) {
        fail();
    }
    o.hinting = usual(c, byte(c) % 5);
    for (int i = 0; i < 6; i++) {
        o.transform[i] = usual_number32(c, (float)(i == 0 || i == 3));
    }
    o.stroke.width = usual_number32(c, 30.0f);
    o.stroke.cap = usual(c, byte(c) % 3);
    o.stroke.join = usual(c, byte(c) % 3);
    o.stroke.miter_limit = usual_number32(c, 4.0f);
    o.embolden = usual_number32(c, 20.0f);
    o.oblique = usual_number32(c, 0.2f);
    o.has_transform = byte(c) & 1;
    o.has_stroke = byte(c) & 1;
    o.has_embolden = byte(c) & 1;
    o.has_oblique = byte(c) & 1;
    daegun_outline_options *opts = sized(c, &o, sizeof o);
    float px = usual_number32(c, (float)(1 + byte(c) % 64));
    daegun_prepared_glyph prepared;
    held(opts, sizeof o, daegun_font_prepared_outline(font, gid, at, px, opts, &pen, &prepared));
    free(opts);

    daegun_hinted_outline *hinted = NULL;
    if (daegun_font_hinted_outline(font, gid, at, px, usual(c, byte(c) % 5), &hinted) == DAEGUN_OK) {
        LENT(daegun_hinted_outline_points, hinted);
        LENT(daegun_hinted_outline_kinds, hinted);
        LENT(daegun_hinted_outline_contours, hinted);
        (void)daegun_hinted_outline_draw(hinted, &pen);
    }
    daegun_hinted_outline_free(hinted);

    daegun_scene_options s;
    if (daegun_scene_options_default(&s, sizeof s) != DAEGUN_OK) {
        fail();
    }
    s.foreground.r = byte(c);
    s.foreground.a = byte(c);
    s.palette = (uint16_t)usual_word(c, byte(c) % 2);
    daegun_scene_options *sopts = sized(c, &s, sizeof s);
    daegun_scene *scene = NULL;
    daegun_status scene_status = daegun_font_color_scene(font, gid, at, sopts, &scene);
    held(sopts, sizeof s, scene_status);
    if (scene_status == DAEGUN_OK) {
        see_scene(c, scene);
    }
    daegun_scene_free(scene);

    daegun_rasterizer *r = daegun_rasterizer_new();
    (void)daegun_rasterizer_set_cache_budget(r, gid & 1 ? 0 : SIZE_MAX);
    daegun_subpixel_layout *layout = NULL;
    if (byte(c) & 1) {
        (void)daegun_subpixel_layout_horizontal(code(c), &layout);
    }
    daegun_raster_options ro;
    if (daegun_raster_options_default(&ro, sizeof ro) != DAEGUN_OK) {
        fail();
    }
    ro.outline = byte(c) & 1 ? &o : NULL;
    ro.layout = layout;
    ro.offset[0] = usual_number32(c, 0.25f);
    ro.offset[1] = usual_number32(c, 0.5f);
    ro.gamma = usual(c, byte(c) % 2);
    ro.text.r = byte(c);
    ro.contrast = usual_number32(c, 0.5f);
    ro.darken = byte(c) & 1;
    daegun_raster_options *ropts = sized(c, &ro, sizeof ro);
    daegun_image image;
    bool given = byte(c) & 1;
    daegun_status st = daegun_rasterizer_glyph(r, font, gid, at, px, given ? ropts : NULL, &image);
    if (given) {
        held(ropts, sizeof ro, st);
    }
    if (st == DAEGUN_OK) {
        see_image(&image);
    }
    st = daegun_rasterizer_color_glyph(r, font, gid, at, px, ropts, sopts, &image);
    held(ropts, sizeof ro, st);
    held(sopts, sizeof s, st);
    if (st == DAEGUN_OK) {
        see_image(&image);
    }
    uint8_t window[48 * 48 * 4];
    daegun_target target = { -24, 24, 48, 48, 48 * 4, window, byte(c) & 1 ? sizeof window : byte(c) };
    (void)daegun_rasterizer_glyph_into(r, font, gid, at, px, ropts, &target);
    (void)daegun_rasterizer_color_glyph_into(r, font, gid, at, px, ropts, sopts, &target);
    if (daegun_rasterizer_distance_field(r, font, gid, at, px, usual(c, byte(c) % 2), usual_number32(c, 3.0f),
                                         &image) == DAEGUN_OK) {
        see_image(&image);
    }
    free(ropts);
    daegun_subpixel_layout_free(layout);
    daegun_rasterizer_free(r);
    free(sopts);

    char name[DAEGUN_MAX_GLYPH_NAME + 1];
    size_t n = 0;
    (void)daegun_font_glyph_name(font, gid, name, byte(c) % sizeof name, &n);
    double advance[1];
    (void)daegun_font_advances(font, &gid, 1, at, usual(c, DAEGUN_HORIZONTAL), advance);
}

/* Every other read that takes a glyph. */
static void reading_glyph(choices *c, const daegun_font *font, const daegun_location *at, uint32_t gid)
{
    double d = 0, values[16];
    bool b = false;
    uint8_t present[16];
    int32_t kind = 0;
    uint32_t u = 0;
    size_t count = 0, room = byte(c) % 17;
    int32_t orientation = usual(c, byte(c) % 2);
    uint16_t palette = byte(c) & 1 ? 0 : (uint16_t)word(c);
    daegun_bounds bounds;

    (void)daegun_font_glyph_bounds(font, gid, at, &bounds);
    (void)daegun_font_vertical_origin(font, gid, at, &d);
    (void)daegun_font_advance(font, gid, at, orientation, &d);
    (void)daegun_font_glyph_class(font, gid, &kind);
    (void)daegun_font_mark_attachment_class(font, gid, &u);
    (void)daegun_font_has_overlap_flag(font, gid, &b);
    (void)daegun_font_colr_clip_box(font, gid, at, &bounds);
    (void)daegun_font_ligature_carets(font, gid, at, orientation, values, present, room, &count);
    (void)daegun_font_math_italics_correction(font, gid, at, &d);
    (void)daegun_font_math_top_accent_attachment(font, gid, at, &d);
    (void)daegun_font_math_is_extended_shape(font, gid, &b);
    (void)daegun_font_math_kern(font, gid, at, usual(c, byte(c) % 4), usual_number(c, (double)(word(c) % 4096)), &d);

    daegun_math_construction *math = NULL;
    if (daegun_font_math_glyph_variants(font, gid, at, orientation, &math) == DAEGUN_OK) {
        daegun_glyph_assembly assembly;
        LENT(daegun_math_construction_variants, math);
        if (daegun_math_construction_assembly(math, &assembly) == DAEGUN_OK) {
            touch(assembly.parts, assembly.parts_len * sizeof *assembly.parts);
        }
    }
    daegun_math_construction_free(math);

    daegun_bitmap *bitmap = NULL;
    if (daegun_font_bitmap(font, gid, (uint16_t)usual_word(c, 8u + byte(c)), &bitmap) == DAEGUN_OK) {
        daegun_bitmap_info info;
        if (daegun_bitmap_info_of(bitmap, &info) == DAEGUN_OK) {
            touch(info.data, info.data_len);
        }
    }
    daegun_bitmap_free(bitmap);

    daegun_colr_layer layers[16];
    (void)daegun_font_colr_layers(font, gid, palette, layers, room, &count);
    daegun_paint_graph *graph = NULL;
    if (daegun_font_paint_graph(font, gid, at, palette, &graph) == DAEGUN_OK) {
        LENT(daegun_paint_graph_nodes, graph);
        LENT(daegun_paint_graph_stops, graph);
    }
    daegun_paint_graph_free(graph);
    const daegun_paint_visitor visitor = { entered, left, NULL };
    (void)daegun_font_visit_paint_graph(font, gid, at, palette, &visitor);

    daegun_cff_hints *hints = NULL;
    if (daegun_font_cff_hints(font, gid, at, &hints) == DAEGUN_OK) {
        size_t masks = 0;
        LENT(daegun_cff_hints_stems, hints);
        (void)daegun_cff_hints_mask_count(hints, &masks);
        for (size_t i = 0; i <= masks && i < 8; i++) {
            daegun_cff_mask mask;
            if (daegun_cff_hints_mask_at(hints, i, &mask) == DAEGUN_OK) {
                touch(mask.bits, mask.bits_len);
            }
        }
    }
    daegun_cff_hints_free(hints);

    daegun_quads *quads = NULL;
    if (daegun_font_quads(font, gid, at, &quads) == DAEGUN_OK) {
        LENT(daegun_quads_data, quads);
        (void)daegun_quads_normalize_winding(quads);
    }
    daegun_quads_free(quads);

    uint32_t closed[16];
    (void)daegun_font_glyph_closure(font, &gid, 1, at, closed, room, &count);
    size_t added = 0;
    (void)daegun_font_prewarm(font, &gid, 1, at, &added);
    daegun_subset *subset = NULL;
    if (daegun_font_subset(font, &gid, 1, at, &subset) == DAEGUN_OK) {
        uint32_t renumbered = 0;
        LENT(daegun_subset_bytes, subset);
        (void)daegun_subset_renumbered(subset, gid, &renumbered);
        (void)daegun_subset_renumbered(subset, word(c), &renumbered);
    }
    daegun_subset_free(subset);
}

/* Every read of the font as a whole. */
static void reading_font(choices *c, const daegun_font *font, const daegun_location *at)
{
    double d = 0;
    bool b = false;
    uint16_t u16 = 0;
    uint32_t u32 = 0;
    int32_t i32 = 0;
    size_t count = 0, room = byte(c) % 33;
    daegun_tag script = a_script(c, font), system = usual_word(c, 0), tag = usual_word(c, 0);
    int32_t orientation = usual(c, byte(c) % 2);
    daegun_bounds bounds;

    (void)daegun_font_glyph_count(font, &u32);
    (void)daegun_font_units_per_em(font, &u16);
    (void)daegun_font_is_variable(font, &b);
    (void)daegun_font_ascender(font, at, &d);
    (void)daegun_font_descender(font, at, &d);
    (void)daegun_font_cap_height(font, at, &d);
    (void)daegun_font_italic_angle(font, at, &d);
    (void)daegun_font_bounds(font, &bounds);
    (void)daegun_font_default_vertical_origin(font, &d);
    (void)daegun_font_style(font, at, &i32);
    (void)daegun_font_pdf_flags(font, at, &u32);
    (void)daegun_font_tracking(font, usual_number(c, 12.0), orientation, &d);
    (void)daegun_font_math_min_connector_overlap(font, &d);
    (void)daegun_font_has_glyph_free_base(font, &b);
    daegun_line_metrics lines;
    if (daegun_font_line_metrics(font, at, orientation, &lines) == DAEGUN_OK) {
        (void)daegun_line_metrics_line_height(&lines, &d);
    }
    daegun_typographic_metrics typo;
    (void)daegun_font_typographic_metrics(font, at, &typo);
    daegun_math_constants constants;
    (void)daegun_font_math_constants(font, at, &constants);
    daegun_extents extents;
    (void)daegun_font_base_extents(font, script, at, orientation, system, tag, &extents);
    daegun_os2 os2;
    if (daegun_font_os2(font, at, &os2) == DAEGUN_OK) {
        (void)daegun_os2_is_italic(&os2, &b);
        (void)daegun_os2_is_bold(&os2, &b);
        (void)daegun_os2_is_regular(&os2, &b);
        (void)daegun_os2_is_oblique(&os2, &b);
        (void)daegun_os2_uses_typo_metrics(&os2, &b);
    }

    uint32_t id = 0, code_point = word(c), selector = word(c);
    (void)daegun_font_glyph_id(font, code_point, &id);
    (void)daegun_font_has_glyph(font, code_point, &b);
    (void)daegun_font_variation_glyph_id(font, code_point, selector, &id);

    daegun_tag tags[32];
    (void)daegun_font_table_tags(font, tags, 32, &count);
    daegun_tag present_tag = count ? tags[byte(c) % (count < 32 ? count : 32)] : word(c);
    (void)daegun_font_table_tags(font, tags, room, &count);
    (void)daegun_font_script_tags(font, tags, room, &count);
    (void)daegun_font_language_system_tags(font, script, tags, room, &count);
    (void)daegun_font_feature_tags(font, script, system, tags, room, &count);
    daegun_bytes table = { 0 };
    if (daegun_font_table(font, present_tag, &table) == DAEGUN_OK) {
        touch(table.data, table.len);
    }
    (void)daegun_font_table(font, word(c), &(daegun_bytes){ 0 });
    (void)daegun_font_has_table(font, present_tag, &b);
    daegun_axis axes[32];
    (void)daegun_font_axes(font, axes, room, &count);
    uint32_t codes[32], gids[32];
    (void)daegun_font_coverage(font, codes, gids, room, &count);
    (void)daegun_font_justification_glyphs(font, script, gids, room, &count);

    daegun_palette palettes[32];
    daegun_rgba colors[32];
    uint16_t name_ids[32];
    uint8_t named[32] = { 0 };
    (void)daegun_font_palettes(font, palettes, room, &count);
    (void)daegun_font_palette_colors(font, (uint16_t)word(c), colors, room, &count);
    (void)daegun_font_palette_entry_labels(font, name_ids, named, room, &count);

    daegun_text *text = NULL;
    (void)daegun_font_family_name(font, at, &text);
    see_text(text);
    text = NULL;
    (void)daegun_font_name(font, (uint16_t)usual_word(c, byte(c) % 26), at, &text);
    see_text(text);

    daegun_names *names = NULL;
    if (daegun_font_names(font, at, &names) == DAEGUN_OK && daegun_names_count(names, &count) == DAEGUN_OK) {
        for (size_t i = 0; i <= count && i < 8; i++) {
            daegun_str s = { 0 };
            if (daegun_names_at(names, i, &u16, &s) == DAEGUN_OK) {
                see_str(s);
            }
        }
    }
    daegun_names_free(names);

    daegun_localized_names *localized = NULL;
    if (daegun_font_localized_names(font, (uint16_t)usual_word(c, byte(c) % 26), at, &localized) == DAEGUN_OK
        && daegun_localized_names_count(localized, &count) == DAEGUN_OK) {
        for (size_t i = 0; i <= count && i < 8; i++) {
            daegun_str language = { 0 }, s = { 0 };
            if (daegun_localized_names_at(localized, i, &language, &s) == DAEGUN_OK) {
                see_str(language);
                see_str(s);
            }
        }
    }
    daegun_localized_names_free(localized);

    daegun_named_instances *instances = NULL;
    if (daegun_font_named_instances(font, &instances) == DAEGUN_OK
        && daegun_named_instances_count(instances, &count) == DAEGUN_OK) {
        for (size_t i = 0; i <= count && i < 8; i++) {
            daegun_str s = { 0 };
            if (daegun_named_instances_name(instances, i, &s) == DAEGUN_OK) {
                see_str(s);
            }
            if (daegun_named_instances_postscript_name(instances, i, &s) == DAEGUN_OK) {
                see_str(s);
            }
            LENT(daegun_named_instances_coords, instances, i);
        }
    }
    daegun_named_instances_free(instances);

    daegun_str_list *glyph_names = NULL;
    if (daegun_font_glyph_names(font, &glyph_names) == DAEGUN_OK
        && daegun_str_list_count(glyph_names, &count) == DAEGUN_OK) {
        for (size_t i = 0; i <= count && i < 8; i++) {
            daegun_str s = { 0 };
            if (daegun_str_list_at(glyph_names, i, &s) == DAEGUN_OK) {
                see_str(s);
            }
        }
    }
    daegun_str_list_free(glyph_names);

    daegun_stat *stat = NULL;
    if (daegun_font_stat(font, at, &stat) == DAEGUN_OK) {
        daegun_str s = { 0 };
        size_t values = 0;
        LENT(daegun_stat_axes, stat);
        LENT(daegun_stat_combo_values, stat);
        if (daegun_stat_elided_fallback_name(stat, &s) == DAEGUN_OK) {
            see_str(s);
        }
        (void)daegun_stat_value_count(stat, &values);
        for (size_t i = 0; i <= values && i < 8; i++) {
            daegun_stat_value value;
            (void)daegun_stat_value_at(stat, i, &value);
            if (daegun_stat_value_name(stat, i, &s) == DAEGUN_OK) {
                see_str(s);
            }
            if (daegun_stat_axis_name(stat, i, &s) == DAEGUN_OK) {
                see_str(s);
            }
        }
    }
    daegun_stat_free(stat);

    daegun_base_script *base = NULL;
    if (daegun_font_base_script(font, script, at, orientation, &base) == DAEGUN_OK) {
        (void)daegun_base_script_default_baseline(base, &tag);
        LENT(daegun_base_script_baselines, base);
    }
    daegun_base_script_free(base);

    daegun_blob *instance = NULL;
    if (daegun_font_instance(font, at, &instance) == DAEGUN_OK) {
        LENT(daegun_blob_data, instance);
    }
    daegun_blob_free(instance);

    daegun_table_map *map = NULL;
    if (daegun_font_instance_tables(font, at, &map) == DAEGUN_OK) {
        daegun_blob *built = NULL;
        size_t entries = 0;
        (void)daegun_table_map_count(map, &entries);
        for (size_t i = 0; i <= entries && i < 8; i++) {
            daegun_bytes bytes = { 0 };
            (void)daegun_table_map_tag_at(map, i, &tag);
            if (daegun_table_map_bytes_at(map, i, &bytes) == DAEGUN_OK) {
                touch(bytes.data, bytes.len);
            }
        }
        daegun_bytes lent = { 0 };
        if (daegun_table_map_get(map, tag, &lent) == DAEGUN_OK) {
            touch(lent.data, lent.len);
        }
        (void)daegun_table_map_set(map, word(c), named, room);
        (void)daegun_table_map_remove(map, tag);
        (void)daegun_table_map_build(map, &built);
        daegun_blob_free(built);
    }
    daegun_table_map_free(map);
    daegun_table_map *fresh = daegun_table_map_new();
    daegun_blob *lone = NULL;
    (void)daegun_table_map_set(fresh, tag, table.data, table.len);
    (void)daegun_table_map_build(fresh, &lone);
    daegun_blob_free(lone);
    daegun_table_map_free(fresh);

    if (at) {
        LENT(daegun_location_coords, at);
        LENT(daegun_location_normalized, at);
    }

    daegun_cache_stats stats;
    int32_t cache = usual(c, byte(c) % 4);
    /* The font stays in the pool past its free, so what follows draws from it. */
    daegun_cache_pool *pool = daegun_cache_pool_new(1 << 12);
    (void)daegun_font_set_cache_pool(font, cache, pool);
    (void)daegun_font_cache_stats(font, cache, &stats);
    (void)daegun_font_set_cache_budget(font, cache, word(c) % (1 << 16));
    (void)daegun_cache_pool_set_budget(pool, 1 << 16);
    (void)daegun_cache_pool_stats(pool, &stats);
    daegun_cache_pool_free(pool);
    (void)daegun_font_clear_cache(font, cache);
    (void)daegun_font_index_allowance(font, &count);
    (void)daegun_font_set_index_allowance(font, word(c) % (1 << 16));
}

/* Every call on text, which is any bytes: only some are UTF-8. */
static void reading_text(choices *c, const daegun_font *font, const daegun_location *at, const char *t, size_t len)
{
    size_t count = 0, room = byte(c) % 65;
    uint32_t gids[64];
    uint8_t present[64];
    double values[64];
    size_t breaks[64];
    daegun_line_break lines[64];
    daegun_script_run runs[64];
    daegun_visual_run visual[64];

    (void)daegun_font_glyph_ids(font, t, len, gids, present, room, &count);
    (void)daegun_font_caret_positions(font, t, len, at, usual(c, byte(c) % 2), values, room, &count);
    daegun_subset *subset = NULL;
    (void)daegun_font_subset_text(font, t, len, at, &subset);
    daegun_subset_free(subset);

    daegun_jstf_priorities *levels = NULL;
    if (daegun_font_justification_priorities(font, a_script(c, font), usual_word(c, 0), &levels) == DAEGUN_OK) {
        size_t n = 0;
        (void)daegun_jstf_priorities_count(levels, &n);
        for (size_t level = 0; level <= n && level < 4; level++) {
            const uint16_t *lookups = NULL;
            size_t lookup_count = 0;
            if (daegun_jstf_priorities_lookups(levels, level, usual(c, byte(c) % 8), &lookups, &lookup_count)
                == DAEGUN_OK) {
                touch(lookups, lookup_count * sizeof *lookups);
            }
            daegun_shaped_run *run = NULL;
            if (daegun_font_shape_justified(font, t, len, at, usual(c, byte(c) % 2), levels, level,
                                            usual(c, byte(c) % 2), &run) == DAEGUN_OK) {
                see_run(run);
            }
            daegun_shaped_run_free(run);
        }
    }
    daegun_jstf_priorities_free(levels);

    daegun_tag tag = 0;
    daegun_feature feature;
    (void)daegun_tag_from_string(t, len, &tag);
    (void)daegun_feature_from_string(t, len, &feature);
    (void)daegun_grapheme_boundaries(t, len, breaks, room, &count);
    (void)daegun_word_boundaries(t, len, breaks, room, &count);
    (void)daegun_line_break_opportunities(t, len, byte(c) & 1 ? every_third : NULL, NULL, lines, room, &count);
    (void)daegun_script_runs(t, len, runs, room, &count);

    daegun_bidi_paragraph *paragraph = NULL;
    if (daegun_bidi_paragraph_new(t, len, usual(c, byte(c) % 3), &paragraph) == DAEGUN_OK) {
        uint8_t level = 0;
        (void)daegun_bidi_paragraph_base_level(paragraph, &level);
        LENT(daegun_bidi_paragraph_levels, paragraph);
        LENT(daegun_bidi_paragraph_visual_order, paragraph);
        LENT(daegun_bidi_paragraph_paragraphs, paragraph);
        (void)daegun_bidi_paragraph_visual_runs(paragraph, t, len, byte(c), byte(c), visual, room, &count);
    }
    daegun_bidi_paragraph_free(paragraph);
}

/* Every call taking no font and no bytes: only codes, tags and numbers. */
static void reading_codes(choices *c)
{
    bool b = false;
    int32_t i32 = 0;
    uint32_t u32 = 0, point = word(c);
    size_t count = 0;
    daegun_tag tags[8], script = usual_word(c, DAEGUN_TAG('A', 'r', 'a', 'b')), found = 0;
    daegun_str s = { 0 };

    see_str(daegun_version());
    (void)daegun_abi_version();
    (void)daegun_has_dictionaries();
    (void)daegun_status_name((daegun_status)code(c));
    (void)daegun_last_error();
    daegun_thread_release();
    (void)daegun_writing_mode_is_vertical(code(c), &b);
    (void)daegun_cluster_level_is_graphemes(code(c), &b);
    (void)daegun_cluster_level_is_monotone(code(c), &b);
    (void)daegun_hint_mode_may_autohint(code(c), &b);
    (void)daegun_shaper_name(code(c), &s);
    if (daegun_script_name(script, &s) == DAEGUN_OK) {
        see_str(s);
    }
    (void)daegun_script_direction(script, &i32);
    (void)daegun_script_opentype_tags(script, tags, byte(c) % 9, &count);
    (void)daegun_script_is_context_dependent(script, &b);
    (void)daegun_general_category(point, &i32);
    (void)daegun_script(point, &found);
    (void)daegun_vertical_form(point, &u32);
    (void)daegun_vertical_orientation(point, &i32);
    (void)daegun_ot_round(number(c));
    (void)daegun_max_area_for(number32(c), number32(c));

    float backdrop[3], source[3], out[3], alpha = 0;
    for (int i = 0; i < 3; i++) {
        backdrop[i] = number32(c);
        source[i] = number32(c);
    }
    (void)daegun_blend(byte(c) % 32 - 1, backdrop, source, out);
    (void)daegun_composite(byte(c) % 32 - 1, source, number32(c), backdrop, number32(c), out, &alpha);
}

/* A path of the choices' numbers through every call on one, and the raster helpers beside them. */
static void paths(choices *c)
{
    daegun_pen pen = { to, to, quad, curve, close_path, NULL };
    daegun_path *path = daegun_path_new();
    for (int i = byte(c) % 12; i > 0; i--) {
        float a = number32(c), b = number32(c);
        switch (byte(c) % 7) {
        case 0: (void)daegun_path_move_to(path, a, b); break;
        case 1: (void)daegun_path_line_to(path, a, b); break;
        case 2: (void)daegun_path_quad_to(path, a, b, number32(c), number32(c)); break;
        case 3:
            (void)daegun_path_curve_to(path, a, b, number32(c), number32(c), number32(c), number32(c));
            break;
        case 4: (void)daegun_path_close(path); break;
        case 5: (void)daegun_path_clear(path); break;
        default: (void)daegun_path_line_to(path, (float)(byte(c) % 64), (float)(byte(c) % 64)); break;
        }
    }
    bool empty = false;
    size_t cost = 0;
    daegun_bounds bounds;
    (void)daegun_path_is_empty(path, &empty);
    (void)daegun_path_cost(path, &cost);
    (void)daegun_path_bounds(path, &bounds);
    LENT(daegun_path_verbs, path);
    LENT(daegun_path_points, path);
    double transform[6];
    for (int i = 0; i < 6; i++) {
        transform[i] = number(c);
    }
    (void)daegun_path_replay(path, byte(c) & 1 ? transform : NULL, &pen);
    daegun_rasterizer *r = daegun_rasterizer_new();
    daegun_image image;
    if (daegun_rasterizer_path(r, path, usual(c, byte(c) % 2), byte(c) & 1 ? transform : NULL, &image) == DAEGUN_OK) {
        see_image(&image);
    }
    uint8_t tile[32 * 32];
    daegun_target region = { -16, 16, 32, 32, 32, tile, sizeof tile };
    (void)daegun_rasterizer_path_into(r, path, usual(c, byte(c) % 2), byte(c) & 1 ? transform : NULL, &region);
    daegun_rasterizer_free(r);
    daegun_stroke_style style = { number32(c), code(c), code(c), number32(c) };
    (void)daegun_path_stroke(path, &style, number32(c), &pen);
    (void)daegun_path_stroke_simplified(path, &style, number32(c), &pen);
    daegun_quads *quads = NULL;
    if (daegun_path_quads(path, number32(c), &quads) == DAEGUN_OK) {
        LENT(daegun_quads_data, quads);
        (void)daegun_quads_normalize_winding(quads);
    }
    daegun_quads_free(quads);
    daegun_contours *flat = NULL;
    if (daegun_path_flatten(path, number32(c), &flat) == DAEGUN_OK) {
        size_t count = 0;
        (void)daegun_contours_count(flat, &count);
        for (size_t i = 0; i <= count && i < 8; i++) {
            LENT(daegun_contours_points, flat, i);
        }
        daegun_contours *resolved = NULL;
        (void)daegun_contours_resolve_overlaps(flat, &resolved);
        daegun_contours_free(resolved);
    }
    daegun_contours_free(flat);
    daegun_pen into;
    if (daegun_path_as_pen(path, &into) == DAEGUN_OK && into.move_to && into.line_to) {
        into.move_to(into.user, number32(c), number32(c));
        into.line_to(into.user, number32(c), number32(c));
    }
    daegun_path_free(path);

    daegun_point points[16];
    size_t lengths[4] = { byte(c) % 5, byte(c) % 5, byte(c) % 5, byte(c) % 5 };
    for (int i = 0; i < 16; i++) {
        points[i].x = number32(c);
        points[i].y = number32(c);
    }
    daegun_contours *mine = NULL;
    if (daegun_contours_new(points, lengths, byte(c) % 5, &mine) == DAEGUN_OK) {
        daegun_contours *resolved = NULL;
        (void)daegun_contours_resolve_overlaps(mine, &resolved);
        daegun_contours_free(resolved);
    }
    daegun_contours_free(mine);

    daegun_subpixel_layout *layouts[5] = { NULL, NULL, NULL, NULL, NULL };
    (void)daegun_subpixel_layout_grayscale(&layouts[0]);
    (void)daegun_subpixel_layout_horizontal(code(c), &layouts[1]);
    (void)daegun_subpixel_layout_vertical(code(c), &layouts[2]);
    (void)daegun_subpixel_layout_unfiltered(code(c), code(c), &layouts[3]);
    float weights[48];
    for (int i = 0; i < 48; i++) {
        weights[i] = byte(c) & 1 ? number32(c) : 1.0f / 16.0f;
    }
    (void)daegun_subpixel_layout_from_weights(byte(c) % 5, byte(c) % 5, byte(c) % 5, byte(c) % 5, (int8_t)byte(c),
                                              (int8_t)byte(c), weights, byte(c) % 49, &layouts[4]);
    for (int i = 0; i < 5; i++) {
        if (!layouts[i]) {
            continue;
        }
        bool gray = false;
        uint8_t x = 0, y = 0;
        int8_t sx = 0, sy = 0;
        size_t px = 0, py = 0;
        uint64_t key = 0;
        (void)daegun_subpixel_layout_is_grayscale(layouts[i], &gray);
        (void)daegun_subpixel_layout_channels(layouts[i], &x);
        (void)daegun_subpixel_layout_oversample(layouts[i], &x, &y);
        (void)daegun_subpixel_layout_taps(layouts[i], &x, &y);
        (void)daegun_subpixel_layout_origin(layouts[i], &sx, &sy);
        (void)daegun_subpixel_layout_pad(layouts[i], &px, &py);
        (void)daegun_subpixel_layout_key(layouts[i], &key);
        for (size_t channel = 0; channel < 4; channel++) {
            LENT(daegun_subpixel_layout_weights, layouts[i], channel);
        }
        daegun_subpixel_layout_free(layouts[i]);
    }

    daegun_shelf_packer *packer = daegun_shelf_packer_new(word(c) % 4096, word(c) % 4096);
    for (int i = byte(c) % 8; i > 0; i--) {
        daegun_rect rect;
        (void)daegun_shelf_packer_insert(packer, word(c), word(c), &rect);
    }
    (void)daegun_shelf_packer_reset(packer);
    daegun_shelf_packer_free(packer);
}

/* The byte-level readers over a table of the font, from an offset and with indexes past every end. */
static void structures(choices *c, const daegun_font *font)
{
    daegun_tag tags[64];
    size_t tag_count = 0;
    if (daegun_font_table_tags(font, tags, 64, &tag_count) != DAEGUN_OK || !tag_count) {
        return;
    }
    daegun_bytes table = { 0 };
    daegun_tag tag = tags[byte(c) % (tag_count < 64 ? tag_count : 64)];
    if (daegun_font_table(font, tag, &table) != DAEGUN_OK) {
        return;
    }
    size_t skip = byte(c) % 64;
    const uint8_t *data = table.data + (skip < table.len ? skip : table.len);
    size_t len = table.len - (skip < table.len ? skip : table.len);
    uint32_t glyphs = word(c) % (1 << 17), glyph = word(c), got32 = 0;
    uint16_t state = (uint16_t)word(c), kind = (uint16_t)word(c), got16 = 0;
    size_t index = byte(c) & 1 ? word(c) : byte(c), off = byte(c) & 1 ? word(c) : word(c) % (len + 1), count = 0;
    size_t room = byte(c) % 33;
    size_t got = 0;
    int16_t i16 = 0, x = 0, y = 0;

    (void)daegun_read_u16_be(data, len, off, &got16);
    (void)daegun_read_i16_be(data, len, off, &i16);
    (void)daegun_read_u24_be(data, len, off, &got32);
    (void)daegun_read_u32_be(data, len, off, &got32);
    (void)daegun_read_offset24(data, len, off, &got);
    (void)daegun_records_fit(off, index, byte(c), len);
    touch(daegun_bytes_window(data, len, off, 2), 2);
    uint8_t copy[64];
    size_t copy_len = len < sizeof copy ? len : sizeof copy;
    if (copy_len) {
        memcpy(copy, data, copy_len);
    }
    (void)daegun_write_u16_be(copy, copy_len, off, state);
    (void)daegun_write_i16_be(copy, copy_len, off, -1);
    (void)daegun_write_u32_be(copy, copy_len, off, glyph);
    (void)daegun_write_offset24(copy, copy_len, off, index);
    daegun_bytes keys = { data, len };
    bool found = false;
    (void)daegun_search_records(index, glyph, key_of, &keys, &got, &found);

    uint32_t gids[32];
    (void)daegun_coverage_index(data, len, glyph, &got32);
    (void)daegun_coverage_glyphs(data, len, off, gids, room, &count);

    daegun_aat_lookup *lookup = NULL;
    if (daegun_aat_lookup_parse(data, len, glyphs, &lookup) == DAEGUN_OK) {
        daegun_glyph_value values[32];
        (void)daegun_aat_lookup_value(lookup, glyph, &got16);
        (void)daegun_aat_lookup_entries(lookup, values, room, &count);
    }
    daegun_aat_lookup_free(lookup);

    daegun_aat_state_table *states = NULL;
    if (daegun_aat_state_table_parse(data, len, byte(c) % 4, glyphs, &states) == DAEGUN_OK) {
        daegun_aat_entry entry;
        (void)daegun_aat_state_table_class(states, glyph, &got16);
        (void)daegun_aat_state_table_entry(states, state, kind, &entry);
    }
    daegun_aat_state_table_free(states);

    (void)daegun_ankr_version(data, len, &got16);
    (void)daegun_ankr_control_point(data, len, off, &x, &y);
    daegun_ankr *ankr = NULL;
    if (daegun_ankr_parse(data, len, glyphs, &ankr) == DAEGUN_OK) {
        (void)daegun_ankr_point_count(ankr, glyph, &got32);
        (void)daegun_ankr_anchor_point(ankr, glyph, state, &x, &y);
    }
    daegun_ankr_free(ankr);

    daegun_ivs *ivs = NULL;
    if (daegun_ivs_parse(data, len, off, &ivs) == DAEGUN_OK) {
        size_t axes = 0, regions = 0, ivds = 0, rows = 0;
        (void)daegun_ivs_axis_count(ivs, &axes);
        (void)daegun_ivs_region_count(ivs, &regions);
        (void)daegun_ivs_ivd_count(ivs, &ivds);
        LENT(daegun_ivs_region, ivs, index);
        (void)daegun_ivs_ivd_rows(ivs, index, &rows);
        LENT(daegun_ivs_ivd_region_indices, ivs, index);
        LENT(daegun_ivs_ivd_row, ivs, index, off);
        double location[4] = { number(c), number(c), number(c), number(c) }, scalars[32] = { 0 }, delta = 0;
        (void)daegun_ivs_region_scalars(ivs, location, byte(c) % 5, scalars, room, &count);
        (void)daegun_ivs_delta(ivs, index, off, scalars, room < count ? room : count, &delta);
    }
    daegun_delta_set_index_map *map = NULL;
    if (daegun_delta_set_index_map_parse(data, len, off, &map) == DAEGUN_OK) {
        size_t outer = 0, inner = 0;
        (void)daegun_delta_set_index_map_count(map, &count);
        (void)daegun_delta_set_index_map_lookup(map, index, &outer, &inner);
    }
    daegun_delta_set_index_map_free(map);

    for (int way = 0; way < 2; way++) {
        daegun_feature_variations *variations = NULL;
        daegun_status st = way ? daegun_feature_variations_at(data, len, off, &variations)
                               : daegun_feature_variations_parse(data, len, &variations);
        if (st == DAEGUN_OK) {
            int32_t coords[3] = { (int32_t)word(c), (int32_t)word(c), (int32_t)word(c) };
            (void)daegun_feature_variations_find(variations, coords, byte(c) % 4, ivs, &got32);
            (void)daegun_feature_variations_substitute(variations, glyph, state, &got);
        }
        daegun_feature_variations_free(variations);
    }
    daegun_ivs_free(ivs);

    size_t loca[32] = { 0 };
    int16_t format = (int16_t)(byte(c) % 3) - 1;
    daegun_pen pen = { to, to, quad, curve, close_path, NULL };
    (void)daegun_parse_loca(data, len, format, glyphs, loca, room, &count);
    (void)daegun_outline_glyf_bytes(data, len, byte(c) & 1, loca, count < room ? count : room, glyph, &pen);
}

/* The other ways a font comes in. */
static void doors(choices *c, const uint8_t *bytes, size_t len)
{
    size_t faces = 0, index = byte(c) % 4;
    (void)daegun_face_count(bytes, len, &faces);
    daegun_font *font = NULL;
    (void)daegun_font_open_static(bytes, len, index, &font);
    daegun_font_free(font);
    uint8_t *owned = daegun_font_buffer_new(len);
    if (owned) {
        memcpy(owned, bytes, len);
        font = NULL;
        (void)daegun_font_open_owned(owned, len, index, &font);
        daegun_font_free(font);
    }
    size_t spare = byte(c);
    daegun_font_buffer_free(daegun_font_buffer_new(spare), spare);
}

/* Each _default refuses a size it doesn't serve and then writes nothing. */
#define REFUSES(type, call)                                 \
    do {                                                    \
        union {                                             \
            type options;                                   \
            uint8_t bytes[sizeof(type) + 16];               \
        } u;                                                \
        memset(&u, 0xa5, sizeof u);                         \
        size_t size = byte(c) % sizeof u;                   \
        if (call(&u.options, size) != DAEGUN_OK) {          \
            for (size_t i = 0; i < sizeof u; i++) {         \
                if (u.bytes[i] != 0xa5) {                   \
                    fail();                                 \
                }                                           \
            }                                               \
        } else if (size != sizeof(type)) {                  \
            fail();                                         \
        }                                                   \
    } while (0)

static void defaults(choices *c)
{
    REFUSES(daegun_shape_options, daegun_shape_options_default);
    REFUSES(daegun_layout_options, daegun_layout_options_default);
    REFUSES(daegun_justify_options, daegun_justify_options_default);
    REFUSES(daegun_outline_options, daegun_outline_options_default);
    REFUSES(daegun_scene_options, daegun_scene_options_default);
    REFUSES(daegun_raster_options, daegun_raster_options_default);
}

static choices stage(const uint8_t *data, size_t size, int which)
{
    size_t at = (size_t)which * STAGE;
    size_t left = at < size ? size - at : 0;
    choices c = { data + (at < size ? at : size), left < STAGE ? left : STAGE };
    return c;
}

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size);

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size)
{
    choices c = stage(data, size, 0);
    defaults(&c);
    reading_codes(&c);
    c = stage(data, size, 1);
    paths(&c);
    if (size <= CHOICES) {
        return 0;
    }
    const uint8_t *bytes = data + CHOICES;
    size_t len = size - CHOICES;

    c = stage(data, size, 2);
    doors(&c, bytes, len);
    daegun_font *font = NULL;
    if (daegun_font_open(bytes, len, &font) != DAEGUN_OK
        && daegun_font_open_collection(bytes, len, byte(&c) % 4, &font) != DAEGUN_OK) {
        return 0;
    }
    daegun_coord coords[2] = { { DAEGUN_TAG('w', 'g', 'h', 't'), number(&c) }, { word(&c), number(&c) } };
    daegun_location *at = NULL;
    (void)daegun_font_location(font, coords, byte(&c) % 3, &at);

    char room[32];
    size_t text_len = 0;
    const char *t = text(&c, &text_len, room);
    shaping(&c, font, at, t, text_len);
    c = stage(data, size, 3);
    laying_out(&c, font, at, t, text_len);
    c = stage(data, size, 4);
    reading_font(&c, font, at);
    c = stage(data, size, 5);
    reading_text(&c, font, at, t, text_len);
    c = stage(data, size, 6);
    structures(&c, font);
    c = stage(data, size, 7);
    for (int i = 0; i < 3; i++) {
        drawing(&c, font, i == 2 ? NULL : at, a_glyph(&c, font));
    }
    c = stage(data, size, 8);
    for (int i = 0; i < 3; i++) {
        reading_glyph(&c, font, i == 2 ? NULL : at, a_glyph(&c, font));
    }

    daegun_location_free(at);
    daegun_font_free(font);
    return 0;
}
